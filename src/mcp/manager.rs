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
//! let result = load_mcp_config(Path::new("."))?;
//! let mut manager = McpManager::start_all(result.servers, Duration::from_secs(10)).await;
//!
//! // Get all tool definitions for the API
//! let tools = manager.tool_definitions();
//!
//! // Route a tool call (note: &self, not &mut self)
//! let result = manager.call_tool("server__echo", serde_json::json!({"text": "hi"})).await?;
//!
//! // Clean shutdown
//! manager.shutdown_all().await;
//! # Ok(())
//! # }
//! ```

use crate::api::tools::ToolDefinition;
use crate::mcp::auth::{auth_headers, resolve_bearer_token};
use crate::mcp::config::{McpAuthConfig, McpServerEntry};
use crate::mcp::connection::McpConnection;
use crate::mcp::token_storage::McpTokenStore;
use crate::tools::ToolResult;
use anyhow::{anyhow, Result};
use rmcp::model::{CallToolResult as SdkCallToolResult, Tool};
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
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

/// A managed MCP server with its connection, status, and tool count.
struct ManagedServer {
    name: String,
    connection: Option<McpConnection>,
    status: ServerStatus,
}

impl ManagedServer {
    /// Returns the tool count from the connection's shared tool list.
    fn tool_count(&self) -> usize {
        self.connection
            .as_ref()
            .map(|c| c.tools().len())
            .unwrap_or(0)
    }
}

