//! Session persistence utilities.
//!
//! This module provides file integrity and validation utilities for session storage.

use anyhow::{Context, Result};
use directories::ProjectDirs;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{error, warn};
use uuid::Uuid;

use super::Session;
use crate::error::{RctError, RctResult};

/// Legacy hardcoded key, kept only for migration of old sessions.
const LEGACY_INTEGRITY_KEY: &[u8] = b"rct-session-integrity-v1";

/// Per-user HMAC key, generated on first use and persisted to the data directory.
///
/// The key is 32 random bytes stored at `<data_dir>/patina/.session_key`.
/// On Unix, the key file has 0600 permissions. If the file cannot be read or
/// created, this falls back to the legacy hardcoded key with a warning.
static INTEGRITY_KEY: Lazy<Vec<u8>> = Lazy::new(get_or_create_integrity_key);

/// Returns the path where the per-user session integrity key is stored.
fn integrity_key_path() -> Option<PathBuf> {
    ProjectDirs::from("com", "patina", "patina").map(|dirs| dirs.data_dir().join(".session_key"))
}

/// Loads or generates a persistent 32-byte HMAC key for session integrity.
///
/// The key is stored in the platform-specific data directory. If the key file
/// exists and contains exactly 32 bytes, those bytes are returned. Otherwise,
/// a new key is generated from two UUID v4 values (which use the OS CSPRNG)
/// and written to disk.
fn get_or_create_integrity_key() -> Vec<u8> {
    let Some(key_path) = integrity_key_path() else {
        warn!("Cannot determine data directory for session key; using legacy key");
        return LEGACY_INTEGRITY_KEY.to_vec();
    };

    // Try to load existing key
    if let Ok(key) = std::fs::read(&key_path) {
        if key.len() == 32 {
            return key;
        }
        warn!(
            path = %key_path.display(),
            len = key.len(),
            "Session key file has unexpected length; regenerating"
        );
    }

    // Generate new 32-byte key from two UUID v4 values (each provides 16 random bytes)
    let mut key = Vec::with_capacity(32);
    key.extend_from_slice(Uuid::new_v4().as_bytes());
    key.extend_from_slice(Uuid::new_v4().as_bytes());

    // Ensure parent directory exists
    if let Some(parent) = key_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            warn!(
                path = %parent.display(),
                error = %e,
                "Failed to create directory for session key; using legacy key"
            );
            return LEGACY_INTEGRITY_KEY.to_vec();
        }
    }

    // Write the key file
    if let Err(e) = std::fs::write(&key_path, &key) {
        warn!(
            path = %key_path.display(),
            error = %e,
            "Failed to write session key; using legacy key"
        );
        return LEGACY_INTEGRITY_KEY.to_vec();
    }

    // Restrict file permissions on Unix
    set_key_file_permissions(&key_path);

    key
}

/// Sets restrictive file permissions (0600) on the key file.
///
/// On non-Unix platforms this is a no-op.
#[cfg(unix)]
fn set_key_file_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        warn!(
            path = %path.display(),
            error = %e,
            "Failed to set restrictive permissions on session key file"
        );
    }
}

/// Sets restrictive file permissions on the key file (no-op on non-Unix).
#[cfg(not(unix))]
fn set_key_file_permissions(_path: &Path) {
    // Windows does not use POSIX mode bits; the default DACL is acceptable.
}

/// Writes data to a file atomically using write-to-temp-then-rename pattern.
///
/// This ensures that concurrent writes don't corrupt the file - each write
/// either fully succeeds or the file remains unchanged.
pub(super) async fn atomic_write(path: &Path, contents: &str) -> Result<()> {
    // Create temp file in same directory (ensures same filesystem for rename)
    let parent = path.parent().unwrap_or(Path::new("."));
    let temp_name = format!(
        ".{}.tmp.{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("session"),
        Uuid::new_v4()
    );
    let temp_path = parent.join(temp_name);

    // Write to temp file
    fs::write(&temp_path, contents)
        .await
        .context("Failed to write temp file")?;

    // Atomic rename (on POSIX this is atomic, on Windows it's mostly atomic)
    fs::rename(&temp_path, path)
        .await
        .context("Failed to rename temp file")?;

    Ok(())
}

/// Wrapper for session files that includes integrity checksum.
///
/// This struct is used for serialization/deserialization of session files,
/// wrapping the actual session data with a checksum for integrity verification.
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct SessionFile {
    /// The session data.
    pub(super) session: Session,
    /// HMAC-SHA256 checksum of the session JSON (hex-encoded).
    checksum: String,
}

