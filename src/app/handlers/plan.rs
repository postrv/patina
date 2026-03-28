//! Plan handler for interactive plan review prompts.
//!
//! [`PlanHandler`] intercepts [`AppEvent::Key`] events when a plan review
//! prompt is active and converts them into plan decisions.
//!
//! This handler **must** run after `PermissionHandler` and before
//! `KeyboardHandler` in the dispatch chain so that key events are consumed
//! by the plan review modal rather than reaching normal input handling.

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use crossterm::event::KeyCode;
use tracing::debug;

use crate::app::context::AppContext;
use crate::app::dispatch::{EventHandler, Handled};
use crate::app::events::AppEvent;
use crate::types::ui_state::PlanState;

/// Handles plan review prompts.
///
/// When a plan is pending, `PlanHandler` intercepts all key events to
/// drive the plan review UI:
///
/// - **Enter**: Approve the plan
/// - **Esc / n**: Reject the plan
/// - **Up / k**: Navigate to previous step
/// - **Down / j**: Navigate to next step
///
/// All key events are consumed while the plan review is active,
/// preventing them from reaching the `KeyboardHandler`.
pub struct PlanHandler;

impl PlanHandler {
    /// Processes a navigation key event for the plan review modal.
    ///
    /// Handles Up/Down/j/k for step navigation. Returns `None` for non-navigation
    /// keys (approve/reject/other) which need cross-cutting logic.
    fn handle_navigation(
        key: &crossterm::event::KeyEvent,
        plan: &mut PlanState,
    ) -> Option<Handled> {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                plan.select_prev();
                Some(Handled::CONSUMED)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                plan.select_next();
                Some(Handled::CONSUMED)
            }
            _ => None, // not a navigation key
        }
    }
}

impl EventHandler for PlanHandler {
    fn handle<'a>(
        &'a mut self,
        event: &'a AppEvent,
        ctx: &'a mut AppContext<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<Handled>> + Send + 'a>> {
        Box::pin(async move {
            let AppEvent::Key(key) = event else {
                return Ok(Handled::IGNORED);
            };

            if !ctx.state.has_pending_plan() {
                return Ok(Handled::IGNORED);
            }

            // Clear chord buffer so stale chord state from before the
            // modal appeared does not corrupt the next keypress (V-2).
            ctx.state.keybindings_mut().clear_chord();

            // Try navigation first (narrow path — only touches PlanState)
            if let Some(plan) = ctx.state.pending_plan_mut() {
                if let Some(handled) = Self::handle_navigation(key, plan) {
                    return Ok(handled);
                }
            }

            // Approval/rejection (broad path — crosses sub-states)
            match key.code {
                KeyCode::Enter => {
                    if let Some(result) = ctx.state.approve_plan() {
                        debug!("Plan approved, sending tool result");
                        send_tool_result(ctx, result).await;
                    }
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                    if let Some(result) = ctx.state.reject_plan() {
                        debug!("Plan rejected, sending tool result");
                        send_tool_result(ctx, result).await;
                    }
                }
                _ => {} // consume but ignore
            }

            Ok(Handled::CONSUMED)
        })
    }

    fn name(&self) -> &str {
        "plan"
    }
}

