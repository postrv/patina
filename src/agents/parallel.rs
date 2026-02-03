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
use uuid::Uuid;

/// Strategy for dividing work among parallel agents.
///
/// Each strategy determines how tasks are partitioned and assigned
/// to individual agents working in separate worktrees.
///
/// # Example
///
/// ```
/// use patina::agents::parallel::DivisionStrategy;
/// use std::path::PathBuf;
///
/// let strategy = DivisionStrategy::ByModule {
///     modules: vec![PathBuf::from("src/api"), PathBuf::from("src/tui")],
/// };
/// assert_eq!(strategy.name(), "by_module");
/// assert_eq!(strategy.partition_count(), 2);
/// ```
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

impl DivisionStrategy {
    /// Returns the strategy name as a snake_case identifier.
    ///
    /// # Example
    ///
    /// ```
    /// use patina::agents::parallel::DivisionStrategy;
    /// use std::path::PathBuf;
    ///
    /// let strategy = DivisionStrategy::ByModule { modules: vec![] };
    /// assert_eq!(strategy.name(), "by_module");
    /// ```
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::ByModule { .. } => "by_module",
            Self::AlternativeApproaches { .. } => "alternative_approaches",
            Self::ByTestCase { .. } => "by_test_case",
        }
    }

    /// Returns the number of partitions (parallel tasks) this strategy creates.
    ///
    /// # Example
    ///
    /// ```
    /// use patina::agents::parallel::DivisionStrategy;
    ///
    /// let strategy = DivisionStrategy::AlternativeApproaches {
    ///     count: 3,
    ///     problem: "optimize".to_string(),
    /// };
    /// assert_eq!(strategy.partition_count(), 3);
    /// ```
    #[must_use]
    pub fn partition_count(&self) -> usize {
        match self {
            Self::ByModule { modules } => modules.len(),
            Self::AlternativeApproaches { count, .. } => *count,
            Self::ByTestCase { test_patterns } => test_patterns.len(),
        }
    }

    /// Returns a human-readable description of the strategy.
    ///
    /// # Example
    ///
    /// ```
    /// use patina::agents::parallel::DivisionStrategy;
    /// use std::path::PathBuf;
    ///
    /// let strategy = DivisionStrategy::ByModule {
    ///     modules: vec![PathBuf::from("src"), PathBuf::from("tests")],
    /// };
    /// assert!(strategy.describe().contains("2 modules"));
    /// ```
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::ByModule { modules } => {
                format!("Divide work across {} modules", modules.len())
            }
            Self::AlternativeApproaches { count, problem } => {
                format!("Explore {} approaches for: {}", count, problem)
            }
            Self::ByTestCase { test_patterns } => {
                format!("Parallelize {} test patterns", test_patterns.len())
            }
        }
    }
}

impl fmt::Display for DivisionStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ByModule { modules } => {
                write!(f, "By Module ({} modules)", modules.len())
            }
            Self::AlternativeApproaches { count, .. } => {
                write!(f, "Alternative Approaches ({} variants)", count)
            }
            Self::ByTestCase { test_patterns } => {
                write!(f, "By Test Case ({} patterns)", test_patterns.len())
            }
        }
    }
}

// NOTE: Task 2.5.3 (RED) - Stub types for tests
// Task 2.5.4 (GREEN) will complete the implementation

/// Strategy for merging results from parallel agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeStrategy {
    /// Use the first agent that completes successfully.
    FirstSuccess,
    /// Compare results and choose the best one.
    BestResult,
    /// Combine all results (for non-conflicting changes).
    CombineAll,
    /// Require manual review and selection.
    Manual,
}

/// Status of the parallel orchestrator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrchestratorStatus {
    /// No tasks are running.
    Idle,
    /// Tasks are being executed in parallel.
    Running,
    /// All tasks completed, results available.
    Completed,
    /// One or more tasks failed.
    Failed,
}

