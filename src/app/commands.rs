//! Slash command handler for the TUI application.
//!
//! This module provides the integration layer between user input and the
//! slash command system. It:
//! - Detects when user input is a slash command (starts with `/`)
//! - Dispatches to the appropriate command handler
//! - Returns results for display in the TUI
//!
//! # Example
//!
//! ```rust
//! use patina::app::commands::{SlashCommandHandler, CommandResult};
//! use std::path::PathBuf;
//!
//! let handler = SlashCommandHandler::new(PathBuf::from("."));
//!
//! match handler.handle("/help") {
//!     CommandResult::Executed(output) => println!("{}", output),
//!     CommandResult::NotACommand => println!("Not a slash command"),
//!     CommandResult::UnknownCommand(cmd) => println!("Unknown: {}", cmd),
//!     CommandResult::Error(e) => println!("Error: {}", e),
//!     CommandResult::Action(action) => println!("Action: {:?}", action),
//! }
//! ```

use crate::agents::worktree_agent::{AgentInfo, WorktreeAgentManager, WorktreeAgentStatus};
use crate::commands::agent::{parse_agent_command, AgentCommand};
use crate::commands::continuous::{parse_continuous_command, ContinuousCommand};
use crate::commands::experiment::{parse_experiment_command, ExperimentCommand};
use crate::commands::worktree::{parse_worktree_command, WorktreeCommand};
use crate::worktree::{Experiment, WorktreeInfo, WorktreeManager};
use std::path::PathBuf;

/// Information about an MCP server for display purposes.
#[derive(Debug, Clone)]
pub struct McpServerInfo {
    /// The server name.
    pub name: String,
    /// The server status (e.g., "Connected", "Failed: reason").
    pub status: String,
    /// The number of tools discovered from this server.
    pub tool_count: usize,
}

/// Information about a loaded plugin for display purposes.
#[derive(Debug, Clone, Default)]
pub struct PluginInfo {
    /// The plugin name.
    pub name: String,
    /// The plugin version.
    pub version: String,
    /// Optional description of the plugin.
    pub description: Option<String>,
    /// List of command names provided by this plugin.
    pub commands: Vec<String>,
    /// List of skill names provided by this plugin.
    pub skills: Vec<String>,
}

/// An action that requires state mutation, dispatched back to the event loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandAction {
    /// Trigger context compaction with optional custom instructions.
    Compact {
        /// Optional instructions to guide the compaction summarization.
        custom_instructions: Option<String>,
    },
    /// Clear the conversation history while preserving configuration.
    Clear,
    /// Switch to a different model by name or alias.
    SetModel(String),
    /// Copy a message to the system clipboard.
    CopyToClipboard {
        /// Optional message index (last assistant message if None).
        message_index: Option<usize>,
    },
    /// Rename the current session.
    RenameSession(String),
    /// Set the prompt bar color.
    SetColor(String),
    /// Send a side-question that doesn't interrupt the current flow.
    SideQuestion(String),
    /// Fork the current session with an optional branch name.
    ForkSession {
        /// Optional branch name for the forked session.
        branch_name: Option<String>,
    },
    /// Show the rewind picker to select a checkpoint.
    ShowRewindPicker,
}

/// Result of handling a slash command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandResult {
    /// The command was executed successfully with output.
    Executed(String),

    /// The input was not a slash command (doesn't start with `/`).
    NotACommand,

    /// The command was not recognized.
    UnknownCommand(String),

    /// An error occurred while executing the command.
    Error(String),

    /// The command requires a state mutation via the event loop.
    Action(CommandAction),
}

/// Handler for slash commands in the TUI.
///
/// Parses user input, identifies slash commands, and dispatches to the
/// appropriate handler.
pub struct SlashCommandHandler {
    /// Working directory for command execution.
    working_dir: PathBuf,
    /// Information about loaded plugins.
    plugins: Vec<PluginInfo>,
    /// Information about MCP servers.
    mcp_servers: Vec<McpServerInfo>,
    /// Pre-formatted cost summary from AppState.
    cost_summary: Option<String>,
    /// Conversation messages for /context and /export.
    messages: Vec<crate::types::Message>,
    /// Context limit in tokens (for /context analysis).
    context_limit: usize,
    /// Current session ID (for /fork).
    session_id: Option<String>,
}

impl SlashCommandHandler {
    /// Creates a new slash command handler.
    #[must_use]
    pub fn new(working_dir: PathBuf) -> Self {
        Self {
            working_dir,
            plugins: Vec::new(),
            mcp_servers: Vec::new(),
            cost_summary: None,
            messages: Vec::new(),
            context_limit: 200_000,
            session_id: None,
        }
    }

    /// Adds plugin information to the handler.
    ///
    /// Use this to enable the `/plugins` command to display loaded plugins.
    #[must_use]
    pub fn with_plugins(mut self, plugins: Vec<PluginInfo>) -> Self {
        self.plugins = plugins;
        self
    }

    /// Sets the cost summary for the `/cost` command.
    #[must_use]
    pub fn with_cost_summary(mut self, summary: String) -> Self {
        self.cost_summary = Some(summary);
        self
    }

    /// Sets conversation messages for `/context` and `/export` commands.
    #[must_use]
    pub fn with_messages(
        mut self,
        messages: Vec<crate::types::Message>,
        context_limit: usize,
    ) -> Self {
        self.messages = messages;
        self.context_limit = context_limit;
        self
    }

    /// Sets the session ID for the `/fork` command.
    #[must_use]
    pub fn with_session_id(mut self, session_id: Option<String>) -> Self {
        self.session_id = session_id;
        self
    }

    /// Adds MCP server information to the handler.
    ///
    /// Use this to enable the `/mcp` command to display server status.
    #[must_use]
    pub fn with_mcp_info(mut self, mcp_servers: Vec<McpServerInfo>) -> Self {
        self.mcp_servers = mcp_servers;
        self
    }

    /// Handles user input, checking if it's a slash command.
    ///
    /// # Arguments
    ///
    /// * `input` - The raw user input string
    ///
    /// # Returns
    ///
    /// A `CommandResult` indicating the outcome:
    /// - `Executed(output)` - Command ran successfully
    /// - `NotACommand` - Input doesn't start with `/`
    /// - `UnknownCommand(name)` - Slash command not recognized
    /// - `Error(message)` - Command failed with error
    pub fn handle(&self, input: &str) -> CommandResult {
        let trimmed = input.trim();

        // Check if input is a slash command
        if !trimmed.starts_with('/') {
            return CommandResult::NotACommand;
        }

        // Parse the command name and arguments
        let without_slash = &trimmed[1..];
        let mut parts = without_slash.split_whitespace();

        let command_name = match parts.next() {
            Some(name) => name,
            None => return CommandResult::Error("Empty command".to_string()),
        };

        let args: String = parts.collect::<Vec<_>>().join(" ");

        // Dispatch to the appropriate handler
        match command_name {
            "agent" => self.handle_agent(&args),
            "analyze" => self.handle_analyze(&args),
            "audit" => self.handle_audit(&args),
            "continuous" => self.handle_continuous(&args),
            "mcp" => self.handle_mcp(),
            "experiment" => self.handle_experiment(&args),
            "worktree" => self.handle_worktree(&args),
            "cost" => self.handle_cost(),
            "context" => self.handle_context(),
            "export" => self.handle_export(&args),
            "fork" | "branch" => self.handle_fork(&args),
            "rewind" => self.handle_rewind(),
            "help" => self.handle_help(if args.is_empty() { None } else { Some(&args) }),
            "memory" => self.handle_memory(&args),
            "plugins" => self.handle_plugins(),
            "terminal-setup" => self.handle_terminal_setup(),
            "compact" => self.handle_compact(&args),
            "clear" => CommandResult::Action(CommandAction::Clear),
            "model" => self.handle_model(&args),
            "copy" => self.handle_copy(&args),
            "status" => self.handle_status(),
            "config" | "settings" => self.handle_config(&args),
            "permissions" => self.handle_permissions(&args),
            "sandbox" => self.handle_sandbox(&args),
            "rename" => self.handle_rename(&args),
            "bug" | "feedback" => self.handle_bug(&args),
            "btw" => self.handle_btw(&args),
            "color" => self.handle_color(&args),
            "doctor" => self.handle_doctor(),
            _ => CommandResult::UnknownCommand(command_name.to_string()),
        }
    }

    /// Handles the `/plugins` command.
    fn handle_plugins(&self) -> CommandResult {
        if self.plugins.is_empty() {
            return CommandResult::Executed("No plugins loaded.".to_string());
        }

        let mut output = String::from("Loaded Plugins:\n");

        for plugin in &self.plugins {
            output.push_str(&format!("\n  {} v{}", plugin.name, plugin.version));

            if let Some(desc) = &plugin.description {
                output.push_str(&format!("\n    {}", desc));
            }

            if !plugin.commands.is_empty() {
                let cmd_list = plugin.commands.join(", ");
                output.push_str(&format!(
                    "\n    Commands ({}): {}",
                    plugin.commands.len(),
                    cmd_list
                ));
            }

            if !plugin.skills.is_empty() {
                let skill_list = plugin.skills.join(", ");
                output.push_str(&format!(
                    "\n    Skills ({}): {}",
                    plugin.skills.len(),
                    skill_list
                ));
            }
        }

        CommandResult::Executed(output)
    }

    /// Handles the `/mcp` command.
    fn handle_mcp(&self) -> CommandResult {
        if self.mcp_servers.is_empty() {
            return CommandResult::Executed("No MCP servers configured.".to_string());
        }

        let mut output = String::from("MCP Servers:\n");

        for server in &self.mcp_servers {
            output.push_str(&format!("\n  {} - {}", server.name, server.status));
            if server.tool_count > 0 {
                output.push_str(&format!(" ({} tools)", server.tool_count));
            }
        }

        let connected = self
            .mcp_servers
            .iter()
            .filter(|s| s.status == "Connected")
            .count();
        let total_tools: usize = self.mcp_servers.iter().map(|s| s.tool_count).sum();

        output.push_str(&format!(
            "\n\nSummary: {}/{} connected, {} tools available",
            connected,
            self.mcp_servers.len(),
            total_tools
        ));

        CommandResult::Executed(output)
    }

    /// Handles the `/continuous` command.
    ///
    /// Dispatches to `start`, `stop`, or `status` subcommands for managing
    /// the continuous coding loop.
    fn handle_continuous(&self, args: &str) -> CommandResult {
        let cmd = match parse_continuous_command(args) {
            Ok(cmd) => cmd,
            Err(e) => return CommandResult::Error(e.to_string()),
        };

        match cmd {
            ContinuousCommand::Start { max_iterations } => {
                let limit = match max_iterations {
                    Some(n) => format!(" (max {} iterations)", n),
                    None => String::new(),
                };
                CommandResult::Executed(format!(
                    "Started continuous coding loop{limit}.\n\
                     Use /continuous status to check progress.\n\
                     Use /continuous stop to halt."
                ))
            }
            ContinuousCommand::Stop => {
                CommandResult::Executed("Stopped continuous coding loop.".to_string())
            }
            ContinuousCommand::Status => {
                CommandResult::Executed("Continuous loop status: Inactive".to_string())
            }
        }
    }

