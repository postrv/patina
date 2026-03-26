//! UI state types shared between application state and TUI rendering.
//!
//! These types hold pure data for UI state management. They are defined here
//! (rather than in `tui/`) so that `app::state` can use them without creating
//! a circular dependency between the `app` and `tui` modules.
//!
//! Types moved here include:
//! - [`ScrollState`] / [`AutoScrollMode`] - scroll position tracking
//! - [`CompactionProgressState`] / [`CompactionStatus`] - compaction progress
//! - [`SelectionState`] / [`ContentPosition`] / [`FocusArea`] - UI selection
//! - [`ToolBlockState`] - tool execution display
//! - [`ContinuousLoopStatus`] - continuous loop status for TUI display
//! - [`GateResult`] - quality gate results for TUI display
//! - [`AgentPanelStatus`] / [`AgentPanelEntry`] - agent panel display
//! - [`PlanState`] / [`PlanStep`] - plan review state
//! - [`QuestionState`] - interactive question prompt state
//! - [`CompletionState`] / [`CompletionEntry`] / [`CompletionSource`] - completion engine
//!
//! Rendering widgets that consume these types remain in `tui/`.

use std::fmt;

use super::ToolResultBlock;

// =============================================================================
// Scroll State
// =============================================================================

/// Auto-scroll mode that determines scrolling behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AutoScrollMode {
    /// Automatically follow new content (default).
    ///
    /// In this mode, the scroll position is updated when new content
    /// arrives to keep the latest content visible.
    #[default]
    Follow,

    /// Manual scroll position set by user.
    ///
    /// In this mode, the scroll position is preserved and new content
    /// does not affect the current view.
    Manual,

    /// Auto-scroll temporarily paused.
    ///
    /// Similar to Manual, but indicates a temporary pause that may
    /// resume Follow mode automatically in certain conditions.
    Paused,
}

impl AutoScrollMode {
    /// Returns true if this mode should auto-scroll on new content.
    #[must_use]
    pub fn should_auto_scroll(&self) -> bool {
        matches!(self, Self::Follow)
    }

    /// Returns true if this is a user-controlled mode.
    #[must_use]
    pub fn is_user_controlled(&self) -> bool {
        matches!(self, Self::Manual | Self::Paused)
    }
}

/// Smart scroll state with mode-based behavior.
#[derive(Debug, Clone)]
pub struct ScrollState {
    /// Current scroll offset (lines from top).
    offset: usize,

    /// Current auto-scroll mode.
    mode: AutoScrollMode,

    /// Total content height (for scroll-to-bottom detection).
    content_height: usize,

    /// Visible viewport height.
    viewport_height: usize,
}

impl Default for ScrollState {
    fn default() -> Self {
        Self::new()
    }
}

impl ScrollState {
    /// Creates a new scroll state in Follow mode.
    #[must_use]
    pub fn new() -> Self {
        Self {
            offset: 0,
            mode: AutoScrollMode::Follow,
            content_height: 0,
            viewport_height: 0,
        }
    }

    /// Returns the current scroll offset.
    #[must_use]
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Returns the current auto-scroll mode.
    #[must_use]
    pub fn mode(&self) -> AutoScrollMode {
        self.mode
    }

    /// Returns the content height.
    #[must_use]
    pub fn content_height(&self) -> usize {
        self.content_height
    }

    /// Returns the viewport height.
    #[must_use]
    pub fn viewport_height(&self) -> usize {
        self.viewport_height
    }

    /// Sets the viewport height.
    ///
    /// This should be called when the terminal is resized.
    pub fn set_viewport_height(&mut self, height: usize) {
        self.viewport_height = height;
        self.clamp_offset();
    }

    /// Updates the content height and auto-scrolls if in Follow mode.
    ///
    /// This should be called when new content is added.
    pub fn set_content_height(&mut self, height: usize) {
        let previous_height = self.content_height;
        self.content_height = height;

        // If in Follow mode and content grew, scroll to show new content
        if self.mode.should_auto_scroll() && height > previous_height {
            self.scroll_to_bottom_internal();
        }
    }

