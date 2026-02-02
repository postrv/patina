//! Continuous coding event types.
//!
//! Events emitted during continuous coding sessions for plugin hooks and monitoring.
//!
//! # Event Types (Task 2.4.2 will implement)
//!
//! - `IterationStart` - Emitted when a new iteration begins
//! - `IterationComplete` - Emitted when an iteration finishes
//! - `QualityGateCheck` - Emitted before running a quality gate
//! - `QualityGateResult` - Emitted with the result of a quality gate
//! - `StagnationDetected` - Emitted when no progress is detected
//! - `HumanCheckpointRequired` - Emitted when human intervention is needed

// NOTE: Task 2.4.1 (RED) - Tests below document expected behavior
// Task 2.4.2 (GREEN) will add serde derives and impl methods to make tests pass

/// Events emitted during continuous coding sessions.
///
/// These events are sent to plugins via the `ContinuousCodingPlugin::on_event` method
/// and can be used for logging, metrics, or custom automation logic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContinuousEvent {
    /// A new iteration is starting.
    IterationStart {
        /// The iteration number (1-indexed).
        iteration: u32,
    },

    /// An iteration has completed.
    IterationComplete {
        /// The iteration number that completed.
        iteration: u32,
        /// Duration of the iteration in milliseconds.
        duration_ms: u64,
    },

    /// A quality gate check is about to run.
    QualityGateCheck {
        /// Name of the quality gate being checked.
        gate: String,
    },

    /// Result of a quality gate check.
    QualityGateResult {
        /// Name of the quality gate that was checked.
        gate: String,
        /// Whether the gate passed.
        passed: bool,
        /// Optional message with details.
        message: Option<String>,
    },

    /// Stagnation has been detected (no progress for N iterations).
    StagnationDetected {
        /// Number of iterations without progress.
        iterations_without_progress: u32,
        /// The stagnation threshold that was exceeded.
        threshold: u32,
    },

    /// Human intervention is required.
    HumanCheckpointRequired {
        /// Reason why human intervention is needed.
        reason: String,
    },
}

// NOTE: impl block with event_type(), is_iteration_event(), requires_attention()
// will be added in Task 2.4.2 (GREEN) to make tests pass

#[cfg(test)]
mod tests {
    use super::*;

    // =============================================================================
    // ContinuousEvent variant tests (Task 2.4.1)
    // These tests document the expected variants and their structure
    // =============================================================================

    #[test]
    fn test_continuous_event_variants_iteration_start() {
        let event = ContinuousEvent::IterationStart { iteration: 1 };
        assert_eq!(event.event_type(), "iteration_start");
        assert!(event.is_iteration_event());
        assert!(!event.requires_attention());
    }

    #[test]
    fn test_continuous_event_variants_iteration_complete() {
        let event = ContinuousEvent::IterationComplete {
            iteration: 5,
            duration_ms: 1234,
        };
        assert_eq!(event.event_type(), "iteration_complete");
        assert!(event.is_iteration_event());
        assert!(!event.requires_attention());
    }

    #[test]
    fn test_continuous_event_variants_quality_gate_check() {
        let event = ContinuousEvent::QualityGateCheck {
            gate: "clippy".to_string(),
        };
        assert_eq!(event.event_type(), "quality_gate_check");
        assert!(!event.is_iteration_event());
        assert!(!event.requires_attention());
    }

    #[test]
    fn test_continuous_event_variants_quality_gate_result_passed() {
        let event = ContinuousEvent::QualityGateResult {
            gate: "tests".to_string(),
            passed: true,
            message: Some("All 100 tests passed".to_string()),
        };
        assert_eq!(event.event_type(), "quality_gate_result");
        assert!(!event.requires_attention());
    }

    #[test]
    fn test_continuous_event_variants_quality_gate_result_failed() {
        let event = ContinuousEvent::QualityGateResult {
            gate: "clippy".to_string(),
            passed: false,
            message: Some("3 warnings found".to_string()),
        };
        assert_eq!(event.event_type(), "quality_gate_result");
        assert!(event.requires_attention());
    }

    #[test]
    fn test_continuous_event_variants_stagnation_detected() {
        let event = ContinuousEvent::StagnationDetected {
            iterations_without_progress: 5,
            threshold: 3,
        };
        assert_eq!(event.event_type(), "stagnation_detected");
        assert!(event.requires_attention());
    }

