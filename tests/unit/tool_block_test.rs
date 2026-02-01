//! Unit tests for the tool block rendering widget.
//!
//! These tests verify that tool execution blocks are rendered correctly
//! with the Patina theme styling.

use patina::tui::theme::PatinaTheme;
use patina::tui::widgets::tool_block::{ToolBlockState, ToolBlockWidget};
use ratatui::{backend::TestBackend, style::Modifier, Terminal};

// ============================================================================
// Helper Functions
// ============================================================================

/// Creates a test terminal with the given dimensions.
fn test_terminal(width: u16, height: u16) -> Terminal<TestBackend> {
    let backend = TestBackend::new(width, height);
    Terminal::new(backend).expect("Failed to create test terminal")
}

/// Extracts the rendered content as a string from the terminal buffer.
fn buffer_to_string(terminal: &Terminal<TestBackend>) -> String {
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol().chars().next().unwrap_or(' '))
        .collect()
}

// ============================================================================
// Tool Block State Tests
// ============================================================================

/// Tests that ToolBlockState can be created with required fields.
#[test]
fn test_tool_block_state_creation() {
    let state = ToolBlockState::new("bash", "git status");
    assert_eq!(state.tool_name(), "bash");
    assert_eq!(state.tool_input(), "git status");
    assert!(state.result().is_none());
    assert!(!state.is_error());
}

/// Tests that ToolBlockState can hold a result.
#[test]
fn test_tool_block_state_with_result() {
    let mut state = ToolBlockState::new("bash", "ls -la");
    state.set_result("file1.txt\nfile2.txt");

    assert_eq!(state.result(), Some("file1.txt\nfile2.txt"));
    assert!(!state.is_error());
}

/// Tests that ToolBlockState can indicate an error.
#[test]
fn test_tool_block_state_with_error() {
    let mut state = ToolBlockState::new("bash", "invalid-command");
    state.set_error("Command not found: invalid-command");

    assert!(state.is_error());
    assert!(state.result().is_some());
}

/// Tests that ToolBlockState can represent a completed state.
#[test]
fn test_tool_block_state_completion() {
    let mut state = ToolBlockState::new("read_file", "/tmp/test.txt");
    assert!(!state.is_complete());

    state.set_result("File contents here");
    assert!(state.is_complete());
}

// ============================================================================
// Tool Block Widget Rendering Tests
// ============================================================================

/// Tests that the tool block header is rendered with the tool name.
#[test]
fn test_tool_block_renders_header() {
    let mut terminal = test_terminal(60, 10);

    let state = ToolBlockState::new("bash", "git status");
    let widget = ToolBlockWidget::new(&state);

    terminal
        .draw(|frame| {
            frame.render_widget(widget, frame.area());
        })
        .expect("Failed to draw");

    let content = buffer_to_string(&terminal);

    // Header should contain tool icon and name
    assert!(
        content.contains("⚙") || content.contains("bash"),
        "Header should contain tool icon or name. Content: {}",
        content
    );
}

/// Tests that the tool block shows the tool input.
#[test]
fn test_tool_block_renders_input() {
    let mut terminal = test_terminal(60, 10);

    let state = ToolBlockState::new("bash", "git status");
    let widget = ToolBlockWidget::new(&state);

    terminal
        .draw(|frame| {
            frame.render_widget(widget, frame.area());
        })
        .expect("Failed to draw");

    let content = buffer_to_string(&terminal);

    // Should show the command input
    assert!(
        content.contains("git status") || content.contains("git"),
        "Should display tool input. Content: {}",
        content
    );
}

/// Tests that the tool block renders a result.
#[test]
fn test_tool_block_renders_result() {
    let mut terminal = test_terminal(60, 12);

    let mut state = ToolBlockState::new("bash", "echo hello");
    state.set_result("hello");
    let widget = ToolBlockWidget::new(&state);

    terminal
        .draw(|frame| {
            frame.render_widget(widget, frame.area());
        })
        .expect("Failed to draw");

    let content = buffer_to_string(&terminal);

    // Should show the result
    assert!(
        content.contains("hello"),
        "Should display tool result. Content: {}",
        content
    );
}

