//! Experiment mode for isolated development with easy accept/reject.
//!
//! This module provides a workflow for trying risky changes in isolation.
//! Experiments create dedicated worktrees where changes can be safely made,
//! then either merged back (accepted) or discarded (rejected).
//!
//! # Library-Only
//!
//! This module provides programmatic API for experiment-based development workflows.
//! It is tested and stable but not yet exposed through the CLI. Planned for CLI
//! integration in a future phase.
//!
//! Use cases:
//! - Automated code exploration (try changes, evaluate, accept or reject)
//! - Safe refactoring workflows with built-in rollback
//! - Parallel AI development with isolated workspaces
//!
//! # Example
//!
//! ```no_run
//! use patina::worktree::{Experiment, WorktreeManager};
//! use std::path::PathBuf;
//!
//! let manager = WorktreeManager::new(PathBuf::from(".")).unwrap();
//! let experiment = Experiment::start(&manager, "risky-refactor").unwrap();
//!
//! // Make changes in experiment.worktree_path()...
//!
//! // If changes are good, merge them back
//! let result = experiment.accept().unwrap();
//! println!("Merged {} commits", result.commits_merged);
//! ```

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{WorktreeError, WorktreeManager};

/// Filename for experiment metadata.
const EXPERIMENT_METADATA_FILE: &str = ".experiment-state";

/// Configuration for experiment mode.
#[derive(Debug, Clone)]
pub struct ExperimentConfig {
    /// Prefix for experiment branch names.
    ///
    /// Default: `exp/`
    pub branch_prefix: String,

    /// Directory for experiment worktrees, relative to repo root.
    ///
    /// Default: `.experiments`
    pub experiment_dir: String,

    /// Whether to automatically clean up worktree on reject.
    ///
    /// Default: `false`
    pub auto_cleanup_on_reject: bool,
}

impl Default for ExperimentConfig {
    fn default() -> Self {
        Self {
            branch_prefix: "exp/".to_string(),
            experiment_dir: ".experiments".to_string(),
            auto_cleanup_on_reject: false,
        }
    }
}

/// State of an experiment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExperimentState {
    /// Experiment is active and changes can be made.
    Active,

    /// Experiment is paused but can be resumed.
    Paused,

    /// Experiment was accepted and changes were merged.
    Accepted,

    /// Experiment was rejected and changes were discarded.
    Rejected,
}

impl ExperimentState {
    /// Returns `true` if this is a terminal state (accepted or rejected).
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Accepted | Self::Rejected)
    }
}

impl fmt::Display for ExperimentState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Active => write!(f, "active"),
            Self::Paused => write!(f, "paused"),
            Self::Accepted => write!(f, "accepted"),
            Self::Rejected => write!(f, "rejected"),
        }
    }
}

/// Result of completing an experiment (accept or reject).
#[derive(Debug)]
pub struct ExperimentResult {
    /// Whether the operation succeeded.
    pub success: bool,

    /// Final state of the experiment.
    pub final_state: ExperimentState,

    /// Number of commits that were merged (for accept).
    pub commits_merged: usize,
}

/// Errors that can occur during experiment operations.
#[derive(Debug)]
pub enum ExperimentError {
    /// An experiment with this name already exists.
    ExperimentExists(String),

    /// The specified experiment was not found.
    ExperimentNotFound(String),

    /// Invalid state transition attempted.
    InvalidState {
        /// Current state of the experiment.
        current: ExperimentState,
        /// Expected state for the operation.
        expected: ExperimentState,
    },

    /// Cannot accept experiment with uncommitted changes.
    UncommittedChanges,

    /// Error from worktree operations.
    Worktree(WorktreeError),

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

impl fmt::Display for ExperimentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExperimentExists(name) => write!(f, "experiment already exists: {}", name),
            Self::ExperimentNotFound(name) => write!(f, "experiment not found: {}", name),
            Self::InvalidState { current, expected } => write!(
                f,
                "invalid state: experiment is {} but expected {}",
                current, expected
            ),
            Self::UncommittedChanges => {
                write!(f, "cannot accept experiment with uncommitted changes")
            }
            Self::Worktree(err) => write!(f, "worktree error: {}", err),
            Self::GitCommand { command, message } => {
                write!(f, "git command '{}' failed: {}", command, message)
            }
            Self::Io(err) => write!(f, "I/O error: {}", err),
        }
    }
}

