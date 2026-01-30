//! Session persistence for RCT.
//!
//! This module provides session save/load functionality, allowing users to
//! resume conversations across application restarts.
//!
//! # Example
//!
//! ```no_run
//! use rct::session::{Session, SessionManager};
//! use std::path::PathBuf;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let manager = SessionManager::new(PathBuf::from("~/.rct/sessions"));
//!
//! // Create a new session
//! let mut session = Session::new(PathBuf::from("/my/project"));
//! // ... add messages to session ...
//!
//! // Save the session
//! let session_id = manager.save(&session).await?;
//!
//! // Later, resume the session
//! let resumed = manager.load(&session_id).await?;
//! # Ok(())
//! # }
//! ```

use crate::error::{RctError, RctResult};
use crate::types::message::Message;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::time::SystemTime;
use tokio::fs;
use uuid::Uuid;

/// Static key used for session integrity HMAC.
///
/// This provides protection against accidental corruption and casual tampering.
/// For stronger security, this should be derived from a user-configured secret.
const INTEGRITY_KEY: &[u8] = b"rct-session-integrity-v1";

/// Wrapper for session files that includes integrity checksum.
///
/// This struct is used for serialization/deserialization of session files,
/// wrapping the actual session data with a checksum for integrity verification.
#[derive(Debug, Serialize, Deserialize)]
struct SessionFile {
    /// The session data.
    session: Session,
    /// HMAC-SHA256 checksum of the session JSON (hex-encoded).
    checksum: String,
}

impl SessionFile {
    /// Creates a new session file with computed checksum.
    fn new(session: Session) -> Result<Self> {
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
    fn verify(self) -> RctResult<Session> {
        let session_json = serde_json::to_string(&self.session)
            .map_err(|e| RctError::session_integrity(format!("failed to serialize: {}", e)))?;
        let expected_checksum = compute_checksum(&session_json);

        if self.checksum != expected_checksum {
            return Err(RctError::session_integrity("checksum mismatch"));
        }

        Ok(self.session)
    }
}

/// Computes HMAC-SHA256 checksum of the given data.
fn compute_checksum(data: &str) -> String {
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
fn validate_session_id(session_id: &str) -> RctResult<()> {
    if session_id.is_empty() {
        return Err(RctError::session_validation("session ID cannot be empty"));
    }

    // Session IDs must be alphanumeric with hyphens and underscores only
    // This is safe because UUIDs only contain hex digits and hyphens
    let is_valid = session_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');

    if !is_valid {
        return Err(RctError::session_validation(
            "invalid session ID: must contain only alphanumeric characters, hyphens, and underscores",
        ));
    }

    Ok(())
}

/// A conversation session with messages and metadata.
///
/// Sessions store the complete conversation history along with metadata
/// like the working directory and timestamps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Unique session identifier, assigned when saved.
    #[serde(default)]
    id: Option<String>,

    /// Messages in this session.
    messages: Vec<Message>,

    /// Working directory for this session.
    working_dir: PathBuf,

    /// When the session was created.
    created_at: SystemTime,

    /// When the session was last updated.
    updated_at: SystemTime,
}

impl Session {
    /// Creates a new empty session.
    ///
    /// # Arguments
    ///
    /// * `working_dir` - The working directory for this session.
    #[must_use]
    pub fn new(working_dir: PathBuf) -> Self {
        let now = SystemTime::now();
        Self {
            id: None,
            messages: Vec::new(),
            working_dir,
            created_at: now,
            updated_at: now,
        }
    }

    /// Adds a message to the session.
    pub fn add_message(&mut self, message: Message) {
        self.messages.push(message);
        self.updated_at = SystemTime::now();
    }

    /// Returns the messages in this session.
    #[must_use]
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// Returns the working directory for this session.
    #[must_use]
    pub fn working_dir(&self) -> &PathBuf {
        &self.working_dir
    }

    /// Returns when this session was created.
    #[must_use]
    pub fn created_at(&self) -> SystemTime {
        self.created_at
    }

    /// Returns when this session was last updated.
    #[must_use]
    pub fn updated_at(&self) -> SystemTime {
        self.updated_at
    }

    /// Returns the session ID, if assigned.
    #[must_use]
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }
}

/// Metadata about a session without the full message content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetadata {
    /// The session ID.
    pub id: String,

    /// Working directory for this session.
    pub working_dir: PathBuf,

    /// When the session was created.
    pub created_at: SystemTime,

    /// When the session was last updated.
    pub updated_at: SystemTime,

    /// Number of messages in the session.
    pub message_count: usize,
}

/// Manages session persistence.
///
/// The `SessionManager` handles saving and loading sessions to/from disk.
/// Sessions are stored as JSON files in the configured directory.
#[derive(Debug, Clone)]
pub struct SessionManager {
    /// Directory where sessions are stored.
    sessions_dir: PathBuf,
}

