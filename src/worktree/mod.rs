//! Git worktree integration for parallel development workflows.
//!
//! This module provides functionality to manage git worktrees, enabling
//! parallel development across multiple branches without switching contexts.
//!
//! # Features
//!
//! - Create and manage worktrees for isolated feature development
//! - Track worktree status (dirty, ahead/behind, locked)
//! - Experiment mode for risky changes with easy accept/reject
//!
//! # Example
//!
//! ```no_run
//! use patina::worktree::{WorktreeManager, WorktreeConfig};
//! use std::path::PathBuf;
//!
//! let manager = WorktreeManager::new(PathBuf::from(".")).unwrap();
//! assert!(manager.is_git_repo());
//! println!("Repo root: {:?}", manager.repo_root());
//! ```

mod experiment;

pub use experiment::{
    Experiment, ExperimentConfig, ExperimentError, ExperimentResult, ExperimentState,
};

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Configuration for worktree management.
///
/// Controls where worktrees are created and how they are named.
#[derive(Debug, Clone)]
pub struct WorktreeConfig {
    /// Directory where worktrees are stored, relative to repo root.
    ///
    /// Default: `.worktrees`
    pub worktree_dir: String,

    /// Prefix for branch names created for worktrees.
    ///
    /// Default: `wt/`
    pub branch_prefix: String,

    /// Whether to automatically clean up prunable worktrees.
    ///
    /// Default: `false`
    pub auto_cleanup: bool,
}

impl Default for WorktreeConfig {
    fn default() -> Self {
        Self {
            worktree_dir: ".worktrees".to_string(),
            branch_prefix: "wt/".to_string(),
            auto_cleanup: false,
        }
    }
}

/// Status of a worktree's working directory.
///
/// Contains counts of files in various states and ahead/behind counts
/// relative to the upstream branch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorktreeStatus {
    /// Number of modified (unstaged) files.
    pub modified: usize,

    /// Number of staged files.
    pub staged: usize,

    /// Number of untracked files.
    pub untracked: usize,

    /// Number of commits ahead of upstream.
    pub ahead: usize,

    /// Number of commits behind upstream.
    pub behind: usize,
}

impl WorktreeStatus {
    /// Returns `true` if the worktree has no uncommitted changes.
    ///
    /// A worktree is considered clean if there are no modified, staged,
    /// or untracked files. Ahead/behind counts do not affect cleanliness.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.modified == 0 && self.staged == 0 && self.untracked == 0
    }
}

/// Information about a git worktree.
///
/// Represents the state of a single worktree in the repository.
#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    /// Name of the worktree (typically derived from branch name).
    pub name: String,

    /// Absolute path to the worktree directory.
    pub path: PathBuf,

    /// Branch checked out in this worktree.
    pub branch: String,

    /// Whether this is the main worktree (the original repository).
    pub is_main: bool,

    /// Whether the worktree is locked (prevents removal).
    pub is_locked: bool,

    /// Whether the worktree can be pruned (missing directory).
    pub is_prunable: bool,
}

/// Manager for git worktree operations.
///
/// Provides methods to create, list, and remove worktrees for a repository.
///
/// # Example
///
/// ```no_run
/// use patina::worktree::WorktreeManager;
/// use std::path::PathBuf;
///
/// let manager = WorktreeManager::new(PathBuf::from(".")).unwrap();
/// println!("Git repo at: {:?}", manager.repo_root());
/// ```
#[derive(Debug)]
pub struct WorktreeManager {
    /// Root directory of the git repository.
    repo_root: PathBuf,

    /// Configuration for worktree operations.
    config: WorktreeConfig,
}

impl WorktreeManager {
    /// Creates a new `WorktreeManager` for the given path.
    ///
    /// The path can be any directory within a git repository. The manager
    /// will find the repository root automatically.
    ///
    /// # Errors
    ///
    /// Returns `WorktreeError::NotAGitRepository` if the path is not within
    /// a git repository.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use patina::worktree::WorktreeManager;
    /// use std::path::PathBuf;
    ///
    /// let manager = WorktreeManager::new(PathBuf::from("./src"))?;
    /// # Ok::<(), patina::worktree::WorktreeError>(())
    /// ```
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, WorktreeError> {
        let path = path.into();
        let repo_root = Self::find_git_root(&path)?;

        Ok(Self {
            repo_root,
            config: WorktreeConfig::default(),
        })
    }

