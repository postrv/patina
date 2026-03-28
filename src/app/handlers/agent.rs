//! Agent handler for processing background agent events.
//!
//! [`AgentHandler`] processes [`AppEvent::Agent`] events, updating the
//! TUI agent panel state and recording conflict reports. It handles:
//!
//! - Agent progress updates (iteration started, content deltas)
//! - Agent completion notifications
//! - Agent failure notifications
//! - Cross-agent conflict detection alerts

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use tracing::debug;

use crate::agents::{AgentEvent, AgentProgress};
use crate::app::context::AppContext;
use crate::app::dispatch::{EventHandler, Handled};
use crate::app::events::AppEvent;
use crate::app::state::AgentPanelState;

/// Handles background agent events for the TUI agent panel.
///
/// When agents are running in worktrees, they emit progress events that
/// flow through the event dispatcher. `AgentHandler` consumes
/// [`AppEvent::Agent`] events, updating [`AppState`](crate::app::state::AppState)
/// agent panel entries and recording conflict reports.
///
/// # Examples
///
/// ```rust,ignore
/// use patina::app::handlers::agent::AgentHandler;
/// use patina::app::dispatch::EventDispatcher;
///
/// let dispatcher = EventDispatcher::new(vec![
///     Box::new(AgentHandler),
/// ]);
/// ```
pub struct AgentHandler;

impl AgentHandler {
    /// Processes an agent event, updating the agent panel state.
    ///
    /// This is the core logic extracted for independent testability.
    /// Takes only `AgentPanelState`, not the full `AppState`.
    fn handle_agent_event(event: &AgentEvent, panel: &mut AgentPanelState) -> Result<Handled> {
        match event {
            AgentEvent::Progress {
                agent_id,
                agent_name,
                progress,
            } => {
                panel.update_progress(agent_id, agent_name, progress);
                Ok(Handled::CONSUMED)
            }
            AgentEvent::ConflictDetected { report } => {
                panel.add_conflict(report.clone());
                Ok(Handled::CONSUMED)
            }
        }
    }
}

impl EventHandler for AgentHandler {
    fn handle<'a>(
        &'a mut self,
        event: &'a AppEvent,
        ctx: &'a mut AppContext<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<Handled>> + Send + 'a>> {
        Box::pin(async move {
            let AppEvent::Agent(agent_event) = event else {
                return Ok(Handled::IGNORED);
            };
            debug!("Agent event received");

            // Fire SubagentStop hook for terminal agent states
            if let AgentEvent::Progress {
                agent_id, progress, ..
            } = agent_event
            {
                let stop_reason = match progress {
                    AgentProgress::Completed { .. } => Some("completed"),
                    AgentProgress::Failed { error, .. } => Some(error.as_str()),
                    _ => None,
                };
                if let Some(reason) = stop_reason {
                    // Send desktop notification for agent completion/failure
                    let notification_body = format!("Agent {agent_id}: {reason}");
                    if let Err(e) = ctx
                        .state
                        .notification_manager()
                        .notify("Patina", &notification_body)
                    {
                        tracing::debug!("Failed to send agent notification: {e}");
                    }

                    let executor = std::sync::Arc::clone(&ctx.state.tool_state().tool_executor);
                    let agent_id = agent_id.clone();
                    let reason = reason.to_string();
                    tokio::spawn(async move {
                        if let Err(e) = executor
                            .hooks()
                            .fire_subagent_stop(&agent_id, &reason)
                            .await
                        {
                            tracing::debug!("Hook fire failed: {e}");
                        }
                    });
                }
            }

            Self::handle_agent_event(agent_event, ctx.state.agent_panel_mut())
        })
    }

    fn name(&self) -> &str {
        "agent"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{AgentProgress, ConflictReport};
    use crate::api::{AnthropicClient, LlmProvider};
    use crate::app::state::{AgentPanelStatus, AppState};
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

    // =========================================================================
    // AgentHandler::name
    // =========================================================================

    #[test]
    fn name_returns_agent() {
        let handler = AgentHandler;
        assert_eq!(handler.name(), "agent");
    }

    // =========================================================================
    // AgentHandler::handle — Agent progress events must be consumed
    // =========================================================================

    #[tokio::test]
    async fn handle_agent_progress_iteration_started_returns_consumed() {
        let mut handler = AgentHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Agent(AgentEvent::Progress {
            agent_id: "agent-1".to_string(),
            agent_name: "explorer".to_string(),
            progress: AgentProgress::IterationStarted {
                iteration: 1,
                max: 10,
            },
        });
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "AgentHandler must consume Agent progress events"
        );
    }

