//! Cross-agent conflict detection for parallel worktree agents.
//!
//! Detects when multiple agents modify the same files or symbols,
//! enabling proactive conflict resolution before merge.
//!
//! # Conflict Severity
//!
//! - [`ConflictSeverity::Warning`] — Two agents modified the same file
//!   but different functions/symbols (likely auto-mergeable).
//! - [`ConflictSeverity::Error`] — Two agents modified the same symbol
//!   within the same file (manual resolution required).
//!
//! # Example
//!
//! ```
//! use patina::agents::conflict::{
//!     AgentFileChanges, ConflictDetector, ConflictSeverity,
//! };
//!
//! let mut detector = ConflictDetector::new();
//! detector.add_agent_changes(AgentFileChanges {
//!     agent_name: "agent-a".into(),
//!     modified_files: vec!["src/main.rs".into()],
//!     modified_symbols: vec![("src/main.rs".into(), "run".into())],
//! });
//! detector.add_agent_changes(AgentFileChanges {
//!     agent_name: "agent-b".into(),
//!     modified_files: vec!["src/main.rs".into()],
//!     modified_symbols: vec![("src/main.rs".into(), "run".into())],
//! });
//!
//! let report = detector.detect();
//! assert!(report.has_conflicts());
//! assert!(report.has_errors());
//! ```

use std::collections::{HashMap, HashSet};
use std::fmt;

/// Severity level for a detected conflict between agents.
///
/// Higher severity means greater risk of merge failure or semantic errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConflictSeverity {
    /// Two agents modified the same file but different symbols.
    ///
    /// These conflicts are often auto-resolvable by git merge.
    Warning,

    /// Two agents modified the same symbol within the same file.
    ///
    /// These conflicts almost always require manual resolution.
    Error,
}

impl fmt::Display for ConflictSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Warning => write!(f, "warning"),
            Self::Error => write!(f, "error"),
        }
    }
}

/// A single detected conflict between two agents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// Severity of the conflict.
    pub severity: ConflictSeverity,

    /// The file where the conflict occurs.
    pub file: String,

    /// The symbol involved, if the conflict is at the symbol level.
    ///
    /// `None` for file-level (warning) conflicts.
    pub symbol: Option<String>,

    /// Names of the agents involved in the conflict.
    pub agents: Vec<String>,
}

impl fmt::Display for Conflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let agents = self.agents.join(", ");
        match (&self.severity, &self.symbol) {
            (ConflictSeverity::Error, Some(sym)) => {
                write!(f, "[ERROR] {} :: {} — agents: {}", self.file, sym, agents)
            }
            _ => {
                write!(f, "[WARNING] {} — agents: {}", self.file, agents)
            }
        }
    }
}

/// Files and symbols modified by a single agent.
///
/// Constructed by querying each agent's worktree for changes.
#[derive(Debug, Clone)]
pub struct AgentFileChanges {
    /// Name of the agent.
    pub agent_name: String,

    /// List of file paths modified by this agent.
    pub modified_files: Vec<String>,

    /// List of (file, symbol) pairs modified by this agent.
    ///
    /// The file path must also appear in `modified_files`.
    pub modified_symbols: Vec<(String, String)>,
}

/// Report summarising all detected conflicts across agents.
#[derive(Debug, Clone)]
pub struct ConflictReport {
    /// All detected conflicts, ordered by severity (errors first).
    conflicts: Vec<Conflict>,
}

impl ConflictReport {
    /// Returns `true` if any conflicts were detected.
    #[must_use]
    pub fn has_conflicts(&self) -> bool {
        !self.conflicts.is_empty()
    }

