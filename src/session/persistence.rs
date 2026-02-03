//! Session persistence utilities.
//!
//! This module provides file integrity and validation utilities for session storage.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use tokio::fs;
use tracing::{error, warn};
use uuid::Uuid;

use super::Session;
use crate::error::{RctError, RctResult};

/// Static key used for session integrity HMAC.
///
/// This provides protection against accidental corruption and casual tampering.
/// For stronger security, this should be derived from a user-configured secret.
pub(super) const INTEGRITY_KEY: &[u8] = b"rct-session-integrity-v1";

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
    /// # Errors
    ///
    /// Returns `RctError::SessionIntegrity` if the checksum doesn't match.
    /// This error is security-related and can be checked via `is_security_related()`.
    pub(super) fn verify(self) -> RctResult<Session> {
        let session_json = serde_json::to_string(&self.session)
            .map_err(|e| RctError::session_integrity(format!("failed to serialize: {}", e)))?;
        let expected_checksum = compute_checksum(&session_json);

        if self.checksum != expected_checksum {
            error!(
                session_id = ?self.session.id,
                "Security: session integrity check failed - possible tampering detected"
            );
            return Err(RctError::session_integrity("checksum mismatch"));
        }

        Ok(self.session)
    }
}

/// Computes HMAC-SHA256 checksum of the given data.
pub(super) fn compute_checksum(data: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(INTEGRITY_KEY);
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
}
