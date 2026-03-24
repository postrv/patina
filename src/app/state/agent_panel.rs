use super::background::{AgentPanelEntry, AgentPanelStatus};
use crate::agents::{AgentProgress, ConflictReport, SubagentSpawner};

/// Agent panel state extracted from AppState.
///
/// Groups the subagent spawner, agent panel entries for TUI display,
/// and pending conflict reports.
pub struct AgentPanelState {
    /// Optional subagent spawner for creating subagent sessions.
    spawner: Option<SubagentSpawner>,
    /// Agent panel entries for the TUI agent status display.
    entries: Vec<AgentPanelEntry>,
    /// Pending conflict reports from cross-agent conflict detection.
    pending_conflicts: Vec<ConflictReport>,
}

impl AgentPanelState {
    /// Creates a new `AgentPanelState` with the given spawner.
    #[must_use]
    pub fn new(spawner: Option<SubagentSpawner>) -> Self {
        Self {
            spawner,
            entries: Vec::new(),
            pending_conflicts: Vec::new(),
        }
    }

    /// Returns whether subagent orchestration is enabled.
    #[must_use]
    pub fn subagents_enabled(&self) -> bool {
        self.spawner.is_some()
    }

    /// Returns a reference to the subagent spawner if enabled.
    #[must_use]
    pub fn spawner(&self) -> Option<&SubagentSpawner> {
        self.spawner.as_ref()
    }

    /// Returns the current agent panel entries for TUI rendering.
    #[must_use]
    pub fn entries(&self) -> &[AgentPanelEntry] {
        &self.entries
    }

    /// Updates the agent panel with a progress event.
    ///
    /// If an entry for the given `agent_id` exists, it is updated in place.
    /// Otherwise, a new entry is created. Returns `true` if state changed.
    pub fn update_progress(&mut self, agent_id: &str, agent_name: &str, progress: &AgentProgress) {
        let status = match progress {
            AgentProgress::IterationStarted { iteration, max } => AgentPanelStatus::Running {
                iteration: *iteration,
                max_iterations: *max,
            },
            AgentProgress::ContentDelta(_) => {
                // Keep existing status for content deltas; just update last_content.
                if let Some(entry) = self.entries.iter_mut().find(|e| e.agent_id == agent_id) {
                    if let AgentProgress::ContentDelta(text) = progress {
                        entry.last_content = text.chars().take(80).collect();
                    }
                    return;
                }
                // First event for this agent; default to iteration 0.
                AgentPanelStatus::Running {
                    iteration: 0,
                    max_iterations: 0,
                }
            }
            AgentProgress::Completed {
                iterations_used, ..
            } => AgentPanelStatus::Completed {
                iterations_used: *iterations_used,
            },
            AgentProgress::Failed { error, .. } => AgentPanelStatus::Failed {
                error: error.clone(),
            },
        };

        if let Some(entry) = self.entries.iter_mut().find(|e| e.agent_id == agent_id) {
            entry.status = status;
            if let AgentProgress::ContentDelta(text) = progress {
                entry.last_content = text.chars().take(80).collect();
            }
            if let AgentProgress::Completed { output, .. } = progress {
                entry.last_content = output.chars().take(80).collect();
            }
        } else {
            let last_content = match progress {
                AgentProgress::ContentDelta(text) => text.chars().take(80).collect(),
                AgentProgress::Completed { output, .. } => output.chars().take(80).collect(),
                _ => String::new(),
            };
            self.entries.push(AgentPanelEntry {
                agent_id: agent_id.to_string(),
                agent_name: agent_name.to_string(),
                status,
                last_content,
            });
        }
    }

    /// Records a conflict report.
    pub fn add_conflict(&mut self, report: ConflictReport) {
        self.pending_conflicts.push(report);
    }

    /// Takes all pending conflict reports, leaving the internal list empty.
    pub fn take_conflicts(&mut self) -> Vec<ConflictReport> {
        std::mem::take(&mut self.pending_conflicts)
    }

