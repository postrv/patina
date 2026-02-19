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
/// Creates an LLM provider based on the application configuration.
///
/// Reads the [`ProviderConfig`](crate::types::config::ProviderConfig) from the
/// given [`Config`](crate::types::Config) and constructs the appropriate provider:
///
/// - [`ProviderKind::Anthropic`](crate::types::config::ProviderKind::Anthropic): Creates
///   an [`AnthropicClient`](super::AnthropicClient) using `config.api_key` and `config.model`.
/// - [`ProviderKind::OpenRouter`](crate::types::config::ProviderKind::OpenRouter): Creates
///   an [`OpenRouterProvider`](crate::api::providers::openrouter::OpenRouterProvider) using
///   the OpenRouter API key and model from the provider config.
///
/// # Arguments
///
/// * `config` - The application configuration containing provider settings.
///
/// # Panics
///
/// Panics if the provider config is invalid (e.g., OpenRouter without an API key).
/// Call [`ProviderConfig::validate()`](crate::types::config::ProviderConfig::validate)
/// before calling this function to handle errors gracefully.
///
/// # Examples
///
/// ```rust,ignore
/// use patina::api::provider::create_provider;
/// use patina::types::config::Config;
/// use secrecy::SecretString;
///
/// let config = Config::new(
///     SecretString::from("sk-ant-..."),
///     "claude-sonnet-4",
///     std::path::PathBuf::from("."),
/// );
/// let provider = create_provider(&config);
/// assert_eq!(provider.name(), "anthropic");
/// ```
#[must_use]
pub fn create_provider(config: &crate::types::Config) -> Box<dyn LlmProvider> {
    use crate::types::config::ProviderKind;

    match config.provider.kind {
        ProviderKind::Anthropic => Box::new(super::AnthropicClient::new(
            config.api_key.clone(),
            &config.model,
        )),
        ProviderKind::OpenRouter => {
            use crate::api::providers::openrouter::OpenRouterProvider;

            let api_key = config
                .provider
                .openrouter_api_key
                .clone()
                .expect("OpenRouter provider requires an API key");
            let model = config
                .provider
                .openrouter_model
                .as_deref()
                .expect("OpenRouter provider requires a model identifier");

            let mut provider = OpenRouterProvider::new(api_key, model);

            if let Some(ref url) = config.provider.site_url {
                provider = provider.with_site_url(url.clone());
            }
            if let Some(ref name) = config.provider.app_name {
                provider = provider.with_app_name(name.clone());
            }

            Box::new(provider)
        }
    }
}

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

/// A provider that tries multiple providers in order, falling back on failure.
///
/// When [`stream_message`](LlmProvider::stream_message) is called, the
/// `FallbackProvider` attempts each inner provider in sequence. If a provider's
/// `stream_message` returns an error, the next provider is tried. If all
/// providers fail, a combined error is returned.
///
/// Mid-stream errors (delivered as [`StreamEvent::Error`]) do **not** trigger
/// fallback — only initial connection/request failures do.
///
/// # Examples
///
/// ```rust,ignore
/// use patina::api::provider::{FallbackProvider, LlmProvider};
///
/// let primary: Box<dyn LlmProvider> = /* ... */;
/// let backup: Box<dyn LlmProvider> = /* ... */;
/// let provider = FallbackProvider::new(vec![primary, backup]);
/// assert_eq!(provider.name(), "fallback");
/// ```
pub struct FallbackProvider {
    providers: Vec<Box<dyn LlmProvider>>,
}

impl FallbackProvider {
    /// Creates a new fallback provider from a list of providers.
    ///
    /// Providers are tried in order — the first provider is the primary,
    /// subsequent providers are fallbacks.
    ///
    /// # Panics
    ///
    /// Panics if `providers` is empty. A fallback chain must have at least
    /// one provider.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use patina::api::provider::FallbackProvider;
    ///
    /// let provider = FallbackProvider::new(vec![primary, backup]);
    /// ```
    #[must_use]
    pub fn new(providers: Vec<Box<dyn LlmProvider>>) -> Self {
        assert!(!providers.is_empty(), "FallbackProvider requires at least one provider");
        Self { providers }
    }

    /// Returns the number of providers in the fallback chain.
    #[must_use]
    pub fn provider_count(&self) -> usize {
        self.providers.len()
    }
}

