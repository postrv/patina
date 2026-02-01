//! Message and Role types for conversation handling.
//!
//! These types represent the core data structures for chat messages
//! exchanged between user and assistant.
//!
//! # Content Types
//!
//! Messages can contain either:
//! - Simple text content (most common for user messages)
//! - Content blocks (for tool results sent back to Claude)
//!
//! The `MessageContent` enum handles both cases transparently.

use serde::{Deserialize, Serialize};
use std::fmt;

use super::content::ContentBlock;

/// Represents a participant role in a conversation.
///
/// # Serialization
///
/// Roles are serialized as lowercase strings for API compatibility:
/// - `Role::User` -> `"user"`
/// - `Role::Assistant` -> `"assistant"`
///
/// # Examples
///
/// ```
/// use patina::types::message::Role;
///
/// let role = Role::User;
/// assert_eq!(format!("{}", role), "user");
///
/// let json = serde_json::to_string(&role).unwrap();
/// assert_eq!(json, "\"user\"");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Human user sending messages
    User,
    /// AI assistant responding to messages
    Assistant,
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Role::User => write!(f, "user"),
            Role::Assistant => write!(f, "assistant"),
        }
    }
}

/// Content of a message - can be simple text or content blocks.
///
/// The Anthropic API accepts either format:
/// - A simple string for text-only messages
/// - An array of content blocks for messages with tool results
///
/// # Serialization
///
/// `Text` serializes as a plain JSON string:
/// ```json
/// "Hello, Claude!"
/// ```
///
/// `Blocks` serializes as a JSON array:
/// ```json
/// [{"type": "text", "text": "Here's the result:"}, {"type": "tool_result", ...}]
/// ```
///
/// # Examples
///
/// ```rust
/// use patina::types::message::MessageContent;
/// use patina::types::content::ContentBlock;
///
/// // Simple text
/// let content = MessageContent::text("Hello!");
/// assert!(content.is_text());
///
/// // From string (automatic conversion)
/// let content: MessageContent = "Hello!".into();
///
/// // With content blocks
/// let content = MessageContent::blocks(vec![
///     ContentBlock::text("Here's the output:"),
///     ContentBlock::tool_result("toolu_123", "file1.txt\nfile2.txt"),
/// ]);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum MessageContent {
    /// Simple text content (most common).
    Text(String),
    /// Array of content blocks (for tool results).
    Blocks(Vec<ContentBlock>),
}

impl MessageContent {
    /// Creates a text content.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(text.into())
    }

    /// Creates content from a list of content blocks.
    #[must_use]
    pub fn blocks(blocks: Vec<ContentBlock>) -> Self {
        Self::Blocks(blocks)
    }

    /// Returns true if this is simple text content.
    #[must_use]
    pub fn is_text(&self) -> bool {
        matches!(self, Self::Text(_))
    }

    /// Returns true if this contains content blocks.
    #[must_use]
    pub fn is_blocks(&self) -> bool {
        matches!(self, Self::Blocks(_))
    }

    /// Extracts the text content if this is text.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s),
            Self::Blocks(_) => None,
        }
    }

    /// Extracts the content blocks if this is blocks.
    #[must_use]
    pub fn as_blocks(&self) -> Option<&[ContentBlock]> {
        match self {
            Self::Blocks(blocks) => Some(blocks),
            Self::Text(_) => None,
        }
    }

    /// Returns the text representation of the content.
    ///
    /// For text content, returns the text directly.
    /// For blocks, extracts and concatenates all text blocks.
    #[must_use]
    pub fn to_text(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::Blocks(blocks) => blocks
                .iter()
                .filter_map(|b| b.as_text())
                .collect::<Vec<_>>()
                .join(""),
        }
    }
}

impl Default for MessageContent {
    fn default() -> Self {
        Self::Text(String::new())
    }
}

impl From<String> for MessageContent {
    fn from(s: String) -> Self {
        Self::Text(s)
    }
}

impl From<&str> for MessageContent {
    fn from(s: &str) -> Self {
        Self::Text(s.to_string())
    }
}

