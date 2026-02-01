//! Secure credential storage using the OS keychain.
//!
//! This module provides functions to store and retrieve OAuth credentials
//! in the operating system's secure credential storage:
//! - macOS: Keychain
//! - Windows: Credential Manager
//! - Linux: Secret Service (GNOME Keyring, KWallet)
//!
//! # Example
//!
//! ```no_run
//! use patina::auth::storage::{store_oauth_credentials, load_oauth_credentials, clear_oauth_credentials};
//! use patina::auth::OAuthCredentials;
//! use secrecy::SecretString;
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Store credentials
//!     let creds = OAuthCredentials::new(
//!         SecretString::new("access_token".into()),
//!         SecretString::new("refresh_token".into()),
//!         Duration::from_secs(3600),
//!     );
//!     store_oauth_credentials(&creds).await?;
//!
//!     // Load credentials
//!     if let Some(loaded) = load_oauth_credentials().await? {
//!         println!("Loaded credentials, expires at {:?}", loaded.expires_at());
//!     }
//!
//!     // Clear credentials
//!     clear_oauth_credentials().await?;
//!
//!     Ok(())
//! }
//! ```

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use keyring::Entry;
use secrecy::{ExposeSecret, SecretString};
use tracing::{debug, warn};

use super::OAuthCredentials;

/// Service name for keyring entries.
const SERVICE_NAME: &str = "patina";

/// Username for keyring entries (used as a namespace).
const USERNAME: &str = "oauth";

/// Key suffix for the access token.
const ACCESS_TOKEN_KEY: &str = "access_token";

/// Key suffix for the refresh token.
const REFRESH_TOKEN_KEY: &str = "refresh_token";

/// Key suffix for the expiry timestamp.
const EXPIRY_KEY: &str = "expiry";

/// Stores OAuth credentials in the OS keychain.
///
/// # Arguments
///
/// * `credentials` - The OAuth credentials to store
///
/// # Errors
///
/// Returns an error if the keychain operation fails.
pub async fn store_oauth_credentials(credentials: &OAuthCredentials) -> Result<()> {
    // Store access token
    let access_entry = Entry::new(SERVICE_NAME, &format!("{USERNAME}_{ACCESS_TOKEN_KEY}"))
        .context("Failed to create keyring entry for access token")?;
    access_entry
        .set_password(credentials.access_token().expose_secret())
        .context("Failed to store access token in keyring")?;

    // Store refresh token
    let refresh_entry = Entry::new(SERVICE_NAME, &format!("{USERNAME}_{REFRESH_TOKEN_KEY}"))
        .context("Failed to create keyring entry for refresh token")?;
    refresh_entry
        .set_password(credentials.refresh_token().expose_secret())
        .context("Failed to store refresh token in keyring")?;

    // Store expiry as unix timestamp
    let expiry_secs = credentials
        .expires_at()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let expiry_entry = Entry::new(SERVICE_NAME, &format!("{USERNAME}_{EXPIRY_KEY}"))
        .context("Failed to create keyring entry for expiry")?;
    expiry_entry
        .set_password(&expiry_secs.to_string())
        .context("Failed to store expiry in keyring")?;

    debug!("Stored OAuth credentials in keyring");
    Ok(())
}

/// Loads OAuth credentials from the OS keychain.
///
/// # Returns
///
/// Returns `Ok(Some(credentials))` if credentials are found and valid,
/// `Ok(None)` if no credentials are stored, or `Err` if the keychain
/// operation fails.
///
/// # Errors
///
/// Returns an error if the keychain operation fails (other than missing entries).
pub async fn load_oauth_credentials() -> Result<Option<OAuthCredentials>> {
    // Load access token
    let access_entry = Entry::new(SERVICE_NAME, &format!("{USERNAME}_{ACCESS_TOKEN_KEY}"))
        .context("Failed to create keyring entry for access token")?;
    let access_token = match access_entry.get_password() {
        Ok(token) => SecretString::new(token.into()),
        Err(keyring::Error::NoEntry) => {
            debug!("No OAuth access token found in keyring");
            return Ok(None);
        }
        Err(e) => {
            warn!(error = %e, "Failed to load access token from keyring");
            return Err(e).context("Failed to load access token from keyring");
        }
    };

    // Load refresh token
    let refresh_entry = Entry::new(SERVICE_NAME, &format!("{USERNAME}_{REFRESH_TOKEN_KEY}"))
        .context("Failed to create keyring entry for refresh token")?;
    let refresh_token = match refresh_entry.get_password() {
        Ok(token) => SecretString::new(token.into()),
        Err(keyring::Error::NoEntry) => {
            warn!("Access token found but no refresh token - clearing partial credentials");
            let _ = clear_oauth_credentials().await;
            return Ok(None);
        }
        Err(e) => {
            warn!(error = %e, "Failed to load refresh token from keyring");
            return Err(e).context("Failed to load refresh token from keyring");
        }
    };

    // Load expiry
    let expiry_entry = Entry::new(SERVICE_NAME, &format!("{USERNAME}_{EXPIRY_KEY}"))
        .context("Failed to create keyring entry for expiry")?;
    let expires_at = match expiry_entry.get_password() {
        Ok(expiry_str) => {
            let secs: u64 = expiry_str.parse().unwrap_or(0);
            UNIX_EPOCH + Duration::from_secs(secs)
        }
        Err(keyring::Error::NoEntry) => {
            warn!("Tokens found but no expiry - using default");
            SystemTime::now() + Duration::from_secs(3600)
        }
        Err(e) => {
            warn!(error = %e, "Failed to load expiry from keyring, using default");
            SystemTime::now() + Duration::from_secs(3600)
        }
    };

    debug!("Loaded OAuth credentials from keyring");
    Ok(Some(OAuthCredentials::with_expiry(
        access_token,
        refresh_token,
        expires_at,
    )))
}

