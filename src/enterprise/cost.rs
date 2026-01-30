//! Cost controls for enterprise budget management.
//!
//! This module provides token tracking, cost calculation, and budget
//! enforcement for Claude API usage.
//!
//! # Example
//!
//! ```
//! use rct::enterprise::cost::{BudgetLimit, BudgetPeriod, CostConfig, CostTracker, UsageRecord};
//! use std::time::Duration;
//!
//! // Configure cost controls with a session limit
//! let config = CostConfig {
//!     enabled: true,
//!     session_limit: Some(BudgetLimit::new(BudgetPeriod::Session, 10.0)),
//!     daily_limit: Some(BudgetLimit::new(BudgetPeriod::Daily, 100.0)),
//!     monthly_limit: None,
//!     warning_threshold: 0.8,
//! };
//!
//! let mut tracker = CostTracker::new(config);
//!
//! // Record API usage
//! let record = UsageRecord::new("claude-3-sonnet-20240229", 1000, 500, Duration::from_secs(2));
//! tracker.record_usage(record);
//!
//! // Check budget status
//! if tracker.is_budget_exceeded() {
//!     println!("Budget exceeded!");
//! }
//!
//! // Get cost statistics
//! let stats = tracker.statistics();
//! println!("Total cost: ${:.2}", tracker.total_cost());
//! ```

use anyhow::{bail, Result};
use std::collections::HashMap;
use std::time::Duration;

/// Budget period for cost limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetPeriod {
    /// Per-session limit (resets when session ends).
    Session,
    /// Daily limit (resets at midnight UTC).
    Daily,
    /// Monthly limit (resets at start of month).
    Monthly,
}

/// Budget limit configuration.
#[derive(Debug, Clone)]
pub struct BudgetLimit {
    /// The period this limit applies to.
    pub period: BudgetPeriod,
    /// Maximum cost in USD.
    pub max_cost: f64,
}

impl BudgetLimit {
    /// Creates a new budget limit.
    #[must_use]
    pub fn new(period: BudgetPeriod, max_cost: f64) -> Self {
        Self { period, max_cost }
    }
}

/// Pricing for a model (per million tokens).
#[derive(Debug, Clone, Copy)]
pub struct ModelPricing {
    /// Cost per million input tokens in USD.
    pub input_per_million: f64,
    /// Cost per million output tokens in USD.
    pub output_per_million: f64,
}

impl ModelPricing {
    /// Creates new model pricing.
    #[must_use]
    pub fn new(input_per_million: f64, output_per_million: f64) -> Self {
        Self {
            input_per_million,
            output_per_million,
        }
    }

    /// Calculates the cost for given token counts.
    #[must_use]
    pub fn calculate_cost(&self, input_tokens: u32, output_tokens: u32) -> f64 {
        let input_cost = (input_tokens as f64 / 1_000_000.0) * self.input_per_million;
        let output_cost = (output_tokens as f64 / 1_000_000.0) * self.output_per_million;
        input_cost + output_cost
    }
}

/// Default pricing for known models.
fn default_model_pricing() -> HashMap<&'static str, ModelPricing> {
    let mut pricing = HashMap::new();

    // Claude 3 Opus: $15/1M input, $75/1M output
    pricing.insert("claude-3-opus-20240229", ModelPricing::new(15.0, 75.0));
    pricing.insert("claude-3-opus", ModelPricing::new(15.0, 75.0));

    // Claude 3 Sonnet: $3/1M input, $15/1M output
    pricing.insert("claude-3-sonnet-20240229", ModelPricing::new(3.0, 15.0));
    pricing.insert("claude-3-sonnet", ModelPricing::new(3.0, 15.0));

    // Claude 3.5 Sonnet: $3/1M input, $15/1M output
    pricing.insert("claude-3-5-sonnet-20240620", ModelPricing::new(3.0, 15.0));
    pricing.insert("claude-3-5-sonnet", ModelPricing::new(3.0, 15.0));

    // Claude 3 Haiku: $0.25/1M input, $1.25/1M output
    pricing.insert("claude-3-haiku-20240307", ModelPricing::new(0.25, 1.25));
    pricing.insert("claude-3-haiku", ModelPricing::new(0.25, 1.25));

    pricing
}

/// Default pricing for unknown models (conservative estimate).
const DEFAULT_UNKNOWN_PRICING: ModelPricing = ModelPricing {
    input_per_million: 10.0,
    output_per_million: 50.0,
};

/// Configuration for cost controls.
#[derive(Debug, Clone)]
pub struct CostConfig {
    /// Whether cost tracking is enabled.
    pub enabled: bool,
    /// Per-session cost limit.
    pub session_limit: Option<BudgetLimit>,
    /// Daily cost limit.
    pub daily_limit: Option<BudgetLimit>,
    /// Monthly cost limit.
    pub monthly_limit: Option<BudgetLimit>,
    /// Threshold (0.0-1.0) at which to issue warnings.
    pub warning_threshold: f64,
}

