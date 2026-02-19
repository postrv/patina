//! Worktree-based agent lifecycle management.
//!
//! This module provides [`WorktreeAgentManager`] for spawning agents in isolated
//! git worktrees, tracking their lifecycle, and cleaning up when done.
//!
//! Each agent gets its own worktree with a dedicated branch, enabling parallel
//! development without file conflicts.
//!
//! # Example
//!
//! ```no_run
//! use patina::agents::worktree_agent::WorktreeAgentManager;
//! use std::path::PathBuf;
//!
//! let mut manager = WorktreeAgentManager::new(PathBuf::from(".")).unwrap();
//! let handle = manager.spawn("feature-agent", "Implement login form").unwrap();
//! assert_eq!(handle.name(), "feature-agent");
//! ```

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::worktree::{WorktreeError, WorktreeManager};

/// Maximum number of concurrent worktree agents allowed by default.
const DEFAULT_MAX_CONCURRENT: usize = 4;

/// Prefix for agent worktree branch names.
const AGENT_BRANCH_PREFIX: &str = "agent/";

/// Directory for agent worktrees, relative to repo root.
const AGENT_WORKTREE_DIR: &str = ".agent-worktrees";

/// Lifecycle status of a worktree agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeAgentStatus {
    /// Agent is actively running in its worktree.
    Running,

    /// Agent completed successfully.
    Completed,

    /// Agent failed with an error message.
    Failed(String),

    /// Agent was stopped by user request.
    Stopped,
}

impl WorktreeAgentStatus {
    /// Returns `true` if this is a terminal state (completed, failed, or stopped).
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Running)
    }
}

impl fmt::Display for WorktreeAgentStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed(msg) => write!(f, "failed: {}", msg),
            Self::Stopped => write!(f, "stopped"),
        }
    }
}

/// Handle to an active worktree agent.
///
/// Contains all metadata about a running or completed agent,
/// including its worktree location and current status.
#[derive(Debug)]
pub struct AgentHandle {
    /// Unique name identifying this agent.
    name: String,

    /// Description of the task the agent is performing.
    task: String,

    /// Absolute path to the agent's worktree directory.
    worktree_path: PathBuf,

    /// Branch name checked out in the worktree.
    branch: String,

    /// Current lifecycle status.
    status: WorktreeAgentStatus,

    /// When the agent was spawned.
    spawned_at: Instant,
}

impl AgentHandle {
    /// Returns the agent name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the task description.
    #[must_use]
    pub fn task(&self) -> &str {
        &self.task
    }

    /// Returns the worktree path.
    #[must_use]
    pub fn worktree_path(&self) -> &Path {
        &self.worktree_path
    }

    /// Returns the branch name.
    #[must_use]
    pub fn branch(&self) -> &str {
        &self.branch
    }

    /// Returns the current status.
    #[must_use]
    pub fn status(&self) -> &WorktreeAgentStatus {
        &self.status
    }

    /// Returns the time elapsed since spawning.
    #[must_use]
    pub fn elapsed(&self) -> std::time::Duration {
        self.spawned_at.elapsed()
    }
}

/// Summary information about an agent, suitable for display.
///
/// This is a snapshot of agent state returned by [`WorktreeAgentManager::status`].
#[derive(Debug, Clone)]
pub struct AgentInfo {
    /// Agent name.
    pub name: String,

    /// Task description.
    pub task: String,

    /// Worktree path.
    pub worktree_path: PathBuf,

    /// Branch name.
    pub branch: String,

    /// Current status.
    pub status: WorktreeAgentStatus,
}

/// Errors that can occur during worktree agent operations.
#[derive(Debug)]
pub enum WorktreeAgentError {
    /// An agent with this name already exists.
    AgentExists(String),

    /// The specified agent was not found.
    AgentNotFound(String),

    /// Cannot spawn more agents; concurrent limit reached.
    ConcurrencyLimitReached {
        /// Current number of running agents.
        current: usize,
        /// Maximum allowed.
        max: usize,
    },

