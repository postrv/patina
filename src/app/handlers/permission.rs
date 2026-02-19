//! Permission handler for tool execution approval prompts.
//!
//! [`PermissionHandler`] intercepts [`AppEvent::Key`] events when a permission
//! prompt is active and converts them into permission decisions. It also handles
//! [`AppEvent::PermissionResponse`] events directly for programmatic responses.
//!
//! This handler **must** run before `KeyboardHandler` in the dispatch chain so
//! that key events are consumed by the permission prompt modal rather than
//! reaching normal input handling.

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use crossterm::event::KeyCode;
use tracing::debug;

use crate::app::context::AppContext;
use crate::app::dispatch::{EventHandler, Handled};
use crate::app::events::AppEvent;
use crate::permissions::PermissionResponse;
use crate::tui::widgets::handle_permission_key;
use crate::tui::widgets::permission_prompt::PermissionPromptState;

/// Handles permission prompts for tool execution approval.
///
/// When a permission prompt is pending, `PermissionHandler` intercepts all
/// key events to drive the permission UI:
///
/// - **y/Y**: Allow once (session grant)
/// - **a/A**: Allow always (persistent rule)
/// - **n/N**: Deny this execution
/// - **Esc**: Deny (alias for n)
/// - **Enter**: Confirm the currently selected option
/// - **Left/Right**: Navigate between options (does not resolve the prompt)
///
/// All key events are consumed while the permission prompt is active,
/// preventing them from reaching the `KeyboardHandler`.
///
/// # Examples
///
/// ```rust,ignore
/// use patina::app::handlers::permission::PermissionHandler;
/// use patina::app::dispatch::EventDispatcher;
///
/// let dispatcher = EventDispatcher::new(vec![
///     Box::new(PermissionHandler),  // Must come before KeyboardHandler
///     Box::new(keyboard_handler),
///     Box::new(stream_handler),
/// ]);
/// ```
pub struct PermissionHandler;

impl EventHandler for PermissionHandler {
    fn handle<'a>(
        &'a mut self,
        event: &'a AppEvent,
        ctx: &'a mut AppContext<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<Handled>> + Send + 'a>> {
        Box::pin(async move {
            match event {
                AppEvent::Key(key) => {
                    if !ctx.state.has_pending_permission() {
                        return Ok(Handled::IGNORED);
                    }

                    // Convert the key event to a permission response.
                    let response = convert_key_to_response(ctx, *key);

                    if let Some(response) = response {
                        apply_permission_response(ctx, response).await?;
                    }

                    // All key events are consumed while the permission
                    // prompt is active — even navigation or unrecognized keys.
                    Ok(Handled::CONSUMED)
                }
                AppEvent::PermissionResponse(response) => {
                    let response = *response;
                    apply_permission_response(ctx, response).await?;
                    Ok(Handled::CONSUMED)
                }
                _ => Ok(Handled::IGNORED),
            }
        })
    }

    fn name(&self) -> &str {
        "permission"
    }
}

/// Converts a crossterm key event into a permission response, if the key
/// maps to a decision (y/n/a/Enter/Esc). Navigation keys return `None`.
fn convert_key_to_response(
    ctx: &mut AppContext<'_>,
    key: crossterm::event::KeyEvent,
) -> Option<PermissionResponse> {
    let request = ctx.state.pending_permission()?.clone();
    let mut prompt_state = PermissionPromptState::new(request);

    let key_char = match key.code {
        KeyCode::Char(c) => c,
        KeyCode::Enter => '\r',
        KeyCode::Esc => '\x1b',
        KeyCode::Tab => '\t',
        KeyCode::Backspace => '\x08',
        KeyCode::Left => 'h',
        KeyCode::Right => 'l',
        _ => return None,
    };

    let response = handle_permission_key(&mut prompt_state, key_char);

    if response.is_some() {
        ctx.state.clear_pending_permission();
    }

    response
}

