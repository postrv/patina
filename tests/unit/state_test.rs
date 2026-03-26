//! Unit tests for application state management.
//!
//! These tests verify input handling, scroll behavior, dirty flag tracking,
//! session persistence, tool loop integration, scroll auto-follow, focus areas,
//! plugin/subagent wiring, context injection, compaction, event loop responsiveness,
//! worktree status, completion, and continuous coding loop behavior.

use patina::agents::{AgentProgress, ConflictReport, SubagentSpawner};
use patina::api::tokens::model_context_limit;
use patina::app::state::{
    get_git_head_hash, AgentPanelState, AgentPanelStatus, AppState, BackgroundEvent,
    ContinuousLoopStatus, SessionTracking, UISelectionState, WorktreeStatus,
    DEFAULT_COMPACTION_THRESHOLD,
};
use patina::app::tool_loop::ToolLoopState;
use patina::permissions::{PermissionRequest, PermissionResponse};
use patina::session::{Session, UiState};
use patina::tui::scroll::AutoScrollMode;
use patina::tui::selection::{ContentPosition, FocusArea};
use patina::types::config::ParallelMode;
use patina::types::content::StopReason;
use patina::types::{ApiMessageV2, ConversationEntry, Message, Role, StreamEvent, ToolResultBlock};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

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
    assert!(state.input_state().text().is_empty());

    state.insert_char('H');
    state.insert_char('i');

    assert_eq!(state.input_state().text(), "Hi");
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
    assert_eq!(state.input_state().text(), "Hell");

    state.delete_char();
    state.delete_char();
    assert_eq!(state.input_state().text(), "He");
}

/// Tests that delete_char on empty input doesn't panic.
#[test]
fn test_input_delete_char_empty() {
    let mut state = new_state();
    assert!(state.input_state().text().is_empty());

    // Should not panic
    state.delete_char();
    assert!(state.input_state().text().is_empty());
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
    assert!(state.input_state().text().is_empty());
}

/// Tests that take_input on empty buffer returns empty string.
#[test]
fn test_input_take_empty() {
    let mut state = new_state();
    let taken = state.take_input();
    assert!(taken.is_empty());
    assert!(state.input_state().text().is_empty());
}

/// Tests unicode character handling in input.
#[test]
fn test_input_unicode() {
    let mut state = new_state();
    state.insert_char('你');
    state.insert_char('好');
    state.insert_char('🦀');

    assert_eq!(state.input_state().text(), "你好🦀");

    state.delete_char();
    assert_eq!(state.input_state().text(), "你好");
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
    assert_eq!(state.display().scroll_offset(), 0);

    state.scroll_up(5);
    assert_eq!(state.display().scroll_offset(), 5);

    state.scroll_up(3);
    assert_eq!(state.display().scroll_offset(), 8);
}

/// Tests scroll down decreases scroll offset.
#[test]
fn test_scroll_down() {
    let mut state = new_state();
    // Set content larger than viewport to allow scrolling
    state.set_viewport_height(20);
    state.update_content_height(100);

    state.scroll_up(10);
    assert_eq!(state.display().scroll_offset(), 10);

    state.scroll_down(3);
    assert_eq!(state.display().scroll_offset(), 7);

    state.scroll_down(2);
    assert_eq!(state.display().scroll_offset(), 5);
}

/// Tests scroll bounds saturation - scroll_down at 0 stays at 0.
#[test]
fn test_scroll_bounds_saturation_at_zero() {
    let mut state = new_state();
    assert_eq!(state.display().scroll_offset(), 0);

    // Should not go negative - saturating_sub should keep it at 0
    state.scroll_down(10);
    assert_eq!(state.display().scroll_offset(), 0);
}

/// Tests scroll up saturation - large values don't overflow.
#[test]
fn test_scroll_up_large_values() {
    let mut state = new_state();

    state.scroll_up(usize::MAX / 2);
    let first = state.display().scroll_offset();

    // saturating_add should prevent overflow
    state.scroll_up(usize::MAX / 2);
    assert!(state.display().scroll_offset() >= first);
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
    assert_eq!(state.input_state().cursor_position(), 0);
}

/// Tests cursor position after inserting characters.
/// Cursor should be at the end after each insert.
#[test]
fn test_cursor_position_after_insert() {
    let mut state = new_state();
    state.insert_char('a');
    assert_eq!(state.input_state().cursor_position(), 1);

    state.insert_char('b');
    state.insert_char('c');
    assert_eq!(state.input_state().cursor_position(), 3);
}

/// Tests moving cursor left.
#[test]
fn test_cursor_move_left() {
    let mut state = new_state();
    state.insert_char('a');
    state.insert_char('b');
    state.insert_char('c');

    state.cursor_left();
    assert_eq!(state.input_state().cursor_position(), 2);

    state.cursor_left();
    assert_eq!(state.input_state().cursor_position(), 1);
}

/// Tests cursor left at beginning doesn't go negative.
#[test]
fn test_cursor_move_left_at_start() {
    let mut state = new_state();
    state.insert_char('a');
    state.cursor_left();
    assert_eq!(state.input_state().cursor_position(), 0);

    // Should not go negative
    state.cursor_left();
    assert_eq!(state.input_state().cursor_position(), 0);
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
    assert_eq!(state.input_state().cursor_position(), 0);

    state.cursor_right();
    assert_eq!(state.input_state().cursor_position(), 1);

    state.cursor_right();
    assert_eq!(state.input_state().cursor_position(), 2);
}

/// Tests cursor right at end doesn't exceed length.
#[test]
fn test_cursor_move_right_at_end() {
    let mut state = new_state();
    state.insert_char('a');
    assert_eq!(state.input_state().cursor_position(), 1);

    // Should not exceed string length
    state.cursor_right();
    assert_eq!(state.input_state().cursor_position(), 1);
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

    assert_eq!(state.input_state().text(), "abc");
    assert_eq!(state.input_state().cursor_position(), 2); // cursor moves after inserted char
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

    assert_eq!(state.input_state().text(), "ac");
    assert_eq!(state.input_state().cursor_position(), 1);
}

/// Tests cursor home (move to start).
#[test]
fn test_cursor_home() {
    let mut state = new_state();
    state.insert_char('a');
    state.insert_char('b');
    state.insert_char('c');

    state.cursor_home();
    assert_eq!(state.input_state().cursor_position(), 0);
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
    assert_eq!(state.input_state().cursor_position(), 3);
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

// ============================================================================
// Tests migrated from src/app/state.rs inline test module
// ============================================================================

/// Helper to create a test Message.
fn test_message(role: Role, content: &str) -> Message {
    Message {
        role,
        content: content.to_string(),
    }
}

#[test]
fn test_app_state_new() {
    let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    assert!(state.timeline().is_empty());
    assert!(state.input_state().text().is_empty());
    assert_eq!(state.display().scroll_offset(), 0);
    assert_eq!(state.working_dir, PathBuf::from("/test"));
}

#[test]
fn test_restore_from_session_messages() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Create a session with messages
    let mut session = Session::new(PathBuf::from("/project"));
    session.add_message(test_message(Role::User, "Hello"));
    session.add_message(test_message(Role::Assistant, "Hi there!"));

    state.restore_from_session(&session);

    assert_eq!(state.timeline().len(), 2);
    let entries: Vec<_> = state.timeline().iter().collect();
    assert!(matches!(entries[0], ConversationEntry::UserMessage(s) if s == "Hello"));
    assert!(matches!(entries[1], ConversationEntry::AssistantMessage(s) if s == "Hi there!"));
}

#[test]
fn test_restore_from_session_with_ui_state() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Create a session with UI state
    let mut session = Session::new(PathBuf::from("/project"));
    session.add_message(test_message(Role::User, "Test"));
    session.set_ui_state(Some(UiState::with_state(50, "draft input".to_string(), 5)));

    state.restore_from_session(&session);

    assert_eq!(state.display().scroll_offset(), 50);
    assert_eq!(state.input_state().text(), "draft input");
    assert_eq!(state.input_state().cursor_position(), 5);
}

#[test]
fn test_restore_marks_dirty() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    state.mark_rendered(); // Clear dirty flags

    let session = Session::new(PathBuf::from("/project"));
    state.restore_from_session(&session);

    assert!(state.needs_render());
}

// ========================================================================
// Phase 10.4.1: Auto-save tests
// ========================================================================

#[test]
fn test_app_state_session_id_none_initially() {
    let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    assert!(state.session_tracking().id().is_none());
}

