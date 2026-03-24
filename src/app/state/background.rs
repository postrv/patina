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
