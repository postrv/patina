//! Experiment slash command parsing and execution.
//!
//! This module provides parsing for `/experiment` commands that manage
//! experiment-based development workflows with isolated worktrees.

use thiserror::Error;

/// Parsed experiment command.
///
/// Represents the different operations that can be performed on experiments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExperimentCommand {
    /// Start a new experiment with the given name.
    ///
    /// Creates an isolated worktree for the experiment based on the current branch.
    Start {
        /// Name for the new experiment.
        name: String,
    },

    /// List all experiments in the repository.
    List,

    /// Accept an experiment, merging changes back to the original branch.
    Accept {
        /// Name of the experiment to accept.
        name: String,
    },

    /// Reject an experiment, discarding all changes.
    Reject {
        /// Name of the experiment to reject.
        name: String,
    },

    /// Pause an active experiment.
    Pause {
        /// Name of the experiment to pause.
        name: String,
    },

    /// Resume a paused experiment.
    Resume {
        /// Name of the experiment to resume.
        name: String,
    },

    /// Show status of a specific experiment.
    Status {
        /// Name of the experiment to inspect.
        name: String,
    },
}

/// Errors that can occur when parsing experiment commands.
#[derive(Debug, Error)]
pub enum ExperimentCommandError {
    /// No subcommand was provided.
    #[error(
        "no subcommand provided. Usage: /experiment <start|list|accept|reject|pause|resume|status>"
    )]
    NoSubcommand,

    /// An unknown subcommand was provided.
    #[error(
        "unknown subcommand '{0}'. Valid subcommands: start, list, accept, reject, pause, resume, status"
    )]
    UnknownSubcommand(String),

    /// A required argument is missing.
    #[error("missing required argument '{0}' for subcommand '{1}'")]
    MissingArgument(String, String),
}

