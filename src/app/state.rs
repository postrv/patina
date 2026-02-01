//! Application state management

use crate::api::tools::default_tools;
use crate::api::{AnthropicClient, StreamEvent, TokenBudget, ToolChoice};
use crate::app::tool_loop::{ContinuationData, ToolLoop, ToolLoopState};
use crate::hooks::HookManager;
use crate::permissions::{PermissionManager, PermissionRequest, PermissionResponse};
use crate::session::Session;
use crate::tools::{HookedToolExecutor, ParallelConfig};
use crate::tui::scroll::ScrollState;
use crate::tui::selection::{FocusArea, SelectionState};
use crate::tui::widgets::{CompactionProgressState, ToolBlockState};
use crate::types::config::ParallelMode;
use crate::types::content::StopReason;
use crate::types::{ApiMessageV2, Message, Role, Timeline};
use anyhow::Result;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// Formats tool input JSON into a readable string for display.
///
/// Extracts the most relevant field based on tool type:
/// - `bash` / `Bash`: Shows the command
/// - `read` / `Read`: Shows the file path
/// - `write` / `Write`: Shows the file path
/// - `edit` / `Edit`: Shows the file path
/// - `glob` / `Glob`: Shows the pattern
/// - `grep` / `Grep`: Shows the pattern
/// - Other tools: Shows compact JSON
#[must_use]
fn format_tool_input(tool_name: &str, input: &Value) -> String {
    let name_lower = tool_name.to_lowercase();

    // Try to extract the most relevant field based on tool type
    match name_lower.as_str() {
        "bash" => input
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| compact_json(input))
            .to_string(),
        "read" | "read_file" => input
            .get("file_path")
            .or_else(|| input.get("path"))
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| compact_json(input))
            .to_string(),
        "write" | "write_file" => input
            .get("file_path")
            .or_else(|| input.get("path"))
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| compact_json(input))
            .to_string(),
        "edit" => input
            .get("file_path")
            .or_else(|| input.get("path"))
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| compact_json(input))
            .to_string(),
        "glob" => input
            .get("pattern")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| compact_json(input))
            .to_string(),
        "grep" => input
            .get("pattern")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| compact_json(input))
            .to_string(),
        _ => compact_json(input).to_string(),
    }
}

/// Returns a compact single-line JSON representation.
fn compact_json(value: &Value) -> &str {
    // For simple values, return a static string representation
    // For complex values, the caller will need to format it
    match value {
        Value::Null => "null",
        Value::Bool(true) => "true",
        Value::Bool(false) => "false",
        Value::String(s) => s.as_str(),
        _ => "...",
    }
}

pub struct AppState {
    /// Full API messages with content blocks (tool_use, tool_result).
    /// This is the authoritative conversation history sent to the API.
    api_messages: Vec<ApiMessageV2>,

    pub input: String,
    pub working_dir: PathBuf,

    /// Smart scroll state with auto-follow behavior.
    scroll: ScrollState,

    cursor_pos: usize,
    loading: bool,
    throbber_frame: usize,
    streaming_rx: Option<mpsc::Receiver<StreamEvent>>,

    dirty: DirtyFlags,

    // Worktree status bar state
    worktree_branch: Option<String>,
    worktree_modified: usize,
    worktree_ahead: usize,
    worktree_behind: usize,

    // Session tracking for auto-save
    session_id: Option<String>,

    // Tool execution state
    tool_loop: ToolLoop,
    tool_executor: Arc<HookedToolExecutor>,
    permission_manager: Arc<Mutex<PermissionManager>>,
    pending_permission: Option<PermissionRequest>,

    /// Tool blocks for UI display.
    /// Each block represents a tool execution with its name, input, and result.
    tool_blocks: Vec<ToolBlockState>,

    /// Unified timeline for conversation display.
    /// This is the single source of truth for display ordering, replacing the
    /// dual-system of `messages` + `current_response`.
    timeline: Timeline,

    /// Channel receiver for async tool results.
    /// When set, tool execution runs in the background and results
    /// are streamed back through this channel.
    tool_result_rx: Option<mpsc::UnboundedReceiver<(String, crate::types::ToolResultBlock)>>,

    /// Set of tool IDs currently being executed.
    /// Used to track which tools are in-flight for progress display.
    executing_tool_ids: std::collections::HashSet<String>,

    /// Text selection state for copy/paste functionality.
    selection: SelectionState,

    /// Flag indicating a copy operation was requested.
    /// Set by keyboard handler, consumed during render when lines are available.
    copy_pending: bool,

    /// Cached rendered lines for copy operations.
    /// Updated during render to enable copy extraction.
    rendered_lines_cache: Vec<String>,

    /// Which area of the UI currently has focus.
    /// Determines how shortcuts like Ctrl+A behave.
    focus_area: FocusArea,

    /// Token budget tracking for the current session.
    /// Displays usage in the status bar with color-coded warnings.
    token_budget: TokenBudget,

    /// Optional compaction progress state for displaying the compaction overlay.
    /// When set, the compaction progress widget is shown as a modal.
    compaction_state: Option<CompactionProgressState>,
}

#[derive(Default)]
struct DirtyFlags {
    messages: bool,
    input: bool,
    full: bool,
}

impl DirtyFlags {
    fn any(&self) -> bool {
        self.messages || self.input || self.full
    }

    fn clear(&mut self) {
        *self = Self::default();
    }
}

impl AppState {
    /// Creates a new AppState with tool execution support.
    ///
    /// # Arguments
    ///
    /// * `working_dir` - The working directory for file operations
    /// * `skip_permissions` - If true, bypass all permission prompts
    /// * `parallel_mode` - Controls parallel tool execution
    pub fn new(working_dir: PathBuf, skip_permissions: bool, parallel_mode: ParallelMode) -> Self {
        // Generate a unique session ID for hooks
        let hook_session_id = uuid::Uuid::new_v4().to_string();
        let hook_manager = HookManager::new(hook_session_id);

        // Create permission manager with skip_permissions setting
        let mut pm = PermissionManager::new();
        pm.set_skip_permissions(skip_permissions);
        let permission_manager = Arc::new(Mutex::new(pm));

        // Convert ParallelMode to ParallelConfig
        let parallel_config = match parallel_mode {
            ParallelMode::Enabled => ParallelConfig::enabled(),
            ParallelMode::Disabled => ParallelConfig::disabled(),
            ParallelMode::Aggressive => ParallelConfig::aggressive(),
        };

        // Create tool executor with hook, permission, and parallel configuration
        let tool_executor = Arc::new(
            HookedToolExecutor::new(working_dir.clone(), hook_manager)
                .with_permissions(Arc::clone(&permission_manager))
                .with_parallel_config(parallel_config),
        );

        Self {
            api_messages: Vec::new(),
            input: String::new(),
            working_dir,
            scroll: ScrollState::new(),
            cursor_pos: 0,
            loading: false,
            throbber_frame: 0,
            streaming_rx: None,
            dirty: DirtyFlags {
                full: true,
                ..Default::default()
            },
            worktree_branch: None,
            worktree_modified: 0,
            worktree_ahead: 0,
            worktree_behind: 0,
            session_id: None,
            tool_loop: ToolLoop::new(),
            tool_executor,
            permission_manager,
            pending_permission: None,
            tool_blocks: Vec::new(),
            timeline: Timeline::new(),
            tool_result_rx: None,
            executing_tool_ids: std::collections::HashSet::new(),
            selection: SelectionState::new(),
            copy_pending: false,
            rendered_lines_cache: Vec::new(),
            focus_area: FocusArea::default(),
            token_budget: TokenBudget::new(100_000), // Claude's typical context window
            compaction_state: None,
        }
    }

    /// Inserts a character at the current cursor position.
    pub fn insert_char(&mut self, c: char) {
        // Get byte position from char position
        let byte_pos = self
            .input
            .char_indices()
            .nth(self.cursor_pos)
            .map(|(i, _)| i)
            .unwrap_or(self.input.len());
        self.input.insert(byte_pos, c);
        self.cursor_pos += 1;
        self.dirty.input = true;
    }

    /// Deletes the character before the cursor (backspace behavior).
    pub fn delete_char(&mut self) {
        if self.cursor_pos > 0 {
            // Get byte position of the character to delete (one before cursor)
            let byte_pos = self
                .input
                .char_indices()
                .nth(self.cursor_pos - 1)
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.input.remove(byte_pos);
            self.cursor_pos -= 1;
        }
        self.dirty.input = true;
    }

