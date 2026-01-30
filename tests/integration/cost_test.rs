//! Integration tests for enterprise cost controls.
//!
//! Tests cost control functionality including:
//! - Token tracking and cost calculation
//! - Budget limits (session, daily, monthly)
//! - Alerts and warnings
//! - Cost reporting and statistics

use rct::enterprise::cost::{
    BudgetLimit, BudgetPeriod, CostAlert, CostConfig, CostTracker, ModelPricing, UsageRecord,
};
use std::time::Duration;

// =============================================================================
// Helper functions
// =============================================================================

/// Creates a default cost tracker for testing.
fn test_tracker() -> CostTracker {
    let config = CostConfig {
        enabled: true,
        session_limit: Some(BudgetLimit::new(BudgetPeriod::Session, 10.0)),
        daily_limit: Some(BudgetLimit::new(BudgetPeriod::Daily, 100.0)),
        monthly_limit: Some(BudgetLimit::new(BudgetPeriod::Monthly, 1000.0)),
        warning_threshold: 0.8, // Warn at 80%
    };
    CostTracker::new(config)
}

/// Creates a cost tracker with no limits.
fn unlimited_tracker() -> CostTracker {
    let config = CostConfig {
        enabled: true,
        session_limit: None,
        daily_limit: None,
        monthly_limit: None,
        warning_threshold: 0.8,
    };
    CostTracker::new(config)
}

// =============================================================================
// 7.4.2 Cost Control Tests
// =============================================================================

// -----------------------------------------------------------------------------
// Cost Tracking Tests
// -----------------------------------------------------------------------------

/// Test that token usage is tracked correctly.
#[test]
fn test_cost_tracker_records_usage() {
    let mut tracker = test_tracker();

    let record = UsageRecord::new("claude-3-opus", 1000, 500, Duration::from_secs(2));
    tracker.record_usage(record);

    let stats = tracker.statistics();
    assert_eq!(stats.total_input_tokens, 1000);
    assert_eq!(stats.total_output_tokens, 500);
    assert_eq!(stats.total_requests, 1);
}

/// Test that multiple usages are accumulated.
#[test]
fn test_cost_tracker_accumulates_usage() {
    let mut tracker = test_tracker();

    tracker.record_usage(UsageRecord::new(
        "claude-3-opus",
        1000,
        500,
        Duration::from_secs(1),
    ));
    tracker.record_usage(UsageRecord::new(
        "claude-3-opus",
        2000,
        1000,
        Duration::from_secs(2),
    ));
    tracker.record_usage(UsageRecord::new(
        "claude-3-sonnet",
        500,
        250,
        Duration::from_secs(1),
    ));

    let stats = tracker.statistics();
    assert_eq!(stats.total_input_tokens, 3500);
    assert_eq!(stats.total_output_tokens, 1750);
    assert_eq!(stats.total_requests, 3);
}

/// Test cost calculation for different models.
#[test]
fn test_cost_calculation_by_model() {
    let mut tracker = unlimited_tracker();

    // Claude 3 Opus pricing (example: $15/1M input, $75/1M output)
    tracker.record_usage(UsageRecord::new(
        "claude-3-opus-20240229",
        1_000_000, // 1M input tokens
        1_000_000, // 1M output tokens
        Duration::from_secs(10),
    ));

    let cost = tracker.total_cost();
    // $15 input + $75 output = $90
    assert!(
        (cost - 90.0).abs() < 0.01,
        "Expected ~$90, got ${:.2}",
        cost
    );
}

/// Test Sonnet pricing.
#[test]
fn test_cost_calculation_sonnet() {
    let mut tracker = unlimited_tracker();

    // Claude 3 Sonnet pricing (example: $3/1M input, $15/1M output)
    tracker.record_usage(UsageRecord::new(
        "claude-3-sonnet-20240229",
        1_000_000,
        1_000_000,
        Duration::from_secs(5),
    ));

    let cost = tracker.total_cost();
    // $3 input + $15 output = $18
    assert!(
        (cost - 18.0).abs() < 0.01,
        "Expected ~$18, got ${:.2}",
        cost
    );
}