    /// Returns `true` if there are pending conflict reports.
    #[must_use]
    pub fn has_pending_conflicts(&self) -> bool {
        !self.pending_conflicts.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create an `AgentPanelState` with no spawner.
    fn state() -> AgentPanelState {
        AgentPanelState::new(None)
    }

    #[test]
    fn test_update_progress_content_delta_existing() {
        let mut s = state();
        // Seed the agent with an IterationStarted so it already exists.
        let iter = AgentProgress::IterationStarted {
            iteration: 1,
            max: 5,
        };
        s.update_progress("a1", "Agent-1", &iter);

        // Now send a ContentDelta — should update last_content.
        let delta = AgentProgress::ContentDelta("hello world".to_string());
        s.update_progress("a1", "Agent-1", &delta);

        assert_eq!(s.entries().len(), 1);
        assert_eq!(s.entries()[0].last_content, "hello world");
        // Status must remain Running from the earlier IterationStarted.
        assert_eq!(
            s.entries()[0].status,
            AgentPanelStatus::Running {
                iteration: 1,
                max_iterations: 5,
            }
        );
    }

    #[test]
    fn test_update_progress_content_delta_new() {
        let mut s = state();
        let delta = AgentProgress::ContentDelta("first message".to_string());
        s.update_progress("new-agent", "NewAgent", &delta);

        assert_eq!(s.entries().len(), 1);
        let entry = &s.entries()[0];
        assert_eq!(entry.agent_id, "new-agent");
        assert_eq!(entry.agent_name, "NewAgent");
        assert_eq!(entry.last_content, "first message");
        // Default status for brand-new agent on ContentDelta.
        assert_eq!(
            entry.status,
            AgentPanelStatus::Running {
                iteration: 0,
                max_iterations: 0,
            }
        );
    }

    #[test]
    fn test_update_progress_completed() {
        let mut s = state();
        let completed = AgentProgress::Completed {
            output: "done".to_string(),
            iterations_used: 3,
        };
        s.update_progress("a1", "Agent-1", &completed);

        assert_eq!(s.entries().len(), 1);
        let entry = &s.entries()[0];
        assert_eq!(
            entry.status,
            AgentPanelStatus::Completed { iterations_used: 3 }
        );
        assert_eq!(entry.last_content, "done");
    }

    #[test]
    fn test_update_progress_failed() {
        let mut s = state();
        let failed = AgentProgress::Failed {
            error: "timeout".to_string(),
            iterations_used: 2,
        };
        s.update_progress("a1", "Agent-1", &failed);

        assert_eq!(s.entries().len(), 1);
        let entry = &s.entries()[0];
        assert_eq!(
            entry.status,
            AgentPanelStatus::Failed {
                error: "timeout".to_string(),
            }
        );
    }

    #[test]
    fn test_content_delta_truncates_at_80() {
        let mut s = state();
        let long_text = "x".repeat(200);
        let delta = AgentProgress::ContentDelta(long_text);
        s.update_progress("a1", "Agent-1", &delta);

        assert_eq!(s.entries()[0].last_content.len(), 80);
    }

    #[test]
    fn test_completed_truncates_output() {
        let mut s = state();
        let long_output = "y".repeat(200);
        let completed = AgentProgress::Completed {
            output: long_output,
            iterations_used: 1,
        };
        s.update_progress("a1", "Agent-1", &completed);

        assert_eq!(s.entries()[0].last_content.len(), 80);
    }

    #[test]
    fn test_multiple_agents_tracked() {
        let mut s = state();
        for i in 0..3 {
            let id = format!("agent-{i}");
            let name = format!("Agent {i}");
            let iter = AgentProgress::IterationStarted {
                iteration: i + 1,
                max: 10,
            };
            s.update_progress(&id, &name, &iter);
        }
        assert_eq!(s.entries().len(), 3);

        // Update the middle agent only.
        let delta = AgentProgress::ContentDelta("middle update".to_string());
        s.update_progress("agent-1", "Agent 1", &delta);

        assert_eq!(s.entries()[1].last_content, "middle update");
        // Other agents' content must be unchanged.
        assert_eq!(s.entries()[0].last_content, "");
        assert_eq!(s.entries()[2].last_content, "");
    }

    #[test]
    fn test_completed_updates_last_content() {
        let mut s = state();
        // Seed with an iteration event.
        let iter = AgentProgress::IterationStarted {
            iteration: 1,
            max: 5,
        };
        s.update_progress("a1", "Agent-1", &iter);
        assert_eq!(s.entries()[0].last_content, "");

        // Now complete — last_content should come from the Completed output.
        let completed = AgentProgress::Completed {
            output: "final result".to_string(),
            iterations_used: 2,
        };
        s.update_progress("a1", "Agent-1", &completed);

        assert_eq!(s.entries()[0].last_content, "final result");
        assert_eq!(
            s.entries()[0].status,
            AgentPanelStatus::Completed { iterations_used: 2 }
        );
    }
}
