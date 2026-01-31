//! Stream event types for API response handling.
//!
//! These types represent events received during streaming API responses.

use serde::{Deserialize, Serialize};

/// Events received during a streaming API response.
///
/// When streaming messages from the Claude API, various events are emitted
/// that indicate the state of the response.
///
/// # Examples
///
/// ```
/// use patina::types::stream::StreamEvent;
///
/// let event = StreamEvent::ContentDelta("Hello".to_string());
/// match event {
///     StreamEvent::ContentDelta(text) => println!("Received: {}", text),
///     StreamEvent::MessageStop => println!("Stream complete"),
///     StreamEvent::Error(e) => eprintln!("Error: {}", e),
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum StreamEvent {
    /// A delta containing new content text
    ContentDelta(String),
    /// The message stream has completed
    MessageStop,
    /// An error occurred during streaming
    Error(String),
}

impl StreamEvent {
    /// Returns true if this is an error event.
    #[must_use]
    pub fn is_error(&self) -> bool {
        matches!(self, StreamEvent::Error(_))
    }

    /// Returns true if this is a content delta event.
    #[must_use]
    pub fn is_content(&self) -> bool {
        matches!(self, StreamEvent::ContentDelta(_))
    }

    /// Returns true if this is a message stop event.
    #[must_use]
    pub fn is_stop(&self) -> bool {
        matches!(self, StreamEvent::MessageStop)
    }

    /// Extracts the content text if this is a content delta event.
    #[must_use]
    pub fn content(&self) -> Option<&str> {
        match self {
            StreamEvent::ContentDelta(text) => Some(text),
            _ => None,
        }
    }

    /// Extracts the error message if this is an error event.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        match self {
            StreamEvent::Error(msg) => Some(msg),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_event_is_error() {
        assert!(StreamEvent::Error("test".to_string()).is_error());
        assert!(!StreamEvent::ContentDelta("text".to_string()).is_error());
        assert!(!StreamEvent::MessageStop.is_error());
    }

    #[test]
    fn test_stream_event_is_content() {
        assert!(StreamEvent::ContentDelta("text".to_string()).is_content());
        assert!(!StreamEvent::Error("test".to_string()).is_content());
        assert!(!StreamEvent::MessageStop.is_content());
    }

    #[test]
    fn test_stream_event_is_stop() {
        assert!(StreamEvent::MessageStop.is_stop());
        assert!(!StreamEvent::ContentDelta("text".to_string()).is_stop());
        assert!(!StreamEvent::Error("test".to_string()).is_stop());
    }

    #[test]
    fn test_stream_event_content() {
        let delta = StreamEvent::ContentDelta("hello".to_string());
        assert_eq!(delta.content(), Some("hello"));

        let stop = StreamEvent::MessageStop;
        assert_eq!(stop.content(), None);
    }

    #[test]
    fn test_stream_event_error() {
        let err = StreamEvent::Error("failed".to_string());
        assert_eq!(err.error(), Some("failed"));

        let stop = StreamEvent::MessageStop;
        assert_eq!(stop.error(), None);
    }

    #[test]
    fn test_stream_event_serialization() {
        let delta = StreamEvent::ContentDelta("test".to_string());
        let json = serde_json::to_string(&delta).expect("Should serialize");
        let parsed: StreamEvent = serde_json::from_str(&json).expect("Should deserialize");
        assert_eq!(delta, parsed);
    }
}
