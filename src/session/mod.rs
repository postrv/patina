//! Session persistence for Patina.
//!
//! This module provides session save/load functionality, allowing users to
//! resume conversations across application restarts.
//!
//! # Example
//!
//! ```no_run
//! use patina::session::{Session, SessionManager};
//! use std::path::PathBuf;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let manager = SessionManager::new(PathBuf::from("~/.patina/sessions"));
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

mod ui_state;
mod worktree;

// Re-export types
pub use ui_state::UiState;
pub use worktree::{WorktreeCommit, WorktreeSession};

use crate::error::{RctError, RctResult};
use crate::types::message::Message;
use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tokio::fs;
use tracing::{error, warn};
use uuid::Uuid;

/// Returns the default sessions directory for Patina.
///
/// This uses the platform-specific data directory:
/// - Linux: `~/.local/share/patina/sessions`
/// - macOS: `~/Library/Application Support/patina/sessions`
/// - Windows: `C:\Users\<User>\AppData\Roaming\patina\sessions`
///
/// # Errors
///
/// Returns an error if the project directories cannot be determined.
pub fn default_sessions_dir() -> Result<PathBuf> {
    let project_dirs = ProjectDirs::from("com", "patina", "patina")
        .context("Failed to determine application data directory")?;
    Ok(project_dirs.data_dir().join("sessions"))
}

/// Writes data to a file atomically using write-to-temp-then-rename pattern.
///
/// This ensures that concurrent writes don't corrupt the file - each write
/// either fully succeeds or the file remains unchanged.
async fn atomic_write(path: &Path, contents: &str) -> Result<()> {
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

/// Static key used for session integrity HMAC.
///
/// This provides protection against accidental corruption and casual tampering.
/// For stronger security, this should be derived from a user-configured secret.
const INTEGRITY_KEY: &[u8] = b"rct-session-integrity-v1";

/// A file that was read during the session and may be needed for context restoration.
///
/// When resuming a session, context files can be re-read to restore the conversation
/// context. The optional content hash allows detecting if the file has changed since
/// the session was saved.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextFile {
    /// Path to the file (absolute or relative to working directory).
    path: PathBuf,

    /// Optional SHA-256 hash of the file content at the time it was read.
    /// Used to detect if the file has changed since the session was saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content_hash: Option<String>,
}

impl ContextFile {
    /// Creates a new context file entry without a content hash.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the file.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            content_hash: None,
        }
    }

    /// Creates a new context file entry with a content hash.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the file.
    /// * `content_hash` - SHA-256 hash of the file content.
    #[must_use]
    pub fn with_hash(path: impl Into<PathBuf>, content_hash: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            content_hash: Some(content_hash.into()),
        }
    }

    /// Returns the path to the context file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the content hash, if available.
    #[must_use]
    pub fn content_hash(&self) -> Option<&str> {
        self.content_hash.as_deref()
    }

    /// Computes the SHA-256 hash of a file's content.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the file to hash.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use patina::session::ContextFile;
    /// # async fn example() -> anyhow::Result<()> {
    /// let hash = ContextFile::compute_hash("/path/to/file.rs").await?;
    /// println!("File hash: {}", hash);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn compute_hash(path: impl AsRef<Path>) -> Result<String> {
        let content = fs::read(path.as_ref())
            .await
            .context("Failed to read file for hashing")?;
        let mut hasher = Sha256::new();
        hasher.update(&content);
        Ok(hex::encode(hasher.finalize()))
    }

    /// Checks if the file is unchanged since the hash was computed.
    ///
    /// Returns `true` if the file exists, has a stored hash, and the current
    /// content matches the stored hash. Returns `false` if:
    /// - The file doesn't exist
    /// - No hash was stored
    /// - The file content has changed
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use patina::session::ContextFile;
    /// # async fn example() -> anyhow::Result<()> {
    /// let cf = ContextFile::with_hash("/path/to/file.rs", "abc123...");
    /// if cf.is_unchanged().await? {
    ///     println!("File hasn't changed, safe to restore context");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn is_unchanged(&self) -> Result<bool> {
        // No stored hash means we can't verify - treat as changed
        let Some(stored_hash) = &self.content_hash else {
            return Ok(false);
        };

        // Check if file exists and compute current hash
        match Self::compute_hash(&self.path).await {
            Ok(current_hash) => Ok(&current_hash == stored_hash),
            Err(_) => {
                // File doesn't exist or can't be read - treat as changed
                Ok(false)
            }
        }
    }
}

/// Result of restoring session context.
///
/// When restoring a session, context files are checked against their stored
/// hashes to determine which files can be safely restored and which have changed.
#[derive(Debug, Clone)]
pub struct ContextRestoreResult {
    /// Files that were successfully verified as unchanged.
    /// These files can be safely used to restore context.
    pub restored_files: Vec<PathBuf>,

    /// Files that have been modified since the session was saved.
    /// The user should be notified that these files have changed.
    pub changed_files: Vec<PathBuf>,

    /// Files that no longer exist.
    /// The user should be notified that these files are missing.
    pub missing_files: Vec<PathBuf>,

    /// Skills that were active during the session.
    /// These should be re-enabled on resume.
    pub active_skills: Vec<String>,
}

/// Tracks session context including files read and active skills.
///
/// This struct captures the context state at a point in time so it can be
/// restored when resuming a session. It tracks:
/// - Files that were read during the session (for context restoration)
/// - Skills that were active (so they can be re-enabled on resume)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionContext {
    /// Files that were read during the session.
    context_files: Vec<ContextFile>,

    /// Names of skills that were active during the session.
    active_skills: Vec<String>,
}

impl SessionContext {
    /// Creates a new empty session context.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the context files.
    #[must_use]
    pub fn context_files(&self) -> &[ContextFile] {
        &self.context_files
    }