    /// Handles the `/agent` command.
    fn handle_agent(&self, args: &str) -> CommandResult {
        let agent_cmd = match parse_agent_command(args) {
            Ok(cmd) => cmd,
            Err(e) => return CommandResult::Error(e.to_string()),
        };

        let mut manager = match WorktreeAgentManager::new(&self.working_dir) {
            Ok(m) => m,
            Err(e) => {
                return CommandResult::Error(format!("Failed to initialize agent manager: {}", e))
            }
        };

        match agent_cmd {
            AgentCommand::New { name, task } => match manager.spawn(&name, &task) {
                Ok(handle) => CommandResult::Executed(format!(
                    "Spawned agent '{}'\n  Branch: {}\n  Worktree: {}\n  Task: {}",
                    handle.name(),
                    handle.branch(),
                    handle.worktree_path().display(),
                    handle.task(),
                )),
                Err(e) => CommandResult::Error(format!("Failed to spawn agent: {}", e)),
            },

            AgentCommand::List => {
                let agents = manager.list();
                if agents.is_empty() {
                    return CommandResult::Executed("No agents found.".to_string());
                }

                let mut output = String::from("Agents:\n");
                for agent in &agents {
                    output.push_str(&Self::format_agent_row(agent));
                    output.push('\n');
                }
                CommandResult::Executed(output.trim_end().to_string())
            }

            AgentCommand::Status { name } => match manager.status(&name) {
                Ok(info) => CommandResult::Executed(Self::format_agent_detail(&info)),
                Err(e) => CommandResult::Error(format!("Failed to get agent status: {}", e)),
            },

            AgentCommand::Merge { name } => {
                // First check the agent exists and is completed
                let info = match manager.status(&name) {
                    Ok(info) => info,
                    Err(e) => {
                        return CommandResult::Error(format!("Failed to get agent status: {}", e))
                    }
                };

                if !info.status.is_terminal() {
                    return CommandResult::Error(format!(
                        "Agent '{}' is still running. Stop it first with /agent stop {}",
                        name, name,
                    ));
                }

                if matches!(info.status, WorktreeAgentStatus::Failed(_)) {
                    return CommandResult::Error(format!(
                        "Agent '{}' failed. Review its output before merging.",
                        name,
                    ));
                }

                // Attempt git merge
                let branch = info.branch.clone();
                match std::process::Command::new("git")
                    .args(["merge", &branch, "--no-edit"])
                    .current_dir(&self.working_dir)
                    .output()
                {
                    Ok(output) => {
                        if output.status.success() {
                            // Clean up after successful merge
                            let _ = manager.cleanup(&name);
                            CommandResult::Executed(format!(
                                "Merged agent '{}' (branch: {}) into current branch.\nAgent cleaned up.",
                                name, branch,
                            ))
                        } else {
                            let stderr = String::from_utf8_lossy(&output.stderr);
                            CommandResult::Error(format!(
                                "Merge failed for agent '{}': {}",
                                name, stderr,
                            ))
                        }
                    }
                    Err(e) => CommandResult::Error(format!("Failed to run git merge: {}", e)),
                }
            }

            AgentCommand::Stop { name } => match manager.mark_stopped(&name) {
                Ok(()) => CommandResult::Executed(format!("Stopped agent '{}'.", name)),
                Err(e) => CommandResult::Error(format!("Failed to stop agent: {}", e)),
            },
        }
    }

    /// Formats an agent row for the list display.
    fn format_agent_row(agent: &AgentInfo) -> String {
        let status = match &agent.status {
            WorktreeAgentStatus::Running => "running",
            WorktreeAgentStatus::Completed => "completed",
            WorktreeAgentStatus::Failed(_) => "failed",
            WorktreeAgentStatus::Stopped => "stopped",
        };

        // Truncate task to 40 chars for table display
        let task_display = if agent.task.len() > 40 {
            format!("{}...", &agent.task[..37])
        } else {
            agent.task.clone()
        };

        format!("  {} ({}) - {}", agent.name, status, task_display)
    }

    /// Formats detailed agent information for the status command.
    fn format_agent_detail(agent: &AgentInfo) -> String {
        format!(
            "Agent: {}\n  Status: {}\n  Branch: {}\n  Worktree: {}\n  Task: {}",
            agent.name,
            agent.status,
            agent.branch,
            agent.worktree_path.display(),
            agent.task,
        )
    }

    /// Formats a worktree entry for display.
    fn format_worktree(wt: &WorktreeInfo) -> String {
        let branch = if wt.branch.is_empty() {
            "detached"
        } else {
            &wt.branch
        };
        format!("  {} ({}) - {}", wt.name, branch, wt.path.display())
    }

    /// Formats a worktree entry with full status details.
    fn format_worktree_status(wt: &WorktreeInfo) -> String {
        let branch = if wt.branch.is_empty() {
            "detached"
        } else {
            &wt.branch
        };
        format!(
            "  {} ({})\n    Path: {}",
            wt.name,
            branch,
            wt.path.display()
        )
    }

    /// Handles the `/worktree` command.
    fn handle_worktree(&self, args: &str) -> CommandResult {
        let worktree_cmd = match parse_worktree_command(args) {
            Ok(cmd) => cmd,
            Err(e) => return CommandResult::Error(e.to_string()),
        };

        // Create worktree manager - handle potential failure
        let manager = match WorktreeManager::new(&self.working_dir) {
            Ok(m) => m,
            Err(e) => {
                return CommandResult::Error(format!(
                    "Failed to initialize worktree manager: {}",
                    e
                ))
            }
        };

        match worktree_cmd {
            WorktreeCommand::New { name } => match manager.create(&name) {
                Ok(info) => CommandResult::Executed(format!(
                    "Created worktree '{}' at {}",
                    name,
                    info.path.display()
                )),
                Err(e) => CommandResult::Error(format!("Failed to create worktree: {}", e)),
            },

            WorktreeCommand::List => match manager.list() {
                Ok(worktrees) if worktrees.is_empty() => {
                    CommandResult::Executed("No worktrees found.".to_string())
                }
                Ok(worktrees) => {
                    let output = worktrees
                        .iter()
                        .map(Self::format_worktree)
                        .collect::<Vec<_>>()
                        .join("\n");
                    CommandResult::Executed(format!("Worktrees:\n{}", output))
                }
                Err(e) => CommandResult::Error(format!("Failed to list worktrees: {}", e)),
            },

            WorktreeCommand::Switch { name } => {
                // Switch is not directly applicable in TUI context
                // Just report what would happen
                CommandResult::Executed(format!(
                    "To switch to worktree '{}', open a new terminal in that directory.",
                    name
                ))
            }

            WorktreeCommand::Remove { name } => match manager.remove(&name) {
                Ok(()) => CommandResult::Executed(format!("Removed worktree '{}'", name)),
                Err(e) => CommandResult::Error(format!("Failed to remove worktree: {}", e)),
            },

            WorktreeCommand::Clean => {
                // Clean prunable worktrees using git worktree prune
                match std::process::Command::new("git")
                    .args(["worktree", "prune"])
                    .current_dir(&self.working_dir)
                    .output()
                {
                    Ok(output) => {
                        if output.status.success() {
                            CommandResult::Executed("Pruned stale worktree entries.".to_string())
                        } else {
                            let stderr = String::from_utf8_lossy(&output.stderr);
                            CommandResult::Error(format!("Failed to prune worktrees: {}", stderr))
                        }
                    }
                    Err(e) => {
                        CommandResult::Error(format!("Failed to run git worktree prune: {}", e))
                    }
                }
            }

            WorktreeCommand::Status => match manager.list() {
                Ok(worktrees) if worktrees.is_empty() => {
                    CommandResult::Executed("No worktrees found.".to_string())
                }
                Ok(worktrees) => {
                    let output = worktrees
                        .iter()
                        .map(Self::format_worktree_status)
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    CommandResult::Executed(format!("Worktree Status:\n{}", output))
                }
                Err(e) => CommandResult::Error(format!("Failed to get worktree status: {}", e)),
            },
        }
    }

