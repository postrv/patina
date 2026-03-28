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
    /// Extracts the command from the tool input, validates it against the security
    /// policy, then spawns it as a background task. Returns an error result if the
    /// command is blocked by the security policy.
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

        // Validate the command against the security policy before spawning
        let policy = self.tool_state.tool_executor.policy();
        if let Err(msg) = crate::tools::validate_command(&command, policy) {
            let result = ToolResultBlock::error(id, msg);
            let tx_clone = tx.clone();
            let id_clone = id.to_string();
            tokio::spawn(async move {
                let _ = tx_clone.send((id_clone, result)).await;
            });
            return;
        }

        let task_id = match self.background_tasks.spawn(command, &self.working_dir) {
            Ok(tid) => tid,
            Err(msg) => {
                let result = ToolResultBlock::error(id, msg);
                let tx_clone = tx.clone();
                let id_clone = id.to_string();
                tokio::spawn(async move {
                    let _ = tx_clone.send((id_clone, result)).await;
                });
                return;
            }
        };
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;
    use tokio::sync::mpsc;

    use crate::app::state::AppState;
    use crate::tools::ToolResult;
    use crate::types::config::ParallelMode;
    use crate::types::content::{ToolResultBlock, ToolUseBlock};

    fn test_state() -> AppState {
        AppState::new(PathBuf::from("/tmp/test"), false, ParallelMode::Enabled)
    }

    fn make_tool_use(name: &str, input: serde_json::Value) -> ToolUseBlock {
        ToolUseBlock {
            id: format!("test_{name}"),
            name: name.to_string(),
            input,
        }
    }

    // =========================================================================
    // Interception dispatch
    // =========================================================================

    #[tokio::test]
    async fn test_intercept_plan_tool() {
        let mut state = test_state();
        let (tx, _rx) = mpsc::channel::<(String, ToolResultBlock)>(100);

        let plan_input = json!({
            "title": "My Plan",
            "steps": [
                { "description": "Step one", "tool_calls": ["bash"] }
            ]
        });
        let tool_use = make_tool_use("plan", plan_input);
        let pending = vec![("toolu_plan_1".to_string(), tool_use)];

        let (intercepted, remaining) = state.intercept_special_tools(pending, &tx);

        assert_eq!(intercepted.len(), 1);
        assert_eq!(intercepted[0], "toolu_plan_1");
        assert!(remaining.is_empty());
        assert!(state.has_pending_plan());

        let plan = state.pending_plan().expect("plan should be set");
        assert_eq!(plan.title, "My Plan");
        assert_eq!(plan.steps.len(), 1);
    }

    #[tokio::test]
    async fn test_intercept_ask_user_tool() {
        let mut state = test_state();
        let (tx, _rx) = mpsc::channel::<(String, ToolResultBlock)>(100);

        let question_input = json!({
            "question": "Which database should I use?",
            "options": ["PostgreSQL", "MySQL"]
        });
        let tool_use = make_tool_use("ask_user", question_input);
        let pending = vec![("toolu_ask_1".to_string(), tool_use)];

        let (intercepted, remaining) = state.intercept_special_tools(pending, &tx);

        assert_eq!(intercepted.len(), 1);
        assert_eq!(intercepted[0], "toolu_ask_1");
        assert!(remaining.is_empty());
        assert!(state.has_pending_question());

        let q = state.pending_question().expect("question should be set");
        assert_eq!(q.question, "Which database should I use?");
        assert_eq!(q.options.len(), 2);
    }

    #[tokio::test]
    async fn test_intercept_passes_through_normal_tools() {
        let mut state = test_state();
        let (tx, _rx) = mpsc::channel::<(String, ToolResultBlock)>(100);

        let bash_tool = ToolUseBlock {
            id: "toolu_bash_1".to_string(),
            name: "bash".to_string(),
            input: json!({"command": "ls -la"}),
        };
        let read_tool = ToolUseBlock {
            id: "toolu_read_1".to_string(),
            name: "read".to_string(),
            input: json!({"file_path": "/tmp/test.txt"}),
        };
        let pending = vec![
            ("toolu_bash_1".to_string(), bash_tool),
            ("toolu_read_1".to_string(), read_tool),
        ];

        let (intercepted, remaining) = state.intercept_special_tools(pending, &tx);

        assert!(intercepted.is_empty());
        assert_eq!(remaining.len(), 2);
        assert_eq!(remaining[0].0, "toolu_bash_1");
        assert_eq!(remaining[1].0, "toolu_read_1");
    }

    // =========================================================================
    // Background tasks
    // =========================================================================

    #[tokio::test]
    async fn test_intercept_background_bash_tool() {
        let mut state = test_state();
        let (tx, mut rx) = mpsc::channel::<(String, ToolResultBlock)>(100);

        let tool_use = ToolUseBlock {
            id: "toolu_bg_1".to_string(),
            name: "bash".to_string(),
            input: json!({"command": "echo hello", "run_in_background": true}),
        };
        let pending = vec![("toolu_bg_1".to_string(), tool_use)];

        let (intercepted, remaining) = state.intercept_special_tools(pending, &tx);

        assert_eq!(intercepted.len(), 1);
        assert_eq!(intercepted[0], "toolu_bg_1");
        assert!(remaining.is_empty());

        // A result should be sent through the channel with the task ID
        let (id, result) = rx.recv().await.expect("should receive result");
        assert_eq!(id, "toolu_bg_1");
        assert!(!result.is_error);
        assert!(
            result.content.contains("Background task"),
            "Expected background task message, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_intercept_task_stop_no_such_task() {
        let mut state = test_state();
        let (tx, mut rx) = mpsc::channel::<(String, ToolResultBlock)>(100);

        let tool_use = ToolUseBlock {
            id: "toolu_stop_1".to_string(),
            name: "task_stop".to_string(),
            input: json!({"task_id": "nonexistent_task"}),
        };
        let pending = vec![("toolu_stop_1".to_string(), tool_use)];

        let (intercepted, remaining) = state.intercept_special_tools(pending, &tx);

        assert_eq!(intercepted.len(), 1);
        assert!(remaining.is_empty());

        // Should get an error because the task does not exist
        let (id, result) = rx.recv().await.expect("should receive result");
        assert_eq!(id, "toolu_stop_1");
        assert!(result.is_error);
        assert!(
            result.content.contains("No task with ID"),
            "Expected task-not-found error, got: {}",
            result.content
        );
    }

    // =========================================================================
    // Free functions
    // =========================================================================

    #[test]
    fn test_execute_builtin_tool_result_success() {
        let result: Result<ToolResult, anyhow::Error> =
            Ok(ToolResult::Success("output data".to_string()));
        let block = super::execute_builtin_tool_result("toolu_ok", result);

        assert_eq!(block.tool_use_id, "toolu_ok");
        assert_eq!(block.content, "output data");
        assert!(!block.is_error);
    }

    #[test]
    fn test_execute_builtin_tool_result_error() {
        let result: Result<ToolResult, anyhow::Error> =
            Err(anyhow::anyhow!("something went wrong"));
        let block = super::execute_builtin_tool_result("toolu_err", result);

        assert_eq!(block.tool_use_id, "toolu_err");
        assert_eq!(block.content, "something went wrong");
        assert!(block.is_error);
    }
}
