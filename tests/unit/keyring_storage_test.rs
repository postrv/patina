//! Unit tests for keyring credential storage.
//!
//! These tests verify:
//! - Store and load credentials roundtrip
//! - Clear credentials removes all stored data
//! - Load nonexistent returns None
//! - Store overwrites existing credentials
//! - Credentials expiry is preserved through storage
//!
//! Note: These tests use a mock storage backend to avoid
//! interacting with the real OS keychain during testing.

use patina::auth::storage::{
    clear_oauth_credentials, has_stored_credentials, load_oauth_credentials,
    store_oauth_credentials,
};
use patina::auth::OAuthCredentials;
use secrecy::{ExposeSecret, SecretString};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// ============================================================================
// Mock Storage Tests - These run without real keychain access
// ============================================================================

/// Tests that credentials can be stored and loaded back (roundtrip).
///
/// This test verifies that:
/// - Access token is preserved
/// - Refresh token is preserved
/// - Token can be loaded after storing
#[tokio::test]
#[ignore = "requires real keychain - run with --ignored"]
async fn test_store_and_load_credentials_roundtrip() {
    // Use unique test identifiers to avoid conflicts
    let access_token = format!("test_access_{}", std::process::id());
    let refresh_token = format!("test_refresh_{}", std::process::id());

    let creds = OAuthCredentials::new(
        SecretString::new(access_token.clone().into()),
        SecretString::new(refresh_token.clone().into()),
        Duration::from_secs(3600),
    );

    // Store credentials
    store_oauth_credentials(&creds)
        .await
        .expect("Failed to store credentials");

    // Load credentials
    let loaded = load_oauth_credentials()
        .await
        .expect("Failed to load credentials")
        .expect("Credentials should be present");

    // Verify tokens match
    assert_eq!(
        loaded.access_token().expose_secret(),
        &access_token,
        "Access token should be preserved"
    );
    assert_eq!(
        loaded.refresh_token().expose_secret(),
        &refresh_token,
        "Refresh token should be preserved"
    );

    // Cleanup
    clear_oauth_credentials()
        .await
        .expect("Failed to clear credentials");
}

/// Tests that clearing credentials removes all stored data.
///
/// After clearing:
/// - has_stored_credentials() returns false
/// - load_oauth_credentials() returns None
#[tokio::test]
#[ignore = "requires real keychain - run with --ignored"]
async fn test_clear_credentials_removes_all() {
    // Store some credentials first
    let creds = OAuthCredentials::new(
        SecretString::new("clear_test_access".into()),
        SecretString::new("clear_test_refresh".into()),
        Duration::from_secs(3600),
    );

    store_oauth_credentials(&creds)
        .await
        .expect("Failed to store credentials");

    // Verify they exist
    assert!(
        has_stored_credentials(),
        "Credentials should exist before clearing"
    );

    // Clear credentials
    clear_oauth_credentials()
        .await
        .expect("Failed to clear credentials");

    // Verify they're gone
    assert!(
        !has_stored_credentials(),
        "has_stored_credentials should return false after clearing"
    );

    let loaded = load_oauth_credentials()
        .await
        .expect("Load should not error");
    assert!(
        loaded.is_none(),
        "load_oauth_credentials should return None after clearing"
    );
}

/// Tests that loading nonexistent credentials returns None.
///
/// This ensures clean behavior when no credentials are stored.
#[tokio::test]
#[ignore = "requires real keychain - run with --ignored"]
async fn test_load_nonexistent_returns_none() {
    // First ensure no credentials exist
    let _ = clear_oauth_credentials().await;

    // Load should return None, not an error
    let loaded = load_oauth_credentials()
        .await
        .expect("Load should not error for nonexistent");

    assert!(
        loaded.is_none(),
        "Loading nonexistent credentials should return None"
    );
}

/// Tests that storing credentials overwrites existing ones.
///
/// When storing new credentials, the old ones should be replaced.
#[tokio::test]
#[ignore = "requires real keychain - run with --ignored"]
async fn test_store_overwrites_existing() {
    // Store initial credentials
    let initial_creds = OAuthCredentials::new(
        SecretString::new("initial_access".into()),
        SecretString::new("initial_refresh".into()),
        Duration::from_secs(3600),
    );

    store_oauth_credentials(&initial_creds)
        .await
        .expect("Failed to store initial credentials");

    // Store new credentials (should overwrite)
    let new_creds = OAuthCredentials::new(
        SecretString::new("new_access".into()),
        SecretString::new("new_refresh".into()),
        Duration::from_secs(7200),
    );

    store_oauth_credentials(&new_creds)
        .await
        .expect("Failed to store new credentials");

    // Load and verify new credentials are returned
    let loaded = load_oauth_credentials()
        .await
        .expect("Failed to load credentials")
        .expect("Credentials should be present");

    assert_eq!(
        loaded.access_token().expose_secret(),
        "new_access",
        "Access token should be from new credentials"
    );
    assert_eq!(
        loaded.refresh_token().expose_secret(),
        "new_refresh",
        "Refresh token should be from new credentials"
    );

    // Cleanup
    clear_oauth_credentials()
        .await
        .expect("Failed to clear credentials");
}

