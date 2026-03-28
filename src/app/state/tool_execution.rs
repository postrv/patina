use crate::app::tool_loop::{ContinuationData, ToolLoop, ToolLoopState};
use crate::permissions::{PermissionManager, PermissionRequest, PermissionResponse};
use crate::tools::HookedToolExecutor;
use crate::types::content::StopReason;
use crate::types::ui_state::{PlanState, QuestionState, ToolBlockState};
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

use super::background_tasks::BackgroundTaskRegistry;

/// Tool execution state extracted from AppState.
///
/// Groups all tool-related fields: the tool loop state machine, executor,
/// permission management, UI tool blocks, async result channels,
/// in-flight tracking, tool interception state (plan/question modals),
/// and background task management.
///
/// Methods on this struct operate exclusively on tool-related data, making
/// `ToolExecutionState` independently testable. `AppState` retains thin
/// wrappers for dirty flag management and cross-cutting orchestration.
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

    /// Pending plan awaiting user review (plan tool intercept).
    pub(crate) pending_plan: Option<PlanState>,

    /// Pending question awaiting user response (ask_user tool intercept).
    pub(crate) pending_question: Option<QuestionState>,

    /// Registry for background bash tasks (run_in_background).
    pub(crate) background_tasks: BackgroundTaskRegistry,
}

impl ToolExecutionState {
    // ========================================================================
    // Tool Loop Accessors
    // ========================================================================

    /// Returns a reference to the tool loop state.
    #[must_use]
    pub fn tool_loop(&self) -> &ToolLoop {
        &self.tool_loop
    }

    /// Returns a mutable reference to the tool loop.
    pub fn tool_loop_mut(&mut self) -> &mut ToolLoop {
        &mut self.tool_loop
    }

    /// Returns the current tool loop state.
    #[must_use]
    pub fn tool_loop_state(&self) -> &ToolLoopState {
        self.tool_loop.state()
    }

    // ========================================================================
    // Permission Management
    // ========================================================================

    /// Returns the pending permission request, if any.
    #[must_use]
    pub fn pending_permission(&self) -> Option<&PermissionRequest> {
        self.pending_permission.as_ref()
    }

    /// Returns true if there's a pending permission prompt.
    #[must_use]
    pub fn has_pending_permission(&self) -> bool {
        self.pending_permission.is_some()
    }

    /// Sets a pending permission request.
    ///
    /// The UI should display this as a modal prompt.
    pub fn set_pending_permission(&mut self, request: PermissionRequest) {
        self.pending_permission = Some(request);
    }

    /// Clears the pending permission request.
    pub fn clear_pending_permission(&mut self) {
        self.pending_permission = None;
    }

    /// Handles a permission response from the user.
    ///
    /// Grants or denies permission for the pending tool call and
    /// updates the permission manager accordingly.
    pub async fn handle_permission_response(&mut self, response: PermissionResponse) {
        if let Some(request) = self.pending_permission.take() {
            let mut manager = self.permission_manager.lock().await;
            manager.handle_response(&request.tool_name, request.tool_input.as_deref(), response);
        }
    }

    // ========================================================================
    // Tool Use Stream Events
    // ========================================================================

    /// Handles a tool_use stream event.
    ///
    /// Routes the event to the tool loop state machine.
    pub fn handle_tool_use_start(&mut self, id: String, name: String, index: usize) {
        self.tool_loop.start_tool_use(index, id, name);
    }

    /// Handles a tool_use input delta.
    pub fn handle_tool_use_input_delta(&mut self, index: usize, partial_json: &str) {
        self.tool_loop.append_tool_input(index, partial_json);
    }

    /// Handles tool_use completion.
    ///
    /// # Errors
    ///
    /// Returns an error if the tool loop rejects the completion (e.g.,
    /// unknown index or invalid JSON).
    pub fn handle_tool_use_complete(&mut self, index: usize) -> Result<()> {
        self.tool_loop
            .complete_tool_use(index)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(())
    }

    /// Handles message completion with a stop reason.
    ///
    /// If the stop reason is `ToolUse`, transitions the tool loop to
    /// `PendingApproval` state.
    ///
    /// # Errors
    ///
    /// Returns an error if the state transition is invalid.
    pub fn handle_message_complete(&mut self, stop_reason: StopReason) -> Result<()> {
        self.tool_loop
            .message_complete(stop_reason)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(())
    }

    // ========================================================================
    // Tool Execution Lifecycle
    // ========================================================================

