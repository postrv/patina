//! Patina - High-performance terminal client for Claude API
//!
//! A modular, extensible CLI for interacting with Claude.
//!
//! This library exposes the core types and functionality for testing and extension.

pub mod agents;
pub mod api;
pub mod app;
pub mod commands;
pub mod context;
pub mod enterprise;
pub mod error;
pub mod hooks;
pub mod ide;
pub mod mcp;
pub mod plugins;
pub mod session;
pub mod shell;
pub mod skills;
pub mod tools;
pub mod tui;
pub mod types;
pub mod update;
pub mod util;
pub mod worktree;

// Re-export core types for convenient access
pub use session::{
    ContextFile, Session, SessionContext, SessionManager, WorktreeCommit, WorktreeSession,
};
pub use types::{Config, Message, Role, StreamEvent};