    /// Scrolls up by the specified number of lines.
    ///
    /// This switches to Manual mode if not already in a user-controlled mode.
    pub fn scroll_up(&mut self, lines: usize) {
        self.offset = self.offset.saturating_add(lines);
        self.clamp_offset();

        // Switch to Manual mode when user scrolls up
        if !self.mode.is_user_controlled() {
            self.mode = AutoScrollMode::Manual;
        }
    }

    /// Scrolls down by the specified number of lines.
    ///
    /// If scrolling to the bottom, resumes Follow mode.
    pub fn scroll_down(&mut self, lines: usize) {
        self.offset = self.offset.saturating_sub(lines);

        // If we're at the bottom, resume Follow mode
        if self.is_at_bottom() {
            self.mode = AutoScrollMode::Follow;
        }
    }

    /// Scrolls to show the bottom of the content.
    ///
    /// This resumes Follow mode.
    pub fn scroll_to_bottom(&mut self, content_height: usize) {
        self.content_height = content_height;
        self.scroll_to_bottom_internal();
        self.mode = AutoScrollMode::Follow;
    }

    /// Scrolls to the top of the content.
    ///
    /// This switches to Manual mode.
    pub fn scroll_to_top(&mut self) {
        self.offset = self.max_offset();
        self.mode = AutoScrollMode::Manual;
    }

    /// Pauses auto-scroll temporarily.
    pub fn pause(&mut self) {
        if self.mode == AutoScrollMode::Follow {
            self.mode = AutoScrollMode::Paused;
        }
    }

    /// Resumes Follow mode from Paused state.
    pub fn resume(&mut self) {
        if self.mode == AutoScrollMode::Paused {
            self.mode = AutoScrollMode::Follow;
            self.scroll_to_bottom_internal();
        }
    }

    /// Forces Follow mode regardless of current state.
    pub fn force_follow(&mut self) {
        self.mode = AutoScrollMode::Follow;
        self.scroll_to_bottom_internal();
    }

    /// Restores scroll state from a saved offset.
    ///
    /// This is used when restoring from a saved session.
    /// If the offset is non-zero, switches to Manual mode.
    pub fn restore_offset(&mut self, offset: usize) {
        self.offset = offset;
        if offset > 0 {
            self.mode = AutoScrollMode::Manual;
        } else {
            self.mode = AutoScrollMode::Follow;
        }
    }

    /// Returns true if the view is at the bottom of the content.
    #[must_use]
    pub fn is_at_bottom(&self) -> bool {
        self.offset == 0
    }

    /// Returns true if the view is at the top of the content.
    #[must_use]
    pub fn is_at_top(&self) -> bool {
        self.offset >= self.max_offset()
    }

    /// Returns the index of the first visible line in the content.
    ///
    /// This converts from the internal "offset from bottom" representation
    /// to the actual line number that appears at the top of the viewport.
    #[must_use]
    pub fn first_visible_line(&self) -> usize {
        self.max_offset().saturating_sub(self.offset)
    }

    /// Returns the maximum scroll offset.
    fn max_offset(&self) -> usize {
        self.content_height.saturating_sub(self.viewport_height)
    }

    /// Clamps the offset to valid range.
    fn clamp_offset(&mut self) {
        let max = self.max_offset();
        if self.offset > max {
            self.offset = max;
        }
    }

    /// Internal scroll to bottom without changing mode.
    fn scroll_to_bottom_internal(&mut self) {
        self.offset = 0;
    }
}

// =============================================================================
// Selection State
// =============================================================================

/// Represents which area of the UI has focus.
///
/// Used to determine how keyboard shortcuts like Ctrl+A behave:
/// - `Input`: The text input area has focus (default)
/// - `Content`: The message/response content area has focus
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FocusArea {
    /// The text input area (default)
    #[default]
    Input,
    /// The message/response content area
    Content,
}

/// Position in rendered timeline content.
///
/// Represents a cursor position in the visible content area, where `line`
/// is the visual line number (after text wrapping) and `col` is the
/// character offset within that line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ContentPosition {
    /// Visual line number (0-indexed, after wrapping)
    pub line: usize,
    /// Character offset within line (0-indexed)
    pub col: usize,
}