    /// Returns `true` if any error-level conflicts were detected.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.conflicts
            .iter()
            .any(|c| c.severity == ConflictSeverity::Error)
    }

    /// Returns `true` if only warning-level conflicts were detected.
    #[must_use]
    pub fn has_warnings_only(&self) -> bool {
        self.has_conflicts() && !self.has_errors()
    }

    /// Returns the total number of conflicts.
    #[must_use]
    pub fn conflict_count(&self) -> usize {
        self.conflicts.len()
    }

    /// Returns the number of error-level conflicts.
    #[must_use]
    pub fn error_count(&self) -> usize {
        self.conflicts
            .iter()
            .filter(|c| c.severity == ConflictSeverity::Error)
            .count()
    }

    /// Returns the number of warning-level conflicts.
    #[must_use]
    pub fn warning_count(&self) -> usize {
        self.conflicts
            .iter()
            .filter(|c| c.severity == ConflictSeverity::Warning)
            .count()
    }

    /// Returns a slice of all conflicts.
    #[must_use]
    pub fn conflicts(&self) -> &[Conflict] {
        &self.conflicts
    }

    /// Returns a human-readable summary of the conflict report.
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::agents::conflict::{ConflictReport, ConflictDetector};
    ///
    /// let detector = ConflictDetector::new();
    /// let report = detector.detect();
    /// assert_eq!(report.summary(), "No conflicts detected");
    /// ```
    #[must_use]
    pub fn summary(&self) -> String {
        if self.conflicts.is_empty() {
            return "No conflicts detected".to_string();
        }

        let errors = self.error_count();
        let warnings = self.warning_count();

        match (errors, warnings) {
            (0, w) => format!("{} warning(s)", w),
            (e, 0) => format!("{} error(s)", e),
            (e, w) => format!("{} error(s), {} warning(s)", e, w),
        }
    }

    /// Formats the report as a detailed merge preview string.
    ///
    /// Lists all conflicts grouped by severity, showing involved files,
    /// symbols, and agents.
    #[must_use]
    pub fn merge_preview(&self) -> String {
        if self.conflicts.is_empty() {
            return "Merge preview: No conflicts — safe to merge.".to_string();
        }

        let mut lines = vec![format!("Merge preview: {}", self.summary())];
        lines.push(String::new());

        // Errors first
        let errors: Vec<&Conflict> = self
            .conflicts
            .iter()
            .filter(|c| c.severity == ConflictSeverity::Error)
            .collect();

        if !errors.is_empty() {
            lines.push("Errors (manual resolution required):".to_string());
            for conflict in &errors {
                lines.push(format!("  {}", conflict));
            }
            lines.push(String::new());
        }

        let warnings: Vec<&Conflict> = self
            .conflicts
            .iter()
            .filter(|c| c.severity == ConflictSeverity::Warning)
            .collect();

        if !warnings.is_empty() {
            lines.push("Warnings (likely auto-mergeable):".to_string());
            for conflict in &warnings {
                lines.push(format!("  {}", conflict));
            }
        }

        lines.join("\n")
    }
}

/// Detects conflicts between changes from multiple agents.
///
/// Feed in [`AgentFileChanges`] for each agent, then call [`detect`](Self::detect)
/// to get a [`ConflictReport`].
#[derive(Debug, Default)]
pub struct ConflictDetector {
    agent_changes: Vec<AgentFileChanges>,
}

impl ConflictDetector {
    /// Creates a new empty detector.
    #[must_use]
    pub fn new() -> Self {
        Self {
            agent_changes: Vec::new(),
        }
    }

    /// Adds changes from one agent.
    pub fn add_agent_changes(&mut self, changes: AgentFileChanges) {
        self.agent_changes.push(changes);
    }

