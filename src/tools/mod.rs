//! Tool execution for agentic capabilities.
//!
//! This module provides secure tool execution including:
//! - Bash command execution with security policy
//! - File operations with path traversal protection
//! - Edit operations with diff generation
//! - Glob pattern matching for file discovery
//! - Grep content search with regex support
//! - Web content fetching with HTML to markdown conversion
//! - External editor integration (`$EDITOR` / `$VISUAL`)
//! - Hook integration via `HookedToolExecutor`
//! - Parallel tool execution for performance optimization

pub mod ask_user;
pub mod cron;
mod executor;
pub mod external_editor;
mod hooked;
pub mod lsp;
pub mod parallel;
pub mod sandbox;
mod security;
mod stateful;
pub mod tasks;
pub mod todo;
pub mod tool_search;
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
pub use security::{
    is_dangerous_pattern, normalize_command, resolve_alias, resolve_alias_with_timeout,
    validate_command, ToolExecutionPolicy,
};

// Re-export parallel execution types for convenience
pub use parallel::{ParallelConfig, ParallelExecutor};

// Re-export tool search types
pub use tool_search::{ToolSearchError, ToolSearchRegistry, ToolSearchResult};

// Re-export task management types
pub use tasks::{
    task_create_tool, task_get_tool, task_list_tool, task_update_tool, Task, TaskStatus, TaskStore,
    TaskUpdate,
};

// Re-export external editor type
pub use external_editor::ExternalEditor;

// Re-export sandbox types
pub use sandbox::config::SandboxConfig;
pub use sandbox::policy::SandboxPolicy;
