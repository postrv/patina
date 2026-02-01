//! Core type definitions for Patina.
//!
//! This module contains the fundamental data types used throughout the application,
//! organized into submodules by domain:
//!
//! - [`config`] - Application configuration types
//! - [`content`] - Content block types for API messages (text, tool_use, tool_result)
//! - [`message`] - Message and Role types for conversation handling
//! - [`stream`] - Stream event types for API response handling
//!
//! # Re-exports
//!
//! Common types are re-exported at the module level for convenience:
//!
//! ```
//! use patina::types::{Message, Role, StreamEvent, ContentBlock, StopReason};
//! ```

pub mod config;
pub mod content;
pub mod conversation;
pub mod message;
pub mod stream;

// Re-export common types for convenience
pub use config::Config;
pub use content::{ContentBlock, StopReason, ToolResultBlock, ToolUseBlock};
pub use message::{ApiMessageV2, Message, MessageContent, Role};
pub use stream::{StreamEvent, ToolUseAccumulator};

// Unified timeline types
pub use conversation::{ConversationEntry, Timeline, TimelineError};