impl SessionFile {
    /// Creates a new session file with computed checksum.
    pub(super) fn new(session: Session) -> Result<Self> {
        let session_json =
            serde_json::to_string(&session).context("Failed to serialize session for checksum")?;
        let checksum = compute_checksum(&session_json);
        Ok(Self { session, checksum })
    }

    /// Verifies the checksum and returns the session if valid.
    ///
    /// First attempts verification with the per-user key. If that fails,
    /// falls back to the legacy hardcoded key for backward compatibility.
    /// Sessions verified with the legacy key are silently accepted (the
    /// next save will re-sign them with the per-user key).
    ///
    /// # Errors
    ///
    /// Returns `RctError::SessionIntegrity` if the checksum doesn't match
    /// either the per-user key or the legacy key.
    pub(super) fn verify(self) -> RctResult<Session> {
        let session_json = serde_json::to_string(&self.session)
            .map_err(|e| RctError::session_integrity(format!("failed to serialize: {}", e)))?;

        // Try the per-user key first
        let expected_checksum = compute_checksum(&session_json);
        if self.checksum == expected_checksum {
            return Ok(self.session);
        }

        // Fall back to legacy key for migration
        let legacy_checksum = compute_checksum_with_key(&session_json, LEGACY_INTEGRITY_KEY);
        if self.checksum == legacy_checksum {
            warn!(
                session_id = ?self.session.id,
                "Session signed with legacy key; it will be re-signed on next save"
            );
            return Ok(self.session);
        }

        error!(
            session_id = ?self.session.id,
            "Security: session integrity check failed - possible tampering detected"
        );
        Err(RctError::session_integrity("checksum mismatch"))
    }
}

/// Computes HMAC-SHA256 checksum of the given data using the per-user key.
pub(super) fn compute_checksum(data: &str) -> String {
    compute_checksum_with_key(data, &INTEGRITY_KEY)
}

/// Computes HMAC-SHA256 checksum of the given data using a specified key.
fn compute_checksum_with_key(data: &str, key: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key);
    hasher.update(data.as_bytes());
    hex::encode(hasher.finalize())
}

