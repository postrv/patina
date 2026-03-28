//! Model config delegation and request building methods for [`AppState`].

use super::*;

impl AppState {
    /// Returns a reference to the cost tracker.
    #[must_use]
    pub fn cost_tracker(&self) -> &CostTracker {
        &self.cost_tracker
    }

    /// Returns a formatted cost summary for display.
    #[must_use]
    pub fn cost_summary(&self) -> String {
        let stats = self.cost_tracker.statistics();
        if stats.total_requests == 0 {
            return "No usage data recorded yet.".to_string();
        }
        format!(
            "Session cost: ${:.4}\n\
             Requests: {}\n\
             Input tokens: {}\n\
             Output tokens: {}\n\
             Cost by model:\n{}",
            stats.total_cost,
            stats.total_requests,
            stats.total_input_tokens,
            stats.total_output_tokens,
            self.cost_tracker
                .cost_by_model()
                .iter()
                .map(|(m, c)| format!("  {}: ${:.4}", m, c))
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    /// Builds [`RequestOptions`] from the current state, gated by model capabilities.
    ///
    /// Determines the thinking budget from either the explicit `thinking_budget`
    /// field (if set) or the `effort` level. Returns `None` for thinking if the
    /// model doesn't support it. Wraps the system prompt in [`SystemBlock`]s with
    /// cache control if the model supports it.
    #[must_use]
    pub fn build_request_options(&self, model: &str) -> RequestOptions {
        let caps = ModelCapabilities::for_model(model);

        // Determine thinking config
        let thinking = if caps.supports_thinking {
            let budget = self
                .model_config
                .thinking_budget
                .or_else(|| self.model_config.effort.thinking_budget());
            budget.map(|b| ThinkingConfig {
                config_type: "enabled".to_string(),
                budget_tokens: b,
            })
        } else {
            None
        };

        // Build system prompt text, appending memory if available
        let mut prompt_text = self
            .model_config
            .system_prompt
            .as_deref()
            .unwrap_or_default()
            .to_string();
        if let Some(store) = &self.model_config.memory_store {
            let memory_text = store.render_for_system_prompt();
            if !memory_text.is_empty() {
                if !prompt_text.is_empty() {
                    prompt_text.push_str("\n\n");
                }
                prompt_text.push_str(&memory_text);
            }
        }

        // Append always-active skill context if skill engine is configured
        if let Some(engine) = &self.model_config.skill_engine {
            let always_active: Vec<&crate::skills::Skill> = engine
                .all_skills()
                .iter()
                .filter(|s| s.config.triggers.always_active)
                .collect();
            if !always_active.is_empty() {
                let skill_context = engine.format_skills(&always_active);
                if !skill_context.is_empty() {
                    if !prompt_text.is_empty() {
                        prompt_text.push_str("\n\n");
                    }
                    prompt_text.push_str("# Active Skills\n\n");
                    prompt_text.push_str(&skill_context);
                }
            }
        }

        // Build system blocks with optional cache control
        let system = if prompt_text.is_empty() {
            None
        } else {
            let cache_control = if caps.supports_cache_control {
                Some(crate::api::CacheControl {
                    cache_type: "ephemeral".to_string(),
                })
            } else {
                None
            };
            Some(vec![SystemBlock {
                block_type: "text".to_string(),
                text: prompt_text,
                cache_control,
            }])
        };

        RequestOptions { thinking, system }
    }
}
