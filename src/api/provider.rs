//! LLM provider abstraction trait.
//!
//! This module defines the [`LlmProvider`] trait that abstracts over different
//! LLM API providers (Anthropic, OpenRouter, etc.). All providers share the same
//! streaming interface using [`StreamEvent`] channels.
//!
//! # Example
//!
//! ```rust,ignore
//! use patina::api::provider::LlmProvider;
//! use patina::types::ApiMessageV2;
//! use tokio::sync::mpsc;
//!
//! async fn send_message(provider: &dyn LlmProvider) {
//!     let messages = vec![ApiMessageV2::user("Hello!")];
//!     let (tx, mut rx) = mpsc::channel(32);
//!     provider.stream_message(&messages, None, None, tx).await.unwrap();
//!     while let Some(event) = rx.recv().await {
//!         println!("{:?}", event);
//!     }
//! }
//! ```

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use tokio::sync::mpsc;

use crate::api::tools::{ToolChoice, ToolDefinition};
use crate::types::{ApiMessageV2, StreamEvent};

/// Trait abstracting over LLM API providers.
///
/// Implementors provide streaming message completion with optional tool support.
/// All providers emit [`StreamEvent`]s through an mpsc channel, allowing the
/// application layer to remain provider-agnostic.
///
/// # Required Bounds
///
/// Implementations must be `Send + Sync` to allow use across async task
/// boundaries (e.g., `tokio::spawn`).
///
/// # Errors
///
/// [`stream_message`](LlmProvider::stream_message) returns an error if the
/// provider fails to initiate the streaming request (network error, auth
/// failure, etc.). Individual stream errors are delivered as
/// [`StreamEvent::Error`] through the channel.
///
/// # Examples
///
/// ```rust,ignore
/// use patina::api::provider::LlmProvider;
///
/// struct MyProvider;
///
/// impl LlmProvider for MyProvider {
///     fn name(&self) -> &str { "my-provider" }
///     fn model(&self) -> &str { "my-model" }
///     // ...
/// }
/// ```
pub trait LlmProvider: Send + Sync {
    /// Returns the provider name (e.g., "anthropic", "openrouter").
    fn name(&self) -> &str;

    /// Returns the model identifier (e.g., "claude-sonnet-4-20250514").
    fn model(&self) -> &str;

    /// Sends a streaming message request with optional tools.
    ///
    /// Events are sent through `tx` as they arrive from the provider.
    /// The channel is dropped when streaming completes (success or failure).
    ///
    /// # Arguments
    ///
    /// * `messages` - The conversation history
    /// * `tools` - Optional tool definitions the model can use
    /// * `tool_choice` - Optional constraint on tool selection
    /// * `tx` - Channel sender for streaming events
    ///
    /// # Errors
    ///
    /// Returns an error if the request cannot be initiated. Stream-level
    /// errors are sent as [`StreamEvent::Error`] through the channel.
    fn stream_message<'a>(
        &'a self,
        messages: &'a [ApiMessageV2],
        tools: Option<&'a [ToolDefinition]>,
        tool_choice: Option<&'a ToolChoice>,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
}

impl LlmProvider for super::AnthropicClient {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn model(&self) -> &str {
        super::AnthropicClient::model(self)
    }

    fn stream_message<'a>(
        &'a self,
        messages: &'a [ApiMessageV2],
        tools: Option<&'a [ToolDefinition]>,
        tool_choice: Option<&'a ToolChoice>,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(self.stream_message_v2_with_tools(messages, tools, tool_choice, tx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::content::StopReason;

    /// A mock provider for testing the trait contract.
    struct MockProvider {
        provider_name: String,
        model_name: String,
        /// Events to send when `stream_message` is called.
        events: Vec<StreamEvent>,
    }

    impl MockProvider {
        fn new(name: &str, model: &str, events: Vec<StreamEvent>) -> Self {
            Self {
                provider_name: name.to_string(),
                model_name: model.to_string(),
                events,
            }
        }
    }

    impl LlmProvider for MockProvider {
        fn name(&self) -> &str {
            &self.provider_name
        }

        fn model(&self) -> &str {
            &self.model_name
        }

