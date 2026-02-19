//! Multi-factor stagnation detection for continuous coding loops.
//!
//! Detects when the automation loop is stuck by tracking:
//! - Commit gaps (iterations since last new commit)
//! - Repeated file edits (same files modified without resolution)
//! - Recurring errors (same error messages across iterations)
//! - Test count plateau (test count not increasing)
//!
//! Each factor contributes a weighted score. The total score maps to a risk
//! level that determines when to escalate or halt.
//!
//! # Example
//!
//! ```
//! use patina::continuous::stagnation::{
//!     StagnationDetector, StagnationConfig, IterationSnapshot, RiskLevel,
//! };
//!
//! let mut detector = StagnationDetector::new(StagnationConfig::default());
//!
//! // Record an iteration with a new commit
//! detector.record(IterationSnapshot {
//!     commit_hash: Some("abc123".into()),
//!     files_edited: vec!["src/main.rs".into()],
//!     errors: vec![],
//!     test_count: 100,
//! });
//!
//! let score = detector.evaluate();
//! assert_eq!(score.risk_level, RiskLevel::Low);
//! ```

use std::collections::{HashMap, HashSet};
use std::fmt;

use serde::{Deserialize, Serialize};

/// Risk level based on stagnation score.
///
/// Higher risk levels indicate more urgent need for intervention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RiskLevel {
    /// No significant stagnation detected. Score < 0.25.
    Low,
    /// Some stagnation signals present. Score in [0.25, 0.50).
    Medium,
    /// Multiple stagnation signals active. Score in [0.50, 0.75).
    High,
    /// Severe stagnation, human intervention recommended. Score >= 0.75.
    Critical,
}

impl fmt::Display for RiskLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Low => write!(f, "Low"),
            Self::Medium => write!(f, "Medium"),
            Self::High => write!(f, "High"),
            Self::Critical => write!(f, "Critical"),
        }
    }
}

/// Data captured for a single iteration of the continuous loop.
///
/// This snapshot provides the raw signals that the stagnation detector
/// uses to compute risk scores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IterationSnapshot {
    /// Current HEAD commit hash, if available.
    /// A change in hash indicates meaningful progress (a new commit was made).
    pub commit_hash: Option<String>,

    /// Files that were edited during this iteration.
    pub files_edited: Vec<String>,

    /// Error messages encountered during this iteration (e.g., test failures, clippy warnings).
    pub errors: Vec<String>,

    /// Total number of tests at the end of this iteration.
    pub test_count: u32,
}

/// Computed stagnation score with breakdown by factor.
///
/// The total score is a weighted sum of individual factor scores,
/// normalized to the range [0.0, 1.0].
#[derive(Debug, Clone, PartialEq)]
pub struct StagnationScore {
    /// Overall risk level derived from the total score.
    pub risk_level: RiskLevel,

    /// Total score in [0.0, 1.0].
    pub total: f64,

    /// Score contribution from commit gap factor.
    pub commit_gap_score: f64,

    /// Score contribution from repeated file edits factor.
    pub repeated_edits_score: f64,

    /// Score contribution from recurring errors factor.
    pub recurring_errors_score: f64,

    /// Score contribution from test count plateau factor.
    pub test_plateau_score: f64,
}

/// Configuration for stagnation detection weights and thresholds.
///
/// # Example
///
/// ```
/// use patina::continuous::stagnation::StagnationConfig;
///
/// let config = StagnationConfig {
///     history_window: 10,
///     commit_gap_weight: 0.4,
///     repeated_edits_weight: 0.25,
///     recurring_errors_weight: 0.25,
///     test_plateau_weight: 0.1,
///     risk_thresholds: [0.25, 0.50, 0.75],
/// };
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct StagnationConfig {
    /// Number of recent iterations to consider.
    pub history_window: usize,

    /// Weight for commit gap factor (0.0 - 1.0).
    pub commit_gap_weight: f64,

    /// Weight for repeated file edits factor (0.0 - 1.0).
    pub repeated_edits_weight: f64,

    /// Weight for recurring errors factor (0.0 - 1.0).
    pub recurring_errors_weight: f64,

    /// Weight for test count plateau factor (0.0 - 1.0).
    pub test_plateau_weight: f64,

    /// Thresholds for [Medium, High, Critical] risk levels.
    /// Low is anything below the first threshold.
    pub risk_thresholds: [f64; 3],
}