    /// Detects conflicts across all registered agent changes.
    ///
    /// The detection algorithm:
    /// 1. Build a map of file → agents that modified it.
    /// 2. For each file modified by 2+ agents, check if the same symbols were modified.
    /// 3. Same-symbol overlap → `Error`. Same-file only → `Warning`.
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::agents::conflict::{AgentFileChanges, ConflictDetector};
    ///
    /// let mut detector = ConflictDetector::new();
    /// detector.add_agent_changes(AgentFileChanges {
    ///     agent_name: "a".into(),
    ///     modified_files: vec!["src/lib.rs".into()],
    ///     modified_symbols: vec![],
    /// });
    /// let report = detector.detect();
    /// assert!(!report.has_conflicts());
    /// ```
    #[must_use]
    pub fn detect(&self) -> ConflictReport {
        let mut conflicts = Vec::new();

        // Step 1: Build file → agents map
        let mut file_to_agents: HashMap<&str, Vec<&str>> = HashMap::new();
        for changes in &self.agent_changes {
            for file in &changes.modified_files {
                file_to_agents
                    .entry(file.as_str())
                    .or_default()
                    .push(&changes.agent_name);
            }
        }

        // Step 2: For each file with 2+ agents, check for symbol overlap
        for (file, agents) in &file_to_agents {
            if agents.len() < 2 {
                continue;
            }

            // Build symbol → agents map for this file
            let mut symbol_to_agents: HashMap<&str, Vec<&str>> = HashMap::new();
            for changes in &self.agent_changes {
                if !agents.contains(&changes.agent_name.as_str()) {
                    continue;
                }
                for (sym_file, symbol) in &changes.modified_symbols {
                    if sym_file.as_str() == *file {
                        symbol_to_agents
                            .entry(symbol.as_str())
                            .or_default()
                            .push(&changes.agent_name);
                    }
                }
            }

            // Check for symbol-level conflicts (Error)
            let mut has_symbol_conflict = false;
            let mut file_agents_in_symbol_conflicts: HashSet<&str> = HashSet::new();

            for (symbol, sym_agents) in &symbol_to_agents {
                if sym_agents.len() >= 2 {
                    has_symbol_conflict = true;
                    let mut sorted_agents: Vec<String> =
                        sym_agents.iter().map(|s| (*s).to_string()).collect();
                    sorted_agents.sort();
                    sorted_agents.dedup();

                    for a in &sorted_agents {
                        file_agents_in_symbol_conflicts.insert(
                            // Re-find the reference from agents list so lifetime works
                            agents.iter().find(|&&x| x == a.as_str()).unwrap_or(&""),
                        );
                    }

                    conflicts.push(Conflict {
                        severity: ConflictSeverity::Error,
                        file: file.to_string(),
                        symbol: Some(symbol.to_string()),
                        agents: sorted_agents,
                    });
                }
            }

            // If no symbol conflict for this file, emit a file-level warning.
            // If there IS a symbol conflict, only emit a warning for agents
            // that modified the file but NOT the conflicting symbols.
            if !has_symbol_conflict {
                let mut sorted_agents: Vec<String> =
                    agents.iter().map(|s| (*s).to_string()).collect();
                sorted_agents.sort();
                sorted_agents.dedup();

                conflicts.push(Conflict {
                    severity: ConflictSeverity::Warning,
                    file: file.to_string(),
                    symbol: None,
                    agents: sorted_agents,
                });
            }
        }

        // Sort: errors first, then warnings, then by file name
        conflicts.sort_by(|a, b| {
            b.severity
                .cmp(&a.severity)
                .then_with(|| a.file.cmp(&b.file))
        });

        ConflictReport { conflicts }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // ConflictSeverity tests
    // =========================================================================

    #[test]
    fn test_severity_ordering() {
        assert!(ConflictSeverity::Warning < ConflictSeverity::Error);
    }

    #[test]
    fn test_severity_display() {
        assert_eq!(format!("{}", ConflictSeverity::Warning), "warning");
        assert_eq!(format!("{}", ConflictSeverity::Error), "error");
    }

    #[test]
    fn test_severity_equality() {
        assert_eq!(ConflictSeverity::Warning, ConflictSeverity::Warning);
        assert_eq!(ConflictSeverity::Error, ConflictSeverity::Error);
        assert_ne!(ConflictSeverity::Warning, ConflictSeverity::Error);
    }

    // =========================================================================
    // Conflict Display tests
    // =========================================================================

    #[test]
    fn test_conflict_display_warning() {
        let conflict = Conflict {
            severity: ConflictSeverity::Warning,
            file: "src/main.rs".into(),
            symbol: None,
            agents: vec!["agent-a".into(), "agent-b".into()],
        };
        let display = format!("{}", conflict);
        assert!(display.contains("[WARNING]"));
        assert!(display.contains("src/main.rs"));
        assert!(display.contains("agent-a"));
        assert!(display.contains("agent-b"));
    }

    #[test]
    fn test_conflict_display_error() {
        let conflict = Conflict {
            severity: ConflictSeverity::Error,
            file: "src/lib.rs".into(),
            symbol: Some("process_data".into()),
            agents: vec!["agent-x".into(), "agent-y".into()],
        };
        let display = format!("{}", conflict);
        assert!(display.contains("[ERROR]"));
        assert!(display.contains("src/lib.rs"));
        assert!(display.contains("process_data"));
        assert!(display.contains("agent-x"));
    }

    // =========================================================================
    // ConflictDetector — no conflicts
    // =========================================================================

    #[test]
    fn test_no_agents_no_conflicts() {
        let detector = ConflictDetector::new();
        let report = detector.detect();
        assert!(!report.has_conflicts());
        assert_eq!(report.conflict_count(), 0);
    }

    #[test]
    fn test_single_agent_no_conflicts() {
        let mut detector = ConflictDetector::new();
        detector.add_agent_changes(AgentFileChanges {
            agent_name: "agent-a".into(),
            modified_files: vec!["src/main.rs".into(), "src/lib.rs".into()],
            modified_symbols: vec![("src/main.rs".into(), "main".into())],
        });

        let report = detector.detect();
        assert!(!report.has_conflicts());
    }

    #[test]
    fn test_non_overlapping_files_no_conflicts() {
        let mut detector = ConflictDetector::new();
        detector.add_agent_changes(AgentFileChanges {
            agent_name: "agent-a".into(),
            modified_files: vec!["src/api.rs".into()],
            modified_symbols: vec![],
        });
        detector.add_agent_changes(AgentFileChanges {
            agent_name: "agent-b".into(),
            modified_files: vec!["src/tui.rs".into()],
            modified_symbols: vec![],
        });

        let report = detector.detect();
        assert!(!report.has_conflicts());
    }

    // =========================================================================
    // ConflictDetector — file-level warnings
    // =========================================================================

    #[test]
    fn test_same_file_different_symbols_is_warning() {
        let mut detector = ConflictDetector::new();
        detector.add_agent_changes(AgentFileChanges {
            agent_name: "agent-a".into(),
            modified_files: vec!["src/main.rs".into()],
            modified_symbols: vec![("src/main.rs".into(), "function_a".into())],
        });
        detector.add_agent_changes(AgentFileChanges {
            agent_name: "agent-b".into(),
            modified_files: vec!["src/main.rs".into()],
            modified_symbols: vec![("src/main.rs".into(), "function_b".into())],
        });

        let report = detector.detect();
        assert!(report.has_conflicts());
        assert!(report.has_warnings_only());
        assert_eq!(report.warning_count(), 1);
        assert_eq!(report.error_count(), 0);
    }

    #[test]
    fn test_same_file_no_symbols_is_warning() {
        let mut detector = ConflictDetector::new();
        detector.add_agent_changes(AgentFileChanges {
            agent_name: "agent-a".into(),
            modified_files: vec!["src/main.rs".into()],
            modified_symbols: vec![],
        });
        detector.add_agent_changes(AgentFileChanges {
            agent_name: "agent-b".into(),
            modified_files: vec!["src/main.rs".into()],
            modified_symbols: vec![],
        });

        let report = detector.detect();
        assert!(report.has_conflicts());
        assert!(report.has_warnings_only());
    }

    // =========================================================================
    // ConflictDetector — symbol-level errors
    // =========================================================================

    #[test]
    fn test_same_file_same_symbol_is_error() {
        let mut detector = ConflictDetector::new();
        detector.add_agent_changes(AgentFileChanges {
            agent_name: "agent-a".into(),
            modified_files: vec!["src/main.rs".into()],
            modified_symbols: vec![("src/main.rs".into(), "run".into())],
        });
        detector.add_agent_changes(AgentFileChanges {
            agent_name: "agent-b".into(),
            modified_files: vec!["src/main.rs".into()],
            modified_symbols: vec![("src/main.rs".into(), "run".into())],
        });

        let report = detector.detect();
        assert!(report.has_conflicts());
        assert!(report.has_errors());
        assert_eq!(report.error_count(), 1);

        let conflict = &report.conflicts()[0];
        assert_eq!(conflict.severity, ConflictSeverity::Error);
        assert_eq!(conflict.file, "src/main.rs");
        assert_eq!(conflict.symbol, Some("run".into()));
        assert!(conflict.agents.contains(&"agent-a".to_string()));
        assert!(conflict.agents.contains(&"agent-b".to_string()));
    }

    #[test]
    fn test_three_agents_same_symbol_conflict() {
        let mut detector = ConflictDetector::new();
        for name in &["a", "b", "c"] {
            detector.add_agent_changes(AgentFileChanges {
                agent_name: name.to_string(),
                modified_files: vec!["src/shared.rs".into()],
                modified_symbols: vec![("src/shared.rs".into(), "init".into())],
            });
        }

        let report = detector.detect();
        assert!(report.has_errors());
        let conflict = &report.conflicts()[0];
        assert_eq!(conflict.agents.len(), 3);
    }

    #[test]
    fn test_mixed_conflicts_multiple_files() {
        let mut detector = ConflictDetector::new();

        // Both agents modify file_a (same symbol) and file_b (different symbols)
        detector.add_agent_changes(AgentFileChanges {
            agent_name: "agent-1".into(),
            modified_files: vec!["file_a.rs".into(), "file_b.rs".into()],
            modified_symbols: vec![
                ("file_a.rs".into(), "shared_fn".into()),
                ("file_b.rs".into(), "fn_x".into()),
            ],
        });
        detector.add_agent_changes(AgentFileChanges {
            agent_name: "agent-2".into(),
            modified_files: vec!["file_a.rs".into(), "file_b.rs".into()],
            modified_symbols: vec![
                ("file_a.rs".into(), "shared_fn".into()),
                ("file_b.rs".into(), "fn_y".into()),
            ],
        });

        let report = detector.detect();
        assert!(report.has_errors());

        // file_a: error (same symbol "shared_fn")
        // file_b: warning (different symbols)
        assert_eq!(report.error_count(), 1);
        assert_eq!(report.warning_count(), 1);
    }

    // =========================================================================
    // ConflictReport methods
    // =========================================================================

    #[test]
    fn test_report_summary_no_conflicts() {
        let report = ConflictDetector::new().detect();
        assert_eq!(report.summary(), "No conflicts detected");
    }

    #[test]
    fn test_report_summary_warnings_only() {
        let mut detector = ConflictDetector::new();
        detector.add_agent_changes(AgentFileChanges {
            agent_name: "a".into(),
            modified_files: vec!["f.rs".into()],
            modified_symbols: vec![],
        });
        detector.add_agent_changes(AgentFileChanges {
            agent_name: "b".into(),
            modified_files: vec!["f.rs".into()],
            modified_symbols: vec![],
        });
        let report = detector.detect();
        assert_eq!(report.summary(), "1 warning(s)");
    }

    #[test]
    fn test_report_summary_errors_only() {
        let mut detector = ConflictDetector::new();
        detector.add_agent_changes(AgentFileChanges {
            agent_name: "a".into(),
            modified_files: vec!["f.rs".into()],
            modified_symbols: vec![("f.rs".into(), "sym".into())],
        });
        detector.add_agent_changes(AgentFileChanges {
            agent_name: "b".into(),
            modified_files: vec!["f.rs".into()],
            modified_symbols: vec![("f.rs".into(), "sym".into())],
        });
        let report = detector.detect();
        assert_eq!(report.summary(), "1 error(s)");
    }

    #[test]
    fn test_report_summary_mixed() {
        let mut detector = ConflictDetector::new();
        detector.add_agent_changes(AgentFileChanges {
            agent_name: "a".into(),
            modified_files: vec!["err.rs".into(), "warn.rs".into()],
            modified_symbols: vec![("err.rs".into(), "dup".into())],
        });
        detector.add_agent_changes(AgentFileChanges {
            agent_name: "b".into(),
            modified_files: vec!["err.rs".into(), "warn.rs".into()],
            modified_symbols: vec![("err.rs".into(), "dup".into())],
        });
        let report = detector.detect();
        assert!(report.summary().contains("error"));
        assert!(report.summary().contains("warning"));
    }

    // =========================================================================
    // Merge preview
    // =========================================================================

    #[test]
    fn test_merge_preview_no_conflicts() {
        let report = ConflictDetector::new().detect();
        let preview = report.merge_preview();
        assert!(preview.contains("safe to merge"));
    }

    #[test]
    fn test_merge_preview_with_errors() {
        let mut detector = ConflictDetector::new();
        detector.add_agent_changes(AgentFileChanges {
            agent_name: "a".into(),
            modified_files: vec!["f.rs".into()],
            modified_symbols: vec![("f.rs".into(), "fn_x".into())],
        });
        detector.add_agent_changes(AgentFileChanges {
            agent_name: "b".into(),
            modified_files: vec!["f.rs".into()],
            modified_symbols: vec![("f.rs".into(), "fn_x".into())],
        });
        let report = detector.detect();
        let preview = report.merge_preview();
        assert!(preview.contains("manual resolution required"));
        assert!(preview.contains("fn_x"));
    }

    #[test]
    fn test_merge_preview_with_warnings() {
        let mut detector = ConflictDetector::new();
        detector.add_agent_changes(AgentFileChanges {
            agent_name: "a".into(),
            modified_files: vec!["f.rs".into()],
            modified_symbols: vec![],
        });
        detector.add_agent_changes(AgentFileChanges {
            agent_name: "b".into(),
            modified_files: vec!["f.rs".into()],
            modified_symbols: vec![],
        });
        let report = detector.detect();
        let preview = report.merge_preview();
        assert!(preview.contains("auto-mergeable"));
    }

    // =========================================================================
    // Edge cases
    // =========================================================================

    #[test]
    fn test_agent_modifying_empty_file_list() {
        let mut detector = ConflictDetector::new();
        detector.add_agent_changes(AgentFileChanges {
            agent_name: "empty".into(),
            modified_files: vec![],
            modified_symbols: vec![],
        });
        detector.add_agent_changes(AgentFileChanges {
            agent_name: "also-empty".into(),
            modified_files: vec![],
            modified_symbols: vec![],
        });
        let report = detector.detect();
        assert!(!report.has_conflicts());
    }

    #[test]
    fn test_conflicts_sorted_errors_first() {
        let mut detector = ConflictDetector::new();
        // Create a warning on aaa.rs and an error on zzz.rs
        // Despite alphabetical order, errors should come first
        detector.add_agent_changes(AgentFileChanges {
            agent_name: "a".into(),
            modified_files: vec!["aaa.rs".into(), "zzz.rs".into()],
            modified_symbols: vec![("zzz.rs".into(), "sym".into())],
        });
        detector.add_agent_changes(AgentFileChanges {
            agent_name: "b".into(),
            modified_files: vec!["aaa.rs".into(), "zzz.rs".into()],
            modified_symbols: vec![("zzz.rs".into(), "sym".into())],
        });

        let report = detector.detect();
        let conflicts = report.conflicts();
        assert_eq!(conflicts.len(), 2);
        assert_eq!(conflicts[0].severity, ConflictSeverity::Error);
        assert_eq!(conflicts[1].severity, ConflictSeverity::Warning);
    }

    #[test]
    fn test_duplicate_files_in_agent_changes_handled() {
        let mut detector = ConflictDetector::new();
        detector.add_agent_changes(AgentFileChanges {
            agent_name: "a".into(),
            modified_files: vec!["f.rs".into(), "f.rs".into()],
            modified_symbols: vec![],
        });
        detector.add_agent_changes(AgentFileChanges {
            agent_name: "b".into(),
            modified_files: vec!["f.rs".into()],
            modified_symbols: vec![],
        });

        let report = detector.detect();
        // Should still detect the conflict, but agents list should be deduped
        assert!(report.has_conflicts());
        let conflict = &report.conflicts()[0];
        // "a" appears only once even though it listed f.rs twice
        assert_eq!(
            conflict.agents.iter().filter(|a| *a == "a").count(),
            1,
            "Agent 'a' should appear only once"
        );
    }
}
