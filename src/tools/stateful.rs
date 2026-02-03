//! Stateful tool execution with persistent shell state.
//!
//! This module provides tool executors that maintain shell state (current directory,
//! environment variables) across command executions.

use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::RwLock;
use tokio::process::Command;
use tracing::{debug, warn};

use super::executor::{ToolCall, ToolExecutor, ToolResult};
use super::security::{normalize_command, ToolExecutionPolicy};
use crate::shell::ShellConfig;

/// Shell state that persists across command executions.
///
/// Tracks the current working directory and environment variables set during
/// the session. This allows `cd` and `export` commands to affect subsequent
/// commands.
#[derive(Debug)]
pub struct ShellState {
    /// Current working directory for command execution.
    cwd: PathBuf,
    /// Environment variables set during the session via export.
    env: HashMap<String, String>,
}

impl ShellState {
    /// Creates a new shell state with the given initial working directory.
    #[must_use]
    pub fn new(initial_cwd: PathBuf) -> Self {
        Self {
            cwd: initial_cwd,
            env: HashMap::new(),
        }
    }

    /// Returns the current working directory.
    #[must_use]
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// Returns the environment variables set during the session.
    #[must_use]
    pub fn env(&self) -> &HashMap<String, String> {
        &self.env
    }

    /// Processes a command and updates shell state accordingly.
    ///
    /// Parses `cd` and `export` commands to update the tracked state.
    pub fn process_command(&mut self, command: &str) {
        // Handle cd commands
        if let Some(new_dir) = Self::parse_cd(command) {
            self.update_cwd(new_dir);
        }

        // Handle export commands
        if let Some((key, value)) = Self::parse_export(command) {
            self.env.insert(key.to_string(), value.to_string());
        }
    }

    /// Updates the current working directory.
    fn update_cwd(&mut self, new_dir: &str) {
        // Use std::env::var for HOME since directories crate is for user directories
        let home_dir = || {
            std::env::var("HOME")
                .ok()
                .map(PathBuf::from)
                .or_else(|| std::env::var("USERPROFILE").ok().map(PathBuf::from))
        };

        let target = if new_dir.starts_with('/') {
            PathBuf::from(new_dir)
        } else if new_dir == "~" {
            home_dir().unwrap_or_else(|| self.cwd.clone())
        } else if let Some(rest) = new_dir.strip_prefix("~/") {
            home_dir()
                .map(|h| h.join(rest))
                .unwrap_or_else(|| self.cwd.clone())
        } else if new_dir == "-" {
            // cd - not supported without tracking previous dir
            return;
        } else {
            self.cwd.join(new_dir)
        };

        // Canonicalize if the path exists
        if let Ok(canonical) = target.canonicalize() {
            debug!(
                old_cwd = %self.cwd.display(),
                new_cwd = %canonical.display(),
                "Shell state: updated cwd"
            );
            self.cwd = canonical;
        } else if target.exists() {
            // Path exists but canonicalize failed - just use it
            self.cwd = target;
        }
        // If path doesn't exist, don't change cwd
    }

    /// Parses a `cd` command and extracts the target directory.
    fn parse_cd(command: &str) -> Option<&str> {
        let trimmed = command.trim();

        if trimmed == "cd" {
            return Some("~");
        }

        if let Some(rest) = trimmed.strip_prefix("cd ") {
            // Handle cd in compounds: "cd foo && ls" -> "foo"
            let dir = rest
                .split(['&', '|', ';'])
                .next()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());
            return dir;
        }

        None
    }

    /// Parses an `export` command and extracts the key-value pair.
    fn parse_export(command: &str) -> Option<(&str, &str)> {
        let trimmed = command.trim();

        if let Some(rest) = trimmed.strip_prefix("export ") {
            // Handle: export VAR=value or export VAR="value"
            let assignment = rest.split(['&', '|', ';']).next().map(|s| s.trim())?;

            if let Some(eq_pos) = assignment.find('=') {
                let key = assignment[..eq_pos].trim();
                let value = assignment[eq_pos + 1..]
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'');
                if !key.is_empty() {
                    return Some((key, value));
                }
            }
        }

        None
    }
}

