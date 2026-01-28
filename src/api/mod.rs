//! Anthropic API client

use anyhow::Result;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::app::state::{Message, Role};

#[derive(Clone)]
pub struct AnthropicClient {
    client: reqwest::Client,
    api_key: SecretString,
    model: String,
}

#[derive(Debug)]
pub enum StreamEvent {
    ContentDelta(String),
    MessageStop,
    Error(String),
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
    #[serde(rename = "type")]
    #[allow(dead_code)]
    delta_type: Option<String>,
    text: Option<String>,
}

impl AnthropicClient {
    pub fn new(api_key: SecretString, model: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            model: model.to_string(),
        }
    }

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

        let response = self.client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", self.api_key.expose_secret())
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            tx.send(StreamEvent::Error(format!("{}: {}", status, body))).await.ok();
            return Ok(());
        }

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();

        use futures::StreamExt;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim();

                if line.starts_with("data: ") {
                    let json = &line[6..];
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
