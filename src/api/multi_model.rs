//! Multi-model provider support for Patina.
//!
//! This module provides support for multiple AI providers and models,
//! allowing users to switch between different backends like Anthropic
//! direct API and AWS Bedrock.
//!
//! # Example
//!
//! ```
//! use patina::api::multi_model::{
//!     MultiModelClient, ModelConfig, ModelProvider, ProviderConfig,
//! };
//! use std::collections::HashMap;
//!
//! // Configure models
//! let mut configs = HashMap::new();
//! configs.insert(
//!     "claude-opus".to_string(),
//!     ModelConfig {
//!         model_id: "claude-3-opus-20240229".to_string(),
//!         provider: ModelProvider::Anthropic,
//!         max_tokens: 4096,
//!     },
//! );
//!
//! // Create client
//! let provider = ProviderConfig::Anthropic {
//!     api_key: secrecy::SecretString::new(
//!         std::env::var("ANTHROPIC_API_KEY").unwrap_or_default().into()
//!     ),
//! };
//! let mut client = MultiModelClient::new(configs, provider);
//!
//! // Switch models
//! client.set_model("claude-opus").unwrap();
//! ```

use anyhow::{bail, Result};
use secrecy::SecretString;
use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::types::{ApiMessageV2, MessageContent};

/// Supported AI providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelProvider {
    /// Direct Anthropic API.
    Anthropic,
    /// AWS Bedrock runtime.
    Bedrock,
}

/// Configuration for a specific model.
#[derive(Debug, Clone)]
pub struct ModelConfig {
    /// The model identifier (e.g., "claude-3-opus-20240229").
    pub model_id: String,
    /// The provider to use for this model.
    pub provider: ModelProvider,
    /// Maximum tokens for this model.
    pub max_tokens: u32,
}

/// Placeholder for the Model type (for future use).
#[derive(Debug, Clone)]
pub struct Model {
    /// Model name/alias.
    pub name: String,
    /// Model configuration.
    pub config: ModelConfig,
}

/// Configuration for the Anthropic provider.
///
/// The API key is stored as a [`SecretString`] to prevent accidental exposure
/// in logs, debug output, or memory dumps.
#[derive(Clone)]
pub struct AnthropicConfig {
    /// API key for Anthropic (protected by SecretString).
    pub api_key: SecretString,
}

impl fmt::Debug for AnthropicConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnthropicConfig")
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

/// Configuration for AWS Bedrock provider.
#[derive(Debug, Clone)]
pub struct BedrockConfig {
    /// AWS region (e.g., "us-east-1").
    pub region: String,
    /// AWS profile name (optional).
    pub profile: Option<String>,
    /// IAM role ARN for cross-account access (optional).
    pub role_arn: Option<String>,
}

/// Provider configuration enum.
///
/// API keys are stored as [`SecretString`] to prevent accidental exposure
/// in logs, debug output, or memory dumps.
#[derive(Clone)]
pub enum ProviderConfig {
    /// Anthropic direct API configuration.
    Anthropic {
        /// API key (protected by SecretString).
        api_key: SecretString,
    },
    /// AWS Bedrock configuration.
    Bedrock(BedrockConfig),
}

impl fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Anthropic { .. } => f
                .debug_struct("Anthropic")
                .field("api_key", &"[REDACTED]")
                .finish(),
            Self::Bedrock(config) => f.debug_tuple("Bedrock").field(config).finish(),
        }
    }
}

/// Multi-model client supporting multiple providers.
///
/// This client manages model configurations and allows switching
/// between different models and providers at runtime.
#[derive(Debug)]
pub struct MultiModelClient {
    /// Model configurations keyed by alias.
    models: HashMap<String, ModelConfig>,
    /// Currently selected model alias.
    current: Option<String>,
    /// Provider configuration.
    provider_config: ProviderConfig,
    /// Model aliases.
    aliases: HashMap<String, String>,
}

