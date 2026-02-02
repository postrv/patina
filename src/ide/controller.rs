//! IDE server controller for managing connections and message routing
//!
//! This module provides the `IdeController` which manages the lifecycle of
//! the IDE integration server, handling connections and routing messages
//! to the appropriate handlers.

use super::handlers::{PromptContext, QueuedPrompt, StatusContext};
use super::protocol::{parse_request, serialize_response, IdeRequest, IdeResponse};
use anyhow::Result;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Shared state for IDE connections
#[derive(Debug, Default)]
pub struct IdeSharedState {
    /// Whether the application is currently busy
    pub busy: bool,
    /// Number of conversation turns
    pub turn_count: u32,
    /// Names of currently executing tools
    pub active_tools: Vec<String>,
    /// Pending request IDs that can be cancelled
    pub pending_requests: HashSet<String>,
}

/// Controller for the IDE integration server
pub struct IdeController {
    /// Port to listen on
    port: u16,
    /// Shared state accessible by all connections
    state: Arc<Mutex<IdeSharedState>>,
    /// Channel to send prompts to the main application
    prompt_tx: mpsc::UnboundedSender<QueuedPrompt>,
    /// Receiver for prompts (held by controller, given to main app)
    prompt_rx: Option<mpsc::UnboundedReceiver<QueuedPrompt>>,
}

impl IdeController {
    /// Creates a new IDE controller for the specified port
    #[must_use]
    pub fn new(port: u16) -> Self {
        let (prompt_tx, prompt_rx) = mpsc::unbounded_channel();
        Self {
            port,
            state: Arc::new(Mutex::new(IdeSharedState::default())),
            prompt_tx,
            prompt_rx: Some(prompt_rx),
        }
    }

    /// Takes the prompt receiver for the main application to consume
    ///
    /// This can only be called once. Subsequent calls return `None`.
    pub fn take_prompt_receiver(&mut self) -> Option<mpsc::UnboundedReceiver<QueuedPrompt>> {
        self.prompt_rx.take()
    }

    /// Returns a clone of the shared state for updating from the main app
    #[must_use]
    pub fn shared_state(&self) -> Arc<Mutex<IdeSharedState>> {
        Arc::clone(&self.state)
    }

    /// Starts the IDE server and listens for connections
    ///
    /// This function runs indefinitely, spawning a new task for each connection.
    ///
    /// # Errors
    ///
    /// Returns an error if the server fails to bind to the specified port.
    pub async fn run(&self) -> Result<()> {
        let addr = format!("127.0.0.1:{}", self.port);
        let listener = TcpListener::bind(&addr).await?;
        info!("IDE server listening on {}", addr);

        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    let session_id = Uuid::new_v4().to_string();
                    info!("IDE connection from {} (session {})", addr, session_id);

                    let state = Arc::clone(&self.state);
                    let prompt_tx = self.prompt_tx.clone();

                    tokio::spawn(async move {
                        if let Err(e) =
                            handle_connection(stream, state, prompt_tx, session_id.clone()).await
                        {
                            warn!("IDE connection {} error: {}", session_id, e);
                        }
                        debug!("IDE connection {} closed", session_id);
                    });
                }
                Err(e) => {
                    error!("Failed to accept IDE connection: {}", e);
                }
            }
        }
    }
}

/// Handles a single IDE connection
async fn handle_connection(
    mut stream: TcpStream,
    state: Arc<Mutex<IdeSharedState>>,
    prompt_tx: mpsc::UnboundedSender<QueuedPrompt>,
    session_id: String,
) -> Result<()> {
    let mut buffer = vec![0u8; 8192];

    loop {
        // Read message length (4 bytes, big-endian) followed by JSON
        // For simplicity, we use a newline-delimited protocol here
        let n = stream.read(&mut buffer).await?;
        if n == 0 {
            break; // Connection closed
        }

        // Find complete messages (newline-delimited JSON)
        let data = &buffer[..n];
        for line in data.split(|&b| b == b'\n') {
            if line.is_empty() {
                continue;
            }

            let response = match parse_request(line) {
                Ok(request) => {
                    debug!("IDE request: {:?}", request);
                    process_request(request, &state, &prompt_tx, &session_id).await
                }
                Err(e) => {
                    warn!("Failed to parse IDE request: {}", e);
                    IdeResponse::Error {
                        code: "PARSE_ERROR".to_string(),
                        message: format!("Failed to parse request: {}", e),
                        request_id: None,
                    }
                }
            };

            // Send response
            let response_bytes = serialize_response(&response)?;
            stream.write_all(&response_bytes).await?;
            stream.write_all(b"\n").await?;
            stream.flush().await?;
        }
    }

    Ok(())
}