impl ContentPosition {
    /// Creates a new content position.
    #[must_use]
    pub const fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }
}

impl PartialOrd for ContentPosition {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ContentPosition {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.line.cmp(&other.line) {
            std::cmp::Ordering::Equal => self.col.cmp(&other.col),
            ordering => ordering,
        }
    }
}

/// Selection state for copy/paste functionality.
///
/// Manages the anchor (start) and cursor (current) positions of a text
/// selection. Supports mouse-based selection (click-drag) and keyboard
/// shortcuts (select all).
///
/// Note: The `extract_text` method that takes `ratatui::text::Line` is
/// defined in `tui::selection` to avoid a ratatui dependency here.
#[derive(Debug, Clone, Default)]
pub struct SelectionState {
    /// Anchor point where selection started
    anchor: Option<ContentPosition>,
    /// Current cursor position (end of selection during drag)
    cursor: Option<ContentPosition>,
    /// Whether actively selecting (mouse button held)
    selecting: bool,
}

impl SelectionState {
    /// Creates a new empty selection state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            anchor: None,
            cursor: None,
            selecting: false,
        }
    }

    /// Starts a new selection at the given position.
    ///
    /// Called when the user clicks to begin selecting.
    pub fn start(&mut self, pos: ContentPosition) {
        self.anchor = Some(pos);
        self.cursor = Some(pos);
        self.selecting = true;
    }

    /// Updates the selection end position during drag.
    ///
    /// Called as the user drags to extend the selection.
    pub fn update(&mut self, pos: ContentPosition) {
        if self.selecting {
            self.cursor = Some(pos);
        }
    }

    /// Completes the selection.
    ///
    /// Called when the user releases the mouse button.
    pub fn end(&mut self) {
        self.selecting = false;
    }

    /// Clears the current selection.
    pub fn clear(&mut self) {
        self.anchor = None;
        self.cursor = None;
        self.selecting = false;
    }

    /// Selects all content (Cmd/Ctrl+A).
    ///
    /// Sets selection from (0, 0) to (total_lines - 1, MAX).
    pub fn select_all(&mut self, total_lines: usize) {
        if total_lines == 0 {
            self.clear();
            return;
        }
        self.anchor = Some(ContentPosition::new(0, 0));
        self.cursor = Some(ContentPosition::new(total_lines - 1, usize::MAX));
        self.selecting = false;
    }

    /// Returns whether there is an active selection.
    #[must_use]
    pub fn has_selection(&self) -> bool {
        self.anchor.is_some() && self.cursor.is_some() && !self.selecting
    }

    /// Returns whether currently in the process of selecting.
    #[must_use]
    pub fn is_selecting(&self) -> bool {
        self.selecting
    }

    /// Returns the normalized selection range (start, end) where start <= end.
    ///
    /// Returns `None` if there is no complete selection.
    #[must_use]
    pub fn range(&self) -> Option<(ContentPosition, ContentPosition)> {
        match (self.anchor, self.cursor) {
            (Some(anchor), Some(cursor)) if !self.selecting => {
                let (start, end) = if anchor <= cursor {
                    (anchor, cursor)
                } else {
                    (cursor, anchor)
                };
                Some((start, end))
            }
            _ => None,
        }
    }
}

// =============================================================================
// Compaction Progress State
// =============================================================================

/// Status of the compaction operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompactionStatus {
    /// No compaction in progress.
    #[default]
    Idle,
    /// Compaction is currently running.
    Compacting,
    /// Compaction completed successfully.
    Complete,
    /// Compaction failed.
    Failed,
}

impl fmt::Display for CompactionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => write!(f, "Idle"),
            Self::Compacting => write!(f, "Compacting..."),
            Self::Complete => write!(f, "Complete"),
            Self::Failed => write!(f, "Failed"),
        }
    }
}

