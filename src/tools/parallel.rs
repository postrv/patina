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

/// Classification of tool safety for parallel execution.
///
/// This enum categorizes tools based on their side effects to determine
/// whether they can safely be executed in parallel with other tools.
///
/// # Variants
///
/// - `ReadOnly` - Tool only reads data, safe to parallelize
/// - `Mutating` - Tool modifies state, must run sequentially
/// - `Unknown` - Tool behavior unknown, treated as mutating (pessimistic)
///
/// # Examples
///
/// ```
/// use patina::tools::parallel::ToolSafetyClass;
///
/// let read_class = ToolSafetyClass::ReadOnly;
/// assert!(read_class.is_parallelizable());
///
/// let mutating_class = ToolSafetyClass::Mutating;
/// assert!(!mutating_class.is_parallelizable());
///
/// let unknown_class = ToolSafetyClass::Unknown;
/// assert!(!unknown_class.is_parallelizable());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSafetyClass {
    /// Tool only reads data and has no side effects.
    /// Safe to execute in parallel with other ReadOnly tools.
    ReadOnly,

    /// Tool modifies state (files, system, etc.).
    /// Must be executed sequentially to preserve correctness.
    Mutating,

    /// Tool behavior is unknown or unpredictable.
    /// Treated as Mutating for safety (pessimistic approach).
    Unknown,
}

impl ToolSafetyClass {
    /// Returns whether this tool class can be safely parallelized.
    ///
    /// Only `ReadOnly` tools return `true`. Both `Mutating` and `Unknown`
    /// return `false` to ensure correctness over performance.
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::tools::parallel::ToolSafetyClass;
    ///
    /// assert!(ToolSafetyClass::ReadOnly.is_parallelizable());
    /// assert!(!ToolSafetyClass::Mutating.is_parallelizable());
    /// assert!(!ToolSafetyClass::Unknown.is_parallelizable());
    /// ```
    #[must_use]
    pub const fn is_parallelizable(&self) -> bool {
        matches!(self, Self::ReadOnly)
    }
}

/// Classifies a tool by name to determine its safety class for parallel execution.
///
/// # Classification Rules
///
/// - **ReadOnly**: `read_file`, `glob`, `grep`, `list_files`, `web_fetch`, `web_search`
/// - **Mutating**: `write_file`, `edit`
/// - **Unknown**: `bash`, any MCP tools (starting with `mcp__`), unrecognized tools
///
/// # Arguments
///
/// * `tool_name` - The name of the tool to classify
///
/// # Examples
///
/// ```
/// use patina::tools::parallel::{classify_tool, ToolSafetyClass};
///
/// assert_eq!(classify_tool("read_file"), ToolSafetyClass::ReadOnly);
/// assert_eq!(classify_tool("write_file"), ToolSafetyClass::Mutating);
/// assert_eq!(classify_tool("bash"), ToolSafetyClass::Unknown);
/// ```
#[must_use]
pub fn classify_tool(tool_name: &str) -> ToolSafetyClass {
    match tool_name {
        // ReadOnly tools - safe to parallelize
        "read_file" | "glob" | "grep" | "list_files" | "web_fetch" | "web_search" => {
            ToolSafetyClass::ReadOnly
        }

        // Mutating tools - must run sequentially
        "write_file" | "edit" => ToolSafetyClass::Mutating,

        // Bash is inherently unpredictable - classify as Unknown
        "bash" => ToolSafetyClass::Unknown,

        // MCP tools are external - classify as Unknown (pessimistic)
        name if name.starts_with("mcp__") => ToolSafetyClass::Unknown,

        // Any unrecognized tool is treated as Unknown (pessimistic by default)
        _ => ToolSafetyClass::Unknown,
    }
}

use once_cell::sync::Lazy;
use std::collections::HashSet;