    /// Invalid agent name (contains disallowed characters).
    InvalidName {
        /// The invalid name.
        name: String,
        /// Reason it is invalid.
        reason: String,
    },

    /// An error from the underlying worktree manager.
    Worktree(WorktreeError),
}

impl fmt::Display for WorktreeAgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AgentExists(name) => write!(f, "agent already exists: {}", name),
            Self::AgentNotFound(name) => write!(f, "agent not found: {}", name),
            Self::ConcurrencyLimitReached { current, max } => {
                write!(
                    f,
                    "concurrency limit reached: {} running of {} max",
                    current, max
                )
            }
            Self::InvalidName { name, reason } => {
                write!(f, "invalid agent name '{}': {}", name, reason)
            }
            Self::Worktree(err) => write!(f, "worktree error: {}", err),
        }
    }
}

impl std::error::Error for WorktreeAgentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Worktree(err) => Some(err),
            _ => None,
        }
    }
}

impl From<WorktreeError> for WorktreeAgentError {
    fn from(err: WorktreeError) -> Self {
        Self::Worktree(err)
    }
}

/// Manager for agents running in isolated git worktrees.
///
/// Each agent gets its own worktree with a dedicated branch, allowing
/// parallel development without file conflicts. The manager handles:
///
/// - Spawning agents with worktree creation
/// - Tracking agent lifecycle status
/// - Listing and querying agents
/// - Cleaning up worktrees when agents complete
///
/// # Example
///
/// ```no_run
/// use patina::agents::worktree_agent::WorktreeAgentManager;
/// use std::path::PathBuf;
///
/// let mut manager = WorktreeAgentManager::new(PathBuf::from(".")).unwrap();
///
/// // Spawn an agent
/// let handle = manager.spawn("refactor-auth", "Refactor authentication module").unwrap();
/// assert_eq!(handle.name(), "refactor-auth");
///
/// // List agents
/// let agents = manager.list();
/// assert_eq!(agents.len(), 1);
///
/// // Clean up when done
/// manager.cleanup("refactor-auth").unwrap();
/// ```
pub struct WorktreeAgentManager {
    /// Active agents indexed by name.
    agents: HashMap<String, AgentHandle>,

    /// Underlying worktree manager for git operations.
    worktree_manager: WorktreeManager,

    /// Maximum number of concurrently running agents.
    max_concurrent: usize,
}

impl WorktreeAgentManager {
    /// Creates a new `WorktreeAgentManager` for the repository at the given path.
    ///
    /// # Errors
    ///
    /// Returns `WorktreeAgentError::Worktree` if the path is not within a git repository.
    pub fn new(repo_path: impl Into<PathBuf>) -> Result<Self, WorktreeAgentError> {
        let worktree_manager = WorktreeManager::new(repo_path)?;
        Ok(Self {
            agents: HashMap::new(),
            worktree_manager,
            max_concurrent: DEFAULT_MAX_CONCURRENT,
        })
    }

    /// Sets the maximum number of concurrently running agents.
    #[must_use]
    pub fn with_max_concurrent(mut self, max: usize) -> Self {
        self.max_concurrent = max;
        self
    }

    /// Returns the maximum concurrent agent limit.
    #[must_use]
    pub fn max_concurrent(&self) -> usize {
        self.max_concurrent
    }

    /// Returns the number of currently running agents.
    #[must_use]
    pub fn running_count(&self) -> usize {
        self.agents
            .values()
            .filter(|a| a.status == WorktreeAgentStatus::Running)
            .count()
    }

