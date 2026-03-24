//! Print mode (non-interactive) for Patina.
//!
//! When Patina is invoked with `-p` / `--print`, this module handles
//! the non-interactive flow: send a single prompt, stream the response
//! to stdout, execute any requested tools, and exit.

use anyhow::Result;
use tracing::warn;

use crate::api::{LlmProvider, StreamEvent, ToolChoice};
use crate::types::{ApiMessageV2, Message, Role};

use super::state::AppState;
use super::tool_loop::{self, ToolLoopState};
use super::{
    initialize_compression_orchestrator, initialize_mcp_servers, Config, STREAMING_CHANNEL_BUFFER,
};

/// Result of processing a print mode stream.
pub(crate) enum PrintStreamResult {
    /// Stream completed successfully (MessageStop or MessageComplete).
    Completed(String),
    /// Stream ended with an error.
    Error(String),
}

/// Processes a print mode stream, printing content and handling tool use events.
///
/// Returns the accumulated response text and the final result.
pub(crate) async fn process_print_stream(
    rx: &mut tokio::sync::mpsc::Receiver<StreamEvent>,
    state: &mut AppState,
) -> Result<PrintStreamResult> {
    let mut response = String::new();

    while let Some(event) = rx.recv().await {
        match event {
            StreamEvent::ContentDelta(text) => {
                print!("{}", text);
                response.push_str(&text);
            }
            StreamEvent::MessageStop | StreamEvent::MessageComplete { .. } => {
                println!(); // Newline after response
                return Ok(PrintStreamResult::Completed(response));
            }
            StreamEvent::Error(e) => {
                eprintln!("Error: {}", e);
                return Ok(PrintStreamResult::Error(e));
            }
            StreamEvent::ToolUseStart { id, name, index } => {
                state.tool_loop_mut().start_streaming().ok();
                state.handle_tool_use_start(id, name, index);
            }
            StreamEvent::ToolUseInputDelta {
                index,
                partial_json,
            } => {
                state.handle_tool_use_input_delta(index, &partial_json);
            }
            StreamEvent::ToolUseComplete { index } => {
                state.handle_tool_use_complete(index)?;
            }
            _ => {}
        }
    }

    // Channel closed without explicit completion
    Ok(PrintStreamResult::Completed(response))
}

/// Runs in print mode (non-interactive).
///
/// This function:
/// 1. Sends the prompt to Claude
/// 2. Streams and prints the response to stdout
/// 3. Executes any tools Claude requests
/// 4. Continues the conversation until Claude is done
/// 5. Exits
///
/// This matches Claude Code's `-p` / `--print` flag behavior.
pub(crate) async fn run_print_mode(config: &Config, prompt: &str) -> Result<()> {
    let client: std::sync::Arc<dyn LlmProvider> =
        std::sync::Arc::from(crate::api::provider::create_provider(config));
    let mut state = AppState::with_performance_config(
        config.working_dir.clone(),
        config.skip_permissions,
        &config.performance,
        config.plugins_enabled,
        config.subagents_enabled,
    );

    // Initialize compression orchestrator for CCG context management
    // This enables auto-context injection in print mode when narsil is available
    initialize_compression_orchestrator(&mut state, config);

    // Initialize MCP servers from .mcp.json / ~/.claude.json
    if let Some(manager) = initialize_mcp_servers(&config.working_dir).await {
        state.set_mcp_manager(manager);
    }

    // Refresh CCG context before the first API call (async, requires narsil MCP)
    state.refresh_build_context().await;

    // Add the user's prompt (adds to both display and API messages via submit logic)
    let user_msg = ApiMessageV2::user(prompt);
    state.add_message(Message {
        role: Role::User,
        content: prompt.to_string(),
    });
    state.api_messages_mut().push(user_msg);

    // Set up streaming using build_api_messages which includes context injection
    let tools = state.all_tool_definitions();
    let (tx, mut rx) = tokio::sync::mpsc::channel(STREAMING_CHANNEL_BUFFER);
    let api_messages = state.build_api_messages();
    let client_clone = std::sync::Arc::clone(&client);
    let tools_clone = tools.clone();

    tokio::spawn(async move {
        if let Err(e) = client_clone
            .stream_message(
                &api_messages,
                Some(&tools_clone),
                Some(&ToolChoice::Auto),
                tx,
            )
            .await
        {
            tracing::error!("API error: {}", e);
        }
    });

    // Collect and print the response
    let response = match process_print_stream(&mut rx, &mut state).await? {
        PrintStreamResult::Completed(text) => text,
        PrintStreamResult::Error(e) => return Err(anyhow::anyhow!("API error: {}", e)),
    };

    // If there are no tool uses, add the assistant message to both display and API
    if !response.is_empty() && !matches!(state.tool_loop_state(), ToolLoopState::PendingApproval) {
        state.add_message(Message {
            role: Role::Assistant,
            content: response.clone(),
        });
        state
            .api_messages_mut()
            .push(ApiMessageV2::assistant(&response));
    }

    // Handle any tool execution if needed
    while matches!(state.tool_loop_state(), ToolLoopState::PendingApproval) {
        // Auto-approve all tools in non-interactive mode
        state.approve_all_tools()?;

        // Execute the tools
        let needs_permission = state.execute_pending_tools().await?;

        // Check if any tools still need permission
        if !needs_permission.is_empty() {
            warn!(
                "Tools need permission in print mode (skipping): {:?}",
                needs_permission
            );
            break;
        }

        // Complete tool execution: build messages, add to history, truncate
        tool_loop::complete_tool_cycle(&mut state)?;

        let (tx, mut rx) = tokio::sync::mpsc::channel(STREAMING_CHANNEL_BUFFER);
        let api_messages = state.prepare_api_messages_for_send(client.model()).await;
        let client_clone = std::sync::Arc::clone(&client);
        let tools = state.all_tool_definitions();

        tokio::spawn(async move {
            if let Err(e) = client_clone
                .stream_message(&api_messages, Some(&tools), Some(&ToolChoice::Auto), tx)
                .await
            {
                tracing::error!("API error during tool continuation: {}", e);
            }
        });

        // Process the continuation using the same helper
        match process_print_stream(&mut rx, &mut state).await? {
            PrintStreamResult::Completed(_) => {} // Continue loop if more tools
            PrintStreamResult::Error(e) => {
                warn!("Error during tool continuation: {}", e);
                break;
            }
        }
    }

    // Shut down MCP servers
    if let Some(manager) = state.mcp_manager_mut() {
        manager.shutdown_all().await;
    }

    Ok(())
}
