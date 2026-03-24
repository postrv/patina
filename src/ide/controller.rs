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
    prompt_tx: mpsc::Sender<QueuedPrompt>,
    /// Receiver for prompts (held by controller, given to main app)
    prompt_rx: Option<mpsc::Receiver<QueuedPrompt>>,
    /// Authentication token required for connections
    auth_token: String,
}

impl IdeController {
    /// Creates a new IDE controller for the specified port
    ///
    /// Generates a unique authentication token that clients must present
    /// before any requests are processed.
    #[must_use]
    pub fn new(port: u16) -> Self {
        let (prompt_tx, prompt_rx) = mpsc::channel(32);
        Self {
            port,
            state: Arc::new(Mutex::new(IdeSharedState::default())),
            prompt_tx,
            prompt_rx: Some(prompt_rx),
            auth_token: Uuid::new_v4().to_string(),
        }
    }

    /// Returns the authentication token for this controller
    ///
    /// IDE clients must send this token via an `Authenticate` request
    /// before any other requests will be processed.
    #[must_use]
    pub fn auth_token(&self) -> &str {
        &self.auth_token
    }

    /// Takes the prompt receiver for the main application to consume
    ///
    /// This can only be called once. Subsequent calls return `None`.
    pub fn take_prompt_receiver(&mut self) -> Option<mpsc::Receiver<QueuedPrompt>> {
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
                    let auth_token = self.auth_token.clone();

                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(
                            stream,
                            state,
                            prompt_tx,
                            session_id.clone(),
                            auth_token,
                        )
                        .await
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
///
/// Connections must authenticate with a valid token before any requests
/// are processed. Unauthenticated requests receive an `AUTH_REQUIRED` error.
async fn handle_connection(
    mut stream: TcpStream,
    state: Arc<Mutex<IdeSharedState>>,
    prompt_tx: mpsc::Sender<QueuedPrompt>,
    session_id: String,
    auth_token: String,
) -> Result<()> {
    let mut buffer = vec![0u8; 8192];
    let mut authenticated = false;

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
                    if !authenticated {
                        if let IdeRequest::Authenticate { token } = &request {
                            if token == &auth_token {
                                authenticated = true;
                                info!("IDE connection {} authenticated", session_id);
                                IdeResponse::AuthResult { success: true }
                            } else {
                                warn!("IDE connection {} failed authentication", session_id);
                                IdeResponse::AuthResult { success: false }
                            }
                        } else {
                            IdeResponse::Error {
                                code: "AUTH_REQUIRED".to_string(),
                                message: "Connection must authenticate before sending requests"
                                    .to_string(),
                                request_id: None,
                            }
                        }
                    } else {
                        process_request(request, &state, &prompt_tx, &session_id).await
                    }
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
    prompt_tx: &mpsc::Sender<QueuedPrompt>,
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

    /// Helper: spawn a server handler on a random port with the given controller's
    /// state, prompt_tx, and auth_token. Returns the address and the token.
    async fn spawn_server(controller: &IdeController) -> (std::net::SocketAddr, String) {
        let state = controller.shared_state();
        let token = controller.auth_token().to_string();
        let prompt_tx = controller.prompt_tx.clone();
        let token_for_server = token.clone();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                let session_id = "test-session".to_string();
                let _ =
                    handle_connection(stream, state, prompt_tx, session_id, token_for_server).await;
            }
        });

