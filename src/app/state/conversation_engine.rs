//! Conversation state extracted from AppState.
//!
//! Groups the authoritative conversation history (`api_messages`), the display
//! timeline, the streaming channel, and the thinking buffer. Pure conversation
//! operations live here; orchestration methods that touch tool_state, view,
//! or compression remain on `AppState`.

use crate::types::conversation::Timeline;
use crate::types::message::{ApiMessageV2, Message};
use crate::types::stream::StreamEvent;
use tokio::sync::mpsc;

/// Core conversation state extracted from AppState.
///
/// Groups all fields related to the conversation lifecycle:
/// - `api_messages` — authoritative conversation history sent to the API
/// - `timeline` — display-oriented timeline for TUI rendering
/// - `streaming_rx` — channel for receiving stream events from the API
/// - `thinking_buffer` — accumulator for thinking block text
///
/// Self-tracks render dirty state via an internal flag.
pub struct ConversationEngine {
    /// Full API messages with content blocks (tool_use, tool_result).
    /// This is the authoritative conversation history sent to the API.
    pub(crate) api_messages: Vec<ApiMessageV2>,

    /// Unified timeline for conversation display.
    /// This is the single source of truth for display ordering.
    pub(crate) timeline: Timeline,

    /// Channel receiver for streaming API responses.
    pub(crate) streaming_rx: Option<mpsc::Receiver<StreamEvent>>,

    /// Buffer for accumulating thinking text from stream events.
    pub(crate) thinking_buffer: String,

    /// Whether any render-visible mutation has occurred since last `mark_clean()`.
    pub(crate) dirty: bool,
}

impl ConversationEngine {
    /// Creates a new empty conversation engine.
    #[must_use]
    pub fn new() -> Self {
        Self {
            api_messages: Vec::new(),
            timeline: Timeline::new(),
            streaming_rx: None,
            thinking_buffer: String::new(),
            dirty: false,
        }
    }

    // ========================================================================
    // Dirty Flag Tracking
    // ========================================================================

    /// Returns whether any render-visible mutation has occurred since last `mark_clean()`.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Clears the dirty flag after rendering.
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    // ========================================================================
    // API Messages
    // ========================================================================

    /// Returns a reference to the API message history.
    #[must_use]
    pub fn api_messages(&self) -> &[ApiMessageV2] {
        &self.api_messages
    }

    /// Returns a mutable reference to the API message history.
    pub fn api_messages_mut(&mut self) -> &mut Vec<ApiMessageV2> {
        &mut self.api_messages
    }

    /// Returns the number of API messages.
    #[must_use]
    pub fn api_messages_len(&self) -> usize {
        self.api_messages.len()
    }

    // ========================================================================
    // Timeline
    // ========================================================================

    /// Returns a reference to the conversation timeline.
    #[must_use]
    pub fn timeline(&self) -> &Timeline {
        &self.timeline
    }

    /// Returns a mutable reference to the conversation timeline.
    pub fn timeline_mut(&mut self) -> &mut Timeline {
        &mut self.timeline
    }

    /// Converts timeline entries to the legacy `Vec<Message>` format.
    #[must_use]
    pub fn messages(&self) -> Vec<Message> {
        use crate::types::message::Role;
        self.timeline
            .entries()
            .iter()
            .filter_map(|entry| {
                let role = if entry.is_user() {
                    Role::User
                } else if entry.is_assistant() {
                    Role::Assistant
                } else {
                    return None;
                };
                entry.text().map(|text| Message {
                    role,
                    content: text.to_string(),
                })
            })
            .collect()
    }

    // ========================================================================
    // Streaming
    // ========================================================================

    /// Returns true if a streaming channel is currently active.
    #[must_use]
    pub fn has_streaming(&self) -> bool {
        self.streaming_rx.is_some()
    }

    /// Sets the receiver channel for streaming API responses.
    pub fn set_streaming_rx(&mut self, rx: mpsc::Receiver<StreamEvent>) {
        self.streaming_rx = Some(rx);
    }

    /// Receives the next stream event, if a channel is active.
    ///
    /// Returns `None` immediately if no channel is set.
    pub async fn recv_api_chunk(&mut self) -> Option<StreamEvent> {
        match &mut self.streaming_rx {
            Some(rx) => rx.recv().await,
            None => None,
        }
    }

    // ========================================================================
    // Thinking Buffer
    // ========================================================================

    /// Clears the thinking buffer (called on ThinkingStart).
    pub fn clear_thinking_buffer(&mut self) {
        self.thinking_buffer.clear();
    }

    /// Appends text to the thinking buffer (called on ThinkingDelta).
    pub fn append_thinking(&mut self, text: &str) {
        self.thinking_buffer.push_str(text);
    }

    /// Takes the thinking buffer content, replacing it with an empty string.
    ///
    /// Returns `None` if the buffer was empty.
    #[must_use]
    pub fn take_thinking(&mut self) -> Option<String> {
        if self.thinking_buffer.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.thinking_buffer))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_conversation_engine_new_defaults() {
        let engine = ConversationEngine::new();
        assert!(engine.api_messages().is_empty());
        assert_eq!(engine.timeline().len(), 0);
        assert!(!engine.has_streaming());
        assert!(engine.thinking_buffer.is_empty());
        assert!(!engine.is_dirty());
    }

    #[test]
    fn test_api_message_accessors() {
        let mut engine = ConversationEngine::new();
        assert_eq!(engine.api_messages_len(), 0);

        engine
            .api_messages_mut()
            .push(ApiMessageV2::user("Hello"));
        assert_eq!(engine.api_messages_len(), 1);
        assert_eq!(engine.api_messages()[0].content.to_text(), "Hello");
    }

    #[test]
    fn test_timeline_accessors() {
        let mut engine = ConversationEngine::new();
        engine.timeline_mut().push_user_message("Hi");
        assert_eq!(engine.timeline().len(), 1);
    }

    #[test]
    fn test_messages_from_timeline() {
        let mut engine = ConversationEngine::new();
        engine.timeline_mut().push_user_message("Hello");
        engine
            .timeline_mut()
            .push_assistant_message("Hi there".to_string());

        let msgs = engine.messages();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].content, "Hello");
        assert_eq!(msgs[1].content, "Hi there");
    }

    #[test]
    fn test_streaming_lifecycle() {
        let mut engine = ConversationEngine::new();
        assert!(!engine.has_streaming());

        let (_tx, rx) = mpsc::channel(10);
        engine.set_streaming_rx(rx);
        assert!(engine.has_streaming());
    }

    #[test]
    fn test_thinking_buffer_operations() {
        let mut engine = ConversationEngine::new();

        // Take from empty returns None
        assert!(engine.take_thinking().is_none());

        // Accumulate
        engine.append_thinking("step 1, ");
        engine.append_thinking("step 2");

        // Take returns content and clears
        let content = engine.take_thinking();
        assert_eq!(content, Some("step 1, step 2".to_string()));
        assert!(engine.thinking_buffer.is_empty());

        // Clear
        engine.append_thinking("something");
        engine.clear_thinking_buffer();
        assert!(engine.thinking_buffer.is_empty());
    }

    #[test]
    fn test_dirty_tracking() {
        let mut engine = ConversationEngine::new();
        assert!(!engine.is_dirty());

        engine.dirty = true;
        assert!(engine.is_dirty());

        engine.mark_clean();
        assert!(!engine.is_dirty());
    }
}
