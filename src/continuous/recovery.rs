//! Root-cause analysis and recovery for continuous coding failures.
//!
//! Uses narsil-mcp code intelligence to identify root causes of test failures,
//! clippy warnings, and other quality gate failures. Parses common Rust error
//! formats to extract file/function locations, then queries call graphs, file
//! history, and blame information to produce actionable analysis.
//!
//! # Architecture
//!
//! 1. Parse error output to extract [`ErrorLocation`]s
//! 2. Query narsil via MCP for callers, file history, and blame
//! 3. Assemble into a structured [`RootCauseAnalysis`]
//! 4. Attempt automated recovery via [`attempt_recovery`]
//!
//! # Recovery Loop
//!
//! When a quality gate fails, the recovery loop:
//! 1. Builds a recovery prompt from the [`RootCauseAnalysis`]
//! 2. Sends it to an LLM via a [`RecoveryExecutor`]
//! 3. Re-checks quality gates
//! 4. Retries up to [`RecoveryConfig::max_attempts`] times
//!
//! # Graceful Degradation
//!
//! When narsil-mcp is unavailable, the analysis falls back to error-location
//! parsing only, returning whatever locations could be extracted from the
//! raw error output.
//!
//! # Example
//!
//! ```
//! use patina::continuous::recovery::{ErrorLocation, RootCauseAnalysis, parse_error_locations};
//!
//! let error_output = "error[E0308]: mismatched types\n --> src/main.rs:42:5";
//! let locations = parse_error_locations(error_output);
//! assert_eq!(locations.len(), 1);
//! assert_eq!(locations[0].file, "src/main.rs");
//! assert_eq!(locations[0].line, Some(42));
//! ```

use std::fmt;
use std::future::Future;

use serde::{Deserialize, Serialize};

/// A location extracted from a compiler or test error message.
///
/// Represents a file (and optionally line/column/function) referenced in
/// an error or warning.
///
/// # Example
///
/// ```
/// use patina::continuous::recovery::ErrorLocation;
///
/// let loc = ErrorLocation {
///     file: "src/main.rs".to_string(),
///     line: Some(42),
///     column: Some(5),
///     function: Some("main".to_string()),
/// };
/// assert_eq!(format!("{}", loc), "src/main.rs:42:5 (main)");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorLocation {
    /// File path relative to the project root.
    pub file: String,
    /// Line number (1-indexed), if available.
    pub line: Option<u32>,
    /// Column number (1-indexed), if available.
    pub column: Option<u32>,
    /// Function name, if extractable from the error context.
    pub function: Option<String>,
}

impl fmt::Display for ErrorLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.file)?;
        if let Some(line) = self.line {
            write!(f, ":{line}")?;
            if let Some(col) = self.column {
                write!(f, ":{col}")?;
            }
        }
        if let Some(ref func) = self.function {
            write!(f, " ({func})")?;
        }
        Ok(())
    }
}

/// Information about a recent change to a file relevant to the failure.
///
/// Extracted from narsil's `get_file_history` or `get_blame` tools.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentChange {
    /// The commit hash.
    pub commit_hash: String,
    /// Commit message summary.
    pub message: String,
    /// Author of the commit.
    pub author: String,
    /// File path that was changed.
    pub file: String,
}

/// Information about a function that calls into the failing code.
///
/// Extracted from narsil's `get_callers` tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallerContext {
    /// Name of the calling function.
    pub function: String,
    /// File containing the caller.
    pub file: String,
    /// Line number of the call site.
    pub line: u32,
}

/// Structured root-cause analysis for a quality gate failure.
///
/// Contains the extracted error locations, relevant callers, recent changes,
/// and a list of files that should be investigated.
///
/// # Example
///
/// ```
/// use patina::continuous::recovery::RootCauseAnalysis;
///
/// let analysis = RootCauseAnalysis::empty("cargo test failed");
/// assert!(analysis.error_locations.is_empty());
/// assert!(analysis.relevant_files.is_empty());
/// assert!(!analysis.narsil_available);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootCauseAnalysis {
    /// The original error output that triggered analysis.
    pub error_summary: String,

    /// Locations extracted from the error output.
    pub error_locations: Vec<ErrorLocation>,

    /// Functions that call into the failing code.
    pub callers: Vec<CallerContext>,

    /// Recent changes to relevant files.
    pub recent_changes: Vec<RecentChange>,

    /// Deduplicated list of files relevant to the failure.
    pub relevant_files: Vec<String>,

    /// Whether narsil was available for deep analysis.
    pub narsil_available: bool,
}

impl RootCauseAnalysis {
    /// Creates an empty analysis with no narsil data.
    ///
    /// Used as a fallback when narsil is unavailable or when no error
    /// locations can be extracted.
    ///
    /// # Example
    ///
    /// ```
    /// use patina::continuous::recovery::RootCauseAnalysis;
    ///
    /// let analysis = RootCauseAnalysis::empty("test failed");
    /// assert_eq!(analysis.error_summary, "test failed");
    /// assert!(!analysis.narsil_available);
    /// ```
    #[must_use]
    pub fn empty(error_summary: &str) -> Self {
        Self {
            error_summary: error_summary.to_string(),
            error_locations: Vec::new(),
            callers: Vec::new(),
            recent_changes: Vec::new(),
            relevant_files: Vec::new(),
            narsil_available: false,
        }
    }

    /// Returns true if the analysis contains actionable information.
    ///
    /// An analysis is actionable if it has at least one error location or
    /// relevant file to investigate.
    ///
    /// # Example
    ///
    /// ```
    /// use patina::continuous::recovery::{RootCauseAnalysis, ErrorLocation};
    ///
    /// let mut analysis = RootCauseAnalysis::empty("error");
    /// assert!(!analysis.is_actionable());
    ///
    /// analysis.error_locations.push(ErrorLocation {
    ///     file: "src/main.rs".to_string(),
    ///     line: Some(1),
    ///     column: None,
    ///     function: None,
    /// });
    /// assert!(analysis.is_actionable());
    /// ```
    #[must_use]
    pub fn is_actionable(&self) -> bool {
        !self.error_locations.is_empty() || !self.relevant_files.is_empty()
    }
}

impl fmt::Display for RootCauseAnalysis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Root Cause Analysis")?;
        writeln!(f, "===================")?;
        writeln!(f, "Error: {}", self.error_summary)?;
        writeln!(
            f,
            "Narsil: {}",
            if self.narsil_available {
                "available"
            } else {
                "unavailable"
            }
        )?;

        if !self.error_locations.is_empty() {
            writeln!(f, "\nError Locations:")?;
            for loc in &self.error_locations {
                writeln!(f, "  - {loc}")?;
            }
        }

        if !self.callers.is_empty() {
            writeln!(f, "\nCallers:")?;
            for caller in &self.callers {
                writeln!(
                    f,
                    "  - {}() in {}:{}",
                    caller.function, caller.file, caller.line
                )?;
            }
        }

        if !self.recent_changes.is_empty() {
            writeln!(f, "\nRecent Changes:")?;
            for change in &self.recent_changes {
                writeln!(
                    f,
                    "  - {} ({}) {}",
                    &change.commit_hash[..7.min(change.commit_hash.len())],
                    change.author,
                    change.message
                )?;
            }
        }

        if !self.relevant_files.is_empty() {
            writeln!(f, "\nRelevant Files:")?;
            for file in &self.relevant_files {
                writeln!(f, "  - {file}")?;
            }
        }

        Ok(())
    }
}

