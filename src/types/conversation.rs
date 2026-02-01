//! Unified timeline type system for conversation display.
//!
//! This module provides a single source of truth for conversation display ordering,
//! replacing the previous dual-system of `messages: Vec<Message>` plus
//! `current_response: Option<String>`.
//!
//! # Architecture
//!
//! The [`Timeline`] struct maintains an ordered sequence of [`ConversationEntry`] items
//! representing everything that can appear in the conversation view:
//! - User messages
//! - Complete assistant messages
//! - Currently streaming assistant response
//! - Tool execution blocks with results
//!
//! # Example
//!
//! ```
//! use patina::types::{ConversationEntry, Timeline};
//!
//! let mut timeline = Timeline::new();
//!
//! // User sends a message
//! timeline.push_user_message("Hello!");
//!
//! // Assistant starts streaming
//! timeline.push_streaming();
//! timeline.append_to_streaming("Hi there!");
//! timeline.finalize_streaming_as_message();
//!
//! assert_eq!(timeline.len(), 2);
//! ```

use std::fmt;

/// Everything that can appear in the conversation display, in order.
///
/// This enum represents the unified display model for conversation items.
/// Each variant corresponds to a distinct visual element in the TUI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationEntry {
    /// A complete user message.
    UserMessage(String),

    /// A complete assistant message.
    AssistantMessage(String),

    /// Currently streaming assistant response.
    ///
    /// Only one streaming entry should exist at a time (enforced by [`Timeline`]).
    Streaming {
        /// The accumulated text content.
        text: String,
        /// Whether the stream has completed (used during finalization).
        complete: bool,
    },

    /// A tool execution block with optional result.
    ToolExecution {
        /// Name of the tool (e.g., "bash", "read_file").
        name: String,
        /// Input/command for the tool (e.g., "ls -la", file path).
        input: String,
        /// Output from tool execution, if complete.
        output: Option<String>,
        /// Whether the tool execution resulted in an error.
        is_error: bool,
        /// Index of the assistant message this tool block follows.
        /// Used for rendering tool blocks inline with their producing message.
        follows_message_idx: Option<usize>,
    },

    /// An image for display in the conversation.
    ///
    /// Contains the decoded pixel data ready for TUI rendering.
    ImageDisplay {
        /// Width of the image in pixels.
        width: u32,
        /// Height of the image in pixels.
        height: u32,
        /// RGBA pixel data (4 bytes per pixel).
        pixels: Vec<u8>,
        /// Optional alt text or description.
        alt_text: Option<String>,
    },
}

impl ConversationEntry {
    /// Returns `true` if this is a user message.
    #[must_use]
    pub fn is_user(&self) -> bool {
        matches!(self, Self::UserMessage(_))
    }

    /// Returns `true` if this is a complete assistant message.
    #[must_use]
    pub fn is_assistant(&self) -> bool {
        matches!(self, Self::AssistantMessage(_))
    }

    /// Returns `true` if this is a streaming entry.
    #[must_use]
    pub fn is_streaming(&self) -> bool {
        matches!(self, Self::Streaming { .. })
    }

    /// Returns `true` if this is a tool execution entry.
    #[must_use]
    pub fn is_tool_execution(&self) -> bool {
        matches!(self, Self::ToolExecution { .. })
    }

    /// Returns `true` if this is an image display entry.
    #[must_use]
    pub fn is_image_display(&self) -> bool {
        matches!(self, Self::ImageDisplay { .. })
    }

    /// Returns the text content if this entry has displayable text.
    ///
    /// Returns `Some(&str)` for user messages, assistant messages, and streaming entries.
    /// Returns `None` for tool execution and image display entries (use structured accessors instead).
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::UserMessage(text) | Self::AssistantMessage(text) => Some(text),
            Self::Streaming { text, .. } => Some(text),
            Self::ToolExecution { .. } | Self::ImageDisplay { .. } => None,
        }
    }

    /// Returns the image display data if this is an image entry.
    ///
    /// Returns `Some((width, height, &pixels, alt_text))` for image display entries.
    /// Returns `None` for other entry types.
    #[must_use]
    pub fn as_image_display(&self) -> Option<(u32, u32, &[u8], Option<&str>)> {
        match self {
            Self::ImageDisplay {
                width,
                height,
                pixels,
                alt_text,
            } => Some((*width, *height, pixels.as_slice(), alt_text.as_deref())),
            _ => None,
        }
    }
}