impl CostConfig {
    /// Validates the configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration is invalid.
    pub fn validate(&self) -> Result<()> {
        if self.warning_threshold < 0.0 || self.warning_threshold > 1.0 {
            bail!(
                "Warning threshold must be between 0.0 and 1.0, got {}",
                self.warning_threshold
            );
        }

        if let Some(ref limit) = self.session_limit {
            if limit.max_cost <= 0.0 {
                bail!("Session limit must be positive, got {}", limit.max_cost);
            }
        }

        if let Some(ref limit) = self.daily_limit {
            if limit.max_cost <= 0.0 {
                bail!("Daily limit must be positive, got {}", limit.max_cost);
            }
        }

        if let Some(ref limit) = self.monthly_limit {
            if limit.max_cost <= 0.0 {
                bail!("Monthly limit must be positive, got {}", limit.max_cost);
            }
        }

        Ok(())
    }
}

impl Default for CostConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            session_limit: None,
            daily_limit: None,
            monthly_limit: None,
            warning_threshold: 0.8,
        }
    }
}

/// Record of a single API usage.
#[derive(Debug, Clone)]
pub struct UsageRecord {
    /// Model used.
    pub model: String,
    /// Number of input tokens.
    pub input_tokens: u32,
    /// Number of output tokens.
    pub output_tokens: u32,
    /// Duration of the request.
    pub duration: Duration,
    /// Calculated cost in USD.
    pub cost: f64,
}

impl UsageRecord {
    /// Creates a new usage record.
    ///
    /// Cost is calculated automatically based on model pricing.
    #[must_use]
    pub fn new(model: &str, input_tokens: u32, output_tokens: u32, duration: Duration) -> Self {
        let pricing = default_model_pricing();
        let model_pricing = pricing
            .get(model)
            .copied()
            .unwrap_or(DEFAULT_UNKNOWN_PRICING);
        let cost = model_pricing.calculate_cost(input_tokens, output_tokens);

        Self {
            model: model.to_string(),
            input_tokens,
            output_tokens,
            duration,
            cost,
        }
    }
}

/// Cost alert types.
#[derive(Debug, Clone)]
pub enum CostAlert {
    /// Approaching a budget limit.
    ApproachingLimit {
        /// The period being approached.
        period: BudgetPeriod,
        /// Current usage percentage (0.0-1.0).
        usage_percentage: f64,
        /// Remaining budget in USD.
        remaining: f64,
    },
    /// Budget limit exceeded.
    LimitExceeded {
        /// The period that was exceeded.
        period: BudgetPeriod,
        /// Amount over limit in USD.
        overage: f64,
    },
}

/// Budget status report.
#[derive(Debug, Clone, Default)]
pub struct BudgetStatus {
    /// Remaining session budget (if limited).
    pub session_remaining: Option<f64>,
    /// Remaining daily budget (if limited).
    pub daily_remaining: Option<f64>,
    /// Remaining monthly budget (if limited).
    pub monthly_remaining: Option<f64>,
    /// Current total cost.
    pub current_cost: f64,
}

/// Usage statistics.
#[derive(Debug, Clone, Default)]
pub struct CostStatistics {
    /// Total API requests made.
    pub total_requests: usize,
    /// Total input tokens used.
    pub total_input_tokens: u32,
    /// Total output tokens generated.
    pub total_output_tokens: u32,
    /// Total cost in USD.
    pub total_cost: f64,
}

/// Cost tracker for monitoring and enforcing budget limits.
#[derive(Debug)]
pub struct CostTracker {
    /// Configuration.
    config: CostConfig,
    /// Custom model pricing overrides.
    custom_pricing: HashMap<String, ModelPricing>,
    /// Usage records for the current session.
    session_records: Vec<UsageRecord>,
    /// Accumulated session cost.
    session_cost: f64,
    /// Accumulated daily cost.
    daily_cost: f64,
    /// Accumulated monthly cost.
    monthly_cost: f64,
    /// Cost breakdown by model.
    cost_by_model: HashMap<String, f64>,
}

impl CostTracker {
    /// Creates a new cost tracker.
    #[must_use]
    pub fn new(config: CostConfig) -> Self {
        Self {
            config,
            custom_pricing: HashMap::new(),
            session_records: Vec::new(),
            session_cost: 0.0,
            daily_cost: 0.0,
            monthly_cost: 0.0,
            cost_by_model: HashMap::new(),
        }
    }

    /// Records a usage event.
    pub fn record_usage(&mut self, mut record: UsageRecord) {
        // Recalculate cost if we have custom pricing for this model
        if let Some(pricing) = self.custom_pricing.get(&record.model) {
            record.cost = pricing.calculate_cost(record.input_tokens, record.output_tokens);
        }

        self.session_cost += record.cost;
        self.daily_cost += record.cost;
        self.monthly_cost += record.cost;

        *self
            .cost_by_model
            .entry(record.model.clone())
            .or_insert(0.0) += record.cost;

        self.session_records.push(record);
    }

    /// Sets custom pricing for a model.
    pub fn set_model_pricing(&mut self, model: &str, pricing: ModelPricing) {
        self.custom_pricing.insert(model.to_string(), pricing);
    }