/// Parses error locations from raw compiler/test output.
///
/// Recognizes common Rust error formats:
/// - `error[E0308]: ... --> src/file.rs:42:5` (rustc errors)
/// - `warning: ... --> src/file.rs:42:5` (clippy/rustc warnings)
/// - `test module::test_name ... FAILED` with subsequent `at src/file.rs:42` (test failures)
/// - `thread 'test_name' panicked at src/file.rs:42:5` (panic locations)
///
/// # Arguments
///
/// * `output` - Raw error output from cargo test, cargo clippy, or rustc
///
/// # Returns
///
/// A vector of error locations extracted from the output, deduplicated by file+line.
///
/// # Example
///
/// ```
/// use patina::continuous::recovery::parse_error_locations;
///
/// let output = "error[E0308]: mismatched types\n  --> src/main.rs:42:5";
/// let locations = parse_error_locations(output);
/// assert_eq!(locations.len(), 1);
/// assert_eq!(locations[0].file, "src/main.rs");
/// assert_eq!(locations[0].line, Some(42));
/// assert_eq!(locations[0].column, Some(5));
/// ```
#[must_use]
pub fn parse_error_locations(output: &str) -> Vec<ErrorLocation> {
    let mut locations = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let lines: Vec<&str> = output.lines().collect();

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        // Pattern 1: " --> src/file.rs:42:5" (rustc/clippy pointer)
        if let Some(rest) = trimmed.strip_prefix("--> ") {
            if let Some(loc) = parse_file_line_col(rest) {
                let key = format!("{}:{}", loc.file, loc.line.unwrap_or(0));
                if seen.insert(key) {
                    locations.push(loc);
                }
            }
            continue;
        }

        // Pattern 2: "thread 'test_name' panicked at src/file.rs:42:5"
        if let Some(rest) = trimmed.strip_prefix("thread '") {
            if let Some(panic_idx) = rest.find("' panicked at ") {
                let test_name = &rest[..panic_idx];
                let location_str = &rest[panic_idx + "' panicked at ".len()..];
                // Remove trailing comma if present
                let location_str = location_str.trim_end_matches(',');
                if let Some(mut loc) = parse_file_line_col(location_str) {
                    if loc.function.is_none() {
                        loc.function = extract_test_function_name(test_name);
                    }
                    let key = format!("{}:{}", loc.file, loc.line.unwrap_or(0));
                    if seen.insert(key) {
                        locations.push(loc);
                    }
                }
            }
            continue;
        }

        // Pattern 3: "test module::test_name ... FAILED" (test failure summary)
        if trimmed.starts_with("test ") && trimmed.ends_with("FAILED") {
            let test_path = trimmed
                .strip_prefix("test ")
                .and_then(|s| s.split_whitespace().next());

            if let Some(test_path) = test_path {
                // Look ahead for "at src/..." location
                for lookahead in lines.iter().skip(i + 1).take(5) {
                    let la_trimmed = lookahead.trim();
                    if let Some(at_rest) = la_trimmed.strip_prefix("at ") {
                        if let Some(mut loc) = parse_file_line_col(at_rest) {
                            if loc.function.is_none() {
                                loc.function = extract_test_function_name(test_path);
                            }
                            let key = format!("{}:{}", loc.file, loc.line.unwrap_or(0));
                            if seen.insert(key) {
                                locations.push(loc);
                            }
                            break;
                        }
                    }
                }
            }
        }
    }

    locations
}

/// Parses a "file:line:col" string into an [`ErrorLocation`].
///
/// Handles formats like:
/// - `src/main.rs:42:5`
/// - `src/main.rs:42`
/// - `src/main.rs`
fn parse_file_line_col(s: &str) -> Option<ErrorLocation> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    // Split on colon, but be careful with Windows paths (C:\...)
    // For Rust projects we expect Unix-style relative paths
    let parts: Vec<&str> = s.splitn(3, ':').collect();

    let file = parts[0].to_string();
    // Reject if the "file" doesn't look like a file path
    if !file.contains('/') && !file.ends_with(".rs") {
        return None;
    }

    let line = parts.get(1).and_then(|s| s.parse::<u32>().ok());
    let column = parts.get(2).and_then(|s| s.parse::<u32>().ok());

    Some(ErrorLocation {
        file,
        line,
        column,
        function: None,
    })
}

/// Extracts the test function name from a test path like "module::submodule::test_name".
fn extract_test_function_name(test_path: &str) -> Option<String> {
    test_path.rsplit("::").next().map(String::from)
}

/// Parses a narsil `get_callers` response into caller context.
///
/// # Arguments
///
/// * `response` - JSON value from narsil MCP `get_callers` tool
///
/// # Returns
///
/// A vector of [`CallerContext`] extracted from the response.
#[must_use]
pub fn parse_callers_into_context(response: &serde_json::Value) -> Vec<CallerContext> {
    let Some(callers) = response.get("callers").and_then(|c| c.as_array()) else {
        return Vec::new();
    };

    callers
        .iter()
        .filter_map(|caller| {
            let function = caller.get("function")?.as_str()?.to_string();
            let file = caller.get("file")?.as_str()?.to_string();
            let line = caller.get("line")?.as_u64()? as u32;
            Some(CallerContext {
                function,
                file,
                line,
            })
        })
        .collect()
}

/// Parses a narsil `get_file_history` response into recent changes.
///
/// # Arguments
///
/// * `response` - JSON value from narsil MCP `get_file_history` tool
///
/// # Returns
///
/// A vector of [`RecentChange`] extracted from the response.
#[must_use]
pub fn parse_file_history_into_changes(response: &serde_json::Value) -> Vec<RecentChange> {
    let Some(commits) = response.get("commits").and_then(|c| c.as_array()) else {
        return Vec::new();
    };

    commits
        .iter()
        .filter_map(|commit| {
            let commit_hash = commit.get("hash")?.as_str()?.to_string();
            let message = commit
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("")
                .to_string();
            let author = commit
                .get("author")
                .and_then(|a| a.as_str())
                .unwrap_or("unknown")
                .to_string();
            let file = commit
                .get("file")
                .and_then(|f| f.as_str())
                .unwrap_or("")
                .to_string();
            Some(RecentChange {
                commit_hash,
                message,
                author,
                file,
            })
        })
        .collect()
}

