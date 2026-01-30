//! MCP transport implementations.
//!
//! This module provides transport layers for MCP communication:
//! - `StdioTransport`: Communication via stdin/stdout with a child process
//!
//! # Example
//!
//! ```ignore
//! use rct::mcp::transport::{StdioTransport, Transport};
//! use rct::mcp::protocol::JsonRpcRequest;
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
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, Mutex};

/// Transport trait for MCP communication.
///
/// Implementations of this trait provide the communication layer
/// between the MCP client and server.
#[allow(async_fn_in_trait)]
pub trait Transport {
    /// Starts the transport connection.
    ///
    /// # Errors
    ///
    /// Returns an error if the transport cannot be started.
    async fn start(&mut self) -> Result<()>;

    /// Stops the transport connection.
    ///
    /// # Errors
    ///
    /// Returns an error if the transport cannot be cleanly stopped.
    async fn stop(&mut self) -> Result<()>;

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
    async fn send_request(
        &mut self,
        request: JsonRpcRequest,
        timeout: Duration,
    ) -> Result<JsonRpcResponse>;

    /// Sends a notification (no response expected).
    ///
    /// # Arguments
    ///
    /// * `notification` - The JSON-RPC notification to send
    ///
    /// # Errors
    ///
    /// Returns an error if the notification cannot be sent.
    async fn send_notification(&mut self, notification: JsonRpcRequest) -> Result<()>;
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
