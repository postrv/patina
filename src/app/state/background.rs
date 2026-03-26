use crate::api::StreamEvent;

// Re-export shared UI types from their canonical location for backward compatibility.
pub use crate::types::ui_state::{
    AgentPanelEntry, AgentPanelStatus, ContinuousLoopStatus, GateResult,
};

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
