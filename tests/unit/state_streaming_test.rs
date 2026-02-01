//! Tests for streaming display fix.
//!
//! Phase 3 (RED/GREEN): These tests verify that:
//! - Tool use messages don't cause duplicates
//! - Message ordering is correct with tool blocks
//! - Scroll auto-follow works during streaming

use patina::app::state::AppState;
use patina::types::config::ParallelMode;
use patina::types::{ConversationEntry, Role, StopReason, StreamEvent};
use std::path::PathBuf;

/// Helper to check if scroll is in auto-follow mode.
fn is_auto_scrolling(state: &AppState) -> bool {
    state.scroll_state().mode().should_auto_scroll()
}

// ============================================================================
// Tool Use Message Handling Tests
// ============================================================================

/// Tests that tool_use completion does NOT add a duplicate assistant message.
///
/// Scenario:
/// 1. User sends "run ls"
/// 2. Assistant starts streaming "I'll run that command."
/// 3. MessageComplete arrives with stop_reason=tool_use
///
/// Expected: The assistant message is finalized in timeline but NOT yet added
/// to API messages (handle_tool_execution will add it later with proper tool_use blocks).
#[test]
fn test_tool_use_no_duplicate_message() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true, ParallelMode::Enabled);

    // Start streaming
    state.set_streaming(true);
    state
        .append_chunk(StreamEvent::ContentDelta(
            "I'll run that command.".to_string(),
        ))
        .unwrap();

    let timeline_len_before = state.timeline().len();

    // Complete with tool_use stop reason
    state
        .append_chunk(StreamEvent::MessageComplete {
            stop_reason: StopReason::ToolUse,
        })
        .unwrap();

    // Timeline length should remain the same (streaming entry converted to assistant)
    // The timeline should have the streaming entry finalized
    assert_eq!(
        state.timeline().len(),
        timeline_len_before,
        "Timeline length should remain unchanged (streaming becomes assistant)"
    );

    // The entry should now be an assistant message
    let entries: Vec<_> = state.timeline().iter().collect();
    assert!(
        entries.last().is_some_and(|e| e.is_assistant()),
        "Last entry should be an assistant message"
    );
}

/// Tests that normal message completion DOES add the assistant message.
#[test]
fn test_normal_completion_adds_message() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true, ParallelMode::Enabled);

    // Initialize streaming
    state.set_streaming(true);

    // Stream content
    state
        .append_chunk(StreamEvent::ContentDelta("Here's my response.".to_string()))
        .unwrap();

    let timeline_len_before = state.timeline().len();

    // Complete with normal end_turn stop reason
    state
        .append_chunk(StreamEvent::MessageComplete {
            stop_reason: StopReason::EndTurn,
        })
        .unwrap();

    // Timeline should have the assistant message finalized (same length, converted entry)
    assert_eq!(
        state.timeline().len(),
        timeline_len_before,
        "Timeline length should remain unchanged (streaming becomes assistant)"
    );

    let entries: Vec<_> = state.timeline().iter().collect();
    let last = entries.last().unwrap();
    assert!(matches!(
        last,
        ConversationEntry::AssistantMessage(s) if s == "Here's my response."
    ));
}

/// Tests that streaming updates the timeline.
#[test]
fn test_streaming_updates_timeline() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true, ParallelMode::Enabled);

    // Initialize streaming
    state.set_streaming(true);

    // Stream content
    state
        .append_chunk(StreamEvent::ContentDelta("Hello".to_string()))
        .unwrap();
    state
        .append_chunk(StreamEvent::ContentDelta(" world!".to_string()))
        .unwrap();

    // Verify timeline streaming entry updated
    assert!(state.timeline().is_streaming());
    let entries: Vec<_> = state.timeline().iter().collect();
    assert_eq!(entries[0].text(), Some("Hello world!"));
}

// ============================================================================
// Message Ordering Tests
// ============================================================================

