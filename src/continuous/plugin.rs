//! Continuous coding plugin trait and quality gates.
//!
//! This module defines the plugin interface for continuous coding automation,
//! allowing custom plugins to hook into the iteration loop and define quality gates.
//!
//! # Quality Gates
//!
//! Quality gates are checks that must pass before an iteration is considered successful:
//! - `TestsPass` - All tests must pass
//! - `ClippyClean` - No clippy warnings
//! - `SecurityScan` - No critical/high security findings
//!
//! # Plugin Trait
//!
//! Implement `ContinuousCodingPlugin` to create custom automation plugins.

use super::ContinuousEvent;

// NOTE: Task 2.4.3 (RED) - Tests below document expected behavior
// Task 2.4.4 (GREEN) will complete the implementation

/// Quality gates that must pass for an iteration to succeed.
///
/// Each gate represents a specific check that the automation loop can verify.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QualityGate {
    /// All tests must pass (`cargo test`).
    TestsPass,

    /// No clippy warnings (`cargo clippy -- -D warnings`).
    ClippyClean,

    /// No critical or high severity security findings.
    SecurityScan,

    /// Code must be formatted (`cargo fmt --check`).
    FormatCheck,
}

/// Trait for continuous coding automation plugins.
///
/// Implement this trait to hook into the continuous coding loop and customize
/// behavior such as quality gates, iteration limits, and event handling.
///
/// # Example
///
/// ```ignore
/// use patina::continuous::{ContinuousCodingPlugin, ContinuousEvent, QualityGate};
///
/// struct MyPlugin;
///
/// impl ContinuousCodingPlugin for MyPlugin {
///     fn on_event(&mut self, event: &ContinuousEvent) {
///         println!("Event: {:?}", event);
///     }
///
///     fn quality_gates(&self) -> Vec<QualityGate> {
///         vec![QualityGate::TestsPass, QualityGate::ClippyClean]
///     }
/// }
/// ```
pub trait ContinuousCodingPlugin: Send + Sync {
    /// Called when a continuous coding event occurs.
    ///
    /// Use this to log events, update metrics, or trigger custom behavior.
    fn on_event(&mut self, event: &ContinuousEvent);

    /// Returns the list of quality gates that must pass each iteration.
    ///
    /// Defaults to tests and clippy checks.
    fn quality_gates(&self) -> Vec<QualityGate> {
        vec![QualityGate::TestsPass, QualityGate::ClippyClean]
    }

    /// Returns the maximum number of iterations before stopping.
    ///
    /// Defaults to 50 iterations.
    fn max_iterations(&self) -> u32 {
        50
    }

    /// Returns the number of iterations without progress before stagnation is detected.
    ///
    /// Defaults to 3 iterations.
    fn stagnation_threshold(&self) -> u32 {
        3
    }

    /// Returns the plugin name for identification in logs.
    fn name(&self) -> &str {
        "default"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =============================================================================
    // QualityGate enum tests (Task 2.4.3)
    // =============================================================================

    #[test]
    fn test_quality_gate_variants() {
        // Test all variants exist and can be matched
        let gates = [
            QualityGate::TestsPass,
            QualityGate::ClippyClean,
            QualityGate::SecurityScan,
            QualityGate::FormatCheck,
        ];

        for gate in gates {
            match gate {
                QualityGate::TestsPass => assert_eq!(gate.name(), "tests_pass"),
                QualityGate::ClippyClean => assert_eq!(gate.name(), "clippy_clean"),
                QualityGate::SecurityScan => assert_eq!(gate.name(), "security_scan"),
                QualityGate::FormatCheck => assert_eq!(gate.name(), "format_check"),
            }
        }
    }

    #[test]
    fn test_quality_gate_display() {
        assert_eq!(format!("{}", QualityGate::TestsPass), "Tests Pass");
        assert_eq!(format!("{}", QualityGate::ClippyClean), "Clippy Clean");
        assert_eq!(format!("{}", QualityGate::SecurityScan), "Security Scan");
        assert_eq!(format!("{}", QualityGate::FormatCheck), "Format Check");
    }

    #[test]
    fn test_quality_gate_command() {
        assert_eq!(QualityGate::TestsPass.command(), "cargo test");
        assert_eq!(
            QualityGate::ClippyClean.command(),
            "cargo clippy --all-targets -- -D warnings"
        );
        assert_eq!(QualityGate::SecurityScan.command(), "scan_security");
        assert_eq!(QualityGate::FormatCheck.command(), "cargo fmt -- --check");
    }

    #[test]
    fn test_quality_gate_equality() {
        assert_eq!(QualityGate::TestsPass, QualityGate::TestsPass);
        assert_ne!(QualityGate::TestsPass, QualityGate::ClippyClean);
    }

    #[test]
    fn test_quality_gate_clone() {
        let gate = QualityGate::SecurityScan;
        let cloned = gate;
        assert_eq!(gate, cloned);
    }

    #[test]
    fn test_quality_gate_debug() {
        let debug = format!("{:?}", QualityGate::TestsPass);
        assert!(debug.contains("TestsPass"));
    }

    #[test]
    fn test_quality_gate_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(QualityGate::TestsPass);
        set.insert(QualityGate::ClippyClean);
        set.insert(QualityGate::TestsPass); // duplicate

        assert_eq!(set.len(), 2);
        assert!(set.contains(&QualityGate::TestsPass));
        assert!(set.contains(&QualityGate::ClippyClean));
    }

