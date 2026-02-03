//! Anthropic API client

pub mod compaction;
pub mod context;
pub mod multi_model;
pub mod tokens;
pub mod tools;

// Re-export token utilities for convenience
pub use tokens::{
    estimate_image_tokens, estimate_message_tokens, estimate_messages_tokens, estimate_tokens,
    TokenBudget, TokenEstimator, DEFAULT_IMAGE_TOKENS,
};

// Re-export context utilities for convenience
pub use context::{
    compact_or_truncate_context, truncate_context, DEFAULT_MAX_INPUT_TOKENS, DEFAULT_MAX_MESSAGES,
};

// Re-export compaction types for convenience
pub use compaction::{CompactionConfig, CompactionResult, ContextCompactor, SummaryStyle};

use std::time::Duration;

use anyhow::Result;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::types::{Message, Role};

// Re-export tool types for convenience
pub use tools::{ToolChoice, ToolDefinition};

// Re-export StreamEvent for backward compatibility
pub use crate::types::StreamEvent;

/// Default Anthropic API endpoint.
const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

/// Maximum number of retry attempts for retryable errors.
const MAX_RETRIES: u32 = 2;

/// Base delay for exponential backoff in milliseconds.
const BASE_BACKOFF_MS: u64 = 100;

#[derive(Clone)]
pub struct AnthropicClient {
    client: reqwest::Client,
    api_key: SecretString,
    model: String,
    base_url: String,
}

#[derive(Serialize)]
struct ApiRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    stream: bool,
    messages: Vec<ApiMessage<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a [ToolDefinition]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'a ToolChoice>,
}

#[derive(Serialize)]
struct ApiMessage<'a> {
    role: &'static str,
    content: &'a str,
}

/// API request type that supports content blocks (for tool_result messages).
#[derive(Serialize)]
struct ApiRequestV2<'a> {
    model: &'a str,
    max_tokens: u32,
    stream: bool,
    messages: &'a [crate::types::ApiMessageV2],
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a [ToolDefinition]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'a ToolChoice>,
}

// ============================================================================
// Stream Parsing Types
// ============================================================================

/// Top-level stream event from the Anthropic API.
#[derive(Deserialize, Debug)]
struct StreamLine {
    #[serde(rename = "type")]
    event_type: String,
    /// For content_block_delta and message_delta events.
    delta: Option<DeltaPayload>,
    /// For content_block_start events - contains the content block.
    content_block: Option<ContentBlockStart>,
    /// For content_block_stop events - the block index.
    index: Option<usize>,
}

/// Payload for delta events (content or message).
#[derive(Deserialize, Debug)]
struct DeltaPayload {
    /// For text content deltas.
    text: Option<String>,
    /// For tool_use input JSON deltas.
    partial_json: Option<String>,
    /// For message_delta - the stop reason.
    stop_reason: Option<String>,
    /// Delta type indicator.
    #[serde(rename = "type")]
    delta_type: Option<String>,
}

/// Content block info from content_block_start events.
#[derive(Deserialize, Debug)]
struct ContentBlockStart {
    #[serde(rename = "type")]
    block_type: String,
    /// For tool_use blocks - the tool ID.
    id: Option<String>,
    /// For tool_use blocks - the tool name.
    name: Option<String>,
}

impl AnthropicClient {
    /// Creates a new Anthropic API client with the default base URL.
    ///
    /// # Arguments
    ///
    /// * `api_key` - The Anthropic API key
    /// * `model` - The model identifier (e.g., "claude-3-opus")
    #[must_use]
    pub fn new(api_key: SecretString, model: &str) -> Self {
        Self::new_with_base_url(api_key, model, DEFAULT_BASE_URL)
    }

