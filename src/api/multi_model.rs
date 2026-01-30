//! Multi-model provider support for RCT.
//!
//! This module provides support for multiple AI providers and models,
//! allowing users to switch between different backends like Anthropic
//! direct API and AWS Bedrock.
//!
//! # Example
//!
//! ```
//! use rct::api::multi_model::{
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
//!     api_key: std::env::var("ANTHROPIC_API_KEY").unwrap_or_default().to_string(),
//! };
//! let mut client = MultiModelClient::new(configs, provider);
//!
//! // Switch models
//! client.set_model("claude-opus").unwrap();
//! ```

use anyhow::{bail, Result};
use std::collections::{HashMap, HashSet};

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
#[derive(Debug, Clone)]
pub struct AnthropicConfig {
    /// API key for Anthropic.
    pub api_key: String,
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
#[derive(Debug, Clone)]
pub enum ProviderConfig {
    /// Anthropic direct API configuration.
    Anthropic {
        /// API key.
        api_key: String,
    },
    /// AWS Bedrock configuration.
    Bedrock(BedrockConfig),
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
            api_key: "sk-ant-test-placeholder".to_string(),
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
            api_key: "sk-ant-test-placeholder".to_string(),
        };
        let client = MultiModelClient::new(configs, provider);

        assert!(client.list_models().is_empty());
        assert!(client.current_model().is_none());
    }
}