    /// Returns the active skills.
    #[must_use]
    pub fn active_skills(&self) -> &[String] {
        &self.active_skills
    }

    /// Adds a context file.
    ///
    /// # Arguments
    ///
    /// * `file` - The context file to add.
    pub fn add_file(&mut self, file: ContextFile) {
        self.context_files.push(file);
    }

    /// Adds an active skill if not already present.
    ///
    /// # Arguments
    ///
    /// * `skill_name` - Name of the skill to add.
    pub fn add_skill(&mut self, skill_name: impl Into<String>) {
        let name = skill_name.into();
        if !self.active_skills.contains(&name) {
            self.active_skills.push(name);
        }
    }

    /// Removes an active skill.
    ///
    /// # Arguments
    ///
    /// * `skill_name` - Name of the skill to remove.
    pub fn remove_skill(&mut self, skill_name: &str) {
        self.active_skills.retain(|s| s != skill_name);
    }

    /// Restores the session context by verifying context files.
    ///
    /// Checks each tracked file against its stored hash to determine which
    /// files are unchanged and can be safely used for context restoration.
    /// Files that have changed or are missing are reported separately.
    ///
    /// # Errors
    ///
    /// Returns an error if file hashing fails unexpectedly.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use patina::session::SessionContext;
    /// # async fn example() -> anyhow::Result<()> {
    /// let ctx = SessionContext::new();
    /// // ... context is loaded from a saved session ...
    ///
    /// let result = ctx.restore().await?;
    /// for path in &result.restored_files {
    ///     println!("Restored context from: {}", path.display());
    /// }
    /// for path in &result.changed_files {
    ///     println!("Warning: {} has changed since last session", path.display());
    /// }
    /// for skill in &result.active_skills {
    ///     println!("Re-enabling skill: {}", skill);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn restore(&self) -> Result<ContextRestoreResult> {
        let mut restored_files = Vec::new();
        let mut changed_files = Vec::new();
        let mut missing_files = Vec::new();

        for context_file in &self.context_files {
            let path = context_file.path().to_path_buf();

            // Check if file exists
            if !path.exists() {
                missing_files.push(path);
                continue;
            }

            // Check if file is unchanged
            if context_file.is_unchanged().await? {
                restored_files.push(path);
            } else {
                changed_files.push(path);
            }
        }

