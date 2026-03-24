use crate::api::StreamEvent;

/// Events received from background tasks (API streaming or tool execution).
///
/// Used by `recv_background_event()` to return events from multiple channels
/// without borrow checker conflicts.
#[derive(Debug)]
pub enum BackgroundEvent {
    /// An API streaming chunk was received.
    ApiChunk(StreamEvent),
    /// A tool execution completed with its result.
    ToolResult(String, crate::types::ToolResultBlock),
}

/// Status of the continuous coding loop as displayed in the TUI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContinuousLoopStatus {
    /// No continuous loop is running.
    Inactive,
    /// Loop is actively running at the given iteration.
    Running {
        /// Current iteration (1-indexed).
        iteration: u32,
    },
    /// Stagnation has been detected.
    Stagnated {
        /// Number of iterations without progress.
        iterations_without_progress: u32,
        /// The threshold that was exceeded.
        threshold: u32,
    },
    /// Human intervention is required.
    HumanRequired {
        /// Reason why human intervention is needed.
        reason: String,
    },
}

/// Result of a single quality gate check for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateResult {
    /// Name of the quality gate.
    pub gate: String,
    /// Whether the gate passed.
    pub passed: bool,
    /// Optional detail message.
    pub message: Option<String>,
}

/// Status of an agent as displayed in the TUI agent panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentPanelStatus {
    /// Agent is actively running.
    Running {
        /// Current iteration (1-indexed).
        iteration: usize,
        /// Maximum iterations allowed.
        max_iterations: usize,
    },
    /// Agent completed successfully.
    Completed {
        /// Number of iterations used.
        iterations_used: usize,
    },
    /// Agent failed with an error.
    Failed {
        /// Error description.
        error: String,
    },
}

/// An entry in the TUI agent panel showing an agent's current state.
#[derive(Debug, Clone)]
pub struct AgentPanelEntry {
    /// Unique identifier for the agent instance.
    pub agent_id: String,
    /// Human-readable agent name.
    pub agent_name: String,
    /// Current status of the agent.
    pub status: AgentPanelStatus,
    /// Most recent content snippet from the agent.
    pub last_content: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gate_result_construction() {
        let gate = GateResult {
            gate: "clippy".to_string(),
            passed: true,
            message: Some("0 warnings".to_string()),
        };
        assert_eq!(gate.gate, "clippy");
        assert!(gate.passed);
        assert_eq!(gate.message.as_deref(), Some("0 warnings"));
    }

    #[test]
    fn test_agent_panel_status_variants() {
        let running = AgentPanelStatus::Running {
            iteration: 3,
            max_iterations: 10,
        };
        let completed = AgentPanelStatus::Completed { iterations_used: 5 };
        let failed = AgentPanelStatus::Failed {
            error: "timeout".to_string(),
        };

        assert_eq!(running, running.clone());
        assert_ne!(running, completed);
        assert_ne!(completed, failed);
    }

    #[test]
    fn test_agent_panel_entry_construction() {
        let entry = AgentPanelEntry {
            agent_id: "agent-1".to_string(),
            agent_name: "explorer".to_string(),
            status: AgentPanelStatus::Running {
                iteration: 1,
                max_iterations: 50,
            },
            last_content: "Searching...".to_string(),
        };
        assert_eq!(entry.agent_id, "agent-1");
        assert_eq!(entry.agent_name, "explorer");
        assert_eq!(entry.last_content, "Searching...");
    }

    #[test]
    fn test_continuous_loop_status_variants() {
        let inactive = ContinuousLoopStatus::Inactive;
        let running = ContinuousLoopStatus::Running { iteration: 3 };
        let stagnated = ContinuousLoopStatus::Stagnated {
            iterations_without_progress: 5,
            threshold: 3,
        };
        let human = ContinuousLoopStatus::HumanRequired {
            reason: "conflict".to_string(),
        };

        assert_eq!(inactive, ContinuousLoopStatus::Inactive);
        assert_ne!(inactive, running);
        assert_ne!(running, stagnated);
        assert_ne!(stagnated, human);
    }
}