/// Manages multiple MCP server connections.
///
/// Handles server lifecycle (start, discover tools, route calls, shutdown),
/// tool namespacing, and graceful error handling.
///
/// Tool calls use `&self` (not `&mut self`) because the SDK uses interior
/// mutability via `RunningService`.
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
        let mut futures: Vec<Pin<Box<dyn Future<Output = ManagedServer> + Send>>> = Vec::new();

        for (name, entry) in configs {
            if entry.is_disabled() {
                tracing::debug!(server = %name, "Skipping disabled MCP server");
                continue;
            }

            if entry.is_sse() {
                futures.push(Box::pin(start_http_server(name, entry, timeout)));
            } else {
                futures.push(Box::pin(start_stdio_server(name, entry, timeout)));
            }
        }

        let servers = futures::future::join_all(futures).await;

        Self { servers }
    }

    /// Creates an MCP manager connected to Forge as a single gateway server.
    ///
    /// Instead of connecting to individual MCP servers, this connects to the
    /// Forge process which routes tool calls through a V8 sandbox. The Forge
    /// binary is started as a stdio MCP server using the prepared context.
    ///
    /// # Arguments
    ///
    /// * `forge_context` - Prepared Forge context with binary path, args, and env
    /// * `timeout` - Connection timeout for the Forge process
    pub async fn start_with_forge(
        forge_context: &super::forge::ForgeContext,
        timeout: Duration,
    ) -> Self {
        let entry = McpServerEntry {
            command: forge_context.command().to_string_lossy().to_string(),
            args: forge_context.args(),
            env: forge_context.env().clone(),
            url: None,
            headers: None,
            transport_type: None,
            disabled: false,
            auth: None,
        };

        let server = start_stdio_server("forge".to_string(), entry, timeout).await;
        Self {
            servers: vec![server],
        }
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
            if let Some(conn) = &server.connection {
                for tool in conn.tools() {
                    defs.push(sdk_tool_to_definition(&server.name, &tool));
                }
            }
        }
        defs
    }

    /// Routes a namespaced tool call to the correct server.
    ///
    /// Parses the namespace from the tool name, finds the matching server,
    /// and forwards the call with the original (non-namespaced) tool name.
    ///
    /// Uses `&self` instead of `&mut self` because the SDK uses interior mutability.
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
    pub async fn call_tool(&self, namespaced_name: &str, input: Value) -> Result<ToolResult> {
        let (server_name, tool_name) = parse_namespaced_tool(namespaced_name).ok_or_else(|| {
            anyhow!("Not an MCP tool (no namespace separator): {namespaced_name}")
        })?;

        let server = self
            .servers
            .iter()
            .find(|s| s.name == server_name)
            .ok_or_else(|| anyhow!("MCP server not found: {server_name}"))?;

        if !server.status.is_connected() {
            return Err(anyhow!(
                "MCP server '{}' is not connected (status: {:?})",
                server_name,
                server.status
            ));
        }

        let conn = server
            .connection
            .as_ref()
            .ok_or_else(|| anyhow!("MCP server '{}' has no active connection", server_name))?;

        match conn.call_tool(tool_name, input).await {
            Ok(result) => Ok(convert_sdk_result(&result)),
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

            if let Some(mut conn) = server.connection.take() {
                conn.close().await;
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
            .map(|s| s.tool_count())
            .sum()
    }

    /// Returns per-server details: name, status, and tool count.
    ///
    /// Useful for building the `/mcp` command output. Returns all servers
    /// regardless of status.
    #[must_use]
    pub fn server_details(&self) -> Vec<(&str, &ServerStatus, usize)> {
        self.servers
            .iter()
            .map(|s| (s.name.as_str(), &s.status, s.tool_count()))
            .collect()
    }

    /// Drains all pending events from all connected servers.
    ///
    /// Iterates over each managed server and collects buffered
    /// [`McpEvent`](crate::mcp::handler::McpEvent) values.
    /// Non-blocking — returns immediately with whatever events are queued.
    #[must_use]
    pub fn drain_all_events(&self) -> Vec<crate::mcp::handler::McpEvent> {
        let mut events = Vec::new();
        for server in &self.servers {
            if let Some(conn) = &server.connection {
                events.extend(conn.drain_events());
            }
        }
        events
    }
}

/// Starts a single stdio MCP server via `McpConnection`.
///
/// If the entry has an `auth` config, the resolved token is passed to the
/// child process as the `MCP_AUTH_TOKEN` environment variable.
async fn start_stdio_server(
    name: String,
    entry: McpServerEntry,
    timeout: Duration,
) -> ManagedServer {
    let mut env = entry.env.clone();

    // Inject auth token as environment variable for stdio transport
    if let Some(auth) = &entry.auth {
        match resolve_auth_token(&name, auth).await {
            Ok(resolved) => {
                env.insert(
                    "MCP_AUTH_TOKEN".to_string(),
                    resolved.expose_secret().to_string(),
                );
            }
            Err(e) => {
                tracing::warn!(
                    server = %name,
                    error = %e,
                    "Failed to resolve auth token for stdio server"
                );
                return ManagedServer {
                    name,
                    connection: None,
                    status: ServerStatus::Failed(format!("Auth resolution failed: {e}")),
                };
            }
        }
    }

    match McpConnection::connect_stdio(&name, &entry.command, &entry.args, &env, timeout).await {
        Ok(conn) => {
            let tool_count = conn.tools().len();
            tracing::info!(
                server = %name,
                tool_count,
                "MCP server connected, discovered tools"
            );
            ManagedServer {
                name,
                connection: Some(conn),
                status: ServerStatus::Connected,
            }
        }
        Err(e) => {
            tracing::warn!(
                server = %name,
                error = %e,
                "MCP server failed to start"
            );
            ManagedServer {
                name,
                connection: None,
                status: ServerStatus::Failed(format!("{e}")),
            }
        }
    }
}

/// Starts a single HTTP/SSE MCP server via `McpConnection`.
///
/// Routes to legacy SSE or streamable HTTP based on the entry's transport type.
/// If the entry has an `auth` config, the resolved token is added as an
/// `Authorization: Bearer <token>` header.
async fn start_http_server(
    name: String,
    entry: McpServerEntry,
    timeout: Duration,
) -> ManagedServer {
    let url = match &entry.url {
        Some(url) => url.clone(),
        None => {
            return ManagedServer {
                name,
                connection: None,
                status: ServerStatus::Failed("SSE entry missing url".to_string()),
            };
        }
    };

    // Merge auth headers into the connection headers
    let mut headers = entry.headers.clone().unwrap_or_default();
    if let Some(auth) = &entry.auth {
        match resolve_auth_token(&name, auth).await {
            Ok(ref resolved) => {
                for (k, v) in auth_headers(resolved) {
                    headers.insert(k, v);
                }
            }
            Err(e) => {
                tracing::warn!(
                    server = %name,
                    error = %e,
                    "Failed to resolve auth token for HTTP server"
                );
                return ManagedServer {
                    name,
                    connection: None,
                    status: ServerStatus::Failed(format!("Auth resolution failed: {e}")),
                };
            }
        }
    }
    let headers_opt = if headers.is_empty() {
        None
    } else {
        Some(headers)
    };

    let is_legacy = entry.is_legacy_sse();
    let result = if is_legacy {
        tracing::debug!(server = %name, url = %url, "Connecting via legacy SSE transport");
        McpConnection::connect_legacy_sse(&name, &url, &headers_opt, timeout).await
    } else {
        tracing::debug!(server = %name, url = %url, "Connecting via streamable HTTP transport");
        McpConnection::connect_http(&name, &url, &headers_opt, timeout).await
    };

    let transport_label = if is_legacy { "legacy SSE" } else { "HTTP" };

    match result {
        Ok(conn) => {
            let tool_count = conn.tools().len();
            tracing::info!(
                server = %name,
                tool_count,
                transport = transport_label,
                "MCP server connected, discovered tools"
            );
            ManagedServer {
                name,
                connection: Some(conn),
                status: ServerStatus::Connected,
            }
        }
        Err(e) => {
            tracing::warn!(
                server = %name,
                error = %e,
                transport = transport_label,
                "MCP server failed to start"
            );
            ManagedServer {
                name,
                connection: None,
                status: ServerStatus::Failed(format!("{e}")),
            }
        }
    }
}

/// Resolves an authentication token from an [`McpAuthConfig`].
///
/// For [`McpAuthConfig::Bearer`]: resolves literal or `$ENV_VAR` tokens.
/// For [`McpAuthConfig::OAuth`]: loads a cached token from the token store,
/// refreshing if expired.
///
/// # Errors
///
/// Returns an error if token resolution or refresh fails.
async fn resolve_auth_token(server_name: &str, auth: &McpAuthConfig) -> Result<SecretString> {
    match auth {
        McpAuthConfig::Bearer { token } => resolve_bearer_token(token),
        McpAuthConfig::OAuth { .. } => {
            let token_store_dir = token_store_dir()?;
            let store = McpTokenStore::new(token_store_dir);
            let client = crate::mcp::auth::McpOAuthClient::new(server_name, auth, store)?;
            client.get_token().await
        }
    }
}

/// Returns the default directory for storing MCP OAuth tokens.
///
/// Uses `~/.local/share/patina/` on Unix, or the platform-equivalent data
/// directory via the `directories` crate.
///
/// # Errors
///
/// Returns an error if the home/data directory cannot be determined.
fn token_store_dir() -> Result<std::path::PathBuf> {
    let base_dirs = directories::BaseDirs::new()
        .ok_or_else(|| anyhow!("Cannot determine home directory for token storage"))?;
    Ok(base_dirs.data_local_dir().join("patina"))
}

/// Converts an SDK `CallToolResult` to a `ToolResult`.
///
/// Extracts text from the SDK's typed `Content` items. If `is_error` is true,
/// returns `ToolResult::Error`.
fn convert_sdk_result(result: &SdkCallToolResult) -> ToolResult {
    let is_error = result.is_error.unwrap_or(false);

    let text = result
        .content
        .iter()
        .filter_map(|c| match c.raw.as_text() {
            Some(t) => Some(t.text.as_str()),
            None => c.raw.as_resource().and_then(|r| match &r.resource {
                rmcp::model::ResourceContents::TextResourceContents { text, .. } => {
                    Some(text.as_str())
                }
                rmcp::model::ResourceContents::BlobResourceContents { .. } => None,
            }),
        })
        .collect::<Vec<_>>()
        .join("\n");

    if is_error {
        ToolResult::Error(text)
    } else {
        ToolResult::Success(text)
    }
}

/// Converts an SDK `Tool` to an API `ToolDefinition` with a namespaced name.
fn sdk_tool_to_definition(server_name: &str, tool: &Tool) -> ToolDefinition {
    let description = tool.description.as_deref().unwrap_or("");

    // Convert Arc<JsonObject> to serde_json::Value
    let schema = serde_json::to_value(tool.input_schema.as_ref())
        .unwrap_or_else(|_| serde_json::json!({"type": "object"}));

    ToolDefinition::new(
        namespace_tool_name(server_name, &tool.name),
        description,
        schema,
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
                transport_type: None,
                disabled: false,
                auth: None,
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
                transport_type: None,
                disabled: true,
                auth: None,
            },
        );

        let manager = McpManager::start_all(configs, Duration::from_secs(1)).await;
        assert!(manager.is_empty());
    }

    #[tokio::test]
    async fn manager_sse_attempts_connection() {
        let mut configs = HashMap::new();
        configs.insert(
            "sse-server".to_string(),
            McpServerEntry {
                command: String::new(),
                args: vec![],
                env: HashMap::new(),
                url: Some("http://localhost:8080/sse".to_string()),
                headers: None,
                transport_type: None,
                disabled: false,
                auth: None,
            },
        );

        let manager = McpManager::start_all(configs, Duration::from_secs(2)).await;
        assert_eq!(manager.connected_count(), 0);
        let statuses = manager.server_statuses();
        assert_eq!(statuses.len(), 1);
        assert!(matches!(statuses[0].1, ServerStatus::Failed(_)));
    }

    #[tokio::test]
    async fn manager_sse_invalid_url_marks_failed() {
        let mut configs = HashMap::new();
        configs.insert(
            "bad-sse".to_string(),
            McpServerEntry {
                command: String::new(),
                args: vec![],
                env: HashMap::new(),
                url: Some("http://192.0.2.1:1/sse".to_string()),
                headers: None,
                transport_type: None,
                disabled: false,
                auth: None,
            },
        );

        let manager = McpManager::start_all(configs, Duration::from_secs(2)).await;
        assert_eq!(manager.connected_count(), 0);
        let statuses = manager.server_statuses();
        assert_eq!(statuses.len(), 1);
        assert!(matches!(statuses[0].1, ServerStatus::Failed(_)));
    }

    #[tokio::test]
    async fn call_tool_no_double_underscore_returns_error() {
        let manager = McpManager::start_all(HashMap::new(), Duration::from_secs(1)).await;
        let result = manager
            .call_tool("bash", serde_json::json!({"command": "ls"}))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no namespace"));
    }

    #[tokio::test]
    async fn call_tool_unknown_namespace_returns_error() {
        let manager = McpManager::start_all(HashMap::new(), Duration::from_secs(1)).await;
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
    }

    #[test]
    fn convert_sdk_result_text_success() {
        use rmcp::model::Content;
        let result = SdkCallToolResult::success(vec![Content::text("hello world")]);
        match convert_sdk_result(&result) {
            ToolResult::Success(text) => assert_eq!(text, "hello world"),
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[test]
    fn convert_sdk_result_error() {
        use rmcp::model::Content;
        let result = SdkCallToolResult::error(vec![Content::text("something went wrong")]);
        match convert_sdk_result(&result) {
            ToolResult::Error(text) => assert_eq!(text, "something went wrong"),
            other => panic!("Expected Error, got {:?}", other),
        }
    }

    #[test]
    fn convert_sdk_result_multiple_content() {
        use rmcp::model::Content;
        let result =
            SdkCallToolResult::success(vec![Content::text("line 1"), Content::text("line 2")]);
        match convert_sdk_result(&result) {
            ToolResult::Success(text) => assert_eq!(text, "line 1\nline 2"),
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[test]
    fn convert_sdk_result_empty_content() {
        let result = SdkCallToolResult::success(vec![]);
        match convert_sdk_result(&result) {
            ToolResult::Success(text) => assert!(text.is_empty()),
            other => panic!("Expected Success, got {:?}", other),
        }
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
                transport_type: None,
                disabled: false,
                auth: None,
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
                transport_type: None,
                disabled: false,
                auth: None,
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

    /// Verify that `call_tool` works with `&self` (not `&mut self`).
    #[tokio::test]
    async fn call_tool_takes_shared_ref() {
        let manager = McpManager::start_all(HashMap::new(), Duration::from_secs(1)).await;
        // This compiles only if call_tool takes &self
        let _result = manager.call_tool("a__b", serde_json::json!({})).await;
    }

    #[tokio::test]
    async fn drain_all_events_empty_manager_returns_empty() {
        let manager = McpManager::start_all(HashMap::new(), Duration::from_secs(1)).await;
        let events = manager.drain_all_events();
        assert!(events.is_empty(), "Empty manager should yield no events");
    }

    #[tokio::test]
    async fn drain_all_events_takes_shared_ref() {
        let manager = McpManager::start_all(HashMap::new(), Duration::from_secs(1)).await;
        // Verify this compiles with &self (not &mut self) — needed for Arc access
        let _events = manager.drain_all_events();
    }
}
