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
//! }
//! ```

use crate::commands::worktree::{parse_worktree_command, WorktreeCommand};
use crate::worktree::WorktreeManager;
use std::path::PathBuf;

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
}

/// Handler for slash commands in the TUI.
///
/// Parses user input, identifies slash commands, and dispatches to the
/// appropriate handler.
pub struct SlashCommandHandler {
    /// Working directory for command execution.
    working_dir: PathBuf,
}

impl SlashCommandHandler {
    /// Creates a new slash command handler.
    #[must_use]
    pub fn new(working_dir: PathBuf) -> Self {
        Self { working_dir }
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
            "worktree" => self.handle_worktree(&args),
            "help" => self.handle_help(if args.is_empty() { None } else { Some(&args) }),
            _ => CommandResult::UnknownCommand(command_name.to_string()),
        }
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
            Err(e) => return CommandResult::Error(format!("Failed to initialize worktree manager: {}", e)),
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
                Ok(worktrees) => {
                    if worktrees.is_empty() {
                        CommandResult::Executed("No worktrees found.".to_string())
                    } else {
                        let output = worktrees
                            .iter()
                            .map(|wt| {
                                let branch = if wt.branch.is_empty() {
                                    "detached"
                                } else {
                                    &wt.branch
                                };
                                format!(
                                    "  {} ({}) - {}",
                                    wt.name,
                                    branch,
                                    wt.path.display()
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        CommandResult::Executed(format!("Worktrees:\n{}", output))
                    }
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
                    Err(e) => CommandResult::Error(format!("Failed to run git worktree prune: {}", e)),
                }
            }

            WorktreeCommand::Status => {
                // Status showing all worktrees with their git status
                match manager.list() {
                    Ok(worktrees) => {
                        if worktrees.is_empty() {
                            CommandResult::Executed("No worktrees found.".to_string())
                        } else {
                            let output = worktrees
                                .iter()
                                .map(|wt| {
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
                                })
                                .collect::<Vec<_>>()
                                .join("\n\n");
                            CommandResult::Executed(format!("Worktree Status:\n{}", output))
                        }
                    }
                    Err(e) => CommandResult::Error(format!("Failed to get worktree status: {}", e)),
                }
            }
        }
    }

    /// Handles the `/help` command.
    fn handle_help(&self, command: Option<&str>) -> CommandResult {
        match command {
            None => {
                // General help listing all commands
                let help_text = r#"Available Commands:

  /worktree <subcommand>  - Manage git worktrees
    Subcommands: new, list, switch, remove, clean, status

  /help [command]         - Show help for a command

Type /help <command> for detailed help on a specific command."#;
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

            Some(cmd) => CommandResult::UnknownCommand(cmd.to_string()),
        }
    }

    /// Returns available command names for tab completion.
    #[must_use]
    pub fn available_commands(&self) -> Vec<&'static str> {
        vec!["worktree", "help"]
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
}
