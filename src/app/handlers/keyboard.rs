//! Keyboard, mouse, and resize event handler.
//!
//! [`KeyboardHandler`] processes all user input events: keyboard shortcuts,
//! text entry, mouse interactions (click, drag, scroll), and terminal resizes.
//! It translates raw crossterm events into application state mutations.
//!
//! This handler **must** run after `PermissionHandler` in the dispatch chain
//! so that permission prompt key events are consumed before reaching normal
//! input handling.

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use tracing::{debug, info, warn};

use crate::app::context::AppContext;
use crate::app::dispatch::{EventHandler, Handled};
use crate::app::events::AppEvent;
use crate::app::state::UISelectionState;
use crate::types::ui_state::{ContentPosition, FocusArea};
use crate::types::{Message, Role};

/// Handles keyboard, mouse, and resize events.
///
/// `KeyboardHandler` is the primary user-input handler. It processes:
///
/// - **Exit**: Ctrl+C, Ctrl+D → signals quit via `AppEvent::Quit` return
/// - **Submit**: Enter → sends input to API or executes slash commands
/// - **Edit**: Backspace → deletes character, printable chars → inserts
/// - **Scroll**: Ctrl+Up/Down, PageUp/Down, Ctrl+K/J, Home/End → viewport
/// - **Select**: Cmd/Alt/Ctrl+A → select all content
/// - **Copy**: Cmd/Alt/Ctrl+Shift+C, Ctrl+Y → copy selection to clipboard
/// - **Paste**: Cmd/Alt/Ctrl+Shift+V → paste from clipboard
/// - **Escape**: clears active selection
/// - **Mouse**: click sets focus, drag extends selection, scroll moves viewport
/// - **Resize**: marks the UI for a full redraw
///
/// # Examples
///
/// ```rust,ignore
/// use patina::app::handlers::keyboard::KeyboardHandler;
/// use patina::app::dispatch::EventDispatcher;
///
/// let dispatcher = EventDispatcher::new(vec![
///     Box::new(permission_handler),
///     Box::new(KeyboardHandler),  // After PermissionHandler
///     Box::new(stream_handler),
/// ]);
/// ```
pub struct KeyboardHandler;

impl EventHandler for KeyboardHandler {
    fn handle<'a>(
        &'a mut self,
        event: &'a AppEvent,
        ctx: &'a mut AppContext<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<Handled>> + Send + 'a>> {
        Box::pin(async move {
            match event {
                AppEvent::Key(key) => {
                    // Skip key release events — only process Press and Repeat.
                    // Prevents character duplication when REPORT_EVENT_TYPES is enabled.
                    if key.kind == KeyEventKind::Release {
                        return Ok(Handled::CONSUMED);
                    }

                    handle_key(ctx, key.code, key.modifiers).await
                }
                AppEvent::Mouse(mouse) => {
                    let terminal_height = ctx.state.display().terminal_height();
                    handle_mouse(ctx, mouse.kind, mouse.row, mouse.column, terminal_height);
                    Ok(Handled::CONSUMED)
                }
                AppEvent::Resize { width: _, height } => {
                    ctx.state.display_mut().set_terminal_height(*height);
                    ctx.state.mark_full_redraw();
                    Ok(Handled::CONSUMED)
                }
                _ => Ok(Handled::IGNORED),
            }
        })
    }

    fn name(&self) -> &str {
        "keyboard"
    }
}

