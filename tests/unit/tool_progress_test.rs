//! Tests for tool progress indication during execution.
//!
//! These tests verify that tool execution progress is visible in the UI.

use patina::app::state::AppState;
use patina::tui::render_timeline_with_throbber;
use patina::types::ConversationEntry;
use std::path::PathBuf;

/// Helper to create a new AppState for testing.
fn new_state() -> AppState {
    AppState::new(PathBuf::from("/tmp/test"), false)
}

// ============================================================================
// 5.2.1 Tool Progress Display Tests
// ============================================================================

/// Tests that a tool block shows "Running..." when executing.
#[test]
fn test_tool_block_shows_running() {
    let mut state = new_state();

    // Add an assistant message
    state.timeline_mut().push_assistant_message("Let me check.");

    // Add tool in executing state (no output)
    state.add_tool_to_timeline_executing("bash", "pwd");

    // Render the timeline
    let lines = render_timeline_with_throbber(state.timeline(), '⠋');

    // Convert lines to string for checking
    let output: String = lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<Vec<_>>()
                .join("")
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Should show the tool with "Running..." or spinner indicator
    assert!(
        output.contains("bash") && output.contains("pwd"),
        "Should show tool name and input. Output:\n{}",
        output
    );

    // Tool without output should indicate it's still running
    let entries: Vec<_> = state.timeline().iter().collect();
    match &entries[1] {
        ConversationEntry::ToolExecution { output, .. } => {
            assert!(output.is_none(), "Executing tool should have no output yet");
        }
        _ => panic!("Expected ToolExecution entry"),
    }
}

/// Tests that a tool block updates when execution completes.
#[test]
fn test_tool_block_updates_on_complete() {
    let mut state = new_state();

    // Add an assistant message
    state.timeline_mut().push_assistant_message("Let me check.");

    // Add tool in executing state
    state.add_tool_to_timeline_executing("bash", "pwd");

    // Verify it's executing (no output)
    {
        let entries: Vec<_> = state.timeline().iter().collect();
        match &entries[1] {
            ConversationEntry::ToolExecution { output, .. } => {
                assert!(output.is_none());
            }
            _ => panic!("Expected ToolExecution entry"),
        }
    }

    // Complete the tool
    state.update_tool_in_timeline("bash", Some("/home/user".to_string()), false);

    // Verify it's complete with output
    let entries: Vec<_> = state.timeline().iter().collect();
    match &entries[1] {
        ConversationEntry::ToolExecution {
            output, is_error, ..
        } => {
            assert_eq!(output.as_deref(), Some("/home/user"));
            assert!(!is_error);
        }
        _ => panic!("Expected ToolExecution entry"),
    }

    // Render the timeline
    let lines = render_timeline_with_throbber(state.timeline(), '⠋');
    let output: String = lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<Vec<_>>()
                .join("")
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Should show the result
    assert!(
        output.contains("/home/user"),
        "Should show tool result. Output:\n{}",
        output
    );
}

/// Tests that tool error is displayed correctly.
#[test]
fn test_tool_block_shows_error() {
    let mut state = new_state();

    // Add an assistant message
    state
        .timeline_mut()
        .push_assistant_message("Let me run that.");

    // Add tool in executing state
    state.add_tool_to_timeline_executing("bash", "invalid_cmd");

    // Complete with error
    state.update_tool_in_timeline("bash", Some("command not found".to_string()), true);

    // Verify error is marked
    let entries: Vec<_> = state.timeline().iter().collect();
    match &entries[1] {
        ConversationEntry::ToolExecution {
            output, is_error, ..
        } => {
            assert_eq!(output.as_deref(), Some("command not found"));
            assert!(is_error, "Should be marked as error");
        }
        _ => panic!("Expected ToolExecution entry"),
    }
}

/// Tests that multiple tools can be in different states.
#[test]
fn test_multiple_tools_different_states() {
    let mut state = new_state();

    // Add assistant message
    state
        .timeline_mut()
        .push_assistant_message("Running commands.");

    // Add first tool (executing)
    state.add_tool_to_timeline_executing("bash", "pwd");

    // Add second tool (also executing)
    state.add_tool_to_timeline_executing("bash", "ls");

    // Complete a tool - update_tool_in_timeline finds the MOST RECENT matching tool with no output
    // (iterates in reverse), so this will update "ls" (the second one)
    state.update_tool_in_timeline("bash", Some("files".to_string()), false);

    // Verify states
    let entries: Vec<_> = state.timeline().iter().collect();

    // First tool (pwd) should still be executing since update found "ls" first (reverse order)
    match &entries[1] {
        ConversationEntry::ToolExecution { output, input, .. } => {
            assert_eq!(input, "pwd");
            assert!(output.is_none(), "First tool should still be executing");
        }
        _ => panic!("Expected ToolExecution entry"),
    }

    // Second tool (ls) should be complete since it was found first in reverse iteration
    match &entries[2] {
        ConversationEntry::ToolExecution { output, input, .. } => {
            assert_eq!(input, "ls");
            assert_eq!(output.as_deref(), Some("files"));
        }
        _ => panic!("Expected ToolExecution entry"),
    }

    // Now complete the first tool
    state.update_tool_in_timeline("bash", Some("/home/user".to_string()), false);

    let entries: Vec<_> = state.timeline().iter().collect();
    match &entries[1] {
        ConversationEntry::ToolExecution { output, input, .. } => {
            assert_eq!(input, "pwd");
            assert_eq!(output.as_deref(), Some("/home/user"));
        }
        _ => panic!("Expected ToolExecution entry"),
    }
}

/// Tests that throbber is shown during tool execution.
#[test]
fn test_throbber_shown_during_execution() {
    let mut state = new_state();

    // Add assistant streaming with tool
    state.set_streaming(true);
    state.append_streaming_text("Running...");

    // The throbber should be animated during streaming
    let char1 = state.throbber_char();
    state.tick_throbber();
    let char2 = state.throbber_char();

    // Throbber should animate (different characters after tick)
    assert_ne!(char1, char2, "Throbber should animate");
}

/// Tests that progress tracking uses executing_tool_ids set.
#[test]
fn test_executing_tools_tracking() {
    let mut state = new_state();

    // Initially no tools executing
    assert!(!state.has_executing_tools());

    // Mark a tool as executing
    state.mark_tool_executing("tool_1");
    assert!(state.has_executing_tools());

    // Mark another tool
    state.mark_tool_executing("tool_2");
    assert!(state.has_executing_tools());

    // Complete one tool (simulated through record_tool_result)
    let result = patina::types::ToolResultBlock {
        tool_use_id: "tool_1".to_string(),
        content: "done".to_string(),
        is_error: false,
    };
    state.record_tool_result("tool_1", result);

    // Still have one executing
    assert!(state.has_executing_tools());

    // Complete the other
    let result2 = patina::types::ToolResultBlock {
        tool_use_id: "tool_2".to_string(),
        content: "done too".to_string(),
        is_error: false,
    };
    state.record_tool_result("tool_2", result2);

    // Now none executing
    assert!(!state.has_executing_tools());
}
