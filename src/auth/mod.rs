//! Authentication module for Patina.
//!
//! This module provides support for multiple authentication methods:
//! - API key authentication (traditional)
//! - OAuth 2.0 authentication (Claude subscription)
//!
//! # Architecture
//!
//! ```text
//! Startup
//!     ↓
//! Check for stored OAuth token (keychain)
//!     ├─ Found + valid → Use OAuth
//!     ├─ Found + expired → Refresh token
//!     └─ Not found → Check for API key
//!         ├─ Found → Use API key
//!         └─ Not found → Error
//! ```
//!
//! # Example
//!
//! ```no_run
//! use patina::auth::{AuthMethod, AuthManager};
//! use secrecy::SecretString;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let mut manager = AuthManager::new();
//!
//!     // Try to get authentication
//!     let auth = manager.get_auth().await?;
//!
//!     match auth {
//!         AuthMethod::ApiKey(key) => println!("Using API key"),
//!         AuthMethod::OAuth(creds) => println!("Using OAuth, expires at {:?}", creds.expires_at()),
//!     }
//!
//!     Ok(())
//! }
//! ```

pub mod flow;
pub mod pkce;
pub mod refresh;
pub mod storage;

use std::fmt;
use std::time::{Duration, SystemTime};

use anyhow::{bail, Result};
use secrecy::{ExposeSecret, SecretString};

/// Authentication method for API access.
///
/// Patina supports two authentication methods:
/// - API key: Traditional `ANTHROPIC_API_KEY` authentication
/// - OAuth: Browser-based OAuth 2.0 flow for Claude subscription users
#[derive(Clone)]
pub enum AuthMethod {
    /// API key authentication.
    ///
    /// The key is stored as a [`SecretString`] to prevent accidental exposure.
    ApiKey(SecretString),

    /// OAuth 2.0 authentication with tokens.
    OAuth(OAuthCredentials),
}

impl fmt::Debug for AuthMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ApiKey(_) => f.debug_tuple("ApiKey").field(&"[REDACTED]").finish(),
            Self::OAuth(creds) => f.debug_tuple("OAuth").field(creds).finish(),
        }
    }
}

impl AuthMethod {
    /// Returns true if this is an API key authentication method.
    #[must_use]
    pub fn is_api_key(&self) -> bool {
        matches!(self, Self::ApiKey(_))
    }

    /// Returns true if this is an OAuth authentication method.
    #[must_use]
    pub fn is_oauth(&self) -> bool {
        matches!(self, Self::OAuth(_))
    }

    /// Returns the API key if this is an API key method.
    #[must_use]
    pub fn api_key(&self) -> Option<&SecretString> {
        match self {
            Self::ApiKey(key) => Some(key),
            Self::OAuth(_) => None,
        }
    }

    /// Returns the OAuth credentials if this is an OAuth method.
    #[must_use]
    pub fn oauth_credentials(&self) -> Option<&OAuthCredentials> {
        match self {
            Self::ApiKey(_) => None,
            Self::OAuth(creds) => Some(creds),
        }
    }

    /// Returns the authorization header value for API requests.
    ///
    /// For API key: returns the key directly
    /// For OAuth: returns the access token in Bearer format
    #[must_use]
    pub fn authorization_header(&self) -> String {
        match self {
            Self::ApiKey(key) => key.expose_secret().to_string(),
            Self::OAuth(creds) => format!("Bearer {}", creds.access_token.expose_secret()),
        }
    }
}

/// OAuth 2.0 credentials.
///
/// Contains the access token, refresh token, and expiration time.
/// All tokens are stored as [`SecretString`] to prevent accidental exposure.
#[derive(Clone)]
pub struct OAuthCredentials {
    /// The access token for API requests.
    access_token: SecretString,

    /// The refresh token for obtaining new access tokens.
    refresh_token: SecretString,

    /// When the access token expires.
    expires_at: SystemTime,

    /// The token type (usually "Bearer").
    token_type: String,
}

impl fmt::Debug for OAuthCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthCredentials")
            .field("access_token", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .field("token_type", &self.token_type)
            .finish()
    }
}

impl OAuthCredentials {
    /// Creates new OAuth credentials.
    ///
    /// # Arguments
    ///
    /// * `access_token` - The access token for API requests
    /// * `refresh_token` - The refresh token for obtaining new access tokens
    /// * `expires_in` - Duration until the access token expires
    #[must_use]
    pub fn new(
        access_token: SecretString,
        refresh_token: SecretString,
        expires_in: Duration,
    ) -> Self {
        Self {
            access_token,
            refresh_token,
            expires_at: SystemTime::now() + expires_in,
            token_type: "Bearer".to_string(),
        }
    }

    /// Creates credentials with an explicit expiration time.
    #[must_use]
    pub fn with_expiry(
        access_token: SecretString,
        refresh_token: SecretString,
        expires_at: SystemTime,
    ) -> Self {
        Self {
            access_token,
            refresh_token,
            expires_at,
            token_type: "Bearer".to_string(),
        }
    }

