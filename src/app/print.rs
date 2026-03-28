//! Print mode (non-interactive) for Patina.
//!
//! When Patina is invoked with `-p` / `--print`, this module handles
//! the non-interactive flow: send a single prompt, stream the response
//! to stdout, execute any requested tools, and exit.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::mpsc;
use tracing::warn;

use crate::api::provider::RequestOptions;
use crate::api::{LlmProvider, StreamEvent, ToolChoice};
use crate::types::{ApiMessageV2, Message, Role};

use super::state::AppState;
use super::tool_loop::ToolLoopState;
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
    rx: &mut mpsc::Receiver<StreamEvent>,
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
                if let Err(e) = state.tool_state_mut().tool_loop_mut().start_streaming() {
                    tracing::warn!("Failed to start streaming in print mode tool use: {}", e);
                }
                state.handle_tool_use_start(id, name, index);
            }
            StreamEvent::ToolUseInputDelta {
                index,
                partial_json,
            } => {
                state.handle_tool_use_input_delta(index, &partial_json);
            }
            StreamEvent::ToolUseComplete { index } => {
                state.tool_state_mut().handle_tool_use_complete(index)?;
            }
            _ => {}
        }
    }

    // Channel closed without explicit completion
    Ok(PrintStreamResult::Completed(response))
}

/// Creates a streaming channel and spawns the initial API call.
///
/// The user prompt must already be added to `state` before calling this.
/// Returns the receiving end of the channel for stream event consumption.
async fn setup_initial_stream(
    state: &AppState,
    client: &Arc<dyn LlmProvider>,
) -> mpsc::Receiver<StreamEvent> {
    let tools = state.all_tool_definitions();
    let (tx, rx) = mpsc::channel(STREAMING_CHANNEL_BUFFER);
    let api_messages = state.build_api_messages();
    let client_clone = Arc::clone(client);

    tokio::spawn(async move {
        if let Err(e) = client_clone
            .stream_message(
                &api_messages,
                Some(&tools),
                Some(&ToolChoice::Auto),
                &RequestOptions::default(),
                tx,
            )
            .await
        {
            tracing::error!("API error: {}", e);
        }
    });

    rx
}

/// Runs the tool-continuation cycle until no more tools are pending.
///
/// Each iteration: approve tools, execute them, send results back to the API,
/// and stream the continuation response.
async fn run_tool_continuation_cycle(
    state: &mut AppState,
    client: &Arc<dyn LlmProvider>,
) -> Result<()> {
    while matches!(
        state.tool_state().tool_loop_state(),
        ToolLoopState::PendingApproval
    ) {
        // Auto-approve all tools in non-interactive mode
        state.tool_state_mut().approve_all_tools()?;

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
        state.complete_tool_cycle()?;

        let (tx, mut rx) = mpsc::channel(STREAMING_CHANNEL_BUFFER);
        let api_messages = state
            .prepare_api_messages_for_send(client.model(), Some(client))
            .await;
        let client_clone = Arc::clone(client);
        let tools = state.all_tool_definitions();

        tokio::spawn(async move {
            if let Err(e) = client_clone
                .stream_message(
                    &api_messages,
                    Some(&tools),
                    Some(&ToolChoice::Auto),
                    &RequestOptions::default(),
                    tx,
                )
                .await
            {
                tracing::error!("API error during tool continuation: {}", e);
            }
        });

        // Process the continuation using the same helper
        match process_print_stream(&mut rx, state).await? {
            PrintStreamResult::Completed(_) => {} // Continue loop if more tools
            PrintStreamResult::Error(e) => {
                warn!("Error during tool continuation: {}", e);
                break;
            }
        }
    }

    Ok(())
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
    let client: Arc<dyn LlmProvider> =
        Arc::from(crate::api::provider_factory::create_provider(config)?);
    // In bare mode, disable plugins and subagents for faster startup
    let plugins_enabled = config.plugins_enabled && !config.is_bare();
    let subagents_enabled = config.subagents_enabled && !config.is_bare();
    let mut state = AppState::with_performance_config(
        config.working_dir.clone(),
        config.skip_permissions,
        &config.performance,
        plugins_enabled,
        subagents_enabled,
    );

    // Initialize compression orchestrator for CCG context management (skipped in bare mode)
    if !config.is_bare() {
        initialize_compression_orchestrator(&mut state, config);
    }

    // Initialize MCP servers from .mcp.json / ~/.claude.json (skipped in bare mode)
    if !config.is_bare() {
        if let Some(manager) = initialize_mcp_servers(&config.working_dir).await {
            state.set_mcp_manager(manager);
        }
    }

    // Refresh CCG context before the first API call (skipped in bare mode)
    if !config.is_bare() {
        state.refresh_build_context().await;
    }

    // Build the API message content, optionally with CCG context (mirrors submit_message)
    let api_content = if state.compression().auto_context_enabled() {
        if let Some(context) = state.compression_mut().take_cached_ccg_context() {
            tracing::info!(
                context_len = context.len(),
                "Injecting CCG context into user message (print mode)"
            );
            format!("<context>\n{}\n</context>\n\n{}", context, prompt)
        } else {
            prompt.to_string()
        }
    } else {
        prompt.to_string()
    };

    // Timeline shows original user input (cleaner UI)
    state.add_message(Message {
        role: Role::User,
        content: prompt.to_string(),
    });
    // API gets potentially context-augmented message
    let user_msg = ApiMessageV2::user(&api_content);
    state.api_messages_mut().push(user_msg);

    // Stream the initial response
    let mut rx = setup_initial_stream(&state, &client).await;

    // Collect and print the response
    let response = match process_print_stream(&mut rx, &mut state).await? {
        PrintStreamResult::Completed(text) => text,
        PrintStreamResult::Error(e) => return Err(anyhow::anyhow!("API error: {}", e)),
    };

    // If there are no tool uses, add the assistant message to both display and API
    if !response.is_empty()
        && !matches!(
            state.tool_state().tool_loop_state(),
            ToolLoopState::PendingApproval
        )
    {
        state.add_message(Message {
            role: Role::Assistant,
            content: response.clone(),
        });
        state
            .api_messages_mut()
            .push(ApiMessageV2::assistant(&response));
    }

    // Run tool continuation cycle until Claude is done
    run_tool_continuation_cycle(&mut state, &client).await?;

    // Shut down MCP servers
    if let Some(manager) = state.mcp_manager_mut() {
        manager.shutdown_all().await;
    }

    Ok(())
}
