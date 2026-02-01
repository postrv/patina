//! Unit tests for application state management.
//!
//! These tests verify input handling, scroll behavior, and dirty flag tracking.
//! Following TDD RED phase - cursor movement tests will fail until implemented.

use patina::app::state::AppState;
use patina::types::config::ParallelMode;
use std::path::PathBuf;

/// Helper to create a new AppState for testing.
fn new_state() -> AppState {
    AppState::new(PathBuf::from("/tmp/test"), false, ParallelMode::Enabled)
}

// ============================================================================
// Input Handling Tests
// ============================================================================

/// Tests basic character insertion into the input buffer.
#[test]
fn test_input_insert_char() {
    let mut state = new_state();
    assert!(state.input.is_empty());

    state.insert_char('H');
    state.insert_char('i');

    assert_eq!(state.input, "Hi");
}

/// Tests character deletion from the input buffer.
#[test]
fn test_input_delete_char() {
    let mut state = new_state();
    state.insert_char('H');
    state.insert_char('e');
    state.insert_char('l');
    state.insert_char('l');
    state.insert_char('o');

    state.delete_char();
    assert_eq!(state.input, "Hell");

    state.delete_char();
    state.delete_char();
    assert_eq!(state.input, "He");
}

/// Tests that delete_char on empty input doesn't panic.
#[test]
fn test_input_delete_char_empty() {
    let mut state = new_state();
    assert!(state.input.is_empty());

    // Should not panic
    state.delete_char();
    assert!(state.input.is_empty());
}

/// Tests that take_input returns the content and clears the buffer.
#[test]
fn test_input_take() {
    let mut state = new_state();
    state.insert_char('T');
    state.insert_char('e');
    state.insert_char('s');
    state.insert_char('t');

    let taken = state.take_input();
    assert_eq!(taken, "Test");
    assert!(state.input.is_empty());
}

/// Tests that take_input on empty buffer returns empty string.
#[test]
fn test_input_take_empty() {
    let mut state = new_state();
    let taken = state.take_input();
    assert!(taken.is_empty());
    assert!(state.input.is_empty());
}

/// Tests unicode character handling in input.
#[test]
fn test_input_unicode() {
    let mut state = new_state();
    state.insert_char('你');
    state.insert_char('好');
    state.insert_char('🦀');

    assert_eq!(state.input, "你好🦀");

    state.delete_char();
    assert_eq!(state.input, "你好");
}

// ============================================================================
// Scroll Bounds Tests
// ============================================================================

/// Tests scroll up increases scroll offset.
#[test]
fn test_scroll_up() {
    let mut state = new_state();
    // Set content larger than viewport to allow scrolling
    state.set_viewport_height(20);
    state.update_content_height(100);
    assert_eq!(state.scroll_offset(), 0);

    state.scroll_up(5);
    assert_eq!(state.scroll_offset(), 5);

    state.scroll_up(3);
    assert_eq!(state.scroll_offset(), 8);
}

/// Tests scroll down decreases scroll offset.
#[test]
fn test_scroll_down() {
    let mut state = new_state();
    // Set content larger than viewport to allow scrolling
    state.set_viewport_height(20);
    state.update_content_height(100);

    state.scroll_up(10);
    assert_eq!(state.scroll_offset(), 10);

    state.scroll_down(3);
    assert_eq!(state.scroll_offset(), 7);

    state.scroll_down(2);
    assert_eq!(state.scroll_offset(), 5);
}

/// Tests scroll bounds saturation - scroll_down at 0 stays at 0.
#[test]
fn test_scroll_bounds_saturation_at_zero() {
    let mut state = new_state();
    assert_eq!(state.scroll_offset(), 0);

    // Should not go negative - saturating_sub should keep it at 0
    state.scroll_down(10);
    assert_eq!(state.scroll_offset(), 0);
}

/// Tests scroll up saturation - large values don't overflow.
#[test]
fn test_scroll_up_large_values() {
    let mut state = new_state();

    state.scroll_up(usize::MAX / 2);
    let first = state.scroll_offset();

    // saturating_add should prevent overflow
    state.scroll_up(usize::MAX / 2);
    assert!(state.scroll_offset() >= first);
}

// ============================================================================
// Dirty Flag Tests
// ============================================================================

/// Tests that new state needs initial render.
#[test]
fn test_dirty_flag_initial_state() {
    let state = new_state();
    assert!(state.needs_render(), "New state should need initial render");
}

/// Tests that mark_rendered clears dirty flag.
#[test]
fn test_dirty_flag_mark_rendered() {
    let mut state = new_state();
    assert!(state.needs_render());

    state.mark_rendered();
    assert!(
        !state.needs_render(),
        "After mark_rendered, should not need render"
    );
}