/// Static set of bash commands that are safe to run in parallel.
///
/// These commands only read data and have no side effects that could
/// conflict with other parallel operations.
///
/// # Safety Criteria
///
/// Commands are included only if they:
/// 1. Never modify files or system state
/// 2. Are deterministic (same input = same output)
/// 3. Do not have hidden side effects (like network calls)
///
/// # Commands Included
///
/// - File inspection: `cat`, `head`, `tail`, `wc`, `file`, `stat`
/// - Directory listing: `ls`, `find`, `tree`, `du`
/// - Text search: `grep`, `rg`, `ag`, `ack`
/// - System info: `pwd`, `whoami`, `hostname`, `uname`, `date`
/// - Environment: `env`, `printenv`, `echo`, `which`, `type`
/// - Git read operations: `git status`, `git log`, `git diff`, `git show`, etc.
pub static SAFE_BASH_COMMANDS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    let commands = [
        // File inspection
        "cat",
        "head",
        "tail",
        "wc",
        "file",
        "stat",
        "md5sum",
        "sha256sum",
        "xxd",
        "hexdump",
        "strings",
        // Directory listing
        "ls",
        "find",
        "tree",
        "du",
        "df",
        "exa",
        "lsd",
        // Text search
        "grep",
        "rg",
        "ag",
        "ack",
        "sed", // Note: sed with -i is mutating, but we check for that
        "awk",
        // System info
        "pwd",
        "whoami",
        "hostname",
        "uname",
        "date",
        "uptime",
        "id",
        // Environment
        "env",
        "printenv",
        "echo",
        "printf",
        "which",
        "type",
        "whereis",
        "command",
        // Version/help
        "man",
        "help",
        "info",
        // Path manipulation
        "basename",
        "dirname",
        "realpath",
        "readlink",
        // Text processing (read-only)
        "sort",
        "uniq",
        "cut",
        "tr",
        "tee",
        "diff",
        "cmp",
        "comm",
        "join",
        "paste",
        "fold",
        "fmt",
        "nl",
        "rev",
        "tac",
        "expand",
        "unexpand",
        // JSON/data processing
        "jq",
        "yq",
        "xq",
        // Git read operations
        "git",
        // Cargo read operations
        "cargo",
        // npm read operations
        "npm",
        // Other read operations
        "test",
        "[",
        "true",
        "false",
    ];
    commands.into_iter().collect()
});

/// Git subcommands that are safe to run in parallel (read-only).
static SAFE_GIT_SUBCOMMANDS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    let subcommands = [
        "status",
        "log",
        "diff",
        "show",
        "branch",
        "tag",
        "describe",
        "rev-parse",
        "rev-list",
        "ls-files",
        "ls-tree",
        "cat-file",
        "blame",
        "shortlog",
        "config",
        "remote",
        "stash",
        "reflog",
        "name-rev",
        "for-each-ref",
    ];
    subcommands.into_iter().collect()
});

/// Cargo subcommands that are safe to run in parallel (read-only).
static SAFE_CARGO_SUBCOMMANDS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    let subcommands = [
        "check",
        "clippy",
        "test",
        "doc",
        "tree",
        "metadata",
        "pkgid",
        "verify-project",
        "locate-project",
        "read-manifest",
    ];
    subcommands.into_iter().collect()
});

/// npm subcommands that are safe to run in parallel (read-only).
static SAFE_NPM_SUBCOMMANDS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    let subcommands = [
        "ls", "list", "view", "info", "show", "outdated", "search", "audit", "doctor", "explain",
        "fund", "pack", "query",
    ];
    subcommands.into_iter().collect()
});

