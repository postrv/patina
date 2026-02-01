//! OAuth token refresh implementation.
//!
//! This module handles automatic refresh of expired OAuth access tokens
//! using the refresh token.
//!
//! # OAuth Status: DISABLED
//!
//! OAuth is currently disabled pending client_id registration with Anthropic.
//! See [`super::flow`] for details.
//!
//! # Example
//!
//! ```no_run
//! use patina::auth::refresh::refresh_token;
//! use patina::auth::OAuthCredentials;
//! use secrecy::SecretString;
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let old_creds = OAuthCredentials::new(
//!         SecretString::new("expired_access".into()),
//!         SecretString::new("valid_refresh".into()),
//!         Duration::from_secs(0), // Already expired
//!     );
//!
//!     let new_creds = refresh_token(&old_creds).await?;
//!     println!("Got new credentials, expires at {:?}", new_creds.expires_at());
//!
//!     Ok(())
//! }
//! ```

use std::time::Duration;

use anyhow::{bail, Context, Result};
use secrecy::{ExposeSecret, SecretString};
use tracing::{debug, info};

use super::OAuthCredentials;

/// Token endpoint URL.
///
/// Using the correct Anthropic console endpoint (pending client_id registration).
const TOKEN_URL: &str = "https://console.anthropic.com/oauth/token";

/// Client ID.
///
/// PLACEHOLDER: Must be replaced with a valid UUID from Anthropic's
/// developer registration process.
const CLIENT_ID: &str = "00000000-0000-0000-0000-000000000000";

/// Refreshes OAuth credentials using the refresh token.
///
/// # Arguments
///
/// * `credentials` - The current credentials containing the refresh token
///
/// # Returns
///
/// New OAuth credentials with a fresh access token.
///
/// # Errors
///
/// Returns an error if the token refresh request fails.
pub async fn refresh_token(credentials: &OAuthCredentials) -> Result<OAuthCredentials> {
    info!("Refreshing OAuth access token");

    let client = reqwest::Client::new();
    let response = client
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", CLIENT_ID),
            ("refresh_token", credentials.refresh_token().expose_secret()),
        ])
        .send()
        .await
        .context("Failed to send token refresh request")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!("Token refresh failed: {} - {}", status, body);
    }

    let token_response: TokenRefreshResponse = response
        .json()
        .await
        .context("Failed to parse token refresh response")?;

    debug!(
        expires_in = token_response.expires_in,
        "Token refresh successful"
    );

    // Use the new refresh token if provided, otherwise keep the old one
    let refresh_token = token_response
        .refresh_token
        .map(|t| SecretString::new(t.into()))
        .unwrap_or_else(|| credentials.refresh_token().clone());

    Ok(OAuthCredentials::new(
        SecretString::new(token_response.access_token.into()),
        refresh_token,
        Duration::from_secs(token_response.expires_in),
    ))
}

/// Response from the token refresh endpoint.
#[derive(Debug, serde::Deserialize)]
struct TokenRefreshResponse {
    access_token: String,
    /// Some OAuth providers return a new refresh token on refresh.
    refresh_token: Option<String>,
    expires_in: u64,
    #[allow(dead_code)]
    #[serde(default)]
    token_type: String,
}

/// Checks if credentials should be refreshed.
///
/// Returns true if the access token is expired or will expire
/// within the given buffer time.
///
/// # Arguments
///
/// * `credentials` - The credentials to check
/// * `buffer` - Time buffer before expiration to trigger refresh
#[must_use]
pub fn should_refresh(credentials: &OAuthCredentials, buffer: Duration) -> bool {
    credentials.is_expired() || credentials.expires_within(buffer)
}

/// Default buffer time before expiration to trigger refresh (60 seconds).
pub const DEFAULT_REFRESH_BUFFER: Duration = Duration::from_secs(60);

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    #[test]
    fn test_should_refresh_expired() {
        let creds = OAuthCredentials::with_expiry(
            SecretString::new("access".into()),
            SecretString::new("refresh".into()),
            SystemTime::now() - Duration::from_secs(10),
        );

        assert!(should_refresh(&creds, Duration::from_secs(60)));
    }

    #[test]
    fn test_should_refresh_expiring_soon() {
        let creds = OAuthCredentials::new(
            SecretString::new("access".into()),
            SecretString::new("refresh".into()),
            Duration::from_secs(30), // Expires in 30 seconds
        );

        assert!(should_refresh(&creds, Duration::from_secs(60)));
        assert!(!should_refresh(&creds, Duration::from_secs(10)));
    }

    #[test]
    fn test_should_refresh_valid() {
        let creds = OAuthCredentials::new(
            SecretString::new("access".into()),
            SecretString::new("refresh".into()),
            Duration::from_secs(3600), // Expires in 1 hour
        );

        assert!(!should_refresh(&creds, Duration::from_secs(60)));
        assert!(!should_refresh(&creds, Duration::from_secs(300)));
    }
}
