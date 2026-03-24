//! Persistent memory system for cross-session context.
//!
//! Stores user preferences, project knowledge, feedback, and references
//! that persist across sessions. Memories are injected into the system
//! prompt to maintain context continuity.
//!
//! # Memory Types
//!
//! - **User**: Role, preferences, working style
//! - **Feedback**: Corrections and approach guidance
//! - **Project**: Project-specific facts and context
//! - **Reference**: Pointers to external resources
//!
//! # Storage
//!
//! Memories are stored as JSON in the application data directory,
//! with SHA-256 content hashing for integrity verification.

pub mod store;
pub mod types;

pub use store::MemoryStore;
pub use types::{MemoryEntry, MemorySource, MemoryType};