/// Processes a single key event and mutates application state accordingly.
///
/// Returns [`Handled::CONSUMED`] for recognized key combinations. Ctrl+C and
/// Ctrl+D return a special quit signal that the event loop must detect.
///
/// # Errors
///
/// Returns an error if message submission or session saving fails.
async fn handle_key(
    ctx: &mut AppContext<'_>,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Result<Handled> {
    debug!(?code, ?modifiers, "key event received");

    match (code, modifiers) {
        // Exit commands
        (KeyCode::Char('c'), KeyModifiers::CONTROL)
        | (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
            // Signal quit — the event loop detects this via `wants_quit`.
            ctx.state.request_quit();
            Ok(Handled::CONSUMED)
        }

        // Submit input
        (KeyCode::Enter, KeyModifiers::NONE) if !ctx.state.input_state().text().is_empty() => {
            handle_submit(ctx).await?;
            Ok(Handled::CONSUMED)
        }

        // Delete character
        (KeyCode::Backspace, _) => {
            ctx.state.delete_char();
            Ok(Handled::CONSUMED)
        }

        // Scroll up: Ctrl+Up, PageUp, Ctrl+K (vim-style)
        (KeyCode::Up, KeyModifiers::CONTROL)
        | (KeyCode::PageUp, _)
        | (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
            debug!("scroll_up triggered");
            ctx.state.scroll_up(10);
            Ok(Handled::CONSUMED)
        }

        // Scroll down: Ctrl+Down, PageDown, Ctrl+J (vim-style)
        (KeyCode::Down, KeyModifiers::CONTROL)
        | (KeyCode::PageDown, _)
        | (KeyCode::Char('j'), KeyModifiers::CONTROL) => {
            debug!("scroll_down triggered");
            ctx.state.scroll_down(10);
            Ok(Handled::CONSUMED)
        }

        // Scroll to top: Home, Ctrl+G
        (KeyCode::Home, _) | (KeyCode::Char('g'), KeyModifiers::CONTROL) => {
            debug!("scroll_to_top triggered");
            ctx.state.scroll_to_top();
            Ok(Handled::CONSUMED)
        }

        // Scroll to bottom: End
        (KeyCode::End, _) => {
            debug!("scroll_to_bottom triggered");
            let height = ctx.state.display().scroll_state().content_height();
            ctx.state.scroll_to_bottom(height);
            Ok(Handled::CONSUMED)
        }

        // Select all: Cmd+A, Option+A, Ctrl+A, Ctrl+Shift+A
        (KeyCode::Char('a') | KeyCode::Char('A'), modifiers)
            if modifiers == KeyModifiers::SUPER
                || modifiers == KeyModifiers::ALT
                || modifiers == KeyModifiers::CONTROL
                || modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT) =>
        {
            handle_select_all(ctx, modifiers);
            Ok(Handled::CONSUMED)
        }

        // Copy selection: Cmd+C, Option+C, Ctrl+Shift+C
        (KeyCode::Char('c') | KeyCode::Char('C'), modifiers)
            if modifiers == KeyModifiers::SUPER
                || modifiers == KeyModifiers::ALT
                || modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT) =>
        {
            debug!(modifier = ?modifiers, "copy triggered");
            handle_copy(ctx.state);
            Ok(Handled::CONSUMED)
        }

        // Alternative copy: Ctrl+Y (yank)
        (KeyCode::Char('y') | KeyCode::Char('Y'), KeyModifiers::CONTROL) => {
            handle_copy(ctx.state);
            Ok(Handled::CONSUMED)
        }

        // Paste: Cmd+V, Option+V, Ctrl+Shift+V
        (KeyCode::Char('v') | KeyCode::Char('V'), modifiers)
            if modifiers == KeyModifiers::SUPER
                || modifiers == KeyModifiers::ALT
                || modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT) =>
        {
            debug!(modifier = ?modifiers, "paste triggered");
            handle_paste(ctx.state);
            Ok(Handled::CONSUMED)
        }

        // Clear selection: Escape (only when selection is active)
        (KeyCode::Esc, KeyModifiers::NONE)
            if ctx.state.ui_selection().selection().has_selection() =>
        {
            ctx.state.ui_selection_mut().selection_mut().clear();
            ctx.state.mark_full_redraw();
            Ok(Handled::CONSUMED)
        }

        // Text input (must come after special char bindings)
        (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            ctx.state.insert_char(c);
            Ok(Handled::CONSUMED)
        }

        _ => {
            debug!(?code, ?modifiers, "unhandled key");
            Ok(Handled::CONSUMED)
        }
    }
}