/// Tests that input changes set dirty flag.
#[test]
fn test_dirty_flag_on_input() {
    let mut state = new_state();
    state.mark_rendered();
    assert!(!state.needs_render());

    state.insert_char('a');
    assert!(state.needs_render(), "insert_char should set dirty flag");

    state.mark_rendered();
    state.delete_char();
    assert!(state.needs_render(), "delete_char should set dirty flag");

    state.mark_rendered();
    state.take_input();
    assert!(state.needs_render(), "take_input should set dirty flag");
}

/// Tests that scroll changes set dirty flag.
#[test]
fn test_dirty_flag_on_scroll() {
    let mut state = new_state();
    state.mark_rendered();
    assert!(!state.needs_render());

    state.scroll_up(1);
    assert!(state.needs_render(), "scroll_up should set dirty flag");

    state.mark_rendered();
    state.scroll_down(1);
    assert!(state.needs_render(), "scroll_down should set dirty flag");
}

/// Tests that mark_full_redraw sets dirty flag.
#[test]
fn test_dirty_flag_full_redraw() {
    let mut state = new_state();
    state.mark_rendered();
    assert!(!state.needs_render());

    state.mark_full_redraw();
    assert!(
        state.needs_render(),
        "mark_full_redraw should set dirty flag"
    );
}

/// Tests that throbber tick sets dirty flag.
#[test]
fn test_dirty_flag_on_throbber() {
    let mut state = new_state();
    state.mark_rendered();
    assert!(!state.needs_render());

    state.tick_throbber();
    assert!(state.needs_render(), "tick_throbber should set dirty flag");
}

/// Tests that adding a message sets dirty flag.
#[test]
fn test_dirty_flag_on_message_add() {
    use patina::types::{Message, Role};

    let mut state = new_state();
    state.mark_rendered();
    assert!(!state.needs_render());

    state.add_message(Message {
        role: Role::User,
        content: "Hello".to_string(),
    });

    assert!(state.needs_render(), "add_message should set dirty flag");
    assert_eq!(state.timeline().len(), 1);
}

// ============================================================================
// Cursor Movement Tests
// ============================================================================

/// Tests that cursor position is tracked.
/// Initial cursor should be at position 0 (end of empty string).
#[test]
fn test_cursor_initial_position() {
    let state = new_state();
    assert_eq!(state.cursor_position(), 0);
}

/// Tests cursor position after inserting characters.
/// Cursor should be at the end after each insert.
#[test]
fn test_cursor_position_after_insert() {
    let mut state = new_state();
    state.insert_char('a');
    assert_eq!(state.cursor_position(), 1);

    state.insert_char('b');
    state.insert_char('c');
    assert_eq!(state.cursor_position(), 3);
}

/// Tests moving cursor left.
#[test]
fn test_cursor_move_left() {
    let mut state = new_state();
    state.insert_char('a');
    state.insert_char('b');
    state.insert_char('c');

    state.cursor_left();
    assert_eq!(state.cursor_position(), 2);

    state.cursor_left();
    assert_eq!(state.cursor_position(), 1);
}

/// Tests cursor left at beginning doesn't go negative.
#[test]
fn test_cursor_move_left_at_start() {
    let mut state = new_state();
    state.insert_char('a');
    state.cursor_left();
    assert_eq!(state.cursor_position(), 0);

    // Should not go negative
    state.cursor_left();
    assert_eq!(state.cursor_position(), 0);
}

/// Tests moving cursor right.
#[test]
fn test_cursor_move_right() {
    let mut state = new_state();
    state.insert_char('a');
    state.insert_char('b');
    state.insert_char('c');

    // Move to start first
    state.cursor_left();
    state.cursor_left();
    state.cursor_left();
    assert_eq!(state.cursor_position(), 0);

    state.cursor_right();
    assert_eq!(state.cursor_position(), 1);

    state.cursor_right();
    assert_eq!(state.cursor_position(), 2);
}

/// Tests cursor right at end doesn't exceed length.
#[test]
fn test_cursor_move_right_at_end() {
    let mut state = new_state();
    state.insert_char('a');
    assert_eq!(state.cursor_position(), 1);

    // Should not exceed string length
    state.cursor_right();
    assert_eq!(state.cursor_position(), 1);
}

/// Tests inserting at cursor position (not at end).
#[test]
fn test_insert_at_cursor() {
    let mut state = new_state();
    state.insert_char('a');
    state.insert_char('c');
    // Input: "ac", cursor at 2

    state.cursor_left(); // cursor at 1
    state.insert_char('b'); // insert 'b' at position 1

    assert_eq!(state.input, "abc");
    assert_eq!(state.cursor_position(), 2); // cursor moves after inserted char
}

