//! Message and Role types for conversation handling.
//!
//! These types represent the core data structures for chat messages
//! exchanged between user and assistant.

use serde::{Deserialize, Serialize};
use std::fmt;

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
/// use rct::types::message::Role;
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
/// # Examples
///
/// ```
/// use rct::types::message::{Message, Role};
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
    /// The text content of the message
    pub content: String,
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
}