    /// Takes and returns the current input, clearing the buffer and resetting cursor.
    pub fn take_input(&mut self) -> String {
        self.dirty.input = true;
        self.cursor_pos = 0;
        std::mem::take(&mut self.input)
    }

    /// Returns the current cursor position (character index, not byte index).
    #[must_use]
    pub fn cursor_position(&self) -> usize {
        self.cursor_pos
    }

    /// Moves the cursor left by one character.
    pub fn cursor_left(&mut self) {
        self.cursor_pos = self.cursor_pos.saturating_sub(1);
        self.dirty.input = true;
    }

    /// Moves the cursor right by one character.
    pub fn cursor_right(&mut self) {
        let char_count = self.input.chars().count();
        if self.cursor_pos < char_count {
            self.cursor_pos += 1;
        }
        self.dirty.input = true;
    }

    /// Moves the cursor to the beginning of the input.
    pub fn cursor_home(&mut self) {
        self.cursor_pos = 0;
        self.dirty.input = true;
    }

    /// Moves the cursor to the end of the input.
    pub fn cursor_end(&mut self) {
        self.cursor_pos = self.input.chars().count();
        self.dirty.input = true;
    }

    /// Returns the current scroll offset for rendering.
    ///
    /// This provides backward compatibility with TUI rendering.
    #[must_use]
    pub fn scroll_offset(&self) -> usize {
        self.scroll.offset()
    }

    /// Scrolls up by the specified number of lines.
    ///
    /// This switches to Manual mode, preserving the scroll position
    /// during streaming updates.
    pub fn scroll_up(&mut self, lines: usize) {
        let before = self.scroll.offset();
        self.scroll.scroll_up(lines);
        let after = self.scroll.offset();
        tracing::debug!(
            lines,
            before,
            after,
            mode = ?self.scroll.mode(),
            content_height = self.scroll.content_height(),
            viewport_height = self.scroll.viewport_height(),
            cache_size = self.rendered_lines_cache.len(),
            timeline_entries = self.timeline.len(),
            "scroll_up"
        );
        self.dirty.messages = true;
    }

    /// Scrolls down by the specified number of lines.
    ///
    /// If scrolling to the bottom, resumes Follow mode for auto-scroll.
    pub fn scroll_down(&mut self, lines: usize) {
        let before = self.scroll.offset();
        self.scroll.scroll_down(lines);
        let after = self.scroll.offset();
        tracing::debug!(
            lines,
            before,
            after,
            mode = ?self.scroll.mode(),
            "scroll_down"
        );
        self.dirty.messages = true;
    }

    /// Scrolls to the bottom of the content.
    ///
    /// This resumes Follow mode for auto-scroll.
    pub fn scroll_to_bottom(&mut self, content_height: usize) {
        self.scroll.scroll_to_bottom(content_height);
        self.dirty.messages = true;
    }

    /// Scrolls to the top of the content.
    ///
    /// This switches to Manual mode.
    pub fn scroll_to_top(&mut self) {
        self.scroll.scroll_to_top();
        self.dirty.messages = true;
    }

    /// Updates the content height for scroll calculations.
    ///
    /// In Follow mode, this auto-scrolls to show new content.
    pub fn update_content_height(&mut self, height: usize) {
        self.scroll.set_content_height(height);
        if self.scroll.mode().should_auto_scroll() {
            self.dirty.messages = true;
        }
    }

    /// Updates the viewport height for scroll calculations.
    pub fn set_viewport_height(&mut self, height: usize) {
        self.scroll.set_viewport_height(height);
    }

    /// Returns the scroll state for read access.
    #[must_use]
    pub fn scroll_state(&self) -> &ScrollState {
        &self.scroll
    }

    /// Returns the selection state for read access.
    #[must_use]
    pub fn selection(&self) -> &SelectionState {
        &self.selection
    }

    /// Returns the selection state for modification.
    pub fn selection_mut(&mut self) -> &mut SelectionState {
        &mut self.selection
    }

    /// Returns the current focus area.
    #[must_use]
    pub fn focus_area(&self) -> FocusArea {
        self.focus_area
    }

    /// Sets the focus area, clearing selection if focus changes.
    ///
    /// When focus moves between Input and Content areas, any existing
    /// selection is cleared to prevent confusion about what would be copied.
    pub fn set_focus_area(&mut self, area: FocusArea) {
        if self.focus_area != area {
            self.selection.clear();
            self.focus_area = area;
        }
    }

    /// Determines which focus area a screen row belongs to.
    ///
    /// Layout (from top to bottom):
    /// - Messages/Content: rows 0 to (terminal_height - 5)
    /// - Status bar: row (terminal_height - 4)
    /// - Input: rows (terminal_height - 3) to (terminal_height - 1)
    ///
    /// # Arguments
    ///
    /// * `row` - The screen row (0-indexed, 0 = top)
    /// * `terminal_height` - Total terminal height in rows
    ///
    /// # Returns
    ///
    /// The `FocusArea` that the row belongs to.
    #[must_use]
    pub fn focus_area_for_row(row: u16, terminal_height: u16) -> FocusArea {
        // Input area is the bottom 3 rows
        // Status bar is 1 row above input
        // Content area is everything else
        let input_start = terminal_height.saturating_sub(3);
        if row >= input_start {
            FocusArea::Input
        } else {
            FocusArea::Content
        }
    }

