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
//!     color: Some("cyan".to_string()),
//!     effort: None,
//! };
//!
//! let id = orchestrator.spawn(config);
//! assert!(orchestrator.is_tool_allowed(id, "read"));
//! assert!(!orchestrator.is_tool_allowed(id, "bash"));
//! ```

pub mod conflict;
pub mod orchestrator;
pub mod parallel;
pub mod worktree_agent;

// Re-export orchestrator types for convenience
pub use orchestrator::{
    SubagentContext, SubagentExecutionResult, SubagentResultCollector, SubagentRunner,
    SubagentSession, SubagentSpawner,
};

// Re-export parallel orchestration types
pub use parallel::{
    DivisionStrategy, MergeStrategy, OrchestratorStatus, ParallelAgentOrchestrator,
    ParallelTaskResult,
};

// Re-export conflict detection types
pub use conflict::{
    AgentFileChanges, Conflict, ConflictDetector, ConflictReport, ConflictSeverity,
};

// Re-export worktree agent types
pub use worktree_agent::{
    run_agent_loop, AgentLoopConfig, AgentLoopResult, AgentProgress, WorktreeAgentManager,
};

/// Events emitted by agents for processing by the main event loop.
///
/// Agent events flow from background agent tasks through the event dispatcher
/// to update the TUI agent panel and trigger completion/failure handling.
///
/// # Examples
///
/// ```
/// use patina::agents::{AgentEvent, AgentProgress};
///
/// let event = AgentEvent::Progress {
///     agent_id: "explorer-1".to_string(),
///     agent_name: "explorer".to_string(),
///     progress: AgentProgress::IterationStarted { iteration: 1, max: 10 },
/// };
///
/// assert!(matches!(event, AgentEvent::Progress { .. }));
/// ```
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// An agent reported progress (iteration start, content, completion, or failure).
    Progress {
        /// Unique identifier for the agent instance.
        agent_id: String,
        /// Human-readable agent name.
        agent_name: String,
        /// The progress update.
        progress: AgentProgress,
    },

    /// A conflict was detected between agents operating on overlapping files.
    ConflictDetected {
        /// The full conflict report.
        report: ConflictReport,
    },
}

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
    /// Visual indicator color for TUI display (e.g., "red", "green", "cyan").
    pub color: Option<String>,
    /// Reasoning effort level override (e.g., "low", "medium", "high").
    pub effort: Option<String>,
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

/// A message sent between agents via the [`AgentRegistry`].
///
/// # Examples
///
/// ```
/// use patina::agents::AgentMessage;
///
/// let msg = AgentMessage {
///     from: "agent-1".to_string(),
///     content: "Hello from agent 1".to_string(),
/// };
/// assert_eq!(msg.from, "agent-1");
/// ```
#[derive(Debug, Clone)]
pub struct AgentMessage {
    /// The ID or name of the sending agent.
    pub from: String,
    /// The message content to deliver.
    pub content: String,
}

/// Thread-safe registry for inter-agent communication.
///
/// Agents register themselves with a message channel on spawn and unregister
/// on completion. The `SendMessage` tool uses this registry to route messages
/// between running agents.
///
/// # Examples
///
/// ```
/// use patina::agents::{AgentMessage, AgentRegistry};
///
/// let registry = AgentRegistry::new();
/// let (tx, mut rx) = tokio::sync::mpsc::channel(16);
/// registry.register("agent-1".to_string(), tx);
/// assert!(registry.is_registered("agent-1"));
/// ```
pub struct AgentRegistry {
    agents: std::sync::RwLock<HashMap<String, tokio::sync::mpsc::Sender<AgentMessage>>>,
}