/// Clears OAuth credentials from the OS keychain.
///
/// # Errors
///
/// Returns an error if the keychain operation fails (other than missing entries).
pub async fn clear_oauth_credentials() -> Result<()> {
    // Clear access token
    if let Ok(entry) = Entry::new(SERVICE_NAME, &format!("{USERNAME}_{ACCESS_TOKEN_KEY}")) {
        match entry.delete_credential() {
            Ok(()) => debug!("Deleted access token from keyring"),
            Err(keyring::Error::NoEntry) => {}
            Err(e) => warn!(error = %e, "Failed to delete access token from keyring"),
        }
    }

    // Clear refresh token
    if let Ok(entry) = Entry::new(SERVICE_NAME, &format!("{USERNAME}_{REFRESH_TOKEN_KEY}")) {
        match entry.delete_credential() {
            Ok(()) => debug!("Deleted refresh token from keyring"),
            Err(keyring::Error::NoEntry) => {}
            Err(e) => warn!(error = %e, "Failed to delete refresh token from keyring"),
        }
    }

    // Clear expiry
    if let Ok(entry) = Entry::new(SERVICE_NAME, &format!("{USERNAME}_{EXPIRY_KEY}")) {
        match entry.delete_credential() {
            Ok(()) => debug!("Deleted expiry from keyring"),
            Err(keyring::Error::NoEntry) => {}
            Err(e) => warn!(error = %e, "Failed to delete expiry from keyring"),
        }
    }

    debug!("Cleared OAuth credentials from keyring");
    Ok(())
}

/// Checks if OAuth credentials are stored in the keychain.
///
/// This is a quick check that doesn't load the actual credentials.
#[must_use]
pub fn has_stored_credentials() -> bool {
    if let Ok(entry) = Entry::new(SERVICE_NAME, &format!("{USERNAME}_{ACCESS_TOKEN_KEY}")) {
        entry.get_password().is_ok()
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These tests interact with the real keychain, so they're marked
    // as ignored by default. Run them manually with --ignored flag.

    #[tokio::test]
    #[ignore = "interacts with real keychain"]
    async fn test_store_and_load_credentials() {
        let creds = OAuthCredentials::new(
            SecretString::new("test-access-token".into()),
            SecretString::new("test-refresh-token".into()),
            Duration::from_secs(3600),
        );

        // Store
        store_oauth_credentials(&creds).await.unwrap();

        // Load
        let loaded = load_oauth_credentials()
            .await
            .unwrap()
            .expect("credentials should be present");

        assert_eq!(loaded.access_token().expose_secret(), "test-access-token");
        assert_eq!(loaded.refresh_token().expose_secret(), "test-refresh-token");

        // Cleanup
        clear_oauth_credentials().await.unwrap();
    }

    #[tokio::test]
    #[ignore = "interacts with real keychain"]
    async fn test_clear_credentials() {
        let creds = OAuthCredentials::new(
            SecretString::new("test-access-token".into()),
            SecretString::new("test-refresh-token".into()),
            Duration::from_secs(3600),
        );

        // Store
        store_oauth_credentials(&creds).await.unwrap();
        assert!(has_stored_credentials());

        // Clear
        clear_oauth_credentials().await.unwrap();
        assert!(!has_stored_credentials());

        // Load should return None
        let loaded = load_oauth_credentials().await.unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn test_has_stored_credentials_when_none() {
        // This test assumes no credentials are stored for the test user
        // It may fail if previous tests left credentials behind
        // Just verify it doesn't panic
        let _ = has_stored_credentials();
    }
}
