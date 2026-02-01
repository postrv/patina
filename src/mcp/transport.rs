//! MCP transport implementations.
//!
//! This module provides transport layers for MCP communication:
//! - `StdioTransport`: Communication via stdin/stdout with a child process
//! - `SseTransport`: Communication via HTTP Server-Sent Events
//!
//! # Example
//!
//! ```ignore
//! use patina::mcp::transport::{StdioTransport, Transport};
//! use patina::mcp::protocol::JsonRpcRequest;
//! use serde_json::json;
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let mut transport = StdioTransport::new("mcp-server", vec![]);
//!     transport.start().await?;
//!
//!     let request = JsonRpcRequest::new(1, "initialize", json!({}));
//!     let response = transport.send_request(request, Duration::from_secs(5)).await?;
//!
//!     transport.stop().await?;
//!     Ok(())
//! }
//! ```

use crate::mcp::protocol::{JsonRpcRequest, JsonRpcResponse};
use anyhow::{anyhow, Context, Result};
use std::collections::HashMap;
use std::future::Future;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, Mutex};

/// Warmup delay after spawning a child process.
///
/// This small delay ensures the child process has time to:
/// 1. Initialize its runtime
/// 2. Set up stdin/stdout pipes
/// 3. Enter its main read loop
///
/// Without this delay, requests sent immediately after spawn may arrive
/// before the child is ready to read, causing timeouts.
const SPAWN_WARMUP_MS: u64 = 50;

/// Transport trait for MCP communication.
///
/// Implementations of this trait provide the communication layer
/// between the MCP client and server.
///
/// The trait methods return `impl Future + Send` to ensure compatibility
/// with async runtimes that require Send futures.
pub trait Transport {
    /// Starts the transport connection.
    ///
    /// # Errors
    ///
    /// Returns an error if the transport cannot be started.
    fn start(&mut self) -> impl Future<Output = Result<()>> + Send;

    /// Stops the transport connection.
    ///
    /// # Errors
    ///
    /// Returns an error if the transport cannot be cleanly stopped.
    fn stop(&mut self) -> impl Future<Output = Result<()>> + Send;

    /// Sends a request and waits for a response.
    ///
    /// # Arguments
    ///
    /// * `request` - The JSON-RPC request to send
    /// * `timeout` - Maximum time to wait for a response
    ///
    /// # Errors
    ///
    /// Returns an error if the request cannot be sent or times out.
    fn send_request(
        &mut self,
        request: JsonRpcRequest,
        timeout: Duration,
    ) -> impl Future<Output = Result<JsonRpcResponse>> + Send;

    /// Sends a notification (no response expected).
    ///
    /// # Arguments
    ///
    /// * `notification` - The JSON-RPC notification to send
    ///
    /// # Errors
    ///
    /// Returns an error if the notification cannot be sent.
    fn send_notification(
        &mut self,
        notification: JsonRpcRequest,
    ) -> impl Future<Output = Result<()>> + Send;
}

/// Message sent to the writer task.
enum WriterMessage {
    /// Send data to the server (request or notification)
    Send { data: String },
    /// Stop the writer task
    Stop,
}

/// Stdio transport for MCP servers.
///
/// Communicates with an MCP server via stdin/stdout using
/// newline-delimited JSON-RPC messages.
pub struct StdioTransport {
    command: String,
    args: Vec<String>,
    child: Option<Child>,
    writer_tx: Option<mpsc::Sender<WriterMessage>>,
    pending_requests: Arc<Mutex<HashMap<String, oneshot::Sender<Result<JsonRpcResponse>>>>>,
}

