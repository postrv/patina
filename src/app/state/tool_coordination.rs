//! Tool execution coordination methods for AppState.
//!
//! This module contains thin wrappers on `AppState` that delegate to
//! `ToolExecutionState` and manage dirty flags and cross-cutting concerns
//! (timeline, display, API messages). The core tool logic lives in
//! `tool_execution.rs` on `impl ToolExecutionState`.

use super::*;

impl AppState {
    // ========================================================================
    // Tool Execution Integration (thin wrappers around ToolExecutionState)
    // ========================================================================

    /// Sets a pending permission request.
    ///
    /// The UI should display this as a modal prompt.
    pub fn set_pending_permission(&mut self, request: PermissionRequest) {
        self.tool_state.set_pending_permission(request);
        self.dirty.full = true;
    }

    /// Clears the pending permission request.
    pub fn clear_pending_permission(&mut self) {
        self.tool_state.clear_pending_permission();
        self.dirty.full = true;
    }

    // --- Plan review state ---

    /// Returns the pending plan, if any.
    #[must_use]
    pub fn pending_plan(&self) -> Option<&PlanState> {
        self.pending_plan.as_ref()
    }

    /// Returns a mutable reference to the pending plan, if any.
    #[must_use]
    pub fn pending_plan_mut(&mut self) -> Option<&mut PlanState> {
        self.pending_plan.as_mut()
    }

    /// Returns true if there's a pending plan awaiting user review.
    #[must_use]
    pub fn has_pending_plan(&self) -> bool {
        self.pending_plan.is_some()
    }

    /// Sets a pending plan for user review.
    ///
    /// The UI should display this as a modal plan review overlay.
    pub fn set_pending_plan(&mut self, plan: PlanState) {
        self.pending_plan = Some(plan);
        self.dirty.full = true;
    }

    /// Approves the pending plan, returning a success tool result block.
    ///
    /// Clears the pending plan state.
    ///
    /// # Returns
    ///
    /// The tool result block to send back to the API, or `None` if no plan is pending.
    pub fn approve_plan(&mut self) -> Option<crate::types::ToolResultBlock> {
        let plan = self.pending_plan.take()?;
        self.dirty.full = true;
        Some(plan.approve())
    }

    /// Rejects the pending plan, returning an error tool result block.
    ///
    /// Clears the pending plan state.
    ///
    /// # Returns
    ///
    /// The tool result block to send back to the API, or `None` if no plan is pending.
    pub fn reject_plan(&mut self) -> Option<crate::types::ToolResultBlock> {
        let plan = self.pending_plan.take()?;
        self.dirty.full = true;
        Some(plan.reject())
    }

    // --- Question prompt state ---

    /// Returns the pending question, if any.
    #[must_use]
    pub fn pending_question(&self) -> Option<&QuestionState> {
        self.pending_question.as_ref()
    }

    /// Returns a mutable reference to the pending question, if any.
    #[must_use]
    pub fn pending_question_mut(&mut self) -> Option<&mut QuestionState> {
        self.pending_question.as_mut()
    }

    /// Returns true if there's a pending question awaiting user response.
    #[must_use]
    pub fn has_pending_question(&self) -> bool {
        self.pending_question.is_some()
    }

    /// Sets a pending question for user response.
    ///
    /// The UI should display this as a modal question prompt.
    pub fn set_pending_question(&mut self, question: QuestionState) {
        self.pending_question = Some(question);
        self.dirty.full = true;
    }

    /// Submits the pending question response, returning a success tool result block.
    ///
    /// Clears the pending question state.
    pub fn submit_question(&mut self) -> Option<crate::types::ToolResultBlock> {
        let question = self.pending_question.take()?;
        self.dirty.full = true;
        Some(question.submit())
    }

    /// Cancels the pending question, returning an error tool result block.
    ///
    /// Clears the pending question state.
    pub fn cancel_question(&mut self) -> Option<crate::types::ToolResultBlock> {
        let question = self.pending_question.take()?;
        self.dirty.full = true;
        Some(question.cancel())
    }

    // --- Background task state ---

    /// Returns a reference to the background task registry.
    #[must_use]
    pub fn background_tasks(&self) -> &BackgroundTaskRegistry {
        &self.background_tasks
    }