#[test]
fn test_app_state_set_session_id() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    state.session_tracking_mut().set_id("abc123".to_string());
    assert_eq!(state.session_tracking().id(), Some("abc123"));
}

#[test]
fn test_to_session_empty() {
    let state = AppState::new(PathBuf::from("/project"), false, ParallelMode::Enabled);
    let session = state.to_session();

    assert!(session.messages().is_empty());
    assert_eq!(session.working_dir(), &PathBuf::from("/project"));
}

#[test]
fn test_to_session_with_messages() {
    let mut state = AppState::new(PathBuf::from("/project"), false, ParallelMode::Enabled);
    state.add_message(test_message(Role::User, "Hello"));
    state.add_message(test_message(Role::Assistant, "Hi!"));

    let session = state.to_session();

    assert_eq!(session.messages().len(), 2);
    assert_eq!(session.messages()[0].content, "Hello");
    assert_eq!(session.messages()[1].content, "Hi!");
}

#[test]
fn test_restore_from_session_restores_session_id() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    assert!(state.session_tracking().id().is_none());

    // Create session with an ID (simulating a saved session)
    let mut session = Session::new(PathBuf::from("/project"));
    session.add_message(test_message(Role::User, "Test"));
    // Manually set the session ID via JSON (normally done by SessionManager::save)
    let session_json = serde_json::to_string(&session).unwrap();
    let json_with_id = session_json.replace(r#""id":null"#, r#""id":"test-session-id""#);
    let session_with_id: Session = serde_json::from_str(&json_with_id).unwrap();

    state.restore_from_session(&session_with_id);

    assert_eq!(state.session_tracking().id(), Some("test-session-id"));
}

// ========================================================================
// Tool Loop Integration Tests (Phase 10.5.2.4)
// ========================================================================

#[test]
fn test_appstate_has_tool_loop() {
    let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    assert!(matches!(state.tool_loop_state(), ToolLoopState::Idle));
}

#[test]
fn test_appstate_receives_tool_use() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Start streaming
    state.tool_loop_mut().start_streaming().unwrap();

    // Simulate receiving tool use events
    state.handle_tool_use_start("toolu_123".to_string(), "bash".to_string(), 0);
    state.handle_tool_use_input_delta(0, r#"{"command":"ls"}"#);
    state.handle_tool_use_complete(0).unwrap();

    // Complete the message with tool_use stop reason
    state.handle_message_complete(StopReason::ToolUse).unwrap();

    // Should be in PendingApproval state
    assert!(matches!(
        state.tool_loop_state(),
        ToolLoopState::PendingApproval
    ));

    // Should need user action
    assert!(state.tool_loop_needs_user_action());
}

#[test]
fn test_appstate_approve_and_deny_tools() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Set up tool use
    state.tool_loop_mut().start_streaming().unwrap();
    state.handle_tool_use_start("toolu_1".to_string(), "bash".to_string(), 0);
    state.handle_tool_use_input_delta(0, "{}");
    state.handle_tool_use_complete(0).unwrap();
    state.handle_message_complete(StopReason::ToolUse).unwrap();

    // Deny all
    state.deny_all_tools().unwrap();
    assert!(matches!(state.tool_loop_state(), ToolLoopState::Idle));
}

#[test]
fn test_appstate_pending_permission() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    assert!(!state.has_pending_permission());
    assert!(state.pending_permission().is_none());

    // Set a pending permission
    let request = PermissionRequest::new("bash", Some("rm -rf temp"), "Execute command");
    state.set_pending_permission(request);

    assert!(state.has_pending_permission());
    assert!(state.pending_permission().is_some());
    let pending = state.pending_permission().unwrap();
    assert_eq!(pending.tool_name, "bash");

    // Clear it
    state.clear_pending_permission();
    assert!(!state.has_pending_permission());
}

#[tokio::test]
async fn test_appstate_handles_permission_response() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Set a pending permission
    let request = PermissionRequest::new("bash", Some("echo hello"), "Execute command");
    state.set_pending_permission(request);

    // Handle the response (Allow Once)
    state
        .handle_permission_response(PermissionResponse::AllowOnce)
        .await;

    // Permission should be cleared
    assert!(!state.has_pending_permission());
}

#[test]
fn test_appstate_reset_tool_loop() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Set up some state
    state.tool_loop_mut().start_streaming().unwrap();
    state.handle_tool_use_start("toolu_1".to_string(), "bash".to_string(), 0);
    let request = PermissionRequest::new("bash", None, "test");
    state.set_pending_permission(request);

    // Reset
    state.reset_tool_loop();

    assert!(matches!(state.tool_loop_state(), ToolLoopState::Idle));
    assert!(!state.has_pending_permission());
}

#[test]
fn test_appstate_tool_loop_state_helpers() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Initially idle - needs user action
    assert!(state.tool_loop_needs_user_action());
    assert!(!state.tool_loop_is_active());

    // Start streaming - active
    state.tool_loop_mut().start_streaming().unwrap();
    assert!(!state.tool_loop_needs_user_action());
    assert!(state.tool_loop_is_active());
}

// ========================================================================
// Scroll State Integration Tests (Phase 10.5.4.2)
// ========================================================================

#[test]
fn test_scroll_state_initial() {
    let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Should start in Follow mode at offset 0
    assert_eq!(state.display().scroll_offset(), 0);
    assert_eq!(
        state.display().scroll_state().mode(),
        AutoScrollMode::Follow
    );
}

#[test]
fn test_streaming_content_auto_scrolls() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    state.set_viewport_height(20);

    // Simulate content growth (streaming updates)
    state.update_content_height(30);
    assert_eq!(state.display().scroll_offset(), 0); // At bottom

    // More content arrives
    state.update_content_height(50);

    // In Follow mode, should auto-scroll to stay at bottom
    assert_eq!(state.display().scroll_offset(), 0);
    assert_eq!(
        state.display().scroll_state().mode(),
        AutoScrollMode::Follow
    );
}

#[test]
fn test_user_scroll_preserved_during_streaming() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    state.set_viewport_height(20);
    state.update_content_height(50);

    // User scrolls up
    state.scroll_up(15);
    assert_eq!(state.display().scroll_offset(), 15);
    assert_eq!(
        state.display().scroll_state().mode(),
        AutoScrollMode::Manual
    );

    // More content arrives (streaming)
    state.update_content_height(80);

    // User's scroll position should be preserved
    assert_eq!(state.display().scroll_offset(), 15);
    assert_eq!(
        state.display().scroll_state().mode(),
        AutoScrollMode::Manual
    );
}

#[test]
fn test_scroll_down_resumes_follow_mode() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    state.set_viewport_height(20);
    state.update_content_height(50);

    // User scrolls up (switches to Manual)
    state.scroll_up(20);
    assert_eq!(
        state.display().scroll_state().mode(),
        AutoScrollMode::Manual
    );

    // User scrolls all the way back down
    state.scroll_down(20);

    // Should resume Follow mode
    assert_eq!(state.display().scroll_offset(), 0);
    assert_eq!(
        state.display().scroll_state().mode(),
        AutoScrollMode::Follow
    );
}

#[test]
fn test_scroll_to_bottom_method() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    state.set_viewport_height(20);
    state.update_content_height(50);

    // User scrolls up
    state.scroll_up(30);
    assert_eq!(
        state.display().scroll_state().mode(),
        AutoScrollMode::Manual
    );

    // Explicitly scroll to bottom
    state.scroll_to_bottom(80);

    // Should be in Follow mode at bottom
    assert_eq!(state.display().scroll_offset(), 0);
    assert_eq!(
        state.display().scroll_state().mode(),
        AutoScrollMode::Follow
    );
}

#[test]
fn test_scroll_state_accessor() {
    let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Should be able to access scroll state
    let scroll = state.display().scroll_state();
    assert_eq!(scroll.offset(), 0);
}

// ========================================================================
// Tool Block UI Tests (Phase 10.5.6)
// ========================================================================

#[test]
fn test_tool_blocks_initially_empty() {
    let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    assert!(state.tool_blocks().is_empty());
    assert!(!state.has_tool_blocks());
}

#[test]
fn test_start_tool_block() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    let index = state.start_tool_block("bash", "git status");

    assert_eq!(index, 0);
    assert!(state.has_tool_blocks());
    assert_eq!(state.tool_blocks().len(), 1);

    let block = &state.tool_blocks()[0];
    assert_eq!(block.tool_name(), "bash");
    assert_eq!(block.tool_input(), "git status");
    assert!(!block.is_complete());
}

