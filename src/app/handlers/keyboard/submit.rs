//! Submit handling: user input submission, slash commands, and command actions.

use anyhow::Result;
use tracing::debug;

use crate::app::context::AppContext;
use crate::mcp::manager::ServerStatus;
use crate::types::{Message, Role};

/// Handles Enter key: submits input as a slash command or API message.
///
/// # Errors
///
/// Returns an error if API message submission fails.
pub(super) async fn handle_submit(ctx: &mut AppContext<'_>) -> Result<()> {
    let input = ctx.state.take_input();

    if input.trim().starts_with('/') {
        handle_slash_command(ctx, &input);
    } else {
        // Resolve @-mentions: read referenced files and prepend context
        let (file_context, original_input) =
            crate::app::completion::resolve_mentions(&input, &ctx.state.working_dir);
        let final_input = if file_context.is_empty() {
            original_input
        } else {
            format!("{file_context}{original_input}")
        };

        // Fire UserPromptSubmit hook before sending to API
        if let Err(e) = ctx
            .state
            .tool_state()
            .tool_executor
            .hooks()
            .fire_user_prompt_submit(&final_input)
            .await
        {
            debug!("hook fire failed: {e}");
        }
        ctx.state.submit_message(&ctx.client, final_input).await?;
        // SessionHandler observer saves when it sees the dirty flag.
        ctx.state.session_tracking_mut().mark_dirty();
    }

    Ok(())
}

/// Processes a slash command and displays the result in the timeline.
fn handle_slash_command(ctx: &mut AppContext<'_>, input: &str) {
    use crate::app::commands::{CommandResult, SlashCommandHandler};

    let plugin_info = SlashCommandHandler::build_plugin_info(ctx.state.plugins());
    let mcp_info = build_mcp_server_info(ctx.state);
    let cost_summary = ctx.state.cost_summary();
    let session_id = ctx.state.session_tracking().id().map(String::from);
    let plan_summary = ctx
        .state
        .pending_plan()
        .map(|p| crate::app::commands::PlanSummary {
            title: p.title.clone(),
            steps: p.steps.iter().map(|s| s.description.clone()).collect(),
        });
    let mut handler = SlashCommandHandler::new(ctx.state.working_dir.clone())
        .with_plugins(plugin_info)
        .with_mcp_info(mcp_info)
        .with_cost_summary(cost_summary)
        .with_session_id(session_id)
        .with_plan_summary(plan_summary);

    // Only clone messages for commands that actually need them (/context, /export)
    let cmd = input.split_whitespace().next().unwrap_or("");
    if cmd == "/context" || cmd == "/export" {
        let messages: Vec<_> = ctx
            .state
            .api_messages()
            .iter()
            .map(|m| m.to_legacy())
            .collect();
        let context_limit = crate::api::tokens::model_context_limit(ctx.client.model());
        handler = handler.with_messages(messages, context_limit);
    }

    let result = handler.handle(input);

    // Display the user's command in timeline
    ctx.state.add_message(Message {
        role: Role::User,
        content: input.to_string(),
    });

    // Display the command result
    let response = match result {
        CommandResult::Executed(output) => output,
        CommandResult::NotACommand => {
            format!("Input doesn't look like a command: {input}")
        }
        CommandResult::UnknownCommand(cmd) => {
            format!("Unknown command: /{cmd}. Type /help for available commands.")
        }
        CommandResult::Error(err) => {
            format!("Error: {err}")
        }
        CommandResult::Action(action) => {
            handle_command_action(ctx, action);
            return;
        }
    };

    ctx.state.add_message(Message {
        role: Role::Assistant,
        content: response,
    });

    ctx.state.mark_full_redraw();
}

