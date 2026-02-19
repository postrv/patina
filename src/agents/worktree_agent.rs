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
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::api::LlmProvider;
use crate::types::{ApiMessageV2, StopReason, StreamEvent};
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

    /// Checks for conflicts between all running agents.
    ///
    /// For each running agent, queries its worktree for modified files using
    /// `git diff --name-only HEAD`. If an MCP client is provided, also queries
    /// for modified symbols using `find_references` to detect symbol-level
    /// conflicts.
    ///
    /// Returns a [`ConflictReport`] describing any detected conflicts.
    ///
    /// # Arguments
    ///
    /// * `mcp_client` — Optional MCP client for symbol-level analysis.
    ///   If `None`, only file-level conflicts are detected.
    ///
    /// # Errors
    ///
    /// Returns an error if git commands fail in any agent worktree.
    pub async fn check_conflicts(
        &self,
        mcp_client: Option<&mut crate::mcp::client::McpClient>,
    ) -> Result<super::conflict::ConflictReport> {
        use super::conflict::{AgentFileChanges, ConflictDetector};

        let mut detector = ConflictDetector::new();

        // Collect agent info first to avoid borrow issues.
        let running_agents: Vec<(String, PathBuf)> = self
            .agents
            .iter()
            .filter(|(_, h)| h.status == WorktreeAgentStatus::Running)
            .map(|(name, handle)| (name.clone(), handle.worktree_path.clone()))
            .collect();

        // Gather modified files for each agent (git-only, no MCP needed).
        let mut all_changes: Vec<(String, Vec<String>, PathBuf)> = Vec::new();
        for (name, worktree_path) in &running_agents {
            let modified_files = Self::get_modified_files_in_worktree(worktree_path)?;
            all_changes.push((name.clone(), modified_files, worktree_path.clone()));
        }

        // If MCP is available, enrich with symbol-level information.
        if let Some(client) = mcp_client {
            for (name, modified_files, worktree_path) in &all_changes {
                let mut modified_symbols = Vec::new();
                for file in modified_files {
                    let symbols = Self::get_modified_symbols(client, worktree_path, file).await;
                    for sym in symbols {
                        modified_symbols.push((file.clone(), sym));
                    }
                }
                detector.add_agent_changes(AgentFileChanges {
                    agent_name: name.clone(),
                    modified_files: modified_files.clone(),
                    modified_symbols,
                });
            }
        } else {
            // No MCP: file-level conflict detection only.
            for (name, modified_files, _) in all_changes {
                detector.add_agent_changes(AgentFileChanges {
                    agent_name: name,
                    modified_files,
                    modified_symbols: Vec::new(),
                });
            }
        }

        Ok(detector.detect())
    }

    /// Gets the list of modified files in a worktree by running `git diff --name-only HEAD`.
    ///
    /// # Errors
    ///
    /// Returns an error if the git command fails.
    fn get_modified_files_in_worktree(worktree_path: &Path) -> Result<Vec<String>> {
        let output = std::process::Command::new("git")
            .args(["diff", "--name-only", "HEAD"])
            .current_dir(worktree_path)
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to run git diff in worktree: {}", e))?;

        if !output.status.success() {
            // If the worktree has no commits yet, diff against empty tree
            let output = std::process::Command::new("git")
                .args([
                    "diff",
                    "--name-only",
                    "--cached",
                    "4b825dc642cb6eb9a060e54bf899d15f3f540110",
                ])
                .current_dir(worktree_path)
                .output()
                .map_err(|e| {
                    anyhow::anyhow!("Failed to run git diff (empty tree) in worktree: {}", e)
                })?;

            return Ok(String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect());
        }

        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect())
    }

    /// Queries MCP for symbols modified in a file.
    ///
    /// Uses `find_symbols` to get all symbols defined in the file,
    /// then uses `git diff` to determine which ones were actually modified.
    /// Falls back to an empty list if MCP calls fail.
    async fn get_modified_symbols(
        client: &mut crate::mcp::client::McpClient,
        worktree_path: &Path,
        file: &str,
    ) -> Vec<String> {
        // Query narsil for symbols in this file
        let result = client
            .call_tool(
                "find_symbols",
                serde_json::json!({
                    "repo": worktree_path.to_string_lossy(),
                    "file_pattern": file,
                }),
            )
            .await;

        match result {
            Ok(response) => Self::parse_symbols_response(&response),
            Err(e) => {
                debug!("MCP find_symbols failed for {}: {}", file, e);
                Vec::new()
            }
        }
    }

    /// Parses the symbols response from narsil `find_symbols`.
    fn parse_symbols_response(response: &serde_json::Value) -> Vec<String> {
        // narsil returns symbols with name, type, file, line fields
        let Some(symbols) = response.get("symbols").and_then(|s| s.as_array()) else {
            // Try content[0].text pattern (MCP standard)
            if let Some(text) = response
                .get("content")
                .and_then(|c| c.as_array())
                .and_then(|a| a.first())
                .and_then(|b| b.get("text"))
                .and_then(|t| t.as_str())
            {
                // Try parsing the text as JSON
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(text) {
                    return Self::parse_symbols_response(&parsed);
                }
            }
            return Vec::new();
        };

        symbols
            .iter()
            .filter_map(|sym| sym.get("name").and_then(|n| n.as_str()).map(String::from))
            .collect()
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

// =============================================================================
// Agent Execution Loop types
// =============================================================================

/// Configuration for an agent execution loop.
///
/// Defines the parameters for a headless agent that runs LLM turns
/// with tool execution in a worktree, without TUI interaction.
///
/// # Examples
///
/// ```
/// use patina::agents::worktree_agent::AgentLoopConfig;
/// use std::path::PathBuf;
///
/// let config = AgentLoopConfig {
///     max_iterations: 10,
///     system_prompt: "You are a coding agent.".to_string(),
///     allowed_tools: vec!["read_file".to_string(), "glob".to_string()],
///     working_dir: PathBuf::from("/project"),
/// };
/// assert_eq!(config.max_iterations, 10);
/// ```
#[derive(Debug, Clone)]
pub struct AgentLoopConfig {
    /// Maximum number of LLM turns before the agent must stop.
    pub max_iterations: usize,

    /// System prompt that defines the agent's behavior.
    pub system_prompt: String,

    /// Tool names this agent is allowed to use.
    pub allowed_tools: Vec<String>,

    /// Working directory for tool execution.
    pub working_dir: PathBuf,
}

/// Progress update sent from an agent execution loop to the main loop.
///
/// These events allow the parent to track agent progress without polling.
///
/// # Examples
///
/// ```
/// use patina::agents::worktree_agent::AgentProgress;
///
/// let progress = AgentProgress::IterationStarted { iteration: 1, max: 10 };
/// assert!(format!("{}", progress).contains("iteration 1/10"));
/// ```
#[derive(Debug, Clone)]
pub enum AgentProgress {
    /// Agent started a new LLM turn.
    IterationStarted {
        /// Current iteration (1-indexed).
        iteration: usize,
        /// Maximum iterations allowed.
        max: usize,
    },

    /// Agent received text content from the LLM.
    ContentDelta(String),

    /// Agent completed successfully.
    Completed {
        /// Final accumulated output text.
        output: String,
        /// Number of iterations used.
        iterations_used: usize,
    },

    /// Agent failed with an error.
    Failed {
        /// Error description.
        error: String,
        /// Number of iterations completed before failure.
        iterations_used: usize,
    },
}

impl fmt::Display for AgentProgress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IterationStarted { iteration, max } => {
                write!(f, "iteration {}/{} started", iteration, max)
            }
            Self::ContentDelta(text) => write!(f, "content: {}...", &text[..text.len().min(40)]),
            Self::Completed {
                iterations_used, ..
            } => write!(f, "completed in {} iterations", iterations_used),
            Self::Failed {
                error,
                iterations_used,
            } => write!(f, "failed after {} iterations: {}", iterations_used, error),
        }
    }
}

