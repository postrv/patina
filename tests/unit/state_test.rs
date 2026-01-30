//! Unit tests for application state management.
//!
//! These tests verify input handling, scroll behavior, and dirty flag tracking.
//! Following TDD RED phase - cursor movement tests will fail until implemented.

use rct::app::state::AppState;
use std::path::PathBuf;

/// Helper to create a new AppState for testing.
fn new_state() -> AppState {
    AppState::new(PathBuf::from("/tmp/test"))
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
    assert_eq!(state.scroll_offset, 0);

    state.scroll_up(5);
    assert_eq!(state.scroll_offset, 5);

    state.scroll_up(3);
    assert_eq!(state.scroll_offset, 8);
}

/// Tests scroll down decreases scroll offset.
#[test]
fn test_scroll_down() {
    let mut state = new_state();
    state.scroll_up(10);
    assert_eq!(state.scroll_offset, 10);

    state.scroll_down(3);
    assert_eq!(state.scroll_offset, 7);

    state.scroll_down(2);
    assert_eq!(state.scroll_offset, 5);
}

/// Tests scroll bounds saturation - scroll_down at 0 stays at 0.
#[test]
fn test_scroll_bounds_saturation_at_zero() {
    let mut state = new_state();
    assert_eq!(state.scroll_offset, 0);

    // Should not go negative - saturating_sub should keep it at 0
    state.scroll_down(10);
    assert_eq!(state.scroll_offset, 0);
}

/// Tests scroll up saturation - large values don't overflow.
#[test]
fn test_scroll_up_large_values() {
    let mut state = new_state();

    state.scroll_up(usize::MAX / 2);
    let first = state.scroll_offset;

    // saturating_add should prevent overflow
    state.scroll_up(usize::MAX / 2);
    assert!(state.scroll_offset >= first);
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
    use rct::types::{Message, Role};

    let mut state = new_state();
    state.mark_rendered();
    assert!(!state.needs_render());

    state.add_message(Message {
        role: Role::User,
        content: "Hello".to_string(),
    });

    assert!(state.needs_render(), "add_message should set dirty flag");
    assert_eq!(state.messages.len(), 1);
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
