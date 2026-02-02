//! Agentic tool execution loop.
//!
//! This module implements the state machine that handles Claude's tool_use
//! requests. When Claude wants to use a tool, the loop:
//!
//! 1. Collects all tool_use blocks from the response
//! 2. Executes each tool
//! 3. Sends tool_result blocks back to Claude
//! 4. Continues until Claude stops requesting tools
//!
//! # State Machine
//!
//! ```text
//! ┌───────┐
//! │ Idle  │ ←────────────────────────────────┐
//! └───┬───┘                                  │
//!     │ start_streaming()                    │
//!     ▼                                      │
//! ┌───────────┐                              │
//! │ Streaming │ ──────────────────────────────┤ stop_reason: end_turn
//! └─────┬─────┘                              │
//!       │ stop_reason: tool_use              │
//!       ▼                                    │
//! ┌────────────────┐                         │
//! │ PendingApproval│ ────── user denies ─────┤
//! └───────┬────────┘                         │
//!         │ user approves                    │
//!         ▼                                  │
//! ┌───────────┐                              │
//! │ Executing │                              │
//! └─────┬─────┘                              │
//!       │ execution complete                 │
//!       ▼                                    │
//! ┌───────────┐                              │
//! │ Continuing│ ─────────────────────────────┘
//! └───────────┘
//! ```

use std::collections::HashMap;

use crate::narsil::SecurityVerdict;
use crate::types::content::{ContentBlock, StopReason, ToolResultBlock, ToolUseBlock};
use crate::types::stream::ToolUseAccumulator;

/// State of the agentic tool execution loop.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ToolLoopState {
    /// No active tool loop. Waiting for user input.
    #[default]
    Idle,

    /// Streaming a response from Claude.
    /// Collecting text and tool_use blocks.
    Streaming,

    /// Received tool_use stop_reason. Waiting for user approval.
    /// Contains the tool calls that need approval.
    PendingApproval,

    /// Executing approved tools.
    Executing,

    /// Tools executed. Sending results back to Claude.
    Continuing,

    /// An error occurred during the loop.
    Error(String),
}

impl ToolLoopState {
    /// Returns true if the loop is in a terminal state.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Idle | Self::Error(_))
    }

    /// Returns true if the loop is waiting for user action.
    #[must_use]
    pub fn needs_user_action(&self) -> bool {
        matches!(self, Self::Idle | Self::PendingApproval)
    }

    /// Returns true if the loop is actively processing.
    #[must_use]
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Streaming | Self::Executing | Self::Continuing)
    }
}

/// A pending tool call waiting for execution or approval.
#[derive(Debug, Clone)]
pub struct PendingToolCall {
    /// The tool_use block from Claude.
    pub tool_use: ToolUseBlock,
    /// Whether this tool has been approved for execution.
    pub approved: bool,
    /// Whether this tool has been executed.
    pub executed: bool,
    /// The result of execution (if executed).
    pub result: Option<ToolResultBlock>,
    /// Security pre-flight verdict (if security check was performed).
    pub security_verdict: Option<SecurityVerdict>,
}

impl PendingToolCall {
    /// Creates a new pending tool call.
    #[must_use]
    pub fn new(tool_use: ToolUseBlock) -> Self {
        Self {
            tool_use,
            approved: false,
            executed: false,
            result: None,
            security_verdict: None,
        }
    }

    /// Marks this tool call as approved.
    pub fn approve(&mut self) {
        self.approved = true;
    }

    /// Sets the execution result.
    pub fn set_result(&mut self, result: ToolResultBlock) {
        self.executed = true;
        self.result = Some(result);
    }

    /// Sets the security verdict from pre-flight check.
    pub fn set_security_verdict(&mut self, verdict: SecurityVerdict) {
        self.security_verdict = Some(verdict);
    }

    /// Returns the security verdict if one has been set.
    #[must_use]
    pub fn security_verdict(&self) -> Option<&SecurityVerdict> {
        self.security_verdict.as_ref()
    }

    /// Returns true if the security verdict blocks execution.
    #[must_use]
    pub fn is_security_blocked(&self) -> bool {
        self.security_verdict
            .as_ref()
            .is_some_and(|v| v.blocks_execution())
    }

    /// Returns the security warning reason if the verdict is a warning.
    #[must_use]
    pub fn security_warning(&self) -> Option<&str> {
        self.security_verdict.as_ref().and_then(|v| {
            if v.has_warning() {
                v.reason()
            } else {
                None
            }
        })
    }
}

/// The agentic tool loop state machine.
///
/// Manages the state of tool execution during a conversation with Claude.
/// Tracks pending tool calls, accumulates streaming tool_use inputs, and
/// coordinates the execution flow.
#[derive(Debug, Default)]
pub struct ToolLoop {
    /// Current state of the loop.
    state: ToolLoopState,

    /// Tool calls pending execution.
    /// Key is the tool_use ID.
    pending_calls: HashMap<String, PendingToolCall>,

    /// Accumulators for streaming tool_use inputs.
    /// Key is the content block index.
    accumulators: HashMap<usize, ToolUseAccumulator>,

    /// Text content accumulated during streaming.
    text_content: String,

    /// Stop reason from the most recent message.
    stop_reason: Option<StopReason>,

    /// Maximum number of loop iterations before stopping.
    /// Prevents infinite loops.
    max_iterations: usize,

    /// Current iteration count.
    iteration: usize,
}