    /// Creates a new Anthropic API client with a custom base URL.
    ///
    /// This is primarily useful for testing with mock servers.
    ///
    /// # Arguments
    ///
    /// * `api_key` - The Anthropic API key
    /// * `model` - The model identifier (e.g., "claude-3-opus")
    /// * `base_url` - The base URL for the API (e.g., `https://api.anthropic.com`)
    #[must_use]
    pub fn new_with_base_url(api_key: SecretString, model: &str, base_url: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            model: model.to_string(),
            base_url: base_url.to_string(),
        }
    }

    /// Sends a streaming message request to the Anthropic API.
    ///
    /// # Arguments
    ///
    /// * `messages` - The conversation messages to send
    /// * `tx` - Channel sender for streaming events
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails (network error).
    /// API errors (4xx, 5xx) are sent as `StreamEvent::Error` on the channel.
    ///
    /// # Retries
    ///
    /// The client automatically retries on:
    /// - 429 Too Many Requests (rate limit)
    /// - 5xx Server Errors (500, 502, 503, 504)
    ///
    /// Uses exponential backoff starting at 100ms.
    pub async fn stream_message(
        &self,
        messages: &[Message],
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<()> {
        self.stream_message_with_tools(messages, None, None, tx)
            .await
    }

    /// Sends a streaming message request with tool definitions.
    ///
    /// This is the primary method for agentic tool use. When tools are provided,
    /// Claude can respond with `tool_use` content blocks requesting tool execution.
    ///
    /// # Arguments
    ///
    /// * `messages` - The conversation messages to send
    /// * `tools` - Optional tool definitions Claude can use
    /// * `tool_choice` - Optional constraint on tool selection
    /// * `tx` - Channel sender for streaming events
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails (network error).
    /// API errors (4xx, 5xx) are sent as `StreamEvent::Error` on the channel.
    pub async fn stream_message_with_tools(
        &self,
        messages: &[Message],
        tools: Option<&[ToolDefinition]>,
        tool_choice: Option<&ToolChoice>,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<()> {
        let api_messages: Vec<_> = messages
            .iter()
            .map(|m| ApiMessage {
                role: match m.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                },
                content: &m.content,
            })
            .collect();

        let request = ApiRequest {
            model: &self.model,
            max_tokens: 8192,
            stream: true,
            messages: api_messages,
            tools,
            tool_choice,
        };

        let url = format!("{}/v1/messages", self.base_url);
        let mut last_error: Option<(reqwest::StatusCode, String)> = None;

        for attempt in 0..=MAX_RETRIES {
            let response = self
                .client
                .post(&url)
                .header("x-api-key", self.api_key.expose_secret())
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&request)
                .send()
                .await?;

            let status = response.status();

            if status.is_success() {
                // Success - process the stream
                return self.process_stream(response, tx).await;
            }

            // Check if this is a retryable error
            if Self::is_retryable_status(status) && attempt < MAX_RETRIES {
                let body = response.text().await.unwrap_or_default();
                last_error = Some((status, body));

                // Exponential backoff: 100ms, 200ms, 400ms...
                let delay = Duration::from_millis(BASE_BACKOFF_MS * (1 << attempt));
                tokio::time::sleep(delay).await;
                continue;
            }

            // Non-retryable error or exhausted retries
            let body = response.text().await.unwrap_or_default();
            tx.send(StreamEvent::Error(format!("{}: {}", status, body)))
                .await
                .ok();
            return Ok(());
        }

        // Exhausted retries - send the last error
        if let Some((status, body)) = last_error {
            tx.send(StreamEvent::Error(format!("{}: {}", status, body)))
                .await
                .ok();
        }

        Ok(())
    }

    /// Checks if an HTTP status code should trigger a retry.
    fn is_retryable_status(status: reqwest::StatusCode) -> bool {
        status == reqwest::StatusCode::TOO_MANY_REQUESTS  // 429
            || status.is_server_error() // 5xx
    }

    /// Sends a streaming message request using V2 messages (supports content blocks).
    ///
    /// This method is used for continuing conversations after tool execution.
    /// It properly serializes `MessageContent::Blocks` for tool_result messages.
    ///
    /// # Arguments
    ///
    /// * `messages` - The conversation messages (ApiMessageV2 supports content blocks)
    /// * `tx` - Channel sender for streaming events
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails.
    pub async fn stream_message_v2(
        &self,
        messages: &[crate::types::ApiMessageV2],
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<()> {
        // Include default tools for agentic operation
        let tools = tools::default_tools();

        let request = ApiRequestV2 {
            model: &self.model,
            max_tokens: 8192,
            stream: true,
            messages,
            tools: Some(&tools),
            tool_choice: Some(&ToolChoice::Auto),
        };

        let url = format!("{}/v1/messages", self.base_url);
        let mut last_error: Option<(reqwest::StatusCode, String)> = None;

        for attempt in 0..=MAX_RETRIES {
            let response = self
                .client
                .post(&url)
                .header("x-api-key", self.api_key.expose_secret())
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&request)
                .send()
                .await?;

            let status = response.status();

            if status.is_success() {
                return self.process_stream(response, tx).await;
            }

            if Self::is_retryable_status(status) && attempt < MAX_RETRIES {
                let body = response.text().await.unwrap_or_default();
                last_error = Some((status, body));
                let delay = Duration::from_millis(BASE_BACKOFF_MS * (1 << attempt));
                tokio::time::sleep(delay).await;
                continue;
            }

            let body = response.text().await.unwrap_or_default();
            tx.send(StreamEvent::Error(format!("{}: {}", status, body)))
                .await
                .ok();
            return Ok(());
        }

        if let Some((status, body)) = last_error {
            tx.send(StreamEvent::Error(format!("{}: {}", status, body)))
                .await
                .ok();
        }

        Ok(())
    }

    /// Sends a streaming message request using V2 messages with custom tools.
    ///
    /// This is the primary method for agentic tool use with ApiMessageV2 messages.
    /// It properly serializes `MessageContent::Blocks` for tool_result messages.
    ///
    /// # Arguments
    ///
    /// * `messages` - The conversation messages (ApiMessageV2 supports content blocks)
    /// * `tools` - Optional tool definitions Claude can use
    /// * `tool_choice` - Optional constraint on tool selection
    /// * `tx` - Channel sender for streaming events
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails.
    pub async fn stream_message_v2_with_tools(
        &self,
        messages: &[crate::types::ApiMessageV2],
        tools: Option<&[ToolDefinition]>,
        tool_choice: Option<&ToolChoice>,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<()> {
        let request = ApiRequestV2 {
            model: &self.model,
            max_tokens: 8192,
            stream: true,
            messages,
            tools,
            tool_choice,
        };

        let url = format!("{}/v1/messages", self.base_url);
        let mut last_error: Option<(reqwest::StatusCode, String)> = None;

        for attempt in 0..=MAX_RETRIES {
            let response = self
                .client
                .post(&url)
                .header("x-api-key", self.api_key.expose_secret())
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&request)
                .send()
                .await?;

            let status = response.status();

            if status.is_success() {
                return self.process_stream(response, tx).await;
            }

            if Self::is_retryable_status(status) && attempt < MAX_RETRIES {
                let body = response.text().await.unwrap_or_default();
                last_error = Some((status, body));
                let delay = Duration::from_millis(BASE_BACKOFF_MS * (1 << attempt));
                tokio::time::sleep(delay).await;
                continue;
            }

            let body = response.text().await.unwrap_or_default();
            tx.send(StreamEvent::Error(format!("{}: {}", status, body)))
                .await
                .ok();
            return Ok(());
        }

        if let Some((status, body)) = last_error {
            tx.send(StreamEvent::Error(format!("{}: {}", status, body)))
                .await
                .ok();
        }

        Ok(())
    }

    /// Handles a content_block_start event for tool_use blocks.
    ///
    /// Returns `Some(ToolUseStart)` if the content block is a tool_use with valid id and name.
    /// Returns `None` for text blocks or invalid tool_use blocks.
    fn handle_content_block_start(
        content_block: &ContentBlockStart,
        index: usize,
    ) -> Option<StreamEvent> {
        if content_block.block_type == "tool_use" {
            if let (Some(id), Some(name)) = (&content_block.id, &content_block.name) {
                return Some(StreamEvent::ToolUseStart {
                    id: id.clone(),
                    name: name.clone(),
                    index,
                });
            }
        }
        None
    }

    /// Handles a content_block_stop event, returning the appropriate completion event.
    ///
    /// Returns `ToolUseComplete` if `in_tool_use_block` is true, otherwise `ContentBlockComplete`.
    fn handle_content_block_stop(block_index: usize, in_tool_use_block: bool) -> StreamEvent {
        if in_tool_use_block {
            StreamEvent::ToolUseComplete { index: block_index }
        } else {
            StreamEvent::ContentBlockComplete { index: block_index }
        }
    }

    /// Handles a message_delta event, returning MessageComplete if stop_reason is present.
    ///
    /// Maps stop_reason strings to StopReason enum:
    /// - "tool_use" → `StopReason::ToolUse`
    /// - "max_tokens" → `StopReason::MaxTokens`
    /// - "stop_sequence" → `StopReason::StopSequence`
    /// - anything else → `StopReason::EndTurn`
    fn handle_message_delta(delta: &DeltaPayload) -> Option<StreamEvent> {
        use crate::types::content::StopReason;

        delta.stop_reason.as_ref().map(|stop_reason_str| {
            let stop_reason = match stop_reason_str.as_str() {
                "tool_use" => StopReason::ToolUse,
                "max_tokens" => StopReason::MaxTokens,
                "stop_sequence" => StopReason::StopSequence,
                _ => StopReason::EndTurn,
            };
            StreamEvent::MessageComplete { stop_reason }
        })
    }

    /// Handles a content_block_delta event, returning the appropriate StreamEvent.
    ///
    /// Supports:
    /// - `input_json_delta`: Tool use JSON input fragments → `ToolUseInputDelta`
    /// - `text_delta` or no type: Text content → `ContentDelta`
    /// - Unknown types: Falls back to text if available
    fn handle_content_block_delta(delta: &DeltaPayload, block_index: usize) -> Option<StreamEvent> {
        match delta.delta_type.as_deref() {
            Some("input_json_delta") => {
                // Tool use input JSON fragment
                delta
                    .partial_json
                    .as_ref()
                    .map(|partial_json| StreamEvent::ToolUseInputDelta {
                        index: block_index,
                        partial_json: partial_json.clone(),
                    })
            }
            Some("text_delta") | None => {
                // Regular text content
                delta
                    .text
                    .as_ref()
                    .map(|text| StreamEvent::ContentDelta(text.clone()))
            }
            _ => {
                // Unknown delta type - try text as fallback
                delta
                    .text
                    .as_ref()
                    .map(|text| StreamEvent::ContentDelta(text.clone()))
            }
        }
    }

    /// Processes the SSE stream from a successful response.
    ///
    /// This method parses the Server-Sent Events stream and converts them
    /// to `StreamEvent` values. It handles both text content and tool_use
    /// content blocks.
    async fn process_stream(
        &self,
        response: reqwest::Response,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<()> {
        use futures::StreamExt;

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();

        // Track the current content block index for correlating events
        let mut current_block_index: usize = 0;
        // Track if current block is tool_use (vs text)
        let mut in_tool_use_block = false;

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim();

                if let Some(json) = line.strip_prefix("data: ") {
                    if json != "[DONE]" {
                        if let Ok(parsed) = serde_json::from_str::<StreamLine>(json) {
                            match parsed.event_type.as_str() {
                                // A content block is starting
                                "content_block_start" => {
                                    if let Some(ref content_block) = parsed.content_block {
                                        current_block_index = parsed.index.unwrap_or(0);
                                        in_tool_use_block = content_block.block_type == "tool_use";

                                        if let Some(event) = Self::handle_content_block_start(
                                            content_block,
                                            current_block_index,
                                        ) {
                                            tx.send(event).await.ok();
                                        }
                                    }
                                }

                                // Content is being streamed
                                "content_block_delta" => {
                                    if let Some(ref delta) = parsed.delta {
                                        let block_index =
                                            parsed.index.unwrap_or(current_block_index);
                                        if let Some(event) =
                                            Self::handle_content_block_delta(delta, block_index)
                                        {
                                            tx.send(event).await.ok();
                                        }
                                    }
                                }

                                // A content block has completed
                                "content_block_stop" => {
                                    let block_index = parsed.index.unwrap_or(current_block_index);
                                    let event = Self::handle_content_block_stop(
                                        block_index,
                                        in_tool_use_block,
                                    );
                                    tx.send(event).await.ok();
                                    in_tool_use_block = false;
                                }

                                // Message metadata update (includes stop_reason)
                                "message_delta" => {
                                    if let Some(ref delta) = parsed.delta {
                                        if let Some(event) = Self::handle_message_delta(delta) {
                                            tx.send(event).await.ok();
                                        }
                                    }
                                }

                                // Message stream complete (legacy)
                                "message_stop" => {
                                    tx.send(StreamEvent::MessageStop).await.ok();
                                }

                                // Ignore other event types (message_start, ping, etc.)
                                _ => {}
                            }
                        }
                    }
                }

                buffer = buffer[pos + 1..].to_string();
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tools::{bash_tool, default_tools};
    use crate::types::content::StopReason;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // ============================================================================
    // Phase 2.9.1.1: Characterization tests for process_stream
    // ============================================================================
    // These tests capture the current behavior of process_stream before refactoring.
    // They cover all event types handled by the SSE stream parser.

    /// Helper to create a test client pointing at a mock server.
    fn test_client(base_url: &str) -> AnthropicClient {
        AnthropicClient::new_with_base_url(
            SecretString::from("test-key"),
            "claude-3-opus",
            base_url,
        )
    }

    /// Helper to send a message and collect all events.
    async fn collect_stream_events(
        client: &AnthropicClient,
        sse_response: &str,
        mock_server: &MockServer,
    ) -> Vec<StreamEvent> {
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(sse_response, "text/event-stream")
                    .append_header("content-type", "text/event-stream"),
            )
            .mount(mock_server)
            .await;

        let messages = vec![Message {
            role: Role::User,
            content: "test".to_string(),
        }];

        let (tx, mut rx) = mpsc::channel::<StreamEvent>(64);
        client.stream_message(&messages, tx).await.unwrap();

        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        events
    }

    /// Test: message_start event is silently ignored (no event emitted).
    #[tokio::test]
    async fn test_process_stream_message_start_ignored() {
        let mock_server = MockServer::start().await;
        let client = test_client(&mock_server.uri());

        // message_start should be ignored, only message_stop produces output
        let sse_response = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_123","type":"message","role":"assistant","content":[]}}

event: message_stop
data: {"type":"message_stop"}

"#;

        let events = collect_stream_events(&client, sse_response, &mock_server).await;

        // Should only have MessageStop, not a message_start event
        assert_eq!(events.len(), 1, "Expected only MessageStop event");
        assert!(matches!(events[0], StreamEvent::MessageStop));
    }

    /// Test: content_block_start for text blocks sets state but emits no event.
    #[tokio::test]
    async fn test_process_stream_content_block_start_text() {
        let mock_server = MockServer::start().await;
        let client = test_client(&mock_server.uri());

        let sse_response = r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_stop
data: {"type":"message_stop"}

"#;

        let events = collect_stream_events(&client, sse_response, &mock_server).await;

        // Should have: ContentDelta("Hello"), ContentBlockComplete, MessageStop
        assert!(events.len() >= 2, "Expected at least 2 events");
        assert!(matches!(&events[0], StreamEvent::ContentDelta(s) if s == "Hello"));
        assert!(matches!(
            events[1],
            StreamEvent::ContentBlockComplete { index: 0 }
        ));
        assert!(events.iter().any(|e| matches!(e, StreamEvent::MessageStop)));
    }

    /// Test: content_block_start for tool_use blocks emits ToolUseStart.
    #[tokio::test]
    async fn test_process_stream_content_block_start_tool_use() {
        let mock_server = MockServer::start().await;
        let client = test_client(&mock_server.uri());

        let sse_response = r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_abc123","name":"bash"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_stop
data: {"type":"message_stop"}

"#;

        let events = collect_stream_events(&client, sse_response, &mock_server).await;

        // Should have ToolUseStart with correct id/name/index
        assert!(events.len() >= 2);
        match &events[0] {
            StreamEvent::ToolUseStart { id, name, index } => {
                assert_eq!(id, "toolu_abc123");
                assert_eq!(name, "bash");
                assert_eq!(*index, 0);
            }
            _ => panic!("Expected ToolUseStart, got {:?}", events[0]),
        }
        // ToolUseComplete should follow
        assert!(matches!(
            events[1],
            StreamEvent::ToolUseComplete { index: 0 }
        ));
    }

    /// Test: content_block_delta with text_delta type emits ContentDelta.
    #[tokio::test]
    async fn test_process_stream_content_block_delta_text() {
        let mock_server = MockServer::start().await;
        let client = test_client(&mock_server.uri());

        let sse_response = r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"First "}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Second"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_stop
data: {"type":"message_stop"}

"#;

        let events = collect_stream_events(&client, sse_response, &mock_server).await;

        // Should have multiple ContentDelta events
        let deltas: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                StreamEvent::ContentDelta(s) => Some(s.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(deltas, vec!["First ", "Second"]);
    }

    /// Test: content_block_delta with input_json_delta type emits ToolUseInputDelta.
    #[tokio::test]
    async fn test_process_stream_content_block_delta_input_json() {
        let mock_server = MockServer::start().await;
        let client = test_client(&mock_server.uri());

        let sse_response = r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_abc","name":"bash"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"command\":"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"\"ls\"}"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_stop
data: {"type":"message_stop"}

"#;

        let events = collect_stream_events(&client, sse_response, &mock_server).await;

        // Should have ToolUseInputDelta events
        let json_deltas: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                StreamEvent::ToolUseInputDelta {
                    index,
                    partial_json,
                } => Some((*index, partial_json.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(json_deltas.len(), 2);
        assert_eq!(json_deltas[0], (0, "{\"command\":"));
        assert_eq!(json_deltas[1], (0, "\"ls\"}"));
    }

    /// Test: content_block_delta with no type falls back to text handling.
    #[tokio::test]
    async fn test_process_stream_content_block_delta_no_type() {
        let mock_server = MockServer::start().await;
        let client = test_client(&mock_server.uri());

        // Some older API versions might not include delta type
        let sse_response = r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"text":"No type field"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_stop
data: {"type":"message_stop"}

"#;

        let events = collect_stream_events(&client, sse_response, &mock_server).await;

        // Should still emit ContentDelta
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::ContentDelta(s) if s == "No type field")));
    }

    /// Test: content_block_delta with unknown type falls back to text if available.
    #[tokio::test]
    async fn test_process_stream_content_block_delta_unknown_type() {
        let mock_server = MockServer::start().await;
        let client = test_client(&mock_server.uri());

        let sse_response = r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"future_new_type","text":"Fallback text"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_stop
data: {"type":"message_stop"}

"#;

        let events = collect_stream_events(&client, sse_response, &mock_server).await;

        // Unknown type should fallback to text field
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::ContentDelta(s) if s == "Fallback text")));
    }

    /// Test: content_block_stop in text mode emits ContentBlockComplete.
    #[tokio::test]
    async fn test_process_stream_content_block_stop_text() {
        let mock_server = MockServer::start().await;
        let client = test_client(&mock_server.uri());

        let sse_response = r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_stop
data: {"type":"message_stop"}

"#;

        let events = collect_stream_events(&client, sse_response, &mock_server).await;

        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::ContentBlockComplete { index: 0 })));
    }

    /// Test: content_block_stop in tool_use mode emits ToolUseComplete.
    #[tokio::test]
    async fn test_process_stream_content_block_stop_tool_use() {
        let mock_server = MockServer::start().await;
        let client = test_client(&mock_server.uri());

        let sse_response = r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_123","name":"read_file"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_stop
data: {"type":"message_stop"}

"#;

        let events = collect_stream_events(&client, sse_response, &mock_server).await;

        // Should emit ToolUseComplete, not ContentBlockComplete
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::ToolUseComplete { index: 0 })));
        assert!(!events
            .iter()
            .any(|e| matches!(e, StreamEvent::ContentBlockComplete { .. })));
    }

    /// Test: message_delta with stop_reason "end_turn" emits MessageComplete.
    #[tokio::test]
    async fn test_process_stream_message_delta_end_turn() {
        let mock_server = MockServer::start().await;
        let client = test_client(&mock_server.uri());

        let sse_response = r#"event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"}}

event: message_stop
data: {"type":"message_stop"}

"#;

        let events = collect_stream_events(&client, sse_response, &mock_server).await;

        assert!(events.iter().any(|e| matches!(
            e,
            StreamEvent::MessageComplete {
                stop_reason: StopReason::EndTurn
            }
        )));
    }

    /// Test: message_delta with stop_reason "tool_use" emits MessageComplete.
    #[tokio::test]
    async fn test_process_stream_message_delta_tool_use() {
        let mock_server = MockServer::start().await;
        let client = test_client(&mock_server.uri());

        let sse_response = r#"event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use"}}

event: message_stop
data: {"type":"message_stop"}

"#;

        let events = collect_stream_events(&client, sse_response, &mock_server).await;

        assert!(events.iter().any(|e| matches!(
            e,
            StreamEvent::MessageComplete {
                stop_reason: StopReason::ToolUse
            }
        )));
    }

    /// Test: message_delta with stop_reason "max_tokens" emits MessageComplete.
    #[tokio::test]
    async fn test_process_stream_message_delta_max_tokens() {
        let mock_server = MockServer::start().await;
        let client = test_client(&mock_server.uri());

        let sse_response = r#"event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"max_tokens"}}

event: message_stop
data: {"type":"message_stop"}

"#;

        let events = collect_stream_events(&client, sse_response, &mock_server).await;

        assert!(events.iter().any(|e| matches!(
            e,
            StreamEvent::MessageComplete {
                stop_reason: StopReason::MaxTokens
            }
        )));
    }

    /// Test: message_delta with stop_reason "stop_sequence" emits MessageComplete.
    #[tokio::test]
    async fn test_process_stream_message_delta_stop_sequence() {
        let mock_server = MockServer::start().await;
        let client = test_client(&mock_server.uri());

        let sse_response = r#"event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"stop_sequence"}}

event: message_stop
data: {"type":"message_stop"}

"#;

        let events = collect_stream_events(&client, sse_response, &mock_server).await;

        assert!(events.iter().any(|e| matches!(
            e,
            StreamEvent::MessageComplete {
                stop_reason: StopReason::StopSequence
            }
        )));
    }

    /// Test: message_stop event emits MessageStop.
    #[tokio::test]
    async fn test_process_stream_message_stop() {
        let mock_server = MockServer::start().await;
        let client = test_client(&mock_server.uri());

        let sse_response = r#"event: message_stop
data: {"type":"message_stop"}

"#;

        let events = collect_stream_events(&client, sse_response, &mock_server).await;

        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], StreamEvent::MessageStop));
    }

    /// Test: malformed SSE data is silently skipped.
    #[tokio::test]
    async fn test_process_stream_malformed_sse_skipped() {
        let mock_server = MockServer::start().await;
        let client = test_client(&mock_server.uri());

        let sse_response = r#"event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Before"}}

event: content_block_delta
data: {not valid json!!!}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"After"}}