/// Classifies a bash command to determine if it's safe to run in parallel.
///
/// # Classification Rules
///
/// A bash command is classified as `ReadOnly` only if:
/// 1. It starts with a command from `SAFE_BASH_COMMANDS`
/// 2. It does not contain shell operators that could have side effects:
///    - Pipes (`|`) - could chain to mutating commands
///    - Output redirection (`>`, `>>`) - writes to files
///    - Command substitution (`$()`, `` ` ``) - could execute arbitrary code
///    - Background execution (`&`) - side effects unknown
///    - Logical operators (`&&`, `||`) - could chain to mutating commands
///
/// For git/cargo/npm, only specific read-only subcommands are allowed.
///
/// # Arguments
///
/// * `command` - The full bash command string to classify
///
/// # Examples
///
/// ```
/// use patina::tools::parallel::{classify_bash_command, ToolSafetyClass};
///
/// assert_eq!(classify_bash_command("ls -la"), ToolSafetyClass::ReadOnly);
/// assert_eq!(classify_bash_command("cat file.txt"), ToolSafetyClass::ReadOnly);
/// assert_eq!(classify_bash_command("rm -rf /"), ToolSafetyClass::Unknown);
/// assert_eq!(classify_bash_command("ls | grep foo"), ToolSafetyClass::Unknown);
/// ```
#[must_use]
pub fn classify_bash_command(command: &str) -> ToolSafetyClass {
    let trimmed = command.trim();

    if trimmed.is_empty() {
        return ToolSafetyClass::Unknown;
    }

    // Reject commands with shell operators that could chain to mutating operations
    // These patterns are checked BEFORE command parsing for security
    if contains_shell_operators(trimmed) {
        return ToolSafetyClass::Unknown;
    }

    // Extract the first word (command name)
    let first_word = trimmed.split_whitespace().next().unwrap_or("");

    // Check if the base command is in our safe list
    if !SAFE_BASH_COMMANDS.contains(first_word) {
        return ToolSafetyClass::Unknown;
    }

    // Special handling for commands that have dangerous flags
    if has_mutating_flags(trimmed, first_word) {
        return ToolSafetyClass::Unknown;
    }

    // Special handling for git - check subcommand
    if first_word == "git" {
        return classify_git_command(trimmed);
    }

    // Special handling for cargo - check subcommand
    if first_word == "cargo" {
        return classify_cargo_command(trimmed);
    }

    // Special handling for npm - check subcommand
    if first_word == "npm" {
        return classify_npm_command(trimmed);
    }

    ToolSafetyClass::ReadOnly
}

/// Checks if a command contains shell operators that could chain to mutating operations.
fn contains_shell_operators(command: &str) -> bool {
    // Check for pipes, redirections, command substitution, etc.
    // These could chain a read-only command to a mutating one

    // Output redirection (> or >>)
    if command.contains('>') {
        return true;
    }

    // Pipes (|)
    if command.contains('|') {
        return true;
    }

    // Background execution (&)
    // Be careful not to match && which we check separately
    if command.contains(" & ") || command.ends_with(" &") || command.ends_with('&') {
        // Check it's not && at the end
        if !command.ends_with("&&") {
            return true;
        }
    }

    // Logical operators (&& or ||)
    if command.contains("&&") || command.contains("||") {
        return true;
    }

    // Command substitution $() or backticks
    if command.contains("$(") || command.contains('`') {
        return true;
    }

    // Semicolon (command separator)
    if command.contains(';') {
        return true;
    }

    false
}

/// Checks if a command has flags that make it mutating.
fn has_mutating_flags(command: &str, base_command: &str) -> bool {
    match base_command {
        // sed with -i (in-place edit)
        "sed" => command.contains(" -i") || command.contains(" --in-place"),

        // tee without -a could overwrite (but tee is typically used with pipes which we reject)
        // For safety, we don't flag tee specially since pipes are already rejected
        _ => false,
    }
}

/// Git flags that take an argument (the next word is not a subcommand).
static GIT_FLAGS_WITH_ARGS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    let flags = [
        "-C",
        "-c",
        "--git-dir",
        "--work-tree",
        "--namespace",
        "-p",
        "--paginate",
    ];
    flags.into_iter().collect()
});

/// Classifies a git command based on its subcommand.
fn classify_git_command(command: &str) -> ToolSafetyClass {
    let parts: Vec<&str> = command.split_whitespace().collect();

    // Need at least "git <subcommand>"
    if parts.len() < 2 {
        return ToolSafetyClass::Unknown;
    }

    // Find the subcommand, skipping flags and their arguments
    let mut skip_next = false;
    let subcommand = parts
        .iter()
        .skip(1)
        .find(|part| {
            if skip_next {
                skip_next = false;
                return false;
            }
            if part.starts_with('-') {
                // Check if this flag takes an argument
                if GIT_FLAGS_WITH_ARGS.contains(*part) {
                    skip_next = true;
                }
                return false;
            }
            true
        })
        .copied()
        .unwrap_or("");

    if SAFE_GIT_SUBCOMMANDS.contains(subcommand) {
        ToolSafetyClass::ReadOnly
    } else {
        ToolSafetyClass::Unknown
    }
}