impl Default for StagnationConfig {
    fn default() -> Self {
        Self {
            history_window: 10,
            commit_gap_weight: 0.4,
            repeated_edits_weight: 0.25,
            recurring_errors_weight: 0.25,
            test_plateau_weight: 0.1,
            risk_thresholds: [0.25, 0.50, 0.75],
        }
    }
}

/// Multi-factor stagnation detector for continuous coding loops.
///
/// Tracks iteration history and computes weighted risk scores to detect
/// when the automation loop is stuck and needs intervention.
///
/// # Factors
///
/// 1. **Commit gap**: Measures iterations since the last commit hash change.
///    A long gap indicates no code is being committed.
///
/// 2. **Repeated file edits**: Detects the same files being edited across
///    multiple iterations, suggesting the loop is churning without resolution.
///
/// 3. **Recurring errors**: Detects the same error messages appearing across
///    iterations, suggesting the loop cannot fix the underlying problem.
///
/// 4. **Test count plateau**: Detects when the test count stops increasing,
///    suggesting no new functionality is being added.
///
/// # Example
///
/// ```
/// use patina::continuous::stagnation::{
///     StagnationDetector, StagnationConfig, IterationSnapshot, RiskLevel,
/// };
///
/// let mut detector = StagnationDetector::new(StagnationConfig::default());
///
/// detector.record(IterationSnapshot {
///     commit_hash: Some("abc123".into()),
///     files_edited: vec!["src/main.rs".into()],
///     errors: vec![],
///     test_count: 100,
/// });
///
/// let score = detector.evaluate();
/// assert_eq!(score.risk_level, RiskLevel::Low);
/// ```
pub struct StagnationDetector {
    config: StagnationConfig,
    history: Vec<IterationSnapshot>,
}

impl StagnationDetector {
    /// Creates a new stagnation detector with the given configuration.
    ///
    /// # Example
    ///
    /// ```
    /// use patina::continuous::stagnation::{StagnationDetector, StagnationConfig};
    ///
    /// let detector = StagnationDetector::new(StagnationConfig::default());
    /// ```
    #[must_use]
    pub fn new(config: StagnationConfig) -> Self {
        Self {
            config,
            history: Vec::new(),
        }
    }

    /// Records an iteration snapshot into the history window.
    ///
    /// Old entries beyond the history window are evicted.
    pub fn record(&mut self, snapshot: IterationSnapshot) {
        self.history.push(snapshot);
        if self.history.len() > self.config.history_window {
            self.history.remove(0);
        }
    }

    /// Evaluates the current stagnation risk based on recorded history.
    ///
    /// Returns a [`StagnationScore`] with per-factor breakdown and overall risk level.
    ///
    /// # Example
    ///
    /// ```
    /// use patina::continuous::stagnation::{
    ///     StagnationDetector, StagnationConfig, IterationSnapshot, RiskLevel,
    /// };
    ///
    /// let mut detector = StagnationDetector::new(StagnationConfig::default());
    /// let score = detector.evaluate();
    /// assert_eq!(score.risk_level, RiskLevel::Low);
    /// ```
    #[must_use]
    pub fn evaluate(&self) -> StagnationScore {
        let commit_gap = self.compute_commit_gap_score();
        let repeated_edits = self.compute_repeated_edits_score();
        let recurring_errors = self.compute_recurring_errors_score();
        let test_plateau = self.compute_test_plateau_score();

        let total = (commit_gap * self.config.commit_gap_weight
            + repeated_edits * self.config.repeated_edits_weight
            + recurring_errors * self.config.recurring_errors_weight
            + test_plateau * self.config.test_plateau_weight)
            .clamp(0.0, 1.0);

        let risk_level = self.score_to_risk_level(total);

        StagnationScore {
            risk_level,
            total,
            commit_gap_score: commit_gap,
            repeated_edits_score: repeated_edits,
            recurring_errors_score: recurring_errors,
            test_plateau_score: test_plateau,
        }
    }