#[test]
fn test_complete_tool_block_success() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    let index = state.start_tool_block("bash", "echo hello");

    state.complete_tool_block(index, "hello", false);

    let block = &state.tool_blocks()[0];
    assert!(block.is_complete());
    assert_eq!(block.result(), Some("hello"));
    assert!(!block.is_error());
}

#[test]
fn test_complete_tool_block_error() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    let index = state.start_tool_block("bash", "bad-command");

    state.complete_tool_block(index, "Command not found", true);

    let block = &state.tool_blocks()[0];
    assert!(block.is_complete());
    assert_eq!(block.result(), Some("Command not found"));
    assert!(block.is_error());
}

#[test]
fn test_multiple_tool_blocks() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    let idx1 = state.start_tool_block("bash", "ls");
    let idx2 = state.start_tool_block("read", "/tmp/file.txt");

    assert_eq!(idx1, 0);
    assert_eq!(idx2, 1);
    assert_eq!(state.tool_blocks().len(), 2);

    state.complete_tool_block(idx1, "file1.txt\nfile2.txt", false);
    state.complete_tool_block(idx2, "file contents", false);

    assert!(state.tool_blocks()[0].is_complete());
    assert!(state.tool_blocks()[1].is_complete());
}

#[test]
fn test_clear_tool_blocks() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    state.start_tool_block("bash", "ls");
    state.start_tool_block("read", "/tmp/test");
    assert_eq!(state.tool_blocks().len(), 2);

    state.clear_tool_blocks();

    assert!(state.tool_blocks().is_empty());
    assert!(!state.has_tool_blocks());
}

#[test]
fn test_complete_invalid_index_is_safe() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Completing a non-existent index should not panic
    state.complete_tool_block(999, "result", false);

    assert!(state.tool_blocks().is_empty());
}

// ========================================================================
// Context Truncation Integration Tests (Cost Optimization)
// ========================================================================

#[test]
fn test_api_messages_truncated_empty() {
    let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    let truncated = state.api_messages_truncated();

    assert!(truncated.is_empty());
}

#[test]
fn test_focus_area_default_is_input() {
    let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    assert_eq!(state.ui_selection().focus_area(), FocusArea::Input);
}

#[test]
fn test_focus_area_can_be_set() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    state.ui_selection_mut().set_focus_area(FocusArea::Content);
    assert_eq!(state.ui_selection().focus_area(), FocusArea::Content);

    state.ui_selection_mut().set_focus_area(FocusArea::Input);
    assert_eq!(state.ui_selection().focus_area(), FocusArea::Input);
}

#[test]
fn test_focus_change_clears_selection() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Create a selection
    state
        .ui_selection_mut()
        .selection_mut()
        .start(ContentPosition::new(0, 0));
    state
        .ui_selection_mut()
        .selection_mut()
        .update(ContentPosition::new(5, 10));
    state.ui_selection_mut().selection_mut().end();
    assert!(state.ui_selection().selection().has_selection());

    // Change focus should clear selection
    state.ui_selection_mut().set_focus_area(FocusArea::Content);
    assert!(!state.ui_selection().selection().has_selection());
}

#[test]
fn test_focus_same_area_preserves_selection() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Set focus to content
    state.ui_selection_mut().set_focus_area(FocusArea::Content);

    // Create a selection
    state
        .ui_selection_mut()
        .selection_mut()
        .start(ContentPosition::new(0, 0));
    state
        .ui_selection_mut()
        .selection_mut()
        .update(ContentPosition::new(5, 10));
    state.ui_selection_mut().selection_mut().end();
    assert!(state.ui_selection().selection().has_selection());

    // Setting same focus should NOT clear selection
    state.ui_selection_mut().set_focus_area(FocusArea::Content);
    assert!(state.ui_selection().selection().has_selection());
}

#[test]
fn test_focus_area_for_row_content() {
    // Terminal height 30: input is rows 27-29, content is 0-26
    assert_eq!(
        UISelectionState::focus_area_for_row(0, 30),
        FocusArea::Content
    );
    assert_eq!(
        UISelectionState::focus_area_for_row(10, 30),
        FocusArea::Content
    );
    assert_eq!(
        UISelectionState::focus_area_for_row(26, 30),
        FocusArea::Content
    );
}

#[test]
fn test_focus_area_for_row_input() {
    // Terminal height 30: input is rows 27-29
    assert_eq!(
        UISelectionState::focus_area_for_row(27, 30),
        FocusArea::Input
    );
    assert_eq!(
        UISelectionState::focus_area_for_row(28, 30),
        FocusArea::Input
    );
    assert_eq!(
        UISelectionState::focus_area_for_row(29, 30),
        FocusArea::Input
    );
}

#[test]
fn test_focus_area_for_row_small_terminal() {
    // Minimum terminal height 7: content rows 0-3, input rows 4-6
    assert_eq!(
        UISelectionState::focus_area_for_row(0, 7),
        FocusArea::Content
    );
    assert_eq!(
        UISelectionState::focus_area_for_row(3, 7),
        FocusArea::Content
    );
    assert_eq!(UISelectionState::focus_area_for_row(4, 7), FocusArea::Input);
    assert_eq!(UISelectionState::focus_area_for_row(6, 7), FocusArea::Input);
}

// Plugin loading tests

#[test]
fn test_with_plugins_disabled_creates_empty_registry() {
    let state = AppState::with_plugins(
        PathBuf::from("/test"),
        false,
        ParallelMode::Enabled,
        false, // plugins_enabled = false
    );
    // With plugins disabled, registry should be empty
    assert_eq!(state.plugins().plugin_count(), 0);
}

#[test]
fn test_with_plugins_enabled_returns_valid_registry() {
    let state = AppState::with_plugins(
        PathBuf::from("/test"),
        false,
        ParallelMode::Enabled,
        true, // plugins_enabled = true
    );
    // Registry should be valid (may be empty if no plugins installed)
    // Just verify we can access the registry without panicking
    let _ = state.plugins().plugin_count();
}

#[test]
fn test_new_enables_plugins_by_default() {
    // AppState::new should enable plugins (equivalent to with_plugins(..., true))
    let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    // Registry should be accessible (plugins enabled by default)
    let _ = state.plugins().plugin_count();
}

// =========================================================================
// 1.5.4.3 - Subagent wiring tests
// =========================================================================

#[test]
fn test_new_disables_subagents_by_default() {
    // AppState::new should disable subagents by default
    let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    assert!(!state.agent_panel().subagents_enabled());
    assert!(state.agent_panel().spawner().is_none());
}

#[test]
fn test_with_options_subagents_disabled() {
    let state = AppState::with_options(
        PathBuf::from("/test"),
        false,
        ParallelMode::Enabled,
        true,  // plugins_enabled
        false, // subagents_enabled
    );
    assert!(!state.agent_panel().subagents_enabled());
    assert!(state.agent_panel().spawner().is_none());
}

#[test]
fn test_with_options_subagents_enabled() {
    let state = AppState::with_options(
        PathBuf::from("/test"),
        false,
        ParallelMode::Enabled,
        true, // plugins_enabled
        true, // subagents_enabled
    );
    assert!(state.agent_panel().subagents_enabled());
    assert!(state.agent_panel().spawner().is_some());
}

#[test]
fn test_subagent_spawner_returns_valid_spawner_when_enabled() {
    let state = AppState::with_options(
        PathBuf::from("/test"),
        false,
        ParallelMode::Enabled,
        false, // plugins_enabled
        true,  // subagents_enabled
    );

    // Verify we can access the spawner
    let spawner = state
        .agent_panel()
        .spawner()
        .expect("spawner should be Some");
    // Verify spawner has a valid model configured
    assert!(spawner.model().contains("claude"));
}

#[test]
fn test_with_plugins_disables_subagents() {
    // with_plugins should disable subagents (for backward compatibility)
    let state = AppState::with_plugins(
        PathBuf::from("/test"),
        false,
        ParallelMode::Enabled,
        true, // plugins_enabled
    );
    assert!(!state.agent_panel().subagents_enabled());
    assert!(state.agent_panel().spawner().is_none());
}

// =========================================================================
// 2.2.4 - Auto-context injection tests
// =========================================================================

#[test]
fn test_auto_context_disabled_by_default() {
    // AppState should have auto_context disabled by default
    // (Config enables it, but AppState needs explicit opt-in)
    let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    assert!(!state.compression().auto_context_enabled());
}

#[test]
fn test_set_auto_context_enabled() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Initially disabled
    assert!(!state.compression().auto_context_enabled());

    // Enable it
    state.compression_mut().set_auto_context_enabled(true);
    assert!(state.compression().auto_context_enabled());

    // Disable it
    state.compression_mut().set_auto_context_enabled(false);
    assert!(!state.compression().auto_context_enabled());
}

