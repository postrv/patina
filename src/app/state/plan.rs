//! Plan review state for interactive plan approval.
//!
//! [`PlanState`] holds a pending plan from the model's `plan` tool_use,
//! allowing the user to review steps before execution proceeds.
//! This follows the same intercept pattern as [`PermissionRequest`].

use crate::types::ToolResultBlock;

/// A single step in a proposed plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanStep {
    /// Human-readable description of this step.
    pub description: String,
    /// Tool calls this step intends to make (informational).
    pub tool_calls: Vec<String>,
}

/// State for a pending plan awaiting user review.
///
/// Created when the model emits a `plan` tool_use. The plan is held
/// in this state until the user approves or rejects it via the
/// [`PlanHandler`](crate::app::handlers::plan::PlanHandler).
#[derive(Debug, Clone)]
pub struct PlanState {
    /// The tool_use ID to include in the tool result response.
    pub tool_use_id: String,
    /// Plan title summarizing the proposed work.
    pub title: String,
    /// Ordered list of steps in the plan.
    pub steps: Vec<PlanStep>,
    /// Index of the currently highlighted step in the review UI.
    pub selected_index: usize,
}

impl PlanState {
    /// Creates a new plan state from a tool_use.
    ///
    /// # Arguments
    ///
    /// * `tool_use_id` - The tool_use ID for the response
    /// * `title` - Plan title
    /// * `steps` - Ordered plan steps
    #[must_use]
    pub fn new(tool_use_id: String, title: String, steps: Vec<PlanStep>) -> Self {
        Self {
            tool_use_id,
            title,
            steps,
            selected_index: 0,
        }
    }

