//! Continuous coding handler for processing loop progress events.
//!
//! [`ContinuousHandler`] processes [`AppEvent::Continuous`] events, updating the
//! TUI continuous panel state. It handles:
//!
//! - Iteration progress (start, complete)
//! - Quality gate results (pass, fail)
//! - Stagnation warnings
//! - Human checkpoint notifications

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use tracing::debug;

use crate::app::context::AppContext;
use crate::app::dispatch::{EventHandler, Handled};
use crate::app::events::AppEvent;
use crate::continuous::ContinuousEvent;

/// Handles continuous coding loop events for the TUI progress display.
///
/// When a continuous coding session is running, the `ContinuousRunner` emits
/// [`ContinuousEvent`] values that flow through the event dispatcher.
/// `ContinuousHandler` consumes [`AppEvent::Continuous`] events, updating
/// [`AppState`](crate::app::state::AppState) continuous panel state for rendering.
///
/// # Examples
///
/// ```rust,ignore
/// use patina::app::handlers::continuous::ContinuousHandler;
/// use patina::app::dispatch::EventDispatcher;
///
/// let dispatcher = EventDispatcher::new(vec![
///     Box::new(ContinuousHandler),
/// ]);
/// ```
pub struct ContinuousHandler;

impl EventHandler for ContinuousHandler {
    fn handle<'a>(
        &'a mut self,
        event: &'a AppEvent,
        ctx: &'a mut AppContext<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<Handled>> + Send + 'a>> {
        Box::pin(async move {
            match event {
                AppEvent::Continuous(continuous_event) => {
                    match continuous_event {
                        ContinuousEvent::IterationStart { iteration } => {
                            debug!(iteration = %iteration, "Continuous iteration started");
                            ctx.state.update_continuous_iteration(*iteration);
                        }
                        ContinuousEvent::IterationComplete {
                            iteration,
                            duration_ms,
                        } => {
                            debug!(
                                iteration = %iteration,
                                duration_ms = %duration_ms,
                                "Continuous iteration completed"
                            );
                            ctx.state
                                .complete_continuous_iteration(*iteration, *duration_ms);
                        }
                        ContinuousEvent::QualityGateCheck { gate } => {
                            debug!(gate = %gate, "Quality gate check starting");
                            ctx.state.set_continuous_gate_checking(gate);
                        }
                        ContinuousEvent::QualityGateResult {
                            gate,
                            passed,
                            message,
                        } => {
                            debug!(
                                gate = %gate,
                                passed = %passed,
                                "Quality gate result"
                            );
                            ctx.state.record_continuous_gate_result(
                                gate,
                                *passed,
                                message.as_deref(),
                            );
                        }
                        ContinuousEvent::StagnationDetected {
                            iterations_without_progress,
                            threshold,
                        } => {
                            debug!(
                                iterations = %iterations_without_progress,
                                threshold = %threshold,
                                "Stagnation detected"
                            );
                            ctx.state.set_continuous_stagnation(
                                *iterations_without_progress,
                                *threshold,
                            );
                        }
                        ContinuousEvent::HumanCheckpointRequired { reason } => {
                            debug!(reason = %reason, "Human checkpoint required");
                            ctx.state.set_continuous_human_checkpoint(reason);
                        }
                    }
                    Ok(Handled::CONSUMED)
                }
                _ => Ok(Handled::IGNORED),
            }
        })
    }

    fn name(&self) -> &str {
        "continuous"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{AnthropicClient, LlmProvider};
    use crate::app::state::AppState;
    use crate::continuous::ContinuousEvent;
    use crate::types::ui_state::ContinuousLoopStatus;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use secrecy::SecretString;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::TempDir;

    use crate::session::SessionManager;
    use crate::types::config::ParallelMode;

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
    // ContinuousHandler::name
    // =========================================================================

    #[test]
    fn name_returns_continuous() {
        let handler = ContinuousHandler;
        assert_eq!(handler.name(), "continuous");
    }

    // =========================================================================
    // ContinuousHandler::handle — Continuous events must be consumed
    // =========================================================================

    #[tokio::test]
    async fn handle_iteration_start_returns_consumed() {
        let mut handler = ContinuousHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Continuous(ContinuousEvent::IterationStart { iteration: 1 });
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "ContinuousHandler must consume IterationStart events"
        );
    }