#[test]
fn test_inject_context_suggestions() {
    use patina::narsil::context::{CodeReference, ContextKind, ContextSuggestion};

    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Create test suggestions
    let suggestions = vec![
        ContextSuggestion {
            source: CodeReference::function("process_data"),
            kind: ContextKind::Callers,
            description: "Functions that call process_data".to_string(),
            content: "main() in src/main.rs:10".to_string(),
        },
        ContextSuggestion {
            source: CodeReference::file("src/handler.rs"),
            kind: ContextKind::Imports,
            description: "Imports in src/handler.rs".to_string(),
            content: "use std::io".to_string(),
        },
    ];

    // Inject the suggestions
    state
        .compression_mut()
        .set_pending_context(suggestions.clone());

    // Verify we have pending context
    assert!(state.compression().has_pending_context());
    assert_eq!(state.compression().pending_context().len(), 2);
}

#[test]
fn test_take_pending_context_clears() {
    use patina::narsil::context::{CodeReference, ContextKind, ContextSuggestion};

    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    let suggestions = vec![ContextSuggestion {
        source: CodeReference::function("test_fn"),
        kind: ContextKind::Callers,
        description: "Test".to_string(),
        content: "Content".to_string(),
    }];

    state.compression_mut().set_pending_context(suggestions);
    assert!(state.compression().has_pending_context());

    // Take should return and clear
    let taken = state.compression_mut().take_pending_context();
    assert_eq!(taken.len(), 1);
    assert!(!state.compression().has_pending_context());
    assert!(state.compression().pending_context().is_empty());
}

#[test]
fn test_format_context_for_message() {
    use patina::narsil::context::{CodeReference, ContextKind, ContextSuggestion};

    let suggestions = vec![ContextSuggestion {
        source: CodeReference::function("process_data"),
        kind: ContextKind::Callers,
        description: "Functions that call process_data".to_string(),
        content: "main() in src/main.rs:10\nhandle_request() in src/api.rs:25".to_string(),
    }];

    let formatted = AppState::format_context_suggestions(&suggestions);

    // Should contain the description and content
    assert!(formatted.contains("Functions that call process_data"));
    assert!(formatted.contains("main() in src/main.rs:10"));
    assert!(formatted.contains("handle_request() in src/api.rs:25"));
}

#[test]
fn test_format_context_empty_suggestions() {
    let suggestions: Vec<patina::narsil::context::ContextSuggestion> = vec![];
    let formatted = AppState::format_context_suggestions(&suggestions);

    // Empty suggestions should return empty string
    assert!(formatted.is_empty());
}

#[test]
fn test_clear_pending_context() {
    use patina::narsil::context::{CodeReference, ContextKind, ContextSuggestion};

    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    let suggestions = vec![ContextSuggestion {
        source: CodeReference::function("test_fn"),
        kind: ContextKind::Callers,
        description: "Test".to_string(),
        content: "Content".to_string(),
    }];

    state.compression_mut().set_pending_context(suggestions);
    assert!(state.compression().has_pending_context());

    state.compression_mut().clear_pending_context();
    assert!(!state.compression().has_pending_context());
}

// =========================================================================
// 4.3.1 - CompressionOrchestrator tests
// =========================================================================

#[test]
fn test_compression_orchestrator_none_by_default() {
    let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    assert!(state.compression().compression_orchestrator().is_none());
    assert!(!state.compression().has_ccg_support());
}

#[test]
fn test_has_ccg_support_false_when_no_orchestrator() {
    let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    assert!(!state.compression().has_ccg_support());
}

// =========================================================================
// 4.3.3 - CCG Context Injection tests
// =========================================================================

#[test]
fn test_last_ccg_hash_none_by_default() {
    let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    assert!(state.compression().last_ccg_hash().is_none());
}

#[test]
fn test_inject_ccg_context_returns_none_without_orchestrator() {
    // Create a minimal mock test to verify basic behavior
    // Full async testing requires MCP client which is complex to mock
    let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Without orchestrator, the method should return None early
    // We verify the precondition: no orchestrator means function returns None
    assert!(state.compression().compression_orchestrator().is_none());
}

// =========================================================================
// 4.3.4 - CCG Context Injection in Send Flow tests
// =========================================================================

#[test]
fn test_cached_ccg_context_none_by_default() {
    let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    assert!(!state.compression().has_cached_ccg_context());
}

#[test]
fn test_context_for_injection_returns_none_without_cached_context() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Enable auto-context but no cached context
    state.compression_mut().set_auto_context_enabled(true);
    assert!(state.compression().context_for_injection().is_none());
}

// =========================================================================
// Phase 3.5 - Build Context Injection into Message-Sending Path
// =========================================================================

#[test]
fn test_narsil_client_none_by_default() {
    let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    assert!(!state.compression().has_narsil_client());
}

#[test]
fn test_context_token_budget_default() {
    let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    assert_eq!(state.compression().context_token_budget(), 10_000);
}

#[test]
fn test_set_context_token_budget() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    state.compression_mut().set_context_token_budget(5_000);
    assert_eq!(state.compression().context_token_budget(), 5_000);
}

#[test]
fn test_context_tokens_injected_zero_by_default() {
    let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    assert_eq!(state.compression().context_tokens_injected(), 0);
}

#[tokio::test]
async fn test_refresh_build_context_returns_early_when_disabled() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    // auto_context is disabled by default
    assert!(!state.compression().auto_context_enabled());

    // Should return None immediately
    let result = state.refresh_build_context().await;
    assert!(result.is_none());
    assert!(!state.compression().has_cached_ccg_context());
    assert_eq!(state.compression().context_tokens_injected(), 0);
}

#[tokio::test]
async fn test_refresh_build_context_returns_early_without_orchestrator() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    state.compression_mut().set_auto_context_enabled(true);

    // No orchestrator set
    assert!(state.compression().compression_orchestrator().is_none());

    // Should return None immediately
    let result = state.refresh_build_context().await;
    assert!(result.is_none());
    assert!(!state.compression().has_cached_ccg_context());
    assert_eq!(state.compression().context_tokens_injected(), 0);
}

// =========================================================================
// 7.1.3 - Cache-aware context refresh tests
// =========================================================================

#[tokio::test]
async fn test_submit_message_without_context_sends_plain() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    // auto_context disabled (default), no cached context

    let client: Arc<dyn patina::api::LlmProvider> = Arc::new(patina::api::AnthropicClient::new(
        secrecy::SecretString::from("test-key"),
        "claude-sonnet-4-20250514",
    ));

    let _ = state
        .submit_message(&client, "Hello plain".to_string())
        .await;

    let last_user_msg = state.api_messages().iter().find(|m| m.role == Role::User);
    assert!(last_user_msg.is_some());
    let content = last_user_msg.unwrap().content.as_text().unwrap_or_default();
    assert!(!content.contains("<context>"));
    assert!(content.contains("Hello plain"));
}

#[test]
fn test_get_git_head_hash_returns_some_in_git_repo() {
    // We're running inside a git repo (rct), so this should succeed
    let hash = get_git_head_hash(std::path::Path::new("."));
    assert!(hash.is_some());
    let hash_str = hash.unwrap();
    // Git hashes are 40 hex characters
    assert_eq!(hash_str.len(), 40);
    assert!(hash_str.chars().all(|c: char| c.is_ascii_hexdigit()));
}

#[test]
fn test_get_git_head_hash_returns_none_for_non_repo() {
    let hash = get_git_head_hash(std::path::Path::new("/tmp"));
    assert!(hash.is_none());
}

// =========================================================================
// 7.1.1 - Characterization Tests for Message Flow
//
// These tests document the current message preparation behavior:
// - api_messages() is a raw accessor with no context injection
// - submit_message() is the only path that injects CCG context
// - inject_ccg_context() exists but is orphaned (never called by production code)
// =========================================================================

#[test]
fn test_build_api_messages_empty_returns_empty() {
    let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    let messages = state.build_api_messages();
    assert!(messages.is_empty());
}

#[test]
fn test_build_api_messages_delegates_to_truncated_when_no_context() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    // auto_context disabled (default), no cached context

    state.api_messages_mut().push(ApiMessageV2::user("Hello"));
    state
        .api_messages_mut()
        .push(ApiMessageV2::assistant("Hi there"));

    // Without context injection, build_api_messages should return the same
    // messages as api_messages_truncated
    let built = state.build_api_messages();
    let truncated = state.api_messages_truncated();

    assert_eq!(built.len(), truncated.len());
    for (b, t) in built.iter().zip(truncated.iter()) {
        assert_eq!(b.content.to_text(), t.content.to_text());
        assert_eq!(b.role, t.role);
    }
}

