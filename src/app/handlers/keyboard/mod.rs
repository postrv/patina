//! Keyboard, mouse, and resize event handler.
//!
//! [`KeyboardHandler`] processes all user input events: keyboard shortcuts,
//! text entry, mouse interactions (click, drag, scroll), and terminal resizes.
//! It delegates key dispatch to [`KeybindingManager`] for configurable bindings
//! and falls back to direct handling for text input and editing keys.
//!
//! This handler **must** run after `PermissionHandler` in the dispatch chain
//! so that permission prompt key events are consumed before reaching normal
//! input handling.
//!
//! # Sub-modules
//!
//! - [`submit`] — input submission, slash commands, command actions
//! - [`clipboard`] — select all, copy, paste
//! - [`mouse`] — click, drag, scroll handling

mod clipboard;
mod mouse;
mod submit;

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use tracing::{debug, info};

use crate::app::context::AppContext;
use crate::app::dispatch::{EventHandler, Handled};
use crate::app::events::AppEvent;
use crate::keybindings::{Action, KeyPress, KeyResolution};
use crate::types::ui_state::FocusArea;

/// Handles keyboard, mouse, and resize events.
///
/// `KeyboardHandler` delegates key dispatch to [`KeybindingManager`] for
/// configurable bindings (including user overrides from `keybindings.json`)
/// and falls back to direct handling for text input and editing keys.
///
/// Key events flow through: `KeyPress` → `KeybindingManager::resolve()` →
/// `Action` dispatch → state mutation. Unbound keys are handled as text input.
///
/// Mouse events handle click (focus), drag (selection), and scroll.
/// Resize events mark the UI for a full redraw.
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
                AppEvent::Mouse(mouse_evt) => {
                    let terminal_height = ctx.state.display().terminal_height();
                    mouse::handle_mouse(
                        ctx,
                        mouse_evt.kind,
                        mouse_evt.row,
                        mouse_evt.column,
                        terminal_height,
                    );
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

/// Processes a single key event via the [`KeybindingManager`].
///
/// When the transcript search bar is active, most keys are routed to the
/// search input instead of the normal keybinding system. Escape closes
/// search, Enter / Shift+Enter navigate matches. All other keys are
/// appended to / deleted from the search query.
///
/// Outside search mode, keys are resolved through the configurable keybinding
/// system. If the key matches a binding, the corresponding [`Action`] is
/// dispatched. If the key is a chord prefix, it returns `CONSUMED` and
/// waits for the next key. Unbound keys fall through to text input handling.
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

    // When the search bar is active, intercept keys for search navigation.
    if ctx.state.search_state().is_active() {
        return handle_search_key(ctx, code, modifiers);
    }

    let keypress = KeyPress::from_crossterm(code, modifiers);
    let resolution = ctx.state.keybindings_mut().resolve(keypress);

    match resolution {
        KeyResolution::Action(action) => dispatch_action(ctx, action).await,
        KeyResolution::Partial => {
            debug!("chord prefix matched, waiting for next key");
            Ok(Handled::CONSUMED)
        }
        KeyResolution::Unbound => handle_unbound_key(ctx, code, modifiers),
    }
}

/// Handles key events while the transcript search bar is active.
///
/// - `Escape` closes the search bar and clears the query.
/// - `Enter` navigates to the next match.
/// - `Shift+Enter` navigates to the previous match.
/// - `Backspace` deletes the last character of the query.
/// - Printable characters are appended to the search query.
fn handle_search_key(
    ctx: &mut AppContext<'_>,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Result<Handled> {
    match (code, modifiers) {
        (KeyCode::Esc, _) => {
            debug!("search: closing search bar");
            ctx.state.search_state_mut().clear();
            ctx.state.search_state_mut().set_active(false);
            ctx.state.mark_full_redraw();
        }
        (KeyCode::Enter, KeyModifiers::NONE) => {
            debug!("search: next match");
            ctx.state.search_state_mut().next_match();
            ctx.state.mark_full_redraw();
        }
        (KeyCode::Enter, m) if m.contains(KeyModifiers::SHIFT) => {
            debug!("search: previous match");
            ctx.state.search_state_mut().prev_match();
            ctx.state.mark_full_redraw();
        }
        (KeyCode::Backspace, _) => {
            let query = ctx.state.search_state().query().to_string();
            if !query.is_empty() {
                let new_query: String = query.chars().take(query.chars().count() - 1).collect();
                let entries = collect_search_entries(ctx);
                ctx.state.search_state_mut().set_query(&new_query, &entries);
                ctx.state.mark_full_redraw();
            }
        }
        (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            let mut query = ctx.state.search_state().query().to_string();
            query.push(c);
            let entries = collect_search_entries(ctx);
            ctx.state.search_state_mut().set_query(&query, &entries);
            ctx.state.mark_full_redraw();
        }
        _ => {
            debug!(?code, ?modifiers, "unhandled search key");
        }
    }
    Ok(Handled::CONSUMED)
}

