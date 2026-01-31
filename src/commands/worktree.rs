//! Worktree slash command parsing and execution.
//!
//! This module provides parsing for `/worktree` commands that manage
//! git worktrees for parallel development workflows.

use thiserror::Error;

/// Parsed worktree command.
///
/// Represents the different operations that can be performed on worktrees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeCommand {
    /// Create a new worktree with the given name.
    ///
    /// Creates a new branch with the configured prefix and checks it out
    /// in a new worktree directory.
    New {
        /// Name for the new worktree (used for both directory and branch).
        name: String,
    },

    /// List all worktrees in the repository.
    List,

    /// Switch to an existing worktree.
    Switch {
        /// Name of the worktree to switch to.
        name: String,
    },

    /// Remove an existing worktree.
    Remove {
        /// Name of the worktree to remove.
        name: String,
    },

    /// Clean up prunable worktrees.
    ///
    /// Removes worktree entries whose directories no longer exist.
    Clean,

    /// Show status of worktrees.
    ///
    /// Displays modified/staged/untracked counts and ahead/behind status.
    Status,
}

/// Errors that can occur when parsing worktree commands.
#[derive(Debug, Error)]
pub enum WorktreeCommandError {
    /// No subcommand was provided.
    #[error("no subcommand provided. Usage: /worktree <new|list|switch|remove|clean|status>")]
    NoSubcommand,

    /// An unknown subcommand was provided.
    #[error(
        "unknown subcommand '{0}'. Valid subcommands: new, list, switch, remove, clean, status"
    )]
    UnknownSubcommand(String),

    /// A required argument is missing.
    #[error("missing required argument '{0}' for subcommand '{1}'")]
    MissingArgument(String, String),
}

/// Parses a worktree command string into a `WorktreeCommand`.
///
/// The input string should be the arguments after `/worktree`, e.g.,
/// `"new my-feature"` for `/worktree new my-feature`.
///
/// # Errors
///
/// Returns `WorktreeCommandError` if:
/// - No subcommand is provided (empty input)
/// - An unknown subcommand is used
/// - A required argument is missing
///
/// # Examples
///
/// ```
/// use patina::commands::worktree::{parse_worktree_command, WorktreeCommand};
///
/// let cmd = parse_worktree_command("new my-feature").unwrap();
/// assert!(matches!(cmd, WorktreeCommand::New { name } if name == "my-feature"));
///
/// let cmd = parse_worktree_command("list").unwrap();
/// assert!(matches!(cmd, WorktreeCommand::List));
/// ```
pub fn parse_worktree_command(input: &str) -> Result<WorktreeCommand, WorktreeCommandError> {
    let trimmed = input.trim();

    if trimmed.is_empty() {
        return Err(WorktreeCommandError::NoSubcommand);
    }

    let mut parts = trimmed.split_whitespace();
    let subcommand = parts.next().unwrap(); // Safe because we checked for empty

    match subcommand {
        "new" => {
            let name = parts.next().ok_or_else(|| {
                WorktreeCommandError::MissingArgument("name".to_string(), "new".to_string())
            })?;
            Ok(WorktreeCommand::New {
                name: name.to_string(),
            })
        }

        "list" => Ok(WorktreeCommand::List),

        "switch" => {
            let name = parts.next().ok_or_else(|| {
                WorktreeCommandError::MissingArgument("name".to_string(), "switch".to_string())
            })?;
            Ok(WorktreeCommand::Switch {
                name: name.to_string(),
            })
        }

        "remove" => {
            let name = parts.next().ok_or_else(|| {
                WorktreeCommandError::MissingArgument("name".to_string(), "remove".to_string())
            })?;
            Ok(WorktreeCommand::Remove {
                name: name.to_string(),
            })
        }

        "clean" => Ok(WorktreeCommand::Clean),

        "status" => Ok(WorktreeCommand::Status),

        _ => Err(WorktreeCommandError::UnknownSubcommand(
            subcommand.to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_debug() {
        let cmd = WorktreeCommand::New {
            name: "test".to_string(),
        };
        assert!(format!("{:?}", cmd).contains("New"));
    }

    #[test]
    fn test_error_display() {
        let err = WorktreeCommandError::NoSubcommand;
        assert!(err.to_string().contains("no subcommand"));

        let err = WorktreeCommandError::UnknownSubcommand("foo".to_string());
        assert!(err.to_string().contains("foo"));

        let err = WorktreeCommandError::MissingArgument("name".to_string(), "new".to_string());
        assert!(err.to_string().contains("name"));
    }
}