/// Processes a single IDE request and returns the response
async fn process_request(
    request: IdeRequest,
    state: &Arc<Mutex<IdeSharedState>>,
    prompt_tx: &mpsc::UnboundedSender<QueuedPrompt>,
    session_id: &str,
) -> IdeResponse {
    let shared = state.lock().await;

    let status_ctx = StatusContext {
        busy: shared.busy,
        turn_count: shared.turn_count,
        active_tools: shared.active_tools.clone(),
    };

    let prompt_ctx = PromptContext {
        prompt_tx: prompt_tx.clone(),
    };

    super::handlers::route_request(
        request,
        &status_ctx,
        &prompt_ctx,
        &shared.pending_requests,
        session_id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::time::timeout;

    #[tokio::test]
    async fn test_ide_controller_creation() {
        let controller = IdeController::new(0); // Port 0 = let OS assign
        assert!(controller.prompt_rx.is_some());
    }

    #[tokio::test]
    async fn test_ide_controller_take_receiver() {
        let mut controller = IdeController::new(0);
        let rx = controller.take_prompt_receiver();
        assert!(rx.is_some());

        // Second call should return None
        let rx2 = controller.take_prompt_receiver();
        assert!(rx2.is_none());
    }

    #[tokio::test]
    async fn test_ide_shared_state_default() {
        let state = IdeSharedState::default();
        assert!(!state.busy);
        assert_eq!(state.turn_count, 0);
        assert!(state.active_tools.is_empty());
        assert!(state.pending_requests.is_empty());
    }

    #[tokio::test]
    async fn test_ide_server_ping_pong() {
        // Start server on random port
        let controller = IdeController::new(0);
        let state = controller.shared_state();

        // Bind to port 0 to get a random available port
        let addr = "127.0.0.1:0";
        let listener = TcpListener::bind(addr).await.unwrap();
        let actual_addr = listener.local_addr().unwrap();

        // Spawn server handler
        let prompt_tx = controller.prompt_tx.clone();
        tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                let session_id = "test-session".to_string();
                let _ = handle_connection(stream, state, prompt_tx, session_id).await;
            }
        });

        // Connect as client
        let mut stream = TcpStream::connect(actual_addr).await.unwrap();

        // Send ping
        stream.write_all(b"{\"type\": \"ping\"}\n").await.unwrap();
        stream.flush().await.unwrap();

        // Read response
        let mut reader = BufReader::new(&mut stream);
        let mut response_line = String::new();

        let result = timeout(Duration::from_secs(2), reader.read_line(&mut response_line)).await;

        assert!(result.is_ok(), "Timeout waiting for response");
        let response: serde_json::Value = serde_json::from_str(&response_line).unwrap();
        assert_eq!(response["type"], "pong");
        assert!(response["version"].as_str().is_some());
    }

    #[tokio::test]
    async fn test_ide_server_status() {
        let controller = IdeController::new(0);
        let state = controller.shared_state();

        // Set some state
        {
            let mut s = state.lock().await;
            s.busy = true;
            s.turn_count = 5;
            s.active_tools = vec!["bash".to_string()];
        }

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let actual_addr = listener.local_addr().unwrap();

        let prompt_tx = controller.prompt_tx.clone();
        let state_clone = Arc::clone(&state);
        tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                let session_id = "test-session".to_string();
                let _ = handle_connection(stream, state_clone, prompt_tx, session_id).await;
            }
        });

        let mut stream = TcpStream::connect(actual_addr).await.unwrap();
        stream
            .write_all(b"{\"type\": \"get_status\"}\n")
            .await
            .unwrap();
        stream.flush().await.unwrap();

        let mut reader = BufReader::new(&mut stream);
        let mut response_line = String::new();

        let result = timeout(Duration::from_secs(2), reader.read_line(&mut response_line)).await;

        assert!(result.is_ok(), "Timeout waiting for response");
        let response: serde_json::Value = serde_json::from_str(&response_line).unwrap();
        assert_eq!(response["type"], "status");
        assert_eq!(response["busy"], true);
        assert_eq!(response["turn_count"], 5);
        assert_eq!(response["active_tools"][0], "bash");
    }

    #[tokio::test]
    async fn test_ide_server_prompt_queuing() {
        let mut controller = IdeController::new(0);
        let state = controller.shared_state();
        let mut prompt_rx = controller.take_prompt_receiver().unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let actual_addr = listener.local_addr().unwrap();

        let prompt_tx = controller.prompt_tx.clone();
        tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                let session_id = "test-session".to_string();
                let _ = handle_connection(stream, state, prompt_tx, session_id).await;
            }
        });

        let mut stream = TcpStream::connect(actual_addr).await.unwrap();
        stream
            .write_all(b"{\"type\": \"send_prompt\", \"text\": \"Hello, Claude!\"}\n")
            .await
            .unwrap();
        stream.flush().await.unwrap();

        let mut reader = BufReader::new(&mut stream);
        let mut response_line = String::new();

        let result = timeout(Duration::from_secs(2), reader.read_line(&mut response_line)).await;

        assert!(result.is_ok(), "Timeout waiting for response");
        let response: serde_json::Value = serde_json::from_str(&response_line).unwrap();
        assert_eq!(response["type"], "prompt_received");
        assert!(response["request_id"].as_str().is_some());

        // Check that prompt was queued
        let queued = timeout(Duration::from_secs(1), prompt_rx.recv())
            .await
            .unwrap();
        assert!(queued.is_some());
        let prompt = queued.unwrap();
        assert_eq!(prompt.text, "Hello, Claude!");
    }
}
