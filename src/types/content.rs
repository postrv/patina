//! Content block types for Anthropic API responses.
//!
//! The Claude API returns messages as arrays of content blocks rather than
//! simple text strings. This module provides the types needed to handle
//! text, tool_use, and tool_result content blocks.
//!
//! # Overview
//!
//! When Claude wants to use a tool, it returns a `tool_use` content block
//! containing the tool name and input parameters. The client must:
//!
//! 1. Execute the tool
//! 2. Send the result back as a `tool_result` content block
//! 3. Continue the conversation
//!
//! # Example
//!
//! ```rust
//! use patina::types::content::{ContentBlock, ToolUseBlock, StopReason};
//! use serde_json::json;
//!
//! // Claude's response with a tool_use
//! let block = ContentBlock::ToolUse(ToolUseBlock {
//!     id: "toolu_01abc".to_string(),
//!     name: "bash".to_string(),
//!     input: json!({"command": "ls -la"}),
//! });
//!
//! // Check if we need to handle a tool call
//! if let ContentBlock::ToolUse(tool_use) = block {
//!     println!("Claude wants to run: {}", tool_use.name);
//! }
//! ```

use crate::types::image::ImageSource;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A content block in an API message.
///
/// Messages from Claude consist of one or more content blocks.
/// During normal conversation, this is typically just text.
/// When Claude wants to use tools, it returns `ToolUse` blocks.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Plain text content from Claude.
    Text {
        /// The text content.
        text: String,
    },

    /// A request from Claude to execute a tool.
    ToolUse(ToolUseBlock),

    /// The result of a tool execution (sent by the client).
    ToolResult(ToolResultBlock),

    /// An image content block for vision requests.
    ///
    /// Images can be provided as base64-encoded data or as URLs.
    /// This is used when sending images to Claude for analysis.
    Image {
        /// The source of the image (base64 data or URL).
        source: ImageSource,
    },
}

impl ContentBlock {
    /// Creates a new text content block.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    /// Creates a new tool_use content block.
    #[must_use]
    pub fn tool_use(id: impl Into<String>, name: impl Into<String>, input: Value) -> Self {
        Self::ToolUse(ToolUseBlock {
            id: id.into(),
            name: name.into(),
            input,
        })
    }

    /// Creates a new tool_result content block.
    #[must_use]
    pub fn tool_result(tool_use_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self::ToolResult(ToolResultBlock {
            tool_use_id: tool_use_id.into(),
            content: content.into(),
            is_error: false,
        })
    }

    /// Creates a new tool_result content block for an error.
    #[must_use]
    pub fn tool_error(tool_use_id: impl Into<String>, error: impl Into<String>) -> Self {
        Self::ToolResult(ToolResultBlock {
            tool_use_id: tool_use_id.into(),
            content: error.into(),
            is_error: true,
        })
    }

    /// Creates a new image content block from an image source.
    #[must_use]
    pub fn image(source: ImageSource) -> Self {
        Self::Image { source }
    }

    /// Returns true if this is a text block.
    #[must_use]
    pub fn is_text(&self) -> bool {
        matches!(self, Self::Text { .. })
    }

    /// Returns true if this is a tool_use block.
    #[must_use]
    pub fn is_tool_use(&self) -> bool {
        matches!(self, Self::ToolUse(_))
    }

    /// Returns true if this is a tool_result block.
    #[must_use]
    pub fn is_tool_result(&self) -> bool {
        matches!(self, Self::ToolResult(_))
    }

    /// Returns true if this is an image block.
    #[must_use]
    pub fn is_image(&self) -> bool {
        matches!(self, Self::Image { .. })
    }

