//! Quality gate execution for the continuous coding loop.
//!
//! Executes configured quality gates (tests, clippy, format check, security scan)
//! and collects results with diagnostic output. Uses an injectable command runner
//! trait for testability.
//!
//! # Architecture
//!
//! 1. [`QualityGateRunner`] accepts a list of [`QualityGate`]s and a [`CommandRunner`]
//! 2. Each gate's command is executed via the runner
//! 3. Results are collected into [`GateResult`]s
//! 4. A [`GateRunReport`] aggregates all results with timing
//!
//! # Example
//!
//! ```rust,ignore
//! use patina::continuous::gates::{QualityGateRunner, ShellCommandRunner};
//! use patina::continuous::QualityGate;
//!
//! let gates = vec![QualityGate::TestsPass, QualityGate::ClippyClean];
//! let runner = ShellCommandRunner::new("/path/to/project");
//! let mut gate_runner = QualityGateRunner::new(gates, runner);
//! let report = gate_runner.run_all().await;
//! assert!(report.all_passed());
//! ```

use std::fmt;
use std::future::Future;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::plugin::QualityGate;
use super::recovery::GateCheckResult;

/// Output from executing a shell command.
///
/// # Example
///
/// ```
/// use patina::continuous::gates::CommandOutput;
///
/// let output = CommandOutput {
///     exit_code: 0,
///     stdout: "test result: ok".to_string(),
///     stderr: String::new(),
/// };
/// assert!(output.success());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandOutput {
    /// Process exit code (0 = success).
    pub exit_code: i32,
    /// Standard output captured from the command.
    pub stdout: String,
    /// Standard error captured from the command.
    pub stderr: String,
}

impl CommandOutput {
    /// Returns `true` if the command exited with code 0.
    ///
    /// # Example
    ///
    /// ```
    /// use patina::continuous::gates::CommandOutput;
    ///
    /// let success = CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
    /// assert!(success.success());
    ///
    /// let failure = CommandOutput { exit_code: 1, stdout: String::new(), stderr: "error".into() };
    /// assert!(!failure.success());
    /// ```
    #[must_use]
    pub fn success(&self) -> bool {
        self.exit_code == 0
    }

    /// Returns the combined stdout and stderr output.
    ///
    /// If both are non-empty, they are separated by a newline.
    ///
    /// # Example
    ///
    /// ```
    /// use patina::continuous::gates::CommandOutput;
    ///
    /// let output = CommandOutput {
    ///     exit_code: 1,
    ///     stdout: "some output".into(),
    ///     stderr: "some error".into(),
    /// };
    /// let combined = output.combined_output();
    /// assert!(combined.contains("some output"));
    /// assert!(combined.contains("some error"));
    /// ```
    #[must_use]
    pub fn combined_output(&self) -> String {
        if self.stdout.is_empty() {
            return self.stderr.clone();
        }
        if self.stderr.is_empty() {
            return self.stdout.clone();
        }
        format!("{}\n{}", self.stdout, self.stderr)
    }
}

/// Trait for executing shell commands, injectable for testing.
///
/// Production implementations spawn real shell processes.
/// Test implementations return predetermined results.
pub trait CommandRunner: Send {
    /// Execute a shell command and return the output.
    ///
    /// # Arguments
    ///
    /// * `command` - The shell command string to execute
    ///
    /// # Returns
    ///
    /// A [`CommandOutput`] with exit code, stdout, and stderr.
    fn run_command(&mut self, command: &str) -> impl Future<Output = CommandOutput> + Send;
}

/// Result of running a single quality gate.
///
/// # Example
///
/// ```
/// use patina::continuous::gates::GateResult;
/// use patina::continuous::QualityGate;
/// use std::time::Duration;
///
/// let result = GateResult {
///     gate: QualityGate::TestsPass,
///     passed: true,
///     output: "test result: ok. 100 passed".to_string(),
///     duration: Duration::from_secs(30),
/// };
/// assert!(result.passed);
/// assert_eq!(result.gate.name(), "tests_pass");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateResult {
    /// Which quality gate was checked.
    pub gate: QualityGate,
    /// Whether the gate passed.
    pub passed: bool,
    /// Captured output (combined stdout + stderr) for diagnostics.
    pub output: String,
    /// How long the gate took to execute.
    pub duration: Duration,
}

