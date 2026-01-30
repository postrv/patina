//! Unit tests for multi-model API key security.
//!
//! These tests verify that API keys are properly protected using SecretString
//! to prevent accidental exposure in logs, debug output, or memory dumps.
//!
//! Security issue: H-1 - Plain string API key in multi_model

use rct::api::multi_model::{AnthropicConfig, ProviderConfig};
use secrecy::{ExposeSecret, SecretString};

/// The test API key used in security tests.
/// Using a realistic-looking but clearly fake key to ensure tests catch exposure.
const TEST_API_KEY: &str = "sk-ant-api03-FAKE-KEY-DO-NOT-USE-IN-PRODUCTION-aaaaaaaa";

// =============================================================================
// 0.2.1 API Key Secrecy Tests (RED - these should fail initially)
// =============================================================================

/// Test that API key does not appear in Debug output of ProviderConfig.
///
/// Security rationale: Debug output is often logged and could expose secrets.
/// The API key must be redacted when formatting the config for debugging.
///
/// Expected behavior after fix: Debug output shows "[REDACTED]" or similar
/// instead of the actual API key value.
#[test]
fn test_api_key_not_in_debug_output_provider_config() {
    let config = ProviderConfig::Anthropic {
        api_key: SecretString::new(TEST_API_KEY.into()),
    };

    // Format as debug string
    let debug_output = format!("{:?}", config);

    // The API key should NOT appear in debug output
    assert!(
        !debug_output.contains(TEST_API_KEY),
        "SECURITY ISSUE: API key '{}' was exposed in Debug output: {}",
        TEST_API_KEY,
        debug_output
    );

    // Should contain some indication that the value is redacted
    assert!(
        debug_output.contains("REDACTED")
            || debug_output.contains("***")
            || debug_output.contains("Secret"),
        "Debug output should indicate the API key is redacted, got: {}",
        debug_output
    );
}

/// Test that API key does not appear in Debug output of AnthropicConfig.
///
/// Security rationale: Same as above - any struct containing API keys
/// must not expose them in Debug output.
#[test]
fn test_api_key_not_in_debug_output_anthropic_config() {
    let config = AnthropicConfig {
        api_key: SecretString::new(TEST_API_KEY.into()),
    };

    // Format as debug string
    let debug_output = format!("{:?}", config);

    // The API key should NOT appear in debug output
    assert!(
        !debug_output.contains(TEST_API_KEY),
        "SECURITY ISSUE: API key '{}' was exposed in Debug output: {}",
        TEST_API_KEY,
        debug_output
    );

    // Should contain some indication that the value is redacted
    assert!(
        debug_output.contains("REDACTED")
            || debug_output.contains("***")
            || debug_output.contains("Secret"),
        "Debug output should indicate the API key is redacted, got: {}",
        debug_output
    );
}

/// Test that API key can still be accessed when needed for actual API calls.
///
/// While the key must be protected from accidental exposure, it still needs
/// to be accessible for legitimate use (making API calls).
///
/// This test verifies the key can be retrieved via expose_secret().
#[test]
fn test_api_key_accessible_via_expose_secret() {
    let config = ProviderConfig::Anthropic {
        api_key: SecretString::new(TEST_API_KEY.into()),
    };

    // The key should be accessible via expose_secret()
    if let ProviderConfig::Anthropic { api_key } = config {
        // After the fix, api_key will be SecretString and we call expose_secret()
        // Currently with String, this will fail to compile until we implement the fix
        let exposed = api_key.expose_secret();
        assert_eq!(
            exposed, TEST_API_KEY,
            "API key should be retrievable via expose_secret()"
        );
    } else {
        panic!("Expected Anthropic config");
    }
}

/// Test that AnthropicConfig API key can still be accessed when needed.
#[test]
fn test_anthropic_config_api_key_accessible_via_expose_secret() {
    let config = AnthropicConfig {
        api_key: SecretString::new(TEST_API_KEY.into()),
    };

    // After the fix, api_key will be SecretString and we call expose_secret()
    let exposed = config.api_key.expose_secret();
    assert_eq!(
        exposed, TEST_API_KEY,
        "API key should be retrievable via expose_secret()"
    );
}
