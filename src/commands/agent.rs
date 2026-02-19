//! Agent slash command parsing.
//!
//! This module provides parsing for `/agent` commands that manage
//! worktree-based agents for parallel task execution.

use thiserror::Error;

/// Parsed agent command.
///
/// Represents the different operations that can be performed on agents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentCommand {
    /// Spawn a new agent with the given name and task.
    ///
    /// Creates a new worktree agent that works on the given task
    /// in an isolated git worktree.
    New {
        /// Name for the new agent.
        name: String,
        /// Task description for the agent to work on.
        task: String,
    },

    /// List all agents and their current status.
    List,

    /// Show detailed status of a specific agent.
    Status {
        /// Name of the agent to query.
        name: String,
    },

    /// Merge an agent's changes back into the current branch.
    ///
    /// Performs conflict detection before merging to warn about
    /// overlapping changes from multiple agents.
    Merge {
        /// Name of the agent whose changes to merge.
        name: String,
    },

    /// Stop a running agent.
    Stop {
        /// Name of the agent to stop.
        name: String,
    },
}

/// Errors that can occur when parsing agent commands.
#[derive(Debug, Error)]
pub enum AgentCommandError {
    /// No subcommand was provided.
    #[error("no subcommand provided. Usage: /agent <new|list|status|merge|stop>")]
    NoSubcommand,

    /// An unknown subcommand was provided.
    #[error("unknown subcommand '{0}'. Valid subcommands: new, list, status, merge, stop")]
    UnknownSubcommand(String),

    /// A required argument is missing.
    #[error("missing required argument '{0}' for subcommand '{1}'")]
    MissingArgument(String, String),
}

