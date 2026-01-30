//! MCP client for managing server connections.
//!
//! This module provides a high-level client interface for MCP servers,
//! handling initialization, tool discovery, and tool execution.
//!
//! # Example
//!
//! ```ignore
//! use rct::mcp::client::McpClient;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let mut client = McpClient::new("my-server", "mcp-server", vec![]);
//!     client.start().await?;
//!
//!     let tools = client.list_tools().await?;
//!     println!("Available tools: {:?}", tools);
//!
//!     client.stop().await?;
//!     Ok(())
//! }
//! ```

use crate::mcp::protocol::{JsonRpcRequest, JsonRpcResponse};
use crate::mcp::transport::{StdioTransport, Transport};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Duration;

/// Default timeout for MCP requests.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// MCP tool definition from tools/list response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    /// Unique tool name
    pub name: String,
    /// Human-readable description
    #[serde(default)]
    pub description: String,
    /// JSON Schema for tool input
    #[serde(rename = "inputSchema")]
    pub input_schema: serde_json::Value,
}

/// MCP server capabilities from initialize response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerCapabilities {
    /// Tool-related capabilities
    #[serde(default)]
    pub tools: Option<serde_json::Value>,
    /// Resource-related capabilities
    #[serde(default)]
    pub resources: Option<serde_json::Value>,
    /// Prompt-related capabilities
    #[serde(default)]
    pub prompts: Option<serde_json::Value>,
}

/// MCP server information from initialize response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerInfo {
    /// Server name
    pub name: String,
    /// Server version
    #[serde(default)]
    pub version: String,
}

/// High-level MCP client for server management.
///
/// Manages the connection lifecycle and provides methods for
/// tool discovery and execution.
pub struct McpClient {
    /// Server name for identification
    name: String,
    /// Underlying transport
    transport: StdioTransport,
    /// Whether the transport is connected
    connected: AtomicBool,
    /// Whether the server has been initialized
    initialized: AtomicBool,
    /// Request ID counter
    request_id: AtomicI64,
    /// Server capabilities (set after initialization)
    capabilities: Option<ServerCapabilities>,
    /// Server info (set after initialization)
    server_info: Option<ServerInfo>,
}

impl McpClient {
    /// Creates a new MCP client.
    ///
    /// # Arguments
    ///
    /// * `name` - Name to identify this server connection
    /// * `command` - Command to spawn the MCP server
    /// * `args` - Arguments for the command
    #[must_use]
    pub fn new<S: Into<String>>(name: S, command: &str, args: Vec<&str>) -> Self {
        Self {
            name: name.into(),
            transport: StdioTransport::new(command, args),
            connected: AtomicBool::new(false),
            initialized: AtomicBool::new(false),
            request_id: AtomicI64::new(1),
            capabilities: None,
            server_info: None,
        }
    }