/// State for the compaction progress widget.
///
/// Tracks the progress of a context compaction operation.
#[derive(Debug, Clone)]
pub struct CompactionProgressState {
    /// Target token count after compaction.
    target_tokens: usize,
    /// Token count before compaction.
    before_tokens: usize,
    /// Token count after compaction (set when complete).
    after_tokens: Option<usize>,
    /// Current progress (0.0 to 1.0).
    progress: f64,
    /// Current status of the compaction.
    status: CompactionStatus,
    /// Whether this is an auto-triggered compaction (vs manual).
    is_auto: bool,
}

impl CompactionProgressState {
    /// Creates a new compaction progress state for manual compaction.
    ///
    /// # Arguments
    ///
    /// * `target_tokens` - Target token count after compaction
    /// * `before_tokens` - Token count before compaction
    #[must_use]
    pub fn new(target_tokens: usize, before_tokens: usize) -> Self {
        Self {
            target_tokens,
            before_tokens,
            after_tokens: None,
            progress: 0.0,
            status: CompactionStatus::Idle,
            is_auto: false,
        }
    }

    /// Creates a new compaction progress state for auto-triggered compaction.
    ///
    /// This is used when compaction is automatically triggered due to
    /// context limit approaching, rather than manually requested.
    ///
    /// # Arguments
    ///
    /// * `target_tokens` - Target token count after compaction
    /// * `before_tokens` - Token count before compaction
    #[must_use]
    pub fn new_auto(target_tokens: usize, before_tokens: usize) -> Self {
        Self {
            target_tokens,
            before_tokens,
            after_tokens: None,
            progress: 0.0,
            status: CompactionStatus::Idle,
            is_auto: true,
        }
    }

    /// Returns the target token count.
    #[must_use]
    pub fn target_tokens(&self) -> usize {
        self.target_tokens
    }

    /// Returns the token count before compaction.
    #[must_use]
    pub fn before_tokens(&self) -> usize {
        self.before_tokens
    }

    /// Returns the token count after compaction, if available.
    #[must_use]
    pub fn after_tokens(&self) -> Option<usize> {
        self.after_tokens
    }

    /// Sets the token count after compaction.
    pub fn set_after_tokens(&mut self, tokens: usize) {
        self.after_tokens = Some(tokens);
    }

    /// Returns the number of tokens saved by compaction.
    ///
    /// Returns `None` if compaction has not completed.
    #[must_use]
    pub fn saved_tokens(&self) -> Option<usize> {
        self.after_tokens
            .map(|after| self.before_tokens.saturating_sub(after))
    }

    /// Returns the current progress (0.0 to 1.0).
    #[must_use]
    pub fn progress(&self) -> f64 {
        self.progress
    }

    /// Sets the current progress.
    ///
    /// The value is clamped to the range \[0.0, 1.0\].
    pub fn set_progress(&mut self, progress: f64) {
        self.progress = progress.clamp(0.0, 1.0);
    }

    /// Returns the current status.
    #[must_use]
    pub fn status(&self) -> CompactionStatus {
        self.status
    }

    /// Sets the current status.
    pub fn set_status(&mut self, status: CompactionStatus) {
        self.status = status;
    }

    /// Returns whether this is an auto-triggered compaction.
    ///
    /// Auto compaction is triggered when the context approaches the limit,
    /// as opposed to manual compaction requested by the user.
    #[must_use]
    pub fn is_auto(&self) -> bool {
        self.is_auto
    }

    /// Sets whether this is an auto-triggered compaction.
    pub fn set_auto(&mut self, is_auto: bool) {
        self.is_auto = is_auto;
    }
}

// =============================================================================
// Tool Block State
// =============================================================================

/// State for the tool block widget.
#[derive(Debug, Clone)]
pub struct ToolBlockState {
    /// Name of the tool being executed (e.g., "bash", "read_file").
    tool_name: String,

    /// Input provided to the tool (e.g., command, file path).
    tool_input: String,

    /// Result of tool execution, if complete.
    result: Option<String>,

    /// Whether the result represents an error.
    is_error: bool,
}

