//! Hooked tool executor with permission and hook integration.
//!
//! This module provides `HookedToolExecutor`, which wraps `StatefulToolExecutor`
//! to automatically fire lifecycle hooks and check permissions before and after
//! tool execution.

use anyhow::Result;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::debug;

use crate::hooks::{HookDecision, HookManager};
use crate::permissions::{
    PermissionDecision, PermissionManager, PermissionRequest, PermissionResponse,
};

use super::parallel::{ParallelConfig, ParallelExecutor};
use super::security::ToolExecutionPolicy;
use super::stateful::{ShellState, StatefulToolExecutor};
use super::{ToolCall, ToolResult};
use crate::tools::cron::CronStore;
use crate::tools::tasks::TaskStore;
use crate::tools::tool_search::ToolSearchRegistry;

/// Tool executor with hook and permission integration.
///
/// Wraps `ToolExecutor` to automatically fire lifecycle hooks and check
/// permissions before and after tool execution.
///
/// # Hook Events
///
/// - `PreToolUse` - Fired before tool execution. Can block execution by returning exit code 2.
/// - `PostToolUse` - Fired after successful tool execution.
/// - `PostToolUseFailure` - Fired after failed tool execution.
///
/// # Permission Checks
///
/// When a `PermissionManager` is configured, tools are checked against permission
/// rules before execution:
/// - If allowed by rule or session grant: proceeds to execution
/// - If denied by rule: returns `ToolResult::Cancelled`
/// - If no rule matches: returns `ToolResult::NeedsPermission` with request details
///
/// # Examples
///
/// ```no_run
/// use patina::tools::{HookedToolExecutor, ToolCall};
/// use patina::hooks::HookManager;
/// use patina::permissions::PermissionManager;
/// use std::path::PathBuf;
/// use std::sync::Arc;
/// use tokio::sync::Mutex;
/// use serde_json::json;
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let hooks = HookManager::new("session-123".to_string());
///     let permissions = Arc::new(Mutex::new(PermissionManager::new()));
///     let executor = HookedToolExecutor::new(PathBuf::from("."), hooks)
///         .with_permissions(permissions);
///
///     let call = ToolCall {
///         name: "bash".to_string(),
///         input: json!({ "command": "echo hello" }),
///     };
///
///     let result = executor.execute(call).await?;
///     Ok(())
/// }
/// ```
pub struct HookedToolExecutor {
    inner: StatefulToolExecutor,
    hooks: HookManager,
    permissions: Option<Arc<Mutex<PermissionManager>>>,
    parallel: ParallelExecutor,
}

impl HookedToolExecutor {
    /// Creates a new hooked tool executor.
    ///
    /// # Arguments
    ///
    /// * `working_dir` - The working directory for tool execution
    /// * `hook_manager` - The hook manager for firing lifecycle hooks
    #[must_use]
    pub fn new(working_dir: PathBuf, hook_manager: HookManager) -> Self {
        Self {
            inner: StatefulToolExecutor::new(working_dir),
            hooks: hook_manager,
            permissions: None,
            parallel: ParallelExecutor::new(ParallelConfig::default()),
        }
    }

    /// Returns a reference to the hook manager.
    ///
    /// Use this to fire lifecycle hooks from application code.
    /// Since `HookedToolExecutor` is typically stored behind an `Arc`,
    /// this provides read-only access sufficient for `fire_*` methods
    /// which take `&self`.
    #[must_use]
    pub fn hooks(&self) -> &HookManager {
        &self.hooks
    }

    /// Returns a mutable reference to the hook manager.
    ///
    /// Use this to register hooks or load configuration before the
    /// executor is wrapped in an `Arc`. Once inside an `Arc`, only
    /// shared (`&self`) access is available via [`hooks()`](Self::hooks).
    pub fn hooks_mut(&mut self) -> &mut HookManager {
        &mut self.hooks
    }

