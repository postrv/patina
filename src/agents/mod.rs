//! Subagent execution and multi-agent coordination

use anyhow::Result;
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct SubagentConfig {
    pub name: String,
    pub description: String,
    pub system_prompt: String,
    pub allowed_tools: Vec<String>,
    pub max_turns: usize,
}

#[derive(Debug)]
pub struct SubagentResult {
    pub id: Uuid,
    pub name: String,
    pub output: String,
    pub success: bool,
}

pub struct SubagentOrchestrator {
    active_agents: HashMap<Uuid, ActiveSubagent>,
    max_concurrent: usize,
}

struct ActiveSubagent {
    config: SubagentConfig,
    status: SubagentStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubagentStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

impl SubagentOrchestrator {
    pub fn new() -> Self {
        Self {
            active_agents: HashMap::new(),
            max_concurrent: 4,
        }
    }

    pub fn with_max_concurrent(mut self, max: usize) -> Self {
        self.max_concurrent = max;
        self
    }

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

    pub async fn run(&mut self, id: Uuid) -> Result<SubagentResult> {
        let agent = self
            .active_agents
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("Subagent not found: {}", id))?;

        agent.status = SubagentStatus::Running;

        agent.status = SubagentStatus::Completed;

        Ok(SubagentResult {
            id,
            name: agent.config.name.clone(),
            output: "Subagent completed".to_string(),
            success: true,
        })
    }

    pub fn get_status(&self, id: Uuid) -> Option<&str> {
        self.active_agents.get(&id).map(|a| match a.status {
            SubagentStatus::Pending => "pending",
            SubagentStatus::Running => "running",
            SubagentStatus::Completed => "completed",
            SubagentStatus::Failed => "failed",
        })
    }

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
}

impl Default for SubagentOrchestrator {
    fn default() -> Self {
        Self::new()
    }
}