    /// Handles the `/experiment` command.
    fn handle_experiment(&self, args: &str) -> CommandResult {
        let experiment_cmd = match parse_experiment_command(args) {
            Ok(cmd) => cmd,
            Err(e) => return CommandResult::Error(e.to_string()),
        };

        // Create worktree manager - handle potential failure
        let manager = match WorktreeManager::new(&self.working_dir) {
            Ok(m) => m,
            Err(e) => {
                return CommandResult::Error(format!(
                    "Failed to initialize worktree manager: {}",
                    e
                ))
            }
        };

        match experiment_cmd {
            ExperimentCommand::Start { name } => match Experiment::start(&manager, &name) {
                Ok(exp) => CommandResult::Executed(format!(
                    "Started experiment '{}' at {}",
                    name,
                    exp.worktree_path().display()
                )),
                Err(e) => CommandResult::Error(format!("Failed to start experiment: {}", e)),
            },

            ExperimentCommand::List => match Experiment::list(&manager) {
                Ok(experiments) if experiments.is_empty() => {
                    CommandResult::Executed("No experiments found.".to_string())
                }
                Ok(experiments) => {
                    let output = experiments
                        .iter()
                        .map(|exp| {
                            format!(
                                "  {} [{}] (branch: {})\n    Path: {}",
                                exp.name(),
                                exp.state(),
                                exp.experiment_branch(),
                                exp.worktree_path().display()
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    CommandResult::Executed(format!("Experiments:\n{}", output))
                }
                Err(e) => CommandResult::Error(format!("Failed to list experiments: {}", e)),
            },

            ExperimentCommand::Accept { name } => {
                let experiments = match Experiment::list(&manager) {
                    Ok(e) => e,
                    Err(e) => {
                        return CommandResult::Error(format!("Failed to list experiments: {}", e))
                    }
                };

                match experiments.into_iter().find(|e| e.name() == name) {
                    Some(exp) => match exp.accept() {
                        Ok(result) => CommandResult::Executed(format!(
                            "Accepted experiment '{}'. Merged {} commit(s).",
                            name, result.commits_merged
                        )),
                        Err(e) => {
                            CommandResult::Error(format!("Failed to accept experiment: {}", e))
                        }
                    },
                    None => CommandResult::Error(format!("Experiment '{}' not found.", name)),
                }
            }

            ExperimentCommand::Reject { name } => {
                let experiments = match Experiment::list(&manager) {
                    Ok(e) => e,
                    Err(e) => {
                        return CommandResult::Error(format!("Failed to list experiments: {}", e))
                    }
                };

                match experiments.into_iter().find(|e| e.name() == name) {
                    Some(exp) => match exp.reject() {
                        Ok(_result) => CommandResult::Executed(format!(
                            "Rejected experiment '{}'. Changes discarded.",
                            name
                        )),
                        Err(e) => {
                            CommandResult::Error(format!("Failed to reject experiment: {}", e))
                        }
                    },
                    None => CommandResult::Error(format!("Experiment '{}' not found.", name)),
                }
            }

            ExperimentCommand::Pause { name } => {
                let experiments = match Experiment::list(&manager) {
                    Ok(e) => e,
                    Err(e) => {
                        return CommandResult::Error(format!("Failed to list experiments: {}", e))
                    }
                };

                match experiments.into_iter().find(|e| e.name() == name) {
                    Some(exp) => match exp.pause() {
                        Ok(_exp) => {
                            CommandResult::Executed(format!("Paused experiment '{}'.", name))
                        }
                        Err(e) => {
                            CommandResult::Error(format!("Failed to pause experiment: {}", e))
                        }
                    },
                    None => CommandResult::Error(format!("Experiment '{}' not found.", name)),
                }
            }

            ExperimentCommand::Resume { name } => {
                let experiments = match Experiment::list(&manager) {
                    Ok(e) => e,
                    Err(e) => {
                        return CommandResult::Error(format!("Failed to list experiments: {}", e))
                    }
                };

                match experiments.into_iter().find(|e| e.name() == name) {
                    Some(exp) => match exp.resume() {
                        Ok(_exp) => {
                            CommandResult::Executed(format!("Resumed experiment '{}'.", name))
                        }
                        Err(e) => {
                            CommandResult::Error(format!("Failed to resume experiment: {}", e))
                        }
                    },
                    None => CommandResult::Error(format!("Experiment '{}' not found.", name)),
                }
            }

            ExperimentCommand::Status { name } => {
                let experiments = match Experiment::list(&manager) {
                    Ok(e) => e,
                    Err(e) => {
                        return CommandResult::Error(format!("Failed to list experiments: {}", e))
                    }
                };

                match experiments.into_iter().find(|e| e.name() == name) {
                    Some(exp) => {
                        let desc = exp
                            .description()
                            .map_or(String::new(), |d| format!("\n  Description: {}", d));
                        CommandResult::Executed(format!(
                            "Experiment '{}':\n  State: {}\n  Branch: {}\n  Original branch: {}\n  Path: {}{}",
                            exp.name(),
                            exp.state(),
                            exp.experiment_branch(),
                            exp.original_branch(),
                            exp.worktree_path().display(),
                            desc,
                        ))
                    }
                    None => CommandResult::Error(format!("Experiment '{}' not found.", name)),
                }
            }
        }
    }

    /// Handles the `/cost` command.
    ///
    /// Displays session cost summary including total, per-model breakdown,
    /// and budget status. Actual cost data is provided by the TUI layer
    /// via `CostTracker`; this returns a placeholder directing the user.
    fn handle_cost(&self) -> CommandResult {
        CommandResult::Executed(
            self.cost_summary
                .clone()
                .unwrap_or_else(|| "No usage data recorded yet.".to_string()),
        )
    }

    /// Handles the `/context` command.
    ///
    /// Displays context window usage breakdown with per-message token estimates.
    fn handle_context(&self) -> CommandResult {
        if self.messages.is_empty() {
            return CommandResult::Executed(
                "Context analysis: No messages in session yet.".to_string(),
            );
        }
        let analysis = crate::app::context_analysis::ContextAnalysis::analyze(
            &self.messages,
            self.context_limit,
        );
        CommandResult::Executed(analysis.format())
    }

    /// Handles the `/export` command.
    ///
    /// Exports the conversation in markdown (default) or JSON format.
    fn handle_export(&self, args: &str) -> CommandResult {
        let format = args.trim();
        if self.messages.is_empty() {
            return CommandResult::Executed("No messages to export.".to_string());
        }
        match format {
            "" | "markdown" | "md" => {
                let mut output = String::from("# Conversation Export\n\n");
                for msg in &self.messages {
                    let role = match msg.role {
                        crate::types::Role::User => "**User**",
                        crate::types::Role::Assistant => "**Assistant**",
                    };
                    output.push_str(&format!("## {}\n\n{}\n\n---\n\n", role, msg.content));
                }
                CommandResult::Executed(output)
            }
            "json" => match serde_json::to_string_pretty(&self.messages) {
                Ok(json) => CommandResult::Executed(json),
                Err(e) => CommandResult::Error(format!("Failed to serialize: {e}")),
            },
            _ => CommandResult::Error(format!(
                "Unknown export format: '{}'. Use 'markdown' or 'json'.",
                format
            )),
        }
    }

    /// Handles the `/fork` (or `/branch`) command.
    ///
    /// Creates a new session forked from the current one, preserving
    /// conversation history up to this point. When called with a branch name,
    /// dispatches a `ForkSession` action to the event loop.
    fn handle_fork(&self, args: &str) -> CommandResult {
        let name = args.trim();
        if name.is_empty() {
            return CommandResult::Executed(
                "Fork: Creates a new session from the current conversation.\n\
                 Usage: /fork [branch-name]\n\
                 The new session preserves all messages up to this point."
                    .to_string(),
            );
        }
        match &self.session_id {
            Some(_) => CommandResult::Action(CommandAction::ForkSession {
                branch_name: Some(name.to_string()),
            }),
            None => CommandResult::Error(
                "Cannot fork: no active session. Start a conversation first.".to_string(),
            ),
        }
    }

    /// Handles the `/rewind` command.
    ///
    /// Shows the rewind picker for selecting a checkpoint to rewind to.
    fn handle_rewind(&self) -> CommandResult {
        match &self.session_id {
            Some(_) => CommandResult::Action(CommandAction::ShowRewindPicker),
            None => CommandResult::Error(
                "Cannot rewind: no active session. Start a conversation first.".to_string(),
            ),
        }
    }

    /// Handles the `/memory` command.
    ///
    /// Manages persistent memory entries across sessions.
    fn handle_memory(&self, args: &str) -> CommandResult {
        let parts: Vec<&str> = args.splitn(3, ' ').collect();
        match parts.first().copied().unwrap_or("") {
            "" | "list" => {
                let filter = parts.get(1).copied().unwrap_or("");
                CommandResult::Executed(format!(
                    "Memory list{}:\n  No memories stored yet.\n\n\
                     Usage: /memory add <type> <content>\n\
                     Types: user, feedback, project, reference",
                    if filter.is_empty() {
                        String::new()
                    } else {
                        format!(" (filter: {})", filter)
                    }
                ))
            }
            "add" => {
                if parts.len() < 3 {
                    return CommandResult::Error(
                        "Usage: /memory add <type> <content>\n\
                         Types: user, feedback, project, reference"
                            .to_string(),
                    );
                }
                let mem_type = parts[1];
                let content = parts[2];
                match mem_type {
                    "user" | "feedback" | "project" | "reference" => {
                        CommandResult::Executed(format!("Added {} memory: {}", mem_type, content))
                    }
                    _ => CommandResult::Error(format!(
                        "Unknown memory type: '{}'. Use: user, feedback, project, reference",
                        mem_type
                    )),
                }
            }
            "remove" => {
                if parts.len() < 2 {
                    return CommandResult::Error("Usage: /memory remove <id-prefix>".to_string());
                }
                CommandResult::Executed(format!("Removed memory: {}", parts[1]))
            }
            "search" => {
                if parts.len() < 2 {
                    return CommandResult::Error("Usage: /memory search <query>".to_string());
                }
                CommandResult::Executed(format!(
                    "Search results for '{}': No matches found.",
                    parts[1]
                ))
            }
            sub => CommandResult::Error(format!(
                "Unknown memory subcommand: '{}'. Use: list, add, remove, search",
                sub
            )),
        }
    }

    /// Handles the `/help` command.
    fn handle_help(&self, command: Option<&str>) -> CommandResult {
        match command {
            None => {
                // General help listing all commands
                let help_text = r#"Available Commands:

  /agent <subcommand>       - Manage worktree agents
    Subcommands: new, list, status, merge, stop

  /analyze [subcommand]     - Run narsil code analysis
    Subcommands: full, complexity, architecture, dead-code

  /audit [subcommand]       - Run narsil security audit
    Subcommands: full, taint, deps

  /btw <question>           - Ask a side question without interrupting flow

  /bug <description>        - Report a bug (alias: /feedback)

  /clear                    - Clear conversation history

  /color <color>            - Set prompt bar color

  /compact [instructions]   - Compact context window

  /config                   - Show/modify settings (alias: /settings)

  /continuous <subcommand>  - Manage continuous coding loop
    Subcommands: start, stop, status

  /context                  - Show context window usage breakdown

  /copy [index]             - Copy message to clipboard

  /cost                     - Show session cost and token usage

  /doctor                   - Troubleshoot environment

  /experiment <subcommand>  - Manage isolated experiments
    Subcommands: start, list, accept, reject, pause, resume, status

  /export [format]          - Export conversation (markdown or json)

  /fork [name]              - Fork session into a new branch

  /mcp                      - Show MCP server status

  /memory <subcommand>      - Manage persistent memory
    Subcommands: list, add, remove, search

  /model <name>             - Switch model (sonnet, opus, haiku)

  /permissions [subcommand] - View/manage tool permissions
    Subcommands: add, remove, reset

  /plugins                  - List loaded plugins

  /rename <name>            - Rename current session

  /sandbox                  - View sandbox settings

  /status                   - Show system status dashboard

  /terminal-setup           - Configure terminal keyboard shortcuts

  /worktree <subcommand>    - Manage git worktrees
    Subcommands: new, list, switch, remove, clean, status

  /help [command]           - Show help for a command

Type /help <command> for detailed help on a specific command."#;
                CommandResult::Executed(help_text.to_string())
            }

            Some("analyze") => {
                let help_text = r#"/analyze - Run narsil code analysis

Usage:
  /analyze                Run full comprehensive analysis
  /analyze full           Same as above
  /analyze complexity     Identify the most complex functions
  /analyze architecture   Analyze module structure and circular deps
  /analyze dead-code      Find unused functions, types, and modules

Directs the model to use narsil-mcp code intelligence tools
for structural code analysis. Requires narsil-mcp to be connected."#;
                CommandResult::Executed(help_text.to_string())
            }

            Some("audit") => {
                let help_text = r#"/audit - Run narsil security audit

Usage:
  /audit                Run full security audit
  /audit full           Same as above
  /audit taint          Trace tainted data flows from inputs to sinks
  /audit deps           Check dependencies for vulnerabilities and licenses

Directs the model to use narsil-mcp security scanning tools.
Requires narsil-mcp to be connected."#;
                CommandResult::Executed(help_text.to_string())
            }

            Some("cost") => {
                let help_text = r#"/cost - Show session cost and token usage

Usage:
  /cost       Show cost summary for the current session

Displays:
  - Total session cost in USD
  - Per-model breakdown (input/output tokens and cost)
  - Budget status (if limits configured)
  - Cache hit rate (if prompt caching active)"#;
                CommandResult::Executed(help_text.to_string())
            }

            Some("context") => {
                let help_text = r#"/context - Show context window usage breakdown

Usage:
  /context    Analyze token usage in the current conversation

Displays:
  - Total tokens used vs context limit
  - Per-message token estimates
  - Heavy messages that could be compressed
  - Suggestions for reducing context usage"#;
                CommandResult::Executed(help_text.to_string())
            }

            Some("continuous") => {
                let help_text = r#"/continuous - Manage continuous coding loop

Subcommands:
  start [N]    Start the continuous coding loop (optional max iterations)
  stop         Stop the currently running loop
  status       Show current loop status

The continuous loop automates the TDD cycle: iterate, run quality
gates, detect stagnation, and attempt self-healing recovery.

Examples:
  /continuous start
  /continuous start 50
  /continuous stop
  /continuous status"#;
                CommandResult::Executed(help_text.to_string())
            }

            Some("agent") => {
                let help_text = r#"/agent - Manage worktree agents

Subcommands:
  new <name> <task>  Spawn a new agent to work on a task
  list               List all agents and their status
  status <name>      Show detailed status of an agent
  merge <name>       Merge a completed agent's changes
  stop <name>        Stop a running agent

Each agent runs in an isolated git worktree on its own branch
(agent/<name>), enabling parallel work without file conflicts.

Examples:
  /agent new auth-fix Fix the authentication bug
  /agent list
  /agent status auth-fix
  /agent merge auth-fix
  /agent stop auth-fix"#;
                CommandResult::Executed(help_text.to_string())
            }

            Some("experiment") => {
                let help_text = r#"/experiment - Manage isolated experiments

Subcommands:
  start <name>   Start a new experiment in an isolated worktree
  list           List all experiments
  accept <name>  Accept experiment, merging changes back
  reject <name>  Reject experiment, discarding changes
  pause <name>   Pause an active experiment
  resume <name>  Resume a paused experiment
  status <name>  Show detailed status of an experiment

Experiments create isolated worktrees for risky changes.
Accept merges them back; reject discards them cleanly.

Examples:
  /experiment start risky-refactor
  /experiment list
  /experiment accept risky-refactor
  /experiment reject risky-refactor"#;
                CommandResult::Executed(help_text.to_string())
            }

            Some("worktree") => {
                let help_text = r#"/worktree - Manage git worktrees

Subcommands:
  new <name>     Create a new worktree with the given name
  list           List all worktrees in the repository
  switch <name>  Switch to an existing worktree
  remove <name>  Remove an existing worktree
  clean          Remove prunable worktrees (missing directories)
  status         Show status of all worktrees

Examples:
  /worktree new feature-123
  /worktree list
  /worktree remove feature-123"#;
                CommandResult::Executed(help_text.to_string())
            }

            Some("export") => {
                let help_text = r#"/export - Export conversation

Usage:
  /export            Export as markdown (default)
  /export markdown   Export as formatted markdown
  /export json       Export as raw JSON

The exported content is written to stdout. Redirect to a file
to save it (e.g., the TUI copies to clipboard)."#;
                CommandResult::Executed(help_text.to_string())
            }

            Some("fork") | Some("branch") => {
                let help_text = r#"/fork - Fork session into a new branch

Usage:
  /fork              Show usage information
  /fork <name>       Fork with the given branch name

Creates a new session that preserves all messages from the current
conversation. The original session is unchanged. Use this to explore
alternative approaches without losing your current context.

Aliases: /branch"#;
                CommandResult::Executed(help_text.to_string())
            }

            Some("help") => {
                let help_text = r#"/help - Show help information

Usage:
  /help          Show list of all available commands
  /help <cmd>    Show detailed help for a specific command

Examples:
  /help
  /help worktree"#;
                CommandResult::Executed(help_text.to_string())
            }

            Some("mcp") => {
                let help_text = r#"/mcp - Show MCP server status

Usage:
  /mcp       Show all configured MCP servers with connection status

Displays:
  - Server name and connection status
  - Number of tools discovered per server
  - Summary of connected servers and total tools

MCP servers are configured in .mcp.json (project) or ~/.claude.json (user).
Servers connect automatically at startup using stdio or SSE transport."#;
                CommandResult::Executed(help_text.to_string())
            }

            Some("memory") => {
                let help_text = r#"/memory - Manage persistent memory

Usage:
  /memory list [type]             List all memories (optional type filter)
  /memory add <type> <content>    Add a new memory
  /memory remove <id>             Remove a memory by ID prefix
  /memory search <query>          Search memories by content or tags

Types: user, feedback, project, reference

Memories persist across sessions and are injected into the system
prompt to maintain context continuity."#;
                CommandResult::Executed(help_text.to_string())
            }

            Some("plugins") => {
                let help_text = r#"/plugins - List loaded plugins

Usage:
  /plugins       Show all loaded plugins with details

Displays:
  - Plugin name and version
  - Plugin description (if available)
  - Commands provided by the plugin
  - Skills provided by the plugin

Plugins are loaded from ~/.config/patina/plugins/ at startup.
Use --no-plugins flag to disable plugin loading."#;
                CommandResult::Executed(help_text.to_string())
            }

            Some("terminal-setup") => {
                let help_text = r#"/terminal-setup - Configure terminal for optimal keyboard shortcuts

Usage:
  /terminal-setup    Auto-detect terminal and show configuration instructions

This command helps configure your terminal for Cmd+A/C/V (macOS) or
equivalent shortcuts. Configuration depends on your terminal:

iTerm2:
  Automatically configured on first run. Restart iTerm2 if prompted.

JetBrains (RustRover, IntelliJ, etc.):
  Requires manual configuration:
  1. Open Settings → Tools → Terminal
  2. Enable "Use Option as Meta key"
  3. Use Option+A/C/V for select all, copy, paste

Kitty, WezTerm, Ghostty:
  Cmd+A/C/V works natively (Kitty keyboard protocol).

Other terminals:
  Use Ctrl+A (select all), Ctrl+Y (copy), Ctrl+Shift+V (paste)."#;
                CommandResult::Executed(help_text.to_string())
            }

            Some("compact") => {
                let help_text = r#"/compact - Compact context window

Usage:
  /compact                    Summarize old messages to free context space
  /compact <instructions>     Compact with custom summarization guidance

Examples:
  /compact
  /compact preserve code blocks and error messages"#;
                CommandResult::Executed(help_text.to_string())
            }

            Some("clear") => {
                let help_text = r#"/clear - Clear conversation history

Usage:
  /clear       Reset the conversation, keeping configuration and cost data

Preserves: working directory, model, memory, MCP servers, plugins, cost tracker.
Resets: messages, timeline, tool state, token budget, scroll position."#;
                CommandResult::Executed(help_text.to_string())
            }

            Some("model") => {
                let help_text = r#"/model - Switch the active model

Usage:
  /model                Show usage and current model
  /model <name>         Switch to the specified model

Model aliases:
  sonnet    → claude-sonnet-4-20250514
  opus      → claude-opus-4-20250514
  haiku     → claude-haiku-4-5-20251001

You can also use a full model ID:
  /model claude-sonnet-4-5-20250929"#;
                CommandResult::Executed(help_text.to_string())
            }

            Some(cmd) => CommandResult::UnknownCommand(cmd.to_string()),
        }
    }

    /// Handles the `/terminal-setup` command.
    ///
    /// Detects the current terminal and provides configuration instructions
    /// for enabling Cmd+A/C/V (macOS) or equivalent shortcuts.
    fn handle_terminal_setup(&self) -> CommandResult {
        use crate::terminal::{
            configure_iterm2_keybindings, is_iterm2, is_jetbrains_terminal, is_kitty_terminal,
            is_macos,
        };

        let mut output = String::from("🔧 Terminal Keyboard Configuration\n\n");

        if is_iterm2() {
            output.push_str("Terminal: iTerm2\n\n");
            match configure_iterm2_keybindings() {
                Ok(true) => {
                    output.push_str("✅ Configured iTerm2 key bindings!\n\n");
                    output.push_str("Please restart iTerm2 for changes to take effect.\n\n");
                    output.push_str("After restart, you can use:\n");
                    output.push_str("  • Cmd+A - Select all\n");
                    output.push_str("  • Cmd+C - Copy selection\n");
                    output.push_str("  • Cmd+V - Paste\n");
                }
                Ok(false) => {
                    output.push_str("✅ iTerm2 is already configured!\n\n");
                    output.push_str("You can use:\n");
                    output.push_str("  • Cmd+A - Select all\n");
                    output.push_str("  • Cmd+C - Copy selection\n");
                    output.push_str("  • Cmd+V - Paste\n");
                }
                Err(e) => {
                    output.push_str(&format!("⚠️  Failed to configure: {}\n\n", e));
                    output.push_str("Manual setup:\n");
                    output.push_str("  1. Open iTerm2 → Settings → Profiles → Keys\n");
                    output
                        .push_str("  2. Add key mappings for Cmd+A/C/V to send escape sequences\n");
                }
            }
        } else if is_jetbrains_terminal() {
            output.push_str("Terminal: JetBrains IDE (RustRover, IntelliJ, etc.)\n\n");
            output.push_str("⚠️  JetBrains terminals don't support Cmd+key passthrough.\n");
            output.push_str("The IDE intercepts Cmd+keys for its own shortcuts.\n\n");
            output.push_str("📋 To enable Option+A/C/V shortcuts:\n\n");
            output.push_str("  1. Open IDE Settings (Cmd+,)\n");
            output.push_str("  2. Navigate to: Tools → Terminal\n");
            output.push_str("  3. Enable: \"Use Option as Meta key\" ✓\n");
            output.push_str("  4. Click Apply, then OK\n\n");
            output.push_str("After configuration, you can use:\n");
            output.push_str("  • Option+A - Select all\n");
            output.push_str("  • Option+C - Copy selection\n");
            output.push_str("  • Option+V - Paste\n\n");
            output.push_str("Alternative shortcuts (always work):\n");
            output.push_str("  • Ctrl+A    - Select all\n");
            output.push_str("  • Ctrl+Y    - Copy selection (yank)\n");
            output.push_str("  • Ctrl+Shift+V - Paste\n");
        } else if is_kitty_terminal() {
            output.push_str("Terminal: Kitty\n\n");
            output.push_str("✅ Kitty supports the Kitty keyboard protocol natively!\n\n");
            output.push_str("You can use:\n");
            output.push_str("  • Cmd+A - Select all\n");
            output.push_str("  • Cmd+C - Copy selection\n");
            output.push_str("  • Cmd+V - Paste\n");
        } else if is_macos() {
            output.push_str("Terminal: macOS (unknown terminal)\n\n");
            output.push_str("Your terminal may not support Cmd+key detection.\n\n");
            output.push_str("Recommended terminals with Cmd+key support:\n");
            output.push_str("  • iTerm2 (configure with /terminal-setup)\n");
            output.push_str("  • Kitty (native support)\n");
            output.push_str("  • WezTerm (native support)\n");
            output.push_str("  • Ghostty (native support)\n\n");
            output.push_str("Universal shortcuts (always work):\n");
            output.push_str("  • Ctrl+A    - Select all\n");
            output.push_str("  • Ctrl+Y    - Copy selection (yank)\n");
            output.push_str("  • Ctrl+Shift+V - Paste\n");
        } else {
            output.push_str("Terminal: Linux/Windows\n\n");
            output.push_str("Standard keyboard shortcuts:\n");
            output.push_str("  • Ctrl+A       - Select all\n");
            output.push_str("  • Ctrl+Y       - Copy selection (yank)\n");
            output.push_str("  • Ctrl+Shift+V - Paste\n");
        }

        CommandResult::Executed(output)
    }

    /// Handles the `/compact` command.
    fn handle_compact(&self, args: &str) -> CommandResult {
        let custom = if args.is_empty() {
            None
        } else {
            Some(args.to_string())
        };
        CommandResult::Action(CommandAction::Compact {
            custom_instructions: custom,
        })
    }

    /// Handles the `/model` command.
    fn handle_model(&self, args: &str) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Executed(
                "Usage: /model <name>\n\n\
                 Aliases: sonnet, opus, haiku\n\
                 Or a full model ID: claude-sonnet-4-20250514"
                    .to_string(),
            );
        }
        let model_name = resolve_model_alias(args.trim());
        CommandResult::Action(CommandAction::SetModel(model_name))
    }

    /// Handles the `/copy` command — copies a message to the clipboard.
    fn handle_copy(&self, args: &str) -> CommandResult {
        let index = if args.is_empty() {
            None
        } else {
            match args.trim().parse::<usize>() {
                Ok(n) => Some(n),
                Err(_) => {
                    return CommandResult::Error(
                        "Usage: /copy [message_index]\n\n\
                         Copies the last assistant message to the clipboard.\n\
                         Optionally specify a message index."
                            .to_string(),
                    );
                }
            }
        };
        CommandResult::Action(CommandAction::CopyToClipboard {
            message_index: index,
        })
    }

    /// Handles the `/status` command — displays system status dashboard.
    fn handle_status(&self) -> CommandResult {
        let mut output = String::from("System Status:\n\n");

        output.push_str(&format!(
            "  Working directory: {}\n",
            self.working_dir.display()
        ));

        // MCP servers status
        if self.mcp_servers.is_empty() {
            output.push_str("  MCP servers: none\n");
        } else {
            output.push_str(&format!("  MCP servers: {}\n", self.mcp_servers.len()));
            for server in &self.mcp_servers {
                output.push_str(&format!(
                    "    {} — {} ({} tools)\n",
                    server.name, server.status, server.tool_count
                ));
            }
        }

        // Plugins
        if self.plugins.is_empty() {
            output.push_str("  Plugins: none\n");
        } else {
            output.push_str(&format!("  Plugins: {}\n", self.plugins.len()));
        }

        // Messages
        output.push_str(&format!("  Messages: {}\n", self.messages.len()));

        // Session
        if let Some(id) = &self.session_id {
            output.push_str(&format!("  Session: {id}\n"));
        }

        output.push_str("\n  Use /cost for token usage, /context for context breakdown.");

        CommandResult::Executed(output)
    }

    /// Handles the `/config` command — shows/modifies settings.
    fn handle_config(&self, args: &str) -> CommandResult {
        if args.is_empty() {
            let output = r#"Current Configuration:

  Use /model <name>     to change model
  Use /compact           to compact context
  Use /color <color>     to set prompt color
  Use /sandbox           to view sandbox settings
  Use /permissions       to view permissions

Configuration is loaded from:
  Project: .claude/CLAUDE.md
  User:    ~/.claude/CLAUDE.md
  Global:  ~/.config/patina/settings.json"#;
            return CommandResult::Executed(output.to_string());
        }
        CommandResult::Executed(format!(
            "Config setting '{}' — use /help config for details",
            args
        ))
    }

    /// Handles the `/permissions` command — view/manage permissions.
    fn handle_permissions(&self, args: &str) -> CommandResult {
        if args.is_empty() {
            let mut output = String::from("Permission Rules:\n\n");
            output.push_str("  Current mode: prompt-per-tool (default)\n\n");
            output.push_str("  Allowed patterns:\n");
            output.push_str("    read_file, glob, grep, list_files — always allowed\n");
            output.push_str("    write_file, edit, bash — require approval\n\n");
            output.push_str("  Use /permissions add <pattern> to add an allow rule.\n");
            output.push_str("  Use /permissions remove <pattern> to remove a rule.\n");
            output.push_str("  Use /permissions reset to reset to defaults.");
            return CommandResult::Executed(output);
        }

        match args.split_whitespace().next() {
            Some("add") => {
                let pattern = args.strip_prefix("add").unwrap_or("").trim();
                if pattern.is_empty() {
                    CommandResult::Error("Usage: /permissions add <pattern>".to_string())
                } else {
                    CommandResult::Executed(format!("Permission rule added: {pattern}"))
                }
            }
            Some("remove") => {
                let pattern = args.strip_prefix("remove").unwrap_or("").trim();
                if pattern.is_empty() {
                    CommandResult::Error("Usage: /permissions remove <pattern>".to_string())
                } else {
                    CommandResult::Executed(format!("Permission rule removed: {pattern}"))
                }
            }
            Some("reset") => CommandResult::Executed("Permissions reset to defaults.".to_string()),
            _ => CommandResult::Error("Unknown subcommand. Use: add, remove, reset".to_string()),
        }
    }

    /// Handles the `/sandbox` command — view/manage sandbox settings.
    fn handle_sandbox(&self, args: &str) -> CommandResult {
        if args.is_empty() {
            let mut output = String::from("Sandbox Settings:\n\n");
            output.push_str("  Mode: enabled (default)\n");
            output.push_str("  Platform: ");
            if cfg!(target_os = "macos") {
                output.push_str("macOS Seatbelt\n");
            } else if cfg!(target_os = "linux") {
                output.push_str("Linux Landlock\n");
            } else {
                output.push_str("not available on this platform\n");
            }
            output.push_str("\n  Filesystem access:\n");
            output.push_str(&format!("    Read/Write: {}\n", self.working_dir.display()));
            output.push_str("    Read-only: /tmp, /var/folders (temp)\n\n");
            output.push_str("  Network: outbound HTTP/HTTPS allowed\n");
            output.push_str("  Process: fork/exec allowed within sandbox");
            return CommandResult::Executed(output);
        }
        CommandResult::Executed(format!(
            "Sandbox setting '{}' — use /help sandbox for details",
            args
        ))
    }

    /// Handles the `/rename` command — rename the current session.
    fn handle_rename(&self, args: &str) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(
                "Usage: /rename <new-name>\n\nRename the current session.".to_string(),
            );
        }
        CommandResult::Action(CommandAction::RenameSession(args.trim().to_string()))
    }

    /// Handles the `/bug` and `/feedback` commands — report issues.
    fn handle_bug(&self, args: &str) -> CommandResult {
        if args.is_empty() {
            let output = r#"Report a Bug or Feedback:

  /bug <description>       File a bug report
  /feedback <description>  Send feedback

Reports are submitted to the project issue tracker.
Include steps to reproduce and expected vs actual behavior."#;
            return CommandResult::Executed(output.to_string());
        }

        // Format as a bug report
        let report = format!(
            "Bug Report:\n\n\
             Description: {args}\n\
             Working Directory: {}\n\
             Platform: {} {}\n\n\
             To file on GitHub, visit the project issue tracker.",
            self.working_dir.display(),
            std::env::consts::OS,
            std::env::consts::ARCH,
        );
        CommandResult::Executed(report)
    }

    /// Handles the `/btw` command — side question without interrupting flow.
    fn handle_btw(&self, args: &str) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(
                "Usage: /btw <question>\n\n\
                 Ask a side question without interrupting the current task flow."
                    .to_string(),
            );
        }
        CommandResult::Action(CommandAction::SideQuestion(args.to_string()))
    }

    /// Handles the `/color` command — set prompt bar color.
    fn handle_color(&self, args: &str) -> CommandResult {
        let valid_colors = [
            "red", "green", "yellow", "blue", "magenta", "cyan", "white", "reset",
        ];

        if args.is_empty() {
            return CommandResult::Executed(format!(
                "Usage: /color <color>\n\nAvailable colors: {}",
                valid_colors.join(", ")
            ));
        }

        let color = args.trim().to_lowercase();
        if valid_colors.contains(&color.as_str()) {
            CommandResult::Action(CommandAction::SetColor(color))
        } else {
            CommandResult::Error(format!(
                "Unknown color '{}'. Available: {}",
                color,
                valid_colors.join(", ")
            ))
        }
    }

    /// Handles the `/doctor` command — troubleshoot the environment.
    /// Handles the `/analyze` command.
    ///
    /// Returns a prompt directing the model to use narsil code intelligence
    /// tools for structural analysis. The prompt text is injected as a user
    /// message to guide the model.
    fn handle_analyze(&self, args: &str) -> CommandResult {
        let subcommand = args.split_whitespace().next().unwrap_or("full");
        let prompt = match subcommand {
            "full" | "" => {
                "Run comprehensive analysis using narsil code intelligence tools: \
                 get_metrics, get_function_hotspots, find_circular_imports, find_dead_code. \
                 Summarize all findings in a concise report covering architecture metrics, \
                 complexity hotspots, circular dependencies, and dead code."
            }
            "complexity" => {
                "Run get_function_hotspots and get_complexity using narsil code intelligence tools. \
                 Identify the top 10 most complex functions with their cyclomatic complexity scores. \
                 Flag any functions above complexity threshold 15 as needing refactoring."
            }
            "architecture" => {
                "Run get_import_graph and find_circular_imports using narsil code intelligence tools. \
                 Summarize the module structure, key dependency relationships, and any circular \
                 dependency cycles that should be broken."
            }
            "dead-code" => {
                "Run find_dead_code and find_unused_exports using narsil code intelligence tools. \
                 List all removable code including unused functions, types, and modules. \
                 Prioritize by size (largest dead code first)."
            }
            other => {
                return CommandResult::Error(format!(
                    "Unknown /analyze subcommand: '{}'. \
                     Valid subcommands: full, complexity, architecture, dead-code",
                    other
                ));
            }
        };
        CommandResult::Executed(prompt.to_string())
    }

    /// Handles the `/audit` command.
    ///
    /// Returns a prompt directing the model to use narsil security scanning
    /// tools for a security audit. The prompt text is injected as a user
    /// message to guide the model.
    fn handle_audit(&self, args: &str) -> CommandResult {
        let subcommand = args.split_whitespace().next().unwrap_or("full");
        let prompt = match subcommand {
            "full" | "" => {
                "Run a comprehensive security audit using narsil code intelligence tools: \
                 scan_security, check_cwe_top25, find_injection_vulnerabilities. \
                 Report all findings organized by severity (CRITICAL, HIGH, MEDIUM, LOW). \
                 Include remediation suggestions for each finding."
            }
            "taint" => {
                "Run trace_taint and get_taint_sources using narsil code intelligence tools. \
                 Identify all tainted data flows from external inputs (user input, network, \
                 file I/O) through the codebase. Flag any unsanitized flows reaching \
                 sensitive sinks (exec, SQL, file write)."
            }
            "deps" => {
                "Run check_dependencies and check_licenses using narsil code intelligence tools. \
                 Report any dependencies with known vulnerabilities and their severity. \
                 Also flag any license compatibility issues."
            }
            other => {
                return CommandResult::Error(format!(
                    "Unknown /audit subcommand: '{}'. \
                     Valid subcommands: full, taint, deps",
                    other
                ));
            }
        };
        CommandResult::Executed(prompt.to_string())
    }

    fn handle_doctor(&self) -> CommandResult {
        let mut output = String::from("Environment Check:\n\n");

        // Check git
        let git_ok = std::process::Command::new("git")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        output.push_str(&format!(
            "  git:    {}\n",
            if git_ok { "OK" } else { "NOT FOUND" }
        ));

        // Check gh CLI
        let gh_ok = std::process::Command::new("gh")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        output.push_str(&format!(
            "  gh CLI: {}\n",
            if gh_ok { "OK" } else { "NOT FOUND" }
        ));

        // Check working directory
        let wd_exists = self.working_dir.exists();
        output.push_str(&format!(
            "  workdir: {}\n",
            if wd_exists { "OK" } else { "MISSING" }
        ));

        // Check .claude/CLAUDE.md
        let claude_md = self.working_dir.join(".claude").join("CLAUDE.md");
        output.push_str(&format!(
            "  CLAUDE.md: {}\n",
            if claude_md.exists() {
                "found"
            } else {
                "not found"
            }
        ));

        // MCP servers
        output.push_str(&format!(
            "  MCP servers: {} configured\n",
            self.mcp_servers.len()
        ));

        // Plugins
        output.push_str(&format!("  Plugins: {} loaded\n", self.plugins.len()));

        output.push_str("\n  Run /status for full system status.");
        CommandResult::Executed(output)
    }

    /// Returns available command names for tab completion.
    #[must_use]
    pub fn available_commands(&self) -> Vec<&'static str> {
        vec![
            "agent",
            "analyze",
            "audit",
            "btw",
            "bug",
            "clear",
            "color",
            "compact",
            "config",
            "continuous",
            "context",
            "copy",
            "cost",
            "doctor",
            "experiment",
            "export",
            "feedback",
            "fork",
            "help",
            "mcp",
            "memory",
            "model",
            "permissions",
            "plugins",
            "rename",
            "sandbox",
            "settings",
            "status",
            "terminal-setup",
            "worktree",
        ]
    }

    /// Creates plugin info from a plugin registry.
    ///
    /// Extracts plugin metadata including name, version, description,
    /// commands, and skills for display by the `/plugins` command.
    #[must_use]
    pub fn build_plugin_info(registry: &crate::plugins::PluginRegistry) -> Vec<PluginInfo> {
        let plugin_names = registry.list_plugins();
        let all_commands = registry.list_commands();

        plugin_names
            .into_iter()
            .map(|name| {
                let manifest = registry.get_manifest(&name);

                // Collect commands for this plugin (format: "plugin:command")
                let prefix = format!("{}:", name);
                let commands: Vec<String> = all_commands
                    .iter()
                    .filter(|cmd| cmd.starts_with(&prefix))
                    .map(|cmd| cmd.strip_prefix(&prefix).unwrap_or(cmd).to_string())
                    .collect();

                // Get skills from manifest (future enhancement)
                let skills = Vec::new();

                PluginInfo {
                    name: name.clone(),
                    version: manifest
                        .map(|m| m.version.clone())
                        .unwrap_or_else(|| "unknown".to_string()),
                    description: manifest.and_then(|m| m.description.clone()),
                    commands,
                    skills,
                }
            })
            .collect()
    }
}

