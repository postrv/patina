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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use secrecy::{ExposeSecret, SecretString};
use tokio::task::JoinHandle;
use tokio::time::{interval, sleep};
use tracing::{debug, info, warn};

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
    #[serde(default, rename = "token_type")]
    _token_type: String,
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

/// Default buffer for background refresh (5 minutes before expiry).
const BACKGROUND_REFRESH_BUFFER: Duration = Duration::from_secs(300);

/// Initial backoff delay on refresh failure (1 second).
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);

/// Maximum backoff delay (5 minutes).
const MAX_BACKOFF: Duration = Duration::from_secs(300);

/// Callback type for credential updates.
pub type CredentialCallback = Arc<dyn Fn(OAuthCredentials) + Send + Sync>;

/// Background token refresher that automatically refreshes tokens before expiry.
///
/// The `TokenRefresher` monitors OAuth credentials and refreshes them automatically
/// when they are about to expire. It runs as a background task and supports:
///
/// - Configurable refresh buffer (when to refresh before expiry)
/// - Exponential backoff on failure
/// - Graceful shutdown
/// - Callback for credential updates
///
/// # Example
///
/// ```no_run
/// use patina::auth::refresh::TokenRefresher;
/// use patina::auth::OAuthCredentials;
/// use secrecy::SecretString;
/// use std::sync::Arc;
/// use std::time::Duration;
///
/// #[tokio::main]
/// async fn main() {
///     let creds = OAuthCredentials::new(
///         SecretString::new("access".into()),
///         SecretString::new("refresh".into()),
///         Duration::from_secs(3600),
///     );
///
///     let refresher = TokenRefresher::new(creds)
///         .with_callback(Arc::new(|new_creds| {
///             println!("Got new credentials!");
///         }));
///
///     let handle = refresher.start();
///
///     // ... do work ...
///
///     refresher.stop();
///     handle.await.ok();
/// }
/// ```
pub struct TokenRefresher {
    /// Current credentials being monitored.
    credentials: Arc<Mutex<OAuthCredentials>>,

    /// Time before expiry to trigger refresh.
    refresh_buffer: Duration,

    /// Callback invoked when credentials are refreshed.
    callback: Option<CredentialCallback>,

    /// Signal to stop the background task.
    shutdown: Arc<AtomicBool>,

    /// Current backoff delay (reset on success).
    current_backoff: Arc<Mutex<Duration>>,
}

impl TokenRefresher {
    /// Creates a new token refresher with the given credentials.
    ///
    /// The refresher is created but not started. Call [`start`](Self::start)
    /// to begin the background refresh task.
    #[must_use]
    pub fn new(credentials: OAuthCredentials) -> Self {
        Self {
            credentials: Arc::new(Mutex::new(credentials)),
            refresh_buffer: BACKGROUND_REFRESH_BUFFER,
            callback: None,
            shutdown: Arc::new(AtomicBool::new(false)),
            current_backoff: Arc::new(Mutex::new(INITIAL_BACKOFF)),
        }
    }

    /// Sets the refresh buffer (time before expiry to trigger refresh).
    ///
    /// Default is 5 minutes (300 seconds).
    #[must_use]
    pub fn with_refresh_buffer(mut self, buffer: Duration) -> Self {
        self.refresh_buffer = buffer;
        self
    }

    /// Sets the callback invoked when credentials are refreshed.
    ///
    /// The callback receives the new credentials after a successful refresh.
    #[must_use]
    pub fn with_callback(mut self, callback: CredentialCallback) -> Self {
        self.callback = Some(callback);
        self
    }

    /// Starts the background refresh task.
    ///
    /// Returns a `JoinHandle` that can be awaited for clean shutdown.
    /// Call [`stop`](Self::stop) to signal the task to terminate.
    pub fn start(&self) -> JoinHandle<()> {
        let credentials = Arc::clone(&self.credentials);
        let refresh_buffer = self.refresh_buffer;
        let callback = self.callback.clone();
        let shutdown = Arc::clone(&self.shutdown);
        let current_backoff = Arc::clone(&self.current_backoff);

        tokio::spawn(async move {
            let mut check_interval = interval(Duration::from_secs(10));

            loop {
                check_interval.tick().await;

                // Check for shutdown signal
                if shutdown.load(Ordering::Relaxed) {
                    debug!("TokenRefresher received shutdown signal");
                    break;
                }

                // Check if credentials need refresh
                let needs_refresh = {
                    let creds = credentials.lock().expect("credentials lock poisoned");
                    should_refresh(&creds, refresh_buffer)
                };

                if needs_refresh {
                    debug!("Credentials expiring soon, attempting refresh");

                    // Clone credentials for refresh attempt
                    let creds_snapshot = {
                        let creds = credentials.lock().expect("credentials lock poisoned");
                        creds.clone()
                    };

                    match refresh_token(&creds_snapshot).await {
                        Ok(new_creds) => {
                            info!("Token refresh successful");

                            // Update stored credentials
                            {
                                let mut creds =
                                    credentials.lock().expect("credentials lock poisoned");
                                *creds = new_creds.clone();
                            }

                            // Reset backoff on success
                            {
                                let mut backoff =
                                    current_backoff.lock().expect("backoff lock poisoned");
                                *backoff = INITIAL_BACKOFF;
                            }

                            // Invoke callback
                            if let Some(ref cb) = callback {
                                cb(new_creds);
                            }
                        }
                        Err(e) => {
                            let backoff_duration = {
                                let mut backoff =
                                    current_backoff.lock().expect("backoff lock poisoned");
                                let current = *backoff;
                                // Double backoff, capped at max
                                *backoff = (*backoff * 2).min(MAX_BACKOFF);
                                current
                            };

                            warn!(
                                error = %e,
                                backoff_secs = backoff_duration.as_secs(),
                                "Token refresh failed, will retry"
                            );

                            // Wait for backoff before next attempt
                            sleep(backoff_duration).await;
                        }
                    }
                }
            }

            debug!("TokenRefresher background task terminated");
        })
    }

    /// Signals the background task to stop.
    ///
    /// This method returns immediately. To wait for the task to complete,
    /// await the `JoinHandle` returned by [`start`](Self::start).
    pub fn stop(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }

    /// Returns true if the refresher has been signaled to stop.
    #[must_use]
    pub fn is_stopping(&self) -> bool {
        self.shutdown.load(Ordering::Relaxed)
    }

    /// Returns the current backoff duration.
    ///
    /// Useful for testing exponential backoff behavior.
    #[must_use]
    pub fn current_backoff(&self) -> Duration {
        *self.current_backoff.lock().expect("backoff lock poisoned")
    }

    /// Returns a clone of the current credentials.
    #[must_use]
    pub fn credentials(&self) -> OAuthCredentials {
        self.credentials
            .lock()
            .expect("credentials lock poisoned")
            .clone()
    }

    /// Updates the credentials being monitored.
    ///
    /// Use this to update credentials from an external source.
    pub fn update_credentials(&self, credentials: OAuthCredentials) {
        let mut creds = self.credentials.lock().expect("credentials lock poisoned");
        *creds = credentials;
    }
}

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