        Ok(ContextRestoreResult {
            restored_files,
            changed_files,
            missing_files,
            active_skills: self.active_skills.clone(),
        })
    }
}

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

    /// Optional worktree session linking.
    ///
    /// When present, this session is associated with a git worktree,
    /// enabling isolated development and session resume in the correct context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    worktree_session: Option<WorktreeSession>,

    /// Optional UI state for session resume.
    ///
    /// When present, stores the TUI state (scroll position, input buffer, cursor)
    /// so users can resume exactly where they left off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ui_state: Option<UiState>,

    /// Optional session context for resume.
    ///
    /// When present, tracks files that were read during the session and
    /// skills that were active, enabling context restoration on resume.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context: Option<SessionContext>,
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
            worktree_session: None,
            ui_state: None,
            context: None,
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

    /// Returns the worktree session, if this session is linked to a worktree.
    #[must_use]
    pub fn worktree_session(&self) -> Option<&WorktreeSession> {
        self.worktree_session.as_ref()
    }

    /// Sets the worktree session.
    ///
    /// # Arguments
    ///
    /// * `worktree_session` - The worktree session to link, or `None` to unlink.
    pub fn set_worktree_session(&mut self, worktree_session: Option<WorktreeSession>) {
        self.worktree_session = worktree_session;
        self.updated_at = SystemTime::now();
    }

    /// Returns the UI state, if saved.
    ///
    /// The UI state captures scroll position, input buffer, and cursor position
    /// so the session can be resumed exactly where it was left off.
    #[must_use]
    pub fn ui_state(&self) -> Option<&UiState> {
        self.ui_state.as_ref()
    }

    /// Sets the UI state.
    ///
    /// # Arguments
    ///
    /// * `ui_state` - The UI state to save, or `None` to clear it.
    pub fn set_ui_state(&mut self, ui_state: Option<UiState>) {
        self.ui_state = ui_state;
        self.updated_at = SystemTime::now();
    }

    /// Returns the session context, if saved.
    ///
    /// The session context tracks files read during the session and active skills,
    /// enabling context restoration on session resume.
    #[must_use]
    pub fn context(&self) -> Option<&SessionContext> {
        self.context.as_ref()
    }

    /// Sets the session context.
    ///
    /// # Arguments
    ///
    /// * `context` - The session context to save, or `None` to clear it.
    pub fn set_context(&mut self, context: Option<SessionContext>) {
        self.context = context;
        self.updated_at = SystemTime::now();
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

/// Context information for restoring a session in a worktree.
///
/// When a session is linked to a worktree, this struct provides the
/// information needed to restore the correct working context.
#[derive(Debug, Clone)]
pub struct WorktreeRestoreContext {
    /// Name of the worktree to restore to.
    pub worktree_name: String,

    /// The original branch from which the worktree was created.
    pub original_branch: String,

    /// Commits made during the session.
    pub commits: Vec<WorktreeCommit>,
}

/// Result of restoring a session with worktree context.
///
/// Contains the restored session and optional worktree context
/// if the session was linked to a worktree.
#[derive(Debug)]
pub struct SessionRestoreResult {
    /// The restored session.
    pub session: Session,

    /// Worktree context, if the session was linked to a worktree.
    pub worktree_context: Option<WorktreeRestoreContext>,
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
        atomic_write(&path, &json)
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
        atomic_write(&path, &json)
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

    /// Restores a session with its worktree context.
    ///
    /// Loads the session and extracts worktree context information if the
    /// session is linked to a worktree. This enables the caller to switch
    /// to the correct worktree context before resuming the session.
    ///
    /// # Arguments
    ///
    /// * `session_id` - The ID of the session to restore.
    ///
    /// # Errors
    ///
    /// Returns an error if the session cannot be loaded.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use patina::session::SessionManager;
    /// # use std::path::PathBuf;
    /// # async fn example() -> anyhow::Result<()> {
    /// let manager = SessionManager::new(PathBuf::from("~/.patina/sessions"));
    /// let result = manager.restore_with_worktree("session-id").await?;
    ///
    /// if let Some(ctx) = result.worktree_context {
    ///     println!("Session was in worktree: {}", ctx.worktree_name);
    ///     // Switch to worktree before resuming...
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn restore_with_worktree(&self, session_id: &str) -> Result<SessionRestoreResult> {
        let session = self.load(session_id).await?;

        let worktree_context = session
            .worktree_session
            .as_ref()
            .map(|wt| WorktreeRestoreContext {
                worktree_name: wt.worktree_name().to_string(),
                original_branch: wt.original_branch().to_string(),
                commits: wt.commits().to_vec(),
            });

        Ok(SessionRestoreResult {
            session,
            worktree_context,
        })
    }

    /// Finds all sessions linked to a specific worktree.
    ///
    /// Returns session IDs and metadata for all sessions that are linked
    /// to the specified worktree name.
    ///
    /// # Arguments
    ///
    /// * `worktree_name` - The name of the worktree to search for.
    ///
    /// # Errors
    ///
    /// Returns an error if sessions cannot be read.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use patina::session::SessionManager;
    /// # use std::path::PathBuf;
    /// # async fn example() -> anyhow::Result<()> {
    /// let manager = SessionManager::new(PathBuf::from("~/.patina/sessions"));
    /// let sessions = manager.find_by_worktree("feature-branch").await?;
    ///
    /// for (id, metadata) in sessions {
    ///     println!("Found session {} with {} messages", id, metadata.message_count);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn find_by_worktree(
        &self,
        worktree_name: &str,
    ) -> Result<Vec<(String, SessionMetadata)>> {
        let session_ids = self.list().await?;
        let mut matching = Vec::new();

        for id in session_ids {
            if let Ok(session) = self.load(&id).await {
                if let Some(wt) = session.worktree_session.as_ref() {
                    if wt.worktree_name() == worktree_name {
                        matching.push((
                            id.clone(),
                            SessionMetadata {
                                id,
                                working_dir: session.working_dir,
                                created_at: session.created_at,
                                updated_at: session.updated_at,
                                message_count: session.messages.len(),
                            },
                        ));
                    }
                }
            }
        }

        Ok(matching)
    }

    /// Finds the most recently updated session for a worktree.
    ///
    /// Returns the session ID and metadata for the session with the most
    /// recent `updated_at` timestamp that is linked to the specified worktree.
    ///
    /// # Arguments
    ///
    /// * `worktree_name` - The name of the worktree to search for.
    ///
    /// # Errors
    ///
    /// Returns an error if sessions cannot be read.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use patina::session::SessionManager;
    /// # use std::path::PathBuf;
    /// # async fn example() -> anyhow::Result<()> {
    /// let manager = SessionManager::new(PathBuf::from("~/.patina/sessions"));
    ///
    /// if let Some((id, metadata)) = manager.find_latest_for_worktree("feature").await? {
    ///     println!("Most recent session: {}", id);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn find_latest_for_worktree(
        &self,
        worktree_name: &str,
    ) -> Result<Option<(String, SessionMetadata)>> {
        let sessions = self.find_by_worktree(worktree_name).await?;

        Ok(sessions
            .into_iter()
            .max_by_key(|(_, metadata)| metadata.updated_at))
    }

    /// Finds the most recently updated session across all sessions.
    ///
    /// Returns the session ID and metadata for the session with the most
    /// recent `updated_at` timestamp, or `None` if no sessions exist.
    ///
    /// # Errors
    ///
    /// Returns an error if sessions cannot be read.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use patina::session::SessionManager;
    /// # use std::path::PathBuf;
    /// # async fn example() -> anyhow::Result<()> {
    /// let manager = SessionManager::new(PathBuf::from("~/.patina/sessions"));
    ///
    /// if let Some((id, metadata)) = manager.find_latest().await? {
    ///     println!("Most recent session: {}", id);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn find_latest(&self) -> Result<Option<(String, SessionMetadata)>> {
        let sessions = self.list_with_metadata().await?;

        Ok(sessions
            .into_iter()
            .max_by_key(|(_, metadata)| metadata.updated_at))
    }

    /// Lists all sessions sorted by most recently updated first.
    ///
    /// Returns session metadata sorted in descending order by `updated_at` timestamp.
    ///
    /// # Errors
    ///
    /// Returns an error if sessions cannot be read.
    pub async fn list_sorted(&self) -> Result<Vec<SessionMetadata>> {
        let sessions = self.list_with_metadata().await?;

        let mut sorted: Vec<SessionMetadata> =
            sessions.into_iter().map(|(_, metadata)| metadata).collect();

        sorted.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

        Ok(sorted)
    }
}

/// Formats a single session entry for display.
///
/// Returns a formatted string with session ID, working directory, message count,
/// and timestamp.
#[must_use]
pub fn format_session_entry(metadata: &SessionMetadata) -> String {
    let updated = format_timestamp(metadata.updated_at);
    format!(
        "{} | {} | {} msgs | {}",
        metadata.id,
        metadata.working_dir.display(),
        metadata.message_count,
        updated
    )
}