    /// Validates and sanitizes an agent name.
    ///
    /// Valid names must:
    /// - Not be empty
    /// - Contain only alphanumeric characters, hyphens, and underscores
    /// - Not start with a hyphen
    /// - Be at most 64 characters
    ///
    /// # Errors
    ///
    /// Returns `WorktreeAgentError::InvalidName` if the name is invalid.
    fn validate_name(name: &str) -> Result<(), WorktreeAgentError> {
        if name.is_empty() {
            return Err(WorktreeAgentError::InvalidName {
                name: name.to_string(),
                reason: "name cannot be empty".to_string(),
            });
        }

        if name.len() > 64 {
            return Err(WorktreeAgentError::InvalidName {
                name: name.to_string(),
                reason: "name cannot exceed 64 characters".to_string(),
            });
        }

        if name.starts_with('-') {
            return Err(WorktreeAgentError::InvalidName {
                name: name.to_string(),
                reason: "name cannot start with a hyphen".to_string(),
            });
        }

        if !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            return Err(WorktreeAgentError::InvalidName {
                name: name.to_string(),
                reason: "name must contain only alphanumeric characters, hyphens, and underscores"
                    .to_string(),
            });
        }

        Ok(())
    }

    /// Spawns a new agent in an isolated git worktree.
    ///
    /// Creates a new worktree with branch `agent/<name>` and registers
    /// the agent for lifecycle tracking.
    ///
    /// # Errors
    ///
    /// - `WorktreeAgentError::InvalidName` if the name is invalid.
    /// - `WorktreeAgentError::AgentExists` if an agent with this name already exists.
    /// - `WorktreeAgentError::ConcurrencyLimitReached` if at the concurrent agent limit.
    /// - `WorktreeAgentError::Worktree` if worktree creation fails.
    pub fn spawn(&mut self, name: &str, task: &str) -> Result<&AgentHandle, WorktreeAgentError> {
        Self::validate_name(name)?;

        if self.agents.contains_key(name) {
            return Err(WorktreeAgentError::AgentExists(name.to_string()));
        }

        let running = self.running_count();
        if running >= self.max_concurrent {
            return Err(WorktreeAgentError::ConcurrencyLimitReached {
                current: running,
                max: self.max_concurrent,
            });
        }

        // Build worktree path and branch name
        let worktree_path = self
            .worktree_manager
            .repo_root()
            .join(AGENT_WORKTREE_DIR)
            .join(name);
        let branch = format!("{}{}", AGENT_BRANCH_PREFIX, name);

        // Create the worktree directory if needed
        let worktree_dir = self.worktree_manager.repo_root().join(AGENT_WORKTREE_DIR);
        std::fs::create_dir_all(&worktree_dir).map_err(WorktreeError::from)?;

        // Create git worktree with a new branch
        let output = std::process::Command::new("git")
            .args([
                "worktree",
                "add",
                worktree_path.to_str().unwrap_or_default(),
                "-b",
                &branch,
            ])
            .current_dir(self.worktree_manager.repo_root())
            .output()
            .map_err(|e| WorktreeError::GitCommand {
                command: "git worktree add".to_string(),
                message: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(WorktreeError::GitCommand {
                command: "git worktree add".to_string(),
                message: String::from_utf8_lossy(&output.stderr).to_string(),
            }
            .into());
        }

        let handle = AgentHandle {
            name: name.to_string(),
            task: task.to_string(),
            worktree_path,
            branch,
            status: WorktreeAgentStatus::Running,
            spawned_at: Instant::now(),
        };

        self.agents.insert(name.to_string(), handle);
        Ok(self.agents.get(name).expect("just inserted"))
    }

    /// Returns a list of all agents with their current info.
    #[must_use]
    pub fn list(&self) -> Vec<AgentInfo> {
        self.agents
            .values()
            .map(|h| AgentInfo {
                name: h.name.clone(),
                task: h.task.clone(),
                worktree_path: h.worktree_path.clone(),
                branch: h.branch.clone(),
                status: h.status.clone(),
            })
            .collect()
    }

    /// Returns status information for a specific agent.
    ///
    /// # Errors
    ///
    /// Returns `WorktreeAgentError::AgentNotFound` if no agent with this name exists.
    pub fn status(&self, name: &str) -> Result<AgentInfo, WorktreeAgentError> {
        let handle = self
            .agents
            .get(name)
            .ok_or_else(|| WorktreeAgentError::AgentNotFound(name.to_string()))?;

        Ok(AgentInfo {
            name: handle.name.clone(),
            task: handle.task.clone(),
            worktree_path: handle.worktree_path.clone(),
            branch: handle.branch.clone(),
            status: handle.status.clone(),
        })
    }

    /// Marks an agent as completed.
    ///
    /// # Errors
    ///
    /// Returns `WorktreeAgentError::AgentNotFound` if no agent with this name exists.
    pub fn mark_completed(&mut self, name: &str) -> Result<(), WorktreeAgentError> {
        let handle = self
            .agents
            .get_mut(name)
            .ok_or_else(|| WorktreeAgentError::AgentNotFound(name.to_string()))?;

        handle.status = WorktreeAgentStatus::Completed;
        Ok(())
    }

    /// Marks an agent as failed with an error message.
    ///
    /// # Errors
    ///
    /// Returns `WorktreeAgentError::AgentNotFound` if no agent with this name exists.
    pub fn mark_failed(
        &mut self,
        name: &str,
        error: impl Into<String>,
    ) -> Result<(), WorktreeAgentError> {
        let handle = self
            .agents
            .get_mut(name)
            .ok_or_else(|| WorktreeAgentError::AgentNotFound(name.to_string()))?;

        handle.status = WorktreeAgentStatus::Failed(error.into());
        Ok(())
    }

    /// Marks an agent as stopped.
    ///
    /// # Errors
    ///
    /// Returns `WorktreeAgentError::AgentNotFound` if no agent with this name exists.
    pub fn mark_stopped(&mut self, name: &str) -> Result<(), WorktreeAgentError> {
        let handle = self
            .agents
            .get_mut(name)
            .ok_or_else(|| WorktreeAgentError::AgentNotFound(name.to_string()))?;

        handle.status = WorktreeAgentStatus::Stopped;
        Ok(())
    }

    /// Cleans up a single agent by removing its worktree and tracking entry.
    ///
    /// The agent must be in a terminal state (completed, failed, or stopped)
    /// or the worktree will be force-removed.
    ///
    /// # Errors
    ///
    /// - `WorktreeAgentError::AgentNotFound` if no agent with this name exists.
    /// - `WorktreeAgentError::Worktree` if worktree removal fails.
    pub fn cleanup(&mut self, name: &str) -> Result<(), WorktreeAgentError> {
        let handle = self
            .agents
            .get(name)
            .ok_or_else(|| WorktreeAgentError::AgentNotFound(name.to_string()))?;

        let worktree_path = handle.worktree_path.clone();
        let branch = handle.branch.clone();
        let force = !handle.status.is_terminal();

        // Remove worktree
        if worktree_path.exists() {
            let mut args = vec!["worktree", "remove"];
            if force {
                args.push("--force");
            }
            args.push(worktree_path.to_str().unwrap_or_default());

            let output = std::process::Command::new("git")
                .args(&args)
                .current_dir(self.worktree_manager.repo_root())
                .output()
                .map_err(|e| WorktreeError::GitCommand {
                    command: "git worktree remove".to_string(),
                    message: e.to_string(),
                })?;

            if !output.status.success() {
                return Err(WorktreeError::GitCommand {
                    command: "git worktree remove".to_string(),
                    message: String::from_utf8_lossy(&output.stderr).to_string(),
                }
                .into());
            }
        }

        // Delete the branch
        let _ = std::process::Command::new("git")
            .args(["branch", "-D", &branch])
            .current_dir(self.worktree_manager.repo_root())
            .output();

        // Remove from tracking
        self.agents.remove(name);

        Ok(())
    }

    /// Cleans up all agents in terminal states (completed, failed, stopped).
    ///
    /// Returns the names of agents that were cleaned up.
    ///
    /// # Errors
    ///
    /// Returns the first error encountered during cleanup. Agents cleaned up
    /// before the error are not rolled back.
    pub fn cleanup_completed(&mut self) -> Result<Vec<String>, WorktreeAgentError> {
        let terminal_names: Vec<String> = self
            .agents
            .iter()
            .filter(|(_, h)| h.status.is_terminal())
            .map(|(name, _)| name.clone())
            .collect();

        let mut cleaned = Vec::new();
        for name in &terminal_names {
            self.cleanup(name)?;
            cleaned.push(name.clone());
        }

        Ok(cleaned)
    }

    /// Returns a reference to the underlying worktree manager.
    #[must_use]
    pub fn worktree_manager(&self) -> &WorktreeManager {
        &self.worktree_manager
    }
}

