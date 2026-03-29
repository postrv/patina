//! Provider factory functions.
//!
//! This module contains the factory logic for creating [`LlmProvider`]
//! instances from application configuration. It is separated from the
//! [`provider`](super::provider) module to break the circular dependency
//! between `provider.rs` (which defines the trait) and concrete provider
//! implementations like [`OpenRouterProvider`](super::providers::openrouter::OpenRouterProvider).

use anyhow::Result;
use secrecy::SecretString;

use super::provider::{FallbackProvider, LlmProvider};
use super::providers::bedrock::BedrockProvider;
use super::providers::openrouter::OpenRouterProvider;
use super::providers::vertex::VertexProvider;
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
/// - [`ProviderKind::Bedrock`]: Creates a [`BedrockProvider`] using AWS
///   credentials from environment variables.
/// - [`ProviderKind::Vertex`]: Creates a [`VertexProvider`] using Google
///   Cloud credentials from config or environment.
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
/// an API key, Bedrock without AWS credentials, or Fallback with an invalid
/// sub-provider).
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
/// an API key or model, Bedrock without AWS credentials, Vertex without Google
/// Cloud credentials, or Fallback with an invalid sub-provider).
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
        ProviderKind::Bedrock => {
            let region = provider_config
                .bedrock_region
                .clone()
                .or_else(|| std::env::var("AWS_REGION").ok())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Bedrock provider requires an AWS region. Set AWS_REGION environment \
                         variable or configure bedrock_region in provider config."
                    )
                })?;

            let bedrock_model = provider_config.bedrock_model.as_deref().unwrap_or(model);

            let access_key_id = std::env::var("AWS_ACCESS_KEY_ID")
                .map(SecretString::from)
                .map_err(|_| {
                    anyhow::anyhow!(
                        "Bedrock provider requires AWS_ACCESS_KEY_ID environment variable"
                    )
                })?;

            let secret_access_key = std::env::var("AWS_SECRET_ACCESS_KEY")
                .map(SecretString::from)
                .map_err(|_| {
                    anyhow::anyhow!(
                        "Bedrock provider requires AWS_SECRET_ACCESS_KEY environment variable"
                    )
                })?;

            let session_token = std::env::var("AWS_SESSION_TOKEN")
                .ok()
                .map(SecretString::from);

            Ok(Box::new(BedrockProvider::new(
                access_key_id,
                secret_access_key,
                session_token,
                region,
                bedrock_model,
            )))
        }
        ProviderKind::Vertex => {
            let project = provider_config.vertex_project.as_deref().ok_or_else(|| {
                anyhow::anyhow!(
                    "Vertex AI provider requires a Google Cloud project ID. Set \
                         GOOGLE_CLOUD_PROJECT environment variable or configure \
                         vertex_project in provider config."
                )
            })?;

            let region = provider_config.vertex_region.as_deref().ok_or_else(|| {
                anyhow::anyhow!(
                    "Vertex AI provider requires a Google Cloud region. Set \
                     CLOUD_ML_REGION environment variable or configure \
                     vertex_region in provider config."
                )
            })?;

            let vertex_model = provider_config.vertex_model.as_deref().unwrap_or(model);

            // Try to get access token from environment first, then fall back to ADC
            let access_token = if let Ok(token) = std::env::var("GOOGLE_ACCESS_TOKEN") {
                SecretString::from(token)
            } else {
                // Fall back to reading credentials file
                let creds_path = std::env::var("GOOGLE_APPLICATION_CREDENTIALS").map_err(|_| {
                    anyhow::anyhow!(
                        "Vertex AI provider requires authentication. Set GOOGLE_ACCESS_TOKEN \
                         for a pre-fetched token or GOOGLE_APPLICATION_CREDENTIALS for \
                         service account credentials."
                    )
                })?;

                let creds_json = std::fs::read_to_string(&creds_path).map_err(|e| {
                    anyhow::anyhow!("Failed to read credentials file at {creds_path}: {e}")
                })?;

                // Extract credential material from service account key
                let value: serde_json::Value = serde_json::from_str(&creds_json).map_err(|e| {
                    anyhow::anyhow!("Invalid credentials JSON in {creds_path}: {e}")
                })?;

                let private_key = value
                    .get("private_key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        anyhow::anyhow!("Credentials file missing 'private_key' field")
                    })?;

                SecretString::from(private_key.to_string())
            };

            Ok(Box::new(VertexProvider::new(
                project,
                region,
                vertex_model,
                access_token,
            )))
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
            bedrock_region: None,
            bedrock_model: None,
            vertex_project: None,
            vertex_region: None,
            vertex_model: None,
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

    #[test]
    #[serial_test::serial]
    fn test_create_provider_bedrock() {
        // Set required AWS environment variables
        std::env::set_var("AWS_ACCESS_KEY_ID", "AKIAIOSFODNN7EXAMPLE");
        std::env::set_var(
            "AWS_SECRET_ACCESS_KEY",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        );
        std::env::remove_var("AWS_SESSION_TOKEN");

        let config = crate::types::Config::new(
            SecretString::from("sk-ant-test"),
            "claude-sonnet-4",
            std::path::PathBuf::from("."),
        )
        .with_provider(ProviderConfig::bedrock(
            "us-east-1",
            "claude-sonnet-4-20250514",
        ));

        let provider = create_provider(&config).expect("should create Bedrock provider");
        assert_eq!(provider.name(), "bedrock");
        assert_eq!(provider.model(), "claude-sonnet-4-20250514");

        std::env::remove_var("AWS_ACCESS_KEY_ID");
        std::env::remove_var("AWS_SECRET_ACCESS_KEY");
    }

    #[test]
    #[serial_test::serial]
    fn test_create_provider_bedrock_missing_credentials() {
        std::env::remove_var("AWS_ACCESS_KEY_ID");
        std::env::remove_var("AWS_SECRET_ACCESS_KEY");
        std::env::remove_var("AWS_SESSION_TOKEN");

        let config = crate::types::Config::new(
            SecretString::from("sk-ant-test"),
            "claude-sonnet-4",
            std::path::PathBuf::from("."),
        )
        .with_provider(ProviderConfig::bedrock(
            "us-east-1",
            "claude-sonnet-4-20250514",
        ));

        let result = create_provider(&config);
        assert!(result.is_err());
        let err = result.err().expect("already checked is_err").to_string();
        assert!(
            err.contains("AWS_ACCESS_KEY_ID"),
            "Error should mention AWS_ACCESS_KEY_ID, got: {err}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_create_provider_vertex_with_access_token() {
        std::env::set_var("GOOGLE_ACCESS_TOKEN", "ya29.test-token");

        let config = crate::types::Config::new(
            SecretString::from("sk-ant-test"),
            "claude-sonnet-4",
            std::path::PathBuf::from("."),
        )
        .with_provider(ProviderConfig::vertex(
            "my-project",
            "us-east5",
            "claude-sonnet-4@20250514",
        ));

        let provider = create_provider(&config).expect("should create Vertex provider");
        assert_eq!(provider.name(), "vertex");
        assert_eq!(provider.model(), "claude-sonnet-4@20250514");

        std::env::remove_var("GOOGLE_ACCESS_TOKEN");
    }

    #[test]
    fn test_create_provider_vertex_missing_project() {
        let config = crate::types::Config::new(
            SecretString::from("sk-ant-test"),
            "claude-sonnet-4",
            std::path::PathBuf::from("."),
        )
        .with_provider(ProviderConfig {
            kind: ProviderKind::Vertex,
            openrouter_api_key: None,
            openrouter_model: None,
            site_url: None,
            app_name: None,
            bedrock_region: None,
            bedrock_model: None,
            vertex_project: None,
            vertex_region: Some("us-east5".to_string()),
            vertex_model: Some("claude-sonnet-4@20250514".to_string()),
            fallback_chain: Vec::new(),
        });

        let result = create_provider(&config);
        assert!(result.is_err());
        let err = result.err().expect("already checked is_err").to_string();
        assert!(
            err.contains("project"),
            "Error should mention project, got: {err}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_create_provider_vertex_missing_auth() {
        std::env::remove_var("GOOGLE_ACCESS_TOKEN");
        std::env::remove_var("GOOGLE_APPLICATION_CREDENTIALS");

        let config = crate::types::Config::new(
            SecretString::from("sk-ant-test"),
            "claude-sonnet-4",
            std::path::PathBuf::from("."),
        )
        .with_provider(ProviderConfig::vertex(
            "my-project",
            "us-east5",
            "claude-sonnet-4@20250514",
        ));

        let result = create_provider(&config);
        assert!(result.is_err());
        let err = result.err().expect("already checked is_err").to_string();
        assert!(
            err.contains("GOOGLE_ACCESS_TOKEN") || err.contains("GOOGLE_APPLICATION_CREDENTIALS"),
            "Error should mention auth, got: {err}"
        );
    }
}