/// Tests that credentials expiry is preserved through storage.
///
/// The expiry timestamp should survive the store/load cycle.
#[tokio::test]
#[ignore = "requires real keychain - run with --ignored"]
async fn test_credentials_expiry_preserved() {
    // Create credentials with a specific expiry time
    let expiry_secs: u64 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 7200; // 2 hours from now

    let expected_expiry = UNIX_EPOCH + Duration::from_secs(expiry_secs);

    let creds = OAuthCredentials::with_expiry(
        SecretString::new("expiry_test_access".into()),
        SecretString::new("expiry_test_refresh".into()),
        expected_expiry,
    );

    // Store credentials
    store_oauth_credentials(&creds)
        .await
        .expect("Failed to store credentials");

    // Load credentials
    let loaded = load_oauth_credentials()
        .await
        .expect("Failed to load credentials")
        .expect("Credentials should be present");

    // Verify expiry is preserved (within 1 second tolerance due to storage format)
    let loaded_expiry = loaded.expires_at();
    let expected_duration = expected_expiry
        .duration_since(UNIX_EPOCH)
        .expect("Expected expiry should be after UNIX_EPOCH");
    let loaded_duration = loaded_expiry
        .duration_since(UNIX_EPOCH)
        .expect("Loaded expiry should be after UNIX_EPOCH");

    // Allow 1 second tolerance for rounding
    let diff = if loaded_duration > expected_duration {
        loaded_duration - expected_duration
    } else {
        expected_duration - loaded_duration
    };

    assert!(
        diff <= Duration::from_secs(1),
        "Expiry should be preserved within 1 second tolerance. Expected: {:?}, Loaded: {:?}",
        expected_duration,
        loaded_duration
    );

    // Cleanup
    clear_oauth_credentials()
        .await
        .expect("Failed to clear credentials");
}

// ============================================================================
// CredentialStorage Trait Tests - Test the trait abstraction
// ============================================================================

/// Tests that the CredentialStorage trait is properly implemented.
///
/// This test verifies the trait exists and has the expected methods.
#[test]
fn test_credential_storage_trait_exists() {
    use patina::auth::storage::CredentialStorage;

    // The trait should be importable - this is a compile-time check
    fn _assert_trait_bounds<T: CredentialStorage>() {}

    // Verify the trait is object-safe by attempting to create a trait object type
    fn _assert_object_safe(_: &dyn CredentialStorage) {}
}

/// Tests that MockCredentialStorage can be used for testing.
#[tokio::test]
async fn test_mock_storage_store_and_load() {
    use patina::auth::storage::{CredentialStorage, MockCredentialStorage};

    let mut storage = MockCredentialStorage::new();

    let creds = OAuthCredentials::new(
        SecretString::new("mock_access".into()),
        SecretString::new("mock_refresh".into()),
        Duration::from_secs(3600),
    );

    // Store credentials
    storage
        .store(&creds)
        .await
        .expect("Mock store should succeed");

    // Load credentials
    let loaded = storage
        .load()
        .await
        .expect("Mock load should succeed")
        .expect("Credentials should be present in mock");

    assert_eq!(
        loaded.access_token().expose_secret(),
        "mock_access",
        "Mock should preserve access token"
    );
}

/// Tests that MockCredentialStorage properly handles clear.
#[tokio::test]
async fn test_mock_storage_clear() {
    use patina::auth::storage::{CredentialStorage, MockCredentialStorage};

    let mut storage = MockCredentialStorage::new();

    // Store credentials
    let creds = OAuthCredentials::new(
        SecretString::new("access".into()),
        SecretString::new("refresh".into()),
        Duration::from_secs(3600),
    );
    storage.store(&creds).await.unwrap();

    // Clear
    storage.clear().await.expect("Mock clear should succeed");

    // Load should return None
    let loaded = storage.load().await.expect("Load should not error");
    assert!(loaded.is_none(), "Mock should be empty after clear");

    // has_stored should return false
    assert!(
        !storage.has_stored(),
        "has_stored should return false after clear"
    );
}

