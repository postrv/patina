//! Integration tests for multi-model provider support.
//!
//! Tests model switching and multiple provider backends including:
//! - Anthropic direct API
//! - AWS Bedrock
//! - Provider switching at runtime

use rct::api::multi_model::{
    BedrockConfig, ModelConfig, ModelProvider, MultiModelClient, ProviderConfig,
};
use std::collections::HashMap;

// =============================================================================
// Helper functions
// =============================================================================

/// Creates a test provider configuration for Anthropic.
fn anthropic_config() -> ProviderConfig {
    ProviderConfig::Anthropic {
        api_key: "sk-ant-test-placeholder-not-real".to_string(),
    }
}

/// Creates a test provider configuration for Bedrock.
fn bedrock_config() -> BedrockConfig {
    BedrockConfig {
        region: "us-east-1".to_string(),
        profile: Some("default".to_string()),
        role_arn: None,
    }
}

// =============================================================================
// 7.3.1 Model switching tests
// =============================================================================

/// Test that models can be listed from the client.
#[test]
fn test_model_list() {
    let mut configs = HashMap::new();
    configs.insert(
        "claude-opus".to_string(),
        ModelConfig {
            model_id: "claude-3-opus-20240229".to_string(),
            provider: ModelProvider::Anthropic,
            max_tokens: 4096,
        },
    );
    configs.insert(
        "claude-sonnet".to_string(),
        ModelConfig {
            model_id: "claude-3-sonnet-20240229".to_string(),
            provider: ModelProvider::Anthropic,
            max_tokens: 4096,
        },
    );

    let client = MultiModelClient::new(configs, anthropic_config());
    let models = client.list_models();

    assert_eq!(models.len(), 2);
    assert!(models.contains(&"claude-opus".to_string()));
    assert!(models.contains(&"claude-sonnet".to_string()));
}

/// Test that the current model can be retrieved.
#[test]
fn test_model_get_current() {
    let mut configs = HashMap::new();
    configs.insert(
        "claude-opus".to_string(),
        ModelConfig {
            model_id: "claude-3-opus-20240229".to_string(),
            provider: ModelProvider::Anthropic,
            max_tokens: 4096,
        },
    );

    let mut client = MultiModelClient::new(configs, anthropic_config());
    client
        .set_model("claude-opus")
        .expect("Failed to set model");

    assert_eq!(client.current_model(), Some("claude-opus"));
}

/// Test switching between different models.
#[test]
fn test_model_switching() {
    let mut configs = HashMap::new();
    configs.insert(
        "claude-opus".to_string(),
        ModelConfig {
            model_id: "claude-3-opus-20240229".to_string(),
            provider: ModelProvider::Anthropic,
            max_tokens: 4096,
        },
    );
    configs.insert(
        "claude-sonnet".to_string(),
        ModelConfig {
            model_id: "claude-3-sonnet-20240229".to_string(),
            provider: ModelProvider::Anthropic,
            max_tokens: 4096,
        },
    );

    let mut client = MultiModelClient::new(configs, anthropic_config());

    // Start with no model selected
    assert!(client.current_model().is_none());

    // Switch to opus
    assert!(client.set_model("claude-opus").is_ok());
    assert_eq!(client.current_model(), Some("claude-opus"));

    // Switch to sonnet
    assert!(client.set_model("claude-sonnet").is_ok());
    assert_eq!(client.current_model(), Some("claude-sonnet"));

    // Try switching to non-existent model
    assert!(client.set_model("nonexistent").is_err());
    // Should remain on sonnet
    assert_eq!(client.current_model(), Some("claude-sonnet"));
}

/// Test model configuration retrieval.
#[test]
fn test_model_config_retrieval() {
    let mut configs = HashMap::new();
    configs.insert(
        "claude-opus".to_string(),
        ModelConfig {
            model_id: "claude-3-opus-20240229".to_string(),
            provider: ModelProvider::Anthropic,
            max_tokens: 8192,
        },
    );

    let client = MultiModelClient::new(configs, anthropic_config());
    let config = client.get_model_config("claude-opus");

    assert!(config.is_some());
    let config = config.unwrap();
    assert_eq!(config.model_id, "claude-3-opus-20240229");
    assert_eq!(config.max_tokens, 8192);
    assert_eq!(config.provider, ModelProvider::Anthropic);
}

// =============================================================================
// 7.3.1 Bedrock provider tests
// =============================================================================

/// Test that Bedrock provider can be configured.
#[test]
fn test_bedrock_provider_config() {
    let mut configs = HashMap::new();
    configs.insert(
        "claude-bedrock".to_string(),
        ModelConfig {
            model_id: "anthropic.claude-3-opus-20240229-v1:0".to_string(),
            provider: ModelProvider::Bedrock,
            max_tokens: 4096,
        },
    );

    let bedrock = bedrock_config();
    let provider_config = ProviderConfig::Bedrock(bedrock.clone());
    let client = MultiModelClient::new(configs, provider_config);

    let config = client.get_model_config("claude-bedrock").unwrap();
    assert_eq!(config.provider, ModelProvider::Bedrock);
}

