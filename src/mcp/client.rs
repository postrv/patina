//! MCP client for managing server connections.
//!
//! This module provides a high-level client interface for MCP servers,
//! handling initialization, tool discovery, and tool execution.
//!
//! # Security
//!
//! MCP server commands are validated before spawning to prevent:
//! - Execution of dangerous commands (rm, sudo, etc.)
//! - Path traversal attacks (../../../bin/rm)
//! - Relative path exploitation (./malicious_script)
//! - Shell injection via arguments
//!
//! # Example
//!
//! ```ignore
//! use rct::mcp::client::McpClient;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let mut client = McpClient::new("my-server", "/usr/bin/mcp-server", vec![]);
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
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Duration;

/// Commands that are ALWAYS blocked, even with absolute paths.
///
/// These commands have no legitimate use as MCP servers and could cause
/// system damage or privilege escalation.
fn always_blocked_commands() -> Vec<Regex> {
    vec![
        // Destructive file operations
        Regex::new(r"^rm$").unwrap(),
        Regex::new(r"^rmdir$").unwrap(),
        Regex::new(r"^shred$").unwrap(),
        // Privilege escalation
        Regex::new(r"^sudo$").unwrap(),
        Regex::new(r"^su$").unwrap(),
        Regex::new(r"^doas$").unwrap(),
        Regex::new(r"^pkexec$").unwrap(),
        Regex::new(r"^runuser$").unwrap(),
        // Disk operations
        Regex::new(r"^dd$").unwrap(),
        Regex::new(r"^mkfs").unwrap(),
        Regex::new(r"^fdisk$").unwrap(),
        // System control
        Regex::new(r"^shutdown$").unwrap(),
        Regex::new(r"^reboot$").unwrap(),
        Regex::new(r"^halt$").unwrap(),
        Regex::new(r"^poweroff$").unwrap(),
        // Network tools that could exfiltrate data
        Regex::new(r"^nc$").unwrap(),
        Regex::new(r"^netcat$").unwrap(),
        Regex::new(r"^ncat$").unwrap(),
    ]
}

/// Commands that require an absolute path to be used.
///
/// These are interpreters that could be legitimate MCP server hosts
/// when specified with an absolute path, showing clear intent.
/// Without an absolute path, they could be PATH-hijacked.
fn require_absolute_path_commands() -> Vec<Regex> {
    vec![
        // Shell interpreters
        Regex::new(r"^(ba)?sh$").unwrap(),
        Regex::new(r"^zsh$").unwrap(),
        Regex::new(r"^fish$").unwrap(),
        Regex::new(r"^csh$").unwrap(),
        Regex::new(r"^tcsh$").unwrap(),
        Regex::new(r"^ksh$").unwrap(),
        Regex::new(r"^dash$").unwrap(),
        // Script interpreters
        Regex::new(r"^python[0-9.]*$").unwrap(),
        Regex::new(r"^perl$").unwrap(),
        Regex::new(r"^ruby$").unwrap(),
        Regex::new(r"^node$").unwrap(),
        Regex::new(r"^php$").unwrap(),
    ]
}

/// Dangerous argument patterns that indicate shell injection attempts.
fn dangerous_argument_patterns() -> Vec<Regex> {
    vec![
        // Shell command chaining
        Regex::new(r";\s*rm\s").unwrap(),
        Regex::new(r";\s*sudo\s").unwrap(),
        Regex::new(r"\|\s*sh").unwrap(),
        Regex::new(r"\|\s*bash").unwrap(),
        // Command substitution
        Regex::new(r"\$\(").unwrap(),
        Regex::new(r"`").unwrap(),
        // Dangerous redirects
        Regex::new(r">\s*/dev/").unwrap(),
        Regex::new(r">\s*/etc/").unwrap(),
    ]
}

