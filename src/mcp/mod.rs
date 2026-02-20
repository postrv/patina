//! MCP (Model Context Protocol) client
//!
//! This module implements the Model Context Protocol using the rmcp SDK
//! for communication with external tools and services.
//!
//! # Modules
//!
//! - `config` - Configuration file loading (`.mcp.json`, `~/.claude.json`)
//! - `connection` - Single-server connection wrapping the rmcp SDK
//! - `handler` - Client notification handler and event types
//! - `manager` - Multi-server management, tool namespacing, and routing
//! - `security` - MCP command validation and blocklists

pub mod config;
pub mod connection;
pub mod handler;
pub mod legacy_sse;
pub mod manager;
pub mod security;

// Re-export key types for convenience
pub use config::McpServerEntry;
pub use connection::McpConnection;
pub use handler::{McpEvent, PatinaClientHandler};
pub use manager::McpManager;
pub use security::validate_mcp_command;