/// Result from a parallel agent execution.
#[derive(Debug, Clone)]
pub struct ParallelTaskResult {
    /// Unique task ID.
    pub task_id: Uuid,
    /// The worktree path where changes were made.
    pub worktree_path: PathBuf,
    /// Whether the task succeeded.
    pub success: bool,
    /// Output from the task.
    pub output: String,
}

/// Orchestrator for managing parallel agent execution in git worktrees.
///
/// Spawns multiple agents working on different aspects of a task,
/// each in its own isolated worktree.
pub struct ParallelAgentOrchestrator {
    strategy: Option<DivisionStrategy>,
    merge_strategy: MergeStrategy,
    tasks: Vec<Uuid>,
    results: Vec<ParallelTaskResult>,
    status: OrchestratorStatus,
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

    // =============================================================================
    // MergeStrategy enum tests (Task 2.5.3)
    // =============================================================================

    #[test]
    fn test_merge_strategy_variants() {
        let strategies = [
            MergeStrategy::FirstSuccess,
            MergeStrategy::BestResult,
            MergeStrategy::CombineAll,
            MergeStrategy::Manual,
        ];

        for strategy in strategies {
            match strategy {
                MergeStrategy::FirstSuccess => assert_eq!(strategy.name(), "first_success"),
                MergeStrategy::BestResult => assert_eq!(strategy.name(), "best_result"),
                MergeStrategy::CombineAll => assert_eq!(strategy.name(), "combine_all"),
                MergeStrategy::Manual => assert_eq!(strategy.name(), "manual"),
            }
        }
    }

    #[test]
    fn test_merge_strategy_display() {
        assert!(format!("{}", MergeStrategy::FirstSuccess).contains("First Success"));
        assert!(format!("{}", MergeStrategy::BestResult).contains("Best Result"));
    }

    // =============================================================================
    // ParallelAgentOrchestrator tests (Task 2.5.3)
    // =============================================================================

    #[test]
    fn test_orchestrator_new() {
        let orchestrator = ParallelAgentOrchestrator::new();
        assert_eq!(orchestrator.active_count(), 0);
        assert!(orchestrator.results().is_empty());
    }

    #[test]
    fn test_orchestrator_with_strategy() {
        let strategy = DivisionStrategy::ByModule {
            modules: vec![PathBuf::from("src/api")],
        };
        let orchestrator = ParallelAgentOrchestrator::new().with_strategy(strategy.clone());
        assert_eq!(orchestrator.strategy(), Some(&strategy));
    }

    #[test]
    fn test_orchestrator_with_merge_strategy() {
        let orchestrator =
            ParallelAgentOrchestrator::new().with_merge_strategy(MergeStrategy::BestResult);
        assert_eq!(orchestrator.merge_strategy(), MergeStrategy::BestResult);
    }

    #[test]
    fn test_orchestrator_spawn_parallel() {
        let strategy = DivisionStrategy::AlternativeApproaches {
            count: 3,
            problem: "test".to_string(),
        };
        let mut orchestrator = ParallelAgentOrchestrator::new().with_strategy(strategy);

        // spawn_parallel should create worktree-based tasks
        let task_ids = orchestrator.spawn_parallel();
        assert_eq!(task_ids.len(), 3);
        assert_eq!(orchestrator.active_count(), 3);
    }

    #[test]
    fn test_orchestrator_collect_results() {
        let strategy = DivisionStrategy::ByModule {
            modules: vec![PathBuf::from("src")],
        };
        let mut orchestrator = ParallelAgentOrchestrator::new()
            .with_strategy(strategy)
            .with_merge_strategy(MergeStrategy::FirstSuccess);

        // After spawning and completing, results can be collected
        let _task_ids = orchestrator.spawn_parallel();

        // Simulate completion (in real use, this would be async)
        let results = orchestrator.collect_results();
        assert!(!results.is_empty());
    }

    #[test]
    fn test_orchestrator_status() {
        let orchestrator = ParallelAgentOrchestrator::new();
        assert_eq!(orchestrator.status(), OrchestratorStatus::Idle);
    }
}