/// Parses an agent command string into an [`AgentCommand`].
///
/// The input string should be the arguments after `/agent`, e.g.,
/// `"new my-agent Implement login form"` for `/agent new my-agent Implement login form`.
///
/// # Errors
///
/// Returns [`AgentCommandError`] if:
/// - No subcommand is provided (empty input)
/// - An unknown subcommand is used
/// - A required argument is missing
///
/// # Examples
///
/// ```
/// use patina::commands::agent::{parse_agent_command, AgentCommand};
///
/// let cmd = parse_agent_command("new my-agent Fix the login bug").unwrap();
/// assert!(matches!(cmd, AgentCommand::New { name, task }
///     if name == "my-agent" && task == "Fix the login bug"));
///
/// let cmd = parse_agent_command("list").unwrap();
/// assert!(matches!(cmd, AgentCommand::List));
///
/// let cmd = parse_agent_command("status my-agent").unwrap();
/// assert!(matches!(cmd, AgentCommand::Status { name } if name == "my-agent"));
/// ```
pub fn parse_agent_command(input: &str) -> Result<AgentCommand, AgentCommandError> {
    let trimmed = input.trim();

    if trimmed.is_empty() {
        return Err(AgentCommandError::NoSubcommand);
    }

    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let subcommand = parts.next().unwrap(); // Safe because we checked for empty
    let rest = parts.next().unwrap_or("").trim();

    match subcommand {
        "new" => {
            if rest.is_empty() {
                return Err(AgentCommandError::MissingArgument(
                    "name".to_string(),
                    "new".to_string(),
                ));
            }

            // Split rest into name (first word) and task (remainder)
            let mut name_and_task = rest.splitn(2, char::is_whitespace);
            let name = name_and_task.next().unwrap().to_string();
            let task = name_and_task.next().unwrap_or("").trim();

            if task.is_empty() {
                return Err(AgentCommandError::MissingArgument(
                    "task".to_string(),
                    "new".to_string(),
                ));
            }

            Ok(AgentCommand::New {
                name,
                task: task.to_string(),
            })
        }

        "list" => Ok(AgentCommand::List),

        "status" => {
            if rest.is_empty() {
                return Err(AgentCommandError::MissingArgument(
                    "name".to_string(),
                    "status".to_string(),
                ));
            }
            let name = rest.split_whitespace().next().unwrap().to_string();
            Ok(AgentCommand::Status { name })
        }

        "merge" => {
            if rest.is_empty() {
                return Err(AgentCommandError::MissingArgument(
                    "name".to_string(),
                    "merge".to_string(),
                ));
            }
            let name = rest.split_whitespace().next().unwrap().to_string();
            Ok(AgentCommand::Merge { name })
        }

        "stop" => {
            if rest.is_empty() {
                return Err(AgentCommandError::MissingArgument(
                    "name".to_string(),
                    "stop".to_string(),
                ));
            }
            let name = rest.split_whitespace().next().unwrap().to_string();
            Ok(AgentCommand::Stop { name })
        }

        _ => Err(AgentCommandError::UnknownSubcommand(subcommand.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // parse_agent_command: new subcommand
    // =========================================================================

    #[test]
    fn test_parse_new_with_name_and_task() {
        let cmd = parse_agent_command("new my-agent Fix the login bug").unwrap();
        assert_eq!(
            cmd,
            AgentCommand::New {
                name: "my-agent".to_string(),
                task: "Fix the login bug".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_new_task_preserves_full_text() {
        let cmd = parse_agent_command("new feature-bot Implement the entire authentication system")
            .unwrap();
        assert_eq!(
            cmd,
            AgentCommand::New {
                name: "feature-bot".to_string(),
                task: "Implement the entire authentication system".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_new_missing_name() {
        let err = parse_agent_command("new").unwrap_err();
        assert!(matches!(err, AgentCommandError::MissingArgument(arg, sub)
            if arg == "name" && sub == "new"));
    }

    #[test]
    fn test_parse_new_missing_task() {
        let err = parse_agent_command("new my-agent").unwrap_err();
        assert!(matches!(err, AgentCommandError::MissingArgument(arg, sub)
            if arg == "task" && sub == "new"));
    }

    #[test]
    fn test_parse_new_name_only_with_trailing_spaces() {
        let err = parse_agent_command("new my-agent   ").unwrap_err();
        assert!(matches!(err, AgentCommandError::MissingArgument(arg, _)
            if arg == "task"));
    }

    // =========================================================================
    // parse_agent_command: list subcommand
    // =========================================================================

    #[test]
    fn test_parse_list() {
        let cmd = parse_agent_command("list").unwrap();
        assert_eq!(cmd, AgentCommand::List);
    }

    #[test]
    fn test_parse_list_ignores_extra_args() {
        let cmd = parse_agent_command("list --all").unwrap();
        assert_eq!(cmd, AgentCommand::List);
    }

    // =========================================================================
    // parse_agent_command: status subcommand
    // =========================================================================

    #[test]
    fn test_parse_status_with_name() {
        let cmd = parse_agent_command("status my-agent").unwrap();
        assert_eq!(
            cmd,
            AgentCommand::Status {
                name: "my-agent".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_status_missing_name() {
        let err = parse_agent_command("status").unwrap_err();
        assert!(matches!(err, AgentCommandError::MissingArgument(arg, sub)
            if arg == "name" && sub == "status"));
    }

    #[test]
    fn test_parse_status_takes_first_word_only() {
        let cmd = parse_agent_command("status my-agent extra-stuff").unwrap();
        assert_eq!(
            cmd,
            AgentCommand::Status {
                name: "my-agent".to_string(),
            }
        );
    }

    // =========================================================================
    // parse_agent_command: merge subcommand
    // =========================================================================

    #[test]
    fn test_parse_merge_with_name() {
        let cmd = parse_agent_command("merge my-agent").unwrap();
        assert_eq!(
            cmd,
            AgentCommand::Merge {
                name: "my-agent".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_merge_missing_name() {
        let err = parse_agent_command("merge").unwrap_err();
        assert!(matches!(err, AgentCommandError::MissingArgument(arg, sub)
            if arg == "name" && sub == "merge"));
    }

    // =========================================================================
    // parse_agent_command: stop subcommand
    // =========================================================================

    #[test]
    fn test_parse_stop_with_name() {
        let cmd = parse_agent_command("stop my-agent").unwrap();
        assert_eq!(
            cmd,
            AgentCommand::Stop {
                name: "my-agent".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_stop_missing_name() {
        let err = parse_agent_command("stop").unwrap_err();
        assert!(matches!(err, AgentCommandError::MissingArgument(arg, sub)
            if arg == "name" && sub == "stop"));
    }

    // =========================================================================
    // parse_agent_command: error cases
    // =========================================================================

    #[test]
    fn test_parse_empty_input() {
        let err = parse_agent_command("").unwrap_err();
        assert!(matches!(err, AgentCommandError::NoSubcommand));
    }

    #[test]
    fn test_parse_whitespace_only() {
        let err = parse_agent_command("   ").unwrap_err();
        assert!(matches!(err, AgentCommandError::NoSubcommand));
    }

    #[test]
    fn test_parse_unknown_subcommand() {
        let err = parse_agent_command("foobar").unwrap_err();
        assert!(matches!(err, AgentCommandError::UnknownSubcommand(s) if s == "foobar"));
    }

    // =========================================================================
    // Display/Debug trait tests
    // =========================================================================

    #[test]
    fn test_command_debug() {
        let cmd = AgentCommand::New {
            name: "test".to_string(),
            task: "Do stuff".to_string(),
        };
        assert!(format!("{:?}", cmd).contains("New"));
    }

    #[test]
    fn test_error_display_no_subcommand() {
        let err = AgentCommandError::NoSubcommand;
        let msg = err.to_string();
        assert!(msg.contains("no subcommand"));
        assert!(msg.contains("new"));
        assert!(msg.contains("list"));
        assert!(msg.contains("status"));
        assert!(msg.contains("merge"));
        assert!(msg.contains("stop"));
    }

    #[test]
    fn test_error_display_unknown_subcommand() {
        let err = AgentCommandError::UnknownSubcommand("xyz".to_string());
        let msg = err.to_string();
        assert!(msg.contains("xyz"));
        assert!(msg.contains("unknown subcommand"));
    }

    #[test]
    fn test_error_display_missing_argument() {
        let err = AgentCommandError::MissingArgument("name".to_string(), "new".to_string());
        let msg = err.to_string();
        assert!(msg.contains("name"));
        assert!(msg.contains("new"));
    }

    // =========================================================================
    // Whitespace handling
    // =========================================================================

    #[test]
    fn test_parse_leading_trailing_whitespace() {
        let cmd = parse_agent_command("  list  ").unwrap();
        assert_eq!(cmd, AgentCommand::List);
    }

    #[test]
    fn test_parse_new_with_extra_whitespace() {
        let cmd = parse_agent_command("  new   my-agent   Fix the bug  ").unwrap();
        assert_eq!(
            cmd,
            AgentCommand::New {
                name: "my-agent".to_string(),
                task: "Fix the bug".to_string(),
            }
        );
    }
}
