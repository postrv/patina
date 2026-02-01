//! TUI snapshot tests using insta.
//!
//! These tests capture the rendered terminal output and compare against snapshots.
//! Run `cargo insta test` to run tests and `cargo insta review` to accept new snapshots.

use patina::app::state::AppState;
use patina::tui::render;
use patina::types::{Message, Role};
use ratatui::{backend::TestBackend, Terminal};
use std::path::PathBuf;

/// Helper to render state to a string buffer for snapshot testing.
fn render_to_string(state: &mut AppState, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("Failed to create terminal");

    terminal
        .draw(|frame| render(frame, state))
        .expect("Failed to draw");

    // Convert buffer to string for snapshot
    let buffer = terminal.backend().buffer();
    let mut output = String::new();

    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, y)];
            output.push_str(cell.symbol());
        }
        output.push('\n');
    }

    output
}

/// Helper to create a new AppState for testing.
fn new_state() -> AppState {
    AppState::new(PathBuf::from("/tmp/test"), false)
}

// ============================================================================
// Empty State Tests
// ============================================================================

/// Tests rendering of an empty state - no messages, no input.
#[test]
fn test_empty_state_render() {
    let mut state = new_state();
    let output = render_to_string(&mut state, 60, 20);
    insta::assert_snapshot!(output);
}

/// Tests rendering with only input text, no messages.
#[test]
fn test_input_only_render() {
    let mut state = new_state();
    state.insert_char('H');
    state.insert_char('e');
    state.insert_char('l');
    state.insert_char('l');
    state.insert_char('o');

    let output = render_to_string(&mut state, 60, 20);
    insta::assert_snapshot!(output);
}

// ============================================================================
// Message Rendering Tests
// ============================================================================

/// Tests rendering a single user message.
#[test]
fn test_single_user_message_render() {
    let mut state = new_state();
    state.add_message(Message {
        role: Role::User,
        content: "Hello, Claude!".to_string(),
    });

    let output = render_to_string(&mut state, 60, 20);
    insta::assert_snapshot!(output);
}

/// Tests rendering a single assistant message.
#[test]
fn test_single_assistant_message_render() {
    let mut state = new_state();
    state.add_message(Message {
        role: Role::Assistant,
        content: "Hello! How can I help you today?".to_string(),
    });

    let output = render_to_string(&mut state, 60, 20);
    insta::assert_snapshot!(output);
}

/// Tests rendering a conversation with multiple messages.
#[test]
fn test_conversation_render() {
    let mut state = new_state();

    state.add_message(Message {
        role: Role::User,
        content: "What is Rust?".to_string(),
    });

    state.add_message(Message {
        role: Role::Assistant,
        content:
            "Rust is a systems programming language focused on safety, speed, and concurrency."
                .to_string(),
    });

    state.add_message(Message {
        role: Role::User,
        content: "What are its main features?".to_string(),
    });

    let output = render_to_string(&mut state, 80, 25);
    insta::assert_snapshot!(output);
}

/// Tests rendering multi-line message content.
#[test]
fn test_multiline_message_render() {
    let mut state = new_state();

    state.add_message(Message {
        role: Role::Assistant,
        content: "Here's a list:\n- First item\n- Second item\n- Third item".to_string(),
    });

    let output = render_to_string(&mut state, 60, 20);
    insta::assert_snapshot!(output);
}

// ============================================================================
// Streaming State Tests
// ============================================================================

/// Tests rendering when streaming a response (loading state with partial content).
#[test]
fn test_streaming_response_render() {
    let mut state = new_state();

    // Add a user message first
    state.add_message(Message {
        role: Role::User,
        content: "Tell me about Rust".to_string(),
    });

    // Simulate streaming response using the timeline API
    state.set_streaming(true);
    state.append_streaming_text("Rust is a systems programming language that...");

    let output = render_to_string(&mut state, 60, 20);
    insta::assert_snapshot!(output);
}

/// Tests throbber animation frames.
/// Verifies that each tick produces a different character.
#[test]
fn test_throbber_animation() {
    let mut state = new_state();

    // Collect all 4 throbber frames
    let mut frames = Vec::new();
    for _ in 0..4 {
        frames.push(state.throbber_char());
        state.tick_throbber();
    }

    // Verify we get 4 distinct frames that cycle
    assert_eq!(frames.len(), 4);
    assert_eq!(frames[0], '⠋');
    assert_eq!(frames[1], '⠙');
    assert_eq!(frames[2], '⠹');
    assert_eq!(frames[3], '⠸');

    // After 4 ticks, should return to first frame
    assert_eq!(state.throbber_char(), '⠋');
}

// ============================================================================
// 3.5.1 TUI Rendering Tests
// ============================================================================

