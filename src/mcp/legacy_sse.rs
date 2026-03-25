//! Legacy SSE transport for MCP servers.
//!
//! Implements the legacy Server-Sent Events (SSE) transport protocol where:
//! 1. Client GETs an SSE endpoint and receives a POST URL via an `endpoint` event
//! 2. Client POSTs JSON-RPC messages to that URL
//! 3. Server sends JSON-RPC responses as `message` events on the SSE stream
//!
//! This transport is needed for MCP servers (like JetBrains) that use the
//! legacy SSE protocol rather than the newer Streamable HTTP protocol.
//!
//! # Protocol Flow
//!
//! ```text
//! Client                          Server
//!   |--- GET /sse (Accept: text/event-stream) --->|
//!   |<-- event: endpoint                          |
//!   |    data: /messages?session=abc               |
//!   |                                             |
//!   |--- POST /messages?session=abc (JSON-RPC) -->|
//!   |<-- event: message                           |
//!   |    data: {"jsonrpc":"2.0",...}               |
//! ```

use bytes::Bytes;
use futures::StreamExt;
use reqwest::header::HeaderMap;
use rmcp::model::{ClientJsonRpcMessage, ServerJsonRpcMessage};
use rmcp::service::RoleClient;
use rmcp::transport::Transport;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Error type for the legacy SSE transport.
#[derive(Debug, thiserror::Error)]
pub enum LegacySseError {
    /// HTTP request or stream error.
    #[error("HTTP error: {0}")]
    Http(reqwest::Error),

    /// JSON serialization error.
    #[error("JSON error: {0}")]
    Json(serde_json::Error),

    /// Invalid or unresolvable URL.
    #[error("Invalid URL: {0}")]
    InvalidUrl(String),

    /// Timed out waiting for the `endpoint` SSE event.
    #[error("Timed out waiting for endpoint event")]
    EndpointTimeout,

    /// SSE stream closed before receiving the `endpoint` event.
    #[error("SSE stream ended before endpoint event was received")]
    StreamEnded,
}

/// A transport for MCP servers using the legacy SSE protocol.
///
/// Connects to an SSE endpoint, receives a POST URL via an `endpoint` event,
/// and implements the rmcp `Transport<RoleClient>` trait for bidirectional
/// JSON-RPC messaging.
///
/// # Architecture
///
/// - **Sending**: JSON-RPC messages are POSTed to the endpoint URL
/// - **Receiving**: A background task reads the SSE stream and forwards
///   `message` events through an mpsc channel
/// - **Closing**: The background reader task is aborted
#[derive(Debug)]
pub struct LegacySseTransport {
    /// HTTP client for POST requests (clone-cheap via internal Arc).
    http_client: reqwest::Client,
    /// POST URL received from the `endpoint` SSE event.
    post_url: Arc<String>,
    /// Custom headers to include in POST requests.
    custom_headers: HeaderMap,
    /// Receiver for JSON-RPC messages from the SSE stream.
    server_rx: mpsc::Receiver<ServerJsonRpcMessage>,
    /// Handle for the background SSE reader task.
    reader_handle: Option<JoinHandle<()>>,
}