/// Tests that MockCredentialStorage preserves expiry.
#[tokio::test]
async fn test_mock_storage_preserves_expiry() {
    use patina::auth::storage::{CredentialStorage, MockCredentialStorage};

    let mut storage = MockCredentialStorage::new();

    let expiry = SystemTime::now() + Duration::from_secs(7200);
    let creds = OAuthCredentials::with_expiry(
        SecretString::new("access".into()),
        SecretString::new("refresh".into()),
        expiry,
    );

    storage.store(&creds).await.unwrap();
    let loaded = storage.load().await.unwrap().unwrap();

    // Verify expiry is exact (no serialization)
    assert_eq!(
        loaded.expires_at(),
        expiry,
        "Mock storage should preserve exact expiry"
    );
}

// ============================================================================
// StorageError Tests - Error handling for keyring operations (0.10.2)
// ============================================================================

/// Tests that StorageError enum has the expected variants.
#[test]
fn test_storage_error_variants() {
    use patina::auth::storage::StorageError;

    // Test each variant can be created
    let unavailable = StorageError::KeyringUnavailable("test platform".to_string());
    let not_found = StorageError::EntryNotFound("access_token".to_string());
    let access_denied = StorageError::AccessDenied("permission denied".to_string());
    let platform = StorageError::Platform("specific error".to_string());

    // Verify Debug is implemented
    assert!(format!("{:?}", unavailable).contains("KeyringUnavailable"));
    assert!(format!("{:?}", not_found).contains("EntryNotFound"));
    assert!(format!("{:?}", access_denied).contains("AccessDenied"));
    assert!(format!("{:?}", platform).contains("Platform"));
}

/// Tests that StorageError implements std::error::Error.
#[test]
fn test_storage_error_is_std_error() {
    use patina::auth::storage::StorageError;
    use std::error::Error;

    let err = StorageError::KeyringUnavailable("test".to_string());

    // Verify it implements Error trait
    fn assert_error<T: Error>(_: &T) {}
    assert_error(&err);

    // Verify Display is implemented with helpful messages
    let display = err.to_string();
    assert!(
        display.contains("keyring") || display.contains("Keyring"),
        "Error message should mention keyring"
    );
}

/// Tests that StorageError provides platform-specific messages.
#[test]
fn test_storage_error_platform_messages() {
    use patina::auth::storage::StorageError;

    // Platform error should contain helpful context
    let platform_err = StorageError::Platform("D-Bus connection failed".to_string());
    let display = platform_err.to_string();
    assert!(
        display.contains("D-Bus") || display.contains("platform"),
        "Platform error should include specific details"
    );

    // Access denied should suggest solutions
    let access_err = StorageError::AccessDenied("keychain locked".to_string());
    let display = access_err.to_string();
    assert!(
        display.contains("locked") || display.contains("denied"),
        "Access error should include reason"
    );
}

/// Tests that StorageError can be converted from keyring::Error.
#[test]
fn test_storage_error_from_keyring_error() {
    use patina::auth::storage::StorageError;

    // NoEntry should map to EntryNotFound
    let keyring_err = keyring::Error::NoEntry;
    let storage_err: StorageError = keyring_err.into();
    assert!(
        matches!(storage_err, StorageError::EntryNotFound(_)),
        "NoEntry should map to EntryNotFound"
    );

    // NoStorageAccess with "permission denied" should map to AccessDenied
    let keyring_err = keyring::Error::NoStorageAccess(Box::new(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "permission denied",
    )));
    let storage_err: StorageError = keyring_err.into();
    assert!(
        matches!(storage_err, StorageError::AccessDenied(_)),
        "NoStorageAccess with permission denied should map to AccessDenied"
    );

    // NoStorageAccess without permission keywords should map to KeyringUnavailable
    let keyring_err = keyring::Error::NoStorageAccess(Box::new(std::io::Error::new(
        std::io::ErrorKind::NotConnected,
        "service not available",
    )));
    let storage_err: StorageError = keyring_err.into();
    assert!(
        matches!(storage_err, StorageError::KeyringUnavailable(_)),
        "NoStorageAccess without permission should map to KeyringUnavailable"
    );
}

/// Tests that MockCredentialStorage can simulate errors for testing.
#[tokio::test]
async fn test_mock_storage_error_simulation() {
    use patina::auth::storage::{CredentialStorage, MockCredentialStorage, StorageError};

    let mut storage = MockCredentialStorage::new();

    // Configure mock to return an error on next store
    storage.set_error_on_store(Some(StorageError::AccessDenied("simulated".to_string())));

    let creds = OAuthCredentials::new(
        SecretString::new("access".into()),
        SecretString::new("refresh".into()),
        Duration::from_secs(3600),
    );

    // Store should fail
    let result = storage.store(&creds).await;
    assert!(result.is_err(), "Store should fail when error is set");

    // Error should be cleared after use (one-shot)
    let result = storage.store(&creds).await;
    assert!(
        result.is_ok(),
        "Store should succeed after error is consumed"
    );
}
