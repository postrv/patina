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
}