/// Records an interactive tool result as if it completed normally.
///
/// Removes the tool from the executing set and feeds the result into the
/// tool loop state machine, allowing the conversation to continue.
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
    use crate::types::ui_state::PlanStep;
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

    fn test_plan() -> PlanState {
        PlanState::new(
            "toolu_plan".into(),
            "Test Plan".into(),
            vec![
                PlanStep {
                    description: "Step 1".into(),
                    tool_calls: vec!["bash".into()],
                },
                PlanStep {
                    description: "Step 2".into(),
                    tool_calls: vec!["edit".into()],
                },
            ],
        )
    }

    // =========================================================================
    // name
    // =========================================================================

    #[test]
    fn name_returns_plan() {
        let handler = PlanHandler;
        assert_eq!(handler.name(), "plan");
    }

    // =========================================================================
    // No plan pending — IGNORED
    // =========================================================================

    #[tokio::test]
    async fn handle_key_when_no_plan_returns_ignored() {
        let mut handler = PlanHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        assert!(!state.has_pending_plan());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let result = handler.handle(&event, &mut ctx).await.unwrap();
        assert_eq!(result, Handled::IGNORED);
    }

    #[tokio::test]
    async fn handle_tick_returns_ignored() {
        let mut handler = PlanHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Tick;
        let result = handler.handle(&event, &mut ctx).await.unwrap();
        assert_eq!(result, Handled::IGNORED);
    }

    // =========================================================================
    // Plan pending — Key events CONSUMED
    // =========================================================================

    #[tokio::test]
    async fn handle_key_when_plan_pending_returns_consumed() {
        let mut handler = PlanHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_plan(test_plan());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        let result = handler.handle(&event, &mut ctx).await.unwrap();
        assert_eq!(result, Handled::CONSUMED);
    }

    // =========================================================================
    // Navigation
    // =========================================================================

    #[tokio::test]
    async fn down_navigates_to_next_step() {
        let mut handler = PlanHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_plan(test_plan());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(ctx.state.pending_plan().unwrap().selected_index, 1);
    }

    #[tokio::test]
    async fn up_navigates_to_prev_step() {
        let mut handler = PlanHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        let mut plan = test_plan();
        plan.selected_index = 1;
        state.set_pending_plan(plan);

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(ctx.state.pending_plan().unwrap().selected_index, 0);
    }

    // =========================================================================
    // Approve / Reject clears pending plan
    // =========================================================================

    #[tokio::test]
    async fn enter_approves_and_clears_plan() {
        let mut handler = PlanHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_plan(test_plan());
        assert!(state.has_pending_plan());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(!ctx.state.has_pending_plan());
    }

    #[tokio::test]
    async fn esc_rejects_and_clears_plan() {
        let mut handler = PlanHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_plan(test_plan());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(!ctx.state.has_pending_plan());
    }

    #[tokio::test]
    async fn n_key_rejects_and_clears_plan() {
        let mut handler = PlanHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_pending_plan(test_plan());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(!ctx.state.has_pending_plan());
    }

    // =========================================================================
    // Narrow tests — PlanHandler::handle_navigation
    // =========================================================================

    #[test]
    fn handle_navigation_down_selects_next() {
        let mut plan = test_plan();
        assert_eq!(plan.selected_index, 0);

        let key = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        let result = PlanHandler::handle_navigation(&key, &mut plan);

        assert_eq!(result, Some(Handled::CONSUMED));
        assert_eq!(plan.selected_index, 1);
    }

    #[test]
    fn handle_navigation_up_selects_prev() {
        let mut plan = test_plan();
        plan.selected_index = 1;

        let key = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        let result = PlanHandler::handle_navigation(&key, &mut plan);

        assert_eq!(result, Some(Handled::CONSUMED));
        assert_eq!(plan.selected_index, 0);
    }

    #[test]
    fn handle_navigation_j_selects_next() {
        let mut plan = test_plan();
        assert_eq!(plan.selected_index, 0);

        let key = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE);
        let result = PlanHandler::handle_navigation(&key, &mut plan);

        assert_eq!(result, Some(Handled::CONSUMED));
        assert_eq!(plan.selected_index, 1);
    }

    #[test]
    fn handle_navigation_k_selects_prev() {
        let mut plan = test_plan();
        plan.selected_index = 1;

        let key = KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE);
        let result = PlanHandler::handle_navigation(&key, &mut plan);

        assert_eq!(result, Some(Handled::CONSUMED));
        assert_eq!(plan.selected_index, 0);
    }

    #[test]
    fn handle_navigation_non_nav_key_returns_none() {
        let mut plan = test_plan();

        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let result = PlanHandler::handle_navigation(&key, &mut plan);

        assert_eq!(result, None);
        // Plan state unchanged
        assert_eq!(plan.selected_index, 0);
    }

    // =========================================================================
    // Send bound
    // =========================================================================

    #[test]
    fn plan_handler_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<PlanHandler>();
    }
}
