//! Authentication support for MCP servers.
//!
//! Provides bearer token resolution (including `$ENV_VAR` references) and an
//! OAuth client that manages token caching and refresh for MCP HTTP connections.
//!
//! # Bearer Token Resolution
//!
//! Bearer tokens can be specified as literal strings or as environment variable
//! references prefixed with `$`:
//!
//! ```
//! use patina::mcp::auth::resolve_bearer_token;
//! use secrecy::ExposeSecret;
//!
//! // Literal token
//! let token = resolve_bearer_token("my-secret-token").unwrap();
//! assert_eq!(token.expose_secret(), "my-secret-token");
//! ```
//!
//! # OAuth Client
//!
//! The [`McpOAuthClient`] handles cached token loading, expiration checks,
//! and automatic refresh via the OAuth `refresh_token` grant.

use crate::mcp::config::McpAuthConfig;
use crate::mcp::token_storage::{McpOAuthToken, McpTokenStore};
use anyhow::{anyhow, Context, Result};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

/// Resolves a bearer token from configuration.
///
/// If the token starts with `$`, it is treated as an environment variable
/// reference and the value is read from the process environment. Otherwise
/// the token string is returned as-is. The result is wrapped in a
/// [`SecretString`] to prevent accidental logging.
///
/// # Arguments
///
/// * `token_config` - A literal token string or `$ENV_VAR` reference
///
/// # Errors
///
/// Returns an error if:
/// - The token references an environment variable (`$VAR`) that is not set
/// - The token string is empty
///
/// # Examples
///
/// ```
/// use patina::mcp::auth::resolve_bearer_token;
/// use secrecy::ExposeSecret;
///
/// let token = resolve_bearer_token("literal-token").unwrap();
/// assert_eq!(token.expose_secret(), "literal-token");
/// ```
pub fn resolve_bearer_token(token_config: &str) -> Result<SecretString> {
    if token_config.is_empty() {
        return Err(anyhow!("Bearer token is empty"));
    }

    if let Some(var_name) = token_config.strip_prefix('$') {
        if var_name.is_empty() {
            return Err(anyhow!("Bearer token '$' has no variable name"));
        }
        let value = std::env::var(var_name).with_context(|| {
            format!("Environment variable '{var_name}' not set (referenced in MCP bearer token)")
        })?;
        Ok(SecretString::from(value))
    } else {
        Ok(SecretString::from(token_config.to_owned()))
    }
}

/// Creates HTTP headers containing the `Authorization: Bearer <token>` header.
///
/// # Arguments
///
/// * `token` - The bearer token value (as a `SecretString`)
///
/// # Returns
///
/// A vector of `(header_name, header_value)` pairs suitable for injecting
/// into HTTP requests.
///
/// # Examples
///
/// ```
/// use patina::mcp::auth::auth_headers;
/// use secrecy::SecretString;
///
/// let token = SecretString::from("my-token".to_owned());
/// let headers = auth_headers(&token);
/// assert_eq!(headers.len(), 1);
/// assert_eq!(headers[0].0, "Authorization");
/// assert_eq!(headers[0].1, "Bearer my-token");
/// ```
#[must_use]
pub fn auth_headers(token: &SecretString) -> Vec<(String, String)> {
    vec![(
        "Authorization".to_string(),
        format!("Bearer {}", token.expose_secret()),
    )]
}

/// OAuth client for a specific MCP server.
///
/// Manages token caching via [`McpTokenStore`] and automatic refresh when
/// tokens expire. Uses `reqwest` for HTTP calls to the token endpoint.
///
/// # Token Lifecycle
///
/// 1. On [`get_token`](McpOAuthClient::get_token): check store for cached token
/// 2. If cached and not expired, return it
/// 3. If cached but expired and has a refresh token, attempt refresh
/// 4. If no cached token or no refresh token, return an error
pub struct McpOAuthClient {
    client_id: String,
    scopes: Vec<String>,
    auth_url: String,
    token_url: String,
    token_store: McpTokenStore,
    server_name: String,
}

