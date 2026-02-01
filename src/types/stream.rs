//! Stream event types for API response handling.
//!
//! These types represent events received during streaming API responses.
//! When Claude uses tools, the stream includes additional events for
//! tool_use content blocks.
//!
//! # Streaming Protocol
//!
//! The Anthropic streaming API sends Server-Sent Events (SSE) in this order:
//!
//! 1. `message_start` - Message begins
//! 2. `content_block_start` - Each content block begins (text or tool_use)
//! 3. `content_block_delta` - Content fragments (text_delta or input_json_delta)
//! 4. `content_block_stop` - Content block ends
//! 5. `message_delta` - Message metadata (including stop_reason)
//! 6. `message_stop` - Message ends
//!
//! # Tool Use Flow
//!
//! When Claude wants to use a tool:
//!
//! 1. `ToolUseStart` event with tool ID and name
//! 2. Zero or more `ToolUseInputDelta` events with JSON fragments
//! 3. `ToolUseComplete` event (implicit when content_block_stop is received)
//! 4. `MessageComplete` with `StopReason::ToolUse`

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::content::StopReason;

/// Events received during a streaming API response.
///
/// When streaming messages from the Claude API, various events are emitted
/// that indicate the state of the response.
///
/// # Examples
///
/// ```rust
/// use patina::types::stream::StreamEvent;
/// use patina::types::content::StopReason;
///
/// // Handle different event types
/// fn handle_event(event: StreamEvent) {
///     match event {
///         StreamEvent::ContentDelta(text) => print!("{}", text),
///         StreamEvent::ToolUseStart { id, name, index } => {
///             println!("Tool call: {} ({}) at index {}", name, id, index);
///         }
///         StreamEvent::MessageComplete { stop_reason } => {
///             if stop_reason.needs_tool_execution() {
///                 println!("Need to execute tools");
///             }
///         }
///         _ => {}
///     }
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum StreamEvent {
    /// A delta containing new content text.
    ContentDelta(String),

    /// A tool_use content block is starting.
    ///
    /// The tool ID is used to correlate tool results with tool calls.
    /// The name identifies which tool Claude wants to use.
    ToolUseStart {
        /// Unique identifier for this tool use (e.g., "toolu_01abc123").
        id: String,
        /// The name of the tool (e.g., "bash", "read_file").
        name: String,
        /// Index of this content block in the message.
        index: usize,
    },

    /// A fragment of JSON input for the current tool_use.
    ///
    /// These fragments should be concatenated to form the complete JSON input.
    /// Parse the accumulated JSON when `ToolUseComplete` is received.
    ToolUseInputDelta {
        /// Index of the content block this delta belongs to.
        index: usize,
        /// A fragment of the JSON input string.
        partial_json: String,
    },

    /// A tool_use content block is complete.
    ///
    /// At this point, the accumulated JSON from `ToolUseInputDelta` events
    /// can be parsed and the tool can be executed.
    ToolUseComplete {
        /// Index of the completed content block.
        index: usize,
    },

    /// A content block has completed (generic).
    ContentBlockComplete {
        /// Index of the completed content block.
        index: usize,
    },

    /// The message is complete with a stop reason.
    ///
    /// If `stop_reason` is `StopReason::ToolUse`, the message contains
    /// tool_use blocks that need to be executed before continuing.
    MessageComplete {
        /// Why Claude stopped generating.
        stop_reason: StopReason,
    },

    /// The message stream has completed (legacy, deprecated).
    ///
    /// Use `MessageComplete` instead for new code to access the stop_reason.
    MessageStop,

    /// An error occurred during streaming.
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

    /// Returns true if this is a message stop event (including MessageComplete).
    #[must_use]
    pub fn is_stop(&self) -> bool {
        matches!(
            self,
            StreamEvent::MessageStop | StreamEvent::MessageComplete { .. }
        )
    }

    /// Returns true if this is a tool_use related event.
    #[must_use]
    pub fn is_tool_use(&self) -> bool {
        matches!(
            self,
            StreamEvent::ToolUseStart { .. }
                | StreamEvent::ToolUseInputDelta { .. }
                | StreamEvent::ToolUseComplete { .. }
        )
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

    /// Extracts the stop reason if this is a MessageComplete event.
    #[must_use]
    pub fn stop_reason(&self) -> Option<StopReason> {
        match self {
            StreamEvent::MessageComplete { stop_reason } => Some(*stop_reason),
            StreamEvent::MessageStop => Some(StopReason::EndTurn),
            _ => None,
        }
    }
}

/// Accumulator for building complete tool_use blocks from stream events.
///
/// During streaming, tool_use content comes in fragments. This struct
/// accumulates those fragments and produces a complete `ToolUseBlock`
/// when the content block is complete.
#[derive(Debug, Clone, Default)]
pub struct ToolUseAccumulator {
    /// The tool use ID (set on ToolUseStart).
    pub id: Option<String>,
    /// The tool name (set on ToolUseStart).
    pub name: Option<String>,
    /// Accumulated JSON input string.
    pub input_json: String,
}

impl ToolUseAccumulator {
    /// Creates a new empty accumulator.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Initializes the accumulator from a ToolUseStart event.
    pub fn start(&mut self, id: String, name: String) {
        self.id = Some(id);
        self.name = Some(name);
        self.input_json.clear();
    }

    /// Appends a JSON fragment from a ToolUseInputDelta event.
    pub fn append_input(&mut self, partial_json: &str) {
        self.input_json.push_str(partial_json);
    }

    /// Parses the accumulated input and returns the JSON value.
    ///
    /// # Errors
    ///
    /// Returns an error if the accumulated JSON is invalid.
    pub fn parse_input(&self) -> Result<Value, serde_json::Error> {
        if self.input_json.is_empty() {
            Ok(Value::Object(serde_json::Map::new()))
        } else {
            serde_json::from_str(&self.input_json)
        }
    }

    /// Resets the accumulator for reuse.
    pub fn reset(&mut self) {
        self.id = None;
        self.name = None;
        self.input_json.clear();
    }

    /// Returns true if the accumulator has a tool use in progress.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.id.is_some()
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
        assert!(StreamEvent::MessageComplete {
            stop_reason: StopReason::EndTurn
        }
        .is_stop());
        assert!(!StreamEvent::ContentDelta("text".to_string()).is_stop());
        assert!(!StreamEvent::Error("test".to_string()).is_stop());
    }

    #[test]
    fn test_stream_event_is_tool_use() {
        let start = StreamEvent::ToolUseStart {
            id: "id".to_string(),
            name: "bash".to_string(),
            index: 0,
        };
        assert!(start.is_tool_use());

        let delta = StreamEvent::ToolUseInputDelta {
            index: 0,
            partial_json: "{".to_string(),
        };
        assert!(delta.is_tool_use());

        let complete = StreamEvent::ToolUseComplete { index: 0 };
        assert!(complete.is_tool_use());

        assert!(!StreamEvent::ContentDelta("text".to_string()).is_tool_use());
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
    fn test_stream_event_stop_reason() {
        let complete = StreamEvent::MessageComplete {
            stop_reason: StopReason::ToolUse,
        };
        assert_eq!(complete.stop_reason(), Some(StopReason::ToolUse));

        let stop = StreamEvent::MessageStop;
        assert_eq!(stop.stop_reason(), Some(StopReason::EndTurn));

        let delta = StreamEvent::ContentDelta("text".to_string());
        assert_eq!(delta.stop_reason(), None);
    }

    #[test]
    fn test_stream_event_serialization() {
        let delta = StreamEvent::ContentDelta("test".to_string());
        let json = serde_json::to_string(&delta).expect("Should serialize");
        let parsed: StreamEvent = serde_json::from_str(&json).expect("Should deserialize");
        assert_eq!(delta, parsed);
    }

    #[test]
    fn test_tool_use_start_serialization() {
        let event = StreamEvent::ToolUseStart {
            id: "toolu_123".to_string(),
            name: "bash".to_string(),
            index: 0,
        };
        let json = serde_json::to_string(&event).expect("Should serialize");
        assert!(json.contains("\"id\":\"toolu_123\""));
        assert!(json.contains("\"name\":\"bash\""));
    }

    #[test]
    fn test_message_complete_serialization() {
        let event = StreamEvent::MessageComplete {
            stop_reason: StopReason::ToolUse,
        };
        let json = serde_json::to_string(&event).expect("Should serialize");
        assert!(json.contains("tool_use"));
    }

    #[test]
    fn test_tool_use_accumulator_new() {
        let acc = ToolUseAccumulator::new();
        assert!(acc.id.is_none());
        assert!(acc.name.is_none());
        assert!(acc.input_json.is_empty());
        assert!(!acc.is_active());
    }

    #[test]
    fn test_tool_use_accumulator_start() {
        let mut acc = ToolUseAccumulator::new();
        acc.start("toolu_123".to_string(), "bash".to_string());

        assert_eq!(acc.id, Some("toolu_123".to_string()));
        assert_eq!(acc.name, Some("bash".to_string()));
        assert!(acc.is_active());
    }

    #[test]
    fn test_tool_use_accumulator_append_input() {
        let mut acc = ToolUseAccumulator::new();
        acc.start("id".to_string(), "bash".to_string());
        acc.append_input("{\"command\":");
        acc.append_input("\"ls\"}");

        assert_eq!(acc.input_json, "{\"command\":\"ls\"}");
    }

    #[test]
    fn test_tool_use_accumulator_parse_input() {
        let mut acc = ToolUseAccumulator::new();
        acc.start("id".to_string(), "bash".to_string());
        acc.append_input("{\"command\":\"pwd\"}");

        let value = acc.parse_input().expect("Should parse");
        assert_eq!(value["command"], "pwd");
    }

    #[test]
    fn test_tool_use_accumulator_parse_empty_input() {
        let acc = ToolUseAccumulator::new();
        let value = acc.parse_input().expect("Should parse empty as object");
        assert!(value.is_object());
    }

    #[test]
    fn test_tool_use_accumulator_reset() {
        let mut acc = ToolUseAccumulator::new();
        acc.start("id".to_string(), "bash".to_string());
        acc.append_input("{\"x\":1}");
        acc.reset();

        assert!(acc.id.is_none());
        assert!(acc.name.is_none());
        assert!(acc.input_json.is_empty());
        assert!(!acc.is_active());
    }

    #[test]
    fn test_tool_use_accumulator_default() {
        let acc = ToolUseAccumulator::default();
        assert!(!acc.is_active());
    }
}
