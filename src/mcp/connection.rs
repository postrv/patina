//! MCP connection wrapping the rmcp SDK's `RunningService`.
//!
//! Provides `McpConnection`, a thin wrapper around the SDK that handles:
//! - Stdio connections (child process) with command validation
//! - HTTP connections (streamable HTTP / SSE)
//! - Tool listing, tool calling, and graceful shutdown
//!
//! # Example
//!
//! ```ignore
//! use patina::mcp::connection::McpConnection;
//! use std::time::Duration;
//!
//! let conn = McpConnection::connect_stdio(
//!     "my-server",
//!     "/usr/bin/mcp-server",
//!     &[],
//!     &Default::default(),
//!     Duration::from_secs(10),
//! ).await?;
//!
//! let tools = conn.tools();
//! let result = conn.call_tool("echo", serde_json::json!({"text": "hi"})).await?;
//! conn.close().await;
//! ```

use crate::mcp::handler::{McpEvent, PatinaClientHandler};
use crate::mcp::security::validate_mcp_command;
use anyhow::{anyhow, Context, Result};
use rmcp::model::{CallToolRequestParams, CallToolResult, Tool};
use rmcp::service::{RoleClient, RunningService, ServiceExt};
use rmcp::transport::TokioChildProcess;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::mpsc;

/// A connection to a single MCP server via the rmcp SDK.
///
/// Wraps `RunningService<RoleClient, PatinaClientHandler>` with a shared
/// tool list and event channel. The tool list is updated automatically
/// when the server sends `notifications/tools/list_changed`.
pub struct McpConnection {
    /// Human-readable server name.
    name: String,
    /// The running SDK service. `None` after `close()`.
    client: Option<RunningService<RoleClient, PatinaClientHandler>>,
    /// Shared tool list, kept in sync by the handler.
    tools: Arc<RwLock<Vec<Tool>>>,
    /// Receiver for events from the handler.
    event_rx: mpsc::UnboundedReceiver<McpEvent>,
}

impl McpConnection {
    /// Connects to an MCP server via stdio (child process).
    ///
    /// Validates the command against security policies, spawns the process,
    /// performs the MCP handshake, and fetches the initial tool list.
    ///
    /// # Arguments
    ///
    /// * `name` - Server name for identification
    /// * `command` - Command to spawn (should be absolute path for interpreters)
    /// * `args` - Arguments for the command
    /// * `env` - Environment variables for the child process
    /// * `timeout` - Maximum time for connection + initialization
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Command validation fails (security policy)
    /// - The child process cannot be spawned
    /// - The MCP handshake fails or times out
    pub async fn connect_stdio(
        name: &str,
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
        timeout: Duration,
    ) -> Result<Self> {
        // Validate command security
        validate_mcp_command(command, args)?;

        // Build the tokio Command.
        // Redirect stderr to null to prevent child process output (e.g. MCP
        // server logs) from corrupting the TUI alternate screen.
        let mut cmd = tokio::process::Command::new(command);
        cmd.args(args);
        cmd.envs(env);
        cmd.stderr(std::process::Stdio::null());

        // Create transport
        let transport =
            TokioChildProcess::new(cmd).context("Failed to spawn MCP server process")?;

        // Create shared state
        let tools: Arc<RwLock<Vec<Tool>>> = Arc::new(RwLock::new(Vec::new()));
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let handler = PatinaClientHandler::new(name.to_string(), Arc::clone(&tools), event_tx);

        // Connect with timeout
        let client = tokio::time::timeout(timeout, handler.serve(transport))
            .await
            .map_err(|_| anyhow!("MCP server '{}' connection timed out", name))?
            .map_err(|e| anyhow!("MCP server '{}' initialization failed: {}", name, e))?;

        // Fetch initial tool list
        match client.list_all_tools().await {
            Ok(initial_tools) => {
                if let Ok(mut guard) = tools.write() {
                    *guard = initial_tools;
                }
            }
            Err(e) => {
                tracing::warn!(
                    server = %name,
                    error = %e,
                    "Failed to fetch initial tool list"
                );
            }
        }

        Ok(Self {
            name: name.to_string(),
            client: Some(client),
            tools,
            event_rx,
        })
    }

