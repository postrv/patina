//! UI state for session persistence.
//!
//! Captures and restores the terminal UI state across session resume operations.

use serde::{Deserialize, Serialize};

/// UI state for session resume.
///
/// Captures the terminal UI state so it can be restored when resuming a session.
/// This allows users to continue exactly where they left off, including their
/// scroll position, any unsent input, and cursor position.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiState {
    /// Vertical scroll offset in the message view.
    scroll_offset: usize,

    /// Content of the input buffer (unsent text).
    input_buffer: String,

    /// Cursor position within the input buffer.
    cursor_position: usize,
}

impl UiState {
    /// Creates a new UI state with default values.
    ///
    /// Default state has scroll at top, empty input, cursor at position 0.
    #[must_use]
    pub fn new() -> Self {
        Self {
            scroll_offset: 0,
            input_buffer: String::new(),
            cursor_position: 0,
        }
    }

    /// Creates a UI state with specified values.
    ///
    /// # Arguments
    ///
    /// * `scroll_offset` - Vertical scroll position in the message view.
    /// * `input_buffer` - Current text in the input field.
    /// * `cursor_position` - Cursor position within the input buffer.
    #[must_use]
    pub fn with_state(scroll_offset: usize, input_buffer: String, cursor_position: usize) -> Self {
        Self {
            scroll_offset,
            input_buffer,
            cursor_position,
        }
    }

    /// Returns the scroll offset.
    #[must_use]
    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    /// Returns the input buffer contents.
    #[must_use]
    pub fn input_buffer(&self) -> &str {
        &self.input_buffer
    }

    /// Returns the cursor position.
    #[must_use]
    pub fn cursor_position(&self) -> usize {
        self.cursor_position
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ui_state_default() {
        let state = UiState::default();
        assert_eq!(state.scroll_offset(), 0);
        assert_eq!(state.input_buffer(), "");
        assert_eq!(state.cursor_position(), 0);
    }

    #[test]
    fn test_ui_state_new() {
        let state = UiState::new();
        assert_eq!(state.scroll_offset(), 0);
        assert_eq!(state.input_buffer(), "");
        assert_eq!(state.cursor_position(), 0);
    }

    #[test]
    fn test_ui_state_with_state() {
        let state = UiState::with_state(10, "hello world".to_string(), 5);
        assert_eq!(state.scroll_offset(), 10);
        assert_eq!(state.input_buffer(), "hello world");
        assert_eq!(state.cursor_position(), 5);
    }
}