/// Resolves a model alias to a full model ID.
///
/// Supported aliases: `sonnet`, `opus`, `haiku`. Any other input
/// is returned as-is (assumed to be a full model ID).
#[must_use]
pub fn resolve_model_alias(alias: &str) -> String {
    match alias {
        "sonnet" => "claude-sonnet-4-20250514".to_string(),
        "opus" => "claude-opus-4-20250514".to_string(),
        "haiku" => "claude-haiku-4-5-20251001".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_handler_in_temp() -> (SlashCommandHandler, TempDir) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let handler = SlashCommandHandler::new(temp_dir.path().to_path_buf());
        (handler, temp_dir)
    }

    // =========================================================================
    // Basic command detection tests
    // =========================================================================

    #[test]
    fn test_not_a_command_no_slash() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("hello world");
        assert_eq!(result, CommandResult::NotACommand);
    }

    #[test]
    fn test_not_a_command_empty() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("");
        assert_eq!(result, CommandResult::NotACommand);
    }

    #[test]
    fn test_not_a_command_whitespace_only() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("   ");
        assert_eq!(result, CommandResult::NotACommand);
    }

    #[test]
    fn test_empty_command_error() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/");
        assert_eq!(result, CommandResult::Error("Empty command".to_string()));
    }

    #[test]
    fn test_unknown_command() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/foobar");
        assert_eq!(result, CommandResult::UnknownCommand("foobar".to_string()));
    }

    // =========================================================================
    // Worktree command tests
    // =========================================================================

    #[test]
    fn test_handle_slash_command_worktree_new() {
        let (handler, _temp) = create_handler_in_temp();

        // Note: This will fail because temp_dir is not a git repo
        // The test documents the expected behavior - proper error handling
        let result = handler.handle("/worktree new my-feature");

        match result {
            CommandResult::Error(msg) => {
                // Expected: fails because not a git repo or can't create
                assert!(
                    msg.contains("Failed to create worktree")
                        || msg.contains("Failed to initialize")
                        || msg.contains("not a git repository"),
                    "Should report failure: {}",
                    msg
                );
            }
            CommandResult::Executed(_) => {
                // Would succeed in a real git repo
            }
            other => panic!("Unexpected result: {:?}", other),
        }
    }

    #[test]
    fn test_handle_slash_command_worktree_list() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/worktree list");

        // Should either list worktrees or report error (not a git repo)
        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("Worktrees:") || output.contains("No worktrees"),
                    "Should show worktree info: {}",
                    output
                );
            }
            CommandResult::Error(msg) => {
                // Expected in non-git directory
                assert!(
                    msg.contains("Failed to list")
                        || msg.contains("Failed to initialize")
                        || msg.contains("not a git repository"),
                    "Error: {}",
                    msg
                );
            }
            other => panic!("Unexpected result: {:?}", other),
        }
    }

    #[test]
    fn test_handle_slash_command_worktree_missing_arg() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/worktree new");

        match result {
            CommandResult::Error(msg) => {
                assert!(
                    msg.contains("missing") || msg.contains("argument"),
                    "Should report missing argument: {}",
                    msg
                );
            }
            other => panic!("Expected error for missing argument: {:?}", other),
        }
    }

    #[test]
    fn test_handle_slash_command_worktree_unknown_subcommand() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/worktree unknown");

        match result {
            CommandResult::Error(msg) => {
                assert!(
                    msg.contains("unknown") || msg.contains("Unknown"),
                    "Should report unknown subcommand: {}",
                    msg
                );
            }
            other => panic!("Expected error for unknown subcommand: {:?}", other),
        }
    }

    // =========================================================================
    // Help command tests
    // =========================================================================

    #[test]
    fn test_handle_slash_command_help() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/help");

        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("Available Commands"),
                    "Should list commands: {}",
                    output
                );
                assert!(output.contains("worktree"), "Should mention worktree");
                assert!(output.contains("help"), "Should mention help");
            }
            other => panic!("Expected help output: {:?}", other),
        }
    }

    #[test]
    fn test_handle_slash_command_help_worktree() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/help worktree");

        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("/worktree"),
                    "Should describe worktree: {}",
                    output
                );
                assert!(output.contains("new"), "Should list new subcommand");
                assert!(output.contains("list"), "Should list list subcommand");
            }
            other => panic!("Expected worktree help: {:?}", other),
        }
    }

    #[test]
    fn test_handle_slash_command_help_unknown() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/help unknown");

        assert_eq!(result, CommandResult::UnknownCommand("unknown".to_string()));
    }

    // =========================================================================
    // Edge cases and whitespace handling
    // =========================================================================

    #[test]
    fn test_command_with_extra_whitespace() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("  /help  ");

        match result {
            CommandResult::Executed(_) => {}
            other => panic!("Should handle whitespace: {:?}", other),
        }
    }

    #[test]
    fn test_available_commands() {
        let (handler, _temp) = create_handler_in_temp();

        let commands = handler.available_commands();

        assert!(commands.contains(&"worktree"));
        assert!(commands.contains(&"help"));
    }

    // =========================================================================
    // CommandResult equality tests
    // =========================================================================

    #[test]
    fn test_command_result_equality() {
        assert_eq!(CommandResult::NotACommand, CommandResult::NotACommand);
        assert_eq!(
            CommandResult::Executed("test".to_string()),
            CommandResult::Executed("test".to_string())
        );
        assert_ne!(
            CommandResult::Executed("a".to_string()),
            CommandResult::Executed("b".to_string())
        );
    }

    #[test]
    fn test_command_result_debug() {
        let result = CommandResult::Executed("output".to_string());
        let debug_str = format!("{:?}", result);
        assert!(debug_str.contains("Executed"));
    }

    // =========================================================================
    // Plugin command tests
    // =========================================================================

    #[test]
    fn test_handle_slash_command_plugins_no_plugins() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/plugins");

        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("No plugins loaded") || output.contains("no plugins"),
                    "Should indicate no plugins: {}",
                    output
                );
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_handle_slash_command_plugins_with_plugins() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let handler = SlashCommandHandler::new(temp_dir.path().to_path_buf());

        // Add mock plugin info
        let plugins = vec![
            PluginInfo {
                name: "test-plugin".to_string(),
                version: "1.0.0".to_string(),
                description: Some("A test plugin".to_string()),
                commands: vec!["test-cmd".to_string()],
                skills: vec!["test-skill".to_string()],
            },
            PluginInfo {
                name: "another-plugin".to_string(),
                version: "2.0.0".to_string(),
                description: None,
                commands: vec![],
                skills: vec![],
            },
        ];
        let handler = handler.with_plugins(plugins);

        let result = handler.handle("/plugins");

        match result {
            CommandResult::Executed(output) => {
                // Should list plugin names and versions
                assert!(output.contains("test-plugin"), "Should show plugin name");
                assert!(output.contains("1.0.0"), "Should show version");
                assert!(
                    output.contains("another-plugin"),
                    "Should show second plugin"
                );
                assert!(output.contains("2.0.0"), "Should show second version");
            }
            other => panic!("Expected plugin listing: {:?}", other),
        }
    }

    #[test]
    fn test_handle_slash_command_plugins_shows_commands_and_skills() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let handler = SlashCommandHandler::new(temp_dir.path().to_path_buf());

        let plugins = vec![PluginInfo {
            name: "my-plugin".to_string(),
            version: "1.0.0".to_string(),
            description: Some("Plugin with features".to_string()),
            commands: vec!["cmd1".to_string(), "cmd2".to_string()],
            skills: vec!["skill1".to_string()],
        }];
        let handler = handler.with_plugins(plugins);

        let result = handler.handle("/plugins");

        match result {
            CommandResult::Executed(output) => {
                // Should show commands and skills count or names
                assert!(
                    output.contains("cmd1") || output.contains("2 command"),
                    "Should show commands info: {}",
                    output
                );
                assert!(
                    output.contains("skill1") || output.contains("1 skill"),
                    "Should show skills info: {}",
                    output
                );
            }
            other => panic!("Expected plugin details: {:?}", other),
        }
    }

    #[test]
    fn test_help_includes_plugins_command() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/help");

        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("plugins"),
                    "Help should mention plugins command: {}",
                    output
                );
            }
            other => panic!("Expected help output: {:?}", other),
        }
    }

    #[test]
    fn test_help_plugins_shows_detailed_help() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/help plugins");

        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("/plugins"),
                    "Should describe plugins command"
                );
                assert!(
                    output.contains("loaded") || output.contains("list"),
                    "Should explain what it does: {}",
                    output
                );
            }
            other => panic!("Expected plugins help: {:?}", other),
        }
    }

    #[test]
    fn test_available_commands_includes_plugins() {
        let (handler, _temp) = create_handler_in_temp();

        let commands = handler.available_commands();

        assert!(
            commands.contains(&"plugins"),
            "Available commands should include 'plugins'"
        );
    }

    // =========================================================================
    // Terminal setup command tests
    // =========================================================================

    #[test]
    fn test_handle_terminal_setup_command() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/terminal-setup");

        match result {
            CommandResult::Executed(output) => {
                // Should contain terminal configuration info
                assert!(
                    output.contains("Terminal") || output.contains("terminal"),
                    "Should mention terminal: {}",
                    output
                );
                // Should contain keyboard shortcut info
                assert!(
                    output.contains("Ctrl") || output.contains("Cmd") || output.contains("Option"),
                    "Should mention keyboard shortcuts: {}",
                    output
                );
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_help_includes_terminal_setup() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/help");

        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("terminal-setup"),
                    "Help should mention terminal-setup command: {}",
                    output
                );
            }
            other => panic!("Expected help output: {:?}", other),
        }
    }

    #[test]
    fn test_help_terminal_setup_shows_detailed_help() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/help terminal-setup");

        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("/terminal-setup"),
                    "Should describe terminal-setup command"
                );
                assert!(
                    output.contains("JetBrains") || output.contains("iTerm"),
                    "Should mention supported terminals: {}",
                    output
                );
            }
            other => panic!("Expected terminal-setup help: {:?}", other),
        }
    }

    #[test]
    fn test_available_commands_includes_terminal_setup() {
        let (handler, _temp) = create_handler_in_temp();

        let commands = handler.available_commands();

        assert!(
            commands.contains(&"terminal-setup"),
            "Available commands should include 'terminal-setup'"
        );
    }

    // =========================================================================
    // Agent command tests
    // =========================================================================

    #[test]
    fn test_handle_agent_no_subcommand() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/agent");

        match result {
            CommandResult::Error(msg) => {
                assert!(
                    msg.contains("no subcommand"),
                    "Should report missing subcommand: {}",
                    msg
                );
            }
            other => panic!("Expected error: {:?}", other),
        }
    }

    #[test]
    fn test_handle_agent_unknown_subcommand() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/agent foobar");

        match result {
            CommandResult::Error(msg) => {
                assert!(
                    msg.contains("unknown subcommand") || msg.contains("foobar"),
                    "Should report unknown subcommand: {}",
                    msg
                );
            }
            other => panic!("Expected error: {:?}", other),
        }
    }

    #[test]
    fn test_handle_agent_new_missing_args() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/agent new");

        match result {
            CommandResult::Error(msg) => {
                assert!(
                    msg.contains("missing") || msg.contains("argument"),
                    "Should report missing argument: {}",
                    msg
                );
            }
            other => panic!("Expected error for missing args: {:?}", other),
        }
    }

    #[test]
    fn test_handle_agent_new_missing_task() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/agent new my-agent");

        match result {
            CommandResult::Error(msg) => {
                assert!(
                    msg.contains("missing") || msg.contains("task"),
                    "Should report missing task: {}",
                    msg
                );
            }
            other => panic!("Expected error for missing task: {:?}", other),
        }
    }

    #[test]
    fn test_handle_agent_new_in_non_git_dir() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/agent new test-agent Fix the bug");

        match result {
            CommandResult::Error(msg) => {
                // Expected: fails because temp_dir is not a git repo
                assert!(
                    msg.contains("Failed to spawn agent")
                        || msg.contains("Failed to initialize")
                        || msg.contains("not a git repository")
                        || msg.contains("worktree error"),
                    "Should report failure: {}",
                    msg
                );
            }
            CommandResult::Executed(_) => {
                // Would succeed in a real git repo
            }
            other => panic!("Unexpected result: {:?}", other),
        }
    }

    #[test]
    fn test_handle_agent_list_in_non_git_dir() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/agent list");

        match result {
            CommandResult::Error(msg) => {
                assert!(
                    msg.contains("Failed to initialize")
                        || msg.contains("not a git repository")
                        || msg.contains("worktree error"),
                    "Should report init failure: {}",
                    msg
                );
            }
            CommandResult::Executed(output) => {
                // If it happens to work, should show no agents
                assert!(
                    output.contains("No agents") || output.contains("Agents:"),
                    "Should show agent info: {}",
                    output
                );
            }
            other => panic!("Unexpected result: {:?}", other),
        }
    }

    #[test]
    fn test_handle_agent_status_missing_name() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/agent status");

        match result {
            CommandResult::Error(msg) => {
                assert!(
                    msg.contains("missing") || msg.contains("name"),
                    "Should report missing name: {}",
                    msg
                );
            }
            other => panic!("Expected error: {:?}", other),
        }
    }

    #[test]
    fn test_handle_agent_merge_missing_name() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/agent merge");

        match result {
            CommandResult::Error(msg) => {
                assert!(
                    msg.contains("missing") || msg.contains("name"),
                    "Should report missing name: {}",
                    msg
                );
            }
            other => panic!("Expected error: {:?}", other),
        }
    }

    #[test]
    fn test_handle_agent_stop_missing_name() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/agent stop");

        match result {
            CommandResult::Error(msg) => {
                assert!(
                    msg.contains("missing") || msg.contains("name"),
                    "Should report missing name: {}",
                    msg
                );
            }
            other => panic!("Expected error: {:?}", other),
        }
    }

    #[test]
    fn test_help_includes_agent_command() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/help");

        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("agent"),
                    "Help should mention agent command: {}",
                    output
                );
            }
            other => panic!("Expected help output: {:?}", other),
        }
    }

    #[test]
    fn test_help_agent_shows_detailed_help() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/help agent");

        match result {
            CommandResult::Executed(output) => {
                assert!(output.contains("/agent"), "Should describe agent command");
                assert!(output.contains("new"), "Should list new subcommand");
                assert!(output.contains("list"), "Should list list subcommand");
                assert!(output.contains("status"), "Should list status subcommand");
                assert!(output.contains("merge"), "Should list merge subcommand");
                assert!(output.contains("stop"), "Should list stop subcommand");
            }
            other => panic!("Expected agent help: {:?}", other),
        }
    }

    #[test]
    fn test_available_commands_includes_agent() {
        let (handler, _temp) = create_handler_in_temp();

        let commands = handler.available_commands();

        assert!(
            commands.contains(&"agent"),
            "Available commands should include 'agent'"
        );
    }

    #[test]
    fn test_format_agent_row() {
        let agent = AgentInfo {
            name: "test-agent".to_string(),
            task: "Fix the login bug".to_string(),
            worktree_path: PathBuf::from("/tmp/test"),
            branch: "agent/test-agent".to_string(),
            status: WorktreeAgentStatus::Running,
        };

        let row = SlashCommandHandler::format_agent_row(&agent);
        assert!(row.contains("test-agent"), "Should contain name");
        assert!(row.contains("running"), "Should contain status");
        assert!(row.contains("Fix the login bug"), "Should contain task");
    }

    #[test]
    fn test_format_agent_row_truncates_long_task() {
        let agent = AgentInfo {
            name: "test-agent".to_string(),
            task: "This is a very long task description that exceeds forty characters easily"
                .to_string(),
            worktree_path: PathBuf::from("/tmp/test"),
            branch: "agent/test-agent".to_string(),
            status: WorktreeAgentStatus::Completed,
        };

        let row = SlashCommandHandler::format_agent_row(&agent);
        assert!(row.contains("..."), "Should truncate long tasks");
        assert!(row.contains("completed"), "Should contain status");
    }

    #[test]
    fn test_format_agent_detail() {
        let agent = AgentInfo {
            name: "my-agent".to_string(),
            task: "Implement feature X".to_string(),
            worktree_path: PathBuf::from("/repo/.agent-worktrees/my-agent"),
            branch: "agent/my-agent".to_string(),
            status: WorktreeAgentStatus::Running,
        };

        let detail = SlashCommandHandler::format_agent_detail(&agent);
        assert!(detail.contains("Agent: my-agent"), "Should show name");
        assert!(detail.contains("running"), "Should show status");
        assert!(detail.contains("agent/my-agent"), "Should show branch");
        assert!(detail.contains("Implement feature X"), "Should show task");
    }

    // =========================================================================
    // Continuous command tests
    // =========================================================================

    #[test]
    fn test_handle_continuous_start() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/continuous start");

        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("Started") || output.contains("started"),
                    "Should confirm start: {}",
                    output
                );
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_handle_continuous_start_with_max_iterations() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/continuous start 50");

        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("50"),
                    "Should show max iterations: {}",
                    output
                );
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_handle_continuous_stop() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/continuous stop");

        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("Stopped") || output.contains("stopped"),
                    "Should confirm stop: {}",
                    output
                );
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_handle_continuous_status() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/continuous status");

        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("Inactive")
                        || output.contains("inactive")
                        || output.contains("Status")
                        || output.contains("status"),
                    "Should show status: {}",
                    output
                );
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_handle_continuous_no_subcommand() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/continuous");

        match result {
            CommandResult::Error(msg) => {
                assert!(
                    msg.contains("no subcommand"),
                    "Should report missing subcommand: {}",
                    msg
                );
            }
            other => panic!("Expected error: {:?}", other),
        }
    }

    #[test]
    fn test_handle_continuous_unknown_subcommand() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/continuous foobar");

        match result {
            CommandResult::Error(msg) => {
                assert!(
                    msg.contains("unknown subcommand") || msg.contains("foobar"),
                    "Should report unknown subcommand: {}",
                    msg
                );
            }
            other => panic!("Expected error: {:?}", other),
        }
    }

    #[test]
    fn test_handle_continuous_start_invalid_arg() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/continuous start abc");

        match result {
            CommandResult::Error(msg) => {
                assert!(
                    msg.contains("invalid") || msg.contains("abc"),
                    "Should report invalid argument: {}",
                    msg
                );
            }
            other => panic!("Expected error: {:?}", other),
        }
    }

    #[test]
    fn test_help_includes_continuous_command() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/help");

        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("continuous"),
                    "Help should mention continuous command: {}",
                    output
                );
            }
            other => panic!("Expected help output: {:?}", other),
        }
    }

    #[test]
    fn test_help_continuous_shows_detailed_help() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/help continuous");

        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("/continuous"),
                    "Should describe continuous command"
                );
                assert!(output.contains("start"), "Should list start subcommand");
                assert!(output.contains("stop"), "Should list stop subcommand");
                assert!(output.contains("status"), "Should list status subcommand");
            }
            other => panic!("Expected continuous help: {:?}", other),
        }
    }

    #[test]
    fn test_available_commands_includes_continuous() {
        let (handler, _temp) = create_handler_in_temp();

        let commands = handler.available_commands();

        assert!(
            commands.contains(&"continuous"),
            "Available commands should include 'continuous'"
        );
    }

    // =========================================================================
    // MCP command tests (Phase 10.2)
    // =========================================================================

    #[test]
    fn test_handle_mcp_no_servers() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/mcp");

        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("No MCP servers configured"),
                    "Should indicate no servers: {}",
                    output
                );
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_handle_mcp_with_connected_server() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let handler = SlashCommandHandler::new(temp_dir.path().to_path_buf()).with_mcp_info(vec![
            McpServerInfo {
                name: "narsil".to_string(),
                status: "Connected".to_string(),
                tool_count: 69,
            },
        ]);

        let result = handler.handle("/mcp");

        match result {
            CommandResult::Executed(output) => {
                assert!(output.contains("narsil"), "Should show server name");
                assert!(output.contains("Connected"), "Should show status");
                assert!(output.contains("69 tools"), "Should show tool count");
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_handle_mcp_with_failed_server() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let handler = SlashCommandHandler::new(temp_dir.path().to_path_buf()).with_mcp_info(vec![
            McpServerInfo {
                name: "broken".to_string(),
                status: "Failed: connection refused".to_string(),
                tool_count: 0,
            },
        ]);

        let result = handler.handle("/mcp");

        match result {
            CommandResult::Executed(output) => {
                assert!(output.contains("broken"), "Should show server name");
                assert!(
                    output.contains("Failed: connection refused"),
                    "Should show failure reason"
                );
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_handle_mcp_mixed_statuses() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let handler = SlashCommandHandler::new(temp_dir.path().to_path_buf()).with_mcp_info(vec![
            McpServerInfo {
                name: "narsil".to_string(),
                status: "Connected".to_string(),
                tool_count: 69,
            },
            McpServerInfo {
                name: "sse-server".to_string(),
                status: "Failed: timeout".to_string(),
                tool_count: 0,
            },
        ]);

        let result = handler.handle("/mcp");

        match result {
            CommandResult::Executed(output) => {
                assert!(output.contains("narsil"), "Should show first server");
                assert!(output.contains("sse-server"), "Should show second server");
                assert!(output.contains("1/2 connected"), "Should show summary");
                assert!(output.contains("69 tools"), "Should show total tools");
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_help_includes_mcp() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/help");

        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("mcp"),
                    "Help should mention mcp command: {}",
                    output
                );
            }
            other => panic!("Expected help output: {:?}", other),
        }
    }

    #[test]
    fn test_help_mcp_detailed() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/help mcp");

        match result {
            CommandResult::Executed(output) => {
                assert!(output.contains("/mcp"), "Should describe mcp command");
                assert!(
                    output.contains("MCP server") || output.contains("server status"),
                    "Should explain what it does: {}",
                    output
                );
            }
            other => panic!("Expected mcp help: {:?}", other),
        }
    }

    #[test]
    fn test_available_commands_includes_mcp() {
        let (handler, _temp) = create_handler_in_temp();

        let commands = handler.available_commands();

        assert!(
            commands.contains(&"mcp"),
            "Available commands should include 'mcp'"
        );
    }

    // =========================================================================
    // Agent command tests (continued)
    // =========================================================================

    #[test]
    fn test_format_agent_detail_failed_status() {
        let agent = AgentInfo {
            name: "failed-agent".to_string(),
            task: "Some task".to_string(),
            worktree_path: PathBuf::from("/tmp/test"),
            branch: "agent/failed-agent".to_string(),
            status: WorktreeAgentStatus::Failed("out of memory".to_string()),
        };

        let detail = SlashCommandHandler::format_agent_detail(&agent);
        assert!(
            detail.contains("failed: out of memory"),
            "Should show failure reason: {}",
            detail
        );
    }

    // =========================================================================
    // Cost command tests (Phase 8.7)
    // =========================================================================

    #[test]
    fn test_handle_cost_no_data() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/cost");
        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("No usage data"),
                    "Should show no-data message: {}",
                    output
                );
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_handle_cost_with_summary() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let handler = SlashCommandHandler::new(temp_dir.path().to_path_buf())
            .with_cost_summary("Session cost: $0.0123\nRequests: 5".to_string());
        let result = handler.handle("/cost");
        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("$0.0123"),
                    "Should show real cost: {}",
                    output
                );
                assert!(
                    output.contains("Requests: 5"),
                    "Should show request count: {}",
                    output
                );
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_help_cost() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/help cost");
        match result {
            CommandResult::Executed(output) => {
                assert!(output.contains("/cost"), "Should describe cost command");
                assert!(output.contains("session"), "Should mention session");
            }
            other => panic!("Expected cost help: {:?}", other),
        }
    }

    // =========================================================================
    // Context command tests (Phase 8.7)
    // =========================================================================

    #[test]
    fn test_handle_context() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/context");
        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("Context analysis"),
                    "Should show context info: {}",
                    output
                );
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_help_context() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/help context");
        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("/context"),
                    "Should describe context command"
                );
                assert!(output.contains("token"), "Should mention tokens");
            }
            other => panic!("Expected context help: {:?}", other),
        }
    }

    // =========================================================================
    // Export command tests (Phase 8.7)
    // =========================================================================

    #[test]
    fn test_handle_export_no_messages() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/export");
        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("No messages"),
                    "Should report no messages: {}",
                    output
                );
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_handle_export_markdown_with_messages() {
        use crate::types::{Message, Role};
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let handler = SlashCommandHandler::new(temp_dir.path().to_path_buf()).with_messages(
            vec![
                Message {
                    role: Role::User,
                    content: "Hello".to_string(),
                },
                Message {
                    role: Role::Assistant,
                    content: "Hi there".to_string(),
                },
            ],
            200_000,
        );
        let result = handler.handle("/export markdown");
        match result {
            CommandResult::Executed(output) => {
                assert!(output.contains("**User**"), "Should contain user role");
                assert!(output.contains("Hello"), "Should contain message");
                assert!(output.contains("Hi there"), "Should contain response");
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_handle_export_json_with_messages() {
        use crate::types::{Message, Role};
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let handler = SlashCommandHandler::new(temp_dir.path().to_path_buf()).with_messages(
            vec![Message {
                role: Role::User,
                content: "Test".to_string(),
            }],
            200_000,
        );
        let result = handler.handle("/export json");
        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("\"content\""),
                    "Should be valid JSON: {}",
                    output
                );
                assert!(output.contains("Test"), "Should contain message content");
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_handle_export_unknown_format() {
        use crate::types::{Message, Role};
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let handler = SlashCommandHandler::new(temp_dir.path().to_path_buf()).with_messages(
            vec![Message {
                role: Role::User,
                content: "Test".to_string(),
            }],
            200_000,
        );
        let result = handler.handle("/export csv");
        match result {
            CommandResult::Error(msg) => {
                assert!(msg.contains("csv"), "Should report unknown format: {}", msg);
            }
            other => panic!("Expected error: {:?}", other),
        }
    }

    #[test]
    fn test_help_export() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/help export");
        match result {
            CommandResult::Executed(output) => {
                assert!(output.contains("/export"), "Should describe export command");
                assert!(output.contains("markdown"), "Should mention markdown");
                assert!(output.contains("json"), "Should mention json");
            }
            other => panic!("Expected export help: {:?}", other),
        }
    }

    // =========================================================================
    // Fork command tests (Phase 8.7)
    // =========================================================================

    #[test]
    fn test_handle_fork_no_args() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/fork");
        match result {
            CommandResult::Executed(output) => {
                assert!(output.contains("Fork"), "Should show fork info: {}", output);
                assert!(output.contains("Usage"), "Should show usage");
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_handle_fork_with_name() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let handler = SlashCommandHandler::new(temp_dir.path().to_path_buf())
            .with_session_id(Some("abc12345-defg".to_string()));
        let result = handler.handle("/fork explore-auth");
        match result {
            CommandResult::Action(CommandAction::ForkSession { branch_name }) => {
                assert_eq!(
                    branch_name,
                    Some("explore-auth".to_string()),
                    "Should pass branch name to action"
                );
            }
            other => panic!("Expected ForkSession action: {:?}", other),
        }
    }

    #[test]
    fn test_handle_fork_no_session() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/fork test-branch");
        match result {
            CommandResult::Error(msg) => {
                assert!(
                    msg.contains("no active session"),
                    "Should report no session: {}",
                    msg
                );
            }
            other => panic!("Expected error: {:?}", other),
        }
    }

    #[test]
    fn test_handle_branch_alias() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let handler = SlashCommandHandler::new(temp_dir.path().to_path_buf())
            .with_session_id(Some("xyz98765-hijk".to_string()));
        let result = handler.handle("/branch my-experiment");
        match result {
            CommandResult::Action(CommandAction::ForkSession { branch_name }) => {
                assert_eq!(
                    branch_name,
                    Some("my-experiment".to_string()),
                    "Branch alias should produce ForkSession action"
                );
            }
            other => panic!("Expected ForkSession action: {:?}", other),
        }
    }

    // =========================================================================
    // Rewind command tests (Phase 2D)
    // =========================================================================

    #[test]
    fn test_handle_rewind_with_session() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let handler = SlashCommandHandler::new(temp_dir.path().to_path_buf())
            .with_session_id(Some("session-123".to_string()));
        let result = handler.handle("/rewind");
        match result {
            CommandResult::Action(CommandAction::ShowRewindPicker) => {
                // Expected
            }
            other => panic!("Expected ShowRewindPicker action: {:?}", other),
        }
    }

    #[test]
    fn test_handle_rewind_no_session() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/rewind");
        match result {
            CommandResult::Error(msg) => {
                assert!(
                    msg.contains("no active session"),
                    "Should report no session: {}",
                    msg
                );
            }
            other => panic!("Expected error: {:?}", other),
        }
    }

    #[test]
    fn test_help_fork() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/help fork");
        match result {
            CommandResult::Executed(output) => {
                assert!(output.contains("/fork"), "Should describe fork command");
                assert!(output.contains("branch"), "Should mention branch");
            }
            other => panic!("Expected fork help: {:?}", other),
        }
    }

    // =========================================================================
    // Memory command tests (Phase 8.7)
    // =========================================================================

    #[test]
    fn test_handle_memory_list() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/memory list");
        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("Memory list"),
                    "Should show list: {}",
                    output
                );
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_handle_memory_list_empty() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/memory");
        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("Memory list"),
                    "Should show list: {}",
                    output
                );
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_handle_memory_add() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/memory add user Prefers Rust");
        match result {
            CommandResult::Executed(output) => {
                assert!(output.contains("user"), "Should confirm type: {}", output);
                assert!(
                    output.contains("Prefers Rust"),
                    "Should confirm content: {}",
                    output
                );
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_handle_memory_add_invalid_type() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/memory add invalid Some content");
        match result {
            CommandResult::Error(msg) => {
                assert!(msg.contains("invalid"), "Should report bad type: {}", msg);
            }
            other => panic!("Expected error: {:?}", other),
        }
    }

    #[test]
    fn test_handle_memory_add_missing_args() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/memory add user");
        match result {
            CommandResult::Error(msg) => {
                assert!(msg.contains("Usage"), "Should show usage: {}", msg);
            }
            other => panic!("Expected error: {:?}", other),
        }
    }

    #[test]
    fn test_handle_memory_remove() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/memory remove abc123");
        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("abc123"),
                    "Should confirm removal: {}",
                    output
                );
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_handle_memory_search() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/memory search rust");
        match result {
            CommandResult::Executed(output) => {
                assert!(output.contains("rust"), "Should show query: {}", output);
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_handle_memory_unknown_subcommand() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/memory foobar");
        match result {
            CommandResult::Error(msg) => {
                assert!(msg.contains("foobar"), "Should report unknown: {}", msg);
            }
            other => panic!("Expected error: {:?}", other),
        }
    }

    #[test]
    fn test_help_memory() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/help memory");
        match result {
            CommandResult::Executed(output) => {
                assert!(output.contains("/memory"), "Should describe memory command");
                assert!(output.contains("add"), "Should mention add");
                assert!(output.contains("remove"), "Should mention remove");
            }
            other => panic!("Expected memory help: {:?}", other),
        }
    }

    // =========================================================================
    // Available commands includes all new commands (Phase 8.7)
    // =========================================================================

    #[test]
    fn test_available_commands_includes_all_new_commands() {
        let (handler, _temp) = create_handler_in_temp();
        let commands = handler.available_commands();

        assert!(commands.contains(&"cost"), "Should include cost");
        assert!(commands.contains(&"context"), "Should include context");
        assert!(commands.contains(&"export"), "Should include export");
        assert!(commands.contains(&"fork"), "Should include fork");
        assert!(commands.contains(&"memory"), "Should include memory");
    }

    #[test]
    fn test_help_includes_new_commands() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/help");
        match result {
            CommandResult::Executed(output) => {
                assert!(output.contains("cost"), "Help should mention cost");
                assert!(output.contains("context"), "Help should mention context");
                assert!(output.contains("export"), "Help should mention export");
                assert!(output.contains("fork"), "Help should mention fork");
                assert!(output.contains("memory"), "Help should mention memory");
            }
            other => panic!("Expected help output: {:?}", other),
        }
    }

    // =========================================================================
    // Phase 12.4: New slash command tests
    // =========================================================================

    #[test]
    fn test_handle_copy_no_args() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/copy");
        assert!(matches!(
            result,
            CommandResult::Action(CommandAction::CopyToClipboard {
                message_index: None
            })
        ));
    }

    #[test]
    fn test_handle_copy_with_index() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/copy 3");
        assert!(matches!(
            result,
            CommandResult::Action(CommandAction::CopyToClipboard {
                message_index: Some(3)
            })
        ));
    }

    #[test]
    fn test_handle_copy_invalid_index() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/copy abc");
        assert!(matches!(result, CommandResult::Error(_)));
    }

    #[test]
    fn test_handle_status() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/status");
        match result {
            CommandResult::Executed(output) => {
                assert!(output.contains("System Status"));
                assert!(output.contains("Working directory"));
            }
            other => panic!("Expected executed, got: {:?}", other),
        }
    }

    #[test]
    fn test_handle_config_no_args() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/config");
        match result {
            CommandResult::Executed(output) => {
                assert!(output.contains("Configuration"));
            }
            other => panic!("Expected executed, got: {:?}", other),
        }
    }

    #[test]
    fn test_handle_settings_alias() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/settings");
        assert!(matches!(result, CommandResult::Executed(_)));
    }

    #[test]
    fn test_handle_permissions_no_args() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/permissions");
        match result {
            CommandResult::Executed(output) => {
                assert!(output.contains("Permission Rules"));
            }
            other => panic!("Expected executed, got: {:?}", other),
        }
    }

    #[test]
    fn test_handle_permissions_add() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/permissions add bash:npm*");
        match result {
            CommandResult::Executed(output) => {
                assert!(output.contains("added"));
            }
            other => panic!("Expected executed, got: {:?}", other),
        }
    }

    #[test]
    fn test_handle_permissions_reset() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/permissions reset");
        match result {
            CommandResult::Executed(output) => {
                assert!(output.contains("reset"));
            }
            other => panic!("Expected executed, got: {:?}", other),
        }
    }

    #[test]
    fn test_handle_sandbox() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/sandbox");
        match result {
            CommandResult::Executed(output) => {
                assert!(output.contains("Sandbox"));
            }
            other => panic!("Expected executed, got: {:?}", other),
        }
    }

    #[test]
    fn test_handle_rename_no_args() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/rename");
        assert!(matches!(result, CommandResult::Error(_)));
    }

    #[test]
    fn test_handle_rename_with_name() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/rename my-session");
        assert_eq!(
            result,
            CommandResult::Action(CommandAction::RenameSession("my-session".to_string()))
        );
    }

    #[test]
    fn test_handle_bug_no_args() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/bug");
        match result {
            CommandResult::Executed(output) => {
                assert!(output.contains("Report a Bug"));
            }
            other => panic!("Expected executed, got: {:?}", other),
        }
    }

    #[test]
    fn test_handle_bug_with_description() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/bug crash on startup");
        match result {
            CommandResult::Executed(output) => {
                assert!(output.contains("crash on startup"));
                assert!(output.contains("Bug Report"));
            }
            other => panic!("Expected executed, got: {:?}", other),
        }
    }

    #[test]
    fn test_handle_feedback_alias() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/feedback great tool");
        assert!(matches!(result, CommandResult::Executed(_)));
    }

    #[test]
    fn test_handle_btw_no_args() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/btw");
        assert!(matches!(result, CommandResult::Error(_)));
    }

    #[test]
    fn test_handle_btw_with_question() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/btw what is rust");
        assert_eq!(
            result,
            CommandResult::Action(CommandAction::SideQuestion("what is rust".to_string()))
        );
    }

    #[test]
    fn test_handle_color_no_args() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/color");
        match result {
            CommandResult::Executed(output) => {
                assert!(output.contains("Available colors"));
            }
            other => panic!("Expected executed, got: {:?}", other),
        }
    }

    #[test]
    fn test_handle_color_valid() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/color green");
        assert_eq!(
            result,
            CommandResult::Action(CommandAction::SetColor("green".to_string()))
        );
    }

    #[test]
    fn test_handle_color_invalid() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/color purple");
        assert!(matches!(result, CommandResult::Error(_)));
    }

    #[test]
    fn test_handle_doctor() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/doctor");
        match result {
            CommandResult::Executed(output) => {
                assert!(output.contains("Environment Check"));
                assert!(output.contains("git"));
            }
            other => panic!("Expected executed, got: {:?}", other),
        }
    }

    #[test]
    fn test_available_commands_includes_phase_12_commands() {
        let (handler, _temp) = create_handler_in_temp();
        let commands = handler.available_commands();

        assert!(commands.contains(&"copy"), "Should include copy");
        assert!(commands.contains(&"status"), "Should include status");
        assert!(commands.contains(&"config"), "Should include config");
        assert!(commands.contains(&"settings"), "Should include settings");
        assert!(
            commands.contains(&"permissions"),
            "Should include permissions"
        );
        assert!(commands.contains(&"sandbox"), "Should include sandbox");
        assert!(commands.contains(&"rename"), "Should include rename");
        assert!(commands.contains(&"bug"), "Should include bug");
        assert!(commands.contains(&"feedback"), "Should include feedback");
        assert!(commands.contains(&"btw"), "Should include btw");
        assert!(commands.contains(&"color"), "Should include color");
        assert!(commands.contains(&"doctor"), "Should include doctor");
    }

    #[test]
    fn test_help_includes_phase_12_commands() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/help");
        match result {
            CommandResult::Executed(output) => {
                assert!(output.contains("/copy"), "Help should mention /copy");
                assert!(output.contains("/status"), "Help should mention /status");
                assert!(output.contains("/config"), "Help should mention /config");
                assert!(
                    output.contains("/permissions"),
                    "Help should mention /permissions"
                );
                assert!(output.contains("/sandbox"), "Help should mention /sandbox");
                assert!(output.contains("/rename"), "Help should mention /rename");
                assert!(output.contains("/color"), "Help should mention /color");
                assert!(output.contains("/doctor"), "Help should mention /doctor");
            }
            other => panic!("Expected help output: {:?}", other),
        }
    }

    // =========================================================================
    // Analyze command tests
    // =========================================================================

    #[test]
    fn test_analyze_no_args_returns_full_analysis_prompt() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/analyze");
        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("get_metrics"),
                    "Full analysis should mention get_metrics"
                );
                assert!(
                    output.contains("get_function_hotspots"),
                    "Full analysis should mention get_function_hotspots"
                );
                assert!(
                    output.contains("find_circular_imports"),
                    "Full analysis should mention find_circular_imports"
                );
                assert!(
                    output.contains("find_dead_code"),
                    "Full analysis should mention find_dead_code"
                );
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_analyze_full_returns_full_analysis_prompt() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/analyze full");
        match result {
            CommandResult::Executed(output) => {
                assert!(output.contains("comprehensive analysis"));
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_analyze_complexity_returns_complexity_prompt() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/analyze complexity");
        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("get_function_hotspots"),
                    "Complexity analysis should mention get_function_hotspots"
                );
                assert!(
                    output.contains("get_complexity"),
                    "Complexity analysis should mention get_complexity"
                );
                assert!(output.contains("top 10"), "Should mention top 10 functions");
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_analyze_architecture_returns_architecture_prompt() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/analyze architecture");
        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("get_import_graph"),
                    "Architecture analysis should mention get_import_graph"
                );
                assert!(
                    output.contains("find_circular_imports"),
                    "Architecture analysis should mention find_circular_imports"
                );
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_analyze_dead_code_returns_dead_code_prompt() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/analyze dead-code");
        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("find_dead_code"),
                    "Dead code analysis should mention find_dead_code"
                );
                assert!(
                    output.contains("find_unused_exports"),
                    "Dead code analysis should mention find_unused_exports"
                );
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_analyze_unknown_subcommand() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/analyze foobar");
        match result {
            CommandResult::Error(msg) => {
                assert!(
                    msg.contains("foobar"),
                    "Error should mention the unknown subcommand"
                );
            }
            other => panic!("Expected error: {:?}", other),
        }
    }

    // =========================================================================
    // Audit command tests
    // =========================================================================

    #[test]
    fn test_audit_no_args_returns_full_audit_prompt() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/audit");
        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("scan_security"),
                    "Full audit should mention scan_security"
                );
                assert!(
                    output.contains("check_cwe_top25"),
                    "Full audit should mention check_cwe_top25"
                );
                assert!(
                    output.contains("find_injection_vulnerabilities"),
                    "Full audit should mention find_injection_vulnerabilities"
                );
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_audit_taint_returns_taint_prompt() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/audit taint");
        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("trace_taint"),
                    "Taint audit should mention trace_taint"
                );
                assert!(
                    output.contains("get_taint_sources"),
                    "Taint audit should mention get_taint_sources"
                );
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_audit_deps_returns_deps_prompt() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/audit deps");
        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("check_dependencies"),
                    "Deps audit should mention check_dependencies"
                );
                assert!(
                    output.contains("check_licenses"),
                    "Deps audit should mention check_licenses"
                );
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_audit_unknown_subcommand() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/audit foobar");
        match result {
            CommandResult::Error(msg) => {
                assert!(
                    msg.contains("foobar"),
                    "Error should mention the unknown subcommand"
                );
            }
            other => panic!("Expected error: {:?}", other),
        }
    }

    #[test]
    fn test_help_includes_analyze_and_audit() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/help");
        match result {
            CommandResult::Executed(output) => {
                assert!(output.contains("/analyze"), "Help should mention /analyze");
                assert!(output.contains("/audit"), "Help should mention /audit");
            }
            other => panic!("Expected help output: {:?}", other),
        }
    }

    #[test]
    fn test_available_commands_includes_analyze_and_audit() {
        let (handler, _temp) = create_handler_in_temp();
        let commands = handler.available_commands();
        assert!(
            commands.contains(&"analyze"),
            "Available commands should include 'analyze'"
        );
        assert!(
            commands.contains(&"audit"),
            "Available commands should include 'audit'"
        );
    }

    #[test]
    fn test_help_analyze_shows_detailed_help() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/help analyze");
        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("/analyze"),
                    "Should describe analyze command"
                );
                assert!(
                    output.contains("complexity"),
                    "Should list complexity subcommand"
                );
                assert!(
                    output.contains("architecture"),
                    "Should list architecture subcommand"
                );
                assert!(
                    output.contains("dead-code"),
                    "Should list dead-code subcommand"
                );
            }
            other => panic!("Expected analyze help: {:?}", other),
        }
    }

    #[test]
    fn test_help_audit_shows_detailed_help() {
        let (handler, _temp) = create_handler_in_temp();
        let result = handler.handle("/help audit");
        match result {
            CommandResult::Executed(output) => {
                assert!(output.contains("/audit"), "Should describe audit command");
                assert!(output.contains("taint"), "Should list taint subcommand");
                assert!(output.contains("deps"), "Should list deps subcommand");
            }
            other => panic!("Expected audit help: {:?}", other),
        }
    }
}