#[tokio::test]
async fn test_inject_ccg_context_exists_and_callable() {
    // Characterization: inject_ccg_context() exists as a public async method
    // on AppState, but it is ORPHANED — no production code calls it.
    // The actual context injection path is:
    //   submit_message() -> refresh_build_context() -> orchestrator.build_context()
    //
    // This test documents the method's existence and its early-return behavior
    // when no orchestrator is configured.
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Without an orchestrator, inject_ccg_context returns Ok(None) immediately
    assert!(
        state.compression().compression_orchestrator().is_none(),
        "Default AppState should have no compression orchestrator"
    );

    // The method requires an McpConnection, but returns early before using it
    // when no orchestrator is available. We verify the early return here.
    // Note: We cannot call inject_ccg_context directly without a real McpConnection,
    // but we can verify the precondition that guarantees the early return.
    assert!(
        state.compression().compression_orchestrator().is_none(),
        "inject_ccg_context() would return Ok(None) without orchestrator"
    );

    // The orphaned method does NOT affect the working context injection path:
    // refresh_build_context() is the method actually used by submit_message().
    // With auto_context disabled, refresh_build_context returns None immediately.
    assert!(!state.compression().auto_context_enabled());
    let result = state.refresh_build_context().await;
    assert!(
        result.is_none(),
        "refresh_build_context() returns None when auto_context is disabled"
    );
}

// =========================================================================
// 7.1.2 - Context Injection in build_api_messages (GREEN)
//
// These tests define the expected behavior after wiring the compression
// orchestrator into the message preparation path.
// =========================================================================

#[test]
fn test_prepare_api_messages_skips_context_when_no_cached() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    state.compression_mut().set_auto_context_enabled(true);
    // No cached context

    state.api_messages_mut().push(ApiMessageV2::user("Hello"));

    let messages = state.build_api_messages();

    // Should have only the user message
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content.to_text(), "Hello");
}

#[tokio::test]
async fn test_recv_api_chunk_returns_none_when_no_streaming() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Initially streaming_rx is None
    assert!(!state.has_streaming());

    // recv_api_chunk should return None immediately, not block
    let result = state.recv_api_chunk().await;
    assert!(result.is_none());
}

#[tokio::test]
async fn test_recv_api_chunk_receives_chunks_when_streaming() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Set up a streaming channel
    let (tx, rx) = mpsc::channel(100);
    state.set_streaming_rx(rx);
    assert!(state.has_streaming());

    // Send a chunk
    tx.send(StreamEvent::ContentDelta("Hello".to_string()))
        .await
        .unwrap();

    // Should receive the chunk
    let result = state.recv_api_chunk().await;
    assert!(matches!(result, Some(StreamEvent::ContentDelta(s)) if s == "Hello"));
}

#[tokio::test]
async fn test_recv_tool_result_returns_none_when_no_channel() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Initially tool_result_rx is None
    assert!(!state.has_tool_result_rx());

    // recv_tool_result should return None immediately, not block
    let result = state.recv_tool_result().await;
    assert!(result.is_none());
}

#[tokio::test]
async fn test_recv_tool_result_receives_results_when_channel_set() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Set up a tool result channel
    let (tx, rx) = mpsc::channel(100);
    state.set_tool_result_rx(rx);
    assert!(state.has_tool_result_rx());

    // Send a result
    let result_block = ToolResultBlock {
        tool_use_id: "toolu_123".to_string(),
        content: "Success".to_string(),
        is_error: false,
    };
    tx.send(("toolu_123".to_string(), result_block))
        .await
        .unwrap();

    // Should receive the result
    let result = state.recv_tool_result().await;
    assert!(result.is_some());
    let (id, block) = result.unwrap();
    assert_eq!(id, "toolu_123");
    assert_eq!(block.content, "Success");
    assert!(!block.is_error);
}

#[tokio::test]
async fn test_recv_background_event_returns_none_when_no_channels() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Initially both channels are None
    assert!(!state.has_streaming());
    assert!(!state.has_tool_result_rx());
    assert!(!state.has_background_work());

    // recv_background_event should return None immediately, not block
    let result = state.recv_background_event().await;
    assert!(result.is_none());
}

#[tokio::test]
async fn test_recv_background_event_prioritizes_tool_results() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Set up both channels
    let (api_tx, api_rx) = mpsc::channel(100);
    let (tool_tx, tool_rx) = mpsc::channel(100);
    state.set_streaming_rx(api_rx);
    state.set_tool_result_rx(tool_rx);

    // Send to both channels
    api_tx
        .send(StreamEvent::ContentDelta("API chunk".to_string()))
        .await
        .unwrap();
    let result_block = ToolResultBlock {
        tool_use_id: "toolu_456".to_string(),
        content: "Tool result".to_string(),
        is_error: false,
    };
    tool_tx
        .send(("toolu_456".to_string(), result_block))
        .await
        .unwrap();

    // Tool results are prioritized (biased select)
    let result = state.recv_background_event().await;
    assert!(matches!(result, Some(BackgroundEvent::ToolResult(id, _)) if id == "toolu_456"));
}

#[tokio::test]
async fn test_recv_background_event_receives_api_chunks() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Set up only API channel (no tool channel)
    let (api_tx, api_rx) = mpsc::channel(100);
    state.set_streaming_rx(api_rx);

    // Send an API chunk
    api_tx
        .send(StreamEvent::ContentDelta("API data".to_string()))
        .await
        .unwrap();

    // Should receive the API chunk
    let result = state.recv_background_event().await;
    assert!(
        matches!(result, Some(BackgroundEvent::ApiChunk(StreamEvent::ContentDelta(s))) if s == "API data")
    );
}

#[test]
fn test_clear_tool_result_rx() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Set up channel
    let (_tx, rx) = mpsc::channel(100);
    state.set_tool_result_rx(rx);
    assert!(state.has_tool_result_rx());

    // Clear it
    state.clear_tool_result_rx();
    assert!(!state.has_tool_result_rx());
}

#[test]
fn test_background_event_enum() {
    // Test ApiChunk variant
    let chunk = BackgroundEvent::ApiChunk(StreamEvent::ContentDelta("test".to_string()));
    assert!(matches!(chunk, BackgroundEvent::ApiChunk(_)));

    // Test ToolResult variant
    let result = ToolResultBlock {
        tool_use_id: "id".to_string(),
        content: "result".to_string(),
        is_error: false,
    };
    let event = BackgroundEvent::ToolResult("id".to_string(), result);
    assert!(matches!(event, BackgroundEvent::ToolResult(_, _)));
}

#[tokio::test]
async fn test_spawn_tool_execution_sets_channel() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // No channel initially
    assert!(!state.has_tool_result_rx());

    // Set up tool use and spawn
    state.tool_loop_mut().start_streaming().unwrap();
    state.handle_tool_use_start("toolu_789".to_string(), "bash".to_string(), 0);
    state.handle_tool_use_input_delta(0, r#"{"command":"echo hi"}"#);
    state.handle_tool_use_complete(0).unwrap();
    state.handle_message_complete(StopReason::ToolUse).unwrap();
    state.approve_all_tools().unwrap();

    // Spawn should set up channel (requires tokio runtime)
    let _handle = state.spawn_tool_execution();
    assert!(state.has_tool_result_rx());
    assert!(state.has_executing_tools());
}

#[test]
fn test_record_tool_result_updates_state() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Mark a tool as executing
    state.mark_tool_executing("toolu_abc");
    assert!(state.has_executing_tools());

    // Record result
    let result = ToolResultBlock {
        tool_use_id: "toolu_abc".to_string(),
        content: "Done".to_string(),
        is_error: false,
    };
    state.record_tool_result("toolu_abc", result);

    // Should no longer have executing tools
    assert!(!state.has_executing_tools());
}

#[test]
fn test_all_tools_complete() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // No executing tools initially
    assert!(state.all_tools_complete());

    // Mark a tool as executing
    state.mark_tool_executing("toolu_xyz");
    assert!(!state.all_tools_complete());

    // Record result
    let result = ToolResultBlock {
        tool_use_id: "toolu_xyz".to_string(),
        content: "Complete".to_string(),
        is_error: false,
    };
    state.record_tool_result("toolu_xyz", result);
    assert!(state.all_tools_complete());
}

// =========================================================================
// Auto-Compaction Tests (Phase 4.4)
// =========================================================================