    /// Returns the access token.
    #[must_use]
    pub fn access_token(&self) -> &SecretString {
        &self.access_token
    }

    /// Returns the refresh token.
    #[must_use]
    pub fn refresh_token(&self) -> &SecretString {
        &self.refresh_token
    }

    /// Returns when the access token expires.
    #[must_use]
    pub fn expires_at(&self) -> SystemTime {
        self.expires_at
    }

    /// Returns the token type.
    #[must_use]
    pub fn token_type(&self) -> &str {
        &self.token_type
    }

    /// Returns true if the access token has expired.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        SystemTime::now() > self.expires_at
    }

    /// Returns true if the access token will expire within the given duration.
    ///
    /// Useful for proactively refreshing tokens before they expire.
    #[must_use]
    pub fn expires_within(&self, duration: Duration) -> bool {
        SystemTime::now() + duration > self.expires_at
    }

    /// Returns the time remaining until expiration.
    ///
    /// Returns `Duration::ZERO` if already expired.
    #[must_use]
    pub fn time_remaining(&self) -> Duration {
        self.expires_at
            .duration_since(SystemTime::now())
            .unwrap_or(Duration::ZERO)
    }
}

/// Manages authentication for Patina.
///
/// The manager handles:
/// - Loading OAuth credentials from the OS keychain
/// - Refreshing expired OAuth tokens
/// - Falling back to API key authentication
pub struct AuthManager {
    /// The current authentication method.
    current_auth: Option<AuthMethod>,

    /// Whether to force API key usage (--use-api-key flag).
    force_api_key: bool,

    /// Cached API key from environment.
    api_key: Option<SecretString>,
}

impl Default for AuthManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AuthManager {
    /// Creates a new authentication manager.
    #[must_use]
    pub fn new() -> Self {
        Self {
            current_auth: None,
            force_api_key: false,
            api_key: None,
        }
    }

    /// Sets the API key for fallback authentication.
    pub fn set_api_key(&mut self, key: SecretString) {
        self.api_key = Some(key);
    }

    /// Forces use of API key even if OAuth is available.
    pub fn set_force_api_key(&mut self, force: bool) {
        self.force_api_key = force;
    }

    /// Gets the current authentication method.
    ///
    /// This will:
    /// 1. If force_api_key is true, use API key
    /// 2. Try to load OAuth credentials from keychain
    /// 3. If OAuth is expired, refresh it
    /// 4. Fall back to API key
    ///
    /// # Errors
    ///
    /// Returns an error if no valid authentication method is available.
    pub async fn get_auth(&mut self) -> Result<AuthMethod> {
        // Return cached auth if available and valid
        if let Some(ref auth) = self.current_auth {
            match auth {
                AuthMethod::ApiKey(_) => return Ok(auth.clone()),
                AuthMethod::OAuth(creds) if !creds.expires_within(Duration::from_secs(60)) => {
                    return Ok(auth.clone())
                }
                AuthMethod::OAuth(_) => {
                    // Token is about to expire, try to refresh
                }
            }
        }

        // If forcing API key, use it directly
        if self.force_api_key {
            return self.get_api_key_auth();
        }

        // Try to load OAuth from keychain
        match storage::load_oauth_credentials().await {
            Ok(Some(creds)) => {
                if creds.is_expired() || creds.expires_within(Duration::from_secs(60)) {
                    // Try to refresh
                    match refresh::refresh_token(&creds).await {
                        Ok(new_creds) => {
                            // Store the refreshed credentials
                            storage::store_oauth_credentials(&new_creds).await?;
                            let auth = AuthMethod::OAuth(new_creds);
                            self.current_auth = Some(auth.clone());
                            return Ok(auth);
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "Failed to refresh OAuth token, falling back to API key");
                        }
                    }
                } else {
                    let auth = AuthMethod::OAuth(creds);
                    self.current_auth = Some(auth.clone());
                    return Ok(auth);
                }
            }
            Ok(None) => {
                // No OAuth credentials stored
            }
            Err(e) => {
                tracing::debug!(error = %e, "Failed to load OAuth credentials");
            }
        }