impl AgentRegistry {
    /// Creates a new empty agent registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            agents: std::sync::RwLock::new(HashMap::new()),
        }
    }

    /// Registers an agent with a message channel.
    ///
    /// If an agent with the same ID is already registered, its channel is replaced.
    pub fn register(&self, id: String, sender: tokio::sync::mpsc::Sender<AgentMessage>) {
        let mut agents = self.agents.write().unwrap_or_else(|e| e.into_inner());
        agents.insert(id, sender);
    }

    /// Unregisters an agent, removing its message channel.
    ///
    /// Returns `true` if the agent was found and removed.
    pub fn unregister(&self, id: &str) -> bool {
        let mut agents = self.agents.write().unwrap_or_else(|e| e.into_inner());
        agents.remove(id).is_some()
    }

    /// Sends a message to a registered agent.
    ///
    /// # Errors
    ///
    /// Returns an error if the target agent is not registered or the channel is closed.
    pub async fn send_message(&self, to: &str, message: AgentMessage) -> Result<()> {
        let sender = {
            let agents = self.agents.read().unwrap_or_else(|e| e.into_inner());
            agents.get(to).cloned()
        };

        match sender {
            Some(tx) => tx
                .send(message)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to send message to agent '{}': {}", to, e)),
            None => Err(anyhow::anyhow!("Agent '{}' is not registered", to)),
        }
    }

    /// Returns `true` if an agent with the given ID is registered.
    #[must_use]
    pub fn is_registered(&self, id: &str) -> bool {
        let agents = self.agents.read().unwrap_or_else(|e| e.into_inner());
        agents.contains_key(id)
    }

    /// Returns the IDs of all registered agents.
    #[must_use]
    pub fn list_agents(&self) -> Vec<String> {
        let agents = self.agents.read().unwrap_or_else(|e| e.into_inner());
        agents.keys().cloned().collect()
    }
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for AgentRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let agents = self.agents.read().unwrap_or_else(|e| e.into_inner());
        f.debug_struct("AgentRegistry")
            .field("agent_count", &agents.len())
            .field("agent_ids", &agents.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_orchestrator_spawn_and_status() {
        let mut orch = SubagentOrchestrator::new();
        let config = SubagentConfig {
            name: "test".to_string(),
            description: "test agent".to_string(),
            system_prompt: "You are a test.".to_string(),
            allowed_tools: vec!["read_file".to_string()],
            max_turns: 5,
            color: None,
            effort: None,
        };
        let id = orch.spawn(config);
        assert_eq!(orch.get_status(id), Some("pending"));
        assert!(orch.is_tool_allowed(id, "read_file"));
        assert!(!orch.is_tool_allowed(id, "bash"));
    }

    #[test]
    fn test_orchestrator_concurrency_limit() {
        let orch = SubagentOrchestrator::new().with_max_concurrent(2);
        assert_eq!(orch.max_concurrent(), 2);
        assert!(orch.can_spawn());
    }

    #[tokio::test]
    async fn test_registry_register_and_send() {
        let registry = AgentRegistry::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        registry.register("agent-1".to_string(), tx);

        assert!(registry.is_registered("agent-1"));

        let msg = AgentMessage {
            from: "agent-2".to_string(),
            content: "Hello from agent 2".to_string(),
        };
        registry.send_message("agent-1", msg).await.unwrap();

        let received = rx.recv().await.unwrap();
        assert_eq!(received.from, "agent-2");
        assert_eq!(received.content, "Hello from agent 2");
    }

    #[tokio::test]
    async fn test_registry_send_to_unknown_agent() {
        let registry = AgentRegistry::new();
        let msg = AgentMessage {
            from: "sender".to_string(),
            content: "hello".to_string(),
        };
        let result = registry.send_message("nonexistent", msg).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not registered"));
    }

    #[test]
    fn test_registry_unregister() {
        let registry = AgentRegistry::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        registry.register("agent-1".to_string(), tx);
        assert!(registry.is_registered("agent-1"));

        assert!(registry.unregister("agent-1"));
        assert!(!registry.is_registered("agent-1"));
        assert!(!registry.unregister("agent-1"));
    }

    #[test]
    fn test_registry_list_agents() {
        let registry = AgentRegistry::new();
        let (tx1, _rx1) = tokio::sync::mpsc::channel(16);
        let (tx2, _rx2) = tokio::sync::mpsc::channel(16);
        registry.register("alpha".to_string(), tx1);
        registry.register("beta".to_string(), tx2);

        let mut agents = registry.list_agents();
        agents.sort();
        assert_eq!(agents, vec!["alpha", "beta"]);
    }

    #[test]
    fn test_registry_debug_format() {
        let registry = AgentRegistry::new();
        let debug_str = format!("{:?}", registry);
        assert!(debug_str.contains("AgentRegistry"));
        assert!(debug_str.contains("agent_count"));
    }

    #[test]
    fn test_registry_default() {
        let registry = AgentRegistry::default();
        assert!(registry.list_agents().is_empty());
    }
}