    /// Copies the current selection to the system clipboard.
    ///
    /// Uses multiple clipboard backends:
    /// 1. Native clipboard (arboard) - works on desktop
    /// 2. OSC 52 escape sequence - works in iTerm2, kitty, tmux, SSH, etc.
    ///
    /// Returns `Ok(true)` if text was copied, `Ok(false)` if no selection,
    /// or an error if clipboard access fails.
    ///
    /// # Errors
    ///
    /// Returns an error if all clipboard methods fail.
    pub fn copy_selection_to_clipboard(&self, lines: &[ratatui::text::Line<'_>]) -> Result<bool> {
        let text = self.selection.extract_text(lines);
        if text.is_empty() {
            return Ok(false);
        }

        crate::tui::clipboard::copy_to_clipboard(&text)?;
        Ok(true)
    }

    /// Requests a copy operation to be performed during the next render.
    pub fn request_copy(&mut self) {
        self.copy_pending = true;
    }

    /// Checks and clears the copy pending flag.
    ///
    /// Returns `true` if a copy was requested.
    pub fn take_copy_pending(&mut self) -> bool {
        std::mem::take(&mut self.copy_pending)
    }

    /// Returns the total number of rendered lines.
    ///
    /// This is the count from the cached rendered lines, which represents
    /// the actual number of visual lines in the conversation display.
    /// Used for select-all functionality.
    #[must_use]
    pub fn rendered_line_count(&self) -> usize {
        self.rendered_lines_cache.len()
    }

    /// Updates the cached rendered lines for copy operations.
    ///
    /// This stores the **wrapped** visual lines, accounting for terminal width.
    /// Selection and copy operations use visual line indices, so we must cache
    /// the post-wrapping content.
    ///
    /// # Arguments
    ///
    /// * `lines` - The logical lines before wrapping
    /// * `width` - The terminal content width (excluding borders)
    pub fn update_rendered_lines_cache(&mut self, lines: &[ratatui::text::Line<'_>], width: usize) {
        self.rendered_lines_cache = crate::tui::wrap_lines_to_strings(lines, width);
    }

    /// Copies the current selection to clipboard using cached lines.
    ///
    /// # Errors
    ///
    /// Returns an error if clipboard access fails.
    pub fn copy_from_cache(&self) -> Result<bool> {
        let Some((start, end)) = self.selection.range() else {
            tracing::debug!("copy_from_cache: no selection range");
            return Ok(false);
        };

        tracing::debug!(
            ?start,
            ?end,
            cache_len = self.rendered_lines_cache.len(),
            "copy_from_cache: extracting"
        );

        if self.rendered_lines_cache.is_empty() {
            tracing::debug!("copy_from_cache: cache is empty");
            return Ok(false);
        }

        // Extract text from cached lines
        let mut result = String::new();
        for (line_idx, line_text) in self.rendered_lines_cache.iter().enumerate() {
            if line_idx < start.line {
                continue;
            }
            if line_idx > end.line {
                break;
            }

            let (col_start, col_end) = if line_idx == start.line && line_idx == end.line {
                (start.col, end.col.min(line_text.len()))
            } else if line_idx == start.line {
                (start.col, line_text.len())
            } else if line_idx == end.line {
                (0, end.col.min(line_text.len()))
            } else {
                (0, line_text.len())
            };

            let col_start = col_start.min(line_text.len());
            let col_end = col_end.min(line_text.len());

            if col_start <= col_end {
                if !result.is_empty() {
                    result.push('\n');
                }
                result.push_str(&line_text[col_start..col_end]);
            }
        }

        if result.is_empty() {
            tracing::debug!("copy_from_cache: extracted empty result");
            return Ok(false);
        }

        tracing::debug!(
            result_len = result.len(),
            result_lines = result.lines().count(),
            "copy_from_cache: copying to clipboard"
        );

        crate::tui::clipboard::copy_to_clipboard(&result)?;
        Ok(true)
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn tick_throbber(&mut self) {
        self.throbber_frame = (self.throbber_frame + 1) % 4;
        self.dirty.messages = true;
    }

    pub fn throbber_char(&self) -> char {
        ['⠋', '⠙', '⠹', '⠸'][self.throbber_frame]
    }

    pub fn needs_render(&self) -> bool {
        self.dirty.any()
    }

    pub fn mark_rendered(&mut self) {
        self.dirty.clear();
    }

    pub fn mark_full_redraw(&mut self) {
        self.dirty.full = true;
    }

    /// Adds a message to the conversation timeline and display.
    ///
    /// This updates the unified timeline and sets the dirty flag so the UI
    /// will re-render. Note: This only adds to the display timeline, not
    /// the API messages. Use `add_api_message` to add to the API conversation.
    pub fn add_message(&mut self, message: Message) {
        // Add to unified timeline based on role
        match message.role {
            Role::User => self.timeline.push_user_message(&message.content),
            Role::Assistant => self.timeline.push_assistant_message(&message.content),
        }
        self.dirty.messages = true;
    }

    /// Adds a full API message with content blocks.
    ///
    /// This adds to both the API message history (with full content blocks)
    /// and the display timeline (as text summary).
    pub fn add_api_message(&mut self, message: ApiMessageV2) {
        // Add to display timeline as text summary
        let legacy = message.to_legacy();
        match legacy.role {
            Role::User => self.timeline.push_user_message(&legacy.content),
            Role::Assistant => self.timeline.push_assistant_message(&legacy.content),
        }
        // Add to API messages with full content blocks
        self.api_messages.push(message);
        self.dirty.messages = true;
    }

    /// Returns the API messages for continuation.
    ///
    /// These messages include full content blocks (tool_use, tool_result)
    /// and should be used when sending to the API.
    #[must_use]
    pub fn api_messages(&self) -> &[ApiMessageV2] {
        &self.api_messages
    }

    /// Returns a mutable reference to the API messages.
    pub fn api_messages_mut(&mut self) -> &mut Vec<ApiMessageV2> {
        &mut self.api_messages
    }

    /// Returns the count of API messages.
    #[must_use]
    pub fn api_messages_len(&self) -> usize {
        self.api_messages.len()
    }

    /// Returns API messages truncated to fit within the token budget.
    ///
    /// This should be used when sending messages to the API instead of
    /// `api_messages()` directly to prevent context overflow and control costs.
    ///
    /// The truncation:
    /// - Always preserves the first message (system/project context)
    /// - Prioritizes recent messages over older ones
    /// - Respects the `DEFAULT_MAX_INPUT_TOKENS` limit
    ///
    /// # Returns
    ///
    /// A new vector containing the truncated message history.
    #[must_use]
    pub fn api_messages_truncated(&self) -> Vec<ApiMessageV2> {
        use crate::api::{truncate_context, DEFAULT_MAX_INPUT_TOKENS};
        truncate_context(&self.api_messages, DEFAULT_MAX_INPUT_TOKENS)
    }

    pub async fn submit_message(
        &mut self,
        client: &AnthropicClient,
        content: String,
    ) -> Result<()> {
        // Add to both timeline and API messages
        let user_msg = ApiMessageV2::user(&content);
        self.timeline.push_user_message(&content);
        self.api_messages.push(user_msg);

        self.loading = true;
        // Start streaming in timeline
        if self.timeline.try_push_streaming().is_err() {
            tracing::warn!("Timeline already streaming when submitting message");
        }

        // Initialize tool loop state machine for streaming
        // This must be called BEFORE the API stream starts so tool events are captured
        if let Err(e) = self.tool_loop.start_streaming() {
            tracing::warn!("Failed to start tool loop streaming: {}", e);
            // Reset and try again - the loop might be in an unexpected state
            self.tool_loop.reset();
            self.tool_loop.start_streaming().ok();
        }

        let (tx, rx) = mpsc::channel(100);
        self.streaming_rx = Some(rx);

        // Use truncated api_messages for the API call to control costs
        // while preserving content blocks for tool results
        let total_messages = self.api_messages.len();
        let api_messages = self.api_messages_truncated();
        let truncated_messages = api_messages.len();

        if truncated_messages < total_messages {
            tracing::info!(
                total = total_messages,
                sending = truncated_messages,
                dropped = total_messages - truncated_messages,
                "Context truncated for API call"
            );
        }

        let client = client.clone();
        let tools = default_tools();
        tokio::spawn(async move {
            if let Err(e) = client
                .stream_message_v2_with_tools(
                    &api_messages,
                    Some(&tools),
                    Some(&ToolChoice::Auto),
                    tx,
                )
                .await
            {
                tracing::error!("API error: {}", e);
            }
        });

        Ok(())
    }

    pub async fn recv_api_chunk(&mut self) -> Option<StreamEvent> {
        if let Some(rx) = &mut self.streaming_rx {
            rx.recv().await
        } else {
            std::future::pending::<Option<StreamEvent>>().await
        }
    }

    /// Sets the streaming receiver for API response chunks.
    ///
    /// This is used by the tool execution flow to set up continuation streaming
    /// without blocking the event loop.
    pub fn set_streaming_rx(&mut self, rx: mpsc::Receiver<StreamEvent>) {
        self.streaming_rx = Some(rx);
    }

    /// Sets the loading state.
    ///
    /// When loading is true, the throbber animates and content accumulates.
    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
        self.dirty.messages = true;
    }

    /// Initializes the streaming buffer for continuation streaming.
    ///
    /// This starts a new streaming entry in the timeline with optional initial content.
    pub fn set_current_response(&mut self, response: String) {
        // Start streaming in timeline if not already streaming
        if self.timeline.try_push_streaming().is_ok() && !response.is_empty() {
            self.timeline.append_to_streaming(&response);
        }
        self.dirty.messages = true;
    }

    pub fn append_chunk(&mut self, event: StreamEvent) -> Result<()> {
        match event {
            StreamEvent::ContentDelta(text) => {
                // Update timeline streaming entry
                self.timeline.append_to_streaming(&text);
                // Also forward to tool loop for tracking assistant text
                self.tool_loop.append_text(&text);
                self.dirty.messages = true;
            }
            StreamEvent::MessageStop => {
                // Only process if we're actually streaming (prevents duplicates)
                // MessageComplete may have already handled this
                if self.timeline.is_streaming() {
                    self.timeline.finalize_streaming_as_message();
                    // Get the finalized text for API messages
                    if let Some(crate::types::ConversationEntry::AssistantMessage(text)) =
                        self.timeline.entries().last()
                    {
                        self.api_messages.push(ApiMessageV2::assistant(text));
                    }
                }
                self.loading = false;
                self.streaming_rx = None;
                self.dirty.messages = true;
            }
            StreamEvent::MessageComplete { stop_reason } => {
                // For tool_use stop reasons, the assistant message will be added
                // later by handle_tool_execution with full content blocks
                let needs_tool_execution = stop_reason.needs_tool_execution();

                if needs_tool_execution {
                    // P0-1 FIX: For tool_use responses, finalize streaming for tool use.
                    // The text is already in tool_loop.text_content (via append_text calls).
                    // handle_tool_execution() will build the proper assistant message with
                    // both text AND tool_use blocks, preventing duplicate messages.
                    self.timeline.finalize_streaming_for_tool_use();
                    tracing::debug!(
                        "Tool use response - text stored in tool_loop, not adding to API yet"
                    );
                } else {
                    // For normal responses, finalize streaming and add to API messages
                    self.timeline.finalize_streaming_as_message();
                    if let Some(crate::types::ConversationEntry::AssistantMessage(text)) =
                        self.timeline.entries().last()
                    {
                        self.api_messages.push(ApiMessageV2::assistant(text));
                    }
                }
                // Handle stop reason in tool loop
                self.handle_message_complete(stop_reason)?;
                self.loading = false;
                self.streaming_rx = None;
                self.dirty.messages = true;
            }
            StreamEvent::Error(e) => {
                tracing::error!("Stream error: {}", e);
                self.loading = false;
                self.streaming_rx = None;
                self.dirty.messages = true;
            }
            StreamEvent::ToolUseStart { id, name, index } => {
                self.handle_tool_use_start(id, name, index);
            }
            StreamEvent::ToolUseInputDelta {
                index,
                partial_json,
            } => {
                self.handle_tool_use_input_delta(index, &partial_json);
            }
            StreamEvent::ToolUseComplete { index } => {
                self.handle_tool_use_complete(index)?;
            }
            StreamEvent::ContentBlockComplete { .. } => {
                // Content block completion is tracked internally
                tracing::debug!("Content block complete");
            }
        }
        Ok(())
    }

    // ========================================================================
    // Worktree Status Bar State
    // ========================================================================

    /// Sets the current worktree branch name.
    ///
    /// This is displayed in the status bar.
    pub fn set_worktree_branch(&mut self, branch: String) {
        self.worktree_branch = Some(branch);
        self.dirty.full = true;
    }

    /// Returns the current worktree branch name, if set.
    #[must_use]
    pub fn worktree_branch(&self) -> Option<&str> {
        self.worktree_branch.as_deref()
    }

    /// Sets the number of modified files in the worktree.
    pub fn set_worktree_modified(&mut self, count: usize) {
        self.worktree_modified = count;
        self.dirty.full = true;
    }

    /// Returns the number of modified files in the worktree.
    #[must_use]
    pub fn worktree_modified(&self) -> usize {
        self.worktree_modified
    }

    /// Sets the number of commits ahead of upstream.
    pub fn set_worktree_ahead(&mut self, count: usize) {
        self.worktree_ahead = count;
        self.dirty.full = true;
    }

    /// Returns the number of commits ahead of upstream.
    #[must_use]
    pub fn worktree_ahead(&self) -> usize {
        self.worktree_ahead
    }

    /// Sets the number of commits behind upstream.
    pub fn set_worktree_behind(&mut self, count: usize) {
        self.worktree_behind = count;
        self.dirty.full = true;
    }

    /// Returns the number of commits behind upstream.
    #[must_use]
    pub fn worktree_behind(&self) -> usize {
        self.worktree_behind
    }

    // ========================================================================
    // Token Budget Tracking
    // ========================================================================

    /// Returns a reference to the token budget for display.
    #[must_use]
    pub fn token_budget(&self) -> &TokenBudget {
        &self.token_budget
    }

    /// Returns a mutable reference to the token budget.
    pub fn token_budget_mut(&mut self) -> &mut TokenBudget {
        &mut self.token_budget
    }

    /// Adds token usage to the budget.
    ///
    /// Call this after each API request to track cumulative usage.
    pub fn add_token_usage(&mut self, tokens: usize) {
        self.token_budget.add_usage(tokens);
        self.dirty.full = true;
    }

    /// Resets the token budget for a new conversation.
    pub fn reset_token_budget(&mut self) {
        self.token_budget.reset();
        self.dirty.full = true;
    }

    // ========================================================================
    // Compaction Progress
    // ========================================================================

    /// Returns the compaction progress state, if compaction is active.
    #[must_use]
    pub fn compaction_state(&self) -> Option<&CompactionProgressState> {
        self.compaction_state.as_ref()
    }

    /// Returns a mutable reference to the compaction progress state.
    pub fn compaction_state_mut(&mut self) -> Option<&mut CompactionProgressState> {
        self.compaction_state.as_mut()
    }

    /// Starts a compaction operation with the given target and before tokens.
    ///
    /// This will display the compaction progress overlay in the UI.
    pub fn start_compaction(&mut self, target_tokens: usize, before_tokens: usize) {
        let mut state = CompactionProgressState::new(target_tokens, before_tokens);
        state.set_status(crate::tui::widgets::CompactionStatus::Compacting);
        self.compaction_state = Some(state);
        self.dirty.full = true;
    }

    /// Updates the compaction progress (0.0 to 1.0).
    pub fn update_compaction_progress(&mut self, progress: f64) {
        if let Some(state) = &mut self.compaction_state {
            state.set_progress(progress);
            self.dirty.full = true;
        }
    }

    /// Completes the compaction operation with the final token count.
    pub fn complete_compaction(&mut self, after_tokens: usize) {
        if let Some(state) = &mut self.compaction_state {
            state.set_after_tokens(after_tokens);
            state.set_status(crate::tui::widgets::CompactionStatus::Complete);
            state.set_progress(1.0);
            self.dirty.full = true;
        }
    }

    /// Marks the compaction operation as failed.
    pub fn fail_compaction(&mut self) {
        if let Some(state) = &mut self.compaction_state {
            state.set_status(crate::tui::widgets::CompactionStatus::Failed);
            self.dirty.full = true;
        }
    }

    /// Clears the compaction state (closes the overlay).
    pub fn clear_compaction(&mut self) {
        self.compaction_state = None;
        self.dirty.full = true;
    }

    // ========================================================================
    // Session Restoration and Auto-Save
    // ========================================================================

    /// Returns the current session ID, if one has been assigned.
    ///
    /// A session ID is assigned when the session is first saved, or when
    /// restoring from a previous session.
    #[must_use]
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Sets the session ID.
    ///
    /// This is called after saving a session or when restoring from one.
    pub fn set_session_id(&mut self, id: String) {
        self.session_id = Some(id);
    }

    /// Creates a `Session` from the current application state.
    ///
    /// The resulting session includes:
    /// - All conversation messages (converted from timeline)
    /// - Current UI state (scroll position, input buffer, cursor position)
    /// - Working directory
    ///
    /// This is used for auto-save functionality.
    #[must_use]
    pub fn to_session(&self) -> Session {
        use crate::session::UiState;

        let mut session = Session::new(self.working_dir.clone());

        // Convert timeline entries to messages for session persistence
        for entry in self.timeline.iter() {
            match entry {
                crate::types::ConversationEntry::UserMessage(text) => {
                    session.add_message(Message {
                        role: Role::User,
                        content: text.clone(),
                    });
                }
                crate::types::ConversationEntry::AssistantMessage(text) => {
                    session.add_message(Message {
                        role: Role::Assistant,
                        content: text.clone(),
                    });
                }
                // Skip streaming and tool execution entries for session persistence
                _ => {}
            }
        }

        // Capture UI state (use scroll offset for backward compatibility)
        let ui_state =
            UiState::with_state(self.scroll.offset(), self.input.clone(), self.cursor_pos);
        session.set_ui_state(Some(ui_state));

        session
    }

    /// Restores application state from a saved session.
    ///
    /// This restores:
    /// - Message history (to timeline)
    /// - UI state (scroll position, input buffer, cursor position) if saved
    /// - Session ID for subsequent saves
    ///
    /// # Arguments
    ///
    /// * `session` - The session to restore from.
    pub fn restore_from_session(&mut self, session: &Session) {
        // Clear and rebuild timeline from session messages
        self.timeline = Timeline::new();
        for message in session.messages() {
            match message.role {
                Role::User => self.timeline.push_user_message(&message.content),
                Role::Assistant => self.timeline.push_assistant_message(&message.content),
            }
        }

        // Restore UI state if available
        if let Some(ui_state) = session.ui_state() {
            self.scroll.restore_offset(ui_state.scroll_offset());
            self.input = ui_state.input_buffer().to_string();
            self.cursor_pos = ui_state.cursor_position();
        }

        // Restore session ID if available
        if let Some(id) = session.id() {
            self.session_id = Some(id.to_string());
        }

        // Mark for full redraw
        self.dirty.full = true;
    }

    // ========================================================================
    // Tool Execution Integration
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
        self.dirty.full = true;
    }

    /// Clears the pending permission request.
    pub fn clear_pending_permission(&mut self) {
        self.pending_permission = None;
        self.dirty.full = true;
    }

    /// Handles a permission response from the user.
    ///
    /// This grants or denies permission for the pending tool call and
    /// updates the permission manager accordingly.
    pub async fn handle_permission_response(&mut self, response: PermissionResponse) {
        if let Some(request) = self.pending_permission.take() {
            let mut manager = self.permission_manager.lock().await;
            manager.handle_response(&request.tool_name, request.tool_input.as_deref(), response);
            self.dirty.full = true;
        }
    }

    /// Handles a tool_use stream event.
    ///
    /// Routes the event to the tool loop state machine.
    pub fn handle_tool_use_start(&mut self, id: String, name: String, index: usize) {
        self.tool_loop.start_tool_use(index, id, name);
        self.dirty.messages = true;
    }

    /// Handles a tool_use input delta.
    pub fn handle_tool_use_input_delta(&mut self, index: usize, partial_json: &str) {
        self.tool_loop.append_tool_input(index, partial_json);
    }

    /// Handles tool_use completion.
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
    pub fn handle_message_complete(&mut self, stop_reason: StopReason) -> Result<()> {
        self.tool_loop
            .message_complete(stop_reason)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(())
    }

    /// Executes all pending tools that have been approved.
    ///
    /// Creates tool blocks for UI display, executes the tools, and
    /// updates the blocks with results.
    ///
    /// Returns a list of tool IDs that need permission (if any).
    ///
    /// # Errors
    ///
    /// Returns an error if the tool loop is not in `Executing` state.
    pub async fn execute_pending_tools(&mut self) -> Result<Vec<String>> {
        use crate::app::tool_loop::ToolLoopError;
        use std::collections::HashMap;

        // Collect tool info before creating blocks (to avoid borrow issues)
        let tools_to_display: Vec<(String, String, String)> = self
            .tool_loop
            .pending_calls()
            .iter()
            .filter(|(_, call)| call.approved && !call.executed)
            .map(|(tool_id, call)| {
                let input_str = format_tool_input(&call.tool_use.name, &call.tool_use.input);
                (tool_id.clone(), call.tool_use.name.clone(), input_str)
            })
            .collect();

        // Create tool blocks for pending tools (before execution)
        let mut tool_id_to_block_index: HashMap<String, usize> = HashMap::new();
        for (tool_id, tool_name, input_str) in tools_to_display {
            let index = self.start_tool_block(&tool_name, &input_str);
            tool_id_to_block_index.insert(tool_id, index);
        }

        // Execute the tools
        let result = self
            .tool_loop
            .execute_pending(&self.tool_executor)
            .await
            .map_err(|e| match e {
                ToolLoopError::InvalidStateTransition { from, to } => {
                    anyhow::anyhow!("Invalid state transition from {} to {}", from, to)
                }
                _ => anyhow::anyhow!("{}", e),
            })?;

        // Collect results (to avoid borrow issues)
        let results: Vec<(String, String, bool)> = self
            .tool_loop
            .pending_calls()
            .iter()
            .filter_map(|(tool_id, call)| {
                if let Some(result_block) = &call.result {
                    if tool_id_to_block_index.contains_key(tool_id) {
                        return Some((
                            tool_id.clone(),
                            result_block.content.clone(),
                            result_block.is_error,
                        ));
                    }
                }
                None
            })
            .collect();

        // Update tool blocks with results
        for (tool_id, content, is_error) in results {
            if let Some(&block_index) = tool_id_to_block_index.get(&tool_id) {
                self.complete_tool_block(block_index, &content, is_error);
            }
        }

        Ok(result)
    }

    /// Finishes tool execution and returns continuation data.
    ///
    /// The continuation data contains the messages needed to continue
    /// the conversation with Claude.
    pub fn finish_tool_execution(&mut self) -> Result<ContinuationData> {
        self.tool_loop
            .finish_execution()
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// Approves all pending tools for execution.
    pub fn approve_all_tools(&mut self) -> Result<()> {
        self.tool_loop
            .approve_all()
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// Denies all pending tools.
    pub fn deny_all_tools(&mut self) -> Result<()> {
        self.tool_loop
            .deny_all()
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// Resets the tool loop to idle state.
    pub fn reset_tool_loop(&mut self) {
        self.tool_loop.reset();
        self.pending_permission = None;
        self.dirty.full = true;
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

    // ========================================================================
    // Tool Block UI Methods (Phase 10.5.6)
    // ========================================================================

    /// Starts a new tool block for UI display.
    ///
    /// Returns the index of the created block for later updates.
    ///
    /// # Arguments
    ///
    /// * `tool_name` - Name of the tool (e.g., "bash", "read")
    /// * `tool_input` - Input provided to the tool
    pub fn start_tool_block(&mut self, tool_name: &str, tool_input: &str) -> usize {
        let block = ToolBlockState::new(tool_name, tool_input);
        self.tool_blocks.push(block);
        self.dirty.messages = true;
        self.tool_blocks.len() - 1
    }

    /// Completes a tool block with its result.
    ///
    /// # Arguments
    ///
    /// * `index` - Index of the tool block to update
    /// * `result` - The tool's output
    /// * `is_error` - Whether the result is an error
    pub fn complete_tool_block(&mut self, index: usize, result: &str, is_error: bool) {
        if let Some(block) = self.tool_blocks.get_mut(index) {
            if is_error {
                block.set_error(result);
            } else {
                block.set_result(result);
            }
            self.dirty.messages = true;
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
        self.dirty.messages = true;
    }

    /// Returns true if there are any tool blocks to display.
    #[must_use]
    pub fn has_tool_blocks(&self) -> bool {
        !self.tool_blocks.is_empty()
    }

    // ========================================================================
    // Timeline Integration (Phase 2)
    // ========================================================================

    /// Returns a reference to the conversation timeline.
    #[must_use]
    pub fn timeline(&self) -> &Timeline {
        &self.timeline
    }

    /// Returns a mutable reference to the conversation timeline.
    pub fn timeline_mut(&mut self) -> &mut Timeline {
        &mut self.timeline
    }

    /// Starts streaming mode.
    ///
    /// This creates a streaming entry in the timeline.
    pub fn set_streaming(&mut self, _streaming: bool) {
        self.loading = true;

        // Add streaming entry to timeline
        if self.timeline.try_push_streaming().is_err() {
            // Already streaming - this is a no-op
            tracing::warn!("set_streaming called but timeline already streaming");
        }

        self.dirty.messages = true;
    }

    /// Appends text to the current streaming response.
    ///
    /// Updates the timeline streaming entry.
    pub fn append_streaming_text(&mut self, text: &str) {
        // Update timeline streaming entry
        self.timeline.append_to_streaming(text);
        self.dirty.messages = true;
    }

    /// Finalizes streaming as a complete assistant message.
    ///
    /// Converts the streaming entry to an assistant message in the timeline.
    pub fn finalize_streaming_as_message(&mut self) {
        // Finalize timeline streaming entry
        self.timeline.finalize_streaming_as_message();
        self.loading = false;
        self.dirty.messages = true;
    }

    /// Adds a tool block with result to both legacy tool_blocks and timeline.
    ///
    /// # Arguments
    ///
    /// * `tool_name` - Name of the tool (e.g., "bash", "read_file")
    /// * `input` - Tool input/command
    /// * `output` - Tool output
    /// * `is_error` - Whether the execution resulted in an error
    pub fn add_tool_block_with_result(
        &mut self,
        tool_name: &str,
        input: &str,
        output: &str,
        is_error: bool,
    ) {
        // Add to legacy tool_blocks using the constructor
        let mut block = ToolBlockState::new(tool_name, input);
        if is_error {
            block.set_error(output);
        } else {
            block.set_result(output);
        }
        self.tool_blocks.push(block);

        // Add to timeline with message index tracking
        self.timeline.push_tool_after_current_assistant(
            tool_name,
            input,
            Some(output.to_string()),
            is_error,
        );

        self.dirty.messages = true;
    }

    /// Clears all conversation state (timeline, API messages, tool blocks).
    pub fn clear_conversation(&mut self) {
        self.api_messages.clear();
        self.tool_blocks.clear();
        self.timeline = Timeline::new();
        self.dirty.messages = true;
    }

    // ========================================================================
    // Async Tool Execution (Phase 5)
    // ========================================================================

    /// Sets the receiver channel for async tool results.
    ///
    /// When tool execution is spawned in the background, results will be
    /// streamed back through this channel.
    pub fn set_tool_result_rx(
        &mut self,
        rx: mpsc::UnboundedReceiver<(String, crate::types::ToolResultBlock)>,
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

    /// Adds a pending tool to the tool loop.
    ///
    /// # Arguments
    ///
    /// * `tool_use` - The tool use block to add
    pub fn add_pending_tool(&mut self, tool_use: crate::types::ToolUseBlock) {
        self.tool_loop.add_tool_use(tool_use);
    }

    /// Spawns tool execution in the background.
    ///
    /// Returns immediately with a handle to the background task.
    /// Results are sent through the tool_result_rx channel.
    ///
    /// # Returns
    ///
    /// `Some(JoinHandle)` if tools were spawned, `None` if no tools pending.
    #[must_use]
    pub fn spawn_tool_execution(
        &mut self,
    ) -> Option<tokio::task::JoinHandle<Vec<(String, crate::types::ToolResultBlock)>>> {
        // Create channel for results
        let (tx, rx) = mpsc::unbounded_channel();
        self.tool_result_rx = Some(rx);

        // Get pending tools
        let pending: Vec<_> = self
            .tool_loop
            .pending_calls()
            .iter()
            .filter(|(_, call)| !call.executed)
            .map(|(id, call)| (id.clone(), call.tool_use.clone()))
            .collect();

        if pending.is_empty() {
            return None;
        }

        // Mark all as executing
        for (id, _) in &pending {
            self.executing_tool_ids.insert(id.clone());
        }

        let executor = Arc::clone(&self.tool_executor);

        // Spawn background task
        let handle = tokio::spawn(async move {
            use crate::app::tool_loop::tool_use_to_call;
            use crate::tools::ToolResult as TR;

            let mut results = Vec::new();
            for (tool_id, tool_use) in pending {
                let call = tool_use_to_call(&tool_use);
                let result = executor.execute(call).await;

                let result_block = match &result {
                    Ok(TR::Success(output)) => crate::types::ToolResultBlock {
                        tool_use_id: tool_id.clone(),
                        content: output.clone(),
                        is_error: false,
                    },
                    Ok(TR::Error(error)) => crate::types::ToolResultBlock {
                        tool_use_id: tool_id.clone(),
                        content: error.clone(),
                        is_error: true,
                    },
                    Ok(TR::Cancelled) => crate::types::ToolResultBlock {
                        tool_use_id: tool_id.clone(),
                        content: "Tool execution cancelled".to_string(),
                        is_error: true,
                    },
                    Ok(TR::NeedsPermission(perm)) => crate::types::ToolResultBlock {
                        tool_use_id: tool_id.clone(),
                        content: format!("Permission required: {perm:?}"),
                        is_error: true,
                    },
                    Err(e) => crate::types::ToolResultBlock {
                        tool_use_id: tool_id.clone(),
                        content: e.to_string(),
                        is_error: true,
                    },
                };

                // Send through channel (ignore error if receiver dropped)
                let _ = tx.send((tool_id.clone(), result_block.clone()));
                results.push((tool_id, result_block));
            }
            results
        });

        Some(handle)
    }

    /// Returns true if there are any tools currently executing.
    #[must_use]
    pub fn has_executing_tools(&self) -> bool {
        !self.executing_tool_ids.is_empty()
    }

    /// Marks a tool as currently executing.
    ///
    /// # Arguments
    ///
    /// * `tool_id` - The ID of the tool to mark as executing
    pub fn mark_tool_executing(&mut self, tool_id: &str) {
        self.executing_tool_ids.insert(tool_id.to_string());
    }

    /// Records a tool result and removes the tool from executing set.
    ///
    /// # Arguments
    ///
    /// * `tool_id` - The ID of the tool that completed
    /// * `result` - The result of the tool execution
    pub fn record_tool_result(&mut self, tool_id: &str, result: crate::types::ToolResultBlock) {
        // Remove from executing set
        self.executing_tool_ids.remove(tool_id);

        // Update tool loop with result (ignore error if tool not found)
        let _ = self.tool_loop.set_tool_result(tool_id, result.clone());

        // Update timeline tool entry if it exists
        self.update_timeline_tool_by_id(tool_id, Some(result.content), result.is_error);

        self.dirty.messages = true;
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

    /// Adds a tool to the timeline in executing state (no output yet).
    ///
    /// # Arguments
    ///
    /// * `tool_name` - Name of the tool (e.g., "bash")
    /// * `input` - The tool input/command
    pub fn add_tool_to_timeline_executing(&mut self, tool_name: &str, input: &str) {
        self.timeline.push_tool_after_current_assistant(
            tool_name, input, None, // No output yet - executing
            false,
        );
        self.dirty.messages = true;
    }

    /// Updates a tool in the timeline with its result.
    ///
    /// Finds the most recent tool entry with the given name that has no output
    /// and updates it with the provided output.
    ///
    /// # Arguments
    ///
    /// * `tool_name` - Name of the tool to update
    /// * `output` - The tool output (None if still executing)
    /// * `is_error` - Whether the result is an error
    pub fn update_tool_in_timeline(
        &mut self,
        tool_name: &str,
        output: Option<String>,
        is_error: bool,
    ) {
        self.timeline
            .update_tool_result(tool_name, output, is_error);
        self.dirty.messages = true;
    }

    /// Updates a tool in the timeline by its ID.
    ///
    /// This is used internally when recording tool results.
    fn update_timeline_tool_by_id(
        &mut self,
        _tool_id: &str,
        output: Option<String>,
        is_error: bool,
    ) {
        // For now, update the most recent executing tool
        // In the future, we could track tool_id -> timeline_index mapping
        for entry in self.timeline.entries_mut().iter_mut().rev() {
            if let crate::types::ConversationEntry::ToolExecution {
                output: ref mut o @ None,
                is_error: ref mut err,
                ..
            } = entry
            {
                *o = output;
                *err = is_error;
                break;
            }
        }
        self.dirty.messages = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::UiState;

    fn test_message(role: Role, content: &str) -> Message {
        Message {
            role,
            content: content.to_string(),
        }
    }

    #[test]
    fn test_app_state_new() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        assert!(state.timeline().is_empty());
        assert!(state.input.is_empty());
        assert_eq!(state.scroll_offset(), 0);
        assert_eq!(state.working_dir, PathBuf::from("/test"));
    }

    #[test]
    fn test_restore_from_session_messages() {
        use crate::types::ConversationEntry;
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Create a session with messages
        let mut session = Session::new(PathBuf::from("/project"));
        session.add_message(test_message(Role::User, "Hello"));
        session.add_message(test_message(Role::Assistant, "Hi there!"));

        state.restore_from_session(&session);

        assert_eq!(state.timeline().len(), 2);
        let entries: Vec<_> = state.timeline().iter().collect();
        assert!(matches!(entries[0], ConversationEntry::UserMessage(s) if s == "Hello"));
        assert!(matches!(entries[1], ConversationEntry::AssistantMessage(s) if s == "Hi there!"));
    }

    #[test]
    fn test_restore_from_session_with_ui_state() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Create a session with UI state
        let mut session = Session::new(PathBuf::from("/project"));
        session.add_message(test_message(Role::User, "Test"));
        session.set_ui_state(Some(UiState::with_state(50, "draft input".to_string(), 5)));

        state.restore_from_session(&session);

        assert_eq!(state.scroll_offset(), 50);
        assert_eq!(state.input, "draft input");
        assert_eq!(state.cursor_position(), 5);
    }

    #[test]
    fn test_restore_from_session_without_ui_state() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        // Set some initial state
        state.scroll.restore_offset(100);
        state.input = "existing".to_string();
        state.cursor_pos = 8;

        // Create a session without UI state
        let mut session = Session::new(PathBuf::from("/project"));
        session.add_message(test_message(Role::User, "Test"));

        state.restore_from_session(&session);

        // UI state should remain unchanged since session has no UI state
        assert_eq!(state.scroll_offset(), 100);
        assert_eq!(state.input, "existing");
        assert_eq!(state.cursor_position(), 8);
    }

    #[test]
    fn test_restore_marks_dirty() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.mark_rendered(); // Clear dirty flags

        let session = Session::new(PathBuf::from("/project"));
        state.restore_from_session(&session);

        assert!(state.needs_render());
    }

    // ========================================================================
    // Phase 10.4.1: Auto-save tests
    // ========================================================================

    #[test]
    fn test_app_state_session_id_none_initially() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        assert!(state.session_id().is_none());
    }

    #[test]
    fn test_app_state_set_session_id() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.set_session_id("abc123".to_string());
        assert_eq!(state.session_id(), Some("abc123"));
    }

    #[test]
    fn test_to_session_empty() {
        let state = AppState::new(PathBuf::from("/project"), false, ParallelMode::Enabled);
        let session = state.to_session();

        assert!(session.messages().is_empty());
        assert_eq!(session.working_dir(), &PathBuf::from("/project"));
    }

    #[test]
    fn test_to_session_with_messages() {
        let mut state = AppState::new(PathBuf::from("/project"), false, ParallelMode::Enabled);
        state.add_message(test_message(Role::User, "Hello"));
        state.add_message(test_message(Role::Assistant, "Hi!"));

        let session = state.to_session();

        assert_eq!(session.messages().len(), 2);
        assert_eq!(session.messages()[0].content, "Hello");
        assert_eq!(session.messages()[1].content, "Hi!");
    }

    #[test]
    fn test_to_session_preserves_ui_state() {
        let mut state = AppState::new(PathBuf::from("/project"), false, ParallelMode::Enabled);
        state.scroll.restore_offset(42);
        state.input = "draft text".to_string();
        state.cursor_pos = 5;

        let session = state.to_session();
        let ui_state = session.ui_state().expect("UI state should be present");

        assert_eq!(ui_state.scroll_offset(), 42);
        assert_eq!(ui_state.input_buffer(), "draft text");
        assert_eq!(ui_state.cursor_position(), 5);
    }

    #[test]
    fn test_to_session_roundtrip() {
        // Create state with data
        let mut state = AppState::new(PathBuf::from("/project"), false, ParallelMode::Enabled);
        state.add_message(test_message(Role::User, "Test message"));
        state.scroll.restore_offset(100);
        state.input = "unsent input".to_string();
        state.cursor_pos = 6;

        // Convert to session
        let session = state.to_session();

        // Create new state and restore
        let mut new_state =
            AppState::new(PathBuf::from("/different"), false, ParallelMode::Enabled);
        new_state.restore_from_session(&session);

        // Verify roundtrip preserves data
        assert_eq!(new_state.timeline().len(), 1);
        let entries: Vec<_> = new_state.timeline().iter().collect();
        assert!(
            matches!(entries[0], crate::types::ConversationEntry::UserMessage(s) if s == "Test message")
        );
        assert_eq!(new_state.scroll_offset(), 100);
        assert_eq!(new_state.input, "unsent input");
        assert_eq!(new_state.cursor_position(), 6);
    }

    #[test]
    fn test_restore_from_session_restores_session_id() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        assert!(state.session_id().is_none());

        // Create session with an ID (simulating a saved session)
        let mut session = Session::new(PathBuf::from("/project"));
        session.add_message(test_message(Role::User, "Test"));
        // Manually set the session ID via JSON (normally done by SessionManager::save)
        let session_json = serde_json::to_string(&session).unwrap();
        let json_with_id = session_json.replace(r#""id":null"#, r#""id":"test-session-id""#);
        let session_with_id: Session = serde_json::from_str(&json_with_id).unwrap();

        state.restore_from_session(&session_with_id);

        assert_eq!(state.session_id(), Some("test-session-id"));
    }

    // ========================================================================
    // Tool Loop Integration Tests (Phase 10.5.2.4)
    // ========================================================================

    #[test]
    fn test_appstate_has_tool_loop() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        assert!(matches!(state.tool_loop_state(), ToolLoopState::Idle));
    }

    #[test]
    fn test_appstate_receives_tool_use() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Start streaming
        state.tool_loop_mut().start_streaming().unwrap();

        // Simulate receiving tool use events
        state.handle_tool_use_start("toolu_123".to_string(), "bash".to_string(), 0);
        state.handle_tool_use_input_delta(0, r#"{"command":"ls"}"#);
        state.handle_tool_use_complete(0).unwrap();

        // Complete the message with tool_use stop reason
        state.handle_message_complete(StopReason::ToolUse).unwrap();

        // Should be in PendingApproval state
        assert!(matches!(
            state.tool_loop_state(),
            ToolLoopState::PendingApproval
        ));

        // Should need user action
        assert!(state.tool_loop_needs_user_action());
    }

    #[test]
    fn test_appstate_approve_and_deny_tools() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Set up tool use
        state.tool_loop_mut().start_streaming().unwrap();
        state.handle_tool_use_start("toolu_1".to_string(), "bash".to_string(), 0);
        state.handle_tool_use_input_delta(0, "{}");
        state.handle_tool_use_complete(0).unwrap();
        state.handle_message_complete(StopReason::ToolUse).unwrap();

        // Deny all
        state.deny_all_tools().unwrap();
        assert!(matches!(state.tool_loop_state(), ToolLoopState::Idle));
    }

    #[test]
    fn test_appstate_pending_permission() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        assert!(!state.has_pending_permission());
        assert!(state.pending_permission().is_none());

        // Set a pending permission
        let request = PermissionRequest::new("bash", Some("rm -rf temp"), "Execute command");
        state.set_pending_permission(request);

        assert!(state.has_pending_permission());
        assert!(state.pending_permission().is_some());
        let pending = state.pending_permission().unwrap();
        assert_eq!(pending.tool_name, "bash");

        // Clear it
        state.clear_pending_permission();
        assert!(!state.has_pending_permission());
    }