/// Collects searchable text from the conversation timeline.
///
/// Returns a `Vec<String>` where each element is the text content of a
/// conversation entry. Tool execution entries are represented as
/// `"[tool_name] input"` for searchability.
fn collect_search_entries(ctx: &AppContext<'_>) -> Vec<String> {
    use crate::types::ConversationEntry;
    ctx.state
        .conversation
        .timeline()
        .entries()
        .iter()
        .map(|entry| match entry {
            ConversationEntry::UserMessage(text) | ConversationEntry::AssistantMessage(text) => {
                text.clone()
            }
            ConversationEntry::Streaming { text, .. } => text.clone(),
            ConversationEntry::ToolExecution {
                name,
                input,
                output,
                ..
            } => {
                let mut s = format!("[{name}] {input}");
                if let Some(out) = output {
                    s.push('\n');
                    s.push_str(out);
                }
                s
            }
            ConversationEntry::ImageDisplay { alt_text, .. } => {
                alt_text.clone().unwrap_or_default()
            }
        })
        .collect()
}

/// Opens the user's external editor with the current input buffer contents.
///
/// Suspends the TUI (disables raw mode, leaves alternate screen) so the
/// editor owns the terminal. On success the edited content replaces the
/// input buffer; on failure a warning is logged.
fn open_external_editor(ctx: &mut AppContext<'_>) {
    use crate::tools::external_editor::ExternalEditor;
    use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
    use crossterm::execute;
    use crossterm::terminal::{
        disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
    };

    let current_input = ctx.state.input_state().text().to_string();

    // Suspend TUI so the editor gets a clean terminal.
    let mut stdout = std::io::stdout();
    let _ = disable_raw_mode();
    let _ = execute!(stdout, LeaveAlternateScreen, DisableMouseCapture);

    let editor = ExternalEditor::new();
    let result = editor.open_with_content(&current_input);

    // Restore TUI.
    let _ = enable_raw_mode();
    let _ = execute!(stdout, EnterAlternateScreen, EnableMouseCapture);

    match result {
        Ok(new_content) => {
            let trimmed = new_content.trim_end().to_string();
            ctx.state.input_state_mut().set_text(trimmed);
            info!("External editor returned successfully");
        }
        Err(e) => {
            info!("External editor error: {e}");
        }
    }
    ctx.state.mark_full_redraw();
}

