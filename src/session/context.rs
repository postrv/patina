//! Session context tracking for file reads and active skills.
//!
//! This module provides types for tracking files read during a session
//! and skills that were active, enabling context restoration on resume.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::fs;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_file_new() {
        let cf = ContextFile::new("/path/to/file.rs");
        assert_eq!(cf.path(), Path::new("/path/to/file.rs"));
        assert!(cf.content_hash().is_none());
    }

    #[test]
    fn test_context_file_with_hash() {
        let cf = ContextFile::with_hash("/path/to/file.rs", "abc123");
        assert_eq!(cf.path(), Path::new("/path/to/file.rs"));
        assert_eq!(cf.content_hash(), Some("abc123"));
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
        ctx.add_file(ContextFile::new("/path/to/file.rs"));
        assert_eq!(ctx.context_files().len(), 1);
    }

    #[test]
    fn test_session_context_add_skill() {
        let mut ctx = SessionContext::new();
        ctx.add_skill("test-skill");
        ctx.add_skill("test-skill"); // duplicate
        assert_eq!(ctx.active_skills().len(), 1);
        assert_eq!(ctx.active_skills()[0], "test-skill");
    }

    #[test]
    fn test_session_context_remove_skill() {
        let mut ctx = SessionContext::new();
        ctx.add_skill("skill1");
        ctx.add_skill("skill2");
        ctx.remove_skill("skill1");
        assert_eq!(ctx.active_skills().len(), 1);
        assert_eq!(ctx.active_skills()[0], "skill2");
    }
}