impl From<Vec<ContentBlock>> for MessageContent {
    fn from(blocks: Vec<ContentBlock>) -> Self {
        Self::Blocks(blocks)
    }
}

/// A single message in a conversation.
///
/// Messages consist of a role (who sent it) and content (what was said).
///
/// # Serialization
///
/// Messages serialize to JSON with the following structure:
/// ```json
/// {
///   "role": "user",
///   "content": "Hello, Claude!"
/// }
/// ```
///
/// Or with content blocks:
/// ```json
/// {
///   "role": "user",
///   "content": [{"type": "tool_result", "tool_use_id": "...", "content": "..."}]
/// }
/// ```
///
/// # Examples
///
/// ```
/// use patina::types::message::{Message, Role};
///
/// let msg = Message {
///     role: Role::User,
///     content: "Hello!".to_string(),
/// };
///
/// let json = serde_json::to_string(&msg).unwrap();
/// assert!(json.contains("\"role\":\"user\""));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// The role of the message sender
    pub role: Role,
    /// The text content of the message (backward compatible field)
    pub content: String,
}

impl Message {
    /// Creates a new user message with text content.
    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    /// Creates a new assistant message with text content.
    #[must_use]
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

/// A message with flexible content (text or content blocks).
///
/// This is the preferred type for building API requests with tool results.
/// Unlike `Message` which only supports string content, `ApiMessageV2`
/// supports both text and content block arrays.
///
/// # Examples
///
/// ```rust
/// use patina::types::message::{ApiMessageV2, Role, MessageContent};
/// use patina::types::content::ContentBlock;
///
/// // Simple text message
/// let msg = ApiMessageV2::user("What's 2+2?");
///
/// // Message with tool result
/// let msg = ApiMessageV2::user_with_content(MessageContent::blocks(vec![
///     ContentBlock::tool_result("toolu_123", "4"),
/// ]));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiMessageV2 {
    /// The role of the message sender.
    pub role: Role,
    /// The content of the message (text or content blocks).
    pub content: MessageContent,
}

impl ApiMessageV2 {
    /// Creates a new message.
    #[must_use]
    pub fn new(role: Role, content: impl Into<MessageContent>) -> Self {
        Self {
            role,
            content: content.into(),
        }
    }

    /// Creates a user message with text content.
    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: MessageContent::text(content),
        }
    }

    /// Creates a user message with arbitrary content.
    #[must_use]
    pub fn user_with_content(content: MessageContent) -> Self {
        Self {
            role: Role::User,
            content,
        }
    }

    /// Creates an assistant message with text content.
    #[must_use]
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: MessageContent::text(content),
        }
    }

    /// Creates an assistant message with arbitrary content.
    #[must_use]
    pub fn assistant_with_content(content: MessageContent) -> Self {
        Self {
            role: Role::Assistant,
            content,
        }
    }

    /// Converts this message to a legacy `Message`.
    ///
    /// Content blocks are converted to their text representation.
    #[must_use]
    pub fn to_legacy(&self) -> Message {
        Message {
            role: self.role,
            content: self.content.to_text(),
        }
    }
}

impl From<Message> for ApiMessageV2 {
    fn from(msg: Message) -> Self {
        Self {
            role: msg.role,
            content: MessageContent::text(msg.content),
        }
    }
}

