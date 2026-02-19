//! Continuous coding session slash command parsing.
//!
//! This module provides parsing for `/continuous` commands that manage
//! the continuous coding loop for automated task execution.

use thiserror::Error;

/// Parsed continuous command.
///
/// Represents the different operations that can be performed on the
/// continuous coding loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContinuousCommand {
    /// Start the continuous coding loop.
    ///
    /// Begins automated iteration with quality gate checks.
    Start {
        /// Maximum number of iterations before stopping.
        max_iterations: Option<u32>,
    },

    /// Stop the currently running continuous loop.
    Stop,

    /// Show current status of the continuous loop.
    Status,
}

/// Errors that can occur when parsing continuous commands.
#[derive(Debug, Error)]
pub enum ContinuousCommandError {
    /// No subcommand was provided.
    #[error("no subcommand provided. Usage: /continuous <start|stop|status>")]
    NoSubcommand,

    /// An unknown subcommand was provided.
    #[error("unknown subcommand '{0}'. Valid subcommands: start, stop, status")]
    UnknownSubcommand(String),

    /// An invalid argument value was provided.
    #[error("invalid value for '{0}': {1}")]
    InvalidArgument(String, String),
}

/// Parses a continuous command string into a [`ContinuousCommand`].
///
/// The input string should be the arguments after `/continuous`, e.g.,
/// `"start --max-iterations 50"` for `/continuous start --max-iterations 50`.
///
/// # Errors
///
/// Returns [`ContinuousCommandError`] if:
/// - No subcommand is provided (empty input)
/// - An unknown subcommand is used
/// - An invalid argument value is provided
///
/// # Examples
///
/// ```
/// use patina::commands::continuous::{parse_continuous_command, ContinuousCommand};
///
/// let cmd = parse_continuous_command("start").unwrap();
/// assert!(matches!(cmd, ContinuousCommand::Start { max_iterations: None }));
///
/// let cmd = parse_continuous_command("start 50").unwrap();
/// assert!(matches!(cmd, ContinuousCommand::Start { max_iterations: Some(50) }));
///
/// let cmd = parse_continuous_command("stop").unwrap();
/// assert!(matches!(cmd, ContinuousCommand::Stop));
///
/// let cmd = parse_continuous_command("status").unwrap();
/// assert!(matches!(cmd, ContinuousCommand::Status));
/// ```
pub fn parse_continuous_command(input: &str) -> Result<ContinuousCommand, ContinuousCommandError> {
    let trimmed = input.trim();

    if trimmed.is_empty() {
        return Err(ContinuousCommandError::NoSubcommand);
    }

    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let subcommand = parts.next().unwrap(); // Safe because we checked for empty
    let rest = parts.next().unwrap_or("").trim();

    match subcommand {
        "start" => {
            let max_iterations = if rest.is_empty() {
                None
            } else {
                let value = rest.parse::<u32>().map_err(|_| {
                    ContinuousCommandError::InvalidArgument(
                        "max_iterations".to_string(),
                        format!("'{rest}' is not a valid positive number"),
                    )
                })?;
                Some(value)
            };
            Ok(ContinuousCommand::Start { max_iterations })
        }

        "stop" => Ok(ContinuousCommand::Stop),

        "status" => Ok(ContinuousCommand::Status),

        _ => Err(ContinuousCommandError::UnknownSubcommand(
            subcommand.to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // parse_continuous_command: start subcommand
    // =========================================================================

    #[test]
    fn parse_start_no_args() {
        let cmd = parse_continuous_command("start").unwrap();
        assert_eq!(
            cmd,
            ContinuousCommand::Start {
                max_iterations: None
            }
        );
    }

    #[test]
    fn parse_start_with_max_iterations() {
        let cmd = parse_continuous_command("start 50").unwrap();
        assert_eq!(
            cmd,
            ContinuousCommand::Start {
                max_iterations: Some(50)
            }
        );
    }

    #[test]
    fn parse_start_invalid_max_iterations() {
        let err = parse_continuous_command("start abc").unwrap_err();
        assert!(
            matches!(err, ContinuousCommandError::InvalidArgument(arg, _) if arg == "max_iterations")
        );
    }

    #[test]
    fn parse_start_negative_number() {
        let err = parse_continuous_command("start -5").unwrap_err();
        assert!(matches!(err, ContinuousCommandError::InvalidArgument(_, _)));
    }

    // =========================================================================
    // parse_continuous_command: stop subcommand
    // =========================================================================

    #[test]
    fn parse_stop() {
        let cmd = parse_continuous_command("stop").unwrap();
        assert_eq!(cmd, ContinuousCommand::Stop);
    }

    #[test]
    fn parse_stop_ignores_extra_args() {
        let cmd = parse_continuous_command("stop now").unwrap();
        assert_eq!(cmd, ContinuousCommand::Stop);
    }

    // =========================================================================
    // parse_continuous_command: status subcommand
    // =========================================================================

    #[test]
    fn parse_status() {
        let cmd = parse_continuous_command("status").unwrap();
        assert_eq!(cmd, ContinuousCommand::Status);
    }

    #[test]
    fn parse_status_ignores_extra_args() {
        let cmd = parse_continuous_command("status verbose").unwrap();
        assert_eq!(cmd, ContinuousCommand::Status);
    }

    // =========================================================================
    // parse_continuous_command: error cases
    // =========================================================================

    #[test]
    fn parse_empty_input() {
        let err = parse_continuous_command("").unwrap_err();
        assert!(matches!(err, ContinuousCommandError::NoSubcommand));
    }

    #[test]
    fn parse_whitespace_only() {
        let err = parse_continuous_command("   ").unwrap_err();
        assert!(matches!(err, ContinuousCommandError::NoSubcommand));
    }

    #[test]
    fn parse_unknown_subcommand() {
        let err = parse_continuous_command("foobar").unwrap_err();
        assert!(matches!(err, ContinuousCommandError::UnknownSubcommand(s) if s == "foobar"));
    }

    // =========================================================================
    // Whitespace handling
    // =========================================================================

    #[test]
    fn parse_leading_trailing_whitespace() {
        let cmd = parse_continuous_command("  start  ").unwrap();
        assert_eq!(
            cmd,
            ContinuousCommand::Start {
                max_iterations: None
            }
        );
    }

    #[test]
    fn parse_extra_whitespace_around_args() {
        let cmd = parse_continuous_command("  start   50  ").unwrap();
        assert_eq!(
            cmd,
            ContinuousCommand::Start {
                max_iterations: Some(50)
            }
        );
    }

    // =========================================================================
    // Display/Debug trait tests
    // =========================================================================

    #[test]
    fn command_debug() {
        let cmd = ContinuousCommand::Start {
            max_iterations: Some(10),
        };
        assert!(format!("{:?}", cmd).contains("Start"));
    }

    #[test]
    fn error_display_no_subcommand() {
        let err = ContinuousCommandError::NoSubcommand;
        let msg = err.to_string();
        assert!(msg.contains("no subcommand"));
        assert!(msg.contains("start"));
        assert!(msg.contains("stop"));
        assert!(msg.contains("status"));
    }

    #[test]
    fn error_display_unknown_subcommand() {
        let err = ContinuousCommandError::UnknownSubcommand("xyz".to_string());
        let msg = err.to_string();
        assert!(msg.contains("xyz"));
        assert!(msg.contains("unknown subcommand"));
    }

    #[test]
    fn error_display_invalid_argument() {
        let err = ContinuousCommandError::InvalidArgument(
            "max_iterations".to_string(),
            "not a number".to_string(),
        );
        let msg = err.to_string();
        assert!(msg.contains("max_iterations"));
        assert!(msg.contains("not a number"));
    }
}