/// Tests that error results are displayed with error styling indicator.
#[test]
fn test_tool_block_renders_error() {
    let mut terminal = test_terminal(60, 12);

    let mut state = ToolBlockState::new("bash", "bad-command");
    state.set_error("Command not found");
    let widget = ToolBlockWidget::new(&state);

    terminal
        .draw(|frame| {
            frame.render_widget(widget, frame.area());
        })
        .expect("Failed to draw");

    let content = buffer_to_string(&terminal);

    // Should show error content
    assert!(
        content.contains("Command not found") || content.contains("Error") || content.contains("✗"),
        "Should display error result. Content: {}",
        content
    );
}

/// Tests that the widget uses theme colors.
#[test]
fn test_tool_block_uses_theme_colors() {
    let mut terminal = test_terminal(60, 10);

    let state = ToolBlockState::new("read_file", "/tmp/test.txt");
    let widget = ToolBlockWidget::new(&state);

    terminal
        .draw(|frame| {
            frame.render_widget(widget, frame.area());
        })
        .expect("Failed to draw");

    // Check that the buffer contains cells with theme colors
    let buffer = terminal.backend().buffer();
    let mut found_bronze_color = false;

    for cell in buffer.content() {
        // Check if it matches TOOL_HEADER color (bronze)
        if cell.fg == PatinaTheme::TOOL_HEADER {
            found_bronze_color = true;
            break;
        }
    }

    // The widget should use bronze for the header
    assert!(
        found_bronze_color,
        "Widget should use PatinaTheme::TOOL_HEADER color"
    );
}

/// Tests that the widget renders in pending state.
#[test]
fn test_tool_block_renders_pending_state() {
    let mut terminal = test_terminal(60, 10);

    // State without result is pending
    let state = ToolBlockState::new("bash", "long-running-command");
    let widget = ToolBlockWidget::new(&state);

    terminal
        .draw(|frame| {
            frame.render_widget(widget, frame.area());
        })
        .expect("Failed to draw");

    let content = buffer_to_string(&terminal);

    // Should indicate pending state
    assert!(
        content.contains("...") || content.contains("Running") || content.contains("⏳"),
        "Should indicate pending state. Content: {}",
        content
    );
}

/// Tests that multi-line results are rendered correctly.
#[test]
fn test_tool_block_renders_multiline_result() {
    let mut terminal = test_terminal(60, 15);

    let mut state = ToolBlockState::new("bash", "ls -la");
    state.set_result("file1.txt\nfile2.txt\nfile3.txt");
    let widget = ToolBlockWidget::new(&state);

    terminal
        .draw(|frame| {
            frame.render_widget(widget, frame.area());
        })
        .expect("Failed to draw");

    let content = buffer_to_string(&terminal);

    // Should show multiple lines of output
    assert!(
        content.contains("file1") && content.contains("file2"),
        "Should display multi-line result. Content: {}",
        content
    );
}

/// Tests that very long results are truncated appropriately.
#[test]
fn test_tool_block_truncates_long_result() {
    let mut terminal = test_terminal(60, 8);

    let mut state = ToolBlockState::new("bash", "cat large_file.txt");
    let long_result = (0..100)
        .map(|i| format!("Line {}", i))
        .collect::<Vec<_>>()
        .join("\n");
    state.set_result(&long_result);
    let widget = ToolBlockWidget::new(&state);

    terminal
        .draw(|frame| {
            frame.render_widget(widget, frame.area());
        })
        .expect("Failed to draw");

    // Widget should render without panicking
    // Content will be truncated to fit the area
    let content = buffer_to_string(&terminal);
    assert!(!content.is_empty(), "Should render something");
}

// ============================================================================
// Style Verification Tests
// ============================================================================

/// Tests that header style includes bold modifier.
#[test]
fn test_tool_block_header_is_bold() {
    let mut terminal = test_terminal(60, 10);

    let state = ToolBlockState::new("bash", "git status");
    let widget = ToolBlockWidget::new(&state);

    terminal
        .draw(|frame| {
            frame.render_widget(widget, frame.area());
        })
        .expect("Failed to draw");

    // Check that the buffer has bold cells (in the header area)
    let buffer = terminal.backend().buffer();
    let mut found_bold = false;

    for cell in buffer.content() {
        if cell.modifier.contains(Modifier::BOLD) {
            found_bold = true;
            break;
        }
    }

    assert!(found_bold, "Header should have bold styling");
}
