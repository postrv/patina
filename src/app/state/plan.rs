//! Plan review state for interactive plan approval.
//!
//! [`PlanState`] holds a pending plan from the model's `plan` tool_use,
//! allowing the user to review steps before execution proceeds.
//! This follows the same intercept pattern as [`PermissionRequest`].
//!
//! The canonical definitions live in [`crate::types::ui_state`]; this module
//! re-exports them for backward compatibility.

pub use crate::types::ui_state::{PlanState, PlanStep};

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
