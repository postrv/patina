//! Unit tests for ConversationEntry and Timeline types.
//!
//! Phase 1.1.1 (RED): These tests define the expected behavior of the unified
//! timeline type system for conversation display.

use patina::types::{ConversationEntry, Timeline};

// ============================================================================
// ConversationEntry Variant Tests
// ============================================================================

/// Tests that a UserMessage entry can be created and queried.
#[test]
fn test_user_message_creation() {
    let entry = ConversationEntry::UserMessage("Hello, Claude!".to_string());

    assert!(entry.is_user());
    assert!(!entry.is_assistant());
    assert!(!entry.is_streaming());
    assert!(!entry.is_tool_execution());
    assert_eq!(entry.text(), Some("Hello, Claude!"));
}

/// Tests that an AssistantMessage entry can be created and queried.
#[test]
fn test_assistant_message_creation() {
    let entry = ConversationEntry::AssistantMessage("I'll help you with that.".to_string());

    assert!(!entry.is_user());
    assert!(entry.is_assistant());
    assert!(!entry.is_streaming());
    assert!(!entry.is_tool_execution());
    assert_eq!(entry.text(), Some("I'll help you with that."));
}

/// Tests that a Streaming entry can be created and queried.
#[test]
fn test_streaming_entry() {
    let entry = ConversationEntry::Streaming {
        text: "Partial response...".to_string(),
        complete: false,
    };

    assert!(!entry.is_user());
    assert!(!entry.is_assistant());
    assert!(entry.is_streaming());
    assert!(!entry.is_tool_execution());
    assert_eq!(entry.text(), Some("Partial response..."));
}

/// Tests that a ToolExecution entry can be created and queried.
#[test]
fn test_tool_execution_entry() {
    let entry = ConversationEntry::ToolExecution {
        name: "bash".to_string(),
        input: "ls -la".to_string(),
        output: Some("file1.txt\nfile2.txt".to_string()),
        is_error: false,
        follows_message_idx: Some(0),
    };

    assert!(!entry.is_user());
    assert!(!entry.is_assistant());
    assert!(!entry.is_streaming());
    assert!(entry.is_tool_execution());
    // Tool executions have no direct text() - they have structured data
    assert_eq!(entry.text(), None);
}

/// Tests that ConversationEntry can derive Debug.
#[test]
fn test_conversation_entry_debug() {
    let entry = ConversationEntry::UserMessage("test".to_string());
    let debug_str = format!("{:?}", entry);
    assert!(debug_str.contains("UserMessage"));
}

/// Tests that ConversationEntry can be cloned.
#[test]
fn test_conversation_entry_clone() {
    let entry = ConversationEntry::AssistantMessage("test".to_string());
    let cloned = entry.clone();
    assert_eq!(entry.text(), cloned.text());
}

// ============================================================================
// Timeline Struct Tests
// ============================================================================

/// Tests that an empty Timeline can be created.
#[test]
fn test_timeline_new() {
    let timeline = Timeline::new();
    assert!(timeline.is_empty());
    assert_eq!(timeline.len(), 0);
}

/// Tests pushing a user message to the timeline.
#[test]
fn test_push_user_message() {
    let mut timeline = Timeline::new();
    timeline.push_user_message("Hello!");

    assert_eq!(timeline.len(), 1);
    assert!(!timeline.is_empty());

    let entries: Vec<_> = timeline.iter().collect();
    assert!(entries[0].is_user());
}

/// Tests starting and appending to a streaming entry.
#[test]
fn test_push_streaming() {
    let mut timeline = Timeline::new();
    timeline.push_streaming();

    assert_eq!(timeline.len(), 1);
    assert!(timeline.is_streaming());

    // Append text to streaming
    timeline.append_to_streaming("Hello");
    timeline.append_to_streaming(" world!");

    let entries: Vec<_> = timeline.iter().collect();
    assert_eq!(entries[0].text(), Some("Hello world!"));
}