    /// Returns a mutable reference to the background task registry.
    pub fn background_tasks_mut(&mut self) -> &mut BackgroundTaskRegistry {
        &mut self.background_tasks
    }

    // --- Completion state (delegates to InputState) ---

    /// Returns the active completion state, if any.
    #[must_use]
    pub fn completion(&self) -> Option<&crate::types::CompletionState> {
        self.input_state.completion()
    }

    /// Returns a mutable reference to the active completion state.
    #[must_use]
    pub fn completion_mut(&mut self) -> Option<&mut crate::types::CompletionState> {
        self.input_state.completion_mut()
    }

    /// Returns true if the completion popup is currently active.
    #[must_use]
    pub fn has_completion(&self) -> bool {
        self.input_state.has_completion()
    }

    /// Activates the completion popup by gathering candidates from all providers.
    ///
    /// This is a cross-domain coordinator: it reads `plugin_registry` to gather
    /// candidates and writes the result into `input_state`.
    pub fn show_completion(&mut self) {
        use crate::app::completion::{
            BuiltinCommandProvider, CompletionProvider, CompletionState, McpToolProvider,
            PluginCommandProvider,
        };

        let builtin = BuiltinCommandProvider;
        let plugins = PluginCommandProvider::from_registry(&self.plugin_registry);
        let mcp = McpToolProvider::empty();

        let mut candidates = builtin.candidates();
        candidates.extend(plugins.candidates());
        candidates.extend(mcp.candidates());

        self.input_state
            .set_completion(CompletionState::new(candidates));
        self.dirty.input = true;
    }

    /// Dismisses the completion popup.
    pub fn dismiss_completion(&mut self) {
        self.input_state.dismiss_completion();
        self.dirty.input = true;
    }

    /// Accepts the selected completion and replaces input with `/name `.
    ///
    /// Returns the accepted command name, or `None` if nothing was selected.
    pub fn accept_completion(&mut self) -> Option<String> {
        let name = self.input_state.accept_completion();
        if name.is_some() {
            self.dirty.input = true;
        }
        name
    }

    // ========================================================================
    // Tool Lifecycle (thin wrappers with dirty flags)
    // ========================================================================

    /// Handles a permission response from the user.
    ///
    /// This grants or denies permission for the pending tool call and
    /// updates the permission manager accordingly.
    pub async fn handle_permission_response(&mut self, response: PermissionResponse) {
        self.tool_state.handle_permission_response(response).await;
        self.dirty.full = true;
    }

    /// Handles a tool_use stream event.
    ///
    /// Routes the event to the tool loop state machine.
    pub fn handle_tool_use_start(&mut self, id: String, name: String, index: usize) {
        self.tool_state.handle_tool_use_start(id, name, index);
        self.dirty.messages = true;
    }

    /// Handles a tool_use input delta.
    pub fn handle_tool_use_input_delta(&mut self, index: usize, partial_json: &str) {
        self.tool_state
            .handle_tool_use_input_delta(index, partial_json);
        self.dirty.messages = true;
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
            .tool_state
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

        // Execute the tools (pass MCP manager for namespaced tool routing)
        let result = self
            .tool_state
            .tool_loop
            .execute_pending(&self.tool_state.tool_executor, self.mcp_manager.as_deref())
            .await
            .map_err(|e| match e {
                ToolLoopError::InvalidStateTransition { from, to } => {
                    anyhow::anyhow!("Invalid state transition from {} to {}", from, to)
                }
                _ => anyhow::anyhow!("{}", e),
            })?;

        // Collect results (to avoid borrow issues)
        let results: Vec<(String, String, bool)> = self
            .tool_state
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

    /// Completes tool execution and prepares the conversation for continuation.
    ///
    /// This is the shared logic between `run_print_mode()` and
    /// [`AppContext::finish_tool_execution_and_continue`](super::super::context::AppContext::finish_tool_execution_and_continue).
    /// It:
    /// 1. Finishes tool execution and gets continuation data
    /// 2. Builds assistant and user messages from tool results
    /// 3. Adds both to the API message history
    /// 4. Truncates large tool results
    /// 5. Adds a display summary to the timeline
    /// 6. Transitions the tool loop to streaming state
    ///
    /// # Errors
    ///
    /// Returns an error if `finish_tool_execution()` or `start_streaming()` fails.
    pub fn complete_tool_cycle(&mut self) -> Result<()> {
        use crate::app::tool_loop::{format_tool_results_for_display, truncate_tool_results};

        let continuation = self.tool_state.finish_tool_execution()?;
        let (assistant_msg, mut user_msg) = continuation.build_messages();

        self.api_messages_mut().push(assistant_msg);

        truncate_tool_results(&mut user_msg);

        let tool_result_summary = format_tool_results_for_display(&user_msg);
        self.add_message(Message {
            role: Role::User,
            content: tool_result_summary,
        });
        self.api_messages_mut().push(user_msg);

        self.tool_state.tool_loop_mut().start_streaming()?;
        Ok(())
    }

    /// Resets the tool loop to idle state.
    pub fn reset_tool_loop(&mut self) {
        self.tool_state.reset_tool_loop();
        self.dirty.full = true;
    }

    // ========================================================================
    // Tool Block UI Methods (thin wrappers with dirty flags)
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
        let index = self.tool_state.start_tool_block(tool_name, tool_input);
        self.dirty.messages = true;
        index
    }