impl McpOAuthClient {
    /// Creates a new OAuth client from an [`McpAuthConfig::OAuth`] variant.
    ///
    /// # Errors
    ///
    /// Returns an error if the config is not an `OAuth` variant or if required
    /// URLs (`auth_url`, `token_url`) are missing.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use patina::mcp::auth::McpOAuthClient;
    /// use patina::mcp::config::McpAuthConfig;
    /// use patina::mcp::token_storage::McpTokenStore;
    ///
    /// # fn example() -> anyhow::Result<()> {
    /// let config = McpAuthConfig::OAuth {
    ///     client_id: "my-client".to_string(),
    ///     scopes: vec!["read".to_string()],
    ///     auth_url: Some("https://auth.example.com/authorize".to_string()),
    ///     token_url: Some("https://auth.example.com/token".to_string()),
    /// };
    /// let store = McpTokenStore::new("/tmp/tokens".into());
    /// let client = McpOAuthClient::new("my-server", &config, store)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(
        server_name: &str,
        config: &McpAuthConfig,
        token_store: McpTokenStore,
    ) -> Result<Self> {
        match config {
            McpAuthConfig::OAuth {
                client_id,
                scopes,
                auth_url,
                token_url,
            } => {
                let auth_url = auth_url.clone().ok_or_else(|| {
                    anyhow!("OAuth auth_url is required for server '{server_name}'")
                })?;
                let token_url = token_url.clone().ok_or_else(|| {
                    anyhow!("OAuth token_url is required for server '{server_name}'")
                })?;

                Ok(Self {
                    client_id: client_id.clone(),
                    scopes: scopes.clone(),
                    auth_url,
                    token_url,
                    token_store,
                    server_name: server_name.to_string(),
                })
            }
            McpAuthConfig::Bearer { .. } => Err(anyhow!(
                "Cannot create OAuth client from Bearer config for server '{server_name}'"
            )),
        }
    }

    /// Returns the authorization URL for this client.
    #[must_use]
    pub fn auth_url(&self) -> &str {
        &self.auth_url
    }

    /// Returns the token URL for this client.
    #[must_use]
    pub fn token_url(&self) -> &str {
        &self.token_url
    }

    /// Gets a valid access token, loading from cache or refreshing if expired.
    ///
    /// The returned token is wrapped in a [`SecretString`] to prevent accidental
    /// logging.
    ///
    /// # Token Resolution Order
    ///
    /// 1. Load from token store
    /// 2. If present and not expired, return the access token
    /// 3. If present but expired and has a refresh token, refresh it
    /// 4. Otherwise return an error
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - No cached token exists (initial auth flow not yet completed)
    /// - The token is expired and has no refresh token
    /// - The refresh request fails
    pub async fn get_token(&self) -> Result<SecretString> {
        let cached = self.token_store.load_token(&self.server_name)?;

        match cached {
            Some(ref t) if !McpOAuthToken::is_token_expired(t) => Ok(t.access_token.clone()),
            Some(ref t) if t.refresh_token.is_some() => {
                let rt = t.refresh_token.as_ref().expect("checked is_some above");
                let new_tok = self.refresh_token(rt.expose_secret()).await?;
                self.token_store.store_token(&self.server_name, &new_tok)?;
                Ok(new_tok.access_token)
            }
            Some(_) => Err(anyhow!(
                "MCP OAuth token for '{}' is expired and has no refresh token",
                self.server_name
            )),
            None => Err(anyhow!(
                "No cached OAuth token for MCP server '{}'. Initial authorization required.",
                self.server_name
            )),
        }
    }

    /// Refreshes the access token using a refresh token.
    ///
    /// Sends a POST request to the token endpoint with `grant_type=refresh_token`.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails or the response is not valid JSON.
    async fn refresh_token(&self, refresh_token: &str) -> Result<McpOAuthToken> {
        let client = reqwest::Client::new();

        let mut params = vec![
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", refresh_token.to_string()),
            ("client_id", self.client_id.clone()),
        ];

        if !self.scopes.is_empty() {
            params.push(("scope", self.scopes.join(" ")));
        }

        let response = client
            .post(&self.token_url)
            .form(&params)
            .send()
            .await
            .with_context(|| {
                format!(
                    "Failed to refresh OAuth token for MCP server '{}'",
                    self.server_name
                )
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            // Truncate error body to prevent leaking server-side secrets
            let truncated = if body.len() > 200 {
                format!("{}...[truncated]", &body[..200])
            } else {
                body
            };
            return Err(anyhow!(
                "OAuth token refresh failed for '{}': HTTP {} — {}",
                self.server_name,
                status,
                truncated
            ));
        }

        let token_response: TokenResponse = response
            .json()
            .await
            .context("Failed to parse OAuth token response")?;

        let expires_at = std::time::SystemTime::now()
            + std::time::Duration::from_secs(token_response.expires_in.unwrap_or(3600));

        Ok(McpOAuthToken::new(
            SecretString::from(token_response.access_token),
            Some(SecretString::from(
                token_response
                    .refresh_token
                    .unwrap_or_else(|| refresh_token.to_string()),
            )),
            expires_at,
        ))
    }

    /// Handles a 401 Unauthorized response by clearing the cached token and
    /// attempting to refresh.
    ///
    /// # Errors
    ///
    /// Returns an error if no refresh token is available or if the refresh fails.
    pub async fn handle_unauthorized(&self) -> Result<SecretString> {
        let cached = self.token_store.load_token(&self.server_name)?;

        match cached {
            Some(ref t) if t.refresh_token.is_some() => {
                let rt = t.refresh_token.as_ref().expect("checked is_some above");
                let new_tok = self.refresh_token(rt.expose_secret()).await?;
                self.token_store.store_token(&self.server_name, &new_tok)?;
                Ok(new_tok.access_token)
            }
            _ => Err(anyhow!(
                "Cannot handle 401 for MCP server '{}': no refresh token available",
                self.server_name
            )),
        }
    }
}

