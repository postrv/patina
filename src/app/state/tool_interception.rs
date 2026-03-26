//! Tool interception handlers for special tools that need synchronous or UI-driven handling.
//!
//! This module contains the `intercept_special_tools` dispatcher and its per-tool-type
//! handler functions, plus the free functions for executing MCP and built-in tools.
//! Split from `tool_coordination.rs` to reduce cognitive complexity.

use super::*;

impl AppState {
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
    pub(super) fn intercept_special_tools(
        &mut self,
        pending: Vec<(String, ToolUseBlock)>,
        tx: &mpsc::Sender<(String, ToolResultBlock)>,
    ) -> (Vec<String>, Vec<(String, ToolUseBlock)>) {
        let mut intercepted = Vec::new();
        let mut remaining = Vec::new();
        for (id, tool_use) in pending {
            let is_background_bash = tool_use.name == "bash"
                && tool_use
                    .input
                    .get("run_in_background")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

            match tool_use.name.as_str() {
                "plan" => {
                    self.intercept_plan_tool(&id, &tool_use, tx);
                    intercepted.push(id);
                }
                "ask_user" => {
                    self.intercept_ask_user_tool(&id, &tool_use, tx);
                    intercepted.push(id);
                }
                _ if is_background_bash => {
                    self.intercept_background_bash_tool(&id, &tool_use, tx);
                    intercepted.push(id);
                }
                "task_output" => {
                    self.intercept_task_output_tool(&id, &tool_use, tx);
                    intercepted.push(id);
                }
                "task_stop" => {
                    self.intercept_task_stop_tool(&id, &tool_use, tx);
                    intercepted.push(id);
                }
                _ => remaining.push((id, tool_use)),
            }
        }
        (intercepted, remaining)
    }

    /// Intercepts a plan tool, setting up the modal UI for user review.
    ///
    /// If the plan format is valid, sets the pending plan state. Otherwise,
    /// sends an error result through the channel.
    fn intercept_plan_tool(
        &mut self,
        id: &str,
        tool_use: &ToolUseBlock,
        tx: &mpsc::Sender<(String, ToolResultBlock)>,
    ) {
        if let Some(plan) = PlanState::from_tool_input(id.to_string(), &tool_use.input) {
            self.set_pending_plan(plan);
        } else {
            let result =
                ToolResultBlock::error(id, "Invalid plan format: expected title and steps");
            let tx_clone = tx.clone();
            let id_clone = id.to_string();
            tokio::spawn(async move {
                let _ = tx_clone.send((id_clone, result)).await;
            });
        }
    }

    /// Intercepts an ask_user tool, setting up the question modal.
    ///
    /// If the question format is valid, sets the pending question state. Otherwise,
    /// sends an error result through the channel.
    fn intercept_ask_user_tool(
        &mut self,
        id: &str,
        tool_use: &ToolUseBlock,
        tx: &mpsc::Sender<(String, ToolResultBlock)>,
    ) {
        if let Some(question) = QuestionState::from_tool_input(id.to_string(), &tool_use.input) {
            self.set_pending_question(question);
        } else {
            let result =
                ToolResultBlock::error(id, "Invalid ask_user format: expected question field");
            let tx_clone = tx.clone();
            let id_clone = id.to_string();
            tokio::spawn(async move {
                let _ = tx_clone.send((id_clone, result)).await;
            });
        }
    }

    /// Intercepts a bash tool with `run_in_background`, spawning a background task.
    ///
    /// Extracts the command from the tool input, spawns it as a background task,
    /// and sends a success result with the task ID.
    fn intercept_background_bash_tool(
        &mut self,
        id: &str,
        tool_use: &ToolUseBlock,
        tx: &mpsc::Sender<(String, ToolResultBlock)>,
    ) {
        let command = tool_use
            .input
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let task_id = self.background_tasks.spawn(command, &self.working_dir);
        let result = ToolResultBlock::success(id, format!("Background task {task_id} started"));
        let tx_clone = tx.clone();
        let id_clone = id.to_string();
        tokio::spawn(async move {
            let _ = tx_clone.send((id_clone, result)).await;
        });
    }

    /// Intercepts a task_output tool, reading output from a background task.
    ///
    /// Looks up the background task by ID, reads its output buffer and status,
    /// and sends the result through the channel.
    fn intercept_task_output_tool(
        &mut self,
        id: &str,
        tool_use: &ToolUseBlock,
        tx: &mpsc::Sender<(String, ToolResultBlock)>,
    ) {
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
        let tool_id_clone = id.to_string();
        let tx_clone = tx.clone();
        tokio::spawn(async move {
            let result = match output_buf {
                Some((buf, completed)) => {
                    let content = buf.lock().await;
                    let status = if completed.load(std::sync::atomic::Ordering::Acquire) {
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
    }

    /// Intercepts a task_stop tool, stopping a background task.
    ///
    /// Stops the background task by ID and sends the result (success or error)
    /// through the channel.
    fn intercept_task_stop_tool(
        &mut self,
        id: &str,
        tool_use: &ToolUseBlock,
        tx: &mpsc::Sender<(String, ToolResultBlock)>,
    ) {
        let task_id_val = tool_use
            .input
            .get("task_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let result_msg = self.background_tasks.stop(&task_id_val);
        let result = match result_msg {
            Ok(msg) => ToolResultBlock::success(id, msg),
            Err(msg) => ToolResultBlock::error(id, msg),
        };
        let tx_clone = tx.clone();
        let id_clone = id.to_string();
        tokio::spawn(async move {
            let _ = tx_clone.send((id_clone, result)).await;
        });
    }
}

/// Spawns background execution for a set of tools.
///
/// Executes each tool in sequence, sending results through the channel as they complete.
/// MCP-namespaced tools are routed through the MCP manager; built-in tools go through
/// the hooked tool executor.
///
/// # Arguments
///
/// * `tools` - Tools to execute in the background
/// * `executor` - The hooked tool executor for built-in tools
/// * `mcp_manager` - Optional MCP manager for MCP-namespaced tools
/// * `tx` - Channel sender for streaming results back
///
/// # Returns
///
/// A `JoinHandle` that resolves to the collected results.
pub(super) fn spawn_background_execution(
    tools: Vec<(String, ToolUseBlock)>,
    executor: Arc<HookedToolExecutor>,
    mcp_manager: Option<Arc<crate::mcp::manager::McpManager>>,
    tx: mpsc::Sender<(String, ToolResultBlock)>,
) -> tokio::task::JoinHandle<Vec<(String, ToolResultBlock)>> {
    tokio::spawn(async move {
        use crate::app::tool_loop::tool_use_to_call;
        use crate::mcp::manager::is_mcp_tool;

        let mut results = Vec::new();
        for (tool_id, tool_use) in tools {
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
    })
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
pub(super) async fn execute_mcp_tool(
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
pub(super) fn execute_builtin_tool_result(
    tool_id: &str,
    result: std::result::Result<crate::tools::ToolResult, anyhow::Error>,
) -> ToolResultBlock {
    match result {
        Ok(ref tr) => ToolResultBlock::from_result(tool_id, tr),
        Err(e) => ToolResultBlock::error(tool_id, e.to_string()),
    }
}