impl ToolBlockState {
    /// Creates a new tool block state.
    ///
    /// # Arguments
    ///
    /// * `tool_name` - Name of the tool (e.g., "bash", "read_file")
    /// * `tool_input` - Input provided to the tool
    #[must_use]
    pub fn new(tool_name: impl Into<String>, tool_input: impl Into<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            tool_input: tool_input.into(),
            result: None,
            is_error: false,
        }
    }

    /// Returns the tool name.
    #[must_use]
    pub fn tool_name(&self) -> &str {
        &self.tool_name
    }

    /// Returns the tool input.
    #[must_use]
    pub fn tool_input(&self) -> &str {
        &self.tool_input
    }

    /// Returns the result, if any.
    #[must_use]
    pub fn result(&self) -> Option<&str> {
        self.result.as_deref()
    }

    /// Returns whether this is an error result.
    #[must_use]
    pub fn is_error(&self) -> bool {
        self.is_error
    }

    /// Returns whether the tool execution is complete.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.result.is_some()
    }

    /// Sets the result of tool execution.
    pub fn set_result(&mut self, result: impl Into<String>) {
        self.result = Some(result.into());
        self.is_error = false;
    }

    /// Sets an error result.
    pub fn set_error(&mut self, error: impl Into<String>) {
        self.result = Some(error.into());
        self.is_error = true;
    }
}

// =============================================================================
// Continuous Loop Status
// =============================================================================

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

// =============================================================================
// Gate Result (UI display variant)
// =============================================================================

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

// =============================================================================
// Agent Panel Types
// =============================================================================

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

// =============================================================================
// Plan State
// =============================================================================

/// A single step in a proposed plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanStep {
    /// Human-readable description of this step.
    pub description: String,
    /// Tool calls this step intends to make (informational).
    pub tool_calls: Vec<String>,
}

/// State for a pending plan awaiting user review.
///
/// Created when the model emits a `plan` tool_use. The plan is held
/// in this state until the user approves or rejects it via the
/// [`PlanHandler`](crate::app::handlers::plan::PlanHandler).
#[derive(Debug, Clone)]
pub struct PlanState {
    /// The tool_use ID to include in the tool result response.
    pub tool_use_id: String,
    /// Plan title summarizing the proposed work.
    pub title: String,
    /// Ordered list of steps in the plan.
    pub steps: Vec<PlanStep>,
    /// Index of the currently highlighted step in the review UI.
    pub selected_index: usize,
}

impl PlanState {
    /// Creates a new plan state from a tool_use.
    ///
    /// # Arguments
    ///
    /// * `tool_use_id` - The tool_use ID for the response
    /// * `title` - Plan title
    /// * `steps` - Ordered plan steps
    #[must_use]
    pub fn new(tool_use_id: String, title: String, steps: Vec<PlanStep>) -> Self {
        Self {
            tool_use_id,
            title,
            steps,
            selected_index: 0,
        }
    }

    /// Moves the selection to the next step (wraps around).
    pub fn select_next(&mut self) {
        if !self.steps.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.steps.len();
        }
    }

    /// Moves the selection to the previous step (wraps around).
    pub fn select_prev(&mut self) {
        if !self.steps.is_empty() {
            self.selected_index = if self.selected_index == 0 {
                self.steps.len() - 1
            } else {
                self.selected_index - 1
            };
        }
    }

    /// Returns the number of steps in the plan.
    #[must_use]
    pub fn step_count(&self) -> usize {
        self.steps.len()
    }

    /// Approves the plan, returning a success tool result block.
    #[must_use]
    pub fn approve(&self) -> ToolResultBlock {
        ToolResultBlock {
            tool_use_id: self.tool_use_id.clone(),
            content: "Plan approved by user. Proceed with execution.".to_string(),
            is_error: false,
        }
    }

    /// Rejects the plan, returning an error tool result block.
    #[must_use]
    pub fn reject(&self) -> ToolResultBlock {
        ToolResultBlock {
            tool_use_id: self.tool_use_id.clone(),
            content: "Plan rejected by user.".to_string(),
            is_error: true,
        }
    }

    /// Parses a plan from tool_use JSON input.
    ///
    /// Expected schema:
    /// ```json
    /// {
    ///   "title": "...",
    ///   "steps": [{ "description": "...", "tool_calls": ["..."] }]
    /// }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns `None` if the input doesn't match the expected schema.
    #[must_use]
    pub fn from_tool_input(tool_use_id: String, input: &serde_json::Value) -> Option<Self> {
        let title = input.get("title")?.as_str()?.to_string();
        let steps_arr = input.get("steps")?.as_array()?;

        let steps: Vec<PlanStep> = steps_arr
            .iter()
            .filter_map(|step| {
                let description = step.get("description")?.as_str()?.to_string();
                let tool_calls = step
                    .get("tool_calls")
                    .and_then(|tc| tc.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                Some(PlanStep {
                    description,
                    tool_calls,
                })
            })
            .collect();

        if steps.is_empty() {
            return None;
        }

        Some(Self::new(tool_use_id, title, steps))
    }
}

