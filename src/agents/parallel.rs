//! Parallel agent orchestration for git worktree-based execution.
//!
//! This module provides infrastructure for running multiple agents in parallel,
//! each working in its own git worktree with isolated file changes.
//!
//! # Division Strategies
//!
//! - `ByModule` - Divide work by code module/directory
//! - `AlternativeApproaches` - Explore different solutions in parallel
//! - `ByTestCase` - Parallelize by test suite or test case
//!
//! # Parallel Orchestration
//!
//! The `ParallelAgentOrchestrator` spawns subagents in separate worktrees,
//! allowing them to make independent changes that are later merged.

use std::fmt;
use std::path::PathBuf;

// NOTE: Task 2.5.1 (RED) - Tests below document expected behavior
// Task 2.5.2 (GREEN) will complete the implementation

/// Strategy for dividing work among parallel agents.
///
/// Each strategy determines how tasks are partitioned and assigned
/// to individual agents working in separate worktrees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DivisionStrategy {
    /// Divide work by code module or directory.
    ///
    /// Each agent works on a specific part of the codebase,
    /// minimizing merge conflicts.
    ByModule {
        /// List of module paths to divide work by.
        modules: Vec<PathBuf>,
    },

    /// Explore alternative approaches in parallel.
    ///
    /// Multiple agents attempt different solutions to the same problem,
    /// with the best approach selected at the end.
    AlternativeApproaches {
        /// Number of alternative approaches to try.
        count: usize,
        /// Description of the problem to solve.
        problem: String,
    },

    /// Divide work by test case or test suite.
    ///
    /// Each agent focuses on implementing or fixing specific tests.
    ByTestCase {
        /// List of test patterns to divide.
        test_patterns: Vec<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    // =============================================================================
    // DivisionStrategy enum tests (Task 2.5.1)
    // =============================================================================

    #[test]
    fn test_division_strategy_variants_by_module() {
        let strategy = DivisionStrategy::ByModule {
            modules: vec![PathBuf::from("src/api"), PathBuf::from("src/tui")],
        };
        assert_eq!(strategy.name(), "by_module");
        assert_eq!(strategy.partition_count(), 2);
    }

    #[test]
    fn test_division_strategy_variants_alternative_approaches() {
        let strategy = DivisionStrategy::AlternativeApproaches {
            count: 3,
            problem: "Optimize database queries".to_string(),
        };
        assert_eq!(strategy.name(), "alternative_approaches");
        assert_eq!(strategy.partition_count(), 3);
    }

    #[test]
    fn test_division_strategy_variants_by_test_case() {
        let strategy = DivisionStrategy::ByTestCase {
            test_patterns: vec![
                "test_api_*".to_string(),
                "test_tui_*".to_string(),
                "test_tools_*".to_string(),
            ],
        };
        assert_eq!(strategy.name(), "by_test_case");
        assert_eq!(strategy.partition_count(), 3);
    }

    #[test]
    fn test_division_strategy_display() {
        let strategy = DivisionStrategy::ByModule {
            modules: vec![PathBuf::from("src/api")],
        };
        assert!(format!("{}", strategy).contains("By Module"));

        let strategy = DivisionStrategy::AlternativeApproaches {
            count: 2,
            problem: "test".to_string(),
        };
        assert!(format!("{}", strategy).contains("Alternative Approaches"));
    }

    #[test]
    fn test_division_strategy_equality() {
        let s1 = DivisionStrategy::ByModule {
            modules: vec![PathBuf::from("src")],
        };
        let s2 = DivisionStrategy::ByModule {
            modules: vec![PathBuf::from("src")],
        };
        let s3 = DivisionStrategy::ByModule {
            modules: vec![PathBuf::from("tests")],
        };

        assert_eq!(s1, s2);
        assert_ne!(s1, s3);
    }

    #[test]
    fn test_division_strategy_clone() {
        let strategy = DivisionStrategy::AlternativeApproaches {
            count: 5,
            problem: "complex problem".to_string(),
        };
        let cloned = strategy.clone();
        assert_eq!(strategy, cloned);
    }

    #[test]
    fn test_division_strategy_debug() {
        let strategy = DivisionStrategy::ByTestCase {
            test_patterns: vec!["test_*".to_string()],
        };
        let debug = format!("{:?}", strategy);
        assert!(debug.contains("ByTestCase"));
        assert!(debug.contains("test_*"));
    }

    #[test]
    fn test_division_strategy_describe() {
        let strategy = DivisionStrategy::ByModule {
            modules: vec![PathBuf::from("src/api"), PathBuf::from("src/tui")],
        };
        let desc = strategy.describe();
        assert!(desc.contains("2 modules"));

        let strategy = DivisionStrategy::AlternativeApproaches {
            count: 3,
            problem: "Optimize queries".to_string(),
        };
        let desc = strategy.describe();
        assert!(desc.contains("3 approaches"));
        assert!(desc.contains("Optimize queries"));
    }

    #[test]
    fn test_division_strategy_empty_partitions() {
        let strategy = DivisionStrategy::ByModule { modules: vec![] };
        assert_eq!(strategy.partition_count(), 0);
    }
}
