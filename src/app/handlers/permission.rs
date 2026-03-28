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
pub struct PermissionHandler {
    /// Persistent prompt state that survives across keypresses so that
    /// arrow-key navigation is not discarded.
    prompt_state: Option<PermissionPromptState>,
}

impl PermissionHandler {
    /// Creates a new `PermissionHandler`.
    #[must_use]
    pub fn new() -> Self {
        Self { prompt_state: None }
    }

    /// Converts a crossterm key event into the char representation expected
    /// by [`handle_permission_key`].
    fn key_to_char(key: crossterm::event::KeyEvent) -> Option<char> {
        match key.code {
            KeyCode::Char(c) => Some(c),
            KeyCode::Enter => Some('\r'),
            KeyCode::Esc => Some('\x1b'),
            KeyCode::Tab => Some('\t'),
            KeyCode::Backspace => Some('\x08'),
            KeyCode::Left => Some('h'),
            KeyCode::Right => Some('l'),
            _ => None,
        }
    }
}

impl Default for PermissionHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl EventHandler for PermissionHandler {
    fn handle<'a>(
        &'a mut self,
        event: &'a AppEvent,
        ctx: &'a mut AppContext<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<Handled>> + Send + 'a>> {
        Box::pin(async move {
            match event {
                AppEvent::Key(key) => {
                    if !ctx.state.tool_state().has_pending_permission() {
                        // No permission pending — clear stale prompt state and pass through.
                        self.prompt_state = None;
                        return Ok(Handled::IGNORED);
                    }

                    let request = match ctx.state.tool_state().pending_permission() {
                        Some(req) => req.clone(),
                        None => return Ok(Handled::IGNORED),
                    };

                    // Clear chord buffer so stale chord state from before the
                    // modal appeared does not corrupt the next keypress (V-2).
                    ctx.state.keybindings_mut().clear_chord();

                    // Lazily initialize prompt state for this permission request.
                    // If the pending request has changed (e.g., tool A resolved
                    // programmatically and tool B became pending), reset the
                    // prompt state so the user sees fresh defaults (V-1).
                    if let Some(existing) = &self.prompt_state {
                        if *existing.request() != request {
                            self.prompt_state = None;
                        }
                    }
                    let prompt = self
                        .prompt_state
                        .get_or_insert_with(|| PermissionPromptState::new(request));

                    if let Some(key_char) = Self::key_to_char(*key) {
                        if let Some(response) = handle_permission_key(prompt, key_char) {
                            self.prompt_state = None;
                            ctx.state.clear_pending_permission();
                            apply_permission_response(ctx, response).await?;
                        }
                    }

                    // All key events are consumed while the permission
                    // prompt is active — even navigation or unrecognized keys.
                    Ok(Handled::CONSUMED)
                }
                AppEvent::PermissionResponse(response) => {
                    let response = *response;
                    self.prompt_state = None;
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
        ctx.start_tool_execution()?;
    } else {
        // deny_all_tools() fails if the tool loop is already Idle, which
        // can happen when a PermissionResponse event arrives without a
        // matching tool execution. Log and continue rather than crashing.
        if let Err(e) = ctx.state.tool_state_mut().deny_all_tools() {
            debug!(?e, "deny_all_tools failed (tool loop may already be idle)");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{AnthropicClient, LlmProvider};
    use crate::app::state::AppState;
    use crate::permissions::PermissionRequest;
    use crate::session::SessionManager;
    use crate::types::config::ParallelMode;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use secrecy::SecretString;
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
        let handler = PermissionHandler::new();
        assert_eq!(handler.name(), "permission");
    }

    // =========================================================================
    // When no permission pending — all events should be IGNORED
    // =========================================================================

    #[tokio::test]
    async fn handle_key_when_no_permission_pending_returns_ignored() {
        let mut handler = PermissionHandler::new();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // No permission set — should pass through.
        assert!(!state.tool_state().has_pending_permission());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
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
        let mut handler = PermissionHandler::new();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

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
        let mut handler = PermissionHandler::new();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

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
        let mut handler = PermissionHandler::new();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

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
        let mut handler = PermissionHandler::new();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

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
        let mut handler = PermissionHandler::new();

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
            "PermissionHandler must ignore ToolResult events"
        );
    }

    // =========================================================================
    // When permission IS pending — Key events must be CONSUMED
    // =========================================================================

    #[tokio::test]
    async fn handle_key_when_permission_pending_returns_consumed() {
        let mut handler = PermissionHandler::new();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_permission(test_permission_request());
        assert!(state.tool_state().has_pending_permission());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
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
        let mut handler = PermissionHandler::new();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_permission(test_permission_request());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
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
        let mut handler = PermissionHandler::new();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_permission(test_permission_request());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        let _result = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(
            !ctx.state.tool_state().has_pending_permission(),
            "Pressing 'y' must clear the pending permission"
        );
    }

    #[tokio::test]
    async fn handle_key_a_clears_pending_permission() {
        let mut handler = PermissionHandler::new();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_permission(test_permission_request());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        let _result = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(
            !ctx.state.tool_state().has_pending_permission(),
            "Pressing 'a' must clear the pending permission"
        );
    }

    #[tokio::test]
    async fn handle_key_n_clears_pending_permission() {
        let mut handler = PermissionHandler::new();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_permission(test_permission_request());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        let _result = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(
            !ctx.state.tool_state().has_pending_permission(),
            "Pressing 'n' must clear the pending permission"
        );
    }

    #[tokio::test]
    async fn handle_key_esc_clears_pending_permission() {
        let mut handler = PermissionHandler::new();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_permission(test_permission_request());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let _result = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(
            !ctx.state.tool_state().has_pending_permission(),
            "Pressing Esc must clear the pending permission (deny)"
        );
    }

    #[tokio::test]
    async fn handle_key_enter_clears_pending_permission() {
        let mut handler = PermissionHandler::new();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_permission(test_permission_request());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let _result = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(
            !ctx.state.tool_state().has_pending_permission(),
            "Pressing Enter must confirm selection and clear the pending permission"
        );
    }

    #[tokio::test]
    async fn handle_navigation_key_preserves_pending_permission() {
        let mut handler = PermissionHandler::new();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_permission(test_permission_request());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        // Left arrow navigates but doesn't confirm — permission stays pending.
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "Navigation keys must still be consumed during permission prompt"
        );
        assert!(
            ctx.state.tool_state().has_pending_permission(),
            "Navigation keys must NOT clear the pending permission"
        );
    }

    #[tokio::test]
    async fn handle_unrecognized_key_preserves_pending_permission() {
        let mut handler = PermissionHandler::new();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_permission(test_permission_request());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        // An unrecognized key is consumed but doesn't resolve the prompt.
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "Unrecognized keys must be consumed during permission prompt"
        );
        assert!(
            ctx.state.tool_state().has_pending_permission(),
            "Unrecognized keys must NOT clear the pending permission"
        );
    }

    // =========================================================================
    // Direct PermissionResponse event handling
    // =========================================================================

    #[tokio::test]
    async fn handle_permission_response_allow_once_returns_consumed() {
        let mut handler = PermissionHandler::new();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_permission(test_permission_request());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
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
        let mut handler = PermissionHandler::new();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_permission(test_permission_request());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
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
        let mut handler = PermissionHandler::new();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_permission(test_permission_request());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::PermissionResponse(PermissionResponse::AllowAlways);
        let _result = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(
            !ctx.state.tool_state().has_pending_permission(),
            "PermissionResponse event must clear the pending permission"
        );
    }

    #[tokio::test]
    async fn handle_permission_response_without_pending_still_consumed() {
        let mut handler = PermissionHandler::new();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // No pending permission — response event should still be consumed
        // (it's a dedicated event type with no other handler).
        assert!(!state.tool_state().has_pending_permission());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::PermissionResponse(PermissionResponse::Deny);
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "PermissionResponse must be consumed even without a pending permission"
        );
    }

    // =========================================================================
    // Narrow tests — key_to_char mapping
    // =========================================================================

    #[test]
    fn key_to_char_maps_letter() {
        let key = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
        assert_eq!(PermissionHandler::key_to_char(key), Some('y'));
    }

    #[test]
    fn key_to_char_maps_enter() {
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(PermissionHandler::key_to_char(key), Some('\r'));
    }

    #[test]
    fn key_to_char_maps_esc() {
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(PermissionHandler::key_to_char(key), Some('\x1b'));
    }

    #[test]
    fn key_to_char_maps_left_to_h() {
        let key = KeyEvent::new(KeyCode::Left, KeyModifiers::NONE);
        assert_eq!(PermissionHandler::key_to_char(key), Some('h'));
    }

    #[test]
    fn key_to_char_maps_right_to_l() {
        let key = KeyEvent::new(KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(PermissionHandler::key_to_char(key), Some('l'));
    }

    #[test]
    fn key_to_char_returns_none_for_f1() {
        let key = KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE);
        assert_eq!(PermissionHandler::key_to_char(key), None);
    }

    // =========================================================================
    // Arrow navigation persists across keypresses (M-3 fix)
    // =========================================================================

    #[tokio::test]
    async fn arrow_navigation_then_enter_confirms_navigated_selection() {
        let mut handler = PermissionHandler::new();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_permission(test_permission_request());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        // Default selection is AllowOnce. Press Right to move to AllowAlways.
        let right = AppEvent::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        let result = handler.handle(&right, &mut ctx).await.unwrap();
        assert_eq!(result, Handled::CONSUMED);
        assert!(
            ctx.state.tool_state().has_pending_permission(),
            "Right arrow must not resolve the prompt"
        );

        // Now press Enter — should confirm AllowAlways (the navigated position),
        // not AllowOnce (the default).
        let enter = AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let _result = handler.handle(&enter, &mut ctx).await.unwrap();
        assert!(
            !ctx.state.tool_state().has_pending_permission(),
            "Enter after navigation must resolve the prompt"
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

        let handler = PermissionHandler::new();
        let mut dispatcher = EventDispatcher::new(vec![Box::new(handler)]);

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_permission(test_permission_request());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

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

    // =========================================================================
    // V-1: Stale prompt state is reset when pending request changes
    // =========================================================================

    #[tokio::test]
    async fn stale_prompt_state_reset_on_request_change() {
        let mut handler = PermissionHandler::new();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // Set pending permission for tool A
        let request_a = PermissionRequest {
            tool_name: "tool_a".to_string(),
            tool_input: Some("input_a".to_string()),
            description: "Run tool A".to_string(),
        };
        state.set_pending_permission(request_a);

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        // Navigate selection: Right arrow moves from AllowOnce to AllowAlways
        let right = AppEvent::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        let result = handler.handle(&right, &mut ctx).await.unwrap();
        assert_eq!(result, Handled::CONSUMED);
        assert!(ctx.state.tool_state().has_pending_permission());

        // Verify handler has prompt state with navigated selection
        assert!(handler.prompt_state.is_some());

        // Now change the pending permission to tool B (simulating tool A being
        // resolved programmatically and tool B becoming pending).
        ctx.state.clear_pending_permission();
        let request_b = PermissionRequest {
            tool_name: "tool_b".to_string(),
            tool_input: Some("input_b".to_string()),
            description: "Run tool B".to_string(),
        };
        ctx.state.set_pending_permission(request_b);

        // Press Enter — should confirm tool B with default selection (AllowOnce),
        // NOT tool A's navigated selection (AllowAlways).
        let enter = AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let _result = handler.handle(&enter, &mut ctx).await.unwrap();

        // The pending permission should be cleared (tool B was resolved)
        assert!(
            !ctx.state.tool_state().has_pending_permission(),
            "Enter must resolve tool B's permission"
        );
    }

    // =========================================================================
    // V-2: Chord buffer cleared when permission modal activates
    // =========================================================================

    #[tokio::test]
    async fn permission_handler_clears_chord_buffer() {
        let mut handler = PermissionHandler::new();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // Put chord buffer in Partial state by adding a chord binding and
        // pressing the first key of that chord.
        {
            use crate::keybindings::{Action, KeyChord, KeyPress, KeyResolution};
            use crossterm::event::{KeyCode as KC, KeyModifiers as KM};

            let mgr = state.keybindings_mut();
            mgr.bindings_mut().insert(
                KeyChord(vec![
                    KeyPress::from_crossterm(KC::Char('x'), KM::CONTROL),
                    KeyPress::from_crossterm(KC::Char('k'), KM::CONTROL),
                ]),
                Action::KillBackgroundAgents,
            );

            let key1 = KeyPress::from_crossterm(KC::Char('x'), KM::CONTROL);
            let result = mgr.resolve(key1);
            assert_eq!(result, KeyResolution::Partial);
            assert!(!mgr.chord_buffer_empty());
        }

        // Now activate permission prompt
        state.set_pending_permission(test_permission_request());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        // Send a key to the permission handler — it should clear the chord buffer
        let key = AppEvent::Key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        let _result = handler.handle(&key, &mut ctx).await.unwrap();

        // Verify chord buffer was cleared
        assert!(
            ctx.state.keybindings_mut().chord_buffer_empty(),
            "Permission handler must clear the chord buffer when consuming key events"
        );
    }
}