// =============================================================================
// Question State
// =============================================================================

/// State for a pending user question awaiting response.
///
/// Created when the model emits an `ask_user` tool_use. The question is held
/// in this state until the user responds via the
/// [`QuestionHandler`](crate::app::handlers::question::QuestionHandler).
#[derive(Debug, Clone)]
pub struct QuestionState {
    /// The tool_use ID to include in the tool result response.
    pub tool_use_id: String,
    /// The question text to display.
    pub question: String,
    /// Optional list of choices for the user.
    pub options: Vec<String>,
    /// Whether free-text input is allowed.
    pub allow_free_text: bool,
    /// Index of the currently selected option (0 if no options).
    pub selected_option: usize,
    /// Free-text input buffer.
    pub free_text_input: String,
    /// Whether the cursor is in the free-text input field (vs option list).
    pub in_free_text: bool,
}

impl QuestionState {
    /// Creates a new question state from tool input parameters.
    ///
    /// # Arguments
    ///
    /// * `tool_use_id` - The tool_use ID for the response
    /// * `question` - The question text
    /// * `options` - Optional list of choices
    /// * `allow_free_text` - Whether free-text is allowed
    #[must_use]
    pub fn new(
        tool_use_id: String,
        question: String,
        options: Vec<String>,
        allow_free_text: bool,
    ) -> Self {
        let in_free_text = options.is_empty();
        Self {
            tool_use_id,
            question,
            options,
            allow_free_text,
            selected_option: 0,
            free_text_input: String::new(),
            in_free_text,
        }
    }

    /// Moves the selection to the next option (wraps around).
    pub fn select_next(&mut self) {
        if !self.options.is_empty() {
            self.selected_option = (self.selected_option + 1) % self.options.len();
        }
    }

    /// Moves the selection to the previous option (wraps around).
    pub fn select_prev(&mut self) {
        if !self.options.is_empty() {
            self.selected_option = if self.selected_option == 0 {
                self.options.len() - 1
            } else {
                self.selected_option - 1
            };
        }
    }

    /// Toggles between option selection and free-text input.
    pub fn toggle_input_mode(&mut self) {
        if self.allow_free_text && !self.options.is_empty() {
            self.in_free_text = !self.in_free_text;
        }
    }

    /// Appends a character to the free-text input.
    pub fn push_char(&mut self, c: char) {
        if self.in_free_text {
            self.free_text_input.push(c);
        }
    }

    /// Removes the last character from the free-text input.
    pub fn pop_char(&mut self) {
        if self.in_free_text {
            self.free_text_input.pop();
        }
    }

    /// Returns the user's response as a string.
    ///
    /// If in free-text mode and text is non-empty, returns the text.
    /// Otherwise, returns the selected option (if any).
    #[must_use]
    pub fn response_text(&self) -> String {
        if self.in_free_text && !self.free_text_input.is_empty() {
            self.free_text_input.clone()
        } else if !self.options.is_empty() {
            self.options[self.selected_option].clone()
        } else {
            self.free_text_input.clone()
        }
    }

    /// Submits the response, returning a success tool result block.
    #[must_use]
    pub fn submit(&self) -> ToolResultBlock {
        ToolResultBlock {
            tool_use_id: self.tool_use_id.clone(),
            content: self.response_text(),
            is_error: false,
        }
    }