impl ToolLoop {
    /// Creates a new tool loop with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            max_iterations: 50,
            ..Default::default()
        }
    }

    /// Creates a new tool loop with a custom iteration limit.
    #[must_use]
    pub fn with_max_iterations(max_iterations: usize) -> Self {
        Self {
            max_iterations,
            ..Default::default()
        }
    }

    /// Returns the current state of the loop.
    #[must_use]
    pub fn state(&self) -> &ToolLoopState {
        &self.state
    }

    /// Returns the accumulated text content.
    #[must_use]
    pub fn text_content(&self) -> &str {
        &self.text_content
    }

    /// Returns the pending tool calls.
    #[must_use]
    pub fn pending_calls(&self) -> &HashMap<String, PendingToolCall> {
        &self.pending_calls
    }

    /// Returns the stop reason from the last message.
    #[must_use]
    pub fn stop_reason(&self) -> Option<StopReason> {
        self.stop_reason
    }

    /// Returns true if completing the current iteration would exceed the limit.
    ///
    /// With max_iterations=2, this returns true when iteration=1 (allowing iterations 0 and 1).
    #[must_use]
    pub fn is_at_limit(&self) -> bool {
        self.iteration + 1 >= self.max_iterations
    }

    /// Returns the current iteration count.
    #[must_use]
    pub fn iteration(&self) -> usize {
        self.iteration
    }

    // =========================================================================
    // State Transitions
    // =========================================================================

    /// Starts a new streaming response.
    ///
    /// Transitions from Idle to Streaming.
    ///
    /// # Errors
    ///
    /// Returns an error if not in Idle state.
    pub fn start_streaming(&mut self) -> Result<(), ToolLoopError> {
        match &self.state {
            ToolLoopState::Idle | ToolLoopState::Continuing => {
                self.state = ToolLoopState::Streaming;
                self.text_content.clear();
                self.accumulators.clear();
                self.stop_reason = None;
                Ok(())
            }
            _ => Err(ToolLoopError::InvalidStateTransition {
                from: format!("{:?}", self.state),
                to: "Streaming".to_string(),
            }),
        }
    }

    /// Handles a text content delta.
    pub fn append_text(&mut self, text: &str) {
        if matches!(self.state, ToolLoopState::Streaming) {
            self.text_content.push_str(text);
        }
    }

    /// Handles a tool_use start event.
    pub fn start_tool_use(&mut self, index: usize, id: String, name: String) {
        if matches!(self.state, ToolLoopState::Streaming) {
            let mut acc = ToolUseAccumulator::new();
            acc.start(id, name);
            self.accumulators.insert(index, acc);
        }
    }

    /// Handles a tool_use input delta.
    pub fn append_tool_input(&mut self, index: usize, partial_json: &str) {
        if matches!(self.state, ToolLoopState::Streaming) {
            if let Some(acc) = self.accumulators.get_mut(&index) {
                acc.append_input(partial_json);
            }
        }
    }

    /// Handles a tool_use complete event.
    ///
    /// Parses the accumulated JSON and creates a pending tool call.
    pub fn complete_tool_use(&mut self, index: usize) -> Result<(), ToolLoopError> {
        if !matches!(self.state, ToolLoopState::Streaming) {
            return Ok(());
        }

        if let Some(mut acc) = self.accumulators.remove(&index) {
            // Take ownership of id and name before calling parse_input
            let id = acc.id.take().ok_or(ToolLoopError::MissingToolId)?;
            let name = acc.name.take().ok_or(ToolLoopError::MissingToolName)?;

            let input = acc
                .parse_input()
                .map_err(|e| ToolLoopError::InvalidToolInput {
                    tool_id: id.clone(),
                    error: e.to_string(),
                })?;

            let tool_use = ToolUseBlock::new(id.clone(), name, input);
            self.pending_calls
                .insert(id, PendingToolCall::new(tool_use));
        }

        Ok(())
    }

    /// Directly adds a tool use block to pending calls.
    ///
    /// This is useful for testing and manual tool injection.
    /// The tool is added in unapproved state.
    ///
    /// # Arguments
    ///
    /// * `tool_use` - The tool use block to add
    pub fn add_tool_use(&mut self, tool_use: ToolUseBlock) {
        let id = tool_use.id.clone();
        self.pending_calls
            .insert(id, PendingToolCall::new(tool_use));
    }

    /// Handles message completion with a stop reason.
    ///
    /// Transitions based on the stop reason:
    /// - `ToolUse` -> PendingApproval (if there are tool calls)
    /// - `EndTurn` -> Idle
    /// - `MaxTokens` -> Idle (with truncation warning)
    pub fn message_complete(&mut self, stop_reason: StopReason) -> Result<(), ToolLoopError> {
        if !matches!(self.state, ToolLoopState::Streaming) {
            return Ok(());
        }

        self.stop_reason = Some(stop_reason);

        match stop_reason {
            StopReason::ToolUse => {
                if self.pending_calls.is_empty() {
                    // Got tool_use stop reason but no tool calls - shouldn't happen
                    self.state = ToolLoopState::Error(
                        "Received tool_use stop reason but no tool calls".to_string(),
                    );
                } else {
                    self.state = ToolLoopState::PendingApproval;
                }
            }
            StopReason::EndTurn | StopReason::StopSequence => {
                self.state = ToolLoopState::Idle;
                self.pending_calls.clear();
            }
            StopReason::MaxTokens => {
                self.state = ToolLoopState::Idle;
                self.pending_calls.clear();
                // Note: Could emit a warning about truncation here
            }
        }

        Ok(())
    }

    /// Approves all pending tool calls for execution.
    ///
    /// Transitions from PendingApproval to Executing.
    pub fn approve_all(&mut self) -> Result<(), ToolLoopError> {
        if !matches!(self.state, ToolLoopState::PendingApproval) {
            return Err(ToolLoopError::InvalidStateTransition {
                from: format!("{:?}", self.state),
                to: "Executing".to_string(),
            });
        }

        for call in self.pending_calls.values_mut() {
            call.approve();
        }

        self.state = ToolLoopState::Executing;
        Ok(())
    }

    /// Approves a specific tool call by ID.
    pub fn approve_tool(&mut self, tool_id: &str) -> Result<(), ToolLoopError> {
        if let Some(call) = self.pending_calls.get_mut(tool_id) {
            call.approve();
            Ok(())
        } else {
            Err(ToolLoopError::ToolNotFound(tool_id.to_string()))
        }
    }

    /// Denies all pending tool calls.
    ///
    /// Transitions from PendingApproval back to Idle.
    pub fn deny_all(&mut self) -> Result<(), ToolLoopError> {
        if !matches!(self.state, ToolLoopState::PendingApproval) {
            return Err(ToolLoopError::InvalidStateTransition {
                from: format!("{:?}", self.state),
                to: "Idle".to_string(),
            });
        }

        self.pending_calls.clear();
        self.state = ToolLoopState::Idle;
        Ok(())
    }

    /// Records a tool execution result.
    pub fn set_tool_result(
        &mut self,
        tool_id: &str,
        result: ToolResultBlock,
    ) -> Result<(), ToolLoopError> {
        if let Some(call) = self.pending_calls.get_mut(tool_id) {
            call.set_result(result);
            Ok(())
        } else {
            Err(ToolLoopError::ToolNotFound(tool_id.to_string()))
        }
    }

    /// Checks if all approved tools have been executed.
    #[must_use]
    pub fn all_tools_executed(&self) -> bool {
        self.pending_calls
            .values()
            .all(|c| !c.approved || c.executed)
    }

    /// Gets the tool calls that need to be executed.
    #[must_use]
    pub fn tools_to_execute(&self) -> Vec<&ToolUseBlock> {
        self.pending_calls
            .values()
            .filter(|c| c.approved && !c.executed)
            .map(|c| &c.tool_use)
            .collect()
    }

    /// Collects all tool results as content blocks.
    #[must_use]
    pub fn collect_tool_results(&self) -> Vec<ContentBlock> {
        self.pending_calls
            .values()
            .filter_map(|c| c.result.clone())
            .map(ContentBlock::ToolResult)
            .collect()
    }

    /// Transitions to Continuing state after all tools are executed.
    ///
    /// Returns a `ContinuationData` containing:
    /// - The tool results (to be sent back to Claude)
    /// - The assistant content (text + tool_use blocks)
    ///
    /// # Errors
    ///
    /// Returns an error if not in Executing state, tools are incomplete, or
    /// the iteration limit has been reached.
    pub fn finish_execution(&mut self) -> Result<ContinuationData, ToolLoopError> {
        if !matches!(self.state, ToolLoopState::Executing) {
            return Err(ToolLoopError::InvalidStateTransition {
                from: format!("{:?}", self.state),
                to: "Continuing".to_string(),
            });
        }

        if !self.all_tools_executed() {
            return Err(ToolLoopError::IncompleteExecution);
        }

        if self.is_at_limit() {
            self.state = ToolLoopState::Error(format!(
                "Reached maximum iteration limit ({})",
                self.max_iterations
            ));
            return Err(ToolLoopError::IterationLimitReached);
        }

        // Build continuation data BEFORE clearing state
        let tool_results = self.collect_tool_results();

        // Build assistant content: text + tool_use blocks
        let mut assistant_content: Vec<ContentBlock> = Vec::new();
        if !self.text_content.is_empty() {
            assistant_content.push(ContentBlock::text(&self.text_content));
        }
        for call in self.pending_calls.values() {
            if call.approved {
                assistant_content.push(ContentBlock::ToolUse(call.tool_use.clone()));
            }
        }

        let data = ContinuationData {
            assistant_content,
            tool_results,
        };

        // Now clear state
        self.pending_calls.clear();
        self.iteration += 1;
        self.state = ToolLoopState::Continuing;

        Ok(data)
    }

    /// Resets the loop to Idle state.
    pub fn reset(&mut self) {
        self.state = ToolLoopState::Idle;
        self.pending_calls.clear();
        self.accumulators.clear();
        self.text_content.clear();
        self.stop_reason = None;
        self.iteration = 0;
    }

    /// Collects all tool_use blocks from the pending calls.
    ///
    /// Useful for building the assistant message content.
    #[must_use]
    pub fn collect_tool_uses(&self) -> Vec<ContentBlock> {
        self.pending_calls
            .values()
            .filter(|c| c.approved)
            .map(|c| ContentBlock::ToolUse(c.tool_use.clone()))
            .collect()
    }

    // =========================================================================
    // Recovery Methods
    // =========================================================================

    /// Returns the error message if in Error state, None otherwise.
    #[must_use]
    pub fn error_message(&self) -> Option<&str> {
        match &self.state {
            ToolLoopState::Error(msg) => Some(msg),
            _ => None,
        }
    }

    /// Returns true if recovery from the current state is possible.
    ///
    /// Recovery is possible from Error state and PendingApproval state.
    #[must_use]
    pub fn can_recover(&self) -> bool {
        matches!(
            self.state,
            ToolLoopState::Error(_) | ToolLoopState::PendingApproval
        )
    }

    /// Recovers from Error state back to Idle.
    ///
    /// Unlike `reset()`, this preserves the iteration count, allowing
    /// the conversation to continue from where it left off.
    ///
    /// # Returns
    ///
    /// Returns the error message if recovery was successful,
    /// or an error if not in Error state.
    pub fn recover_from_error(&mut self) -> Result<String, ToolLoopError> {
        match &self.state {
            ToolLoopState::Error(msg) => {
                let error_msg = msg.clone();
                self.state = ToolLoopState::Idle;
                self.pending_calls.clear();
                self.accumulators.clear();
                self.text_content.clear();
                self.stop_reason = None;
                // Note: iteration count is preserved
                Ok(error_msg)
            }
            _ => Err(ToolLoopError::InvalidStateTransition {
                from: format!("{:?}", self.state),
                to: "Idle (recovery)".to_string(),
            }),
        }
    }

    /// Retries tool execution by transitioning from PendingApproval back to
    /// allow re-approval.
    ///
    /// This can be used when a tool approval was cancelled and the user
    /// wants to try again.
    ///
    /// # Returns
    ///
    /// Returns the number of pending tool calls that can be retried.
    pub fn retry_approval(&mut self) -> Result<usize, ToolLoopError> {
        match &self.state {
            ToolLoopState::PendingApproval => {
                // Reset approval status but keep the tool calls
                for call in self.pending_calls.values_mut() {
                    call.approved = false;
                    call.executed = false;
                    call.result = None;
                }
                Ok(self.pending_calls.len())
            }
            ToolLoopState::Executing => {
                // Allow retry from Executing if something went wrong
                self.state = ToolLoopState::PendingApproval;
                for call in self.pending_calls.values_mut() {
                    call.approved = false;
                    call.executed = false;
                    call.result = None;
                }
                Ok(self.pending_calls.len())
            }
            _ => Err(ToolLoopError::InvalidStateTransition {
                from: format!("{:?}", self.state),
                to: "PendingApproval (retry)".to_string(),
            }),
        }
    }

    /// Forces the state machine into a specific state.
    ///
    /// **Warning**: This bypasses normal state transition validation
    /// and should only be used for recovery scenarios or testing.
    ///
    /// # Safety
    ///
    /// This method can put the state machine into an inconsistent state
    /// if used incorrectly. Prefer using the normal transition methods
    /// when possible.
    pub fn force_state(&mut self, state: ToolLoopState) {
        self.state = state;
    }

    /// Creates a recovery snapshot of the current state.
    ///
    /// This can be used to restore state after a crash or interruption.
    #[must_use]
    pub fn snapshot(&self) -> ToolLoopSnapshot {
        ToolLoopSnapshot {
            state: self.state.clone(),
            pending_tool_ids: self.pending_calls.keys().cloned().collect(),
            text_content_len: self.text_content.len(),
            iteration: self.iteration,
            max_iterations: self.max_iterations,
        }
    }

    /// Restores state from a snapshot.
    ///
    /// Note: This only restores metadata, not the actual pending tool calls.
    /// It's primarily useful for continuing after a state recovery.
    pub fn restore_from_snapshot(&mut self, snapshot: &ToolLoopSnapshot) {
        self.iteration = snapshot.iteration;
        self.max_iterations = snapshot.max_iterations;
        // State is set to Idle after recovery - caller can resume as needed
        self.state = ToolLoopState::Idle;
    }

    // =========================================================================
    // Security Pre-Flight Methods
    // =========================================================================

    /// Sets the security verdict for a pending tool call.
    ///
    /// # Arguments
    ///
    /// * `tool_id` - The ID of the tool to set the verdict for
    /// * `verdict` - The security verdict from pre-flight check
    ///
    /// # Errors
    ///
    /// Returns an error if the tool ID is not found.
    pub fn set_security_verdict(
        &mut self,
        tool_id: &str,
        verdict: SecurityVerdict,
    ) -> Result<(), ToolLoopError> {
        if let Some(call) = self.pending_calls.get_mut(tool_id) {
            call.set_security_verdict(verdict);
            Ok(())
        } else {
            Err(ToolLoopError::ToolNotFound(tool_id.to_string()))
        }
    }

    /// Returns the tool IDs that are blocked by security verdicts.
    #[must_use]
    pub fn security_blocked_tools(&self) -> Vec<&str> {
        self.pending_calls
            .iter()
            .filter(|(_, call)| call.is_security_blocked())
            .map(|(id, _)| id.as_str())
            .collect()
    }

    /// Returns the tool IDs and warning reasons for tools with security warnings.
    #[must_use]
    pub fn security_warned_tools(&self) -> Vec<(&str, &str)> {
        self.pending_calls
            .iter()
            .filter_map(|(id, call)| {
                call.security_warning().map(|reason| (id.as_str(), reason))
            })
            .collect()
    }

    /// Returns true if any pending tools are blocked by security.
    #[must_use]
    pub fn has_security_blocks(&self) -> bool {
        self.pending_calls.values().any(|c| c.is_security_blocked())
    }

    /// Returns true if any pending tools have security warnings.
    #[must_use]
    pub fn has_security_warnings(&self) -> bool {
        self.pending_calls
            .values()
            .any(|c| c.security_warning().is_some())
    }

    /// Approves all pending tool calls that are not security blocked.
    ///
    /// Transitions from PendingApproval to Executing.
    /// Tools with `SecurityVerdict::Block` are skipped (not approved).
    /// Tools with `SecurityVerdict::Warn` or `SecurityVerdict::Allow` are approved.
    ///
    /// # Returns
    ///
    /// A list of tool IDs that were blocked and not approved.
    ///
    /// # Errors
    ///
    /// Returns an error if not in PendingApproval state.
    pub fn approve_safe(&mut self) -> Result<Vec<String>, ToolLoopError> {
        if !matches!(self.state, ToolLoopState::PendingApproval) {
            return Err(ToolLoopError::InvalidStateTransition {
                from: format!("{:?}", self.state),
                to: "Executing".to_string(),
            });
        }

        let mut blocked_ids = Vec::new();

        for (id, call) in self.pending_calls.iter_mut() {
            if call.is_security_blocked() {
                blocked_ids.push(id.clone());
            } else {
                call.approve();
            }
        }

        self.state = ToolLoopState::Executing;
        Ok(blocked_ids)
    }
}