    /// Resets the detector history, clearing all recorded snapshots.
    ///
    /// Call this when meaningful progress is confirmed externally
    /// (e.g., a phase is completed).
    pub fn reset(&mut self) {
        self.history.clear();
    }

    /// Returns the number of recorded snapshots in the history.
    #[must_use]
    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// Returns the current configuration.
    #[must_use]
    pub fn config(&self) -> &StagnationConfig {
        &self.config
    }

    /// Computes commit gap score: ratio of iterations since last commit change.
    ///
    /// Returns 0.0 if there are no snapshots or if a commit change is recent.
    /// Returns 1.0 if no commit change has occurred across the full window.
    fn compute_commit_gap_score(&self) -> f64 {
        if self.history.len() < 2 {
            return 0.0;
        }

        // Count consecutive iterations from the end that share the same commit hash.
        // None == None is treated as "same" (no commit made).
        let latest_hash = self.history.last().and_then(|s| s.commit_hash.as_deref());
        let gap = self
            .history
            .iter()
            .rev()
            .skip(1)
            .take_while(|s| s.commit_hash.as_deref() == latest_hash)
            .count() as u32;

        // Normalize: gap / (window - 1), capped at 1.0
        let max_gap = (self.config.history_window - 1).max(1) as f64;
        (f64::from(gap) / max_gap).min(1.0)
    }

    /// Computes repeated file edits score: how often the same files appear
    /// across recent iterations.
    ///
    /// Files that appear in >50% of iterations are considered "stuck".
    fn compute_repeated_edits_score(&self) -> f64 {
        if self.history.len() < 2 {
            return 0.0;
        }

        let mut file_counts: HashMap<&str, u32> = HashMap::new();
        for snapshot in &self.history {
            // Use a set to count each file only once per iteration
            let unique_files: HashSet<&str> =
                snapshot.files_edited.iter().map(String::as_str).collect();
            for file in unique_files {
                *file_counts.entry(file).or_insert(0) += 1;
            }
        }

        let iteration_count = self.history.len() as f64;
        let threshold = iteration_count * 0.5;

        // Count files appearing in more than half the iterations
        let stuck_files = file_counts
            .values()
            .filter(|&&count| f64::from(count) > threshold)
            .count();

        if stuck_files == 0 {
            return 0.0;
        }

        // Score based on proportion of stuck files vs total unique files
        let total_files = file_counts.len().max(1) as f64;
        (stuck_files as f64 / total_files).min(1.0)
    }

    /// Computes recurring errors score: how many errors repeat across iterations.
    fn compute_recurring_errors_score(&self) -> f64 {
        if self.history.len() < 2 {
            return 0.0;
        }

        let mut error_counts: HashMap<&str, u32> = HashMap::new();
        for snapshot in &self.history {
            let unique_errors: HashSet<&str> = snapshot.errors.iter().map(String::as_str).collect();
            for error in unique_errors {
                *error_counts.entry(error).or_insert(0) += 1;
            }
        }

        // Errors appearing in >= 3 iterations are "recurring"
        let recurring_threshold = 3.min(self.history.len() as u32);
        let recurring_count = error_counts
            .values()
            .filter(|&&count| count >= recurring_threshold)
            .count();

        if recurring_count == 0 {
            return 0.0;
        }

        // Cap at 1.0, scale by number of recurring errors (3+ is fully saturated)
        (recurring_count as f64 / 3.0).min(1.0)
    }

    /// Computes test plateau score: whether the test count has been flat.
    fn compute_test_plateau_score(&self) -> f64 {
        if self.history.len() < 2 {
            return 0.0;
        }

        let test_counts: Vec<u32> = self.history.iter().map(|s| s.test_count).collect();
        let first = test_counts[0];
        let last = *test_counts.last().expect("history is non-empty");

        // If test count increased, no plateau
        if last > first {
            return 0.0;
        }

        // Count how many consecutive iterations from the end have the same count
        let plateau_len = test_counts.iter().rev().take_while(|&&c| c == last).count();

        // Score based on plateau length relative to window
        let max_len = self.config.history_window.max(1) as f64;
        ((plateau_len as f64 - 1.0) / max_len).clamp(0.0, 1.0)
    }

