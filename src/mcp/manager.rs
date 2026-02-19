//! Production MCP server manager.
//!
//! Manages multiple MCP server connections, namespaces their tools, and routes
//! tool calls to the correct server. Handles startup, discovery, and shutdown.
//!
//! # Tool Namespacing
//!
//! Tools from MCP servers are namespaced as `servername__toolname` (double underscore)
//! to avoid collisions between servers and with built-in tools.
//!
//! # Example
//!
//! ```no_run
//! use patina::mcp::config::load_mcp_config;
//! use patina::mcp::manager::McpManager;
//! use std::path::Path;
//! use std::time::Duration;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let configs = load_mcp_config(Path::new("."))?;
//! let mut manager = McpManager::start_all(configs, Duration::from_secs(10)).await;
//!
//! // Get all tool definitions for the API
//! let tools = manager.tool_definitions();
//!
//! // Route a tool call
//! let result = manager.call_tool("server__echo", serde_json::json!({"text": "hi"})).await?;
//!
//! // Clean shutdown
//! manager.shutdown_all().await;
//! # Ok(())
//! # }
//! ```

use crate::api::tools::ToolDefinition;
use crate::mcp::client::{validate_mcp_command, McpClient, McpTool};
use crate::mcp::config::McpServerEntry;
use crate::tools::ToolResult;
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

/// The double-underscore separator used for tool namespacing.
const NAMESPACE_SEPARATOR: &str = "__";

/// Status of a managed MCP server.
#[derive(Debug, Clone)]
pub enum ServerStatus {
    /// Server is being started.
    Starting,
    /// Server is connected and ready.
    Connected,
    /// Server failed to start or crashed.
    Failed(String),
    /// Server has been stopped.
    Stopped,
}

impl ServerStatus {
    /// Returns `true` if the server is connected and ready.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        matches!(self, Self::Connected)
    }
}

/// A managed MCP server with its client, status, and discovered tools.
struct ManagedServer {
    name: String,
    client: McpClient,
    status: ServerStatus,
    tools: Vec<McpTool>,
}

/// Manages multiple MCP server connections.
///
/// Handles server lifecycle (start, discover tools, route calls, shutdown),
/// tool namespacing, and graceful error handling.
pub struct McpManager {
    servers: Vec<ManagedServer>,
}

impl McpManager {
    /// Starts all configured MCP servers in parallel.
    ///
    /// Each server is started independently with the given timeout. If a server
    /// fails to start or times out, it is marked as `Failed` and other servers
    /// are unaffected.
    ///
    /// # Arguments
    ///
    /// * `configs` - Map of server name to configuration entry
    /// * `timeout` - Maximum time to wait for each server to start and respond
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use patina::mcp::manager::McpManager;
    /// use std::collections::HashMap;
    /// use std::time::Duration;
    ///
    /// # async fn example() {
    /// let manager = McpManager::start_all(HashMap::new(), Duration::from_secs(10)).await;
    /// assert!(manager.is_empty());
    /// # }
    /// ```
    pub async fn start_all(configs: HashMap<String, McpServerEntry>, timeout: Duration) -> Self {
        let mut servers = Vec::new();

        // Build startup futures for all servers
        let mut futures = Vec::new();
        let mut names = Vec::new();

        for (name, entry) in configs {
            if entry.is_disabled() {
                tracing::debug!(server = %name, "Skipping disabled MCP server");
                continue;
            }

            if entry.is_sse() {
                tracing::warn!(
                    server = %name,
                    url = ?entry.url,
                    "SSE MCP transport not yet supported, skipping"
                );
                servers.push(ManagedServer {
                    name: name.clone(),
                    client: McpClient::new(&name, "unused", vec![]),
                    status: ServerStatus::Failed("SSE transport not yet supported".to_string()),
                    tools: Vec::new(),
                });
                continue;
            }

            names.push(name.clone());
            futures.push(start_single_server(name, entry, timeout));
        }

        // Run all startups in parallel
        let results = futures::future::join_all(futures).await;

        for result in results {
            servers.push(result);
        }

        Self { servers }
    }

