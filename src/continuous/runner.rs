//! ContinuousRunner state machine for orchestrating the continuous coding loop.
//!
//! The runner coordinates: iterate -> quality gates -> stagnation check -> recovery -> continue/stop.
//!
//! # Architecture
//!
//! The [`ContinuousRunner`] operates as a state machine over iterations:
//!
//! 1. Run an LLM coding iteration via [`ContinuousExecutor::run_iteration`]
//! 2. Check quality gates via [`RecoveryExecutor::check_gates`]
//! 3. If gates pass, return [`RunOutcome::AllGatesPassed`]
//! 4. Record stagnation snapshot and evaluate risk
//! 5. If stagnation is critical, return [`RunOutcome::HumanCheckpointRequired`]
//! 6. Attempt recovery via [`attempt_recovery`]
//! 7. If recovery succeeds, return [`RunOutcome::AllGatesPassed`]
//! 8. Continue to next iteration
//!
//! # Example
//!
//! ```rust,ignore
//! use patina::continuous::runner::{ContinuousRunner, RunnerConfig, RunOutcome};
//!
//! let mut runner = ContinuousRunner::new(executor, RunnerConfig::default(), |event| {
//!     println!("Event: {:?}", event);
//! });
//! let outcome = runner.run().await;
//! assert!(outcome.is_success());
//! ```

use std::fmt;
use std::future::Future;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use super::events::ContinuousEvent;
use super::recovery::{
    attempt_recovery, parse_error_locations, GateCheckResult, RecoveryConfig, RecoveryExecutor,
    RecoveryMetrics, RecoveryOutcome, RootCauseAnalysis,
};
use super::stagnation::{IterationSnapshot, RiskLevel, StagnationConfig, StagnationDetector};

/// Outcome of running the continuous coding loop.
///
/// # Example
///
/// ```
/// use patina::continuous::runner::RunOutcome;
///
/// let outcome = RunOutcome::AllGatesPassed { iterations: 3 };
/// assert!(outcome.is_success());
/// assert_eq!(outcome.iteration_count(), 3);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunOutcome {
    /// All quality gates passed successfully.
    AllGatesPassed {
        /// Number of iterations completed.
        iterations: u32,
    },

    /// Human intervention is required.
    HumanCheckpointRequired {
        /// Reason why human help is needed.
        reason: String,
        /// Number of iterations completed before stopping.
        iterations: u32,
    },

    /// Maximum iteration limit reached without all gates passing.
    MaxIterationsReached {
        /// The configured maximum iterations.
        iterations: u32,
    },
}

impl RunOutcome {
    /// Returns `true` if the loop completed successfully (all gates passed).
    ///
    /// # Example
    ///
    /// ```
    /// use patina::continuous::runner::RunOutcome;
    ///
    /// assert!(RunOutcome::AllGatesPassed { iterations: 1 }.is_success());
    /// assert!(!RunOutcome::MaxIterationsReached { iterations: 5 }.is_success());
    /// ```
    #[must_use]
    pub fn is_success(&self) -> bool {
        matches!(self, Self::AllGatesPassed { .. })
    }

    /// Returns the number of iterations completed.
    ///
    /// # Example
    ///
    /// ```
    /// use patina::continuous::runner::RunOutcome;
    ///
    /// let outcome = RunOutcome::AllGatesPassed { iterations: 3 };
    /// assert_eq!(outcome.iteration_count(), 3);
    /// ```
    #[must_use]
    pub fn iteration_count(&self) -> u32 {
        match self {
            Self::AllGatesPassed { iterations }
            | Self::HumanCheckpointRequired { iterations, .. }
            | Self::MaxIterationsReached { iterations } => *iterations,
        }
    }
}

impl fmt::Display for RunOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AllGatesPassed { iterations } => {
                write!(f, "All gates passed after {iterations} iteration(s)")
            }
            Self::HumanCheckpointRequired {
                reason, iterations, ..
            } => {
                write!(
                    f,
                    "Human checkpoint required after {iterations} iteration(s): {reason}"
                )
            }
            Self::MaxIterationsReached { iterations } => {
                write!(f, "Maximum iterations ({iterations}) reached")
            }
        }
    }
}

