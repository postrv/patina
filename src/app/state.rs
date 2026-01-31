//! Application state management

use crate::api::{AnthropicClient, StreamEvent};
use crate::types::{Message, Role};
use anyhow::Result;
use std::path::PathBuf;
use tokio::sync::mpsc;

pub struct AppState {
    pub messages: Vec<Message>,
    pub input: String,
    pub scroll_offset: usize,
    pub working_dir: PathBuf,
    pub current_response: Option<String>,

    cursor_pos: usize,
    loading: bool,
    throbber_frame: usize,
    streaming_rx: Option<mpsc::Receiver<StreamEvent>>,

    dirty: DirtyFlags,

    // Worktree status bar state
    worktree_branch: Option<String>,
    worktree_modified: usize,
    worktree_ahead: usize,
    worktree_behind: usize,
}

#[derive(Default)]
struct DirtyFlags {
    messages: bool,
    input: bool,
    full: bool,
}

impl DirtyFlags {
    fn any(&self) -> bool {
        self.messages || self.input || self.full
    }

    fn clear(&mut self) {
        *self = Self::default();
    }
}

impl AppState {
    pub fn new(working_dir: PathBuf) -> Self {
        Self {
            messages: Vec::new(),
            input: String::new(),
            scroll_offset: 0,
            working_dir,
            cursor_pos: 0,
            loading: false,
            throbber_frame: 0,
            streaming_rx: None,
            current_response: None,
            dirty: DirtyFlags {
                full: true,
                ..Default::default()
            },
            worktree_branch: None,
            worktree_modified: 0,
            worktree_ahead: 0,
            worktree_behind: 0,
        }
    }

    /// Inserts a character at the current cursor position.
    pub fn insert_char(&mut self, c: char) {
        // Get byte position from char position
        let byte_pos = self
            .input
            .char_indices()
            .nth(self.cursor_pos)
            .map(|(i, _)| i)
            .unwrap_or(self.input.len());
        self.input.insert(byte_pos, c);
        self.cursor_pos += 1;
        self.dirty.input = true;
    }

    /// Deletes the character before the cursor (backspace behavior).
    pub fn delete_char(&mut self) {
        if self.cursor_pos > 0 {
            // Get byte position of the character to delete (one before cursor)
            let byte_pos = self
                .input
                .char_indices()
                .nth(self.cursor_pos - 1)
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.input.remove(byte_pos);
            self.cursor_pos -= 1;
        }
        self.dirty.input = true;
    }

    /// Takes and returns the current input, clearing the buffer and resetting cursor.
    pub fn take_input(&mut self) -> String {
        self.dirty.input = true;
        self.cursor_pos = 0;
        std::mem::take(&mut self.input)
    }

    /// Returns the current cursor position (character index, not byte index).
    #[must_use]
    pub fn cursor_position(&self) -> usize {
        self.cursor_pos
    }

    /// Moves the cursor left by one character.
    pub fn cursor_left(&mut self) {
        self.cursor_pos = self.cursor_pos.saturating_sub(1);
        self.dirty.input = true;
    }

    /// Moves the cursor right by one character.
    pub fn cursor_right(&mut self) {
        let char_count = self.input.chars().count();
        if self.cursor_pos < char_count {
            self.cursor_pos += 1;
        }
        self.dirty.input = true;
    }

    /// Moves the cursor to the beginning of the input.
    pub fn cursor_home(&mut self) {
        self.cursor_pos = 0;
        self.dirty.input = true;
    }

    /// Moves the cursor to the end of the input.
    pub fn cursor_end(&mut self) {
        self.cursor_pos = self.input.chars().count();
        self.dirty.input = true;
    }

    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(lines);
        self.dirty.messages = true;
    }

    pub fn scroll_down(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
        self.dirty.messages = true;
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn tick_throbber(&mut self) {
        self.throbber_frame = (self.throbber_frame + 1) % 4;
        self.dirty.messages = true;
    }

    pub fn throbber_char(&self) -> char {
        ['⠋', '⠙', '⠹', '⠸'][self.throbber_frame]
    }

    pub fn needs_render(&self) -> bool {
        self.dirty.any()
    }

    pub fn mark_rendered(&mut self) {
        self.dirty.clear();
    }

    pub fn mark_full_redraw(&mut self) {
        self.dirty.full = true;
    }

    /// Adds a message to the conversation history.
    ///
    /// This sets the dirty flag so the UI will re-render.
    pub fn add_message(&mut self, message: Message) {
        self.messages.push(message);
        self.dirty.messages = true;
    }

    pub async fn submit_message(
        &mut self,
        client: &AnthropicClient,
        content: String,
    ) -> Result<()> {
        self.add_message(Message {
            role: Role::User,
            content,
        });

        self.loading = true;
        self.current_response = Some(String::new());

        let (tx, rx) = mpsc::channel(100);
        self.streaming_rx = Some(rx);

        let messages = self.messages.clone();
        let client = client.clone();
        tokio::spawn(async move {
            if let Err(e) = client.stream_message(&messages, tx).await {
                tracing::error!("API error: {}", e);
            }
        });

        Ok(())
    }

    pub async fn recv_api_chunk(&mut self) -> Option<StreamEvent> {
        if let Some(rx) = &mut self.streaming_rx {
            rx.recv().await
        } else {
            std::future::pending::<Option<StreamEvent>>().await
        }
    }

    pub fn append_chunk(&mut self, event: StreamEvent) -> Result<()> {
        match event {
            StreamEvent::ContentDelta(text) => {
                if let Some(ref mut response) = self.current_response {
                    response.push_str(&text);
                    self.dirty.messages = true;
                }
            }
            StreamEvent::MessageStop => {
                if let Some(response) = self.current_response.take() {
                    self.messages.push(Message {
                        role: Role::Assistant,
                        content: response,
                    });
                }
                self.loading = false;
                self.streaming_rx = None;
                self.dirty.messages = true;
            }
            StreamEvent::Error(e) => {
                tracing::error!("Stream error: {}", e);
                self.loading = false;
                self.streaming_rx = None;
                self.dirty.messages = true;
            }
        }
        Ok(())
    }

    // ========================================================================
    // Worktree Status Bar State
    // ========================================================================

    /// Sets the current worktree branch name.
    ///
    /// This is displayed in the status bar.
    pub fn set_worktree_branch(&mut self, branch: String) {
        self.worktree_branch = Some(branch);
        self.dirty.full = true;
    }

    /// Returns the current worktree branch name, if set.
    #[must_use]
    pub fn worktree_branch(&self) -> Option<&str> {
        self.worktree_branch.as_deref()
    }

    /// Sets the number of modified files in the worktree.
    pub fn set_worktree_modified(&mut self, count: usize) {
        self.worktree_modified = count;
        self.dirty.full = true;
    }

    /// Returns the number of modified files in the worktree.
    #[must_use]
    pub fn worktree_modified(&self) -> usize {
        self.worktree_modified
    }

    /// Sets the number of commits ahead of upstream.
    pub fn set_worktree_ahead(&mut self, count: usize) {
        self.worktree_ahead = count;
        self.dirty.full = true;
    }

    /// Returns the number of commits ahead of upstream.
    #[must_use]
    pub fn worktree_ahead(&self) -> usize {
        self.worktree_ahead
    }

    /// Sets the number of commits behind upstream.
    pub fn set_worktree_behind(&mut self, count: usize) {
        self.worktree_behind = count;
        self.dirty.full = true;
    }

    /// Returns the number of commits behind upstream.
    #[must_use]
    pub fn worktree_behind(&self) -> usize {
        self.worktree_behind
    }
}