    /// Returns namespaced tool definitions for all connected servers.
    ///
    /// Tools are returned in `ToolDefinition` format suitable for the Anthropic API.
    /// Each tool name is prefixed with `servername__`.
    #[must_use]
    pub fn tool_definitions(&self) -> Vec<ToolDefinition> {
        let mut defs = Vec::new();
        for server in &self.servers {
            if !server.status.is_connected() {
                continue;
            }
            for tool in &server.tools {
                defs.push(mcp_tool_to_definition(&server.name, tool));
            }
        }
        defs
    }

    /// Routes a namespaced tool call to the correct server.
    ///
    /// Parses the namespace from the tool name, finds the matching server,
    /// and forwards the call with the original (non-namespaced) tool name.
    ///
    /// # Arguments
    ///
    /// * `namespaced_name` - Tool name in `server__tool` format
    /// * `input` - Tool input arguments as JSON
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The tool name doesn't contain the namespace separator
    /// - The server namespace is not found
    /// - The server is not connected
    /// - The underlying tool call fails
    pub async fn call_tool(&mut self, namespaced_name: &str, input: Value) -> Result<ToolResult> {
        let (server_name, tool_name) = parse_namespaced_tool(namespaced_name).ok_or_else(|| {
            anyhow!("Not an MCP tool (no namespace separator): {namespaced_name}")
        })?;

        let server = self
            .servers
            .iter_mut()
            .find(|s| s.name == server_name)
            .ok_or_else(|| anyhow!("MCP server not found: {server_name}"))?;

        if !server.status.is_connected() {
            return Err(anyhow!(
                "MCP server '{}' is not connected (status: {:?})",
                server_name,
                server.status
            ));
        }

        match server.client.call_tool(tool_name, input).await {
            Ok(result) => Ok(convert_mcp_result(&result)),
            Err(e) => {
                tracing::warn!(
                    server = %server_name,
                    tool = %tool_name,
                    error = %e,
                    "MCP tool call failed"
                );
                Ok(ToolResult::Error(format!(
                    "MCP tool call failed ({server_name}/{tool_name}): {e}"
                )))
            }
        }
    }

    /// Shuts down all managed servers.
    ///
    /// Attempts a graceful shutdown for each server. Servers that are already
    /// stopped or failed are skipped without error.
    pub async fn shutdown_all(&mut self) {
        for server in &mut self.servers {
            if !server.status.is_connected() {
                server.status = ServerStatus::Stopped;
                continue;
            }

            tracing::debug!(server = %server.name, "Shutting down MCP server");

            match tokio::time::timeout(Duration::from_secs(5), server.client.stop()).await {
                Ok(Ok(())) => {
                    tracing::debug!(server = %server.name, "MCP server stopped cleanly");
                }
                Ok(Err(e)) => {
                    tracing::warn!(server = %server.name, error = %e, "Error stopping MCP server");
                    server.client.force_stop().await;
                }
                Err(_) => {
                    tracing::warn!(server = %server.name, "MCP server stop timed out, force-stopping");
                    server.client.force_stop().await;
                }
            }

            server.status = ServerStatus::Stopped;
        }
    }

    /// Returns the status of all servers.
    #[must_use]
    pub fn server_statuses(&self) -> Vec<(&str, &ServerStatus)> {
        self.servers
            .iter()
            .map(|s| (s.name.as_str(), &s.status))
            .collect()
    }

    /// Returns `true` if there are no managed servers.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }

    /// Returns the number of connected servers.
    #[must_use]
    pub fn connected_count(&self) -> usize {
        self.servers
            .iter()
            .filter(|s| s.status.is_connected())
            .count()
    }

    /// Returns the total number of tools across all connected servers.
    #[must_use]
    pub fn tool_count(&self) -> usize {
        self.servers
            .iter()
            .filter(|s| s.status.is_connected())
            .map(|s| s.tools.len())
            .sum()
    }
}