/// Dispatches a resolved keybinding [`Action`] to the corresponding handler.
///
/// # Errors
///
/// Returns an error if submit or other async operations fail.
async fn dispatch_action(ctx: &mut AppContext<'_>, action: Action) -> Result<Handled> {
    debug!(?action, "dispatching keybinding action");
    match action {
        Action::Quit => {
            ctx.state.request_quit();
            Ok(Handled::CONSUMED)
        }
        Action::Submit => {
            if !ctx.state.input_state().text().is_empty() {
                submit::handle_submit(ctx).await?;
            }
            Ok(Handled::CONSUMED)
        }
        Action::NewLine => {
            ctx.state.insert_char('\n');
            Ok(Handled::CONSUMED)
        }
        Action::ScrollUp => {
            debug!("scroll_up triggered");
            ctx.state.scroll_up(10);
            Ok(Handled::CONSUMED)
        }
        Action::ScrollDown => {
            debug!("scroll_down triggered");
            ctx.state.scroll_down(10);
            Ok(Handled::CONSUMED)
        }
        Action::PageUp => {
            debug!("page_up triggered");
            let page = page_scroll_amount(ctx);
            ctx.state.scroll_up(page);
            Ok(Handled::CONSUMED)
        }
        Action::PageDown => {
            debug!("page_down triggered");
            let page = page_scroll_amount(ctx);
            ctx.state.scroll_down(page);
            Ok(Handled::CONSUMED)
        }
        Action::ScrollToTop => {
            debug!("scroll_to_top triggered");
            ctx.state.scroll_to_top();
            Ok(Handled::CONSUMED)
        }
        Action::ScrollToBottom => {
            debug!("scroll_to_bottom triggered");
            let height = ctx.state.display().scroll_state().content_height();
            ctx.state.scroll_to_bottom(height);
            Ok(Handled::CONSUMED)
        }
        Action::SelectAll => {
            clipboard::handle_select_all(ctx);
            Ok(Handled::CONSUMED)
        }
        Action::Copy => {
            debug!("copy triggered");
            clipboard::handle_copy(ctx.state);
            Ok(Handled::CONSUMED)
        }
        Action::Paste => {
            debug!("paste triggered");
            clipboard::handle_paste(ctx.state);
            Ok(Handled::CONSUMED)
        }
        Action::CancelOperation => {
            if ctx.state.search_state().is_active() {
                ctx.state.search_state_mut().clear();
                ctx.state.search_state_mut().set_active(false);
                ctx.state.mark_full_redraw();
            } else if ctx.state.ui_selection().selection().has_selection() {
                ctx.state.ui_selection_mut().selection_mut().clear();
                ctx.state.mark_full_redraw();
            }
            Ok(Handled::CONSUMED)
        }
        Action::FocusContent => {
            ctx.state
                .ui_selection_mut()
                .set_focus_area(FocusArea::Content);
            ctx.state.mark_full_redraw();
            Ok(Handled::CONSUMED)
        }
        Action::FocusInput => {
            ctx.state
                .ui_selection_mut()
                .set_focus_area(FocusArea::Input);
            ctx.state.mark_full_redraw();
            Ok(Handled::CONSUMED)
        }
        Action::ToggleHelp => {
            info!("Help toggle not yet available");
            Ok(Handled::CONSUMED)
        }
        Action::KillBackgroundAgents => {
            info!("Kill background agents not yet available");
            Ok(Handled::CONSUMED)
        }
        Action::OpenEditor => {
            open_external_editor(ctx);
            Ok(Handled::CONSUMED)
        }
        Action::Search => {
            debug!("search: activating search bar");
            ctx.state.search_state_mut().set_active(true);
            ctx.state.search_state_mut().clear();
            ctx.state.mark_full_redraw();
            Ok(Handled::CONSUMED)
        }
        Action::SearchNext => {
            if ctx.state.search_state().is_active() {
                ctx.state.search_state_mut().next_match();
                ctx.state.mark_full_redraw();
            }
            Ok(Handled::CONSUMED)
        }
        Action::SearchPrev => {
            if ctx.state.search_state().is_active() {
                ctx.state.search_state_mut().prev_match();
                ctx.state.mark_full_redraw();
            }
            Ok(Handled::CONSUMED)
        }
        Action::Custom(ref name) => {
            debug!(%name, "custom action triggered");
            Ok(Handled::CONSUMED)
        }
    }
}

/// Returns the scroll amount for page-up/page-down based on viewport height.
fn page_scroll_amount(ctx: &AppContext<'_>) -> usize {
    let height = ctx.state.display().terminal_height();
    // Subtract header/footer chrome, minimum 10 lines
    (height.saturating_sub(4) as usize).max(10)
}