impl std::error::Error for ExperimentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Worktree(err) => Some(err),
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<WorktreeError> for ExperimentError {
    fn from(err: WorktreeError) -> Self {
        Self::Worktree(err)
    }
}

impl From<std::io::Error> for ExperimentError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// An experiment for isolated development.
///
/// Experiments provide a safe way to try risky changes. Each experiment
/// creates a dedicated worktree where changes can be made without affecting
/// the main codebase. When done, changes can be either:
///
/// - **Accepted**: Merged back to the original branch
/// - **Rejected**: Discarded completely
///
/// Experiments can also be **paused** and **resumed** later.
#[derive(Debug)]
pub struct Experiment {
    /// Name of the experiment.
    name: String,

    /// Optional description.
    description: Option<String>,

    /// Path to the experiment worktree.
    worktree_path: PathBuf,

    /// Branch name in the experiment worktree.
    experiment_branch: String,

    /// Original branch name (to merge back to on accept).
    original_branch: String,

    /// Path to the main repository root.
    repo_root: PathBuf,

    /// Current state of the experiment.
    state: ExperimentState,
}

impl Experiment {
    /// Starts a new experiment with the given name.
    ///
    /// Creates an isolated worktree for the experiment based on the current branch.
    ///
    /// # Errors
    ///
    /// - `ExperimentError::ExperimentExists` if an experiment with this name exists.
    /// - `ExperimentError::Worktree` if worktree creation fails.
    pub fn start(manager: &WorktreeManager, name: &str) -> Result<Self, ExperimentError> {
        Self::start_internal(manager, name, None, ExperimentConfig::default())
    }

    /// Starts a new experiment with a description.
    ///
    /// # Errors
    ///
    /// Same as [`start`](Self::start).
    pub fn start_with_description(
        manager: &WorktreeManager,
        name: &str,
        description: &str,
    ) -> Result<Self, ExperimentError> {
        Self::start_internal(
            manager,
            name,
            Some(description.to_string()),
            ExperimentConfig::default(),
        )
    }