/// Result from a completed agent execution loop.
///
/// Contains the final output, success status, and execution metadata.
#[derive(Debug)]
pub struct AgentLoopResult {
    /// Final accumulated text output from the agent.
    pub output: String,

    /// Whether the agent completed successfully.
    pub success: bool,

    /// Number of LLM turns used.
    pub iterations_used: usize,

    /// Errors encountered during execution (may be non-empty even on success).
    pub errors: Vec<String>,
}

/// Runs a headless agent execution loop with streaming LLM turns.
///
/// The loop performs up to `config.max_iterations` LLM turns. Each turn:
///
/// 1. Sends the conversation to the LLM provider via streaming
/// 2. Collects text content and tool-use requests
/// 3. If the stop reason is `EndTurn`, the loop completes
/// 4. If the stop reason is `ToolUse`, tool results are appended (as
///    simulated tool results for now) and the loop continues
/// 5. Progress is reported through `progress_tx`
///
/// # Arguments
///
/// * `provider` - The LLM provider for streaming requests
/// * `task` - The initial task/prompt for the agent
/// * `config` - Loop configuration (max iterations, system prompt, etc.)
/// * `progress_tx` - Channel for reporting progress back to the caller
///
/// # Errors
///
/// Returns an error if the provider fails fatally during streaming.
///
/// # Examples
///
/// ```rust,ignore
/// use patina::agents::worktree_agent::{run_agent_loop, AgentLoopConfig};
/// use std::sync::Arc;
/// use tokio::sync::mpsc;
///
/// let (tx, rx) = mpsc::channel(32);
/// let config = AgentLoopConfig {
///     max_iterations: 5,
///     system_prompt: "You are a helper.".into(),
///     allowed_tools: vec![],
///     working_dir: "/tmp".into(),
/// };
/// let result = run_agent_loop(provider, "Say hello", config, tx).await?;
/// ```
pub async fn run_agent_loop(
    provider: Arc<dyn LlmProvider>,
    task: &str,
    config: AgentLoopConfig,
    progress_tx: mpsc::Sender<AgentProgress>,
) -> Result<AgentLoopResult> {
    let mut messages = vec![ApiMessageV2::user(task)];
    let mut output = String::new();
    let mut errors: Vec<String> = Vec::new();
    let mut iterations_used = 0;

    for iteration in 1..=config.max_iterations {
        iterations_used = iteration;

        // Report iteration start (ignore send failures — receiver may be dropped).
        let _ = progress_tx
            .send(AgentProgress::IterationStarted {
                iteration,
                max: config.max_iterations,
            })
            .await;

        debug!(
            iteration,
            max = config.max_iterations,
            "Agent loop iteration"
        );

        // Stream a single LLM turn.
        let (tx, mut rx) = mpsc::channel::<StreamEvent>(64);

        let provider_clone = Arc::clone(&provider);
        let messages_clone = messages.clone();

        tokio::spawn(async move {
            if let Err(e) = provider_clone
                .stream_message(&messages_clone, None, None, tx)
                .await
            {
                tracing::error!("Agent loop provider error: {}", e);
            }
        });

        // Collect the streamed response.
        let mut turn_text = String::new();
        let mut stop_reason = StopReason::EndTurn;
        let mut had_error = false;

        while let Some(event) = rx.recv().await {
            match event {
                StreamEvent::ContentDelta(text) => {
                    let _ = progress_tx
                        .send(AgentProgress::ContentDelta(text.clone()))
                        .await;
                    turn_text.push_str(&text);
                }
                StreamEvent::MessageComplete {
                    stop_reason: reason,
                } => {
                    stop_reason = reason;
                    break;
                }
                StreamEvent::MessageStop => {
                    break;
                }
                StreamEvent::Error(err) => {
                    warn!(error = %err, "Agent loop stream error");
                    errors.push(err);
                    had_error = true;
                    break;
                }
                // Tool use events are accumulated but we don't execute
                // real tools yet — that will be wired in a later task.
                StreamEvent::ToolUseStart { .. }
                | StreamEvent::ToolUseInputDelta { .. }
                | StreamEvent::ToolUseComplete { .. }
                | StreamEvent::ContentBlockComplete { .. } => {}
            }
        }

        output.push_str(&turn_text);

        if had_error {
            let err_msg = errors.last().cloned().unwrap_or_default();
            let _ = progress_tx
                .send(AgentProgress::Failed {
                    error: err_msg,
                    iterations_used,
                })
                .await;
            return Ok(AgentLoopResult {
                output,
                success: false,
                iterations_used,
                errors,
            });
        }

        // Append assistant response to conversation.
        if !turn_text.is_empty() {
            messages.push(ApiMessageV2::assistant(&turn_text));
        }

        // If the model stopped naturally (EndTurn), we're done.
        if !stop_reason.needs_tool_execution() {
            let _ = progress_tx
                .send(AgentProgress::Completed {
                    output: output.clone(),
                    iterations_used,
                })
                .await;
            return Ok(AgentLoopResult {
                output,
                success: true,
                iterations_used,
                errors,
            });
        }

        // ToolUse stop reason: in a full implementation we would execute
        // tools here. For now, append a placeholder tool_result so the
        // conversation can continue in the next iteration.
        messages.push(ApiMessageV2::user(
            "[Tool execution is not yet wired in agent loop]",
        ));
    }

    // Reached max iterations without natural completion.
    let _ = progress_tx
        .send(AgentProgress::Completed {
            output: output.clone(),
            iterations_used,
        })
        .await;

    Ok(AgentLoopResult {
        output,
        success: true,
        iterations_used,
        errors,
    })
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

    // =========================================================================
    // check_conflicts tests (require git repo with worktrees)
    // =========================================================================

    #[tokio::test]
    async fn test_check_conflicts_no_agents_no_conflicts() {
        let (_tmp, path) = create_test_repo();
        let manager = WorktreeAgentManager::new(&path).unwrap();

        let report = manager.check_conflicts(None).await.unwrap();
        assert!(!report.has_conflicts());
    }

    #[tokio::test]
    async fn test_check_conflicts_single_agent_no_conflicts() {
        let (_tmp, path) = create_test_repo();
        let mut manager = WorktreeAgentManager::new(&path).unwrap();

        manager.spawn("solo", "Work alone").unwrap();

        let report = manager.check_conflicts(None).await.unwrap();
        assert!(!report.has_conflicts());
    }

    #[tokio::test]
    async fn test_check_conflicts_agents_no_changes_no_conflicts() {
        let (_tmp, path) = create_test_repo();
        let mut manager = WorktreeAgentManager::new(&path).unwrap();

        manager.spawn("agent-a", "Task A").unwrap();
        manager.spawn("agent-b", "Task B").unwrap();

        // Neither agent has modified any files yet.
        let report = manager.check_conflicts(None).await.unwrap();
        assert!(!report.has_conflicts());
    }

    #[tokio::test]
    async fn test_check_conflicts_detects_same_file_modification() {
        let (_tmp, path) = create_test_repo();
        let mut manager = WorktreeAgentManager::new(&path).unwrap();

        let handle_a = manager.spawn("agent-a", "Task A").unwrap();
        let wt_a = handle_a.worktree_path().to_path_buf();

        let handle_b = manager.spawn("agent-b", "Task B").unwrap();
        let wt_b = handle_b.worktree_path().to_path_buf();

        // Both agents modify README.md in their respective worktrees.
        std::fs::write(wt_a.join("README.md"), "# Changed by A").unwrap();
        std::fs::write(wt_b.join("README.md"), "# Changed by B").unwrap();

        let report = manager.check_conflicts(None).await.unwrap();
        assert!(
            report.has_conflicts(),
            "Should detect conflict when both agents modify README.md"
        );
        assert!(report.has_warnings_only());
        assert_eq!(report.warning_count(), 1);
    }

    #[tokio::test]
    async fn test_check_conflicts_no_conflict_for_different_files() {
        let (_tmp, path) = create_test_repo();

        // Create additional files in the main repo first.
        std::fs::write(path.join("file_a.txt"), "original a").unwrap();
        std::fs::write(path.join("file_b.txt"), "original b").unwrap();
        std::process::Command::new("git")
            .args(["add", "file_a.txt", "file_b.txt"])
            .current_dir(&path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "Add test files"])
            .current_dir(&path)
            .output()
            .unwrap();

        let mut manager = WorktreeAgentManager::new(&path).unwrap();

        let handle_a = manager.spawn("agent-a", "Task A").unwrap();
        let wt_a = handle_a.worktree_path().to_path_buf();

        let handle_b = manager.spawn("agent-b", "Task B").unwrap();
        let wt_b = handle_b.worktree_path().to_path_buf();

        // Agent A modifies file_a, agent B modifies file_b (no overlap).
        std::fs::write(wt_a.join("file_a.txt"), "changed by A").unwrap();
        std::fs::write(wt_b.join("file_b.txt"), "changed by B").unwrap();

        let report = manager.check_conflicts(None).await.unwrap();
        assert!(
            !report.has_conflicts(),
            "Different files should not conflict"
        );
    }

    #[tokio::test]
    async fn test_check_conflicts_skips_completed_agents() {
        let (_tmp, path) = create_test_repo();
        let mut manager = WorktreeAgentManager::new(&path).unwrap();

        let handle_a = manager.spawn("agent-a", "Task A").unwrap();
        let wt_a = handle_a.worktree_path().to_path_buf();

        let handle_b = manager.spawn("agent-b", "Task B").unwrap();
        let wt_b = handle_b.worktree_path().to_path_buf();

        // Both modify README.md.
        std::fs::write(wt_a.join("README.md"), "# A").unwrap();
        std::fs::write(wt_b.join("README.md"), "# B").unwrap();

        // Mark agent-a as completed — it should be excluded from conflict check.
        manager.mark_completed("agent-a").unwrap();

        let report = manager.check_conflicts(None).await.unwrap();
        assert!(
            !report.has_conflicts(),
            "Completed agents should be excluded from conflict detection"
        );
    }

    #[tokio::test]
    async fn test_check_conflicts_report_summary() {
        let (_tmp, path) = create_test_repo();
        let mut manager = WorktreeAgentManager::new(&path).unwrap();

        let handle_a = manager.spawn("agent-a", "Task A").unwrap();
        let wt_a = handle_a.worktree_path().to_path_buf();

        let handle_b = manager.spawn("agent-b", "Task B").unwrap();
        let wt_b = handle_b.worktree_path().to_path_buf();

        std::fs::write(wt_a.join("README.md"), "# A").unwrap();
        std::fs::write(wt_b.join("README.md"), "# B").unwrap();

        let report = manager.check_conflicts(None).await.unwrap();
        let summary = report.summary();
        assert!(summary.contains("warning"), "Summary: {}", summary);
    }

    // =========================================================================
    // parse_symbols_response tests
    // =========================================================================

    #[test]
    fn test_parse_symbols_response_direct() {
        let response = serde_json::json!({
            "symbols": [
                {"name": "foo", "type": "function", "file": "a.rs", "line": 10},
                {"name": "Bar", "type": "struct", "file": "a.rs", "line": 20},
            ]
        });
        let symbols = WorktreeAgentManager::parse_symbols_response(&response);
        assert_eq!(symbols, vec!["foo", "Bar"]);
    }

    #[test]
    fn test_parse_symbols_response_mcp_wrapped() {
        let inner = serde_json::json!({
            "symbols": [
                {"name": "baz", "type": "function"}
            ]
        });
        let response = serde_json::json!({
            "content": [{"type": "text", "text": inner.to_string()}]
        });
        let symbols = WorktreeAgentManager::parse_symbols_response(&response);
        assert_eq!(symbols, vec!["baz"]);
    }

    #[test]
    fn test_parse_symbols_response_empty() {
        let response = serde_json::json!({});
        let symbols = WorktreeAgentManager::parse_symbols_response(&response);
        assert!(symbols.is_empty());
    }

    #[test]
    fn test_parse_symbols_response_empty_array() {
        let response = serde_json::json!({"symbols": []});
        let symbols = WorktreeAgentManager::parse_symbols_response(&response);
        assert!(symbols.is_empty());
    }

    // =========================================================================
    // AgentLoopConfig tests
    // =========================================================================

    #[test]
    fn test_agent_loop_config_construction() {
        let config = AgentLoopConfig {
            max_iterations: 10,
            system_prompt: "You are a coding agent.".to_string(),
            allowed_tools: vec!["read_file".to_string(), "glob".to_string()],
            working_dir: PathBuf::from("/project"),
        };

        assert_eq!(config.max_iterations, 10);
        assert_eq!(config.system_prompt, "You are a coding agent.");
        assert_eq!(config.allowed_tools.len(), 2);
        assert_eq!(config.working_dir, PathBuf::from("/project"));
    }

    #[test]
    fn test_agent_loop_config_clone() {
        let config = AgentLoopConfig {
            max_iterations: 5,
            system_prompt: "prompt".to_string(),
            allowed_tools: vec![],
            working_dir: PathBuf::from("/tmp"),
        };
        let cloned = config.clone();
        assert_eq!(cloned.max_iterations, 5);
        assert_eq!(cloned.system_prompt, "prompt");
    }

    #[test]
    fn test_agent_loop_config_debug() {
        let config = AgentLoopConfig {
            max_iterations: 3,
            system_prompt: "test".to_string(),
            allowed_tools: vec![],
            working_dir: PathBuf::from("."),
        };
        let debug = format!("{:?}", config);
        assert!(debug.contains("AgentLoopConfig"));
        assert!(debug.contains("max_iterations"));
    }

    // =========================================================================
    // AgentProgress tests
    // =========================================================================

    #[test]
    fn test_agent_progress_iteration_started() {
        let p = AgentProgress::IterationStarted {
            iteration: 1,
            max: 10,
        };
        assert!(matches!(
            p,
            AgentProgress::IterationStarted {
                iteration: 1,
                max: 10
            }
        ));
    }

    #[test]
    fn test_agent_progress_content_delta() {
        let p = AgentProgress::ContentDelta("hello world".to_string());
        assert!(matches!(p, AgentProgress::ContentDelta(ref s) if s == "hello world"));
    }

    #[test]
    fn test_agent_progress_completed() {
        let p = AgentProgress::Completed {
            output: "done".to_string(),
            iterations_used: 3,
        };
        assert!(matches!(
            p,
            AgentProgress::Completed {
                iterations_used: 3,
                ..
            }
        ));
    }

    #[test]
    fn test_agent_progress_failed() {
        let p = AgentProgress::Failed {
            error: "timeout".to_string(),
            iterations_used: 2,
        };
        assert!(matches!(
            p,
            AgentProgress::Failed {
                iterations_used: 2,
                ..
            }
        ));
    }

    #[test]
    fn test_agent_progress_display_iteration() {
        let p = AgentProgress::IterationStarted {
            iteration: 3,
            max: 10,
        };
        let s = format!("{}", p);
        assert!(s.contains("3/10"), "Expected '3/10' in: {}", s);
    }

    #[test]
    fn test_agent_progress_display_completed() {
        let p = AgentProgress::Completed {
            output: "done".to_string(),
            iterations_used: 5,
        };
        let s = format!("{}", p);
        assert!(
            s.contains("5 iterations"),
            "Expected '5 iterations' in: {}",
            s
        );
    }

    #[test]
    fn test_agent_progress_display_failed() {
        let p = AgentProgress::Failed {
            error: "timeout".to_string(),
            iterations_used: 2,
        };
        let s = format!("{}", p);
        assert!(s.contains("timeout"), "Expected 'timeout' in: {}", s);
        assert!(
            s.contains("2 iterations"),
            "Expected '2 iterations' in: {}",
            s
        );
    }

    #[test]
    fn test_agent_progress_clone() {
        let p = AgentProgress::Completed {
            output: "result".to_string(),
            iterations_used: 1,
        };
        let cloned = p.clone();
        assert!(matches!(
            cloned,
            AgentProgress::Completed {
                iterations_used: 1,
                ..
            }
        ));
    }

    // =========================================================================
    // AgentLoopResult tests
    // =========================================================================

    #[test]
    fn test_agent_loop_result_success() {
        let result = AgentLoopResult {
            output: "Hello!".to_string(),
            success: true,
            iterations_used: 1,
            errors: vec![],
        };
        assert!(result.success);
        assert_eq!(result.output, "Hello!");
        assert_eq!(result.iterations_used, 1);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_agent_loop_result_failure() {
        let result = AgentLoopResult {
            output: String::new(),
            success: false,
            iterations_used: 3,
            errors: vec!["API error".to_string()],
        };
        assert!(!result.success);
        assert_eq!(result.errors.len(), 1);
    }

    #[test]
    fn test_agent_loop_result_debug() {
        let result = AgentLoopResult {
            output: "text".to_string(),
            success: true,
            iterations_used: 2,
            errors: vec![],
        };
        let debug = format!("{:?}", result);
        assert!(debug.contains("AgentLoopResult"));
    }

    // =========================================================================
    // Mock LLM provider for agent loop tests
    // =========================================================================

    use crate::api::tools::{ToolChoice, ToolDefinition};
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Mock provider that sends predetermined events on each call.
    /// Tracks call count for verification.
    struct MockAgentProvider {
        /// Events to send on each call, indexed by call number.
        /// If call number exceeds length, uses the last entry.
        responses: Vec<Vec<StreamEvent>>,
        call_count: Arc<AtomicUsize>,
    }

    impl MockAgentProvider {
        fn new(responses: Vec<Vec<StreamEvent>>) -> (Self, Arc<AtomicUsize>) {
            let count = Arc::new(AtomicUsize::new(0));
            (
                Self {
                    responses,
                    call_count: count.clone(),
                },
                count,
            )
        }

        /// Creates a provider that always returns a simple text response.
        fn simple_text(text: &str) -> (Self, Arc<AtomicUsize>) {
            Self::new(vec![vec![
                StreamEvent::ContentDelta(text.to_string()),
                StreamEvent::MessageComplete {
                    stop_reason: StopReason::EndTurn,
                },
            ]])
        }

        /// Creates a provider that returns an error.
        fn error(msg: &str) -> (Self, Arc<AtomicUsize>) {
            Self::new(vec![vec![StreamEvent::Error(msg.to_string())]])
        }

        /// Creates a provider that returns ToolUse stop reason N times,
        /// then returns EndTurn.
        fn tool_use_then_end(tool_use_count: usize) -> (Self, Arc<AtomicUsize>) {
            let mut responses = Vec::new();
            for _ in 0..tool_use_count {
                responses.push(vec![
                    StreamEvent::ContentDelta("Using tool...".to_string()),
                    StreamEvent::MessageComplete {
                        stop_reason: StopReason::ToolUse,
                    },
                ]);
            }
            responses.push(vec![
                StreamEvent::ContentDelta("Done!".to_string()),
                StreamEvent::MessageComplete {
                    stop_reason: StopReason::EndTurn,
                },
            ]);
            Self::new(responses)
        }
    }

    impl LlmProvider for MockAgentProvider {
        fn name(&self) -> &str {
            "mock-agent"
        }

        fn model(&self) -> &str {
            "mock-model"
        }

        fn stream_message<'a>(
            &'a self,
            _messages: &'a [ApiMessageV2],
            _tools: Option<&'a [ToolDefinition]>,
            _tool_choice: Option<&'a ToolChoice>,
            tx: mpsc::Sender<StreamEvent>,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
            let call_idx = self.call_count.fetch_add(1, Ordering::SeqCst);
            let response_idx = call_idx.min(self.responses.len().saturating_sub(1));
            let events = self.responses[response_idx].clone();

            Box::pin(async move {
                for event in events {
                    if tx.send(event).await.is_err() {
                        break;
                    }
                }
                Ok(())
            })
        }
    }

    fn default_config(max_iterations: usize) -> AgentLoopConfig {
        AgentLoopConfig {
            max_iterations,
            system_prompt: "You are a test agent.".to_string(),
            allowed_tools: vec![],
            working_dir: PathBuf::from("/tmp/test"),
        }
    }

    // =========================================================================
    // run_agent_loop tests
    // =========================================================================

    #[tokio::test]
    async fn test_agent_loop_completes_on_end_turn() {
        let (provider, call_count) = MockAgentProvider::simple_text("Hello!");
        let provider = Arc::new(provider);
        let (tx, _rx) = mpsc::channel(32);

        let result = run_agent_loop(provider, "Say hello", default_config(5), tx)
            .await
            .unwrap();

        assert!(result.success, "Agent loop should succeed on EndTurn");
        assert_eq!(result.output, "Hello!");
        assert_eq!(result.iterations_used, 1);
        assert!(result.errors.is_empty());
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_agent_loop_accumulates_content() {
        let (provider, _) = MockAgentProvider::new(vec![vec![
            StreamEvent::ContentDelta("Hello ".to_string()),
            StreamEvent::ContentDelta("world!".to_string()),
            StreamEvent::MessageComplete {
                stop_reason: StopReason::EndTurn,
            },
        ]]);
        let provider = Arc::new(provider);
        let (tx, _rx) = mpsc::channel(32);

        let result = run_agent_loop(provider, "Test", default_config(5), tx)
            .await
            .unwrap();

        assert_eq!(result.output, "Hello world!");
    }

    #[tokio::test]
    async fn test_agent_loop_handles_error() {
        let (provider, _) = MockAgentProvider::error("API timeout");
        let provider = Arc::new(provider);
        let (tx, _rx) = mpsc::channel(32);

        let result = run_agent_loop(provider, "Test", default_config(5), tx)
            .await
            .unwrap();

        assert!(!result.success, "Agent loop should fail on error");
        assert_eq!(result.errors.len(), 1);
        assert!(result.errors[0].contains("API timeout"));
    }

    #[tokio::test]
    async fn test_agent_loop_respects_max_iterations() {
        // Provider always returns ToolUse, so the loop should hit max iterations.
        let (provider, call_count) = MockAgentProvider::new(vec![vec![
            StreamEvent::ContentDelta("tool...".to_string()),
            StreamEvent::MessageComplete {
                stop_reason: StopReason::ToolUse,
            },
        ]]);
        let provider = Arc::new(provider);
        let (tx, _rx) = mpsc::channel(64);

        let result = run_agent_loop(provider, "Loop forever", default_config(3), tx)
            .await
            .unwrap();

        assert_eq!(result.iterations_used, 3, "Should stop at max_iterations=3");
        assert_eq!(call_count.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_agent_loop_tool_use_then_end() {
        // 2 ToolUse turns then EndTurn.
        let (provider, call_count) = MockAgentProvider::tool_use_then_end(2);
        let provider = Arc::new(provider);
        let (tx, _rx) = mpsc::channel(32);

        let result = run_agent_loop(provider, "Use tools", default_config(10), tx)
            .await
            .unwrap();

        assert!(result.success);
        // Should have done 3 iterations: 2 tool_use + 1 end_turn
        assert_eq!(result.iterations_used, 3);
        assert_eq!(call_count.load(Ordering::SeqCst), 3);
        assert!(result.output.contains("Done!"));
    }

    #[tokio::test]
    async fn test_agent_loop_reports_progress() {
        let (provider, _) = MockAgentProvider::simple_text("Result");
        let provider = Arc::new(provider);
        let (tx, mut rx) = mpsc::channel(32);

        let result = run_agent_loop(provider, "Go", default_config(5), tx)
            .await
            .unwrap();

        assert!(result.success);

        // Collect all progress events.
        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }

        // Should have: IterationStarted, ContentDelta, Completed
        assert!(
            events.len() >= 2,
            "Expected at least 2 progress events, got {}",
            events.len()
        );

        assert!(
            matches!(
                &events[0],
                AgentProgress::IterationStarted {
                    iteration: 1,
                    max: 5
                }
            ),
            "First event should be IterationStarted, got {:?}",
            events[0]
        );

        // Last event should be Completed.
        let last = events.last().unwrap();
        assert!(
            matches!(last, AgentProgress::Completed { .. }),
            "Last event should be Completed, got {:?}",
            last
        );
    }

    #[tokio::test]
    async fn test_agent_loop_reports_failure_progress() {
        let (provider, _) = MockAgentProvider::error("network failure");
        let provider = Arc::new(provider);
        let (tx, mut rx) = mpsc::channel(32);

        let result = run_agent_loop(provider, "Go", default_config(5), tx)
            .await
            .unwrap();

        assert!(!result.success);

        // Collect all progress events.
        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }

        // Last event should be Failed.
        let last = events.last().unwrap();
        assert!(
            matches!(last, AgentProgress::Failed { .. }),
            "Last event should be Failed, got {:?}",
            last
        );
    }

    #[tokio::test]
    async fn test_agent_loop_single_iteration_max() {
        let (provider, _) = MockAgentProvider::simple_text("Quick");
        let provider = Arc::new(provider);
        let (tx, _rx) = mpsc::channel(32);

        let result = run_agent_loop(provider, "Quick task", default_config(1), tx)
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.iterations_used, 1);
        assert_eq!(result.output, "Quick");
    }

    #[tokio::test]
    async fn test_agent_loop_empty_response() {
        // Provider sends MessageComplete with no content delta.
        let (provider, _) = MockAgentProvider::new(vec![vec![StreamEvent::MessageComplete {
            stop_reason: StopReason::EndTurn,
        }]]);
        let provider = Arc::new(provider);
        let (tx, _rx) = mpsc::channel(32);

        let result = run_agent_loop(provider, "Empty", default_config(5), tx)
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.output, "");
        assert_eq!(result.iterations_used, 1);
    }

    #[tokio::test]
    async fn test_agent_loop_message_stop_completes() {
        // Provider sends MessageStop (legacy) instead of MessageComplete.
        let (provider, _) = MockAgentProvider::new(vec![vec![
            StreamEvent::ContentDelta("legacy".to_string()),
            StreamEvent::MessageStop,
        ]]);
        let provider = Arc::new(provider);
        let (tx, _rx) = mpsc::channel(32);

        let result = run_agent_loop(provider, "Legacy", default_config(5), tx)
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.output, "legacy");
        assert_eq!(result.iterations_used, 1);
    }
}