/// Tests that TUI handles Unicode characters correctly.
/// Verifies:
/// - Unicode in messages renders without crashing
/// - Unicode in input renders correctly
/// - Emoji and CJK characters display properly
#[test]
fn test_tui_handles_unicode() {
    let mut state = new_state();

    // Add message with various unicode characters
    state.add_message(Message {
        role: Role::User,
        content: "Hello 你好 こんにちは 🎉🚀".to_string(),
    });

    state.add_message(Message {
        role: Role::Assistant,
        content: "Unicode test: αβγδ ← → ↑ ↓ • ★ ♠ ♣ ♥ ♦".to_string(),
    });

    // Render - should not panic or corrupt
    let output = render_to_string(&mut state, 80, 20);

    // Verify output contains something (rendering didn't fail)
    assert!(!output.is_empty(), "Output should not be empty");
    assert!(output.len() > 100, "Output should have substantial content");
}

/// Tests that Unicode input is handled correctly.
#[test]
fn test_tui_unicode_input() {
    let mut state = new_state();

    // Insert unicode characters
    for c in "你好世界".chars() {
        state.insert_char(c);
    }

    // Verify input state
    assert_eq!(state.input, "你好世界");
    assert_eq!(state.cursor_position(), 4); // 4 unicode characters

    // Cursor navigation should work on characters, not bytes
    state.cursor_left();
    assert_eq!(state.cursor_position(), 3);

    // Backspace deletes character BEFORE cursor (at position 2, which is "世")
    state.delete_char();
    assert_eq!(state.input, "你好界"); // "世" was deleted, "界" remains

    // Render - should not panic
    let output = render_to_string(&mut state, 60, 10);
    assert!(!output.is_empty(), "Unicode input should render");
}

/// Tests that TUI scrolls long content correctly.
/// Verifies:
/// - Scroll offset is applied
/// - Content shifts when scrolling
#[test]
fn test_tui_scrolls_long_content() {
    let mut state = new_state();

    // Add many messages to create scrollable content
    for i in 0..20 {
        state.add_message(Message {
            role: if i % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            },
            content: format!("Message number {} with some content", i),
        });
    }

    // Set viewport and content height for scroll to work
    state.set_viewport_height(15);
    state.update_content_height(100); // Enough content to scroll

    // Render without scroll
    let output_no_scroll = render_to_string(&mut state, 60, 15);

    // Scroll down and render again
    state.scroll_up(5);
    let output_scrolled = render_to_string(&mut state, 60, 15);

    // The outputs should be different (content shifted)
    assert_ne!(
        output_no_scroll, output_scrolled,
        "Scrolling should change the visible content"
    );

    // Verify scroll offset is set
    assert_eq!(state.scroll_offset(), 5);
}

/// Tests that input cursor position tracking works correctly.
#[test]
fn test_tui_input_cursor_visible() {
    let mut state = new_state();

    // Type some text
    for c in "Hello World".chars() {
        state.insert_char(c);
    }

    // Cursor should be at position 11 (after "Hello World")
    assert_eq!(state.cursor_position(), 11);

    // Render
    let output = render_to_string(&mut state, 60, 10);

    // Input should be visible
    assert!(
        output.contains("Hello") || output.contains("World"),
        "Input text should be visible"
    );
}

/// Tests cursor movement within input.
#[test]
fn test_tui_cursor_movement() {
    let mut state = new_state();

    // Type some text
    for c in "Hello".chars() {
        state.insert_char(c);
    }

    assert_eq!(state.cursor_position(), 5);

    // Move to beginning
    state.cursor_home();
    assert_eq!(state.cursor_position(), 0);

    // Move to end
    state.cursor_end();
    assert_eq!(state.cursor_position(), 5);

    // Move left
    state.cursor_left();
    assert_eq!(state.cursor_position(), 4);

    // Move right
    state.cursor_right();
    assert_eq!(state.cursor_position(), 5);

    // Move right at end should not go past
    state.cursor_right();
    assert_eq!(state.cursor_position(), 5);

    // Move to beginning and try left - should stay at 0
    state.cursor_home();
    state.cursor_left();
    assert_eq!(state.cursor_position(), 0);
}

// ============================================================================
// 3.5.2 TUI Event Tests
// ============================================================================

/// Tests that key events are processed correctly.
/// Verifies character insertion, deletion, and navigation.
#[test]
fn test_tui_key_events() {
    let mut state = new_state();

    // Test character insertion
    state.insert_char('A');
    assert_eq!(state.input, "A");

    state.insert_char('B');
    assert_eq!(state.input, "AB");

    state.insert_char('C');
    assert_eq!(state.input, "ABC");

    // Test backspace
    state.delete_char();
    assert_eq!(state.input, "AB");

    // Test cursor movement and insert in middle
    state.cursor_left();
    state.insert_char('X');
    assert_eq!(state.input, "AXB");

    // Test take_input (simulates Enter)
    let taken = state.take_input();
    assert_eq!(taken, "AXB");
    assert_eq!(state.input, "");
    assert_eq!(state.cursor_position(), 0);
}