    /// Connects to an MCP server via legacy SSE transport.
    ///
    /// Uses the legacy SSE protocol where the client GETs an SSE endpoint,
    /// receives a POST URL via an `endpoint` event, then sends JSON-RPC
    /// messages via POST while receiving responses on the SSE stream.
    ///
    /// # Arguments
    ///
    /// * `name` - Server name for identification
    /// * `url` - URL of the SSE endpoint
    /// * `headers` - Optional HTTP headers (e.g., Authorization)
    /// * `timeout` - Maximum time for connection + initialization
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The SSE connection fails
    /// - The `endpoint` event is not received in time
    /// - The MCP handshake fails or times out
    pub async fn connect_legacy_sse(
        name: &str,
        url: &str,
        headers: &Option<HashMap<String, String>>,
        timeout: Duration,
    ) -> Result<Self> {
        use crate::mcp::legacy_sse::LegacySseTransport;

        let transport = LegacySseTransport::connect(url, headers, timeout)
            .await
            .with_context(|| format!("Legacy SSE connection to '{}' failed", name))?;

        let tools: Arc<RwLock<Vec<Tool>>> = Arc::new(RwLock::new(Vec::new()));
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let handler = PatinaClientHandler::new(name.to_string(), Arc::clone(&tools), event_tx);

        let client = tokio::time::timeout(timeout, handler.serve(transport))
            .await
            .map_err(|_| anyhow!("MCP legacy SSE server '{}' handshake timed out", name))?
            .map_err(|e| {
                anyhow!(
                    "MCP legacy SSE server '{}' initialization failed: {}",
                    name,
                    e
                )
            })?;

        // Fetch initial tool list
        match client.list_all_tools().await {
            Ok(initial_tools) => {
                if let Ok(mut guard) = tools.write() {
                    *guard = initial_tools;
                }
            }
            Err(e) => {
                tracing::warn!(
                    server = %name,
                    error = %e,
                    "Failed to fetch initial tool list from legacy SSE server"
                );
            }
        }

        Ok(Self {
            name: name.to_string(),
            client: Some(client),
            tools,
            event_rx,
        })
    }

    /// Connects to an MCP server via HTTP (streamable HTTP / SSE).
    ///
    /// # Arguments
    ///
    /// * `name` - Server name for identification
    /// * `url` - URL of the HTTP endpoint
    /// * `headers` - Optional HTTP headers (e.g., Authorization)
    /// * `timeout` - Maximum time for connection + initialization
    ///
    /// # Errors
    ///
    /// Returns an error if the connection or MCP handshake fails.
    pub async fn connect_http(
        name: &str,
        url: &str,
        headers: &Option<HashMap<String, String>>,
        timeout: Duration,
    ) -> Result<Self> {
        use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
        use rmcp::transport::StreamableHttpClientTransport;

        let mut config = StreamableHttpClientTransportConfig::with_uri(url);

        if let Some(hdrs) = headers {
            for (key, value) in hdrs {
                if let (Ok(header_name), Ok(header_value)) = (
                    key.parse::<reqwest::header::HeaderName>(),
                    value.parse::<reqwest::header::HeaderValue>(),
                ) {
                    config.custom_headers.insert(header_name, header_value);
                }
            }
        }

        let transport = StreamableHttpClientTransport::from_config(config);

        let tools: Arc<RwLock<Vec<Tool>>> = Arc::new(RwLock::new(Vec::new()));
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let handler = PatinaClientHandler::new(name.to_string(), Arc::clone(&tools), event_tx);

        let client = tokio::time::timeout(timeout, handler.serve(transport))
            .await
            .map_err(|_| anyhow!("MCP HTTP server '{}' connection timed out", name))?
            .map_err(|e| anyhow!("MCP HTTP server '{}' initialization failed: {}", name, e))?;

        // Fetch initial tool list
        match client.list_all_tools().await {
            Ok(initial_tools) => {
                if let Ok(mut guard) = tools.write() {
                    *guard = initial_tools;
                }
            }
            Err(e) => {
                tracing::warn!(
                    server = %name,
                    error = %e,
                    "Failed to fetch initial tool list from HTTP server"
                );
            }
        }

        Ok(Self {
            name: name.to_string(),
            client: Some(client),
            tools,
            event_rx,
        })
    }