    /// Returns the current shell state.
    ///
    /// This provides access to the tracked working directory and environment
    /// variables that persist across command executions.
    ///
    /// # Panics
    ///
    /// Panics if the lock is poisoned.
    pub fn shell_state(&self) -> std::sync::RwLockReadGuard<'_, ShellState> {
        self.inner.shell_state()
    }

    /// Returns a reference to the current execution policy.
    #[must_use]
    pub fn policy(&self) -> &ToolExecutionPolicy {
        self.inner.policy()
    }

    /// Creates a new hooked tool executor with a custom policy.
    #[must_use]
    pub fn with_policy(mut self, policy: ToolExecutionPolicy) -> Self {
        self.inner = self.inner.with_policy(policy);
        self
    }

    /// Configures the permission manager for this executor.
    ///
    /// When configured, tools will be checked against permission rules
    /// before execution.
    #[must_use]
    pub fn with_permissions(mut self, permissions: Arc<Mutex<PermissionManager>>) -> Self {
        self.permissions = Some(permissions);
        self
    }

    /// Configures parallel execution for this executor.
    ///
    /// When configured with parallel execution enabled, consecutive ReadOnly
    /// tools will be executed concurrently for improved performance.
    ///
    /// # Arguments
    ///
    /// * `config` - The parallel execution configuration
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use patina::tools::{HookedToolExecutor, ParallelConfig};
    /// use patina::hooks::HookManager;
    /// use std::path::PathBuf;
    ///
    /// let hooks = HookManager::new("session-123".to_string());
    /// let executor = HookedToolExecutor::new(PathBuf::from("."), hooks)
    ///     .with_parallel_config(ParallelConfig::default().with_max_concurrency(16));
    /// ```
    #[must_use]
    pub fn with_parallel_config(mut self, config: ParallelConfig) -> Self {
        self.parallel = ParallelExecutor::new(config);
        self
    }

    /// Returns the parallel executor configuration.
    #[must_use]
    pub fn parallel_config(&self) -> &ParallelConfig {
        self.parallel.config()
    }

    /// Sets the tool search registry for deferred tool discovery.
    #[must_use]
    pub fn with_tool_search_registry(mut self, registry: Arc<ToolSearchRegistry>) -> Self {
        self.inner = self.inner.with_tool_search_registry(registry);
        self
    }

    /// Sets the task store for LLM-callable task CRUD operations.
    #[must_use]
    pub fn with_task_store(mut self, store: Arc<TaskStore>) -> Self {
        self.inner = self.inner.with_task_store(store);
        self
    }

    /// Sets the cron store for LLM-callable schedule CRUD operations.
    #[must_use]
    pub fn with_cron_store(mut self, store: Arc<CronStore>) -> Self {
        self.inner = self.inner.with_cron_store(store);
        self
    }

    /// Grants permission for a specific tool execution.
    ///
    /// This should be called after the user responds to a permission prompt.
    /// The response will be handled by the permission manager to either:
    /// - Add a session grant (AllowOnce)
    /// - Add a persistent rule (AllowAlways)
    /// - Track denial count (Deny)
    ///
    /// # Arguments
    ///
    /// * `tool_name` - The tool that was granted/denied
    /// * `tool_input` - The specific input that was granted/denied
    /// * `response` - The user's response to the permission prompt
    pub async fn grant_permission(
        &self,
        tool_name: &str,
        tool_input: Option<&str>,
        response: PermissionResponse,
    ) {
        if let Some(ref permissions) = self.permissions {
            let mut manager = permissions.lock().await;
            manager.handle_response(tool_name, tool_input, response);
        }
    }

    /// Extracts a human-readable input string from the tool call.
    fn extract_tool_input(&self, call: &ToolCall) -> Option<String> {
        match call.name.as_str() {
            "bash" => call
                .input
                .get("command")
                .and_then(|v| v.as_str())
                .map(String::from),
            "read_file" | "write_file" | "list_files" => call
                .input
                .get("path")
                .and_then(|v| v.as_str())
                .map(String::from),
            "edit" => call
                .input
                .get("path")
                .and_then(|v| v.as_str())
                .map(String::from),
            "glob" | "grep" => call
                .input
                .get("pattern")
                .and_then(|v| v.as_str())
                .map(String::from),
            "web_fetch" => call
                .input
                .get("url")
                .and_then(|v| v.as_str())
                .map(String::from),
            "web_search" => call
                .input
                .get("query")
                .and_then(|v| v.as_str())
                .map(String::from),
            _ => {
                // For MCP tools, try to extract a meaningful input
                serde_json::to_string(&call.input).ok()
            }
        }
    }

    /// Generates a human-readable description for a tool call.
    fn generate_description(&self, call: &ToolCall) -> String {
        match call.name.as_str() {
            "bash" => {
                let cmd = call
                    .input
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown command");
                format!("Execute shell command: {cmd}")
            }
            "read_file" => {
                let path = call
                    .input
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown path");
                format!("Read file: {path}")
            }
            "write_file" => {
                let path = call
                    .input
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown path");
                format!("Write to file: {path}")
            }
            "edit" => {
                let path = call
                    .input
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown path");
                format!("Edit file: {path}")
            }
            "list_files" => {
                let path = call
                    .input
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or(".");
                format!("List directory: {path}")
            }
            "glob" => {
                let pattern = call
                    .input
                    .get("pattern")
                    .and_then(|v| v.as_str())
                    .unwrap_or("*");
                format!("Search for files matching: {pattern}")
            }
            "grep" => {
                let pattern = call
                    .input
                    .get("pattern")
                    .and_then(|v| v.as_str())
                    .unwrap_or("*");
                format!("Search file contents for: {pattern}")
            }
            "web_fetch" => {
                let url = call
                    .input
                    .get("url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown URL");
                format!("Fetch web content from: {url}")
            }
            "web_search" => {
                let query = call
                    .input
                    .get("query")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown query");
                format!("Search the web for: {query}")
            }
            name if name.starts_with("mcp__") => {
                format!("Execute MCP tool: {name}")
            }
            name => {
                format!("Execute tool: {name}")
            }
        }
    }

    /// Executes a tool call with permission checks and hook integration.
    ///
    /// This method:
    /// 1. Checks permissions - if denied, returns `ToolResult::Cancelled`
    ///    If no rule matches, returns `ToolResult::NeedsPermission`
    /// 2. Fires `PreToolUse` hook - if it returns Block, returns `ToolResult::Cancelled`
    /// 3. Executes the actual tool
    /// 4. Fires `PostToolUse` on success or `PostToolUseFailure` on failure
    ///
    /// # Errors
    ///
    /// Returns an error if hook execution or tool execution fails.
    pub async fn execute(&self, call: ToolCall) -> Result<ToolResult> {
        let tool_input = call.input.clone();
        let tool_name = call.name.clone();

        // Check permissions if configured
        if let Some(ref permissions) = self.permissions {
            let input_str = self.extract_tool_input(&call);
            let manager = permissions.lock().await;
            let decision = manager.check(&tool_name, input_str.as_deref());

            match decision {
                PermissionDecision::Denied => {
                    debug!(
                        tool = %tool_name,
                        input = ?input_str,
                        "Tool execution denied by permission rule"
                    );
                    return Ok(ToolResult::Cancelled);
                }
                PermissionDecision::NeedsPrompt => {
                    debug!(
                        tool = %tool_name,
                        input = ?input_str,
                        "Tool execution requires permission prompt"
                    );

                    // Fire PermissionRequest hook
                    if let Err(e) = self
                        .hooks
                        .fire_permission_request(&tool_name, call.input.clone())
                        .await
                    {
                        tracing::debug!("Hook fire failed: {e}");
                    }

                    let description = self.generate_description(&call);
                    let request =
                        PermissionRequest::new(&tool_name, input_str.as_deref(), &description);
                    return Ok(ToolResult::NeedsPermission(request));
                }
                PermissionDecision::Allowed | PermissionDecision::SessionGrant => {
                    debug!(
                        tool = %tool_name,
                        input = ?input_str,
                        decision = ?decision,
                        "Tool execution permitted"
                    );
                    // Continue with execution
                }
            }
        }

        // Fire PreToolUse hook
        let pre_result = self
            .hooks
            .fire_pre_tool_use(&tool_name, tool_input.clone())
            .await?;

        // Check if hook blocked execution
        if matches!(pre_result.decision, HookDecision::Block { .. }) {
            return Ok(ToolResult::Cancelled);
        }

        // Execute the actual tool
        let result = self.inner.execute(call).await?;

        // Fire post-execution hooks based on result
        match &result {
            ToolResult::Success(output) => {
                let response = json!({
                    "status": "success",
                    "output": output
                });
                self.hooks
                    .fire_post_tool_use(&tool_name, tool_input, response)
                    .await?;
            }
            ToolResult::Error(error) => {
                let response = json!({
                    "status": "error",
                    "error": error
                });
                self.hooks
                    .fire_post_tool_use_failure(&tool_name, tool_input, response)
                    .await?;
            }
            ToolResult::Cancelled | ToolResult::NeedsPermission(_) => {
                // No hook for cancelled/needs permission
            }
        }

        Ok(result)
    }

    /// Executes a batch of tool calls with parallel execution for ReadOnly tools.
    ///
    /// This method uses the `ParallelExecutor` to optimize execution by running
    /// consecutive ReadOnly tools concurrently while maintaining sequential
    /// execution for Mutating and Unknown tools.
    ///
    /// # Algorithm
    ///
    /// 1. Classify each tool by safety class (ReadOnly, Mutating, Unknown)
    /// 2. Group consecutive parallelizable tools
    /// 3. Execute groups appropriately:
    ///    - Parallelizable groups: concurrent execution with semaphore control
    ///    - Non-parallelizable tools: sequential execution
    /// 4. Return results in original order
    ///
    /// # Arguments
    ///
    /// * `calls` - Vector of tool calls to execute
    ///
    /// # Returns
    ///
    /// Vector of results in the same order as the input calls.
    ///
    /// # Errors
    ///
    /// Returns an error if any tool execution fails critically.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use patina::tools::{HookedToolExecutor, ToolCall, ToolResult};
    /// use patina::hooks::HookManager;
    /// use std::path::PathBuf;
    /// use serde_json::json;
    ///
    /// #[tokio::main]
    /// async fn main() -> anyhow::Result<()> {
    ///     let hooks = HookManager::new("session".to_string());
    ///     let executor = HookedToolExecutor::new(PathBuf::from("."), hooks);
    ///
    ///     let calls = vec![
    ///         ToolCall { name: "read_file".to_string(), input: json!({"path": "a.rs"}) },
    ///         ToolCall { name: "read_file".to_string(), input: json!({"path": "b.rs"}) },
    ///         ToolCall { name: "read_file".to_string(), input: json!({"path": "c.rs"}) },
    ///     ];
    ///
    ///     // These 3 read_file calls will execute in parallel
    ///     let results = executor.execute_batch(calls).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn execute_batch(&self, calls: Vec<ToolCall>) -> Result<Vec<ToolResult>> {
        if calls.is_empty() {
            return Ok(Vec::new());
        }

        // Classify tools and group consecutive parallelizable ones.
        // ReadOnly tools in the same group run concurrently via join_all;
        // all other tools run sequentially (one-element groups).
        let mut results = Vec::with_capacity(calls.len());
        let mut parallel_group: Vec<ToolCall> = Vec::new();

        for call in calls {
            let class = super::parallel::classify_tool(&call.name);
            let is_parallelizable = self.parallel.is_parallelizable(class);

            if is_parallelizable {
                parallel_group.push(call);
            } else {
                // Flush any accumulated parallel group first
                if !parallel_group.is_empty() {
                    let group = std::mem::take(&mut parallel_group);
                    let group_results =
                        futures::future::join_all(group.into_iter().map(|c| self.inner.execute(c)))
                            .await;
                    for r in group_results {
                        results.push(r?);
                    }
                }
                // Execute the non-parallelizable tool sequentially
                results.push(self.inner.execute(call).await?);
            }
        }

        // Flush any remaining parallel group
        if !parallel_group.is_empty() {
            let group_results = futures::future::join_all(
                parallel_group.into_iter().map(|c| self.inner.execute(c)),
            )
            .await;
            for r in group_results {
                results.push(r?);
            }
        }

        Ok(results)
    }

    /// Executes a batch of tool calls with full hook support.
    ///
    /// Unlike `execute_batch`, this method runs all tools sequentially but
    /// includes full hook integration for each tool call.
    ///
    /// Use this when you need lifecycle hooks for each tool, and use
    /// `execute_batch` when you need maximum parallelism.
    pub async fn execute_batch_with_hooks(&self, calls: Vec<ToolCall>) -> Result<Vec<ToolResult>> {
        let mut results = Vec::with_capacity(calls.len());

        for call in calls {
            let result = self.execute(call).await?;
            results.push(result);
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_hooked_executor_new() {
        let hooks = HookManager::new("test-session".to_string());
        let executor = HookedToolExecutor::new(PathBuf::from("/tmp"), hooks);

        // Verify parallel config is default
        assert!(executor.parallel_config().enabled);
    }

    #[test]
    fn test_hooked_executor_with_parallel_config() {
        let hooks = HookManager::new("test-session".to_string());
        let executor = HookedToolExecutor::new(PathBuf::from("/tmp"), hooks)
            .with_parallel_config(ParallelConfig::disabled());

        assert!(!executor.parallel_config().enabled);
    }

    #[test]
    fn test_extract_tool_input_bash() {
        let hooks = HookManager::new("test".to_string());
        let executor = HookedToolExecutor::new(PathBuf::from("/tmp"), hooks);

        let call = ToolCall {
            name: "bash".to_string(),
            input: json!({"command": "echo hello"}),
        };
        assert_eq!(
            executor.extract_tool_input(&call),
            Some("echo hello".to_string())
        );
    }

    #[test]
    fn test_extract_tool_input_read_file() {
        let hooks = HookManager::new("test".to_string());
        let executor = HookedToolExecutor::new(PathBuf::from("/tmp"), hooks);

        let call = ToolCall {
            name: "read_file".to_string(),
            input: json!({"path": "/etc/hosts"}),
        };
        assert_eq!(
            executor.extract_tool_input(&call),
            Some("/etc/hosts".to_string())
        );
    }

    #[test]
    fn test_extract_tool_input_glob() {
        let hooks = HookManager::new("test".to_string());
        let executor = HookedToolExecutor::new(PathBuf::from("/tmp"), hooks);

        let call = ToolCall {
            name: "glob".to_string(),
            input: json!({"pattern": "**/*.rs"}),
        };
        assert_eq!(
            executor.extract_tool_input(&call),
            Some("**/*.rs".to_string())
        );
    }

    #[test]
    fn test_extract_tool_input_web_fetch() {
        let hooks = HookManager::new("test".to_string());
        let executor = HookedToolExecutor::new(PathBuf::from("/tmp"), hooks);

        let call = ToolCall {
            name: "web_fetch".to_string(),
            input: json!({"url": "https://example.com"}),
        };
        assert_eq!(
            executor.extract_tool_input(&call),
            Some("https://example.com".to_string())
        );
    }

    #[test]
    fn test_extract_tool_input_unknown() {
        let hooks = HookManager::new("test".to_string());
        let executor = HookedToolExecutor::new(PathBuf::from("/tmp"), hooks);

        let call = ToolCall {
            name: "mcp__custom_tool".to_string(),
            input: json!({"foo": "bar"}),
        };
        let input = executor.extract_tool_input(&call);
        assert!(input.is_some());
        assert!(input.unwrap().contains("foo"));
    }

    #[test]
    fn test_generate_description_bash() {
        let hooks = HookManager::new("test".to_string());
        let executor = HookedToolExecutor::new(PathBuf::from("/tmp"), hooks);

        let call = ToolCall {
            name: "bash".to_string(),
            input: json!({"command": "ls -la"}),
        };
        assert_eq!(
            executor.generate_description(&call),
            "Execute shell command: ls -la"
        );
    }

    #[test]
    fn test_generate_description_read_file() {
        let hooks = HookManager::new("test".to_string());
        let executor = HookedToolExecutor::new(PathBuf::from("/tmp"), hooks);

        let call = ToolCall {
            name: "read_file".to_string(),
            input: json!({"path": "src/main.rs"}),
        };
        assert_eq!(
            executor.generate_description(&call),
            "Read file: src/main.rs"
        );
    }

    #[test]
    fn test_generate_description_write_file() {
        let hooks = HookManager::new("test".to_string());
        let executor = HookedToolExecutor::new(PathBuf::from("/tmp"), hooks);

        let call = ToolCall {
            name: "write_file".to_string(),
            input: json!({"path": "output.txt"}),
        };
        assert_eq!(
            executor.generate_description(&call),
            "Write to file: output.txt"
        );
    }

    #[test]
    fn test_generate_description_list_files() {
        let hooks = HookManager::new("test".to_string());
        let executor = HookedToolExecutor::new(PathBuf::from("/tmp"), hooks);

        let call = ToolCall {
            name: "list_files".to_string(),
            input: json!({"path": "src"}),
        };
        assert_eq!(executor.generate_description(&call), "List directory: src");
    }

    #[test]
    fn test_generate_description_glob() {
        let hooks = HookManager::new("test".to_string());
        let executor = HookedToolExecutor::new(PathBuf::from("/tmp"), hooks);

        let call = ToolCall {
            name: "glob".to_string(),
            input: json!({"pattern": "*.md"}),
        };
        assert_eq!(
            executor.generate_description(&call),
            "Search for files matching: *.md"
        );
    }

    #[test]
    fn test_generate_description_grep() {
        let hooks = HookManager::new("test".to_string());
        let executor = HookedToolExecutor::new(PathBuf::from("/tmp"), hooks);

        let call = ToolCall {
            name: "grep".to_string(),
            input: json!({"pattern": "TODO"}),
        };
        assert_eq!(
            executor.generate_description(&call),
            "Search file contents for: TODO"
        );
    }

    #[test]
    fn test_generate_description_web_fetch() {
        let hooks = HookManager::new("test".to_string());
        let executor = HookedToolExecutor::new(PathBuf::from("/tmp"), hooks);

        let call = ToolCall {
            name: "web_fetch".to_string(),
            input: json!({"url": "https://rust-lang.org"}),
        };
        assert_eq!(
            executor.generate_description(&call),
            "Fetch web content from: https://rust-lang.org"
        );
    }

    #[test]
    fn test_generate_description_web_search() {
        let hooks = HookManager::new("test".to_string());
        let executor = HookedToolExecutor::new(PathBuf::from("/tmp"), hooks);

        let call = ToolCall {
            name: "web_search".to_string(),
            input: json!({"query": "rust async"}),
        };
        assert_eq!(
            executor.generate_description(&call),
            "Search the web for: rust async"
        );
    }

    #[test]
    fn test_generate_description_mcp_tool() {
        let hooks = HookManager::new("test".to_string());
        let executor = HookedToolExecutor::new(PathBuf::from("/tmp"), hooks);

        let call = ToolCall {
            name: "mcp__server__tool".to_string(),
            input: json!({}),
        };
        assert_eq!(
            executor.generate_description(&call),
            "Execute MCP tool: mcp__server__tool"
        );
    }

    #[test]
    fn test_generate_description_unknown_tool() {
        let hooks = HookManager::new("test".to_string());
        let executor = HookedToolExecutor::new(PathBuf::from("/tmp"), hooks);

        let call = ToolCall {
            name: "custom_tool".to_string(),
            input: json!({}),
        };
        assert_eq!(
            executor.generate_description(&call),
            "Execute tool: custom_tool"
        );
    }

    #[tokio::test]
    async fn test_execute_batch_empty() {
        let hooks = HookManager::new("test".to_string());
        let executor = HookedToolExecutor::new(PathBuf::from("/tmp"), hooks);

        let results = executor.execute_batch(vec![]).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_execute_batch_with_hooks_empty() {
        let hooks = HookManager::new("test".to_string());
        let executor = HookedToolExecutor::new(PathBuf::from("/tmp"), hooks);

        let results = executor.execute_batch_with_hooks(vec![]).await.unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_hooks_accessor_returns_hook_manager() {
        let hooks = HookManager::new("test-session-abc".to_string());
        let executor = HookedToolExecutor::new(PathBuf::from("/tmp"), hooks);

        assert_eq!(executor.hooks().session_id(), "test-session-abc");
    }

    #[test]
    fn test_hooks_mut_allows_registration() {
        let hooks = HookManager::new("test-session".to_string());
        let mut executor = HookedToolExecutor::new(PathBuf::from("/tmp"), hooks);

        // Register a hook via hooks_mut
        executor.hooks_mut().register_tool_hook(
            crate::hooks::HookEvent::PreToolUse,
            Some("Bash"),
            "echo test",
        );

        // Verify via hooks() accessor (session ID unchanged)
        assert_eq!(executor.hooks().session_id(), "test-session");
    }

    // =========================================================================
    // A5: execute_batch returns real results
    // =========================================================================

    #[tokio::test]
    async fn test_execute_batch_returns_real_results() {
        let hooks = HookManager::new("test".to_string());
        let executor = HookedToolExecutor::new(PathBuf::from("/tmp"), hooks);

        let calls = vec![ToolCall {
            name: "bash".to_string(),
            input: json!({"command": "echo batch_test_output"}),
        }];

        let results = executor.execute_batch(calls).await.unwrap();
        assert_eq!(results.len(), 1);

        match &results[0] {
            ToolResult::Success(output) => {
                assert!(
                    output.contains("batch_test_output"),
                    "Expected real command output, got: {output}"
                );
                assert!(
                    !output.contains("Executed bash"),
                    "Should not return fake placeholder result"
                );
            }
            other => panic!("Expected Success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_execute_batch_multiple_tools() {
        let hooks = HookManager::new("test".to_string());
        let executor = HookedToolExecutor::new(PathBuf::from("/tmp"), hooks);

        let calls = vec![
            ToolCall {
                name: "bash".to_string(),
                input: json!({"command": "echo first"}),
            },
            ToolCall {
                name: "bash".to_string(),
                input: json!({"command": "echo second"}),
            },
        ];

        let results = executor.execute_batch(calls).await.unwrap();
        assert_eq!(results.len(), 2);

        // Both should contain real output, not fake placeholders
        for (i, result) in results.iter().enumerate() {
            match result {
                ToolResult::Success(output) => {
                    assert!(
                        !output.starts_with("Executed "),
                        "Result {i} should not be a fake placeholder: {output}"
                    );
                }
                other => panic!("Result {i}: expected Success, got: {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn test_execute_batch_unknown_tool_returns_error() {
        let hooks = HookManager::new("test".to_string());
        let executor = HookedToolExecutor::new(PathBuf::from("/tmp"), hooks);

        let calls = vec![ToolCall {
            name: "nonexistent_tool".to_string(),
            input: json!({}),
        }];

        let results = executor.execute_batch(calls).await.unwrap();
        assert_eq!(results.len(), 1);

        match &results[0] {
            ToolResult::Error(msg) => {
                assert!(
                    msg.contains("Unknown tool"),
                    "Expected unknown tool error, got: {msg}"
                );
            }
            other => panic!("Expected Error for unknown tool, got: {other:?}"),
        }
    }

    #[test]
    fn test_policy_accessor() {
        let hooks = HookManager::new("test".to_string());
        let executor = HookedToolExecutor::new(PathBuf::from("/tmp"), hooks);

        let policy = executor.policy();
        assert!(
            !policy.dangerous_patterns.is_empty(),
            "Default policy should have dangerous patterns"
        );
        assert!(!policy.allowlist_mode);
    }
}
