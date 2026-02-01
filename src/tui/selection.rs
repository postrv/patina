//! Text selection state management for copy/paste functionality.
//!
//! Provides selection tracking for the rendered timeline content, enabling
//! users to select text with mouse drag or keyboard shortcuts and copy
//! to clipboard.

use ratatui::text::Line;

/// Position in rendered timeline content.
///
/// Represents a cursor position in the visible content area, where `line`
/// is the visual line number (after text wrapping) and `col` is the
/// character offset within that line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ContentPosition {
    /// Visual line number (0-indexed, after wrapping)
    pub line: usize,
    /// Character offset within line (0-indexed)
    pub col: usize,
}

impl ContentPosition {
    /// Creates a new content position.
    #[must_use]
    pub const fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }
}

impl PartialOrd for ContentPosition {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ContentPosition {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.line.cmp(&other.line) {
            std::cmp::Ordering::Equal => self.col.cmp(&other.col),
            ordering => ordering,
        }
    }
}

/// Selection state for copy/paste functionality.
///
/// Manages the anchor (start) and cursor (current) positions of a text
/// selection. Supports mouse-based selection (click-drag) and keyboard
/// shortcuts (select all).
#[derive(Debug, Clone, Default)]
pub struct SelectionState {
    /// Anchor point where selection started
    anchor: Option<ContentPosition>,
    /// Current cursor position (end of selection during drag)
    cursor: Option<ContentPosition>,
    /// Whether actively selecting (mouse button held)
    selecting: bool,
}

impl SelectionState {
    /// Creates a new empty selection state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            anchor: None,
            cursor: None,
            selecting: false,
        }
    }

    /// Starts a new selection at the given position.
    ///
    /// Called when the user clicks to begin selecting.
    pub fn start(&mut self, pos: ContentPosition) {
        self.anchor = Some(pos);
        self.cursor = Some(pos);
        self.selecting = true;
    }

    /// Updates the selection end position during drag.
    ///
    /// Called as the user drags to extend the selection.
    pub fn update(&mut self, pos: ContentPosition) {
        if self.selecting {
            self.cursor = Some(pos);
        }
    }

    /// Completes the selection.
    ///
    /// Called when the user releases the mouse button.
    pub fn end(&mut self) {
        self.selecting = false;
    }

    /// Clears the current selection.
    pub fn clear(&mut self) {
        self.anchor = None;
        self.cursor = None;
        self.selecting = false;
    }

    /// Selects all content (Cmd/Ctrl+A).
    ///
    /// Sets selection from (0, 0) to (total_lines - 1, MAX).
    pub fn select_all(&mut self, total_lines: usize) {
        if total_lines == 0 {
            self.clear();
            return;
        }
        self.anchor = Some(ContentPosition::new(0, 0));
        self.cursor = Some(ContentPosition::new(total_lines - 1, usize::MAX));
        self.selecting = false;
    }

    /// Returns whether there is an active selection.
    #[must_use]
    pub fn has_selection(&self) -> bool {
        self.anchor.is_some() && self.cursor.is_some() && !self.selecting
    }

    /// Returns whether currently in the process of selecting.
    #[must_use]
    pub fn is_selecting(&self) -> bool {
        self.selecting
    }

    /// Returns the normalized selection range (start, end) where start <= end.
    ///
    /// Returns `None` if there is no complete selection.
    #[must_use]
    pub fn range(&self) -> Option<(ContentPosition, ContentPosition)> {
        match (self.anchor, self.cursor) {
            (Some(anchor), Some(cursor)) if !self.selecting => {
                let (start, end) = if anchor <= cursor {
                    (anchor, cursor)
                } else {
                    (cursor, anchor)
                };
                Some((start, end))
            }
            _ => None,
        }
    }

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
}