        (addr, token)
    }

    /// Helper: spawn a server handler that uses a custom state Arc (for pre-set state).
    async fn spawn_server_with_state(
        controller: &IdeController,
        state: Arc<Mutex<IdeSharedState>>,
    ) -> (std::net::SocketAddr, String) {
        let token = controller.auth_token().to_string();
        let prompt_tx = controller.prompt_tx.clone();
        let token_for_server = token.clone();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                let session_id = "test-session".to_string();
                let _ =
                    handle_connection(stream, state, prompt_tx, session_id, token_for_server).await;
            }
        });

        (addr, token)
    }

    /// Helper: connect to address and split into (reader, writer).
    async fn connect_split(
        addr: std::net::SocketAddr,
    ) -> (
        BufReader<tokio::net::tcp::OwnedReadHalf>,
        tokio::net::tcp::OwnedWriteHalf,
    ) {
        let stream = TcpStream::connect(addr).await.unwrap();
        let (read_half, write_half) = stream.into_split();
        (BufReader::new(read_half), write_half)
    }

    /// Helper: send a JSON message (newline-terminated) on the write half.
    async fn send_msg(writer: &mut tokio::net::tcp::OwnedWriteHalf, msg: &str) {
        writer.write_all(msg.as_bytes()).await.unwrap();
        writer.write_all(b"\n").await.unwrap();
        writer.flush().await.unwrap();
    }

    /// Helper: read one newline-terminated JSON response, with a 2-second timeout.
    async fn recv_json(
        reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>,
    ) -> serde_json::Value {
        let mut line = String::new();
        let result = timeout(Duration::from_secs(2), reader.read_line(&mut line)).await;
        assert!(result.is_ok(), "Timeout waiting for response");
        serde_json::from_str(&line).unwrap()
    }

    /// Helper: authenticate a connection with the given token.
    async fn authenticate(
        reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>,
        writer: &mut tokio::net::tcp::OwnedWriteHalf,
        token: &str,
    ) {
        let auth_msg = format!("{{\"type\": \"authenticate\", \"token\": \"{token}\"}}");
        send_msg(writer, &auth_msg).await;
        let resp = recv_json(reader).await;
        assert_eq!(resp["type"], "auth_result");
        assert_eq!(resp["success"], true);
    }

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
        let controller = IdeController::new(0);
        let (addr, token) = spawn_server(&controller).await;
        let (mut reader, mut writer) = connect_split(addr).await;

        // Authenticate first
        authenticate(&mut reader, &mut writer, &token).await;

        // Send ping
        send_msg(&mut writer, "{\"type\": \"ping\"}").await;
        let response = recv_json(&mut reader).await;

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

        let (addr, token) = spawn_server_with_state(&controller, Arc::clone(&state)).await;
        let (mut reader, mut writer) = connect_split(addr).await;

        // Authenticate first
        authenticate(&mut reader, &mut writer, &token).await;

        // Send get_status
        send_msg(&mut writer, "{\"type\": \"get_status\"}").await;
        let response = recv_json(&mut reader).await;

        assert_eq!(response["type"], "status");
        assert_eq!(response["busy"], true);
        assert_eq!(response["turn_count"], 5);
        assert_eq!(response["active_tools"][0], "bash");
    }

    #[tokio::test]
    async fn test_ide_server_prompt_queuing() {
        let mut controller = IdeController::new(0);
        let mut prompt_rx = controller.take_prompt_receiver().unwrap();
        let (addr, token) = spawn_server(&controller).await;
        let (mut reader, mut writer) = connect_split(addr).await;

        // Authenticate first
        authenticate(&mut reader, &mut writer, &token).await;

        // Send prompt
        send_msg(
            &mut writer,
            "{\"type\": \"send_prompt\", \"text\": \"Hello, Claude!\"}",
        )
        .await;
        let response = recv_json(&mut reader).await;

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

    // =========================================================================
    // Authentication tests
    // =========================================================================

    #[tokio::test]
    async fn test_ide_auth_token_generated_nonempty() {
        let controller = IdeController::new(0);
        assert!(!controller.auth_token().is_empty());
        // Should be a valid UUID v4
        assert!(uuid::Uuid::parse_str(controller.auth_token()).is_ok());
    }

    #[tokio::test]
    async fn test_ide_auth_token_unique() {
        let controller1 = IdeController::new(0);
        let controller2 = IdeController::new(0);
        assert_ne!(controller1.auth_token(), controller2.auth_token());
    }

    #[tokio::test]
    async fn test_ide_connection_requires_auth() {
        let controller = IdeController::new(0);
        let (addr, _token) = spawn_server(&controller).await;
        let (mut reader, mut writer) = connect_split(addr).await;

        // Send ping without authenticating first
        send_msg(&mut writer, "{\"type\": \"ping\"}").await;
        let response = recv_json(&mut reader).await;

        assert_eq!(response["type"], "error");
        assert_eq!(response["code"], "AUTH_REQUIRED");
    }

    #[tokio::test]
    async fn test_ide_connection_wrong_token() {
        let controller = IdeController::new(0);
        let (addr, _token) = spawn_server(&controller).await;
        let (mut reader, mut writer) = connect_split(addr).await;

        // Send authenticate with wrong token
        send_msg(
            &mut writer,
            "{\"type\": \"authenticate\", \"token\": \"wrong-token-value\"}",
        )
        .await;
        let response = recv_json(&mut reader).await;

        assert_eq!(response["type"], "auth_result");
        assert_eq!(response["success"], false);
    }

    #[tokio::test]
    async fn test_ide_connection_auth_success() {
        let controller = IdeController::new(0);
        let (addr, token) = spawn_server(&controller).await;
        let (mut reader, mut writer) = connect_split(addr).await;

        // Authenticate with correct token
        let auth_msg = format!("{{\"type\": \"authenticate\", \"token\": \"{token}\"}}");
        send_msg(&mut writer, &auth_msg).await;
        let auth_resp = recv_json(&mut reader).await;

        assert_eq!(auth_resp["type"], "auth_result");
        assert_eq!(auth_resp["success"], true);

        // Now send ping - should work after authentication
        send_msg(&mut writer, "{\"type\": \"ping\"}").await;
        let pong_resp = recv_json(&mut reader).await;

        assert_eq!(pong_resp["type"], "pong");
        assert!(pong_resp["version"].as_str().is_some());
    }
}
