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
pub mod trust;

// Re-export key types for convenience
pub use config::McpServerEntry;
pub use connection::McpConnection;
pub use handler::{McpEvent, PatinaClientHandler};
pub use manager::McpManager;
pub use security::validate_mcp_command;

use reqwest::header::HeaderMap;
use std::collections::HashMap;

/// Parses optional string-keyed headers into a [`HeaderMap`].
///
/// Silently skips entries whose key or value cannot be parsed as valid
/// HTTP header components. This is used by both the legacy SSE transport
/// and the streamable HTTP transport for custom header injection.
///
/// # Arguments
///
/// * `headers` - Optional map of header name/value string pairs
///
/// # Returns
///
/// A `HeaderMap` containing all successfully parsed headers.
#[must_use]
pub fn parse_custom_headers(headers: &Option<HashMap<String, String>>) -> HeaderMap {
    let mut header_map = HeaderMap::new();
    if let Some(hdrs) = headers {
        for (k, v) in hdrs {
            if let (Ok(name), Ok(val)) = (
                k.parse::<reqwest::header::HeaderName>(),
                v.parse::<reqwest::header::HeaderValue>(),
            ) {
                header_map.insert(name, val);
            }
        }
    }
    header_map
}