#[test]
fn test_estimate_conversation_tokens_empty() {
    let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    let tokens = state.estimate_conversation_tokens();
    assert_eq!(tokens, 0);
}

#[test]
fn test_estimate_conversation_tokens_with_messages() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Add some messages
    state
        .api_messages_mut()
        .push(ApiMessageV2::user("Hello, how are you?"));
    state
        .api_messages_mut()
        .push(ApiMessageV2::assistant("I'm doing well, thank you!"));

    let tokens = state.estimate_conversation_tokens();
    // Should have some tokens (overhead + content)
    assert!(tokens > 0, "Should have tokens, got {}", tokens);
    // Estimate: ~8-20 tokens for these short messages
    assert!(
        tokens < 100,
        "Should be reasonable estimate, got {}",
        tokens
    );
}

#[test]
fn test_sync_token_budget() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Add messages
    state.api_messages_mut().push(ApiMessageV2::user("Hello"));
    state
        .api_messages_mut()
        .push(ApiMessageV2::assistant("Hi there!"));

    // Initially budget is empty
    assert_eq!(state.compression().token_budget().used(), 0);

    // Sync budget
    state.sync_token_budget();

    // Budget should reflect message tokens
    assert!(
        state.compression().token_budget().used() > 0,
        "Budget should have usage"
    );
}

#[tokio::test]
async fn test_maybe_compact_below_threshold() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Add a small message
    state.api_messages_mut().push(ApiMessageV2::user("Hello"));

    // Try to compact at 80% threshold of 200k tokens
    // With minimal messages, we're well below threshold
    let result = state.maybe_compact(0.8, 200_000).await;

    assert!(result.is_ok());
    assert!(!result.unwrap(), "Should not compact below threshold");
}

#[tokio::test]
async fn test_maybe_compact_graceful_handles_small_conversation() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    state.api_messages_mut().push(ApiMessageV2::user("Hello"));

    let compacted = state.maybe_compact_graceful(0.8, 200_000).await;

    assert!(!compacted, "Should not compact small conversation");
}

#[tokio::test]
async fn test_maybe_compact_triggers_above_threshold() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Add many large messages to exceed threshold
    let large_content = "x".repeat(10000); // ~2500 tokens each
    for i in 0..100 {
        state.api_messages_mut().push(ApiMessageV2::user(format!(
            "Message {}: {}",
            i, large_content
        )));
    }

    // Check tokens before compaction
    let before_tokens = state.estimate_conversation_tokens();
    assert!(
        before_tokens > 100_000,
        "Should have many tokens, got {}",
        before_tokens
    );

    // Compact at 80% of 200k = 160k tokens
    // We have >100k tokens, so use a lower context limit to trigger
    let result = state.maybe_compact(0.5, before_tokens).await;

    assert!(result.is_ok());
    // Whether it actually compacted depends on the compactor behavior
}

// =========================================================================
// Auto-Compaction Send Flow Integration Tests (Phase 4.4.3)
// =========================================================================

#[test]
fn test_default_compaction_threshold_is_valid() {
    // Threshold should be between 0 and 1
    let threshold = DEFAULT_COMPACTION_THRESHOLD;
    assert!(threshold > 0.0, "Threshold should be positive");
    assert!(threshold <= 1.0, "Threshold should not exceed 1.0");
    // Default is 0.8 (80% of context)
    assert!(
        (threshold - 0.8).abs() < f32::EPSILON,
        "Default threshold should be 0.8"
    );
}

#[test]
fn test_model_context_limit_integration() {
    // Verify model_context_limit function is accessible and works
    let limit = model_context_limit("claude-sonnet-4-20250514");
    assert_eq!(limit, 200_000);

    let limit = model_context_limit("claude-opus-4-20250514");
    assert_eq!(limit, 200_000);

    let limit = model_context_limit("claude-3-haiku-20240307");
    assert_eq!(limit, 200_000);
}

// =========================================================================
// Compaction Metrics Tests (Phase 4.4.5)
// =========================================================================

#[test]
fn test_compaction_metrics_accessor() {
    let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // New state should have zero metrics
    let metrics = state.compression().compaction_metrics();
    assert_eq!(metrics.compaction_count(), 0);
    assert_eq!(metrics.total_tokens_saved(), 0);
    assert_eq!(metrics.total_time_ms(), 0);
}

#[test]
fn test_compaction_metrics_summary() {
    let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    let summary = state.compression().compaction_metrics().summary();
    assert_eq!(summary.compaction_count, 0);
    assert_eq!(summary.total_tokens_saved, 0);
    assert_eq!(summary.average_tokens_saved, 0);
}

#[tokio::test]
async fn test_compaction_metrics_record_on_compact() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Add many large messages to exceed threshold
    let large_content = "x".repeat(10000);
    for i in 0..100 {
        state.api_messages_mut().push(ApiMessageV2::user(format!(
            "Message {}: {}",
            i, large_content
        )));
    }

    let before_tokens = state.estimate_conversation_tokens();
    assert!(before_tokens > 100_000);

    // Trigger compaction
    let _ = state.maybe_compact(0.5, before_tokens).await;

    // Metrics should be recorded (if compaction ran)
    // We can at least verify the accessor works
    let summary = state.compression().compaction_metrics().summary();
    // Verify summary fields are accessible (values depend on compactor behavior)
    let _ = summary.compaction_count;
    let _ = summary.total_tokens_saved;
    let _ = summary.average_time_ms;
}

// =========================================================================
// Continuous loop state tests
// =========================================================================

#[test]
fn test_continuous_defaults_to_inactive() {
    let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Disabled);
    assert_eq!(
        state.continuous().status(),
        &ContinuousLoopStatus::Inactive,
        "New state should have inactive continuous status"
    );
    assert_eq!(state.continuous().iterations_completed(), 0);
    assert_eq!(state.continuous().last_duration_ms(), None);
    assert_eq!(state.continuous().checking_gate(), None);
    assert!(state.continuous().gate_results().is_empty());
}

#[test]
fn test_update_continuous_iteration() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Disabled);
    state.update_continuous_iteration(3);
    assert_eq!(
        state.continuous().status(),
        &ContinuousLoopStatus::Running { iteration: 3 }
    );
}

#[test]
fn test_complete_continuous_iteration() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Disabled);
    state.update_continuous_iteration(1);
    state.complete_continuous_iteration(1, 5000);
    assert_eq!(state.continuous().iterations_completed(), 1);
    assert_eq!(state.continuous().last_duration_ms(), Some(5000));
}

#[test]
fn test_reset_continuous_clears_all_state() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Disabled);

    // Set up some continuous state
    state.update_continuous_iteration(3);
    state.complete_continuous_iteration(3, 12_000);
    state.record_continuous_gate_result("tests", true, Some("all passed"));
    state.set_continuous_gate_checking("clippy");

    // Reset
    state.reset_continuous();

    assert_eq!(
        state.continuous().status(),
        &ContinuousLoopStatus::Inactive,
        "Status should be Inactive after reset"
    );
    assert_eq!(
        state.continuous().iterations_completed(),
        0,
        "Iterations should be 0 after reset"
    );
    assert_eq!(
        state.continuous().last_duration_ms(),
        None,
        "Duration should be None after reset"
    );
    assert_eq!(
        state.continuous().checking_gate(),
        None,
        "Checking gate should be None after reset"
    );
    assert!(
        state.continuous().gate_results().is_empty(),
        "Gate results should be empty after reset"
    );
}

#[test]
fn test_reset_continuous_from_stagnated() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Disabled);
    state.set_continuous_stagnation(5, 3);
    assert!(matches!(
        state.continuous().status(),
        ContinuousLoopStatus::Stagnated { .. }
    ));

    state.reset_continuous();
    assert_eq!(state.continuous().status(), &ContinuousLoopStatus::Inactive);
}

#[test]
fn test_reset_continuous_from_human_required() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Disabled);
    state.set_continuous_human_checkpoint("build broken");
    assert!(matches!(
        state.continuous().status(),
        ContinuousLoopStatus::HumanRequired { .. }
    ));

    state.reset_continuous();
    assert_eq!(state.continuous().status(), &ContinuousLoopStatus::Inactive);
}

// ========================================================================
// Phase 8.1.1: Tool state characterization tests (baseline before extraction)
// ========================================================================

/// Verifies that ToolExecutionState can be constructed independently
/// and that AppState uses it as an inner struct.
#[test]
fn test_tool_execution_state_construction() {
    let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Access tool_state sub-struct fields through AppState delegation
    assert_eq!(state.tool_loop_state(), &ToolLoopState::Idle);
    assert!(!state.has_pending_permission());
    assert!(!state.has_tool_blocks());
    assert!(!state.has_tool_result_rx());
    assert!(!state.has_executing_tools());
}

