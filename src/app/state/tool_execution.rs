use crate::app::tool_loop::ToolLoop;
use crate::permissions::{PermissionManager, PermissionRequest};
use crate::tools::HookedToolExecutor;
use crate::tui::widgets::ToolBlockState;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// Tool execution state extracted from AppState.
///
/// Groups all tool-related fields: the tool loop state machine, executor,
/// permission management, UI tool blocks, async result channels, and
/// in-flight tracking. This struct owns no behavior — all methods live
/// on `AppState` and delegate through `self.tool_state`.
pub struct ToolExecutionState {
    /// State machine coordinating tool approval, execution, and continuation.
    pub(crate) tool_loop: ToolLoop,

    /// Shared tool executor with hook integration and parallel support.
    pub(crate) tool_executor: Arc<HookedToolExecutor>,

    /// Thread-safe permission manager for tool approval decisions.
    pub(crate) permission_manager: Arc<Mutex<PermissionManager>>,

    /// Current permission request awaiting user response, if any.
    pub(crate) pending_permission: Option<PermissionRequest>,

    /// Tool blocks for UI display.
    /// Each block represents a tool execution with its name, input, and result.
    pub(crate) tool_blocks: Vec<ToolBlockState>,

    /// Channel receiver for async tool results.
    /// When set, tool execution runs in the background and results
    /// are streamed back through this channel.
    pub(crate) tool_result_rx: Option<mpsc::Receiver<(String, crate::types::ToolResultBlock)>>,

    /// Set of tool IDs currently being executed.
    /// Used to track which tools are in-flight for progress display.
    pub(crate) executing_tool_ids: std::collections::HashSet<String>,
}