    /// Completes a tool block with its result.
    ///
    /// # Arguments
    ///
    /// * `index` - Index of the tool block to update
    /// * `result` - The tool's output
    /// * `is_error` - Whether the result is an error
    pub fn complete_tool_block(&mut self, index: usize, result: &str, is_error: bool) {
        self.tool_state.complete_tool_block(index, result, is_error);
        self.dirty.messages = true;
    }

    /// Clears all tool blocks.
    ///
    /// Call this when starting a new conversation turn.
    pub fn clear_tool_blocks(&mut self) {
        self.tool_state.clear_tool_blocks();
        self.dirty.messages = true;
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
        self.display.loading = true;

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
        self.display.loading = false;
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
        self.tool_state.tool_blocks.push(block);

        // Add to timeline with message index tracking
        self.timeline.push_tool_after_current_assistant(
            tool_name,
            input,
            Some(output.to_string()),
            is_error,
        );

        self.dirty.messages = true;
    }

    /// Clears conversation history while preserving configuration.
    ///
    /// Resets messages, timeline, tool state, token budget, and scroll position.
    /// Preserves working directory, model, memory, MCP servers, plugins, cost
    /// tracker, system prompt, effort, and thinking budget.
    pub fn clear_conversation(&mut self) {
        self.api_messages.clear();
        self.tool_state.clear_tool_blocks();
        self.tool_state.clear_pending_permission();
        self.timeline = Timeline::new();
        self.reset_tool_loop();
        self.reset_token_budget();
        self.thinking_buffer.clear();
        self.display.scroll = crate::tui::scroll::ScrollState::new();
        self.display.loading = false;
        self.streaming_rx = None;
        self.dirty.messages = true;
        self.session.mark_dirty();
    }

    // ========================================================================
    // Async Tool Execution (thin wrappers)
    // ========================================================================

    /// Sets the receiver channel for async tool results.
    ///
    /// When tool execution is spawned in the background, results will be
    /// streamed back through this channel.
    pub fn set_tool_result_rx(
        &mut self,
        rx: mpsc::Receiver<(String, crate::types::ToolResultBlock)>,
    ) {
        self.tool_state.set_tool_result_rx(rx);
    }