    /// Extracts the text content if this is a text block.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text { text } => Some(text),
            _ => None,
        }
    }

    /// Extracts the tool_use block if this is a tool_use.
    #[must_use]
    pub fn as_tool_use(&self) -> Option<&ToolUseBlock> {
        match self {
            Self::ToolUse(block) => Some(block),
            _ => None,
        }
    }

    /// Extracts the tool_result block if this is a tool_result.
    #[must_use]
    pub fn as_tool_result(&self) -> Option<&ToolResultBlock> {
        match self {
            Self::ToolResult(block) => Some(block),
            _ => None,
        }
    }

    /// Extracts the image source if this is an image block.
    #[must_use]
    pub fn as_image(&self) -> Option<&ImageSource> {
        match self {
            Self::Image { source } => Some(source),
            _ => None,
        }
    }
}

/// A tool_use content block from Claude.
///
/// When Claude decides to use a tool, it returns this block containing:
/// - A unique ID for tracking the tool call
/// - The tool name (must match a defined tool)
/// - The input parameters as JSON
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolUseBlock {
    /// Unique identifier for this tool use (e.g., "toolu_01abc123").
    pub id: String,

    /// The name of the tool to call.
    pub name: String,

    /// The input parameters for the tool as JSON.
    pub input: Value,
}

impl ToolUseBlock {
    /// Creates a new tool use block.
    #[must_use]
    pub fn new(id: impl Into<String>, name: impl Into<String>, input: Value) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            input,
        }
    }
}

/// A tool_result content block sent back to Claude.
///
/// After executing a tool, the client sends the result back to Claude
/// using this block type. The `tool_use_id` must match the ID from
/// the corresponding `ToolUseBlock`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolResultBlock {
    /// The ID of the tool_use this is a response to.
    pub tool_use_id: String,

    /// The content/output of the tool execution.
    pub content: String,

    /// Whether the tool execution resulted in an error.
    #[serde(default)]
    pub is_error: bool,
}

impl ToolResultBlock {
    /// Creates a new successful tool result.
    #[must_use]
    pub fn success(tool_use_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            tool_use_id: tool_use_id.into(),
            content: content.into(),
            is_error: false,
        }
    }

    /// Creates a new error tool result.
    #[must_use]
    pub fn error(tool_use_id: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            tool_use_id: tool_use_id.into(),
            content: error.into(),
            is_error: true,
        }
    }
}

/// The reason why Claude stopped generating.
///
/// This is critical for the agentic loop:
/// - `EndTurn` means Claude is done and waiting for user input
/// - `ToolUse` means Claude wants to use tools - continue the loop
/// - `MaxTokens` means the response was cut off
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// Claude finished its response naturally.
    #[default]
    EndTurn,

    /// Claude wants to use one or more tools.
    /// The response will contain `tool_use` content blocks.
    ToolUse,

    /// The response hit the max_tokens limit.
    MaxTokens,

    /// Stop sequence was encountered.
    StopSequence,
}

impl StopReason {
    /// Returns true if this stop reason requires tool execution.
    #[must_use]
    pub fn needs_tool_execution(&self) -> bool {
        matches!(self, Self::ToolUse)
    }

    /// Returns true if the conversation should continue automatically.
    ///
    /// This is true for `ToolUse` (execute tools and continue) but not
    /// for `EndTurn` (wait for user) or `MaxTokens` (response truncated).
    #[must_use]
    pub fn should_continue(&self) -> bool {
        matches!(self, Self::ToolUse)
    }

    /// Returns true if this is a terminal state (no more automatic actions).
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::EndTurn | Self::MaxTokens | Self::StopSequence)
    }
}

/// Extracts all tool_use blocks from a list of content blocks.
#[must_use]
pub fn extract_tool_uses(content: &[ContentBlock]) -> Vec<&ToolUseBlock> {
    content
        .iter()
        .filter_map(|block| block.as_tool_use())
        .collect()
}