impl fmt::Display for ConversationEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UserMessage(text) => write!(f, "User: {text}"),
            Self::AssistantMessage(text) => write!(f, "Assistant: {text}"),
            Self::Streaming { text, complete } => {
                let status = if *complete { "complete" } else { "streaming" };
                write!(f, "Assistant ({status}): {text}")
            }
            Self::ToolExecution {
                name,
                input,
                output,
                is_error,
                ..
            } => {
                let status = if *is_error { "error" } else { "success" };
                let out = output.as_deref().unwrap_or("(pending)");
                write!(f, "Tool[{name}] ({status}): {input} -> {out}")
            }
            Self::ImageDisplay {
                width,
                height,
                alt_text,
                ..
            } => {
                let alt = alt_text.as_deref().unwrap_or("image");
                write!(f, "Image[{width}x{height}]: {alt}")
            }
        }
    }
}

/// Error type for timeline operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimelineError {
    /// Attempted to start streaming while already streaming.
    AlreadyStreaming,
    /// Attempted to modify streaming when not streaming.
    NotStreaming,
}

impl fmt::Display for TimelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyStreaming => write!(f, "Timeline is already streaming"),
            Self::NotStreaming => write!(f, "Timeline is not currently streaming"),
        }
    }
}

impl std::error::Error for TimelineError {}

/// Manages the ordered sequence of conversation entries.
///
/// The timeline is the single source of truth for conversation display ordering.
/// It enforces invariants like "only one streaming entry at a time" and provides
/// methods for state transitions.
///
/// # Streaming Lifecycle
///
/// 1. `push_streaming()` - Start a new streaming entry
/// 2. `append_to_streaming(text)` - Add content as it arrives
/// 3. `finalize_streaming_as_message()` or `finalize_streaming_for_tool_use()` - Complete
#[derive(Debug, Clone, Default)]
pub struct Timeline {
    /// The ordered list of conversation entries.
    entries: Vec<ConversationEntry>,
    /// Index of the current streaming entry, if any.
    streaming_idx: Option<usize>,
}

impl Timeline {
    /// Creates a new empty timeline.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the number of entries in the timeline.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if the timeline has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns `true` if the timeline is currently streaming.
    #[must_use]
    pub fn is_streaming(&self) -> bool {
        self.streaming_idx.is_some()
    }

    /// Returns an iterator over the entries.
    pub fn iter(&self) -> impl Iterator<Item = &ConversationEntry> {
        self.entries.iter()
    }

    /// Returns the entries as a slice.
    #[must_use]
    pub fn entries(&self) -> &[ConversationEntry] {
        &self.entries
    }

    /// Pushes a user message to the timeline.
    pub fn push_user_message(&mut self, content: impl Into<String>) {
        self.entries
            .push(ConversationEntry::UserMessage(content.into()));
    }

    /// Pushes a complete assistant message to the timeline.
    pub fn push_assistant_message(&mut self, content: impl Into<String>) {
        self.entries
            .push(ConversationEntry::AssistantMessage(content.into()));
    }

    /// Starts a new streaming entry.
    ///
    /// # Panics
    ///
    /// Panics if already streaming. Use [`try_push_streaming`](Self::try_push_streaming)
    /// for fallible version.
    pub fn push_streaming(&mut self) {
        self.try_push_streaming()
            .expect("Cannot start streaming: already streaming");
    }

    /// Attempts to start a new streaming entry.
    ///
    /// # Errors
    ///
    /// Returns [`TimelineError::AlreadyStreaming`] if already streaming.
    pub fn try_push_streaming(&mut self) -> Result<(), TimelineError> {
        if self.streaming_idx.is_some() {
            return Err(TimelineError::AlreadyStreaming);
        }

        let idx = self.entries.len();
        self.entries.push(ConversationEntry::Streaming {
            text: String::new(),
            complete: false,
        });
        self.streaming_idx = Some(idx);
        Ok(())
    }

    /// Appends text to the current streaming entry.
    ///
    /// If not currently streaming, this is a no-op.
    pub fn append_to_streaming(&mut self, text: &str) {
        if let Some(idx) = self.streaming_idx {
            if let ConversationEntry::Streaming {
                text: ref mut t, ..
            } = &mut self.entries[idx]
            {
                t.push_str(text);
            }
        }
    }

    /// Returns mutable access to the streaming text, if streaming.
    #[must_use]
    pub fn streaming_text_mut(&mut self) -> Option<&mut String> {
        self.streaming_idx.and_then(|idx| {
            if let ConversationEntry::Streaming { text, .. } = &mut self.entries[idx] {
                Some(text)
            } else {
                None
            }
        })
    }

