//! Worktree session tracking.
//!
//! This module provides types for linking sessions to git worktrees,
//! enabling isolated development with session context preservation.

use serde::{Deserialize, Serialize};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worktree_commit() {
        let commit = WorktreeCommit {
            hash: "abc123".to_string(),
            message: "Initial commit".to_string(),
        };

        assert_eq!(commit.hash, "abc123");
        assert_eq!(commit.message, "Initial commit");
    }

    #[test]
    fn test_worktree_session_new() {
        let session = WorktreeSession::new("feature-branch", "main");

        assert_eq!(session.worktree_name(), "feature-branch");
        assert_eq!(session.original_branch(), "main");
        assert!(session.commits().is_empty());
    }

    #[test]
    fn test_worktree_session_add_commit() {
        let mut session = WorktreeSession::new("feature-branch", "main");
        session.add_commit("abc123", "First commit");
        session.add_commit("def456", "Second commit");

        assert_eq!(session.commits().len(), 2);
        assert_eq!(session.commits()[0].hash, "abc123");
        assert_eq!(session.commits()[1].message, "Second commit");
    }
}