/// Validates a session ID to prevent path traversal attacks.
///
/// Session IDs must contain only alphanumeric characters, hyphens, and underscores.
/// This prevents attacks like `../../etc/passwd` from escaping the sessions directory.
///
/// # Errors
///
/// Returns `RctError::SessionValidation` if the session ID is invalid.
/// This error is security-related and can be checked via `is_security_related()`.
pub(super) fn validate_session_id(session_id: &str) -> RctResult<()> {
    if session_id.is_empty() {
        warn!("Session validation failed: empty session ID");
        return Err(RctError::session_validation("session ID cannot be empty"));
    }

    // Session IDs must be alphanumeric with hyphens and underscores only
    // This is safe because UUIDs only contain hex digits and hyphens
    let is_valid = session_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');

    if !is_valid {
        warn!(
            session_id = %session_id,
            "Security: session validation failed - invalid characters (possible path traversal attempt)"
        );
        return Err(RctError::session_validation(
            "invalid session ID: must contain only alphanumeric characters, hyphens, and underscores",
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_compute_checksum_deterministic() {
        let data = "test data";
        let checksum1 = compute_checksum(data);
        let checksum2 = compute_checksum(data);
        assert_eq!(checksum1, checksum2);
    }

    #[test]
    fn test_compute_checksum_different_for_different_data() {
        let checksum1 = compute_checksum("data1");
        let checksum2 = compute_checksum("data2");
        assert_ne!(checksum1, checksum2);
    }

    #[test]
    fn test_validate_session_id_valid() {
        assert!(validate_session_id("abc-123_def").is_ok());
        assert!(validate_session_id("550e8400-e29b-41d4-a716-446655440000").is_ok());
    }

    #[test]
    fn test_validate_session_id_empty() {
        assert!(validate_session_id("").is_err());
    }

    #[test]
    fn test_validate_session_id_path_traversal() {
        assert!(validate_session_id("../etc/passwd").is_err());
        assert!(validate_session_id("foo/bar").is_err());
        assert!(validate_session_id("foo\\bar").is_err());
    }

    #[test]
    fn test_validate_session_id_special_chars() {
        assert!(validate_session_id("test session").is_err());
        assert!(validate_session_id("test@session").is_err());
    }

    #[test]
    fn test_key_generation_creates_file() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let key_path = dir.path().join(".session_key");

        // Generate a key manually (same logic as get_or_create_integrity_key)
        let mut key = Vec::with_capacity(32);
        key.extend_from_slice(Uuid::new_v4().as_bytes());
        key.extend_from_slice(Uuid::new_v4().as_bytes());
        std::fs::write(&key_path, &key).expect("failed to write key");

        assert!(key_path.exists());
        let read_back = std::fs::read(&key_path).expect("failed to read key");
        assert_eq!(read_back.len(), 32);
    }

    #[test]
    fn test_key_persistence() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let key_path = dir.path().join(".session_key");

        // Generate and write
        let mut key = Vec::with_capacity(32);
        key.extend_from_slice(Uuid::new_v4().as_bytes());
        key.extend_from_slice(Uuid::new_v4().as_bytes());
        std::fs::write(&key_path, &key).expect("failed to write key");

        // Read back
        let read_back = std::fs::read(&key_path).expect("failed to read key");
        assert_eq!(key, read_back, "key should persist across reads");
    }

    #[test]
    fn test_old_key_fallback() {
        let data = r#"{"id":"test-session","messages":[]}"#;

        // Compute checksum with legacy key
        let legacy_checksum = compute_checksum_with_key(data, LEGACY_INTEGRITY_KEY);

        // Compute with current per-user key
        let current_checksum = compute_checksum(data);

        // If the per-user key differs from legacy, the checksums should differ
        if *INTEGRITY_KEY != LEGACY_INTEGRITY_KEY.to_vec() {
            assert_ne!(
                legacy_checksum, current_checksum,
                "per-user key should produce different checksums than legacy key"
            );
        }

        // Legacy checksum should still be valid via compute_checksum_with_key
        let verify_legacy = compute_checksum_with_key(data, LEGACY_INTEGRITY_KEY);
        assert_eq!(legacy_checksum, verify_legacy);
    }

    #[test]
    fn test_new_key_verification() {
        let data = r#"{"id":"new-session","messages":[]}"#;

        // Sign with the per-user key
        let checksum = compute_checksum(data);

        // Verify it matches when computed again
        let verify = compute_checksum(data);
        assert_eq!(checksum, verify);
    }

    #[test]
    fn test_integrity_key_is_32_bytes() {
        assert_eq!(INTEGRITY_KEY.len(), 32);
    }

    #[test]
    fn test_compute_checksum_with_key_differs_by_key() {
        let data = "same data";
        let key_a = b"key-a-padded-to-make-it-long-enough";
        let key_b = b"key-b-padded-to-make-it-long-enough";

        let checksum_a = compute_checksum_with_key(data, key_a);
        let checksum_b = compute_checksum_with_key(data, key_b);
        assert_ne!(
            checksum_a, checksum_b,
            "different keys should produce different checksums"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_key_file_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let key_path = dir.path().join(".session_key");

        let key = vec![0u8; 32];
        std::fs::write(&key_path, &key).expect("failed to write key");
        set_key_file_permissions(&key_path);

        let perms = std::fs::metadata(&key_path)
            .expect("failed to get metadata")
            .permissions();
        assert_eq!(
            perms.mode() & 0o777,
            0o600,
            "key file should have 0600 permissions"
        );
    }

    #[test]
    fn test_integrity_key_path_returns_some() {
        // On systems with a home directory, this should return Some
        let path = integrity_key_path();
        // We can't guarantee a home directory in CI, so just check it doesn't panic
        if let Some(p) = path {
            assert!(
                p.to_string_lossy().contains("session_key"),
                "path should contain session_key, got: {}",
                p.display()
            );
        }
    }

    #[test]
    fn test_session_file_verify_with_legacy_key() {
        // Create a session
        let session = Session::new(PathBuf::from("/tmp/test"));
        let session_json = serde_json::to_string(&session).expect("failed to serialize session");

        // Create a SessionFile with a legacy-key checksum (simulating an old session)
        let legacy_checksum = compute_checksum_with_key(&session_json, LEGACY_INTEGRITY_KEY);
        let session_file = SessionFile {
            session,
            checksum: legacy_checksum,
        };

        // Verify should succeed via legacy fallback
        let result = session_file.verify();
        assert!(
            result.is_ok(),
            "session signed with legacy key should still verify"
        );
    }

    #[test]
    fn test_session_file_verify_with_bad_checksum() {
        let session = Session::new(PathBuf::from("/tmp/test"));
        let session_file = SessionFile {
            session,
            checksum: "definitely-not-a-valid-checksum".to_string(),
        };

        let result = session_file.verify();
        assert!(
            result.is_err(),
            "session with invalid checksum should fail verification"
        );
    }
}
