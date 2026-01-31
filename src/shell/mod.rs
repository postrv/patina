//! Cross-platform shell abstraction layer.
//!
//! This module provides platform-agnostic shell execution capabilities,
//! abstracting differences between Unix (`sh -c`) and Windows (`cmd.exe /C`).
//!
//! # Examples
//!
//! ```
//! use patina::shell::ShellConfig;
//!
//! let config = ShellConfig::default();
//! // On Unix: command = "sh", args = ["-c"]
//! // On Windows: command = "cmd.exe", args = ["/C"]
//! ```

#[cfg(windows)]
use once_cell::sync::Lazy;
#[cfg(windows)]
use regex::Regex;
use std::io;
use std::process::{Command, Stdio};
use tokio::io::AsyncWriteExt;
use tokio::process::Command as TokioCommand;

/// Configuration for platform-specific shell execution.
///
/// # Examples
///
/// ```
/// use patina::shell::ShellConfig;
/// use std::process::Command;
///
/// let config = ShellConfig::default();
/// let mut cmd = Command::new(&config.command);
/// for arg in &config.args {
///     cmd.arg(arg);
/// }
/// cmd.arg("echo hello");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct ShellConfig {
    /// The shell executable (e.g., "sh" or "cmd.exe").
    pub command: String,
    /// Arguments to pass before the command string (e.g., ["-c"] or ["/C"]).
    pub args: Vec<String>,
    /// Exit code indicating success (typically 0).
    pub exit_success: i32,
}

#[cfg(unix)]
impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            command: "sh".to_string(),
            args: vec!["-c".to_string()],
            exit_success: 0,
        }
    }
}

#[cfg(windows)]
impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            command: "cmd.exe".to_string(),
            args: vec!["/C".to_string()],
            exit_success: 0,
        }
    }
}

impl ShellConfig {
    /// Creates a new `Command` configured with this shell's executable and arguments.
    ///
    /// # Arguments
    ///
    /// * `script` - The shell script or command to execute
    ///
    /// # Returns
    ///
    /// A `Command` ready for further configuration (e.g., setting stdin, stdout, env).
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::shell::ShellConfig;
    ///
    /// let config = ShellConfig::default();
    /// let cmd = config.build_command("echo hello");
    /// // cmd is now ready to spawn
    /// ```
    #[must_use]
    pub fn build_command(&self, script: &str) -> Command {
        let mut cmd = Command::new(&self.command);
        for arg in &self.args {
            cmd.arg(arg);
        }
        cmd.arg(script);
        cmd
    }
}

/// Output from a shell command execution.
///
/// # Examples
///
/// ```
/// use patina::shell::ShellOutput;
///
/// let output = ShellOutput {
///     exit_code: 0,
///     stdout: "hello\n".to_string(),
///     stderr: String::new(),
/// };
/// assert!(output.success());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellOutput {
    /// The exit code of the process.
    pub exit_code: i32,
    /// Standard output captured from the process.
    pub stdout: String,
    /// Standard error captured from the process.
    pub stderr: String,
}

impl ShellOutput {
    /// Returns `true` if the command exited successfully (exit code 0).
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::shell::ShellOutput;
    ///
    /// let success = ShellOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
    /// assert!(success.success());
    ///
    /// let failure = ShellOutput { exit_code: 1, stdout: String::new(), stderr: String::new() };
    /// assert!(!failure.success());
    /// ```
    #[must_use]
    pub fn success(&self) -> bool {
        self.exit_code == 0
    }
}

/// Executes a shell command asynchronously using the platform's default shell.
///
/// This function provides cross-platform shell command execution, automatically
/// using `sh -c` on Unix and `cmd.exe /C` on Windows.
///
/// # Arguments
///
/// * `command` - The shell command to execute
/// * `stdin` - Optional input to write to the command's stdin
///
/// # Returns
///
/// Returns a `ShellOutput` containing the exit code, stdout, and stderr.
///
/// # Errors
///
/// Returns an `io::Error` if the command fails to spawn or if there's an I/O error
/// during stdin/stdout handling.
///
/// # Examples
///
/// ```no_run
/// use patina::shell::execute_shell_command;
///
/// # async fn example() -> std::io::Result<()> {
/// let output = execute_shell_command("echo hello", None).await?;
/// assert!(output.success());
/// assert!(output.stdout.contains("hello"));
/// # Ok(())
/// # }
/// ```
pub async fn execute_shell_command(command: &str, stdin: Option<&str>) -> io::Result<ShellOutput> {
    let config = ShellConfig::default();

    let mut child = TokioCommand::new(&config.command)
        .args(&config.args)
        .arg(command)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // Write stdin if provided
    if let Some(input) = stdin {
        if let Some(mut stdin_handle) = child.stdin.take() {
            stdin_handle.write_all(input.as_bytes()).await?;
            // Drop the handle to close stdin, signaling EOF to the process
        }
    }

    let output = child.wait_with_output().await?;

    Ok(ShellOutput {
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

/// Translates shell commands from Unix syntax to the current platform's syntax.
///
/// On Unix, this function returns the command unchanged. On Windows, it translates
/// common Unix shell patterns to their Windows `cmd.exe` equivalents:
///
/// - `exit N` → `exit /b N` (batch file exit syntax)
/// - `export VAR=val` → `set VAR=val` (environment variable syntax)
///
/// # Arguments
///
/// * `command` - The shell command to translate
///
/// # Returns
///
/// The translated command string appropriate for the current platform.
///
/// # Examples
///
/// ```
/// use patina::shell::translate_command;
///
/// let cmd = translate_command("echo hello");
/// // Returns "echo hello" on all platforms
///
/// let cmd = translate_command("exit 1");
/// // Returns "exit 1" on Unix, "exit /b 1" on Windows
/// ```
#[must_use]
pub fn translate_command(command: &str) -> String {
    #[cfg(unix)]
    {
        command.to_string()
    }

    #[cfg(windows)]
    {
        translate_command_for_windows(command)
    }
}

/// Regex for matching `exit N` commands.
#[cfg(windows)]
static EXIT_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\bexit\s+(\d+)").expect("exit regex should compile"));

/// Regex for matching `export VAR=val` commands.
#[cfg(windows)]
static EXPORT_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\bexport\s+(\w+)=").expect("export regex should compile"));

/// Translates Unix shell command patterns to Windows cmd.exe equivalents.
#[cfg(windows)]
fn translate_command_for_windows(command: &str) -> String {
    let mut result = command.to_string();

    // Translate `exit N` to `exit /b N`
    result = EXIT_REGEX.replace_all(&result, "exit /b $1").to_string();

    // Translate `export VAR=` to `set VAR=`
    result = EXPORT_REGEX.replace_all(&result, "set $1=").to_string();

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shell_config_build_command() {
        let config = ShellConfig::default();
        let cmd = config.build_command("echo test");

        // The command should have the shell as its program
        assert_eq!(cmd.get_program().to_str().unwrap(), &config.command);
    }

    #[test]
    fn test_shell_output_success_returns_true_for_zero_exit() {
        let output = ShellOutput {
            exit_code: 0,
            stdout: String::new(),
            stderr: String::new(),
        };
        assert!(output.success());
    }

    #[test]
    fn test_shell_output_success_returns_false_for_nonzero_exit() {
        let output = ShellOutput {
            exit_code: 1,
            stdout: String::new(),
            stderr: String::new(),
        };
        assert!(!output.success());
    }
}