/// OAuth token endpoint response.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_bearer_token_literal() {
        let token = resolve_bearer_token("my-secret-token").unwrap();
        assert_eq!(token.expose_secret(), "my-secret-token");
    }

    #[test]
    #[serial_test::serial]
    fn resolve_bearer_token_env_var() {
        // Set a test env var — must be serialized to avoid data races
        unsafe { std::env::set_var("PATINA_TEST_MCP_TOKEN", "env-token-value") };
        let token = resolve_bearer_token("$PATINA_TEST_MCP_TOKEN").unwrap();
        assert_eq!(token.expose_secret(), "env-token-value");
        unsafe { std::env::remove_var("PATINA_TEST_MCP_TOKEN") };
    }

    #[test]
    #[serial_test::serial]
    fn resolve_bearer_token_missing_env_var() {
        // Make sure this var doesn't exist — must be serialized to avoid data races
        unsafe { std::env::remove_var("PATINA_NONEXISTENT_VAR_12345") };
        let result = resolve_bearer_token("$PATINA_NONEXISTENT_VAR_12345");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("PATINA_NONEXISTENT_VAR_12345"),
            "Error should mention the variable name: {err}"
        );
    }

    #[test]
    fn resolve_bearer_token_empty_returns_error() {
        let result = resolve_bearer_token("");
        assert!(result.is_err());
    }

    #[test]
    fn resolve_bearer_token_dollar_only_returns_error() {
        let result = resolve_bearer_token("$");
        assert!(result.is_err());
    }

    #[test]
    fn auth_headers_correct_format() {
        let token = SecretString::from("my-token".to_owned());
        let headers = auth_headers(&token);
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].0, "Authorization");
        assert_eq!(headers[0].1, "Bearer my-token");
    }

    #[test]
    fn oauth_client_new_from_oauth_config() {
        let config = McpAuthConfig::OAuth {
            client_id: "test-client".to_string(),
            scopes: vec!["read".to_string()],
            auth_url: Some("https://auth.example.com/authorize".to_string()),
            token_url: Some("https://auth.example.com/token".to_string()),
        };

        let dir = tempfile::TempDir::new().unwrap();
        let store = McpTokenStore::new(dir.path().to_path_buf());
        let client = McpOAuthClient::new("test-server", &config, store).unwrap();

        assert_eq!(client.auth_url(), "https://auth.example.com/authorize");
        assert_eq!(client.token_url(), "https://auth.example.com/token");
    }

    #[test]
    fn oauth_client_new_from_bearer_config_fails() {
        let config = McpAuthConfig::Bearer {
            token: "test-bearer-value".to_string(),
        };

        let dir = tempfile::TempDir::new().unwrap();
        let store = McpTokenStore::new(dir.path().to_path_buf());
        let result = McpOAuthClient::new("test-server", &config, store);
        assert!(result.is_err());
    }

    #[test]
    fn oauth_client_new_missing_auth_url_fails() {
        let config = McpAuthConfig::OAuth {
            client_id: "test-client".to_string(),
            scopes: vec![],
            auth_url: None,
            token_url: Some("https://auth.example.com/token".to_string()),
        };

        let dir = tempfile::TempDir::new().unwrap();
        let store = McpTokenStore::new(dir.path().to_path_buf());
        let result = McpOAuthClient::new("test-server", &config, store);
        assert!(result.is_err());
    }

    #[test]
    fn oauth_client_new_missing_token_url_fails() {
        let config = McpAuthConfig::OAuth {
            client_id: "test-client".to_string(),
            scopes: vec![],
            auth_url: Some("https://auth.example.com/authorize".to_string()),
            token_url: None,
        };

        let dir = tempfile::TempDir::new().unwrap();
        let store = McpTokenStore::new(dir.path().to_path_buf());
        let result = McpOAuthClient::new("test-server", &config, store);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn get_token_no_cached_returns_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = McpTokenStore::new(dir.path().to_path_buf());

        let config = McpAuthConfig::OAuth {
            client_id: "test-client".to_string(),
            scopes: vec![],
            auth_url: Some("https://auth.example.com/authorize".to_string()),
            token_url: Some("https://auth.example.com/token".to_string()),
        };

        let client = McpOAuthClient::new("no-cache", &config, store).unwrap();
        let result = client.get_token().await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("No cached"),
            "Error should indicate no cached token: {err}"
        );
    }

    #[tokio::test]
    async fn get_token_valid_cached_returns_token() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = McpTokenStore::new(dir.path().to_path_buf());

        // Pre-store a valid token
        let token = McpOAuthToken::new(
            SecretString::from("test-cached-access".to_owned()),
            Some(SecretString::from("test-cached-refresh".to_owned())),
            std::time::SystemTime::now() + std::time::Duration::from_secs(3600),
        );
        store.store_token("cached-server", &token).unwrap();

        let config = McpAuthConfig::OAuth {
            client_id: "test-client".to_string(),
            scopes: vec![],
            auth_url: Some("https://auth.example.com/authorize".to_string()),
            token_url: Some("https://auth.example.com/token".to_string()),
        };

        let client = McpOAuthClient::new("cached-server", &config, store).unwrap();
        let result = client.get_token().await.unwrap();
        assert_eq!(result.expose_secret(), "test-cached-access");
    }

    #[tokio::test]
    async fn get_token_expired_no_refresh_returns_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = McpTokenStore::new(dir.path().to_path_buf());

        // Pre-store an expired token with no refresh token
        let token = McpOAuthToken::new(
            SecretString::from("test-old-access".to_owned()),
            None,
            std::time::SystemTime::UNIX_EPOCH,
        );
        store.store_token("expired-server", &token).unwrap();

        let config = McpAuthConfig::OAuth {
            client_id: "test-client".to_string(),
            scopes: vec![],
            auth_url: Some("https://auth.example.com/authorize".to_string()),
            token_url: Some("https://auth.example.com/token".to_string()),
        };

        let client = McpOAuthClient::new("expired-server", &config, store).unwrap();
        let result = client.get_token().await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("expired") && err.contains("no refresh"),
            "Error should mention expired + no refresh: {err}"
        );
    }
}