    /// Moves the selection to the next step (wraps around).
    pub fn select_next(&mut self) {
        if !self.steps.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.steps.len();
        }
    }

    /// Moves the selection to the previous step (wraps around).
    pub fn select_prev(&mut self) {
        if !self.steps.is_empty() {
            self.selected_index = if self.selected_index == 0 {
                self.steps.len() - 1
            } else {
                self.selected_index - 1
            };
        }
    }

    /// Returns the number of steps in the plan.
    #[must_use]
    pub fn step_count(&self) -> usize {
        self.steps.len()
    }

    /// Approves the plan, returning a success tool result block.
    #[must_use]
    pub fn approve(&self) -> ToolResultBlock {
        ToolResultBlock {
            tool_use_id: self.tool_use_id.clone(),
            content: "Plan approved by user. Proceed with execution.".to_string(),
            is_error: false,
        }
    }

    /// Rejects the plan, returning an error tool result block.
    #[must_use]
    pub fn reject(&self) -> ToolResultBlock {
        ToolResultBlock {
            tool_use_id: self.tool_use_id.clone(),
            content: "Plan rejected by user.".to_string(),
            is_error: true,
        }
    }

    /// Parses a plan from tool_use JSON input.
    ///
    /// Expected schema:
    /// ```json
    /// {
    ///   "title": "...",
    ///   "steps": [{ "description": "...", "tool_calls": ["..."] }]
    /// }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns `None` if the input doesn't match the expected schema.
    #[must_use]
    pub fn from_tool_input(tool_use_id: String, input: &serde_json::Value) -> Option<Self> {
        let title = input.get("title")?.as_str()?.to_string();
        let steps_arr = input.get("steps")?.as_array()?;

        let steps: Vec<PlanStep> = steps_arr
            .iter()
            .filter_map(|step| {
                let description = step.get("description")?.as_str()?.to_string();
                let tool_calls = step
                    .get("tool_calls")
                    .and_then(|tc| tc.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                Some(PlanStep {
                    description,
                    tool_calls,
                })
            })
            .collect();

        if steps.is_empty() {
            return None;
        }

        Some(Self::new(tool_use_id, title, steps))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_steps() -> Vec<PlanStep> {
        vec![
            PlanStep {
                description: "Read the config file".to_string(),
                tool_calls: vec!["read_file".to_string()],
            },
            PlanStep {
                description: "Edit the config".to_string(),
                tool_calls: vec!["edit".to_string()],
            },
            PlanStep {
                description: "Run tests".to_string(),
                tool_calls: vec!["bash".to_string()],
            },
        ]
    }

    // =========================================================================
    // Construction
    // =========================================================================

    #[test]
    fn new_creates_with_zero_index() {
        let state = PlanState::new("toolu_1".into(), "My Plan".into(), sample_steps());
        assert_eq!(state.tool_use_id, "toolu_1");
        assert_eq!(state.title, "My Plan");
        assert_eq!(state.steps.len(), 3);
        assert_eq!(state.selected_index, 0);
    }

    #[test]
    fn step_count_returns_correct_value() {
        let state = PlanState::new("id".into(), "title".into(), sample_steps());
        assert_eq!(state.step_count(), 3);
    }

    // =========================================================================
    // Navigation
    // =========================================================================

    #[test]
    fn select_next_wraps_around() {
        let mut state = PlanState::new("id".into(), "title".into(), sample_steps());
        assert_eq!(state.selected_index, 0);
        state.select_next();
        assert_eq!(state.selected_index, 1);
        state.select_next();
        assert_eq!(state.selected_index, 2);
        state.select_next();
        assert_eq!(state.selected_index, 0); // wraps
    }

    #[test]
    fn select_prev_wraps_around() {
        let mut state = PlanState::new("id".into(), "title".into(), sample_steps());
        assert_eq!(state.selected_index, 0);
        state.select_prev();
        assert_eq!(state.selected_index, 2); // wraps to last
        state.select_prev();
        assert_eq!(state.selected_index, 1);
    }

    #[test]
    fn select_next_on_empty_is_noop() {
        let mut state = PlanState::new("id".into(), "title".into(), vec![]);
        state.select_next();
        assert_eq!(state.selected_index, 0);
    }

    #[test]
    fn select_prev_on_empty_is_noop() {
        let mut state = PlanState::new("id".into(), "title".into(), vec![]);
        state.select_prev();
        assert_eq!(state.selected_index, 0);
    }

    // =========================================================================
    // Approve / Reject
    // =========================================================================

    #[test]
    fn approve_returns_success_block() {
        let state = PlanState::new("toolu_abc".into(), "Plan".into(), sample_steps());
        let block = state.approve();
        assert_eq!(block.tool_use_id, "toolu_abc");
        assert!(!block.is_error);
        assert!(block.content.contains("approved"));
    }

    #[test]
    fn reject_returns_error_block() {
        let state = PlanState::new("toolu_abc".into(), "Plan".into(), sample_steps());
        let block = state.reject();
        assert_eq!(block.tool_use_id, "toolu_abc");
        assert!(block.is_error);
        assert!(block.content.contains("rejected"));
    }

    // =========================================================================
    // from_tool_input parsing
    // =========================================================================

    #[test]
    fn from_tool_input_parses_valid_json() {
        let input = json!({
            "title": "Refactor auth",
            "steps": [
                {
                    "description": "Read auth module",
                    "tool_calls": ["read_file"]
                },
                {
                    "description": "Edit auth handler",
                    "tool_calls": ["edit", "bash"]
                }
            ]
        });
        let state = PlanState::from_tool_input("toolu_1".into(), &input).unwrap();
        assert_eq!(state.title, "Refactor auth");
        assert_eq!(state.steps.len(), 2);
        assert_eq!(state.steps[0].description, "Read auth module");
        assert_eq!(state.steps[0].tool_calls, vec!["read_file"]);
        assert_eq!(state.steps[1].tool_calls, vec!["edit", "bash"]);
    }

    #[test]
    fn from_tool_input_returns_none_on_missing_title() {
        let input = json!({
            "steps": [{ "description": "Do something" }]
        });
        assert!(PlanState::from_tool_input("id".into(), &input).is_none());
    }

    #[test]
    fn from_tool_input_returns_none_on_missing_steps() {
        let input = json!({ "title": "Plan" });
        assert!(PlanState::from_tool_input("id".into(), &input).is_none());
    }

    #[test]
    fn from_tool_input_returns_none_on_empty_steps() {
        let input = json!({
            "title": "Plan",
            "steps": []
        });
        assert!(PlanState::from_tool_input("id".into(), &input).is_none());
    }

    #[test]
    fn from_tool_input_skips_invalid_steps() {
        let input = json!({
            "title": "Plan",
            "steps": [
                { "description": "Valid step" },
                { "not_description": "Invalid" },
                { "description": "Another valid" }
            ]
        });
        let state = PlanState::from_tool_input("id".into(), &input).unwrap();
        assert_eq!(state.steps.len(), 2);
    }

    #[test]
    fn from_tool_input_handles_missing_tool_calls() {
        let input = json!({
            "title": "Plan",
            "steps": [{ "description": "Step without tools" }]
        });
        let state = PlanState::from_tool_input("id".into(), &input).unwrap();
        assert!(state.steps[0].tool_calls.is_empty());
    }

    // =========================================================================
    // Send + Sync
    // =========================================================================

    #[test]
    fn plan_state_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<PlanState>();
    }

    #[test]
    fn plan_state_is_sync() {
        fn assert_sync<T: Sync>() {}
        assert_sync::<PlanState>();
    }
}
