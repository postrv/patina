//! Question handler for interactive user question prompts.
//!
//! [`QuestionHandler`] intercepts [`AppEvent::Key`] events when a question
//! prompt is active and converts them into user responses.
//!
//! This handler **must** run after `PlanHandler` and before
//! `KeyboardHandler` in the dispatch chain.

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use crossterm::event::KeyCode;
use tracing::debug;

use crate::app::context::AppContext;
use crate::app::dispatch::{EventHandler, Handled};
use crate::app::events::AppEvent;
use crate::types::ui_state::QuestionState;

/// Handles user question prompts from the `ask_user` tool.
///
/// When a question is pending, `QuestionHandler` intercepts all key events
/// to drive the question prompt UI:
///
/// - **Enter**: Submit the current response
/// - **Esc**: Cancel the question
/// - **Up / k**: Navigate to previous option
/// - **Down / j**: Navigate to next option
/// - **Tab**: Toggle between option list and free-text input
/// - **Backspace**: Delete last character in free-text mode
/// - **Char(c)**: Append character in free-text mode
///
/// All key events are consumed while the question prompt is active.
pub struct QuestionHandler;

impl QuestionHandler {
    /// Processes navigation, text input, and mode toggle keys for the question prompt.
    ///
    /// Returns `Some(Handled::CONSUMED)` for navigation/input keys.
    /// Returns `None` for submit/cancel keys that need cross-cutting logic.
    fn handle_input(
        key: &crossterm::event::KeyEvent,
        question: &mut QuestionState,
    ) -> Option<Handled> {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if !question.in_free_text => {
                question.select_prev();
                Some(Handled::CONSUMED)
            }
            KeyCode::Down | KeyCode::Char('j') if !question.in_free_text => {
                question.select_next();
                Some(Handled::CONSUMED)
            }
            KeyCode::Tab => {
                question.toggle_input_mode();
                Some(Handled::CONSUMED)
            }
            KeyCode::Backspace => {
                question.pop_char();
                Some(Handled::CONSUMED)
            }
            KeyCode::Char(c) => {
                question.push_char(c);
                Some(Handled::CONSUMED)
            }
            _ => None, // not an input key (submit/cancel/other)
        }
    }
}

impl EventHandler for QuestionHandler {
    fn handle<'a>(
        &'a mut self,
        event: &'a AppEvent,
        ctx: &'a mut AppContext<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<Handled>> + Send + 'a>> {
        Box::pin(async move {
            let AppEvent::Key(key) = event else {
                return Ok(Handled::IGNORED);
            };

            if !ctx.state.has_pending_question() {
                return Ok(Handled::IGNORED);
            }

            // Clear chord buffer so stale chord state from before the
            // modal appeared does not corrupt the next keypress (V-2).
            ctx.state.keybindings_mut().clear_chord();

            // Try input/navigation first (narrow path — only touches QuestionState)
            if let Some(question) = ctx.state.pending_question_mut() {
                if let Some(handled) = Self::handle_input(key, question) {
                    return Ok(handled);
                }
            }

            // Submit/cancel (broad path — crosses sub-states)
            match key.code {
                KeyCode::Enter => {
                    if let Some(result) = ctx.state.submit_question() {
                        debug!("Question answered, sending tool result");
                        send_tool_result(ctx, result).await;
                    }
                }
                KeyCode::Esc => {
                    if let Some(result) = ctx.state.cancel_question() {
                        debug!("Question cancelled, sending tool result");
                        send_tool_result(ctx, result).await;
                    }
                }
                _ => {} // consume but ignore
            }

            Ok(Handled::CONSUMED)
        })
    }

    fn name(&self) -> &str {
        "question"
    }
}

