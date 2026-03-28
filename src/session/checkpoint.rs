//! Checkpoint system for session branching and rewind.
//!
//! Checkpoints capture a snapshot of the conversation at a specific message index,
//! allowing users to rewind to previous points in the conversation or fork new
//! sessions from a checkpoint.
//!
//! # Example
//!
//! ```
//! use patina::session::Checkpoint;
//! use patina::types::Message;
//!
//! let messages = vec![
//!     Message::user("Hello"),
//!     Message::assistant("Hi there!"),
//! ];
//! let checkpoint = Checkpoint::new(1, &messages);
//! assert_eq!(checkpoint.message_index(), 1);
//! assert!(!checkpoint.content_hash().is_empty());
//! ```

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::SystemTime;

use crate::types::message::Message;

/// A checkpoint capturing the conversation state at a specific message index.
///
/// Checkpoints store the message index and a SHA-256 hash of all messages up to
/// that point. This allows integrity verification and efficient comparison
/// without storing full message copies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// The 0-based index of the last message included in this checkpoint.
    message_index: usize,

    /// SHA-256 hash of messages[0..=message_index] serialized to JSON.
    content_hash: String,

    /// When this checkpoint was created.
    created_at: SystemTime,

    /// Optional human-readable label for this checkpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
}

impl Checkpoint {
    /// Creates a new checkpoint at the given message index.
    ///
    /// Computes a SHA-256 hash of all messages from index 0 through `message_index`
    /// (inclusive). The hash is used for integrity verification and deduplication.
    ///
    /// # Arguments
    ///
    /// * `message_index` - The 0-based index of the last message to include.
    /// * `messages` - The full message slice; elements `[0..=message_index]` are hashed.
    ///
    /// # Panics
    ///
    /// Panics if `message_index >= messages.len()`.
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::session::Checkpoint;
    /// use patina::types::Message;
    ///
    /// let messages = vec![
    ///     Message::user("Hello"),
    ///     Message::assistant("Hi!"),
    /// ];
    /// let cp = Checkpoint::new(1, &messages);
    /// assert_eq!(cp.message_index(), 1);
    /// ```
    #[must_use]
    pub fn new(message_index: usize, messages: &[Message]) -> Self {
        assert!(
            message_index < messages.len(),
            "message_index {} out of bounds for {} messages",
            message_index,
            messages.len()
        );
        let content_hash = Self::compute_hash(&messages[..=message_index]);
        Self {
            message_index,
            content_hash,
            created_at: SystemTime::now(),
            label: None,
        }
    }

    /// Creates a new checkpoint with a label.
    ///
    /// Identical to [`new`](Self::new) but attaches a human-readable label.
    ///
    /// # Arguments
    ///
    /// * `message_index` - The 0-based index of the last message to include.
    /// * `messages` - The full message slice.
    /// * `label` - A human-readable label for this checkpoint.
    ///
    /// # Panics
    ///
    /// Panics if `message_index >= messages.len()`.
    #[must_use]
    pub fn with_label(message_index: usize, messages: &[Message], label: String) -> Self {
        let mut cp = Self::new(message_index, messages);
        cp.label = Some(label);
        cp
    }

    /// Returns the 0-based message index this checkpoint captures.
    #[must_use]
    pub fn message_index(&self) -> usize {
        self.message_index
    }

    /// Returns the SHA-256 content hash as a hex string.
    #[must_use]
    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    /// Returns when this checkpoint was created.
    #[must_use]
    pub fn created_at(&self) -> SystemTime {
        self.created_at
    }

    /// Returns the optional label for this checkpoint.
    #[must_use]
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    /// Computes a SHA-256 hash of the given message slice.
    ///
    /// The messages are serialized to JSON and then hashed. This produces a
    /// deterministic hash for the same sequence of messages.
    fn compute_hash(messages: &[Message]) -> String {
        let json = serde_json::to_string(messages).unwrap_or_else(|e| {
            tracing::error!("Failed to serialize messages for checkpoint hash: {e}");
            String::new()
        });
        let mut hasher = Sha256::new();
        hasher.update(json.as_bytes());
        hex::encode(hasher.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::message::Role;

    fn make_messages(count: usize) -> Vec<Message> {
        (0..count)
            .map(|i| {
                if i % 2 == 0 {
                    Message {
                        role: Role::User,
                        content: format!("User message {i}"),
                    }
                } else {
                    Message {
                        role: Role::Assistant,
                        content: format!("Assistant message {i}"),
                    }
                }
            })
            .collect()
    }

    #[test]
    fn test_checkpoint_new_creates_valid_checkpoint() {
        let messages = make_messages(4);
        let cp = Checkpoint::new(2, &messages);

        assert_eq!(cp.message_index(), 2);
        assert!(!cp.content_hash().is_empty());
        assert_eq!(cp.content_hash().len(), 64); // SHA-256 hex = 64 chars
        assert!(cp.label().is_none());
    }

    #[test]
    fn test_checkpoint_hash_is_deterministic() {
        let messages = make_messages(3);
        let cp1 = Checkpoint::new(2, &messages);
        let cp2 = Checkpoint::new(2, &messages);

        assert_eq!(
            cp1.content_hash(),
            cp2.content_hash(),
            "Same messages should produce the same hash"
        );
    }

    #[test]
    fn test_checkpoint_hash_differs_for_different_indices() {
        let messages = make_messages(4);
        let cp1 = Checkpoint::new(1, &messages);
        let cp2 = Checkpoint::new(2, &messages);

        assert_ne!(
            cp1.content_hash(),
            cp2.content_hash(),
            "Different message indices should produce different hashes"
        );
    }

    #[test]
    fn test_checkpoint_with_label() {
        let messages = make_messages(2);
        let cp = Checkpoint::with_label(1, &messages, "before refactor".to_string());

        assert_eq!(cp.label(), Some("before refactor"));
        assert_eq!(cp.message_index(), 1);
    }

    #[test]
    fn test_checkpoint_serialization_roundtrip() {
        let messages = make_messages(3);
        let cp = Checkpoint::with_label(2, &messages, "test label".to_string());

        let json = serde_json::to_string(&cp).expect("Failed to serialize checkpoint");
        let deserialized: Checkpoint =
            serde_json::from_str(&json).expect("Failed to deserialize checkpoint");

        assert_eq!(cp.message_index(), deserialized.message_index());
        assert_eq!(cp.content_hash(), deserialized.content_hash());
        assert_eq!(cp.label(), deserialized.label());
    }

    #[test]
    fn test_checkpoint_serialization_without_label_omits_field() {
        let messages = make_messages(2);
        let cp = Checkpoint::new(1, &messages);

        let json = serde_json::to_string(&cp).expect("Failed to serialize");
        assert!(
            !json.contains("label"),
            "JSON should not contain label field when None"
        );
    }

    #[test]
    #[should_panic(expected = "out of bounds")]
    fn test_checkpoint_panics_on_invalid_index() {
        let messages = make_messages(2);
        let _ = Checkpoint::new(5, &messages);
    }

    #[test]
    fn test_checkpoint_at_index_zero() {
        let messages = make_messages(3);
        let cp = Checkpoint::new(0, &messages);

        assert_eq!(cp.message_index(), 0);
        assert!(!cp.content_hash().is_empty());
    }

    #[test]
    fn test_checkpoint_created_at_is_recent() {
        let before = SystemTime::now();
        let messages = make_messages(2);
        let cp = Checkpoint::new(1, &messages);

        assert!(
            cp.created_at() >= before,
            "Checkpoint created_at should be >= test start time"
        );
    }
}
