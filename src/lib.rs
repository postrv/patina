//! RCT - Rust Claude Terminal
//!
//! High-performance CLI for Claude API with a modular, extensible architecture.
//!
//! This library exposes the core types and functionality for testing and extension.

pub mod agents;
pub mod api;
pub mod app;
pub mod commands;
pub mod context;
pub mod hooks;
pub mod ide;
pub mod mcp;
pub mod plugins;
pub mod session;
pub mod skills;
pub mod tools;
pub mod tui;
pub mod types;
pub mod update;
pub mod util;

// Re-export core types for convenient access
pub use types::{Config, Message, Role, StreamEvent};