/// Classifies a cargo command based on its subcommand.
fn classify_cargo_command(command: &str) -> ToolSafetyClass {
    let parts: Vec<&str> = command.split_whitespace().collect();

    if parts.len() < 2 {
        return ToolSafetyClass::Unknown;
    }

    let subcommand = parts
        .iter()
        .skip(1)
        .find(|part| !part.starts_with('-'))
        .copied()
        .unwrap_or("");

    if SAFE_CARGO_SUBCOMMANDS.contains(subcommand) {
        ToolSafetyClass::ReadOnly
    } else {
        ToolSafetyClass::Unknown
    }
}

/// Classifies an npm command based on its subcommand.
fn classify_npm_command(command: &str) -> ToolSafetyClass {
    let parts: Vec<&str> = command.split_whitespace().collect();

    if parts.len() < 2 {
        return ToolSafetyClass::Unknown;
    }

    let subcommand = parts
        .iter()
        .skip(1)
        .find(|part| !part.starts_with('-'))
        .copied()
        .unwrap_or("");

    if SAFE_NPM_SUBCOMMANDS.contains(subcommand) {
        ToolSafetyClass::ReadOnly
    } else {
        ToolSafetyClass::Unknown
    }
}

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
    /// ```no_run
    /// use patina::tools::parallel::{ParallelExecutor, ParallelConfig};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let executor = ParallelExecutor::new(ParallelConfig::default());
    ///
    ///     let tools = vec![
    ///         ("read_file", "file1.txt"),
    ///         ("read_file", "file2.txt"),
    ///         ("write_file", "output.txt"),
    ///     ];
    ///
    ///     let results = executor.execute_batch(
    ///         tools.into_iter(),
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
    use super::{
        classify_bash_command, classify_tool, IndexedResult, ParallelConfig, ParallelExecutor,
        SortByIndex, ToolSafetyClass, SAFE_BASH_COMMANDS, SAFE_CARGO_SUBCOMMANDS,
        SAFE_GIT_SUBCOMMANDS, SAFE_NPM_SUBCOMMANDS,
    };
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    // =========================================================================
    // 1.1.1 Tests for ToolSafetyClass enum
    // =========================================================================

    #[test]
    fn test_tool_safety_class_variants() {
        // Verify all three variants exist and can be instantiated
        let readonly = ToolSafetyClass::ReadOnly;
        let mutating = ToolSafetyClass::Mutating;
        let unknown = ToolSafetyClass::Unknown;

        // Verify variants are distinct
        assert_ne!(readonly, mutating);
        assert_ne!(readonly, unknown);
        assert_ne!(mutating, unknown);

        // Verify Debug trait
        assert_eq!(format!("{:?}", readonly), "ReadOnly");
        assert_eq!(format!("{:?}", mutating), "Mutating");
        assert_eq!(format!("{:?}", unknown), "Unknown");
    }

    #[test]
    fn test_tool_safety_class_is_parallelizable() {
        // ReadOnly tools CAN be parallelized
        assert!(
            ToolSafetyClass::ReadOnly.is_parallelizable(),
            "ReadOnly tools should be parallelizable"
        );

        // Mutating tools CANNOT be parallelized
        assert!(
            !ToolSafetyClass::Mutating.is_parallelizable(),
            "Mutating tools should NOT be parallelizable"
        );

        // Unknown tools CANNOT be parallelized (pessimistic by default)
        assert!(
            !ToolSafetyClass::Unknown.is_parallelizable(),
            "Unknown tools should NOT be parallelizable (pessimistic approach)"
        );
    }

    #[test]
    fn test_tool_safety_class_clone() {
        // Verify Clone implementation
        let original = ToolSafetyClass::ReadOnly;
        let cloned = original;
        assert_eq!(original, cloned);
    }

    #[test]
    fn test_tool_safety_class_copy() {
        // Verify Copy implementation
        let original = ToolSafetyClass::Mutating;
        let copied = original;
        // Both should still be usable (Copy semantics)
        assert_eq!(original, copied);
        assert!(!original.is_parallelizable());
        assert!(!copied.is_parallelizable());
    }

    // =========================================================================
    // 1.2.1 Tests for classify_tool function
    // =========================================================================

    #[test]
    fn test_classify_read_file_readonly() {
        // read_file only reads data - should be ReadOnly
        assert_eq!(
            classify_tool("read_file"),
            ToolSafetyClass::ReadOnly,
            "read_file should be classified as ReadOnly"
        );
    }

    #[test]
    fn test_classify_glob_readonly() {
        // glob only searches for files - should be ReadOnly
        assert_eq!(
            classify_tool("glob"),
            ToolSafetyClass::ReadOnly,
            "glob should be classified as ReadOnly"
        );
    }

    #[test]
    fn test_classify_grep_readonly() {
        // grep only searches file contents - should be ReadOnly
        assert_eq!(
            classify_tool("grep"),
            ToolSafetyClass::ReadOnly,
            "grep should be classified as ReadOnly"
        );
    }

    #[test]
    fn test_classify_list_files_readonly() {
        // list_files only lists directory contents - should be ReadOnly
        assert_eq!(
            classify_tool("list_files"),
            ToolSafetyClass::ReadOnly,
            "list_files should be classified as ReadOnly"
        );
    }

    #[test]
    fn test_classify_web_fetch_readonly() {
        // web_fetch only fetches data from URLs - should be ReadOnly
        assert_eq!(
            classify_tool("web_fetch"),
            ToolSafetyClass::ReadOnly,
            "web_fetch should be classified as ReadOnly"
        );
    }

    #[test]
    fn test_classify_web_search_readonly() {
        // web_search only searches the web - should be ReadOnly
        assert_eq!(
            classify_tool("web_search"),
            ToolSafetyClass::ReadOnly,
            "web_search should be classified as ReadOnly"
        );
    }

    #[test]
    fn test_classify_write_file_mutating() {
        // write_file modifies files - should be Mutating
        assert_eq!(
            classify_tool("write_file"),
            ToolSafetyClass::Mutating,
            "write_file should be classified as Mutating"
        );
    }

    #[test]
    fn test_classify_edit_mutating() {
        // edit modifies files - should be Mutating
        assert_eq!(
            classify_tool("edit"),
            ToolSafetyClass::Mutating,
            "edit should be classified as Mutating"
        );
    }

    #[test]
    fn test_classify_bash_unknown() {
        // bash can do anything - should be Unknown (pessimistic)
        assert_eq!(
            classify_tool("bash"),
            ToolSafetyClass::Unknown,
            "bash should be classified as Unknown (can have any side effects)"
        );
    }

    #[test]
    fn test_classify_mcp_tools_unknown() {
        // MCP tools are external - should be Unknown (pessimistic)
        assert_eq!(
            classify_tool("mcp__narsil-mcp__search_code"),
            ToolSafetyClass::Unknown,
            "MCP tools should be classified as Unknown"
        );
        assert_eq!(
            classify_tool("mcp__jetbrains__get_file_text"),
            ToolSafetyClass::Unknown,
            "MCP tools should be classified as Unknown"
        );
    }

    #[test]
    fn test_classify_unknown_tools() {
        // Unknown tools should be classified as Unknown (pessimistic by default)
        assert_eq!(
            classify_tool("some_random_tool"),
            ToolSafetyClass::Unknown,
            "Unknown tools should be classified as Unknown"
        );
        assert_eq!(
            classify_tool(""),
            ToolSafetyClass::Unknown,
            "Empty tool name should be classified as Unknown"
        );
    }

    // =========================================================================
    // 1.3.1 Tests for SAFE_BASH_COMMANDS whitelist
    // =========================================================================

    #[test]
    fn test_safe_bash_whitelist_contains_ls() {
        assert!(
            SAFE_BASH_COMMANDS.contains("ls"),
            "SAFE_BASH_COMMANDS should contain 'ls'"
        );
    }

    #[test]
    fn test_safe_bash_whitelist_contains_cat() {
        assert!(
            SAFE_BASH_COMMANDS.contains("cat"),
            "SAFE_BASH_COMMANDS should contain 'cat'"
        );
    }

    #[test]
    fn test_safe_bash_whitelist_contains_common_commands() {
        // Verify the whitelist contains common read-only commands
        let expected_commands = [
            "head", "tail", "wc", "find", "grep", "pwd", "echo", "which", "file", "du", "df",
            "date", "whoami", "hostname", "uname", "env", "printenv",
        ];

        for cmd in expected_commands {
            assert!(
                SAFE_BASH_COMMANDS.contains(cmd),
                "SAFE_BASH_COMMANDS should contain '{}'",
                cmd
            );
        }
    }

    #[test]
    fn test_safe_bash_whitelist_excludes_dangerous_commands() {
        // Verify the whitelist does NOT contain dangerous commands
        let dangerous_commands = [
            "rm", "mv", "cp", "chmod", "chown", "sudo", "su", "dd", "mkfs",
        ];

        for cmd in dangerous_commands {
            assert!(
                !SAFE_BASH_COMMANDS.contains(cmd),
                "SAFE_BASH_COMMANDS should NOT contain dangerous command '{}'",
                cmd
            );
        }
    }

    #[test]
    fn test_safe_git_subcommands() {
        // Verify git read-only subcommands
        let expected = [
            "status", "log", "diff", "show", "branch", "tag", "blame", "ls-files",
        ];

        for cmd in expected {
            assert!(
                SAFE_GIT_SUBCOMMANDS.contains(cmd),
                "SAFE_GIT_SUBCOMMANDS should contain '{}'",
                cmd
            );
        }

        // Verify mutating git commands are NOT included
        let mutating = [
            "push", "pull", "merge", "rebase", "commit", "add", "reset", "checkout", "clone",
        ];

        for cmd in mutating {
            assert!(
                !SAFE_GIT_SUBCOMMANDS.contains(cmd),
                "SAFE_GIT_SUBCOMMANDS should NOT contain mutating command '{}'",
                cmd
            );
        }
    }

    #[test]
    fn test_safe_cargo_subcommands() {
        let expected = ["check", "clippy", "test", "doc", "tree", "metadata"];

        for cmd in expected {
            assert!(
                SAFE_CARGO_SUBCOMMANDS.contains(cmd),
                "SAFE_CARGO_SUBCOMMANDS should contain '{}'",
                cmd
            );
        }

        // Verify mutating cargo commands are NOT included
        let mutating = ["build", "run", "install", "uninstall", "publish", "update"];

        for cmd in mutating {
            assert!(
                !SAFE_CARGO_SUBCOMMANDS.contains(cmd),
                "SAFE_CARGO_SUBCOMMANDS should NOT contain mutating command '{}'",
                cmd
            );
        }
    }

    #[test]
    fn test_safe_npm_subcommands() {
        let expected = ["ls", "list", "view", "outdated", "search", "audit"];

        for cmd in expected {
            assert!(
                SAFE_NPM_SUBCOMMANDS.contains(cmd),
                "SAFE_NPM_SUBCOMMANDS should contain '{}'",
                cmd
            );
        }

        // Verify mutating npm commands are NOT included
        let mutating = ["install", "uninstall", "update", "publish", "init"];

        for cmd in mutating {
            assert!(
                !SAFE_NPM_SUBCOMMANDS.contains(cmd),
                "SAFE_NPM_SUBCOMMANDS should NOT contain mutating command '{}'",
                cmd
            );
        }
    }

    // =========================================================================
    // 1.3.3 Tests for classify_bash_command function
    // =========================================================================

    #[test]
    fn test_classify_bash_ls_readonly() {
        // Simple ls should be ReadOnly
        assert_eq!(
            classify_bash_command("ls"),
            ToolSafetyClass::ReadOnly,
            "'ls' should be classified as ReadOnly"
        );
        assert_eq!(
            classify_bash_command("ls -la"),
            ToolSafetyClass::ReadOnly,
            "'ls -la' should be classified as ReadOnly"
        );
        assert_eq!(
            classify_bash_command("ls -la /tmp"),
            ToolSafetyClass::ReadOnly,
            "'ls -la /tmp' should be classified as ReadOnly"
        );
    }

    #[test]
    fn test_classify_bash_cat_readonly() {
        assert_eq!(
            classify_bash_command("cat file.txt"),
            ToolSafetyClass::ReadOnly,
            "'cat file.txt' should be classified as ReadOnly"
        );
    }

    #[test]
    fn test_classify_bash_rm_unknown() {
        // rm is not in the whitelist - should be Unknown
        assert_eq!(
            classify_bash_command("rm file.txt"),
            ToolSafetyClass::Unknown,
            "'rm file.txt' should be classified as Unknown"
        );
        assert_eq!(
            classify_bash_command("rm -rf /"),
            ToolSafetyClass::Unknown,
            "'rm -rf /' should be classified as Unknown"
        );
    }

    #[test]
    fn test_classify_bash_pipe_unknown() {
        // Pipes could chain to mutating commands - should be Unknown
        assert_eq!(
            classify_bash_command("ls | grep foo"),
            ToolSafetyClass::Unknown,
            "'ls | grep foo' should be classified as Unknown (pipes rejected)"
        );
        assert_eq!(
            classify_bash_command("cat file | wc -l"),
            ToolSafetyClass::Unknown,
            "'cat file | wc -l' should be classified as Unknown (pipes rejected)"
        );
    }

    #[test]
    fn test_classify_bash_redirect_unknown() {
        // Output redirection writes files - should be Unknown
        assert_eq!(
            classify_bash_command("echo hello > file.txt"),
            ToolSafetyClass::Unknown,
            "Commands with '>' should be classified as Unknown"
        );
        assert_eq!(
            classify_bash_command("ls >> log.txt"),
            ToolSafetyClass::Unknown,
            "Commands with '>>' should be classified as Unknown"
        );
    }

    #[test]
    fn test_classify_bash_command_substitution_unknown() {
        // Command substitution could execute arbitrary code
        assert_eq!(
            classify_bash_command("echo $(whoami)"),
            ToolSafetyClass::Unknown,
            "Commands with '$()' should be classified as Unknown"
        );
        assert_eq!(
            classify_bash_command("echo `date`"),
            ToolSafetyClass::Unknown,
            "Commands with backticks should be classified as Unknown"
        );
    }

    #[test]
    fn test_classify_bash_logical_operators_unknown() {
        // Logical operators could chain to mutating commands
        assert_eq!(
            classify_bash_command("ls && rm file"),
            ToolSafetyClass::Unknown,
            "Commands with '&&' should be classified as Unknown"
        );
        assert_eq!(
            classify_bash_command("ls || echo fail"),
            ToolSafetyClass::Unknown,
            "Commands with '||' should be classified as Unknown"
        );
    }

    #[test]
    fn test_classify_bash_semicolon_unknown() {
        // Semicolons separate commands - could chain to mutating
        assert_eq!(
            classify_bash_command("ls; rm file"),
            ToolSafetyClass::Unknown,
            "Commands with ';' should be classified as Unknown"
        );
    }

    #[test]
    fn test_classify_bash_background_unknown() {
        // Background execution has unknown timing effects
        assert_eq!(
            classify_bash_command("ls &"),
            ToolSafetyClass::Unknown,
            "Commands with '&' (background) should be classified as Unknown"
        );
    }

    #[test]
    fn test_classify_bash_sed_inplace_unknown() {
        // sed with -i is mutating
        assert_eq!(
            classify_bash_command("sed -i 's/foo/bar/' file.txt"),
            ToolSafetyClass::Unknown,
            "'sed -i' should be classified as Unknown (mutating)"
        );
        assert_eq!(
            classify_bash_command("sed --in-place 's/foo/bar/' file.txt"),
            ToolSafetyClass::Unknown,
            "'sed --in-place' should be classified as Unknown (mutating)"
        );
        // sed without -i is ReadOnly
        assert_eq!(
            classify_bash_command("sed 's/foo/bar/' file.txt"),
            ToolSafetyClass::ReadOnly,
            "'sed' without -i should be classified as ReadOnly"
        );
    }

    #[test]
    fn test_classify_bash_git_status_readonly() {
        assert_eq!(
            classify_bash_command("git status"),
            ToolSafetyClass::ReadOnly,
            "'git status' should be classified as ReadOnly"
        );
    }

    #[test]
    fn test_classify_bash_git_log_readonly() {
        assert_eq!(
            classify_bash_command("git log"),
            ToolSafetyClass::ReadOnly,
            "'git log' should be classified as ReadOnly"
        );
        assert_eq!(
            classify_bash_command("git log --oneline -10"),
            ToolSafetyClass::ReadOnly,
            "'git log --oneline -10' should be classified as ReadOnly"
        );
    }

    #[test]
    fn test_classify_bash_git_diff_readonly() {
        assert_eq!(
            classify_bash_command("git diff"),
            ToolSafetyClass::ReadOnly,
            "'git diff' should be classified as ReadOnly"
        );
        assert_eq!(
            classify_bash_command("git diff HEAD~1"),
            ToolSafetyClass::ReadOnly,
            "'git diff HEAD~1' should be classified as ReadOnly"
        );
    }

    #[test]
    fn test_classify_bash_git_mutating_unknown() {
        // Mutating git commands should be Unknown
        assert_eq!(
            classify_bash_command("git commit -m 'test'"),
            ToolSafetyClass::Unknown,
            "'git commit' should be classified as Unknown"
        );
        assert_eq!(
            classify_bash_command("git push"),
            ToolSafetyClass::Unknown,
            "'git push' should be classified as Unknown"
        );
        assert_eq!(
            classify_bash_command("git pull"),
            ToolSafetyClass::Unknown,
            "'git pull' should be classified as Unknown"
        );
        assert_eq!(
            classify_bash_command("git add ."),
            ToolSafetyClass::Unknown,
            "'git add' should be classified as Unknown"
        );
    }

    #[test]
    fn test_classify_bash_git_with_flags_before_subcommand() {
        // git -C /path status should still work
        assert_eq!(
            classify_bash_command("git -C /some/path status"),
            ToolSafetyClass::ReadOnly,
            "'git -C /path status' should be classified as ReadOnly"
        );
    }

    #[test]
    fn test_classify_bash_cargo_check_readonly() {
        assert_eq!(
            classify_bash_command("cargo check"),
            ToolSafetyClass::ReadOnly,
            "'cargo check' should be classified as ReadOnly"
        );
    }

    #[test]
    fn test_classify_bash_cargo_clippy_readonly() {
        assert_eq!(
            classify_bash_command("cargo clippy"),
            ToolSafetyClass::ReadOnly,
            "'cargo clippy' should be classified as ReadOnly"
        );
        assert_eq!(
            classify_bash_command("cargo clippy --all-targets -- -D warnings"),
            ToolSafetyClass::ReadOnly,
            "'cargo clippy --all-targets' should be classified as ReadOnly"
        );
    }

    #[test]
    fn test_classify_bash_cargo_test_readonly() {
        assert_eq!(
            classify_bash_command("cargo test"),
            ToolSafetyClass::ReadOnly,
            "'cargo test' should be classified as ReadOnly"
        );
    }

    #[test]
    fn test_classify_bash_cargo_build_unknown() {
        // cargo build creates files - Unknown
        assert_eq!(
            classify_bash_command("cargo build"),
            ToolSafetyClass::Unknown,
            "'cargo build' should be classified as Unknown"
        );
    }

    #[test]
    fn test_classify_bash_npm_ls_readonly() {
        assert_eq!(
            classify_bash_command("npm ls"),
            ToolSafetyClass::ReadOnly,
            "'npm ls' should be classified as ReadOnly"
        );
    }

    #[test]
    fn test_classify_bash_npm_install_unknown() {
        assert_eq!(
            classify_bash_command("npm install"),
            ToolSafetyClass::Unknown,
            "'npm install' should be classified as Unknown"
        );
    }

    #[test]
    fn test_classify_bash_empty_unknown() {
        assert_eq!(
            classify_bash_command(""),
            ToolSafetyClass::Unknown,
            "Empty command should be classified as Unknown"
        );
        assert_eq!(
            classify_bash_command("   "),
            ToolSafetyClass::Unknown,
            "Whitespace-only command should be classified as Unknown"
        );
    }

    #[test]
    fn test_classify_bash_common_readonly_commands() {
        // Test a variety of common read-only commands
        let readonly_commands = [
            "head -n 10 file.txt",
            "tail -f log.txt",
            "wc -l file.txt",
            "find . -name '*.rs'",
            "grep -r 'pattern' src/",
            "pwd",
            "whoami",
            "date",
            "uname -a",
            "env",
        ];

        for cmd in readonly_commands {
            assert_eq!(
                classify_bash_command(cmd),
                ToolSafetyClass::ReadOnly,
                "'{}' should be classified as ReadOnly",
                cmd
            );
        }
    }

    // =========================================================================
    // 1.4.1 Tests for ParallelExecutor struct
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