/// Handles a `CommandAction` that requires state mutation.
fn handle_command_action(
    ctx: &mut crate::app::context::AppContext<'_>,
    action: crate::app::commands::CommandAction,
) {
    use crate::app::commands::CommandAction;

    let response = match action {
        CommandAction::Compact {
            custom_instructions,
        } => {
            let msg = if custom_instructions.is_some() {
                "Context compaction requested with custom instructions."
            } else {
                "Context compaction requested."
            };
            ctx.state.force_compact(custom_instructions);
            msg.to_string()
        }
        CommandAction::Clear => {
            ctx.state.clear_conversation();
            "Conversation cleared.".to_string()
        }
        CommandAction::SetModel(model_name) => {
            match crate::api::AnthropicClient::with_model(&model_name) {
                Ok(new_client) => {
                    ctx.client = std::sync::Arc::new(new_client);
                    ctx.state
                        .model_config_mut()
                        .set_current_model(model_name.clone());
                    format!("Switched to model: {model_name}")
                }
                Err(e) => format!("Failed to switch model: {e}"),
            }
        }
        CommandAction::CopyToClipboard { message_index } => {
            // Find the message to copy
            let messages = ctx.state.messages();
            let msg = match message_index {
                Some(idx) => messages.get(idx),
                None => messages.iter().rev().find(|m| m.role == Role::Assistant),
            };
            match msg {
                Some(m) => {
                    // Attempt to copy to clipboard via pbcopy (macOS) or xclip (Linux)
                    let copy_cmd = if cfg!(target_os = "macos") {
                        "pbcopy"
                    } else {
                        "xclip -selection clipboard"
                    };
                    match std::process::Command::new("sh")
                        .args(["-c", copy_cmd])
                        .stdin(std::process::Stdio::piped())
                        .spawn()
                        .and_then(|mut child| {
                            use std::io::Write;
                            if let Some(stdin) = child.stdin.as_mut() {
                                stdin.write_all(m.content.as_bytes())?;
                            }
                            child.wait()
                        }) {
                        Ok(_) => "Copied to clipboard.".to_string(),
                        Err(e) => format!("Failed to copy: {e}"),
                    }
                }
                None => "No message to copy.".to_string(),
            }
        }
        CommandAction::RenameSession(name) => {
            ctx.state
                .session_tracking_mut()
                .set_name(Some(name.clone()));
            format!("Session renamed to: {name}")
        }
        CommandAction::SetColor(color) => {
            ctx.state
                .display_mut()
                .set_prompt_color(Some(color.clone()));
            format!("Prompt color set to: {color}")
        }
        CommandAction::SideQuestion(question) => {
            format!("(btw) {question}")
        }
        CommandAction::ForkSession { branch_name } => {
            let label = branch_name.as_deref().unwrap_or("unnamed");
            format!("Fork requested: branch '{label}'. Use session manager to complete the fork.")
        }
        CommandAction::ShowRewindPicker => {
            "Rewind picker requested. Use /rewind in TUI mode to select a checkpoint.".to_string()
        }
        CommandAction::PlanAccept => {
            if let Some(result) = ctx.state.approve_plan() {
                let tool_id = result.tool_use_id.clone();
                ctx.state.record_tool_result(&tool_id, result);
                "Plan accepted. Proceeding with execution.".to_string()
            } else {
                "No plan is currently pending.".to_string()
            }
        }
        CommandAction::PlanReject => {
            if let Some(result) = ctx.state.reject_plan() {
                let tool_id = result.tool_use_id.clone();
                ctx.state.record_tool_result(&tool_id, result);
                "Plan rejected.".to_string()
            } else {
                "No plan is currently pending.".to_string()
            }
        }
    };

    ctx.state.add_message(Message {
        role: Role::Assistant,
        content: response,
    });
    ctx.state.mark_full_redraw();
}

/// Builds MCP server info from the current application state.
fn build_mcp_server_info(
    state: &crate::app::state::AppState,
) -> Vec<crate::app::commands::McpServerInfo> {
    use crate::app::commands::McpServerInfo;

    let Some(manager) = state.mcp_manager() else {
        return Vec::new();
    };

    manager
        .server_details()
        .into_iter()
        .map(|(name, status, tool_count)| {
            let status_str = match status {
                ServerStatus::Starting => "Starting".to_string(),
                ServerStatus::Connected => "Connected".to_string(),
                ServerStatus::Failed(reason) => format!("Failed: {reason}"),
                ServerStatus::Stopped => "Stopped".to_string(),
            };
            McpServerInfo {
                name: name.to_string(),
                status: status_str,
                tool_count,
            }
        })
        .collect()
}