/// Configuration for the continuous runner.
///
/// # Example
///
/// ```
/// use patina::continuous::runner::RunnerConfig;
///
/// let config = RunnerConfig::default();
/// assert_eq!(config.max_iterations, 50);
/// ```
#[derive(Debug, Clone)]
pub struct RunnerConfig {
    /// Maximum number of iterations before stopping.
    pub max_iterations: u32,

    /// Recovery loop configuration.
    pub recovery: RecoveryConfig,

    /// Stagnation detection configuration.
    pub stagnation: StagnationConfig,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            max_iterations: 50,
            recovery: RecoveryConfig::default(),
            stagnation: StagnationConfig::default(),
        }
    }
}

/// Trait for executing a single coding iteration in the continuous loop.
///
/// Extends [`RecoveryExecutor`] (which provides `execute_fix` and `check_gates`)
/// with the ability to run a full LLM coding iteration.
///
/// # Implementors
///
/// Production implementations connect to the LLM provider and tool execution
/// system. Test implementations use mock data for deterministic testing.
pub trait ContinuousExecutor: RecoveryExecutor {
    /// Run a single LLM coding iteration.
    ///
    /// Returns an [`IterationSnapshot`] with information about what happened
    /// during the iteration (files edited, commit hash, etc.).
    ///
    /// # Errors
    ///
    /// Returns an error for fatal failures (e.g., LLM unavailable, disk full).
    fn run_iteration(&mut self) -> impl Future<Output = anyhow::Result<IterationSnapshot>> + Send;
}

/// Orchestrates the continuous coding loop with quality gates, stagnation
/// detection, and recovery.
///
/// The runner executes iterations, checks quality gates, detects stagnation,
/// and attempts automatic recovery on failures. Events are emitted via the
/// callback for TUI progress display.
///
/// # Type Parameters
///
/// - `E`: The executor implementing [`ContinuousExecutor`]
/// - `F`: Event callback `FnMut(ContinuousEvent)`
pub struct ContinuousRunner<E, F> {
    executor: E,
    config: RunnerConfig,
    stagnation_detector: StagnationDetector,
    recovery_metrics: RecoveryMetrics,
    event_callback: F,
}

