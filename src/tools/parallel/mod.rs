//! Parallel tool execution for performance optimization.
//!
//! This module provides safe parallel execution of tools by classifying them
//! based on their side effects:
//!
//! - **ReadOnly**: Tools that only read data (can run in parallel)
//! - **Mutating**: Tools that modify state (must run sequentially)
//! - **Unknown**: Tools with unknown side effects (treated as mutating for safety)
//!
//! # Core Principle: "Pessimistic by Default"
//!
//! Only ReadOnly tools are parallelized. Any tool with unknown behavior is
//! treated as potentially mutating and executed sequentially.

mod classification;

// Re-export classification types
pub use classification::{
    classify_bash_command, classify_tool, ToolSafetyClass, SAFE_BASH_COMMANDS,
};

// =============================================================================
// Parallel Execution Engine
// =============================================================================

use std::future::Future;
use std::sync::Arc;
use tokio::sync::Semaphore;

/// Configuration for parallel execution.
///
/// # Examples
///
/// ```
/// use patina::tools::parallel::ParallelConfig;
///
/// // Default configuration
/// let config = ParallelConfig::default();
/// assert!(config.enabled);
/// assert_eq!(config.max_concurrency, 8);
///
/// // Custom configuration
/// let config = ParallelConfig {
///     enabled: true,
///     max_concurrency: 16,
///     aggressive: false,
/// };
/// ```
#[derive(Debug, Clone)]
pub struct ParallelConfig {
    /// Whether parallel execution is enabled.
    pub enabled: bool,

    /// Maximum number of concurrent tool executions.
    /// Must be at least 1.
    pub max_concurrency: usize,

    /// Aggressive mode: also parallelize Unknown tools.
    /// WARNING: This can cause race conditions with external tools.
    /// Only enable if you understand the risks.
    pub aggressive: bool,
}

impl Default for ParallelConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_concurrency: 8,
            aggressive: false,
        }
    }
}

impl ParallelConfig {
    /// Creates a new configuration with parallel execution enabled.
    #[must_use]
    pub fn enabled() -> Self {
        Self::default()
    }

    /// Creates a new configuration with parallel execution disabled.
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }

    /// Creates an aggressive configuration that parallelizes all tools.
    ///
    /// # Safety
    ///
    /// This mode can cause race conditions with external tools (MCP, bash).
    /// Only use when you understand the risks and have verified the tools
    /// being executed are actually safe to parallelize.
    #[must_use]
    pub fn aggressive() -> Self {
        Self {
            enabled: true,
            max_concurrency: 16,
            aggressive: true,
        }
    }

    /// Sets the maximum concurrency level.
    ///
    /// # Panics
    ///
    /// Panics if `max_concurrency` is 0.
    #[must_use]
    pub fn with_max_concurrency(mut self, max_concurrency: usize) -> Self {
        assert!(max_concurrency > 0, "max_concurrency must be at least 1");
        self.max_concurrency = max_concurrency;
        self
    }
}

/// Result of a single tool execution with its original index.
#[derive(Debug)]
pub struct IndexedResult<T> {
    /// The original index of this tool in the batch.
    pub index: usize,
    /// The result of the tool execution.
    pub result: T,
}

/// Executor for parallel tool execution with concurrency control.
///
/// Uses a semaphore to limit the number of concurrent operations,
/// preventing resource exhaustion while maximizing throughput.
///
/// # Example
///
/// ```
/// use patina::tools::parallel::{ParallelExecutor, ParallelConfig, ToolSafetyClass};
///
/// #[tokio::main]
/// async fn main() {
///     let executor = ParallelExecutor::new(ParallelConfig::default());
///
///     // Check if a tool is parallelizable
///     assert!(executor.is_parallelizable(ToolSafetyClass::ReadOnly));
///     assert!(!executor.is_parallelizable(ToolSafetyClass::Mutating));
/// }
/// ```
pub struct ParallelExecutor {
    config: ParallelConfig,
    semaphore: Arc<Semaphore>,
}

