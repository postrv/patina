//! OAuth 2.0 authorization flow implementation.
//!
//! This module handles the browser-based OAuth flow for Claude subscription
//! authentication. The flow is:
//!
//! 1. Generate PKCE challenge
//! 2. Start local callback server
//! 3. Open browser to authorization URL
//! 4. Wait for callback with authorization code
//! 5. Exchange code for tokens
//! 6. Store tokens in keychain
//!
//! # Example
//!
//! ```no_run
//! use patina::auth::flow::OAuthFlow;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let flow = OAuthFlow::new();
//!     let credentials = flow.run().await?;
//!     println!("Got credentials: {:?}", credentials);
//!     Ok(())
//! }
//! ```

use std::time::Duration;

use anyhow::{bail, Context, Result};
use base64::engine::{general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::prelude::*;
use secrecy::SecretString;
use tiny_http::{Response, Server};
use tracing::{debug, info, warn};

use super::pkce::PkceChallenge;
use super::storage;
use super::OAuthCredentials;

/// Length of the state parameter in bytes (before base64 encoding).
///
/// RFC 6749 recommends using a random state parameter for CSRF protection.
/// 32 bytes provides 256 bits of entropy.
const STATE_LENGTH: usize = 32;

/// Default port for the local OAuth callback server.
/// Using the same port as Claude Code (54545) for compatibility.
const DEFAULT_CALLBACK_PORT: u16 = 54545;

/// Timeout for waiting for the OAuth callback.
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(300); // 5 minutes

/// OAuth configuration.
///
/// # OAuth Status: DISABLED
///
/// OAuth authentication is currently disabled because Patina does not have
/// a registered OAuth client_id with Anthropic.
///
/// Anthropic's OAuth endpoint requires `client_id` to be a valid UUID that
/// has been registered through their developer program. Using an unregistered
/// client_id (like "patina-cli") or borrowing another application's client_id
/// (like Claude Code's) would violate Anthropic's Terms of Service.
///
/// To enable OAuth in the future:
/// 1. Register Patina with Anthropic's developer program
/// 2. Obtain a valid UUID client_id
/// 3. Update the constants below with the registered values
/// 4. Remove the `OAUTH_DISABLED` flag
///
/// For now, users should authenticate using API keys via the `ANTHROPIC_API_KEY`
/// environment variable or the `--api-key` CLI flag.
mod config {
    /// Whether OAuth is currently disabled (pending client_id registration).
    pub const OAUTH_DISABLED: bool = true;

    /// Message explaining why OAuth is disabled.
    pub const OAUTH_DISABLED_MESSAGE: &str = "\
OAuth authentication is not yet available in Patina.

Anthropic's OAuth requires a registered client_id, which Patina does not
currently have. We are working with Anthropic to obtain proper OAuth
credentials.

In the meantime, please use API key authentication:
  • Set the ANTHROPIC_API_KEY environment variable, or
  • Use the --api-key flag

You can obtain an API key from: https://console.anthropic.com/settings/keys";

    /// The authorization endpoint URL.
    ///
    /// Correct URL: `https://console.anthropic.com/oauth/authorize`
    /// (Placeholder until we have a registered client_id)
    pub const AUTHORIZATION_URL: &str = "https://console.anthropic.com/oauth/authorize";

    /// The token endpoint URL.
    pub const TOKEN_URL: &str = "https://console.anthropic.com/oauth/token";

    /// The client ID for Patina.
    ///
    /// PLACEHOLDER: Must be replaced with a valid UUID from Anthropic's
    /// developer registration process.
    pub const CLIENT_ID: &str = "00000000-0000-0000-0000-000000000000";

    /// The scopes to request.
    ///
    /// Based on Claude Code's OAuth scopes.
    pub const SCOPES: &str = "org:create_api_key user:profile user:inference";

    /// The redirect URI (local callback).
    #[must_use]
    pub fn redirect_uri(port: u16) -> String {
        format!("http://localhost:{port}/callback")
    }
}

/// Manages the OAuth 2.0 authorization flow.
#[derive(Debug)]
pub struct OAuthFlow {
    /// Port for the local callback server.
    callback_port: u16,

    /// The PKCE challenge for this flow.
    pkce: PkceChallenge,

    /// The state parameter for CSRF protection (RFC 6749 Section 10.12).
    state: String,

    /// Custom OAuth client ID (overrides the default placeholder).
    ///
    /// When set, enables OAuth flow even if `OAUTH_DISABLED` is true,
    /// as this indicates the user has a registered client ID.
    custom_client_id: Option<String>,
}

impl Default for OAuthFlow {
    fn default() -> Self {
        Self::new()
    }
}

impl OAuthFlow {
    /// Creates a new OAuth flow with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            callback_port: DEFAULT_CALLBACK_PORT,
            pkce: PkceChallenge::generate(),
            state: generate_state(),
            custom_client_id: None,
        }
    }

    /// Creates a new OAuth flow with a custom callback port.
    #[must_use]
    pub fn with_port(port: u16) -> Self {
        Self {
            callback_port: port,
            pkce: PkceChallenge::generate(),
            state: generate_state(),
            custom_client_id: None,
        }
    }

    /// Sets a custom OAuth client ID.
    ///
    /// When a custom client ID is provided, OAuth is enabled even if
    /// `OAUTH_DISABLED` is true in the config, as this indicates the
    /// user has obtained a registered client ID from Anthropic.
    #[must_use]
    pub fn with_client_id(mut self, client_id: impl Into<String>) -> Self {
        self.custom_client_id = Some(client_id.into());
        self
    }

    /// Returns the custom client ID if set.
    #[must_use]
    pub fn client_id(&self) -> Option<&str> {
        self.custom_client_id.as_deref()
    }

    /// Returns the effective client ID to use in the OAuth flow.
    ///
    /// Uses the custom client ID if set, otherwise falls back to the default.
    #[must_use]
    fn effective_client_id(&self) -> &str {
        self.custom_client_id
            .as_deref()
            .unwrap_or(config::CLIENT_ID)
    }

    /// Returns whether OAuth is enabled for this flow.
    ///
    /// OAuth is enabled if:
    /// - A custom client ID has been provided, OR
    /// - The `OAUTH_DISABLED` flag is false
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.custom_client_id.is_some() || !config::OAUTH_DISABLED
    }

    /// Returns the state parameter for this flow.
    ///
    /// Used for CSRF protection per RFC 6749 Section 10.12.
    #[must_use]
    pub fn state(&self) -> &str {
        &self.state
    }

    /// Builds the authorization URL.
    #[must_use]
    pub fn authorization_url(&self) -> String {
        let redirect_uri = config::redirect_uri(self.callback_port);
        format!(
            "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method={}",
            config::AUTHORIZATION_URL,
            urlencoding::encode(self.effective_client_id()),
            urlencoding::encode(&redirect_uri),
            urlencoding::encode(config::SCOPES),
            urlencoding::encode(&self.state),
            urlencoding::encode(self.pkce.challenge()),
            self.pkce.challenge_method()
        )
    }

    /// Runs the complete OAuth flow.
    ///
    /// This method:
    /// 1. Starts a local callback server
    /// 2. Opens the browser to the authorization URL
    /// 3. Waits for the callback with the authorization code
    /// 4. Exchanges the code for tokens
    /// 5. Stores the tokens in the keychain
    ///
    /// # Errors
    ///
    /// Returns an error if OAuth is disabled (pending client_id registration)
    /// or if any step of the flow fails.
    pub async fn run(&self) -> Result<OAuthCredentials> {
        // Check if OAuth is enabled
        if !self.is_enabled() {
            bail!("{}", config::OAUTH_DISABLED_MESSAGE);
        }

        info!("Starting OAuth login flow");

        // Start local callback server
        let server = Server::http(format!("127.0.0.1:{}", self.callback_port)).map_err(|e| {
            anyhow::anyhow!(
                "Failed to start callback server on port {}: {}",
                self.callback_port,
                e
            )
        })?;

        // Build authorization URL
        let auth_url = self.authorization_url();
        debug!(url = %auth_url, "Opening browser for authorization");

        // Open browser
        if let Err(e) = webbrowser::open(&auth_url) {
            warn!(error = %e, "Failed to open browser automatically");
            println!("\nPlease open this URL in your browser to authenticate:");
            println!("{}\n", auth_url);
        } else {
            info!("Opened browser for authentication");
            println!("\nOpened browser for authentication. Please complete the login flow.");
        }

        println!(
            "Waiting for authorization (timeout: {} seconds)...",
            CALLBACK_TIMEOUT.as_secs()
        );

        // Wait for callback
        let code = self.wait_for_callback(server).await?;
        debug!("Received authorization code");

        // Exchange code for tokens
        let credentials = self.exchange_code(&code).await?;
        info!("Successfully obtained OAuth tokens");

        // Store in keychain
        storage::store_oauth_credentials(&credentials).await?;
        info!("Stored credentials in keychain");

        Ok(credentials)
    }

    /// Waits for the OAuth callback with the authorization code.
    async fn wait_for_callback(&self, server: Server) -> Result<String> {
        // Create a channel to receive the result
        let (tx, rx) = tokio::sync::oneshot::channel();
        let expected_state = self.state.clone();

        // Use a thread for the blocking HTTP server
        std::thread::spawn(move || {
            // Set timeout on the server
            for request in server.incoming_requests() {
                let url = request.url().to_string();

                // Parse the URL to extract the code
                if let Some(code) = extract_code_from_url(&url) {
                    // Validate state parameter for CSRF protection (RFC 6749 Section 10.12)
                    let callback_state = extract_state_from_url(&url);
                    if callback_state.as_deref() != Some(expected_state.as_str()) {
                        // State mismatch - possible CSRF attack
                        let response = Response::from_string(
                            "<html><body><h1>Authentication failed</h1><p>State parameter mismatch. Please try again.</p></body></html>"
                        ).with_header(
                            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/html"[..]).unwrap()
                        );
                        let _ = request.respond(response);
                        let _ = tx.send(Err(anyhow::anyhow!(
                            "State parameter mismatch - possible CSRF attack"
                        )));
                        return;
                    }

                    // Send success response
                    let response = Response::from_string(
                        "<html><body><h1>Authentication successful!</h1><p>You can close this window.</p></body></html>"
                    ).with_header(
                        tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/html"[..]).unwrap()
                    );
                    let _ = request.respond(response);

                    let _ = tx.send(Ok(code));
                    return;
                } else if let Some(error) = extract_error_from_url(&url) {
                    // Send error response
                    let response = Response::from_string(format!(
                        "<html><body><h1>Authentication failed</h1><p>{}</p></body></html>",
                        error
                    ))
                    .with_header(
                        tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/html"[..])
                            .unwrap(),
                    );
                    let _ = request.respond(response);

                    let _ = tx.send(Err(anyhow::anyhow!("OAuth error: {}", error)));
                    return;
                } else {
                    // Unexpected request, send 404
                    let response = Response::from_string("Not found").with_status_code(404);
                    let _ = request.respond(response);
                }
            }
        });

        // Wait for result with timeout
        tokio::select! {
            result = rx => {
                result.map_err(|_| anyhow::anyhow!("Callback server channel closed"))?
            }
            _ = tokio::time::sleep(CALLBACK_TIMEOUT) => {
                bail!("OAuth callback timed out after {} seconds", CALLBACK_TIMEOUT.as_secs())
            }
        }
    }

    /// Exchanges the authorization code for tokens.
    async fn exchange_code(&self, code: &str) -> Result<OAuthCredentials> {
        let redirect_uri = config::redirect_uri(self.callback_port);

        let client = reqwest::Client::new();
        let response: reqwest::Response = client
            .post(config::TOKEN_URL)
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", self.effective_client_id()),
                ("code", code),
                ("redirect_uri", &redirect_uri),
                ("code_verifier", self.pkce.verifier()),
            ])
            .send()
            .await
            .context("Failed to send token exchange request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body: String = response.text().await.unwrap_or_default();
            bail!("Token exchange failed: {} - {}", status, body);
        }

        let token_response: TokenResponse = response
            .json()
            .await
            .context("Failed to parse token response")?;

        Ok(OAuthCredentials::new(
            SecretString::new(token_response.access_token.into()),
            SecretString::new(token_response.refresh_token.into()),
            Duration::from_secs(token_response.expires_in),
        ))
    }
}

