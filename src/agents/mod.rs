//! Subagent execution and multi-agent coordination.
//!
//! This module provides the orchestration infrastructure for spawning and managing
//! multiple concurrent subagents with isolated contexts and tool restrictions.
//!
//! # Example
//!
//! ```
//! use patina::agents::{SubagentConfig, SubagentOrchestrator};
//!
//! let mut orchestrator = SubagentOrchestrator::new()
//!     .with_max_concurrent(4);
//!
//! let config = SubagentConfig {
//!     name: "explorer".to_string(),
//!     description: "Explores codebases".to_string(),
//!     system_prompt: "You explore code.".to_string(),
//!     allowed_tools: vec!["read".to_string(), "glob".to_string()],
//!     max_turns: 10,
//! };
//!
//! let id = orchestrator.spawn(config);
//! assert!(orchestrator.is_tool_allowed(id, "read"));
//! assert!(!orchestrator.is_tool_allowed(id, "bash"));
//! ```

use anyhow::Result;
use std::collections::HashMap;
use uuid::Uuid;

/// Configuration for a subagent.
///
/// Defines the agent's identity, behavior constraints, and tool permissions.
#[derive(Debug, Clone)]
pub struct SubagentConfig {
    /// Unique name identifying this agent type.
    pub name: String,
    /// Human-readable description of what this agent does.
    pub description: String,
    /// System prompt that defines the agent's behavior.
    pub system_prompt: String,
    /// List of tool names this agent is allowed to use.
    pub allowed_tools: Vec<String>,
    /// Maximum number of API turns before the agent must complete.
    pub max_turns: usize,
}

/// Result returned when a subagent completes execution.
#[derive(Debug)]
pub struct SubagentResult {
    /// The unique ID of the subagent.
    pub id: Uuid,
    /// The name of the subagent (from config).
    pub name: String,
    /// The output produced by the subagent.
    pub output: String,
    /// Whether the subagent completed successfully.
    pub success: bool,
}

/// Orchestrator for managing multiple subagents with concurrency control.
///
/// The orchestrator tracks agent lifecycles, enforces concurrency limits,
/// and provides tool restriction checking for isolated agent execution.
pub struct SubagentOrchestrator {
    active_agents: HashMap<Uuid, ActiveSubagent>,
    max_concurrent: usize,
}

/// Internal representation of an active subagent.
struct ActiveSubagent {
    config: SubagentConfig,
    status: SubagentStatus,
}

/// Lifecycle status of a subagent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubagentStatus {
    /// Agent has been spawned but not yet started.
    Pending,
    /// Agent is currently executing.
    Running,
    /// Agent completed successfully.
    Completed,
    /// Agent failed during execution.
    Failed,
}

impl SubagentOrchestrator {
    /// Creates a new orchestrator with default settings.
    ///
    /// Default `max_concurrent` is 4 agents.
    #[must_use]
    pub fn new() -> Self {
        Self {
            active_agents: HashMap::new(),
            max_concurrent: 4,
        }
    }

    /// Sets the maximum number of concurrently running agents.
    #[must_use]
    pub fn with_max_concurrent(mut self, max: usize) -> Self {
        self.max_concurrent = max;
        self
    }

    /// Spawns a new subagent with the given configuration.
    ///
    /// Returns the unique ID assigned to the agent. The agent starts in `pending` status
    /// and must be explicitly run with [`run`](Self::run).
    pub fn spawn(&mut self, config: SubagentConfig) -> Uuid {
        let id = Uuid::new_v4();

        self.active_agents.insert(
            id,
            ActiveSubagent {
                config,
                status: SubagentStatus::Pending,
            },
        );

        id
    }

    /// Runs a subagent to completion.
    ///
    /// # Errors
    ///
    /// Returns an error if the agent ID does not exist.
    pub async fn run(&mut self, id: Uuid) -> Result<SubagentResult> {
        let agent = self
            .active_agents
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("Subagent not found: {}", id))?;

        agent.status = SubagentStatus::Running;

        // Stub implementation - actual API execution handled by integration layer
        agent.status = SubagentStatus::Completed;

        Ok(SubagentResult {
            id,
            name: agent.config.name.clone(),
            output: "Subagent completed".to_string(),
            success: true,
        })
    }

    /// Returns the current status of an agent.
    ///
    /// Returns `None` if the agent ID does not exist.
    #[must_use]
    pub fn get_status(&self, id: Uuid) -> Option<&str> {
        self.active_agents.get(&id).map(|a| match a.status {
            SubagentStatus::Pending => "pending",
            SubagentStatus::Running => "running",
            SubagentStatus::Completed => "completed",
            SubagentStatus::Failed => "failed",
        })
    }

    /// Returns the count of currently running agents.
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.active_agents
            .values()
            .filter(|a| a.status == SubagentStatus::Running)
            .count()
    }

    /// Marks a subagent as failed.
    ///
    /// Returns `true` if the agent was found and marked, `false` otherwise.
    pub fn mark_failed(&mut self, id: Uuid) -> bool {
        if let Some(agent) = self.active_agents.get_mut(&id) {
            agent.status = SubagentStatus::Failed;
            true
        } else {
            false
        }
    }

    /// Returns the configuration for a subagent.
    ///
    /// Returns `None` if the agent ID does not exist.
    #[must_use]
    pub fn get_config(&self, id: Uuid) -> Option<&SubagentConfig> {
        self.active_agents.get(&id).map(|a| &a.config)
    }

    /// Checks if a tool is allowed for a subagent.
    ///
    /// Returns `false` if the agent does not exist or the tool is not in the allowed list.
    /// Tool matching is case-sensitive.
    #[must_use]
    pub fn is_tool_allowed(&self, id: Uuid, tool: &str) -> bool {
        self.active_agents
            .get(&id)
            .map(|a| a.config.allowed_tools.iter().any(|t| t == tool))
            .unwrap_or(false)
    }

    /// Returns the list of allowed tools for a subagent.
    ///
    /// Returns `None` if the agent ID does not exist.
    #[must_use]
    pub fn get_allowed_tools(&self, id: Uuid) -> Option<Vec<String>> {
        self.active_agents
            .get(&id)
            .map(|a| a.config.allowed_tools.clone())
    }

    /// Returns the maximum number of concurrent running agents.
    #[must_use]
    pub fn max_concurrent(&self) -> usize {
        self.max_concurrent
    }

    /// Checks if a new agent can be spawned based on the concurrent limit.
    ///
    /// Returns `true` if the number of running agents is below the max_concurrent limit.
    #[must_use]
    pub fn can_spawn(&self) -> bool {
        self.active_count() < self.max_concurrent
    }

    /// Returns a list of all agent IDs.
    #[must_use]
    pub fn list_agents(&self) -> Vec<Uuid> {
        self.active_agents.keys().copied().collect()
    }

    /// Removes an agent from the orchestrator.
    ///
    /// Returns `true` if the agent was found and removed, `false` otherwise.
    pub fn remove_agent(&mut self, id: Uuid) -> bool {
        self.active_agents.remove(&id).is_some()
    }
}

impl Default for SubagentOrchestrator {
    fn default() -> Self {
        Self::new()
    }
}