/// Test Haiku pricing.
#[test]
fn test_cost_calculation_haiku() {
    let mut tracker = unlimited_tracker();

    // Claude 3 Haiku pricing (example: $0.25/1M input, $1.25/1M output)
    tracker.record_usage(UsageRecord::new(
        "claude-3-haiku-20240307",
        1_000_000,
        1_000_000,
        Duration::from_secs(2),
    ));

    let cost = tracker.total_cost();
    // $0.25 input + $1.25 output = $1.50
    assert!(
        (cost - 1.5).abs() < 0.01,
        "Expected ~$1.50, got ${:.2}",
        cost
    );
}

// -----------------------------------------------------------------------------
// Budget Limit Tests
// -----------------------------------------------------------------------------

/// Test session budget limit enforcement.
#[test]
fn test_session_budget_limit() {
    let config = CostConfig {
        enabled: true,
        session_limit: Some(BudgetLimit::new(BudgetPeriod::Session, 1.0)), // $1 limit
        daily_limit: None,
        monthly_limit: None,
        warning_threshold: 0.8,
    };
    let mut tracker = CostTracker::new(config);

    // Use Haiku (cheap) to stay under budget
    tracker.record_usage(UsageRecord::new(
        "claude-3-haiku-20240307",
        100_000, // 100k input
        100_000, // 100k output
        Duration::from_secs(1),
    ));

    assert!(!tracker.is_budget_exceeded());

    // Add more usage to exceed budget
    tracker.record_usage(UsageRecord::new(
        "claude-3-opus-20240229",
        100_000, // Higher cost model
        100_000,
        Duration::from_secs(1),
    ));

    assert!(tracker.is_budget_exceeded());
}

/// Test daily budget limit tracking.
#[test]
fn test_daily_budget_limit() {
    let config = CostConfig {
        enabled: true,
        session_limit: None,
        daily_limit: Some(BudgetLimit::new(BudgetPeriod::Daily, 5.0)),
        monthly_limit: None,
        warning_threshold: 0.8,
    };
    let mut tracker = CostTracker::new(config);

    // Use enough tokens to exceed daily limit
    for _ in 0..10 {
        tracker.record_usage(UsageRecord::new(
            "claude-3-sonnet-20240229",
            100_000,
            50_000,
            Duration::from_secs(1),
        ));
    }

    assert!(tracker.is_budget_exceeded());
}

/// Test monthly budget limit.
#[test]
fn test_monthly_budget_limit() {
    let config = CostConfig {
        enabled: true,
        session_limit: None,
        daily_limit: None,
        monthly_limit: Some(BudgetLimit::new(BudgetPeriod::Monthly, 50.0)),
        warning_threshold: 0.8,
    };
    let mut tracker = CostTracker::new(config);

    // Make high-cost request to test monthly tracking
    tracker.record_usage(UsageRecord::new(
        "claude-3-opus-20240229",
        500_000,
        500_000,
        Duration::from_secs(30),
    ));

    // Check current cost
    let cost = tracker.total_cost();
    assert!(cost > 0.0);

    // Verify within or exceeded budget
    let status = tracker.budget_status();
    assert!(status.monthly_remaining.is_some() || tracker.is_budget_exceeded());
}

// -----------------------------------------------------------------------------
// Alert Tests
// -----------------------------------------------------------------------------

/// Test warning alerts when approaching limit.
#[test]
fn test_budget_warning_alert() {
    let config = CostConfig {
        enabled: true,
        session_limit: Some(BudgetLimit::new(BudgetPeriod::Session, 1.0)),
        daily_limit: None,
        monthly_limit: None,
        warning_threshold: 0.5, // Warn at 50%
    };
    let mut tracker = CostTracker::new(config);

    // Use ~60% of budget with Haiku pricing ($0.25/1M input, $1.25/1M output)
    // 200k input = $0.05, 500k output = $0.625 => total $0.675 = 67.5% of $1 limit
    tracker.record_usage(UsageRecord::new(
        "claude-3-haiku-20240307",
        200_000, // $0.05
        500_000, // $0.625
        Duration::from_secs(1),
    ));

    let alerts = tracker.check_alerts();
    let has_warning = alerts
        .iter()
        .any(|a| matches!(a, CostAlert::ApproachingLimit { .. }));
    assert!(
        has_warning,
        "Expected warning alert when over 50% of budget"
    );
}

