//! Session management for persistence operations.
//!
//! This module provides the `SessionManager` which handles saving, loading,
//! and querying sessions from disk.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::SystemTime;
use tokio::fs;
use uuid::Uuid;

use super::persistence::{atomic_write, validate_session_id, SessionFile};
use super::worktree::WorktreeCommit;
use super::Session;

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
            .id()
            .map(String::from)
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        // Create a copy with the ID set
        let mut session_to_save = session.clone();
        session_to_save.set_id(Some(session_id.clone()));
        session_to_save.touch();

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
        session_to_save.set_id(Some(session_id.to_string()));
        session_to_save.touch();

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
            working_dir: session.working_dir().to_path_buf(),
            created_at: session.created_at(),
            updated_at: session.updated_at(),
            message_count: session.messages().len(),
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

        let worktree_context = session.worktree_session().map(|wt| WorktreeRestoreContext {
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
                if let Some(wt) = session.worktree_session() {
                    if wt.worktree_name() == worktree_name {
                        matching.push((
                            id.clone(),
                            SessionMetadata {
                                id,
                                working_dir: session.working_dir().to_path_buf(),
                                created_at: session.created_at(),
                                updated_at: session.updated_at(),
                                message_count: session.messages().len(),
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