/// Tests that after tool execution, the order is correct:
/// [assistant_text, tool_blocks, continuation]
///
/// This test simulates what handle_tool_execution should produce.
#[test]
fn test_message_ordering_with_tools() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true, ParallelMode::Enabled);

    // Simulate the correct ordering that handle_tool_execution should produce:
    // 1. User message
    state.add_message(patina::Message {
        role: Role::User,
        content: "Run ls".to_string(),
    });

    // 2. Assistant message (text before tool call)
    state.add_message(patina::Message {
        role: Role::Assistant,
        content: "I'll run that command.".to_string(),
    });

    // 3. Tool block (with result)
    state.add_tool_block_with_result("bash", "ls -la", "file1.txt", false);

    // Verify ordering in timeline
    let entries: Vec<_> = state.timeline().iter().collect();

    assert_eq!(entries.len(), 3);
    assert!(entries[0].is_user(), "First entry should be user message");
    assert!(
        entries[1].is_assistant(),
        "Second entry should be assistant message"
    );
    assert!(
        entries[2].is_tool_execution(),
        "Third entry should be tool execution"
    );
}

/// Tests that multiple tool calls maintain correct ordering.
#[test]
fn test_multiple_tools_ordering() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true, ParallelMode::Enabled);

    // User message
    state.add_message(patina::Message {
        role: Role::User,
        content: "List and show README".to_string(),
    });

    // Assistant message
    state.add_message(patina::Message {
        role: Role::Assistant,
        content: "I'll help with that.".to_string(),
    });

    // First tool
    state.add_tool_block_with_result("bash", "ls", "README.md", false);

    // Second tool
    state.add_tool_block_with_result("read_file", "README.md", "# Project", false);

    // Verify ordering
    let entries: Vec<_> = state.timeline().iter().collect();
    assert_eq!(entries.len(), 4);
    assert!(entries[0].is_user());
    assert!(entries[1].is_assistant());
    assert!(entries[2].is_tool_execution());
    assert!(entries[3].is_tool_execution());
}

// ============================================================================
// Scroll Auto-Follow Tests
// ============================================================================

/// Tests that adding a message triggers scroll to bottom when in auto-follow mode.
#[test]
fn test_scroll_follows_new_message() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true, ParallelMode::Enabled);

    // Set up some content height for scroll to matter
    state.update_content_height(100);

    // Enable auto-scroll mode (scroll starts in Follow mode by default)
    state.scroll_to_bottom(100);

    // Add a message
    state.add_message(patina::Message {
        role: Role::User,
        content: "Hello".to_string(),
    });

    // Scroll should still be at bottom (auto-following)
    assert!(
        is_auto_scrolling(&state),
        "Scroll should auto-follow after adding message"
    );
}

/// Tests that scroll follows during streaming.
#[test]
fn test_scroll_follows_streaming() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true, ParallelMode::Enabled);

    // Set up content height
    state.update_content_height(100);

    // Enable auto-scroll (starts in Follow mode by default)
    state.scroll_to_bottom(100);

    // Start streaming and append content
    state.set_streaming(true);
    state.append_streaming_text("Hello");
    state.append_streaming_text(" world!");

    // Scroll should still be following
    assert!(
        is_auto_scrolling(&state),
        "Scroll should auto-follow during streaming"
    );
}

/// Tests that adding tool blocks triggers scroll follow.
#[test]
fn test_scroll_follows_tool_block() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true, ParallelMode::Enabled);

    // Set up content height
    state.update_content_height(100);

    // Enable auto-scroll (starts in Follow mode by default)
    state.scroll_to_bottom(100);

    // Add an assistant message first
    state.add_message(patina::Message {
        role: Role::Assistant,
        content: "Running command...".to_string(),
    });

    // Add tool block
    state.add_tool_block_with_result("bash", "ls", "output", false);

    // Scroll should still be following
    assert!(
        is_auto_scrolling(&state),
        "Scroll should auto-follow after tool block"
    );
}

// ============================================================================
// Edge Cases
// ============================================================================

/// Tests that MessageStop also doesn't cause issues with timeline.
#[test]
fn test_message_stop_updates_timeline() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true, ParallelMode::Enabled);

    // Initialize streaming
    state.set_streaming(true);

    // Stream content
    state
        .append_chunk(StreamEvent::ContentDelta("Response text.".to_string()))
        .unwrap();

    // Use MessageStop instead of MessageComplete
    state.append_chunk(StreamEvent::MessageStop).unwrap();

    // Should have finalized the assistant message in timeline
    let entries: Vec<_> = state.timeline().iter().collect();
    assert!(!entries.is_empty());
    assert!(matches!(
        entries.last().unwrap(),
        ConversationEntry::AssistantMessage(s) if s == "Response text."
    ));
}

