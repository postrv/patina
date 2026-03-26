use crate::types::CompletionState;

/// Input buffer state extracted from AppState.
///
/// Groups the text buffer, cursor position, and completion popup state
/// for the input line at the bottom of the TUI.
pub struct InputState {
    /// Current input text.
    text: String,
    /// Cursor position as character index (not byte index).
    cursor_pos: usize,
    /// Active slash-command completion popup, if any.
    completion: Option<CompletionState>,
}

impl InputState {
    /// Creates a new empty `InputState`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            text: String::new(),
            cursor_pos: 0,
            completion: None,
        }
    }

    /// Returns the current input text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns a mutable reference to the input text.
    pub fn text_mut(&mut self) -> &mut String {
        &mut self.text
    }

    /// Returns the current cursor position (character index).
    #[must_use]
    pub fn cursor_position(&self) -> usize {
        self.cursor_pos
    }

    /// Sets the cursor position (character index).
    ///
    /// Clamps to the length of the text if `pos` exceeds it.
    pub fn set_cursor_position(&mut self, pos: usize) {
        let max = self.text.chars().count();
        self.cursor_pos = pos.min(max);
    }

    /// Inserts a character at the current cursor position.
    ///
    /// Returns `true` if the completion popup should be activated (/ typed as first char).
    pub fn insert_char(&mut self, c: char) -> bool {
        let byte_pos = self
            .text
            .char_indices()
            .nth(self.cursor_pos)
            .map(|(i, _)| i)
            .unwrap_or(self.text.len());
        self.text.insert(byte_pos, c);
        self.cursor_pos += 1;

        // Update completion filter if active
        if c == '/' && self.text == "/" {
            return true; // Signal caller to activate completion
        } else if self.completion.is_some() && self.text.starts_with('/') {
            self.update_completion_filter();
        } else if self.completion.is_some() {
            self.completion = None;
        }
        false
    }

    /// Deletes the character before the cursor (backspace behavior).
    pub fn delete_char(&mut self) {
        if self.cursor_pos > 0 {
            let byte_pos = self
                .text
                .char_indices()
                .nth(self.cursor_pos - 1)
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.text.remove(byte_pos);
            self.cursor_pos -= 1;
        }

        // Update or dismiss completion
        if self.completion.is_some() {
            if self.text.starts_with('/') {
                self.update_completion_filter();
            } else {
                self.completion = None;
            }
        }
    }

    /// Takes and returns the current input, clearing the buffer and resetting cursor.
    pub fn take(&mut self) -> String {
        self.cursor_pos = 0;
        self.completion = None;
        std::mem::take(&mut self.text)
    }

    /// Moves the cursor left by one character.
    pub fn cursor_left(&mut self) {
        self.cursor_pos = self.cursor_pos.saturating_sub(1);
    }

    /// Moves the cursor right by one character.
    pub fn cursor_right(&mut self) {
        let char_count = self.text.chars().count();
        if self.cursor_pos < char_count {
            self.cursor_pos += 1;
        }
    }

    /// Moves the cursor to the beginning of the input.
    pub fn cursor_home(&mut self) {
        self.cursor_pos = 0;
    }

    /// Moves the cursor to the end of the input.
    pub fn cursor_end(&mut self) {
        self.cursor_pos = self.text.chars().count();
    }

    /// Returns the active completion state, if any.
    #[must_use]
    pub fn completion(&self) -> Option<&CompletionState> {
        self.completion.as_ref()
    }

    /// Returns a mutable reference to the active completion state.
    #[must_use]
    pub fn completion_mut(&mut self) -> Option<&mut CompletionState> {
        self.completion.as_mut()
    }

    /// Returns true if the completion popup is currently active.
    #[must_use]
    pub fn has_completion(&self) -> bool {
        self.completion.is_some()
    }

    /// Sets the completion state.
    pub fn set_completion(&mut self, state: CompletionState) {
        self.completion = Some(state);
    }

    /// Dismisses the completion popup.
    pub fn dismiss_completion(&mut self) {
        self.completion = None;
    }

    /// Accepts the selected completion and replaces input with `/name `.
    ///
    /// Returns the accepted command name, or `None` if nothing was selected.
    pub fn accept_completion(&mut self) -> Option<String> {
        let name = self.completion.as_ref().and_then(|c| c.accept());
        if let Some(ref name) = name {
            self.text = format!("/{name} ");
            self.cursor_pos = self.text.chars().count();
        }
        self.completion = None;
        name
    }

    /// Updates the completion filter from the current input.
    fn update_completion_filter(&mut self) {
        if let Some(ref mut completion) = self.completion {
            let filter = if self.text.starts_with('/') {
                &self.text[1..]
            } else {
                ""
            };
            completion.set_filter(filter);
        }
    }

    /// Returns whether the input is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Sets the input text and moves cursor to the end.
    pub fn set_text(&mut self, text: String) {
        self.cursor_pos = text.chars().count();
        self.text = text;
    }
}