/// Performs root-cause analysis using narsil MCP tools.
///
/// Parses the error output for locations, then queries narsil for:
/// - Callers of failing functions (via `get_callers`)
/// - Recent file history (via `get_file_history`)
/// - Blame information (via `get_blame`)
///
/// Falls back gracefully when narsil is unavailable, returning only
/// parsed error locations.
///
/// # Arguments
///
/// * `client` - Optional MCP client for narsil queries
/// * `repo_name` - Repository name for narsil tool calls
/// * `error_output` - Raw error output from the failing quality gate
///
/// # Returns
///
/// A [`RootCauseAnalysis`] with all available information.
///
/// # Errors
///
/// This function does not return errors; it degrades gracefully.
/// MCP call failures are logged but do not propagate.
pub async fn narsil_root_cause(
    client: Option<&mut crate::mcp::client::McpClient>,
    repo_name: &str,
    error_output: &str,
) -> RootCauseAnalysis {
    let error_locations = parse_error_locations(error_output);

    // Truncate error summary to first 200 chars
    let error_summary = if error_output.len() > 200 {
        format!("{}...", &error_output[..200])
    } else {
        error_output.to_string()
    };

    let mut analysis = RootCauseAnalysis {
        error_summary,
        error_locations: error_locations.clone(),
        callers: Vec::new(),
        recent_changes: Vec::new(),
        relevant_files: Vec::new(),
        narsil_available: false,
    };

    // Collect relevant files from error locations
    let mut relevant_files = std::collections::HashSet::new();
    for loc in &error_locations {
        relevant_files.insert(loc.file.clone());
    }

    // If no MCP client, return parse-only analysis
    let Some(client) = client else {
        analysis.relevant_files = relevant_files.into_iter().collect();
        analysis.relevant_files.sort();
        return analysis;
    };

    analysis.narsil_available = true;

    // Query callers for functions found in error locations
    for loc in &error_locations {
        if let Some(ref func_name) = loc.function {
            if let Ok(response) = client
                .call_tool(
                    "get_callers",
                    serde_json::json!({
                        "repo": repo_name,
                        "function": func_name,
                    }),
                )
                .await
            {
                let callers = parse_callers_into_context(&response);
                for caller in &callers {
                    relevant_files.insert(caller.file.clone());
                }
                analysis.callers.extend(callers);
            }
        }
    }

    // Query file history for each error location file
    for file in &error_locations
        .iter()
        .map(|l| l.file.clone())
        .collect::<Vec<_>>()
    {
        if let Ok(response) = client
            .call_tool(
                "get_file_history",
                serde_json::json!({
                    "repo": repo_name,
                    "path": file,
                    "max_commits": 5,
                }),
            )
            .await
        {
            let changes = parse_file_history_into_changes(&response);
            analysis.recent_changes.extend(changes);
        }
    }

    analysis.relevant_files = relevant_files.into_iter().collect();
    analysis.relevant_files.sort();

    analysis
}

// ============================================================================
// Recovery Loop (Phase 6.4)
// ============================================================================

/// Outcome of the recovery loop after attempting to fix quality gate failures.
///
/// # Variants
///
/// - `Fixed`: The issue was resolved after one or more attempts.
/// - `NeedsHuman`: All recovery attempts failed; human intervention is required.
///
/// # Example
///
/// ```
/// use patina::continuous::recovery::RecoveryOutcome;
///
/// let outcome = RecoveryOutcome::Fixed { attempts: 2 };
/// assert!(outcome.is_fixed());
/// assert_eq!(outcome.attempt_count(), 2);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecoveryOutcome {
    /// The issue was successfully fixed.
    Fixed {
        /// Number of attempts it took to fix the issue.
        attempts: u32,
    },
    /// Recovery failed; human intervention is needed.
    NeedsHuman {
        /// Total attempts made before giving up.
        attempts: u32,
        /// Error output from the last failed attempt.
        last_error: String,
    },
}

impl RecoveryOutcome {
    /// Returns `true` if the recovery was successful.
    ///
    /// # Example
    ///
    /// ```
    /// use patina::continuous::recovery::RecoveryOutcome;
    ///
    /// assert!(RecoveryOutcome::Fixed { attempts: 1 }.is_fixed());
    /// assert!(!RecoveryOutcome::NeedsHuman {
    ///     attempts: 3,
    ///     last_error: "err".into(),
    /// }.is_fixed());
    /// ```
    #[must_use]
    pub fn is_fixed(&self) -> bool {
        matches!(self, Self::Fixed { .. })
    }

    /// Returns the number of attempts made.
    ///
    /// # Example
    ///
    /// ```
    /// use patina::continuous::recovery::RecoveryOutcome;
    ///
    /// let outcome = RecoveryOutcome::Fixed { attempts: 2 };
    /// assert_eq!(outcome.attempt_count(), 2);
    /// ```
    #[must_use]
    pub fn attempt_count(&self) -> u32 {
        match self {
            Self::Fixed { attempts } | Self::NeedsHuman { attempts, .. } => *attempts,
        }
    }
}

impl fmt::Display for RecoveryOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fixed { attempts } => write!(f, "Fixed after {attempts} attempt(s)"),
            Self::NeedsHuman {
                attempts,
                last_error,
            } => {
                write!(
                    f,
                    "Needs human intervention after {attempts} attempt(s): {last_error}"
                )
            }
        }
    }
}

/// Configuration for the recovery loop.
///
/// # Example
///
/// ```
/// use patina::continuous::recovery::RecoveryConfig;
///
/// let config = RecoveryConfig::default();
/// assert_eq!(config.max_attempts, 3);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryConfig {
    /// Maximum number of recovery attempts before giving up.
    pub max_attempts: u32,
}

impl Default for RecoveryConfig {
    fn default() -> Self {
        Self { max_attempts: 3 }
    }
}

/// Result of checking quality gates after a recovery attempt.
///
/// # Example
///
/// ```
/// use patina::continuous::recovery::GateCheckResult;
///
/// let result = GateCheckResult { passed: true, error_output: String::new() };
/// assert!(result.passed);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateCheckResult {
    /// Whether all quality gates passed.
    pub passed: bool,
    /// Error output if any gate failed.
    pub error_output: String,
}

/// Record of a single recovery attempt for logging and diagnostics.
///
/// # Example
///
/// ```
/// use patina::continuous::recovery::RecoveryAttemptRecord;
///
/// let record = RecoveryAttemptRecord {
///     attempt: 1,
///     prompt_summary: "Fix type mismatch in src/main.rs".to_string(),
///     gate_passed: true,
/// };
/// assert_eq!(record.attempt, 1);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryAttemptRecord {
    /// Attempt number (1-indexed).
    pub attempt: u32,
    /// Summary of the recovery prompt sent.
    pub prompt_summary: String,
    /// Whether quality gates passed after this attempt.
    pub gate_passed: bool,
}

/// Metrics tracked across recovery sessions.
///
/// # Example
///
/// ```
/// use patina::continuous::recovery::RecoveryMetrics;
///
/// let metrics = RecoveryMetrics::default();
/// assert_eq!(metrics.total_sessions, 0);
/// ```
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecoveryMetrics {
    /// Total recovery sessions initiated.
    pub total_sessions: u32,
    /// Number of sessions that ended with `Fixed`.
    pub successful_sessions: u32,
    /// Total attempts across all sessions.
    pub total_attempts: u32,
}

impl RecoveryMetrics {
    /// Returns the success rate as a fraction in [0.0, 1.0].
    ///
    /// Returns 0.0 if no sessions have been recorded.
    ///
    /// # Example
    ///
    /// ```
    /// use patina::continuous::recovery::{RecoveryMetrics, RecoveryOutcome};
    ///
    /// let mut metrics = RecoveryMetrics::default();
    /// metrics.record_session(&RecoveryOutcome::Fixed { attempts: 1 });
    /// assert!((metrics.success_rate() - 1.0).abs() < f64::EPSILON);
    /// ```
    #[must_use]
    pub fn success_rate(&self) -> f64 {
        if self.total_sessions == 0 {
            return 0.0;
        }
        f64::from(self.successful_sessions) / f64::from(self.total_sessions)
    }

