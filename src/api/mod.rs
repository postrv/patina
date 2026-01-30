//! Anthropic API client

pub mod multi_model;

use std::time::Duration;

use anyhow::Result;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::types::{Message, Role};

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
}

#[derive(Serialize)]
struct ApiMessage<'a> {
    role: &'static str,
    content: &'a str,
}

#[derive(Deserialize)]
struct StreamLine {
    #[serde(rename = "type")]
    event_type: String,
    delta: Option<ContentDelta>,
}

#[derive(Deserialize)]
struct ContentDelta {
    text: Option<String>,
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

    /// Processes the SSE stream from a successful response.
    async fn process_stream(
        &self,
        response: reqwest::Response,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<()> {
        let mut stream = response.bytes_stream();
        let mut buffer = String::new();

        use futures::StreamExt;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim();

                if let Some(json) = line.strip_prefix("data: ") {
                    if json != "[DONE]" {
                        if let Ok(parsed) = serde_json::from_str::<StreamLine>(json) {
                            match parsed.event_type.as_str() {
                                "content_block_delta" => {
                                    if let Some(delta) = parsed.delta {
                                        if let Some(text) = delta.text {
                                            tx.send(StreamEvent::ContentDelta(text)).await.ok();
                                        }
                                    }
                                }
                                "message_stop" => {
                                    tx.send(StreamEvent::MessageStop).await.ok();
                                }
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