impl LlmProvider for FallbackProvider {
    fn name(&self) -> &str {
        "fallback"
    }

    fn model(&self) -> &str {
        self.providers[0].model()
    }

    fn stream_message<'a>(
        &'a self,
        messages: &'a [ApiMessageV2],
        tools: Option<&'a [ToolDefinition]>,
        tool_choice: Option<&'a ToolChoice>,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let mut errors = Vec::new();

            for (i, provider) in self.providers.iter().enumerate() {
                match provider
                    .stream_message(messages, tools, tool_choice, tx.clone())
                    .await
                {
                    Ok(()) => return Ok(()),
                    Err(e) => {
                        let msg = format!("{} ({}): {e}", provider.name(), provider.model());
                        if i + 1 < self.providers.len() {
                            tracing::warn!(
                                provider = provider.name(),
                                model = provider.model(),
                                error = %e,
                                next_provider = self.providers[i + 1].name(),
                                "Provider failed, falling back to next provider"
                            );
                        }
                        errors.push(msg);
                    }
                }
            }

            anyhow::bail!(
                "All {} providers failed: {}",
                errors.len(),
                errors.join("; ")
            )
        })
    }
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

    // =========================================================================
    // Phase 2.6: create_provider factory tests
    // =========================================================================

    #[test]
    fn test_create_provider_default_anthropic() {
        use secrecy::SecretString;

        let config = crate::types::Config::new(
            SecretString::from("sk-ant-test"),
            "claude-sonnet-4",
            std::path::PathBuf::from("."),
        );

        let provider = create_provider(&config);
        assert_eq!(provider.name(), "anthropic");
        assert_eq!(provider.model(), "claude-sonnet-4");
    }

    #[test]
    fn test_create_provider_explicit_anthropic() {
        use crate::types::config::ProviderConfig;
        use secrecy::SecretString;

        let config = crate::types::Config::new(
            SecretString::from("sk-ant-test"),
            "claude-opus-4-20250514",
            std::path::PathBuf::from("."),
        )
        .with_provider(ProviderConfig::anthropic());

        let provider = create_provider(&config);
        assert_eq!(provider.name(), "anthropic");
        assert_eq!(provider.model(), "claude-opus-4-20250514");
    }

    #[test]
    fn test_create_provider_openrouter() {
        use crate::types::config::ProviderConfig;
        use secrecy::SecretString;

        let config = crate::types::Config::new(
            SecretString::from("sk-ant-test"),
            "claude-sonnet-4",
            std::path::PathBuf::from("."),
        )
        .with_provider(ProviderConfig::openrouter(
            SecretString::from("sk-or-test"),
            "anthropic/claude-sonnet-4",
        ));

        let provider = create_provider(&config);
        assert_eq!(provider.name(), "openrouter");
        assert_eq!(provider.model(), "anthropic/claude-sonnet-4");
    }

    #[test]
    fn test_create_provider_openrouter_with_metadata() {
        use crate::types::config::ProviderConfig;
        use secrecy::SecretString;

        let config = crate::types::Config::new(
            SecretString::from("sk-ant-test"),
            "claude-sonnet-4",
            std::path::PathBuf::from("."),
        )
        .with_provider(
            ProviderConfig::openrouter(
                SecretString::from("sk-or-test"),
                "anthropic/claude-sonnet-4",
            )
            .with_site_url("https://patina.dev")
            .with_app_name("Patina"),
        );

        let provider = create_provider(&config);
        assert_eq!(provider.name(), "openrouter");
    }

    // =========================================================================
    // Phase 2.7: FallbackProvider tests
    // =========================================================================

    /// A mock provider that always fails `stream_message` with the given error.
    struct FailingProvider {
        provider_name: String,
        model_name: String,
        error_msg: String,
    }

    impl FailingProvider {
        fn new(name: &str, model: &str, error_msg: &str) -> Self {
            Self {
                provider_name: name.to_string(),
                model_name: model.to_string(),
                error_msg: error_msg.to_string(),
            }
        }
    }

    impl LlmProvider for FailingProvider {
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
            _tx: mpsc::Sender<StreamEvent>,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
            Box::pin(async move { anyhow::bail!("{}", self.error_msg) })
        }
    }

    // --- Construction ---

    #[test]
    fn test_fallback_provider_name() {
        let providers: Vec<Box<dyn LlmProvider>> = vec![
            Box::new(MockProvider::new("anthropic", "claude-sonnet-4", vec![])),
        ];
        let fallback = FallbackProvider::new(providers);
        assert_eq!(fallback.name(), "fallback");
    }

    #[test]
    fn test_fallback_provider_model_returns_primary() {
        let providers: Vec<Box<dyn LlmProvider>> = vec![
            Box::new(MockProvider::new("anthropic", "claude-sonnet-4", vec![])),
            Box::new(MockProvider::new("openrouter", "anthropic/claude-sonnet-4", vec![])),
        ];
        let fallback = FallbackProvider::new(providers);
        assert_eq!(fallback.model(), "claude-sonnet-4");
    }

    #[test]
    fn test_fallback_provider_count() {
        let providers: Vec<Box<dyn LlmProvider>> = vec![
            Box::new(MockProvider::new("a", "m1", vec![])),
            Box::new(MockProvider::new("b", "m2", vec![])),
        ];
        let fallback = FallbackProvider::new(providers);
        assert_eq!(fallback.provider_count(), 2);
    }

    #[test]
    #[should_panic(expected = "FallbackProvider requires at least one provider")]
    fn test_fallback_provider_empty_panics() {
        let providers: Vec<Box<dyn LlmProvider>> = vec![];
        let _ = FallbackProvider::new(providers);
    }

    #[test]
    fn test_fallback_provider_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<FallbackProvider>();
    }

    // --- First provider succeeds (no fallback) ---

    #[tokio::test]
    async fn test_fallback_first_provider_succeeds() {
        let events = vec![
            StreamEvent::ContentDelta("Hello".to_string()),
            StreamEvent::MessageComplete {
                stop_reason: StopReason::EndTurn,
            },
        ];
        let providers: Vec<Box<dyn LlmProvider>> = vec![
            Box::new(MockProvider::new("primary", "model-1", events.clone())),
            Box::new(MockProvider::new("backup", "model-2", vec![])),
        ];
        let fallback = FallbackProvider::new(providers);

        let (tx, mut rx) = mpsc::channel(32);
        let messages = vec![ApiMessageV2::user("Hi")];
        fallback
            .stream_message(&messages, None, None, tx)
            .await
            .expect("should succeed with first provider");

        let mut received = Vec::new();
        while let Some(event) = rx.recv().await {
            received.push(event);
        }
        assert_eq!(received, events);
    }

    // --- First fails, second succeeds ---

    #[tokio::test]
    async fn test_fallback_first_fails_second_succeeds() {
        let events = vec![
            StreamEvent::ContentDelta("From backup".to_string()),
            StreamEvent::MessageComplete {
                stop_reason: StopReason::EndTurn,
            },
        ];
        let providers: Vec<Box<dyn LlmProvider>> = vec![
            Box::new(FailingProvider::new("primary", "model-1", "connection refused")),
            Box::new(MockProvider::new("backup", "model-2", events.clone())),
        ];
        let fallback = FallbackProvider::new(providers);

        let (tx, mut rx) = mpsc::channel(32);
        let messages = vec![ApiMessageV2::user("Hi")];
        fallback
            .stream_message(&messages, None, None, tx)
            .await
            .expect("should succeed with fallback provider");

        let mut received = Vec::new();
        while let Some(event) = rx.recv().await {
            received.push(event);
        }
        assert_eq!(received, events);
    }

    // --- All providers fail ---

    #[tokio::test]
    async fn test_fallback_all_providers_fail() {
        let providers: Vec<Box<dyn LlmProvider>> = vec![
            Box::new(FailingProvider::new("primary", "m1", "timeout")),
            Box::new(FailingProvider::new("backup", "m2", "auth failed")),
        ];
        let fallback = FallbackProvider::new(providers);

        let (tx, _rx) = mpsc::channel(32);
        let messages = vec![ApiMessageV2::user("Hi")];
        let result = fallback.stream_message(&messages, None, None, tx).await;

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("timeout"),
            "error should mention first failure: {err_msg}"
        );
        assert!(
            err_msg.contains("auth failed"),
            "error should mention second failure: {err_msg}"
        );
    }

    // --- Single provider fallback ---

    #[tokio::test]
    async fn test_fallback_single_provider_succeeds() {
        let events = vec![StreamEvent::MessageComplete {
            stop_reason: StopReason::EndTurn,
        }];
        let providers: Vec<Box<dyn LlmProvider>> = vec![
            Box::new(MockProvider::new("only", "model", events.clone())),
        ];
        let fallback = FallbackProvider::new(providers);

        let (tx, mut rx) = mpsc::channel(32);
        let messages = vec![ApiMessageV2::user("Hi")];
        fallback
            .stream_message(&messages, None, None, tx)
            .await
            .expect("single provider should succeed");

        let event = rx.recv().await.unwrap();
        assert_eq!(event, events[0]);
    }

    #[tokio::test]
    async fn test_fallback_single_provider_fails() {
        let providers: Vec<Box<dyn LlmProvider>> = vec![
            Box::new(FailingProvider::new("only", "model", "network error")),
        ];
        let fallback = FallbackProvider::new(providers);

        let (tx, _rx) = mpsc::channel(32);
        let messages = vec![ApiMessageV2::user("Hi")];
        let result = fallback.stream_message(&messages, None, None, tx).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("network error"));
    }

    // --- Fallback as trait object ---

    #[tokio::test]
    async fn test_fallback_provider_as_boxed_trait_object() {
        let events = vec![StreamEvent::MessageComplete {
            stop_reason: StopReason::EndTurn,
        }];
        let providers: Vec<Box<dyn LlmProvider>> = vec![
            Box::new(MockProvider::new("primary", "model", events)),
        ];
        let provider: Box<dyn LlmProvider> = Box::new(FallbackProvider::new(providers));

        assert_eq!(provider.name(), "fallback");
        assert_eq!(provider.model(), "model");

        let (tx, mut rx) = mpsc::channel(32);
        let messages = vec![ApiMessageV2::user("Hi")];
        provider
            .stream_message(&messages, None, None, tx)
            .await
            .expect("boxed fallback should work");

        let event = rx.recv().await.unwrap();
        assert!(event.is_stop());
    }

    // --- Tools are forwarded ---

    #[tokio::test]
    async fn test_fallback_forwards_tools_to_provider() {
        let events = vec![
            StreamEvent::ToolUseStart {
                id: "toolu_01".to_string(),
                name: "bash".to_string(),
                index: 0,
            },
            StreamEvent::MessageComplete {
                stop_reason: StopReason::ToolUse,
            },
        ];
        let providers: Vec<Box<dyn LlmProvider>> = vec![
            Box::new(MockProvider::new("primary", "model", events.clone())),
        ];
        let fallback = FallbackProvider::new(providers);

        let tools = vec![ToolDefinition::new(
            "bash",
            "Run commands",
            serde_json::json!({"type": "object", "properties": {}}),
        )];
        let (tx, mut rx) = mpsc::channel(32);
        let messages = vec![ApiMessageV2::user("list files")];
        fallback
            .stream_message(&messages, Some(&tools), Some(&ToolChoice::Auto), tx)
            .await
            .expect("should succeed");

        let mut received = Vec::new();
        while let Some(event) = rx.recv().await {
            received.push(event);
        }
        assert_eq!(received, events);
    }

    // --- Three-provider chain ---

    #[tokio::test]
    async fn test_fallback_skips_to_third_provider() {
        let events = vec![
            StreamEvent::ContentDelta("Third provider".to_string()),
            StreamEvent::MessageComplete {
                stop_reason: StopReason::EndTurn,
            },
        ];
        let providers: Vec<Box<dyn LlmProvider>> = vec![
            Box::new(FailingProvider::new("first", "m1", "error 1")),
            Box::new(FailingProvider::new("second", "m2", "error 2")),
            Box::new(MockProvider::new("third", "m3", events.clone())),
        ];
        let fallback = FallbackProvider::new(providers);

        let (tx, mut rx) = mpsc::channel(32);
        let messages = vec![ApiMessageV2::user("Hi")];
        fallback
            .stream_message(&messages, None, None, tx)
            .await
            .expect("should succeed with third provider");

        let first = rx.recv().await.unwrap();
        assert_eq!(first, StreamEvent::ContentDelta("Third provider".to_string()));
    }
}