/// Data needed to continue the conversation after tool execution.
///
/// Returned by `finish_execution()` and used by `build_continuation_messages()`.
#[derive(Debug, Clone)]
pub struct ContinuationData {
    /// Content blocks for the assistant message (text + tool_use blocks).
    pub assistant_content: Vec<ContentBlock>,
    /// Content blocks for the user message (tool_result blocks).
    pub tool_results: Vec<ContentBlock>,
}

impl ContinuationData {
    /// Builds the messages needed to continue the conversation.
    ///
    /// Returns a tuple of (assistant_message, user_message) as `ApiMessageV2`.
    #[must_use]
    pub fn build_messages(&self) -> (crate::types::ApiMessageV2, crate::types::ApiMessageV2) {
        use crate::types::{ApiMessageV2, MessageContent};

        let assistant_msg = ApiMessageV2::assistant_with_content(MessageContent::blocks(
            self.assistant_content.clone(),
        ));

        let user_msg =
            ApiMessageV2::user_with_content(MessageContent::blocks(self.tool_results.clone()));

        (assistant_msg, user_msg)
    }
}

/// A snapshot of tool loop state for recovery purposes.
///
/// This contains the minimal information needed to restore a tool loop
/// after an interruption or crash.
#[derive(Debug, Clone)]
pub struct ToolLoopSnapshot {
    /// The state at the time of the snapshot.
    pub state: ToolLoopState,
    /// IDs of pending tool calls (not the full data).
    pub pending_tool_ids: Vec<String>,
    /// Length of accumulated text content.
    pub text_content_len: usize,
    /// Current iteration count.
    pub iteration: usize,
    /// Maximum allowed iterations.
    pub max_iterations: usize,
}

impl ToolLoopSnapshot {
    /// Returns true if this snapshot represents an error state.
    #[must_use]
    pub fn is_error(&self) -> bool {
        matches!(self.state, ToolLoopState::Error(_))
    }

    /// Returns true if there were pending tool calls at snapshot time.
    #[must_use]
    pub fn has_pending_tools(&self) -> bool {
        !self.pending_tool_ids.is_empty()
    }

    /// Returns the error message if the snapshot represents an error state.
    #[must_use]
    pub fn error_message(&self) -> Option<&str> {
        match &self.state {
            ToolLoopState::Error(msg) => Some(msg),
            _ => None,
        }
    }
}

/// Errors that can occur during tool loop execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolLoopError {
    /// Invalid state transition attempted.
    InvalidStateTransition { from: String, to: String },
    /// Tool use block is missing its ID.
    MissingToolId,
    /// Tool use block is missing its name.
    MissingToolName,
    /// Tool input JSON is invalid.
    InvalidToolInput { tool_id: String, error: String },
    /// Referenced tool was not found.
    ToolNotFound(String),
    /// Tried to finish execution with unexecuted tools.
    IncompleteExecution,
    /// Reached the maximum iteration limit.
    IterationLimitReached,
}

impl std::fmt::Display for ToolLoopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidStateTransition { from, to } => {
                write!(f, "Invalid state transition from {} to {}", from, to)
            }
            Self::MissingToolId => write!(f, "Tool use block is missing ID"),
            Self::MissingToolName => write!(f, "Tool use block is missing name"),
            Self::InvalidToolInput { tool_id, error } => {
                write!(f, "Invalid tool input for {}: {}", tool_id, error)
            }
            Self::ToolNotFound(id) => write!(f, "Tool not found: {}", id),
            Self::IncompleteExecution => write!(f, "Cannot finish execution with unexecuted tools"),
            Self::IterationLimitReached => write!(f, "Tool loop iteration limit reached"),
        }
    }
}

impl std::error::Error for ToolLoopError {}

// =========================================================================
// Tool Execution Bridge
// =========================================================================

/// Converts a `ToolUseBlock` to a `tools::ToolCall`.
///
/// This bridges the API types to the executor types.
#[must_use]
pub fn tool_use_to_call(tool_use: &ToolUseBlock) -> crate::tools::ToolCall {
    crate::tools::ToolCall {
        name: tool_use.name.clone(),
        input: tool_use.input.clone(),
    }
}

/// Converts a `tools::ToolResult` to a `ToolResultBlock`.
///
/// Maps the executor result types to API content blocks:
/// - `Success(output)` → `ToolResultBlock::success(id, output)`
/// - `Error(msg)` → `ToolResultBlock::error(id, msg)`
/// - `Cancelled` → `ToolResultBlock::error(id, "Tool execution cancelled")`
/// - `NeedsPermission(_)` → Not converted (should be handled before execution)
#[must_use]
pub fn result_to_block(
    tool_use_id: &str,
    result: &crate::tools::ToolResult,
) -> Option<ToolResultBlock> {
    use crate::tools::ToolResult;

    match result {
        ToolResult::Success(output) => Some(ToolResultBlock::success(tool_use_id, output)),
        ToolResult::Error(error) => Some(ToolResultBlock::error(tool_use_id, error)),
        ToolResult::Cancelled => Some(ToolResultBlock::error(
            tool_use_id,
            "Tool execution cancelled",
        )),
        ToolResult::NeedsPermission(_) => None, // Should be handled at a higher level
    }
}