impl fmt::Debug for WorktreeAgentManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorktreeAgentManager")
            .field("agent_count", &self.agents.len())
            .field("max_concurrent", &self.max_concurrent)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // WorktreeAgentStatus tests
    // =========================================================================

    #[test]
    fn test_status_running_is_not_terminal() {
        assert!(!WorktreeAgentStatus::Running.is_terminal());
    }

    #[test]
    fn test_status_completed_is_terminal() {
        assert!(WorktreeAgentStatus::Completed.is_terminal());
    }

    #[test]
    fn test_status_failed_is_terminal() {
        assert!(WorktreeAgentStatus::Failed("err".to_string()).is_terminal());
    }

    #[test]
    fn test_status_stopped_is_terminal() {
        assert!(WorktreeAgentStatus::Stopped.is_terminal());
    }

    #[test]
    fn test_status_display() {
        assert_eq!(format!("{}", WorktreeAgentStatus::Running), "running");
        assert_eq!(format!("{}", WorktreeAgentStatus::Completed), "completed");
        assert_eq!(format!("{}", WorktreeAgentStatus::Stopped), "stopped");
        assert!(format!("{}", WorktreeAgentStatus::Failed("timeout".into()))
            .contains("failed: timeout"));
    }

    #[test]
    fn test_status_equality() {
        assert_eq!(WorktreeAgentStatus::Running, WorktreeAgentStatus::Running);
        assert_ne!(WorktreeAgentStatus::Running, WorktreeAgentStatus::Completed);
        assert_eq!(
            WorktreeAgentStatus::Failed("x".into()),
            WorktreeAgentStatus::Failed("x".into())
        );
        assert_ne!(
            WorktreeAgentStatus::Failed("x".into()),
            WorktreeAgentStatus::Failed("y".into())
        );
    }

    // =========================================================================
    // Name validation tests
    // =========================================================================

    #[test]
    fn test_validate_name_valid() {
        assert!(WorktreeAgentManager::validate_name("my-agent").is_ok());
        assert!(WorktreeAgentManager::validate_name("agent_1").is_ok());
        assert!(WorktreeAgentManager::validate_name("AgentFoo").is_ok());
        assert!(WorktreeAgentManager::validate_name("a").is_ok());
    }

    #[test]
    fn test_validate_name_empty() {
        let err = WorktreeAgentManager::validate_name("").unwrap_err();
        assert!(matches!(err, WorktreeAgentError::InvalidName { .. }));
    }

    #[test]
    fn test_validate_name_too_long() {
        let long_name = "a".repeat(65);
        let err = WorktreeAgentManager::validate_name(&long_name).unwrap_err();
        assert!(matches!(err, WorktreeAgentError::InvalidName { .. }));
    }

    #[test]
    fn test_validate_name_starts_with_hyphen() {
        let err = WorktreeAgentManager::validate_name("-agent").unwrap_err();
        assert!(matches!(err, WorktreeAgentError::InvalidName { .. }));
    }

    #[test]
    fn test_validate_name_disallowed_chars() {
        assert!(WorktreeAgentManager::validate_name("my agent").is_err());
        assert!(WorktreeAgentManager::validate_name("my/agent").is_err());
        assert!(WorktreeAgentManager::validate_name("my..agent").is_err());
        assert!(WorktreeAgentManager::validate_name("agent@home").is_err());
    }

    #[test]
    fn test_validate_name_max_length_ok() {
        let name = "a".repeat(64);
        assert!(WorktreeAgentManager::validate_name(&name).is_ok());
    }

    // =========================================================================
    // Error display tests
    // =========================================================================

    #[test]
    fn test_error_display_agent_exists() {
        let err = WorktreeAgentError::AgentExists("foo".into());
        assert!(err.to_string().contains("foo"));
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn test_error_display_agent_not_found() {
        let err = WorktreeAgentError::AgentNotFound("bar".into());
        assert!(err.to_string().contains("bar"));
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn test_error_display_concurrency_limit() {
        let err = WorktreeAgentError::ConcurrencyLimitReached { current: 4, max: 4 };
        assert!(err.to_string().contains("4"));
        assert!(err.to_string().contains("limit"));
    }

    #[test]
    fn test_error_display_invalid_name() {
        let err = WorktreeAgentError::InvalidName {
            name: "bad name".into(),
            reason: "contains spaces".into(),
        };
        assert!(err.to_string().contains("bad name"));
        assert!(err.to_string().contains("contains spaces"));
    }

    // =========================================================================
    // WorktreeAgentManager lifecycle tests (require git repo)
    // =========================================================================

    /// Helper to create a temp git repo for testing.
    fn create_test_repo() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let path = tmp.path().to_path_buf();

        // Initialize a git repo with an initial commit
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(&path)
            .output()
            .expect("git init");

        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(&path)
            .output()
            .expect("git config email");

        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&path)
            .output()
            .expect("git config name");

        // Create a file and commit so we have a HEAD
        std::fs::write(path.join("README.md"), "# Test").expect("write readme");

        std::process::Command::new("git")
            .args(["add", "README.md"])
            .current_dir(&path)
            .output()
            .expect("git add");

        std::process::Command::new("git")
            .args(["commit", "-m", "Initial commit"])
            .current_dir(&path)
            .output()
            .expect("git commit");

        (tmp, path)
    }

    #[test]
    fn test_manager_new_in_git_repo() {
        let (_tmp, path) = create_test_repo();
        let manager = WorktreeAgentManager::new(&path);
        assert!(manager.is_ok());
    }

    #[test]
    fn test_manager_new_not_git_repo() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let result = WorktreeAgentManager::new(tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_manager_default_max_concurrent() {
        let (_tmp, path) = create_test_repo();
        let manager = WorktreeAgentManager::new(&path).unwrap();
        assert_eq!(manager.max_concurrent(), DEFAULT_MAX_CONCURRENT);
    }

    #[test]
    fn test_manager_with_max_concurrent() {
        let (_tmp, path) = create_test_repo();
        let manager = WorktreeAgentManager::new(&path)
            .unwrap()
            .with_max_concurrent(2);
        assert_eq!(manager.max_concurrent(), 2);
    }

    #[test]
    fn test_spawn_creates_worktree() {
        let (_tmp, path) = create_test_repo();
        let mut manager = WorktreeAgentManager::new(&path).unwrap();

        let handle = manager.spawn("test-agent", "Do something").unwrap();
        assert_eq!(handle.name(), "test-agent");
        assert_eq!(handle.task(), "Do something");
        assert_eq!(handle.branch(), "agent/test-agent");
        assert_eq!(*handle.status(), WorktreeAgentStatus::Running);
        assert!(handle.worktree_path().exists());
    }

    #[test]
    fn test_spawn_duplicate_name_errors() {
        let (_tmp, path) = create_test_repo();
        let mut manager = WorktreeAgentManager::new(&path).unwrap();

        manager.spawn("agent-a", "Task A").unwrap();
        let result = manager.spawn("agent-a", "Task B");
        assert!(matches!(
            result,
            Err(WorktreeAgentError::AgentExists(ref n)) if n == "agent-a"
        ));
    }

    #[test]
    fn test_spawn_invalid_name_errors() {
        let (_tmp, path) = create_test_repo();
        let mut manager = WorktreeAgentManager::new(&path).unwrap();

        let result = manager.spawn("bad name", "Task");
        assert!(matches!(
            result,
            Err(WorktreeAgentError::InvalidName { .. })
        ));
    }

    #[test]
    fn test_spawn_concurrency_limit() {
        let (_tmp, path) = create_test_repo();
        let mut manager = WorktreeAgentManager::new(&path)
            .unwrap()
            .with_max_concurrent(2);

        manager.spawn("agent-1", "Task 1").unwrap();
        manager.spawn("agent-2", "Task 2").unwrap();

        let result = manager.spawn("agent-3", "Task 3");
        assert!(matches!(
            result,
            Err(WorktreeAgentError::ConcurrencyLimitReached { current: 2, max: 2 })
        ));
    }

    #[test]
    fn test_list_empty() {
        let (_tmp, path) = create_test_repo();
        let manager = WorktreeAgentManager::new(&path).unwrap();
        assert!(manager.list().is_empty());
    }

    #[test]
    fn test_list_returns_all_agents() {
        let (_tmp, path) = create_test_repo();
        let mut manager = WorktreeAgentManager::new(&path).unwrap();

        manager.spawn("agent-a", "Task A").unwrap();
        manager.spawn("agent-b", "Task B").unwrap();

        let agents = manager.list();
        assert_eq!(agents.len(), 2);

        let names: Vec<&str> = agents.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"agent-a"));
        assert!(names.contains(&"agent-b"));
    }

    #[test]
    fn test_status_existing_agent() {
        let (_tmp, path) = create_test_repo();
        let mut manager = WorktreeAgentManager::new(&path).unwrap();

        manager.spawn("my-agent", "My task").unwrap();
        let info = manager.status("my-agent").unwrap();

        assert_eq!(info.name, "my-agent");
        assert_eq!(info.task, "My task");
        assert_eq!(info.branch, "agent/my-agent");
        assert_eq!(info.status, WorktreeAgentStatus::Running);
    }

    #[test]
    fn test_status_nonexistent_agent() {
        let (_tmp, path) = create_test_repo();
        let manager = WorktreeAgentManager::new(&path).unwrap();

        let result = manager.status("ghost");
        assert!(matches!(
            result,
            Err(WorktreeAgentError::AgentNotFound(ref n)) if n == "ghost"
        ));
    }

    #[test]
    fn test_mark_completed() {
        let (_tmp, path) = create_test_repo();
        let mut manager = WorktreeAgentManager::new(&path).unwrap();

        manager.spawn("agent-x", "Task X").unwrap();
        manager.mark_completed("agent-x").unwrap();

        let info = manager.status("agent-x").unwrap();
        assert_eq!(info.status, WorktreeAgentStatus::Completed);
    }

    #[test]
    fn test_mark_failed() {
        let (_tmp, path) = create_test_repo();
        let mut manager = WorktreeAgentManager::new(&path).unwrap();

        manager.spawn("agent-y", "Task Y").unwrap();
        manager.mark_failed("agent-y", "API timeout").unwrap();

        let info = manager.status("agent-y").unwrap();
        assert_eq!(
            info.status,
            WorktreeAgentStatus::Failed("API timeout".to_string())
        );
    }

    #[test]
    fn test_mark_stopped() {
        let (_tmp, path) = create_test_repo();
        let mut manager = WorktreeAgentManager::new(&path).unwrap();

        manager.spawn("agent-z", "Task Z").unwrap();
        manager.mark_stopped("agent-z").unwrap();

        let info = manager.status("agent-z").unwrap();
        assert_eq!(info.status, WorktreeAgentStatus::Stopped);
    }

    #[test]
    fn test_mark_nonexistent_agent_errors() {
        let (_tmp, path) = create_test_repo();
        let mut manager = WorktreeAgentManager::new(&path).unwrap();

        assert!(manager.mark_completed("nope").is_err());
        assert!(manager.mark_failed("nope", "err").is_err());
        assert!(manager.mark_stopped("nope").is_err());
    }

    #[test]
    fn test_cleanup_removes_agent_and_worktree() {
        let (_tmp, path) = create_test_repo();
        let mut manager = WorktreeAgentManager::new(&path).unwrap();

        manager.spawn("cleanup-me", "Task").unwrap();
        let wt_path = manager.status("cleanup-me").unwrap().worktree_path.clone();
        assert!(wt_path.exists());

        manager.mark_completed("cleanup-me").unwrap();
        manager.cleanup("cleanup-me").unwrap();

        assert!(manager.status("cleanup-me").is_err());
        assert!(!wt_path.exists());
    }

    #[test]
    fn test_cleanup_nonexistent_agent_errors() {
        let (_tmp, path) = create_test_repo();
        let mut manager = WorktreeAgentManager::new(&path).unwrap();

        assert!(matches!(
            manager.cleanup("nope"),
            Err(WorktreeAgentError::AgentNotFound(_))
        ));
    }

    #[test]
    fn test_cleanup_completed_agents() {
        let (_tmp, path) = create_test_repo();
        let mut manager = WorktreeAgentManager::new(&path).unwrap();

        manager.spawn("done-1", "Task 1").unwrap();
        manager.spawn("done-2", "Task 2").unwrap();
        manager.spawn("still-running", "Task 3").unwrap();

        manager.mark_completed("done-1").unwrap();
        manager.mark_failed("done-2", "err").unwrap();

        let cleaned = manager.cleanup_completed().unwrap();
        assert_eq!(cleaned.len(), 2);
        assert!(cleaned.contains(&"done-1".to_string()));
        assert!(cleaned.contains(&"done-2".to_string()));

        // Running agent should still be there
        assert!(manager.status("still-running").is_ok());
        assert_eq!(manager.list().len(), 1);
    }

    #[test]
    fn test_running_count_tracks_correctly() {
        let (_tmp, path) = create_test_repo();
        let mut manager = WorktreeAgentManager::new(&path).unwrap();

        assert_eq!(manager.running_count(), 0);

        manager.spawn("a", "Task A").unwrap();
        assert_eq!(manager.running_count(), 1);

        manager.spawn("b", "Task B").unwrap();
        assert_eq!(manager.running_count(), 2);

        manager.mark_completed("a").unwrap();
        assert_eq!(manager.running_count(), 1);

        manager.mark_stopped("b").unwrap();
        assert_eq!(manager.running_count(), 0);
    }

    #[test]
    fn test_concurrency_limit_freed_after_completion() {
        let (_tmp, path) = create_test_repo();
        let mut manager = WorktreeAgentManager::new(&path)
            .unwrap()
            .with_max_concurrent(1);

        manager.spawn("first", "Task 1").unwrap();
        assert!(manager.spawn("second", "Task 2").is_err());

        manager.mark_completed("first").unwrap();
        // Now we can spawn another
        assert!(manager.spawn("second", "Task 2").is_ok());
    }

    #[test]
    fn test_agent_handle_elapsed() {
        let (_tmp, path) = create_test_repo();
        let mut manager = WorktreeAgentManager::new(&path).unwrap();

        manager.spawn("timer-test", "Task").unwrap();
        let handle = manager.agents.get("timer-test").unwrap();
        // elapsed should be very small but non-negative
        assert!(handle.elapsed().as_secs() < 5);
    }

    #[test]
    fn test_debug_impl() {
        let (_tmp, path) = create_test_repo();
        let manager = WorktreeAgentManager::new(&path).unwrap();
        let debug = format!("{:?}", manager);
        assert!(debug.contains("WorktreeAgentManager"));
        assert!(debug.contains("agent_count"));
    }
}
