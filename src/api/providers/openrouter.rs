//! OpenRouter provider and message format translation.
//!
//! This module provides the [`OpenRouterProvider`] LLM provider implementation
//! and bidirectional translation functions for converting between Patina's
//! internal Anthropic-style message types and the OpenAI Chat Completions API
//! format used by OpenRouter and compatible providers.
//!
//! # Provider
//!
//! [`OpenRouterProvider`] implements [`LlmProvider`](crate::api::provider::LlmProvider)
//! and handles HTTPS request building, SSE streaming with OpenAI format, and
//! translation to Patina's [`StreamEvent`] protocol.
//!
//! # Translation Functions
//!
//! - [`translate_to_openai`]: Converts Anthropic messages to OpenAI format
//! - [`translate_tool_use`]: Converts a single tool use block to an OpenAI tool call
//! - [`translate_stream_event`]: Converts an OpenAI stream chunk to Patina stream events
//!
//! # Key Differences
//!
//! | Anthropic | OpenAI |
//! |-----------|--------|
//! | System prompt is a separate parameter | System prompt is a message with `role: "system"` |
//! | Tool input is a parsed JSON `Value` | Tool arguments is a JSON-encoded `String` |
//! | Tool results are `ContentBlock::ToolResult` inside user messages | Tool results are separate messages with `role: "tool"` |
//! | `StopReason::EndTurn` | `finish_reason: "stop"` |
//!
//! # Examples
//!
//! ```rust
//! use patina::api::providers::openrouter::{translate_to_openai, translate_stream_event};
//! use patina::types::message::ApiMessageV2;
//!
//! let messages = vec![
//!     ApiMessageV2::user("Hello!"),
//!     ApiMessageV2::assistant("Hi there!"),
//! ];
//!
//! let openai_messages = translate_to_openai(Some("You are helpful."), &messages);
//! assert_eq!(openai_messages.len(), 3); // system + user + assistant
//! ```

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use tokio::sync::mpsc;

use crate::api::provider::LlmProvider;
use crate::api::providers::openai_types::{
    OpenAiFunctionCall, OpenAiMessage, OpenAiRole, OpenAiStreamChunk, OpenAiTool, OpenAiToolCall,
    OpenAiToolChoice,
};
use crate::api::tools::{ToolChoice, ToolDefinition};
use crate::types::content::{ContentBlock, StopReason, ToolUseBlock};
use crate::types::message::{ApiMessageV2, MessageContent, Role};
use crate::types::stream::StreamEvent;

/// Default OpenRouter API endpoint.
const DEFAULT_OPENROUTER_URL: &str = "https://openrouter.ai/api/v1";

/// OpenAI Chat Completions API request body.
#[derive(Debug, Serialize)]
struct OpenAiChatRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<OpenAiToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

/// LLM provider for OpenRouter (OpenAI-compatible API).
///
/// Translates between Patina's Anthropic-style messages and the OpenAI Chat
/// Completions format, handling HTTPS request building and SSE stream parsing.
///
/// # Examples
///
/// ```rust,ignore
/// use patina::api::providers::openrouter::OpenRouterProvider;
/// use patina::api::provider::LlmProvider;
/// use secrecy::SecretString;
///
/// let provider = OpenRouterProvider::new(
///     SecretString::from("sk-or-..."),
///     "anthropic/claude-sonnet-4",
/// );
/// assert_eq!(provider.name(), "openrouter");
/// ```
pub struct OpenRouterProvider {
    client: reqwest::Client,
    api_key: SecretString,
    model: String,
    base_url: String,
    site_url: Option<String>,
    app_name: Option<String>,
}

impl OpenRouterProvider {
    /// Creates a new OpenRouter provider with the default API endpoint.
    ///
    /// # Arguments
    ///
    /// * `api_key` - The OpenRouter API key
    /// * `model` - The model identifier (e.g., "anthropic/claude-sonnet-4")
    #[must_use]
    pub fn new(api_key: SecretString, model: &str) -> Self {
        Self::new_with_base_url(api_key, model, DEFAULT_OPENROUTER_URL)
    }

    /// Creates a new OpenRouter provider with a custom base URL.
    ///
    /// This is primarily useful for testing with mock servers.
    ///
    /// # Arguments
    ///
    /// * `api_key` - The OpenRouter API key
    /// * `model` - The model identifier (e.g., "anthropic/claude-sonnet-4")
    /// * `base_url` - The base URL for the API
    #[must_use]
    pub fn new_with_base_url(api_key: SecretString, model: &str, base_url: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            model: model.to_string(),
            base_url: base_url.to_string(),
            site_url: None,
            app_name: None,
        }
    }

    /// Sets the site URL sent via `HTTP-Referer` header.
    ///
    /// OpenRouter uses this for analytics and rate limiting.
    #[must_use]
    pub fn with_site_url(mut self, url: impl Into<String>) -> Self {
        self.site_url = Some(url.into());
        self
    }

    /// Sets the app name sent via `X-Title` header.
    ///
    /// OpenRouter uses this for analytics.
    #[must_use]
    pub fn with_app_name(mut self, name: impl Into<String>) -> Self {
        self.app_name = Some(name.into());
        self
    }

    /// Processes the OpenAI SSE stream from a successful response.
    ///
    /// Parses `data: {...}` lines, deserializes as [`OpenAiStreamChunk`],
    /// and translates to [`StreamEvent`] using [`translate_stream_event`].
    ///
    /// # Errors
    ///
    /// Returns an error if reading the byte stream fails.
    async fn process_stream(
        &self,
        response: reqwest::Response,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<()> {
        use futures::StreamExt;

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim().to_string();
                buffer = buffer[pos + 1..].to_string();

                let Some(json) = line.strip_prefix("data: ") else {
                    continue;
                };

                if json == "[DONE]" {
                    return Ok(());
                }

                let Ok(parsed) = serde_json::from_str::<OpenAiStreamChunk>(json) else {
                    continue;
                };

                for event in translate_stream_event(&parsed) {
                    tx.send(event).await.ok();
                }
            }
        }

        Ok(())
    }
}

impl LlmProvider for OpenRouterProvider {
    fn name(&self) -> &str {
        "openrouter"
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn stream_message<'a>(
        &'a self,
        messages: &'a [ApiMessageV2],
        tools: Option<&'a [ToolDefinition]>,
        tool_choice: Option<&'a ToolChoice>,
        options: &'a crate::api::provider::RequestOptions,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if options.thinking.is_some() {
                tracing::warn!(
                    "OpenRouter does not support extended thinking; ignoring thinking config"
                );
            }
            let system_text: Option<String> = options.system.as_ref().map(|blocks| {
                blocks
                    .iter()
                    .map(|b| b.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            });
            if options
                .system
                .as_ref()
                .is_some_and(|blocks| blocks.iter().any(|b| b.cache_control.is_some()))
            {
                tracing::warn!(
                    "OpenRouter does not support cache_control on system blocks; \
                     system prompt text is passed through but cache directives are ignored"
                );
            }
            let openai_messages = translate_to_openai(system_text.as_deref(), messages);

            let openai_tools: Option<Vec<OpenAiTool>> =
                tools.map(|t| t.iter().map(|td| OpenAiTool::from(td.clone())).collect());

            let openai_tool_choice: Option<OpenAiToolChoice> =
                tool_choice.map(|tc| OpenAiToolChoice::from(tc.clone()));

            let request_body = OpenAiChatRequest {
                model: self.model.clone(),
                messages: openai_messages,
                stream: true,
                tools: openai_tools,
                tool_choice: openai_tool_choice,
                max_tokens: Some(8192),
            };

            let url = format!("{}/chat/completions", self.base_url);

            crate::api::retry::send_with_retry(
                || async {
                    let mut req = self
                        .client
                        .post(&url)
                        .header(
                            "Authorization",
                            format!("Bearer {}", self.api_key.expose_secret()),
                        )
                        .header("Content-Type", "application/json");

                    if let Some(ref site_url) = self.site_url {
                        req = req.header("HTTP-Referer", site_url.as_str());
                    }
                    if let Some(ref app_name) = self.app_name {
                        req = req.header("X-Title", app_name.as_str());
                    }

                    req.json(&request_body).send().await.map_err(Into::into)
                },
                |response, tx| async move { self.process_stream(response, tx).await },
                &tx,
            )
            .await
        })
    }
}