    /// Cancels the question, returning an error tool result block.
    #[must_use]
    pub fn cancel(&self) -> ToolResultBlock {
        ToolResultBlock {
            tool_use_id: self.tool_use_id.clone(),
            content: "User cancelled the question.".to_string(),
            is_error: true,
        }
    }

    /// Parses a question from tool_use JSON input.
    ///
    /// Expected schema:
    /// ```json
    /// {
    ///   "question": "...",
    ///   "options": ["a", "b"],
    ///   "allow_free_text": true
    /// }
    /// ```
    ///
    /// # Returns
    ///
    /// `None` if the input doesn't have a `question` field.
    #[must_use]
    pub fn from_tool_input(tool_use_id: String, input: &serde_json::Value) -> Option<Self> {
        let question = input.get("question")?.as_str()?.to_string();

        let options = input
            .get("options")
            .and_then(|o| o.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let allow_free_text = input
            .get("allow_free_text")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        Some(Self::new(tool_use_id, question, options, allow_free_text))
    }
}

// =============================================================================
// Completion State
// =============================================================================

/// Source of a completion candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionSource {
    /// Built-in slash command.
    Builtin,
    /// Command provided by a plugin.
    Plugin(String),
    /// MCP tool (future).
    McpTool(String),
    /// User-defined command (future).
    User,
}

impl CompletionSource {
    /// Returns a short display indicator for the source.
    ///
    /// Used in the completion menu to show where a command comes from.
    #[must_use]
    pub fn indicator(&self) -> &str {
        match self {
            Self::Builtin => "",
            Self::Plugin(_) => "[P]",
            Self::McpTool(_) => "[M]",
            Self::User => "[U]",
        }
    }
}

/// A single completion candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionEntry {
    /// The command name (without leading `/`).
    pub name: String,
    /// A short description shown alongside the name.
    pub description: String,
    /// Where this candidate comes from.
    pub source: CompletionSource,
}

impl CompletionEntry {
    /// Creates a new completion entry.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        source: CompletionSource,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            source,
        }
    }
}

/// Scores a candidate against a filter string.
///
/// Returns a score from 0 to 100:
/// - 100: exact name match
/// - 80: name starts with filter (prefix match)
/// - 60: filter found as substring in name (fuzzy name)
/// - 40: filter found as substring in description (fuzzy description)
/// - 0: no match
///
/// All comparisons are case-insensitive. An empty filter matches everything
/// with score 100.
#[must_use]
pub fn score_candidate(entry: &CompletionEntry, filter: &str) -> u8 {
    if filter.is_empty() {
        return 100;
    }

    let filter_lower = filter.to_lowercase();
    let name_lower = entry.name.to_lowercase();
    let desc_lower = entry.description.to_lowercase();

    if name_lower == filter_lower {
        100
    } else if name_lower.starts_with(&filter_lower) {
        80
    } else if name_lower.contains(&filter_lower) {
        60
    } else if desc_lower.contains(&filter_lower) {
        40
    } else {
        0
    }
}

/// A scored candidate for internal sorting.
#[derive(Debug, Clone)]
struct ScoredEntry {
    entry: CompletionEntry,
    score: u8,
}

/// Manages the completion popup lifecycle, filtering, and selection.
///
/// Created when the user types `/` at position 0, destroyed when they dismiss
/// or accept a candidate.
#[derive(Debug, Clone)]
pub struct CompletionState {
    /// All candidates from all providers.
    candidates: Vec<CompletionEntry>,
    /// Currently visible (filtered + sorted) entries.
    filtered: Vec<CompletionEntry>,
    /// Index into `filtered` for the highlighted entry.
    selected: usize,
    /// Current filter text (everything after `/`).
    filter: String,
}

impl CompletionState {
    /// Creates a new completion state showing all candidates.
    #[must_use]
    pub fn new(candidates: Vec<CompletionEntry>) -> Self {
        let filtered = candidates.clone();
        Self {
            candidates,
            filtered,
            selected: 0,
            filter: String::new(),
        }
    }

    /// Updates the filter and recomputes the filtered list.
    ///
    /// Resets selection to 0.
    pub fn set_filter(&mut self, filter: &str) {
        self.filter = filter.to_string();
        self.refilter();
        self.selected = 0;
    }

