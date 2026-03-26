use crate::types::ui_state::{ContinuousLoopStatus, GateResult};

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

    /// Whether any mutation has occurred since last `mark_clean()`.
    pub(crate) dirty: bool,
}

impl ContinuousLoopState {
    /// Returns whether any mutation has occurred since last `mark_clean()`.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Clears the dirty flag after rendering.
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

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
        self.dirty = true;
        self.status = ContinuousLoopStatus::Running { iteration };
        self.checking_gate = None;
        self.gate_results.clear();
    }

    /// Records the completion of a continuous iteration.
    pub fn complete_iteration(&mut self, duration_ms: u64) {
        self.dirty = true;
        self.iterations_completed += 1;
        self.last_duration_ms = Some(duration_ms);
    }

    /// Records that a quality gate check is starting.
    pub fn set_gate_checking(&mut self, gate: &str) {
        self.dirty = true;
        self.checking_gate = Some(gate.to_string());
    }

    /// Records the result of a quality gate check.
    pub fn record_gate_result(&mut self, gate: &str, passed: bool, message: Option<&str>) {
        self.dirty = true;
        self.checking_gate = None;
        self.gate_results.push(GateResult {
            gate: gate.to_string(),
            passed,
            message: message.map(String::from),
        });
    }

    /// Records that stagnation was detected.
    pub fn set_stagnation(&mut self, iterations_without_progress: u32, threshold: u32) {
        self.dirty = true;
        self.status = ContinuousLoopStatus::Stagnated {
            iterations_without_progress,
            threshold,
        };
    }

    /// Records that human intervention is required.
    pub fn set_human_checkpoint(&mut self, reason: &str) {
        self.dirty = true;
        self.status = ContinuousLoopStatus::HumanRequired {
            reason: reason.to_string(),
        };
    }

    /// Resets all continuous loop state to inactive.
    pub fn reset(&mut self) {
        self.dirty = true;
        self.status = ContinuousLoopStatus::Inactive;
        self.iterations_completed = 0;
        self.last_duration_ms = None;
        self.checking_gate = None;
        self.gate_results.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state() -> ContinuousLoopState {
        ContinuousLoopState {
            status: ContinuousLoopStatus::Inactive,
            iterations_completed: 0,
            last_duration_ms: None,
            checking_gate: None,
            gate_results: Vec::new(),
            dirty: false,
        }
    }

    #[test]
    fn test_set_gate_checking() {
        let mut state = make_state();

        state.set_gate_checking("clippy");
        assert_eq!(state.checking_gate(), Some("clippy"));

        state.record_gate_result("clippy", true, None);
        assert_eq!(state.checking_gate(), None);
    }

    #[test]
    fn test_record_gate_result_accumulates() {
        let mut state = make_state();

        state.record_gate_result("clippy", true, None);
        state.record_gate_result("tests", false, Some("3 failures"));
        state.record_gate_result("fmt", true, Some("ok"));

        assert_eq!(state.gate_results().len(), 3);
        assert_eq!(
            state.gate_results()[0],
            GateResult {
                gate: "clippy".to_string(),
                passed: true,
                message: None,
            }
        );
        assert_eq!(
            state.gate_results()[1],
            GateResult {
                gate: "tests".to_string(),
                passed: false,
                message: Some("3 failures".to_string()),
            }
        );
        assert_eq!(
            state.gate_results()[2],
            GateResult {
                gate: "fmt".to_string(),
                passed: true,
                message: Some("ok".to_string()),
            }
        );
    }

    #[test]
    fn test_update_iteration_clears_gates() {
        let mut state = make_state();

        state.set_gate_checking("clippy");
        state.record_gate_result("clippy", true, None);
        state.set_gate_checking("tests");

        assert!(!state.gate_results().is_empty());
        assert!(state.checking_gate().is_some());

        state.update_iteration(2);

        assert!(state.gate_results().is_empty());
        assert_eq!(state.checking_gate(), None);
        assert_eq!(
            *state.status(),
            ContinuousLoopStatus::Running { iteration: 2 }
        );
    }

    #[test]
    fn test_complete_iteration_accumulates() {
        let mut state = make_state();

        state.complete_iteration(1000);
        assert_eq!(state.iterations_completed(), 1);
        assert_eq!(state.last_duration_ms(), Some(1000));

        state.complete_iteration(2000);
        assert_eq!(state.iterations_completed(), 2);
        assert_eq!(state.last_duration_ms(), Some(2000));

        state.complete_iteration(500);
        assert_eq!(state.iterations_completed(), 3);
        assert_eq!(state.last_duration_ms(), Some(500));
    }

    #[test]
    fn test_set_human_checkpoint() {
        let mut state = make_state();

        state.set_human_checkpoint("merge conflict detected");

        assert_eq!(
            *state.status(),
            ContinuousLoopStatus::HumanRequired {
                reason: "merge conflict detected".to_string(),
            }
        );
    }
}
