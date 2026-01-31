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

use crate::error::{RctError, RctResult};
use crate::types::message::Message;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tokio::fs;
use tracing::{error, warn};
use uuid::Uuid;

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

/// Information about a commit made during a worktree session.
///
/// Tracks the commit hash and message for commits made while working
/// in a worktree-linked session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeCommit {
    /// The commit hash (short or full).
    pub hash: String,

    /// The commit message (first line).
    pub message: String,
}

/// Links a session to a git worktree for isolated development.
///
/// When a session is associated with a worktree, this struct tracks:
/// - The worktree name for context switching
/// - The original branch the worktree was created from
/// - All commits made during the session
///
/// This enables session resume to restore the correct worktree context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeSession {
    /// Name of the worktree (matches `WorktreeInfo::name`).
    worktree_name: String,

    /// The branch from which the worktree was created.
    original_branch: String,

    /// Commits made during this session.
    commits: Vec<WorktreeCommit>,
}

impl WorktreeSession {
    /// Creates a new worktree session.
    ///
    /// # Arguments
    ///
    /// * `worktree_name` - Name of the worktree.
    /// * `original_branch` - The branch from which the worktree was created.
    #[must_use]
    pub fn new(worktree_name: impl Into<String>, original_branch: impl Into<String>) -> Self {
        Self {
            worktree_name: worktree_name.into(),
            original_branch: original_branch.into(),
            commits: Vec::new(),
        }
    }

    /// Returns the worktree name.
    #[must_use]
    pub fn worktree_name(&self) -> &str {
        &self.worktree_name
    }

    /// Returns the original branch name.
    #[must_use]
    pub fn original_branch(&self) -> &str {
        &self.original_branch
    }

    /// Returns the commits made during this session.
    #[must_use]
    pub fn commits(&self) -> &[WorktreeCommit] {
        &self.commits
    }

    /// Adds a commit to the session.
    ///
    /// # Arguments
    ///
    /// * `hash` - The commit hash (short or full).
    /// * `message` - The commit message (first line).
    pub fn add_commit(&mut self, hash: impl Into<String>, message: impl Into<String>) {
        self.commits.push(WorktreeCommit {
            hash: hash.into(),
            message: message.into(),
        });
    }
}

/// UI state for session resume.
///
/// Captures the terminal UI state so it can be restored when resuming a session.
/// This allows users to continue exactly where they left off, including their
/// scroll position, any unsent input, and cursor position.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiState {
    /// Vertical scroll offset in the message view.
    scroll_offset: usize,

    /// Content of the input buffer (unsent text).
    input_buffer: String,

    /// Cursor position within the input buffer.
    cursor_position: usize,
}

impl UiState {
    /// Creates a new UI state with default values.
    ///
    /// Default state has scroll at top, empty input, cursor at position 0.
    #[must_use]
    pub fn new() -> Self {
        Self {
            scroll_offset: 0,
            input_buffer: String::new(),
            cursor_position: 0,
        }
    }

    /// Creates a UI state with specified values.
    ///
    /// # Arguments
    ///
    /// * `scroll_offset` - Vertical scroll position in the message view.
    /// * `input_buffer` - Current text in the input field.
    /// * `cursor_position` - Cursor position within the input buffer.
    #[must_use]
    pub fn with_state(scroll_offset: usize, input_buffer: String, cursor_position: usize) -> Self {
        Self {
            scroll_offset,
            input_buffer,
            cursor_position,
        }
    }

    /// Returns the scroll offset.
    #[must_use]
    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    /// Returns the input buffer contents.
    #[must_use]
    pub fn input_buffer(&self) -> &str {
        &self.input_buffer
    }

    /// Returns the cursor position.
    #[must_use]
    pub fn cursor_position(&self) -> usize {
        self.cursor_position
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self::new()
    }
}

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
                worktree_name: wt.worktree_name.clone(),
                original_branch: wt.original_branch.clone(),
                commits: wt.commits.clone(),
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
                    if wt.worktree_name == worktree_name {
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
}