/// Handles Enter key: submits input as a slash command or API message.
///
/// # Errors
///
/// Returns an error if API message submission fails.
async fn handle_submit(ctx: &mut AppContext<'_>) -> Result<()> {
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
        let _ = ctx
            .state
            .tool_state()
            .tool_executor
            .hooks()
            .fire_user_prompt_submit(&final_input)
            .await;
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
    let mut handler = SlashCommandHandler::new(ctx.state.working_dir.clone())
        .with_plugins(plugin_info)
        .with_mcp_info(mcp_info)
        .with_cost_summary(cost_summary)
        .with_session_id(session_id);

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
    use crate::mcp::manager::ServerStatus;

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

/// Selects all content and sets focus to the content area.
fn handle_select_all(ctx: &mut AppContext<'_>, modifiers: KeyModifiers) {
    let line_count = ctx.state.ui_selection().rendered_line_count();
    let timeline_len = ctx.state.timeline().len();
    let modifier_name = if modifiers == KeyModifiers::SUPER {
        "Cmd+A"
    } else if modifiers == KeyModifiers::ALT {
        "Option+A"
    } else {
        "Ctrl+A"
    };
    debug!(
        line_count,
        timeline_len,
        modifier = modifier_name,
        focus_area = ?ctx.state.ui_selection().focus_area(),
        "select_all triggered via {}",
        modifier_name
    );
    if line_count == 0 {
        debug!(
            "select_all: no content to select (cache empty, timeline_len={})",
            timeline_len
        );
    } else {
        ctx.state
            .ui_selection_mut()
            .set_focus_area(FocusArea::Content);
        ctx.state
            .ui_selection_mut()
            .selection_mut()
            .select_all(line_count);
        ctx.state.mark_full_redraw();
        let copy_hint = if modifiers == KeyModifiers::SUPER {
            "Cmd+C"
        } else if modifiers == KeyModifiers::ALT {
            "Option+C"
        } else {
            "Ctrl+Y"
        };
        info!(
            line_count,
            "Selected all {} lines ({} to copy)", line_count, copy_hint
        );
    }
}

/// Copies the current selection to the system clipboard.
fn handle_copy(state: &crate::app::state::AppState) {
    let selection = state.ui_selection().selection();
    let cache_len = state.ui_selection().rendered_line_count();

    if let Some((start, end)) = selection.range() {
        let selected_lines = end.line.saturating_sub(start.line) + 1;
        debug!(
            start_line = start.line,
            end_line = end.line,
            selected_lines,
            cache_len,
            "copy: attempting to copy {} lines from cache of {} lines",
            selected_lines,
            cache_len
        );

        match state.ui_selection().copy_from_cache() {
            Ok(true) => {
                info!("Copied {} lines to clipboard", selected_lines);
            }
            Ok(false) => {
                warn!(
                    "copy: no text extracted (cache_len={}, selection=L{}-L{})",
                    cache_len, start.line, end.line
                );
            }
            Err(e) => {
                warn!("copy: clipboard error: {}", e);
            }
        }
    } else {
        debug!(
            "copy: no selection (has_selection={})",
            selection.has_selection()
        );
    }
}

/// Pastes text from the system clipboard into the input buffer.
fn handle_paste(state: &mut crate::app::state::AppState) {
    match crate::tui::clipboard::paste_from_clipboard() {
        Ok(text) => {
            for c in text.chars() {
                // Skip control characters except newlines
                if c == '\n' || (!c.is_control()) {
                    state.insert_char(c);
                }
            }
            info!(len = text.len(), "Pasted from clipboard");
        }
        Err(e) => {
            warn!("paste: clipboard error: {}", e);
        }
    }
}

/// Processes a mouse event (click, drag, scroll).
fn handle_mouse(
    ctx: &mut AppContext<'_>,
    kind: MouseEventKind,
    row: u16,
    column: u16,
    terminal_height: u16,
) {
    match kind {
        MouseEventKind::Down(MouseButton::Left) => {
            let clicked_area = UISelectionState::focus_area_for_row(row, terminal_height);
            ctx.state.ui_selection_mut().set_focus_area(clicked_area);

            if clicked_area == FocusArea::Content {
                let first_visible = ctx.state.display().scroll_state().first_visible_line();
                let content_row = row.saturating_sub(1) as usize;
                let pos = ContentPosition::new(
                    first_visible + content_row,
                    column.saturating_sub(1) as usize,
                );
                ctx.state.ui_selection_mut().selection_mut().start(pos);
            }
            ctx.state.mark_full_redraw();
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if ctx.state.ui_selection().focus_area() == FocusArea::Content {
                let first_visible = ctx.state.display().scroll_state().first_visible_line();
                let content_row = row.saturating_sub(1) as usize;
                let pos = ContentPosition::new(
                    first_visible + content_row,
                    column.saturating_sub(1) as usize,
                );
                ctx.state.ui_selection_mut().selection_mut().update(pos);
                ctx.state.mark_full_redraw();
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            if ctx.state.ui_selection().focus_area() == FocusArea::Content {
                ctx.state.ui_selection_mut().selection_mut().end();
                ctx.state.mark_full_redraw();
            }
        }
        MouseEventKind::ScrollUp => {
            debug!("mouse scroll up");
            ctx.state.scroll_up(3);
        }
        MouseEventKind::ScrollDown => {
            debug!("mouse scroll down");
            ctx.state.scroll_down(3);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{AnthropicClient, LlmProvider};
    use crate::app::state::AppState;
    use crate::session::SessionManager;
    use crate::types::config::ParallelMode;
    use crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use secrecy::SecretString;
    use serial_test::serial;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::TempDir;

    // =========================================================================
    // Test helpers
    // =========================================================================

    fn test_client() -> Arc<dyn LlmProvider> {
        Arc::new(AnthropicClient::new(
            SecretString::from("test-key"),
            "claude-test",
        ))
    }

    fn test_state() -> AppState {
        AppState::new(PathBuf::from("/tmp/test"), true, ParallelMode::Disabled)
    }

    fn test_session_manager() -> (SessionManager, TempDir) {
        let dir = TempDir::new().expect("failed to create temp dir");
        let mgr = SessionManager::new(dir.path().to_path_buf());
        (mgr, dir)
    }

    // =========================================================================
    // KeyboardHandler::name
    // =========================================================================

    #[test]
    fn name_returns_keyboard() {
        let handler = KeyboardHandler;
        assert_eq!(handler.name(), "keyboard");
    }

    // =========================================================================
    // Non-input events should be IGNORED
    // =========================================================================

    #[tokio::test]
    async fn handle_tick_returns_ignored() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Tick;
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::IGNORED,
            "KeyboardHandler must ignore Tick events"
        );
    }

    #[tokio::test]
    async fn handle_quit_returns_ignored() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Quit;
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::IGNORED,
            "KeyboardHandler must ignore Quit events"
        );
    }

    #[tokio::test]
    async fn handle_api_chunk_returns_ignored() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::ApiChunk(crate::api::StreamEvent::ContentDelta("hi".to_string()));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::IGNORED,
            "KeyboardHandler must ignore ApiChunk events"
        );
    }

    #[tokio::test]
    async fn handle_tool_result_returns_ignored() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::ToolResult {
            tool_id: "toolu_test".to_string(),
            result: crate::types::ToolResultBlock {
                tool_use_id: "toolu_test".to_string(),
                content: "ok".to_string(),
                is_error: false,
            },
        };
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::IGNORED,
            "KeyboardHandler must ignore ToolResult events"
        );
    }

    #[tokio::test]
    async fn handle_permission_response_returns_ignored() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::PermissionResponse(crate::permissions::PermissionResponse::AllowOnce);
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::IGNORED,
            "KeyboardHandler must ignore PermissionResponse events"
        );
    }

    // =========================================================================
    // Key release events should be consumed (filtered out)
    // =========================================================================

    #[tokio::test]
    async fn handle_key_release_returns_consumed() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let mut key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        key.kind = KeyEventKind::Release;
        let event = AppEvent::Key(key);
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "Key release events must be consumed (filtered, not passed through)"
        );
    }

    // =========================================================================
    // Exit commands: Ctrl+C, Ctrl+D → request_quit
    // =========================================================================

    #[tokio::test]
    async fn handle_ctrl_c_requests_quit() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(result, Handled::CONSUMED);
        assert!(
            ctx.state.wants_quit(),
            "Ctrl+C must set the quit flag on state"
        );
    }

    #[tokio::test]
    async fn handle_ctrl_d_requests_quit() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(result, Handled::CONSUMED);
        assert!(
            ctx.state.wants_quit(),
            "Ctrl+D must set the quit flag on state"
        );
    }

    // =========================================================================
    // Character input
    // =========================================================================

    #[tokio::test]
    async fn handle_char_inserts_into_input() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(result, Handled::CONSUMED);
        assert_eq!(
            ctx.state.input_state().text(),
            "x",
            "Character key must insert into the input buffer"
        );
    }

    #[tokio::test]
    async fn handle_shifted_char_inserts_into_input() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::SHIFT));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(result, Handled::CONSUMED);
        assert_eq!(
            ctx.state.input_state().text(),
            "X",
            "Shifted character key must insert into the input buffer"
        );
    }

    // =========================================================================
    // Backspace
    // =========================================================================

    #[tokio::test]
    async fn handle_backspace_deletes_char() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // Pre-populate input
        state.insert_char('a');
        state.insert_char('b');
        assert_eq!(state.input_state().text(), "ab");

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(result, Handled::CONSUMED);
        assert_eq!(
            ctx.state.input_state().text(),
            "a",
            "Backspace must delete the last character"
        );
    }

    // =========================================================================
    // Scroll keys
    // =========================================================================

    #[tokio::test]
    async fn handle_ctrl_up_scrolls_up() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "Ctrl+Up must be consumed as a scroll-up command"
        );
    }

    #[tokio::test]
    async fn handle_page_up_scrolls_up() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "PageUp must be consumed as a scroll-up command"
        );
    }

    #[tokio::test]
    async fn handle_ctrl_k_scrolls_up() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "Ctrl+K must be consumed as a scroll-up command"
        );
    }

    #[tokio::test]
    async fn handle_ctrl_down_scrolls_down() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::CONTROL));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "Ctrl+Down must be consumed as a scroll-down command"
        );
    }

    #[tokio::test]
    async fn handle_page_down_scrolls_down() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "PageDown must be consumed as a scroll-down command"
        );
    }

    #[tokio::test]
    async fn handle_ctrl_j_scrolls_down() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "Ctrl+J must be consumed as a scroll-down command"
        );
    }

    #[tokio::test]
    async fn handle_home_scrolls_to_top() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "Home must be consumed as scroll-to-top"
        );
    }

    #[tokio::test]
    async fn handle_ctrl_g_scrolls_to_top() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "Ctrl+G must be consumed as scroll-to-top"
        );
    }

    #[tokio::test]
    async fn handle_end_scrolls_to_bottom() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "End must be consumed as scroll-to-bottom"
        );
    }

    // =========================================================================
    // Enter with empty input should NOT be consumed as submit
    // =========================================================================

    #[tokio::test]
    async fn handle_enter_with_empty_input_consumed_as_char() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        assert!(state.input_state().text().is_empty());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        // Enter with no input falls through to the `_` catch-all, still CONSUMED
        assert_eq!(
            result,
            Handled::CONSUMED,
            "Enter with empty input is still consumed (unhandled key)"
        );
    }

    // =========================================================================
    // Escape clears selection
    // =========================================================================

    #[tokio::test]
    async fn handle_escape_clears_selection() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // Set up an active selection
        state.ui_selection_mut().selection_mut().select_all(10);
        assert!(state.ui_selection().selection().has_selection());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(result, Handled::CONSUMED);
        assert!(
            !ctx.state.ui_selection().selection().has_selection(),
            "Escape must clear the active selection"
        );
    }

    #[tokio::test]
    async fn handle_escape_without_selection_is_unhandled() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        assert!(!state.ui_selection().selection().has_selection());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        // Esc without selection falls through to catch-all → CONSUMED
        assert_eq!(
            result,
            Handled::CONSUMED,
            "Esc without selection is still consumed (unhandled key catch-all)"
        );
    }

    // =========================================================================
    // Resize events
    // =========================================================================

    #[tokio::test]
    async fn handle_resize_marks_full_redraw() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // Clear any initial render flags
        state.mark_rendered();

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Resize {
            width: 120,
            height: 40,
        };
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(result, Handled::CONSUMED);
        assert!(
            ctx.state.needs_render(),
            "Resize must mark the UI for a full redraw"
        );
    }

    // =========================================================================
    // Mouse events
    // =========================================================================

    #[tokio::test]
    async fn handle_mouse_scroll_up_consumed() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let mouse = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        let event = AppEvent::Mouse(mouse);
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "Mouse scroll up must be consumed"
        );
    }

    #[tokio::test]
    async fn handle_mouse_scroll_down_consumed() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let mouse = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        let event = AppEvent::Mouse(mouse);
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "Mouse scroll down must be consumed"
        );
    }

    #[tokio::test]
    async fn handle_mouse_click_consumed() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        let event = AppEvent::Mouse(mouse);
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "Mouse left click must be consumed"
        );
    }

    // =========================================================================
    // Copy bindings (Cmd+C, Option+C, Ctrl+Shift+C, Ctrl+Y)
    // =========================================================================

    #[tokio::test]
    async fn handle_super_c_consumed_as_copy() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::SUPER));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(result, Handled::CONSUMED, "Cmd+C must be consumed as copy");
    }

    #[tokio::test]
    async fn handle_alt_c_consumed_as_copy() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::ALT));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "Option+C must be consumed as copy"
        );
    }

    #[tokio::test]
    async fn handle_ctrl_shift_c_consumed_as_copy() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(
            KeyCode::Char('C'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "Ctrl+Shift+C must be consumed as copy"
        );
    }

    #[tokio::test]
    async fn handle_ctrl_y_consumed_as_copy() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "Ctrl+Y must be consumed as copy (yank)"
        );
    }

    // =========================================================================
    // Paste bindings (Cmd+V, Option+V, Ctrl+Shift+V)
    // =========================================================================

    #[serial]
    #[tokio::test]
    async fn handle_super_v_consumed_as_paste() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::SUPER));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(result, Handled::CONSUMED, "Cmd+V must be consumed as paste");
    }

    #[serial]
    #[tokio::test]
    async fn handle_alt_v_consumed_as_paste() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::ALT));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "Option+V must be consumed as paste"
        );
    }

    // =========================================================================
    // Select all bindings (Cmd+A, Option+A, Ctrl+A)
    // =========================================================================

    #[tokio::test]
    async fn handle_super_a_consumed_as_select_all() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::SUPER));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "Cmd+A must be consumed as select-all"
        );
    }

    #[tokio::test]
    async fn handle_alt_a_consumed_as_select_all() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "Option+A must be consumed as select-all"
        );
    }

    #[tokio::test]
    async fn handle_ctrl_a_consumed_as_select_all() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "Ctrl+A must be consumed as select-all"
        );
    }

    // =========================================================================
    // Send bound verification
    // =========================================================================

    #[test]
    fn keyboard_handler_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<KeyboardHandler>();
    }

    // =========================================================================
    // Integration with EventDispatcher
    // =========================================================================

    #[tokio::test]
    async fn keyboard_handler_works_in_dispatcher() {
        use crate::app::dispatch::EventDispatcher;

        let handler = KeyboardHandler;
        let mut dispatcher = EventDispatcher::new(vec![Box::new(handler)]);

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        // Key event should be consumed
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        let result = dispatcher.dispatch(&event, &mut ctx).await.unwrap();
        assert_eq!(
            result,
            Handled::CONSUMED,
            "KeyboardHandler should consume Key in dispatcher"
        );

        // Tick should pass through
        let tick = AppEvent::Tick;
        let result = dispatcher.dispatch(&tick, &mut ctx).await.unwrap();
        assert_eq!(
            result,
            Handled::IGNORED,
            "KeyboardHandler should ignore Tick in dispatcher"
        );
    }

    // =========================================================================
    // Unhandled keys are consumed (catch-all)
    // =========================================================================

    #[tokio::test]
    async fn handle_unrecognized_key_consumed() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        // F12 is not a recognized binding
        let event = AppEvent::Key(KeyEvent::new(KeyCode::F(12), KeyModifiers::NONE));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "Unrecognized keys should still be consumed (not passed to later handlers)"
        );
    }
}
