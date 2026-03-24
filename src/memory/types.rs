//! Memory entry types for persistent cross-session memory.
//!
//! Defines the core data types for the memory system, including
//! memory categories and entry metadata.

use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// Categories of persistent memory.
///
/// Each type serves a different purpose and is rendered differently
/// in the system prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryType {
    /// User preferences, role, and working style.
    User,
    /// Corrections and guidance on approach.
    Feedback,
    /// Project-specific facts and context.
    Project,
    /// Pointers to external resources.
    Reference,
}

impl MemoryType {
    /// Returns a human-readable label for display.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Feedback => "feedback",
            Self::Project => "project",
            Self::Reference => "reference",
        }
    }

    /// Returns a section heading for system prompt rendering.
    #[must_use]
    pub fn heading(&self) -> &'static str {
        match self {
            Self::User => "User Preferences",
            Self::Feedback => "Feedback & Corrections",
            Self::Project => "Project Knowledge",
            Self::Reference => "External References",
        }
    }
}

impl std::fmt::Display for MemoryType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

/// How a memory entry was created.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MemorySource {
    /// Explicitly added by the user via /memory add.
    Manual,
    /// Automatically detected from conversation patterns.
    AutoDetected,
    /// Imported from an external file.
    Imported,
}

/// A single persistent memory entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    /// Unique identifier (UUID).
    pub id: String,
    /// Category of this memory.
    pub memory_type: MemoryType,
    /// The memory content (markdown).
    pub content: String,
    /// Optional tags for search and filtering.
    pub tags: Vec<String>,
    /// When this entry was created.
    pub created_at: SystemTime,
    /// When this entry was last modified.
    pub updated_at: SystemTime,
    /// How this entry was created.
    pub source: MemorySource,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_type_label() {
        assert_eq!(MemoryType::User.label(), "user");
        assert_eq!(MemoryType::Feedback.label(), "feedback");
        assert_eq!(MemoryType::Project.label(), "project");
        assert_eq!(MemoryType::Reference.label(), "reference");
    }

    #[test]
    fn test_memory_type_heading() {
        assert_eq!(MemoryType::User.heading(), "User Preferences");
        assert_eq!(MemoryType::Project.heading(), "Project Knowledge");
    }

    #[test]
    fn test_memory_type_serialization() {
        let json = serde_json::to_string(&MemoryType::Feedback).unwrap();
        assert_eq!(json, r#""feedback""#);
        let deserialized: MemoryType = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, MemoryType::Feedback);
    }

    #[test]
    fn test_memory_entry_serialization() {
        let entry = MemoryEntry {
            id: "test-id".to_string(),
            memory_type: MemoryType::User,
            content: "Prefers Rust".to_string(),
            tags: vec!["language".to_string()],
            created_at: SystemTime::UNIX_EPOCH,
            updated_at: SystemTime::UNIX_EPOCH,
            source: MemorySource::Manual,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let deserialized: MemoryEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.content, "Prefers Rust");
        assert_eq!(deserialized.memory_type, MemoryType::User);
    }

    #[test]
    fn test_memory_source_variants() {
        let sources = [
            MemorySource::Manual,
            MemorySource::AutoDetected,
            MemorySource::Imported,
        ];
        for source in &sources {
            let json = serde_json::to_string(source).unwrap();
            let deserialized: MemorySource = serde_json::from_str(&json).unwrap();
            assert_eq!(&deserialized, source);
        }
    }
}