    /// Returns the server name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the current tool list (snapshot).
    ///
    /// The list is kept up-to-date by the handler when the server sends
    /// `notifications/tools/list_changed`.
    #[must_use]
    pub fn tools(&self) -> Vec<Tool> {
        self.tools
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// Returns a reference to the shared tool list Arc.
    #[must_use]
    pub fn tools_arc(&self) -> &Arc<RwLock<Vec<Tool>>> {
        &self.tools
    }

    /// Returns whether the connection is still alive.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.client.as_ref().is_some_and(|c| !c.is_closed())
    }

    /// Calls a tool on the server.
    ///
    /// # Arguments
    ///
    /// * `name` - Tool name (not namespaced)
    /// * `arguments` - Tool arguments as JSON
    ///
    /// # Errors
    ///
    /// Returns an error if the connection is closed or the call fails.
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<CallToolResult> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow!("Connection closed"))?;

        let json_object = match arguments {
            serde_json::Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };

        let params = CallToolRequestParams {
            name: Cow::Owned(name.to_string()),
            arguments: Some(json_object),
            meta: None,
            task: None,
        };

        client
            .call_tool(params)
            .await
            .map_err(|e| anyhow!("Tool call '{}' failed: {}", name, e))
    }

    /// Calls a tool and returns the result as a raw JSON value.
    ///
    /// This is a convenience wrapper around [`call_tool`] that serializes the
    /// SDK's [`CallToolResult`] to `serde_json::Value`, matching the JSON-RPC
    /// result format expected by the compression orchestrator and CCG backend.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection is closed, the call fails, or the
    /// result cannot be serialized.
    pub async fn call_tool_json(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let result = self.call_tool(name, arguments).await?;
        serde_json::to_value(&result).context("Failed to serialize CallToolResult")
    }

    /// Drains all pending events from the handler channel.
    ///
    /// Returns immediately with whatever events are buffered. Non-blocking.
    pub fn drain_events(&mut self) -> Vec<McpEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            events.push(event);
        }
        events
    }

    /// Gracefully closes the connection with a timeout.
    ///
    /// If the server doesn't respond to the shutdown within 5 seconds,
    /// the connection is cancelled.
    pub async fn close(&mut self) {
        if let Some(mut client) = self.client.take() {
            match client.close_with_timeout(Duration::from_secs(5)).await {
                Ok(_) => {
                    tracing::debug!(server = %self.name, "MCP connection closed gracefully");
                }
                Err(e) => {
                    tracing::warn!(
                        server = %self.name,
                        error = %e,
                        "Error closing MCP connection"
                    );
                }
            }
        }
    }

    /// Creates a disconnected `McpConnection` for testing purposes.
    ///
    /// The returned connection has no active client and will return errors
    /// for any tool calls. Useful when `build_context` needs a client reference
    /// but all layers are expected to come from cache.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let client = McpConnection::disconnected("test");
    /// assert!(!client.is_connected());
    /// ```
    #[cfg(test)]
    #[must_use]
    pub fn disconnected(name: &str) -> Self {
        let (_event_tx, event_rx) = mpsc::unbounded_channel();
        Self {
            name: name.to_string(),
            client: None,
            tools: Arc::new(RwLock::new(Vec::new())),
            event_rx,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_connect_stdio_invalid_command_fails() {
        let result = McpConnection::connect_stdio(
            "bad",
            "/nonexistent/binary/that/does/not/exist",
            &[],
            &HashMap::new(),
            Duration::from_secs(2),
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_connect_stdio_blocked_command_fails() {
        let result = McpConnection::connect_stdio(
            "blocked",
            "rm",
            &[],
            &HashMap::new(),
            Duration::from_secs(2),
        )
        .await;
        assert!(result.is_err());
        let err = result.as_ref().err().unwrap().to_string().to_lowercase();
        assert!(err.contains("blocked"));
    }

    #[tokio::test]
    async fn test_connect_stdio_discovers_tools() {
        // Build mock_mcp_server path
        let mock_path = std::env::current_dir()
            .unwrap()
            .join("target")
            .join("debug")
            .join("mock_mcp_server");

        if !mock_path.exists() {
            eprintln!("Skipping: mock_mcp_server not built. Run `cargo build` first.");
            return;
        }

        let result = McpConnection::connect_stdio(
            "mock",
            mock_path.to_str().unwrap(),
            &[],
            &HashMap::new(),
            Duration::from_secs(5),
        )
        .await;

        match result {
            Ok(mut conn) => {
                assert!(conn.is_connected());
                let tools = conn.tools();
                assert!(!tools.is_empty(), "Should discover at least one tool");

                // Verify echo tool exists
                let has_echo = tools.iter().any(|t| t.name.as_ref() == "echo");
                assert!(has_echo, "Should find 'echo' tool");

                conn.close().await;
            }
            Err(e) => {
                panic!("Failed to connect to mock server: {e}");
            }
        }
    }

    #[tokio::test]
    async fn test_connect_stdio_call_tool() {
        let mock_path = std::env::current_dir()
            .unwrap()
            .join("target")
            .join("debug")
            .join("mock_mcp_server");

        if !mock_path.exists() {
            eprintln!("Skipping: mock_mcp_server not built");
            return;
        }

        let mut conn = McpConnection::connect_stdio(
            "mock",
            mock_path.to_str().unwrap(),
            &[],
            &HashMap::new(),
            Duration::from_secs(5),
        )
        .await
        .expect("Should connect");

        let result = conn
            .call_tool("echo", serde_json::json!({"text": "hello world"}))
            .await;
        assert!(
            result.is_ok(),
            "Tool call should succeed: {:?}",
            result.err()
        );

        let call_result = result.unwrap();
        // Check content has our text
        let has_text = call_result.content.iter().any(|c| {
            if let Some(text) = c.raw.as_text() {
                text.text.contains("hello world")
            } else {
                false
            }
        });
        assert!(has_text, "Response should contain 'hello world'");

        conn.close().await;
    }

    #[tokio::test]
    async fn test_close_graceful() {
        let mock_path = std::env::current_dir()
            .unwrap()
            .join("target")
            .join("debug")
            .join("mock_mcp_server");

        if !mock_path.exists() {
            return;
        }

        let mut conn = McpConnection::connect_stdio(
            "mock",
            mock_path.to_str().unwrap(),
            &[],
            &HashMap::new(),
            Duration::from_secs(5),
        )
        .await
        .unwrap();

        conn.close().await;
        assert!(!conn.is_connected());
    }

    #[tokio::test]
    async fn test_connect_stdio_timeout() {
        let mock_path = std::env::current_dir()
            .unwrap()
            .join("target")
            .join("debug")
            .join("mock_mcp_server");

        if !mock_path.exists() {
            return;
        }

        // Use --no-response flag so server never responds → should timeout
        let result = McpConnection::connect_stdio(
            "slow",
            mock_path.to_str().unwrap(),
            &["--no-response".to_string()],
            &HashMap::new(),
            Duration::from_secs(2),
        )
        .await;

        assert!(result.is_err(), "Should timeout");
        let err = result.as_ref().err().unwrap().to_string();
        assert!(
            err.contains("timed out") || err.contains("initialization failed"),
            "Error should indicate timeout: {err}"
        );
    }
}