/// Tests deleting at cursor position (backspace behavior).
#[test]
fn test_delete_at_cursor() {
    let mut state = new_state();
    state.insert_char('a');
    state.insert_char('b');
    state.insert_char('c');
    // Input: "abc", cursor at 3

    state.cursor_left(); // cursor at 2
    state.delete_char(); // delete 'b' (char before cursor)

    assert_eq!(state.input, "ac");
    assert_eq!(state.cursor_position(), 1);
}

/// Tests cursor home (move to start).
#[test]
fn test_cursor_home() {
    let mut state = new_state();
    state.insert_char('a');
    state.insert_char('b');
    state.insert_char('c');

    state.cursor_home();
    assert_eq!(state.cursor_position(), 0);
}

/// Tests cursor end (move to end).
#[test]
fn test_cursor_end() {
    let mut state = new_state();
    state.insert_char('a');
    state.insert_char('b');
    state.insert_char('c');
    state.cursor_home();

    state.cursor_end();
    assert_eq!(state.cursor_position(), 3);
}

// ============================================================================
// Stream Chunk Tests
// ============================================================================

/// Tests append_chunk with content delta.
#[test]
fn test_append_chunk_content_delta() {
    use patina::types::StreamEvent;

    let mut state = new_state();
    state.mark_rendered();

    // Simulate starting a response
    state.set_streaming(true);

    let result = state.append_chunk(StreamEvent::ContentDelta("Hello ".to_string()));
    assert!(result.is_ok());

    // Verify content in timeline streaming entry
    let entries: Vec<_> = state.timeline().iter().collect();
    assert_eq!(entries[0].text(), Some("Hello "));
    assert!(state.needs_render());
}

/// Tests append_chunk accumulates content.
#[test]
fn test_append_chunk_accumulates_content() {
    use patina::types::StreamEvent;

    let mut state = new_state();
    state.set_streaming(true);

    state
        .append_chunk(StreamEvent::ContentDelta("Hello ".to_string()))
        .unwrap();
    state
        .append_chunk(StreamEvent::ContentDelta("World!".to_string()))
        .unwrap();

    // Verify content in timeline streaming entry
    let entries: Vec<_> = state.timeline().iter().collect();
    assert_eq!(entries[0].text(), Some("Hello World!"));
}

/// Tests append_chunk message stop finalizes the response.
#[test]
fn test_append_chunk_message_stop() {
    use patina::types::{ConversationEntry, StreamEvent};

    let mut state = new_state();
    state.set_streaming(true);
    state
        .append_chunk(StreamEvent::ContentDelta("Test response".to_string()))
        .unwrap();
    state.mark_rendered();

    let result = state.append_chunk(StreamEvent::MessageStop);
    assert!(result.is_ok());

    // Response should be finalized in timeline as assistant message
    assert!(!state.is_loading());
    let entries: Vec<_> = state.timeline().iter().collect();
    assert_eq!(entries.len(), 1);
    assert!(matches!(
        entries[0],
        ConversationEntry::AssistantMessage(s) if s == "Test response"
    ));
    assert!(state.needs_render());
}

/// Tests append_chunk error sets dirty flag.
#[test]
fn test_append_chunk_error() {
    use patina::types::StreamEvent;

    let mut state = new_state();
    state.set_streaming(true);
    state
        .append_chunk(StreamEvent::ContentDelta("Partial response".to_string()))
        .unwrap();
    state.mark_rendered();

    let result = state.append_chunk(StreamEvent::Error("Connection error".to_string()));
    assert!(result.is_ok());

    // After error, loading should be cleared (verified by is_loading)
    assert!(!state.is_loading());
    assert!(state.needs_render());
}

/// Tests is_loading returns false initially.
#[test]
fn test_is_loading_initial() {
    let state = new_state();
    assert!(!state.is_loading());
}

// ============================================================================
// P0-1: Tool Use Response Deduplication Tests
// ============================================================================