        fn stream_message<'a>(
            &'a self,
            _messages: &'a [ApiMessageV2],
            _tools: Option<&'a [ToolDefinition]>,
            _tool_choice: Option<&'a ToolChoice>,
            tx: mpsc::Sender<StreamEvent>,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
            Box::pin(async move {
                for event in &self.events {
                    tx.send(event.clone())
                        .await
                        .map_err(|e| anyhow::anyhow!("channel send failed: {}", e))?;
                }
                Ok(())
            })
        }
    }

    // =========================================================================
    // Trait contract: name() and model()
    // =========================================================================

    #[test]
    fn test_provider_name_returns_correct_value() {
        let provider = MockProvider::new("anthropic", "claude-sonnet-4", vec![]);
        assert_eq!(provider.name(), "anthropic");
    }

    #[test]
    fn test_provider_model_returns_correct_value() {
        let provider = MockProvider::new("anthropic", "claude-sonnet-4", vec![]);
        assert_eq!(provider.model(), "claude-sonnet-4");
    }

    #[test]
    fn test_provider_name_openrouter() {
        let provider = MockProvider::new("openrouter", "anthropic/claude-sonnet-4", vec![]);
        assert_eq!(provider.name(), "openrouter");
        assert_eq!(provider.model(), "anthropic/claude-sonnet-4");
    }

    // =========================================================================
    // Trait contract: stream_message() returns expected events
    // =========================================================================

    #[tokio::test]
    async fn test_stream_message_sends_content_delta() {
        let events = vec![
            StreamEvent::ContentDelta("Hello".to_string()),
            StreamEvent::MessageComplete {
                stop_reason: StopReason::EndTurn,
            },
        ];
        let provider = MockProvider::new("test", "test-model", events);

        let (tx, mut rx) = mpsc::channel(32);
        let messages = vec![ApiMessageV2::user("Hi")];
        provider
            .stream_message(&messages, None, None, tx)
            .await
            .expect("stream should succeed");

        let first = rx.recv().await.expect("should receive content delta");
        assert_eq!(first, StreamEvent::ContentDelta("Hello".to_string()));

        let second = rx.recv().await.expect("should receive message complete");
        assert_eq!(
            second,
            StreamEvent::MessageComplete {
                stop_reason: StopReason::EndTurn
            }
        );

        assert!(rx.recv().await.is_none(), "channel should be closed");
    }

    #[tokio::test]
    async fn test_stream_message_sends_tool_use_events() {
        let events = vec![
            StreamEvent::ToolUseStart {
                id: "toolu_01".to_string(),
                name: "bash".to_string(),
                index: 0,
            },
            StreamEvent::ToolUseInputDelta {
                index: 0,
                partial_json: r#"{"command":"ls"}"#.to_string(),
            },
            StreamEvent::ToolUseComplete { index: 0 },
            StreamEvent::MessageComplete {
                stop_reason: StopReason::ToolUse,
            },
        ];
        let provider = MockProvider::new("test", "test-model", events);

        let (tx, mut rx) = mpsc::channel(32);
        let messages = vec![ApiMessageV2::user("List files")];
        provider
            .stream_message(&messages, None, None, tx)
            .await
            .expect("stream should succeed");

        // Verify all 4 events arrive in order
        let e1 = rx.recv().await.unwrap();
        assert!(e1.is_tool_use());

        let e2 = rx.recv().await.unwrap();
        assert!(e2.is_tool_use());

        let e3 = rx.recv().await.unwrap();
        assert!(e3.is_tool_use());

        let e4 = rx.recv().await.unwrap();
        assert_eq!(e4.stop_reason(), Some(StopReason::ToolUse));
    }

    #[tokio::test]
    async fn test_stream_message_with_tools_parameter() {
        let events = vec![StreamEvent::MessageComplete {
            stop_reason: StopReason::EndTurn,
        }];
        let provider = MockProvider::new("test", "test-model", events);

        let tools = vec![ToolDefinition::new(
            "bash",
            "Run commands",
            serde_json::json!({"type": "object", "properties": {}}),
        )];
        let (tx, mut rx) = mpsc::channel(32);
        let messages = vec![ApiMessageV2::user("Hello")];
        provider
            .stream_message(&messages, Some(&tools), Some(&ToolChoice::Auto), tx)
            .await
            .expect("stream should succeed");

        let event = rx.recv().await.unwrap();
        assert!(event.is_stop());
    }

    #[tokio::test]
    async fn test_stream_message_error_event() {
        let events = vec![StreamEvent::Error("rate limited".to_string())];
        let provider = MockProvider::new("test", "test-model", events);

        let (tx, mut rx) = mpsc::channel(32);
        let messages = vec![ApiMessageV2::user("Hello")];
        provider
            .stream_message(&messages, None, None, tx)
            .await
            .expect("stream should succeed even when sending error events");

        let event = rx.recv().await.unwrap();
        assert!(event.is_error());
        assert_eq!(event.error(), Some("rate limited"));
    }

    // =========================================================================
    // Trait contract: Send + Sync bounds
    // =========================================================================

    #[test]
    fn test_provider_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<MockProvider>();
    }

    #[test]
    fn test_provider_is_sync() {
        fn assert_sync<T: Sync>() {}
        assert_sync::<MockProvider>();
    }

    /// Verifies that `Box<dyn LlmProvider>` can be stored and used,
    /// which is how the app layer will hold providers.
    #[tokio::test]
    async fn test_provider_as_trait_object() {
        let events = vec![
            StreamEvent::ContentDelta("boxed".to_string()),
            StreamEvent::MessageComplete {
                stop_reason: StopReason::EndTurn,
            },
        ];
        let provider: Box<dyn LlmProvider> =
            Box::new(MockProvider::new("test", "test-model", events));

        assert_eq!(provider.name(), "test");
        assert_eq!(provider.model(), "test-model");

        let (tx, mut rx) = mpsc::channel(32);
        let messages = vec![ApiMessageV2::user("Hello")];
        provider
            .stream_message(&messages, None, None, tx)
            .await
            .expect("boxed provider should work");

        let event = rx.recv().await.unwrap();
        assert_eq!(event, StreamEvent::ContentDelta("boxed".to_string()));
    }

    #[tokio::test]
    async fn test_stream_message_empty_messages() {
        let events = vec![StreamEvent::MessageComplete {
            stop_reason: StopReason::EndTurn,
        }];
        let provider = MockProvider::new("test", "test-model", events);

        let (tx, mut rx) = mpsc::channel(32);
        provider
            .stream_message(&[], None, None, tx)
            .await
            .expect("empty messages should not panic");

        let event = rx.recv().await.unwrap();
        assert!(event.is_stop());
    }

    #[tokio::test]
    async fn test_stream_message_mixed_content_and_tool_events() {
        let events = vec![
            StreamEvent::ContentDelta("I'll run a command.".to_string()),
            StreamEvent::ContentBlockComplete { index: 0 },
            StreamEvent::ToolUseStart {
                id: "toolu_01".to_string(),
                name: "bash".to_string(),
                index: 1,
            },
            StreamEvent::ToolUseInputDelta {
                index: 1,
                partial_json: r#"{"command":"pwd"}"#.to_string(),
            },
            StreamEvent::ToolUseComplete { index: 1 },
            StreamEvent::MessageComplete {
                stop_reason: StopReason::ToolUse,
            },
        ];
        let provider = MockProvider::new("test", "test-model", events.clone());

        let (tx, mut rx) = mpsc::channel(32);
        let messages = vec![ApiMessageV2::user("Where am I?")];
        provider
            .stream_message(&messages, None, None, tx)
            .await
            .expect("stream should succeed");

        let mut received = Vec::new();
        while let Some(event) = rx.recv().await {
            received.push(event);
        }
        assert_eq!(received, events);
    }

    // =========================================================================
    // AnthropicClient as LlmProvider (Phase 2.2)
    // =========================================================================

    #[test]
    fn test_anthropic_client_implements_llm_provider() {
        use crate::api::AnthropicClient;
        use secrecy::SecretString;

        let client = AnthropicClient::new(SecretString::from("test-key"), "claude-sonnet-4");

        // AnthropicClient must implement LlmProvider
        let _provider: &dyn LlmProvider = &client;
    }

    #[test]
    fn test_anthropic_provider_name() {
        use crate::api::AnthropicClient;
        use secrecy::SecretString;

        let client = AnthropicClient::new(SecretString::from("test-key"), "claude-sonnet-4");
        let provider: &dyn LlmProvider = &client;

        assert_eq!(provider.name(), "anthropic");
    }

    #[test]
    fn test_anthropic_provider_model() {
        use crate::api::AnthropicClient;
        use secrecy::SecretString;

        let client = AnthropicClient::new(SecretString::from("test-key"), "claude-sonnet-4");
        let provider: &dyn LlmProvider = &client;

        assert_eq!(provider.model(), "claude-sonnet-4");
    }

    #[test]
    fn test_anthropic_provider_model_custom() {
        use crate::api::AnthropicClient;
        use secrecy::SecretString;

        let client = AnthropicClient::new(SecretString::from("test-key"), "claude-opus-4-20250514");
        let provider: &dyn LlmProvider = &client;

        assert_eq!(provider.model(), "claude-opus-4-20250514");
    }

    #[test]
    fn test_anthropic_provider_as_boxed_trait_object() {
        use crate::api::AnthropicClient;
        use secrecy::SecretString;

        let client = AnthropicClient::new(SecretString::from("test-key"), "claude-sonnet-4");
        let provider: Box<dyn LlmProvider> = Box::new(client);

        assert_eq!(provider.name(), "anthropic");
        assert_eq!(provider.model(), "claude-sonnet-4");
    }

    #[test]
    fn test_anthropic_provider_as_arc_trait_object() {
        use crate::api::AnthropicClient;
        use secrecy::SecretString;
        use std::sync::Arc;

        let client = AnthropicClient::new(SecretString::from("test-key"), "claude-sonnet-4");
        let provider: Arc<dyn LlmProvider> = Arc::new(client);

        assert_eq!(provider.name(), "anthropic");
        assert_eq!(provider.model(), "claude-sonnet-4");

        // Arc cloning should work for spawning patterns
        let provider2 = Arc::clone(&provider);
        assert_eq!(provider2.name(), "anthropic");
    }

    #[tokio::test]
    async fn test_anthropic_provider_stream_message_delegates() {
        use crate::api::AnthropicClient;
        use secrecy::SecretString;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        // Set up mock to return a simple SSE stream
        let sse_response = r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello from provider"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"}}