        // Fall back to API key
        self.get_api_key_auth()
    }

    /// Gets API key authentication.
    fn get_api_key_auth(&mut self) -> Result<AuthMethod> {
        if let Some(ref key) = self.api_key {
            let auth = AuthMethod::ApiKey(key.clone());
            self.current_auth = Some(auth.clone());
            return Ok(auth);
        }

        // Try environment variable
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            let secret = SecretString::new(key.into());
            self.api_key = Some(secret.clone());
            let auth = AuthMethod::ApiKey(secret);
            self.current_auth = Some(auth.clone());
            return Ok(auth);
        }

        bail!("No authentication available. Set ANTHROPIC_API_KEY or use --oauth-login")
    }

    /// Clears the current authentication state.
    pub fn clear(&mut self) {
        self.current_auth = None;
    }

    /// Returns true if OAuth credentials are currently loaded.
    #[must_use]
    pub fn has_oauth(&self) -> bool {
        matches!(self.current_auth, Some(AuthMethod::OAuth(_)))
    }

    /// Returns true if an API key is available.
    #[must_use]
    pub fn has_api_key(&self) -> bool {
        self.api_key.is_some() || std::env::var("ANTHROPIC_API_KEY").is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // AuthMethod tests
    // =========================================================================

    #[test]
    fn test_auth_method_api_key() {
        let auth = AuthMethod::ApiKey(SecretString::new("sk-test".into()));

        assert!(auth.is_api_key());
        assert!(!auth.is_oauth());
        assert!(auth.api_key().is_some());
        assert!(auth.oauth_credentials().is_none());
    }

    #[test]
    fn test_auth_method_oauth() {
        let creds = OAuthCredentials::new(
            SecretString::new("access".into()),
            SecretString::new("refresh".into()),
            Duration::from_secs(3600),
        );
        let auth = AuthMethod::OAuth(creds);

        assert!(!auth.is_api_key());
        assert!(auth.is_oauth());
        assert!(auth.api_key().is_none());
        assert!(auth.oauth_credentials().is_some());
    }

    #[test]
    fn test_auth_method_authorization_header_api_key() {
        let auth = AuthMethod::ApiKey(SecretString::new("sk-test-key".into()));
        assert_eq!(auth.authorization_header(), "sk-test-key");
    }

    #[test]
    fn test_auth_method_authorization_header_oauth() {
        let creds = OAuthCredentials::new(
            SecretString::new("my-access-token".into()),
            SecretString::new("refresh".into()),
            Duration::from_secs(3600),
        );
        let auth = AuthMethod::OAuth(creds);
        assert_eq!(auth.authorization_header(), "Bearer my-access-token");
    }

    #[test]
    fn test_auth_method_debug_redacts_secrets() {
        let auth = AuthMethod::ApiKey(SecretString::new("sk-secret".into()));
        let debug = format!("{auth:?}");
        assert!(!debug.contains("sk-secret"));
        assert!(debug.contains("[REDACTED]"));
    }

    // =========================================================================
    // OAuthCredentials tests
    // =========================================================================

    #[test]
    fn test_oauth_credentials_new() {
        let creds = OAuthCredentials::new(
            SecretString::new("access".into()),
            SecretString::new("refresh".into()),
            Duration::from_secs(3600),
        );

        assert_eq!(creds.token_type(), "Bearer");
        assert!(!creds.is_expired());
    }

    #[test]
    fn test_oauth_credentials_expired() {
        let creds = OAuthCredentials::with_expiry(
            SecretString::new("access".into()),
            SecretString::new("refresh".into()),
            SystemTime::now() - Duration::from_secs(10), // Already expired
        );

        assert!(creds.is_expired());
        assert_eq!(creds.time_remaining(), Duration::ZERO);
    }

    #[test]
    fn test_oauth_credentials_expires_within() {
        let creds = OAuthCredentials::new(
            SecretString::new("access".into()),
            SecretString::new("refresh".into()),
            Duration::from_secs(30), // Expires in 30 seconds
        );

        assert!(creds.expires_within(Duration::from_secs(60))); // Will expire within 60s
        assert!(!creds.expires_within(Duration::from_secs(10))); // Won't expire within 10s
    }

    #[test]
    fn test_oauth_credentials_time_remaining() {
        let creds = OAuthCredentials::new(
            SecretString::new("access".into()),
            SecretString::new("refresh".into()),
            Duration::from_secs(3600),
        );

        let remaining = creds.time_remaining();
        assert!(remaining > Duration::from_secs(3590)); // Should be close to 3600
        assert!(remaining <= Duration::from_secs(3600));
    }

    #[test]
    fn test_oauth_credentials_debug_redacts_secrets() {
        let creds = OAuthCredentials::new(
            SecretString::new("secret-access".into()),
            SecretString::new("secret-refresh".into()),
            Duration::from_secs(3600),
        );

        let debug = format!("{creds:?}");
        assert!(!debug.contains("secret-access"));
        assert!(!debug.contains("secret-refresh"));
        assert!(debug.contains("[REDACTED]"));
    }

    // =========================================================================
    // AuthManager tests
    // =========================================================================

    #[test]
    fn test_auth_manager_new() {
        let manager = AuthManager::new();
        assert!(!manager.has_oauth());
    }

    #[test]
    fn test_auth_manager_set_api_key() {
        let mut manager = AuthManager::new();
        manager.set_api_key(SecretString::new("sk-test".into()));
        assert!(manager.has_api_key());
    }
}