impl From<&Message> for ApiMessageV2 {
    fn from(msg: &Message) -> Self {
        Self {
            role: msg.role,
            content: MessageContent::text(&msg.content),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_role_display_user() {
        assert_eq!(Role::User.to_string(), "user");
    }

    #[test]
    fn test_role_display_assistant() {
        assert_eq!(Role::Assistant.to_string(), "assistant");
    }

    #[test]
    fn test_role_equality() {
        assert_eq!(Role::User, Role::User);
        assert_eq!(Role::Assistant, Role::Assistant);
        assert_ne!(Role::User, Role::Assistant);
    }

    #[test]
    fn test_message_content_text() {
        let content = MessageContent::text("Hello");
        assert!(content.is_text());
        assert!(!content.is_blocks());
        assert_eq!(content.as_text(), Some("Hello"));
        assert!(content.as_blocks().is_none());
    }

    #[test]
    fn test_message_content_blocks() {
        let content = MessageContent::blocks(vec![ContentBlock::text("Hi")]);
        assert!(!content.is_text());
        assert!(content.is_blocks());
        assert!(content.as_text().is_none());
        assert!(content.as_blocks().is_some());
    }

    #[test]
    fn test_message_content_from_string() {
        let content: MessageContent = "Hello".into();
        assert!(content.is_text());
        assert_eq!(content.as_text(), Some("Hello"));
    }

    #[test]
    fn test_message_content_to_text() {
        let text = MessageContent::text("Hello");
        assert_eq!(text.to_text(), "Hello");

        let blocks = MessageContent::blocks(vec![
            ContentBlock::text("Hello "),
            ContentBlock::text("World"),
        ]);
        assert_eq!(blocks.to_text(), "Hello World");
    }

    #[test]
    fn test_message_content_text_serialization() {
        let content = MessageContent::text("Hello");
        let json = serde_json::to_string(&content).expect("should serialize");
        assert_eq!(json, "\"Hello\"");
    }

    #[test]
    fn test_message_content_blocks_serialization() {
        let content = MessageContent::blocks(vec![ContentBlock::text("Hi")]);
        let json = serde_json::to_string(&content).expect("should serialize");
        assert!(json.contains("\"type\":\"text\""));
    }

    #[test]
    fn test_message_content_text_deserialization() {
        let json = "\"Hello\"";
        let content: MessageContent = serde_json::from_str(json).expect("should deserialize");
        assert!(content.is_text());
        assert_eq!(content.as_text(), Some("Hello"));
    }

    #[test]
    fn test_message_content_blocks_deserialization() {
        let json = r#"[{"type":"text","text":"Hi"}]"#;
        let content: MessageContent = serde_json::from_str(json).expect("should deserialize");
        assert!(content.is_blocks());
    }

    #[test]
    fn test_message_user() {
        let msg = Message::user("Hello");
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content, "Hello");
    }

    #[test]
    fn test_message_assistant() {
        let msg = Message::assistant("Hi there");
        assert_eq!(msg.role, Role::Assistant);
        assert_eq!(msg.content, "Hi there");
    }

    #[test]
    fn test_api_message_v2_user() {
        let msg = ApiMessageV2::user("Hello");
        assert_eq!(msg.role, Role::User);
        assert!(msg.content.is_text());
    }

    #[test]
    fn test_api_message_v2_user_with_content() {
        let msg = ApiMessageV2::user_with_content(MessageContent::blocks(vec![
            ContentBlock::tool_result("id", "output"),
        ]));
        assert_eq!(msg.role, Role::User);
        assert!(msg.content.is_blocks());
    }

    #[test]
    fn test_api_message_v2_to_legacy() {
        let msg = ApiMessageV2::user("Hello");
        let legacy = msg.to_legacy();
        assert_eq!(legacy.role, Role::User);
        assert_eq!(legacy.content, "Hello");
    }

    #[test]
    fn test_api_message_v2_from_message() {
        let msg = Message::user("Hello");
        let v2: ApiMessageV2 = msg.into();
        assert_eq!(v2.role, Role::User);
        assert_eq!(v2.content.to_text(), "Hello");
    }

    #[test]
    fn test_api_message_v2_serialization_text() {
        let msg = ApiMessageV2::user("Hello");
        let json = serde_json::to_string(&msg).expect("should serialize");
        assert!(json.contains("\"role\":\"user\""));
        assert!(json.contains("\"content\":\"Hello\""));
    }

    #[test]
    fn test_api_message_v2_serialization_blocks() {
        let msg = ApiMessageV2::user_with_content(MessageContent::blocks(vec![
            ContentBlock::tool_result("toolu_123", "output"),
        ]));
        let json = serde_json::to_string(&msg).expect("should serialize");
        assert!(json.contains("\"role\":\"user\""));
        assert!(json.contains("tool_result"));
        assert!(json.contains("toolu_123"));
    }

    #[test]
    fn test_message_content_default() {
        let content = MessageContent::default();
        assert!(content.is_text());
        assert_eq!(content.as_text(), Some(""));
    }
}
