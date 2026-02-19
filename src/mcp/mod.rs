//! MCP (Model Context Protocol) client
//!
//! This module implements the Model Context Protocol for communication with
//! external tools and services.
//!
//! # Modules
//!
//! - `client` - Low-level MCP client for server connections
//! - `config` - Configuration file loading (`.mcp.json`, `~/.claude.json`)
//! - `manager` - Multi-server management, tool namespacing, and routing
//! - `protocol` - JSON-RPC 2.0 message types
//! - `transport` - Stdio and SSE transport layers

pub mod client;
pub mod config;
pub mod manager;
pub mod protocol;
pub mod transport;

// Re-export key types for convenience
pub use client::McpTool;
pub use config::McpServerEntry;
pub use manager::McpManager;