/// Translates Anthropic-format messages to OpenAI Chat Completions format.
///
/// Converts a sequence of [`ApiMessageV2`] messages (with an optional system prompt)
/// into a sequence of [`OpenAiMessage`] messages suitable for the OpenAI-compatible API.
///
/// # Translation Rules
///
/// - An optional system prompt becomes an `OpenAiMessage` with `role: "system"` prepended
///   to the output.
/// - `MessageContent::Text` user/assistant messages map directly.
/// - `MessageContent::Blocks` assistant messages with `ToolUse` blocks produce an assistant
///   message with `tool_calls`. Text blocks are combined into the `content` field.
/// - `MessageContent::Blocks` user messages with `ToolResult` blocks produce separate
///   `OpenAiMessage` entries with `role: "tool"`. Text blocks become a separate user message.
/// - `ContentBlock::Image` blocks are currently represented as `[image]` placeholder text.
///
/// # Arguments
///
/// * `system` - Optional system prompt to prepend as a system message.
/// * `messages` - Slice of Anthropic-format messages to translate.
///
/// # Examples
///
/// ```rust
/// use patina::api::providers::openrouter::translate_to_openai;
/// use patina::types::message::ApiMessageV2;
///
/// let messages = vec![ApiMessageV2::user("Hello")];
/// let result = translate_to_openai(None, &messages);
/// assert_eq!(result.len(), 1);
/// ```
#[must_use]
pub fn translate_to_openai(system: Option<&str>, messages: &[ApiMessageV2]) -> Vec<OpenAiMessage> {
    let mut result = Vec::new();

    if let Some(sys) = system {
        result.push(OpenAiMessage::system(sys));
    }

    for msg in messages {
        translate_message(msg, &mut result);
    }

    result
}

/// Translates a single Anthropic message into one or more OpenAI messages.
///
/// A single Anthropic message may produce multiple OpenAI messages when it
/// contains `ToolResult` blocks (each becomes a separate `role: "tool"` message).
fn translate_message(msg: &ApiMessageV2, out: &mut Vec<OpenAiMessage>) {
    match (&msg.role, &msg.content) {
        (Role::User, MessageContent::Text(text)) => {
            out.push(OpenAiMessage::user(text));
        }
        (Role::Assistant, MessageContent::Text(text)) => {
            out.push(OpenAiMessage::assistant(text));
        }
        (Role::Assistant, MessageContent::Blocks(blocks)) => {
            translate_assistant_blocks(blocks, out);
        }
        (Role::User, MessageContent::Blocks(blocks)) => {
            translate_user_blocks(blocks, out);
        }
    }
}

/// Translates assistant content blocks into an OpenAI assistant message.
///
/// Combines text blocks into `content` and tool_use blocks into `tool_calls`.
/// Produces a single assistant message with both fields populated as needed.
fn translate_assistant_blocks(blocks: &[ContentBlock], out: &mut Vec<OpenAiMessage>) {
    let mut text_parts: Vec<&str> = Vec::new();
    let mut tool_calls: Vec<OpenAiToolCall> = Vec::new();

    for block in blocks {
        match block {
            ContentBlock::Text { text } => {
                text_parts.push(text);
            }
            ContentBlock::ToolUse(tool_use) => {
                tool_calls.push(translate_tool_use(tool_use));
            }
            ContentBlock::Image { .. } => {
                text_parts.push("[image]");
            }
            ContentBlock::ToolResult(_) => {
                // ToolResult in assistant messages is invalid in Anthropic format,
                // but we handle it gracefully by ignoring it.
            }
            ContentBlock::Thinking { .. } => {
                // Thinking blocks are internal reasoning; not forwarded to OpenAI.
            }
        }
    }

    let content_text = text_parts.join("");
    let has_text = !content_text.is_empty();
    let has_tools = !tool_calls.is_empty();

    match (has_text, has_tools) {
        (true, true) => {
            out.push(OpenAiMessage {
                role: OpenAiRole::Assistant,
                content: Some(content_text),
                name: None,
                tool_calls: Some(tool_calls),
                tool_call_id: None,
            });
        }
        (true, false) => {
            out.push(OpenAiMessage::assistant(content_text));
        }
        (false, true) => {
            out.push(OpenAiMessage::assistant_with_tool_calls(tool_calls));
        }
        (false, false) => {
            // Empty blocks list: produce an empty assistant message
            out.push(OpenAiMessage::assistant(""));
        }
    }
}

/// Translates user content blocks into OpenAI messages.
///
/// Tool result blocks each become a separate `role: "tool"` message.
/// Text blocks are combined into a single user message.
/// Tool results are emitted first, then the user text message (if any).
fn translate_user_blocks(blocks: &[ContentBlock], out: &mut Vec<OpenAiMessage>) {
    let mut text_parts: Vec<&str> = Vec::new();

    for block in blocks {
        match block {
            ContentBlock::ToolResult(result) => {
                let content = if result.is_error {
                    format!("Error: {}", result.content)
                } else {
                    result.content.clone()
                };
                out.push(OpenAiMessage::tool(&result.tool_use_id, content));
            }
            ContentBlock::Text { text } => {
                text_parts.push(text);
            }
            ContentBlock::Image { .. } => {
                text_parts.push("[image]");
            }
            ContentBlock::ToolUse(_) => {
                // ToolUse in user messages is invalid in Anthropic format,
                // but we handle it gracefully by ignoring it.
            }
            ContentBlock::Thinking { .. } => {
                // Thinking blocks are internal reasoning; not forwarded to OpenAI.
            }
        }
    }

    let combined_text = text_parts.join("");
    if !combined_text.is_empty() {
        out.push(OpenAiMessage::user(combined_text));
    }
}