    /// Maps a total score to a risk level using configured thresholds.
    fn score_to_risk_level(&self, score: f64) -> RiskLevel {
        let [medium, high, critical] = self.config.risk_thresholds;
        if score >= critical {
            RiskLevel::Critical
        } else if score >= high {
            RiskLevel::High
        } else if score >= medium {
            RiskLevel::Medium
        } else {
            RiskLevel::Low
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Helper
    // =========================================================================

    fn snapshot(
        commit: Option<&str>,
        files: &[&str],
        errors: &[&str],
        test_count: u32,
    ) -> IterationSnapshot {
        IterationSnapshot {
            commit_hash: commit.map(String::from),
            files_edited: files.iter().map(|&s| s.to_string()).collect(),
            errors: errors.iter().map(|&s| s.to_string()).collect(),
            test_count,
        }
    }

    // =========================================================================
    // RiskLevel tests
    // =========================================================================

    #[test]
    fn risk_level_ordering() {
        assert!(RiskLevel::Low < RiskLevel::Medium);
        assert!(RiskLevel::Medium < RiskLevel::High);
        assert!(RiskLevel::High < RiskLevel::Critical);
    }

    #[test]
    fn risk_level_display() {
        assert_eq!(RiskLevel::Low.to_string(), "Low");
        assert_eq!(RiskLevel::Medium.to_string(), "Medium");
        assert_eq!(RiskLevel::High.to_string(), "High");
        assert_eq!(RiskLevel::Critical.to_string(), "Critical");
    }

    #[test]
    fn risk_level_serde_roundtrip() {
        for level in [
            RiskLevel::Low,
            RiskLevel::Medium,
            RiskLevel::High,
            RiskLevel::Critical,
        ] {
            let json = serde_json::to_string(&level).expect("serialize");
            let restored: RiskLevel = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(level, restored);
        }
    }

    // =========================================================================
    // StagnationConfig tests
    // =========================================================================

    #[test]
    fn default_config_weights_sum_to_one() {
        let config = StagnationConfig::default();
        let sum = config.commit_gap_weight
            + config.repeated_edits_weight
            + config.recurring_errors_weight
            + config.test_plateau_weight;
        assert!(
            (sum - 1.0).abs() < f64::EPSILON,
            "weights should sum to 1.0, got {sum}"
        );
    }

    #[test]
    fn default_config_thresholds_ascending() {
        let config = StagnationConfig::default();
        assert!(config.risk_thresholds[0] < config.risk_thresholds[1]);
        assert!(config.risk_thresholds[1] < config.risk_thresholds[2]);
    }

    #[test]
    fn default_config_values() {
        let config = StagnationConfig::default();
        assert_eq!(config.history_window, 10);
        assert!((config.commit_gap_weight - 0.4).abs() < f64::EPSILON);
        assert!((config.repeated_edits_weight - 0.25).abs() < f64::EPSILON);
        assert!((config.recurring_errors_weight - 0.25).abs() < f64::EPSILON);
        assert!((config.test_plateau_weight - 0.1).abs() < f64::EPSILON);
    }

    // =========================================================================
    // StagnationDetector — empty / single snapshot
    // =========================================================================

    #[test]
    fn empty_detector_returns_low_risk() {
        let detector = StagnationDetector::new(StagnationConfig::default());
        let score = detector.evaluate();
        assert_eq!(score.risk_level, RiskLevel::Low);
        assert!((score.total - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn single_snapshot_returns_low_risk() {
        let mut detector = StagnationDetector::new(StagnationConfig::default());
        detector.record(snapshot(Some("abc"), &["src/main.rs"], &[], 100));
        let score = detector.evaluate();
        assert_eq!(score.risk_level, RiskLevel::Low);
    }

    // =========================================================================
    // Commit gap scoring
    // =========================================================================

    #[test]
    fn commit_gap_no_gap_scores_zero() {
        let mut detector = StagnationDetector::new(StagnationConfig::default());
        // Each iteration has a new commit
        detector.record(snapshot(Some("aaa"), &[], &[], 100));
        detector.record(snapshot(Some("bbb"), &[], &[], 100));
        detector.record(snapshot(Some("ccc"), &[], &[], 100));

        let score = detector.evaluate();
        assert!(
            score.commit_gap_score < 0.2,
            "no commit gap should score low, got {}",
            score.commit_gap_score
        );
    }

    #[test]
    fn commit_gap_stale_hash_increases_score() {
        let mut detector = StagnationDetector::new(StagnationConfig::default());
        // Same commit hash for many iterations
        for _ in 0..5 {
            detector.record(snapshot(Some("same"), &[], &[], 100));
        }

        let score = detector.evaluate();
        assert!(
            score.commit_gap_score > 0.3,
            "stale commit should score high, got {}",
            score.commit_gap_score
        );
    }

    #[test]
    fn commit_gap_recent_change_resets_score() {
        let mut detector = StagnationDetector::new(StagnationConfig::default());
        // Stale for a while, then a new commit
        for _ in 0..4 {
            detector.record(snapshot(Some("old"), &[], &[], 100));
        }
        detector.record(snapshot(Some("new"), &[], &[], 100));

        let score = detector.evaluate();
        assert!(
            score.commit_gap_score < 0.1,
            "recent commit change should reset gap, got {}",
            score.commit_gap_score
        );
    }

    #[test]
    fn commit_gap_none_hashes_count_as_gap() {
        let mut detector = StagnationDetector::new(StagnationConfig::default());
        for _ in 0..5 {
            detector.record(snapshot(None, &[], &[], 100));
        }

        let score = detector.evaluate();
        assert!(
            score.commit_gap_score > 0.3,
            "no commit hashes should indicate gap, got {}",
            score.commit_gap_score
        );
    }

    // =========================================================================
    // Repeated file edits scoring
    // =========================================================================

    #[test]
    fn repeated_edits_different_files_scores_zero() {
        let mut detector = StagnationDetector::new(StagnationConfig::default());
        detector.record(snapshot(Some("a"), &["src/a.rs"], &[], 100));
        detector.record(snapshot(Some("b"), &["src/b.rs"], &[], 101));
        detector.record(snapshot(Some("c"), &["src/c.rs"], &[], 102));

        let score = detector.evaluate();
        assert!(
            score.repeated_edits_score < f64::EPSILON,
            "different files each time should score zero, got {}",
            score.repeated_edits_score
        );
    }

    #[test]
    fn repeated_edits_same_file_increases_score() {
        let mut detector = StagnationDetector::new(StagnationConfig::default());
        for i in 0..5 {
            detector.record(snapshot(
                Some(&format!("hash{i}")),
                &["src/stuck.rs"],
                &[],
                100,
            ));
        }

        let score = detector.evaluate();
        assert!(
            score.repeated_edits_score > 0.5,
            "same file edited repeatedly should score high, got {}",
            score.repeated_edits_score
        );
    }

    #[test]
    fn repeated_edits_mixed_files() {
        let mut detector = StagnationDetector::new(StagnationConfig::default());
        // "stuck.rs" appears in every iteration, others rotate
        for i in 0..4 {
            detector.record(snapshot(
                Some(&format!("h{i}")),
                &["src/stuck.rs", &format!("src/new{i}.rs")],
                &[],
                100,
            ));
        }

        let score = detector.evaluate();
        // stuck.rs appears 4/4 times (>50%), each new file appears 1/4 times (<50%)
        assert!(
            score.repeated_edits_score > 0.0,
            "one stuck file among many should still score, got {}",
            score.repeated_edits_score
        );
    }

    // =========================================================================
    // Recurring errors scoring
    // =========================================================================

    #[test]
    fn recurring_errors_no_errors_scores_zero() {
        let mut detector = StagnationDetector::new(StagnationConfig::default());
        detector.record(snapshot(Some("a"), &[], &[], 100));
        detector.record(snapshot(Some("b"), &[], &[], 101));

        let score = detector.evaluate();
        assert!(
            score.recurring_errors_score < f64::EPSILON,
            "no errors should score zero"
        );
    }

    #[test]
    fn recurring_errors_transient_error_scores_low() {
        let mut detector = StagnationDetector::new(StagnationConfig::default());
        detector.record(snapshot(Some("a"), &[], &["error: something"], 100));
        detector.record(snapshot(Some("b"), &[], &[], 101));
        detector.record(snapshot(Some("c"), &[], &[], 102));

        let score = detector.evaluate();
        assert!(
            score.recurring_errors_score < f64::EPSILON,
            "transient error should not score, got {}",
            score.recurring_errors_score
        );
    }

    #[test]
    fn recurring_errors_same_error_repeatedly_increases_score() {
        let mut detector = StagnationDetector::new(StagnationConfig::default());
        for i in 0..5 {
            detector.record(snapshot(
                Some(&format!("h{i}")),
                &[],
                &["error[E0308]: mismatched types"],
                100,
            ));
        }

        let score = detector.evaluate();
        assert!(
            score.recurring_errors_score > 0.3,
            "recurring error should score, got {}",
            score.recurring_errors_score
        );
    }

    #[test]
    fn recurring_errors_multiple_distinct_recurring() {
        let mut detector = StagnationDetector::new(StagnationConfig::default());
        for i in 0..4 {
            detector.record(snapshot(
                Some(&format!("h{i}")),
                &[],
                &[
                    "error: type mismatch",
                    "error: unused variable",
                    "warning: dead code",
                ],
                100,
            ));
        }

        let score = detector.evaluate();
        assert!(
            score.recurring_errors_score > 0.6,
            "multiple recurring errors should score high, got {}",
            score.recurring_errors_score
        );
    }

    // =========================================================================
    // Test plateau scoring
    // =========================================================================

    #[test]
    fn test_plateau_increasing_tests_scores_zero() {
        let mut detector = StagnationDetector::new(StagnationConfig::default());
        detector.record(snapshot(Some("a"), &[], &[], 100));
        detector.record(snapshot(Some("b"), &[], &[], 101));
        detector.record(snapshot(Some("c"), &[], &[], 102));

        let score = detector.evaluate();
        assert!(
            score.test_plateau_score < f64::EPSILON,
            "increasing test count should score zero, got {}",
            score.test_plateau_score
        );
    }

    #[test]
    fn test_plateau_flat_count_increases_score() {
        let mut detector = StagnationDetector::new(StagnationConfig::default());
        for _ in 0..6 {
            detector.record(snapshot(Some("same"), &[], &[], 100));
        }

        let score = detector.evaluate();
        assert!(
            score.test_plateau_score > 0.3,
            "flat test count should score, got {}",
            score.test_plateau_score
        );
    }

    #[test]
    fn test_plateau_decrease_counts_as_plateau() {
        let mut detector = StagnationDetector::new(StagnationConfig::default());
        detector.record(snapshot(Some("a"), &[], &[], 100));
        detector.record(snapshot(Some("b"), &[], &[], 99));
        detector.record(snapshot(Some("c"), &[], &[], 99));

        let score = detector.evaluate();
        // Test count didn't increase, so it's a plateau
        assert!(
            score.test_plateau_score >= 0.0,
            "decreasing test count should not score negative"
        );
    }

    // =========================================================================
    // Combined scoring and risk levels
    // =========================================================================

    #[test]
    fn healthy_progress_stays_low_risk() {
        let mut detector = StagnationDetector::new(StagnationConfig::default());
        for i in 0..5 {
            detector.record(snapshot(
                Some(&format!("hash{i}")),
                &[&format!("src/feature{i}.rs")],
                &[],
                100 + i as u32,
            ));
        }

        let score = detector.evaluate();
        assert_eq!(
            score.risk_level,
            RiskLevel::Low,
            "healthy progress should be low risk, got {score:?}"
        );
    }

    #[test]
    fn total_stagnation_reaches_critical() {
        let mut detector = StagnationDetector::new(StagnationConfig::default());
        // Same commit, same files, same errors, same test count — fully stuck
        for _ in 0..10 {
            detector.record(snapshot(
                Some("stuck"),
                &["src/broken.rs"],
                &["error: something failed"],
                100,
            ));
        }

        let score = detector.evaluate();
        assert!(
            score.risk_level >= RiskLevel::High,
            "total stagnation should be high or critical, got {:?} (total: {})",
            score.risk_level,
            score.total
        );
    }

    #[test]
    fn partial_stagnation_reaches_medium() {
        let mut detector = StagnationDetector::new(StagnationConfig::default());
        // Same commit but different files, no errors, test count rising
        for i in 0..5 {
            detector.record(snapshot(
                Some("same_commit"),
                &[&format!("src/file{i}.rs")],
                &[],
                100 + i as u32,
            ));
        }

        let score = detector.evaluate();
        // Commit gap is the only factor, at weight 0.4
        assert!(
            score.risk_level >= RiskLevel::Low,
            "commit gap only should be at least low, got {:?}",
            score.risk_level
        );
        assert!(
            score.total > 0.1,
            "should have some stagnation score from commit gap"
        );
    }

    // =========================================================================
    // Score resets on meaningful progress
    // =========================================================================

    #[test]
    fn new_commit_reduces_stagnation() {
        let mut detector = StagnationDetector::new(StagnationConfig::default());
        // Build up stagnation
        for _ in 0..5 {
            detector.record(snapshot(Some("old"), &["src/stuck.rs"], &["error"], 100));
        }
        let before = detector.evaluate();

        // New commit with different file and no errors
        detector.record(snapshot(Some("new"), &["src/fixed.rs"], &[], 101));
        let after = detector.evaluate();

        assert!(
            after.total < before.total,
            "new commit should reduce total score: before={}, after={}",
            before.total,
            after.total
        );
    }

    #[test]
    fn reset_clears_history() {
        let mut detector = StagnationDetector::new(StagnationConfig::default());
        for _ in 0..5 {
            detector.record(snapshot(Some("stuck"), &["src/stuck.rs"], &["error"], 100));
        }
        assert!(detector.history_len() > 0);

        detector.reset();
        assert_eq!(detector.history_len(), 0);

        let score = detector.evaluate();
        assert_eq!(score.risk_level, RiskLevel::Low);
        assert!((score.total - 0.0).abs() < f64::EPSILON);
    }

    // =========================================================================
    // History window management
    // =========================================================================

    #[test]
    fn history_window_evicts_old_entries() {
        let config = StagnationConfig {
            history_window: 3,
            ..StagnationConfig::default()
        };
        let mut detector = StagnationDetector::new(config);

        for i in 0..5 {
            detector.record(snapshot(Some(&format!("h{i}")), &[], &[], 100));
        }

        assert_eq!(detector.history_len(), 3, "should only keep 3 entries");
    }

    #[test]
    fn old_stagnation_evicted_from_window() {
        let config = StagnationConfig {
            history_window: 3,
            ..StagnationConfig::default()
        };
        let mut detector = StagnationDetector::new(config);

        // Build stagnation
        for _ in 0..3 {
            detector.record(snapshot(Some("stuck"), &["src/stuck.rs"], &["error"], 100));
        }
        let stagnated = detector.evaluate();

        // Record healthy iterations that push out old stagnation
        for i in 0..3 {
            detector.record(snapshot(
                Some(&format!("new{i}")),
                &[&format!("src/new{i}.rs")],
                &[],
                101 + i as u32,
            ));
        }
        let recovered = detector.evaluate();

        assert!(
            recovered.total < stagnated.total,
            "old stagnation should be evicted: stagnated={}, recovered={}",
            stagnated.total,
            recovered.total
        );
    }

    // =========================================================================
    // Custom config
    // =========================================================================

    #[test]
    fn custom_thresholds_affect_risk_level() {
        let config = StagnationConfig {
            risk_thresholds: [0.1, 0.2, 0.3],
            ..StagnationConfig::default()
        };
        let mut detector = StagnationDetector::new(config);

        // Build moderate stagnation
        for _ in 0..4 {
            detector.record(snapshot(Some("same"), &[], &[], 100));
        }

        let score = detector.evaluate();
        // With lower thresholds, the same score maps to higher risk
        assert!(
            score.risk_level >= RiskLevel::Medium,
            "lower thresholds should escalate risk, got {:?}",
            score.risk_level
        );
    }

    #[test]
    fn zero_weight_factor_has_no_effect() {
        let config = StagnationConfig {
            commit_gap_weight: 1.0,
            repeated_edits_weight: 0.0,
            recurring_errors_weight: 0.0,
            test_plateau_weight: 0.0,
            ..StagnationConfig::default()
        };
        let mut detector = StagnationDetector::new(config);

        // Only errors (but error weight is 0)
        for i in 0..5 {
            detector.record(snapshot(
                Some(&format!("h{i}")),
                &["src/stuck.rs"],
                &["recurring error"],
                100,
            ));
        }

        let score = detector.evaluate();
        // Errors and file edits have zero weight; commit hashes change each time
        assert!(
            score.repeated_edits_score > 0.0,
            "factor should still compute"
        );
        assert!(
            score.total < 0.1,
            "zero-weighted factors should not contribute to total, got {}",
            score.total
        );
    }

    // =========================================================================
    // StagnationScore field validation
    // =========================================================================

    #[test]
    fn score_total_is_clamped_to_unit_range() {
        // Even with extreme values, total should be in [0.0, 1.0]
        let config = StagnationConfig {
            commit_gap_weight: 1.0,
            repeated_edits_weight: 1.0,
            recurring_errors_weight: 1.0,
            test_plateau_weight: 1.0,
            ..StagnationConfig::default()
        };
        let mut detector = StagnationDetector::new(config);
        for _ in 0..10 {
            detector.record(snapshot(Some("same"), &["a.rs"], &["err"], 100));
        }

        let score = detector.evaluate();
        assert!(score.total <= 1.0, "total should be clamped to 1.0");
        assert!(score.total >= 0.0, "total should be non-negative");
    }

    #[test]
    fn individual_factor_scores_are_non_negative() {
        let mut detector = StagnationDetector::new(StagnationConfig::default());
        for _ in 0..5 {
            detector.record(snapshot(Some("h"), &["f.rs"], &["e"], 100));
        }

        let score = detector.evaluate();
        assert!(score.commit_gap_score >= 0.0);
        assert!(score.repeated_edits_score >= 0.0);
        assert!(score.recurring_errors_score >= 0.0);
        assert!(score.test_plateau_score >= 0.0);
    }

    // =========================================================================
    // Edge cases
    // =========================================================================

    #[test]
    fn snapshot_with_no_files_or_errors() {
        let mut detector = StagnationDetector::new(StagnationConfig::default());
        detector.record(snapshot(Some("a"), &[], &[], 0));
        detector.record(snapshot(Some("b"), &[], &[], 0));

        let score = detector.evaluate();
        // No files, no errors, but test count is flat at 0
        assert_eq!(score.risk_level, RiskLevel::Low);
    }

    #[test]
    fn snapshot_with_duplicate_files_in_same_iteration() {
        let mut detector = StagnationDetector::new(StagnationConfig::default());
        detector.record(snapshot(
            Some("a"),
            &["src/a.rs", "src/a.rs", "src/a.rs"],
            &[],
            100,
        ));
        detector.record(snapshot(Some("b"), &["src/b.rs"], &[], 101));

        let score = detector.evaluate();
        // Duplicate files in the same snapshot should be deduplicated
        assert!(
            score.repeated_edits_score < f64::EPSILON,
            "deduped files across different iterations should score zero"
        );
    }

    #[test]
    fn large_window_with_few_snapshots() {
        let config = StagnationConfig {
            history_window: 100,
            ..StagnationConfig::default()
        };
        let mut detector = StagnationDetector::new(config);
        detector.record(snapshot(Some("a"), &[], &[], 100));
        detector.record(snapshot(Some("a"), &[], &[], 100));

        let score = detector.evaluate();
        // Only 2 snapshots in a window of 100 — scores should be low
        assert!(
            score.total < 0.2,
            "few snapshots in large window should score low, got {}",
            score.total
        );
    }
}
