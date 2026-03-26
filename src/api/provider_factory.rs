//! Provider factory functions.
//!
//! This module contains the factory logic for creating [`LlmProvider`]
//! instances from application configuration. It is separated from the
//! [`provider`](super::provider) module to break the circular dependency
//! between `provider.rs` (which defines the trait) and concrete provider
//! implementations like [`OpenRouterProvider`](super::providers::openrouter::OpenRouterProvider).

use anyhow::Result;

use super::provider::{FallbackProvider, LlmProvider};
use super::providers::openrouter::OpenRouterProvider;
use super::AnthropicClient;
use crate::types::config::{ProviderConfig, ProviderKind};

/// Creates an LLM provider based on the application configuration.
///
/// Reads the [`ProviderConfig`] from the given [`Config`](crate::types::Config)
/// and constructs the appropriate provider:
///
/// - [`ProviderKind::Anthropic`]: Creates an [`AnthropicClient`] using
///   `config.api_key` and `config.model`.
/// - [`ProviderKind::OpenRouter`]: Creates an [`OpenRouterProvider`] using
///   the OpenRouter API key and model from the provider config.
/// - [`ProviderKind::Fallback`]: Creates a [`FallbackProvider`] that tries
///   each provider in the fallback chain in order.
///
/// # Arguments
///
/// * `config` - The application configuration containing provider settings.
///
/// # Errors
///
/// Returns an error if the provider config is invalid (e.g., OpenRouter without
/// an API key, or Fallback with an invalid sub-provider).
///
/// # Examples
///
/// ```rust,ignore
/// use patina::api::provider_factory::create_provider;
/// use patina::types::config::Config;
/// use secrecy::SecretString;
///
/// let config = Config::new(
///     SecretString::from("sk-ant-..."),
///     "claude-sonnet-4",
///     std::path::PathBuf::from("."),
/// );
/// let provider = create_provider(&config)?;
/// assert_eq!(provider.name(), "anthropic");
/// ```
pub fn create_provider(config: &crate::types::Config) -> Result<Box<dyn LlmProvider>> {
    create_provider_from_config(&config.provider, &config.api_key, &config.model)
}

/// Creates a provider from a [`ProviderConfig`], using the given API key and
/// model as defaults for Anthropic providers.
///
/// # Errors
///
/// Returns an error if the provider config is invalid (e.g., OpenRouter without
/// an API key or model, or Fallback with an invalid sub-provider).
fn create_provider_from_config(
    provider_config: &ProviderConfig,
    api_key: &secrecy::SecretString,
    model: &str,
) -> Result<Box<dyn LlmProvider>> {
    match provider_config.kind {
        ProviderKind::Anthropic => Ok(Box::new(AnthropicClient::new(api_key.clone(), model))),
        ProviderKind::OpenRouter => {
            let or_key = provider_config
                .openrouter_api_key
                .clone()
                .ok_or_else(|| anyhow::anyhow!("OpenRouter provider requires an API key"))?;
            let or_model = provider_config.openrouter_model.as_deref().ok_or_else(|| {
                anyhow::anyhow!("OpenRouter provider requires a model identifier")
            })?;

            let mut provider = OpenRouterProvider::new(or_key, or_model);

            if let Some(ref url) = provider_config.site_url {
                provider = provider.with_site_url(url.clone());
            }
            if let Some(ref name) = provider_config.app_name {
                provider = provider.with_app_name(name.clone());
            }

            Ok(Box::new(provider))
        }
        ProviderKind::Fallback => {
            let providers: Vec<Box<dyn LlmProvider>> = provider_config
                .fallback_chain
                .iter()
                .map(|pc| create_provider_from_config(pc, api_key, model))
                .collect::<Result<Vec<_>>>()?;

            Ok(Box::new(FallbackProvider::new(providers)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_provider_default_anthropic() {
        use secrecy::SecretString;

        let config = crate::types::Config::new(
            SecretString::from("sk-ant-test"),
            "claude-sonnet-4",
            std::path::PathBuf::from("."),
        );

        let provider = create_provider(&config).expect("should create Anthropic provider");
        assert_eq!(provider.name(), "anthropic");
        assert_eq!(provider.model(), "claude-sonnet-4");
    }

    #[test]
    fn test_create_provider_explicit_anthropic() {
        use secrecy::SecretString;

        let config = crate::types::Config::new(
            SecretString::from("sk-ant-test"),
            "claude-opus-4-20250514",
            std::path::PathBuf::from("."),
        )
        .with_provider(ProviderConfig::anthropic());

        let provider = create_provider(&config).expect("should create Anthropic provider");
        assert_eq!(provider.name(), "anthropic");
        assert_eq!(provider.model(), "claude-opus-4-20250514");
    }

    #[test]
    fn test_create_provider_openrouter() {
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

        let provider = create_provider(&config).expect("should create OpenRouter provider");
        assert_eq!(provider.name(), "openrouter");
        assert_eq!(provider.model(), "anthropic/claude-sonnet-4");
    }

    #[test]
    fn test_create_provider_openrouter_with_metadata() {
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

        let provider = create_provider(&config).expect("should create OpenRouter provider");
        assert_eq!(provider.name(), "openrouter");
    }

    #[test]
    fn test_create_provider_missing_openrouter_key_returns_error() {
        let config = crate::types::Config::new(
            secrecy::SecretString::from("sk-ant-test"),
            "claude-sonnet-4",
            std::path::PathBuf::from("."),
        )
        .with_provider(ProviderConfig {
            kind: ProviderKind::OpenRouter,
            openrouter_api_key: None,
            openrouter_model: Some("anthropic/claude-sonnet-4".to_string()),
            site_url: None,
            app_name: None,
            fallback_chain: Vec::new(),
        });

        let result = create_provider(&config);
        assert!(result.is_err());
        let err = result.err().expect("already checked is_err").to_string();
        assert!(
            err.contains("API key"),
            "Error should mention API key, got: {err}"
        );
    }

    #[test]
    fn test_create_provider_fallback() {
        use secrecy::SecretString;

        let config = crate::types::Config::new(
            SecretString::from("sk-ant-test"),
            "claude-sonnet-4",
            std::path::PathBuf::from("."),
        )
        .with_provider(ProviderConfig::fallback(vec![
            ProviderConfig::anthropic(),
            ProviderConfig::openrouter(
                SecretString::from("sk-or-test"),
                "anthropic/claude-sonnet-4",
            ),
        ]));

        let provider = create_provider(&config).expect("should create Fallback provider");
        assert_eq!(provider.name(), "fallback");
        // Model comes from primary (first) provider
        assert_eq!(provider.model(), "claude-sonnet-4");
    }
}