/// Formats a list of session metadata for display.
///
/// Sessions are sorted by most recently updated first. If the list is empty,
/// returns a message indicating no sessions were found.
#[must_use]
pub fn format_session_list(sessions: &[SessionMetadata]) -> String {
    if sessions.is_empty() {
        return "No sessions found.".to_string();
    }

    // Sort by updated_at descending (most recent first)
    let mut sorted = sessions.to_vec();
    sorted.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    let mut output = String::from("Available sessions:\n\n");

    for metadata in &sorted {
        output.push_str(&format_session_entry(metadata));
        output.push('\n');
    }

    output.push_str("\nUse --resume <session-id> or --resume last to resume a session.");
    output
}

/// Formats a `SystemTime` as a human-readable timestamp.
fn format_timestamp(time: SystemTime) -> String {
    match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => {
            let secs = duration.as_secs();
            // Simple UTC timestamp without chrono dependency
            // Format: seconds since epoch (or use chrono if available)
            let days = secs / 86400;
            let remaining = secs % 86400;
            let hours = remaining / 3600;
            let mins = (remaining % 3600) / 60;

            // Calculate approximate date from days since epoch (1970-01-01)
            let (year, month, day) = days_to_ymd(days);

            format!(
                "{:04}-{:02}-{:02} {:02}:{:02} UTC",
                year, month, day, hours, mins
            )
        }
        Err(_) => "unknown".to_string(),
    }
}

/// Converts days since Unix epoch to (year, month, day).
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Simplified algorithm for UTC date calculation
    // This is accurate for dates from 1970 onwards
    let mut remaining_days = days;
    let mut year = 1970u64;

    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if remaining_days < days_in_year {
            break;
        }
        remaining_days -= days_in_year;
        year += 1;
    }

    let is_leap = is_leap_year(year);
    let days_in_months: [u64; 12] = if is_leap {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 1u64;
    for days_in_month in days_in_months {
        if remaining_days < days_in_month {
            break;
        }
        remaining_days -= days_in_month;
        month += 1;
    }

    (year, month, remaining_days + 1)
}