    /// Creates a new `WorktreeManager` with custom configuration.
    ///
    /// # Errors
    ///
    /// Returns `WorktreeError::NotAGitRepository` if the path is not within
    /// a git repository.
    pub fn with_config(
        path: impl Into<PathBuf>,
        config: WorktreeConfig,
    ) -> Result<Self, WorktreeError> {
        let path = path.into();
        let repo_root = Self::find_git_root(&path)?;

        Ok(Self { repo_root, config })
    }

    /// Returns `true` if this manager is associated with a git repository.
    #[must_use]
    pub fn is_git_repo(&self) -> bool {
        self.repo_root.join(".git").exists()
    }

    /// Returns the root directory of the git repository.
    #[must_use]
    pub fn repo_root(&self) -> &PathBuf {
        &self.repo_root
    }

    /// Returns the current configuration.
    #[must_use]
    pub fn config(&self) -> &WorktreeConfig {
        &self.config
    }

    /// Lists all worktrees in the repository.
    ///
    /// Returns information about each worktree including the main worktree.
    ///
    /// # Errors
    ///
    /// Returns `WorktreeError::GitCommand` if the git command fails.
    pub fn list(&self) -> Result<Vec<WorktreeInfo>, WorktreeError> {
        let output = Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(&self.repo_root)
            .output()
            .map_err(|e| WorktreeError::GitCommand {
                command: "git worktree list".to_string(),
                message: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(WorktreeError::GitCommand {
                command: "git worktree list".to_string(),
                message: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        self.parse_worktree_list(&stdout)
    }

    /// Creates a new worktree with the given name.
    ///
    /// Creates a new branch with the configured prefix and checks it out
    /// in a new worktree directory.
    ///
    /// # Errors
    ///
    /// - `WorktreeError::WorktreeExists` if a worktree with this name already exists.
    /// - `WorktreeError::GitCommand` if the git command fails.
    pub fn create(&self, name: &str) -> Result<WorktreeInfo, WorktreeError> {
        // Check if worktree already exists
        let existing = self.list()?;
        if existing.iter().any(|w| w.name == name) {
            return Err(WorktreeError::WorktreeExists(name.to_string()));
        }

        // Build paths and branch name
        let worktree_dir = self.repo_root.join(&self.config.worktree_dir);
        let worktree_path = worktree_dir.join(name);
        let branch_name = format!("{}{}", self.config.branch_prefix, name);

        // Create the worktree directory if it doesn't exist
        std::fs::create_dir_all(&worktree_dir)?;

        // Create the worktree with a new branch
        let output = Command::new("git")
            .args([
                "worktree",
                "add",
                worktree_path.to_str().unwrap_or_default(),
                "-b",
                &branch_name,
            ])
            .current_dir(&self.repo_root)
            .output()
            .map_err(|e| WorktreeError::GitCommand {
                command: "git worktree add".to_string(),
                message: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(WorktreeError::GitCommand {
                command: "git worktree add".to_string(),
                message: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        Ok(WorktreeInfo {
            name: name.to_string(),
            path: worktree_path,
            branch: branch_name,
            is_main: false,
            is_locked: false,
            is_prunable: false,
        })
    }

    /// Removes a worktree by name.
    ///
    /// This removes the worktree directory and its git tracking, but does not
    /// delete the associated branch.
    ///
    /// # Errors
    ///
    /// - `WorktreeError::WorktreeNotFound` if no worktree with this name exists.
    /// - `WorktreeError::GitCommand` if the git command fails.
    pub fn remove(&self, name: &str) -> Result<(), WorktreeError> {
        // Find the worktree
        let worktrees = self.list()?;
        let worktree = worktrees
            .iter()
            .find(|w| w.name == name && !w.is_main)
            .ok_or_else(|| WorktreeError::WorktreeNotFound(name.to_string()))?;

        let output = Command::new("git")
            .args([
                "worktree",
                "remove",
                worktree.path.to_str().unwrap_or_default(),
            ])
            .current_dir(&self.repo_root)
            .output()
            .map_err(|e| WorktreeError::GitCommand {
                command: "git worktree remove".to_string(),
                message: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(WorktreeError::GitCommand {
                command: "git worktree remove".to_string(),
                message: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        Ok(())
    }

    /// Gets the status of a worktree at the given path.
    ///
    /// Returns information about modified, staged, and untracked files,
    /// as well as ahead/behind counts relative to the upstream branch.
    ///
    /// # Errors
    ///
    /// Returns `WorktreeError::GitCommand` if git commands fail.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use patina::worktree::WorktreeManager;
    /// use std::path::PathBuf;
    ///
    /// let manager = WorktreeManager::new(PathBuf::from(".")).unwrap();
    /// let status = manager.status(&PathBuf::from(".")).unwrap();
    /// println!("Modified: {}, Staged: {}", status.modified, status.staged);
    /// ```
    pub fn status(&self, path: &Path) -> Result<WorktreeStatus, WorktreeError> {
        let mut status = WorktreeStatus::default();

        // Get porcelain status for modified/staged/untracked counts
        let output = Command::new("git")
            .args(["status", "--porcelain=v1"])
            .current_dir(path)
            .output()
            .map_err(|e| WorktreeError::GitCommand {
                command: "git status".to_string(),
                message: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(WorktreeError::GitCommand {
                command: "git status".to_string(),
                message: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.len() < 2 {
                continue;
            }

            let index_status = line.chars().next().unwrap_or(' ');
            let worktree_status = line.chars().nth(1).unwrap_or(' ');

            // Staged changes (index column has a status letter)
            if index_status != ' ' && index_status != '?' {
                status.staged += 1;
            }

            // Modified (unstaged) changes in worktree
            if worktree_status != ' ' && worktree_status != '?' {
                status.modified += 1;
            }

            // Untracked files
            if index_status == '?' {
                status.untracked += 1;
            }
        }

        // Get ahead/behind counts from rev-list
        if let Ok(ahead_behind) = self.get_ahead_behind(path) {
            status.ahead = ahead_behind.0;
            status.behind = ahead_behind.1;
        }

        Ok(status)
    }

    /// Gets the ahead/behind counts relative to upstream.
    fn get_ahead_behind(&self, path: &Path) -> Result<(usize, usize), WorktreeError> {
        // First check if we have an upstream
        let upstream_output = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "@{upstream}"])
            .current_dir(path)
            .output()
            .map_err(|e| WorktreeError::GitCommand {
                command: "git rev-parse upstream".to_string(),
                message: e.to_string(),
            })?;

        if !upstream_output.status.success() {
            // No upstream configured
            return Ok((0, 0));
        }

        // Get ahead/behind counts
        let output = Command::new("git")
            .args(["rev-list", "--left-right", "--count", "@{upstream}...HEAD"])
            .current_dir(path)
            .output()
            .map_err(|e| WorktreeError::GitCommand {
                command: "git rev-list".to_string(),
                message: e.to_string(),
            })?;

        if !output.status.success() {
            return Ok((0, 0));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let parts: Vec<&str> = stdout.trim().split('\t').collect();

        if parts.len() == 2 {
            let behind = parts[0].parse().unwrap_or(0);
            let ahead = parts[1].parse().unwrap_or(0);
            Ok((ahead, behind))
        } else {
            Ok((0, 0))
        }
    }

    /// Parses the output of `git worktree list --porcelain`.
    fn parse_worktree_list(&self, output: &str) -> Result<Vec<WorktreeInfo>, WorktreeError> {
        let mut worktrees = Vec::new();
        let mut current_path: Option<PathBuf> = None;
        let mut current_branch: Option<String> = None;
        let mut is_locked = false;
        let mut is_prunable = false;

        for line in output.lines() {
            if line.starts_with("worktree ") {
                // If we have a previous worktree, save it
                if let Some(path) = current_path.take() {
                    let branch = current_branch.take().unwrap_or_default();
                    let is_main = path == self.repo_root;
                    let name = self.derive_worktree_name(&path, &branch, is_main);

                    worktrees.push(WorktreeInfo {
                        name,
                        path,
                        branch,
                        is_main,
                        is_locked,
                        is_prunable,
                    });
                    is_locked = false;
                    is_prunable = false;
                }

                // Start new worktree
                current_path = Some(PathBuf::from(line.strip_prefix("worktree ").unwrap()));
            } else if line.starts_with("branch refs/heads/") {
                current_branch = line.strip_prefix("branch refs/heads/").map(String::from);
            } else if line.starts_with("HEAD ") {
                // Detached HEAD - use the commit hash as branch name
                if current_branch.is_none() {
                    current_branch = Some("(detached)".to_string());
                }
            } else if line == "locked" {
                is_locked = true;
            } else if line == "prunable" {
                is_prunable = true;
            }
        }

        // Don't forget the last worktree
        if let Some(path) = current_path.take() {
            let branch = current_branch.take().unwrap_or_default();
            let is_main = path == self.repo_root;
            let name = self.derive_worktree_name(&path, &branch, is_main);

            worktrees.push(WorktreeInfo {
                name,
                path,
                branch,
                is_main,
                is_locked,
                is_prunable,
            });
        }

        Ok(worktrees)
    }

    /// Derives a worktree name from its path and branch.
    fn derive_worktree_name(&self, path: &Path, branch: &str, is_main: bool) -> String {
        if is_main {
            // For the main worktree, use the branch name
            branch.to_string()
        } else {
            // For other worktrees, use the directory name
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string()
        }
    }

    /// Finds the git repository root from the given path.
    ///
    /// Walks up the directory tree until it finds a `.git` directory.
    fn find_git_root(path: &PathBuf) -> Result<PathBuf, WorktreeError> {
        // Use `git rev-parse --show-toplevel` for reliable detection
        let output = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(path)
            .output()
            .map_err(|_| WorktreeError::NotAGitRepository(path.clone()))?;

        if output.status.success() {
            let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Ok(PathBuf::from(root))
        } else {
            Err(WorktreeError::NotAGitRepository(path.clone()))
        }
    }
}

/// Errors that can occur during worktree operations.
#[derive(Debug)]
pub enum WorktreeError {
    /// The specified path is not within a git repository.
    NotAGitRepository(PathBuf),

    /// A worktree with the specified name already exists.
    WorktreeExists(String),

    /// The specified worktree was not found.
    WorktreeNotFound(String),

    /// A git command failed.
    GitCommand {
        /// The command that failed.
        command: String,
        /// Error message from git.
        message: String,
    },

    /// An I/O error occurred.
    Io(std::io::Error),
}

impl fmt::Display for WorktreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAGitRepository(path) => {
                write!(f, "not a git repository: {}", path.display())
            }
            Self::WorktreeExists(name) => {
                write!(f, "worktree already exists: {}", name)
            }
            Self::WorktreeNotFound(name) => {
                write!(f, "worktree not found: {}", name)
            }
            Self::GitCommand { command, message } => {
                write!(f, "git command '{}' failed: {}", command, message)
            }
            Self::Io(err) => {
                write!(f, "I/O error: {}", err)
            }
        }
    }
}

impl std::error::Error for WorktreeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for WorktreeError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = WorktreeConfig::default();
        assert_eq!(config.worktree_dir, ".worktrees");
        assert_eq!(config.branch_prefix, "wt/");
        assert!(!config.auto_cleanup);
    }

    #[test]
    fn test_error_display() {
        let err = WorktreeError::NotAGitRepository(PathBuf::from("/test"));
        assert!(err.to_string().contains("/test"));

        let err = WorktreeError::WorktreeExists("feature".to_string());
        assert!(err.to_string().contains("feature"));
    }
}