/// Starts a single MCP server: validates command, starts client, discovers tools.
async fn start_single_server(
    name: String,
    entry: McpServerEntry,
    timeout: Duration,
) -> ManagedServer {
    let args_refs: Vec<&str> = entry.args.iter().map(String::as_str).collect();

    // Validate the command before attempting to start
    if let Err(e) = validate_mcp_command(&entry.command, &entry.args) {
        tracing::warn!(
            server = %name,
            command = %entry.command,
            error = %e,
            "MCP server command validation failed"
        );
        return ManagedServer {
            name: name.clone(),
            client: McpClient::new(&name, &entry.command, args_refs),
            status: ServerStatus::Failed(format!("validation failed: {e}")),
            tools: Vec::new(),
        };
    }

    let mut client = McpClient::new_with_env(&name, &entry.command, args_refs, entry.env.clone());

    // Start client with timeout
    match tokio::time::timeout(timeout, client.start()).await {
        Ok(Ok(())) => {
            tracing::debug!(server = %name, "MCP server started");
        }
        Ok(Err(e)) => {
            tracing::warn!(server = %name, error = %e, "MCP server failed to start");
            return ManagedServer {
                name,
                client,
                status: ServerStatus::Failed(format!("start failed: {e}")),
                tools: Vec::new(),
            };
        }
        Err(_) => {
            tracing::warn!(server = %name, "MCP server start timed out");
            client.force_stop().await;
            return ManagedServer {
                name,
                client,
                status: ServerStatus::Failed("start timed out".to_string()),
                tools: Vec::new(),
            };
        }
    }

    // Discover tools with timeout
    match tokio::time::timeout(timeout, client.list_tools()).await {
        Ok(Ok(tools)) => {
            tracing::info!(
                server = %name,
                tool_count = tools.len(),
                "MCP server connected, discovered tools"
            );
            ManagedServer {
                name,
                client,
                status: ServerStatus::Connected,
                tools,
            }
        }
        Ok(Err(e)) => {
            tracing::warn!(server = %name, error = %e, "Failed to discover MCP tools");
            ManagedServer {
                name,
                client,
                status: ServerStatus::Failed(format!("tool discovery failed: {e}")),
                tools: Vec::new(),
            }
        }
        Err(_) => {
            tracing::warn!(server = %name, "MCP tool discovery timed out");
            ManagedServer {
                name,
                client,
                status: ServerStatus::Failed("tool discovery timed out".to_string()),
                tools: Vec::new(),
            }
        }
    }
}

/// Converts an MCP tool result (JSON) to a `ToolResult`.
///
/// Extracts text from the MCP `content` array. If `isError` is true in the
/// response, returns `ToolResult::Error`.
fn convert_mcp_result(result: &Value) -> ToolResult {
    let is_error = result
        .get("isError")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let text = result
        .get("content")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_else(|| result.to_string());

    if is_error {
        ToolResult::Error(text)
    } else {
        ToolResult::Success(text)
    }
}

/// Converts an MCP tool to an API `ToolDefinition` with a namespaced name.
///
/// # Arguments
///
/// * `server_name` - The server name for namespacing
/// * `tool` - The MCP tool from the server
///
/// # Examples
///
/// ```
/// use patina::mcp::manager::mcp_tool_to_definition;
/// use patina::mcp::client::McpTool;
/// use serde_json::json;
///
/// let tool = McpTool {
///     name: "read".to_string(),
///     description: "Read a file".to_string(),
///     input_schema: json!({"type": "object"}),
/// };
/// let def = mcp_tool_to_definition("fs", &tool);
/// assert_eq!(def.name, "fs__read");
/// assert_eq!(def.description, "Read a file");
/// ```
#[must_use]
pub fn mcp_tool_to_definition(server_name: &str, tool: &McpTool) -> ToolDefinition {
    ToolDefinition::new(
        namespace_tool_name(server_name, &tool.name),
        &tool.description,
        tool.input_schema.clone(),
    )
}

/// Creates a namespaced tool name from server and tool names.
///
/// # Examples
///
/// ```
/// use patina::mcp::manager::namespace_tool_name;
///
/// assert_eq!(namespace_tool_name("fs", "read"), "fs__read");
/// assert_eq!(namespace_tool_name("narsil", "scan_security"), "narsil__scan_security");
/// ```
#[must_use]
pub fn namespace_tool_name(server: &str, tool: &str) -> String {
    format!("{server}{NAMESPACE_SEPARATOR}{tool}")
}

