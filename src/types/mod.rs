//! Core type definitions for Patina.
//!
//! This module contains the fundamental data types used throughout the application,
//! organized into submodules by domain:
//!
//! - [`config`] - Application configuration types
//! - [`message`] - Message and Role types for conversation handling
//! - [`stream`] - Stream event types for API response handling
//!
//! # Re-exports
//!
//! Common types are re-exported at the module level for convenience:
//!
//! ```
//! use patina::types::{Message, Role, StreamEvent};
//! ```

pub mod config;
pub mod message;
pub mod stream;

// Re-export common types for convenience
pub use config::Config;
pub use message::{Message, Role};
pub use stream::StreamEvent;
