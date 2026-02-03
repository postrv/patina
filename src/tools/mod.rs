//! Tool execution for agentic capabilities.
//!
//! This module provides secure tool execution including:
//! - Bash command execution with security policy
//! - File operations with path traversal protection
//! - Edit operations with diff generation
//! - Glob pattern matching for file discovery
//! - Grep content search with regex support
//! - Web content fetching with HTML to markdown conversion
//! - Hook integration via `HookedToolExecutor`
//! - Parallel tool execution for performance optimization

mod executor;
mod hooked;
pub mod parallel;
mod security;
mod stateful;
pub mod vision;
pub mod web_fetch;
pub mod web_search;

// Re-export executor types
pub use executor::{ToolCall, ToolExecutor, ToolResult};

// Re-export hooked executor types
pub use hooked::HookedToolExecutor;

// Re-export stateful executor types
pub use stateful::{ShellState, StatefulToolExecutor};

// Re-export security types
pub use security::{normalize_command, ToolExecutionPolicy};

// Re-export parallel execution types for convenience
pub use parallel::{ParallelConfig, ParallelExecutor};