/// Tool executor with persistent shell state.
///
/// Wraps `ToolExecutor` to track shell state (cwd, environment variables)
/// across command executions. This allows `cd` and `export` commands to
/// affect subsequent commands.
///
/// # Example
///
/// ```no_run
/// use patina::tools::{StatefulToolExecutor, ToolCall};
/// use serde_json::json;
/// use std::path::PathBuf;
///
/// # async fn example() -> anyhow::Result<()> {
/// let executor = StatefulToolExecutor::new(PathBuf::from("."));
///
/// // cd into a subdirectory
/// executor.execute(ToolCall {
///     name: "bash".to_string(),
///     input: json!({ "command": "cd subdir" }),
/// }).await?;
///
/// // Subsequent commands run in subdir
/// let result = executor.execute(ToolCall {
///     name: "bash".to_string(),
///     input: json!({ "command": "ls" }),
/// }).await?;
/// # Ok(())
/// # }
/// ```
pub struct StatefulToolExecutor {
    pub(crate) inner: ToolExecutor,
    state: RwLock<ShellState>,
}

impl StatefulToolExecutor {
    /// Creates a new stateful executor with the given working directory.
    #[must_use]
    pub fn new(working_dir: PathBuf) -> Self {
        let canonical = working_dir
            .canonicalize()
            .unwrap_or_else(|_| working_dir.clone());
        Self {
            inner: ToolExecutor::new(working_dir),
            state: RwLock::new(ShellState::new(canonical)),
        }
    }