/// Parses an experiment command string into an `ExperimentCommand`.
///
/// The input string should be the arguments after `/experiment`, e.g.,
/// `"start my-experiment"` for `/experiment start my-experiment`.
///
/// # Errors
///
/// Returns `ExperimentCommandError` if:
/// - No subcommand is provided (empty input)
/// - An unknown subcommand is used
/// - A required argument is missing
///
/// # Examples
///
/// ```
/// use patina::commands::experiment::{parse_experiment_command, ExperimentCommand};
///
/// let cmd = parse_experiment_command("start risky-refactor").unwrap();
/// assert!(matches!(cmd, ExperimentCommand::Start { name } if name == "risky-refactor"));
///
/// let cmd = parse_experiment_command("list").unwrap();
/// assert!(matches!(cmd, ExperimentCommand::List));
/// ```
pub fn parse_experiment_command(input: &str) -> Result<ExperimentCommand, ExperimentCommandError> {
    let trimmed = input.trim();

    if trimmed.is_empty() {
        return Err(ExperimentCommandError::NoSubcommand);
    }

    let mut parts = trimmed.split_whitespace();
    let subcommand = parts.next().unwrap(); // Safe because we checked for empty

    match subcommand {
        "start" => {
            let name = parts.next().ok_or_else(|| {
                ExperimentCommandError::MissingArgument("name".to_string(), "start".to_string())
            })?;
            Ok(ExperimentCommand::Start {
                name: name.to_string(),
            })
        }

        "list" => Ok(ExperimentCommand::List),

        "accept" => {
            let name = parts.next().ok_or_else(|| {
                ExperimentCommandError::MissingArgument("name".to_string(), "accept".to_string())
            })?;
            Ok(ExperimentCommand::Accept {
                name: name.to_string(),
            })
        }

        "reject" => {
            let name = parts.next().ok_or_else(|| {
                ExperimentCommandError::MissingArgument("name".to_string(), "reject".to_string())
            })?;
            Ok(ExperimentCommand::Reject {
                name: name.to_string(),
            })
        }

        "pause" => {
            let name = parts.next().ok_or_else(|| {
                ExperimentCommandError::MissingArgument("name".to_string(), "pause".to_string())
            })?;
            Ok(ExperimentCommand::Pause {
                name: name.to_string(),
            })
        }

        "resume" => {
            let name = parts.next().ok_or_else(|| {
                ExperimentCommandError::MissingArgument("name".to_string(), "resume".to_string())
            })?;
            Ok(ExperimentCommand::Resume {
                name: name.to_string(),
            })
        }

        "status" => {
            let name = parts.next().ok_or_else(|| {
                ExperimentCommandError::MissingArgument("name".to_string(), "status".to_string())
            })?;
            Ok(ExperimentCommand::Status {
                name: name.to_string(),
            })
        }

        _ => Err(ExperimentCommandError::UnknownSubcommand(
            subcommand.to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_experiment_start() {
        let cmd = parse_experiment_command("start risky-refactor").unwrap();
        assert_eq!(
            cmd,
            ExperimentCommand::Start {
                name: "risky-refactor".to_string()
            }
        );
    }

    #[test]
    fn test_parse_experiment_start_missing_name() {
        let err = parse_experiment_command("start").unwrap_err();
        assert!(err.to_string().contains("name"));
        assert!(err.to_string().contains("start"));
    }

    #[test]
    fn test_parse_experiment_list() {
        let cmd = parse_experiment_command("list").unwrap();
        assert_eq!(cmd, ExperimentCommand::List);
    }

    #[test]
    fn test_parse_experiment_accept() {
        let cmd = parse_experiment_command("accept my-exp").unwrap();
        assert_eq!(
            cmd,
            ExperimentCommand::Accept {
                name: "my-exp".to_string()
            }
        );
    }

    #[test]
    fn test_parse_experiment_reject() {
        let cmd = parse_experiment_command("reject my-exp").unwrap();
        assert_eq!(
            cmd,
            ExperimentCommand::Reject {
                name: "my-exp".to_string()
            }
        );
    }

    #[test]
    fn test_parse_experiment_pause() {
        let cmd = parse_experiment_command("pause my-exp").unwrap();
        assert_eq!(
            cmd,
            ExperimentCommand::Pause {
                name: "my-exp".to_string()
            }
        );
    }

    #[test]
    fn test_parse_experiment_resume() {
        let cmd = parse_experiment_command("resume my-exp").unwrap();
        assert_eq!(
            cmd,
            ExperimentCommand::Resume {
                name: "my-exp".to_string()
            }
        );
    }

    #[test]
    fn test_parse_experiment_status() {
        let cmd = parse_experiment_command("status my-exp").unwrap();
        assert_eq!(
            cmd,
            ExperimentCommand::Status {
                name: "my-exp".to_string()
            }
        );
    }

    #[test]
    fn test_parse_experiment_unknown_subcommand() {
        let err = parse_experiment_command("destroy my-exp").unwrap_err();
        assert!(err.to_string().contains("destroy"));
        assert!(err.to_string().contains("unknown subcommand"));
    }

    #[test]
    fn test_parse_experiment_no_subcommand() {
        let err = parse_experiment_command("").unwrap_err();
        assert!(err.to_string().contains("no subcommand"));
    }

    #[test]
    fn test_parse_experiment_whitespace_only() {
        let err = parse_experiment_command("   ").unwrap_err();
        assert!(err.to_string().contains("no subcommand"));
    }

    #[test]
    fn test_command_debug() {
        let cmd = ExperimentCommand::Start {
            name: "test".to_string(),
        };
        assert!(format!("{:?}", cmd).contains("Start"));
    }

    #[test]
    fn test_error_display() {
        let err = ExperimentCommandError::NoSubcommand;
        assert!(err.to_string().contains("no subcommand"));

        let err = ExperimentCommandError::UnknownSubcommand("foo".to_string());
        assert!(err.to_string().contains("foo"));

        let err = ExperimentCommandError::MissingArgument("name".to_string(), "start".to_string());
        assert!(err.to_string().contains("name"));
    }
}