    /// Returns the average number of attempts per successful fix.
    ///
    /// Returns 0.0 if no successful sessions have been recorded.
    ///
    /// # Example
    ///
    /// ```
    /// use patina::continuous::recovery::{RecoveryMetrics, RecoveryOutcome};
    ///
    /// let mut metrics = RecoveryMetrics::default();
    /// metrics.record_session(&RecoveryOutcome::Fixed { attempts: 2 });
    /// assert!((metrics.avg_attempts_per_fix() - 2.0).abs() < f64::EPSILON);
    /// ```
    #[must_use]
    pub fn avg_attempts_per_fix(&self) -> f64 {
        if self.successful_sessions == 0 {
            return 0.0;
        }
        f64::from(self.total_attempts) / f64::from(self.successful_sessions)
    }

    /// Records a recovery session outcome.
    ///
    /// Updates total sessions, successful sessions, and total attempts
    /// based on the outcome.
    pub fn record_session(&mut self, outcome: &RecoveryOutcome) {
        self.total_sessions += 1;
        self.total_attempts += outcome.attempt_count();
        if outcome.is_fixed() {
            self.successful_sessions += 1;
        }
    }
}

/// Trait abstracting the execution steps of recovery for testability.
///
/// Implementations handle:
/// 1. Sending a recovery prompt to an LLM and applying generated fixes
/// 2. Re-checking quality gates after fixes are applied
///
/// # Testing
///
/// Use mock implementations of this trait to test recovery logic without
/// requiring a real LLM or build system.
pub trait RecoveryExecutor: Send {
    /// Execute a recovery prompt: send to LLM and apply generated fixes.
    ///
    /// # Errors
    ///
    /// Returns an error if the LLM call fails or the fix cannot be applied.
    fn execute_fix(
        &mut self,
        prompt: &str,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    /// Re-run quality gates after a fix attempt.
    ///
    /// Returns a [`GateCheckResult`] indicating whether all gates passed
    /// and any error output from failed gates.
    fn check_gates(&mut self) -> impl Future<Output = GateCheckResult> + Send;
}

/// Builds a recovery prompt from a root-cause analysis.
///
/// The prompt includes error locations, relevant files, callers, and recent
/// changes to help the LLM understand what needs to be fixed.
///
/// # Arguments
///
/// * `analysis` - The root-cause analysis to build the prompt from
/// * `attempt` - The current attempt number (1-indexed)
/// * `current_error` - The current error output to include in the prompt
///
/// # Returns
///
/// A formatted recovery prompt string.
#[must_use]
pub fn build_recovery_prompt(
    _analysis: &RootCauseAnalysis,
    _attempt: u32,
    _current_error: &str,
) -> String {
    // Stub: returns empty string. Tests will fail (RED phase).
    String::new()
}

/// Attempts to recover from a quality gate failure using LLM-powered fixes.
///
/// Iterates up to `config.max_attempts` times:
/// 1. Builds a recovery prompt from the root-cause analysis
/// 2. Sends the prompt to the LLM via the executor
/// 3. Re-runs quality gates
/// 4. If gates pass, returns `Fixed`; otherwise, retries with updated error context
///
/// # Arguments
///
/// * `analysis` - Root-cause analysis of the failure
/// * `config` - Recovery configuration (max attempts, etc.)
/// * `executor` - Implementation of fix execution and gate checking
/// * `metrics` - Metrics tracker (updated with session results)
///
/// # Returns
///
/// A [`RecoveryOutcome`] indicating success or need for human intervention.
pub async fn attempt_recovery<E: RecoveryExecutor>(
    _analysis: &RootCauseAnalysis,
    _config: &RecoveryConfig,
    _executor: &mut E,
    _metrics: &mut RecoveryMetrics,
) -> RecoveryOutcome {
    // Stub: always returns NeedsHuman. Tests will fail (RED phase).
    RecoveryOutcome::NeedsHuman {
        attempts: 0,
        last_error: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // ErrorLocation tests
    // =========================================================================

    #[test]
    fn error_location_display_full() {
        let loc = ErrorLocation {
            file: "src/main.rs".to_string(),
            line: Some(42),
            column: Some(5),
            function: Some("main".to_string()),
        };
        assert_eq!(format!("{loc}"), "src/main.rs:42:5 (main)");
    }

    #[test]
    fn error_location_display_no_column() {
        let loc = ErrorLocation {
            file: "src/lib.rs".to_string(),
            line: Some(10),
            column: None,
            function: None,
        };
        assert_eq!(format!("{loc}"), "src/lib.rs:10");
    }

    #[test]
    fn error_location_display_file_only() {
        let loc = ErrorLocation {
            file: "src/app/mod.rs".to_string(),
            line: None,
            column: None,
            function: None,
        };
        assert_eq!(format!("{loc}"), "src/app/mod.rs");
    }

    #[test]
    fn error_location_display_with_function_no_column() {
        let loc = ErrorLocation {
            file: "src/api.rs".to_string(),
            line: Some(100),
            column: None,
            function: Some("process".to_string()),
        };
        assert_eq!(format!("{loc}"), "src/api.rs:100 (process)");
    }

    #[test]
    fn error_location_equality() {
        let a = ErrorLocation {
            file: "src/main.rs".to_string(),
            line: Some(42),
            column: Some(5),
            function: None,
        };
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn error_location_serde_roundtrip() {
        let loc = ErrorLocation {
            file: "src/main.rs".to_string(),
            line: Some(42),
            column: Some(5),
            function: Some("main".to_string()),
        };
        let json = serde_json::to_string(&loc).expect("serialize");
        let restored: ErrorLocation = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(loc, restored);
    }

    // =========================================================================
    // RootCauseAnalysis tests
    // =========================================================================

    #[test]
    fn root_cause_analysis_empty() {
        let analysis = RootCauseAnalysis::empty("some error");
        assert_eq!(analysis.error_summary, "some error");
        assert!(analysis.error_locations.is_empty());
        assert!(analysis.callers.is_empty());
        assert!(analysis.recent_changes.is_empty());
        assert!(analysis.relevant_files.is_empty());
        assert!(!analysis.narsil_available);
    }

    #[test]
    fn root_cause_analysis_not_actionable_when_empty() {
        let analysis = RootCauseAnalysis::empty("error");
        assert!(!analysis.is_actionable());
    }

    #[test]
    fn root_cause_analysis_actionable_with_location() {
        let mut analysis = RootCauseAnalysis::empty("error");
        analysis.error_locations.push(ErrorLocation {
            file: "src/main.rs".to_string(),
            line: Some(1),
            column: None,
            function: None,
        });
        assert!(analysis.is_actionable());
    }

    #[test]
    fn root_cause_analysis_actionable_with_relevant_file() {
        let mut analysis = RootCauseAnalysis::empty("error");
        analysis.relevant_files.push("src/lib.rs".to_string());
        assert!(analysis.is_actionable());
    }

    #[test]
    fn root_cause_analysis_display() {
        let analysis = RootCauseAnalysis {
            error_summary: "test failed".to_string(),
            error_locations: vec![ErrorLocation {
                file: "src/main.rs".to_string(),
                line: Some(42),
                column: Some(5),
                function: None,
            }],
            callers: vec![CallerContext {
                function: "run".to_string(),
                file: "src/app.rs".to_string(),
                line: 10,
            }],
            recent_changes: vec![RecentChange {
                commit_hash: "abc1234def".to_string(),
                message: "feat: add feature".to_string(),
                author: "dev".to_string(),
                file: "src/main.rs".to_string(),
            }],
            relevant_files: vec!["src/main.rs".to_string(), "src/app.rs".to_string()],
            narsil_available: true,
        };

        let output = format!("{analysis}");
        assert!(output.contains("Root Cause Analysis"));
        assert!(output.contains("src/main.rs:42:5"));
        assert!(output.contains("run()"));
        assert!(output.contains("abc1234"));
        assert!(output.contains("Narsil: available"));
    }

    #[test]
    fn root_cause_analysis_serde_roundtrip() {
        let analysis = RootCauseAnalysis {
            error_summary: "test failed".to_string(),
            error_locations: vec![ErrorLocation {
                file: "src/main.rs".to_string(),
                line: Some(42),
                column: Some(5),
                function: Some("test_something".to_string()),
            }],
            callers: vec![CallerContext {
                function: "run".to_string(),
                file: "src/app.rs".to_string(),
                line: 10,
            }],
            recent_changes: vec![RecentChange {
                commit_hash: "abc1234".to_string(),
                message: "fix".to_string(),
                author: "dev".to_string(),
                file: "src/main.rs".to_string(),
            }],
            relevant_files: vec!["src/main.rs".to_string()],
            narsil_available: true,
        };

        let json = serde_json::to_string(&analysis).expect("serialize");
        let restored: RootCauseAnalysis = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(analysis, restored);
    }

    // =========================================================================
    // parse_error_locations - rustc errors
    // =========================================================================

    #[test]
    fn parse_rustc_error_with_arrow() {
        let output = "error[E0308]: mismatched types\n  --> src/main.rs:42:5\n   |\n42 |     let x: u32 = \"hello\";\n   |                  ^^^^^^^";
        let locations = parse_error_locations(output);
        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].file, "src/main.rs");
        assert_eq!(locations[0].line, Some(42));
        assert_eq!(locations[0].column, Some(5));
    }

    #[test]
    fn parse_multiple_rustc_errors() {
        let output = "\
error[E0308]: mismatched types
  --> src/main.rs:42:5
error[E0425]: cannot find value `x`
  --> src/lib.rs:10:9";
        let locations = parse_error_locations(output);
        assert_eq!(locations.len(), 2);
        assert_eq!(locations[0].file, "src/main.rs");
        assert_eq!(locations[1].file, "src/lib.rs");
    }

    #[test]
    fn parse_clippy_warning() {
        let output = "\
warning: unused variable: `x`
  --> src/app/state.rs:100:9
  = note: `#[warn(unused_variables)]` on by default";
        let locations = parse_error_locations(output);
        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].file, "src/app/state.rs");
        assert_eq!(locations[0].line, Some(100));
        assert_eq!(locations[0].column, Some(9));
    }

    // =========================================================================
    // parse_error_locations - test failures
    // =========================================================================

    #[test]
    fn parse_panic_location() {
        let output =
            "thread 'continuous::stagnation::tests::test_something' panicked at src/continuous/stagnation.rs:42:5,";
        let locations = parse_error_locations(output);
        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].file, "src/continuous/stagnation.rs");
        assert_eq!(locations[0].line, Some(42));
        assert_eq!(locations[0].column, Some(5));
        assert_eq!(locations[0].function.as_deref(), Some("test_something"));
    }

    #[test]
    fn parse_test_failed_with_at_location() {
        let output = "\
test continuous::recovery::tests::test_parse ... FAILED
at src/continuous/recovery.rs:55";
        let locations = parse_error_locations(output);
        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].file, "src/continuous/recovery.rs");
        assert_eq!(locations[0].line, Some(55));
        assert_eq!(locations[0].function.as_deref(), Some("test_parse"));
    }

    // =========================================================================
    // parse_error_locations - edge cases
    // =========================================================================

    #[test]
    fn parse_empty_output() {
        let locations = parse_error_locations("");
        assert!(locations.is_empty());
    }

    #[test]
    fn parse_no_locations_in_output() {
        let locations = parse_error_locations("everything is fine\nno errors here");
        assert!(locations.is_empty());
    }

    #[test]
    fn parse_deduplicates_same_location() {
        let output = "\
error[E0308]: first error
  --> src/main.rs:42:5
error[E0308]: second error
  --> src/main.rs:42:5";
        let locations = parse_error_locations(output);
        assert_eq!(
            locations.len(),
            1,
            "duplicate locations should be deduplicated"
        );
    }

    #[test]
    fn parse_different_lines_same_file() {
        let output = "\
error: first
  --> src/main.rs:10:1
error: second
  --> src/main.rs:20:1";
        let locations = parse_error_locations(output);
        assert_eq!(locations.len(), 2);
    }

    #[test]
    fn parse_line_without_column() {
        let output = "thread 'test' panicked at src/main.rs:42";
        let locations = parse_error_locations(output);
        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].file, "src/main.rs");
        assert_eq!(locations[0].line, Some(42));
        assert_eq!(locations[0].column, None);
    }

    // =========================================================================
    // parse_callers_into_context
    // =========================================================================

    #[test]
    fn parse_callers_response_valid() {
        let response = serde_json::json!({
            "callers": [
                {"function": "main", "file": "src/main.rs", "line": 10},
                {"function": "run", "file": "src/app.rs", "line": 25}
            ]
        });
        let callers = parse_callers_into_context(&response);
        assert_eq!(callers.len(), 2);
        assert_eq!(callers[0].function, "main");
        assert_eq!(callers[0].file, "src/main.rs");
        assert_eq!(callers[0].line, 10);
        assert_eq!(callers[1].function, "run");
    }

    #[test]
    fn parse_callers_response_empty() {
        let response = serde_json::json!({"callers": []});
        let callers = parse_callers_into_context(&response);
        assert!(callers.is_empty());
    }

    #[test]
    fn parse_callers_response_missing_field() {
        let response = serde_json::json!({"other": "data"});
        let callers = parse_callers_into_context(&response);
        assert!(callers.is_empty());
    }

    #[test]
    fn parse_callers_response_partial_entry() {
        let response = serde_json::json!({
            "callers": [
                {"function": "main"},
                {"function": "run", "file": "src/app.rs", "line": 25}
            ]
        });
        let callers = parse_callers_into_context(&response);
        assert_eq!(callers.len(), 1, "incomplete entries should be skipped");
        assert_eq!(callers[0].function, "run");
    }

    // =========================================================================
    // parse_file_history_into_changes
    // =========================================================================

    #[test]
    fn parse_file_history_valid() {
        let response = serde_json::json!({
            "commits": [
                {
                    "hash": "abc1234",
                    "message": "feat: add feature",
                    "author": "dev",
                    "file": "src/main.rs"
                }
            ]
        });
        let changes = parse_file_history_into_changes(&response);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].commit_hash, "abc1234");
        assert_eq!(changes[0].message, "feat: add feature");
        assert_eq!(changes[0].author, "dev");
        assert_eq!(changes[0].file, "src/main.rs");
    }

    #[test]
    fn parse_file_history_empty() {
        let response = serde_json::json!({"commits": []});
        let changes = parse_file_history_into_changes(&response);
        assert!(changes.is_empty());
    }

    #[test]
    fn parse_file_history_missing_field() {
        let response = serde_json::json!({"other": "data"});
        let changes = parse_file_history_into_changes(&response);
        assert!(changes.is_empty());
    }

    #[test]
    fn parse_file_history_defaults_for_optional_fields() {
        let response = serde_json::json!({
            "commits": [
                {"hash": "abc1234"}
            ]
        });
        let changes = parse_file_history_into_changes(&response);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].author, "unknown");
        assert_eq!(changes[0].message, "");
        assert_eq!(changes[0].file, "");
    }

    // =========================================================================
    // narsil_root_cause - without MCP client (fallback)
    // =========================================================================

    #[tokio::test]
    async fn narsil_root_cause_without_client() {
        let output = "error[E0308]: mismatched types\n  --> src/main.rs:42:5";
        let analysis = narsil_root_cause(None, "test-repo", output).await;

        assert!(!analysis.narsil_available);
        assert_eq!(analysis.error_locations.len(), 1);
        assert_eq!(analysis.error_locations[0].file, "src/main.rs");
        assert!(analysis.callers.is_empty());
        assert!(analysis.recent_changes.is_empty());
        assert_eq!(analysis.relevant_files, vec!["src/main.rs"]);
    }

    #[tokio::test]
    async fn narsil_root_cause_empty_output() {
        let analysis = narsil_root_cause(None, "test-repo", "").await;

        assert!(!analysis.narsil_available);
        assert!(analysis.error_locations.is_empty());
        assert!(analysis.relevant_files.is_empty());
        assert!(!analysis.is_actionable());
    }

    #[tokio::test]
    async fn narsil_root_cause_truncates_long_error_summary() {
        let long_output = "x".repeat(500);
        let analysis = narsil_root_cause(None, "test-repo", &long_output).await;

        assert!(analysis.error_summary.len() <= 204); // 200 + "..."
        assert!(analysis.error_summary.ends_with("..."));
    }

    #[tokio::test]
    async fn narsil_root_cause_multiple_locations() {
        let output = "\
error[E0308]: first
  --> src/a.rs:10:1
error[E0425]: second
  --> src/b.rs:20:1";
        let analysis = narsil_root_cause(None, "test-repo", output).await;

        assert_eq!(analysis.error_locations.len(), 2);
        assert_eq!(analysis.relevant_files.len(), 2);
        assert!(analysis.relevant_files.contains(&"src/a.rs".to_string()));
        assert!(analysis.relevant_files.contains(&"src/b.rs".to_string()));
    }

    // =========================================================================
    // RecentChange and CallerContext
    // =========================================================================

    #[test]
    fn recent_change_serde_roundtrip() {
        let change = RecentChange {
            commit_hash: "abc1234".to_string(),
            message: "fix: bug".to_string(),
            author: "dev".to_string(),
            file: "src/main.rs".to_string(),
        };
        let json = serde_json::to_string(&change).expect("serialize");
        let restored: RecentChange = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(change, restored);
    }

    #[test]
    fn caller_context_serde_roundtrip() {
        let caller = CallerContext {
            function: "main".to_string(),
            file: "src/main.rs".to_string(),
            line: 10,
        };
        let json = serde_json::to_string(&caller).expect("serialize");
        let restored: CallerContext = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(caller, restored);
    }

    // =========================================================================
    // extract_test_function_name
    // =========================================================================

    #[test]
    fn extract_function_from_test_path() {
        assert_eq!(
            extract_test_function_name("module::submodule::test_something"),
            Some("test_something".to_string())
        );
    }

    #[test]
    fn extract_function_from_simple_name() {
        assert_eq!(
            extract_test_function_name("test_simple"),
            Some("test_simple".to_string())
        );
    }

    #[test]
    fn extract_function_from_deeply_nested() {
        assert_eq!(
            extract_test_function_name("a::b::c::d::test_deep"),
            Some("test_deep".to_string())
        );
    }

    // =========================================================================
    // Recovery Loop tests (Phase 6.4)
    // =========================================================================

    // ---- Mock executor for testing ----

    struct MockRecoveryExecutor {
        fix_results: Vec<Result<(), String>>,
        gate_results: Vec<GateCheckResult>,
        fix_call_count: usize,
        gate_call_count: usize,
        prompts_received: Vec<String>,
    }

    impl MockRecoveryExecutor {
        fn new(
            fix_results: Vec<Result<(), String>>,
            gate_results: Vec<GateCheckResult>,
        ) -> Self {
            Self {
                fix_results,
                gate_results,
                fix_call_count: 0,
                gate_call_count: 0,
                prompts_received: Vec::new(),
            }
        }

        /// Creates an executor where every fix succeeds and the gate passes
        /// on the first check.
        fn always_succeeds() -> Self {
            Self::new(
                vec![Ok(())],
                vec![GateCheckResult {
                    passed: true,
                    error_output: String::new(),
                }],
            )
        }

        /// Creates an executor where every fix succeeds but the gate always fails.
        fn always_fails(error: &str) -> Self {
            Self::new(
                vec![Ok(())],
                vec![GateCheckResult {
                    passed: false,
                    error_output: error.to_string(),
                }],
            )
        }

        /// Creates an executor that fails N times then succeeds.
        fn succeeds_after(failures: u32, error: &str) -> Self {
            let fix_results: Vec<Result<(), String>> =
                vec![Ok(()); (failures + 1) as usize];
            let mut gate_results: Vec<GateCheckResult> = (0..failures)
                .map(|_| GateCheckResult {
                    passed: false,
                    error_output: error.to_string(),
                })
                .collect();
            gate_results.push(GateCheckResult {
                passed: true,
                error_output: String::new(),
            });
            Self::new(fix_results, gate_results)
        }
    }

    impl RecoveryExecutor for MockRecoveryExecutor {
        fn execute_fix(
            &mut self,
            prompt: &str,
        ) -> impl std::future::Future<Output = anyhow::Result<()>> + Send {
            self.prompts_received.push(prompt.to_string());
            let idx = self
                .fix_call_count
                .min(self.fix_results.len().saturating_sub(1));
            self.fix_call_count += 1;
            let result = self.fix_results[idx].clone();
            async move {
                match result {
                    Ok(()) => Ok(()),
                    Err(e) => Err(anyhow::anyhow!("{}", e)),
                }
            }
        }

        fn check_gates(
            &mut self,
        ) -> impl std::future::Future<Output = GateCheckResult> + Send {
            let idx = self
                .gate_call_count
                .min(self.gate_results.len().saturating_sub(1));
            self.gate_call_count += 1;
            let result = self.gate_results[idx].clone();
            async move { result }
        }
    }

    // ---- Helper to create a test analysis ----

    fn test_analysis() -> RootCauseAnalysis {
        RootCauseAnalysis {
            error_summary: "error[E0308]: mismatched types".to_string(),
            error_locations: vec![ErrorLocation {
                file: "src/main.rs".to_string(),
                line: Some(42),
                column: Some(5),
                function: Some("process_data".to_string()),
            }],
            callers: vec![CallerContext {
                function: "run".to_string(),
                file: "src/app.rs".to_string(),
                line: 10,
            }],
            recent_changes: vec![RecentChange {
                commit_hash: "abc1234def".to_string(),
                message: "feat: add processing".to_string(),
                author: "dev".to_string(),
                file: "src/main.rs".to_string(),
            }],
            relevant_files: vec!["src/main.rs".to_string(), "src/app.rs".to_string()],
            narsil_available: true,
        }
    }

    // ---- RecoveryOutcome tests ----

    #[test]
    fn recovery_outcome_fixed_is_fixed() {
        let outcome = RecoveryOutcome::Fixed { attempts: 1 };
        assert!(outcome.is_fixed());
    }

    #[test]
    fn recovery_outcome_needs_human_is_not_fixed() {
        let outcome = RecoveryOutcome::NeedsHuman {
            attempts: 3,
            last_error: "err".to_string(),
        };
        assert!(!outcome.is_fixed());
    }

    #[test]
    fn recovery_outcome_attempt_count_fixed() {
        let outcome = RecoveryOutcome::Fixed { attempts: 2 };
        assert_eq!(outcome.attempt_count(), 2);
    }

    #[test]
    fn recovery_outcome_attempt_count_needs_human() {
        let outcome = RecoveryOutcome::NeedsHuman {
            attempts: 5,
            last_error: "err".to_string(),
        };
        assert_eq!(outcome.attempt_count(), 5);
    }

    #[test]
    fn recovery_outcome_display_fixed() {
        let outcome = RecoveryOutcome::Fixed { attempts: 2 };
        let display = format!("{outcome}");
        assert!(display.contains("Fixed"));
        assert!(display.contains("2"));
    }

    #[test]
    fn recovery_outcome_display_needs_human() {
        let outcome = RecoveryOutcome::NeedsHuman {
            attempts: 3,
            last_error: "test failure".to_string(),
        };
        let display = format!("{outcome}");
        assert!(display.contains("human"));
        assert!(display.contains("3"));
        assert!(display.contains("test failure"));
    }

    #[test]
    fn recovery_outcome_serde_roundtrip_fixed() {
        let outcome = RecoveryOutcome::Fixed { attempts: 2 };
        let json = serde_json::to_string(&outcome).expect("serialize");
        let restored: RecoveryOutcome = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(outcome, restored);
    }

    #[test]
    fn recovery_outcome_serde_roundtrip_needs_human() {
        let outcome = RecoveryOutcome::NeedsHuman {
            attempts: 3,
            last_error: "err".to_string(),
        };
        let json = serde_json::to_string(&outcome).expect("serialize");
        let restored: RecoveryOutcome = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(outcome, restored);
    }

    // ---- RecoveryConfig tests ----

    #[test]
    fn recovery_config_default() {
        let config = RecoveryConfig::default();
        assert_eq!(config.max_attempts, 3);
    }

    #[test]
    fn recovery_config_custom() {
        let config = RecoveryConfig { max_attempts: 5 };
        assert_eq!(config.max_attempts, 5);
    }

    #[test]
    fn recovery_config_serde_roundtrip() {
        let config = RecoveryConfig { max_attempts: 7 };
        let json = serde_json::to_string(&config).expect("serialize");
        let restored: RecoveryConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(config, restored);
    }

    // ---- GateCheckResult tests ----

    #[test]
    fn gate_check_result_passed() {
        let result = GateCheckResult {
            passed: true,
            error_output: String::new(),
        };
        assert!(result.passed);
        assert!(result.error_output.is_empty());
    }

    #[test]
    fn gate_check_result_failed() {
        let result = GateCheckResult {
            passed: false,
            error_output: "test failed".to_string(),
        };
        assert!(!result.passed);
        assert_eq!(result.error_output, "test failed");
    }

    #[test]
    fn gate_check_result_serde_roundtrip() {
        let result = GateCheckResult {
            passed: false,
            error_output: "error".to_string(),
        };
        let json = serde_json::to_string(&result).expect("serialize");
        let restored: GateCheckResult = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(result, restored);
    }

    // ---- RecoveryMetrics tests ----

    #[test]
    fn recovery_metrics_default() {
        let metrics = RecoveryMetrics::default();
        assert_eq!(metrics.total_sessions, 0);
        assert_eq!(metrics.successful_sessions, 0);
        assert_eq!(metrics.total_attempts, 0);
    }

    #[test]
    fn recovery_metrics_success_rate_no_sessions() {
        let metrics = RecoveryMetrics::default();
        assert!((metrics.success_rate() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn recovery_metrics_success_rate_all_successful() {
        let mut metrics = RecoveryMetrics::default();
        metrics.record_session(&RecoveryOutcome::Fixed { attempts: 1 });
        metrics.record_session(&RecoveryOutcome::Fixed { attempts: 2 });
        assert!((metrics.success_rate() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn recovery_metrics_success_rate_half() {
        let mut metrics = RecoveryMetrics::default();
        metrics.record_session(&RecoveryOutcome::Fixed { attempts: 1 });
        metrics.record_session(&RecoveryOutcome::NeedsHuman {
            attempts: 3,
            last_error: "err".to_string(),
        });
        assert!((metrics.success_rate() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn recovery_metrics_avg_attempts_per_fix() {
        let mut metrics = RecoveryMetrics::default();
        metrics.record_session(&RecoveryOutcome::Fixed { attempts: 1 });
        metrics.record_session(&RecoveryOutcome::Fixed { attempts: 3 });
        // Total attempts: 4, successful sessions: 2, avg: 2.0
        assert!((metrics.avg_attempts_per_fix() - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn recovery_metrics_avg_attempts_no_fixes() {
        let mut metrics = RecoveryMetrics::default();
        metrics.record_session(&RecoveryOutcome::NeedsHuman {
            attempts: 3,
            last_error: "err".to_string(),
        });
        assert!((metrics.avg_attempts_per_fix() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn recovery_metrics_record_session_updates_counters() {
        let mut metrics = RecoveryMetrics::default();

        metrics.record_session(&RecoveryOutcome::Fixed { attempts: 2 });
        assert_eq!(metrics.total_sessions, 1);
        assert_eq!(metrics.successful_sessions, 1);
        assert_eq!(metrics.total_attempts, 2);

        metrics.record_session(&RecoveryOutcome::NeedsHuman {
            attempts: 3,
            last_error: "err".to_string(),
        });
        assert_eq!(metrics.total_sessions, 2);
        assert_eq!(metrics.successful_sessions, 1);
        assert_eq!(metrics.total_attempts, 5);
    }

    // ---- build_recovery_prompt tests ----

    #[test]
    fn build_prompt_includes_error() {
        let analysis = test_analysis();
        let prompt = build_recovery_prompt(&analysis, 1, "error[E0308]: mismatched types");
        assert!(
            prompt.contains("mismatched types"),
            "prompt should include the error: {prompt}"
        );
    }

    #[test]
    fn build_prompt_includes_error_locations() {
        let analysis = test_analysis();
        let prompt = build_recovery_prompt(&analysis, 1, "error");
        assert!(
            prompt.contains("src/main.rs"),
            "prompt should include error location: {prompt}"
        );
    }

    #[test]
    fn build_prompt_includes_relevant_files() {
        let analysis = test_analysis();
        let prompt = build_recovery_prompt(&analysis, 1, "error");
        assert!(
            prompt.contains("src/app.rs"),
            "prompt should include relevant files: {prompt}"
        );
    }

    #[test]
    fn build_prompt_includes_callers() {
        let analysis = test_analysis();
        let prompt = build_recovery_prompt(&analysis, 1, "error");
        assert!(
            prompt.contains("run()") || prompt.contains("run"),
            "prompt should include caller info: {prompt}"
        );
    }

    #[test]
    fn build_prompt_includes_recent_changes() {
        let analysis = test_analysis();
        let prompt = build_recovery_prompt(&analysis, 1, "error");
        assert!(
            prompt.contains("abc1234"),
            "prompt should include recent changes: {prompt}"
        );
    }

    #[test]
    fn build_prompt_includes_attempt_context_on_retry() {
        let analysis = test_analysis();
        let prompt = build_recovery_prompt(&analysis, 3, "still broken");
        assert!(
            prompt.contains("3") || prompt.contains("attempt"),
            "retry prompt should mention attempt number: {prompt}"
        );
    }

    #[test]
    fn build_prompt_empty_analysis() {
        let analysis = RootCauseAnalysis::empty("some error");
        let prompt = build_recovery_prompt(&analysis, 1, "some error");
        assert!(
            prompt.contains("some error"),
            "prompt should still include error even with empty analysis: {prompt}"
        );
    }

    // ---- attempt_recovery tests ----

    #[tokio::test]
    async fn recovery_single_attempt_succeeds() {
        let analysis = test_analysis();
        let config = RecoveryConfig::default();
        let mut executor = MockRecoveryExecutor::always_succeeds();
        let mut metrics = RecoveryMetrics::default();

        let outcome = attempt_recovery(&analysis, &config, &mut executor, &mut metrics).await;

        assert_eq!(
            outcome,
            RecoveryOutcome::Fixed { attempts: 1 },
            "should succeed on first attempt"
        );
    }

    #[tokio::test]
    async fn recovery_multiple_attempts_then_succeeds() {
        let analysis = test_analysis();
        let config = RecoveryConfig { max_attempts: 5 };
        let mut executor = MockRecoveryExecutor::succeeds_after(2, "still broken");
        let mut metrics = RecoveryMetrics::default();

        let outcome = attempt_recovery(&analysis, &config, &mut executor, &mut metrics).await;

        assert_eq!(
            outcome,
            RecoveryOutcome::Fixed { attempts: 3 },
            "should succeed on third attempt"
        );
    }

    #[tokio::test]
    async fn recovery_max_attempts_respected() {
        let analysis = test_analysis();
        let config = RecoveryConfig { max_attempts: 3 };
        let mut executor = MockRecoveryExecutor::always_fails("persistent error");
        let mut metrics = RecoveryMetrics::default();

        let outcome = attempt_recovery(&analysis, &config, &mut executor, &mut metrics).await;

        assert!(!outcome.is_fixed(), "should not be fixed");
        assert_eq!(outcome.attempt_count(), 3, "should have made exactly 3 attempts");
    }

    #[tokio::test]
    async fn recovery_needs_human_contains_last_error() {
        let analysis = test_analysis();
        let config = RecoveryConfig { max_attempts: 2 };
        let mut executor = MockRecoveryExecutor::always_fails("persistent error");
        let mut metrics = RecoveryMetrics::default();

        let outcome = attempt_recovery(&analysis, &config, &mut executor, &mut metrics).await;

        match &outcome {
            RecoveryOutcome::NeedsHuman { last_error, .. } => {
                assert!(
                    last_error.contains("persistent error"),
                    "last_error should contain the gate error: {last_error}"
                );
            }
            other => panic!("expected NeedsHuman, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn recovery_updates_metrics_on_success() {
        let analysis = test_analysis();
        let config = RecoveryConfig::default();
        let mut executor = MockRecoveryExecutor::always_succeeds();
        let mut metrics = RecoveryMetrics::default();

        let _ = attempt_recovery(&analysis, &config, &mut executor, &mut metrics).await;

        assert_eq!(metrics.total_sessions, 1);
        assert_eq!(metrics.successful_sessions, 1);
        assert_eq!(metrics.total_attempts, 1);
    }

    #[tokio::test]
    async fn recovery_updates_metrics_on_failure() {
        let analysis = test_analysis();
        let config = RecoveryConfig { max_attempts: 3 };
        let mut executor = MockRecoveryExecutor::always_fails("err");
        let mut metrics = RecoveryMetrics::default();

        let _ = attempt_recovery(&analysis, &config, &mut executor, &mut metrics).await;

        assert_eq!(metrics.total_sessions, 1);
        assert_eq!(metrics.successful_sessions, 0);
        assert_eq!(metrics.total_attempts, 3);
    }

    #[tokio::test]
    async fn recovery_executor_receives_prompt() {
        let analysis = test_analysis();
        let config = RecoveryConfig { max_attempts: 1 };
        let mut executor = MockRecoveryExecutor::always_succeeds();
        let mut metrics = RecoveryMetrics::default();

        let _ = attempt_recovery(&analysis, &config, &mut executor, &mut metrics).await;

        assert_eq!(
            executor.prompts_received.len(),
            1,
            "executor should have received exactly one prompt"
        );
        assert!(
            !executor.prompts_received[0].is_empty(),
            "prompt should not be empty"
        );
    }

    #[tokio::test]
    async fn recovery_handles_executor_error() {
        let analysis = test_analysis();
        let config = RecoveryConfig { max_attempts: 2 };
        // First fix call errors, second succeeds
        let mut executor = MockRecoveryExecutor::new(
            vec![Err("LLM unavailable".to_string()), Ok(())],
            vec![GateCheckResult {
                passed: true,
                error_output: String::new(),
            }],
        );
        let mut metrics = RecoveryMetrics::default();

        let outcome = attempt_recovery(&analysis, &config, &mut executor, &mut metrics).await;

        // Should recover: first attempt fails at execute_fix, second succeeds
        assert_eq!(
            outcome,
            RecoveryOutcome::Fixed { attempts: 2 },
            "should succeed on second attempt after executor error"
        );
    }

    #[tokio::test]
    async fn recovery_single_max_attempt() {
        let analysis = test_analysis();
        let config = RecoveryConfig { max_attempts: 1 };
        let mut executor = MockRecoveryExecutor::always_fails("error");
        let mut metrics = RecoveryMetrics::default();

        let outcome = attempt_recovery(&analysis, &config, &mut executor, &mut metrics).await;

        assert!(!outcome.is_fixed());
        assert_eq!(outcome.attempt_count(), 1);
    }
}
