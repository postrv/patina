//! Tests for text selection functionality.
//!
//! TDD tests for copy/paste support.

use patina::tui::selection::{ContentPosition, FocusArea, SelectionState};
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

// ============================================================================
// Focus Area Tests
// ============================================================================

#[test]
fn test_focus_area_default_is_input() {
    // Default focus should be on input area
    let focus = FocusArea::default();
    assert_eq!(focus, FocusArea::Input);
}

#[test]
fn test_focus_area_equality() {
    assert_eq!(FocusArea::Input, FocusArea::Input);
    assert_eq!(FocusArea::Content, FocusArea::Content);
    assert_ne!(FocusArea::Input, FocusArea::Content);
}

#[test]
fn test_focus_area_debug() {
    // Ensure Debug trait works
    let input_str = format!("{:?}", FocusArea::Input);
    let content_str = format!("{:?}", FocusArea::Content);
    assert!(input_str.contains("Input"));
    assert!(content_str.contains("Content"));
}

#[test]
fn test_focus_area_copy() {
    // FocusArea should be Copy
    let focus = FocusArea::Content;
    let copied = focus;
    assert_eq!(focus, copied);
}

// ============================================================================
// Integration Tests for Select-All and Copy
// ============================================================================

/// Test that select_all covers all lines and extract_text gets everything
#[test]
fn test_select_all_extracts_complete_content() {
    // Create a large set of test lines (simulating a long conversation)
    let lines: Vec<Line> = (0..500)
        .map(|i| Line::from(format!("This is line number {} with some content", i)))
        .collect();

    let mut selection = SelectionState::new();

    // Select all 500 lines
    selection.select_all(lines.len());

    // Verify selection range
    let (start, end) = selection.range().expect("should have selection");
    assert_eq!(start.line, 0, "Selection should start at line 0");
    assert_eq!(end.line, 499, "Selection should end at line 499");

    // Extract text
    let extracted = selection.extract_text(&lines);

    // Verify we got all lines
    let extracted_lines: Vec<&str> = extracted.lines().collect();
    assert_eq!(
        extracted_lines.len(),
        500,
        "Should extract all 500 lines, got {}",
        extracted_lines.len()
    );

    // Verify first and last lines
    assert!(
        extracted_lines[0].contains("line number 0"),
        "First line should be line 0"
    );
    assert!(
        extracted_lines[499].contains("line number 499"),
        "Last line should be line 499"
    );
}

/// Test that wrapped lines are handled correctly in select-all
#[test]
fn test_select_all_with_wrapped_lines() {
    use patina::tui::wrap_lines_to_strings;

    // Create lines that will wrap at width 40
    let lines: Vec<Line> = vec![
        Line::from("Short line"),
        Line::from(
            "This is a very long line that should wrap to multiple visual lines when the width is constrained to 40 characters",
        ),
        Line::from("Another short line"),
        Line::from(
            "Yet another extremely long line that will definitely need to be wrapped across several visual lines to fit within the width constraint",
        ),
        Line::from("Final short line"),
    ];

    // Wrap to width 40 (simulating terminal width)
    let wrapped = wrap_lines_to_strings(&lines, 40);

    // The wrapped lines should have more entries than the original
    assert!(
        wrapped.len() > lines.len(),
        "Wrapped lines ({}) should be more than original ({})",
        wrapped.len(),
        lines.len()
    );

    // Create selection over ALL wrapped lines
    let mut selection = SelectionState::new();
    selection.select_all(wrapped.len());

    // Convert wrapped strings back to Lines for extraction
    let wrapped_lines: Vec<Line> = wrapped.iter().map(|s| Line::from(s.as_str())).collect();

    // Extract all text
    let extracted = selection.extract_text(&wrapped_lines);

    // Verify we got all the wrapped lines
    let extracted_lines: Vec<&str> = extracted.lines().collect();
    assert_eq!(
        extracted_lines.len(),
        wrapped.len(),
        "Should extract all {} wrapped lines",
        wrapped.len()
    );
}