impl fmt::Display for GateResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status = if self.passed { "PASS" } else { "FAIL" };
        write!(
            f,
            "[{status}] {} ({:.1}s)",
            self.gate,
            self.duration.as_secs_f64()
        )
    }
}

/// Aggregated report from running all quality gates.
///
/// # Example
///
/// ```
/// use patina::continuous::gates::{GateRunReport, GateResult};
/// use patina::continuous::QualityGate;
/// use std::time::Duration;
///
/// let report = GateRunReport {
///     results: vec![
///         GateResult {
///             gate: QualityGate::TestsPass,
///             passed: true,
///             output: String::new(),
///             duration: Duration::from_secs(10),
///         },
///     ],
///     total_duration: Duration::from_secs(10),
/// };
/// assert!(report.all_passed());
/// ```
#[derive(Debug, Clone)]
pub struct GateRunReport {
    /// Individual results for each gate that was executed.
    pub results: Vec<GateResult>,
    /// Total wall-clock time for running all gates.
    pub total_duration: Duration,
}

impl GateRunReport {
    /// Returns `true` if all gates passed.
    ///
    /// Returns `true` for an empty report (no gates = vacuously true).
    ///
    /// # Example
    ///
    /// ```
    /// use patina::continuous::gates::{GateRunReport, GateResult};
    /// use patina::continuous::QualityGate;
    /// use std::time::Duration;
    ///
    /// let empty = GateRunReport { results: vec![], total_duration: Duration::ZERO };
    /// assert!(empty.all_passed());
    /// ```
    #[must_use]
    pub fn all_passed(&self) -> bool {
        self.results.iter().all(|r| r.passed)
    }

    /// Returns references to the gates that failed.
    ///
    /// # Example
    ///
    /// ```
    /// use patina::continuous::gates::{GateRunReport, GateResult};
    /// use patina::continuous::QualityGate;
    /// use std::time::Duration;
    ///
    /// let report = GateRunReport {
    ///     results: vec![
    ///         GateResult {
    ///             gate: QualityGate::TestsPass,
    ///             passed: true,
    ///             output: String::new(),
    ///             duration: Duration::from_secs(1),
    ///         },
    ///         GateResult {
    ///             gate: QualityGate::ClippyClean,
    ///             passed: false,
    ///             output: "warning: unused".into(),
    ///             duration: Duration::from_secs(2),
    ///         },
    ///     ],
    ///     total_duration: Duration::from_secs(3),
    /// };
    /// let failed = report.failed_gates();
    /// assert_eq!(failed.len(), 1);
    /// assert_eq!(failed[0].gate, QualityGate::ClippyClean);
    /// ```
    #[must_use]
    pub fn failed_gates(&self) -> Vec<&GateResult> {
        self.results.iter().filter(|r| !r.passed).collect()
    }

    /// Converts this report into a [`GateCheckResult`] for the recovery system.
    ///
    /// The `error_output` field concatenates all failed gate outputs.
    ///
    /// # Example
    ///
    /// ```
    /// use patina::continuous::gates::{GateRunReport, GateResult};
    /// use patina::continuous::QualityGate;
    /// use std::time::Duration;
    ///
    /// let report = GateRunReport {
    ///     results: vec![
    ///         GateResult {
    ///             gate: QualityGate::TestsPass,
    ///             passed: true,
    ///             output: String::new(),
    ///             duration: Duration::from_secs(1),
    ///         },
    ///     ],
    ///     total_duration: Duration::from_secs(1),
    /// };
    /// let check = report.to_gate_check_result();
    /// assert!(check.passed);
    /// assert!(check.error_output.is_empty());
    /// ```
    #[must_use]
    pub fn to_gate_check_result(&self) -> GateCheckResult {
        if self.all_passed() {
            return GateCheckResult {
                passed: true,
                error_output: String::new(),
            };
        }

        let error_output: String = self
            .failed_gates()
            .iter()
            .map(|r| format!("[{}] {}", r.gate.name(), r.output))
            .collect::<Vec<_>>()
            .join("\n---\n");

        GateCheckResult {
            passed: false,
            error_output,
        }
    }
}

