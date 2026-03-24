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