/// Returns true if the given year is a leap year.
const fn is_leap_year(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
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

    // =========================================================================
    // WorktreeSession tests
    // =========================================================================

    #[test]
    fn test_worktree_session_new() {
        let wt_session = WorktreeSession::new("feature-x", "main");
        assert_eq!(wt_session.worktree_name(), "feature-x");
        assert_eq!(wt_session.original_branch(), "main");
        assert!(wt_session.commits().is_empty());
    }

    #[test]
    fn test_worktree_session_add_commit() {
        let mut wt_session = WorktreeSession::new("feature-x", "main");
        wt_session.add_commit("abc123", "Initial commit");
        wt_session.add_commit("def456", "Add feature");

        let commits = wt_session.commits();
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].hash, "abc123");
        assert_eq!(commits[0].message, "Initial commit");
        assert_eq!(commits[1].hash, "def456");
        assert_eq!(commits[1].message, "Add feature");
    }

    #[test]
    fn test_worktree_session_serialization() {
        let mut wt_session = WorktreeSession::new("experiment", "develop");
        wt_session.add_commit("123abc", "Test commit");

        let json = serde_json::to_string(&wt_session).expect("Failed to serialize");
        let deserialized: WorktreeSession =
            serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.worktree_name(), "experiment");
        assert_eq!(deserialized.original_branch(), "develop");
        assert_eq!(deserialized.commits().len(), 1);
    }

    #[test]
    fn test_session_with_worktree() {
        let mut session = Session::new(PathBuf::from("/test"));
        assert!(session.worktree_session().is_none());

        let wt_session = WorktreeSession::new("feature", "main");
        session.set_worktree_session(Some(wt_session));

        assert!(session.worktree_session().is_some());
        assert_eq!(
            session.worktree_session().unwrap().worktree_name(),
            "feature"
        );
    }

    #[test]
    fn test_session_with_worktree_serialization() {
        let mut session = Session::new(PathBuf::from("/project"));
        let mut wt_session = WorktreeSession::new("wt-test", "main");
        wt_session.add_commit("abc", "commit 1");
        session.set_worktree_session(Some(wt_session));
        session.add_message(test_message(Role::User, "Hello"));

        let json = serde_json::to_string(&session).expect("Failed to serialize");
        let deserialized: Session = serde_json::from_str(&json).expect("Failed to deserialize");

        assert!(deserialized.worktree_session().is_some());
        let wt = deserialized.worktree_session().unwrap();
        assert_eq!(wt.worktree_name(), "wt-test");
        assert_eq!(wt.commits().len(), 1);
    }

    #[tokio::test]
    async fn test_session_manager_save_load_with_worktree() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SessionManager::new(temp_dir.path().to_path_buf());

        let mut session = Session::new(PathBuf::from("/test"));
        let mut wt_session = WorktreeSession::new("feature-branch", "main");
        wt_session.add_commit("abc123", "Implement feature");
        session.set_worktree_session(Some(wt_session));
        session.add_message(test_message(Role::User, "Working on feature"));

        let id = manager.save(&session).await.unwrap();
        let loaded = manager.load(&id).await.unwrap();

        assert!(loaded.worktree_session().is_some());
        let wt = loaded.worktree_session().unwrap();
        assert_eq!(wt.worktree_name(), "feature-branch");
        assert_eq!(wt.original_branch(), "main");
        assert_eq!(wt.commits().len(), 1);
        assert_eq!(wt.commits()[0].hash, "abc123");
    }

    // =========================================================================
    // Session restore per worktree tests (8.5.2)
    // =========================================================================

    #[tokio::test]
    async fn test_session_restore_in_worktree() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SessionManager::new(temp_dir.path().to_path_buf());

        // Create a session linked to a worktree
        let mut session = Session::new(PathBuf::from("/project/.worktrees/feature-x"));
        let wt_session = WorktreeSession::new("feature-x", "main");
        session.set_worktree_session(Some(wt_session));
        session.add_message(test_message(Role::User, "Working in worktree"));

        let id = manager.save(&session).await.unwrap();

        // Restore should return the session with worktree context
        let restore_result = manager.restore_with_worktree(&id).await.unwrap();

        assert_eq!(restore_result.session.messages().len(), 1);
        assert!(restore_result.worktree_context.is_some());
        let ctx = restore_result.worktree_context.unwrap();
        assert_eq!(ctx.worktree_name, "feature-x");
        assert_eq!(ctx.original_branch, "main");
    }

    #[tokio::test]
    async fn test_find_sessions_by_worktree() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SessionManager::new(temp_dir.path().to_path_buf());

        // Create sessions for different worktrees
        let mut session1 = Session::new(PathBuf::from("/project/.worktrees/feature-a"));
        session1.set_worktree_session(Some(WorktreeSession::new("feature-a", "main")));
        session1.add_message(test_message(Role::User, "Session 1"));

        let mut session2 = Session::new(PathBuf::from("/project/.worktrees/feature-a"));
        session2.set_worktree_session(Some(WorktreeSession::new("feature-a", "main")));
        session2.add_message(test_message(Role::User, "Session 2"));

        let mut session3 = Session::new(PathBuf::from("/project/.worktrees/feature-b"));
        session3.set_worktree_session(Some(WorktreeSession::new("feature-b", "develop")));
        session3.add_message(test_message(Role::User, "Session 3"));

        // Session without worktree
        let mut session4 = Session::new(PathBuf::from("/project"));
        session4.add_message(test_message(Role::User, "Session 4"));

        manager.save(&session1).await.unwrap();
        manager.save(&session2).await.unwrap();
        manager.save(&session3).await.unwrap();
        manager.save(&session4).await.unwrap();

        // Find sessions for feature-a
        let feature_a_sessions = manager.find_by_worktree("feature-a").await.unwrap();
        assert_eq!(feature_a_sessions.len(), 2);

        // Find sessions for feature-b
        let feature_b_sessions = manager.find_by_worktree("feature-b").await.unwrap();
        assert_eq!(feature_b_sessions.len(), 1);

        // Find sessions for non-existent worktree
        let no_sessions = manager.find_by_worktree("non-existent").await.unwrap();
        assert!(no_sessions.is_empty());
    }

    #[tokio::test]
    async fn test_restore_session_without_worktree() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SessionManager::new(temp_dir.path().to_path_buf());

        // Create a session without worktree
        let mut session = Session::new(PathBuf::from("/project"));
        session.add_message(test_message(Role::User, "Regular session"));

        let id = manager.save(&session).await.unwrap();

        // Restore should work but have no worktree context
        let restore_result = manager.restore_with_worktree(&id).await.unwrap();

        assert_eq!(restore_result.session.messages().len(), 1);
        assert!(restore_result.worktree_context.is_none());
    }

    #[tokio::test]
    async fn test_find_latest_session_for_worktree() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SessionManager::new(temp_dir.path().to_path_buf());

        // Create two sessions for the same worktree at different times
        let mut session1 = Session::new(PathBuf::from("/project/.worktrees/feature"));
        session1.set_worktree_session(Some(WorktreeSession::new("feature", "main")));
        session1.add_message(test_message(Role::User, "First session"));
        let id1 = manager.save(&session1).await.unwrap();

        // Small delay to ensure different timestamps
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let mut session2 = Session::new(PathBuf::from("/project/.worktrees/feature"));
        session2.set_worktree_session(Some(WorktreeSession::new("feature", "main")));
        session2.add_message(test_message(Role::User, "Second session"));
        let id2 = manager.save(&session2).await.unwrap();

        // Find latest should return the most recently updated session
        let latest = manager.find_latest_for_worktree("feature").await.unwrap();
        assert!(latest.is_some());
        assert_eq!(latest.unwrap().0, id2);

        // Verify it's not the first session
        assert_ne!(id1, id2);
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

    // =========================================================================
    // Phase 10.1.1: UI State tests
    // =========================================================================

    #[test]
    fn test_ui_state_new() {
        let ui_state = UiState::new();
        assert_eq!(ui_state.scroll_offset(), 0);
        assert!(ui_state.input_buffer().is_empty());
        assert_eq!(ui_state.cursor_position(), 0);
    }

    #[test]
    fn test_ui_state_with_values() {
        let ui_state = UiState::with_state(42, "Hello, world!".to_string(), 7);
        assert_eq!(ui_state.scroll_offset(), 42);
        assert_eq!(ui_state.input_buffer(), "Hello, world!");
        assert_eq!(ui_state.cursor_position(), 7);
    }

    #[test]
    fn test_ui_state_serialization() {
        let ui_state = UiState::with_state(100, "Test input".to_string(), 5);

        let json = serde_json::to_string(&ui_state).expect("Failed to serialize");
        let deserialized: UiState = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.scroll_offset(), 100);
        assert_eq!(deserialized.input_buffer(), "Test input");
        assert_eq!(deserialized.cursor_position(), 5);
    }

    #[test]
    fn test_session_with_ui_state() {
        let mut session = Session::new(PathBuf::from("/test"));
        assert!(session.ui_state().is_none());

        let ui_state = UiState::with_state(50, "Draft message".to_string(), 13);
        session.set_ui_state(Some(ui_state));

        assert!(session.ui_state().is_some());
        assert_eq!(session.ui_state().unwrap().scroll_offset(), 50);
        assert_eq!(session.ui_state().unwrap().input_buffer(), "Draft message");
    }

    #[test]
    fn test_session_ui_state_serialization() {
        let mut session = Session::new(PathBuf::from("/project"));
        let ui_state = UiState::with_state(200, "Unsent draft".to_string(), 12);
        session.set_ui_state(Some(ui_state));
        session.add_message(test_message(Role::User, "Previous message"));

        let json = serde_json::to_string(&session).expect("Failed to serialize");
        let deserialized: Session = serde_json::from_str(&json).expect("Failed to deserialize");

        assert!(deserialized.ui_state().is_some());
        let ui = deserialized.ui_state().unwrap();
        assert_eq!(ui.scroll_offset(), 200);
        assert_eq!(ui.input_buffer(), "Unsent draft");
        assert_eq!(ui.cursor_position(), 12);
    }

    #[tokio::test]
    async fn test_session_ui_state_persistence() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SessionManager::new(temp_dir.path().to_path_buf());

        let mut session = Session::new(PathBuf::from("/test"));
        let ui_state = UiState::with_state(75, "Work in progress".to_string(), 16);
        session.set_ui_state(Some(ui_state));
        session.add_message(test_message(Role::User, "Hello"));

        let id = manager.save(&session).await.unwrap();
        let loaded = manager.load(&id).await.unwrap();

        assert!(loaded.ui_state().is_some());
        let ui = loaded.ui_state().unwrap();
        assert_eq!(ui.scroll_offset(), 75);
        assert_eq!(ui.input_buffer(), "Work in progress");
        assert_eq!(ui.cursor_position(), 16);
    }

    // =========================================================================
    // Phase 10.2.1: Context file tracking tests
    // =========================================================================

    #[test]
    fn test_context_file_new() {
        let cf = ContextFile::new("/project/src/main.rs");
        assert_eq!(cf.path(), Path::new("/project/src/main.rs"));
        assert!(cf.content_hash().is_none());
    }

    #[test]
    fn test_context_file_with_hash() {
        let cf = ContextFile::with_hash("/project/src/lib.rs", "abc123hash");
        assert_eq!(cf.path(), Path::new("/project/src/lib.rs"));
        assert_eq!(cf.content_hash(), Some("abc123hash"));
    }

    #[test]
    fn test_context_file_serialization() {
        let cf = ContextFile::with_hash("/project/README.md", "deadbeef");

        let json = serde_json::to_string(&cf).expect("Failed to serialize");
        let deserialized: ContextFile = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.path(), Path::new("/project/README.md"));
        assert_eq!(deserialized.content_hash(), Some("deadbeef"));
    }

    #[test]
    fn test_session_context_new() {
        let ctx = SessionContext::new();
        assert!(ctx.context_files().is_empty());
        assert!(ctx.active_skills().is_empty());
    }

    #[test]
    fn test_session_context_add_file() {
        let mut ctx = SessionContext::new();
        ctx.add_file(ContextFile::new("/project/src/main.rs"));
        ctx.add_file(ContextFile::with_hash("/project/Cargo.toml", "hash123"));

        assert_eq!(ctx.context_files().len(), 2);
    }

    #[test]
    fn test_session_context_add_skill() {
        let mut ctx = SessionContext::new();
        ctx.add_skill("narsil");
        ctx.add_skill("commit");

        assert_eq!(ctx.active_skills().len(), 2);
        assert!(ctx.active_skills().contains(&"narsil".to_string()));
        assert!(ctx.active_skills().contains(&"commit".to_string()));
    }

    #[test]
    fn test_session_context_add_skill_deduplicates() {
        let mut ctx = SessionContext::new();
        ctx.add_skill("narsil");
        ctx.add_skill("commit");
        ctx.add_skill("narsil"); // duplicate

        assert_eq!(ctx.active_skills().len(), 2);
    }

    #[test]
    fn test_session_context_remove_skill() {
        let mut ctx = SessionContext::new();
        ctx.add_skill("narsil");
        ctx.add_skill("commit");

        ctx.remove_skill("narsil");

        assert_eq!(ctx.active_skills().len(), 1);
        assert!(!ctx.active_skills().contains(&"narsil".to_string()));
        assert!(ctx.active_skills().contains(&"commit".to_string()));
    }

    #[test]
    fn test_session_context_serialization() {
        let mut ctx = SessionContext::new();
        ctx.add_file(ContextFile::new("/project/src/main.rs"));
        ctx.add_skill("narsil");

        let json = serde_json::to_string(&ctx).expect("Failed to serialize");
        let deserialized: SessionContext =
            serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.context_files().len(), 1);
        assert_eq!(deserialized.active_skills().len(), 1);
    }

    #[test]
    fn test_session_with_context() {
        let mut session = Session::new(PathBuf::from("/test"));
        assert!(session.context().is_none());

        let mut ctx = SessionContext::new();
        ctx.add_file(ContextFile::new("/test/src/lib.rs"));
        ctx.add_skill("commit");
        session.set_context(Some(ctx));

        assert!(session.context().is_some());
        assert_eq!(session.context().unwrap().context_files().len(), 1);
        assert_eq!(session.context().unwrap().active_skills().len(), 1);
    }

    #[test]
    fn test_session_context_serialization_with_session() {
        let mut session = Session::new(PathBuf::from("/project"));
        let mut ctx = SessionContext::new();
        ctx.add_file(ContextFile::with_hash("/project/Cargo.toml", "abc"));
        ctx.add_skill("narsil");
        session.set_context(Some(ctx));
        session.add_message(test_message(Role::User, "Hello"));

        let json = serde_json::to_string(&session).expect("Failed to serialize");
        let deserialized: Session = serde_json::from_str(&json).expect("Failed to deserialize");

        assert!(deserialized.context().is_some());
        let ctx = deserialized.context().unwrap();
        assert_eq!(ctx.context_files().len(), 1);
        assert_eq!(
            ctx.context_files()[0].path(),
            Path::new("/project/Cargo.toml")
        );
        assert_eq!(ctx.active_skills().len(), 1);
    }

    #[tokio::test]
    async fn test_session_context_persistence() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SessionManager::new(temp_dir.path().to_path_buf());

        let mut session = Session::new(PathBuf::from("/test"));
        let mut ctx = SessionContext::new();
        ctx.add_file(ContextFile::with_hash("/test/src/main.rs", "hash123"));
        ctx.add_file(ContextFile::new("/test/README.md"));
        ctx.add_skill("narsil");
        ctx.add_skill("commit");
        session.set_context(Some(ctx));
        session.add_message(test_message(Role::User, "Working on feature"));

        let id = manager.save(&session).await.unwrap();
        let loaded = manager.load(&id).await.unwrap();

        assert!(loaded.context().is_some());
        let ctx = loaded.context().unwrap();
        assert_eq!(ctx.context_files().len(), 2);
        assert_eq!(ctx.active_skills().len(), 2);
        assert!(ctx.active_skills().contains(&"narsil".to_string()));
    }

    // =========================================================================
    // Phase 10.2.2: Context restoration tests
    // =========================================================================

    #[tokio::test]
    async fn test_compute_file_hash() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        fs::write(&file_path, "Hello, world!").await.unwrap();

        let hash = ContextFile::compute_hash(&file_path).await.unwrap();
        assert!(!hash.is_empty());
        assert_eq!(hash.len(), 64); // SHA-256 hex = 64 chars

        // Same content should produce same hash
        let hash2 = ContextFile::compute_hash(&file_path).await.unwrap();
        assert_eq!(hash, hash2);
    }

    #[tokio::test]
    async fn test_compute_file_hash_different_content() {
        let temp_dir = TempDir::new().unwrap();
        let file1 = temp_dir.path().join("file1.txt");
        let file2 = temp_dir.path().join("file2.txt");

        fs::write(&file1, "Content A").await.unwrap();
        fs::write(&file2, "Content B").await.unwrap();

        let hash1 = ContextFile::compute_hash(&file1).await.unwrap();
        let hash2 = ContextFile::compute_hash(&file2).await.unwrap();

        assert_ne!(hash1, hash2);
    }

    #[tokio::test]
    async fn test_context_file_is_unchanged() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.rs");
        fs::write(&file_path, "fn main() {}").await.unwrap();

        // Create context file with current hash
        let hash = ContextFile::compute_hash(&file_path).await.unwrap();
        let cf = ContextFile::with_hash(&file_path, &hash);

        // File should be unchanged
        assert!(cf.is_unchanged().await.unwrap());
    }

    #[tokio::test]
    async fn test_context_file_is_changed() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.rs");
        fs::write(&file_path, "fn main() {}").await.unwrap();

        // Create context file with current hash
        let hash = ContextFile::compute_hash(&file_path).await.unwrap();
        let cf = ContextFile::with_hash(&file_path, &hash);

        // Modify the file
        fs::write(&file_path, "fn main() { println!(\"modified\"); }")
            .await
            .unwrap();

        // File should now be changed
        assert!(!cf.is_unchanged().await.unwrap());
    }

    #[tokio::test]
    async fn test_context_file_without_hash_is_changed() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.rs");
        fs::write(&file_path, "fn main() {}").await.unwrap();

        // Create context file without hash
        let cf = ContextFile::new(&file_path);

        // Without stored hash, we can't verify - treat as changed
        assert!(!cf.is_unchanged().await.unwrap());
    }

    #[tokio::test]
    async fn test_context_file_missing_file() {
        let cf = ContextFile::with_hash("/nonexistent/path/file.rs", "somehash");

        // Missing file should be treated as changed
        assert!(!cf.is_unchanged().await.unwrap());
    }

    #[tokio::test]
    async fn test_restore_context_unchanged_files() {
        let temp_dir = TempDir::new().unwrap();

        // Create test files
        let file1 = temp_dir.path().join("src/main.rs");
        let file2 = temp_dir.path().join("src/lib.rs");
        fs::create_dir_all(temp_dir.path().join("src"))
            .await
            .unwrap();
        fs::write(&file1, "fn main() {}").await.unwrap();
        fs::write(&file2, "pub fn lib_fn() {}").await.unwrap();

        // Create context with hashes
        let hash1 = ContextFile::compute_hash(&file1).await.unwrap();
        let hash2 = ContextFile::compute_hash(&file2).await.unwrap();

        let mut ctx = SessionContext::new();
        ctx.add_file(ContextFile::with_hash(&file1, &hash1));
        ctx.add_file(ContextFile::with_hash(&file2, &hash2));
        ctx.add_skill("narsil");

        // Restore context
        let result = ctx.restore().await.unwrap();

        // All files should be restored (unchanged)
        assert_eq!(result.restored_files.len(), 2);
        assert!(result.changed_files.is_empty());
        assert!(result.missing_files.is_empty());
        assert_eq!(result.active_skills, vec!["narsil".to_string()]);
    }

    #[tokio::test]
    async fn test_restore_context_with_changed_file() {
        let temp_dir = TempDir::new().unwrap();

        // Create test files
        let file1 = temp_dir.path().join("unchanged.rs");
        let file2 = temp_dir.path().join("changed.rs");
        fs::write(&file1, "unchanged content").await.unwrap();
        fs::write(&file2, "original content").await.unwrap();

        // Create context with hashes
        let hash1 = ContextFile::compute_hash(&file1).await.unwrap();
        let hash2 = ContextFile::compute_hash(&file2).await.unwrap();

        let mut ctx = SessionContext::new();
        ctx.add_file(ContextFile::with_hash(&file1, &hash1));
        ctx.add_file(ContextFile::with_hash(&file2, &hash2));

        // Modify one file
        fs::write(&file2, "modified content").await.unwrap();

        // Restore context
        let result = ctx.restore().await.unwrap();

        assert_eq!(result.restored_files.len(), 1);
        assert_eq!(result.changed_files.len(), 1);
        assert!(result.missing_files.is_empty());
    }

    #[tokio::test]
    async fn test_restore_context_with_missing_file() {
        let temp_dir = TempDir::new().unwrap();

        // Create only one file
        let file1 = temp_dir.path().join("exists.rs");
        fs::write(&file1, "content").await.unwrap();

        let hash1 = ContextFile::compute_hash(&file1).await.unwrap();

        let mut ctx = SessionContext::new();
        ctx.add_file(ContextFile::with_hash(&file1, &hash1));
        ctx.add_file(ContextFile::with_hash(
            temp_dir.path().join("missing.rs"),
            "oldhash",
        ));

        // Restore context
        let result = ctx.restore().await.unwrap();

        assert_eq!(result.restored_files.len(), 1);
        assert!(result.changed_files.is_empty());
        assert_eq!(result.missing_files.len(), 1);
    }

    // =========================================================================
    // Phase 10.3.1: Resume flag tests
    // =========================================================================

    #[test]
    fn test_default_sessions_dir() {
        // Should return a valid path on all platforms
        let result = super::default_sessions_dir();
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.to_string_lossy().contains("sessions"));
    }

    #[tokio::test]
    async fn test_find_latest_empty() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SessionManager::new(temp_dir.path().to_path_buf());

        let result = manager.find_latest().await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_find_latest_single_session() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SessionManager::new(temp_dir.path().to_path_buf());

        let mut session = Session::new(PathBuf::from("/test"));
        session.add_message(test_message(Role::User, "Hello"));
        let id = manager.save(&session).await.unwrap();

        let result = manager.find_latest().await.unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, id);
    }

    #[tokio::test]
    async fn test_find_latest_multiple_sessions() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SessionManager::new(temp_dir.path().to_path_buf());

        // Create first session
        let mut session1 = Session::new(PathBuf::from("/test1"));
        session1.add_message(test_message(Role::User, "First"));
        let _id1 = manager.save(&session1).await.unwrap();

        // Small delay to ensure different timestamps
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // Create second session (should be latest)
        let mut session2 = Session::new(PathBuf::from("/test2"));
        session2.add_message(test_message(Role::User, "Second"));
        let id2 = manager.save(&session2).await.unwrap();

        let result = manager.find_latest().await.unwrap();
        assert!(result.is_some());
        let (latest_id, _) = result.unwrap();
        assert_eq!(latest_id, id2);
    }

    // =========================================================================
    // Phase 10.3.2: List sessions flag tests
    // =========================================================================

    #[test]
    fn test_format_session_entry() {
        use std::time::{Duration, UNIX_EPOCH};

        let metadata = SessionMetadata {
            id: "abc123-def456".to_string(),
            working_dir: PathBuf::from("/home/user/project"),
            created_at: UNIX_EPOCH + Duration::from_secs(1706745600), // 2024-02-01 00:00:00 UTC
            updated_at: UNIX_EPOCH + Duration::from_secs(1706745600),
            message_count: 5,
        };

        let output = super::format_session_entry(&metadata);

        // Should contain the session ID
        assert!(output.contains("abc123-def456"));
        // Should contain the working directory
        assert!(output.contains("/home/user/project"));
        // Should contain the message count
        assert!(output.contains("5"));
    }

    #[test]
    fn test_format_session_list_empty() {
        let sessions: Vec<SessionMetadata> = vec![];
        let output = super::format_session_list(&sessions);

        assert!(output.contains("No sessions found"));
    }

    #[test]
    fn test_format_session_list_single() {
        use std::time::{Duration, UNIX_EPOCH};

        let sessions = vec![SessionMetadata {
            id: "session-1".to_string(),
            working_dir: PathBuf::from("/project"),
            created_at: UNIX_EPOCH + Duration::from_secs(1706745600),
            updated_at: UNIX_EPOCH + Duration::from_secs(1706745600),
            message_count: 3,
        }];

        let output = super::format_session_list(&sessions);

        assert!(output.contains("session-1"));
        assert!(output.contains("/project"));
        assert!(!output.contains("No sessions found"));
    }

    #[test]
    fn test_format_session_list_multiple_sorted_by_updated() {
        use std::time::{Duration, UNIX_EPOCH};

        // Sessions provided unsorted
        let sessions = vec![
            SessionMetadata {
                id: "old-session".to_string(),
                working_dir: PathBuf::from("/old"),
                created_at: UNIX_EPOCH + Duration::from_secs(1000),
                updated_at: UNIX_EPOCH + Duration::from_secs(1000),
                message_count: 1,
            },
            SessionMetadata {
                id: "new-session".to_string(),
                working_dir: PathBuf::from("/new"),
                created_at: UNIX_EPOCH + Duration::from_secs(2000),
                updated_at: UNIX_EPOCH + Duration::from_secs(2000),
                message_count: 2,
            },
        ];

        let output = super::format_session_list(&sessions);

        // Most recent should appear first
        let new_pos = output
            .find("new-session")
            .expect("new-session should exist");
        let old_pos = output
            .find("old-session")
            .expect("old-session should exist");
        assert!(
            new_pos < old_pos,
            "Newer session should appear first in the list"
        );
    }

    #[tokio::test]
    async fn test_list_all_sessions_sorted() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SessionManager::new(temp_dir.path().to_path_buf());

        // Create sessions with different timestamps
        let mut session1 = Session::new(PathBuf::from("/project1"));
        session1.add_message(test_message(Role::User, "First"));
        manager.save(&session1).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let mut session2 = Session::new(PathBuf::from("/project2"));
        session2.add_message(test_message(Role::User, "Second"));
        manager.save(&session2).await.unwrap();

        // list_sorted should return sessions sorted by updated_at descending
        let sorted = manager.list_sorted().await.unwrap();
        assert_eq!(sorted.len(), 2);
        assert!(sorted[0].updated_at >= sorted[1].updated_at);
    }
}