    /// Returns whether any budget limit has been exceeded.
    #[must_use]
    pub fn is_budget_exceeded(&self) -> bool {
        if !self.config.enabled {
            return false;
        }

        if let Some(ref limit) = self.config.session_limit {
            if self.session_cost > limit.max_cost {
                return true;
            }
        }

        if let Some(ref limit) = self.config.daily_limit {
            if self.daily_cost > limit.max_cost {
                return true;
            }
        }

        if let Some(ref limit) = self.config.monthly_limit {
            if self.monthly_cost > limit.max_cost {
                return true;
            }
        }

        false
    }

    /// Checks for budget alerts.
    #[must_use]
    pub fn check_alerts(&self) -> Vec<CostAlert> {
        if !self.config.enabled {
            return Vec::new();
        }

        let mut alerts = Vec::new();

        // Check session limit
        if let Some(ref limit) = self.config.session_limit {
            self.check_limit_alert(&mut alerts, BudgetPeriod::Session, self.session_cost, limit);
        }

        // Check daily limit
        if let Some(ref limit) = self.config.daily_limit {
            self.check_limit_alert(&mut alerts, BudgetPeriod::Daily, self.daily_cost, limit);
        }

        // Check monthly limit
        if let Some(ref limit) = self.config.monthly_limit {
            self.check_limit_alert(&mut alerts, BudgetPeriod::Monthly, self.monthly_cost, limit);
        }

        alerts
    }

    /// Helper to check a single limit and add appropriate alerts.
    fn check_limit_alert(
        &self,
        alerts: &mut Vec<CostAlert>,
        period: BudgetPeriod,
        current: f64,
        limit: &BudgetLimit,
    ) {
        let usage_percentage = current / limit.max_cost;

        if usage_percentage > 1.0 {
            alerts.push(CostAlert::LimitExceeded {
                period,
                overage: current - limit.max_cost,
            });
        } else if usage_percentage >= self.config.warning_threshold {
            alerts.push(CostAlert::ApproachingLimit {
                period,
                usage_percentage,
                remaining: limit.max_cost - current,
            });
        }
    }

    /// Returns the current budget status.
    #[must_use]
    pub fn budget_status(&self) -> BudgetStatus {
        BudgetStatus {
            session_remaining: self
                .config
                .session_limit
                .as_ref()
                .map(|l| (l.max_cost - self.session_cost).max(0.0)),
            daily_remaining: self
                .config
                .daily_limit
                .as_ref()
                .map(|l| (l.max_cost - self.daily_cost).max(0.0)),
            monthly_remaining: self
                .config
                .monthly_limit
                .as_ref()
                .map(|l| (l.max_cost - self.monthly_cost).max(0.0)),
            current_cost: self.session_cost,
        }
    }

    /// Returns usage statistics.
    #[must_use]
    pub fn statistics(&self) -> CostStatistics {
        let mut stats = CostStatistics::default();

        for record in &self.session_records {
            stats.total_requests += 1;
            stats.total_input_tokens += record.input_tokens;
            stats.total_output_tokens += record.output_tokens;
            stats.total_cost += record.cost;
        }

        stats
    }

    /// Returns the total cost across all usage.
    #[must_use]
    pub fn total_cost(&self) -> f64 {
        self.session_records.iter().map(|r| r.cost).sum()
    }

    /// Returns cost breakdown by model.
    #[must_use]
    pub fn cost_by_model(&self) -> &HashMap<String, f64> {
        &self.cost_by_model
    }

    /// Returns the current session cost.
    #[must_use]
    pub fn session_cost(&self) -> f64 {
        self.session_cost
    }

    /// Resets session-level tracking.
    pub fn reset_session(&mut self) {
        self.session_cost = 0.0;
        self.session_records.clear();
        // Note: daily and monthly costs are preserved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_pricing_calculation() {
        let pricing = ModelPricing::new(10.0, 50.0); // $10/1M input, $50/1M output
        let cost = pricing.calculate_cost(1_000_000, 1_000_000);
        assert!((cost - 60.0).abs() < 0.01);
    }

    #[test]
    fn test_cost_config_default() {
        let config = CostConfig::default();
        assert!(!config.enabled);
        assert!(config.session_limit.is_none());
    }

    #[test]
    fn test_usage_record_cost_calculation() {
        let record = UsageRecord::new(
            "claude-3-haiku-20240307",
            1_000_000,
            1_000_000,
            Duration::from_secs(1),
        );
        // Haiku: $0.25/1M input + $1.25/1M output = $1.50
        assert!((record.cost - 1.5).abs() < 0.01);
    }

    #[test]
    fn test_budget_limit_creation() {
        let limit = BudgetLimit::new(BudgetPeriod::Daily, 100.0);
        assert_eq!(limit.period, BudgetPeriod::Daily);
        assert!((limit.max_cost - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_cost_tracker_creation() {
        let config = CostConfig::default();
        let tracker = CostTracker::new(config);
        assert_eq!(tracker.session_cost(), 0.0);
    }
}