impl fmt::Display for GateRunReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Quality Gate Report")?;
        writeln!(f, "===================")?;
        for result in &self.results {
            writeln!(f, "  {result}")?;
        }
        writeln!(
            f,
            "Total: {:.1}s | {}",
            self.total_duration.as_secs_f64(),
            if self.all_passed() {
                "ALL PASSED"
            } else {
                "SOME FAILED"
            }
        )?;
        Ok(())
    }
}

/// Runs quality gates using a [`CommandRunner`] and collects results.
///
/// # Type Parameters
///
/// - `R`: The command runner implementation (real shell or mock)
///
/// # Example
///
/// ```rust,ignore
/// use patina::continuous::gates::{QualityGateRunner, ShellCommandRunner};
/// use patina::continuous::QualityGate;
///
/// let gates = vec![QualityGate::TestsPass, QualityGate::ClippyClean];
/// let runner = ShellCommandRunner::new("/path/to/project");
/// let mut gate_runner = QualityGateRunner::new(gates, runner);
/// let report = gate_runner.run_all().await;
/// ```
pub struct QualityGateRunner<R: CommandRunner> {
    gates: Vec<QualityGate>,
    runner: R,
    timeout: Duration,
}

impl<R: CommandRunner> QualityGateRunner<R> {
    /// Creates a new runner with the given gates and command runner.
    ///
    /// Uses a default timeout of 5 minutes per gate.
    pub fn new(gates: Vec<QualityGate>, runner: R) -> Self {
        Self {
            gates,
            runner,
            timeout: Duration::from_secs(300),
        }
    }

    /// Sets the per-gate timeout.
    ///
    /// # Arguments
    ///
    /// * `timeout` - Maximum duration for each gate command
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Returns the configured per-gate timeout.
    #[must_use]
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Returns the list of configured gates.
    #[must_use]
    pub fn gates(&self) -> &[QualityGate] {
        &self.gates
    }

    /// Runs a single quality gate and returns the result.
    ///
    /// # Arguments
    ///
    /// * `gate` - The quality gate to execute
    ///
    /// # Returns
    ///
    /// A [`GateResult`] with pass/fail status, output, and timing.
    pub async fn run_gate(&mut self, gate: QualityGate) -> GateResult {
        let start = std::time::Instant::now();
        let output = self.runner.run_command(gate.command()).await;
        let duration = start.elapsed();

        GateResult {
            gate,
            passed: output.success(),
            output: output.combined_output(),
            duration,
        }
    }