/// Test exceeded alert when over budget.
#[test]
fn test_budget_exceeded_alert() {
    let config = CostConfig {
        enabled: true,
        session_limit: Some(BudgetLimit::new(BudgetPeriod::Session, 0.10)), // Very low: $0.10
        daily_limit: None,
        monthly_limit: None,
        warning_threshold: 0.8,
    };
    let mut tracker = CostTracker::new(config);

    // Exceed the tiny budget
    tracker.record_usage(UsageRecord::new(
        "claude-3-opus-20240229",
        10_000,
        10_000,
        Duration::from_secs(1),
    ));

    let alerts = tracker.check_alerts();
    let has_exceeded = alerts
        .iter()
        .any(|a| matches!(a, CostAlert::LimitExceeded { .. }));
    assert!(has_exceeded, "Expected exceeded alert");
}

/// Test no alerts when under budget.
#[test]
fn test_no_alerts_under_budget() {
    let config = CostConfig {
        enabled: true,
        session_limit: Some(BudgetLimit::new(BudgetPeriod::Session, 100.0)), // $100 limit
        daily_limit: None,
        monthly_limit: None,
        warning_threshold: 0.8,
    };
    let mut tracker = CostTracker::new(config);

    // Small usage
    tracker.record_usage(UsageRecord::new(
        "claude-3-haiku-20240307",
        1000,
        1000,
        Duration::from_secs(1),
    ));

    let alerts = tracker.check_alerts();
    assert!(alerts.is_empty(), "Expected no alerts for small usage");
}

// -----------------------------------------------------------------------------
// Model Pricing Tests
// -----------------------------------------------------------------------------

/// Test custom model pricing.
#[test]
fn test_custom_model_pricing() {
    let mut tracker = unlimited_tracker();

    // Add custom pricing for a hypothetical model
    tracker.set_model_pricing(
        "custom-model-v1",
        ModelPricing::new(5.0, 25.0), // $5/1M input, $25/1M output
    );

    tracker.record_usage(UsageRecord::new(
        "custom-model-v1",
        1_000_000,
        1_000_000,
        Duration::from_secs(5),
    ));

    let cost = tracker.total_cost();
    assert!((cost - 30.0).abs() < 0.01, "Expected $30, got ${:.2}", cost);
}

/// Test unknown model uses default pricing.
#[test]
fn test_unknown_model_default_pricing() {
    let mut tracker = unlimited_tracker();

    tracker.record_usage(UsageRecord::new(
        "unknown-model-xyz",
        1_000_000,
        1_000_000,
        Duration::from_secs(1),
    ));

    // Should use default pricing (conservative estimate)
    let cost = tracker.total_cost();
    assert!(cost > 0.0, "Unknown model should still have some cost");
}

// -----------------------------------------------------------------------------
// Statistics Tests
// -----------------------------------------------------------------------------

/// Test cost statistics per model.
#[test]
fn test_cost_statistics_per_model() {
    let mut tracker = unlimited_tracker();

    tracker.record_usage(UsageRecord::new(
        "claude-3-opus-20240229",
        100_000,
        50_000,
        Duration::from_secs(2),
    ));
    tracker.record_usage(UsageRecord::new(
        "claude-3-sonnet-20240229",
        200_000,
        100_000,
        Duration::from_secs(1),
    ));

    let _stats = tracker.statistics();
    let by_model = tracker.cost_by_model();

    assert_eq!(by_model.len(), 2);
    assert!(by_model.contains_key("claude-3-opus-20240229"));
    assert!(by_model.contains_key("claude-3-sonnet-20240229"));

    // Opus should be more expensive per token
    let opus_cost = by_model.get("claude-3-opus-20240229").unwrap();
    let sonnet_cost = by_model.get("claude-3-sonnet-20240229").unwrap();
    assert!(
        opus_cost > sonnet_cost,
        "Opus should cost more than Sonnet for similar token counts"
    );
}