    // =============================================================================
    // ContinuousCodingPlugin trait tests (Task 2.4.3)
    // =============================================================================

    /// Test plugin implementation for verifying trait behavior
    struct TestPlugin {
        events: Vec<String>,
        custom_gates: Option<Vec<QualityGate>>,
        custom_max: Option<u32>,
        custom_threshold: Option<u32>,
    }

    impl TestPlugin {
        fn new() -> Self {
            Self {
                events: Vec::new(),
                custom_gates: None,
                custom_max: None,
                custom_threshold: None,
            }
        }

        fn with_gates(mut self, gates: Vec<QualityGate>) -> Self {
            self.custom_gates = Some(gates);
            self
        }

        fn with_max_iterations(mut self, max: u32) -> Self {
            self.custom_max = Some(max);
            self
        }

        fn with_stagnation_threshold(mut self, threshold: u32) -> Self {
            self.custom_threshold = Some(threshold);
            self
        }
    }

    impl ContinuousCodingPlugin for TestPlugin {
        fn on_event(&mut self, event: &ContinuousEvent) {
            self.events.push(event.event_type().to_string());
        }

        fn quality_gates(&self) -> Vec<QualityGate> {
            self.custom_gates
                .clone()
                .unwrap_or_else(|| vec![QualityGate::TestsPass, QualityGate::ClippyClean])
        }

        fn max_iterations(&self) -> u32 {
            self.custom_max.unwrap_or(50)
        }

        fn stagnation_threshold(&self) -> u32 {
            self.custom_threshold.unwrap_or(3)
        }

        fn name(&self) -> &str {
            "test_plugin"
        }
    }

    #[test]
    fn test_plugin_on_event() {
        let mut plugin = TestPlugin::new();

        plugin.on_event(&ContinuousEvent::IterationStart { iteration: 1 });
        plugin.on_event(&ContinuousEvent::IterationComplete {
            iteration: 1,
            duration_ms: 1000,
        });

        assert_eq!(plugin.events.len(), 2);
        assert_eq!(plugin.events[0], "iteration_start");
        assert_eq!(plugin.events[1], "iteration_complete");
    }

    #[test]
    fn test_plugin_default_quality_gates() {
        let plugin = TestPlugin::new();
        let gates = plugin.quality_gates();

        assert_eq!(gates.len(), 2);
        assert!(gates.contains(&QualityGate::TestsPass));
        assert!(gates.contains(&QualityGate::ClippyClean));
    }

    #[test]
    fn test_plugin_custom_quality_gates() {
        let plugin = TestPlugin::new().with_gates(vec![
            QualityGate::TestsPass,
            QualityGate::ClippyClean,
            QualityGate::SecurityScan,
            QualityGate::FormatCheck,
        ]);

        let gates = plugin.quality_gates();
        assert_eq!(gates.len(), 4);
        assert!(gates.contains(&QualityGate::SecurityScan));
    }

    #[test]
    fn test_plugin_default_max_iterations() {
        let plugin = TestPlugin::new();
        assert_eq!(plugin.max_iterations(), 50);
    }

    #[test]
    fn test_plugin_custom_max_iterations() {
        let plugin = TestPlugin::new().with_max_iterations(100);
        assert_eq!(plugin.max_iterations(), 100);
    }

    #[test]
    fn test_plugin_default_stagnation_threshold() {
        let plugin = TestPlugin::new();
        assert_eq!(plugin.stagnation_threshold(), 3);
    }

    #[test]
    fn test_plugin_custom_stagnation_threshold() {
        let plugin = TestPlugin::new().with_stagnation_threshold(5);
        assert_eq!(plugin.stagnation_threshold(), 5);
    }

    #[test]
    fn test_plugin_name() {
        let plugin = TestPlugin::new();
        assert_eq!(plugin.name(), "test_plugin");
    }

    #[test]
    fn test_plugin_trait_object() {
        // Verify the trait can be used as a trait object
        let plugin: Box<dyn ContinuousCodingPlugin> = Box::new(TestPlugin::new());
        assert_eq!(plugin.max_iterations(), 50);
        assert_eq!(plugin.stagnation_threshold(), 3);
        assert_eq!(plugin.name(), "test_plugin");
    }
}