impl MultiModelClient {
    /// Creates a new multi-model client.
    ///
    /// # Arguments
    ///
    /// * `models` - Model configurations keyed by alias.
    /// * `provider_config` - Provider configuration.
    #[must_use]
    pub fn new(models: HashMap<String, ModelConfig>, provider_config: ProviderConfig) -> Self {
        Self {
            models,
            current: None,
            provider_config,
            aliases: HashMap::new(),
        }
    }

    /// Creates a new multi-model client with a default model selected.
    ///
    /// # Arguments
    ///
    /// * `models` - Model configurations keyed by alias.
    /// * `provider_config` - Provider configuration.
    /// * `default_model` - The model alias to select by default.
    #[must_use]
    pub fn with_default(
        models: HashMap<String, ModelConfig>,
        provider_config: ProviderConfig,
        default_model: &str,
    ) -> Self {
        let current = if models.contains_key(default_model) {
            Some(default_model.to_string())
        } else {
            None
        };

        Self {
            models,
            current,
            provider_config,
            aliases: HashMap::new(),
        }
    }

    /// Lists all available model aliases.
    #[must_use]
    pub fn list_models(&self) -> Vec<String> {
        self.models.keys().cloned().collect()
    }

    /// Returns the currently selected model alias.
    #[must_use]
    pub fn current_model(&self) -> Option<&str> {
        self.current.as_deref()
    }

    /// Sets the current model by alias or alias shorthand.
    ///
    /// # Arguments
    ///
    /// * `model_or_alias` - Model alias or alias shorthand.
    ///
    /// # Errors
    ///
    /// Returns an error if the model/alias doesn't exist.
    pub fn set_model(&mut self, model_or_alias: &str) -> Result<()> {
        // Resolve alias if present
        let model_name = self
            .aliases
            .get(model_or_alias)
            .cloned()
            .unwrap_or_else(|| model_or_alias.to_string());

        if !self.models.contains_key(&model_name) {
            bail!("Unknown model: {}", model_or_alias);
        }

        self.current = Some(model_name);
        Ok(())
    }

    /// Gets the configuration for a specific model.
    ///
    /// # Arguments
    ///
    /// * `model_alias` - The model alias.
    #[must_use]
    pub fn get_model_config(&self, model_alias: &str) -> Option<&ModelConfig> {
        self.models.get(model_alias)
    }

    /// Adds an alias for a model.
    ///
    /// # Arguments
    ///
    /// * `alias` - The short alias.
    /// * `model` - The full model name.
    ///
    /// # Errors
    ///
    /// Returns an error if the target model doesn't exist.
    pub fn add_alias(&mut self, alias: &str, model: &str) -> Result<()> {
        if !self.models.contains_key(model) {
            bail!("Cannot add alias for unknown model: {}", model);
        }
        self.aliases.insert(alias.to_string(), model.to_string());
        Ok(())
    }

    /// Lists all unique providers used by configured models.
    #[must_use]
    pub fn list_providers(&self) -> Vec<ModelProvider> {
        let providers: HashSet<_> = self.models.values().map(|c| c.provider).collect();
        providers.into_iter().collect()
    }

    /// Validates the client configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if no models are configured.
    pub fn validate(&self) -> Result<()> {
        if self.models.is_empty() {
            bail!("No models configured");
        }
        Ok(())
    }

    /// Returns the provider configuration.
    #[must_use]
    pub fn provider_config(&self) -> &ProviderConfig {
        &self.provider_config
    }
}