/// Extracts all text content from a list of content blocks.
#[must_use]
pub fn extract_text(content: &[ContentBlock]) -> String {
    content
        .iter()
        .filter_map(|block| block.as_text())
        .collect::<Vec<_>>()
        .join("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_content_block_text_creation() {
        let block = ContentBlock::text("Hello, world!");
        assert!(block.is_text());
        assert_eq!(block.as_text(), Some("Hello, world!"));
    }

    #[test]
    fn test_content_block_tool_use_creation() {
        let block = ContentBlock::tool_use("toolu_123", "bash", json!({"command": "ls"}));
        assert!(block.is_tool_use());

        let tool_use = block.as_tool_use().unwrap();
        assert_eq!(tool_use.id, "toolu_123");
        assert_eq!(tool_use.name, "bash");
        assert_eq!(tool_use.input["command"], "ls");
    }

    #[test]
    fn test_content_block_tool_result_creation() {
        let block = ContentBlock::tool_result("toolu_123", "file1.txt\nfile2.txt");
        assert!(block.is_tool_result());

        let result = block.as_tool_result().unwrap();
        assert_eq!(result.tool_use_id, "toolu_123");
        assert_eq!(result.content, "file1.txt\nfile2.txt");
        assert!(!result.is_error);
    }

    #[test]
    fn test_content_block_tool_error_creation() {
        let block = ContentBlock::tool_error("toolu_456", "Permission denied");

        let result = block.as_tool_result().unwrap();
        assert_eq!(result.tool_use_id, "toolu_456");
        assert_eq!(result.content, "Permission denied");
        assert!(result.is_error);
    }

    #[test]
    fn test_text_block_serialization() {
        let block = ContentBlock::text("Hello");
        let json = serde_json::to_string(&block).expect("serialization should succeed");

        assert!(json.contains("\"type\":\"text\""));
        assert!(json.contains("\"text\":\"Hello\""));
    }

    #[test]
    fn test_tool_use_block_serialization() {
        let block = ContentBlock::tool_use("id", "bash", json!({"command": "pwd"}));
        let json = serde_json::to_string(&block).expect("serialization should succeed");

        assert!(json.contains("\"type\":\"tool_use\""));
        assert!(json.contains("\"id\":\"id\""));
        assert!(json.contains("\"name\":\"bash\""));
    }

    #[test]
    fn test_tool_result_block_serialization() {
        let block = ContentBlock::tool_result("id", "output");
        let json = serde_json::to_string(&block).expect("serialization should succeed");

        assert!(json.contains("\"type\":\"tool_result\""));
        assert!(json.contains("\"tool_use_id\":\"id\""));
    }

    #[test]
    fn test_text_block_deserialization() {
        let json = r#"{"type":"text","text":"Hello"}"#;
        let block: ContentBlock =
            serde_json::from_str(json).expect("deserialization should succeed");

        assert!(block.is_text());
        assert_eq!(block.as_text(), Some("Hello"));
    }

    #[test]
    fn test_tool_use_block_deserialization() {
        let json = r#"{"type":"tool_use","id":"toolu_abc","name":"bash","input":{"command":"ls"}}"#;
        let block: ContentBlock =
            serde_json::from_str(json).expect("deserialization should succeed");

        assert!(block.is_tool_use());
        let tool_use = block.as_tool_use().unwrap();
        assert_eq!(tool_use.id, "toolu_abc");
        assert_eq!(tool_use.name, "bash");
    }

    #[test]
    fn test_tool_result_block_deserialization() {
        let json = r#"{"type":"tool_result","tool_use_id":"toolu_abc","content":"output","is_error":false}"#;
        let block: ContentBlock =
            serde_json::from_str(json).expect("deserialization should succeed");

        assert!(block.is_tool_result());
        let result = block.as_tool_result().unwrap();
        assert_eq!(result.tool_use_id, "toolu_abc");
        assert!(!result.is_error);
    }

    #[test]
    fn test_stop_reason_end_turn() {
        let reason = StopReason::EndTurn;
        assert!(!reason.needs_tool_execution());
        assert!(!reason.should_continue());
        assert!(reason.is_terminal());
    }

    #[test]
    fn test_stop_reason_tool_use() {
        let reason = StopReason::ToolUse;
        assert!(reason.needs_tool_execution());
        assert!(reason.should_continue());
        assert!(!reason.is_terminal());
    }

    #[test]
    fn test_stop_reason_max_tokens() {
        let reason = StopReason::MaxTokens;
        assert!(!reason.needs_tool_execution());
        assert!(!reason.should_continue());
        assert!(reason.is_terminal());
    }

    #[test]
    fn test_stop_reason_serialization() {
        let reason = StopReason::ToolUse;
        let json = serde_json::to_string(&reason).expect("serialization should succeed");
        assert_eq!(json, "\"tool_use\"");
    }

    #[test]
    fn test_stop_reason_deserialization() {
        let json = "\"end_turn\"";
        let reason: StopReason =
            serde_json::from_str(json).expect("deserialization should succeed");
        assert_eq!(reason, StopReason::EndTurn);

        let json = "\"tool_use\"";
        let reason: StopReason =
            serde_json::from_str(json).expect("deserialization should succeed");
        assert_eq!(reason, StopReason::ToolUse);
    }

    #[test]
    fn test_stop_reason_default() {
        let reason = StopReason::default();
        assert_eq!(reason, StopReason::EndTurn);
    }

    #[test]
    fn test_extract_tool_uses() {
        let content = vec![
            ContentBlock::text("Let me run some commands."),
            ContentBlock::tool_use("toolu_1", "bash", json!({"command": "ls"})),
            ContentBlock::text("And also:"),
            ContentBlock::tool_use("toolu_2", "read_file", json!({"path": "README.md"})),
        ];

        let tool_uses = extract_tool_uses(&content);
        assert_eq!(tool_uses.len(), 2);
        assert_eq!(tool_uses[0].name, "bash");
        assert_eq!(tool_uses[1].name, "read_file");
    }

    #[test]
    fn test_extract_tool_uses_empty() {
        let content = vec![ContentBlock::text("Just text")];
        let tool_uses = extract_tool_uses(&content);
        assert!(tool_uses.is_empty());
    }

    #[test]
    fn test_extract_text() {
        let content = vec![
            ContentBlock::text("Hello "),
            ContentBlock::tool_use("id", "bash", json!({})),
            ContentBlock::text("World!"),
        ];

        let text = extract_text(&content);
        assert_eq!(text, "Hello World!");
    }

    #[test]
    fn test_extract_text_empty() {
        let content = vec![ContentBlock::tool_use("id", "bash", json!({}))];
        let text = extract_text(&content);
        assert!(text.is_empty());
    }

    #[test]
    fn test_tool_use_block_new() {
        let block = ToolUseBlock::new("id", "bash", json!({"cmd": "pwd"}));
        assert_eq!(block.id, "id");
        assert_eq!(block.name, "bash");
        assert_eq!(block.input["cmd"], "pwd");
    }

    #[test]
    fn test_tool_result_block_success() {
        let block = ToolResultBlock::success("id", "output");
        assert_eq!(block.tool_use_id, "id");
        assert_eq!(block.content, "output");
        assert!(!block.is_error);
    }

    #[test]
    fn test_tool_result_block_error() {
        let block = ToolResultBlock::error("id", "failed");
        assert_eq!(block.tool_use_id, "id");
        assert_eq!(block.content, "failed");
        assert!(block.is_error);
    }

    #[test]
    fn test_content_block_accessors_return_none_for_wrong_type() {
        let text_block = ContentBlock::text("text");
        assert!(text_block.as_tool_use().is_none());
        assert!(text_block.as_tool_result().is_none());

        let tool_use_block = ContentBlock::tool_use("id", "name", json!({}));
        assert!(tool_use_block.as_text().is_none());
        assert!(tool_use_block.as_tool_result().is_none());

        let result_block = ContentBlock::tool_result("id", "content");
        assert!(result_block.as_text().is_none());
        assert!(result_block.as_tool_use().is_none());
    }

    #[test]
    fn test_content_block_type_checks() {
        let text = ContentBlock::text("text");
        assert!(text.is_text());
        assert!(!text.is_tool_use());
        assert!(!text.is_tool_result());
        assert!(!text.is_image());

        let tool_use = ContentBlock::tool_use("id", "name", json!({}));
        assert!(!tool_use.is_text());
        assert!(tool_use.is_tool_use());
        assert!(!tool_use.is_tool_result());
        assert!(!tool_use.is_image());

        let result = ContentBlock::tool_result("id", "content");
        assert!(!result.is_text());
        assert!(!result.is_tool_use());
        assert!(result.is_tool_result());
        assert!(!result.is_image());
    }

    #[test]
    fn test_content_block_image_creation() {
        let source = ImageSource::Base64 {
            media_type: "image/png".to_string(),
            data: "iVBORw0KGgo=".to_string(),
        };
        let block = ContentBlock::image(source.clone());

        assert!(block.is_image());
        assert!(!block.is_text());
        assert!(!block.is_tool_use());
        assert!(!block.is_tool_result());

        let extracted = block.as_image().unwrap();
        assert_eq!(extracted, &source);
    }

    #[test]
    fn test_image_block_serialization_base64() {
        let source = ImageSource::Base64 {
            media_type: "image/png".to_string(),
            data: "iVBORw0KGgo=".to_string(),
        };
        let block = ContentBlock::image(source);
        let json = serde_json::to_string(&block).expect("serialization should succeed");

        assert!(
            json.contains("\"type\":\"image\""),
            "Should have image type"
        );
        assert!(json.contains("\"source\":"), "Should have source field");
        assert!(
            json.contains("\"media_type\":\"image/png\""),
            "Should have media_type in source"
        );
        assert!(
            json.contains("\"data\":\"iVBORw0KGgo=\""),
            "Should have data in source"
        );
    }

    #[test]
    fn test_image_block_serialization_url() {
        let source = ImageSource::Url {
            url: "https://example.com/image.png".to_string(),
        };
        let block = ContentBlock::image(source);
        let json = serde_json::to_string(&block).expect("serialization should succeed");

        assert!(json.contains("\"type\":\"image\""));
        assert!(json.contains("\"source\":"));
        assert!(json.contains("\"url\":\"https://example.com/image.png\""));
    }

    #[test]
    fn test_image_block_deserialization_base64() {
        let json = r#"{
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": "image/jpeg",
                "data": "SGVsbG8="
            }
        }"#;
        let block: ContentBlock =
            serde_json::from_str(json).expect("deserialization should succeed");

        assert!(block.is_image());
        let source = block.as_image().unwrap();
        match source {
            ImageSource::Base64 { media_type, data } => {
                assert_eq!(media_type, "image/jpeg");
                assert_eq!(data, "SGVsbG8=");
            }
            _ => panic!("Expected Base64 source"),
        }
    }

    #[test]
    fn test_image_block_deserialization_url() {
        let json = r#"{
            "type": "image",
            "source": {
                "type": "url",
                "url": "https://example.com/photo.jpg"
            }
        }"#;
        let block: ContentBlock =
            serde_json::from_str(json).expect("deserialization should succeed");

        assert!(block.is_image());
        let source = block.as_image().unwrap();
        match source {
            ImageSource::Url { url } => {
                assert_eq!(url, "https://example.com/photo.jpg");
            }
            _ => panic!("Expected URL source"),
        }
    }

    #[test]
    fn test_content_block_image_accessors_return_none_for_wrong_type() {
        let image_source = ImageSource::Url {
            url: "https://example.com/img.png".to_string(),
        };
        let image_block = ContentBlock::image(image_source);
        assert!(image_block.as_text().is_none());
        assert!(image_block.as_tool_use().is_none());
        assert!(image_block.as_tool_result().is_none());

        let text_block = ContentBlock::text("text");
        assert!(text_block.as_image().is_none());

        let tool_use_block = ContentBlock::tool_use("id", "name", json!({}));
        assert!(tool_use_block.as_image().is_none());

        let result_block = ContentBlock::tool_result("id", "content");
        assert!(result_block.as_image().is_none());
    }
}