    #[tokio::test]
    async fn handle_iteration_complete_returns_consumed() {
        let mut handler = ContinuousHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Continuous(ContinuousEvent::IterationComplete {
            iteration: 1,
            duration_ms: 5000,
        });
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "ContinuousHandler must consume IterationComplete events"
        );
    }

    #[tokio::test]
    async fn handle_quality_gate_check_returns_consumed() {
        let mut handler = ContinuousHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Continuous(ContinuousEvent::QualityGateCheck {
            gate: "tests".to_string(),
        });
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "ContinuousHandler must consume QualityGateCheck events"
        );
    }

    #[tokio::test]
    async fn handle_quality_gate_result_returns_consumed() {
        let mut handler = ContinuousHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Continuous(ContinuousEvent::QualityGateResult {
            gate: "clippy".to_string(),
            passed: true,
            message: None,
        });
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "ContinuousHandler must consume QualityGateResult events"
        );
    }

    #[tokio::test]
    async fn handle_stagnation_detected_returns_consumed() {
        let mut handler = ContinuousHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Continuous(ContinuousEvent::StagnationDetected {
            iterations_without_progress: 5,
            threshold: 3,
        });
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "ContinuousHandler must consume StagnationDetected events"
        );
    }

    #[tokio::test]
    async fn handle_human_checkpoint_returns_consumed() {
        let mut handler = ContinuousHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Continuous(ContinuousEvent::HumanCheckpointRequired {
            reason: "Test failures unresolvable".to_string(),
        });
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "ContinuousHandler must consume HumanCheckpointRequired events"
        );
    }

    // =========================================================================
    // ContinuousHandler::handle — non-continuous events must be ignored
    // =========================================================================

    #[tokio::test]
    async fn handle_key_returns_ignored() {
        let mut handler = ContinuousHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::IGNORED,
            "ContinuousHandler must ignore Key events"
        );
    }

    #[tokio::test]
    async fn handle_tick_returns_ignored() {
        let mut handler = ContinuousHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Tick;
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::IGNORED,
            "ContinuousHandler must ignore Tick events"
        );
    }

    #[tokio::test]
    async fn handle_quit_returns_ignored() {
        let mut handler = ContinuousHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Quit;
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::IGNORED,
            "ContinuousHandler must ignore Quit events"
        );
    }

    #[tokio::test]
    async fn handle_resize_returns_ignored() {
        let mut handler = ContinuousHandler;
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
            "ContinuousHandler must ignore Resize events"
        );
    }

    // =========================================================================
    // ContinuousHandler::handle — state mutations
    // =========================================================================

    #[tokio::test]
    async fn handle_iteration_start_updates_state() {
        let mut handler = ContinuousHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        assert_eq!(
            state.continuous().status(),
            &ContinuousLoopStatus::Inactive,
            "Status should be Inactive before first event"
        );

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Continuous(ContinuousEvent::IterationStart { iteration: 3 });
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            ctx.state.continuous().status(),
            &ContinuousLoopStatus::Running { iteration: 3 },
            "Status should show Running with current iteration"
        );
    }

    #[tokio::test]
    async fn handle_iteration_complete_updates_state() {
        let mut handler = ContinuousHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        // Start iteration first
        let start = AppEvent::Continuous(ContinuousEvent::IterationStart { iteration: 1 });
        let _ = handler.handle(&start, &mut ctx).await.unwrap();

        // Complete iteration
        let complete = AppEvent::Continuous(ContinuousEvent::IterationComplete {
            iteration: 1,
            duration_ms: 3500,
        });
        let _ = handler.handle(&complete, &mut ctx).await.unwrap();

        assert_eq!(ctx.state.continuous().iterations_completed(), 1);
        assert_eq!(ctx.state.continuous().last_duration_ms(), Some(3500));
    }

    #[tokio::test]
    async fn handle_gate_check_records_checking_gate() {
        let mut handler = ContinuousHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Continuous(ContinuousEvent::QualityGateCheck {
            gate: "tests".to_string(),
        });
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            ctx.state.continuous().checking_gate(),
            Some("tests"),
            "Should record which gate is currently being checked"
        );
    }

    #[tokio::test]
    async fn handle_gate_result_pass_records_result() {
        let mut handler = ContinuousHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Continuous(ContinuousEvent::QualityGateResult {
            gate: "clippy".to_string(),
            passed: true,
            message: Some("0 warnings".to_string()),
        });
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        let results = ctx.state.continuous().gate_results();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].gate, "clippy");
        assert!(results[0].passed);
        assert_eq!(results[0].message.as_deref(), Some("0 warnings"));
    }

    #[tokio::test]
    async fn handle_gate_result_fail_records_result() {
        let mut handler = ContinuousHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Continuous(ContinuousEvent::QualityGateResult {
            gate: "tests".to_string(),
            passed: false,
            message: Some("3 tests failed".to_string()),
        });
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        let results = ctx.state.continuous().gate_results();
        assert_eq!(results.len(), 1);
        assert!(!results[0].passed);
    }

    #[tokio::test]
    async fn handle_stagnation_updates_state() {
        let mut handler = ContinuousHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Continuous(ContinuousEvent::StagnationDetected {
            iterations_without_progress: 5,
            threshold: 3,
        });
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            ctx.state.continuous().status(),
            &ContinuousLoopStatus::Stagnated {
                iterations_without_progress: 5,
                threshold: 3,
            }
        );
    }

    #[tokio::test]
    async fn handle_human_checkpoint_updates_state() {
        let mut handler = ContinuousHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Continuous(ContinuousEvent::HumanCheckpointRequired {
            reason: "Cannot resolve test failures".to_string(),
        });
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            ctx.state.continuous().status(),
            &ContinuousLoopStatus::HumanRequired {
                reason: "Cannot resolve test failures".to_string(),
            }
        );
    }

    #[tokio::test]
    async fn handle_progress_marks_dirty_for_render() {
        let mut handler = ContinuousHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.mark_rendered();

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Continuous(ContinuousEvent::IterationStart { iteration: 1 });
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(
            ctx.needs_render(),
            "Continuous event must mark display dirty for re-render"
        );
    }

    #[tokio::test]
    async fn handle_multiple_gate_results_accumulate() {
        let mut handler = ContinuousHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event1 = AppEvent::Continuous(ContinuousEvent::QualityGateResult {
            gate: "tests".to_string(),
            passed: true,
            message: None,
        });
        let _ = handler.handle(&event1, &mut ctx).await.unwrap();

        let event2 = AppEvent::Continuous(ContinuousEvent::QualityGateResult {
            gate: "clippy".to_string(),
            passed: true,
            message: None,
        });
        let _ = handler.handle(&event2, &mut ctx).await.unwrap();

        assert_eq!(ctx.state.continuous().gate_results().len(), 2);
    }

    // =========================================================================
    // Send bound verification
    // =========================================================================

    #[test]
    fn continuous_handler_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<ContinuousHandler>();
    }

    // =========================================================================
    // Integration with EventDispatcher
    // =========================================================================

    #[tokio::test]
    async fn continuous_handler_works_in_dispatcher() {
        use crate::app::dispatch::EventDispatcher;

        let handler = ContinuousHandler;
        let mut dispatcher = EventDispatcher::new(vec![Box::new(handler)]);
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        // Continuous event should be consumed.
        let event = AppEvent::Continuous(ContinuousEvent::IterationStart { iteration: 1 });
        let result = dispatcher.dispatch(&event, &mut ctx).await.unwrap();
        assert_eq!(result, Handled::CONSUMED);

        // Key event should pass through.
        let key = AppEvent::Key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        let result = dispatcher.dispatch(&key, &mut ctx).await.unwrap();
        assert_eq!(result, Handled::IGNORED);
    }
}