/// Parses a namespaced tool name into (server_name, tool_name).
///
/// Splits on the first double-underscore. If the name contains multiple
/// separators, only the first is used (the rest are part of the tool name).
///
/// # Examples
///
/// ```
/// use patina::mcp::manager::parse_namespaced_tool;
///
/// assert_eq!(parse_namespaced_tool("fs__read"), Some(("fs", "read")));
/// assert_eq!(parse_namespaced_tool("a__b__c"), Some(("a", "b__c")));
/// assert_eq!(parse_namespaced_tool("bash"), None);
/// ```
#[must_use]
pub fn parse_namespaced_tool(name: &str) -> Option<(&str, &str)> {
    name.find(NAMESPACE_SEPARATOR)
        .map(|pos| (&name[..pos], &name[pos + NAMESPACE_SEPARATOR.len()..]))
}

/// Returns `true` if the tool name contains the namespace separator.
///
/// # Examples
///
/// ```
/// use patina::mcp::manager::is_mcp_tool;
///
/// assert!(is_mcp_tool("fs__read"));
/// assert!(!is_mcp_tool("bash"));
/// ```
#[must_use]
pub fn is_mcp_tool(name: &str) -> bool {
    name.contains(NAMESPACE_SEPARATOR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_status_is_connected() {
        assert!(ServerStatus::Connected.is_connected());
        assert!(!ServerStatus::Starting.is_connected());
        assert!(!ServerStatus::Failed("err".to_string()).is_connected());
        assert!(!ServerStatus::Stopped.is_connected());
    }

    #[test]
    fn namespace_tool_name_format() {
        assert_eq!(namespace_tool_name("fs", "read"), "fs__read");
        assert_eq!(
            namespace_tool_name("narsil", "scan_security"),
            "narsil__scan_security"
        );
    }

    #[test]
    fn parse_namespaced_tool_valid() {
        assert_eq!(parse_namespaced_tool("fs__read"), Some(("fs", "read")));
    }

    #[test]
    fn parse_namespaced_tool_no_separator() {
        assert_eq!(parse_namespaced_tool("bash"), None);
        assert_eq!(parse_namespaced_tool("read_file"), None);
    }

    #[test]
    fn parse_namespaced_tool_multiple_separators() {
        assert_eq!(parse_namespaced_tool("a__b__c"), Some(("a", "b__c")));
    }

    #[test]
    fn is_mcp_tool_detection() {
        assert!(is_mcp_tool("fs__read"));
        assert!(is_mcp_tool("narsil__scan_security"));
        assert!(!is_mcp_tool("bash"));
        assert!(!is_mcp_tool("read_file"));
    }

    #[tokio::test]
    async fn manager_empty_configs_yields_empty() {
        let manager = McpManager::start_all(HashMap::new(), Duration::from_secs(1)).await;
        assert!(manager.is_empty());
        assert_eq!(manager.connected_count(), 0);
        assert_eq!(manager.tool_count(), 0);
        assert!(manager.tool_definitions().is_empty());
    }

    #[tokio::test]
    async fn manager_startup_invalid_command_marks_failed() {
        let mut configs = HashMap::new();
        configs.insert(
            "bad-server".to_string(),
            McpServerEntry {
                command: "/nonexistent/bin/server".to_string(),
                args: vec![],
                env: HashMap::new(),
                url: None,
                headers: None,
                disabled: false,
            },
        );

        let manager = McpManager::start_all(configs, Duration::from_secs(2)).await;
        assert_eq!(manager.connected_count(), 0);

        let statuses = manager.server_statuses();
        assert_eq!(statuses.len(), 1);
        assert!(!statuses[0].1.is_connected());
    }

    #[tokio::test]
    async fn manager_startup_skips_disabled() {
        let mut configs = HashMap::new();
        configs.insert(
            "disabled-server".to_string(),
            McpServerEntry {
                command: "echo".to_string(),
                args: vec![],
                env: HashMap::new(),
                url: None,
                headers: None,
                disabled: true,
            },
        );

        let manager = McpManager::start_all(configs, Duration::from_secs(1)).await;
        assert!(manager.is_empty());
    }

    #[tokio::test]
    async fn manager_sse_not_supported_marks_failed() {
        let mut configs = HashMap::new();
        configs.insert(
            "sse-server".to_string(),
            McpServerEntry {
                command: String::new(),
                args: vec![],
                env: HashMap::new(),
                url: Some("http://localhost:8080/sse".to_string()),
                headers: None,
                disabled: false,
            },
        );

        let manager = McpManager::start_all(configs, Duration::from_secs(1)).await;
        assert_eq!(manager.connected_count(), 0);
        let statuses = manager.server_statuses();
        assert_eq!(statuses.len(), 1);
        assert!(matches!(statuses[0].1, ServerStatus::Failed(_)));
    }

    #[tokio::test]
    async fn call_tool_no_double_underscore_returns_error() {
        let mut manager = McpManager::start_all(HashMap::new(), Duration::from_secs(1)).await;
        let result = manager
            .call_tool("bash", serde_json::json!({"command": "ls"}))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no namespace"));
    }

    #[tokio::test]
    async fn call_tool_unknown_namespace_returns_error() {
        let mut manager = McpManager::start_all(HashMap::new(), Duration::from_secs(1)).await;
        let result = manager
            .call_tool("nonexistent__tool", serde_json::json!({}))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[tokio::test]
    async fn shutdown_handles_empty_manager() {
        let mut manager = McpManager::start_all(HashMap::new(), Duration::from_secs(1)).await;
        manager.shutdown_all().await;
        // No panic
    }

    #[test]
    fn convert_mcp_result_text_success() {
        let result = serde_json::json!({
            "content": [
                {"type": "text", "text": "hello world"}
            ]
        });
        match convert_mcp_result(&result) {
            ToolResult::Success(text) => assert_eq!(text, "hello world"),
            _ => panic!("Expected Success"),
        }
    }

    #[test]
    fn convert_mcp_result_error() {
        let result = serde_json::json!({
            "isError": true,
            "content": [
                {"type": "text", "text": "something went wrong"}
            ]
        });
        match convert_mcp_result(&result) {
            ToolResult::Error(text) => assert_eq!(text, "something went wrong"),
            _ => panic!("Expected Error"),
        }
    }

    #[test]
    fn convert_mcp_result_multiple_content() {
        let result = serde_json::json!({
            "content": [
                {"type": "text", "text": "line 1"},
                {"type": "text", "text": "line 2"}
            ]
        });
        match convert_mcp_result(&result) {
            ToolResult::Success(text) => assert_eq!(text, "line 1\nline 2"),
            _ => panic!("Expected Success"),
        }
    }

    #[test]
    fn convert_mcp_result_empty_content() {
        let result = serde_json::json!({"content": []});
        match convert_mcp_result(&result) {
            ToolResult::Success(text) => assert!(text.is_empty()),
            _ => panic!("Expected Success"),
        }
    }

    #[test]
    fn mcp_tool_to_definition_converts_correctly() {
        let tool = McpTool {
            name: "echo".to_string(),
            description: "Echo text back".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "text": {"type": "string"}
                },
                "required": ["text"]
            }),
        };
        let def = mcp_tool_to_definition("mock", &tool);
        assert_eq!(def.name, "mock__echo");
        assert_eq!(def.description, "Echo text back");
        assert_eq!(def.input_schema["type"], "object");
    }

    #[test]
    fn mcp_tool_to_definition_uses_namespaced_name() {
        let tool = McpTool {
            name: "scan_security".to_string(),
            description: "Scan for issues".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        };
        let def = mcp_tool_to_definition("narsil", &tool);
        assert_eq!(def.name, "narsil__scan_security");
    }

    #[tokio::test]
    async fn manager_server_statuses_returns_all() {
        let mut configs = HashMap::new();
        configs.insert(
            "bad1".to_string(),
            McpServerEntry {
                command: "/nonexistent1".to_string(),
                args: vec![],
                env: HashMap::new(),
                url: None,
                headers: None,
                disabled: false,
            },
        );
        configs.insert(
            "bad2".to_string(),
            McpServerEntry {
                command: "/nonexistent2".to_string(),
                args: vec![],
                env: HashMap::new(),
                url: None,
                headers: None,
                disabled: false,
            },
        );

        let manager = McpManager::start_all(configs, Duration::from_secs(1)).await;
        let statuses = manager.server_statuses();
        assert_eq!(statuses.len(), 2);
    }

    #[tokio::test]
    async fn tool_definitions_empty_when_no_servers() {
        let manager = McpManager::start_all(HashMap::new(), Duration::from_secs(1)).await;
        assert!(manager.tool_definitions().is_empty());
    }
}
