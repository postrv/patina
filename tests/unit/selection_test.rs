//! Tests for text selection functionality.
//!
//! TDD tests for copy/paste support.

use patina::tui::selection::{ContentPosition, SelectionState};
use ratatui::text::{Line, Span};

// ============================================================================
// Selection State Tests
// ============================================================================

#[test]
fn test_selection_start_end() {
    let mut selection = SelectionState::new();

    // Initially no selection
    assert!(!selection.has_selection());
    assert!(selection.range().is_none());

    // Start selection
    selection.start(ContentPosition::new(5, 10));
    assert!(!selection.has_selection()); // Not complete until end() called

    // Update during drag
    selection.update(ContentPosition::new(7, 20));
    assert!(selection.is_selecting()); // In the process of selecting

    // End selection
    selection.end();
    assert!(selection.has_selection());

    // Verify range
    let (start, end) = selection.range().unwrap();
    assert_eq!(start.line, 5);
    assert_eq!(start.col, 10);
    assert_eq!(end.line, 7);
    assert_eq!(end.col, 20);
}

#[test]
fn test_selection_clear() {
    let mut selection = SelectionState::new();

    // Create a selection
    selection.start(ContentPosition::new(0, 0));
    selection.update(ContentPosition::new(5, 10));
    selection.end();
    assert!(selection.has_selection());

    // Clear it
    selection.clear();
    assert!(!selection.has_selection());
    assert!(selection.range().is_none());
}

#[test]
fn test_selection_select_all() {
    let mut selection = SelectionState::new();

    // Select all with 100 lines
    selection.select_all(100);

    assert!(selection.has_selection());
    let (start, end) = selection.range().unwrap();
    assert_eq!(start.line, 0);
    assert_eq!(start.col, 0);
    assert_eq!(end.line, 99);
    assert_eq!(end.col, usize::MAX);
}

#[test]
fn test_selection_range_normalized() {
    let mut selection = SelectionState::new();

    // Select backwards (end before start)
    selection.start(ContentPosition::new(10, 30));
    selection.update(ContentPosition::new(5, 10));
    selection.end();

    // Range should be normalized (start < end)
    let (start, end) = selection.range().unwrap();
    assert!(
        start < end,
        "Range should be normalized: start({:?}) should be < end({:?})",
        start,
        end
    );
    assert_eq!(start.line, 5);
    assert_eq!(end.line, 10);
}

#[test]
fn test_selection_single_line() {
    let mut selection = SelectionState::new();

    // Select within a single line
    selection.start(ContentPosition::new(3, 5));
    selection.update(ContentPosition::new(3, 15));
    selection.end();

    let (start, end) = selection.range().unwrap();
    assert_eq!(start.line, 3);
    assert_eq!(end.line, 3);
    assert_eq!(start.col, 5);
    assert_eq!(end.col, 15);
}

// ============================================================================
// Text Extraction Tests
// ============================================================================

fn create_test_lines<'a>() -> Vec<Line<'a>> {
    vec![
        Line::from("Hello, world!"),
        Line::from("This is line 2."),
        Line::from("And line 3 here."),
        Line::from("Fourth line."),
        Line::from("Last line!"),
    ]
}

#[test]
fn test_extract_single_line() {
    let mut selection = SelectionState::new();
    let lines = create_test_lines();

    // Select "world" from line 0
    selection.start(ContentPosition::new(0, 7));
    selection.update(ContentPosition::new(0, 12));
    selection.end();

    let extracted = selection.extract_text(&lines);
    assert_eq!(extracted, "world");
}

#[test]
fn test_extract_multi_line() {
    let mut selection = SelectionState::new();
    let lines = create_test_lines();

    // Select from "world!" to "This"
    selection.start(ContentPosition::new(0, 7));
    selection.update(ContentPosition::new(1, 4));
    selection.end();

    let extracted = selection.extract_text(&lines);
    assert_eq!(extracted, "world!\nThis");
}

#[test]
fn test_extract_full_lines() {
    let mut selection = SelectionState::new();
    let lines = create_test_lines();

    // Select entire lines 1-2
    selection.start(ContentPosition::new(1, 0));
    selection.update(ContentPosition::new(2, 16));
    selection.end();

    let extracted = selection.extract_text(&lines);
    assert_eq!(extracted, "This is line 2.\nAnd line 3 here.");
}

#[test]
fn test_extract_partial_line() {
    let mut selection = SelectionState::new();
    let lines = create_test_lines();

    // Select "line 2" from middle of line 1
    selection.start(ContentPosition::new(1, 8));
    selection.update(ContentPosition::new(1, 14));
    selection.end();

    let extracted = selection.extract_text(&lines);
    assert_eq!(extracted, "line 2");
}

#[test]
fn test_extract_empty_selection() {
    let selection = SelectionState::new();
    let lines = create_test_lines();

    // No selection
    let extracted = selection.extract_text(&lines);
    assert_eq!(extracted, "");
}

#[test]
fn test_extract_all_lines() {
    let mut selection = SelectionState::new();
    let lines = create_test_lines();

    // Select all
    selection.select_all(lines.len());

    let extracted = selection.extract_text(&lines);
    let expected = "Hello, world!\nThis is line 2.\nAnd line 3 here.\nFourth line.\nLast line!";
    assert_eq!(extracted, expected);
}

#[test]
fn test_extract_handles_styled_spans() {
    let mut selection = SelectionState::new();

    // Create lines with styled spans
    let lines: Vec<Line> = vec![
        Line::from(vec![
            Span::raw("Plain "),
            Span::styled("styled", ratatui::style::Style::default()),
            Span::raw(" text"),
        ]),
        Line::from("Normal line"),
    ];

    // Select across the styled spans
    selection.start(ContentPosition::new(0, 0));
    selection.update(ContentPosition::new(0, 17));
    selection.end();

    let extracted = selection.extract_text(&lines);
    assert_eq!(extracted, "Plain styled text");
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn test_selection_beyond_line_length() {
    let mut selection = SelectionState::new();
    let lines = create_test_lines();

    // Select beyond line length (should clamp)
    selection.start(ContentPosition::new(0, 0));
    selection.update(ContentPosition::new(0, 100)); // Way beyond "Hello, world!"
    selection.end();

    let extracted = selection.extract_text(&lines);
    assert_eq!(extracted, "Hello, world!");
}

#[test]
fn test_selection_empty_lines() {
    let mut selection = SelectionState::new();
    let lines: Vec<Line> = vec![Line::from("First"), Line::from(""), Line::from("Third")];

    // Select across empty line
    selection.start(ContentPosition::new(0, 0));
    selection.update(ContentPosition::new(2, 5));
    selection.end();

    let extracted = selection.extract_text(&lines);
    assert_eq!(extracted, "First\n\nThird");
}