event: message_stop
data: {"type":"message_stop"}

"#;

        let events = collect_stream_events(&client, sse_response, &mock_server).await;

        // Should have both valid deltas, skipping invalid
        let deltas: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                StreamEvent::ContentDelta(s) => Some(s.as_str()),
                _ => None,
            })
            .collect();
        assert!(deltas.contains(&"Before"));
        assert!(deltas.contains(&"After"));
        // No error event should be emitted for malformed JSON
        assert!(!events.iter().any(|e| e.is_error()));
    }

    /// Test: [DONE] marker is ignored.
    #[tokio::test]
    async fn test_process_stream_done_marker_ignored() {
        let mock_server = MockServer::start().await;
        let client = test_client(&mock_server.uri());

        let sse_response = r#"event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}

data: [DONE]

event: message_stop
data: {"type":"message_stop"}

"#;

        let events = collect_stream_events(&client, sse_response, &mock_server).await;

        // Should have ContentDelta and MessageStop, [DONE] is ignored
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::ContentDelta(s) if s == "Hello")));
        assert!(events.iter().any(|e| matches!(e, StreamEvent::MessageStop)));
    }

    /// Test: Full tool use flow with all event types.
    #[tokio::test]
    async fn test_process_stream_full_tool_use_flow() {
        let mock_server = MockServer::start().await;
        let client = test_client(&mock_server.uri());

        let sse_response = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_123","type":"message","role":"assistant","content":[]}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_01abc","name":"bash"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"command\":"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"\"ls -la\"}"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use"}}

event: message_stop
data: {"type":"message_stop"}

"#;

        let events = collect_stream_events(&client, sse_response, &mock_server).await;

        // Verify complete tool use flow sequence
        let event_types: Vec<&str> = events
            .iter()
            .map(|e| match e {
                StreamEvent::ToolUseStart { .. } => "ToolUseStart",
                StreamEvent::ToolUseInputDelta { .. } => "ToolUseInputDelta",
                StreamEvent::ToolUseComplete { .. } => "ToolUseComplete",
                StreamEvent::MessageComplete { .. } => "MessageComplete",
                StreamEvent::MessageStop => "MessageStop",
                StreamEvent::ContentDelta(_) => "ContentDelta",
                StreamEvent::ContentBlockComplete { .. } => "ContentBlockComplete",
                StreamEvent::Error(_) => "Error",
            })
            .collect();

        assert!(event_types.contains(&"ToolUseStart"));
        assert!(event_types.contains(&"ToolUseInputDelta"));
        assert!(event_types.contains(&"ToolUseComplete"));
        assert!(event_types.contains(&"MessageComplete"));
        assert!(event_types.contains(&"MessageStop"));

        // Verify ToolUseStart details
        let start_event = events
            .iter()
            .find(|e| matches!(e, StreamEvent::ToolUseStart { .. }))
            .unwrap();
        match start_event {
            StreamEvent::ToolUseStart { id, name, index } => {
                assert_eq!(id, "toolu_01abc");
                assert_eq!(name, "bash");
                assert_eq!(*index, 0);
            }
            _ => unreachable!(),
        }

        // Verify stop_reason is tool_use
        assert!(events.iter().any(|e| matches!(
            e,
            StreamEvent::MessageComplete {
                stop_reason: StopReason::ToolUse
            }
        )));
    }

    /// Test: Multiple content blocks with mixed types.
    #[tokio::test]
    async fn test_process_stream_multiple_content_blocks() {
        let mock_server = MockServer::start().await;
        let client = test_client(&mock_server.uri());

        // Text block followed by tool_use block
        let sse_response = r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"I'll run a command."}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_xyz","name":"bash"}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{}"}}