    #[test]
    fn test_continuous_event_variants_human_checkpoint_required() {
        let event = ContinuousEvent::HumanCheckpointRequired {
            reason: "Unable to resolve merge conflict".to_string(),
        };
        assert_eq!(event.event_type(), "human_checkpoint_required");
        assert!(event.requires_attention());
    }

    // =============================================================================
    // ContinuousEvent serialization tests (Task 2.4.1)
    // These tests document the expected JSON serialization format
    // =============================================================================

    #[test]
    fn test_continuous_event_serialize_iteration_start() {
        let event = ContinuousEvent::IterationStart { iteration: 1 };
        let json = serde_json::to_string(&event).expect("serialization should succeed");
        assert!(json.contains(r#""type":"iteration_start""#));
        assert!(json.contains(r#""iteration":1"#));
    }

    #[test]
    fn test_continuous_event_serialize_iteration_complete() {
        let event = ContinuousEvent::IterationComplete {
            iteration: 3,
            duration_ms: 5000,
        };
        let json = serde_json::to_string(&event).expect("serialization should succeed");
        assert!(json.contains(r#""type":"iteration_complete""#));
        assert!(json.contains(r#""iteration":3"#));
        assert!(json.contains(r#""duration_ms":5000"#));
    }

    #[test]
    fn test_continuous_event_serialize_quality_gate_result() {
        let event = ContinuousEvent::QualityGateResult {
            gate: "tests".to_string(),
            passed: true,
            message: None,
        };
        let json = serde_json::to_string(&event).expect("serialization should succeed");
        assert!(json.contains(r#""type":"quality_gate_result""#));
        assert!(json.contains(r#""gate":"tests""#));
        assert!(json.contains(r#""passed":true"#));
    }

    #[test]
    fn test_continuous_event_deserialize_iteration_start() {
        let json = r#"{"type":"iteration_start","iteration":42}"#;
        let event: ContinuousEvent =
            serde_json::from_str(json).expect("deserialization should succeed");
        assert!(matches!(
            event,
            ContinuousEvent::IterationStart { iteration: 42 }
        ));
    }

    #[test]
    fn test_continuous_event_deserialize_stagnation() {
        let json = r#"{"type":"stagnation_detected","iterations_without_progress":5,"threshold":3}"#;
        let event: ContinuousEvent =
            serde_json::from_str(json).expect("deserialization should succeed");
        assert!(matches!(
            event,
            ContinuousEvent::StagnationDetected {
                iterations_without_progress: 5,
                threshold: 3
            }
        ));
    }

    #[test]
    fn test_continuous_event_roundtrip() {
        let events = vec![
            ContinuousEvent::IterationStart { iteration: 1 },
            ContinuousEvent::IterationComplete {
                iteration: 1,
                duration_ms: 1000,
            },
            ContinuousEvent::QualityGateCheck {
                gate: "clippy".to_string(),
            },
            ContinuousEvent::QualityGateResult {
                gate: "clippy".to_string(),
                passed: true,
                message: Some("0 warnings".to_string()),
            },
            ContinuousEvent::StagnationDetected {
                iterations_without_progress: 3,
                threshold: 3,
            },
            ContinuousEvent::HumanCheckpointRequired {
                reason: "Test failure".to_string(),
            },
        ];

        for event in events {
            let json = serde_json::to_string(&event).expect("serialize");
            let restored: ContinuousEvent = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(event, restored);
        }
    }

    // =============================================================================
    // ContinuousEvent trait tests (Task 2.4.1)
    // These tests verify Clone, PartialEq, Eq, Debug traits work correctly
    // =============================================================================

    #[test]
    fn test_continuous_event_equality() {
        assert_eq!(
            ContinuousEvent::IterationStart { iteration: 1 },
            ContinuousEvent::IterationStart { iteration: 1 }
        );
        assert_ne!(
            ContinuousEvent::IterationStart { iteration: 1 },
            ContinuousEvent::IterationStart { iteration: 2 }
        );
        assert_ne!(
            ContinuousEvent::IterationStart { iteration: 1 },
            ContinuousEvent::IterationComplete {
                iteration: 1,
                duration_ms: 0
            }
        );
    }

    #[test]
    fn test_continuous_event_clone() {
        let event = ContinuousEvent::HumanCheckpointRequired {
            reason: "Test".to_string(),
        };
        let cloned = event.clone();
        assert_eq!(event, cloned);
    }

    #[test]
    fn test_continuous_event_debug() {
        let event = ContinuousEvent::IterationStart { iteration: 1 };
        let debug = format!("{event:?}");
        assert!(debug.contains("IterationStart"));
        assert!(debug.contains("iteration: 1"));
    }
}
