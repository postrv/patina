//! Tests for Timeline integration with AppState.
//!
//! These tests verify that the Timeline is properly integrated with AppState
//! and serves as the single source of truth for conversation display.

use patina::app::state::AppState;
use patina::types::{ConversationEntry, Role};
use std::path::PathBuf;

// ============================================================================
// Timeline Integration Tests
// ============================================================================

/// Tests that AppState exposes timeline accessor.
#[test]
fn test_appstate_has_timeline() {
    let state = AppState::new(PathBuf::from("/tmp"), true);
    let timeline = state.timeline();
    assert!(timeline.is_empty());
}

/// Tests that AppState exposes mutable timeline accessor.
#[test]
fn test_appstate_timeline_mut() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true);
    state.timeline_mut().push_user_message("Hello");
    assert_eq!(state.timeline().len(), 1);
}

// ============================================================================
// User Message Tests
// ============================================================================

/// Tests that adding a user message updates the timeline.
#[test]
fn test_add_user_message_updates_timeline() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true);

    // Use the state's add_message method
    state.add_message(patina::Message {
        role: Role::User,
        content: "Hello, Claude!".to_string(),
    });

    // Verify timeline updated
    assert_eq!(state.timeline().len(), 1);
    let entry = &state.timeline().entries()[0];
    assert!(entry.is_user());
    assert_eq!(entry.text(), Some("Hello, Claude!"));
}

// ============================================================================
// Streaming Tests
// ============================================================================

/// Tests that streaming updates the timeline.
#[test]
fn test_streaming_updates_timeline() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true);

    // Start streaming - this should create a streaming entry in timeline
    state.set_streaming(true);

    // Verify timeline has streaming entry
    assert!(state.timeline().is_streaming());

    // Append content
    state.append_streaming_text("Hello");
    state.append_streaming_text(" world!");

    // Verify timeline streaming text updated
    let entries: Vec<_> = state.timeline().iter().collect();
    assert_eq!(entries[0].text(), Some("Hello world!"));
}

/// Tests that completing streaming updates the timeline.
#[test]
fn test_streaming_complete_updates_timeline() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true);

    // Start and complete streaming as a normal message
    state.set_streaming(true);
    state.append_streaming_text("Complete response.");
    state.finalize_streaming_as_message();

    // Verify timeline updated (no longer streaming, now assistant message)
    assert!(!state.timeline().is_streaming());
    let entries: Vec<_> = state.timeline().iter().collect();
    assert!(entries[0].is_assistant());
    assert_eq!(entries[0].text(), Some("Complete response."));
}

// ============================================================================
// Tool Execution Tests
// ============================================================================

/// Tests that adding a tool block updates the timeline.
#[test]
fn test_tool_block_added_to_timeline() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true);

    // First add an assistant message (tool blocks follow assistant messages)
    state.add_message(patina::Message {
        role: Role::Assistant,
        content: "I'll run that command.".to_string(),
    });

    // Add a tool block
    state.add_tool_block_with_result("bash", "ls -la", "file1.txt\nfile2.txt", false);

    // Verify legacy tool_blocks updated
    assert_eq!(state.tool_blocks().len(), 1);
    assert_eq!(state.tool_blocks()[0].tool_name(), "bash");

    // Verify timeline updated
    assert_eq!(state.timeline().len(), 2); // assistant + tool
    let entries: Vec<_> = state.timeline().iter().collect();
    assert!(entries[0].is_assistant());
    assert!(entries[1].is_tool_execution());
}

/// Tests that tool blocks track the correct assistant message index.
#[test]
fn test_tool_block_follows_assistant_message() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true);

    // Add user message
    state.add_message(patina::Message {
        role: Role::User,
        content: "Run ls".to_string(),
    });

    // Add assistant message at index 1
    state.add_message(patina::Message {
        role: Role::Assistant,
        content: "I'll run that.".to_string(),
    });

    // Add tool block - should follow the assistant message (index 1)
    state.add_tool_block_with_result("bash", "ls", "output", false);

    // Verify tool block follows assistant at index 1
    let entries: Vec<_> = state.timeline().iter().collect();
    if let ConversationEntry::ToolExecution {
        follows_message_idx,
        ..
    } = &entries[2]
    {
        assert_eq!(follows_message_idx, &Some(1));
    } else {
        panic!("Expected ToolExecution entry");
    }
}

// ============================================================================
// Edge Case Tests
// ============================================================================

/// Tests that clearing conversation clears the timeline.
#[test]
fn test_clear_conversation_clears_timeline() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true);

    state.add_message(patina::Message {
        role: Role::User,
        content: "Hello".to_string(),
    });

    // Clear everything
    state.clear_conversation();

    // Timeline should be empty
    assert!(state.timeline().is_empty());
}

/// Tests converting AppState to Session preserves messages.
#[test]
fn test_to_session_preserves_messages() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true);

    state.add_message(patina::Message {
        role: Role::User,
        content: "Hello".to_string(),
    });
    state.add_message(patina::Message {
        role: Role::Assistant,
        content: "Hi!".to_string(),
    });

    // Convert to session and verify messages are preserved
    let session = state.to_session();
    assert_eq!(session.messages().len(), 2);
}