/// Response from the token endpoint.
#[derive(Debug, serde::Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
    #[serde(default, rename = "token_type")]
    _token_type: String,
}

/// Generates a cryptographically random state parameter for CSRF protection.
///
/// The state is base64url encoded to ensure it contains only safe characters.
fn generate_state() -> String {
    let mut rng = rand::thread_rng();
    let random_bytes: Vec<u8> = (0..STATE_LENGTH).map(|_| rng.gen()).collect();
    URL_SAFE_NO_PAD.encode(random_bytes)
}

/// Extracts the authorization code from a callback URL.
fn extract_code_from_url(url: &str) -> Option<String> {
    // URL format: /callback?code=ABC123&state=...
    url.split('?').nth(1)?.split('&').find_map(|param| {
        let (key, value) = param.split_once('=')?;
        if key == "code" {
            Some(urlencoding::decode(value).ok()?.into_owned())
        } else {
            None
        }
    })
}

/// Extracts the state parameter from a callback URL.
fn extract_state_from_url(url: &str) -> Option<String> {
    url.split('?').nth(1)?.split('&').find_map(|param| {
        let (key, value) = param.split_once('=')?;
        if key == "state" {
            Some(urlencoding::decode(value).ok()?.into_owned())
        } else {
            None
        }
    })
}