/// Tests converting a streaming entry to a complete assistant message.
#[test]
fn test_streaming_to_message() {
    let mut timeline = Timeline::new();
    timeline.push_streaming();
    timeline.append_to_streaming("Complete response.");
    timeline.finalize_streaming_as_message();

    assert!(!timeline.is_streaming());

    let entries: Vec<_> = timeline.iter().collect();
    assert!(entries[0].is_assistant());
    assert_eq!(entries[0].text(), Some("Complete response."));
}

/// Tests adding a tool execution entry.
#[test]
fn test_push_tool_execution() {
    let mut timeline = Timeline::new();
    timeline.push_user_message("Run ls");
    timeline.push_assistant_message("I'll run that command.");
    timeline.push_tool_execution("bash", "ls -la", Some("output".to_string()), false);

    assert_eq!(timeline.len(), 3);

    let entries: Vec<_> = timeline.iter().collect();
    assert!(entries[2].is_tool_execution());
}

/// Tests that tool blocks track their associated message index.
#[test]
fn test_tool_block_follows_message_idx() {
    let mut timeline = Timeline::new();
    timeline.push_user_message("Run ls");
    timeline.push_assistant_message("I'll run that command.");
    timeline.push_tool_after_current_assistant("bash", "ls -la", Some("output".to_string()), false);

    let entries: Vec<_> = timeline.iter().collect();
    if let ConversationEntry::ToolExecution {
        follows_message_idx,
        ..
    } = &entries[2]
    {
        // Should follow message at index 1 (the assistant message)
        assert_eq!(*follows_message_idx, Some(1));
    } else {
        panic!("Expected ToolExecution entry");
    }
}

/// Tests iterating over timeline entries.
#[test]
fn test_timeline_iter() {
    let mut timeline = Timeline::new();
    timeline.push_user_message("Hello");
    timeline.push_assistant_message("Hi there!");
    timeline.push_user_message("How are you?");

    let entries: Vec<_> = timeline.iter().collect();
    assert_eq!(entries.len(), 3);
    assert!(entries[0].is_user());
    assert!(entries[1].is_assistant());
    assert!(entries[2].is_user());
}

// ============================================================================
// Streaming State Transition Tests
// ============================================================================

/// Tests the full streaming state transition cycle.
#[test]
fn test_streaming_state_transitions() {
    let mut timeline = Timeline::new();

    // Initially not streaming
    assert!(!timeline.is_streaming());

    // Start streaming
    timeline.push_streaming();
    assert!(timeline.is_streaming());

    // Append content
    timeline.append_to_streaming("text");
    assert!(timeline.is_streaming());

    // Finalize
    timeline.finalize_streaming_as_message();
    assert!(!timeline.is_streaming());
}

/// Tests that attempting to push a second streaming entry returns an error.
#[test]
fn test_double_streaming_error() {
    let mut timeline = Timeline::new();
    timeline.push_streaming();

    // Second push should fail or be a no-op
    let result = timeline.try_push_streaming();
    assert!(result.is_err());
}

/// Tests finalizing streaming as a tool_use pending state.
#[test]
fn test_finalize_streaming_as_tool_use() {
    let mut timeline = Timeline::new();
    timeline.push_streaming();
    timeline.append_to_streaming("I'll run that command.");

    // Finalize for tool use - text should be preserved for later retrieval
    let pending_text = timeline.finalize_streaming_for_tool_use();
    assert_eq!(pending_text, "I'll run that command.");
    assert!(!timeline.is_streaming());
}

/// Tests getting mutable access to streaming text.
#[test]
fn test_streaming_text_mut() {
    let mut timeline = Timeline::new();
    timeline.push_streaming();

    if let Some(text) = timeline.streaming_text_mut() {
        text.push_str("Mutated text");
    }

    let entries: Vec<_> = timeline.iter().collect();
    assert_eq!(entries[0].text(), Some("Mutated text"));
}

/// Tests that appending to streaming when not streaming is a no-op.
#[test]
fn test_append_to_streaming_no_op_when_not_streaming() {
    let mut timeline = Timeline::new();
    timeline.push_user_message("Hello");

    // This should be a no-op (or could warn)
    timeline.append_to_streaming("ignored text");

    let entries: Vec<_> = timeline.iter().collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].text(), Some("Hello"));
}