    /// Finishes tool execution and returns continuation data.
    ///
    /// The continuation data contains the messages needed to continue
    /// the conversation with Claude.
    ///
    /// # Errors
    ///
    /// Returns an error if the tool loop is not in the expected state.
    pub fn finish_tool_execution(&mut self) -> Result<ContinuationData> {
        self.tool_loop
            .finish_execution()
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// Approves all pending tools for execution.
    ///
    /// # Errors
    ///
    /// Returns an error if the tool loop is not in `PendingApproval` state.
    pub fn approve_all_tools(&mut self) -> Result<()> {
        self.tool_loop
            .approve_all()
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// Denies all pending tools.
    ///
    /// # Errors
    ///
    /// Returns an error if the tool loop is not in `PendingApproval` state.
    pub fn deny_all_tools(&mut self) -> Result<()> {
        self.tool_loop
            .deny_all()
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// Resets the tool loop to idle state and clears the pending permission.
    pub fn reset_tool_loop(&mut self) {
        self.tool_loop.reset();
        self.pending_permission = None;
    }

    /// Returns true if the tool loop is waiting for user action.
    #[must_use]
    pub fn tool_loop_needs_user_action(&self) -> bool {
        self.tool_loop.state().needs_user_action() || self.pending_permission.is_some()
    }

    /// Returns true if the tool loop is actively processing.
    #[must_use]
    pub fn tool_loop_is_active(&self) -> bool {
        self.tool_loop.state().is_active()
    }

    /// Adds a pending tool to the tool loop.
    pub fn add_pending_tool(&mut self, tool_use: crate::types::ToolUseBlock) {
        self.tool_loop.add_tool_use(tool_use);
    }

    // ========================================================================
    // Tool Block UI Methods
    // ========================================================================

    /// Starts a new tool block for UI display.
    ///
    /// Returns the index of the created block for later updates.
    pub fn start_tool_block(&mut self, tool_name: &str, tool_input: &str) -> usize {
        let block = ToolBlockState::new(tool_name, tool_input);
        self.tool_blocks.push(block);
        self.tool_blocks.len() - 1
    }

    /// Completes a tool block with its result.
    pub fn complete_tool_block(&mut self, index: usize, result: &str, is_error: bool) {
        if let Some(block) = self.tool_blocks.get_mut(index) {
            if is_error {
                block.set_error(result);
            } else {
                block.set_result(result);
            }
        }
    }

    /// Returns a slice of all tool blocks for rendering.
    #[must_use]
    pub fn tool_blocks(&self) -> &[ToolBlockState] {
        &self.tool_blocks
    }

    /// Clears all tool blocks.
    ///
    /// Call this when starting a new conversation turn.
    pub fn clear_tool_blocks(&mut self) {
        self.tool_blocks.clear();
    }

    /// Returns true if there are any tool blocks to display.
    #[must_use]
    pub fn has_tool_blocks(&self) -> bool {
        !self.tool_blocks.is_empty()
    }

    // ========================================================================
    // Async Tool Result Channel
    // ========================================================================

    /// Sets the receiver channel for async tool results.
    ///
    /// When tool execution is spawned in the background, results will be
    /// streamed back through this channel.
    pub fn set_tool_result_rx(
        &mut self,
        rx: mpsc::Receiver<(String, crate::types::ToolResultBlock)>,
    ) {
        self.tool_result_rx = Some(rx);
    }

    /// Returns true if a tool result channel is currently set.
    #[must_use]
    pub fn has_tool_result_rx(&self) -> bool {
        self.tool_result_rx.is_some()
    }

    /// Attempts to receive a tool result without blocking.
    ///
    /// Returns `Some((tool_id, result))` if a result is available,
    /// `None` if no result is ready or channel is not set.
    pub fn try_recv_tool_result(&mut self) -> Option<(String, crate::types::ToolResultBlock)> {
        if let Some(ref mut rx) = self.tool_result_rx {
            rx.try_recv().ok()
        } else {
            None
        }
    }

    /// Receives a tool result asynchronously.
    ///
    /// Returns `None` immediately if no channel is set, otherwise waits for
    /// the next result. This is designed for use in `tokio::select!`.
    ///
    /// # Returns
    ///
    /// - `Some((tool_id, result))` - A tool completed execution
    /// - `None` - Channel closed or no channel set
    pub async fn recv_tool_result(&mut self) -> Option<(String, crate::types::ToolResultBlock)> {
        match &mut self.tool_result_rx {
            Some(rx) => rx.recv().await,
            None => None, // Return immediately - don't block with pending()
        }
    }

    /// Clears the tool result channel.
    ///
    /// Called after all tools have completed and results have been processed.
    pub fn clear_tool_result_rx(&mut self) {
        self.tool_result_rx = None;
    }

    // ========================================================================
    // Executing Tool Tracking
    // ========================================================================

    /// Returns true if there are any tools currently executing.
    #[must_use]
    pub fn has_executing_tools(&self) -> bool {
        !self.executing_tool_ids.is_empty()
    }

    /// Marks a tool as currently executing.
    pub fn mark_tool_executing(&mut self, tool_id: &str) {
        self.executing_tool_ids.insert(tool_id.to_string());
    }

    /// Records a tool result in the tool loop and removes the tool from the executing set.
    ///
    /// This method handles the tool_state-local bookkeeping. The caller is
    /// responsible for updating timeline and dirty flags.
    pub fn record_tool_result(&mut self, tool_id: &str, result: crate::types::ToolResultBlock) {
        self.executing_tool_ids.remove(tool_id);
        if let Err(e) = self.tool_loop.set_tool_result(tool_id, result) {
            tracing::error!("Failed to set tool result for {tool_id}: {e}");
        }
    }

    /// Returns true if all pending tools have completed execution.
    #[must_use]
    pub fn all_tools_complete(&self) -> bool {
        self.executing_tool_ids.is_empty()
            && self
                .tool_loop
                .pending_calls()
                .values()
                .all(|call| call.executed || call.result.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Creates a fresh `ToolExecutionState` for testing.
    fn new_tool_state() -> ToolExecutionState {
        let hooks = crate::hooks::HookManager::new("test-session".to_string());
        let executor = HookedToolExecutor::new(PathBuf::from("/tmp"), hooks);
        let permission_manager = Arc::new(Mutex::new(PermissionManager::new()));
        ToolExecutionState {
            tool_loop: ToolLoop::new(),
            tool_executor: Arc::new(executor),
            permission_manager,
            pending_permission: None,
            tool_blocks: Vec::new(),
            tool_result_rx: None,
            executing_tool_ids: std::collections::HashSet::new(),
            pending_plan: None,
            pending_question: None,
            background_tasks: BackgroundTaskRegistry::new(),
        }
    }

    #[test]
    fn new_state_has_idle_tool_loop() {
        let state = new_tool_state();
        assert_eq!(state.tool_loop_state(), &ToolLoopState::Idle);
        assert!(!state.tool_loop_is_active());
        assert!(!state.has_pending_permission());
        assert!(!state.has_executing_tools());
        assert!(!state.has_tool_result_rx());
        assert!(state.tool_blocks().is_empty());
        assert!(!state.has_tool_blocks());
        assert!(state.all_tools_complete());
    }

    #[test]
    fn tool_block_start_and_complete() {
        let mut state = new_tool_state();

        let idx = state.start_tool_block("bash", "echo hello");
        assert_eq!(idx, 0);
        assert!(state.has_tool_blocks());
        assert_eq!(state.tool_blocks().len(), 1);
        assert_eq!(state.tool_blocks()[0].tool_name(), "bash");

        state.complete_tool_block(idx, "hello", false);
        assert!(state.tool_blocks()[0].is_complete());

        state.clear_tool_blocks();
        assert!(state.tool_blocks().is_empty());
        assert!(!state.has_tool_blocks());
    }

    #[test]
    fn tool_loop_accessors_reflect_state() {
        let mut state = new_tool_state();

        // Start streaming transitions away from Idle
        state.tool_loop_mut().start_streaming().unwrap();
        assert!(state.tool_loop_is_active());
        assert_eq!(state.tool_loop_state(), &ToolLoopState::Streaming);

        // Reset goes back to Idle
        state.reset_tool_loop();
        assert_eq!(state.tool_loop_state(), &ToolLoopState::Idle);
        assert!(!state.tool_loop_is_active());
    }

    #[test]
    fn pending_permission_management() {
        let mut state = new_tool_state();

        assert!(!state.has_pending_permission());
        assert!(state.pending_permission().is_none());

        let request =
            PermissionRequest::new("bash", Some(r#"{"command":"rm -rf /"}"#), "test permission");
        state.set_pending_permission(request);
        assert!(state.has_pending_permission());
        assert_eq!(state.pending_permission().unwrap().tool_name, "bash");

        // Needs user action when there's a pending permission
        assert!(state.tool_loop_needs_user_action());

        state.clear_pending_permission();
        assert!(!state.has_pending_permission());
    }

    #[test]
    fn executing_tool_tracking() {
        let mut state = new_tool_state();

        assert!(!state.has_executing_tools());
        assert!(state.all_tools_complete());

        state.mark_tool_executing("tool_1");
        assert!(state.has_executing_tools());
        assert!(!state.all_tools_complete());

        // record_tool_result removes from executing set
        let result = crate::types::ToolResultBlock {
            tool_use_id: "tool_1".to_string(),
            content: "done".to_string(),
            is_error: false,
        };
        state.record_tool_result("tool_1", result);
        assert!(!state.has_executing_tools());
        assert!(state.all_tools_complete());
    }

    #[test]
    fn record_tool_result_for_unknown_tool_does_not_panic() {
        let mut state = new_tool_state();

        // Recording a result for a tool_id that was never registered should
        // not panic; the error is logged at error level (S-10).
        let result = crate::types::ToolResultBlock {
            tool_use_id: "nonexistent_tool".to_string(),
            content: "result".to_string(),
            is_error: false,
        };
        // This should not panic even though the tool was never added
        state.record_tool_result("nonexistent_tool", result);
    }
}
