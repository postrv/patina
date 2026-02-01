//! Anthropic API client

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
pub use context::{truncate_context, DEFAULT_MAX_INPUT_TOKENS, DEFAULT_MAX_MESSAGES};

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
        use crate::types::content::StopReason;
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
                                    if let Some(content_block) = parsed.content_block {
                                        current_block_index = parsed.index.unwrap_or(0);

                                        if content_block.block_type == "tool_use" {
                                            in_tool_use_block = true;
                                            // Emit tool use start event
                                            if let (Some(id), Some(name)) =
                                                (content_block.id, content_block.name)
                                            {
                                                tx.send(StreamEvent::ToolUseStart {
                                                    id,
                                                    name,
                                                    index: current_block_index,
                                                })
                                                .await
                                                .ok();
                                            }
                                        } else {
                                            in_tool_use_block = false;
                                        }
                                    }
                                }

                                // Content is being streamed
                                "content_block_delta" => {
                                    if let Some(delta) = parsed.delta {
                                        let block_index =
                                            parsed.index.unwrap_or(current_block_index);

                                        // Check delta type to determine how to handle
                                        match delta.delta_type.as_deref() {
                                            Some("input_json_delta") => {
                                                // Tool use input JSON fragment
                                                if let Some(partial_json) = delta.partial_json {
                                                    tx.send(StreamEvent::ToolUseInputDelta {
                                                        index: block_index,
                                                        partial_json,
                                                    })
                                                    .await
                                                    .ok();
                                                }
                                            }
                                            Some("text_delta") | None => {
                                                // Regular text content
                                                if let Some(text) = delta.text {
                                                    tx.send(StreamEvent::ContentDelta(text))
                                                        .await
                                                        .ok();
                                                }
                                            }
                                            _ => {
                                                // Unknown delta type - try text as fallback
                                                if let Some(text) = delta.text {
                                                    tx.send(StreamEvent::ContentDelta(text))
                                                        .await
                                                        .ok();
                                                }
                                            }
                                        }
                                    }
                                }

                                // A content block has completed
                                "content_block_stop" => {
                                    let block_index = parsed.index.unwrap_or(current_block_index);

                                    if in_tool_use_block {
                                        tx.send(StreamEvent::ToolUseComplete {
                                            index: block_index,
                                        })
                                        .await
                                        .ok();
                                        in_tool_use_block = false;
                                    } else {
                                        tx.send(StreamEvent::ContentBlockComplete {
                                            index: block_index,
                                        })
                                        .await
                                        .ok();
                                    }
                                }

                                // Message metadata update (includes stop_reason)
                                "message_delta" => {
                                    if let Some(delta) = parsed.delta {
                                        if let Some(stop_reason_str) = delta.stop_reason {
                                            let stop_reason = match stop_reason_str.as_str() {
                                                "tool_use" => StopReason::ToolUse,
                                                "max_tokens" => StopReason::MaxTokens,
                                                "stop_sequence" => StopReason::StopSequence,
                                                _ => StopReason::EndTurn,
                                            };
                                            tx.send(StreamEvent::MessageComplete { stop_reason })
                                                .await
                                                .ok();
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