    /// Finalizes the streaming entry as a complete assistant message.
    ///
    /// Converts the streaming entry in-place to an `AssistantMessage`.
    pub fn finalize_streaming_as_message(&mut self) {
        if let Some(idx) = self.streaming_idx.take() {
            if let ConversationEntry::Streaming { text, .. } = &self.entries[idx] {
                let content = text.clone();
                self.entries[idx] = ConversationEntry::AssistantMessage(content);
            }
        }
    }

    /// Finalizes the streaming entry for tool use, returning the accumulated text.
    ///
    /// This should be called when `MessageComplete` is received with `stop_reason=tool_use`.
    /// The returned text should be stored for later use by `handle_tool_execution`.
    ///
    /// # Returns
    ///
    /// The accumulated streaming text. Returns empty string if not streaming.
    pub fn finalize_streaming_for_tool_use(&mut self) -> String {
        if let Some(idx) = self.streaming_idx.take() {
            if let ConversationEntry::Streaming { text, .. } = &self.entries[idx] {
                let content = text.clone();
                self.entries[idx] = ConversationEntry::AssistantMessage(content.clone());
                return content;
            }
        }
        String::new()
    }

    /// Pushes a tool execution entry to the timeline.
    ///
    /// # Arguments
    ///
    /// * `name` - Tool name (e.g., "bash", "read_file")
    /// * `input` - Tool input/command
    /// * `output` - Tool output, if complete
    /// * `is_error` - Whether the execution resulted in an error
    pub fn push_tool_execution(
        &mut self,
        name: impl Into<String>,
        input: impl Into<String>,
        output: Option<String>,
        is_error: bool,
    ) {
        self.entries.push(ConversationEntry::ToolExecution {
            name: name.into(),
            input: input.into(),
            output,
            is_error,
            follows_message_idx: None,
        });
    }

    /// Pushes a tool execution that follows the most recent assistant message.
    ///
    /// This sets `follows_message_idx` to track which assistant message the tool
    /// block should be rendered after.
    pub fn push_tool_after_current_assistant(
        &mut self,
        name: impl Into<String>,
        input: impl Into<String>,
        output: Option<String>,
        is_error: bool,
    ) {
        // Find the index of the most recent assistant message
        let follows_idx = self
            .entries
            .iter()
            .enumerate()
            .rev()
            .find(|(_, e)| e.is_assistant())
            .map(|(i, _)| i);

        self.entries.push(ConversationEntry::ToolExecution {
            name: name.into(),
            input: input.into(),
            output,
            is_error,
            follows_message_idx: follows_idx,
        });
    }

    /// Returns mutable access to the entries.
    ///
    /// Use with care - this allows direct modification of the timeline.
    pub fn entries_mut(&mut self) -> &mut Vec<ConversationEntry> {
        &mut self.entries
    }

    /// Pushes an image display entry to the timeline.
    ///
    /// # Arguments
    ///
    /// * `width` - Width of the image in pixels
    /// * `height` - Height of the image in pixels
    /// * `pixels` - RGBA pixel data (4 bytes per pixel)
    /// * `alt_text` - Optional alt text or description
    pub fn push_image(
        &mut self,
        width: u32,
        height: u32,
        pixels: Vec<u8>,
        alt_text: Option<String>,
    ) {
        self.entries.push(ConversationEntry::ImageDisplay {
            width,
            height,
            pixels,
            alt_text,
        });
    }