/// Checks if any message in the conversation contains image content.
///
/// This is used for vision model routing - when images are present,
/// the request may need to be routed to a vision-capable model.
///
/// # Arguments
///
/// * `messages` - The conversation messages to check
///
/// # Returns
///
/// `true` if any message contains an image content block
///
/// # Examples
///
/// ```rust
/// use patina::api::multi_model::contains_images;
/// use patina::types::{ApiMessageV2, ContentBlock, MessageContent};
/// use patina::types::image::ImageSource;
///
/// // Messages without images
/// let text_only = vec![
///     ApiMessageV2::user("Hello"),
///     ApiMessageV2::assistant("Hi there!"),
/// ];
/// assert!(!contains_images(&text_only));
///
/// // Messages with images
/// let source = ImageSource::Base64 {
///     media_type: "image/png".to_string(),
///     data: "iVBORw0KGgo=".to_string(),
/// };
/// let with_image = vec![
///     ApiMessageV2::user_with_content(MessageContent::blocks(vec![
///         ContentBlock::text("What's in this image?"),
///         ContentBlock::image(source),
///     ])),
/// ];
/// assert!(contains_images(&with_image));
/// ```
#[must_use]
pub fn contains_images(messages: &[ApiMessageV2]) -> bool {
    messages.iter().any(message_contains_images)
}

/// Checks if a single message contains image content.
///
/// # Arguments
///
/// * `message` - The message to check
///
/// # Returns
///
/// `true` if the message contains an image content block
#[must_use]
fn message_contains_images(message: &ApiMessageV2) -> bool {
    match &message.content {
        MessageContent::Text(_) => false,
        MessageContent::Blocks(blocks) => blocks.iter().any(|block| block.is_image()),
    }
}

