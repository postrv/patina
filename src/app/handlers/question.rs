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

impl EventHandler for QuestionHandler {
    fn handle<'a>(
        &'a mut self,
        event: &'a AppEvent,
        ctx: &'a mut AppContext<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<Handled>> + Send + 'a>> {
        Box::pin(async move {
            match event {
                AppEvent::Key(key) => {
                    if !ctx.state.has_pending_question() {
                        return Ok(Handled::IGNORED);
                    }

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
                        KeyCode::Up | KeyCode::Char('k') if !is_in_free_text(ctx) => {
                            if let Some(q) = ctx.state.pending_question_mut() {
                                q.select_prev();
                            }
                        }
                        KeyCode::Down | KeyCode::Char('j') if !is_in_free_text(ctx) => {
                            if let Some(q) = ctx.state.pending_question_mut() {
                                q.select_next();
                            }
                        }
                        KeyCode::Tab => {
                            if let Some(q) = ctx.state.pending_question_mut() {
                                q.toggle_input_mode();
                            }
                        }
                        KeyCode::Backspace => {
                            if let Some(q) = ctx.state.pending_question_mut() {
                                q.pop_char();
                            }
                        }
                        KeyCode::Char(c) => {
                            if let Some(q) = ctx.state.pending_question_mut() {
                                q.push_char(c);
                            }
                        }
                        _ => {} // consume but ignore
                    }

                    Ok(Handled::CONSUMED)
                }
                _ => Ok(Handled::IGNORED),
            }
        })
    }

    fn name(&self) -> &str {
        "question"
    }
}

/// Returns true if the pending question is in free-text input mode.
fn is_in_free_text(ctx: &AppContext<'_>) -> bool {
    ctx.state.pending_question().is_some_and(|q| q.in_free_text)
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
    use crate::types::ui_state::QuestionState;
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
    // Send bound
    // =========================================================================

    #[test]
    fn question_handler_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<QuestionHandler>();
    }
}