/// Records an interactive tool result as if it completed normally.
async fn send_tool_result(ctx: &mut AppContext<'_>, result: crate::types::ToolResultBlock) {
    let tool_id = result.tool_use_id.clone();
    ctx.state.record_tool_result(&tool_id, result);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{AnthropicClient, LlmProvider};
    use crate::app::state::AppState;
    use crate::session::SessionManager;
    use crate::types::config::ParallelMode;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use secrecy::SecretString;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::TempDir;

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

    fn test_question_with_options() -> QuestionState {
        QuestionState::new(
            "toolu_q".into(),
            "Which framework?".into(),
            vec!["React".into(), "Vue".into(), "Svelte".into()],
            true,
        )
    }

    fn test_question_free_text() -> QuestionState {
        QuestionState::new("toolu_q".into(), "What name?".into(), vec![], true)
    }

    // =========================================================================
    // name
    // =========================================================================

    #[test]
    fn name_returns_question() {
        let handler = QuestionHandler;
        assert_eq!(handler.name(), "question");
    }

    // =========================================================================
    // No question pending — IGNORED
    // =========================================================================

    #[tokio::test]
    async fn handle_key_when_no_question_returns_ignored() {
        let mut handler = QuestionHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let result = handler.handle(&event, &mut ctx).await.unwrap();
        assert_eq!(result, Handled::IGNORED);
    }

    #[tokio::test]
    async fn handle_tick_returns_ignored() {
        let mut handler = QuestionHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let result = handler.handle(&AppEvent::Tick, &mut ctx).await.unwrap();
        assert_eq!(result, Handled::IGNORED);
    }

    // =========================================================================
    // Question pending — Key events CONSUMED
    // =========================================================================

    #[tokio::test]
    async fn handle_key_when_question_pending_returns_consumed() {
        let mut handler = QuestionHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_question(test_question_with_options());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
        let result = handler.handle(&event, &mut ctx).await.unwrap();
        assert_eq!(result, Handled::CONSUMED);
    }

    // =========================================================================
    // Navigation with options
    // =========================================================================

    #[tokio::test]
    async fn down_navigates_next_option() {
        let mut handler = QuestionHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_question(test_question_with_options());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(ctx.state.pending_question().unwrap().selected_option, 1);
    }

    #[tokio::test]
    async fn up_navigates_prev_option() {
        let mut handler = QuestionHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        let mut q = test_question_with_options();
        q.selected_option = 2;
        state.set_pending_question(q);

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(ctx.state.pending_question().unwrap().selected_option, 1);
    }

    // =========================================================================
    // Free-text input
    // =========================================================================

    #[tokio::test]
    async fn char_appended_in_free_text_mode() {
        let mut handler = QuestionHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_question(test_question_free_text());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(ctx.state.pending_question().unwrap().free_text_input, "x");
    }

    #[tokio::test]
    async fn backspace_removes_in_free_text_mode() {
        let mut handler = QuestionHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        let mut q = test_question_free_text();
        q.free_text_input = "ab".into();
        state.set_pending_question(q);

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(ctx.state.pending_question().unwrap().free_text_input, "a");
    }

    // =========================================================================
    // Submit / Cancel
    // =========================================================================

    #[tokio::test]
    async fn enter_submits_and_clears_question() {
        let mut handler = QuestionHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_question(test_question_with_options());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(!ctx.state.has_pending_question());
    }

    #[tokio::test]
    async fn esc_cancels_and_clears_question() {
        let mut handler = QuestionHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_question(test_question_with_options());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(!ctx.state.has_pending_question());
    }

    // =========================================================================
    // Tab toggles mode
    // =========================================================================

    #[tokio::test]
    async fn tab_toggles_between_options_and_free_text() {
        let mut handler = QuestionHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_question(test_question_with_options());
        assert!(!state.pending_question().unwrap().in_free_text);

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(ctx.state.pending_question().unwrap().in_free_text);
    }

    // =========================================================================
    // Narrow tests — QuestionHandler::handle_input
    // =========================================================================

    #[test]
    fn handle_input_down_selects_next_option() {
        let mut q = test_question_with_options();
        assert_eq!(q.selected_option, 0);
        assert!(!q.in_free_text);

        let key = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        let result = QuestionHandler::handle_input(&key, &mut q);

        assert_eq!(result, Some(Handled::CONSUMED));
        assert_eq!(q.selected_option, 1);
    }

    #[test]
    fn handle_input_up_selects_prev_option() {
        let mut q = test_question_with_options();
        q.selected_option = 2;

        let key = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        let result = QuestionHandler::handle_input(&key, &mut q);

        assert_eq!(result, Some(Handled::CONSUMED));
        assert_eq!(q.selected_option, 1);
    }

    #[test]
    fn handle_input_char_pushes_in_free_text() {
        let mut q = test_question_free_text();
        assert!(q.in_free_text);

        let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
        let result = QuestionHandler::handle_input(&key, &mut q);

        assert_eq!(result, Some(Handled::CONSUMED));
        assert_eq!(q.free_text_input, "x");
    }

    #[test]
    fn handle_input_char_always_handled_even_when_not_free_text() {
        // Char(c) is always dispatched to push_char; push_char guards on in_free_text
        let mut q = test_question_with_options();
        assert!(!q.in_free_text);

        let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
        let result = QuestionHandler::handle_input(&key, &mut q);

        assert_eq!(result, Some(Handled::CONSUMED));
        // push_char is a no-op when not in free text
        assert_eq!(q.free_text_input, "");
    }

    #[test]
    fn handle_input_tab_toggles_mode() {
        let mut q = test_question_with_options();
        assert!(!q.in_free_text);

        let key = KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);
        let result = QuestionHandler::handle_input(&key, &mut q);

        assert_eq!(result, Some(Handled::CONSUMED));
        assert!(q.in_free_text);
    }

    #[test]
    fn handle_input_backspace_removes_char() {
        let mut q = test_question_free_text();
        q.free_text_input = "ab".into();

        let key = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
        let result = QuestionHandler::handle_input(&key, &mut q);

        assert_eq!(result, Some(Handled::CONSUMED));
        assert_eq!(q.free_text_input, "a");
    }

    #[test]
    fn handle_input_enter_returns_none() {
        let mut q = test_question_with_options();

        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let result = QuestionHandler::handle_input(&key, &mut q);

        assert_eq!(result, None);
    }

    #[test]
    fn handle_input_esc_returns_none() {
        let mut q = test_question_with_options();

        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        let result = QuestionHandler::handle_input(&key, &mut q);

        assert_eq!(result, None);
    }

    #[test]
    fn handle_input_k_navigates_when_not_free_text() {
        let mut q = test_question_with_options();
        q.selected_option = 1;

        let key = KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE);
        let result = QuestionHandler::handle_input(&key, &mut q);

        assert_eq!(result, Some(Handled::CONSUMED));
        assert_eq!(q.selected_option, 0);
    }

    #[test]
    fn handle_input_k_pushes_char_when_in_free_text() {
        let mut q = test_question_with_options();
        q.in_free_text = true;

        let key = KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE);
        let result = QuestionHandler::handle_input(&key, &mut q);

        assert_eq!(result, Some(Handled::CONSUMED));
        assert_eq!(q.free_text_input, "k");
    }

    // =========================================================================
    // Send bound
    // =========================================================================

    #[test]
    fn question_handler_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<QuestionHandler>();
    }
}