/// Selects the appropriate model for a request based on content.
///
/// If the messages contain images and a vision model is configured,
/// returns the vision model. Otherwise, returns the default model.
///
/// # Arguments
///
/// * `messages` - The conversation messages
/// * `default_model` - The default model to use
/// * `vision_model` - Optional vision model for image requests
///
/// # Returns
///
/// The model identifier to use for the request
///
/// # Examples
///
/// ```rust
/// use patina::api::multi_model::select_model_for_content;
/// use patina::types::ApiMessageV2;
///
/// // No images - use default model
/// let messages = vec![ApiMessageV2::user("Hello")];
/// let model = select_model_for_content(&messages, "claude-sonnet-4", Some("claude-opus-4"));
/// assert_eq!(model, "claude-sonnet-4");
/// ```
#[must_use]
pub fn select_model_for_content<'a>(
    messages: &[ApiMessageV2],
    default_model: &'a str,
    vision_model: Option<&'a str>,
) -> &'a str {
    if contains_images(messages) {
        // Use vision model if configured, otherwise fall back to default
        // (all Claude 3+ models support vision)
        vision_model.unwrap_or(default_model)
    } else {
        default_model
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_provider_equality() {
        assert_eq!(ModelProvider::Anthropic, ModelProvider::Anthropic);
        assert_eq!(ModelProvider::Bedrock, ModelProvider::Bedrock);
        assert_ne!(ModelProvider::Anthropic, ModelProvider::Bedrock);
    }

    #[test]
    fn test_model_config_creation() {
        let config = ModelConfig {
            model_id: "test-model".to_string(),
            provider: ModelProvider::Anthropic,
            max_tokens: 4096,
        };
        assert_eq!(config.model_id, "test-model");
        assert_eq!(config.max_tokens, 4096);
    }

    #[test]
    fn test_bedrock_config_creation() {
        let config = BedrockConfig {
            region: "us-east-1".to_string(),
            profile: Some("default".to_string()),
            role_arn: None,
        };
        assert_eq!(config.region, "us-east-1");
        assert!(config.role_arn.is_none());
    }

    #[test]
    fn test_provider_config_variants() {
        let anthropic = ProviderConfig::Anthropic {
            api_key: SecretString::new("sk-ant-test-placeholder".into()),
        };
        let bedrock = ProviderConfig::Bedrock(BedrockConfig {
            region: "us-east-1".to_string(),
            profile: None,
            role_arn: None,
        });

        assert!(matches!(anthropic, ProviderConfig::Anthropic { .. }));
        assert!(matches!(bedrock, ProviderConfig::Bedrock(_)));
    }

    #[test]
    fn test_client_new_empty() {
        let configs = HashMap::new();
        let provider = ProviderConfig::Anthropic {
            api_key: SecretString::new("sk-ant-test-placeholder".into()),
        };
        let client = MultiModelClient::new(configs, provider);

        assert!(client.list_models().is_empty());
        assert!(client.current_model().is_none());
    }

    // =========================================================================
    // Vision model routing tests
    // =========================================================================

    use crate::types::image::ImageSource;
    use crate::types::ContentBlock;

    #[test]
    fn test_contains_images_empty_messages() {
        let messages: Vec<ApiMessageV2> = vec![];
        assert!(!contains_images(&messages));
    }

    #[test]
    fn test_contains_images_text_only() {
        let messages = vec![
            ApiMessageV2::user("Hello"),
            ApiMessageV2::assistant("Hi there!"),
        ];
        assert!(!contains_images(&messages));
    }

    #[test]
    fn test_contains_images_with_blocks_no_images() {
        let messages = vec![ApiMessageV2::user_with_content(MessageContent::blocks(vec![
            ContentBlock::text("Just text"),
            ContentBlock::text("More text"),
        ]))];
        assert!(!contains_images(&messages));
    }

    #[test]
    fn test_contains_images_with_image() {
        let source = ImageSource::Base64 {
            media_type: "image/png".to_string(),
            data: "iVBORw0KGgo=".to_string(),
        };
        let messages = vec![ApiMessageV2::user_with_content(MessageContent::blocks(vec![
            ContentBlock::text("What's this?"),
            ContentBlock::image(source),
        ]))];
        assert!(contains_images(&messages));
    }

    #[test]
    fn test_contains_images_url_source() {
        let source = ImageSource::Url {
            url: "https://example.com/image.png".to_string(),
        };
        let messages = vec![ApiMessageV2::user_with_content(MessageContent::blocks(vec![
            ContentBlock::image(source),
        ]))];
        assert!(contains_images(&messages));
    }

    #[test]
    fn test_contains_images_mixed_messages() {
        // Some messages have images, some don't
        let source = ImageSource::Base64 {
            media_type: "image/jpeg".to_string(),
            data: "/9j/4AAQ".to_string(),
        };
        let messages = vec![
            ApiMessageV2::user("Hello"),
            ApiMessageV2::assistant("Hi!"),
            ApiMessageV2::user_with_content(MessageContent::blocks(vec![
                ContentBlock::text("Look at this:"),
                ContentBlock::image(source),
            ])),
        ];
        assert!(contains_images(&messages));
    }

    #[test]
    fn test_select_model_for_content_no_images_no_vision_model() {
        let messages = vec![ApiMessageV2::user("Hello")];
        let model = select_model_for_content(&messages, "claude-sonnet-4", None);
        assert_eq!(model, "claude-sonnet-4");
    }

    #[test]
    fn test_select_model_for_content_no_images_with_vision_model() {
        let messages = vec![ApiMessageV2::user("Hello")];
        let model = select_model_for_content(&messages, "claude-sonnet-4", Some("claude-opus-4"));
        // No images, so use default model
        assert_eq!(model, "claude-sonnet-4");
    }

    #[test]
    fn test_select_model_for_content_with_images_no_vision_model() {
        let source = ImageSource::Base64 {
            media_type: "image/png".to_string(),
            data: "data".to_string(),
        };
        let messages = vec![ApiMessageV2::user_with_content(MessageContent::blocks(vec![
            ContentBlock::image(source),
        ]))];
        let model = select_model_for_content(&messages, "claude-sonnet-4", None);
        // Has images but no vision model, fall back to default
        assert_eq!(model, "claude-sonnet-4");
    }

    #[test]
    fn test_select_model_for_content_with_images_with_vision_model() {
        let source = ImageSource::Base64 {
            media_type: "image/png".to_string(),
            data: "data".to_string(),
        };
        let messages = vec![ApiMessageV2::user_with_content(MessageContent::blocks(vec![
            ContentBlock::image(source),
        ]))];
        let model = select_model_for_content(&messages, "claude-sonnet-4", Some("claude-opus-4"));
        // Has images and vision model configured, use vision model
        assert_eq!(model, "claude-opus-4");
    }
}