    /// Intercepts special tools that need synchronous or UI-driven handling.
    ///
    /// Separates tools into intercepted (handled immediately) and remaining
    /// (to be executed in background). Intercepted tools include:
    /// - `plan`: Requires modal UI for user approval
    /// - `ask_user`: Requires modal UI for user response
    /// - `bash` with `run_in_background`: Spawned as a background task
    /// - `task_output`: Reads output from a background task
    /// - `task_stop`: Stops a background task
    ///
    /// # Arguments
    ///
    /// * `pending` - Tools awaiting execution
    /// * `tx` - Channel sender for tool results (used for error/immediate results)
    ///
    /// # Returns
    ///
    /// A tuple of (intercepted tool IDs, remaining tools for background execution).
    fn intercept_special_tools(
        &mut self,
        pending: Vec<(String, ToolUseBlock)>,
        tx: &mpsc::Sender<(String, ToolResultBlock)>,
    ) -> (Vec<String>, Vec<(String, ToolUseBlock)>) {
        let mut intercepted = Vec::new();
        let mut remaining = Vec::new();
        for (id, tool_use) in pending {
            match tool_use.name.as_str() {
                "plan" => {
                    if let Some(plan) = PlanState::from_tool_input(id.clone(), &tool_use.input) {
                        self.set_pending_plan(plan);
                        intercepted.push(id);
                    } else {
                        let result = ToolResultBlock::error(
                            &id,
                            "Invalid plan format: expected title and steps",
                        );
                        let tx_clone = tx.clone();
                        let id_clone = id.clone();
                        tokio::spawn(async move {
                            let _ = tx_clone.send((id_clone, result)).await;
                        });
                        intercepted.push(id);
                    }
                }
                "ask_user" => {
                    if let Some(question) =
                        QuestionState::from_tool_input(id.clone(), &tool_use.input)
                    {
                        self.set_pending_question(question);
                        intercepted.push(id);
                    } else {
                        let result = ToolResultBlock::error(
                            &id,
                            "Invalid ask_user format: expected question field",
                        );
                        let tx_clone = tx.clone();
                        let id_clone = id.clone();
                        tokio::spawn(async move {
                            let _ = tx_clone.send((id_clone, result)).await;
                        });
                        intercepted.push(id);
                    }
                }
                "bash"
                    if tool_use
                        .input
                        .get("run_in_background")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false) =>
                {
                    let command = tool_use
                        .input
                        .get("command")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let task_id = self.background_tasks.spawn(command, &self.working_dir);
                    let result =
                        ToolResultBlock::success(&id, format!("Background task {task_id} started"));
                    let tx_clone = tx.clone();
                    let id_clone = id.clone();
                    tokio::spawn(async move {
                        let _ = tx_clone.send((id_clone, result)).await;
                    });
                    intercepted.push(id);
                }
                "task_output" => {
                    let task_id_val = tool_use
                        .input
                        .get("task_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let output_buf = self
                        .background_tasks
                        .tasks_ref()
                        .get(&task_id_val)
                        .map(|t| (Arc::clone(&t.output_buffer), Arc::clone(&t.completed)));
                    let tool_id_clone = id.clone();
                    let tx_clone = tx.clone();
                    tokio::spawn(async move {
                        let result = match output_buf {
                            Some((buf, completed)) => {
                                let content = buf.lock().await;
                                let status = if completed.load(std::sync::atomic::Ordering::Acquire)
                                {
                                    "completed"
                                } else {
                                    "running"
                                };
                                ToolResultBlock::success(
                                    &tool_id_clone,
                                    format!("[Task {task_id_val} ({status})]\n{}", *content),
                                )
                            }
                            None => ToolResultBlock::error(
                                &tool_id_clone,
                                format!("No task with ID '{task_id_val}'"),
                            ),
                        };
                        let _ = tx_clone.send((tool_id_clone, result)).await;
                    });
                    intercepted.push(id);
                }
                "task_stop" => {
                    let task_id_val = tool_use
                        .input
                        .get("task_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let result_msg = self.background_tasks.stop(&task_id_val);
                    let result = match result_msg {
                        Ok(msg) => ToolResultBlock::success(&id, msg),
                        Err(msg) => ToolResultBlock::error(&id, msg),
                    };
                    let tx_clone = tx.clone();
                    let id_clone = id.clone();
                    tokio::spawn(async move {
                        let _ = tx_clone.send((id_clone, result)).await;
                    });
                    intercepted.push(id);
                }
                _ => remaining.push((id, tool_use)),
            }
        }
        (intercepted, remaining)
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
    ) -> Option<tokio::task::JoinHandle<Vec<(String, ToolResultBlock)>>> {
        // Create channel for results
        let (tx, rx) = mpsc::channel(100);
        self.tool_state.set_tool_result_rx(rx);

        // Get pending tools
        let pending: Vec<_> = self
            .tool_state
            .tool_loop
            .pending_calls()
            .iter()
            .filter(|(_, call)| !call.executed)
            .map(|(id, call)| (id.clone(), call.tool_use.clone()))
            .collect();

        if pending.is_empty() {
            return None;
        }

        // Intercept interactive and special tools before spawning background work
        let (intercepted, pending) = self.intercept_special_tools(pending, &tx);

        // Mark intercepted tools as executing (they'll complete when user responds)
        for id in &intercepted {
            self.tool_state.mark_tool_executing(id);
        }

        if pending.is_empty() && intercepted.is_empty() {
            return None;
        }

        // Mark remaining as executing
        for (id, _) in &pending {
            self.tool_state.mark_tool_executing(id);
        }

        let executor = Arc::clone(&self.tool_state.tool_executor);
        let mcp_manager = self.mcp_manager.clone();

        // Spawn background task
        let handle = tokio::spawn(async move {
            use crate::app::tool_loop::tool_use_to_call;
            use crate::mcp::manager::is_mcp_tool;

            let mut results = Vec::new();
            for (tool_id, tool_use) in pending {
                let result_block = if is_mcp_tool(&tool_use.name) {
                    execute_mcp_tool(&tool_id, &tool_use, &mcp_manager).await
                } else {
                    let call = tool_use_to_call(&tool_use);
                    let result = executor.execute(call).await;
                    execute_builtin_tool_result(&tool_id, result)
                };

                // Send through channel (ignore error if receiver dropped)
                let _ = tx.send((tool_id.clone(), result_block.clone())).await;
                results.push((tool_id, result_block));
            }
            results
        });

        Some(handle)
    }

    /// Records a tool result and removes the tool from executing set.
    ///
    /// Updates the tool loop, removes the tool from the executing set,
    /// and updates the timeline entry.
    ///
    /// # Arguments
    ///
    /// * `tool_id` - The ID of the tool that completed
    /// * `result` - The result of the tool execution
    pub fn record_tool_result(&mut self, tool_id: &str, result: crate::types::ToolResultBlock) {
        // Delegate tool_state bookkeeping
        let output = Some(result.content.clone());
        let is_error = result.is_error;
        self.tool_state.record_tool_result(tool_id, result);

        // Update timeline tool entry if it exists
        self.update_timeline_tool_by_id(tool_id, output, is_error);

        self.dirty.messages = true;
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

/// Executes an MCP-namespaced tool via the MCP manager.
///
/// Routes the tool call through the MCP manager and converts the result
/// into a `ToolResultBlock`. If no MCP manager is available, returns an
/// error result.
///
/// # Arguments
///
/// * `tool_id` - The unique tool use ID for result correlation
/// * `tool_use` - The tool use block containing name and input
/// * `mcp_manager` - Optional reference to the MCP manager
async fn execute_mcp_tool(
    tool_id: &str,
    tool_use: &ToolUseBlock,
    mcp_manager: &Option<Arc<crate::mcp::manager::McpManager>>,
) -> ToolResultBlock {
    match mcp_manager {
        Some(mgr) => match mgr.call_tool(&tool_use.name, tool_use.input.clone()).await {
            Ok(ref tr) => ToolResultBlock::from_result(tool_id, tr),
            Err(e) => ToolResultBlock::error(tool_id, format!("MCP tool error: {e}")),
        },
        None => ToolResultBlock::error(
            tool_id,
            format!(
                "MCP tool '{}' called but no MCP manager available",
                tool_use.name
            ),
        ),
    }
}

/// Converts a built-in tool execution result into a `ToolResultBlock`.
///
/// Maps the `Result<ToolResult, Error>` from the tool executor into the
/// corresponding success or error `ToolResultBlock`.
///
/// # Arguments
///
/// * `tool_id` - The unique tool use ID for result correlation
/// * `result` - The tool execution result
#[must_use]
fn execute_builtin_tool_result(
    tool_id: &str,
    result: std::result::Result<crate::tools::ToolResult, anyhow::Error>,
) -> ToolResultBlock {
    match result {
        Ok(ref tr) => ToolResultBlock::from_result(tool_id, tr),
        Err(e) => ToolResultBlock::error(tool_id, e.to_string()),
    }
}