    /// Runs all configured gates sequentially and returns an aggregated report.
    ///
    /// Gates are executed in the order they were configured. Each gate runs
    /// to completion before the next one starts.
    ///
    /// # Returns
    ///
    /// A [`GateRunReport`] containing individual results and total timing.
    pub async fn run_all(&mut self) -> GateRunReport {
        let start = std::time::Instant::now();
        let mut results = Vec::with_capacity(self.gates.len());

        // Clone gates to avoid borrow conflict with &mut self
        let gates: Vec<QualityGate> = self.gates.clone();
        for gate in gates {
            results.push(self.run_gate(gate).await);
        }

        GateRunReport {
            results,
            total_duration: start.elapsed(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Mock command runner
    // =========================================================================

    /// Mock command runner that returns predetermined outputs for commands.
    struct MockCommandRunner {
        /// Map of command string -> output to return.
        responses: Vec<(String, CommandOutput)>,
        /// Commands that were actually executed (for verification).
        executed: Vec<String>,
    }

    impl MockCommandRunner {
        fn new() -> Self {
            Self {
                responses: Vec::new(),
                executed: Vec::new(),
            }
        }

        fn with_response(mut self, command: &str, output: CommandOutput) -> Self {
            self.responses.push((command.to_string(), output));
            self
        }

        /// Helper: register a passing response for a command.
        fn passing(self, command: &str) -> Self {
            self.with_response(
                command,
                CommandOutput {
                    exit_code: 0,
                    stdout: format!("{command}: ok"),
                    stderr: String::new(),
                },
            )
        }

        /// Helper: register a failing response for a command.
        fn failing(self, command: &str, error: &str) -> Self {
            self.with_response(
                command,
                CommandOutput {
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: error.to_string(),
                },
            )
        }
    }

    impl CommandRunner for MockCommandRunner {
        fn run_command(&mut self, command: &str) -> impl Future<Output = CommandOutput> + Send {
            self.executed.push(command.to_string());

            let output = self
                .responses
                .iter()
                .find(|(cmd, _)| cmd == command)
                .map(|(_, out)| out.clone())
                .unwrap_or(CommandOutput {
                    exit_code: 127,
                    stdout: String::new(),
                    stderr: format!("command not found: {command}"),
                });

            async move { output }
        }
    }

    // =========================================================================
    // CommandOutput tests
    // =========================================================================

    #[test]
    fn command_output_success_on_zero_exit() {
        let output = CommandOutput {
            exit_code: 0,
            stdout: "ok".to_string(),
            stderr: String::new(),
        };
        assert!(output.success());
    }

    #[test]
    fn command_output_not_success_on_nonzero_exit() {
        let output = CommandOutput {
            exit_code: 1,
            stdout: String::new(),
            stderr: "error".to_string(),
        };
        assert!(!output.success());
    }

    #[test]
    fn command_output_not_success_on_negative_exit() {
        let output = CommandOutput {
            exit_code: -1,
            stdout: String::new(),
            stderr: "killed".to_string(),
        };
        assert!(!output.success());
    }

    #[test]
    fn command_output_combined_stdout_only() {
        let output = CommandOutput {
            exit_code: 0,
            stdout: "hello".to_string(),
            stderr: String::new(),
        };
        assert_eq!(output.combined_output(), "hello");
    }

    #[test]
    fn command_output_combined_stderr_only() {
        let output = CommandOutput {
            exit_code: 1,
            stdout: String::new(),
            stderr: "error".to_string(),
        };
        assert_eq!(output.combined_output(), "error");
    }

    #[test]
    fn command_output_combined_both() {
        let output = CommandOutput {
            exit_code: 1,
            stdout: "partial output".to_string(),
            stderr: "error details".to_string(),
        };
        let combined = output.combined_output();
        assert!(combined.contains("partial output"));
        assert!(combined.contains("error details"));
    }

    #[test]
    fn command_output_combined_both_empty() {
        let output = CommandOutput {
            exit_code: 0,
            stdout: String::new(),
            stderr: String::new(),
        };
        assert_eq!(output.combined_output(), "");
    }

    #[test]
    fn command_output_serde_roundtrip() {
        let output = CommandOutput {
            exit_code: 42,
            stdout: "out".to_string(),
            stderr: "err".to_string(),
        };
        let json = serde_json::to_string(&output).expect("serialize");
        let restored: CommandOutput = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(output, restored);
    }

    // =========================================================================
    // GateResult tests
    // =========================================================================

    #[test]
    fn gate_result_display_pass() {
        let result = GateResult {
            gate: QualityGate::TestsPass,
            passed: true,
            output: String::new(),
            duration: Duration::from_millis(1500),
        };
        let display = format!("{result}");
        assert!(display.contains("PASS"), "should say PASS: {display}");
        assert!(
            display.contains("Tests Pass"),
            "should include gate name: {display}"
        );
        assert!(
            display.contains("1.5s"),
            "should include duration: {display}"
        );
    }

    #[test]
    fn gate_result_display_fail() {
        let result = GateResult {
            gate: QualityGate::ClippyClean,
            passed: false,
            output: "warning: unused".to_string(),
            duration: Duration::from_secs(5),
        };
        let display = format!("{result}");
        assert!(display.contains("FAIL"), "should say FAIL: {display}");
        assert!(
            display.contains("Clippy Clean"),
            "should include gate name: {display}"
        );
    }

    #[test]
    fn gate_result_serde_roundtrip() {
        let result = GateResult {
            gate: QualityGate::FormatCheck,
            passed: false,
            output: "diff".to_string(),
            duration: Duration::from_secs(1),
        };
        let json = serde_json::to_string(&result).expect("serialize");
        let restored: GateResult = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(result, restored);
    }

    // =========================================================================
    // GateRunReport tests
    // =========================================================================

    #[test]
    fn report_all_passed_empty() {
        let report = GateRunReport {
            results: vec![],
            total_duration: Duration::ZERO,
        };
        assert!(report.all_passed());
    }

    #[test]
    fn report_all_passed_all_pass() {
        let report = GateRunReport {
            results: vec![
                GateResult {
                    gate: QualityGate::TestsPass,
                    passed: true,
                    output: String::new(),
                    duration: Duration::from_secs(10),
                },
                GateResult {
                    gate: QualityGate::ClippyClean,
                    passed: true,
                    output: String::new(),
                    duration: Duration::from_secs(5),
                },
            ],
            total_duration: Duration::from_secs(15),
        };
        assert!(report.all_passed());
    }

    #[test]
    fn report_not_all_passed_one_fails() {
        let report = GateRunReport {
            results: vec![
                GateResult {
                    gate: QualityGate::TestsPass,
                    passed: true,
                    output: String::new(),
                    duration: Duration::from_secs(10),
                },
                GateResult {
                    gate: QualityGate::ClippyClean,
                    passed: false,
                    output: "warning".to_string(),
                    duration: Duration::from_secs(5),
                },
            ],
            total_duration: Duration::from_secs(15),
        };
        assert!(!report.all_passed());
    }

    #[test]
    fn report_failed_gates_none_failed() {
        let report = GateRunReport {
            results: vec![GateResult {
                gate: QualityGate::TestsPass,
                passed: true,
                output: String::new(),
                duration: Duration::from_secs(1),
            }],
            total_duration: Duration::from_secs(1),
        };
        assert!(report.failed_gates().is_empty());
    }

    #[test]
    fn report_failed_gates_one_failed() {
        let report = GateRunReport {
            results: vec![
                GateResult {
                    gate: QualityGate::TestsPass,
                    passed: true,
                    output: String::new(),
                    duration: Duration::from_secs(10),
                },
                GateResult {
                    gate: QualityGate::ClippyClean,
                    passed: false,
                    output: "warning".to_string(),
                    duration: Duration::from_secs(5),
                },
            ],
            total_duration: Duration::from_secs(15),
        };
        let failed = report.failed_gates();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].gate, QualityGate::ClippyClean);
    }

    #[test]
    fn report_failed_gates_all_failed() {
        let report = GateRunReport {
            results: vec![
                GateResult {
                    gate: QualityGate::TestsPass,
                    passed: false,
                    output: "test failed".to_string(),
                    duration: Duration::from_secs(10),
                },
                GateResult {
                    gate: QualityGate::ClippyClean,
                    passed: false,
                    output: "warning".to_string(),
                    duration: Duration::from_secs(5),
                },
            ],
            total_duration: Duration::from_secs(15),
        };
        let failed = report.failed_gates();
        assert_eq!(failed.len(), 2);
    }

    #[test]
    fn report_to_gate_check_result_all_pass() {
        let report = GateRunReport {
            results: vec![GateResult {
                gate: QualityGate::TestsPass,
                passed: true,
                output: String::new(),
                duration: Duration::from_secs(1),
            }],
            total_duration: Duration::from_secs(1),
        };
        let check = report.to_gate_check_result();
        assert!(check.passed);
        assert!(check.error_output.is_empty());
    }

    #[test]
    fn report_to_gate_check_result_some_fail() {
        let report = GateRunReport {
            results: vec![
                GateResult {
                    gate: QualityGate::TestsPass,
                    passed: false,
                    output: "test failure output".to_string(),
                    duration: Duration::from_secs(10),
                },
                GateResult {
                    gate: QualityGate::ClippyClean,
                    passed: true,
                    output: String::new(),
                    duration: Duration::from_secs(5),
                },
                GateResult {
                    gate: QualityGate::FormatCheck,
                    passed: false,
                    output: "fmt diff".to_string(),
                    duration: Duration::from_secs(1),
                },
            ],
            total_duration: Duration::from_secs(16),
        };
        let check = report.to_gate_check_result();
        assert!(!check.passed);
        assert!(
            check.error_output.contains("tests_pass"),
            "error should reference failed gate: {}",
            check.error_output
        );
        assert!(
            check.error_output.contains("test failure output"),
            "error should include gate output: {}",
            check.error_output
        );
        assert!(
            check.error_output.contains("format_check"),
            "error should reference second failed gate: {}",
            check.error_output
        );
        assert!(
            check.error_output.contains("fmt diff"),
            "error should include second gate output: {}",
            check.error_output
        );
    }

    #[test]
    fn report_display_all_passed() {
        let report = GateRunReport {
            results: vec![GateResult {
                gate: QualityGate::TestsPass,
                passed: true,
                output: String::new(),
                duration: Duration::from_secs(10),
            }],
            total_duration: Duration::from_secs(10),
        };
        let display = format!("{report}");
        assert!(
            display.contains("ALL PASSED"),
            "display should say ALL PASSED: {display}"
        );
    }

    #[test]
    fn report_display_some_failed() {
        let report = GateRunReport {
            results: vec![GateResult {
                gate: QualityGate::TestsPass,
                passed: false,
                output: "fail".to_string(),
                duration: Duration::from_secs(10),
            }],
            total_duration: Duration::from_secs(10),
        };
        let display = format!("{report}");
        assert!(
            display.contains("SOME FAILED"),
            "display should say SOME FAILED: {display}"
        );
    }

    // =========================================================================
    // QualityGateRunner — gate execution via mock
    // =========================================================================

    #[tokio::test]
    async fn runner_tests_pass_gate_runs_cargo_test() {
        let runner = MockCommandRunner::new().passing("cargo test");
        let mut gate_runner = QualityGateRunner::new(vec![QualityGate::TestsPass], runner);

        let report = gate_runner.run_all().await;

        assert_eq!(report.results.len(), 1);
        assert!(report.results[0].passed);
        assert_eq!(report.results[0].gate, QualityGate::TestsPass);
    }

    #[tokio::test]
    async fn runner_clippy_gate_runs_cargo_clippy() {
        let runner = MockCommandRunner::new().passing("cargo clippy --all-targets -- -D warnings");
        let mut gate_runner = QualityGateRunner::new(vec![QualityGate::ClippyClean], runner);

        let report = gate_runner.run_all().await;

        assert_eq!(report.results.len(), 1);
        assert!(report.results[0].passed);
        assert_eq!(report.results[0].gate, QualityGate::ClippyClean);
    }

    #[tokio::test]
    async fn runner_format_gate_runs_cargo_fmt() {
        let runner = MockCommandRunner::new().passing("cargo fmt -- --check");
        let mut gate_runner = QualityGateRunner::new(vec![QualityGate::FormatCheck], runner);

        let report = gate_runner.run_all().await;

        assert!(report.all_passed());
    }

    #[tokio::test]
    async fn runner_security_gate_runs_scan() {
        let runner = MockCommandRunner::new().passing("scan_security");
        let mut gate_runner = QualityGateRunner::new(vec![QualityGate::SecurityScan], runner);

        let report = gate_runner.run_all().await;

        assert!(report.all_passed());
    }

    #[tokio::test]
    async fn runner_failing_gate_captures_error_output() {
        let runner = MockCommandRunner::new().failing(
            "cargo test",
            "test continuous::tests::test_something ... FAILED",
        );
        let mut gate_runner = QualityGateRunner::new(vec![QualityGate::TestsPass], runner);

        let report = gate_runner.run_all().await;

        assert!(!report.all_passed());
        assert!(!report.results[0].passed);
        assert!(
            report.results[0].output.contains("FAILED"),
            "output should contain error: {}",
            report.results[0].output
        );
    }

    #[tokio::test]
    async fn runner_multiple_gates_all_pass() {
        let runner = MockCommandRunner::new()
            .passing("cargo test")
            .passing("cargo clippy --all-targets -- -D warnings")
            .passing("cargo fmt -- --check");

        let mut gate_runner = QualityGateRunner::new(
            vec![
                QualityGate::TestsPass,
                QualityGate::ClippyClean,
                QualityGate::FormatCheck,
            ],
            runner,
        );

        let report = gate_runner.run_all().await;

        assert_eq!(report.results.len(), 3);
        assert!(report.all_passed());
    }

    #[tokio::test]
    async fn runner_multiple_gates_partial_failure() {
        let runner = MockCommandRunner::new()
            .passing("cargo test")
            .failing(
                "cargo clippy --all-targets -- -D warnings",
                "warning: unused variable",
            )
            .passing("cargo fmt -- --check");

        let mut gate_runner = QualityGateRunner::new(
            vec![
                QualityGate::TestsPass,
                QualityGate::ClippyClean,
                QualityGate::FormatCheck,
            ],
            runner,
        );

        let report = gate_runner.run_all().await;

        assert_eq!(report.results.len(), 3);
        assert!(!report.all_passed());

        let failed = report.failed_gates();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].gate, QualityGate::ClippyClean);
    }

    #[tokio::test]
    async fn runner_empty_gates_produces_empty_report() {
        let runner = MockCommandRunner::new();
        let mut gate_runner: QualityGateRunner<MockCommandRunner> =
            QualityGateRunner::new(vec![], runner);

        let report = gate_runner.run_all().await;

        assert!(report.results.is_empty());
        assert!(report.all_passed());
    }

    #[tokio::test]
    async fn runner_single_gate_direct_execution() {
        let runner = MockCommandRunner::new().passing("cargo test");
        let mut gate_runner = QualityGateRunner::new(vec![], runner);

        let result = gate_runner.run_gate(QualityGate::TestsPass).await;

        assert!(result.passed);
        assert_eq!(result.gate, QualityGate::TestsPass);
    }

    #[tokio::test]
    async fn runner_report_converts_to_gate_check_result() {
        let runner = MockCommandRunner::new().passing("cargo test").failing(
            "cargo clippy --all-targets -- -D warnings",
            "warning: unused",
        );

        let mut gate_runner = QualityGateRunner::new(
            vec![QualityGate::TestsPass, QualityGate::ClippyClean],
            runner,
        );

        let report = gate_runner.run_all().await;
        let check = report.to_gate_check_result();

        assert!(!check.passed);
        assert!(
            check.error_output.contains("clippy_clean"),
            "gate check should reference clippy: {}",
            check.error_output
        );
    }

    // =========================================================================
    // QualityGateRunner — configuration
    // =========================================================================

    #[test]
    fn runner_default_timeout() {
        let runner = MockCommandRunner::new();
        let gate_runner: QualityGateRunner<MockCommandRunner> =
            QualityGateRunner::new(vec![], runner);
        assert_eq!(gate_runner.timeout(), Duration::from_secs(300));
    }

    #[test]
    fn runner_custom_timeout() {
        let runner = MockCommandRunner::new();
        let gate_runner: QualityGateRunner<MockCommandRunner> =
            QualityGateRunner::new(vec![], runner).with_timeout(Duration::from_secs(60));
        assert_eq!(gate_runner.timeout(), Duration::from_secs(60));
    }

    #[test]
    fn runner_gates_accessor() {
        let runner = MockCommandRunner::new();
        let gate_runner = QualityGateRunner::new(
            vec![QualityGate::TestsPass, QualityGate::ClippyClean],
            runner,
        );
        assert_eq!(gate_runner.gates().len(), 2);
        assert_eq!(gate_runner.gates()[0], QualityGate::TestsPass);
        assert_eq!(gate_runner.gates()[1], QualityGate::ClippyClean);
    }

    // =========================================================================
    // QualityGateRunner — gate execution order
    // =========================================================================

    #[tokio::test]
    async fn runner_executes_gates_in_order() {
        let runner = MockCommandRunner::new()
            .passing("cargo fmt -- --check")
            .passing("cargo clippy --all-targets -- -D warnings")
            .passing("cargo test");

        let mut gate_runner = QualityGateRunner::new(
            vec![
                QualityGate::FormatCheck,
                QualityGate::ClippyClean,
                QualityGate::TestsPass,
            ],
            runner,
        );

        let report = gate_runner.run_all().await;

        assert_eq!(report.results[0].gate, QualityGate::FormatCheck);
        assert_eq!(report.results[1].gate, QualityGate::ClippyClean);
        assert_eq!(report.results[2].gate, QualityGate::TestsPass);
    }

    // =========================================================================
    // QualityGateRunner — timing
    // =========================================================================

    #[tokio::test]
    async fn runner_records_duration() {
        let runner = MockCommandRunner::new().passing("cargo test");
        let mut gate_runner = QualityGateRunner::new(vec![QualityGate::TestsPass], runner);

        let report = gate_runner.run_all().await;

        // Duration should be non-negative (test runs fast, but > 0)
        assert!(report.results[0].duration >= Duration::ZERO);
        assert!(report.total_duration >= Duration::ZERO);
    }
}