impl SessionManager {
    /// Creates a new session manager.
    ///
    /// # Arguments
    ///
    /// * `sessions_dir` - Directory where sessions will be stored.
    #[must_use]
    pub fn new(sessions_dir: PathBuf) -> Self {
        Self { sessions_dir }
    }

    /// Saves a session to disk.
    ///
    /// Returns the session ID (a UUID string).
    ///
    /// The session is saved with an integrity checksum that is verified on load.
    ///
    /// # Errors
    ///
    /// Returns an error if the session cannot be serialized or written to disk.
    pub async fn save(&self, session: &Session) -> Result<String> {
        // Ensure sessions directory exists
        fs::create_dir_all(&self.sessions_dir)
            .await
            .context("Failed to create sessions directory")?;

        // Generate a new ID if not present
        let session_id = session
            .id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        // Create a copy with the ID set
        let mut session_to_save = session.clone();
        session_to_save.id = Some(session_id.clone());
        session_to_save.updated_at = SystemTime::now();

        // Wrap with checksum for integrity
        let session_file = SessionFile::new(session_to_save)?;

        // Serialize and write
        let json =
            serde_json::to_string_pretty(&session_file).context("Failed to serialize session")?;

        let path = self.session_path(&session_id);
        fs::write(&path, json)
            .await
            .context("Failed to write session file")?;

        Ok(session_id)
    }

    /// Loads a session from disk.
    ///
    /// # Arguments
    ///
    /// * `session_id` - The ID of the session to load.
    ///
    /// # Errors
    ///
    /// Returns an error if the session ID is invalid, doesn't exist, cannot be read,
    /// or fails integrity verification.
    pub async fn load(&self, session_id: &str) -> Result<Session> {
        validate_session_id(session_id)?;
        let path = self.session_path(session_id);

        let json = fs::read_to_string(&path)
            .await
            .context("Failed to read session file")?;

        let session_file: SessionFile =
            serde_json::from_str(&json).context("Failed to deserialize session")?;

        // Verify integrity checksum (convert RctError to anyhow::Error)
        Ok(session_file.verify()?)
    }

    /// Updates an existing session.
    ///
    /// # Arguments
    ///
    /// * `session_id` - The ID of the session to update.
    /// * `session` - The updated session data.
    ///
    /// # Errors
    ///
    /// Returns an error if the session ID is invalid or cannot be written.
    pub async fn update(&self, session_id: &str, session: &Session) -> Result<()> {
        validate_session_id(session_id)?;
        // Create a copy with the correct ID
        let mut session_to_save = session.clone();
        session_to_save.id = Some(session_id.to_string());
        session_to_save.updated_at = SystemTime::now();

        // Wrap with checksum for integrity
        let session_file = SessionFile::new(session_to_save)?;

        let json =
            serde_json::to_string_pretty(&session_file).context("Failed to serialize session")?;

        let path = self.session_path(session_id);
        fs::write(&path, json)
            .await
            .context("Failed to write session file")?;

        Ok(())
    }

    /// Deletes a session from disk.
    ///
    /// # Arguments
    ///
    /// * `session_id` - The ID of the session to delete.
    ///
    /// # Errors
    ///
    /// Returns an error if the session ID is invalid or cannot be deleted.
    pub async fn delete(&self, session_id: &str) -> Result<()> {
        validate_session_id(session_id)?;
        let path = self.session_path(session_id);
        fs::remove_file(&path)
            .await
            .context("Failed to delete session file")?;
        Ok(())
    }