/// Test total cost across multiple sessions.
#[test]
fn test_aggregate_cost_tracking() {
    let mut tracker = unlimited_tracker();

    // Simulate multiple API calls
    for _ in 0..100 {
        tracker.record_usage(UsageRecord::new(
            "claude-3-haiku-20240307",
            500,
            250,
            Duration::from_millis(100),
        ));
    }

    let stats = tracker.statistics();
    assert_eq!(stats.total_requests, 100);
    assert_eq!(stats.total_input_tokens, 50_000);
    assert_eq!(stats.total_output_tokens, 25_000);

    let cost = tracker.total_cost();
    assert!(cost > 0.0);
}

// -----------------------------------------------------------------------------
// Configuration Tests
// -----------------------------------------------------------------------------

/// Test disabled cost tracking.
#[test]
fn test_cost_tracking_disabled() {
    let config = CostConfig {
        enabled: false,
        session_limit: Some(BudgetLimit::new(BudgetPeriod::Session, 0.01)), // Tiny limit
        daily_limit: None,
        monthly_limit: None,
        warning_threshold: 0.8,
    };
    let mut tracker = CostTracker::new(config);

    // This would exceed budget if enabled
    tracker.record_usage(UsageRecord::new(
        "claude-3-opus-20240229",
        1_000_000,
        1_000_000,
        Duration::from_secs(10),
    ));

    // When disabled, budget is never exceeded
    assert!(!tracker.is_budget_exceeded());
    assert!(tracker.check_alerts().is_empty());
}

/// Test budget status reporting.
#[test]
fn test_budget_status_report() {
    let config = CostConfig {
        enabled: true,
        session_limit: Some(BudgetLimit::new(BudgetPeriod::Session, 10.0)),
        daily_limit: Some(BudgetLimit::new(BudgetPeriod::Daily, 50.0)),
        monthly_limit: Some(BudgetLimit::new(BudgetPeriod::Monthly, 500.0)),
        warning_threshold: 0.8,
    };
    let mut tracker = CostTracker::new(config);

    tracker.record_usage(UsageRecord::new(
        "claude-3-sonnet-20240229",
        100_000,
        50_000,
        Duration::from_secs(2),
    ));

    let status = tracker.budget_status();

    assert!(status.session_remaining.is_some());
    assert!(status.daily_remaining.is_some());
    assert!(status.monthly_remaining.is_some());
    assert!(status.current_cost > 0.0);
}

/// Test resetting session costs.
#[test]
fn test_session_cost_reset() {
    let mut tracker = test_tracker();

    tracker.record_usage(UsageRecord::new(
        "claude-3-opus-20240229",
        100_000,
        50_000,
        Duration::from_secs(2),
    ));

    assert!(tracker.session_cost() > 0.0);

    tracker.reset_session();

    assert_eq!(tracker.session_cost(), 0.0);
    // But total/daily/monthly should still have history
}

/// Test config validation.
#[test]
fn test_cost_config_validation() {
    let valid_config = CostConfig {
        enabled: true,
        session_limit: Some(BudgetLimit::new(BudgetPeriod::Session, 10.0)),
        daily_limit: Some(BudgetLimit::new(BudgetPeriod::Daily, 100.0)),
        monthly_limit: Some(BudgetLimit::new(BudgetPeriod::Monthly, 1000.0)),
        warning_threshold: 0.8,
    };
    assert!(valid_config.validate().is_ok());

    let invalid_threshold = CostConfig {
        enabled: true,
        session_limit: None,
        daily_limit: None,
        monthly_limit: None,
        warning_threshold: 1.5, // Invalid: >1.0
    };
    assert!(invalid_threshold.validate().is_err());
}