impl<E, F> ContinuousRunner<E, F>
where
    E: ContinuousExecutor,
    F: FnMut(ContinuousEvent),
{
    /// Creates a new runner with the given executor, configuration, and event callback.
    ///
    /// # Arguments
    ///
    /// * `executor` - Implements iteration execution, gate checking, and fix application
    /// * `config` - Runner configuration (max iterations, recovery, stagnation)
    /// * `event_callback` - Called for each [`ContinuousEvent`] emitted during the loop
    pub fn new(executor: E, config: RunnerConfig, event_callback: F) -> Self {
        let stagnation_detector = StagnationDetector::new(config.stagnation.clone());
        Self {
            executor,
            config,
            stagnation_detector,
            recovery_metrics: RecoveryMetrics::default(),
            event_callback,
        }
    }

    /// Returns a reference to the accumulated recovery metrics.
    #[must_use]
    pub fn recovery_metrics(&self) -> &RecoveryMetrics {
        &self.recovery_metrics
    }

    /// Runs the continuous coding loop until completion.
    ///
    /// The loop iterates up to `config.max_iterations` times. Each iteration:
    /// 1. Runs an LLM coding turn
    /// 2. Checks quality gates
    /// 3. On gate failure: checks stagnation, attempts recovery
    /// 4. Continues or stops based on outcomes
    ///
    /// # Returns
    ///
    /// - [`RunOutcome::AllGatesPassed`] if all quality gates pass
    /// - [`RunOutcome::HumanCheckpointRequired`] on fatal error or critical stagnation
    /// - [`RunOutcome::MaxIterationsReached`] if the iteration limit is hit
    pub async fn run(&mut self) -> RunOutcome {
        for iteration in 1..=self.config.max_iterations {
            (self.event_callback)(ContinuousEvent::IterationStart { iteration });
            let start = Instant::now();

            // 1. Run LLM coding iteration
            let snapshot = match self.executor.run_iteration().await {
                Ok(s) => s,
                Err(e) => {
                    warn!(iteration, error = %e, "Fatal error during iteration");
                    let reason = format!("Fatal error during iteration: {e}");
                    (self.event_callback)(ContinuousEvent::HumanCheckpointRequired {
                        reason: reason.clone(),
                    });
                    return RunOutcome::HumanCheckpointRequired {
                        reason,
                        iterations: iteration,
                    };
                }
            };

            // 2. Check quality gates
            let gate_result = self.executor.check_gates().await;

            (self.event_callback)(ContinuousEvent::QualityGateResult {
                gate: "all".to_string(),
                passed: gate_result.passed,
                message: if gate_result.passed {
                    None
                } else {
                    Some(gate_result.error_output.chars().take(200).collect())
                },
            });

            if gate_result.passed {
                info!(iteration, "All quality gates passed");
                let duration_ms = start.elapsed().as_millis() as u64;
                (self.event_callback)(ContinuousEvent::IterationComplete {
                    iteration,
                    duration_ms,
                });
                return RunOutcome::AllGatesPassed {
                    iterations: iteration,
                };
            }

            // 3. Record stagnation snapshot and evaluate risk
            let mut snapshot = snapshot;
            if snapshot.errors.is_empty() {
                snapshot.errors = vec![gate_result.error_output.clone()];
            }
            self.stagnation_detector.record(snapshot);

            let score = self.stagnation_detector.evaluate();
            debug!(
                iteration,
                total = score.total,
                risk = %score.risk_level,
                "Stagnation score"
            );

            if score.risk_level >= RiskLevel::Critical {
                warn!(
                    iteration,
                    score = score.total,
                    "Critical stagnation detected"
                );
                (self.event_callback)(ContinuousEvent::StagnationDetected {
                    iterations_without_progress: iteration,
                    threshold: self.config.stagnation.history_window as u32,
                });
                let reason = format!(
                    "Critical stagnation detected (score: {:.2}, risk: {})",
                    score.total, score.risk_level
                );
                (self.event_callback)(ContinuousEvent::HumanCheckpointRequired {
                    reason: reason.clone(),
                });
                return RunOutcome::HumanCheckpointRequired {
                    reason,
                    iterations: iteration,
                };
            }

            // 4. Attempt recovery
            let analysis = build_analysis_from_gate_error(&gate_result.error_output);
            let recovery_outcome = attempt_recovery(
                &analysis,
                &self.config.recovery,
                &mut self.executor,
                &mut self.recovery_metrics,
            )
            .await;

            if recovery_outcome.is_fixed() {
                info!(iteration, "Recovery succeeded");
                let duration_ms = start.elapsed().as_millis() as u64;
                (self.event_callback)(ContinuousEvent::IterationComplete {
                    iteration,
                    duration_ms,
                });
                return RunOutcome::AllGatesPassed {
                    iterations: iteration,
                };
            }

            debug!(
                iteration,
                attempts = recovery_outcome.attempt_count(),
                "Recovery failed, continuing to next iteration"
            );

            let duration_ms = start.elapsed().as_millis() as u64;
            (self.event_callback)(ContinuousEvent::IterationComplete {
                iteration,
                duration_ms,
            });
        }

        info!(
            max = self.config.max_iterations,
            "Maximum iterations reached"
        );
        RunOutcome::MaxIterationsReached {
            iterations: self.config.max_iterations,
        }
    }
}