    /// Lists all session IDs.
    ///
    /// # Errors
    ///
    /// Returns an error if the sessions directory cannot be read.
    pub async fn list(&self) -> Result<Vec<String>> {
        if !self.sessions_dir.exists() {
            return Ok(Vec::new());
        }

        let mut entries = fs::read_dir(&self.sessions_dir)
            .await
            .context("Failed to read sessions directory")?;

        let mut session_ids = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                if let Some(stem) = path.file_stem() {
                    session_ids.push(stem.to_string_lossy().into_owned());
                }
            }
        }

        Ok(session_ids)
    }

    /// Lists all sessions with their metadata.
    ///
    /// # Errors
    ///
    /// Returns an error if sessions cannot be read.
    pub async fn list_with_metadata(&self) -> Result<Vec<(String, SessionMetadata)>> {
        let session_ids = self.list().await?;
        let mut result = Vec::new();

        for id in session_ids {
            if let Ok(metadata) = self.get_metadata(&id).await {
                result.push((id, metadata));
            }
        }

        Ok(result)
    }

    /// Gets metadata for a specific session without loading full content.
    ///
    /// # Arguments
    ///
    /// * `session_id` - The ID of the session.
    ///
    /// # Errors
    ///
    /// Returns an error if the session ID is invalid or cannot be read.
    pub async fn get_metadata(&self, session_id: &str) -> Result<SessionMetadata> {
        // Note: load() already validates session_id, but we validate here for clarity
        validate_session_id(session_id)?;
        let session = self.load(session_id).await?;

        Ok(SessionMetadata {
            id: session_id.to_string(),
            working_dir: session.working_dir,
            created_at: session.created_at,
            updated_at: session.updated_at,
            message_count: session.messages.len(),
        })
    }

    /// Returns the path to a session file.
    fn session_path(&self, session_id: &str) -> PathBuf {
        self.sessions_dir.join(format!("{}.json", session_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::message::Role;
    use tempfile::TempDir;

    fn test_message(role: Role, content: &str) -> Message {
        Message {
            role,
            content: content.to_string(),
        }
    }

    #[test]
    fn test_session_new() {
        let session = Session::new(PathBuf::from("/test"));
        assert!(session.messages().is_empty());
        assert_eq!(session.working_dir(), &PathBuf::from("/test"));
        assert!(session.id().is_none());
    }

    #[test]
    fn test_session_add_message() {
        let mut session = Session::new(PathBuf::from("/test"));
        session.add_message(test_message(Role::User, "Hello"));
        assert_eq!(session.messages().len(), 1);
        assert_eq!(session.messages()[0].content, "Hello");
    }

    #[tokio::test]
    async fn test_session_manager_save_load() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SessionManager::new(temp_dir.path().to_path_buf());

        let mut session = Session::new(PathBuf::from("/test"));
        session.add_message(test_message(Role::User, "Test message"));

        let id = manager.save(&session).await.unwrap();
        let loaded = manager.load(&id).await.unwrap();

        assert_eq!(loaded.messages().len(), 1);
        assert_eq!(loaded.messages()[0].content, "Test message");
    }

    #[tokio::test]
    async fn test_session_manager_list() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SessionManager::new(temp_dir.path().to_path_buf());

        let session1 = Session::new(PathBuf::from("/test1"));
        let session2 = Session::new(PathBuf::from("/test2"));

        manager.save(&session1).await.unwrap();
        manager.save(&session2).await.unwrap();

        let sessions = manager.list().await.unwrap();
        assert_eq!(sessions.len(), 2);
    }

    #[tokio::test]
    async fn test_session_manager_delete() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SessionManager::new(temp_dir.path().to_path_buf());

        let session = Session::new(PathBuf::from("/test"));
        let id = manager.save(&session).await.unwrap();

        manager.delete(&id).await.unwrap();

        assert!(manager.load(&id).await.is_err());
    }

    #[tokio::test]
    async fn test_session_manager_rejects_path_traversal() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SessionManager::new(temp_dir.path().to_path_buf());

        // Attempt path traversal attacks - all should fail
        let malicious_ids = [
            "../../../etc/passwd",
            "..\\..\\..\\windows\\system32\\config\\sam",
            "valid/../../../escape",
            "/absolute/path",
            "session/with/slashes",
            "session.with.dots.json",
            "session with spaces",
            "",
        ];

        for malicious_id in malicious_ids {
            let result = manager.load(malicious_id).await;
            assert!(
                result.is_err(),
                "Expected error for malicious ID: {:?}",
                malicious_id
            );
        }
    }

    #[tokio::test]
    async fn test_session_manager_accepts_valid_session_ids() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SessionManager::new(temp_dir.path().to_path_buf());

        // These are valid session ID formats (UUID-like)
        let valid_ids = [
            "550e8400-e29b-41d4-a716-446655440000",
            "simple_session_id",
            "session-with-dashes",
            "MixedCase123",
            "a",
            "123",
        ];

        // Create sessions with these IDs and verify they work
        for valid_id in valid_ids {
            let mut session = Session::new(PathBuf::from("/test"));
            session.id = Some(valid_id.to_string());

            // Save should succeed
            let saved_id = manager.save(&session).await.unwrap();
            assert_eq!(saved_id, valid_id);

            // Load should succeed
            let loaded = manager.load(valid_id).await.unwrap();
            assert_eq!(loaded.id(), Some(valid_id));
        }
    }

    #[test]
    fn test_validate_session_id_rejects_empty() {
        assert!(super::validate_session_id("").is_err());
    }

    #[test]
    fn test_validate_session_id_rejects_special_chars() {
        assert!(super::validate_session_id("../parent").is_err());
        assert!(super::validate_session_id("has/slash").is_err());
        assert!(super::validate_session_id("has.dot").is_err());
        assert!(super::validate_session_id("has space").is_err());
        assert!(super::validate_session_id("has:colon").is_err());
    }

    #[test]
    fn test_validate_session_id_accepts_valid() {
        assert!(super::validate_session_id("valid-id").is_ok());
        assert!(super::validate_session_id("valid_id").is_ok());
        assert!(super::validate_session_id("ValidId123").is_ok());
        assert!(super::validate_session_id("550e8400-e29b-41d4-a716-446655440000").is_ok());
    }
}