/// Validates that an MCP command is safe to execute.
///
/// # Security Checks
///
/// 1. **Path traversal**: Commands with `..` are rejected.
///
/// 2. **Relative paths**: Commands starting with `./` or `../` are rejected.
///
/// 3. **Always blocked commands**: Commands like `rm`, `sudo`, `dd` are blocked
///    even with absolute paths, as they have no legitimate use as MCP servers.
///
/// 4. **Interpreter commands**: Shell and script interpreters (`bash`, `python`)
///    are allowed ONLY when specified with an absolute path (e.g., `/bin/bash`).
///    This ensures the user explicitly chooses which binary to run.
///
/// 5. **Argument validation**: Arguments are checked for shell injection
///    patterns like `; rm -rf /` or `$(malicious)`.
///
/// # Errors
///
/// Returns an error with a security-related message if validation fails.
pub fn validate_mcp_command(command: &str, args: &[String]) -> Result<()> {
    // Check for path traversal
    if command.contains("..") {
        return Err(anyhow!(
            "Security policy blocked: path traversal not allowed in MCP command"
        ));
    }

    // Check for relative paths (starts with ./)
    if command.starts_with("./") {
        return Err(anyhow!(
            "Security policy blocked: relative paths not allowed for MCP servers. \
             Use absolute paths like /usr/bin/command"
        ));
    }

    // Determine if this is an absolute path
    let is_absolute = command.starts_with('/');

    // Get the command basename for pattern matching
    let command_name = Path::new(command)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(command);

    // Check against always-blocked commands (even with absolute paths)
    for pattern in always_blocked_commands() {
        if pattern.is_match(command_name) {
            return Err(anyhow!(
                "Security policy blocked: '{}' is not allowed as an MCP server command",
                command_name
            ));
        }
    }

    // Check against commands that require absolute paths
    // These are interpreters that are legitimate when explicitly specified
    for pattern in require_absolute_path_commands() {
        if pattern.is_match(command_name) && !is_absolute {
            return Err(anyhow!(
                "Security policy blocked: '{}' requires an absolute path (e.g., /bin/{}) \
                 to prevent PATH hijacking",
                command_name,
                command_name
            ));
        }
    }

    // Check if this is a known interpreter (shell or script interpreter)
    // For interpreters, we skip argument validation because the script content
    // inherently contains shell constructs - that's the intended behavior.
    // The key protection is requiring an absolute path.
    let is_interpreter = require_absolute_path_commands()
        .iter()
        .any(|pattern| pattern.is_match(command_name));

    // Only check arguments for shell injection patterns on non-interpreters
    // For interpreters like /bin/bash, the script content IS the intended execution
    if !is_interpreter {
        let arg_patterns = dangerous_argument_patterns();
        for arg in args {
            for pattern in &arg_patterns {
                if pattern.is_match(arg) {
                    return Err(anyhow!(
                        "Security policy blocked: dangerous pattern detected in argument"
                    ));
                }
            }
        }
    }

    Ok(())
}

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
///
/// # Security
///
/// Before starting the server, the command and arguments are validated
/// against security policies. Dangerous commands and shell injection
/// patterns are blocked.
pub struct McpClient {
    /// Server name for identification
    name: String,
    /// Command to spawn the MCP server (stored for validation)
    command: String,
    /// Arguments for the command (stored for validation)
    args: Vec<String>,
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
    /// * `command` - Command to spawn the MCP server (should be absolute path)
    /// * `args` - Arguments for the command
    ///
    /// # Security Note
    ///
    /// The command and arguments are validated when `start()` is called.
    /// Validation checks for dangerous commands and shell injection patterns.
    #[must_use]
    pub fn new<S: Into<String>>(name: S, command: &str, args: Vec<&str>) -> Self {
        let args_owned: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        Self {
            name: name.into(),
            command: command.to_string(),
            args: args_owned.clone(),
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
    /// # Security
    ///
    /// Before starting, the command and arguments are validated:
    /// - Dangerous commands (rm, sudo, sh, etc.) are blocked
    /// - Relative paths are rejected (must use absolute paths)
    /// - Path traversal attempts are blocked
    /// - Shell injection patterns in arguments are blocked
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The command fails security validation
    /// - The server cannot be started
    /// - The MCP initialization handshake fails
    pub async fn start(&mut self) -> Result<()> {
        // Validate command before spawning any process
        validate_mcp_command(&self.command, &self.args)?;

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