    /// Returns the current filter string.
    #[must_use]
    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Returns the filtered and sorted entries.
    #[must_use]
    pub fn filtered(&self) -> &[CompletionEntry] {
        &self.filtered
    }

    /// Returns the index of the currently selected entry.
    #[must_use]
    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// Returns the currently selected entry, if any.
    #[must_use]
    pub fn selected_entry(&self) -> Option<&CompletionEntry> {
        self.filtered.get(self.selected)
    }

    /// Moves selection down, wrapping to the top.
    pub fn select_next(&mut self) {
        if !self.filtered.is_empty() {
            self.selected = (self.selected + 1) % self.filtered.len();
        }
    }

    /// Moves selection up, wrapping to the bottom.
    pub fn select_previous(&mut self) {
        if !self.filtered.is_empty() {
            if self.selected == 0 {
                self.selected = self.filtered.len() - 1;
            } else {
                self.selected -= 1;
            }
        }
    }

    /// Accepts the currently selected entry.
    ///
    /// Returns the command name (without `/`) if there is a selection,
    /// or `None` if the filtered list is empty.
    #[must_use]
    pub fn accept(&self) -> Option<String> {
        self.selected_entry().map(|e| e.name.clone())
    }

    /// Recomputes the filtered list from candidates using current filter.
    fn refilter(&mut self) {
        let mut scored: Vec<ScoredEntry> = self
            .candidates
            .iter()
            .filter_map(|entry| {
                let score = score_candidate(entry, &self.filter);
                if score > 0 {
                    Some(ScoredEntry {
                        entry: entry.clone(),
                        score,
                    })
                } else {
                    None
                }
            })
            .collect();

        // Sort by score descending, then by name ascending for stable ordering.
        scored.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.entry.name.cmp(&b.entry.name))
        });

        self.filtered = scored.into_iter().map(|s| s.entry).collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_state_constructs_in_follow_mode() {
        let state = ScrollState::new();
        assert_eq!(state.mode(), AutoScrollMode::Follow);
        assert_eq!(state.offset(), 0);
    }

    #[test]
    fn focus_area_defaults_to_input() {
        assert_eq!(FocusArea::default(), FocusArea::Input);
    }

    #[test]
    fn content_position_ordering() {
        let p1 = ContentPosition::new(0, 5);
        let p2 = ContentPosition::new(1, 0);
        assert!(p1 < p2);
    }

    #[test]
    fn selection_state_lifecycle() {
        let mut sel = SelectionState::new();
        assert!(!sel.has_selection());
        sel.start(ContentPosition::new(0, 0));
        sel.update(ContentPosition::new(1, 5));
        sel.end();
        assert!(sel.has_selection());
    }

    #[test]
    fn compaction_status_display() {
        assert_eq!(CompactionStatus::Compacting.to_string(), "Compacting...");
        assert_eq!(CompactionStatus::Complete.to_string(), "Complete");
    }

    #[test]
    fn compaction_progress_state_tracks_progress() {
        let mut state = CompactionProgressState::new(5000, 10000);
        assert_eq!(state.status(), CompactionStatus::Idle);
        state.set_progress(0.5);
        assert!((state.progress() - 0.5).abs() < f64::EPSILON);
        state.set_after_tokens(6000);
        assert_eq!(state.saved_tokens(), Some(4000));
    }

    #[test]
    fn tool_block_state_tracks_execution() {
        let mut state = ToolBlockState::new("bash", "ls");
        assert!(!state.is_complete());
        state.set_result("file1\nfile2");
        assert!(state.is_complete());
        assert!(!state.is_error());
    }

    #[test]
    fn tool_block_state_tracks_errors() {
        let mut state = ToolBlockState::new("bash", "invalid");
        state.set_error("command not found");
        assert!(state.is_complete());
        assert!(state.is_error());
    }

    #[test]
    fn continuous_loop_status_variants() {
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

    #[test]
    fn gate_result_construction() {
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
    fn agent_panel_status_variants() {
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
    fn agent_panel_entry_construction() {
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
}
