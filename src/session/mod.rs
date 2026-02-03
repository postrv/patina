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

mod context;
mod format;
mod manager;
mod persistence;
mod ui_state;
mod worktree;

// Re-export types
pub use context::{ContextFile, ContextRestoreResult, SessionContext};
pub use format::{format_session_entry, format_session_list};
pub use manager::{SessionManager, SessionMetadata, SessionRestoreResult, WorktreeRestoreContext};
pub use ui_state::UiState;
pub use worktree::{WorktreeCommit, WorktreeSession};

use crate::types::message::Message;
use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::SystemTime;

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

    /// Sets the session ID.
    ///
    /// # Arguments
    ///
    /// * `id` - The session ID to set, or `None` to clear.
    pub(crate) fn set_id(&mut self, id: Option<String>) {
        self.id = id;
    }

    /// Updates the `updated_at` timestamp to the current time.
    pub(crate) fn touch(&mut self) {
        self.updated_at = SystemTime::now();
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

#[cfg(test)]
mod tests {
    use super::persistence::validate_session_id;
    use super::*;
    use crate::types::message::Role;
    use std::path::Path;
    use tempfile::TempDir;
    use tokio::fs;

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
        assert!(validate_session_id("").is_err());
    }

    #[test]
    fn test_validate_session_id_rejects_special_chars() {
        assert!(validate_session_id("../parent").is_err());
        assert!(validate_session_id("has/slash").is_err());
        assert!(validate_session_id("has.dot").is_err());
        assert!(validate_session_id("has space").is_err());
        assert!(validate_session_id("has:colon").is_err());
    }

    #[test]
    fn test_validate_session_id_accepts_valid() {
        assert!(validate_session_id("valid-id").is_ok());
        assert!(validate_session_id("valid_id").is_ok());
        assert!(validate_session_id("ValidId123").is_ok());
        assert!(validate_session_id("550e8400-e29b-41d4-a716-446655440000").is_ok());
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