    /// Returns the server name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns whether the client is connected to the server.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    /// Returns whether the server has been initialized.
    #[must_use]
    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::SeqCst)
    }

    /// Returns the server capabilities, if initialized.
    #[must_use]
    pub fn capabilities(&self) -> Option<&ServerCapabilities> {
        self.capabilities.as_ref()
    }

    /// Returns the server info, if initialized.
    #[must_use]
    pub fn server_info(&self) -> Option<&ServerInfo> {
        self.server_info.as_ref()
    }

    /// Gets the next request ID.
    fn next_request_id(&self) -> i64 {
        self.request_id.fetch_add(1, Ordering::SeqCst)
    }

    /// Starts the MCP server and performs initialization.
    ///
    /// # Errors
    ///
    /// Returns an error if the server cannot be started or initialized.
    pub async fn start(&mut self) -> Result<()> {
        // Start the transport
        self.transport
            .start()
            .await
            .context("Failed to start MCP transport")?;

        self.connected.store(true, Ordering::SeqCst);

        // Perform MCP initialization
        self.initialize().await?;

        Ok(())
    }

    /// Performs the MCP initialization handshake.
    async fn initialize(&mut self) -> Result<()> {
        let request = JsonRpcRequest::new(
            self.next_request_id(),
            "initialize",
            serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "rct",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        );

        let response = self
            .transport
            .send_request(request, DEFAULT_TIMEOUT)
            .await
            .context("Failed to send initialize request")?;

        if response.is_error() {
            let error = response.error().unwrap();
            return Err(anyhow!(
                "Server initialization failed: {} ({})",
                error.message(),
                error.code()
            ));
        }

        // Parse capabilities and server info
        if let Some(result) = response.result() {
            if let Some(caps) = result.get("capabilities") {
                self.capabilities = serde_json::from_value(caps.clone()).ok();
            }
            if let Some(info) = result.get("serverInfo") {
                self.server_info = serde_json::from_value(info.clone()).ok();
            }
        }

        // Send initialized notification
        let notification = JsonRpcRequest::notification("initialized", serde_json::json!({}));
        self.transport
            .send_notification(notification)
            .await
            .context("Failed to send initialized notification")?;

        self.initialized.store(true, Ordering::SeqCst);

        Ok(())
    }

    /// Stops the MCP server cleanly.
    ///
    /// # Errors
    ///
    /// Returns an error if the server cannot be stopped cleanly.
    pub async fn stop(&mut self) -> Result<()> {
        if self.is_connected() {
            self.transport
                .stop()
                .await
                .context("Failed to stop MCP transport")?;

            self.connected.store(false, Ordering::SeqCst);
            self.initialized.store(false, Ordering::SeqCst);
        }

        Ok(())
    }

    /// Forcefully stops the server without clean shutdown.
    pub async fn force_stop(&mut self) {
        let _ = self.transport.stop().await;
        self.connected.store(false, Ordering::SeqCst);
        self.initialized.store(false, Ordering::SeqCst);
    }

    /// Lists available tools from the MCP server.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or response is invalid.
    pub async fn list_tools(&mut self) -> Result<Vec<McpTool>> {
        if !self.is_connected() {
            return Err(anyhow!("Not connected to server"));
        }

        let request =
            JsonRpcRequest::new(self.next_request_id(), "tools/list", serde_json::json!({}));

        let response = self
            .transport
            .send_request(request, DEFAULT_TIMEOUT)
            .await
            .context("Failed to send tools/list request")?;

        if response.is_error() {
            let error = response.error().unwrap();
            return Err(anyhow!(
                "tools/list failed: {} ({})",
                error.message(),
                error.code()
            ));
        }

        let result = response.result().ok_or_else(|| anyhow!("No result"))?;
        let tools_value = result
            .get("tools")
            .ok_or_else(|| anyhow!("No tools field"))?;
        let tools: Vec<McpTool> =
            serde_json::from_value(tools_value.clone()).context("Failed to parse tools")?;

        Ok(tools)
    }

    /// Calls a tool on the MCP server.
    ///
    /// # Arguments
    ///
    /// * `name` - Name of the tool to call
    /// * `arguments` - Arguments to pass to the tool
    ///
    /// # Errors
    ///
    /// Returns an error if the call fails.
    pub async fn call_tool(
        &mut self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value> {
        if !self.is_connected() {
            return Err(anyhow!("Not connected to server"));
        }

        let request = JsonRpcRequest::new(
            self.next_request_id(),
            "tools/call",
            serde_json::json!({
                "name": name,
                "arguments": arguments
            }),
        );

        let response = self
            .transport
            .send_request(request, DEFAULT_TIMEOUT)
            .await
            .context("Failed to send tools/call request")?;

        if response.is_error() {
            let error = response.error().unwrap();
            return Err(anyhow!(
                "tools/call failed: {} ({})",
                error.message(),
                error.code()
            ));
        }

        response
            .result()
            .cloned()
            .ok_or_else(|| anyhow!("No result from tool call"))
    }

    /// Sends a raw JSON-RPC request to the server.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn send_request(&mut self, request: JsonRpcRequest) -> Result<JsonRpcResponse> {
        if !self.is_connected() {
            return Err(anyhow!("Not connected to server"));
        }

        self.transport
            .send_request(request, DEFAULT_TIMEOUT)
            .await
            .context("Failed to send request")
    }
}