/// Error type for tool execution bridge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionError {
    /// The tool requires permission before execution.
    NeedsPermission(String),
    /// The tool execution failed.
    ExecutionFailed(String),
}

impl std::fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NeedsPermission(tool) => write!(f, "Tool '{}' requires permission", tool),
            Self::ExecutionFailed(msg) => write!(f, "Tool execution failed: {}", msg),
        }
    }
}

impl std::error::Error for ExecutionError {}

/// Executes a single tool and returns the result block.
///
/// This function bridges the gap between the tool loop's `ToolUseBlock` and
/// the `HookedToolExecutor`. It:
/// 1. Converts the `ToolUseBlock` to a `ToolCall`
/// 2. Executes via the provided executor
/// 3. Converts the result to a `ToolResultBlock`
///
/// # Arguments
///
/// * `tool_use` - The tool use block from Claude's response
/// * `executor` - The tool executor to run the tool
///
/// # Returns
///
/// - `Ok(ToolResultBlock)` on success or error results
/// - `Err(ExecutionError::NeedsPermission)` if the tool requires user permission
/// - `Err(ExecutionError::ExecutionFailed)` if execution fails unexpectedly
///
/// # Example
///
/// ```no_run
/// use patina::app::tool_loop::execute_tool;
/// use patina::tools::HookedToolExecutor;
/// use patina::types::content::ToolUseBlock;
/// use patina::hooks::HookManager;
/// use serde_json::json;
/// use std::path::PathBuf;
///
/// # async fn example() -> anyhow::Result<()> {
/// let hooks = HookManager::new("session".to_string());
/// let executor = HookedToolExecutor::new(PathBuf::from("."), hooks);
/// let tool_use = ToolUseBlock::new("toolu_123", "bash", json!({"command": "pwd"}));
///
/// let result = execute_tool(&tool_use, &executor).await?;
/// println!("Result: {}", result.content);
/// # Ok(())
/// # }
/// ```
pub async fn execute_tool(
    tool_use: &ToolUseBlock,
    executor: &crate::tools::HookedToolExecutor,
) -> Result<ToolResultBlock, ExecutionError> {
    use crate::tools::ToolResult;

    let call = tool_use_to_call(tool_use);

    let result = executor
        .execute(call)
        .await
        .map_err(|e| ExecutionError::ExecutionFailed(e.to_string()))?;

    match &result {
        ToolResult::NeedsPermission(_) => {
            Err(ExecutionError::NeedsPermission(tool_use.name.clone()))
        }
        _ => result_to_block(&tool_use.id, &result)
            .ok_or_else(|| ExecutionError::ExecutionFailed("Failed to convert result".to_string())),
    }
}