/// Tests that resize events are handled correctly.
/// The UI should re-render correctly at different sizes.
#[test]
fn test_tui_resize_event() {
    let mut state = new_state();

    state.add_message(Message {
        role: Role::User,
        content: "Test message for resize".to_string(),
    });

    // Render at small size
    let small_output = render_to_string(&mut state, 40, 10);

    // Render at large size
    let large_output = render_to_string(&mut state, 120, 30);

    // Both should produce output
    assert!(
        !small_output.is_empty(),
        "Small render should produce output"
    );
    assert!(
        !large_output.is_empty(),
        "Large render should produce output"
    );

    // Large output should have more characters (more area)
    assert!(
        large_output.len() > small_output.len(),
        "Large render should have more characters"
    );
}

/// Tests that paste events (multiple characters) are handled.
/// This simulates pasting text by inserting multiple characters.
#[test]
fn test_tui_paste_event() {
    let mut state = new_state();

    // Simulate paste by inserting multiple characters quickly
    let pasted_text = "This is pasted content!";
    for c in pasted_text.chars() {
        state.insert_char(c);
    }

    assert_eq!(state.input, pasted_text);
    assert_eq!(state.cursor_position(), pasted_text.chars().count());

    // Render should work after paste
    let output = render_to_string(&mut state, 60, 10);
    assert!(!output.is_empty(), "Render after paste should work");
}

/// Tests that the dirty flag system works correctly.
#[test]
fn test_dirty_flags() {
    let mut state = new_state();

    // Initial state should need render
    assert!(state.needs_render(), "New state should need render");

    // After marking rendered, should not need render
    state.mark_rendered();
    assert!(
        !state.needs_render(),
        "After render, should not need render"
    );

    // Typing should set dirty
    state.insert_char('a');
    assert!(state.needs_render(), "After input, should need render");

    state.mark_rendered();

    // Adding message should set dirty
    state.add_message(Message {
        role: Role::User,
        content: "test".to_string(),
    });
    assert!(state.needs_render(), "After message, should need render");

    state.mark_rendered();

    // Scrolling should set dirty
    state.scroll_up(1);
    assert!(state.needs_render(), "After scroll, should need render");
}

// ============================================================================
// 8.4.2 Status Bar Worktree Indicator Tests
// ============================================================================

/// Tests that the status bar displays the current branch name.
#[test]
fn test_status_bar_shows_branch_name() {
    let mut state = new_state();

    // Set worktree info on state
    state.set_worktree_branch("feature/my-branch".to_string());

    let output = render_to_string(&mut state, 80, 20);

    // The branch name should appear in the status bar
    assert!(
        output.contains("feature/my-branch"),
        "Status bar should show branch name. Output:\n{}",
        output
    );
}

/// Tests that the status bar displays ahead/behind counts.
#[test]
fn test_status_bar_shows_ahead_behind() {
    let mut state = new_state();

    state.set_worktree_branch("main".to_string());
    state.set_worktree_ahead(3);
    state.set_worktree_behind(2);

    let output = render_to_string(&mut state, 80, 20);

    // Should show ahead indicator
    assert!(
        output.contains("↑3"),
        "Status bar should show ahead count. Output:\n{}",
        output
    );

    // Should show behind indicator
    assert!(
        output.contains("↓2"),
        "Status bar should show behind count. Output:\n{}",
        output
    );
}

/// Tests that the status bar displays modified file count.
#[test]
fn test_status_bar_shows_modified_count() {
    let mut state = new_state();

    state.set_worktree_branch("main".to_string());
    state.set_worktree_modified(5);

    let output = render_to_string(&mut state, 80, 20);

    // Should show modified count with dirty indicator
    assert!(
        output.contains("●5") || output.contains("*5"),
        "Status bar should show modified count. Output:\n{}",
        output
    );
}

/// Tests that status bar renders with full worktree status.
#[test]
fn test_status_bar_full_worktree_status() {
    let mut state = new_state();

    state.set_worktree_branch("wt/experiment".to_string());
    state.set_worktree_modified(2);
    state.set_worktree_ahead(1);
    state.set_worktree_behind(0);

    let output = render_to_string(&mut state, 80, 20);
    insta::assert_snapshot!(output);
}

/// Tests that status bar handles clean worktree (no modified files).
#[test]
fn test_status_bar_clean_worktree() {
    let mut state = new_state();

    state.set_worktree_branch("main".to_string());
    // No modified files, no ahead/behind

    let output = render_to_string(&mut state, 80, 20);

    // Branch should still appear
    assert!(
        output.contains("main"),
        "Status bar should show branch even when clean. Output:\n{}",
        output
    );
}