    #[tokio::test]
    async fn handle_agent_progress_content_delta_returns_consumed() {
        let mut handler = AgentHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        // Start with an iteration so the agent exists in the panel.
        let init_event = AppEvent::Agent(AgentEvent::Progress {
            agent_id: "agent-1".to_string(),
            agent_name: "explorer".to_string(),
            progress: AgentProgress::IterationStarted {
                iteration: 1,
                max: 10,
            },
        });
        let _ = handler.handle(&init_event, &mut ctx).await.unwrap();

        let event = AppEvent::Agent(AgentEvent::Progress {
            agent_id: "agent-1".to_string(),
            agent_name: "explorer".to_string(),
            progress: AgentProgress::ContentDelta("Exploring the codebase...".to_string()),
        });
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "AgentHandler must consume Agent content delta events"
        );
    }

    #[tokio::test]
    async fn handle_agent_progress_completed_returns_consumed() {
        let mut handler = AgentHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Agent(AgentEvent::Progress {
            agent_id: "agent-1".to_string(),
            agent_name: "explorer".to_string(),
            progress: AgentProgress::Completed {
                output: "Found 5 relevant files.".to_string(),
                iterations_used: 3,
            },
        });
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "AgentHandler must consume Agent completion events"
        );
    }

    #[tokio::test]
    async fn handle_agent_progress_failed_returns_consumed() {
        let mut handler = AgentHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Agent(AgentEvent::Progress {
            agent_id: "agent-1".to_string(),
            agent_name: "explorer".to_string(),
            progress: AgentProgress::Failed {
                error: "API rate limit exceeded".to_string(),
                iterations_used: 2,
            },
        });
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "AgentHandler must consume Agent failure events"
        );
    }

    #[tokio::test]
    async fn handle_conflict_detected_returns_consumed() {
        let mut handler = AgentHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Agent(AgentEvent::ConflictDetected {
            report: ConflictReport::empty(),
        });
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "AgentHandler must consume Agent conflict events"
        );
    }

    // =========================================================================
    // AgentHandler::handle — non-agent events must be ignored
    // =========================================================================

    #[tokio::test]
    async fn handle_key_returns_ignored() {
        let mut handler = AgentHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::IGNORED,
            "AgentHandler must ignore Key events"
        );
    }

    #[tokio::test]
    async fn handle_tick_returns_ignored() {
        let mut handler = AgentHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Tick;
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::IGNORED,
            "AgentHandler must ignore Tick events"
        );
    }

    #[tokio::test]
    async fn handle_quit_returns_ignored() {
        let mut handler = AgentHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Quit;
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::IGNORED,
            "AgentHandler must ignore Quit events"
        );
    }

    #[tokio::test]
    async fn handle_resize_returns_ignored() {
        let mut handler = AgentHandler;
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
            "AgentHandler must ignore Resize events"
        );
    }

    // =========================================================================
    // AgentHandler::handle — state mutations
    // =========================================================================

    #[tokio::test]
    async fn handle_progress_creates_agent_panel_entry() {
        let mut handler = AgentHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        assert!(
            state.agent_panel().entries().is_empty(),
            "No agents before first event"
        );

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Agent(AgentEvent::Progress {
            agent_id: "agent-1".to_string(),
            agent_name: "explorer".to_string(),
            progress: AgentProgress::IterationStarted {
                iteration: 1,
                max: 10,
            },
        });
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            ctx.state.agent_panel().entries().len(),
            1,
            "Handler must create an agent panel entry"
        );
        let entry = &ctx.state.agent_panel().entries()[0];
        assert_eq!(entry.agent_id, "agent-1");
        assert_eq!(entry.agent_name, "explorer");
        assert_eq!(
            entry.status,
            AgentPanelStatus::Running {
                iteration: 1,
                max_iterations: 10,
            }
        );
    }

    #[tokio::test]
    async fn handle_progress_updates_existing_entry() {
        let mut handler = AgentHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        // First event creates the entry.
        let event1 = AppEvent::Agent(AgentEvent::Progress {
            agent_id: "agent-1".to_string(),
            agent_name: "explorer".to_string(),
            progress: AgentProgress::IterationStarted {
                iteration: 1,
                max: 10,
            },
        });
        let _ = handler.handle(&event1, &mut ctx).await.unwrap();

        // Second event updates it.
        let event2 = AppEvent::Agent(AgentEvent::Progress {
            agent_id: "agent-1".to_string(),
            agent_name: "explorer".to_string(),
            progress: AgentProgress::IterationStarted {
                iteration: 2,
                max: 10,
            },
        });
        let _ = handler.handle(&event2, &mut ctx).await.unwrap();

        assert_eq!(
            ctx.state.agent_panel().entries().len(),
            1,
            "Should update existing entry, not create a second"
        );
        assert_eq!(
            ctx.state.agent_panel().entries()[0].status,
            AgentPanelStatus::Running {
                iteration: 2,
                max_iterations: 10,
            }
        );
    }

    #[tokio::test]
    async fn handle_completion_sets_completed_status() {
        let mut handler = AgentHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        // Start agent.
        let event1 = AppEvent::Agent(AgentEvent::Progress {
            agent_id: "agent-1".to_string(),
            agent_name: "explorer".to_string(),
            progress: AgentProgress::IterationStarted {
                iteration: 1,
                max: 10,
            },
        });
        let _ = handler.handle(&event1, &mut ctx).await.unwrap();

        // Complete agent.
        let event2 = AppEvent::Agent(AgentEvent::Progress {
            agent_id: "agent-1".to_string(),
            agent_name: "explorer".to_string(),
            progress: AgentProgress::Completed {
                output: "Found relevant code.".to_string(),
                iterations_used: 3,
            },
        });
        let _ = handler.handle(&event2, &mut ctx).await.unwrap();

        assert_eq!(
            ctx.state.agent_panel().entries()[0].status,
            AgentPanelStatus::Completed { iterations_used: 3 }
        );
        assert_eq!(
            ctx.state.agent_panel().entries()[0].last_content,
            "Found relevant code."
        );
    }

    #[tokio::test]
    async fn handle_failure_sets_failed_status() {
        let mut handler = AgentHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        // Start agent.
        let event1 = AppEvent::Agent(AgentEvent::Progress {
            agent_id: "agent-1".to_string(),
            agent_name: "explorer".to_string(),
            progress: AgentProgress::IterationStarted {
                iteration: 1,
                max: 10,
            },
        });
        let _ = handler.handle(&event1, &mut ctx).await.unwrap();

        // Fail agent.
        let event2 = AppEvent::Agent(AgentEvent::Progress {
            agent_id: "agent-1".to_string(),
            agent_name: "explorer".to_string(),
            progress: AgentProgress::Failed {
                error: "Rate limit exceeded".to_string(),
                iterations_used: 2,
            },
        });
        let _ = handler.handle(&event2, &mut ctx).await.unwrap();

        assert_eq!(
            ctx.state.agent_panel().entries()[0].status,
            AgentPanelStatus::Failed {
                error: "Rate limit exceeded".to_string(),
            }
        );
    }

    #[tokio::test]
    async fn handle_content_delta_updates_last_content() {
        let mut handler = AgentHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        // Start agent.
        let event1 = AppEvent::Agent(AgentEvent::Progress {
            agent_id: "agent-1".to_string(),
            agent_name: "explorer".to_string(),
            progress: AgentProgress::IterationStarted {
                iteration: 1,
                max: 10,
            },
        });
        let _ = handler.handle(&event1, &mut ctx).await.unwrap();

        // Send content delta.
        let event2 = AppEvent::Agent(AgentEvent::Progress {
            agent_id: "agent-1".to_string(),
            agent_name: "explorer".to_string(),
            progress: AgentProgress::ContentDelta("Looking at src/main.rs...".to_string()),
        });
        let _ = handler.handle(&event2, &mut ctx).await.unwrap();

        assert_eq!(
            ctx.state.agent_panel().entries()[0].last_content,
            "Looking at src/main.rs..."
        );
    }

    #[tokio::test]
    async fn handle_conflict_adds_to_pending_reports() {
        let mut handler = AgentHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        assert!(
            !state.agent_panel().has_pending_conflicts(),
            "No conflicts before first event"
        );

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Agent(AgentEvent::ConflictDetected {
            report: ConflictReport::empty(),
        });
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(
            ctx.state.agent_panel().has_pending_conflicts(),
            "Handler must record the conflict report"
        );
    }

    #[tokio::test]
    async fn handle_multiple_agents_tracked_separately() {
        let mut handler = AgentHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        // Agent 1 starts.
        let event1 = AppEvent::Agent(AgentEvent::Progress {
            agent_id: "agent-1".to_string(),
            agent_name: "explorer".to_string(),
            progress: AgentProgress::IterationStarted {
                iteration: 1,
                max: 10,
            },
        });
        let _ = handler.handle(&event1, &mut ctx).await.unwrap();

        // Agent 2 starts.
        let event2 = AppEvent::Agent(AgentEvent::Progress {
            agent_id: "agent-2".to_string(),
            agent_name: "reviewer".to_string(),
            progress: AgentProgress::IterationStarted {
                iteration: 1,
                max: 5,
            },
        });
        let _ = handler.handle(&event2, &mut ctx).await.unwrap();

        assert_eq!(ctx.state.agent_panel().entries().len(), 2);
        assert_eq!(ctx.state.agent_panel().entries()[0].agent_id, "agent-1");
        assert_eq!(ctx.state.agent_panel().entries()[1].agent_id, "agent-2");
    }

    #[tokio::test]
    async fn handle_progress_marks_dirty_for_render() {
        let mut handler = AgentHandler;
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // Clear dirty flags.
        state.mark_rendered();

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Agent(AgentEvent::Progress {
            agent_id: "agent-1".to_string(),
            agent_name: "explorer".to_string(),
            progress: AgentProgress::IterationStarted {
                iteration: 1,
                max: 10,
            },
        });
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(
            ctx.needs_render(),
            "Agent progress must mark display dirty for re-render"
        );
    }

    // =========================================================================
    // Send bound verification
    // =========================================================================

    #[test]
    fn agent_handler_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<AgentHandler>();
    }

    // =========================================================================
    // Integration with EventDispatcher
    // =========================================================================

    #[tokio::test]
    async fn agent_handler_works_in_dispatcher() {
        use crate::app::dispatch::EventDispatcher;

        let handler = AgentHandler;
        let mut dispatcher = EventDispatcher::new(vec![Box::new(handler)]);
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        // Agent event should be consumed.
        let event = AppEvent::Agent(AgentEvent::Progress {
            agent_id: "a1".to_string(),
            agent_name: "test".to_string(),
            progress: AgentProgress::IterationStarted {
                iteration: 1,
                max: 5,
            },
        });
        let result = dispatcher.dispatch(&event, &mut ctx).await.unwrap();
        assert_eq!(result, Handled::CONSUMED);

        // Key event should pass through.
        let key = AppEvent::Key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        let result = dispatcher.dispatch(&key, &mut ctx).await.unwrap();
        assert_eq!(result, Handled::IGNORED);
    }

    // =========================================================================
    // Narrow tests: handle_agent_event with AgentPanelState only (no AppState)
    // =========================================================================

    #[test]
    fn handle_agent_event_progress_creates_entry() {
        let mut panel = AgentPanelState::new(None);
        assert!(panel.entries().is_empty());

        let event = AgentEvent::Progress {
            agent_id: "agent-1".to_string(),
            agent_name: "explorer".to_string(),
            progress: AgentProgress::IterationStarted {
                iteration: 1,
                max: 10,
            },
        };
        let result = AgentHandler::handle_agent_event(&event, &mut panel).unwrap();

        assert_eq!(result, Handled::CONSUMED);
        assert_eq!(panel.entries().len(), 1);
        assert_eq!(panel.entries()[0].agent_id, "agent-1");
        assert_eq!(
            panel.entries()[0].status,
            AgentPanelStatus::Running {
                iteration: 1,
                max_iterations: 10,
            }
        );
    }

    #[test]
    fn handle_agent_event_conflict_adds_report() {
        let mut panel = AgentPanelState::new(None);
        assert!(!panel.has_pending_conflicts());

        let event = AgentEvent::ConflictDetected {
            report: ConflictReport::empty(),
        };
        let result = AgentHandler::handle_agent_event(&event, &mut panel).unwrap();

        assert_eq!(result, Handled::CONSUMED);
        assert!(panel.has_pending_conflicts());
    }

    #[test]
    fn handle_agent_event_sets_dirty_flag() {
        let mut panel = AgentPanelState::new(None);
        panel.mark_clean();
        assert!(!panel.is_dirty());

        let event = AgentEvent::Progress {
            agent_id: "agent-1".to_string(),
            agent_name: "explorer".to_string(),
            progress: AgentProgress::IterationStarted {
                iteration: 1,
                max: 5,
            },
        };
        let _ = AgentHandler::handle_agent_event(&event, &mut panel).unwrap();

        assert!(panel.is_dirty());
    }
}