event: message_stop
data: {"type":"message_stop"}

"#;

        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(sse_response, "text/event-stream")
                    .append_header("content-type", "text/event-stream"),
            )
            .mount(&mock_server)
            .await;

        let client = AnthropicClient::new_with_base_url(
            SecretString::from("test-key"),
            "claude-sonnet-4",
            &mock_server.uri(),
        );

        // Use through the LlmProvider trait
        let provider: &dyn LlmProvider = &client;

        let (tx, mut rx) = mpsc::channel(32);
        let messages = vec![ApiMessageV2::user("Hello")];

        provider
            .stream_message(&messages, None, None, tx)
            .await
            .expect("stream_message should succeed via trait");

        // Verify we get the expected events
        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }

        // Should contain the content delta
        assert!(
            events
                .iter()
                .any(|e| matches!(e, StreamEvent::ContentDelta(s) if s == "Hello from provider")),
            "Expected ContentDelta('Hello from provider'), got: {:?}",
            events
        );

        // Should contain MessageComplete with EndTurn
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
    async fn test_anthropic_provider_stream_message_with_tools() {
        use crate::api::AnthropicClient;
        use secrecy::SecretString;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        let sse_response = r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_abc","name":"bash"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"command\":\"ls\"}"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use"}}

event: message_stop
data: {"type":"message_stop"}

"#;

        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(sse_response, "text/event-stream")
                    .append_header("content-type", "text/event-stream"),
            )
            .mount(&mock_server)
            .await;

        let client = AnthropicClient::new_with_base_url(
            SecretString::from("test-key"),
            "claude-sonnet-4",
            &mock_server.uri(),
        );

        let provider: &dyn LlmProvider = &client;

        let tools = vec![crate::api::tools::ToolDefinition::new(
            "bash",
            "Run commands",
            serde_json::json!({"type": "object", "properties": {}}),
        )];

        let (tx, mut rx) = mpsc::channel(32);
        let messages = vec![ApiMessageV2::user("List files")];

        provider
            .stream_message(&messages, Some(&tools), Some(&ToolChoice::Auto), tx)
            .await
            .expect("stream_message with tools should succeed");

        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }

        // Should produce the same tool use events as the direct client
        assert!(
            events
                .iter()
                .any(|e| matches!(e, StreamEvent::ToolUseStart { name, .. } if name == "bash")),
            "Expected ToolUseStart with name 'bash', got: {:?}",
            events
        );
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
}
