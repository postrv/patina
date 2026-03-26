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
pub mod image;
pub mod message;
pub mod render_view;
pub mod stream;
pub mod ui_state;

// Re-export common types for convenience
pub use config::Config;
pub use content::{ContentBlock, StopReason, ToolResultBlock, ToolUseBlock};
pub use image::{
    contains_images, select_model_for_content, ImageContent, ImageError, ImageSource, MediaType,
};
pub use message::{ApiMessageV2, Message, MessageContent, Role};
#[allow(deprecated)]
pub use stream::{
    CompletedToolUse, StartedToolUse, StreamEvent, ToolUseAccumulator, ToolUseBuilder,
};

// Unified timeline types
pub use conversation::{ConversationEntry, Timeline, TimelineError};

// Shared UI state types (used by both app::state and tui)
pub use ui_state::{AgentPanelEntry, AgentPanelStatus, ContinuousLoopStatus, GateResult};

// Render view model for TUI decoupling
pub use render_view::RenderView;