/// Applies a permission response: updates state and triggers tool execution
/// or denial accordingly.
///
/// # Errors
///
/// Returns an error if tool execution setup or denial fails.
async fn apply_permission_response(
    ctx: &mut AppContext<'_>,
    response: PermissionResponse,
) -> Result<()> {
    debug!(?response, "Applying permission response");

    ctx.state.handle_permission_response(response).await;

    if matches!(
        response,
        PermissionResponse::AllowOnce | PermissionResponse::AllowAlways
    ) {
        crate::app::start_tool_execution(ctx.state)?;
    } else {
        // deny_all_tools() fails if the tool loop is already Idle, which
        // can happen when a PermissionResponse event arrives without a
        // matching tool execution. Log and continue rather than crashing.
        if let Err(e) = ctx.state.deny_all_tools() {
            debug!(?e, "deny_all_tools failed (tool loop may already be idle)");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::AnthropicClient;
    use crate::app::state::AppState;
    use crate::permissions::{PermissionRequest, PermissionResponse};
    use crate::session::SessionManager;
    use crate::types::config::ParallelMode;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::CrosstermBackend;
    use ratatui::Terminal;
    use secrecy::SecretString;
    use std::io;
    use std::path::PathBuf;
    use tempfile::TempDir;

    // =========================================================================
    // Test helpers
    // =========================================================================

    fn test_terminal() -> Terminal<CrosstermBackend<io::Stdout>> {
        let backend = CrosstermBackend::new(io::stdout());
        Terminal::new(backend).expect("failed to create test terminal")
    }

    fn test_client() -> AnthropicClient {
        AnthropicClient::new(SecretString::from("test-key"), "claude-test")
    }

    fn test_state() -> AppState {
        AppState::new(PathBuf::from("/tmp/test"), true, ParallelMode::Disabled)
    }

    fn test_session_manager() -> (SessionManager, TempDir) {
        let dir = TempDir::new().expect("failed to create temp dir");
        let mgr = SessionManager::new(dir.path().to_path_buf());
        (mgr, dir)
    }

    fn test_permission_request() -> PermissionRequest {
        PermissionRequest {
            tool_name: "bash".to_string(),
            tool_input: Some("git status".to_string()),
            description: "Run git status".to_string(),
        }
    }

    // =========================================================================
    // PermissionHandler::name
    // =========================================================================

    #[test]
    fn name_returns_permission() {
        let handler = PermissionHandler;
        assert_eq!(handler.name(), "permission");
    }

    // =========================================================================
    // When no permission pending — all events should be IGNORED
    // =========================================================================

    #[tokio::test]
    async fn handle_key_when_no_permission_pending_returns_ignored() {
        let mut handler = PermissionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // No permission set — should pass through.
        assert!(!state.has_pending_permission());

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::IGNORED,
            "Key events must pass through when no permission is pending"
        );
    }

    #[tokio::test]
    async fn handle_tick_returns_ignored() {
        let mut handler = PermissionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);

        let event = AppEvent::Tick;
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::IGNORED,
            "PermissionHandler must ignore Tick events"
        );
    }

    #[tokio::test]
    async fn handle_quit_returns_ignored() {
        let mut handler = PermissionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);

        let event = AppEvent::Quit;
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::IGNORED,
            "PermissionHandler must ignore Quit events"
        );
    }

    #[tokio::test]
    async fn handle_resize_returns_ignored() {
        let mut handler = PermissionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);

        let event = AppEvent::Resize {
            width: 80,
            height: 24,
        };
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::IGNORED,
            "PermissionHandler must ignore Resize events"
        );
    }

    #[tokio::test]
    async fn handle_api_chunk_returns_ignored() {
        let mut handler = PermissionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);

        let event = AppEvent::ApiChunk(crate::api::StreamEvent::ContentDelta("hi".to_string()));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::IGNORED,
            "PermissionHandler must ignore ApiChunk events"
        );
    }

    #[tokio::test]
    async fn handle_tool_result_returns_ignored() {
        let mut handler = PermissionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);

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
            "PermissionHandler must ignore ToolResult events"
        );
    }

    // =========================================================================
    // When permission IS pending — Key events must be CONSUMED
    // =========================================================================

    #[tokio::test]
    async fn handle_key_when_permission_pending_returns_consumed() {
        let mut handler = PermissionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_permission(test_permission_request());
        assert!(state.has_pending_permission());

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "All Key events must be consumed while permission prompt is active"
        );
    }

    #[tokio::test]
    async fn handle_non_key_when_permission_pending_returns_ignored() {
        let mut handler = PermissionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_permission(test_permission_request());

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = AppEvent::Tick;
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::IGNORED,
            "Non-key events must pass through even with permission pending"
        );
    }

    // =========================================================================
    // Permission decision via Key events — state mutations
    // =========================================================================

    #[tokio::test]
    async fn handle_key_y_clears_pending_permission() {
        let mut handler = PermissionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_permission(test_permission_request());

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        let _result = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(
            !ctx.state.has_pending_permission(),
            "Pressing 'y' must clear the pending permission"
        );
    }

    #[tokio::test]
    async fn handle_key_a_clears_pending_permission() {
        let mut handler = PermissionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_permission(test_permission_request());

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        let _result = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(
            !ctx.state.has_pending_permission(),
            "Pressing 'a' must clear the pending permission"
        );
    }

    #[tokio::test]
    async fn handle_key_n_clears_pending_permission() {
        let mut handler = PermissionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_permission(test_permission_request());

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        let _result = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(
            !ctx.state.has_pending_permission(),
            "Pressing 'n' must clear the pending permission"
        );
    }

    #[tokio::test]
    async fn handle_key_esc_clears_pending_permission() {
        let mut handler = PermissionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_permission(test_permission_request());

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let _result = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(
            !ctx.state.has_pending_permission(),
            "Pressing Esc must clear the pending permission (deny)"
        );
    }

    #[tokio::test]
    async fn handle_key_enter_clears_pending_permission() {
        let mut handler = PermissionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_permission(test_permission_request());

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let _result = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(
            !ctx.state.has_pending_permission(),
            "Pressing Enter must confirm selection and clear the pending permission"
        );
    }

    #[tokio::test]
    async fn handle_navigation_key_preserves_pending_permission() {
        let mut handler = PermissionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_permission(test_permission_request());

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);

        // Left arrow navigates but doesn't confirm — permission stays pending.
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "Navigation keys must still be consumed during permission prompt"
        );
        assert!(
            ctx.state.has_pending_permission(),
            "Navigation keys must NOT clear the pending permission"
        );
    }

    #[tokio::test]
    async fn handle_unrecognized_key_preserves_pending_permission() {
        let mut handler = PermissionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_permission(test_permission_request());

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);

        // An unrecognized key is consumed but doesn't resolve the prompt.
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "Unrecognized keys must be consumed during permission prompt"
        );
        assert!(
            ctx.state.has_pending_permission(),
            "Unrecognized keys must NOT clear the pending permission"
        );
    }

    // =========================================================================
    // Direct PermissionResponse event handling
    // =========================================================================

    #[tokio::test]
    async fn handle_permission_response_allow_once_returns_consumed() {
        let mut handler = PermissionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_permission(test_permission_request());

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = AppEvent::PermissionResponse(PermissionResponse::AllowOnce);
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "PermissionResponse events must always be consumed"
        );
    }

    #[tokio::test]
    async fn handle_permission_response_deny_returns_consumed() {
        let mut handler = PermissionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_permission(test_permission_request());

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = AppEvent::PermissionResponse(PermissionResponse::Deny);
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "PermissionResponse(Deny) events must be consumed"
        );
    }

    #[tokio::test]
    async fn handle_permission_response_clears_pending() {
        let mut handler = PermissionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_permission(test_permission_request());

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = AppEvent::PermissionResponse(PermissionResponse::AllowAlways);
        let _result = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(
            !ctx.state.has_pending_permission(),
            "PermissionResponse event must clear the pending permission"
        );
    }

    #[tokio::test]
    async fn handle_permission_response_without_pending_still_consumed() {
        let mut handler = PermissionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // No pending permission — response event should still be consumed
        // (it's a dedicated event type with no other handler).
        assert!(!state.has_pending_permission());

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = AppEvent::PermissionResponse(PermissionResponse::Deny);
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "PermissionResponse must be consumed even without a pending permission"
        );
    }

    // =========================================================================
    // Send bound verification
    // =========================================================================

    #[test]
    fn permission_handler_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<PermissionHandler>();
    }

    // =========================================================================
    // Integration with EventDispatcher
    // =========================================================================

    #[tokio::test]
    async fn permission_handler_works_in_dispatcher() {
        use crate::app::dispatch::EventDispatcher;

        let handler = PermissionHandler;
        let mut dispatcher = EventDispatcher::new(vec![Box::new(handler)]);
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_permission(test_permission_request());

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);

        // Key event should be consumed when permission pending.
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        let result = dispatcher.dispatch(&event, &mut ctx).await.unwrap();
        assert_eq!(
            result,
            Handled::CONSUMED,
            "PermissionHandler should consume Key in dispatcher when permission pending"
        );

        // Tick should pass through.
        let tick = AppEvent::Tick;
        let result = dispatcher.dispatch(&tick, &mut ctx).await.unwrap();
        assert_eq!(
            result,
            Handled::IGNORED,
            "PermissionHandler should ignore Tick in dispatcher"
        );
    }
}