/// Builds a [`RootCauseAnalysis`] from gate error output using parse-only analysis.
///
/// This is a fallback that works without narsil MCP. It extracts error locations
/// from the raw output and uses them as the basis for the analysis.
#[must_use]
fn build_analysis_from_gate_error(error_output: &str) -> RootCauseAnalysis {
    let locations = parse_error_locations(error_output);
    let mut relevant_files: Vec<String> = locations.iter().map(|l| l.file.clone()).collect();
    relevant_files.sort();
    relevant_files.dedup();

    let error_summary = if error_output.len() > 200 {
        format!("{}...", &error_output[..200])
    } else {
        error_output.to_string()
    };

    RootCauseAnalysis {
        error_summary,
        error_locations: locations,
        callers: Vec::new(),
        recent_changes: Vec::new(),
        relevant_files,
        narsil_available: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    // =========================================================================
    // Mock executor
    // =========================================================================

    struct MockContinuousExecutor {
        iteration_results: Vec<Result<IterationSnapshot, String>>,
        iteration_idx: usize,
        gate_results: Vec<GateCheckResult>,
        gate_idx: usize,
        fix_results: Vec<Result<(), String>>,
        fix_idx: usize,
    }

    impl MockContinuousExecutor {
        fn new() -> Self {
            Self {
                iteration_results: Vec::new(),
                iteration_idx: 0,
                gate_results: Vec::new(),
                gate_idx: 0,
                fix_results: Vec::new(),
                fix_idx: 0,
            }
        }

        fn with_iterations(mut self, results: Vec<Result<IterationSnapshot, String>>) -> Self {
            self.iteration_results = results;
            self
        }

        fn with_gates(mut self, results: Vec<GateCheckResult>) -> Self {
            self.gate_results = results;
            self
        }

        fn with_fixes(mut self, results: Vec<Result<(), String>>) -> Self {
            self.fix_results = results;
            self
        }
    }

    impl RecoveryExecutor for MockContinuousExecutor {
        fn execute_fix(&mut self, _prompt: &str) -> impl Future<Output = anyhow::Result<()>> + Send {
            let idx = self
                .fix_idx
                .min(self.fix_results.len().saturating_sub(1));
            self.fix_idx += 1;
            let result = self.fix_results.get(idx).cloned().unwrap_or(Ok(()));
            async move {
                match result {
                    Ok(()) => Ok(()),
                    Err(e) => Err(anyhow::anyhow!("{e}")),
                }
            }
        }

        fn check_gates(&mut self) -> impl Future<Output = GateCheckResult> + Send {
            let idx = self
                .gate_idx
                .min(self.gate_results.len().saturating_sub(1));
            self.gate_idx += 1;
            let result = self
                .gate_results
                .get(idx)
                .cloned()
                .unwrap_or_else(passing_gates);
            async move { result }
        }
    }

    impl ContinuousExecutor for MockContinuousExecutor {
        fn run_iteration(
            &mut self,
        ) -> impl Future<Output = anyhow::Result<IterationSnapshot>> + Send {
            let idx = self
                .iteration_idx
                .min(self.iteration_results.len().saturating_sub(1));
            self.iteration_idx += 1;
            let result = self
                .iteration_results
                .get(idx)
                .cloned()
                .unwrap_or_else(|| Ok(ok_snapshot("default")));
            async move {
                match result {
                    Ok(s) => Ok(s),
                    Err(e) => Err(anyhow::anyhow!("{e}")),
                }
            }
        }
    }

    // =========================================================================
    // Test helpers
    // =========================================================================

    fn ok_snapshot(commit: &str) -> IterationSnapshot {
        IterationSnapshot {
            commit_hash: Some(commit.to_string()),
            files_edited: vec!["src/main.rs".to_string()],
            errors: vec![],
            test_count: 100,
        }
    }

    fn stagnated_snapshot() -> IterationSnapshot {
        IterationSnapshot {
            commit_hash: Some("stuck".to_string()),
            files_edited: vec!["src/broken.rs".to_string()],
            errors: vec!["error: same error".to_string()],
            test_count: 100,
        }
    }

    fn passing_gates() -> GateCheckResult {
        GateCheckResult {
            passed: true,
            error_output: String::new(),
        }
    }

    fn failing_gates(error: &str) -> GateCheckResult {
        GateCheckResult {
            passed: false,
            error_output: error.to_string(),
        }
    }

    fn collect_events() -> (Arc<Mutex<Vec<ContinuousEvent>>>, impl FnMut(ContinuousEvent)) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = events.clone();
        let callback = move |e: ContinuousEvent| {
            events_clone.lock().expect("mutex poisoned").push(e);
        };
        (events, callback)
    }

    // =========================================================================
    // RunOutcome tests
    // =========================================================================

    #[test]
    fn run_outcome_is_success() {
        assert!(RunOutcome::AllGatesPassed { iterations: 1 }.is_success());
        assert!(!RunOutcome::HumanCheckpointRequired {
            reason: "stuck".to_string(),
            iterations: 5,
        }
        .is_success());
        assert!(!RunOutcome::MaxIterationsReached { iterations: 50 }.is_success());
    }

    #[test]
    fn run_outcome_iteration_count() {
        assert_eq!(
            RunOutcome::AllGatesPassed { iterations: 3 }.iteration_count(),
            3
        );
        assert_eq!(
            RunOutcome::HumanCheckpointRequired {
                reason: "err".to_string(),
                iterations: 7,
            }
            .iteration_count(),
            7
        );
        assert_eq!(
            RunOutcome::MaxIterationsReached { iterations: 50 }.iteration_count(),
            50
        );
    }

    #[test]
    fn run_outcome_display() {
        let outcome = RunOutcome::AllGatesPassed { iterations: 3 };
        let display = format!("{outcome}");
        assert!(display.contains("3"));
        assert!(display.contains("passed"));

        let outcome = RunOutcome::HumanCheckpointRequired {
            reason: "stagnation".to_string(),
            iterations: 5,
        };
        let display = format!("{outcome}");
        assert!(display.contains("5"));
        assert!(display.contains("stagnation"));

        let outcome = RunOutcome::MaxIterationsReached { iterations: 50 };
        let display = format!("{outcome}");
        assert!(display.contains("50"));
    }

    #[test]
    fn run_outcome_serde_roundtrip() {
        let outcomes = vec![
            RunOutcome::AllGatesPassed { iterations: 3 },
            RunOutcome::HumanCheckpointRequired {
                reason: "stuck".to_string(),
                iterations: 5,
            },
            RunOutcome::MaxIterationsReached { iterations: 50 },
        ];
        for outcome in outcomes {
            let json = serde_json::to_string(&outcome).expect("serialize");
            let restored: RunOutcome = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(outcome, restored);
        }
    }

    // =========================================================================
    // RunnerConfig tests
    // =========================================================================

    #[test]
    fn runner_config_defaults() {
        let config = RunnerConfig::default();
        assert_eq!(config.max_iterations, 50);
        assert_eq!(config.recovery.max_attempts, 3);
        assert_eq!(config.stagnation.history_window, 10);
    }

    // =========================================================================
    // build_analysis_from_gate_error tests
    // =========================================================================

    #[test]
    fn build_analysis_extracts_locations() {
        let error = "error[E0308]: mismatched types\n  --> src/main.rs:42:5";
        let analysis = build_analysis_from_gate_error(error);

        assert_eq!(analysis.error_locations.len(), 1);
        assert_eq!(analysis.error_locations[0].file, "src/main.rs");
        assert_eq!(analysis.error_locations[0].line, Some(42));
        assert_eq!(analysis.relevant_files, vec!["src/main.rs"]);
        assert!(!analysis.narsil_available);
    }

    #[test]
    fn build_analysis_truncates_long_error() {
        let error = "x".repeat(500);
        let analysis = build_analysis_from_gate_error(&error);

        assert!(analysis.error_summary.len() <= 204);
        assert!(analysis.error_summary.ends_with("..."));
    }

    #[test]
    fn build_analysis_empty_error() {
        let analysis = build_analysis_from_gate_error("");

        assert!(analysis.error_locations.is_empty());
        assert!(analysis.relevant_files.is_empty());
        assert_eq!(analysis.error_summary, "");
    }

    #[test]
    fn build_analysis_deduplicates_files() {
        let error = "\
error: first
  --> src/main.rs:10:1
error: second
  --> src/main.rs:20:1";
        let analysis = build_analysis_from_gate_error(error);

        assert_eq!(analysis.error_locations.len(), 2);
        assert_eq!(
            analysis.relevant_files,
            vec!["src/main.rs"],
            "duplicate files should be deduplicated"
        );
    }

    // =========================================================================
    // ContinuousRunner — all gates pass on first try
    // =========================================================================

    #[tokio::test]
    async fn all_gates_pass_first_try() {
        let executor = MockContinuousExecutor::new()
            .with_iterations(vec![Ok(ok_snapshot("abc"))])
            .with_gates(vec![passing_gates()]);

        let config = RunnerConfig {
            max_iterations: 10,
            ..RunnerConfig::default()
        };

        let (events, callback) = collect_events();
        let mut runner = ContinuousRunner::new(executor, config, callback);

        let outcome = runner.run().await;

        assert_eq!(outcome, RunOutcome::AllGatesPassed { iterations: 1 });

        let events = events.lock().expect("mutex");
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ContinuousEvent::IterationStart { iteration: 1 })),
            "should emit IterationStart"
        );
    }

    // =========================================================================
    // ContinuousRunner — gate failure triggers recovery, recovery succeeds
    // =========================================================================

    #[tokio::test]
    async fn gate_failure_triggers_recovery_then_succeeds() {
        // First gate check fails (runner), second passes (recovery)
        let executor = MockContinuousExecutor::new()
            .with_iterations(vec![Ok(ok_snapshot("abc"))])
            .with_gates(vec![
                failing_gates("error[E0308]: mismatched types\n  --> src/main.rs:42:5"),
                passing_gates(),
            ])
            .with_fixes(vec![Ok(())]);

        let config = RunnerConfig {
            max_iterations: 10,
            recovery: RecoveryConfig { max_attempts: 3 },
            ..RunnerConfig::default()
        };

        let mut runner = ContinuousRunner::new(executor, config, |_| {});
        let outcome = runner.run().await;

        assert_eq!(
            outcome,
            RunOutcome::AllGatesPassed { iterations: 1 },
            "recovery should fix the issue on first iteration"
        );
    }

    // =========================================================================
    // ContinuousRunner — stagnation detected
    // =========================================================================

    #[tokio::test]
    async fn stagnation_returns_human_checkpoint() {
        // All iterations fail with same error, same files, same commit.
        // Recovery always fails. Stagnation builds up to critical.
        let iterations: Vec<Result<IterationSnapshot, String>> =
            (0..20).map(|_| Ok(stagnated_snapshot())).collect();

        let executor = MockContinuousExecutor::new()
            .with_iterations(iterations)
            .with_gates(vec![failing_gates("error: same error")])
            .with_fixes(vec![Ok(())]);

        let config = RunnerConfig {
            max_iterations: 20,
            recovery: RecoveryConfig { max_attempts: 1 },
            stagnation: StagnationConfig {
                history_window: 5,
                risk_thresholds: [0.2, 0.4, 0.6],
                ..StagnationConfig::default()
            },
        };

        let (events, callback) = collect_events();
        let mut runner = ContinuousRunner::new(executor, config, callback);

        let outcome = runner.run().await;

        match &outcome {
            RunOutcome::HumanCheckpointRequired { reason, .. } => {
                assert!(
                    reason.to_lowercase().contains("stagnation"),
                    "reason should mention stagnation: {reason}"
                );
            }
            other => panic!("expected HumanCheckpointRequired, got {other:?}"),
        }

        // Should have emitted StagnationDetected
        let events = events.lock().expect("mutex");
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ContinuousEvent::StagnationDetected { .. })),
            "should emit StagnationDetected event"
        );
    }

    // =========================================================================
    // ContinuousRunner — max iterations reached
    // =========================================================================

    #[tokio::test]
    async fn max_iterations_reached() {
        // Gates always fail, recovery always fails, but stagnation stays low
        // because each iteration has a unique commit and file.
        let iterations: Vec<Result<IterationSnapshot, String>> = (0..5)
            .map(|i| {
                Ok(IterationSnapshot {
                    commit_hash: Some(format!("hash{i}")),
                    files_edited: vec![format!("src/file{i}.rs")],
                    errors: vec![],
                    test_count: 100 + i as u32,
                })
            })
            .collect();

        let executor = MockContinuousExecutor::new()
            .with_iterations(iterations)
            .with_gates(vec![failing_gates("error: persistent")])
            .with_fixes(vec![Ok(())]);

        let config = RunnerConfig {
            max_iterations: 5,
            recovery: RecoveryConfig { max_attempts: 1 },
            stagnation: StagnationConfig {
                risk_thresholds: [0.9, 0.95, 0.99],
                ..StagnationConfig::default()
            },
        };

        let mut runner = ContinuousRunner::new(executor, config, |_| {});
        let outcome = runner.run().await;

        assert_eq!(
            outcome,
            RunOutcome::MaxIterationsReached { iterations: 5 }
        );
    }

    // =========================================================================
    // ContinuousRunner — fatal error propagated
    // =========================================================================

    #[tokio::test]
    async fn fatal_error_returns_human_checkpoint() {
        let executor = MockContinuousExecutor::new()
            .with_iterations(vec![Err("fatal: disk full".to_string())])
            .with_gates(vec![passing_gates()]);

        let config = RunnerConfig {
            max_iterations: 10,
            ..RunnerConfig::default()
        };

        let (events, callback) = collect_events();
        let mut runner = ContinuousRunner::new(executor, config, callback);

        let outcome = runner.run().await;

        match &outcome {
            RunOutcome::HumanCheckpointRequired { reason, iterations } => {
                assert!(
                    reason.contains("fatal") || reason.contains("Fatal"),
                    "reason should mention the error: {reason}"
                );
                assert_eq!(*iterations, 1);
            }
            other => panic!("expected HumanCheckpointRequired, got {other:?}"),
        }

        // Should have emitted HumanCheckpointRequired event
        let events = events.lock().expect("mutex");
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ContinuousEvent::HumanCheckpointRequired { .. })),
            "should emit HumanCheckpointRequired event"
        );
    }

    // =========================================================================
    // ContinuousRunner — events emitted correctly for happy path
    // =========================================================================

    #[tokio::test]
    async fn events_emitted_for_happy_path() {
        let executor = MockContinuousExecutor::new()
            .with_iterations(vec![Ok(ok_snapshot("abc"))])
            .with_gates(vec![passing_gates()]);

        let config = RunnerConfig {
            max_iterations: 10,
            ..RunnerConfig::default()
        };

        let (events, callback) = collect_events();
        let mut runner = ContinuousRunner::new(executor, config, callback);
        let _ = runner.run().await;

        let events = events.lock().expect("mutex");
        assert!(!events.is_empty(), "should emit events");

        // IterationStart should be first
        assert!(
            matches!(events[0], ContinuousEvent::IterationStart { iteration: 1 }),
            "first event should be IterationStart, got {:?}",
            events[0]
        );

        // Should have QualityGateResult
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ContinuousEvent::QualityGateResult { passed: true, .. })),
            "should emit passing QualityGateResult"
        );

        // Should have IterationComplete
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ContinuousEvent::IterationComplete { iteration: 1, .. })),
            "should emit IterationComplete"
        );
    }

    // =========================================================================
    // ContinuousRunner — events emitted for failed path
    // =========================================================================

    #[tokio::test]
    async fn events_emitted_for_failure_path() {
        let executor = MockContinuousExecutor::new()
            .with_iterations(vec![Ok(ok_snapshot("abc"))])
            .with_gates(vec![failing_gates("error")])
            .with_fixes(vec![Ok(())]);

        let config = RunnerConfig {
            max_iterations: 1,
            recovery: RecoveryConfig { max_attempts: 1 },
            stagnation: StagnationConfig {
                risk_thresholds: [0.9, 0.95, 0.99],
                ..StagnationConfig::default()
            },
        };

        let (events, callback) = collect_events();
        let mut runner = ContinuousRunner::new(executor, config, callback);
        let _ = runner.run().await;

        let events = events.lock().expect("mutex");

        // Should have QualityGateResult with passed=false
        assert!(
            events.iter().any(
                |e| matches!(e, ContinuousEvent::QualityGateResult { passed: false, .. })
            ),
            "should emit failing QualityGateResult"
        );
    }

    // =========================================================================
    // ContinuousRunner — recovery metrics updated
    // =========================================================================

    #[tokio::test]
    async fn recovery_metrics_tracked() {
        // Gate fails, recovery runs and fails
        let executor = MockContinuousExecutor::new()
            .with_iterations(vec![Ok(ok_snapshot("abc"))])
            .with_gates(vec![failing_gates("error")])
            .with_fixes(vec![Ok(())]);

        let config = RunnerConfig {
            max_iterations: 1,
            recovery: RecoveryConfig { max_attempts: 2 },
            stagnation: StagnationConfig {
                risk_thresholds: [0.9, 0.95, 0.99],
                ..StagnationConfig::default()
            },
        };

        let mut runner = ContinuousRunner::new(executor, config, |_| {});
        let _ = runner.run().await;

        let metrics = runner.recovery_metrics();
        assert!(
            metrics.total_sessions > 0,
            "should have recorded recovery sessions"
        );
    }

    // =========================================================================
    // ContinuousRunner — recovery succeeds after multiple gate failures
    // =========================================================================

    #[tokio::test]
    async fn recovery_succeeds_on_second_attempt() {
        // Gate check 0: fail (runner), check 1: fail (recovery attempt 1),
        // check 2: pass (recovery attempt 2) -> Fixed
        let executor = MockContinuousExecutor::new()
            .with_iterations(vec![Ok(ok_snapshot("abc"))])
            .with_gates(vec![
                failing_gates("error: test failed"),
                failing_gates("error: still failing"),
                passing_gates(),
            ])
            .with_fixes(vec![Ok(()), Ok(())]);

        let config = RunnerConfig {
            max_iterations: 10,
            recovery: RecoveryConfig { max_attempts: 3 },
            ..RunnerConfig::default()
        };

        let mut runner = ContinuousRunner::new(executor, config, |_| {});
        let outcome = runner.run().await;

        assert_eq!(
            outcome,
            RunOutcome::AllGatesPassed { iterations: 1 },
            "recovery should succeed on second attempt"
        );
    }

    // =========================================================================
    // ContinuousRunner — multiple iterations before success
    // =========================================================================

    #[tokio::test]
    async fn succeeds_on_second_iteration() {
        // First iteration: gates fail, recovery fails.
        // Second iteration: gates pass.
        let executor = MockContinuousExecutor::new()
            .with_iterations(vec![
                Ok(ok_snapshot("hash1")),
                Ok(ok_snapshot("hash2")),
            ])
            .with_gates(vec![
                failing_gates("error"),  // iter 1: runner check
                failing_gates("error"),  // iter 1: recovery check
                passing_gates(),         // iter 2: runner check
            ])
            .with_fixes(vec![Ok(())]);

        let config = RunnerConfig {
            max_iterations: 10,
            recovery: RecoveryConfig { max_attempts: 1 },
            stagnation: StagnationConfig {
                risk_thresholds: [0.9, 0.95, 0.99],
                ..StagnationConfig::default()
            },
        };

        let mut runner = ContinuousRunner::new(executor, config, |_| {});
        let outcome = runner.run().await;

        assert_eq!(
            outcome,
            RunOutcome::AllGatesPassed { iterations: 2 },
            "should succeed on second iteration"
        );
    }

    // =========================================================================
    // ContinuousRunner — zero max iterations
    // =========================================================================

    #[tokio::test]
    async fn zero_max_iterations_returns_immediately() {
        let executor = MockContinuousExecutor::new()
            .with_iterations(vec![Ok(ok_snapshot("abc"))])
            .with_gates(vec![passing_gates()]);

        let config = RunnerConfig {
            max_iterations: 0,
            ..RunnerConfig::default()
        };

        let mut runner = ContinuousRunner::new(executor, config, |_| {});
        let outcome = runner.run().await;

        assert_eq!(
            outcome,
            RunOutcome::MaxIterationsReached { iterations: 0 }
        );
    }
}