impl StdioTransport {
    /// Creates a new stdio transport.
    ///
    /// # Arguments
    ///
    /// * `command` - The command to spawn
    /// * `args` - Arguments to pass to the command
    #[must_use]
    pub fn new<S: Into<String>>(command: S, args: Vec<&str>) -> Self {
        Self {
            command: command.into(),
            args: args.into_iter().map(String::from).collect(),
            child: None,
            writer_tx: None,
            pending_requests: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Spawns the child process and starts I/O tasks.
    async fn spawn_and_start(&mut self) -> Result<()> {
        let mut child = Command::new(&self.command)
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("Failed to spawn command: {}", self.command))?;

        let stdin = child.stdin.take().ok_or_else(|| anyhow!("No stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("No stdout"))?;

        self.child = Some(child);

        // Create channel for writer task
        let (writer_tx, mut writer_rx) = mpsc::channel::<WriterMessage>(32);
        self.writer_tx = Some(writer_tx);

        // Clone pending_requests for the reader task
        let pending_requests = Arc::clone(&self.pending_requests);

        // Spawn writer task
        let mut stdin = stdin;
        tokio::spawn(async move {
            while let Some(msg) = writer_rx.recv().await {
                match msg {
                    WriterMessage::Send { data } => {
                        if let Err(e) = stdin.write_all(data.as_bytes()).await {
                            tracing::error!("Failed to write to stdin: {e}");
                            break;
                        }
                        if let Err(e) = stdin.write_all(b"\n").await {
                            tracing::error!("Failed to write newline: {e}");
                            break;
                        }
                        if let Err(e) = stdin.flush().await {
                            tracing::error!("Failed to flush stdin: {e}");
                            break;
                        }
                    }
                    WriterMessage::Stop => break,
                }
            }
        });

        // Spawn reader task
        let mut reader = BufReader::new(stdout);
        tokio::spawn(async move {
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break, // EOF
                    Ok(_) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }

                        match serde_json::from_str::<JsonRpcResponse>(trimmed) {
                            Ok(response) => {
                                // Find the pending request by ID
                                if let Some(id) = response.id() {
                                    let id_str = id.to_string();
                                    let mut pending = pending_requests.lock().await;
                                    if let Some(tx) = pending.remove(&id_str) {
                                        let _ = tx.send(Ok(response));
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Failed to parse response: {e}, line: {trimmed}");
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to read from stdout: {e}");
                        break;
                    }
                }
            }
        });

        // Brief warmup delay to ensure child process is ready to receive messages
        tokio::time::sleep(Duration::from_millis(SPAWN_WARMUP_MS)).await;

        Ok(())
    }
}

impl Transport for StdioTransport {
    async fn start(&mut self) -> Result<()> {
        if self.child.is_some() {
            return Err(anyhow!("Transport already started"));
        }
        self.spawn_and_start().await
    }

    async fn stop(&mut self) -> Result<()> {
        // Send stop message to writer
        if let Some(tx) = self.writer_tx.take() {
            let _ = tx.send(WriterMessage::Stop).await;
        }

        // Kill the child process
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }

        // Clear pending requests
        let mut pending = self.pending_requests.lock().await;
        for (_, tx) in pending.drain() {
            let _ = tx.send(Err(anyhow!("Transport stopped")));
        }

        Ok(())
    }

    async fn send_request(
        &mut self,
        request: JsonRpcRequest,
        timeout: Duration,
    ) -> Result<JsonRpcResponse> {
        let tx = self
            .writer_tx
            .as_ref()
            .ok_or_else(|| anyhow!("Transport not started"))?;

        // Get the request ID for correlation
        let id = request.id().ok_or_else(|| anyhow!("Request has no ID"))?;
        let id_value = match id {
            crate::mcp::protocol::RequestId::Number(n) => serde_json::json!(n),
            crate::mcp::protocol::RequestId::String(s) => serde_json::json!(s),
        };
        let id_str = id_value.to_string();

        // Create response channel
        let (response_tx, response_rx) = oneshot::channel();

        // Register pending request
        {
            let mut pending = self.pending_requests.lock().await;
            pending.insert(id_str, response_tx);
        }

        // Serialize request
        let data = serde_json::to_string(&request).context("Failed to serialize request")?;

        // Send to writer
        tx.send(WriterMessage::Send { data })
            .await
            .map_err(|_| anyhow!("Writer task closed"))?;

        // Wait for response with timeout
        match tokio::time::timeout(timeout, response_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(anyhow!("Response channel closed")),
            Err(_) => Err(anyhow!("Timeout waiting for response")),
        }
    }

    async fn send_notification(&mut self, notification: JsonRpcRequest) -> Result<()> {
        let tx = self
            .writer_tx
            .as_ref()
            .ok_or_else(|| anyhow!("Transport not started"))?;

        let data =
            serde_json::to_string(&notification).context("Failed to serialize notification")?;

        tx.send(WriterMessage::Send { data })
            .await
            .map_err(|_| anyhow!("Writer task closed"))
    }
}

/// SSE (Server-Sent Events) transport for MCP servers.
///
/// Communicates with an MCP server via HTTP:
/// - Connects to an SSE endpoint to receive server events
/// - Sends JSON-RPC requests via HTTP POST to the message endpoint
///
/// The MCP SSE protocol works as follows:
/// 1. Client connects to SSE endpoint (GET request with Accept: text/event-stream)
/// 2. Server sends an `endpoint` event containing the POST URL for messages
/// 3. Client sends JSON-RPC requests via POST to the message endpoint
/// 4. Server responds with JSON-RPC responses in the POST response body
pub struct SseTransport {
    sse_url: String,
    headers: HashMap<String, String>,
    client: reqwest::Client,
    message_endpoint: Option<String>,
    connected: bool,
}

impl SseTransport {
    /// Creates a new SSE transport.
    ///
    /// # Arguments
    ///
    /// * `sse_url` - The URL of the SSE endpoint
    #[must_use]
    pub fn new(sse_url: &str) -> Self {
        Self {
            sse_url: sse_url.to_string(),
            headers: HashMap::new(),
            client: reqwest::Client::new(),
            message_endpoint: None,
            connected: false,
        }
    }

    /// Creates a new SSE transport with custom headers.
    ///
    /// # Arguments
    ///
    /// * `sse_url` - The URL of the SSE endpoint
    /// * `headers` - Custom headers to include in all requests
    #[must_use]
    pub fn with_headers(sse_url: &str, headers: HashMap<String, String>) -> Self {
        Self {
            sse_url: sse_url.to_string(),
            headers,
            client: reqwest::Client::new(),
            message_endpoint: None,
            connected: false,
        }
    }

    /// Returns whether the transport is connected.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// Connects to the SSE endpoint and extracts the message endpoint URL.
    async fn connect_sse(&mut self) -> Result<()> {
        let mut request = self
            .client
            .get(&self.sse_url)
            .header("Accept", "text/event-stream")
            .header("Cache-Control", "no-cache");

        // Add custom headers
        for (key, value) in &self.headers {
            request = request.header(key, value);
        }

        let response = request
            .send()
            .await
            .with_context(|| format!("Failed to connect to SSE endpoint: {}", self.sse_url))?;

        if !response.status().is_success() {
            return Err(anyhow!(
                "SSE endpoint returned error status: {}",
                response.status()
            ));
        }

        // Read the SSE response to get the endpoint event
        let body = response
            .text()
            .await
            .context("Failed to read SSE response body")?;

        // Parse SSE events to find the endpoint
        // Format: event: endpoint\ndata: <url>\n\n
        let message_endpoint = self.parse_endpoint_from_sse(&body)?;

        self.message_endpoint = Some(message_endpoint);
        self.connected = true;

        Ok(())
    }

    /// Parses the endpoint URL from SSE event data.
    fn parse_endpoint_from_sse(&self, body: &str) -> Result<String> {
        let mut event_type: Option<&str> = None;
        let mut data: Option<&str> = None;

        for line in body.lines() {
            if let Some(stripped) = line.strip_prefix("event:") {
                event_type = Some(stripped.trim());
            } else if let Some(stripped) = line.strip_prefix("data:") {
                data = Some(stripped.trim());
            }
        }

        // If we found an endpoint event, extract the URL
        if event_type == Some("endpoint") {
            if let Some(endpoint_url) = data {
                // The endpoint might be relative or absolute
                // If relative, combine with SSE URL base
                let endpoint = if endpoint_url.starts_with("http://")
                    || endpoint_url.starts_with("https://")
                {
                    endpoint_url.to_string()
                } else {
                    // Build absolute URL from SSE URL
                    let base_url = self
                        .sse_url
                        .rsplit_once('/')
                        .map_or(&self.sse_url[..], |(base, _)| base);
                    format!(
                        "{}/{}",
                        base_url.trim_end_matches('/'),
                        endpoint_url.trim_start_matches('/')
                    )
                };
                return Ok(endpoint);
            }
        }

        // Default to /message relative to SSE URL if no endpoint event
        let base_url = self
            .sse_url
            .rsplit_once('/')
            .map_or(&self.sse_url[..], |(base, _)| base);
        Ok(format!("{}/message", base_url.trim_end_matches('/')))
    }

    /// Sends a JSON-RPC message via HTTP POST.
    async fn post_message(&self, body: &str) -> Result<String> {
        let endpoint = self
            .message_endpoint
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected - no message endpoint"))?;

        let mut request = self
            .client
            .post(endpoint)
            .header("Content-Type", "application/json")
            .body(body.to_string());

        // Add custom headers
        for (key, value) in &self.headers {
            request = request.header(key, value);
        }

        let response = request
            .send()
            .await
            .with_context(|| format!("Failed to POST to message endpoint: {endpoint}"))?;

        if !response.status().is_success() {
            return Err(anyhow!(
                "Message endpoint returned error status: {}",
                response.status()
            ));
        }

        response
            .text()
            .await
            .context("Failed to read response body")
    }
}

impl Transport for SseTransport {
    async fn start(&mut self) -> Result<()> {
        if self.connected {
            return Err(anyhow!("Transport already connected"));
        }
        self.connect_sse().await
    }

    async fn stop(&mut self) -> Result<()> {
        self.connected = false;
        self.message_endpoint = None;
        Ok(())
    }

    async fn send_request(
        &mut self,
        request: JsonRpcRequest,
        timeout: Duration,
    ) -> Result<JsonRpcResponse> {
        if !self.connected {
            return Err(anyhow!("Transport not connected"));
        }

        let body = serde_json::to_string(&request).context("Failed to serialize request")?;

        let response_text = tokio::time::timeout(timeout, self.post_message(&body))
            .await
            .map_err(|_| anyhow!("Timeout waiting for response"))?
            .context("Failed to send request")?;

        serde_json::from_str(&response_text).context("Failed to parse JSON-RPC response")
    }

    async fn send_notification(&mut self, notification: JsonRpcRequest) -> Result<()> {
        if !self.connected {
            return Err(anyhow!("Transport not connected"));
        }

        let body =
            serde_json::to_string(&notification).context("Failed to serialize notification")?;

        self.post_message(&body).await?;
        Ok(())
    }
}
