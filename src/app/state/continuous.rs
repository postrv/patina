use super::background::{ContinuousLoopStatus, GateResult};

/// Continuous coding loop state extracted from AppState.
///
/// Groups all fields related to the continuous (Ralph-style) coding loop:
/// status, iteration tracking, timing, and quality gate results.
pub struct ContinuousLoopState {
    /// Current status of the continuous coding loop.
    pub(crate) status: ContinuousLoopStatus,

    /// Number of completed iterations in the current continuous session.
    pub(crate) iterations_completed: u32,

    /// Duration of the last completed iteration in milliseconds.
    pub(crate) last_duration_ms: Option<u64>,

    /// Name of the quality gate currently being checked (if any).
    pub(crate) checking_gate: Option<String>,

    /// Accumulated quality gate results for the current iteration.
    pub(crate) gate_results: Vec<GateResult>,
}

impl ContinuousLoopState {
    /// Returns the current status of the continuous coding loop.
    #[must_use]
    pub fn status(&self) -> &ContinuousLoopStatus {
        &self.status
    }

    /// Returns the number of completed iterations.
    #[must_use]
    pub fn iterations_completed(&self) -> u32 {
        self.iterations_completed
    }

    /// Returns the duration of the last completed iteration in milliseconds.
    #[must_use]
    pub fn last_duration_ms(&self) -> Option<u64> {
        self.last_duration_ms
    }

    /// Returns the name of the quality gate currently being checked.
    #[must_use]
    pub fn checking_gate(&self) -> Option<&str> {
        self.checking_gate.as_deref()
    }

    /// Returns accumulated quality gate results for the current iteration.
    #[must_use]
    pub fn gate_results(&self) -> &[GateResult] {
        &self.gate_results
    }

    /// Updates state for a new continuous iteration starting.
    pub fn update_iteration(&mut self, iteration: u32) {
        self.status = ContinuousLoopStatus::Running { iteration };
        self.checking_gate = None;
        self.gate_results.clear();
    }

    /// Records the completion of a continuous iteration.
    pub fn complete_iteration(&mut self, duration_ms: u64) {
        self.iterations_completed += 1;
        self.last_duration_ms = Some(duration_ms);
    }

    /// Records that a quality gate check is starting.
    pub fn set_gate_checking(&mut self, gate: &str) {
        self.checking_gate = Some(gate.to_string());
    }

    /// Records the result of a quality gate check.
    pub fn record_gate_result(&mut self, gate: &str, passed: bool, message: Option<&str>) {
        self.checking_gate = None;
        self.gate_results.push(GateResult {
            gate: gate.to_string(),
            passed,
            message: message.map(String::from),
        });
    }

    /// Records that stagnation was detected.
    pub fn set_stagnation(&mut self, iterations_without_progress: u32, threshold: u32) {
        self.status = ContinuousLoopStatus::Stagnated {
            iterations_without_progress,
            threshold,
        };
    }

    /// Records that human intervention is required.
    pub fn set_human_checkpoint(&mut self, reason: &str) {
        self.status = ContinuousLoopStatus::HumanRequired {
            reason: reason.to_string(),
        };
    }

    /// Resets all continuous loop state to inactive.
    pub fn reset(&mut self) {
        self.status = ContinuousLoopStatus::Inactive;
        self.iterations_completed = 0;
        self.last_duration_ms = None;
        self.checking_gate = None;
        self.gate_results.clear();
    }
}