    /// Internal implementation for starting experiments.
    fn start_internal(
        manager: &WorktreeManager,
        name: &str,
        description: Option<String>,
        config: ExperimentConfig,
    ) -> Result<Self, ExperimentError> {
        // Check if experiment already exists
        let existing = Self::list(manager)?;
        if existing.iter().any(|e| e.name == name) {
            return Err(ExperimentError::ExperimentExists(name.to_string()));
        }

        // Get current branch name
        let original_branch = Self::get_current_branch(manager.repo_root())?;

        // Build experiment paths
        let experiment_dir = manager.repo_root().join(&config.experiment_dir);
        let worktree_path = experiment_dir.join(name);
        let experiment_branch = format!("{}{}", config.branch_prefix, name);

        // Create experiment directory if needed
        std::fs::create_dir_all(&experiment_dir)?;

        // Create the worktree with a new branch
        let output = Command::new("git")
            .args([
                "worktree",
                "add",
                worktree_path.to_str().unwrap_or_default(),
                "-b",
                &experiment_branch,
            ])
            .current_dir(manager.repo_root())
            .output()
            .map_err(|e| ExperimentError::GitCommand {
                command: "git worktree add".to_string(),
                message: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(ExperimentError::GitCommand {
                command: "git worktree add".to_string(),
                message: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        let experiment = Self {
            name: name.to_string(),
            description,
            worktree_path,
            experiment_branch,
            original_branch,
            repo_root: manager.repo_root().clone(),
            state: ExperimentState::Active,
        };

        // Save initial state
        experiment.save_state()?;

        Ok(experiment)
    }

    /// Returns the name of the experiment.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the description, if any.
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Returns the path to the experiment worktree.
    #[must_use]
    pub fn worktree_path(&self) -> &Path {
        &self.worktree_path
    }

    /// Returns the experiment branch name.
    #[must_use]
    pub fn experiment_branch(&self) -> &str {
        &self.experiment_branch
    }

    /// Returns the original branch name.
    #[must_use]
    pub fn original_branch(&self) -> &str {
        &self.original_branch
    }

    /// Returns the current state of the experiment.
    #[must_use]
    pub fn state(&self) -> ExperimentState {
        self.state
    }

    /// Accepts the experiment, merging changes back to the original branch.
    ///
    /// This:
    /// 1. Checks for uncommitted changes (fails if any)
    /// 2. Switches to the original branch in the main repo
    /// 3. Merges the experiment branch
    /// 4. Removes the experiment worktree
    ///
    /// # Errors
    ///
    /// - `ExperimentError::UncommittedChanges` if there are uncommitted changes.
    /// - `ExperimentError::InvalidState` if experiment is not active.
    pub fn accept(mut self) -> Result<ExperimentResult, ExperimentError> {
        if self.state != ExperimentState::Active {
            return Err(ExperimentError::InvalidState {
                current: self.state,
                expected: ExperimentState::Active,
            });
        }

        // Check for uncommitted changes
        if self.has_uncommitted_changes()? {
            return Err(ExperimentError::UncommittedChanges);
        }

        // Count commits to merge
        let commits_merged = self.count_commits_to_merge()?;

        // Switch to original branch in main repo
        let output = Command::new("git")
            .args(["checkout", &self.original_branch])
            .current_dir(&self.repo_root)
            .output()
            .map_err(|e| ExperimentError::GitCommand {
                command: "git checkout".to_string(),
                message: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(ExperimentError::GitCommand {
                command: "git checkout".to_string(),
                message: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        // Merge the experiment branch
        let output = Command::new("git")
            .args(["merge", &self.experiment_branch])
            .current_dir(&self.repo_root)
            .output()
            .map_err(|e| ExperimentError::GitCommand {
                command: "git merge".to_string(),
                message: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(ExperimentError::GitCommand {
                command: "git merge".to_string(),
                message: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        // Remove the worktree
        self.remove_worktree()?;

        // Delete the experiment branch
        let _ = Command::new("git")
            .args(["branch", "-d", &self.experiment_branch])
            .current_dir(&self.repo_root)
            .output();

        self.state = ExperimentState::Accepted;

        Ok(ExperimentResult {
            success: true,
            final_state: ExperimentState::Accepted,
            commits_merged,
        })
    }

    /// Rejects the experiment, discarding all changes.
    ///
    /// This removes the experiment worktree and branch without merging.
    /// Fails if there are uncommitted changes - use [`reject_force`](Self::reject_force)
    /// to discard them.
    ///
    /// # Errors
    ///
    /// - `ExperimentError::InvalidState` if experiment is not active or paused.
    pub fn reject(self) -> Result<ExperimentResult, ExperimentError> {
        if !matches!(
            self.state,
            ExperimentState::Active | ExperimentState::Paused
        ) {
            return Err(ExperimentError::InvalidState {
                current: self.state,
                expected: ExperimentState::Active,
            });
        }

        self.reject_internal(false)
    }

    /// Forcefully rejects the experiment, discarding uncommitted changes.
    ///
    /// # Errors
    ///
    /// - `ExperimentError::InvalidState` if experiment is not active or paused.
    pub fn reject_force(self) -> Result<ExperimentResult, ExperimentError> {
        if !matches!(
            self.state,
            ExperimentState::Active | ExperimentState::Paused
        ) {
            return Err(ExperimentError::InvalidState {
                current: self.state,
                expected: ExperimentState::Active,
            });
        }

        self.reject_internal(true)
    }

    /// Internal reject implementation.
    fn reject_internal(mut self, force: bool) -> Result<ExperimentResult, ExperimentError> {
        // Remove our metadata file first so it doesn't trigger "modified files" warning
        let metadata_path = self.worktree_path.join(EXPERIMENT_METADATA_FILE);
        let _ = std::fs::remove_file(&metadata_path);

        // Force remove the worktree
        let mut args = vec!["worktree", "remove"];
        if force {
            args.push("--force");
        }
        args.push(self.worktree_path.to_str().unwrap_or_default());

        let output = Command::new("git")
            .args(&args)
            .current_dir(&self.repo_root)
            .output()
            .map_err(|e| ExperimentError::GitCommand {
                command: "git worktree remove".to_string(),
                message: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(ExperimentError::GitCommand {
                command: "git worktree remove".to_string(),
                message: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        // Delete the experiment branch
        let _ = Command::new("git")
            .args(["branch", "-D", &self.experiment_branch])
            .current_dir(&self.repo_root)
            .output();

        self.state = ExperimentState::Rejected;

        Ok(ExperimentResult {
            success: true,
            final_state: ExperimentState::Rejected,
            commits_merged: 0,
        })
    }

    /// Pauses the experiment, keeping the worktree for later.
    ///
    /// # Errors
    ///
    /// Returns an error if the experiment is not active.
    pub fn pause(mut self) -> Result<Self, ExperimentError> {
        if self.state != ExperimentState::Active {
            return Err(ExperimentError::InvalidState {
                current: self.state,
                expected: ExperimentState::Active,
            });
        }

        self.state = ExperimentState::Paused;
        self.save_state()?;
        Ok(self)
    }

    /// Resumes a paused experiment.
    ///
    /// # Errors
    ///
    /// Returns an error if the experiment is not paused.
    pub fn resume(mut self) -> Result<Self, ExperimentError> {
        if self.state != ExperimentState::Paused {
            return Err(ExperimentError::InvalidState {
                current: self.state,
                expected: ExperimentState::Paused,
            });
        }

        self.state = ExperimentState::Active;
        self.save_state()?;
        Ok(self)
    }

    /// Saves the experiment state to a metadata file.
    fn save_state(&self) -> Result<(), ExperimentError> {
        let metadata_path = self.worktree_path.join(EXPERIMENT_METADATA_FILE);
        let content = format!(
            "state={}\noriginal_branch={}\n",
            self.state, self.original_branch
        );
        std::fs::write(&metadata_path, content)?;
        Ok(())
    }

    /// Loads the experiment state from a metadata file.
    fn load_state(worktree_path: &Path) -> ExperimentState {
        let metadata_path = worktree_path.join(EXPERIMENT_METADATA_FILE);
        if let Ok(content) = std::fs::read_to_string(&metadata_path) {
            for line in content.lines() {
                if let Some(state_str) = line.strip_prefix("state=") {
                    return match state_str {
                        "paused" => ExperimentState::Paused,
                        "active" => ExperimentState::Active,
                        _ => ExperimentState::Active,
                    };
                }
            }
        }
        ExperimentState::Active
    }

    /// Loads the original branch from a metadata file.
    fn load_original_branch(worktree_path: &Path) -> Option<String> {
        let metadata_path = worktree_path.join(EXPERIMENT_METADATA_FILE);
        if let Ok(content) = std::fs::read_to_string(&metadata_path) {
            for line in content.lines() {
                if let Some(branch) = line.strip_prefix("original_branch=") {
                    return Some(branch.to_string());
                }
            }
        }
        None
    }

    /// Lists all experiments in the repository.
    ///
    /// # Errors
    ///
    /// Returns an error if worktree listing fails.
    pub fn list(manager: &WorktreeManager) -> Result<Vec<Self>, ExperimentError> {
        let config = ExperimentConfig::default();
        let experiment_dir = manager.repo_root().join(&config.experiment_dir);

        if !experiment_dir.exists() {
            return Ok(Vec::new());
        }

        let worktrees = manager.list()?;
        let mut experiments = Vec::new();

        for worktree in worktrees {
            // Check if this worktree is an experiment (in experiment dir)
            if let Ok(relative) = worktree.path.strip_prefix(&experiment_dir) {
                if let Some(name) = relative.to_str() {
                    // Only include direct children, not nested paths
                    if !name.contains(std::path::MAIN_SEPARATOR) && !name.is_empty() {
                        // Load state from metadata file
                        let state = Self::load_state(&worktree.path);

                        // Load original branch from metadata, or fall back to current branch
                        let original_branch = Self::load_original_branch(&worktree.path)
                            .unwrap_or_else(|| {
                                Self::get_original_branch_for_experiment(
                                    manager.repo_root(),
                                    &worktree.branch,
                                )
                                .unwrap_or_default()
                            });

                        experiments.push(Self {
                            name: name.to_string(),
                            description: None,
                            worktree_path: worktree.path.clone(),
                            experiment_branch: worktree.branch.clone(),
                            original_branch,
                            repo_root: manager.repo_root().clone(),
                            state,
                        });
                    }
                }
            }
        }

        Ok(experiments)
    }

    /// Lists only active experiments.
    ///
    /// # Errors
    ///
    /// Returns an error if listing fails.
    pub fn list_active(manager: &WorktreeManager) -> Result<Vec<Self>, ExperimentError> {
        let all = Self::list(manager)?;
        Ok(all
            .into_iter()
            .filter(|e| e.state == ExperimentState::Active)
            .collect())
    }

    /// Checks if the experiment has uncommitted changes.
    ///
    /// Excludes the experiment metadata file from the check.
    fn has_uncommitted_changes(&self) -> Result<bool, ExperimentError> {
        let output = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&self.worktree_path)
            .output()
            .map_err(|e| ExperimentError::GitCommand {
                command: "git status".to_string(),
                message: e.to_string(),
            })?;

        // Filter out our metadata file from the status
        let status = String::from_utf8_lossy(&output.stdout);
        let has_changes = status
            .lines()
            .any(|line| !line.ends_with(EXPERIMENT_METADATA_FILE));

        Ok(has_changes)
    }

    /// Counts commits to merge from experiment branch.
    fn count_commits_to_merge(&self) -> Result<usize, ExperimentError> {
        let range = format!("{}..{}", self.original_branch, self.experiment_branch);
        let output = Command::new("git")
            .args(["rev-list", "--count", &range])
            .current_dir(&self.repo_root)
            .output()
            .map_err(|e| ExperimentError::GitCommand {
                command: "git rev-list".to_string(),
                message: e.to_string(),
            })?;

        let count_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(count_str.parse().unwrap_or(0))
    }

    /// Removes the experiment worktree.
    fn remove_worktree(&self) -> Result<(), ExperimentError> {
        // Remove our metadata file first so it doesn't trigger "modified files" warning
        let metadata_path = self.worktree_path.join(EXPERIMENT_METADATA_FILE);
        let _ = std::fs::remove_file(&metadata_path);

        let output = Command::new("git")
            .args([
                "worktree",
                "remove",
                self.worktree_path.to_str().unwrap_or_default(),
            ])
            .current_dir(&self.repo_root)
            .output()
            .map_err(|e| ExperimentError::GitCommand {
                command: "git worktree remove".to_string(),
                message: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(ExperimentError::GitCommand {
                command: "git worktree remove".to_string(),
                message: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        Ok(())
    }

    /// Gets the current branch name.
    fn get_current_branch(repo_root: &Path) -> Result<String, ExperimentError> {
        let output = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(repo_root)
            .output()
            .map_err(|e| ExperimentError::GitCommand {
                command: "git rev-parse".to_string(),
                message: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(ExperimentError::GitCommand {
                command: "git rev-parse".to_string(),
                message: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Gets the original branch for an experiment by looking at branch history.
    fn get_original_branch_for_experiment(
        repo_root: &Path,
        _experiment_branch: &str,
    ) -> Result<String, ExperimentError> {
        // For now, default to the current branch in the main worktree
        // A more sophisticated implementation would store this in experiment metadata
        Self::get_current_branch(repo_root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_experiment_config_defaults() {
        let config = ExperimentConfig::default();
        assert_eq!(config.branch_prefix, "exp/");
        assert_eq!(config.experiment_dir, ".experiments");
        assert!(!config.auto_cleanup_on_reject);
    }

    #[test]
    fn test_experiment_state_display() {
        assert_eq!(format!("{}", ExperimentState::Active), "active");
        assert_eq!(format!("{}", ExperimentState::Paused), "paused");
        assert_eq!(format!("{}", ExperimentState::Accepted), "accepted");
        assert_eq!(format!("{}", ExperimentState::Rejected), "rejected");
    }

    #[test]
    fn test_experiment_state_is_terminal() {
        assert!(!ExperimentState::Active.is_terminal());
        assert!(!ExperimentState::Paused.is_terminal());
        assert!(ExperimentState::Accepted.is_terminal());
        assert!(ExperimentState::Rejected.is_terminal());
    }

    #[test]
    fn test_experiment_error_display() {
        let err = ExperimentError::ExperimentExists("test".to_string());
        assert!(err.to_string().contains("test"));

        let err = ExperimentError::InvalidState {
            current: ExperimentState::Rejected,
            expected: ExperimentState::Active,
        };
        assert!(err.to_string().contains("rejected"));
        assert!(err.to_string().contains("active"));
    }
}