    /// Returns the current shell state.
    ///
    /// # Panics
    ///
    /// Panics if the lock is poisoned.
    pub fn shell_state(&self) -> std::sync::RwLockReadGuard<'_, ShellState> {
        self.state.read().expect("shell state lock poisoned")
    }

    /// Sets a custom execution policy for the tool executor.
    ///
    /// # Arguments
    ///
    /// * `policy` - The new execution policy to use
    #[must_use]
    pub fn with_policy(mut self, policy: ToolExecutionPolicy) -> Self {
        self.inner = self.inner.with_policy(policy);
        self
    }

    /// Executes a tool call with persistent shell state.
    ///
    /// For bash commands:
    /// 1. Parses `cd`/`export` and updates shell state
    /// 2. Runs command in the tracked cwd with tracked env vars
    /// 3. Stores the state for subsequent commands
    pub async fn execute(&self, call: ToolCall) -> Result<ToolResult> {
        if call.name == "bash" {
            return self.execute_bash_with_state(&call.input).await;
        }

        // Non-bash tools use the inner executor directly
        self.inner.execute(call).await
    }

    /// Executes a bash command with persistent shell state.
    async fn execute_bash_with_state(&self, input: &serde_json::Value) -> Result<ToolResult> {
        let command = input
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing command"))?;

        // Check if this is a pure cd command (just changes directory, no other operation)
        let is_pure_cd = Self::is_pure_cd(command);

        // Get current shell state BEFORE processing the command
        let (effective_cwd, env_vars) = {
            let state = self.state.read().expect("shell state lock poisoned");
            (state.cwd.clone(), state.env.clone())
        };

        // For pure cd commands, update state and return success immediately
        if is_pure_cd {
            let mut state = self.state.write().expect("shell state lock poisoned");
            state.process_command(command);
            return Ok(ToolResult::Success(format!(
                "Changed directory to {}",
                state.cwd.display()
            )));
        }

        // Normalize command for security checks
        let normalized = normalize_command(command);

        // Check dangerous patterns
        for pattern in &self.inner.policy.dangerous_patterns {
            if pattern.is_match(command) || pattern.is_match(&normalized) {
                warn!(
                    pattern = %pattern.as_str(),
                    command = %command,
                    "Security violation: command blocked by dangerous pattern"
                );
                return Ok(ToolResult::Error(format!(
                    "Command blocked by security policy: matches {:?}",
                    pattern.as_str()
                )));
            }
        }

        // Check allowlist mode
        if self.inner.policy.allowlist_mode {
            let is_allowed = self
                .inner
                .policy
                .allowed_commands
                .iter()
                .any(|pattern| pattern.is_match(command) || pattern.is_match(&normalized));
            if !is_allowed {
                warn!(
                    command = %command,
                    "Security: command blocked by allowlist policy"
                );
                return Ok(ToolResult::Error(
                    "Command blocked: not in allowlist".to_string(),
                ));
            }
        }

        // Execute the command with the tracked cwd and env
        let shell = ShellConfig::default();
        let mut cmd = Command::new(&shell.command);
        cmd.args(&shell.args)
            .arg(command)
            .current_dir(&effective_cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        // Apply tracked environment variables
        for (key, value) in &env_vars {
            cmd.env(key, value);
        }

        let child = cmd.spawn()?;

        // Wait for completion with timeout
        match tokio::time::timeout(self.inner.policy.command_timeout, child.wait_with_output())
            .await
        {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let combined = format!("{}{}", stdout, stderr);

                // Truncate if needed
                let (final_output, truncated) =
                    if combined.len() > self.inner.policy.max_output_size {
                        let truncated_output = combined
                            .chars()
                            .take(self.inner.policy.max_output_size)
                            .collect::<String>();
                        (truncated_output, true)
                    } else {
                        (combined, false)
                    };

                if output.status.success() {
                    // Update shell state after successful command execution
                    // This handles compound commands like "cd foo && ls"
                    {
                        let mut state = self.state.write().expect("shell state lock poisoned");
                        state.process_command(command);
                    }

                    let result = if truncated {
                        format!(
                            "{}\n\n[Output truncated: {} bytes exceeded {} byte limit]",
                            final_output,
                            stdout.len() + stderr.len(),
                            self.inner.policy.max_output_size
                        )
                    } else {
                        final_output
                    };
                    Ok(ToolResult::Success(result))
                } else {
                    let result = if truncated {
                        format!(
                            "Exit code {}: {}\n\n[Output truncated]",
                            output.status.code().unwrap_or(-1),
                            final_output
                        )
                    } else {
                        format!(
                            "Exit code {}: {}",
                            output.status.code().unwrap_or(-1),
                            final_output
                        )
                    };
                    Ok(ToolResult::Error(result))
                }
            }
            Ok(Err(e)) => {
                warn!(error = %e, "Bash command execution failed");
                Err(e.into())
            }
            Err(_) => {
                warn!(
                    timeout_ms = %self.inner.policy.command_timeout.as_millis(),
                    "Bash command timed out and was killed"
                );
                Err(anyhow::anyhow!(
                    "Command timed out after {:?}",
                    self.inner.policy.command_timeout
                ))
            }
        }
    }

    /// Checks if a command is a pure `cd` (no other operations).
    fn is_pure_cd(command: &str) -> bool {
        let trimmed = command.trim();
        trimmed == "cd"
            || (trimmed.starts_with("cd ")
                && !trimmed.contains("&&")
                && !trimmed.contains("||")
                && !trimmed.contains(';'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shell_state_new() {
        let state = ShellState::new(PathBuf::from("/test"));
        assert_eq!(state.cwd(), Path::new("/test"));
        assert!(state.env().is_empty());
    }

    #[test]
    fn test_parse_cd_simple() {
        assert_eq!(ShellState::parse_cd("cd"), Some("~"));
        assert_eq!(ShellState::parse_cd("cd foo"), Some("foo"));
        assert_eq!(ShellState::parse_cd("cd /tmp"), Some("/tmp"));
    }

    #[test]
    fn test_parse_cd_compound() {
        assert_eq!(ShellState::parse_cd("cd foo && ls"), Some("foo"));
        assert_eq!(ShellState::parse_cd("cd bar || echo fail"), Some("bar"));
        assert_eq!(ShellState::parse_cd("cd baz; pwd"), Some("baz"));
    }

    #[test]
    fn test_parse_cd_not_cd() {
        assert_eq!(ShellState::parse_cd("ls"), None);
        assert_eq!(ShellState::parse_cd("echo cd"), None);
    }

    #[test]
    fn test_parse_export_simple() {
        assert_eq!(
            ShellState::parse_export("export FOO=bar"),
            Some(("FOO", "bar"))
        );
        assert_eq!(
            ShellState::parse_export("export PATH=/usr/bin"),
            Some(("PATH", "/usr/bin"))
        );
    }

    #[test]
    fn test_parse_export_quoted() {
        assert_eq!(
            ShellState::parse_export("export FOO=\"bar baz\""),
            Some(("FOO", "bar baz"))
        );
        assert_eq!(
            ShellState::parse_export("export FOO='bar baz'"),
            Some(("FOO", "bar baz"))
        );
    }

    #[test]
    fn test_parse_export_not_export() {
        assert_eq!(ShellState::parse_export("echo foo"), None);
        assert_eq!(ShellState::parse_export("FOO=bar"), None);
    }

    #[test]
    fn test_is_pure_cd() {
        assert!(StatefulToolExecutor::is_pure_cd("cd"));
        assert!(StatefulToolExecutor::is_pure_cd("cd foo"));
        assert!(StatefulToolExecutor::is_pure_cd("cd /tmp"));
        assert!(!StatefulToolExecutor::is_pure_cd("cd foo && ls"));
        assert!(!StatefulToolExecutor::is_pure_cd("cd foo; pwd"));
    }
}