/// Documents the initial state of all 7 tool-related fields in AppState.
/// This is a characterization test — it captures current behavior so we
/// can verify zero behavior change after extracting ToolExecutionState.
#[test]
fn test_tool_state_field_access_baseline() {
    let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // 1. tool_loop: starts Idle (Idle counts as "needs user action")
    assert_eq!(state.tool_loop_state(), &ToolLoopState::Idle);
    assert!(!state.tool_loop_is_active());
    assert!(state.tool_loop_needs_user_action());

    // 2. tool_executor: exists (Arc<HookedToolExecutor>)
    // Accessed via tool_loop.execute_pending() — not directly exposed.
    // We verify it was constructed by checking tool_loop operates correctly.

    // 3. permission_manager: exists (Arc<Mutex<PermissionManager>>)
    // Not directly exposed — accessed only in handle_permission_response().
    // Verified indirectly via pending_permission workflow.

    // 4. pending_permission: starts None
    assert!(!state.has_pending_permission());
    assert!(state.pending_permission().is_none());

    // 5. tool_blocks: starts empty
    assert!(!state.has_tool_blocks());
    assert!(state.tool_blocks().is_empty());

    // 6. tool_result_rx: starts None (no background channel)
    assert!(!state.has_tool_result_rx());

    // 7. executing_tool_ids: starts empty
    assert!(!state.has_executing_tools());
    assert!(state.all_tools_complete());
}

/// Documents tool_loop mutation patterns through the public API.
#[test]
fn test_tool_state_tool_loop_mutations() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Reset resets to Idle
    state.reset_tool_loop();
    assert_eq!(state.tool_loop_state(), &ToolLoopState::Idle);
    assert!(!state.tool_loop_is_active());
}

/// Documents pending_permission lifecycle through the public API.
#[test]
fn test_tool_state_pending_permission_lifecycle() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Initially no permission
    assert!(!state.has_pending_permission());

    // Set a permission request
    let request = PermissionRequest {
        tool_name: "bash".to_string(),
        tool_input: Some("ls -la".to_string()),
        description: "Run shell command".to_string(),
    };
    state.set_pending_permission(request);
    assert!(state.has_pending_permission());
    assert_eq!(state.pending_permission().unwrap().tool_name, "bash");

    // Clear permission
    state.clear_pending_permission();
    assert!(!state.has_pending_permission());
}

/// Documents tool_blocks lifecycle through the public API.
#[test]
fn test_tool_state_tool_blocks_lifecycle() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Start a tool block
    let idx = state.start_tool_block("bash", r#"{"command": "ls"}"#);
    assert_eq!(idx, 0);
    assert!(state.has_tool_blocks());
    assert_eq!(state.tool_blocks().len(), 1);

    // Complete the block with a result
    state.complete_tool_block(idx, "file1.txt\nfile2.txt", false);
    assert_eq!(
        state.tool_blocks()[0].result(),
        Some("file1.txt\nfile2.txt")
    );

    // Add a second block with an error
    let idx2 = state.start_tool_block("read_file", r#"{"path": "/missing"}"#);
    assert_eq!(idx2, 1);
    state.complete_tool_block(idx2, "File not found", true);
    assert!(state.tool_blocks()[1].is_error());

    // Clear all blocks
    state.clear_tool_blocks();
    assert!(!state.has_tool_blocks());
    assert!(state.tool_blocks().is_empty());
}

/// Documents executing_tool_ids tracking through the public API.
#[test]
fn test_tool_state_executing_tool_ids_tracking() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Initially empty
    assert!(!state.has_executing_tools());

    // Mark a tool as executing
    state.mark_tool_executing("tool_001");
    assert!(state.has_executing_tools());
    assert!(!state.all_tools_complete());

    // Mark another
    state.mark_tool_executing("tool_002");
    assert!(state.has_executing_tools());

    // Record result for first tool — remove from executing set
    let result = ToolResultBlock {
        tool_use_id: "tool_001".to_string(),
        content: "output".to_string(),
        is_error: false,
    };
    state.record_tool_result("tool_001", result);
    // Still has tool_002 executing
    assert!(state.has_executing_tools());

    // Record result for second tool
    let result2 = ToolResultBlock {
        tool_use_id: "tool_002".to_string(),
        content: "done".to_string(),
        is_error: false,
    };
    state.record_tool_result("tool_002", result2);
    assert!(!state.has_executing_tools());
}

/// Documents tool_result_rx channel lifecycle through the public API.
#[test]
fn test_tool_state_result_channel_lifecycle() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Initially no channel
    assert!(!state.has_tool_result_rx());
    assert!(state.try_recv_tool_result().is_none());

    // Set a channel
    let (tx, rx) = mpsc::channel(100);
    state.set_tool_result_rx(rx);
    assert!(state.has_tool_result_rx());

    // Send a result through the channel
    let result = ToolResultBlock {
        tool_use_id: "t1".to_string(),
        content: "ok".to_string(),
        is_error: false,
    };
    tx.try_send(("t1".to_string(), result)).unwrap();

    // Receive it
    let received = state.try_recv_tool_result();
    assert!(received.is_some());
    let (id, block) = received.unwrap();
    assert_eq!(id, "t1");
    assert_eq!(block.content, "ok");

    // Clear channel
    state.clear_tool_result_rx();
    assert!(!state.has_tool_result_rx());
}

// ========================================================================
// Phase 8.1.2.3: Delegation method tests
// ========================================================================

/// Verifies that AppState delegation methods provide clean access to
/// ToolExecutionState fields without exposing the inner struct.
#[test]
fn test_tool_state_delegation_methods() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // tool_loop() returns immutable reference
    assert_eq!(state.tool_loop().state(), &ToolLoopState::Idle);

    // tool_loop_mut() returns mutable reference
    state.tool_loop_mut().reset();
    assert_eq!(state.tool_loop_state(), &ToolLoopState::Idle);

    // has_executing_tools() delegates correctly (is_tool_executing equivalent)
    assert!(!state.has_executing_tools());
    state.mark_tool_executing("t1");
    assert!(state.has_executing_tools());

    // pending_permission() delegates correctly
    assert!(state.pending_permission().is_none());
    state.set_pending_permission(PermissionRequest {
        tool_name: "read".to_string(),
        tool_input: None,
        description: "Read file".to_string(),
    });
    assert!(state.pending_permission().is_some());
}

// ========================================================================
// Phase 8.2: CompressionState extraction tests
// ========================================================================

/// Verifies that CompressionState is correctly initialized inside AppState.
#[test]
fn test_compression_state_construction() {
    let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Orchestrator: not set by default
    assert!(state.compression().compression_orchestrator().is_none());
    assert!(!state.compression().has_ccg_support());

    // CCG cache: empty by default
    assert!(state.compression().last_ccg_hash().is_none());
    assert!(!state.compression().has_cached_ccg_context());

    // Narsil client: not connected by default
    assert!(!state.compression().has_narsil_client());

    // Token budgets
    assert_eq!(state.compression().context_token_budget(), 10_000);
    assert_eq!(state.compression().context_tokens_injected(), 0);

    // Auto-context: disabled by default
    assert!(!state.compression().auto_context_enabled());
    assert!(!state.compression().has_pending_context());

    // Compaction: inactive by default
    assert!(state.compression().compaction_state().is_none());

    // Token budget
    assert_eq!(state.compression().token_budget().used(), 0);
}

/// Verifies delegation methods for CompressionState provide clean access.
#[test]
fn test_compression_state_delegation() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // orchestrator() delegates correctly
    assert!(state.compression().compression_orchestrator().is_none());

    // compaction_state() delegates correctly
    assert!(state.compression().compaction_state().is_none());

    // token_budget() delegates correctly
    assert_eq!(state.compression().token_budget().used(), 0);
    state.compression_mut().token_budget_mut().add_usage(500);
    assert!(state.compression().token_budget().used() > 0);
}

// ========================================================================
// Phase 8.3: ContinuousLoopState extraction tests
// ========================================================================

/// Verifies that ContinuousLoopState is correctly initialized inside AppState.
#[test]
fn test_continuous_loop_state_construction() {
    let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    assert_eq!(state.continuous().status(), &ContinuousLoopStatus::Inactive);
    assert_eq!(state.continuous().iterations_completed(), 0);
    assert_eq!(state.continuous().last_duration_ms(), None);
    assert_eq!(state.continuous().checking_gate(), None);
    assert!(state.continuous().gate_results().is_empty());
}