impl LegacySseTransport {
    /// Connects to a legacy SSE MCP server.
    ///
    /// 1. GETs the SSE URL with `Accept: text/event-stream`
    /// 2. Reads SSE events until an `endpoint` event provides the POST URL
    /// 3. Spawns a background task to read subsequent `message` events
    ///
    /// # Arguments
    ///
    /// * `sse_url` - URL of the SSE endpoint
    /// * `headers` - Optional HTTP headers (e.g., Authorization)
    /// * `timeout` - Maximum time to wait for the endpoint event
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The URL is invalid
    /// - The GET request fails
    /// - The `endpoint` event is not received within the timeout
    /// - The SSE stream ends before the `endpoint` event
    pub async fn connect(
        sse_url: &str,
        headers: &Option<HashMap<String, String>>,
        timeout: Duration,
    ) -> Result<Self, LegacySseError> {
        let http_client = reqwest::Client::new();

        let mut custom_headers = HeaderMap::new();
        if let Some(hdrs) = headers {
            for (k, v) in hdrs {
                if let (Ok(name), Ok(val)) = (
                    k.parse::<reqwest::header::HeaderName>(),
                    v.parse::<reqwest::header::HeaderValue>(),
                ) {
                    custom_headers.insert(name, val);
                }
            }
        }

        let base_url =
            reqwest::Url::parse(sse_url).map_err(|e| LegacySseError::InvalidUrl(e.to_string()))?;

        // Validate the SSE URL against SSRF (no localhost/private IPs)
        crate::util::url_validation::validate_url(sse_url, false)
            .map_err(|e| LegacySseError::InvalidUrl(e.to_string()))?;

        // GET the SSE endpoint
        let response = http_client
            .get(sse_url)
            .headers(custom_headers.clone())
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .send()
            .await
            .map_err(LegacySseError::Http)?
            .error_for_status()
            .map_err(LegacySseError::Http)?;

        let mut stream = Box::pin(response.bytes_stream());
        let mut line_buffer = String::new();
        let mut event_type = String::new();
        let mut event_data = String::new();

        // Wait for the `endpoint` event with timeout.
        // The async block borrows the stream/buffers; after it completes,
        // ownership returns so they can be moved to the background task.
        let post_endpoint = tokio::time::timeout(timeout, async {
            while let Some(chunk_result) = stream.next().await {
                let chunk = chunk_result.map_err(LegacySseError::Http)?;
                line_buffer.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(newline_pos) = line_buffer.find('\n') {
                    let line = line_buffer[..newline_pos]
                        .trim_end_matches('\r')
                        .to_string();
                    line_buffer = line_buffer[newline_pos + 1..].to_string();

                    if line.is_empty() {
                        // Blank line dispatches the buffered event
                        if event_type == "endpoint" && !event_data.is_empty() {
                            let resolved = base_url
                                .join(&event_data)
                                .map_err(|e| LegacySseError::InvalidUrl(e.to_string()))?;
                            // Validate the server-provided endpoint URL against SSRF
                            crate::util::url_validation::validate_url(resolved.as_str(), false)
                                .map_err(|e| LegacySseError::InvalidUrl(e.to_string()))?;
                            return Ok::<String, LegacySseError>(resolved.to_string());
                        }
                        event_type.clear();
                        event_data.clear();
                    } else if let Some(rest) = line.strip_prefix("event:") {
                        event_type = rest.strip_prefix(' ').unwrap_or(rest).to_string();
                    } else if let Some(rest) = line.strip_prefix("data:") {
                        let value = rest.strip_prefix(' ').unwrap_or(rest);
                        if !event_data.is_empty() {
                            event_data.push('\n');
                        }
                        event_data.push_str(value);
                    }
                    // Lines starting with ':' are comments — ignored
                }
            }
            Err(LegacySseError::StreamEnded)
        })
        .await
        .map_err(|_| LegacySseError::EndpointTimeout)??;

        let post_url = Arc::new(post_endpoint);
        let (tx, rx) = mpsc::channel(256);

        // Spawn background task to continue reading SSE message events.
        // The stream and buffers are moved here, continuing from where
        // the endpoint search left off.
        let reader_handle = tokio::spawn(async move {
            sse_reader_loop(
                &mut stream,
                &mut line_buffer,
                &mut event_type,
                &mut event_data,
                &tx,
            )
            .await;
            tracing::debug!("Legacy SSE stream ended");
        });

        Ok(Self {
            http_client,
            post_url,
            custom_headers,
            server_rx: rx,
            reader_handle: Some(reader_handle),
        })
    }
}

impl Transport<RoleClient> for LegacySseTransport {
    type Error = LegacySseError;

    fn send(
        &mut self,
        item: ClientJsonRpcMessage,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        // Clone cheap handles for the 'static future
        let client = self.http_client.clone();
        let url = Arc::clone(&self.post_url);
        let headers = self.custom_headers.clone();

        async move {
            let body = serde_json::to_vec(&item).map_err(LegacySseError::Json)?;
            client
                .post(url.as_str())
                .headers(headers)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body)
                .send()
                .await
                .map_err(LegacySseError::Http)?;
            Ok(())
        }
    }

    fn receive(&mut self) -> impl Future<Output = Option<ServerJsonRpcMessage>> + Send {
        self.server_rx.recv()
    }

    fn close(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send {
        if let Some(handle) = self.reader_handle.take() {
            handle.abort();
        }
        self.server_rx.close();
        std::future::ready(Ok(()))
    }
}

