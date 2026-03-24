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
    completion: Option<crate::app::completion::CompletionState>,
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
    pub fn completion(&self) -> Option<&crate::app::completion::CompletionState> {
        self.completion.as_ref()
    }

    /// Returns a mutable reference to the active completion state.
    #[must_use]
    pub fn completion_mut(&mut self) -> Option<&mut crate::app::completion::CompletionState> {
        self.completion.as_mut()
    }

    /// Returns true if the completion popup is currently active.
    #[must_use]
    pub fn has_completion(&self) -> bool {
        self.completion.is_some()
    }

    /// Sets the completion state.
    pub fn set_completion(&mut self, state: crate::app::completion::CompletionState) {
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