/// Converts an Anthropic [`ToolUseBlock`] to an OpenAI [`OpenAiToolCall`].
///
/// The key transformation is serializing the parsed JSON `input` [`Value`]
/// into a JSON-encoded `String` for the OpenAI `arguments` field.
///
/// # Arguments
///
/// * `tool_use` - The Anthropic tool use block to convert.
///
/// # Examples
///
/// ```rust
/// use patina::api::providers::openrouter::translate_tool_use;
/// use patina::types::content::ToolUseBlock;
/// use serde_json::json;
///
/// let tool_use = ToolUseBlock::new("toolu_123", "bash", json!({"command": "ls"}));
/// let call = translate_tool_use(&tool_use);
/// assert_eq!(call.function.name, "bash");
/// assert!(call.function.arguments.contains("command"));
/// ```
#[must_use]
pub fn translate_tool_use(tool_use: &ToolUseBlock) -> OpenAiToolCall {
    OpenAiToolCall {
        id: tool_use.id.clone(),
        call_type: "function".to_string(),
        function: OpenAiFunctionCall {
            name: tool_use.name.clone(),
            arguments: serde_json::to_string(&tool_use.input).unwrap_or_else(|_| "{}".to_string()),
        },
    }
}

/// Translates an OpenAI streaming chunk into Patina [`StreamEvent`] variants.
///
/// A single OpenAI chunk may produce zero or more stream events. The function
/// processes the first choice in the chunk (index 0) and maps its fields:
///
/// - `delta.content` (non-empty) → [`StreamEvent::ContentDelta`]
/// - `delta.tool_calls` with `id` + `name` → [`StreamEvent::ToolUseStart`]
/// - `delta.tool_calls` with `arguments` only → [`StreamEvent::ToolUseInputDelta`]
/// - `finish_reason` → [`StreamEvent::MessageComplete`]
///
/// # Arguments
///
/// * `chunk` - The OpenAI streaming chunk to translate.
///
/// # Examples
///
/// ```rust
/// use patina::api::providers::openrouter::translate_stream_event;
/// use patina::api::providers::openai_types::*;
///
/// let chunk = OpenAiStreamChunk {
///     id: "id".to_string(),
///     object: "chat.completion.chunk".to_string(),
///     created: 0,
///     model: "model".to_string(),
///     choices: vec![OpenAiStreamChoice {
///         index: 0,
///         delta: OpenAiDelta {
///             role: None,
///             content: Some("Hello".to_string()),
///             tool_calls: None,
///         },
///         finish_reason: None,
///     }],
///     usage: None,
/// };
///
/// let events = translate_stream_event(&chunk);
/// assert_eq!(events.len(), 1);
/// ```
#[must_use]
pub fn translate_stream_event(chunk: &OpenAiStreamChunk) -> Vec<StreamEvent> {
    let mut events = Vec::new();

    let Some(choice) = chunk.choices.first() else {
        return events;
    };

    // Process content delta
    if let Some(ref content) = choice.delta.content {
        if !content.is_empty() {
            events.push(StreamEvent::ContentDelta(content.clone()));
        }
    }

    // Process tool call deltas
    if let Some(ref tool_calls) = choice.delta.tool_calls {
        for tc_delta in tool_calls {
            let index = tc_delta.index as usize;

            // Emit ToolUseStart when all required fields are present
            if let (Some(id), Some(func)) = (&tc_delta.id, &tc_delta.function) {
                if let Some(name) = &func.name {
                    events.push(StreamEvent::ToolUseStart {
                        id: id.clone(),
                        name: name.clone(),
                        index,
                    });
                }
            }

            // Process argument fragments
            if let Some(ref func) = tc_delta.function {
                if let Some(ref args) = func.arguments {
                    if !args.is_empty() {
                        events.push(StreamEvent::ToolUseInputDelta {
                            index,
                            partial_json: args.clone(),
                        });
                    }
                }
            }
        }
    }

    // Process finish reason
    if let Some(finish_reason) = choice.finish_reason {
        let stop_reason: StopReason = finish_reason.into();
        events.push(StreamEvent::MessageComplete { stop_reason });
    }

    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::provider::RequestOptions;
    use crate::api::providers::openai_types::*;
    use crate::types::image::ImageSource;
    use serde_json::json;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // =========================================================================
    // translate_to_openai: Basic message translation
    // =========================================================================

    #[test]
    fn test_translate_empty_messages_no_system() {
        let result = translate_to_openai(None, &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_translate_system_prompt_only() {
        let result = translate_to_openai(Some("You are helpful."), &[]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].role, OpenAiRole::System);
        assert_eq!(result[0].content.as_deref(), Some("You are helpful."));
    }

    #[test]
    fn test_translate_user_text_message() {
        let messages = vec![ApiMessageV2::user("Hello!")];
        let result = translate_to_openai(None, &messages);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].role, OpenAiRole::User);
        assert_eq!(result[0].content.as_deref(), Some("Hello!"));
    }

    #[test]
    fn test_translate_assistant_text_message() {
        let messages = vec![ApiMessageV2::assistant("Hi there!")];
        let result = translate_to_openai(None, &messages);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].role, OpenAiRole::Assistant);
        assert_eq!(result[0].content.as_deref(), Some("Hi there!"));
    }

    #[test]
    fn test_translate_multi_turn_conversation() {
        let messages = vec![
            ApiMessageV2::user("What is 2+2?"),
            ApiMessageV2::assistant("2+2 equals 4."),
            ApiMessageV2::user("Thanks!"),
        ];
        let result = translate_to_openai(Some("Be concise."), &messages);

        assert_eq!(result.len(), 4);
        assert_eq!(result[0].role, OpenAiRole::System);
        assert_eq!(result[1].role, OpenAiRole::User);
        assert_eq!(result[2].role, OpenAiRole::Assistant);
        assert_eq!(result[3].role, OpenAiRole::User);
    }

    // =========================================================================
    // translate_to_openai: Tool use blocks (assistant)
    // =========================================================================

    #[test]
    fn test_translate_assistant_with_tool_use_only() {
        let messages = vec![ApiMessageV2::assistant_with_content(
            MessageContent::blocks(vec![ContentBlock::tool_use(
                "toolu_123",
                "bash",
                json!({"command": "ls"}),
            )]),
        )];
        let result = translate_to_openai(None, &messages);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].role, OpenAiRole::Assistant);
        assert!(result[0].content.is_none());
        let calls = result[0].tool_calls.as_ref().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "toolu_123");
        assert_eq!(calls[0].call_type, "function");
        assert_eq!(calls[0].function.name, "bash");
        // Arguments should be a JSON string, not a parsed object
        let parsed_args: serde_json::Value =
            serde_json::from_str(&calls[0].function.arguments).unwrap();
        assert_eq!(parsed_args["command"], "ls");
    }

    #[test]
    fn test_translate_assistant_with_text_and_tool_use() {
        let messages = vec![ApiMessageV2::assistant_with_content(
            MessageContent::blocks(vec![
                ContentBlock::text("Let me check that for you."),
                ContentBlock::tool_use("toolu_456", "read_file", json!({"path": "README.md"})),
            ]),
        )];
        let result = translate_to_openai(None, &messages);

        assert_eq!(result.len(), 1);
        let msg = &result[0];
        assert_eq!(msg.role, OpenAiRole::Assistant);
        assert_eq!(msg.content.as_deref(), Some("Let me check that for you."));
        assert_eq!(msg.tool_calls.as_ref().unwrap().len(), 1);
        assert_eq!(
            msg.tool_calls.as_ref().unwrap()[0].function.name,
            "read_file"
        );
    }

    #[test]
    fn test_translate_assistant_with_multiple_tool_uses() {
        let messages = vec![ApiMessageV2::assistant_with_content(
            MessageContent::blocks(vec![
                ContentBlock::text("I'll run these commands:"),
                ContentBlock::tool_use("toolu_1", "bash", json!({"command": "pwd"})),
                ContentBlock::tool_use("toolu_2", "bash", json!({"command": "ls"})),
            ]),
        )];
        let result = translate_to_openai(None, &messages);

        assert_eq!(result.len(), 1);
        let calls = result[0].tool_calls.as_ref().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].function.name, "bash");
        assert_eq!(calls[1].function.name, "bash");
        assert_ne!(calls[0].id, calls[1].id);
    }

    // =========================================================================
    // translate_to_openai: Tool result blocks (user)
    // =========================================================================

    #[test]
    fn test_translate_user_with_tool_result() {
        let messages = vec![ApiMessageV2::user_with_content(MessageContent::blocks(
            vec![ContentBlock::tool_result(
                "toolu_123",
                "file1.txt\nfile2.txt",
            )],
        ))];
        let result = translate_to_openai(None, &messages);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].role, OpenAiRole::Tool);
        assert_eq!(result[0].content.as_deref(), Some("file1.txt\nfile2.txt"));
        assert_eq!(result[0].tool_call_id.as_deref(), Some("toolu_123"));
    }

    #[test]
    fn test_translate_user_with_multiple_tool_results() {
        let messages = vec![ApiMessageV2::user_with_content(MessageContent::blocks(
            vec![
                ContentBlock::tool_result("toolu_1", "result 1"),
                ContentBlock::tool_result("toolu_2", "result 2"),
            ],
        ))];
        let result = translate_to_openai(None, &messages);

        // Each tool result should be a separate tool message
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].role, OpenAiRole::Tool);
        assert_eq!(result[0].tool_call_id.as_deref(), Some("toolu_1"));
        assert_eq!(result[1].role, OpenAiRole::Tool);
        assert_eq!(result[1].tool_call_id.as_deref(), Some("toolu_2"));
    }

    #[test]
    fn test_translate_user_with_tool_result_error() {
        let messages = vec![ApiMessageV2::user_with_content(MessageContent::blocks(
            vec![ContentBlock::tool_error("toolu_err", "Permission denied")],
        ))];
        let result = translate_to_openai(None, &messages);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].role, OpenAiRole::Tool);
        assert!(result[0]
            .content
            .as_deref()
            .unwrap()
            .contains("Permission denied"));
    }

    #[test]
    fn test_translate_user_with_text_and_tool_results() {
        let messages = vec![ApiMessageV2::user_with_content(MessageContent::blocks(
            vec![
                ContentBlock::tool_result("toolu_1", "output"),
                ContentBlock::text("Now do the next step."),
            ],
        ))];
        let result = translate_to_openai(None, &messages);

        // Tool result first, then user text
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].role, OpenAiRole::Tool);
        assert_eq!(result[0].tool_call_id.as_deref(), Some("toolu_1"));
        assert_eq!(result[1].role, OpenAiRole::User);
        assert_eq!(result[1].content.as_deref(), Some("Now do the next step."));
    }

    // =========================================================================
    // translate_to_openai: Full tool use conversation flow
    // =========================================================================

    #[test]
    fn test_translate_full_tool_use_flow() {
        let messages = vec![
            // User asks to list files
            ApiMessageV2::user("List the files in the current directory."),
            // Assistant wants to use bash
            ApiMessageV2::assistant_with_content(MessageContent::blocks(vec![
                ContentBlock::text("I'll list the files for you."),
                ContentBlock::tool_use("toolu_abc", "bash", json!({"command": "ls"})),
            ])),
            // User sends tool result
            ApiMessageV2::user_with_content(MessageContent::blocks(vec![
                ContentBlock::tool_result("toolu_abc", "README.md\nsrc/\nCargo.toml"),
            ])),
            // Assistant responds with the result
            ApiMessageV2::assistant("Here are the files:\n- README.md\n- src/\n- Cargo.toml"),
        ];
        let result = translate_to_openai(Some("You are a file manager."), &messages);

        assert_eq!(result.len(), 5);
        assert_eq!(result[0].role, OpenAiRole::System);
        assert_eq!(result[1].role, OpenAiRole::User);
        assert_eq!(result[2].role, OpenAiRole::Assistant);
        assert!(result[2].tool_calls.is_some());
        assert_eq!(result[3].role, OpenAiRole::Tool);
        assert_eq!(result[4].role, OpenAiRole::Assistant);
    }

    // =========================================================================
    // translate_to_openai: Image content
    // =========================================================================

    #[test]
    fn test_translate_user_with_image() {
        let messages = vec![ApiMessageV2::user_with_content(MessageContent::blocks(
            vec![
                ContentBlock::text("What's in this image?"),
                ContentBlock::image(ImageSource::Base64 {
                    media_type: "image/png".to_string(),
                    data: "iVBORw0KGgo=".to_string(),
                }),
            ],
        ))];
        let result = translate_to_openai(None, &messages);

        // Image gets a placeholder, combined with text into one user message
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].role, OpenAiRole::User);
        let content = result[0].content.as_deref().unwrap();
        assert!(content.contains("What's in this image?"));
        assert!(content.contains("[image]"));
    }

    // =========================================================================
    // translate_to_openai: Edge cases
    // =========================================================================

    #[test]
    fn test_translate_empty_text_message() {
        let messages = vec![ApiMessageV2::user("")];
        let result = translate_to_openai(None, &messages);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content.as_deref(), Some(""));
    }

    #[test]
    fn test_translate_assistant_empty_blocks() {
        let messages = vec![ApiMessageV2::assistant_with_content(
            MessageContent::blocks(vec![]),
        )];
        let result = translate_to_openai(None, &messages);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].role, OpenAiRole::Assistant);
    }

    #[test]
    fn test_translate_system_prompt_with_special_chars() {
        let result = translate_to_openai(Some("You are \"helpful\"\nand concise.\tAlways."), &[]);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].content.as_deref(),
            Some("You are \"helpful\"\nand concise.\tAlways.")
        );
    }

    // =========================================================================
    // translate_tool_use
    // =========================================================================

    #[test]
    fn test_translate_tool_use_basic() {
        let tool_use = ToolUseBlock::new("toolu_123", "bash", json!({"command": "ls -la"}));
        let call = translate_tool_use(&tool_use);

        assert_eq!(call.id, "toolu_123");
        assert_eq!(call.call_type, "function");
        assert_eq!(call.function.name, "bash");
        // Arguments should be a JSON string
        assert!(call.function.arguments.contains("command"));
        assert!(call.function.arguments.contains("ls -la"));
    }

    #[test]
    fn test_translate_tool_use_empty_input() {
        let tool_use = ToolUseBlock::new("toolu_456", "get_time", json!({}));
        let call = translate_tool_use(&tool_use);

        assert_eq!(call.id, "toolu_456");
        assert_eq!(call.function.name, "get_time");
        assert_eq!(call.function.arguments, "{}");
    }

    #[test]
    fn test_translate_tool_use_complex_input() {
        let input = json!({
            "path": "/home/user/project",
            "recursive": true,
            "filters": ["*.rs", "*.toml"],
            "depth": 3
        });
        let tool_use = ToolUseBlock::new("toolu_789", "list_files", input.clone());
        let call = translate_tool_use(&tool_use);

        // Should round-trip the JSON correctly
        let parsed: serde_json::Value = serde_json::from_str(&call.function.arguments).unwrap();
        assert_eq!(parsed, input);
    }

    #[test]
    fn test_translate_tool_use_preserves_id() {
        let tool_use = ToolUseBlock::new("toolu_custom_id_here", "my_tool", json!({}));
        let call = translate_tool_use(&tool_use);
        assert_eq!(call.id, "toolu_custom_id_here");
    }

    // =========================================================================
    // translate_stream_event: Content deltas
    // =========================================================================

    #[test]
    fn test_translate_stream_content_delta() {
        let chunk = make_content_chunk("Hello");
        let events = translate_stream_event(&chunk);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0], StreamEvent::ContentDelta("Hello".to_string()));
    }

    #[test]
    fn test_translate_stream_empty_content_delta_skipped() {
        let chunk = make_content_chunk("");
        let events = translate_stream_event(&chunk);

        // Empty content deltas should be skipped
        assert!(events.is_empty());
    }

    #[test]
    fn test_translate_stream_role_only_chunk() {
        // First chunk typically has just role, with empty or no content
        let chunk = OpenAiStreamChunk {
            id: "chatcmpl-abc".to_string(),
            object: "chat.completion.chunk".to_string(),
            created: 0,
            model: "model".to_string(),
            choices: vec![OpenAiStreamChoice {
                index: 0,
                delta: OpenAiDelta {
                    role: Some(OpenAiRole::Assistant),
                    content: Some(String::new()),
                    tool_calls: None,
                },
                finish_reason: None,
            }],
            usage: None,
        };
        let events = translate_stream_event(&chunk);
        assert!(events.is_empty());
    }

    // =========================================================================
    // translate_stream_event: Tool call streaming
    // =========================================================================

    #[test]
    fn test_translate_stream_tool_call_start() {
        let chunk = make_tool_start_chunk(0, "call_abc", "bash");
        let events = translate_stream_event(&chunk);

        // Should produce a ToolUseStart event
        assert!(events.iter().any(|e| matches!(
            e,
            StreamEvent::ToolUseStart {
                id,
                name,
                index: 0
            } if id == "call_abc" && name == "bash"
        )));
    }

    #[test]
    fn test_translate_stream_tool_call_arguments_fragment() {
        let chunk = make_tool_args_chunk(0, r#"{"comm"#);
        let events = translate_stream_event(&chunk);

        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            StreamEvent::ToolUseInputDelta {
                index: 0,
                partial_json: r#"{"comm"#.to_string()
            }
        );
    }

    #[test]
    fn test_translate_stream_tool_call_empty_arguments_skipped() {
        let chunk = make_tool_args_chunk(0, "");
        let events = translate_stream_event(&chunk);

        // Empty argument fragments should be skipped
        assert!(events.is_empty());
    }

    #[test]
    fn test_translate_stream_tool_start_with_empty_args() {
        // Tool call start often has name + empty arguments in same chunk
        let chunk = OpenAiStreamChunk {
            id: "id".to_string(),
            object: "chat.completion.chunk".to_string(),
            created: 0,
            model: "m".to_string(),
            choices: vec![OpenAiStreamChoice {
                index: 0,
                delta: OpenAiDelta {
                    role: None,
                    content: None,
                    tool_calls: Some(vec![OpenAiToolCallDelta {
                        index: 0,
                        id: Some("call_xyz".to_string()),
                        call_type: Some("function".to_string()),
                        function: Some(OpenAiFunctionCallDelta {
                            name: Some("read_file".to_string()),
                            arguments: Some(String::new()),
                        }),
                    }]),
                },
                finish_reason: None,
            }],
            usage: None,
        };
        let events = translate_stream_event(&chunk);

        // Should have ToolUseStart but NOT an empty ToolUseInputDelta
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            StreamEvent::ToolUseStart {
                id,
                name,
                index: 0
            } if id == "call_xyz" && name == "read_file"
        ));
    }

    // =========================================================================
    // translate_stream_event: Finish reasons
    // =========================================================================

    #[test]
    fn test_translate_stream_finish_stop() {
        let chunk = make_finish_chunk(OpenAiFinishReason::Stop);
        let events = translate_stream_event(&chunk);

        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            StreamEvent::MessageComplete {
                stop_reason: StopReason::EndTurn
            }
        );
    }

    #[test]
    fn test_translate_stream_finish_tool_calls() {
        let chunk = make_finish_chunk(OpenAiFinishReason::ToolCalls);
        let events = translate_stream_event(&chunk);

        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            StreamEvent::MessageComplete {
                stop_reason: StopReason::ToolUse
            }
        );
    }

    #[test]
    fn test_translate_stream_finish_length() {
        let chunk = make_finish_chunk(OpenAiFinishReason::Length);
        let events = translate_stream_event(&chunk);

        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            StreamEvent::MessageComplete {
                stop_reason: StopReason::MaxTokens
            }
        );
    }

    #[test]
    fn test_translate_stream_finish_content_filter() {
        let chunk = make_finish_chunk(OpenAiFinishReason::ContentFilter);
        let events = translate_stream_event(&chunk);

        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            StreamEvent::MessageComplete {
                stop_reason: StopReason::EndTurn
            }
        );
    }

    // =========================================================================
    // translate_stream_event: Edge cases
    // =========================================================================

    #[test]
    fn test_translate_stream_empty_choices() {
        let chunk = OpenAiStreamChunk {
            id: "id".to_string(),
            object: "chat.completion.chunk".to_string(),
            created: 0,
            model: "m".to_string(),
            choices: vec![],
            usage: None,
        };
        let events = translate_stream_event(&chunk);
        assert!(events.is_empty());
    }

    #[test]
    fn test_translate_stream_multiple_tool_calls_in_single_chunk() {
        let chunk = OpenAiStreamChunk {
            id: "id".to_string(),
            object: "chat.completion.chunk".to_string(),
            created: 0,
            model: "m".to_string(),
            choices: vec![OpenAiStreamChoice {
                index: 0,
                delta: OpenAiDelta {
                    role: None,
                    content: None,
                    tool_calls: Some(vec![
                        OpenAiToolCallDelta {
                            index: 0,
                            id: Some("call_1".to_string()),
                            call_type: Some("function".to_string()),
                            function: Some(OpenAiFunctionCallDelta {
                                name: Some("bash".to_string()),
                                arguments: Some(String::new()),
                            }),
                        },
                        OpenAiToolCallDelta {
                            index: 1,
                            id: Some("call_2".to_string()),
                            call_type: Some("function".to_string()),
                            function: Some(OpenAiFunctionCallDelta {
                                name: Some("read_file".to_string()),
                                arguments: Some(String::new()),
                            }),
                        },
                    ]),
                },
                finish_reason: None,
            }],
            usage: None,
        };
        let events = translate_stream_event(&chunk);

        // Should have two ToolUseStart events
        let starts: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, StreamEvent::ToolUseStart { .. }))
            .collect();
        assert_eq!(starts.len(), 2);
    }

    #[test]
    fn test_translate_stream_full_sequence() {
        // Simulate a complete streaming sequence
        let chunks = [
            // First chunk: role
            OpenAiStreamChunk {
                id: "id".to_string(),
                object: "chat.completion.chunk".to_string(),
                created: 0,
                model: "m".to_string(),
                choices: vec![OpenAiStreamChoice {
                    index: 0,
                    delta: OpenAiDelta {
                        role: Some(OpenAiRole::Assistant),
                        content: Some(String::new()),
                        tool_calls: None,
                    },
                    finish_reason: None,
                }],
                usage: None,
            },
            // Content chunks
            make_content_chunk("Hello"),
            make_content_chunk(", world!"),
            // Finish
            make_finish_chunk(OpenAiFinishReason::Stop),
        ];

        let all_events: Vec<StreamEvent> = chunks.iter().flat_map(translate_stream_event).collect();

        assert_eq!(all_events.len(), 3);
        assert_eq!(
            all_events[0],
            StreamEvent::ContentDelta("Hello".to_string())
        );
        assert_eq!(
            all_events[1],
            StreamEvent::ContentDelta(", world!".to_string())
        );
        assert_eq!(
            all_events[2],
            StreamEvent::MessageComplete {
                stop_reason: StopReason::EndTurn
            }
        );
    }

    #[test]
    fn test_translate_stream_tool_use_full_sequence() {
        let chunks = [
            // Tool call start
            make_tool_start_chunk(0, "call_abc", "bash"),
            // Argument fragments
            make_tool_args_chunk(0, r#"{"com"#),
            make_tool_args_chunk(0, r#"mand"#),
            make_tool_args_chunk(0, r#"":"ls"}"#),
            // Finish with tool_calls
            make_finish_chunk(OpenAiFinishReason::ToolCalls),
        ];

        let all_events: Vec<StreamEvent> = chunks.iter().flat_map(translate_stream_event).collect();

        // ToolUseStart + 3 input deltas + MessageComplete
        assert_eq!(all_events.len(), 5);
        assert!(matches!(
            &all_events[0],
            StreamEvent::ToolUseStart { name, .. } if name == "bash"
        ));
        assert!(matches!(
            &all_events[1],
            StreamEvent::ToolUseInputDelta { .. }
        ));
        assert!(matches!(
            &all_events[2],
            StreamEvent::ToolUseInputDelta { .. }
        ));
        assert!(matches!(
            &all_events[3],
            StreamEvent::ToolUseInputDelta { .. }
        ));
        assert_eq!(
            all_events[4],
            StreamEvent::MessageComplete {
                stop_reason: StopReason::ToolUse
            }
        );
    }

    // =========================================================================
    // translate_to_openai: Output serializes correctly for OpenAI API
    // =========================================================================

    #[test]
    fn test_translated_messages_serialize_to_valid_openai_json() {
        let messages = vec![
            ApiMessageV2::user("Hello"),
            ApiMessageV2::assistant_with_content(MessageContent::blocks(vec![
                ContentBlock::text("Let me help."),
                ContentBlock::tool_use("toolu_1", "bash", json!({"command": "ls"})),
            ])),
            ApiMessageV2::user_with_content(MessageContent::blocks(vec![
                ContentBlock::tool_result("toolu_1", "file1.txt"),
            ])),
        ];
        let result = translate_to_openai(Some("System"), &messages);

        // All messages should serialize cleanly
        for msg in &result {
            let json = serde_json::to_value(msg).unwrap();
            assert!(json.is_object());
            assert!(json.get("role").is_some());
        }
    }

    // =========================================================================
    // Helpers for constructing test stream chunks
    // =========================================================================

    fn make_content_chunk(content: &str) -> OpenAiStreamChunk {
        OpenAiStreamChunk {
            id: "chatcmpl-test".to_string(),
            object: "chat.completion.chunk".to_string(),
            created: 0,
            model: "test-model".to_string(),
            choices: vec![OpenAiStreamChoice {
                index: 0,
                delta: OpenAiDelta {
                    role: None,
                    content: Some(content.to_string()),
                    tool_calls: None,
                },
                finish_reason: None,
            }],
            usage: None,
        }
    }

    fn make_tool_start_chunk(index: u32, id: &str, name: &str) -> OpenAiStreamChunk {
        OpenAiStreamChunk {
            id: "chatcmpl-test".to_string(),
            object: "chat.completion.chunk".to_string(),
            created: 0,
            model: "test-model".to_string(),
            choices: vec![OpenAiStreamChoice {
                index: 0,
                delta: OpenAiDelta {
                    role: None,
                    content: None,
                    tool_calls: Some(vec![OpenAiToolCallDelta {
                        index,
                        id: Some(id.to_string()),
                        call_type: Some("function".to_string()),
                        function: Some(OpenAiFunctionCallDelta {
                            name: Some(name.to_string()),
                            arguments: Some(String::new()),
                        }),
                    }]),
                },
                finish_reason: None,
            }],
            usage: None,
        }
    }

    fn make_tool_args_chunk(index: u32, args: &str) -> OpenAiStreamChunk {
        OpenAiStreamChunk {
            id: "chatcmpl-test".to_string(),
            object: "chat.completion.chunk".to_string(),
            created: 0,
            model: "test-model".to_string(),
            choices: vec![OpenAiStreamChoice {
                index: 0,
                delta: OpenAiDelta {
                    role: None,
                    content: None,
                    tool_calls: Some(vec![OpenAiToolCallDelta {
                        index,
                        id: None,
                        call_type: None,
                        function: Some(OpenAiFunctionCallDelta {
                            name: None,
                            arguments: Some(args.to_string()),
                        }),
                    }]),
                },
                finish_reason: None,
            }],
            usage: None,
        }
    }

    fn make_finish_chunk(reason: OpenAiFinishReason) -> OpenAiStreamChunk {
        OpenAiStreamChunk {
            id: "chatcmpl-test".to_string(),
            object: "chat.completion.chunk".to_string(),
            created: 0,
            model: "test-model".to_string(),
            choices: vec![OpenAiStreamChoice {
                index: 0,
                delta: OpenAiDelta::default(),
                finish_reason: Some(reason),
            }],
            usage: None,
        }
    }

    // =========================================================================
    // OpenRouterProvider: Construction and trait contract
    // =========================================================================

    #[test]
    fn test_openrouter_provider_name() {
        let provider =
            OpenRouterProvider::new(SecretString::from("test-key"), "anthropic/claude-sonnet-4");
        assert_eq!(provider.name(), "openrouter");
    }

    #[test]
    fn test_openrouter_provider_model() {
        let provider =
            OpenRouterProvider::new(SecretString::from("test-key"), "anthropic/claude-sonnet-4");
        assert_eq!(provider.model(), "anthropic/claude-sonnet-4");
    }

    #[test]
    fn test_openrouter_provider_implements_llm_provider() {
        let provider =
            OpenRouterProvider::new(SecretString::from("test-key"), "anthropic/claude-sonnet-4");
        let _trait_obj: &dyn LlmProvider = &provider;
    }

    #[test]
    fn test_openrouter_provider_as_boxed_trait_object() {
        let provider =
            OpenRouterProvider::new(SecretString::from("test-key"), "anthropic/claude-sonnet-4");
        let boxed: Box<dyn LlmProvider> = Box::new(provider);
        assert_eq!(boxed.name(), "openrouter");
        assert_eq!(boxed.model(), "anthropic/claude-sonnet-4");
    }

    #[test]
    fn test_openrouter_provider_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<OpenRouterProvider>();
    }

    #[test]
    fn test_openrouter_provider_is_sync() {
        fn assert_sync<T: Sync>() {}
        assert_sync::<OpenRouterProvider>();
    }

    #[test]
    fn test_openrouter_provider_with_site_url() {
        let provider = OpenRouterProvider::new(SecretString::from("test-key"), "model")
            .with_site_url("https://patina.dev");
        assert_eq!(provider.site_url.as_deref(), Some("https://patina.dev"));
    }

    #[test]
    fn test_openrouter_provider_with_app_name() {
        let provider = OpenRouterProvider::new(SecretString::from("test-key"), "model")
            .with_app_name("Patina");
        assert_eq!(provider.app_name.as_deref(), Some("Patina"));
    }

    // =========================================================================
    // OpenRouterProvider: Integration tests with mock HTTP server
    // =========================================================================

    /// Helper: create a provider pointing at a mock server.
    fn test_provider(base_url: &str) -> OpenRouterProvider {
        OpenRouterProvider::new_with_base_url(
            SecretString::from("test-key"),
            "anthropic/claude-sonnet-4",
            base_url,
        )
    }

    /// Helper: collect all stream events from a provider call.
    async fn collect_provider_events(
        provider: &OpenRouterProvider,
        sse_body: &str,
        mock_server: &MockServer,
    ) -> Vec<StreamEvent> {
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(sse_body, "text/event-stream")
                    .append_header("content-type", "text/event-stream"),
            )
            .mount(mock_server)
            .await;

        let messages = vec![ApiMessageV2::user("Hello")];
        let (tx, mut rx) = mpsc::channel(64);

        provider
            .stream_message(&messages, None, None, &RequestOptions::default(), tx)
            .await
            .expect("stream should succeed");

        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        events
    }

    #[tokio::test]
    async fn test_openrouter_streams_text_content() {
        let mock_server = MockServer::start().await;
        let provider = test_provider(&mock_server.uri());

        let sse_body = "\
data: {\"id\":\"gen-abc\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"anthropic/claude-sonnet-4\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"gen-abc\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"anthropic/claude-sonnet-4\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"gen-abc\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"anthropic/claude-sonnet-4\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" world\"},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"gen-abc\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"anthropic/claude-sonnet-4\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
data: [DONE]\n\n";

        let events = collect_provider_events(&provider, sse_body, &mock_server).await;

        assert!(
            events
                .iter()
                .any(|e| matches!(e, StreamEvent::ContentDelta(s) if s == "Hello")),
            "Expected ContentDelta('Hello'), got: {:?}",
            events
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, StreamEvent::ContentDelta(s) if s == " world")),
            "Expected ContentDelta(' world'), got: {:?}",
            events
        );
        assert!(
            events.iter().any(|e| matches!(
                e,
                StreamEvent::MessageComplete {
                    stop_reason: StopReason::EndTurn
                }
            )),
            "Expected MessageComplete(EndTurn), got: {:?}",
            events
        );
    }

    #[tokio::test]
    async fn test_openrouter_streams_tool_use() {
        let mock_server = MockServer::start().await;
        let provider = test_provider(&mock_server.uri());

        let sse_body = "\
data: {\"id\":\"gen-abc\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":null,\"tool_calls\":[{\"index\":0,\"id\":\"call_123\",\"type\":\"function\",\"function\":{\"name\":\"bash\",\"arguments\":\"\"}}]},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"gen-abc\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"command\\\":\"}}]},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"gen-abc\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"ls\\\"}\"}}]},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"gen-abc\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n\
data: [DONE]\n\n";

        let events = collect_provider_events(&provider, sse_body, &mock_server).await;

        // Should contain ToolUseStart
        assert!(
            events.iter().any(|e| matches!(
                e,
                StreamEvent::ToolUseStart { id, name, .. }
                if id == "call_123" && name == "bash"
            )),
            "Expected ToolUseStart, got: {:?}",
            events
        );

        // Should contain ToolUseInputDelta fragments
        let input_deltas: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, StreamEvent::ToolUseInputDelta { .. }))
            .collect();
        assert!(
            !input_deltas.is_empty(),
            "Expected ToolUseInputDelta events, got: {:?}",
            events
        );

        // Should end with tool_calls finish reason
        assert!(
            events.iter().any(|e| matches!(
                e,
                StreamEvent::MessageComplete {
                    stop_reason: StopReason::ToolUse
                }
            )),
            "Expected MessageComplete(ToolUse), got: {:?}",
            events
        );
    }

    #[tokio::test]
    async fn test_openrouter_sends_authorization_header() {
        let mock_server = MockServer::start().await;
        let provider = OpenRouterProvider::new_with_base_url(
            SecretString::from("sk-or-test-key-123"),
            "model",
            &mock_server.uri(),
        );

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(header("Authorization", "Bearer sk-or-test-key-123"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(
                        "data: {\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
                        "text/event-stream",
                    ),
            )
            .expect(1)
            .mount(&mock_server)
            .await;

        let (tx, _rx) = mpsc::channel(32);
        let messages = vec![ApiMessageV2::user("Hi")];
        provider
            .stream_message(&messages, None, None, &RequestOptions::default(), tx)
            .await
            .expect("should send with auth header");
    }

    #[tokio::test]
    async fn test_openrouter_sends_custom_headers() {
        let mock_server = MockServer::start().await;
        let provider = OpenRouterProvider::new_with_base_url(
            SecretString::from("key"),
            "model",
            &mock_server.uri(),
        )
        .with_site_url("https://patina.dev")
        .with_app_name("Patina");

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(header("HTTP-Referer", "https://patina.dev"))
            .and(header("X-Title", "Patina"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(
                        "data: {\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
                        "text/event-stream",
                    ),
            )
            .expect(1)
            .mount(&mock_server)
            .await;

        let (tx, _rx) = mpsc::channel(32);
        let messages = vec![ApiMessageV2::user("Hi")];
        provider
            .stream_message(&messages, None, None, &RequestOptions::default(), tx)
            .await
            .expect("should send with custom headers");
    }

    #[tokio::test]
    async fn test_openrouter_sends_correct_request_body() {
        let mock_server = MockServer::start().await;
        let provider = test_provider(&mock_server.uri());

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(
                        "data: {\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
                        "text/event-stream",
                    ),
            )
            .mount(&mock_server)
            .await;

        let tools = vec![ToolDefinition::new(
            "bash",
            "Run commands",
            json!({"type": "object", "properties": {}}),
        )];
        let (tx, _rx) = mpsc::channel(32);
        let messages = vec![ApiMessageV2::user("List files")];

        provider
            .stream_message(
                &messages,
                Some(&tools),
                Some(&ToolChoice::Auto),
                &RequestOptions::default(),
                tx,
            )
            .await
            .expect("should send request with tools");

        // Verify the mock received exactly 1 request
        let requests = mock_server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);

        let body: serde_json::Value =
            serde_json::from_slice(&requests[0].body).expect("body should be valid JSON");
        assert_eq!(body["model"], "anthropic/claude-sonnet-4");
        assert_eq!(body["stream"], true);
        assert!(body["messages"].is_array());
        assert!(body["tools"].is_array());
    }

    #[tokio::test]
    async fn test_openrouter_handles_api_error() {
        let mock_server = MockServer::start().await;
        let provider = test_provider(&mock_server.uri());

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(401)
                    .set_body_string("{\"error\":{\"message\":\"Invalid API key\"}}"),
            )
            .mount(&mock_server)
            .await;

        let (tx, mut rx) = mpsc::channel(32);
        let messages = vec![ApiMessageV2::user("Hello")];
        provider
            .stream_message(&messages, None, None, &RequestOptions::default(), tx)
            .await
            .expect("should not error on API error, sends via channel");

        let event = rx.try_recv().expect("should receive error event");
        assert!(event.is_error(), "Expected error event, got: {:?}", event);
        let error_msg = event.error().unwrap();
        assert!(
            error_msg.contains("401"),
            "Error should contain status code, got: {}",
            error_msg
        );
    }

    #[tokio::test]
    async fn test_openrouter_handles_rate_limit_retry() {
        let mock_server = MockServer::start().await;
        let provider = test_provider(&mock_server.uri());

        // First request returns 429, second succeeds
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(429).set_body_string("rate limited"))
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(
                        "data: {\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Retried!\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
                        "text/event-stream",
                    ),
            )
            .mount(&mock_server)
            .await;

        let (tx, mut rx) = mpsc::channel(32);
        let messages = vec![ApiMessageV2::user("Hello")];
        provider
            .stream_message(&messages, None, None, &RequestOptions::default(), tx)
            .await
            .expect("should succeed after retry");

        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }

        assert!(
            events
                .iter()
                .any(|e| matches!(e, StreamEvent::ContentDelta(s) if s == "Retried!")),
            "Expected content after retry, got: {:?}",
            events
        );
    }

    #[tokio::test]
    async fn test_openrouter_malformed_sse_skipped() {
        let mock_server = MockServer::start().await;
        let provider = test_provider(&mock_server.uri());

        let sse_body = "\
data: {\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Before\"},\"finish_reason\":null}]}\n\n\
data: {invalid json}\n\n\
data: {\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"After\"},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
data: [DONE]\n\n";

        let events = collect_provider_events(&provider, sse_body, &mock_server).await;

        let deltas: Vec<_> = events.iter().filter_map(|e| e.content()).collect();
        assert!(
            deltas.contains(&"Before"),
            "Should have content before malformed line"
        );
        assert!(
            deltas.contains(&"After"),
            "Should have content after malformed line"
        );
        assert!(
            !events.iter().any(|e| e.is_error()),
            "Malformed JSON should not produce errors"
        );
    }

    #[tokio::test]
    async fn test_openrouter_done_marker_terminates_stream() {
        let mock_server = MockServer::start().await;
        let provider = test_provider(&mock_server.uri());

        // [DONE] without a finish_reason chunk before it
        let sse_body = "\
data: {\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hi\"},\"finish_reason\":null}]}\n\n\
data: [DONE]\n\n";

        let events = collect_provider_events(&provider, sse_body, &mock_server).await;

        assert!(
            events
                .iter()
                .any(|e| matches!(e, StreamEvent::ContentDelta(s) if s == "Hi")),
            "Should receive content before [DONE]"
        );
    }

    #[tokio::test]
    async fn test_openrouter_produces_identical_events_for_text_response() {
        // Verify that OpenRouterProvider produces the same StreamEvent sequence
        // as AnthropicProvider would for an equivalent text response.
        let mock_server = MockServer::start().await;
        let provider = test_provider(&mock_server.uri());

        let sse_body = "\
data: {\"id\":\"gen-abc\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"gen-abc\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"The answer is 42.\"},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"gen-abc\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
data: [DONE]\n\n";

        let events = collect_provider_events(&provider, sse_body, &mock_server).await;

        // Should contain exactly: ContentDelta + MessageComplete
        assert_eq!(events.len(), 2, "Expected 2 events, got: {:?}", events);
        assert_eq!(
            events[0],
            StreamEvent::ContentDelta("The answer is 42.".to_string())
        );
        assert_eq!(
            events[1],
            StreamEvent::MessageComplete {
                stop_reason: StopReason::EndTurn,
            }
        );
    }

    #[tokio::test]
    async fn test_openrouter_request_body_omits_tools_when_none() {
        let mock_server = MockServer::start().await;
        let provider = test_provider(&mock_server.uri());

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(
                        "data: {\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
                        "text/event-stream",
                    ),
            )
            .mount(&mock_server)
            .await;

        let (tx, _rx) = mpsc::channel(32);
        let messages = vec![ApiMessageV2::user("Hello")];
        provider
            .stream_message(&messages, None, None, &RequestOptions::default(), tx)
            .await
            .expect("should send request without tools");

        let requests = mock_server.received_requests().await.unwrap();
        let body: serde_json::Value =
            serde_json::from_slice(&requests[0].body).expect("body should be valid JSON");
        assert!(
            body.get("tools").is_none(),
            "tools should be absent when None"
        );
        assert!(
            body.get("tool_choice").is_none(),
            "tool_choice should be absent when None"
        );
    }

    // =========================================================================
    // translate_stream_event: Tool call safety (no unwrap panics)
    // =========================================================================

    #[test]
    fn test_translate_tool_call_start_missing_id_no_panic() {
        // Tool call delta with function but no id — should not emit ToolUseStart
        let chunk = OpenAiStreamChunk {
            id: "chunk1".to_string(),
            object: "chat.completion.chunk".to_string(),
            created: 0,
            model: "test".to_string(),
            choices: vec![OpenAiStreamChoice {
                index: 0,
                delta: OpenAiDelta {
                    role: None,
                    content: None,
                    tool_calls: Some(vec![OpenAiToolCallDelta {
                        index: 0,
                        id: None, // Missing id
                        call_type: Some("function".to_string()),
                        function: Some(OpenAiFunctionCallDelta {
                            name: Some("test_tool".to_string()),
                            arguments: None,
                        }),
                    }]),
                },
                finish_reason: None,
            }],
            usage: None,
        };

        // Should not panic — just won't emit ToolUseStart
        let events = translate_stream_event(&chunk);
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, StreamEvent::ToolUseStart { .. })),
            "Should not emit ToolUseStart without id"
        );
    }

    #[test]
    fn test_translate_tool_call_start_missing_function_no_panic() {
        // Tool call delta with id but no function — should not emit ToolUseStart
        let chunk = OpenAiStreamChunk {
            id: "chunk1".to_string(),
            object: "chat.completion.chunk".to_string(),
            created: 0,
            model: "test".to_string(),
            choices: vec![OpenAiStreamChoice {
                index: 0,
                delta: OpenAiDelta {
                    role: None,
                    content: None,
                    tool_calls: Some(vec![OpenAiToolCallDelta {
                        index: 0,
                        id: Some("call_123".to_string()),
                        call_type: Some("function".to_string()),
                        function: None, // Missing function
                    }]),
                },
                finish_reason: None,
            }],
            usage: None,
        };

        let events = translate_stream_event(&chunk);
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, StreamEvent::ToolUseStart { .. })),
            "Should not emit ToolUseStart without function"
        );
    }
}