    #[tokio::test]
    async fn test_appstate_handles_permission_response() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Set a pending permission
        let request = PermissionRequest::new("bash", Some("echo hello"), "Execute command");
        state.set_pending_permission(request);

        // Handle the response (Allow Once)
        state
            .handle_permission_response(PermissionResponse::AllowOnce)
            .await;

        // Permission should be cleared
        assert!(!state.has_pending_permission());
    }

    #[test]
    fn test_appstate_reset_tool_loop() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Set up some state
        state.tool_loop_mut().start_streaming().unwrap();
        state.handle_tool_use_start("toolu_1".to_string(), "bash".to_string(), 0);
        let request = PermissionRequest::new("bash", None, "test");
        state.set_pending_permission(request);

        // Reset
        state.reset_tool_loop();

        assert!(matches!(state.tool_loop_state(), ToolLoopState::Idle));
        assert!(!state.has_pending_permission());
    }

    #[test]
    fn test_appstate_tool_loop_state_helpers() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Initially idle - needs user action
        assert!(state.tool_loop_needs_user_action());
        assert!(!state.tool_loop_is_active());

        // Start streaming - active
        state.tool_loop_mut().start_streaming().unwrap();
        assert!(!state.tool_loop_needs_user_action());
        assert!(state.tool_loop_is_active());
    }

    // ========================================================================
    // Scroll State Integration Tests (Phase 10.5.4.2)
    // ========================================================================

    #[test]
    fn test_scroll_state_initial() {
        use crate::tui::scroll::AutoScrollMode;

        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Should start in Follow mode at offset 0
        assert_eq!(state.scroll_offset(), 0);
        assert_eq!(state.scroll_state().mode(), AutoScrollMode::Follow);
    }

    #[test]
    fn test_streaming_content_auto_scrolls() {
        use crate::tui::scroll::AutoScrollMode;

        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.set_viewport_height(20);

        // Simulate content growth (streaming updates)
        state.update_content_height(30);
        assert_eq!(state.scroll_offset(), 0); // At bottom

        // More content arrives
        state.update_content_height(50);

        // In Follow mode, should auto-scroll to stay at bottom
        assert_eq!(state.scroll_offset(), 0);
        assert_eq!(state.scroll_state().mode(), AutoScrollMode::Follow);
    }

    #[test]
    fn test_user_scroll_preserved_during_streaming() {
        use crate::tui::scroll::AutoScrollMode;

        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.set_viewport_height(20);
        state.update_content_height(50);

        // User scrolls up
        state.scroll_up(15);
        assert_eq!(state.scroll_offset(), 15);
        assert_eq!(state.scroll_state().mode(), AutoScrollMode::Manual);

        // More content arrives (streaming)
        state.update_content_height(80);

        // User's scroll position should be preserved
        assert_eq!(state.scroll_offset(), 15);
        assert_eq!(state.scroll_state().mode(), AutoScrollMode::Manual);
    }

    #[test]
    fn test_scroll_down_resumes_follow_mode() {
        use crate::tui::scroll::AutoScrollMode;

        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.set_viewport_height(20);
        state.update_content_height(50);

        // User scrolls up (switches to Manual)
        state.scroll_up(20);
        assert_eq!(state.scroll_state().mode(), AutoScrollMode::Manual);

        // User scrolls all the way back down
        state.scroll_down(20);

        // Should resume Follow mode
        assert_eq!(state.scroll_offset(), 0);
        assert_eq!(state.scroll_state().mode(), AutoScrollMode::Follow);
    }

    #[test]
    fn test_scroll_to_bottom_method() {
        use crate::tui::scroll::AutoScrollMode;

        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.set_viewport_height(20);
        state.update_content_height(50);

        // User scrolls up
        state.scroll_up(30);
        assert_eq!(state.scroll_state().mode(), AutoScrollMode::Manual);

        // Explicitly scroll to bottom
        state.scroll_to_bottom(80);

        // Should be in Follow mode at bottom
        assert_eq!(state.scroll_offset(), 0);
        assert_eq!(state.scroll_state().mode(), AutoScrollMode::Follow);
    }

    #[test]
    fn test_scroll_state_accessor() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Should be able to access scroll state
        let scroll = state.scroll_state();
        assert_eq!(scroll.offset(), 0);
    }

    // ========================================================================
    // Tool Block UI Tests (Phase 10.5.6)
    // ========================================================================

    #[test]
    fn test_tool_blocks_initially_empty() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        assert!(state.tool_blocks().is_empty());
        assert!(!state.has_tool_blocks());
    }

    #[test]
    fn test_start_tool_block() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        let index = state.start_tool_block("bash", "git status");

        assert_eq!(index, 0);
        assert!(state.has_tool_blocks());
        assert_eq!(state.tool_blocks().len(), 1);

        let block = &state.tool_blocks()[0];
        assert_eq!(block.tool_name(), "bash");
        assert_eq!(block.tool_input(), "git status");
        assert!(!block.is_complete());
    }

    #[test]
    fn test_complete_tool_block_success() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        let index = state.start_tool_block("bash", "echo hello");

        state.complete_tool_block(index, "hello", false);

        let block = &state.tool_blocks()[0];
        assert!(block.is_complete());
        assert_eq!(block.result(), Some("hello"));
        assert!(!block.is_error());
    }

    #[test]
    fn test_complete_tool_block_error() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        let index = state.start_tool_block("bash", "bad-command");

        state.complete_tool_block(index, "Command not found", true);

        let block = &state.tool_blocks()[0];
        assert!(block.is_complete());
        assert_eq!(block.result(), Some("Command not found"));
        assert!(block.is_error());
    }

    #[test]
    fn test_multiple_tool_blocks() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        let idx1 = state.start_tool_block("bash", "ls");
        let idx2 = state.start_tool_block("read", "/tmp/file.txt");

        assert_eq!(idx1, 0);
        assert_eq!(idx2, 1);
        assert_eq!(state.tool_blocks().len(), 2);

        state.complete_tool_block(idx1, "file1.txt\nfile2.txt", false);
        state.complete_tool_block(idx2, "file contents", false);

        assert!(state.tool_blocks()[0].is_complete());
        assert!(state.tool_blocks()[1].is_complete());
    }

    #[test]
    fn test_clear_tool_blocks() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        state.start_tool_block("bash", "ls");
        state.start_tool_block("read", "/tmp/test");
        assert_eq!(state.tool_blocks().len(), 2);

        state.clear_tool_blocks();

        assert!(state.tool_blocks().is_empty());
        assert!(!state.has_tool_blocks());
    }

    #[test]
    fn test_complete_invalid_index_is_safe() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Completing a non-existent index should not panic
        state.complete_tool_block(999, "result", false);

        assert!(state.tool_blocks().is_empty());
    }

    // ========================================================================
    // Context Truncation Integration Tests (Cost Optimization)
    // ========================================================================

    #[test]
    fn test_api_messages_truncated_returns_truncated() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Add many messages to potentially exceed budget
        for i in 0..50 {
            state
                .api_messages
                .push(ApiMessageV2::user(format!("Message {}", i)));
        }

        let truncated = state.api_messages_truncated();

        // Should return messages (truncation logic is in api::context)
        assert!(!truncated.is_empty());
        // First message preserved
        assert_eq!(truncated[0].content.to_text(), "Message 0");
        // Most recent preserved
        assert_eq!(truncated.last().unwrap().content.to_text(), "Message 49");
    }

    #[test]
    fn test_api_messages_truncated_under_budget_unchanged() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        state.api_messages.push(ApiMessageV2::user("Hello"));
        state
            .api_messages
            .push(ApiMessageV2::assistant("Hi there!"));

        let truncated = state.api_messages_truncated();

        // Under budget - should be unchanged
        assert_eq!(truncated.len(), 2);
        assert_eq!(truncated[0].content.to_text(), "Hello");
        assert_eq!(truncated[1].content.to_text(), "Hi there!");
    }

    #[test]
    fn test_api_messages_truncated_empty() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        let truncated = state.api_messages_truncated();

        assert!(truncated.is_empty());
    }

    #[test]
    fn test_api_messages_truncated_with_large_content() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Add first message (will be preserved)
        state.api_messages.push(ApiMessageV2::user("System prompt"));

        // Add many large messages that would exceed 100k tokens
        let large_content = "x".repeat(10_000); // ~2500 tokens each
        for _ in 0..50 {
            // 50 * 2500 = 125k tokens
            state
                .api_messages
                .push(ApiMessageV2::assistant(&large_content));
        }

        let truncated = state.api_messages_truncated();

        // Should be fewer than 51 messages
        assert!(
            truncated.len() < 51,
            "Should be truncated, got {}",
            truncated.len()
        );
        // First message always preserved
        assert_eq!(truncated[0].content.to_text(), "System prompt");
    }

    // =========================================================================
    // Focus Area Tests
    // =========================================================================

    #[test]
    fn test_focus_area_default_is_input() {
        use crate::tui::selection::FocusArea;
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        assert_eq!(state.focus_area(), FocusArea::Input);
    }

    #[test]
    fn test_focus_area_can_be_set() {
        use crate::tui::selection::FocusArea;
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        state.set_focus_area(FocusArea::Content);
        assert_eq!(state.focus_area(), FocusArea::Content);

        state.set_focus_area(FocusArea::Input);
        assert_eq!(state.focus_area(), FocusArea::Input);
    }

    #[test]
    fn test_focus_change_clears_selection() {
        use crate::tui::selection::{ContentPosition, FocusArea};
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Create a selection
        state.selection_mut().start(ContentPosition::new(0, 0));
        state.selection_mut().update(ContentPosition::new(5, 10));
        state.selection_mut().end();
        assert!(state.selection().has_selection());

        // Change focus should clear selection
        state.set_focus_area(FocusArea::Content);
        assert!(!state.selection().has_selection());
    }

    #[test]
    fn test_focus_same_area_preserves_selection() {
        use crate::tui::selection::{ContentPosition, FocusArea};
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Set focus to content
        state.set_focus_area(FocusArea::Content);

        // Create a selection
        state.selection_mut().start(ContentPosition::new(0, 0));
        state.selection_mut().update(ContentPosition::new(5, 10));
        state.selection_mut().end();
        assert!(state.selection().has_selection());

        // Setting same focus should NOT clear selection
        state.set_focus_area(FocusArea::Content);
        assert!(state.selection().has_selection());
    }

    #[test]
    fn test_focus_area_for_row_content() {
        use crate::tui::selection::FocusArea;
        // Terminal height 30: input is rows 27-29, content is 0-26
        assert_eq!(AppState::focus_area_for_row(0, 30), FocusArea::Content);
        assert_eq!(AppState::focus_area_for_row(10, 30), FocusArea::Content);
        assert_eq!(AppState::focus_area_for_row(26, 30), FocusArea::Content);
    }

    #[test]
    fn test_focus_area_for_row_input() {
        use crate::tui::selection::FocusArea;
        // Terminal height 30: input is rows 27-29
        assert_eq!(AppState::focus_area_for_row(27, 30), FocusArea::Input);
        assert_eq!(AppState::focus_area_for_row(28, 30), FocusArea::Input);
        assert_eq!(AppState::focus_area_for_row(29, 30), FocusArea::Input);
    }

    #[test]
    fn test_focus_area_for_row_small_terminal() {
        use crate::tui::selection::FocusArea;
        // Minimum terminal height 7: content rows 0-3, input rows 4-6
        assert_eq!(AppState::focus_area_for_row(0, 7), FocusArea::Content);
        assert_eq!(AppState::focus_area_for_row(3, 7), FocusArea::Content);
        assert_eq!(AppState::focus_area_for_row(4, 7), FocusArea::Input);
        assert_eq!(AppState::focus_area_for_row(6, 7), FocusArea::Input);
    }
}
