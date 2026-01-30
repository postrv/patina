//! TUI snapshot tests using insta.
//!
//! These tests capture the rendered terminal output and compare against snapshots.
//! Run `cargo insta test` to run tests and `cargo insta review` to accept new snapshots.

use ratatui::{backend::TestBackend, Terminal};
use rct::app::state::AppState;
use rct::tui::render;
use rct::types::{Message, Role};
use std::path::PathBuf;

/// Helper to render state to a string buffer for snapshot testing.
fn render_to_string(state: &AppState, width: u16, height: u16) -> String {
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
    AppState::new(PathBuf::from("/tmp/test"))
}

// ============================================================================
// Empty State Tests
// ============================================================================

/// Tests rendering of an empty state - no messages, no input.
#[test]
fn test_empty_state_render() {
    let state = new_state();
    let output = render_to_string(&state, 60, 20);
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

    let output = render_to_string(&state, 60, 20);
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

    let output = render_to_string(&state, 60, 20);
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

    let output = render_to_string(&state, 60, 20);
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

    let output = render_to_string(&state, 80, 25);
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

    let output = render_to_string(&state, 60, 20);
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

    // Simulate streaming response - set current_response and loading state
    // We need to access internal state, so we'll use a helper approach
    // For now, just test with the current_response field set directly
    state.current_response = Some("Rust is a systems programming language that...".to_string());

    let output = render_to_string(&state, 60, 20);
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
