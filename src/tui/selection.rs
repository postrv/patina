//! Text selection state management for copy/paste functionality.
//!
//! Provides selection tracking for the rendered timeline content, enabling
//! users to select text with mouse drag or keyboard shortcuts and copy
//! to clipboard.
//!
//! The core state types ([`FocusArea`], [`ContentPosition`], [`SelectionState`])
//! are defined in [`crate::types::ui_state`] to break the app<->tui dependency
//! cycle. They are re-exported here for backward compatibility.
//!
//! The [`SelectionState::extract_text`] method is defined here because it
//! depends on [`ratatui::text::Line`].

use ratatui::text::Line;

// Re-export core types from the shared module
pub use crate::types::ui_state::{ContentPosition, FocusArea, SelectionState};

impl SelectionState {
    /// Extracts selected text from rendered lines.
    ///
    /// Returns the text content within the selection range, joining multiple
    /// lines with newlines.
    #[must_use]
    pub fn extract_text(&self, lines: &[Line<'_>]) -> String {
        let Some((start, end)) = self.range() else {
            return String::new();
        };

        if lines.is_empty() {
            return String::new();
        }

        let mut result = String::new();

        for (line_idx, line) in lines.iter().enumerate() {
            if line_idx < start.line {
                continue;
            }
            if line_idx > end.line {
                break;
            }

            // Get the plain text content of this line
            let line_text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();

            let (col_start, col_end) = if line_idx == start.line && line_idx == end.line {
                // Single line selection
                (start.col, end.col.min(line_text.len()))
            } else if line_idx == start.line {
                // First line of multi-line selection
                (start.col, line_text.len())
            } else if line_idx == end.line {
                // Last line of multi-line selection
                (0, end.col.min(line_text.len()))
            } else {
                // Middle line - select entire line
                (0, line_text.len())
            };

            // Clamp to valid range
            let col_start = col_start.min(line_text.len());
            let col_end = col_end.min(line_text.len());

            if col_start <= col_end {
                // Add newline between lines (not before first)
                if !result.is_empty() {
                    result.push('\n');
                }
                result.push_str(&line_text[col_start..col_end]);
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;
    use ratatui::text::Span;

    fn make_line(text: &str) -> Line<'static> {
        Line::from(text.to_string())
    }

    fn make_lines(texts: &[&str]) -> Vec<Line<'static>> {
        texts.iter().map(|t| make_line(t)).collect()
    }

    #[test]
    fn test_content_position_ordering() {
        let p1 = ContentPosition::new(0, 5);
        let p2 = ContentPosition::new(0, 10);
        let p3 = ContentPosition::new(1, 0);

        assert!(p1 < p2);
        assert!(p2 < p3);
        assert!(p1 < p3);
    }

    #[test]
    fn test_selection_lifecycle() {
        let mut sel = SelectionState::new();

        assert!(!sel.has_selection());
        assert!(!sel.is_selecting());

        sel.start(ContentPosition::new(0, 0));
        assert!(sel.is_selecting());
        assert!(!sel.has_selection());

        sel.update(ContentPosition::new(1, 10));
        assert!(sel.is_selecting());

        sel.end();
        assert!(!sel.is_selecting());
        assert!(sel.has_selection());
    }

    // =========================================================================
    // select_all tests
    // =========================================================================

    #[test]
    fn test_select_all_empty() {
        let mut sel = SelectionState::new();
        sel.select_all(0);
        assert!(!sel.has_selection());
        assert!(sel.range().is_none());
    }

    #[test]
    fn test_select_all_single_line() {
        let mut sel = SelectionState::new();
        sel.select_all(1);

        assert!(sel.has_selection());
        let range = sel.range().expect("should have range");
        assert_eq!(range.0.line, 0);
        assert_eq!(range.1.line, 0);
    }

    #[test]
    fn test_select_all_multiple_lines() {
        let mut sel = SelectionState::new();
        sel.select_all(100);

        assert!(sel.has_selection());
        let range = sel.range().expect("should have range");
        assert_eq!(range.0.line, 0);
        assert_eq!(range.0.col, 0);
        assert_eq!(range.1.line, 99);
        assert_eq!(range.1.col, usize::MAX);
    }

    #[test]
    fn test_select_all_sets_selecting_false() {
        let mut sel = SelectionState::new();
        sel.select_all(10);
        assert!(!sel.is_selecting());
    }

    // =========================================================================
    // range tests
    // =========================================================================

    #[test]
    fn test_range_empty() {
        let sel = SelectionState::new();
        assert!(sel.range().is_none());
    }

    #[test]
    fn test_range_normalized_forward() {
        let mut sel = SelectionState::new();
        sel.start(ContentPosition::new(0, 0));
        sel.update(ContentPosition::new(5, 10));
        sel.end();

        let range = sel.range().expect("should have range");
        assert_eq!(range.0, ContentPosition::new(0, 0));
        assert_eq!(range.1, ContentPosition::new(5, 10));
    }

    #[test]
    fn test_range_normalized_backward() {
        let mut sel = SelectionState::new();
        sel.start(ContentPosition::new(5, 10));
        sel.update(ContentPosition::new(0, 0));
        sel.end();

        let range = sel.range().expect("should have range");
        // Should be normalized: start <= end
        assert_eq!(range.0, ContentPosition::new(0, 0));
        assert_eq!(range.1, ContentPosition::new(5, 10));
    }

    #[test]
    fn test_range_none_while_selecting() {
        let mut sel = SelectionState::new();
        sel.start(ContentPosition::new(0, 0));
        sel.update(ContentPosition::new(5, 10));
        // Don't call end() - still selecting

        assert!(sel.is_selecting());
        assert!(sel.range().is_none());
    }

    // =========================================================================
    // clear tests
    // =========================================================================

    #[test]
    fn test_clear() {
        let mut sel = SelectionState::new();
        sel.select_all(10);
        assert!(sel.has_selection());

        sel.clear();
        assert!(!sel.has_selection());
        assert!(sel.range().is_none());
    }

    // =========================================================================
    // extract_text tests
    // =========================================================================

    #[test]
    fn test_extract_text_empty_lines() {
        let sel = SelectionState::new();
        let lines: Vec<Line> = vec![];
        assert_eq!(sel.extract_text(&lines), "");
    }

    #[test]
    fn test_extract_text_no_selection() {
        let sel = SelectionState::new();
        let lines = make_lines(&["Hello", "World"]);
        assert_eq!(sel.extract_text(&lines), "");
    }

    #[test]
    fn test_extract_text_single_line_full() {
        let mut sel = SelectionState::new();
        sel.select_all(1);
        let lines = make_lines(&["Hello, World!"]);

        let text = sel.extract_text(&lines);
        assert_eq!(text, "Hello, World!");
    }

    #[test]
    fn test_extract_text_multi_line_full() {
        let mut sel = SelectionState::new();
        sel.select_all(3);
        let lines = make_lines(&["Line 1", "Line 2", "Line 3"]);

        let text = sel.extract_text(&lines);
        assert_eq!(text, "Line 1\nLine 2\nLine 3");
    }

    #[test]
    fn test_extract_text_partial_single_line() {
        let mut sel = SelectionState::new();
        sel.start(ContentPosition::new(0, 0));
        sel.update(ContentPosition::new(0, 5));
        sel.end();

        let lines = make_lines(&["Hello, World!"]);
        let text = sel.extract_text(&lines);
        assert_eq!(text, "Hello");
    }

    #[test]
    fn test_extract_text_partial_multi_line() {
        let mut sel = SelectionState::new();
        sel.start(ContentPosition::new(0, 6));
        sel.update(ContentPosition::new(2, 4));
        sel.end();

        let lines = make_lines(&["Line 1", "Line 2", "Line 3"]);
        let text = sel.extract_text(&lines);
        // From char 6 of line 0, through line 1, to char 4 of line 2
        // Line 0: "" (chars 6+ of "Line 1" which has only 6 chars - empty after clamping)
        // Line 1: "Line 2" (whole line)
        // Line 2: "Line" (chars 0-4)
        // Since line 0 extraction is empty, result starts with Line 2
        assert_eq!(text, "Line 2\nLine");
    }

    #[test]
    fn test_extract_text_with_styled_spans() {
        let mut sel = SelectionState::new();
        sel.select_all(1);

        let line = Line::from(vec![
            Span::styled("Hello ".to_string(), Style::default()),
            Span::styled("World".to_string(), Style::default()),
        ]);

        let text = sel.extract_text(&[line]);
        assert_eq!(text, "Hello World");
    }

    #[test]
    fn test_extract_text_clamps_to_line_length() {
        let mut sel = SelectionState::new();
        sel.start(ContentPosition::new(0, 0));
        sel.update(ContentPosition::new(0, 1000)); // Way past end of line
        sel.end();

        let lines = make_lines(&["Short"]);
        let text = sel.extract_text(&lines);
        assert_eq!(text, "Short");
    }

    #[test]
    fn test_extract_text_selection_beyond_lines() {
        let mut sel = SelectionState::new();
        sel.select_all(100); // Select 100 lines

        let lines = make_lines(&["Only", "Three", "Lines"]);
        let text = sel.extract_text(&lines);
        // Should only get the 3 lines that exist
        assert_eq!(text, "Only\nThree\nLines");
    }
}