/// Tests that tool_use responses finalize streaming but don't add to API messages yet.
///
/// When a MessageComplete with stop_reason=ToolUse is received:
/// 1. Streaming entry is finalized in timeline
/// 2. The text should be stored in tool_loop for later use
/// 3. handle_tool_execution() will add the proper API message with tool_use blocks
#[test]
fn test_tool_use_response_not_added_to_display_by_append_chunk() {
    use patina::api::StreamEvent;
    use patina::types::content::StopReason;

    let mut state = new_state();
    state.mark_rendered();

    // Start streaming
    state.tool_loop_mut().start_streaming().unwrap();
    state.set_streaming(true);

    // Simulate streaming text via ContentDelta
    state
        .append_chunk(StreamEvent::ContentDelta("I'll help you.".to_string()))
        .unwrap();

    // Simulate tool_use events
    state.handle_tool_use_start("toolu_123".to_string(), "bash".to_string(), 0);
    state.handle_tool_use_input_delta(0, r#"{"command":"ls"}"#);
    state.handle_tool_use_complete(0).unwrap();

    // Record timeline length before MessageComplete
    let timeline_len_before = state.timeline().len();

    // Complete with tool_use stop reason
    state
        .append_chunk(StreamEvent::MessageComplete {
            stop_reason: StopReason::ToolUse,
        })
        .unwrap();

    // Timeline length should remain the same (streaming converted to assistant)
    assert_eq!(
        state.timeline().len(),
        timeline_len_before,
        "timeline length should remain unchanged (streaming becomes assistant)"
    );

    // The text should be stored in tool_loop for later use
    assert_eq!(state.tool_loop().text_content(), "I'll help you.");
}

/// Tests that normal (non-tool_use) responses are finalized in timeline.
#[test]
fn test_normal_response_added_to_display_by_append_chunk() {
    use patina::api::StreamEvent;
    use patina::types::content::StopReason;
    use patina::types::ConversationEntry;

    let mut state = new_state();
    state.mark_rendered();

    // Start streaming
    state.tool_loop_mut().start_streaming().unwrap();
    state.set_streaming(true);
    state
        .append_chunk(StreamEvent::ContentDelta("Here's my response.".to_string()))
        .unwrap();

    // Complete with EndTurn (normal response)
    state
        .append_chunk(StreamEvent::MessageComplete {
            stop_reason: StopReason::EndTurn,
        })
        .unwrap();

    // Normal responses should be finalized in timeline
    let entries: Vec<_> = state.timeline().iter().collect();
    assert!(!entries.is_empty());
    assert!(matches!(
        entries.last().unwrap(),
        ConversationEntry::AssistantMessage(s) if s == "Here's my response."
    ));
}

/// Tests that tool_loop text content is preserved during streaming.
#[test]
fn test_tool_loop_preserves_text_content() {
    use patina::api::StreamEvent;

    let mut state = new_state();

    // Start streaming
    state.tool_loop_mut().start_streaming().unwrap();
    state.set_streaming(true);

    // Stream some text
    state
        .append_chunk(StreamEvent::ContentDelta("Let me ".to_string()))
        .unwrap();
    state
        .append_chunk(StreamEvent::ContentDelta("help you.".to_string()))
        .unwrap();

    // Tool loop should have the full text
    assert_eq!(state.tool_loop().text_content(), "Let me help you.");
}

// ============================================================================
// P0-2: Tool Results in API Context Tests
// ============================================================================

/// Tests that ContinuationData builds proper assistant message with text AND tool_use.
#[test]
fn test_continuation_data_includes_text_and_tool_use() {
    use patina::app::tool_loop::ContinuationData;
    use patina::types::content::ContentBlock;
    use serde_json::json;

    let continuation = ContinuationData {
        assistant_content: vec![
            ContentBlock::text("Here's what I found:"),
            ContentBlock::tool_use("toolu_123", "bash", json!({"command": "ls"})),
        ],
        tool_results: vec![ContentBlock::tool_result(
            "toolu_123",
            "file1.txt\nfile2.txt",
        )],
    };

    let (assistant_msg, user_msg) = continuation.build_messages();

    // Verify assistant message has both text and tool_use
    let blocks = assistant_msg.content.as_blocks().unwrap();
    assert_eq!(blocks.len(), 2, "Assistant should have text + tool_use");
    assert!(blocks[0].is_text(), "First block should be text");
    assert!(blocks[1].is_tool_use(), "Second block should be tool_use");

    // Verify user message has tool_result
    let results = user_msg.content.as_blocks().unwrap();
    assert_eq!(results.len(), 1, "User should have 1 tool_result");
    assert!(results[0].is_tool_result());
}

/// Tests that continuation messages serialize correctly for API.
#[test]
fn test_continuation_serializes_correctly_for_api() {
    use patina::app::tool_loop::ContinuationData;
    use patina::types::content::ContentBlock;
    use serde_json::json;

    let continuation = ContinuationData {
        assistant_content: vec![
            ContentBlock::text("Checking..."),
            ContentBlock::tool_use("toolu_abc", "read_file", json!({"path": "test.txt"})),
        ],
        tool_results: vec![ContentBlock::tool_result("toolu_abc", "file contents here")],
    };

    let (assistant_msg, _user_msg) = continuation.build_messages();

    // Serialize and verify JSON structure
    let json = serde_json::to_value(&assistant_msg).unwrap();

    assert_eq!(json["role"], "assistant");
    let content = json["content"].as_array().unwrap();
    assert_eq!(content.len(), 2);

    // Verify first block is text
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[0]["text"], "Checking...");

    // Verify second block is tool_use
    assert_eq!(content[1]["type"], "tool_use");
    assert_eq!(content[1]["id"], "toolu_abc");
    assert_eq!(content[1]["name"], "read_file");
}