impl ParallelExecutor {
    /// Creates a new parallel executor with the given configuration.
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::tools::parallel::{ParallelExecutor, ParallelConfig};
    ///
    /// let executor = ParallelExecutor::new(ParallelConfig::default());
    /// ```
    #[must_use]
    pub fn new(config: ParallelConfig) -> Self {
        let semaphore = Arc::new(Semaphore::new(config.max_concurrency));
        Self { config, semaphore }
    }

    /// Returns the configuration for this executor.
    #[must_use]
    pub fn config(&self) -> &ParallelConfig {
        &self.config
    }

    /// Returns whether a tool with the given safety class can be parallelized.
    ///
    /// In normal mode, only `ReadOnly` tools are parallelizable.
    /// In aggressive mode, `Unknown` tools are also parallelizable.
    #[must_use]
    pub fn is_parallelizable(&self, class: ToolSafetyClass) -> bool {
        if !self.config.enabled {
            return false;
        }

        match class {
            ToolSafetyClass::ReadOnly => true,
            ToolSafetyClass::Unknown => self.config.aggressive,
            ToolSafetyClass::Mutating => false,
        }
    }

    /// Executes a batch of tool operations, parallelizing where safe.
    ///
    /// # Algorithm
    ///
    /// 1. Classify each tool by its safety class
    /// 2. Group consecutive parallelizable tools
    /// 3. Execute each group:
    ///    - Parallelizable groups: run concurrently with semaphore control
    ///    - Non-parallelizable tools: run sequentially
    /// 4. Return results in original order
    ///
    /// # Arguments
    ///
    /// * `tools` - Iterator of (tool_name, tool_input, execute_fn) tuples
    ///
    /// # Type Parameters
    ///
    /// * `T` - The result type from tool execution
    /// * `F` - The async function type for executing a tool
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use patina::tools::parallel::{ParallelExecutor, ParallelConfig};
    /// use serde_json::json;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let executor = ParallelExecutor::new(ParallelConfig::default());
    ///
    ///     let tools = vec![
    ///         ("read_file", json!({"path": "file1.txt"})),
    ///         ("read_file", json!({"path": "file2.txt"})),
    ///         ("write_file", json!({"path": "output.txt", "content": "data"})),
    ///     ];
    ///
    ///     let results = executor.execute_batch(
    ///         tools.iter().map(|(n, v)| (*n, v.clone())),
    ///         |name, input| async move {
    ///             // Execute the tool
    ///             format!("Executed {} with {}", name, input)
    ///         },
    ///     ).await;
    ///
    ///     assert_eq!(results.len(), 3);
    /// }
    /// ```
    pub async fn execute_batch<'a, T, I, F, Fut>(
        &self,
        tools: I,
        execute_fn: F,
    ) -> Vec<IndexedResult<T>>
    where
        I: Iterator<Item = (&'a str, serde_json::Value)>,
        F: Fn(&str, serde_json::Value) -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = T> + Send,
        T: Send + 'static,
    {
        // Collect tools with their indices and classifications
        let classified: Vec<(usize, String, serde_json::Value, ToolSafetyClass)> = tools
            .enumerate()
            .map(|(idx, (name, input))| {
                let class = self.classify_for_execution(name, &input);
                (idx, name.to_string(), input, class)
            })
            .collect();

        if classified.is_empty() {
            return Vec::new();
        }

        // If parallel execution is disabled, run everything sequentially
        if !self.config.enabled {
            return self.execute_sequential(classified, execute_fn).await;
        }

        // Group consecutive parallelizable tools
        self.execute_with_grouping(classified, execute_fn).await
    }

    /// Classifies a tool for execution, considering bash command content.
    fn classify_for_execution(&self, name: &str, input: &serde_json::Value) -> ToolSafetyClass {
        // For bash commands, we need to look at the actual command
        if name == "bash" {
            if let Some(command) = input.get("command").and_then(|v| v.as_str()) {
                return classify_bash_command(command);
            }
            return ToolSafetyClass::Unknown;
        }

        classify_tool(name)
    }

    /// Executes all tools sequentially (when parallel is disabled).
    async fn execute_sequential<T, F, Fut>(
        &self,
        tools: Vec<(usize, String, serde_json::Value, ToolSafetyClass)>,
        execute_fn: F,
    ) -> Vec<IndexedResult<T>>
    where
        F: Fn(&str, serde_json::Value) -> Fut,
        Fut: Future<Output = T>,
    {
        let mut results = Vec::with_capacity(tools.len());

        for (index, name, input, _class) in tools {
            let result = execute_fn(&name, input).await;
            results.push(IndexedResult { index, result });
        }

        results
    }

    /// Executes tools with grouping - parallel for ReadOnly, sequential for others.
    async fn execute_with_grouping<T, F, Fut>(
        &self,
        tools: Vec<(usize, String, serde_json::Value, ToolSafetyClass)>,
        execute_fn: F,
    ) -> Vec<IndexedResult<T>>
    where
        F: Fn(&str, serde_json::Value) -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = T> + Send,
        T: Send + 'static,
    {
        let mut results = Vec::with_capacity(tools.len());
        let mut current_group: Vec<(usize, String, serde_json::Value)> = Vec::new();
        let mut current_is_parallel = false;

        for (index, name, input, class) in tools {
            let is_parallelizable = self.is_parallelizable(class);

            if current_group.is_empty() {
                current_is_parallel = is_parallelizable;
                current_group.push((index, name, input));
            } else if is_parallelizable == current_is_parallel && is_parallelizable {
                // Continue building parallel group
                current_group.push((index, name, input));
            } else {
                // Execute current group before starting new one
                let group_results = if current_is_parallel {
                    self.execute_parallel_group(
                        std::mem::take(&mut current_group),
                        execute_fn.clone(),
                    )
                    .await
                } else {
                    self.execute_sequential_group(
                        std::mem::take(&mut current_group),
                        execute_fn.clone(),
                    )
                    .await
                };
                results.extend(group_results);

                // Start new group
                current_is_parallel = is_parallelizable;
                current_group.push((index, name, input));
            }
        }

        // Execute final group
        if !current_group.is_empty() {
            let group_results = if current_is_parallel {
                self.execute_parallel_group(current_group, execute_fn).await
            } else {
                self.execute_sequential_group(current_group, execute_fn)
                    .await
            };
            results.extend(group_results);
        }

        // Results are in execution order, which matches original order for our algorithm
        results
    }

    /// Executes a group of tools in parallel with semaphore control.
    async fn execute_parallel_group<T, F, Fut>(
        &self,
        group: Vec<(usize, String, serde_json::Value)>,
        execute_fn: F,
    ) -> Vec<IndexedResult<T>>
    where
        F: Fn(&str, serde_json::Value) -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = T> + Send,
        T: Send + 'static,
    {
        let semaphore = self.semaphore.clone();

        // Create futures for all tools in the group
        let futures: Vec<_> = group
            .into_iter()
            .map(|(index, name, input)| {
                let sem = semaphore.clone();
                let exec = execute_fn.clone();
                async move {
                    // Acquire semaphore permit
                    let _permit = sem.acquire().await.expect("semaphore closed");
                    let result = exec(&name, input).await;
                    IndexedResult { index, result }
                }
            })
            .collect();

        // Execute all futures concurrently
        futures::future::join_all(futures).await
    }

    /// Executes a group of tools sequentially.
    async fn execute_sequential_group<T, F, Fut>(
        &self,
        group: Vec<(usize, String, serde_json::Value)>,
        execute_fn: F,
    ) -> Vec<IndexedResult<T>>
    where
        F: Fn(&str, serde_json::Value) -> Fut,
        Fut: Future<Output = T>,
    {
        let mut results = Vec::with_capacity(group.len());

        for (index, name, input) in group {
            let result = execute_fn(&name, input).await;
            results.push(IndexedResult { index, result });
        }

        results
    }
}