/// Handles keys that did not match any keybinding (text input, backspace).
fn handle_unbound_key(
    ctx: &mut AppContext<'_>,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Result<Handled> {
    match (code, modifiers) {
        (KeyCode::Backspace, _) => {
            ctx.state.delete_char();
            Ok(Handled::CONSUMED)
        }
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

    // =========================================================================
    // KeybindingManager wiring: custom bindings take effect through handler
    // =========================================================================

    #[tokio::test]
    async fn custom_keybinding_override_takes_effect() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // Override Ctrl+D from Quit to ScrollDown via the keybinding manager
        use crate::keybindings::{KeyChord, KeyPress};
        let chord = KeyChord(vec![KeyPress::from_crossterm(
            KeyCode::Char('d'),
            KeyModifiers::CONTROL,
        )]);
        state
            .keybindings_mut()
            .bindings_mut()
            .insert(chord, crate::keybindings::Action::ScrollDown);

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        let _result = handler.handle(&event, &mut ctx).await.unwrap();

        // Ctrl+D should NOT have triggered quit (it's now ScrollDown)
        assert!(
            !ctx.state.wants_quit(),
            "Custom keybinding override must take effect — Ctrl+D should no longer quit"
        );
    }

    // =========================================================================
    // Shift+Enter inserts a newline
    // =========================================================================

    #[tokio::test]
    async fn shift_enter_inserts_newline() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(result, Handled::CONSUMED);
        assert_eq!(
            ctx.state.input_state().text(),
            "\n",
            "Shift+Enter must insert a newline into the input buffer"
        );
    }

    // =========================================================================
    // PageUp/PageDown dispatch to distinct page-size scroll
    // =========================================================================

    #[tokio::test]
    async fn page_up_dispatches_via_keybinding_manager() {
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
            "PageUp must be dispatched through keybinding manager"
        );
    }

    // =========================================================================
    // Chord sequence resolves through keybinding manager
    // =========================================================================

    #[tokio::test]
    async fn chord_sequence_resolves_through_handler() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // Add a chord: Ctrl+X Ctrl+Q -> Quit
        use crate::keybindings::{KeyChord, KeyPress};
        let chord = KeyChord(vec![
            KeyPress::from_crossterm(KeyCode::Char('x'), KeyModifiers::CONTROL),
            KeyPress::from_crossterm(KeyCode::Char('q'), KeyModifiers::CONTROL),
        ]);
        state
            .keybindings_mut()
            .bindings_mut()
            .insert(chord, crate::keybindings::Action::Quit);

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        // First key: Ctrl+X — should return Partial (consumed, no action yet)
        let event1 = AppEvent::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
        let result1 = handler.handle(&event1, &mut ctx).await.unwrap();
        assert_eq!(result1, Handled::CONSUMED);
        assert!(
            !ctx.state.wants_quit(),
            "First chord key must not trigger action"
        );

        // Second key: Ctrl+Q — should complete the chord and trigger Quit
        let event2 = AppEvent::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL));
        let _result2 = handler.handle(&event2, &mut ctx).await.unwrap();
        assert!(
            ctx.state.wants_quit(),
            "Second chord key must complete the chord and trigger the action"
        );
    }

    // =========================================================================
    // FocusContent / FocusInput actions work
    // =========================================================================

    #[tokio::test]
    async fn focus_content_action_changes_focus_area() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // Bind F5 to FocusContent
        use crate::keybindings::{KeyChord, KeyPress};
        let chord = KeyChord(vec![KeyPress::from_crossterm(
            KeyCode::F(5),
            KeyModifiers::NONE,
        )]);
        state
            .keybindings_mut()
            .bindings_mut()
            .insert(chord, crate::keybindings::Action::FocusContent);

        assert_eq!(
            state.ui_selection().focus_area(),
            FocusArea::Input,
            "Default focus should be Input"
        );

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(result, Handled::CONSUMED);
        assert_eq!(
            ctx.state.ui_selection().focus_area(),
            FocusArea::Content,
            "FocusContent action must change focus area to Content"
        );
    }

    // =========================================================================
    // V-7: Stub actions (ToggleHelp, KillBackgroundAgents, OpenEditor)
    //      consume events without panicking
    // =========================================================================

    #[tokio::test]
    async fn toggle_help_action_returns_consumed() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // Bind F1 to ToggleHelp
        use crate::keybindings::{KeyChord, KeyPress};
        let chord = KeyChord(vec![KeyPress::from_crossterm(
            KeyCode::F(1),
            KeyModifiers::NONE,
        )]);
        state
            .keybindings_mut()
            .bindings_mut()
            .insert(chord, crate::keybindings::Action::ToggleHelp);

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "ToggleHelp must be consumed without panicking"
        );
    }

    #[tokio::test]
    async fn kill_background_agents_action_returns_consumed() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // Bind F2 to KillBackgroundAgents
        use crate::keybindings::{KeyChord, KeyPress};
        let chord = KeyChord(vec![KeyPress::from_crossterm(
            KeyCode::F(2),
            KeyModifiers::NONE,
        )]);
        state
            .keybindings_mut()
            .bindings_mut()
            .insert(chord, crate::keybindings::Action::KillBackgroundAgents);

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "KillBackgroundAgents must be consumed without panicking"
        );
    }

    #[tokio::test]
    async fn open_editor_action_returns_consumed() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // Bind F3 to OpenEditor
        use crate::keybindings::{KeyChord, KeyPress};
        let chord = KeyChord(vec![KeyPress::from_crossterm(
            KeyCode::F(3),
            KeyModifiers::NONE,
        )]);
        state
            .keybindings_mut()
            .bindings_mut()
            .insert(chord, crate::keybindings::Action::OpenEditor);

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "OpenEditor must be consumed without panicking"
        );
    }

    // =========================================================================
    // Transcript search (Ctrl+F) wiring tests
    // =========================================================================

    #[tokio::test]
    async fn ctrl_f_activates_search_mode() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        assert!(!ctx.state.search_state().is_active());

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(result, Handled::CONSUMED);
        assert!(
            ctx.state.search_state().is_active(),
            "Ctrl+F must activate search mode"
        );
    }

    #[tokio::test]
    async fn search_mode_typing_updates_query() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        // Activate search
        ctx.state.search_state_mut().set_active(true);

        // Type characters
        let event_h = AppEvent::Key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
        let _ = handler.handle(&event_h, &mut ctx).await.unwrap();
        let event_i = AppEvent::Key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
        let _ = handler.handle(&event_i, &mut ctx).await.unwrap();

        assert_eq!(ctx.state.search_state().query(), "hi");
    }

    #[tokio::test]
    async fn search_mode_backspace_removes_last_char() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        // Activate search and set a query
        ctx.state.search_state_mut().set_active(true);
        let entries: Vec<String> = Vec::new();
        ctx.state.search_state_mut().set_query("hello", &entries);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(ctx.state.search_state().query(), "hell");
    }

    #[tokio::test]
    async fn search_mode_enter_navigates_next_match() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // Add conversation entries to search through
        state
            .conversation
            .timeline_mut()
            .push_user_message("hello world");
        state
            .conversation
            .timeline_mut()
            .push_user_message("hello rust");

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        // Activate search and type query
        ctx.state.search_state_mut().set_active(true);
        let entries = collect_search_entries(&ctx);
        ctx.state.search_state_mut().set_query("hello", &entries);

        assert_eq!(ctx.state.search_state().match_count(), 2);
        assert_eq!(ctx.state.search_state().current_position(), Some((1, 2)));

        // Press Enter to go to next match
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(ctx.state.search_state().current_position(), Some((2, 2)));
    }

    #[tokio::test]
    async fn search_mode_shift_enter_navigates_prev_match() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state
            .conversation
            .timeline_mut()
            .push_user_message("hello world");
        state
            .conversation
            .timeline_mut()
            .push_user_message("hello rust");

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        ctx.state.search_state_mut().set_active(true);
        let entries = collect_search_entries(&ctx);
        ctx.state.search_state_mut().set_query("hello", &entries);

        // Press Shift+Enter to go to previous match (wraps to last)
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(ctx.state.search_state().current_position(), Some((2, 2)));
    }

    #[tokio::test]
    async fn search_mode_escape_deactivates() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        // Activate search with a query
        ctx.state.search_state_mut().set_active(true);
        let entries: Vec<String> = Vec::new();
        ctx.state.search_state_mut().set_query("test", &entries);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(
            !ctx.state.search_state().is_active(),
            "Escape must deactivate search mode"
        );
        assert!(
            ctx.state.search_state().query().is_empty(),
            "Escape must clear the query"
        );
    }

    #[tokio::test]
    async fn search_mode_does_not_insert_into_input() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        ctx.state.search_state_mut().set_active(true);

        // Type a character while search is active
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(
            ctx.state.input_state().text().is_empty(),
            "Characters typed during search must NOT go to the input buffer"
        );
        assert_eq!(
            ctx.state.search_state().query(),
            "x",
            "Characters typed during search must go to the search query"
        );
    }

    // =========================================================================
    // External editor (Ctrl+X Ctrl+E) chord binding test
    // =========================================================================

    #[tokio::test]
    async fn ctrl_x_ctrl_e_chord_resolves_to_open_editor() {
        let mut handler = KeyboardHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // Send Ctrl+X (should be partial chord)
        {
            let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
            let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
            let result = handler.handle(&event, &mut ctx).await.unwrap();
            assert_eq!(
                result,
                Handled::CONSUMED,
                "Ctrl+X should be consumed as chord prefix"
            );
        }

        // Send Ctrl+E (should complete the chord -> OpenEditor)
        {
            let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
            let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));
            let result = handler.handle(&event, &mut ctx).await.unwrap();
            assert_eq!(
                result,
                Handled::CONSUMED,
                "Ctrl+E after Ctrl+X should be consumed as OpenEditor action"
            );
        }
    }

    #[tokio::test]
    async fn collect_search_entries_includes_all_types() {
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state
            .conversation
            .timeline_mut()
            .push_user_message("user msg");
        state
            .conversation
            .timeline_mut()
            .push_assistant_message("assistant msg");

        let ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let entries = collect_search_entries(&ctx);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], "user msg");
        assert_eq!(entries[1], "assistant msg");
    }
}