event: content_block_stop
data: {"type":"content_block_stop","index":1}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use"}}

event: message_stop
data: {"type":"message_stop"}

"#;

        let events = collect_stream_events(&client, sse_response, &mock_server).await;

        // Should have text content
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::ContentDelta(s) if s == "I'll run a command.")));
        // Should have ContentBlockComplete for index 0
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::ContentBlockComplete { index: 0 })));
        // Should have ToolUseStart for index 1
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::ToolUseStart { index: 1, .. })));
        // Should have ToolUseComplete for index 1
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::ToolUseComplete { index: 1 })));
    }

    /// Test: ping events are silently ignored.
    #[tokio::test]
    async fn test_process_stream_ping_ignored() {
        let mock_server = MockServer::start().await;
        let client = test_client(&mock_server.uri());

        let sse_response = r#"event: ping
data: {"type":"ping"}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"After ping"}}

event: message_stop
data: {"type":"message_stop"}

"#;

        let events = collect_stream_events(&client, sse_response, &mock_server).await;

        // ping should be ignored, only ContentDelta and MessageStop
        assert_eq!(events.len(), 2);
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::ContentDelta(s) if s == "After ping")));
        assert!(events.iter().any(|e| matches!(e, StreamEvent::MessageStop)));
    }

    // ============================================================================
    // End Phase 2.9.1.1 characterization tests
    // ============================================================================

    // ============================================================================
    // Phase 2.9.1.3: Unit tests for extracted handle_content_block_delta
    // ============================================================================

    #[test]
    fn test_handle_content_block_delta_text_delta() {
        let delta = DeltaPayload {
            text: Some("Hello world".to_string()),
            partial_json: None,
            stop_reason: None,
            delta_type: Some("text_delta".to_string()),
        };

        let result = AnthropicClient::handle_content_block_delta(&delta, 0);
        assert!(matches!(
            result,
            Some(StreamEvent::ContentDelta(ref s)) if s == "Hello world"
        ));
    }

    #[test]
    fn test_handle_content_block_delta_input_json() {
        let delta = DeltaPayload {
            text: None,
            partial_json: Some("{\"key\":".to_string()),
            stop_reason: None,
            delta_type: Some("input_json_delta".to_string()),
        };

        let result = AnthropicClient::handle_content_block_delta(&delta, 2);
        match result {
            Some(StreamEvent::ToolUseInputDelta {
                index,
                partial_json,
            }) => {
                assert_eq!(index, 2);
                assert_eq!(partial_json, "{\"key\":");
            }
            _ => panic!("Expected ToolUseInputDelta, got {:?}", result),
        }
    }

    #[test]
    fn test_handle_content_block_delta_no_type_with_text() {
        let delta = DeltaPayload {
            text: Some("Fallback text".to_string()),
            partial_json: None,
            stop_reason: None,
            delta_type: None,
        };

        let result = AnthropicClient::handle_content_block_delta(&delta, 0);
        assert!(matches!(
            result,
            Some(StreamEvent::ContentDelta(ref s)) if s == "Fallback text"
        ));
    }

    #[test]
    fn test_handle_content_block_delta_unknown_type_with_text() {
        let delta = DeltaPayload {
            text: Some("Unknown type text".to_string()),
            partial_json: None,
            stop_reason: None,
            delta_type: Some("future_unknown_type".to_string()),
        };

        let result = AnthropicClient::handle_content_block_delta(&delta, 0);
        assert!(matches!(
            result,
            Some(StreamEvent::ContentDelta(ref s)) if s == "Unknown type text"
        ));
    }

    #[test]
    fn test_handle_content_block_delta_input_json_missing_partial() {
        let delta = DeltaPayload {
            text: None,
            partial_json: None,
            stop_reason: None,
            delta_type: Some("input_json_delta".to_string()),
        };

        let result = AnthropicClient::handle_content_block_delta(&delta, 0);
        assert!(
            result.is_none(),
            "Should return None when partial_json is missing"
        );
    }

    #[test]
    fn test_handle_content_block_delta_text_delta_missing_text() {
        let delta = DeltaPayload {
            text: None,
            partial_json: None,
            stop_reason: None,
            delta_type: Some("text_delta".to_string()),
        };

        let result = AnthropicClient::handle_content_block_delta(&delta, 0);
        assert!(result.is_none(), "Should return None when text is missing");
    }

    // ============================================================================
    // End Phase 2.9.1.3 unit tests
    // ============================================================================

    // ============================================================================
    // Phase 2.9.1.4: Unit tests for extracted handle_message_delta
    // ============================================================================

    #[test]
    fn test_handle_message_delta_tool_use() {
        let delta = DeltaPayload {
            text: None,
            partial_json: None,
            stop_reason: Some("tool_use".to_string()),
            delta_type: None,
        };

        let result = AnthropicClient::handle_message_delta(&delta);
        assert!(matches!(
            result,
            Some(StreamEvent::MessageComplete {
                stop_reason: StopReason::ToolUse
            })
        ));
    }

    #[test]
    fn test_handle_message_delta_max_tokens() {
        let delta = DeltaPayload {
            text: None,
            partial_json: None,
            stop_reason: Some("max_tokens".to_string()),
            delta_type: None,
        };

        let result = AnthropicClient::handle_message_delta(&delta);
        assert!(matches!(
            result,
            Some(StreamEvent::MessageComplete {
                stop_reason: StopReason::MaxTokens
            })
        ));
    }

    #[test]
    fn test_handle_message_delta_stop_sequence() {
        let delta = DeltaPayload {
            text: None,
            partial_json: None,
            stop_reason: Some("stop_sequence".to_string()),
            delta_type: None,
        };

        let result = AnthropicClient::handle_message_delta(&delta);
        assert!(matches!(
            result,
            Some(StreamEvent::MessageComplete {
                stop_reason: StopReason::StopSequence
            })
        ));
    }

    #[test]
    fn test_handle_message_delta_end_turn() {
        let delta = DeltaPayload {
            text: None,
            partial_json: None,
            stop_reason: Some("end_turn".to_string()),
            delta_type: None,
        };

        let result = AnthropicClient::handle_message_delta(&delta);
        assert!(matches!(
            result,
            Some(StreamEvent::MessageComplete {
                stop_reason: StopReason::EndTurn
            })
        ));
    }

    #[test]
    fn test_handle_message_delta_unknown_reason() {
        let delta = DeltaPayload {
            text: None,
            partial_json: None,
            stop_reason: Some("some_future_reason".to_string()),
            delta_type: None,
        };

        let result = AnthropicClient::handle_message_delta(&delta);
        // Unknown reasons default to EndTurn
        assert!(matches!(
            result,
            Some(StreamEvent::MessageComplete {
                stop_reason: StopReason::EndTurn
            })
        ));
    }

    #[test]
    fn test_handle_message_delta_no_stop_reason() {
        let delta = DeltaPayload {
            text: None,
            partial_json: None,
            stop_reason: None,
            delta_type: None,
        };

        let result = AnthropicClient::handle_message_delta(&delta);
        assert!(
            result.is_none(),
            "Should return None when stop_reason is missing"
        );
    }

    // ============================================================================
    // End Phase 2.9.1.4 unit tests
    // ============================================================================

    // ============================================================================
    // Phase 2.9.1.6: Unit tests for additional extracted handlers
    // ============================================================================

    #[test]
    fn test_handle_content_block_start_tool_use() {
        let block = ContentBlockStart {
            block_type: "tool_use".to_string(),
            id: Some("toolu_123".to_string()),
            name: Some("bash".to_string()),
        };

        let result = AnthropicClient::handle_content_block_start(&block, 0);
        match result {
            Some(StreamEvent::ToolUseStart { id, name, index }) => {
                assert_eq!(id, "toolu_123");
                assert_eq!(name, "bash");
                assert_eq!(index, 0);
            }
            _ => panic!("Expected ToolUseStart, got {:?}", result),
        }
    }

    #[test]
    fn test_handle_content_block_start_tool_use_missing_id() {
        let block = ContentBlockStart {
            block_type: "tool_use".to_string(),
            id: None,
            name: Some("bash".to_string()),
        };

        let result = AnthropicClient::handle_content_block_start(&block, 0);
        assert!(result.is_none(), "Should return None when id is missing");
    }

    #[test]
    fn test_handle_content_block_start_tool_use_missing_name() {
        let block = ContentBlockStart {
            block_type: "tool_use".to_string(),
            id: Some("toolu_123".to_string()),
            name: None,
        };

        let result = AnthropicClient::handle_content_block_start(&block, 0);
        assert!(result.is_none(), "Should return None when name is missing");
    }

    #[test]
    fn test_handle_content_block_start_text() {
        let block = ContentBlockStart {
            block_type: "text".to_string(),
            id: None,
            name: None,
        };

        let result = AnthropicClient::handle_content_block_start(&block, 0);
        assert!(result.is_none(), "Should return None for text blocks");
    }

    #[test]
    fn test_handle_content_block_stop_tool_use() {
        let result = AnthropicClient::handle_content_block_stop(1, true);
        assert!(matches!(result, StreamEvent::ToolUseComplete { index: 1 }));
    }

    #[test]
    fn test_handle_content_block_stop_text() {
        let result = AnthropicClient::handle_content_block_stop(2, false);
        assert!(matches!(
            result,
            StreamEvent::ContentBlockComplete { index: 2 }
        ));
    }

    // ============================================================================
    // End Phase 2.9.1.6 unit tests
    // ============================================================================

    #[test]
    fn test_api_request_serialization_without_tools() {
        let messages = vec![ApiMessage {
            role: "user",
            content: "Hello",
        }];

        let request = ApiRequest {
            model: "claude-3-opus",
            max_tokens: 1024,
            stream: true,
            messages,
            tools: None,
            tool_choice: None,
        };

        let json = serde_json::to_string(&request).expect("serialization should succeed");

        // Should NOT contain tools or tool_choice fields when None
        assert!(!json.contains("\"tools\""));
        assert!(!json.contains("\"tool_choice\""));
        assert!(json.contains("\"model\":\"claude-3-opus\""));
        assert!(json.contains("\"stream\":true"));
    }

    #[test]
    fn test_api_request_serialization_with_tools() {
        let messages = vec![ApiMessage {
            role: "user",
            content: "Run a command",
        }];

        let tools = vec![bash_tool()];
        let tool_choice = ToolChoice::Auto;

        let request = ApiRequest {
            model: "claude-3-opus",
            max_tokens: 1024,
            stream: true,
            messages,
            tools: Some(&tools),
            tool_choice: Some(&tool_choice),
        };

        let json = serde_json::to_string(&request).expect("serialization should succeed");

        // Should contain tools and tool_choice
        assert!(json.contains("\"tools\""));
        assert!(json.contains("\"tool_choice\""));
        assert!(json.contains("\"bash\""));
        assert!(json.contains("\"type\":\"auto\""));
    }

    #[test]
    fn test_api_request_with_all_default_tools() {
        let messages = vec![ApiMessage {
            role: "user",
            content: "test",
        }];

        let tools = default_tools();

        let request = ApiRequest {
            model: "claude-sonnet",
            max_tokens: 8192,
            stream: true,
            messages,
            tools: Some(&tools),
            tool_choice: None,
        };

        let json = serde_json::to_string(&request).expect("serialization should succeed");

        // Verify all tools are present
        assert!(json.contains("\"bash\""));
        assert!(json.contains("\"read_file\""));
        assert!(json.contains("\"write_file\""));
        assert!(json.contains("\"edit\""));
        assert!(json.contains("\"list_files\""));
        assert!(json.contains("\"glob\""));
        assert!(json.contains("\"grep\""));
    }
}