/// Extracts the error from a callback URL.
fn extract_error_from_url(url: &str) -> Option<String> {
    url.split('?').nth(1)?.split('&').find_map(|param| {
        let (key, value) = param.split_once('=')?;
        if key == "error" || key == "error_description" {
            Some(urlencoding::decode(value).ok()?.into_owned())
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_code_from_url() {
        let url = "/callback?code=ABC123&state=xyz";
        assert_eq!(extract_code_from_url(url), Some("ABC123".to_string()));

        let url = "/callback?state=xyz&code=DEF456";
        assert_eq!(extract_code_from_url(url), Some("DEF456".to_string()));

        let url = "/callback?error=access_denied";
        assert_eq!(extract_code_from_url(url), None);

        let url = "/callback";
        assert_eq!(extract_code_from_url(url), None);
    }

    #[test]
    fn test_extract_state_from_url() {
        let url = "/callback?code=ABC123&state=xyz123";
        assert_eq!(extract_state_from_url(url), Some("xyz123".to_string()));

        let url = "/callback?state=abc456&code=DEF789";
        assert_eq!(extract_state_from_url(url), Some("abc456".to_string()));

        let url = "/callback?code=ABC123";
        assert_eq!(extract_state_from_url(url), None);

        let url = "/callback?state=url%20encoded%20state";
        assert_eq!(
            extract_state_from_url(url),
            Some("url encoded state".to_string())
        );
    }

    #[test]
    fn test_extract_error_from_url() {
        let url = "/callback?error=access_denied";
        assert_eq!(
            extract_error_from_url(url),
            Some("access_denied".to_string())
        );

        let url = "/callback?error_description=User%20denied%20access";
        assert_eq!(
            extract_error_from_url(url),
            Some("User denied access".to_string())
        );

        let url = "/callback?code=ABC123";
        assert_eq!(extract_error_from_url(url), None);
    }

    #[test]
    fn test_oauth_flow_new() {
        let flow = OAuthFlow::new();
        assert_eq!(flow.callback_port, DEFAULT_CALLBACK_PORT);
    }

    #[test]
    fn test_oauth_flow_with_port() {
        let flow = OAuthFlow::with_port(8080);
        assert_eq!(flow.callback_port, 8080);
    }

    #[test]
    fn test_oauth_flow_has_state() {
        let flow = OAuthFlow::new();
        // State should be a non-empty base64url encoded string
        let state = flow.state();
        assert!(!state.is_empty(), "State parameter should not be empty");
        // 32 bytes base64url encoded = 43 characters
        assert_eq!(
            state.len(),
            43,
            "State should be 43 chars (32 bytes base64url)"
        );
    }

    #[test]
    fn test_oauth_flow_unique_state_per_instance() {
        let flow1 = OAuthFlow::new();
        let flow2 = OAuthFlow::new();

        assert_ne!(
            flow1.state(),
            flow2.state(),
            "Each OAuthFlow instance should have unique state"
        );
    }

    #[test]
    fn test_state_uses_valid_base64url_chars() {
        // Generate multiple states to increase confidence
        for _ in 0..10 {
            let state = generate_state();
            for c in state.chars() {
                assert!(
                    c.is_ascii_alphanumeric() || c == '-' || c == '_',
                    "Invalid character in state: {c}"
                );
            }
        }
    }

    #[test]
    fn test_authorization_url_format() {
        let flow = OAuthFlow::new();
        let url = flow.authorization_url();

        assert!(url.starts_with(config::AUTHORIZATION_URL));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id="));
        assert!(url.contains("redirect_uri="));
        assert!(url.contains("state="));
        assert!(url.contains("code_challenge="));
        assert!(url.contains("code_challenge_method=S256"));
    }

    #[test]
    fn test_authorization_url_includes_state_parameter() {
        let flow = OAuthFlow::new();
        let url = flow.authorization_url();

        // Verify state parameter is present and matches the flow's state
        assert!(
            url.contains(&format!("state={}", urlencoding::encode(flow.state()))),
            "Auth URL should include the flow's state parameter"
        );
    }

    // =========================================================================
    // Task 0.8.4: OAuth client_id injection tests
    // =========================================================================

    #[test]
    fn test_oauth_flow_with_client_id() {
        let custom_id = "12345678-1234-1234-1234-123456789abc";
        let flow = OAuthFlow::new().with_client_id(custom_id);

        assert_eq!(flow.client_id(), Some(custom_id));
    }

    #[test]
    fn test_oauth_flow_default_client_id_is_none() {
        let flow = OAuthFlow::new();
        assert!(flow.client_id().is_none());
    }

    #[test]
    fn test_authorization_url_uses_custom_client_id() {
        let custom_id = "my-custom-client-id";
        let flow = OAuthFlow::new().with_client_id(custom_id);
        let url = flow.authorization_url();

        assert!(
            url.contains(&format!("client_id={}", urlencoding::encode(custom_id))),
            "Auth URL should use the custom client_id"
        );
    }

    #[test]
    fn test_authorization_url_uses_default_client_id_when_none() {
        let flow = OAuthFlow::new();
        let url = flow.authorization_url();

        assert!(
            url.contains(&format!(
                "client_id={}",
                urlencoding::encode(config::CLIENT_ID)
            )),
            "Auth URL should use default client_id when custom is not set"
        );
    }

    #[test]
    fn test_oauth_flow_is_enabled_with_custom_client_id() {
        let flow = OAuthFlow::new().with_client_id("custom-id");
        assert!(
            flow.is_enabled(),
            "OAuth should be enabled when custom client_id is provided"
        );
    }

    #[test]
    fn test_oauth_flow_is_disabled_by_default() {
        let flow = OAuthFlow::new();
        // OAuth is disabled by default (no custom client_id and OAUTH_DISABLED is true)
        assert!(!flow.is_enabled(), "OAuth should be disabled by default");
    }
}