/// Tests that empty streaming content handles gracefully.
#[test]
fn test_empty_streaming_content() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true, ParallelMode::Enabled);

    // Initialize streaming
    state.set_streaming(true);

    // Complete without any content
    state
        .append_chunk(StreamEvent::MessageComplete {
            stop_reason: StopReason::EndTurn,
        })
        .unwrap();

    // Should handle gracefully (might or might not add empty message)
    // The important thing is no panic
}

// ============================================================================
// Duplicate Message Prevention Tests (Issue #1)
// ============================================================================

/// Tests that when both MessageComplete and MessageStop fire, only ONE message is added.
///
/// Root cause of duplicate bug: Both events were calling finalize_streaming_as_message()
/// and adding to api_messages. The fix is to guard MessageStop so it only processes
/// if the timeline is still streaming.
#[test]
fn test_message_complete_and_stop_no_duplicate() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true, ParallelMode::Enabled);

    // Initialize streaming
    state.set_streaming(true);

    // Stream content
    state
        .append_chunk(StreamEvent::ContentDelta("Test response.".to_string()))
        .unwrap();

    let api_msg_count_before = state.api_messages_len();

    // First: MessageComplete fires
    state
        .append_chunk(StreamEvent::MessageComplete {
            stop_reason: StopReason::EndTurn,
        })
        .unwrap();

    let api_msg_count_after_complete = state.api_messages_len();

    // Second: MessageStop fires (should be a no-op now)
    state.append_chunk(StreamEvent::MessageStop).unwrap();

    let api_msg_count_after_stop = state.api_messages_len();

    // Only ONE message should have been added
    assert_eq!(
        api_msg_count_after_complete - api_msg_count_before,
        1,
        "MessageComplete should add exactly one message"
    );
    assert_eq!(
        api_msg_count_after_stop, api_msg_count_after_complete,
        "MessageStop should NOT add another message (duplicate prevention)"
    );
}

/// Tests that MessageStop alone still works correctly.
#[test]
fn test_message_stop_alone_works() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true, ParallelMode::Enabled);

    // Initialize streaming
    state.set_streaming(true);

    // Stream content
    state
        .append_chunk(StreamEvent::ContentDelta("Response via stop.".to_string()))
        .unwrap();

    let api_msg_count_before = state.api_messages_len();

    // Only MessageStop fires (no MessageComplete)
    state.append_chunk(StreamEvent::MessageStop).unwrap();

    let api_msg_count_after = state.api_messages_len();

    // One message should have been added
    assert_eq!(
        api_msg_count_after - api_msg_count_before,
        1,
        "MessageStop alone should add one message"
    );

    // Verify the message content in timeline
    let entries: Vec<_> = state.timeline().iter().collect();
    assert!(matches!(
        entries.last().unwrap(),
        ConversationEntry::AssistantMessage(s) if s == "Response via stop."
    ));
}

/// Tests that tool_use responses don't duplicate either with both events.
#[test]
fn test_tool_use_no_duplicate_with_both_events() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true, ParallelMode::Enabled);

    // Initialize streaming
    state.set_streaming(true);

    // Stream content
    state
        .append_chunk(StreamEvent::ContentDelta("Running tool...".to_string()))
        .unwrap();

    let api_msg_count_before = state.api_messages_len();

    // MessageComplete with tool_use (doesn't add to api_messages - deferred to handle_tool_execution)
    state
        .append_chunk(StreamEvent::MessageComplete {
            stop_reason: StopReason::ToolUse,
        })
        .unwrap();

    let api_msg_count_after_complete = state.api_messages_len();

    // MessageStop follows
    state.append_chunk(StreamEvent::MessageStop).unwrap();

    let api_msg_count_after_stop = state.api_messages_len();

    // Tool use doesn't add to API immediately (handled by tool loop)
    assert_eq!(
        api_msg_count_after_complete, api_msg_count_before,
        "tool_use MessageComplete should not add to api_messages yet"
    );
    assert_eq!(
        api_msg_count_after_stop, api_msg_count_after_complete,
        "MessageStop should not add duplicate for tool_use either"
    );
}