    /// Updates the most recent tool execution with the given name.
    ///
    /// Finds the most recent tool entry with matching name and no output yet,
    /// and updates it with the provided output and error status.
    ///
    /// # Arguments
    ///
    /// * `tool_name` - Name of the tool to update
    /// * `output` - The tool output
    /// * `is_error` - Whether the result is an error
    pub fn update_tool_result(&mut self, tool_name: &str, output: Option<String>, is_error: bool) {
        // Find the most recent matching tool with no output and update it
        for entry in self.entries.iter_mut().rev() {
            if let ConversationEntry::ToolExecution {
                name,
                output: ref mut o @ None,
                is_error: ref mut err,
                ..
            } = entry
            {
                if name == tool_name {
                    *o = output;
                    *err = is_error;
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conversation_entry_display() {
        let user = ConversationEntry::UserMessage("Hello".to_string());
        assert_eq!(format!("{user}"), "User: Hello");

        let assistant = ConversationEntry::AssistantMessage("Hi!".to_string());
        assert_eq!(format!("{assistant}"), "Assistant: Hi!");

        let streaming = ConversationEntry::Streaming {
            text: "Typing...".to_string(),
            complete: false,
        };
        assert_eq!(format!("{streaming}"), "Assistant (streaming): Typing...");

        let tool = ConversationEntry::ToolExecution {
            name: "bash".to_string(),
            input: "ls".to_string(),
            output: Some("files".to_string()),
            is_error: false,
            follows_message_idx: None,
        };
        assert_eq!(format!("{tool}"), "Tool[bash] (success): ls -> files");
    }

    #[test]
    fn test_timeline_error_display() {
        let err = TimelineError::AlreadyStreaming;
        assert_eq!(format!("{err}"), "Timeline is already streaming");

        let err = TimelineError::NotStreaming;
        assert_eq!(format!("{err}"), "Timeline is not currently streaming");
    }

    #[test]
    fn test_image_display_entry() {
        let pixels = vec![255, 0, 0, 255]; // 1 red pixel
        let entry = ConversationEntry::ImageDisplay {
            width: 1,
            height: 1,
            pixels: pixels.clone(),
            alt_text: Some("red pixel".to_string()),
        };

        assert!(entry.is_image_display());
        assert!(!entry.is_user());
        assert!(!entry.is_assistant());
        assert!(!entry.is_streaming());
        assert!(!entry.is_tool_execution());
        assert!(entry.text().is_none());

        let (w, h, px, alt) = entry.as_image_display().unwrap();
        assert_eq!(w, 1);
        assert_eq!(h, 1);
        assert_eq!(px, &pixels);
        assert_eq!(alt, Some("red pixel"));
    }

    #[test]
    fn test_image_display_entry_no_alt_text() {
        let entry = ConversationEntry::ImageDisplay {
            width: 10,
            height: 5,
            pixels: vec![0; 200], // 10x5 image, 4 bytes per pixel
            alt_text: None,
        };

        let (w, h, px, alt) = entry.as_image_display().unwrap();
        assert_eq!(w, 10);
        assert_eq!(h, 5);
        assert_eq!(px.len(), 200);
        assert!(alt.is_none());
    }

    #[test]
    fn test_image_display_display_trait() {
        let entry = ConversationEntry::ImageDisplay {
            width: 800,
            height: 600,
            pixels: vec![],
            alt_text: Some("screenshot".to_string()),
        };
        assert_eq!(format!("{entry}"), "Image[800x600]: screenshot");

        let entry_no_alt = ConversationEntry::ImageDisplay {
            width: 100,
            height: 50,
            pixels: vec![],
            alt_text: None,
        };
        assert_eq!(format!("{entry_no_alt}"), "Image[100x50]: image");
    }

    #[test]
    fn test_timeline_push_image() {
        let mut timeline = Timeline::new();
        let pixels = vec![255, 255, 255, 255]; // 1 white pixel

        timeline.push_image(1, 1, pixels.clone(), Some("white pixel".to_string()));

        assert_eq!(timeline.len(), 1);
        let entry = &timeline.entries()[0];
        assert!(entry.is_image_display());

        let (w, h, px, alt) = entry.as_image_display().unwrap();
        assert_eq!(w, 1);
        assert_eq!(h, 1);
        assert_eq!(px, &pixels);
        assert_eq!(alt, Some("white pixel"));
    }

    #[test]
    fn test_timeline_push_image_no_alt() {
        let mut timeline = Timeline::new();

        timeline.push_image(50, 50, vec![0; 10_000], None);

        assert_eq!(timeline.len(), 1);
        let (_, _, _, alt) = timeline.entries()[0].as_image_display().unwrap();
        assert!(alt.is_none());
    }

    #[test]
    fn test_other_entries_as_image_display_returns_none() {
        let user = ConversationEntry::UserMessage("Hello".to_string());
        assert!(user.as_image_display().is_none());

        let assistant = ConversationEntry::AssistantMessage("Hi".to_string());
        assert!(assistant.as_image_display().is_none());

        let streaming = ConversationEntry::Streaming {
            text: "...".to_string(),
            complete: false,
        };
        assert!(streaming.as_image_display().is_none());

        let tool = ConversationEntry::ToolExecution {
            name: "test".to_string(),
            input: "test".to_string(),
            output: None,
            is_error: false,
            follows_message_idx: None,
        };
        assert!(tool.as_image_display().is_none());
    }
}