/// Reads SSE events from the stream and sends parsed JSON-RPC messages to the channel.
///
/// Processes `message` events (or events with no explicit type) as JSON-RPC messages.
/// Stops when the stream ends, a read error occurs, or the channel is closed.
async fn sse_reader_loop(
    stream: &mut (impl futures::Stream<Item = Result<Bytes, reqwest::Error>> + Unpin),
    line_buffer: &mut String,
    event_type: &mut String,
    event_data: &mut String,
    tx: &mpsc::Sender<ServerJsonRpcMessage>,
) {
    while let Some(chunk_result) = stream.next().await {
        let chunk = match chunk_result {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "Legacy SSE stream error");
                break;
            }
        };

        line_buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(newline_pos) = line_buffer.find('\n') {
            let line = line_buffer[..newline_pos]
                .trim_end_matches('\r')
                .to_string();
            *line_buffer = line_buffer[newline_pos + 1..].to_string();

            if line.is_empty() {
                // Blank line dispatches the buffered event
                let is_message = event_type == "message" || event_type.is_empty();
                if is_message && !event_data.is_empty() {
                    match serde_json::from_str::<ServerJsonRpcMessage>(event_data) {
                        Ok(msg) => {
                            if tx.send(msg).await.is_err() {
                                return; // Channel closed, receiver dropped
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "Failed to parse SSE message as JSON-RPC"
                            );
                        }
                    }
                }
                event_type.clear();
                event_data.clear();
            } else if let Some(rest) = line.strip_prefix("event:") {
                *event_type = rest.strip_prefix(' ').unwrap_or(rest).to_string();
            } else if let Some(rest) = line.strip_prefix("data:") {
                let value = rest.strip_prefix(' ').unwrap_or(rest);
                if !event_data.is_empty() {
                    event_data.push('\n');
                }
                event_data.push_str(value);
            }
            // Lines starting with ':' are comments — ignored
            // Lines starting with 'id:' or 'retry:' — ignored
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_sse_error_display() {
        let err = LegacySseError::EndpointTimeout;
        assert_eq!(err.to_string(), "Timed out waiting for endpoint event");

        let err = LegacySseError::StreamEnded;
        assert!(err.to_string().contains("endpoint event"));

        let err = LegacySseError::InvalidUrl("bad url".to_string());
        assert!(err.to_string().contains("bad url"));
    }

    #[test]
    fn legacy_sse_error_is_send_sync() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<LegacySseError>();
    }

    #[test]
    fn transport_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<LegacySseTransport>();
    }

    #[tokio::test]
    async fn test_connect_rejects_private_ip() {
        let result = LegacySseTransport::connect(
            "http://169.254.169.254/latest/meta-data/",
            &None,
            Duration::from_secs(1),
        )
        .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, LegacySseError::InvalidUrl(_)),
            "Expected InvalidUrl for private IP, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_connect_rejects_localhost() {
        let result =
            LegacySseTransport::connect("http://127.0.0.1/sse", &None, Duration::from_secs(1))
                .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, LegacySseError::InvalidUrl(_)),
            "Expected InvalidUrl for localhost, got: {err}"
        );
    }

    #[tokio::test]
    async fn connect_invalid_url_returns_error() {
        let result = LegacySseTransport::connect("not-a-url", &None, Duration::from_secs(1)).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), LegacySseError::InvalidUrl(_)));
    }

    #[tokio::test]
    async fn connect_unreachable_host_returns_error() {
        let result =
            LegacySseTransport::connect("http://192.0.2.1:1/sse", &None, Duration::from_secs(1))
                .await;
        assert!(result.is_err());
        // Either Http error (connection refused) or EndpointTimeout
        let err = result.unwrap_err();
        assert!(
            matches!(
                err,
                LegacySseError::Http(_) | LegacySseError::EndpointTimeout
            ),
            "Expected Http or EndpointTimeout, got: {err}"
        );
    }

    #[tokio::test]
    async fn connect_with_headers() {
        let headers = Some(HashMap::from([(
            "Authorization".to_string(),
            "Bearer token".to_string(),
        )]));
        // Should fail (no server) but should not panic on header parsing
        let result =
            LegacySseTransport::connect("http://192.0.2.1:1/sse", &headers, Duration::from_secs(1))
                .await;
        assert!(result.is_err());
    }

    /// Test SSE line parsing via the reader loop with a mock stream.
    #[tokio::test]
    async fn sse_reader_parses_message_events() {
        let sse_data = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\n";
        let chunk = Bytes::from(sse_data);
        let stream = futures::stream::once(async { Ok::<_, reqwest::Error>(chunk) });
        let mut pinned = Box::pin(stream);

        let mut line_buffer = String::new();
        let mut event_type = String::new();
        let mut event_data = String::new();
        let (tx, mut rx) = mpsc::channel(256);

        sse_reader_loop(
            &mut pinned,
            &mut line_buffer,
            &mut event_type,
            &mut event_data,
            &tx,
        )
        .await;

        let msg = rx.try_recv().expect("Should have received a message");
        // Verify it's a valid ServerJsonRpcMessage (Response variant)
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["id"], 1);
    }

    /// Test that events without an explicit type default to "message".
    #[tokio::test]
    async fn sse_reader_default_event_type_is_message() {
        let sse_data = "data: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{}}\n\n";
        let chunk = Bytes::from(sse_data);
        let stream = futures::stream::once(async { Ok::<_, reqwest::Error>(chunk) });
        let mut pinned = Box::pin(stream);

        let mut line_buffer = String::new();
        let mut event_type = String::new();
        let mut event_data = String::new();
        let (tx, mut rx) = mpsc::channel(256);

        sse_reader_loop(
            &mut pinned,
            &mut line_buffer,
            &mut event_type,
            &mut event_data,
            &tx,
        )
        .await;

        let msg = rx.try_recv().expect("Should have received a message");
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["id"], 2);
    }

    /// Test that SSE comments (lines starting with ':') are ignored.
    #[tokio::test]
    async fn sse_reader_ignores_comments() {
        let sse_data =
            ": this is a comment\nevent: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{}}\n\n";
        let chunk = Bytes::from(sse_data);
        let stream = futures::stream::once(async { Ok::<_, reqwest::Error>(chunk) });
        let mut pinned = Box::pin(stream);

        let mut line_buffer = String::new();
        let mut event_type = String::new();
        let mut event_data = String::new();
        let (tx, mut rx) = mpsc::channel(256);

        sse_reader_loop(
            &mut pinned,
            &mut line_buffer,
            &mut event_type,
            &mut event_data,
            &tx,
        )
        .await;

        let msg = rx.try_recv().expect("Should have received a message");
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["id"], 3);
    }

    /// Test that non-message event types (like 'endpoint') are ignored by the reader.
    #[tokio::test]
    async fn sse_reader_ignores_non_message_events() {
        let sse_data = "event: endpoint\ndata: /messages\n\nevent: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":4,\"result\":{}}\n\n";
        let chunk = Bytes::from(sse_data);
        let stream = futures::stream::once(async { Ok::<_, reqwest::Error>(chunk) });
        let mut pinned = Box::pin(stream);

        let mut line_buffer = String::new();
        let mut event_type = String::new();
        let mut event_data = String::new();
        let (tx, mut rx) = mpsc::channel(256);

        sse_reader_loop(
            &mut pinned,
            &mut line_buffer,
            &mut event_type,
            &mut event_data,
            &tx,
        )
        .await;

        // Should only have the message event, not the endpoint event
        let msg = rx.try_recv().expect("Should have received a message");
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["id"], 4);
        assert!(rx.try_recv().is_err(), "Should have no more messages");
    }

    /// Test multiline data fields (multiple data: lines for one event).
    #[tokio::test]
    async fn sse_reader_handles_invalid_json_gracefully() {
        let sse_data = "event: message\ndata: not valid json\n\n";
        let chunk = Bytes::from(sse_data);
        let stream = futures::stream::once(async { Ok::<_, reqwest::Error>(chunk) });
        let mut pinned = Box::pin(stream);

        let mut line_buffer = String::new();
        let mut event_type = String::new();
        let mut event_data = String::new();
        let (tx, mut rx) = mpsc::channel(256);

        sse_reader_loop(
            &mut pinned,
            &mut line_buffer,
            &mut event_type,
            &mut event_data,
            &tx,
        )
        .await;

        // Should not have sent anything (invalid JSON is logged and skipped)
        assert!(rx.try_recv().is_err());
    }

    /// Test that chunks arriving split across SSE event boundaries are handled.
    #[tokio::test]
    async fn sse_reader_handles_split_chunks() {
        let chunk1 = Bytes::from("event: message\nda");
        let chunk2 = Bytes::from("ta: {\"jsonrpc\":\"2.0\",\"id\":5,\"result\":{}}\n\n");
        let stream = futures::stream::iter(vec![Ok::<_, reqwest::Error>(chunk1), Ok(chunk2)]);
        let mut pinned = Box::pin(stream);

        let mut line_buffer = String::new();
        let mut event_type = String::new();
        let mut event_data = String::new();
        let (tx, mut rx) = mpsc::channel(256);

        sse_reader_loop(
            &mut pinned,
            &mut line_buffer,
            &mut event_type,
            &mut event_data,
            &tx,
        )
        .await;

        let msg = rx.try_recv().expect("Should have received a message");
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["id"], 5);
    }
}