/// Executes all approved pending tools in the loop.
///
/// This method executes each approved tool that hasn't been executed yet,
/// storing results in the loop state.
///
/// # Arguments
///
/// * `executor` - The tool executor to run tools
///
/// # Returns
///
/// - `Ok(Vec<String>)` - IDs of tools that need permission (not executed)
/// - `Err(ToolLoopError)` - If not in Executing state
///
/// # State
///
/// Must be called when the loop is in `Executing` state.
impl ToolLoop {
    /// Executes all approved pending tools.
    ///
    /// Returns a list of tool IDs that require permission (if any).
    /// Tools that succeed or fail are recorded in the loop state.
    pub async fn execute_pending(
        &mut self,
        executor: &crate::tools::HookedToolExecutor,
    ) -> Result<Vec<String>, ToolLoopError> {
        if !matches!(self.state, ToolLoopState::Executing) {
            return Err(ToolLoopError::InvalidStateTransition {
                from: format!("{:?}", self.state),
                to: "execute_pending".to_string(),
            });
        }

        let mut needs_permission = Vec::new();

        // Get IDs of tools to execute
        let tool_ids: Vec<String> = self
            .pending_calls
            .values()
            .filter(|c| c.approved && !c.executed)
            .map(|c| c.tool_use.id.clone())
            .collect();

        for tool_id in tool_ids {
            // Get the tool_use - we need to clone to avoid borrow issues
            let tool_use = {
                let call = self.pending_calls.get(&tool_id).unwrap();
                call.tool_use.clone()
            };

            match execute_tool(&tool_use, executor).await {
                Ok(result_block) => {
                    if let Some(call) = self.pending_calls.get_mut(&tool_id) {
                        call.set_result(result_block);
                    }
                }
                Err(ExecutionError::NeedsPermission(_)) => {
                    needs_permission.push(tool_id);
                }
                Err(ExecutionError::ExecutionFailed(msg)) => {
                    // Record the error as a tool result
                    let error_block = ToolResultBlock::error(&tool_id, msg);
                    if let Some(call) = self.pending_calls.get_mut(&tool_id) {
                        call.set_result(error_block);
                    }
                }
            }
        }

        Ok(needs_permission)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_tool_loop_initial_state() {
        let loop_state = ToolLoop::new();
        assert_eq!(*loop_state.state(), ToolLoopState::Idle);
        assert!(loop_state.pending_calls().is_empty());
        assert!(loop_state.text_content().is_empty());
    }

    #[test]
    fn test_tool_loop_start_streaming() {
        let mut loop_state = ToolLoop::new();
        assert!(loop_state.start_streaming().is_ok());
        assert_eq!(*loop_state.state(), ToolLoopState::Streaming);
    }

    #[test]
    fn test_tool_loop_append_text() {
        let mut loop_state = ToolLoop::new();
        loop_state.start_streaming().unwrap();
        loop_state.append_text("Hello ");
        loop_state.append_text("World");
        assert_eq!(loop_state.text_content(), "Hello World");
    }

    #[test]
    fn test_tool_loop_text_only_completion() {
        let mut loop_state = ToolLoop::new();
        loop_state.start_streaming().unwrap();
        loop_state.append_text("Just text");
        loop_state.message_complete(StopReason::EndTurn).unwrap();
        assert_eq!(*loop_state.state(), ToolLoopState::Idle);
    }

    #[test]
    fn test_tool_loop_tool_use_flow() {
        let mut loop_state = ToolLoop::new();

        // Start streaming
        loop_state.start_streaming().unwrap();

        // Receive tool use
        loop_state.start_tool_use(0, "toolu_123".to_string(), "bash".to_string());
        loop_state.append_tool_input(0, r#"{"command":"ls"}"#);
        loop_state.complete_tool_use(0).unwrap();

        // Message completes with tool_use stop reason
        loop_state.message_complete(StopReason::ToolUse).unwrap();
        assert_eq!(*loop_state.state(), ToolLoopState::PendingApproval);

        // Approve and execute
        loop_state.approve_all().unwrap();
        assert_eq!(*loop_state.state(), ToolLoopState::Executing);

        // Set result
        let result = ToolResultBlock::success("toolu_123", "file1.txt\nfile2.txt");
        loop_state.set_tool_result("toolu_123", result).unwrap();

        // Finish execution
        let continuation = loop_state.finish_execution().unwrap();
        assert_eq!(continuation.tool_results.len(), 1);
        assert_eq!(*loop_state.state(), ToolLoopState::Continuing);
    }

    #[test]
    fn test_tool_loop_deny_tools() {
        let mut loop_state = ToolLoop::new();
        loop_state.start_streaming().unwrap();
        loop_state.start_tool_use(0, "id".to_string(), "bash".to_string());
        loop_state.append_tool_input(0, "{}");
        loop_state.complete_tool_use(0).unwrap();
        loop_state.message_complete(StopReason::ToolUse).unwrap();

        loop_state.deny_all().unwrap();
        assert_eq!(*loop_state.state(), ToolLoopState::Idle);
        assert!(loop_state.pending_calls().is_empty());
    }

    #[test]
    fn test_tool_loop_multiple_tools() {
        let mut loop_state = ToolLoop::new();
        loop_state.start_streaming().unwrap();

        // Tool 1
        loop_state.start_tool_use(0, "toolu_1".to_string(), "bash".to_string());
        loop_state.append_tool_input(0, r#"{"command":"ls"}"#);
        loop_state.complete_tool_use(0).unwrap();

        // Tool 2
        loop_state.start_tool_use(1, "toolu_2".to_string(), "read_file".to_string());
        loop_state.append_tool_input(1, r#"{"path":"README.md"}"#);
        loop_state.complete_tool_use(1).unwrap();

        loop_state.message_complete(StopReason::ToolUse).unwrap();

        assert_eq!(loop_state.pending_calls().len(), 2);
        assert_eq!(loop_state.tools_to_execute().len(), 0); // Not approved yet

        loop_state.approve_all().unwrap();
        assert_eq!(loop_state.tools_to_execute().len(), 2);
    }

    #[test]
    fn test_tool_loop_iteration_limit() {
        let mut loop_state = ToolLoop::with_max_iterations(2);

        // First iteration
        loop_state.start_streaming().unwrap();
        loop_state.start_tool_use(0, "id1".to_string(), "bash".to_string());
        loop_state.append_tool_input(0, "{}");
        loop_state.complete_tool_use(0).unwrap();
        loop_state.message_complete(StopReason::ToolUse).unwrap();
        loop_state.approve_all().unwrap();
        loop_state
            .set_tool_result("id1", ToolResultBlock::success("id1", "ok"))
            .unwrap();
        loop_state.finish_execution().unwrap();
        assert_eq!(loop_state.iteration(), 1);

        // Second iteration
        loop_state.start_streaming().unwrap();
        loop_state.start_tool_use(0, "id2".to_string(), "bash".to_string());
        loop_state.append_tool_input(0, "{}");
        loop_state.complete_tool_use(0).unwrap();
        loop_state.message_complete(StopReason::ToolUse).unwrap();
        loop_state.approve_all().unwrap();
        loop_state
            .set_tool_result("id2", ToolResultBlock::success("id2", "ok"))
            .unwrap();
        let result = loop_state.finish_execution();

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ToolLoopError::IterationLimitReached
        ));
    }

    #[test]
    fn test_tool_loop_reset() {
        let mut loop_state = ToolLoop::new();
        loop_state.start_streaming().unwrap();
        loop_state.append_text("test");
        loop_state.reset();

        assert_eq!(*loop_state.state(), ToolLoopState::Idle);
        assert!(loop_state.text_content().is_empty());
        assert_eq!(loop_state.iteration(), 0);
    }

    #[test]
    fn test_tool_loop_state_is_terminal() {
        assert!(ToolLoopState::Idle.is_terminal());
        assert!(ToolLoopState::Error("test".to_string()).is_terminal());
        assert!(!ToolLoopState::Streaming.is_terminal());
        assert!(!ToolLoopState::Executing.is_terminal());
    }

    #[test]
    fn test_tool_loop_state_needs_user_action() {
        assert!(ToolLoopState::Idle.needs_user_action());
        assert!(ToolLoopState::PendingApproval.needs_user_action());
        assert!(!ToolLoopState::Streaming.needs_user_action());
        assert!(!ToolLoopState::Executing.needs_user_action());
    }

    #[test]
    fn test_tool_loop_state_is_active() {
        assert!(ToolLoopState::Streaming.is_active());
        assert!(ToolLoopState::Executing.is_active());
        assert!(ToolLoopState::Continuing.is_active());
        assert!(!ToolLoopState::Idle.is_active());
        assert!(!ToolLoopState::PendingApproval.is_active());
    }

    #[test]
    fn test_pending_tool_call() {
        let tool_use = ToolUseBlock::new("id", "bash", json!({"command": "ls"}));
        let mut call = PendingToolCall::new(tool_use);

        assert!(!call.approved);
        assert!(!call.executed);
        assert!(call.result.is_none());

        call.approve();
        assert!(call.approved);

        let result = ToolResultBlock::success("id", "output");
        call.set_result(result);
        assert!(call.executed);
        assert!(call.result.is_some());
    }

    #[test]
    fn test_tool_loop_error_display() {
        let err = ToolLoopError::InvalidStateTransition {
            from: "Idle".to_string(),
            to: "Executing".to_string(),
        };
        assert!(err.to_string().contains("Invalid state transition"));

        let err = ToolLoopError::ToolNotFound("toolu_123".to_string());
        assert!(err.to_string().contains("toolu_123"));
    }

    #[test]
    fn test_tool_loop_invalid_transition() {
        let mut loop_state = ToolLoop::new();
        // Can't approve when in Idle state
        let result = loop_state.approve_all();
        assert!(result.is_err());
    }

    #[test]
    fn test_tool_loop_tool_not_found() {
        let mut loop_state = ToolLoop::new();
        loop_state.start_streaming().unwrap();
        loop_state.start_tool_use(0, "id".to_string(), "bash".to_string());
        loop_state.append_tool_input(0, "{}");
        loop_state.complete_tool_use(0).unwrap();
        loop_state.message_complete(StopReason::ToolUse).unwrap();
        loop_state.approve_all().unwrap();

        let result =
            loop_state.set_tool_result("wrong_id", ToolResultBlock::success("wrong_id", ""));
        assert!(matches!(
            result.unwrap_err(),
            ToolLoopError::ToolNotFound(_)
        ));
    }

    // =========================================================================
    // Tool Execution Bridge Tests
    // =========================================================================

    #[test]
    fn test_tool_use_to_call_conversion() {
        let tool_use = ToolUseBlock::new("toolu_123", "bash", json!({"command": "ls -la"}));
        let call = tool_use_to_call(&tool_use);

        assert_eq!(call.name, "bash");
        assert_eq!(call.input["command"], "ls -la");
    }

    #[test]
    fn test_tool_use_to_call_preserves_complex_input() {
        let tool_use = ToolUseBlock::new(
            "toolu_456",
            "edit",
            json!({
                "path": "src/main.rs",
                "old_string": "fn old()",
                "new_string": "fn new()"
            }),
        );
        let call = tool_use_to_call(&tool_use);

        assert_eq!(call.name, "edit");
        assert_eq!(call.input["path"], "src/main.rs");
        assert_eq!(call.input["old_string"], "fn old()");
        assert_eq!(call.input["new_string"], "fn new()");
    }

    #[test]
    fn test_result_to_block_success() {
        use crate::tools::ToolResult;

        let result = ToolResult::Success("file1.txt\nfile2.txt".to_string());
        let block = result_to_block("toolu_123", &result);

        assert!(block.is_some());
        let block = block.unwrap();
        assert_eq!(block.tool_use_id, "toolu_123");
        assert_eq!(block.content, "file1.txt\nfile2.txt");
        assert!(!block.is_error);
    }

    #[test]
    fn test_result_to_block_error() {
        use crate::tools::ToolResult;

        let result = ToolResult::Error("Permission denied".to_string());
        let block = result_to_block("toolu_456", &result);

        assert!(block.is_some());
        let block = block.unwrap();
        assert_eq!(block.tool_use_id, "toolu_456");
        assert_eq!(block.content, "Permission denied");
        assert!(block.is_error);
    }

    #[test]
    fn test_result_to_block_cancelled() {
        use crate::tools::ToolResult;

        let result = ToolResult::Cancelled;
        let block = result_to_block("toolu_789", &result);

        assert!(block.is_some());
        let block = block.unwrap();
        assert_eq!(block.tool_use_id, "toolu_789");
        assert!(block.content.contains("cancelled"));
        assert!(block.is_error);
    }

    #[test]
    fn test_result_to_block_needs_permission_returns_none() {
        use crate::permissions::PermissionRequest;
        use crate::tools::ToolResult;

        let request = PermissionRequest::new("bash", Some("rm -rf temp"), "Execute shell command");
        let result = ToolResult::NeedsPermission(request);
        let block = result_to_block("toolu_000", &result);

        assert!(block.is_none());
    }

    #[test]
    fn test_execution_error_display() {
        let err = ExecutionError::NeedsPermission("bash".to_string());
        assert!(err.to_string().contains("bash"));
        assert!(err.to_string().contains("permission"));

        let err = ExecutionError::ExecutionFailed("timeout".to_string());
        assert!(err.to_string().contains("timeout"));
    }

    // =========================================================================
    // Async Tool Execution Tests
    // =========================================================================

    #[tokio::test]
    async fn test_execute_bash_tool() {
        use crate::hooks::HookManager;
        use crate::tools::HookedToolExecutor;
        use tempfile::TempDir;

        // Create a temp directory for isolated execution
        let temp_dir = TempDir::new().expect("create temp dir");
        let working_dir = temp_dir.path().to_path_buf();

        // Create executor without permission manager (auto-allows)
        let hooks = HookManager::new("test-session".to_string());
        let executor = HookedToolExecutor::new(working_dir.clone(), hooks);

        // Create a bash tool use
        let tool_use = ToolUseBlock::new("toolu_123", "bash", json!({"command": "echo hello"}));

        // Execute the tool
        let result = execute_tool(&tool_use, &executor).await;

        assert!(result.is_ok());
        let block = result.unwrap();
        assert_eq!(block.tool_use_id, "toolu_123");
        assert!(block.content.contains("hello"));
        assert!(!block.is_error);
    }

    #[tokio::test]
    async fn test_execute_tool_handles_error() {
        use crate::hooks::HookManager;
        use crate::tools::HookedToolExecutor;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().expect("create temp dir");
        let working_dir = temp_dir.path().to_path_buf();

        let hooks = HookManager::new("test-session".to_string());
        let executor = HookedToolExecutor::new(working_dir.clone(), hooks);

        // Create a read_file tool use for a non-existent file
        let tool_use = ToolUseBlock::new(
            "toolu_456",
            "read_file",
            json!({"path": "nonexistent_file_12345.txt"}),
        );

        let result = execute_tool(&tool_use, &executor).await;

        assert!(result.is_ok());
        let block = result.unwrap();
        assert_eq!(block.tool_use_id, "toolu_456");
        assert!(block.is_error);
        assert!(block.content.contains("Failed to read") || block.content.contains("No such file"));
    }

    #[tokio::test]
    async fn test_execute_pending_all_tools() {
        use crate::hooks::HookManager;
        use crate::tools::HookedToolExecutor;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().expect("create temp dir");
        let working_dir = temp_dir.path().to_path_buf();

        let hooks = HookManager::new("test-session".to_string());
        let executor = HookedToolExecutor::new(working_dir.clone(), hooks);

        let mut loop_state = ToolLoop::new();

        // Set up tools
        loop_state.start_streaming().unwrap();
        loop_state.start_tool_use(0, "toolu_1".to_string(), "bash".to_string());
        loop_state.append_tool_input(0, r#"{"command":"echo tool1"}"#);
        loop_state.complete_tool_use(0).unwrap();

        loop_state.start_tool_use(1, "toolu_2".to_string(), "bash".to_string());
        loop_state.append_tool_input(1, r#"{"command":"echo tool2"}"#);
        loop_state.complete_tool_use(1).unwrap();

        loop_state.message_complete(StopReason::ToolUse).unwrap();
        loop_state.approve_all().unwrap();

        // Execute all pending tools
        let needs_permission = loop_state.execute_pending(&executor).await.unwrap();
        assert!(needs_permission.is_empty());

        // Verify all tools executed
        assert!(loop_state.all_tools_executed());

        let results = loop_state.collect_tool_results();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_execute_pending_with_error() {
        use crate::hooks::HookManager;
        use crate::tools::HookedToolExecutor;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().expect("create temp dir");
        let working_dir = temp_dir.path().to_path_buf();

        let hooks = HookManager::new("test-session".to_string());
        let executor = HookedToolExecutor::new(working_dir.clone(), hooks);

        let mut loop_state = ToolLoop::new();

        // One success, one error
        loop_state.start_streaming().unwrap();
        loop_state.start_tool_use(0, "toolu_1".to_string(), "bash".to_string());
        loop_state.append_tool_input(0, r#"{"command":"echo ok"}"#);
        loop_state.complete_tool_use(0).unwrap();

        loop_state.start_tool_use(1, "toolu_2".to_string(), "read_file".to_string());
        loop_state.append_tool_input(1, r#"{"path":"missing.txt"}"#);
        loop_state.complete_tool_use(1).unwrap();

        loop_state.message_complete(StopReason::ToolUse).unwrap();
        loop_state.approve_all().unwrap();

        // Execute
        let needs_permission = loop_state.execute_pending(&executor).await.unwrap();
        assert!(needs_permission.is_empty());

        // Both should have results
        let results = loop_state.collect_tool_results();
        assert_eq!(results.len(), 2);

        // One should be error
        let error_count = results
            .iter()
            .filter_map(|b| b.as_tool_result())
            .filter(|r| r.is_error)
            .count();
        assert_eq!(error_count, 1);
    }

    #[tokio::test]
    async fn test_execute_pending_wrong_state() {
        use crate::hooks::HookManager;
        use crate::tools::HookedToolExecutor;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().expect("create temp dir");
        let working_dir = temp_dir.path().to_path_buf();

        let hooks = HookManager::new("test-session".to_string());
        let executor = HookedToolExecutor::new(working_dir, hooks);

        let mut loop_state = ToolLoop::new();

        // Try to execute in Idle state
        let result = loop_state.execute_pending(&executor).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ToolLoopError::InvalidStateTransition { .. }
        ));
    }

    // =========================================================================
    // Conversation Continuation Tests
    // =========================================================================

    #[test]
    fn test_conversation_continues_after_tool_result() {
        use crate::types::Role;

        let mut loop_state = ToolLoop::new();

        // Set up a tool use flow
        loop_state.start_streaming().unwrap();
        loop_state.append_text("Let me run a command.");
        loop_state.start_tool_use(0, "toolu_123".to_string(), "bash".to_string());
        loop_state.append_tool_input(0, r#"{"command":"ls"}"#);
        loop_state.complete_tool_use(0).unwrap();
        loop_state.message_complete(StopReason::ToolUse).unwrap();

        // Approve and execute
        loop_state.approve_all().unwrap();
        let result = ToolResultBlock::success("toolu_123", "file1.txt\nfile2.txt");
        loop_state.set_tool_result("toolu_123", result).unwrap();

        // Finish execution - now returns ContinuationData
        let continuation = loop_state.finish_execution().unwrap();
        let (assistant_msg, user_msg) = continuation.build_messages();

        // Verify assistant message
        assert_eq!(assistant_msg.role, Role::Assistant);
        let content = assistant_msg.content.as_blocks().unwrap();
        assert_eq!(content.len(), 2); // text + tool_use
        assert!(content[0].as_text().is_some());
        assert!(content[1].as_tool_use().is_some());
        assert_eq!(content[0].as_text().unwrap(), "Let me run a command.");

        // Verify user message
        assert_eq!(user_msg.role, Role::User);
        let results = user_msg.content.as_blocks().unwrap();
        assert_eq!(results.len(), 1);
        let result_block = results[0].as_tool_result().unwrap();
        assert_eq!(result_block.tool_use_id, "toolu_123");
        assert!(result_block.content.contains("file1.txt"));
    }

    #[test]
    fn test_multiple_tool_results_in_single_message() {
        use crate::types::Role;

        let mut loop_state = ToolLoop::new();

        // Multiple tools
        loop_state.start_streaming().unwrap();
        loop_state.start_tool_use(0, "toolu_1".to_string(), "bash".to_string());
        loop_state.append_tool_input(0, r#"{"command":"pwd"}"#);
        loop_state.complete_tool_use(0).unwrap();

        loop_state.start_tool_use(1, "toolu_2".to_string(), "read_file".to_string());
        loop_state.append_tool_input(1, r#"{"path":"README.md"}"#);
        loop_state.complete_tool_use(1).unwrap();

        loop_state.message_complete(StopReason::ToolUse).unwrap();

        // Approve and set results
        loop_state.approve_all().unwrap();
        loop_state
            .set_tool_result("toolu_1", ToolResultBlock::success("toolu_1", "/home/user"))
            .unwrap();
        loop_state
            .set_tool_result("toolu_2", ToolResultBlock::success("toolu_2", "# README"))
            .unwrap();

        let continuation = loop_state.finish_execution().unwrap();
        let (assistant_msg, user_msg) = continuation.build_messages();

        // Assistant message should have 2 tool_use blocks (no text in this case)
        assert_eq!(assistant_msg.role, Role::Assistant);
        let content = assistant_msg.content.as_blocks().unwrap();
        assert_eq!(content.len(), 2); // 2 tool_use blocks
        assert!(content.iter().all(|b| b.is_tool_use()));

        // User message should have 2 tool_result blocks
        assert_eq!(user_msg.role, Role::User);
        let results = user_msg.content.as_blocks().unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|b| b.is_tool_result()));

        // Verify tool IDs
        let ids: Vec<_> = results
            .iter()
            .filter_map(|b| b.as_tool_result())
            .map(|r| &r.tool_use_id)
            .collect();
        assert!(ids.contains(&&"toolu_1".to_string()));
        assert!(ids.contains(&&"toolu_2".to_string()));
    }

    #[test]
    fn test_continuation_with_text_and_tool() {
        let mut loop_state = ToolLoop::new();

        loop_state.start_streaming().unwrap();
        loop_state.append_text("Here's what I found:\n");
        loop_state.start_tool_use(0, "toolu_abc".to_string(), "grep".to_string());
        loop_state.append_tool_input(0, r#"{"pattern":"TODO"}"#);
        loop_state.complete_tool_use(0).unwrap();
        loop_state.message_complete(StopReason::ToolUse).unwrap();

        loop_state.approve_all().unwrap();
        loop_state
            .set_tool_result(
                "toolu_abc",
                ToolResultBlock::success("toolu_abc", "main.rs:10: // TODO"),
            )
            .unwrap();

        let continuation = loop_state.finish_execution().unwrap();
        let (assistant_msg, _user_msg) = continuation.build_messages();

        let content = assistant_msg.content.as_blocks().unwrap();

        // First block is text
        assert!(content[0].is_text());
        assert_eq!(content[0].as_text().unwrap(), "Here's what I found:\n");

        // Second block is tool_use
        assert!(content[1].is_tool_use());
        let tool_use = content[1].as_tool_use().unwrap();
        assert_eq!(tool_use.name, "grep");
    }

    #[test]
    fn test_continuation_with_error_result() {
        let mut loop_state = ToolLoop::new();

        loop_state.start_streaming().unwrap();
        loop_state.start_tool_use(0, "toolu_fail".to_string(), "read_file".to_string());
        loop_state.append_tool_input(0, r#"{"path":"missing.txt"}"#);
        loop_state.complete_tool_use(0).unwrap();
        loop_state.message_complete(StopReason::ToolUse).unwrap();

        loop_state.approve_all().unwrap();
        loop_state
            .set_tool_result(
                "toolu_fail",
                ToolResultBlock::error("toolu_fail", "File not found"),
            )
            .unwrap();

        let continuation = loop_state.finish_execution().unwrap();
        let (_assistant_msg, user_msg) = continuation.build_messages();

        // User message should have the error result
        let results = user_msg.content.as_blocks().unwrap();
        assert_eq!(results.len(), 1);
        let result = results[0].as_tool_result().unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("File not found"));
    }

    #[test]
    fn test_continuation_data_struct() {
        let continuation = ContinuationData {
            assistant_content: vec![
                ContentBlock::text("Hello"),
                ContentBlock::tool_use("id", "bash", json!({})),
            ],
            tool_results: vec![ContentBlock::tool_result("id", "output")],
        };

        let (assistant_msg, user_msg) = continuation.build_messages();

        assert_eq!(assistant_msg.content.as_blocks().unwrap().len(), 2);
        assert_eq!(user_msg.content.as_blocks().unwrap().len(), 1);
    }

    #[test]
    fn test_collect_tool_uses() {
        let mut loop_state = ToolLoop::new();

        loop_state.start_streaming().unwrap();
        loop_state.start_tool_use(0, "t1".to_string(), "bash".to_string());
        loop_state.append_tool_input(0, "{}");
        loop_state.complete_tool_use(0).unwrap();

        loop_state.start_tool_use(1, "t2".to_string(), "read_file".to_string());
        loop_state.append_tool_input(1, "{}");
        loop_state.complete_tool_use(1).unwrap();

        loop_state.message_complete(StopReason::ToolUse).unwrap();

        // Before approval - should be empty
        let uses = loop_state.collect_tool_uses();
        assert!(uses.is_empty());

        // After approval
        loop_state.approve_all().unwrap();
        let uses = loop_state.collect_tool_uses();
        assert_eq!(uses.len(), 2);
        assert!(uses.iter().all(|b| b.is_tool_use()));
    }

    // =========================================================================
    // Recovery Tests
    // =========================================================================

    #[test]
    fn test_error_message_returns_none_for_non_error_state() {
        let loop_state = ToolLoop::new();
        assert!(loop_state.error_message().is_none());
    }

    #[test]
    fn test_error_message_returns_message_for_error_state() {
        let mut loop_state = ToolLoop::new();
        loop_state.force_state(ToolLoopState::Error("test error".to_string()));
        assert_eq!(loop_state.error_message(), Some("test error"));
    }

    #[test]
    fn test_can_recover_from_error_state() {
        let mut loop_state = ToolLoop::new();
        loop_state.force_state(ToolLoopState::Error("test error".to_string()));
        assert!(loop_state.can_recover());
    }

    #[test]
    fn test_can_recover_from_pending_approval_state() {
        let mut loop_state = ToolLoop::new();
        loop_state.start_streaming().unwrap();
        loop_state.start_tool_use(0, "id".to_string(), "bash".to_string());
        loop_state.append_tool_input(0, "{}");
        loop_state.complete_tool_use(0).unwrap();
        loop_state.message_complete(StopReason::ToolUse).unwrap();

        assert!(loop_state.can_recover());
    }

    #[test]
    fn test_cannot_recover_from_idle_state() {
        let loop_state = ToolLoop::new();
        assert!(!loop_state.can_recover());
    }

    #[test]
    fn test_cannot_recover_from_streaming_state() {
        let mut loop_state = ToolLoop::new();
        loop_state.start_streaming().unwrap();
        assert!(!loop_state.can_recover());
    }

    #[test]
    fn test_recover_from_error_resets_to_idle() {
        let mut loop_state = ToolLoop::new();
        loop_state.force_state(ToolLoopState::Error("test error".to_string()));

        let error_msg = loop_state.recover_from_error().unwrap();

        assert_eq!(error_msg, "test error");
        assert_eq!(*loop_state.state(), ToolLoopState::Idle);
    }

    #[test]
    fn test_recover_from_error_preserves_iteration_count() {
        let mut loop_state = ToolLoop::new();

        // Simulate some iterations having happened
        loop_state.start_streaming().unwrap();
        loop_state.start_tool_use(0, "id".to_string(), "bash".to_string());
        loop_state.append_tool_input(0, "{}");
        loop_state.complete_tool_use(0).unwrap();
        loop_state.message_complete(StopReason::ToolUse).unwrap();
        loop_state.approve_all().unwrap();
        loop_state
            .set_tool_result("id", ToolResultBlock::success("id", "ok"))
            .unwrap();
        loop_state.finish_execution().unwrap();

        assert_eq!(loop_state.iteration(), 1);

        // Now simulate an error
        loop_state.force_state(ToolLoopState::Error("test error".to_string()));
        loop_state.recover_from_error().unwrap();

        // Iteration count should be preserved
        assert_eq!(loop_state.iteration(), 1);
    }

    #[test]
    fn test_recover_from_error_fails_when_not_in_error_state() {
        let mut loop_state = ToolLoop::new();
        let result = loop_state.recover_from_error();
        assert!(result.is_err());
    }

    #[test]
    fn test_retry_approval_resets_approval_status() {
        let mut loop_state = ToolLoop::new();
        loop_state.start_streaming().unwrap();
        loop_state.start_tool_use(0, "id".to_string(), "bash".to_string());
        loop_state.append_tool_input(0, "{}");
        loop_state.complete_tool_use(0).unwrap();
        loop_state.message_complete(StopReason::ToolUse).unwrap();

        loop_state.approve_all().unwrap();
        assert!(loop_state.pending_calls().get("id").unwrap().approved);

        // Go to executing state
        assert_eq!(*loop_state.state(), ToolLoopState::Executing);

        // Retry should work from Executing state
        let count = loop_state.retry_approval().unwrap();
        assert_eq!(count, 1);
        assert_eq!(*loop_state.state(), ToolLoopState::PendingApproval);
        assert!(!loop_state.pending_calls().get("id").unwrap().approved);
    }

    #[test]
    fn test_retry_approval_from_pending_approval() {
        let mut loop_state = ToolLoop::new();
        loop_state.start_streaming().unwrap();
        loop_state.start_tool_use(0, "id".to_string(), "bash".to_string());
        loop_state.append_tool_input(0, "{}");
        loop_state.complete_tool_use(0).unwrap();
        loop_state.message_complete(StopReason::ToolUse).unwrap();

        // Should work from PendingApproval
        let count = loop_state.retry_approval().unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_retry_approval_fails_from_idle_state() {
        let mut loop_state = ToolLoop::new();
        let result = loop_state.retry_approval();
        assert!(result.is_err());
    }

    #[test]
    fn test_force_state_sets_state_directly() {
        let mut loop_state = ToolLoop::new();
        loop_state.force_state(ToolLoopState::Executing);
        assert_eq!(*loop_state.state(), ToolLoopState::Executing);
    }

    #[test]
    fn test_snapshot_captures_state() {
        let mut loop_state = ToolLoop::new();
        loop_state.start_streaming().unwrap();
        loop_state.append_text("hello world");
        loop_state.start_tool_use(0, "t1".to_string(), "bash".to_string());
        loop_state.append_tool_input(0, "{}");
        loop_state.complete_tool_use(0).unwrap();
        loop_state.message_complete(StopReason::ToolUse).unwrap();

        let snapshot = loop_state.snapshot();

        assert!(matches!(snapshot.state, ToolLoopState::PendingApproval));
        assert_eq!(snapshot.pending_tool_ids, vec!["t1"]);
        assert_eq!(snapshot.text_content_len, 11); // "hello world"
        assert_eq!(snapshot.iteration, 0);
        assert_eq!(snapshot.max_iterations, 50);
    }

    #[test]
    fn test_snapshot_is_error() {
        let mut loop_state = ToolLoop::new();
        loop_state.force_state(ToolLoopState::Error("test".to_string()));

        let snapshot = loop_state.snapshot();
        assert!(snapshot.is_error());
        assert_eq!(snapshot.error_message(), Some("test"));
    }

    #[test]
    fn test_snapshot_has_pending_tools() {
        let mut loop_state = ToolLoop::new();
        loop_state.start_streaming().unwrap();
        loop_state.start_tool_use(0, "t1".to_string(), "bash".to_string());
        loop_state.append_tool_input(0, "{}");
        loop_state.complete_tool_use(0).unwrap();
        loop_state.message_complete(StopReason::ToolUse).unwrap();

        let snapshot = loop_state.snapshot();
        assert!(snapshot.has_pending_tools());
    }

    #[test]
    fn test_restore_from_snapshot() {
        let mut loop_state = ToolLoop::with_max_iterations(100);

        // Do some work to change iteration count
        loop_state.start_streaming().unwrap();
        loop_state.start_tool_use(0, "id".to_string(), "bash".to_string());
        loop_state.append_tool_input(0, "{}");
        loop_state.complete_tool_use(0).unwrap();
        loop_state.message_complete(StopReason::ToolUse).unwrap();
        loop_state.approve_all().unwrap();
        loop_state
            .set_tool_result("id", ToolResultBlock::success("id", "ok"))
            .unwrap();
        loop_state.finish_execution().unwrap();

        let snapshot = loop_state.snapshot();

        // Create fresh loop and restore
        let mut new_loop = ToolLoop::new();
        new_loop.restore_from_snapshot(&snapshot);

        assert_eq!(new_loop.iteration(), snapshot.iteration);
        assert_eq!(new_loop.max_iterations, snapshot.max_iterations);
        assert_eq!(*new_loop.state(), ToolLoopState::Idle);
    }

    // =========================================================================
    // Security Pre-Flight Tests (Task 2.3.4)
    // =========================================================================

    #[test]
    fn test_pending_tool_call_security_verdict_initially_none() {
        let tool_use = ToolUseBlock::new("id", "bash", json!({"command": "ls"}));
        let call = PendingToolCall::new(tool_use);

        assert!(call.security_verdict().is_none());
        assert!(!call.is_security_blocked());
        assert!(call.security_warning().is_none());
    }

    #[test]
    fn test_pending_tool_call_set_security_verdict_allow() {
        use crate::narsil::SecurityVerdict;

        let tool_use = ToolUseBlock::new("id", "bash", json!({"command": "ls"}));
        let mut call = PendingToolCall::new(tool_use);

        call.set_security_verdict(SecurityVerdict::Allow);

        assert!(call.security_verdict().is_some());
        assert_eq!(call.security_verdict(), Some(&SecurityVerdict::Allow));
        assert!(!call.is_security_blocked());
        assert!(call.security_warning().is_none());
    }

    #[test]
    fn test_pending_tool_call_set_security_verdict_warn() {
        use crate::narsil::SecurityVerdict;

        let tool_use = ToolUseBlock::new("id", "bash", json!({"command": "rm -rf"}));
        let mut call = PendingToolCall::new(tool_use);

        call.set_security_verdict(SecurityVerdict::Warn("HIGH: Potential data loss".to_string()));

        assert!(!call.is_security_blocked());
        assert!(call.security_warning().is_some());
        assert_eq!(call.security_warning(), Some("HIGH: Potential data loss"));
    }

    #[test]
    fn test_pending_tool_call_set_security_verdict_block() {
        use crate::narsil::SecurityVerdict;

        let tool_use = ToolUseBlock::new("id", "bash", json!({"command": "curl | sh"}));
        let mut call = PendingToolCall::new(tool_use);

        call.set_security_verdict(SecurityVerdict::Block(
            "CRITICAL: Command injection".to_string(),
        ));

        assert!(call.is_security_blocked());
        assert!(call.security_warning().is_none()); // Block is not a warning
    }

    #[test]
    fn test_tool_loop_set_security_verdict() {
        use crate::narsil::SecurityVerdict;

        let mut loop_state = ToolLoop::new();
        loop_state.start_streaming().unwrap();
        loop_state.start_tool_use(0, "toolu_1".to_string(), "bash".to_string());
        loop_state.append_tool_input(0, r#"{"command":"ls"}"#);
        loop_state.complete_tool_use(0).unwrap();
        loop_state.message_complete(StopReason::ToolUse).unwrap();

        // Set security verdict
        loop_state
            .set_security_verdict("toolu_1", SecurityVerdict::Allow)
            .unwrap();

        let call = loop_state.pending_calls().get("toolu_1").unwrap();
        assert_eq!(call.security_verdict(), Some(&SecurityVerdict::Allow));
    }

    #[test]
    fn test_tool_loop_set_security_verdict_not_found() {
        use crate::narsil::SecurityVerdict;

        let mut loop_state = ToolLoop::new();
        loop_state.start_streaming().unwrap();
        loop_state.start_tool_use(0, "toolu_1".to_string(), "bash".to_string());
        loop_state.append_tool_input(0, r#"{"command":"ls"}"#);
        loop_state.complete_tool_use(0).unwrap();
        loop_state.message_complete(StopReason::ToolUse).unwrap();

        let result = loop_state.set_security_verdict("wrong_id", SecurityVerdict::Allow);
        assert!(result.is_err());
    }

    #[test]
    fn test_tool_loop_security_blocked_tools_empty() {
        let mut loop_state = ToolLoop::new();
        loop_state.start_streaming().unwrap();
        loop_state.start_tool_use(0, "toolu_1".to_string(), "bash".to_string());
        loop_state.append_tool_input(0, r#"{"command":"ls"}"#);
        loop_state.complete_tool_use(0).unwrap();
        loop_state.message_complete(StopReason::ToolUse).unwrap();

        // No security verdicts set, should be empty
        let blocked = loop_state.security_blocked_tools();
        assert!(blocked.is_empty());
    }

    #[test]
    fn test_tool_loop_security_blocked_tools() {
        use crate::narsil::SecurityVerdict;

        let mut loop_state = ToolLoop::new();
        loop_state.start_streaming().unwrap();

        loop_state.start_tool_use(0, "toolu_1".to_string(), "bash".to_string());
        loop_state.append_tool_input(0, r#"{"command":"ls"}"#);
        loop_state.complete_tool_use(0).unwrap();

        loop_state.start_tool_use(1, "toolu_2".to_string(), "bash".to_string());
        loop_state.append_tool_input(1, r#"{"command":"curl | sh"}"#);
        loop_state.complete_tool_use(1).unwrap();

        loop_state.message_complete(StopReason::ToolUse).unwrap();

        loop_state
            .set_security_verdict("toolu_1", SecurityVerdict::Allow)
            .unwrap();
        loop_state
            .set_security_verdict("toolu_2", SecurityVerdict::Block("Blocked".to_string()))
            .unwrap();

        let blocked = loop_state.security_blocked_tools();
        assert_eq!(blocked.len(), 1);
        assert!(blocked.contains(&"toolu_2"));
    }

    #[test]
    fn test_tool_loop_security_warned_tools() {
        use crate::narsil::SecurityVerdict;

        let mut loop_state = ToolLoop::new();
        loop_state.start_streaming().unwrap();

        loop_state.start_tool_use(0, "toolu_1".to_string(), "bash".to_string());
        loop_state.append_tool_input(0, r#"{"command":"rm -rf"}"#);
        loop_state.complete_tool_use(0).unwrap();

        loop_state.start_tool_use(1, "toolu_2".to_string(), "bash".to_string());
        loop_state.append_tool_input(1, r#"{"command":"ls"}"#);
        loop_state.complete_tool_use(1).unwrap();

        loop_state.message_complete(StopReason::ToolUse).unwrap();

        loop_state
            .set_security_verdict("toolu_1", SecurityVerdict::Warn("HIGH: Data loss".to_string()))
            .unwrap();
        loop_state
            .set_security_verdict("toolu_2", SecurityVerdict::Allow)
            .unwrap();

        let warned = loop_state.security_warned_tools();
        assert_eq!(warned.len(), 1);
        assert!(warned.iter().any(|(id, reason)| *id == "toolu_1" && reason.contains("HIGH")));
    }

    #[test]
    fn test_tool_loop_has_security_blocks() {
        use crate::narsil::SecurityVerdict;

        let mut loop_state = ToolLoop::new();
        loop_state.start_streaming().unwrap();
        loop_state.start_tool_use(0, "toolu_1".to_string(), "bash".to_string());
        loop_state.append_tool_input(0, r#"{"command":"ls"}"#);
        loop_state.complete_tool_use(0).unwrap();
        loop_state.message_complete(StopReason::ToolUse).unwrap();

        assert!(!loop_state.has_security_blocks());

        loop_state
            .set_security_verdict("toolu_1", SecurityVerdict::Block("Blocked".to_string()))
            .unwrap();

        assert!(loop_state.has_security_blocks());
    }

    #[test]
    fn test_tool_loop_has_security_warnings() {
        use crate::narsil::SecurityVerdict;

        let mut loop_state = ToolLoop::new();
        loop_state.start_streaming().unwrap();
        loop_state.start_tool_use(0, "toolu_1".to_string(), "bash".to_string());
        loop_state.append_tool_input(0, r#"{"command":"ls"}"#);
        loop_state.complete_tool_use(0).unwrap();
        loop_state.message_complete(StopReason::ToolUse).unwrap();

        assert!(!loop_state.has_security_warnings());

        loop_state
            .set_security_verdict("toolu_1", SecurityVerdict::Warn("Warning".to_string()))
            .unwrap();

        assert!(loop_state.has_security_warnings());
    }

    #[test]
    fn test_tool_loop_approve_safe_skips_blocked() {
        use crate::narsil::SecurityVerdict;

        let mut loop_state = ToolLoop::new();
        loop_state.start_streaming().unwrap();

        // Tool 1: allowed
        loop_state.start_tool_use(0, "toolu_1".to_string(), "bash".to_string());
        loop_state.append_tool_input(0, r#"{"command":"ls"}"#);
        loop_state.complete_tool_use(0).unwrap();

        // Tool 2: blocked
        loop_state.start_tool_use(1, "toolu_2".to_string(), "bash".to_string());
        loop_state.append_tool_input(1, r#"{"command":"evil"}"#);
        loop_state.complete_tool_use(1).unwrap();

        // Tool 3: warned but still allowed
        loop_state.start_tool_use(2, "toolu_3".to_string(), "bash".to_string());
        loop_state.append_tool_input(2, r#"{"command":"rm"}"#);
        loop_state.complete_tool_use(2).unwrap();

        loop_state.message_complete(StopReason::ToolUse).unwrap();

        loop_state
            .set_security_verdict("toolu_1", SecurityVerdict::Allow)
            .unwrap();
        loop_state
            .set_security_verdict("toolu_2", SecurityVerdict::Block("Blocked".to_string()))
            .unwrap();
        loop_state
            .set_security_verdict("toolu_3", SecurityVerdict::Warn("Warning".to_string()))
            .unwrap();

        // approve_safe should approve only non-blocked tools
        let blocked_ids = loop_state.approve_safe().unwrap();

        assert_eq!(blocked_ids.len(), 1);
        assert!(blocked_ids.contains(&"toolu_2".to_string()));

        // toolu_1 and toolu_3 should be approved
        assert!(loop_state.pending_calls().get("toolu_1").unwrap().approved);
        assert!(!loop_state.pending_calls().get("toolu_2").unwrap().approved);
        assert!(loop_state.pending_calls().get("toolu_3").unwrap().approved);
    }
}