impl Default for InputState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ui_state::{CompletionEntry, CompletionSource, CompletionState};

    /// Helper: builds a `CompletionState` with two sample entries.
    fn sample_completion() -> CompletionState {
        CompletionState::new(vec![
            CompletionEntry::new("help", "Show help", CompletionSource::Builtin),
            CompletionEntry::new("mcp", "MCP status", CompletionSource::Builtin),
        ])
    }

    #[test]
    fn test_input_state_new_is_empty() {
        let state = InputState::new();
        assert_eq!(state.text, "");
        assert_eq!(state.cursor_pos, 0);
        assert!(state.completion.is_none());
    }

    #[test]
    fn test_default_matches_new() {
        let from_new = InputState::new();
        let from_default = InputState::default();
        assert_eq!(from_new.text, from_default.text);
        assert_eq!(from_new.cursor_pos, from_default.cursor_pos);
        assert!(from_new.completion.is_none());
        assert!(from_default.completion.is_none());
    }

    #[test]
    fn test_set_text_moves_cursor_to_end() {
        let mut state = InputState::new();
        state.set_text("hello".to_string());
        assert_eq!(state.text(), "hello");
        assert_eq!(state.cursor_position(), 5);
    }

    #[test]
    fn test_set_text_with_unicode() {
        let mut state = InputState::new();
        // "café" has 4 chars but 5 bytes (é is 2 bytes in UTF-8)
        state.set_text("café".to_string());
        assert_eq!(state.cursor_position(), 4);
        assert_eq!(state.text(), "café");
    }

    #[test]
    fn test_set_cursor_position_clamps() {
        let mut state = InputState::new();
        state.set_text("abc".to_string());
        state.set_cursor_position(999);
        assert_eq!(state.cursor_position(), 3);
    }

    #[test]
    fn test_set_cursor_position_zero() {
        let mut state = InputState::new();
        state.set_text("abc".to_string());
        assert_eq!(state.cursor_position(), 3);
        state.set_cursor_position(0);
        assert_eq!(state.cursor_position(), 0);
    }

    #[test]
    fn test_insert_char_mid_string_utf8() {
        let mut state = InputState::new();
        // Start with "aé" (2 chars), place cursor at position 1 (between 'a' and 'é')
        state.set_text("aé".to_string());
        state.set_cursor_position(1);
        state.insert_char('X');
        assert_eq!(state.text(), "aXé");
        assert_eq!(state.cursor_position(), 2);
    }

    #[test]
    fn test_insert_char_dismisses_completion_no_slash() {
        let mut state = InputState::new();
        state.set_text("hello".to_string());
        state.completion = Some(sample_completion());
        assert!(state.has_completion());

        // Typing 'x' when text doesn't start with '/' should dismiss completion
        state.insert_char('x');
        assert!(!state.has_completion());
        assert_eq!(state.text(), "hellox");
    }

    #[test]
    fn test_delete_char_at_zero_is_noop() {
        let mut state = InputState::new();
        assert_eq!(state.cursor_position(), 0);
        assert_eq!(state.text(), "");
        state.delete_char();
        assert_eq!(state.cursor_position(), 0);
        assert_eq!(state.text(), "");
    }

    #[test]
    fn test_take_clears_everything() {
        let mut state = InputState::new();
        state.set_text("some input".to_string());
        state.completion = Some(sample_completion());

        let taken = state.take();
        assert_eq!(taken, "some input");
        assert_eq!(state.text(), "");
        assert_eq!(state.cursor_position(), 0);
        assert!(state.completion.is_none());
    }

    #[test]
    fn test_accept_completion_with_selection() {
        let mut state = InputState::new();
        state.set_text("/he".to_string());
        state.completion = Some(sample_completion());

        let accepted = state.accept_completion();
        // The first entry in filtered list with default selection (0) is "help"
        assert_eq!(accepted, Some("help".to_string()));
        assert_eq!(state.text(), "/help ");
        assert_eq!(state.cursor_position(), "/help ".chars().count());
        assert!(!state.has_completion());
    }

    #[test]
    fn test_accept_completion_empty_returns_none() {
        let mut state = InputState::new();
        // No completion set at all
        let accepted = state.accept_completion();
        assert!(accepted.is_none());
    }

    #[test]
    fn test_is_empty_true_for_new() {
        let state = InputState::new();
        assert!(state.is_empty());
    }

    #[test]
    fn test_is_empty_false_after_insert() {
        let mut state = InputState::new();
        state.insert_char('a');
        assert!(!state.is_empty());
    }
}