/// Extension trait for sorting results by their original index.
pub trait SortByIndex<T> {
    /// Sorts results by their original index and extracts just the results.
    fn into_sorted_results(self) -> Vec<T>;
}

impl<T> SortByIndex<T> for Vec<IndexedResult<T>> {
    fn into_sorted_results(mut self) -> Vec<T> {
        self.sort_by_key(|r| r.index);
        self.into_iter().map(|r| r.result).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{IndexedResult, ParallelConfig, ParallelExecutor, SortByIndex, ToolSafetyClass};
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    // =========================================================================
    // Tests for ParallelExecutor struct
    // =========================================================================

    #[test]
    fn test_parallel_executor_new() {
        let config = ParallelConfig::default();
        let executor = ParallelExecutor::new(config);

        // Verify executor was created with correct config
        assert!(executor.config().enabled);
        assert_eq!(executor.config().max_concurrency, 8);
        assert!(!executor.config().aggressive);
    }

    #[test]
    fn test_parallel_executor_with_custom_config() {
        let config = ParallelConfig {
            enabled: true,
            max_concurrency: 16,
            aggressive: false,
        };
        let executor = ParallelExecutor::new(config);

        assert_eq!(executor.config().max_concurrency, 16);
    }

    #[test]
    fn test_parallel_executor_disabled() {
        let config = ParallelConfig::disabled();
        let executor = ParallelExecutor::new(config);

        assert!(!executor.config().enabled);
        // When disabled, nothing is parallelizable
        assert!(!executor.is_parallelizable(ToolSafetyClass::ReadOnly));
    }

    #[test]
    fn test_parallel_executor_is_parallelizable() {
        let executor = ParallelExecutor::new(ParallelConfig::default());

        // ReadOnly should be parallelizable
        assert!(executor.is_parallelizable(ToolSafetyClass::ReadOnly));

        // Mutating should NOT be parallelizable
        assert!(!executor.is_parallelizable(ToolSafetyClass::Mutating));

        // Unknown should NOT be parallelizable in normal mode
        assert!(!executor.is_parallelizable(ToolSafetyClass::Unknown));
    }

    #[test]
    fn test_parallel_executor_aggressive_mode() {
        let executor = ParallelExecutor::new(ParallelConfig::aggressive());

        // In aggressive mode, Unknown IS parallelizable
        assert!(executor.is_parallelizable(ToolSafetyClass::Unknown));

        // But Mutating is NEVER parallelizable
        assert!(!executor.is_parallelizable(ToolSafetyClass::Mutating));
    }

    // =========================================================================
    // 1.4.3 Tests for execute_batch method
    // =========================================================================

    #[tokio::test]
    async fn test_execute_batch_all_readonly_parallel() {
        let executor = ParallelExecutor::new(ParallelConfig::default());

        let tools = [
            ("read_file", json!({"path": "file1.txt"})),
            ("read_file", json!({"path": "file2.txt"})),
            ("read_file", json!({"path": "file3.txt"})),
        ];

        let execution_count = Arc::new(AtomicUsize::new(0));
        let count_clone = execution_count.clone();

        let results = executor
            .execute_batch(
                tools.iter().map(|(n, i)| (*n, i.clone())),
                move |name, input| {
                    let cnt = count_clone.clone();
                    let name = name.to_string();
                    async move {
                        cnt.fetch_add(1, Ordering::SeqCst);
                        format!("{}:{}", name, input)
                    }
                },
            )
            .await;

        // All 3 tools should have executed
        assert_eq!(results.len(), 3);
        assert_eq!(execution_count.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_execute_batch_preserves_result_order() {
        let executor = ParallelExecutor::new(ParallelConfig::default());

        let tools = [
            ("read_file", json!({"path": "a.txt"})),
            ("read_file", json!({"path": "b.txt"})),
            ("read_file", json!({"path": "c.txt"})),
        ];

        let results = executor
            .execute_batch(tools.iter().map(|(n, i)| (*n, i.clone())), |name, input| {
                let name = name.to_string();
                async move {
                    let path = input
                        .get("path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    format!("{}:{}", name, path)
                }
            })
            .await;

        // Sort by index to get original order
        let sorted_results = results.into_sorted_results();

        assert_eq!(sorted_results[0], "read_file:a.txt");
        assert_eq!(sorted_results[1], "read_file:b.txt");
        assert_eq!(sorted_results[2], "read_file:c.txt");
    }

    #[tokio::test]
    async fn test_execute_batch_mixed_parallel_sequential() {
        let executor = ParallelExecutor::new(ParallelConfig::default());

        let tools = [
            ("read_file", json!({"path": "file1.txt"})), // ReadOnly
            ("read_file", json!({"path": "file2.txt"})), // ReadOnly
            ("write_file", json!({"path": "out.txt"})),  // Mutating - breaks parallel
            ("read_file", json!({"path": "file3.txt"})), // ReadOnly - new group
        ];

        let execution_order = Arc::new(std::sync::Mutex::new(Vec::new()));
        let order_clone = execution_order.clone();

        let results = executor
            .execute_batch(
                tools.iter().map(|(n, i)| (*n, i.clone())),
                move |name, input| {
                    let order = order_clone.clone();
                    let name = name.to_string();
                    async move {
                        let path = input
                            .get("path")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        order.lock().unwrap().push(format!("{}:{}", name, path));
                        format!("{}:{}", name, path)
                    }
                },
            )
            .await;

        // All 4 should have executed
        assert_eq!(results.len(), 4);

        // Results should be sortable back to original order
        let sorted = results.into_sorted_results();
        assert_eq!(sorted.len(), 4);
    }

    #[tokio::test]
    async fn test_execute_batch_empty() {
        let executor = ParallelExecutor::new(ParallelConfig::default());

        let tools: Vec<(&str, serde_json::Value)> = vec![];

        let results: Vec<IndexedResult<String>> = executor
            .execute_batch(
                tools.iter().map(|(n, i)| (*n, i.clone())),
                |_name, _input| async move { String::new() },
            )
            .await;

        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_execute_batch_single_tool() {
        let executor = ParallelExecutor::new(ParallelConfig::default());

        let tools = [("read_file", json!({"path": "single.txt"}))];

        let results = executor
            .execute_batch(
                tools.iter().map(|(n, i)| (*n, i.clone())),
                |name, _input| {
                    let name = name.to_string();
                    async move { name }
                },
            )
            .await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].result, "read_file");
    }

    #[tokio::test]
    async fn test_execute_batch_respects_semaphore() {
        // Create executor with max_concurrency of 2
        let config = ParallelConfig::default().with_max_concurrency(2);
        let executor = ParallelExecutor::new(config);

        let concurrent_count = Arc::new(AtomicUsize::new(0));
        let max_concurrent = Arc::new(AtomicUsize::new(0));

        let tools = [
            ("read_file", json!({"path": "1.txt"})),
            ("read_file", json!({"path": "2.txt"})),
            ("read_file", json!({"path": "3.txt"})),
            ("read_file", json!({"path": "4.txt"})),
        ];

        let cc = concurrent_count.clone();
        let mc = max_concurrent.clone();

        let results = executor
            .execute_batch(
                tools.iter().map(|(n, i)| (*n, i.clone())),
                move |_name, _input| {
                    let cc = cc.clone();
                    let mc = mc.clone();
                    async move {
                        // Increment concurrent count
                        let current = cc.fetch_add(1, Ordering::SeqCst) + 1;

                        // Track max concurrent
                        mc.fetch_max(current, Ordering::SeqCst);

                        // Small delay to allow concurrent execution
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

                        // Decrement concurrent count
                        cc.fetch_sub(1, Ordering::SeqCst);

                        "done"
                    }
                },
            )
            .await;

        assert_eq!(results.len(), 4);

        // Max concurrent should not exceed 2 (the semaphore limit)
        assert!(
            max_concurrent.load(Ordering::SeqCst) <= 2,
            "Max concurrent {} should not exceed semaphore limit of 2",
            max_concurrent.load(Ordering::SeqCst)
        );
    }

    #[tokio::test]
    async fn test_execute_batch_bash_readonly_command() {
        let executor = ParallelExecutor::new(ParallelConfig::default());

        let tools = [
            // These bash commands should be classified as ReadOnly
            ("bash", json!({"command": "ls -la"})),
            ("bash", json!({"command": "cat file.txt"})),
            ("bash", json!({"command": "git status"})),
        ];

        let results = executor
            .execute_batch(tools.iter().map(|(n, i)| (*n, i.clone())), |name, input| {
                let name = name.to_string();
                async move {
                    let cmd = input.get("command").and_then(|v| v.as_str()).unwrap_or("");
                    format!("{}:{}", name, cmd)
                }
            })
            .await;

        assert_eq!(results.len(), 3);
    }

    #[tokio::test]
    async fn test_execute_batch_disabled_runs_sequential() {
        let executor = ParallelExecutor::new(ParallelConfig::disabled());

        let execution_order = Arc::new(std::sync::Mutex::new(Vec::new()));
        let order_clone = execution_order.clone();

        let tools = [
            ("read_file", json!({"path": "1.txt"})),
            ("read_file", json!({"path": "2.txt"})),
            ("read_file", json!({"path": "3.txt"})),
        ];

        let results = executor
            .execute_batch(
                tools.iter().map(|(n, i)| (*n, i.clone())),
                move |_name, input| {
                    let order = order_clone.clone();
                    async move {
                        let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("");
                        order.lock().unwrap().push(path.to_string());
                        path.to_string()
                    }
                },
            )
            .await;

        assert_eq!(results.len(), 3);

        // When disabled, execution should be sequential and in order
        let order = execution_order.lock().unwrap();
        assert_eq!(order.as_slice(), &["1.txt", "2.txt", "3.txt"]);
    }

    // =========================================================================
    // Tests for ParallelConfig
    // =========================================================================

    #[test]
    fn test_parallel_config_default() {
        let config = ParallelConfig::default();
        assert!(config.enabled);
        assert_eq!(config.max_concurrency, 8);
        assert!(!config.aggressive);
    }

    #[test]
    fn test_parallel_config_enabled() {
        let config = ParallelConfig::enabled();
        assert!(config.enabled);
    }

    #[test]
    fn test_parallel_config_disabled() {
        let config = ParallelConfig::disabled();
        assert!(!config.enabled);
    }

    #[test]
    fn test_parallel_config_aggressive() {
        let config = ParallelConfig::aggressive();
        assert!(config.enabled);
        assert!(config.aggressive);
        assert_eq!(config.max_concurrency, 16);
    }

    #[test]
    fn test_parallel_config_with_max_concurrency() {
        let config = ParallelConfig::default().with_max_concurrency(32);
        assert_eq!(config.max_concurrency, 32);
    }

    #[test]
    #[should_panic(expected = "max_concurrency must be at least 1")]
    fn test_parallel_config_zero_concurrency_panics() {
        let _ = ParallelConfig::default().with_max_concurrency(0);
    }

    // =========================================================================
    // Tests for IndexedResult and SortByIndex
    // =========================================================================

    #[test]
    fn test_indexed_result() {
        let result = IndexedResult {
            index: 5,
            result: "test".to_string(),
        };
        assert_eq!(result.index, 5);
        assert_eq!(result.result, "test");
    }

    #[test]
    fn test_sort_by_index() {
        let results = vec![
            IndexedResult {
                index: 2,
                result: "c",
            },
            IndexedResult {
                index: 0,
                result: "a",
            },
            IndexedResult {
                index: 1,
                result: "b",
            },
        ];

        let sorted = results.into_sorted_results();
        assert_eq!(sorted, vec!["a", "b", "c"]);
    }
}
