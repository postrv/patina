//! Question prompt state for interactive user questions.
//!
//! [`QuestionState`] holds a pending question from the model's `ask_user` tool_use,
//! allowing the user to respond with a choice or free-text answer.
//! This follows the same intercept pattern as [`PlanState`](super::plan::PlanState).
//!
//! The canonical definition lives in [`crate::types::ui_state`]; this module
//! re-exports it for backward compatibility.

pub use crate::types::ui_state::QuestionState;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // =========================================================================
    // Construction
    // =========================================================================

    #[test]
    fn new_creates_with_defaults() {
        let state = QuestionState::new(
            "toolu_1".into(),
            "Which framework?".into(),
            vec!["React".into(), "Vue".into()],
            true,
        );
        assert_eq!(state.tool_use_id, "toolu_1");
        assert_eq!(state.question, "Which framework?");
        assert_eq!(state.options.len(), 2);
        assert!(state.allow_free_text);
        assert_eq!(state.selected_option, 0);
        assert!(state.free_text_input.is_empty());
        assert!(!state.in_free_text); // starts on options when options exist
    }

    #[test]
    fn new_without_options_starts_in_free_text() {
        let state = QuestionState::new("id".into(), "What?".into(), vec![], true);
        assert!(state.in_free_text);
    }

    // =========================================================================
    // Navigation
    // =========================================================================

    #[test]
    fn select_next_wraps_around() {
        let mut state = QuestionState::new(
            "id".into(),
            "q".into(),
            vec!["a".into(), "b".into(), "c".into()],
            false,
        );
        state.select_next();
        assert_eq!(state.selected_option, 1);
        state.select_next();
        assert_eq!(state.selected_option, 2);
        state.select_next();
        assert_eq!(state.selected_option, 0);
    }

    #[test]
    fn select_prev_wraps_around() {
        let mut state =
            QuestionState::new("id".into(), "q".into(), vec!["a".into(), "b".into()], false);
        state.select_prev();
        assert_eq!(state.selected_option, 1);
        state.select_prev();
        assert_eq!(state.selected_option, 0);
    }

    #[test]
    fn select_next_on_empty_is_noop() {
        let mut state = QuestionState::new("id".into(), "q".into(), vec![], true);
        state.select_next();
        assert_eq!(state.selected_option, 0);
    }

    // =========================================================================
    // Free-text input
    // =========================================================================

    #[test]
    fn push_char_adds_to_free_text_when_in_free_text_mode() {
        let mut state = QuestionState::new("id".into(), "q".into(), vec![], true);
        assert!(state.in_free_text);
        state.push_char('h');
        state.push_char('i');
        assert_eq!(state.free_text_input, "hi");
    }

    #[test]
    fn push_char_ignored_when_not_in_free_text_mode() {
        let mut state = QuestionState::new("id".into(), "q".into(), vec!["a".into()], true);
        assert!(!state.in_free_text);
        state.push_char('x');
        assert!(state.free_text_input.is_empty());
    }

    #[test]
    fn pop_char_removes_last() {
        let mut state = QuestionState::new("id".into(), "q".into(), vec![], true);
        state.push_char('a');
        state.push_char('b');
        state.pop_char();
        assert_eq!(state.free_text_input, "a");
    }

    #[test]
    fn toggle_input_mode_switches_between_options_and_text() {
        let mut state = QuestionState::new("id".into(), "q".into(), vec!["a".into()], true);
        assert!(!state.in_free_text);
        state.toggle_input_mode();
        assert!(state.in_free_text);
        state.toggle_input_mode();
        assert!(!state.in_free_text);
    }

    #[test]
    fn toggle_input_mode_noop_when_no_options() {
        let mut state = QuestionState::new("id".into(), "q".into(), vec![], true);
        assert!(state.in_free_text);
        state.toggle_input_mode(); // should not switch since no options
        assert!(state.in_free_text);
    }

    #[test]
    fn toggle_input_mode_noop_when_free_text_disabled() {
        let mut state = QuestionState::new("id".into(), "q".into(), vec!["a".into()], false);
        assert!(!state.in_free_text);
        state.toggle_input_mode(); // free_text disabled, can't switch
        assert!(!state.in_free_text);
    }

    // =========================================================================
    // Response
    // =========================================================================

    #[test]
    fn response_text_returns_selected_option() {
        let state = QuestionState::new(
            "id".into(),
            "q".into(),
            vec!["React".into(), "Vue".into()],
            true,
        );
        assert_eq!(state.response_text(), "React");
    }

    #[test]
    fn response_text_returns_free_text_when_in_free_text_mode() {
        let mut state = QuestionState::new("id".into(), "q".into(), vec!["a".into()], true);
        state.toggle_input_mode();
        state.push_char('c');
        state.push_char('u');
        state.push_char('s');
        state.push_char('t');
        assert_eq!(state.response_text(), "cust");
    }

    #[test]
    fn response_text_falls_back_to_option_when_free_text_empty() {
        let mut state = QuestionState::new("id".into(), "q".into(), vec!["default".into()], true);
        state.toggle_input_mode(); // in free-text but empty
        assert_eq!(state.response_text(), "default");
    }

    // =========================================================================
    // Submit / Cancel
    // =========================================================================

    #[test]
    fn submit_returns_success_block() {
        let state = QuestionState::new("toolu_abc".into(), "q".into(), vec!["yes".into()], false);
        let block = state.submit();
        assert_eq!(block.tool_use_id, "toolu_abc");
        assert!(!block.is_error);
        assert_eq!(block.content, "yes");
    }

    #[test]
    fn cancel_returns_error_block() {
        let state = QuestionState::new("toolu_abc".into(), "q".into(), vec![], true);
        let block = state.cancel();
        assert_eq!(block.tool_use_id, "toolu_abc");
        assert!(block.is_error);
        assert!(block.content.contains("cancelled"));
    }

    // =========================================================================
    // from_tool_input
    // =========================================================================

    #[test]
    fn from_tool_input_parses_valid_json() {
        let input = json!({
            "question": "Which database?",
            "options": ["PostgreSQL", "MySQL", "SQLite"],
            "allow_free_text": false
        });
        let state = QuestionState::from_tool_input("toolu_1".into(), &input).unwrap();
        assert_eq!(state.question, "Which database?");
        assert_eq!(state.options.len(), 3);
        assert!(!state.allow_free_text);
    }

    #[test]
    fn from_tool_input_defaults_allow_free_text_to_true() {
        let input = json!({ "question": "What should I do?" });
        let state = QuestionState::from_tool_input("id".into(), &input).unwrap();
        assert!(state.allow_free_text);
        assert!(state.options.is_empty());
    }

    #[test]
    fn from_tool_input_returns_none_on_missing_question() {
        let input = json!({ "options": ["a"] });
        assert!(QuestionState::from_tool_input("id".into(), &input).is_none());
    }

    // =========================================================================
    // Send + Sync
    // =========================================================================

    #[test]
    fn question_state_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<QuestionState>();
    }

    #[test]
    fn question_state_is_sync() {
        fn assert_sync<T: Sync>() {}
        assert_sync::<QuestionState>();
    }
}