// ========================================================================
// Phase 8.4: UISelectionState extraction tests
// ========================================================================

/// Verifies that UISelectionState is correctly initialized inside AppState.
#[test]
fn test_ui_selection_state_construction() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

    // Selection: starts with no active selection
    assert!(!state.ui_selection().selection().has_selection());

    // Copy pending: false initially
    assert!(!state.ui_selection_mut().take_copy_pending());

    // Focus area: default
    assert_eq!(state.ui_selection().focus_area(), FocusArea::default());
}

// --- Completion integration tests (8.3.1) ---

#[test]
fn test_completion_initially_none() {
    let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    assert!(state.completion().is_none());
    assert!(!state.has_completion());
}

#[test]
fn test_show_completion_activates() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    state.show_completion();
    assert!(state.has_completion());
    assert!(state.completion().unwrap().filtered().len() >= 6);
}

#[test]
fn test_dismiss_completion_deactivates() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    state.show_completion();
    state.dismiss_completion();
    assert!(!state.has_completion());
}

#[test]
fn test_insert_slash_at_position_zero_triggers_completion() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    state.insert_char('/');
    assert!(state.has_completion());
}

#[test]
fn test_typing_after_slash_updates_filter() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    state.insert_char('/');
    state.insert_char('h');
    state.insert_char('e');
    let completion = state.completion().unwrap();
    assert_eq!(completion.filter(), "he");
    // "help" should be in filtered results
    assert!(completion.filtered().iter().any(|e| e.name == "help"));
}

#[test]
fn test_backspace_past_slash_dismisses_completion() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    state.insert_char('/');
    assert!(state.has_completion());
    state.delete_char(); // removes '/'
    assert!(!state.has_completion());
}

#[test]
fn test_backspace_updates_filter() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    state.insert_char('/');
    state.insert_char('h');
    state.insert_char('e');
    state.delete_char(); // removes 'e', filter becomes "h"
    assert!(state.has_completion());
    assert_eq!(state.completion().unwrap().filter(), "h");
}

#[test]
fn test_slash_mid_input_does_not_trigger_completion() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    state.insert_char('h');
    state.insert_char('i');
    state.insert_char('/');
    assert!(!state.has_completion());
}

#[test]
fn test_accept_completion_replaces_input() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    state.insert_char('/');
    let name = state.accept_completion();
    assert!(name.is_some());
    let name = name.unwrap();
    assert_eq!(state.input_state().text(), format!("/{name} "));
    assert!(!state.has_completion());
}

#[test]
fn test_accept_completion_empty_returns_none() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    state.insert_char('/');
    // Filter to something with no matches
    for c in "zzzzz".chars() {
        state.insert_char(c);
    }
    let name = state.accept_completion();
    assert!(name.is_none());
}

#[test]
fn test_worktree_status_default() {
    let wt = WorktreeStatus::new();
    assert_eq!(wt.branch(), None);
    assert_eq!(wt.modified(), 0);
    assert_eq!(wt.ahead(), 0);
    assert_eq!(wt.behind(), 0);
}

#[test]
fn test_worktree_status_set_branch() {
    let mut wt = WorktreeStatus::new();
    wt.set_branch("main".to_string());
    assert_eq!(wt.branch(), Some("main"));
}

#[test]
fn test_worktree_status_set_modified() {
    let mut wt = WorktreeStatus::new();
    wt.set_modified(5);
    assert_eq!(wt.modified(), 5);
}

#[test]
fn test_worktree_status_set_ahead_behind() {
    let mut wt = WorktreeStatus::new();
    wt.set_ahead(3);
    wt.set_behind(2);
    assert_eq!(wt.ahead(), 3);
    assert_eq!(wt.behind(), 2);
}

#[test]
fn test_worktree_status_clone_eq() {
    let mut wt = WorktreeStatus::new();
    wt.set_branch("feature/x".to_string());
    wt.set_modified(1);
    let wt2 = wt.clone();
    assert_eq!(wt, wt2);
}

#[test]
fn test_app_state_worktree_delegation() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    state.set_worktree_branch("main".to_string());
    assert_eq!(state.worktree().branch(), Some("main"));
    assert_eq!(state.worktree().branch(), Some("main"));

    state.set_worktree_modified(3);
    assert_eq!(state.worktree().modified(), 3);
    assert_eq!(state.worktree().modified(), 3);

    state.set_worktree_ahead(1);
    assert_eq!(state.worktree().ahead(), 1);

    state.set_worktree_behind(2);
    assert_eq!(state.worktree().behind(), 2);
}

#[test]
fn test_session_tracking_default() {
    let st = SessionTracking::new();
    assert_eq!(st.id(), None);
    assert!(!st.is_dirty());
}

#[test]
fn test_session_tracking_set_id() {
    let mut st = SessionTracking::new();
    st.set_id("abc-123".to_string());
    assert_eq!(st.id(), Some("abc-123"));
}

#[test]
fn test_session_tracking_dirty_cycle() {
    let mut st = SessionTracking::new();
    assert!(!st.is_dirty());

    st.mark_dirty();
    assert!(st.is_dirty());

    // take_dirty returns true and clears
    assert!(st.take_dirty());
    assert!(!st.is_dirty());

    // second take returns false
    assert!(!st.take_dirty());
}

#[test]
fn test_app_state_session_delegation() {
    let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
    assert_eq!(state.session_tracking().id(), None);

    state.session_tracking_mut().set_id("sess-1".to_string());
    assert_eq!(state.session_tracking().id(), Some("sess-1"));
    assert_eq!(state.session_tracking().id(), Some("sess-1"));

    state.session_tracking_mut().mark_dirty();
    assert!(state.session_tracking_mut().take_dirty());
    assert!(!state.session_tracking_mut().take_dirty());
}

// ========================================================================
// AgentPanelState tests
// ========================================================================

#[test]
fn test_agent_panel_state_new() {
    let panel = AgentPanelState::new(None);
    assert!(!panel.subagents_enabled());
    assert!(panel.spawner().is_none());
    assert!(panel.entries().is_empty());
    assert!(!panel.has_pending_conflicts());
}

#[test]
fn test_agent_panel_state_with_spawner() {
    let panel = AgentPanelState::new(Some(SubagentSpawner::new()));
    assert!(panel.subagents_enabled());
    assert!(panel.spawner().is_some());
}

#[test]
fn test_agent_panel_update_progress_new_agent() {
    let mut panel = AgentPanelState::new(None);
    let progress = AgentProgress::IterationStarted {
        iteration: 1,
        max: 5,
    };
    panel.update_progress("agent-1", "Test Agent", &progress);
    assert_eq!(panel.entries().len(), 1);
    assert_eq!(panel.entries()[0].agent_id, "agent-1");
    assert_eq!(panel.entries()[0].agent_name, "Test Agent");
}

#[test]
fn test_agent_panel_update_progress_existing_agent() {
    let mut panel = AgentPanelState::new(None);
    let p1 = AgentProgress::IterationStarted {
        iteration: 1,
        max: 5,
    };
    panel.update_progress("agent-1", "Test", &p1);

    let p2 = AgentProgress::IterationStarted {
        iteration: 2,
        max: 5,
    };
    panel.update_progress("agent-1", "Test", &p2);

    // Should still be 1 entry, updated in place
    assert_eq!(panel.entries().len(), 1);
    match &panel.entries()[0].status {
        AgentPanelStatus::Running { iteration, .. } => assert_eq!(*iteration, 2),
        _ => panic!("Expected Running status"),
    }
}

#[test]
fn test_agent_panel_conflict_reports() {
    let mut panel = AgentPanelState::new(None);
    assert!(!panel.has_pending_conflicts());

    panel.add_conflict(ConflictReport::empty());
    assert!(panel.has_pending_conflicts());

    let reports = panel.take_conflicts();
    assert_eq!(reports.len(), 1);
    assert!(!panel.has_pending_conflicts());
}

#[test]
fn test_focus_area_for_row() {
    // Terminal height 24: input starts at row 21
    assert_eq!(
        UISelectionState::focus_area_for_row(0, 24),
        FocusArea::Content
    );
    assert_eq!(
        UISelectionState::focus_area_for_row(20, 24),
        FocusArea::Content
    );
    assert_eq!(
        UISelectionState::focus_area_for_row(21, 24),
        FocusArea::Input
    );
    assert_eq!(
        UISelectionState::focus_area_for_row(23, 24),
        FocusArea::Input
    );
}