/// Test that Bedrock provider requires proper region configuration.
#[test]
fn test_bedrock_provider_region() {
    let config = BedrockConfig {
        region: "eu-west-1".to_string(),
        profile: None,
        role_arn: None,
    };

    assert_eq!(config.region, "eu-west-1");
    assert!(config.profile.is_none());
}

/// Test Bedrock provider with role assumption.
#[test]
fn test_bedrock_provider_with_role() {
    let config = BedrockConfig {
        region: "us-west-2".to_string(),
        profile: Some("production".to_string()),
        role_arn: Some("arn:aws:iam::123456789012:role/BedrockRole".to_string()),
    };

    assert_eq!(config.region, "us-west-2");
    assert_eq!(config.profile, Some("production".to_string()));
    assert!(config.role_arn.is_some());
}

/// Test switching between Anthropic and Bedrock providers.
#[test]
fn test_provider_switching() {
    let mut configs = HashMap::new();
    configs.insert(
        "anthropic-opus".to_string(),
        ModelConfig {
            model_id: "claude-3-opus-20240229".to_string(),
            provider: ModelProvider::Anthropic,
            max_tokens: 4096,
        },
    );
    configs.insert(
        "bedrock-opus".to_string(),
        ModelConfig {
            model_id: "anthropic.claude-3-opus-20240229-v1:0".to_string(),
            provider: ModelProvider::Bedrock,
            max_tokens: 4096,
        },
    );

    // Create client with Anthropic as primary
    let mut client = MultiModelClient::new(configs.clone(), anthropic_config());

    // Start with Anthropic model
    client.set_model("anthropic-opus").unwrap();
    assert_eq!(
        client.get_model_config("anthropic-opus").unwrap().provider,
        ModelProvider::Anthropic
    );

    // Switch to Bedrock model
    client.set_model("bedrock-opus").unwrap();
    assert_eq!(
        client.get_model_config("bedrock-opus").unwrap().provider,
        ModelProvider::Bedrock
    );
}

/// Test that model aliases work correctly.
#[test]
fn test_model_aliases() {
    let mut configs = HashMap::new();
    configs.insert(
        "opus".to_string(),
        ModelConfig {
            model_id: "claude-3-opus-20240229".to_string(),
            provider: ModelProvider::Anthropic,
            max_tokens: 4096,
        },
    );

    let mut client = MultiModelClient::new(configs, anthropic_config());

    // Add alias
    client.add_alias("o", "opus").expect("Failed to add alias");

    // Set model using alias
    assert!(client.set_model("o").is_ok());
    assert_eq!(client.current_model(), Some("opus"));
}

/// Test listing available providers.
#[test]
fn test_list_providers() {
    let mut configs = HashMap::new();
    configs.insert(
        "anthropic-model".to_string(),
        ModelConfig {
            model_id: "claude-3-opus".to_string(),
            provider: ModelProvider::Anthropic,
            max_tokens: 4096,
        },
    );
    configs.insert(
        "bedrock-model".to_string(),
        ModelConfig {
            model_id: "anthropic.claude-3-opus".to_string(),
            provider: ModelProvider::Bedrock,
            max_tokens: 4096,
        },
    );

    let client = MultiModelClient::new(configs, anthropic_config());
    let providers = client.list_providers();

    assert!(providers.contains(&ModelProvider::Anthropic));
    assert!(providers.contains(&ModelProvider::Bedrock));
}

/// Test default model selection.
#[test]
fn test_default_model() {
    let mut configs = HashMap::new();
    configs.insert(
        "default-model".to_string(),
        ModelConfig {
            model_id: "claude-3-sonnet".to_string(),
            provider: ModelProvider::Anthropic,
            max_tokens: 4096,
        },
    );

    let client = MultiModelClient::with_default(configs, anthropic_config(), "default-model");

    assert_eq!(client.current_model(), Some("default-model"));
}

/// Test model validation.
#[test]
fn test_model_validation() {
    let configs = HashMap::new();
    let client = MultiModelClient::new(configs, anthropic_config());

    // No models configured - should fail validation
    assert!(client.validate().is_err());
}

/// Test model max tokens configuration.
#[test]
fn test_model_max_tokens() {
    let mut configs = HashMap::new();
    configs.insert(
        "high-tokens".to_string(),
        ModelConfig {
            model_id: "claude-3-opus".to_string(),
            provider: ModelProvider::Anthropic,
            max_tokens: 200_000,
        },
    );
    configs.insert(
        "low-tokens".to_string(),
        ModelConfig {
            model_id: "claude-3-haiku".to_string(),
            provider: ModelProvider::Anthropic,
            max_tokens: 4096,
        },
    );

    let client = MultiModelClient::new(configs, anthropic_config());

    assert_eq!(
        client.get_model_config("high-tokens").unwrap().max_tokens,
        200_000
    );
    assert_eq!(
        client.get_model_config("low-tokens").unwrap().max_tokens,
        4096
    );
}
