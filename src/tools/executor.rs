//! Tool executor for agentic capabilities.
//!
//! This module provides the core tool execution engine with security policy enforcement.

use anyhow::Result;
use glob::Pattern;
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tracing::{debug, warn};
use walkdir::WalkDir;

use super::security::{normalize_command, ToolExecutionPolicy};
use super::{vision, web_fetch, web_search};
use crate::permissions::PermissionRequest;
/// Tool executor with security policy enforcement.
pub struct ToolExecutor {
    working_dir: PathBuf,
    pub(crate) policy: ToolExecutionPolicy,
}

#[derive(Debug)]
pub struct ToolCall {
    pub name: String,
    pub input: serde_json::Value,
}

#[derive(Debug)]
pub enum ToolResult {
    /// Tool executed successfully with output.
    Success(String),
    /// Tool execution failed with error message.
    Error(String),
    /// Tool execution was cancelled (by hook or user).
    Cancelled,
    /// Tool requires permission before execution.
    ///
    /// The caller should display a permission prompt to the user and
    /// re-execute with the appropriate permission grant.
    NeedsPermission(PermissionRequest),
}

/// Formats the output of a completed bash process into a [`ToolResult`].
///
/// Combines stdout and stderr, truncates if the combined output exceeds `max_size`,
/// and returns [`ToolResult::Success`] or [`ToolResult::Error`] depending on the
/// exit code.
///
/// # Arguments
///
/// * `output` - The completed process output containing stdout, stderr, and exit status
/// * `max_size` - Maximum allowed output size in bytes before truncation
#[must_use]
fn format_bash_output(output: std::process::Output, max_size: usize) -> ToolResult {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    let (final_output, truncated) = if combined.len() > max_size {
        let truncated_output = combined.chars().take(max_size).collect::<String>();
        warn!(
            original_size = combined.len(),
            max_size = max_size,
            "Bash command output truncated"
        );
        (truncated_output, true)
    } else {
        (combined, false)
    };

    if output.status.success() {
        let result = if truncated {
            format!(
                "{final_output}\n\n[Output truncated: {} bytes exceeded {max_size} byte limit]",
                stdout.len() + stderr.len(),
            )
        } else {
            final_output
        };
        ToolResult::Success(result)
    } else {
        let exit_code = output.status.code().unwrap_or(-1);
        let result = if truncated {
            format!("Exit code {exit_code}: {final_output}\n\n[Output truncated]")
        } else {
            format!("Exit code {exit_code}: {final_output}")
        };
        ToolResult::Error(result)
    }
}

/// Parsed and validated parameters for a grep search operation.
#[derive(Debug)]
struct GrepParams {
    /// Compiled regex pattern.
    regex: Regex,
    /// Optional glob pattern for filtering files by name.
    file_glob: Option<Pattern>,
    /// Resolved file type extensions (e.g., `[".rs"]` for Rust).
    type_extensions: Option<Vec<&'static str>>,
    /// Output format: `"content"`, `"files_with_matches"`, or `"count"`.
    output_mode: String,
    /// Number of context lines to show around matches (content mode only).
    context_lines: Option<usize>,
    /// Maximum number of results to return.
    head_limit: Option<usize>,
    /// Root directory to search from.
    search_root: PathBuf,
}

impl ToolExecutor {
    pub fn new(working_dir: PathBuf) -> Self {
        Self {
            working_dir,
            policy: ToolExecutionPolicy::default(),
        }
    }

    pub fn with_policy(mut self, policy: ToolExecutionPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Validates that a path is within the working directory.
    ///
    /// Returns the canonicalized path if valid, or an error message if the path
    /// attempts to escape the working directory.
    ///
    /// # Errors
    ///
    /// Returns an error string if:
    /// - The path is absolute and not within the working directory
    /// - The path uses `..` to escape the working directory
    /// - The path cannot be canonicalized
    fn validate_path(&self, path: &str) -> std::result::Result<PathBuf, String> {
        // Reject absolute paths that don't start with working_dir
        if Path::new(path).is_absolute() {
            warn!(
                path = %path,
                "Security: path traversal attempt - absolute path rejected"
            );
            return Err(
                "Absolute paths are not allowed: path traversal outside working directory"
                    .to_string(),
            );
        }

        let full_path = self.working_dir.join(path);

        // Canonicalize the working directory
        let canonical_working_dir = self
            .working_dir
            .canonicalize()
            .map_err(|e| format!("Failed to canonicalize working directory: {e}"))?;

        // For existing files, canonicalize the full path
        // For non-existing files, canonicalize the parent and append the filename
        let canonical_full_path = if full_path.exists() {
            full_path
                .canonicalize()
                .map_err(|e| format!("Failed to canonicalize path: {e}"))?
        } else {
            // For new files, canonicalize the parent directory
            let parent = full_path.parent().unwrap_or(&self.working_dir);
            let filename = full_path
                .file_name()
                .ok_or_else(|| "Invalid path: no filename".to_string())?;

            if parent.exists() {
                let canonical_parent = parent
                    .canonicalize()
                    .map_err(|e| format!("Failed to canonicalize parent directory: {e}"))?;
                canonical_parent.join(filename)
            } else {
                // Parent doesn't exist, check if the path contains ..
                if path.contains("..") {
                    warn!(
                        path = %path,
                        "Security: path traversal attempt - parent escape detected"
                    );
                    return Err("Path traversal outside working directory".to_string());
                }
                full_path
            }
        };

        // Verify the canonical path starts with the working directory
        if !canonical_full_path.starts_with(&canonical_working_dir) {
            warn!(
                path = %path,
                canonical_path = %canonical_full_path.display(),
                working_dir = %canonical_working_dir.display(),
                "Security: path traversal attempt - path escapes working directory"
            );
            return Err("Path traversal outside working directory".to_string());
        }

        Ok(canonical_full_path)
    }

    /// Validates a path for writing, checking both path traversal and protected paths.
    fn validate_write_path(&self, path: &str) -> std::result::Result<PathBuf, String> {
        let canonical_path = self.validate_path(path)?;

        // Check against protected paths
        for protected in &self.policy.protected_paths {
            if canonical_path.starts_with(protected) {
                return Err(format!(
                    "Write blocked: path is in protected directory {:?}",
                    protected
                ));
            }
        }

        Ok(canonical_path)
    }

    /// Checks if a path is a symlink and returns an error if so.
    ///
    /// This is a security measure to prevent TOCTOU (Time-of-Check-Time-of-Use)
    /// attacks. Symlinks can be exploited in race conditions where an attacker
    /// replaces a validated file with a symlink pointing to a sensitive file
    /// between validation and operation.
    ///
    /// By rejecting all symlinks uniformly, we provide defense in depth against
    /// this class of attacks, regardless of where the symlink points.
    ///
    /// # Arguments
    ///
    /// * `path` - The path to check (should be the original, non-canonicalized path)
    ///
    /// # Errors
    ///
    /// Returns an error message if the path is a symlink.
    async fn check_symlink(&self, path: &str) -> std::result::Result<(), String> {
        let full_path = self.working_dir.join(path);

        // Use symlink_metadata to check the path itself, not what it points to
        // fs::metadata follows symlinks, symlink_metadata does not
        match tokio::fs::symlink_metadata(&full_path).await {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    warn!(
                        path = %path,
                        "Security: symlink rejected - TOCTOU mitigation"
                    );
                    return Err(
                        "Symlink not allowed: file operations on symlinks are rejected for security (TOCTOU mitigation)"
                            .to_string(),
                    );
                }
                Ok(())
            }
            Err(_) => {
                // Path doesn't exist yet (for new files), which is fine
                // The path traversal check already validates the parent
                Ok(())
            }
        }
    }

    /// Validates a file path by checking for symlinks and path traversal.
    ///
    /// Performs the security validation sequence: first rejects symlinks to prevent
    /// TOCTOU attacks, then validates the path is within the working directory.
    ///
    /// # Errors
    ///
    /// Returns `ToolResult::Error` if:
    /// - The path is a symlink (TOCTOU mitigation)
    /// - The path escapes the working directory (path traversal)
    async fn validate_file_path(&self, path: &str) -> std::result::Result<PathBuf, ToolResult> {
        self.check_symlink(path).await.map_err(ToolResult::Error)?;
        self.validate_path(path).map_err(ToolResult::Error)
    }

    /// Validates a file path for writing by checking symlinks, path traversal,
    /// and protected path restrictions.
    ///
    /// Performs the full security validation sequence for write operations: first
    /// rejects symlinks, then validates the path is within the working directory
    /// and not in a protected location.
    ///
    /// # Errors
    ///
    /// Returns `ToolResult::Error` if:
    /// - The path is a symlink (TOCTOU mitigation)
    /// - The path escapes the working directory (path traversal)
    /// - The path is in a protected directory
    async fn validate_file_write_path(
        &self,
        path: &str,
    ) -> std::result::Result<PathBuf, ToolResult> {
        self.check_symlink(path).await.map_err(ToolResult::Error)?;
        self.validate_write_path(path).map_err(ToolResult::Error)
    }

    /// Validates a bash command against dangerous patterns and allowlist policy.
    ///
    /// Normalizes the command to detect escape-based bypasses (e.g., `r\m` -> `rm`),
    /// then checks both the original and normalized forms against dangerous patterns.
    /// If allowlist mode is enabled, also verifies the command matches an allowed pattern.
    ///
    /// # Errors
    ///
    /// Returns `ToolResult::Error` if:
    /// - The command matches a dangerous pattern
    /// - Allowlist mode is enabled and the command does not match any allowed pattern
    fn validate_bash_command(&self, command: &str) -> std::result::Result<(), ToolResult> {
        let normalized = normalize_command(command);

        for pattern in &self.policy.dangerous_patterns {
            if pattern.is_match(command) || pattern.is_match(&normalized) {
                warn!(
                    pattern = %pattern.as_str(),
                    command = %command,
                    "Security violation: command blocked by dangerous pattern"
                );
                return Err(ToolResult::Error(format!(
                    "Command blocked by security policy: matches {:?}",
                    pattern.as_str()
                )));
            }
        }

        if self.policy.allowlist_mode {
            let is_allowed = self
                .policy
                .allowed_commands
                .iter()
                .any(|pattern| pattern.is_match(command) || pattern.is_match(&normalized));
            if !is_allowed {
                warn!(
                    command = %command,
                    "Security: command blocked by allowlist policy"
                );
                return Err(ToolResult::Error(
                    "Command blocked: not in allowlist".to_string(),
                ));
            }
        }

        Ok(())
    }

    /// Builds the shell command program and arguments for bash execution.
    ///
    /// When sandboxing is enabled, wraps the command with platform-specific sandbox
    /// enforcement (macOS Seatbelt / Linux Landlock). Otherwise, delegates to the
    /// default shell configuration.
    ///
    /// # Returns
    ///
    /// A tuple of `(program, args)` suitable for passing to `Command::new`.
    #[must_use]
    fn build_shell_command(&self, command: &str) -> (String, Vec<String>) {
        if self.policy.sandbox_enabled {
            let sandbox = super::sandbox::create_sandbox();
            let sandbox_config =
                super::sandbox::SandboxConfig::for_working_dir(self.working_dir.clone());
            sandbox.wrap_command(command, &sandbox_config)
        } else {
            let shell = crate::shell::ShellConfig::default();
            let mut args = shell.args;
            args.push(command.to_string());
            (shell.command, args)
        }
    }

    pub async fn execute(&self, call: ToolCall) -> Result<ToolResult> {
        match call.name.as_str() {
            "bash" => self.execute_bash(&call.input).await,
            "read_file" => self.read_file(&call.input).await,
            "write_file" => self.write_file(&call.input).await,
            "edit" => self.edit_file(&call.input).await,
            "multi_edit" => self.multi_edit(&call.input).await,
            "list_files" => self.list_files(&call.input).await,
            "glob" => self.glob_files(&call.input).await,
            "grep" => self.grep_content(&call.input).await,
            "web_fetch" => self.web_fetch(&call.input).await,
            "web_search" => self.web_search(&call.input).await,
            "analyze_image" => self.analyze_image(&call.input).await,
            "lsp" => self.execute_lsp(&call.input).await,
            "todo_write" => self.execute_todo_write(&call.input).await,
            _ => Ok(ToolResult::Error(format!("Unknown tool: {}", call.name))),
        }
    }

    async fn execute_bash(&self, input: &serde_json::Value) -> Result<ToolResult> {
        let command = input
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing command"))?;

        // Per-call timeout (capped at 600_000ms = 10 minutes)
        let timeout = input
            .get("timeout")
            .and_then(|v| v.as_u64())
            .map(|ms| Duration::from_millis(ms.min(600_000)))
            .unwrap_or(self.policy.command_timeout);

        if let Some(desc) = input.get("description").and_then(|v| v.as_str()) {
            debug!(description = %desc, command = %command, "Bash command with description");
        }

        if let Err(tool_err) = self.validate_bash_command(command) {
            return Ok(tool_err);
        }

        let (program, args) = self.build_shell_command(command);

        let child = Command::new(&program)
            .args(&args)
            .current_dir(&self.working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(Ok(output)) => Ok(format_bash_output(output, self.policy.max_output_size)),
            Ok(Err(e)) => {
                warn!(error = %e, "Bash command execution failed");
                Err(e.into())
            }
            Err(_) => {
                warn!(
                    timeout_ms = %timeout.as_millis(),
                    "Bash command timed out and was killed"
                );
                Err(anyhow::anyhow!("Command timed out after {:?}", timeout))
            }
        }
    }

    async fn read_file(&self, input: &serde_json::Value) -> Result<ToolResult> {
        let path = input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing path"))?;

        let full_path = match self.validate_file_path(path).await {
            Ok(p) => p,
            Err(e) => return Ok(e),
        };

        // Handle special file formats
        if full_path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("ipynb"))
        {
            return self.read_notebook(&full_path).await;
        }

        let offset = input
            .get("offset")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);
        let limit = input
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);

        match tokio::fs::read_to_string(&full_path).await {
            Ok(content) => {
                let output = Self::format_file_content(&content, offset, limit);
                Ok(ToolResult::Success(output))
            }
            Err(e) => {
                debug!(
                    path = %path,
                    error = %e,
                    "File read failed"
                );
                Ok(ToolResult::Error(format!("Failed to read file: {}", e)))
            }
        }
    }

    /// Reads a Jupyter notebook file and formats all cells.
    async fn read_notebook(&self, path: &Path) -> Result<ToolResult> {
        let content = match tokio::fs::read_to_string(path).await {
            Ok(c) => c,
            Err(e) => return Ok(ToolResult::Error(format!("Failed to read notebook: {e}"))),
        };

        let notebook: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(e) => return Ok(ToolResult::Error(format!("Invalid notebook JSON: {e}"))),
        };

        match Self::format_notebook(&notebook) {
            Ok(formatted) => Ok(ToolResult::Success(formatted)),
            Err(e) => Ok(ToolResult::Error(format!("Failed to format notebook: {e}"))),
        }
    }

    /// Formats a parsed Jupyter notebook into readable text.
    fn format_notebook(notebook: &serde_json::Value) -> Result<String> {
        let cells = notebook
            .get("cells")
            .and_then(|c| c.as_array())
            .ok_or_else(|| anyhow::anyhow!("No cells found in notebook"))?;

        let language = notebook
            .pointer("/metadata/kernelspec/language")
            .and_then(|l| l.as_str())
            .unwrap_or("python");

        let mut output = String::new();
        for (i, cell) in cells.iter().enumerate() {
            let cell_type = cell
                .get("cell_type")
                .and_then(|t| t.as_str())
                .unwrap_or("raw");
            let source = Self::join_notebook_source(cell.get("source")).unwrap_or_default();

            match cell_type {
                "code" => {
                    output.push_str(&format!(
                        "### Cell {} [code]\n```{language}\n{source}\n```\n",
                        i + 1
                    ));
                    if let Some(outputs) = cell.get("outputs").and_then(|o| o.as_array()) {
                        for out in outputs {
                            Self::format_cell_output(&mut output, out);
                        }
                    }
                }
                "markdown" => {
                    output.push_str(&format!("### Cell {} [markdown]\n{source}\n", i + 1));
                }
                other => {
                    output.push_str(&format!("### Cell {} [{other}]\n{source}\n", i + 1));
                }
            }
            output.push('\n');
        }
        Ok(output)
    }

    /// Joins a notebook cell source (can be array of strings or single string).
    fn join_notebook_source(source: Option<&serde_json::Value>) -> Option<String> {
        match source? {
            serde_json::Value::Array(arr) => {
                Some(arr.iter().filter_map(|v| v.as_str()).collect::<String>())
            }
            serde_json::Value::String(s) => Some(s.clone()),
            _ => None,
        }
    }

    /// Formats a single notebook cell output.
    fn format_cell_output(output: &mut String, cell_output: &serde_json::Value) {
        if let Some(text) = cell_output.get("text").and_then(|t| t.as_array()) {
            let text_str: String = text.iter().filter_map(|v| v.as_str()).collect();
            if !text_str.is_empty() {
                output.push_str(&format!("```\n{text_str}```\n"));
            }
        }
    }

    async fn write_file(&self, input: &serde_json::Value) -> Result<ToolResult> {
        let path = input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing path"))?;

        let content = input
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing content"))?;

        if content.len() > self.policy.max_file_size {
            warn!(
                path = %path,
                size = content.len(),
                limit = self.policy.max_file_size,
                "File write blocked: size exceeds limit"
            );
            return Ok(ToolResult::Error(format!(
                "File size {} exceeds limit {}",
                content.len(),
                self.policy.max_file_size
            )));
        }

        let full_path = match self.validate_file_write_path(path).await {
            Ok(p) => p,
            Err(e) => return Ok(e),
        };

        // Create backup if file exists
        if full_path.exists() {
            if let Err(e) = self.create_backup(&full_path).await {
                return Ok(ToolResult::Error(format!("Failed to create backup: {e}")));
            }
        }

        if let Some(parent) = full_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        match tokio::fs::write(&full_path, content).await {
            Ok(()) => Ok(ToolResult::Success(format!(
                "Wrote {} bytes to {}",
                content.len(),
                path
            ))),
            Err(e) => {
                debug!(
                    path = %path,
                    error = %e,
                    "File write failed"
                );
                Ok(ToolResult::Error(format!("Failed to write file: {}", e)))
            }
        }
    }

    /// Performs a string replacement edit on a file.
    ///
    /// Requires a unique match of `old_string` in the file. If there are zero
    /// or multiple matches, returns an error. On success, generates a diff-like
    /// output showing the change.
    async fn edit_file(&self, input: &serde_json::Value) -> Result<ToolResult> {
        let path = input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing path"))?;

        let old_string = input
            .get("old_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing old_string"))?;

        let new_string = input
            .get("new_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing new_string"))?;

        let replace_all = input
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let full_path = match self.validate_file_path(path).await {
            Ok(p) => p,
            Err(e) => return Ok(e),
        };

        // Read file content
        let content = match tokio::fs::read_to_string(&full_path).await {
            Ok(c) => c,
            Err(e) => return Ok(ToolResult::Error(format!("Failed to read file: {e}"))),
        };

        // Count matches
        let match_count = content.matches(old_string).count();

        if match_count == 0 {
            return Ok(ToolResult::Error(
                "No matches found for old_string: 0 matches".to_string(),
            ));
        }

        if !replace_all && match_count > 1 {
            return Ok(ToolResult::Error(format!(
                "Multiple matches found: {match_count} matches. Edit requires a unique match to avoid ambiguity."
            )));
        }

        // Create backup before editing
        if let Err(e) = self.create_backup(&full_path).await {
            return Ok(ToolResult::Error(format!("Failed to create backup: {e}")));
        }

        // Perform the replacement
        let new_content = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };

        // Write the modified content
        if let Err(e) = tokio::fs::write(&full_path, &new_content).await {
            return Ok(ToolResult::Error(format!("Failed to write file: {e}")));
        }

        // Generate diff output
        let diff = Self::generate_diff(old_string, new_string);

        let summary = if replace_all && match_count > 1 {
            format!("Successfully replaced {match_count} occurrences in {path}:\n{diff}")
        } else {
            format!("Successfully replaced in {path}:\n{diff}")
        };

        Ok(ToolResult::Success(summary))
    }

    /// Applies edits to multiple files in one operation.
    ///
    /// Each edit is applied sequentially using the same logic as `edit_file`.
    /// Results are collected per-edit. Failures in one edit do not prevent
    /// subsequent edits from being attempted.
    async fn multi_edit(&self, input: &serde_json::Value) -> Result<ToolResult> {
        let edits = input
            .get("edits")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("Missing edits array"))?;

        if edits.is_empty() {
            return Ok(ToolResult::Error("No edits provided".to_string()));
        }

        let mut results = Vec::new();
        let mut success_count = 0;
        let mut error_count = 0;

        for (i, edit) in edits.iter().enumerate() {
            match self.edit_file(edit).await? {
                ToolResult::Success(msg) => {
                    success_count += 1;
                    results.push(format!("Edit {}: OK — {msg}", i + 1));
                }
                ToolResult::Error(msg) => {
                    error_count += 1;
                    results.push(format!("Edit {}: ERROR — {msg}", i + 1));
                }
                other => {
                    results.push(format!("Edit {}: {other:?}", i + 1));
                }
            }
        }

        let summary = format!(
            "Multi-edit complete: {success_count} succeeded, {error_count} failed\n\n{}",
            results.join("\n")
        );

        if error_count > 0 && success_count == 0 {
            Ok(ToolResult::Error(summary))
        } else {
            Ok(ToolResult::Success(summary))
        }
    }

    /// Formats file content with cat -n style line numbers and optional offset/limit.
    ///
    /// # Arguments
    ///
    /// * `content` - The full file content
    /// * `offset` - Number of lines to skip from the start (0-based)
    /// * `limit` - Maximum number of lines to return
    fn format_file_content(content: &str, offset: Option<usize>, limit: Option<usize>) -> String {
        let lines: Vec<&str> = content.lines().collect();
        let start = offset.unwrap_or(0);
        let end = match limit {
            Some(lim) => (start + lim).min(lines.len()),
            None => lines.len(),
        };

        let mut output = String::new();
        for (i, line) in lines.iter().enumerate().take(end).skip(start) {
            // cat -n style: right-aligned line number, tab, content
            output.push_str(&format!("{:>6}\t{line}\n", i + 1));
        }
        output
    }

    /// Generates a simple diff output showing the replacement.
    fn generate_diff(old: &str, new: &str) -> String {
        let old_lines: Vec<&str> = old.lines().collect();
        let new_lines: Vec<&str> = new.lines().collect();

        let mut diff = String::new();

        for line in &old_lines {
            diff.push_str(&format!("- {line}\n"));
        }
        for line in &new_lines {
            diff.push_str(&format!("+ {line}\n"));
        }

        if diff.is_empty() {
            format!("- {old}\n+ {new}\n")
        } else {
            diff
        }
    }

    /// Creates a backup of an existing file before modification.
    async fn create_backup(&self, path: &Path) -> std::result::Result<PathBuf, String> {
        let backup_dir = self.working_dir.join(".rct_backups");

        // Create backup directory if it doesn't exist
        tokio::fs::create_dir_all(&backup_dir)
            .await
            .map_err(|e| format!("Failed to create backup directory: {e}"))?;

        // Generate backup filename with timestamp
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let backup_name = format!("{filename}.{timestamp}.bak");
        let backup_path = backup_dir.join(&backup_name);

        // Copy file to backup location
        tokio::fs::copy(path, &backup_path)
            .await
            .map_err(|e| format!("Failed to copy file to backup: {e}"))?;

        Ok(backup_path)
    }

    async fn list_files(&self, input: &serde_json::Value) -> Result<ToolResult> {
        let path = input.get("path").and_then(|v| v.as_str()).unwrap_or(".");

        // Validate path is within working directory
        let full_path = match self.validate_path(path) {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::Error(e)),
        };

        // Open directory, handling errors gracefully
        let mut dir = match tokio::fs::read_dir(&full_path).await {
            Ok(d) => d,
            Err(e) => {
                debug!(
                    path = %path,
                    error = %e,
                    "Directory listing failed"
                );
                return Ok(ToolResult::Error(format!(
                    "Failed to list directory '{}': {}",
                    path, e
                )));
            }
        };

        let mut entries = Vec::new();

        loop {
            match dir.next_entry().await {
                Ok(Some(entry)) => {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let file_type = match entry.file_type().await {
                        Ok(ft) => ft,
                        Err(_) => continue, // Skip entries we can't get file type for
                    };
                    let prefix = if file_type.is_dir() { "d " } else { "- " };
                    entries.push(format!("{}{}", prefix, name));
                }
                Ok(None) => break,
                Err(e) => {
                    return Ok(ToolResult::Error(format!(
                        "Error reading directory entries: {}",
                        e
                    )))
                }
            }
        }

        entries.sort();
        Ok(ToolResult::Success(entries.join("\n")))
    }

    /// Searches for files matching a glob pattern.
    ///
    /// # Arguments
    ///
    /// * `pattern` - The glob pattern (e.g., `**/*.rs`)
    /// * `respect_gitignore` - Whether to respect .gitignore rules (optional)
    ///
    /// # Errors
    ///
    /// Returns an error if the pattern attempts path traversal.
    async fn glob_files(&self, input: &serde_json::Value) -> Result<ToolResult> {
        let pattern = input
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing pattern"))?;

        let respect_gitignore = input
            .get("respect_gitignore")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Block path traversal attempts
        if pattern.contains("..") {
            return Ok(ToolResult::Error(
                "Invalid pattern: path traversal not allowed".to_string(),
            ));
        }

        // Load gitignore patterns if requested
        let gitignore_patterns = if respect_gitignore {
            self.load_gitignore_patterns()
        } else {
            Vec::new()
        };

        // Compile the glob pattern
        let glob_pattern = match Pattern::new(pattern) {
            Ok(p) => p,
            Err(e) => {
                debug!(
                    pattern = %pattern,
                    error = %e,
                    "Invalid glob pattern"
                );
                return Ok(ToolResult::Error(format!("Invalid glob pattern: {e}")));
            }
        };

        let files = Self::walk_relative_files(&self.working_dir, &self.working_dir);

        let mut matches: Vec<String> = files
            .iter()
            .filter_map(|(_, rel)| {
                let relative_str = rel.to_string_lossy();
                if respect_gitignore && self.is_gitignored(&relative_str, &gitignore_patterns) {
                    return None;
                }
                if glob_pattern.matches(&relative_str) {
                    Some(relative_str.to_string())
                } else {
                    None
                }
            })
            .collect();

        if matches.is_empty() {
            return Ok(ToolResult::Success(String::new()));
        }

        matches.sort();
        Ok(ToolResult::Success(matches.join("\n")))
    }

    /// Loads gitignore patterns from .gitignore file if it exists.
    fn load_gitignore_patterns(&self) -> Vec<String> {
        let gitignore_path = self.working_dir.join(".gitignore");
        if !gitignore_path.exists() {
            return Vec::new();
        }

        let content = match fs::read_to_string(&gitignore_path) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };

        content
            .lines()
            .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
            .map(|line| line.trim().to_string())
            .collect()
    }

    /// Checks if a path matches any gitignore pattern.
    fn is_gitignored(&self, path: &str, patterns: &[String]) -> bool {
        for pattern in patterns {
            // Handle directory patterns (ending with /)
            if pattern.ends_with('/') {
                let dir_name = &pattern[..pattern.len() - 1];
                // Match if path starts with the directory or contains it as a component
                if path.starts_with(dir_name) || path.starts_with(&format!("{dir_name}/")) {
                    return true;
                }
            }
            // Handle glob patterns like *.log
            else if pattern.starts_with('*') {
                if let Ok(glob) = Pattern::new(pattern) {
                    // Check against the full path and just the filename
                    let filename = Path::new(path)
                        .file_name()
                        .map(|n| n.to_string_lossy())
                        .unwrap_or_default();
                    if glob.matches(path) || glob.matches(&filename) {
                        return true;
                    }
                }
            }
            // Handle exact matches and path prefixes
            else if path == pattern || path.starts_with(&format!("{pattern}/")) {
                return true;
            }
        }
        false
    }

    /// Resolves a file type name to file extensions.
    ///
    /// Maps common language names to their typical file extensions.
    fn file_type_extensions(file_type: &str) -> Option<Vec<&'static str>> {
        match file_type.to_lowercase().as_str() {
            "rust" | "rs" => Some(vec![".rs"]),
            "js" | "javascript" => Some(vec![".js", ".mjs", ".cjs"]),
            "ts" | "typescript" => Some(vec![".ts", ".tsx"]),
            "py" | "python" => Some(vec![".py"]),
            "go" | "golang" => Some(vec![".go"]),
            "java" => Some(vec![".java"]),
            "c" => Some(vec![".c", ".h"]),
            "cpp" | "c++" | "cxx" => Some(vec![".cpp", ".hpp", ".cc", ".cxx", ".hxx"]),
            "rb" | "ruby" => Some(vec![".rb"]),
            "sh" | "shell" | "bash" => Some(vec![".sh", ".bash"]),
            "json" => Some(vec![".json"]),
            "yaml" | "yml" => Some(vec![".yaml", ".yml"]),
            "toml" => Some(vec![".toml"]),
            "md" | "markdown" => Some(vec![".md"]),
            "html" => Some(vec![".html", ".htm"]),
            "css" => Some(vec![".css"]),
            "sql" => Some(vec![".sql"]),
            _ => None,
        }
    }

    /// Checks whether a file should be searched based on glob and type filters.
    fn should_search_file(
        relative: &Path,
        file_glob: Option<&Pattern>,
        type_extensions: Option<&[&str]>,
    ) -> bool {
        // Apply file pattern filter
        if let Some(glob) = file_glob {
            let filename = relative
                .file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default();
            if !glob.matches(&filename) {
                return false;
            }
        }

        // Apply file type filter
        if let Some(extensions) = type_extensions {
            let path_str = relative.to_string_lossy();
            if !extensions.iter().any(|ext| path_str.ends_with(ext)) {
                return false;
            }
        }

        true
    }

    /// Walks a directory tree and yields `(absolute_path, relative_path)` pairs for all files.
    ///
    /// Skips directories, symlinks, and entries that cannot be made relative to
    /// `working_dir`. Both `grep_content` and `glob_files` share this traversal logic.
    ///
    /// # Arguments
    ///
    /// * `search_root` - The root directory to begin walking from.
    /// * `working_dir` - The base directory for computing relative paths.
    #[must_use]
    fn walk_relative_files(search_root: &Path, working_dir: &Path) -> Vec<(PathBuf, PathBuf)> {
        WalkDir::new(search_root)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|entry| !entry.path().is_dir())
            .filter_map(|entry| {
                let abs = entry.path().to_path_buf();
                let rel = abs.strip_prefix(working_dir).ok()?.to_path_buf();
                Some((abs, rel))
            })
            .collect()
    }

    /// Parses and validates grep parameters from a JSON input value.
    ///
    /// Extracts pattern, case sensitivity, file filters, output mode, context lines,
    /// head limit, and search path. Compiles the regex and resolves the search root.
    ///
    /// # Errors
    ///
    /// Returns `ToolResult::Error` (wrapped in `Ok`) if the regex is invalid,
    /// or `Err` if the `pattern` field is missing entirely.
    fn parse_grep_params(
        input: &serde_json::Value,
        working_dir: &Path,
    ) -> Result<std::result::Result<GrepParams, ToolResult>> {
        let pattern = input
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing pattern"))?;

        let case_insensitive = input
            .get("case_insensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let regex = if case_insensitive {
            regex::RegexBuilder::new(pattern)
                .case_insensitive(true)
                .build()
        } else {
            Regex::new(pattern)
        };

        let regex = match regex {
            Ok(r) => r,
            Err(e) => {
                debug!(
                    pattern = %pattern,
                    error = %e,
                    "Invalid regex pattern"
                );
                return Ok(Err(ToolResult::Error(format!(
                    "Invalid regex pattern: {e}"
                ))));
            }
        };

        let file_pattern = input
            .get("file_pattern")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let file_glob = file_pattern.as_ref().and_then(|p| Pattern::new(p).ok());

        let file_type = input.get("file_type").and_then(|v| v.as_str());
        let type_extensions = file_type.and_then(Self::file_type_extensions);

        let output_mode = input
            .get("output_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("content")
            .to_string();

        let context_lines = input
            .get("context_lines")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);

        let head_limit = input
            .get("head_limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);

        let search_path = input.get("path").and_then(|v| v.as_str());
        let search_root = match search_path {
            Some(p) => working_dir.join(p),
            None => working_dir.to_path_buf(),
        };

        Ok(Ok(GrepParams {
            regex,
            file_glob,
            type_extensions,
            output_mode,
            context_lines,
            head_limit,
            search_root,
        }))
    }

    /// Searches files and returns paths that contain at least one match.
    ///
    /// Implements the `"files_with_matches"` output mode for grep.
    #[must_use]
    fn grep_files_with_matches(
        files: &[(PathBuf, PathBuf)],
        regex: &Regex,
        head_limit: Option<usize>,
    ) -> Vec<String> {
        let mut results = Vec::new();
        for (abs, rel) in files {
            if head_limit.is_some_and(|lim| results.len() >= lim) {
                break;
            }
            let content = match fs::read_to_string(abs) {
                Ok(c) => c,
                Err(_) => continue,
            };
            if content.lines().any(|line| regex.is_match(line)) {
                results.push(rel.to_string_lossy().to_string());
            }
        }
        results
    }

    /// Searches files and returns per-file match counts.
    ///
    /// Implements the `"count"` output mode for grep.
    /// Each result is formatted as `"relative/path:count"`.
    #[must_use]
    fn grep_count_mode(
        files: &[(PathBuf, PathBuf)],
        regex: &Regex,
        head_limit: Option<usize>,
    ) -> Vec<String> {
        let mut results = Vec::new();
        for (abs, rel) in files {
            if head_limit.is_some_and(|lim| results.len() >= lim) {
                break;
            }
            let content = match fs::read_to_string(abs) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let count = content.lines().filter(|line| regex.is_match(line)).count();
            if count > 0 {
                results.push(format!("{}:{count}", rel.to_string_lossy()));
            }
        }
        results
    }

    /// Formats a single matching line with optional surrounding context lines.
    ///
    /// When `context_lines` is `Some(n)`, includes up to `n` lines before and after
    /// the match. Context lines use `-N-` separators; the matching line uses `:N:`.
    #[must_use]
    fn format_match_with_context(
        lines: &[&str],
        line_num: usize,
        context_lines: Option<usize>,
        relative_str: &str,
    ) -> Vec<String> {
        let mut output = Vec::new();
        if let Some(ctx) = context_lines {
            let start = line_num.saturating_sub(ctx);
            for (i, ctx_line) in lines.iter().enumerate().take(line_num).skip(start) {
                output.push(format!("{relative_str}-{}- {ctx_line}", i + 1));
            }
            output.push(format!(
                "{relative_str}:{}:{}",
                line_num + 1,
                lines[line_num]
            ));
            let end = (line_num + ctx + 1).min(lines.len());
            for (i, ctx_line) in lines.iter().enumerate().take(end).skip(line_num + 1) {
                output.push(format!("{relative_str}-{}- {ctx_line}", i + 1));
            }
        } else {
            output.push(format!(
                "{relative_str}:{}: {}",
                line_num + 1,
                lines[line_num]
            ));
        }
        output
    }

    /// Searches files and returns matching lines with optional context.
    ///
    /// Implements the default `"content"` output mode for grep.
    /// Each match is formatted as `"relative/path:line_num: content"`.
    #[must_use]
    fn grep_content_mode(
        files: &[(PathBuf, PathBuf)],
        regex: &Regex,
        context_lines: Option<usize>,
        head_limit: Option<usize>,
    ) -> Vec<String> {
        let mut results = Vec::new();
        let mut result_count = 0usize;
        for (abs, rel) in files {
            if head_limit.is_some_and(|lim| result_count >= lim) {
                break;
            }
            let content = match fs::read_to_string(abs) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let relative_str = rel.to_string_lossy();
            let lines: Vec<&str> = content.lines().collect();
            for (line_num, line) in lines.iter().enumerate() {
                if head_limit.is_some_and(|lim| result_count >= lim) {
                    break;
                }
                if regex.is_match(line) {
                    let formatted = Self::format_match_with_context(
                        &lines,
                        line_num,
                        context_lines,
                        &relative_str,
                    );
                    results.extend(formatted);
                    result_count += 1;
                }
            }
        }
        results
    }

    /// Searches file contents for a pattern.
    ///
    /// Parses parameters, walks the file tree, applies filters, and delegates to
    /// the appropriate output mode handler (`files_with_matches`, `count`, or `content`).
    ///
    /// # Arguments
    ///
    /// * `pattern` - The regex pattern to search for
    /// * `case_insensitive` - Whether to perform case-insensitive search (optional)
    /// * `file_pattern` - Glob pattern to filter files (optional)
    /// * `output_mode` - Output format: "content", "files_with_matches", or "count"
    /// * `context_lines` - Number of context lines around matches
    /// * `file_type` - Filter by language (e.g., "rust", "js")
    /// * `head_limit` - Maximum number of results
    /// * `path` - Subdirectory or file to search in
    ///
    /// # Errors
    ///
    /// Returns an error if the `pattern` field is missing from the input.
    async fn grep_content(&self, input: &serde_json::Value) -> Result<ToolResult> {
        let params = match Self::parse_grep_params(input, &self.working_dir)? {
            Ok(p) => p,
            Err(tool_err) => return Ok(tool_err),
        };

        let all_files = Self::walk_relative_files(&params.search_root, &self.working_dir);

        let type_ext_refs: Option<Vec<&str>> =
            params.type_extensions.as_deref().map(|v| v.to_vec());
        let filtered_files: Vec<(PathBuf, PathBuf)> = all_files
            .into_iter()
            .filter(|(_, rel)| {
                Self::should_search_file(rel, params.file_glob.as_ref(), type_ext_refs.as_deref())
            })
            .collect();

        let results = match params.output_mode.as_str() {
            "files_with_matches" => {
                Self::grep_files_with_matches(&filtered_files, &params.regex, params.head_limit)
            }
            "count" => Self::grep_count_mode(&filtered_files, &params.regex, params.head_limit),
            _ => Self::grep_content_mode(
                &filtered_files,
                &params.regex,
                params.context_lines,
                params.head_limit,
            ),
        };

        if results.is_empty() {
            return Ok(ToolResult::Success(String::new()));
        }

        Ok(ToolResult::Success(results.join("\n")))
    }

    /// Fetches content from a URL and converts HTML to markdown.
    ///
    /// # Arguments
    ///
    /// * `url` - The URL to fetch content from
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The URL is invalid
    /// - The URL uses a disallowed scheme (file://)
    /// - The URL points to localhost or private IP ranges
    /// - The request times out
    /// - The content exceeds the maximum length
    async fn web_fetch(&self, input: &serde_json::Value) -> Result<ToolResult> {
        let url = input
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing url"))?;

        let tool = web_fetch::WebFetchTool::new(web_fetch::WebFetchConfig::default())?;

        match tool.fetch(url).await {
            Ok(result) => Ok(ToolResult::Success(format!(
                "Fetched {} ({}, status {})\n\n{}",
                url, result.content_type, result.status, result.content
            ))),
            Err(e) => {
                debug!(
                    url = %url,
                    error = %e,
                    "Web fetch failed"
                );
                Ok(ToolResult::Error(format!("Failed to fetch URL: {e}")))
            }
        }
    }

    /// Searches the web using the given query.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The query is empty
    /// - The request times out
    /// - The search API returns an error
    async fn web_search(&self, input: &serde_json::Value) -> Result<ToolResult> {
        let query = input
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing query"))?;

        let max_results = input
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(10);

        let tool = web_search::WebSearchTool::new(web_search::WebSearchConfig::default());

        match tool.search(query, max_results).await {
            Ok(results) => {
                let markdown = web_search::WebSearchTool::format_as_markdown(&results);
                Ok(ToolResult::Success(markdown))
            }
            Err(e) => {
                debug!(
                    query = %query,
                    error = %e,
                    "Web search failed"
                );
                Ok(ToolResult::Error(format!("Search failed: {e}")))
            }
        }
    }

    /// Analyzes an image using Claude's vision capabilities.
    ///
    /// # Arguments
    ///
    /// * `path` - The relative path to the image file
    /// * `prompt` - Optional prompt to guide the analysis
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The path is missing
    /// - The file cannot be read
    /// - The image format is not supported
    async fn analyze_image(&self, input: &serde_json::Value) -> Result<ToolResult> {
        let path = input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing path"))?;

        let prompt = input.get("prompt").and_then(|v| v.as_str());

        let full_path = match self.validate_file_path(path).await {
            Ok(p) => p,
            Err(e) => return Ok(e),
        };

        let tool = vision::VisionTool::new(vision::VisionConfig::default());

        match tool.analyze(&full_path, prompt) {
            Ok(result) => {
                // Return information about the loaded image
                // The actual image data is available via result.image for API submission
                let response = format!(
                    "Image loaded successfully:\n- Path: {}\n- Format: {}\n- Prompt: {}",
                    path,
                    result.media_type.as_str(),
                    result.prompt.as_deref().unwrap_or("(none)")
                );
                Ok(ToolResult::Success(response))
            }
            Err(e) => {
                debug!(
                    path = %path,
                    error = %e,
                    "Image analysis failed"
                );
                Ok(ToolResult::Error(format!("Failed to analyze image: {e}")))
            }
        }
    }

    async fn execute_lsp(&self, input: &serde_json::Value) -> Result<ToolResult> {
        let operation: super::lsp::LspOperation = serde_json::from_value(input.clone())
            .map_err(|e| anyhow::anyhow!("Invalid LSP operation: {e}"))?;
        let tool = super::lsp::LspTool::new(self.working_dir.clone());
        match tool.execute(&operation).await {
            Ok(result) => Ok(ToolResult::Success(super::lsp::LspTool::format_result(
                &result,
            ))),
            Err(e) => Ok(ToolResult::Error(format!("LSP error: {e}"))),
        }
    }

    async fn execute_todo_write(&self, input: &serde_json::Value) -> Result<ToolResult> {
        let operation = input
            .get("operation")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing operation"))?;

        let data_dir = directories::ProjectDirs::from("com", "patina", "patina")
            .map(|d| d.data_dir().to_path_buf())
            .unwrap_or_else(|| self.working_dir.join(".patina"));
        let store = super::todo::TodoStore::new(super::todo::default_todo_path(&data_dir));

        match operation {
            "add" => {
                let content = input
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing content for add operation"))?;
                let priority = input
                    .get("priority")
                    .and_then(|v| v.as_str())
                    .map(|p| match p {
                        "high" => super::todo::Priority::High,
                        "low" => super::todo::Priority::Low,
                        _ => super::todo::Priority::Medium,
                    })
                    .unwrap_or_default();
                let item = store.add(content, priority)?;
                Ok(ToolResult::Success(format!(
                    "Added todo: [{}] {}",
                    &item.id[..8],
                    item.content
                )))
            }
            "complete" => {
                let id = input
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing id for complete operation"))?;
                store.complete(id)?;
                Ok(ToolResult::Success(format!("Completed todo: {id}")))
            }
            "remove" => {
                let id = input
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing id for remove operation"))?;
                store.remove(id)?;
                Ok(ToolResult::Success(format!("Removed todo: {id}")))
            }
            "list" => {
                let formatted = store.format_list()?;
                Ok(ToolResult::Success(formatted))
            }
            other => Ok(ToolResult::Error(format!(
                "Unknown todo_write operation: {other}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_tool_executor_new() {
        let executor = ToolExecutor::new(PathBuf::from("/tmp"));
        assert!(executor.policy.command_timeout.as_secs() > 0);
    }

    #[test]
    fn test_tool_executor_with_policy() {
        let policy = ToolExecutionPolicy::default();
        let executor = ToolExecutor::new(PathBuf::from("/tmp")).with_policy(policy);
        assert!(executor.policy.command_timeout.as_secs() > 0);
    }

    #[test]
    fn test_tool_call_debug() {
        let call = ToolCall {
            name: "test".to_string(),
            input: serde_json::json!({"key": "value"}),
        };
        let debug_str = format!("{:?}", call);
        assert!(debug_str.contains("test"));
    }

    #[test]
    fn test_tool_result_variants() {
        let success = ToolResult::Success("output".to_string());
        assert!(matches!(success, ToolResult::Success(_)));

        let error = ToolResult::Error("error".to_string());
        assert!(matches!(error, ToolResult::Error(_)));

        let cancelled = ToolResult::Cancelled;
        assert!(matches!(cancelled, ToolResult::Cancelled));
    }

    #[test]
    fn test_generate_diff() {
        let diff = ToolExecutor::generate_diff("old line", "new line");
        assert!(diff.contains("- old line"));
        assert!(diff.contains("+ new line"));
    }

    #[test]
    fn test_generate_diff_multiline() {
        let diff = ToolExecutor::generate_diff("line1\nline2", "new1\nnew2");
        assert!(diff.contains("- line1"));
        assert!(diff.contains("- line2"));
        assert!(diff.contains("+ new1"));
        assert!(diff.contains("+ new2"));
    }

    #[tokio::test]
    async fn test_validate_path_rejects_absolute() {
        let temp_dir = TempDir::new().unwrap();
        let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

        let result = executor.validate_path("/etc/passwd");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Absolute paths"));
    }

    #[tokio::test]
    async fn test_validate_path_rejects_parent_traversal() {
        let temp_dir = TempDir::new().unwrap();
        let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

        let result = executor.validate_path("../../../etc/passwd");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_validate_path_accepts_valid() {
        let temp_dir = TempDir::new().unwrap();
        std::fs::write(temp_dir.path().join("test.txt"), "content").unwrap();
        let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

        let result = executor.validate_path("test.txt");
        assert!(result.is_ok());
    }

    #[test]
    fn test_is_gitignored() {
        let executor = ToolExecutor::new(PathBuf::from("/tmp"));
        let patterns = vec![
            "target/".to_string(),
            "*.log".to_string(),
            "node_modules".to_string(),
        ];

        assert!(executor.is_gitignored("target/debug/main", &patterns));
        assert!(executor.is_gitignored("app.log", &patterns));
        assert!(executor.is_gitignored("node_modules/pkg", &patterns));
        assert!(!executor.is_gitignored("src/main.rs", &patterns));
    }

    // =========================================================================
    // Notebook formatting tests
    // =========================================================================

    #[test]
    fn test_format_notebook_code_cells() {
        let notebook = serde_json::json!({
            "cells": [
                {
                    "cell_type": "code",
                    "source": ["print('hello')\n", "print('world')"],
                    "outputs": []
                }
            ],
            "metadata": {"kernelspec": {"language": "python"}}
        });

        let formatted = ToolExecutor::format_notebook(&notebook).unwrap();
        assert!(formatted.contains("### Cell 1 [code]"));
        assert!(formatted.contains("```python"));
        assert!(formatted.contains("print('hello')"));
    }

    #[test]
    fn test_format_notebook_markdown_cells() {
        let notebook = serde_json::json!({
            "cells": [
                {
                    "cell_type": "markdown",
                    "source": "# Title\nSome text"
                }
            ],
            "metadata": {}
        });

        let formatted = ToolExecutor::format_notebook(&notebook).unwrap();
        assert!(formatted.contains("### Cell 1 [markdown]"));
        assert!(formatted.contains("# Title"));
    }

    #[test]
    fn test_format_notebook_mixed_cells() {
        let notebook = serde_json::json!({
            "cells": [
                {"cell_type": "markdown", "source": "# Intro"},
                {"cell_type": "code", "source": ["x = 1"], "outputs": []},
                {"cell_type": "raw", "source": "raw data"}
            ],
            "metadata": {}
        });

        let formatted = ToolExecutor::format_notebook(&notebook).unwrap();
        assert!(formatted.contains("Cell 1 [markdown]"));
        assert!(formatted.contains("Cell 2 [code]"));
        assert!(formatted.contains("Cell 3 [raw]"));
    }

    #[test]
    fn test_format_notebook_with_outputs() {
        let notebook = serde_json::json!({
            "cells": [
                {
                    "cell_type": "code",
                    "source": ["print(42)"],
                    "outputs": [
                        {"text": ["42\n"]}
                    ]
                }
            ],
            "metadata": {}
        });

        let formatted = ToolExecutor::format_notebook(&notebook).unwrap();
        assert!(formatted.contains("42"));
    }

    #[test]
    fn test_format_notebook_no_cells_fails() {
        let notebook = serde_json::json!({"metadata": {}});
        let result = ToolExecutor::format_notebook(&notebook);
        assert!(result.is_err());
    }

    #[test]
    fn test_join_notebook_source_array() {
        let source = serde_json::json!(["line 1\n", "line 2"]);
        let joined = ToolExecutor::join_notebook_source(Some(&source));
        assert_eq!(joined, Some("line 1\nline 2".to_string()));
    }

    #[test]
    fn test_join_notebook_source_string() {
        let source = serde_json::json!("single string");
        let joined = ToolExecutor::join_notebook_source(Some(&source));
        assert_eq!(joined, Some("single string".to_string()));
    }

    #[test]
    fn test_join_notebook_source_none() {
        let joined = ToolExecutor::join_notebook_source(None);
        assert!(joined.is_none());
    }

    // =========================================================================
    // 10.3.1: read_file with offset/limit
    // =========================================================================

    #[tokio::test]
    async fn test_read_file_with_offset_and_limit() {
        let temp_dir = TempDir::new().unwrap();
        let content = "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9\nline10\n";
        std::fs::write(temp_dir.path().join("test.txt"), content).unwrap();
        let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

        let result = executor
            .read_file(&serde_json::json!({
                "path": "test.txt",
                "offset": 3,
                "limit": 4
            }))
            .await
            .unwrap();

        match result {
            ToolResult::Success(output) => {
                // Should return lines 4-7 with cat -n style line numbers
                assert!(
                    output.contains("4\tline4"),
                    "should contain line 4, got: {output}"
                );
                assert!(output.contains("5\tline5"), "should contain line 5");
                assert!(output.contains("6\tline6"), "should contain line 6");
                assert!(output.contains("7\tline7"), "should contain line 7");
                assert!(!output.contains("line3"), "should not contain line 3");
                assert!(!output.contains("line8"), "should not contain line 8");
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_read_file_with_offset_only() {
        let temp_dir = TempDir::new().unwrap();
        let content = "line1\nline2\nline3\nline4\nline5\n";
        std::fs::write(temp_dir.path().join("test.txt"), content).unwrap();
        let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

        let result = executor
            .read_file(&serde_json::json!({
                "path": "test.txt",
                "offset": 2
            }))
            .await
            .unwrap();

        match result {
            ToolResult::Success(output) => {
                // Should return from line 3 onwards
                assert!(
                    output.contains("3\tline3"),
                    "should contain line 3, got: {output}"
                );
                assert!(output.contains("5\tline5"), "should contain line 5");
                assert!(!output.contains("1\tline1"), "should not contain line 1");
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_read_file_with_limit_only() {
        let temp_dir = TempDir::new().unwrap();
        let content = "line1\nline2\nline3\nline4\nline5\n";
        std::fs::write(temp_dir.path().join("test.txt"), content).unwrap();
        let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

        let result = executor
            .read_file(&serde_json::json!({
                "path": "test.txt",
                "limit": 2
            }))
            .await
            .unwrap();

        match result {
            ToolResult::Success(output) => {
                assert!(
                    output.contains("1\tline1"),
                    "should contain line 1, got: {output}"
                );
                assert!(output.contains("2\tline2"), "should contain line 2");
                assert!(!output.contains("3\tline3"), "should not contain line 3");
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_read_file_without_offset_limit_returns_full_content() {
        let temp_dir = TempDir::new().unwrap();
        let content = "hello\nworld\n";
        std::fs::write(temp_dir.path().join("test.txt"), content).unwrap();
        let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

        let result = executor
            .read_file(&serde_json::json!({"path": "test.txt"}))
            .await
            .unwrap();

        match result {
            ToolResult::Success(output) => {
                // Without offset/limit, returns full content with line numbers
                assert!(
                    output.contains("1\thello"),
                    "should have line numbers, got: {output}"
                );
                assert!(output.contains("2\tworld"), "should contain line 2");
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    // =========================================================================
    // 10.3.2: edit with replace_all
    // =========================================================================

    #[tokio::test]
    async fn test_edit_replace_all_replaces_all_occurrences() {
        let temp_dir = TempDir::new().unwrap();
        let content = "foo bar foo baz foo\n";
        std::fs::write(temp_dir.path().join("test.txt"), content).unwrap();
        let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

        let result = executor
            .edit_file(&serde_json::json!({
                "path": "test.txt",
                "old_string": "foo",
                "new_string": "qux",
                "replace_all": true
            }))
            .await
            .unwrap();

        match result {
            ToolResult::Success(msg) => {
                assert!(msg.contains("test.txt"), "should mention file path");
                // Verify file content
                let new_content =
                    std::fs::read_to_string(temp_dir.path().join("test.txt")).unwrap();
                assert_eq!(new_content, "qux bar qux baz qux\n");
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_edit_without_replace_all_rejects_multiple() {
        let temp_dir = TempDir::new().unwrap();
        let content = "foo bar foo baz foo\n";
        std::fs::write(temp_dir.path().join("test.txt"), content).unwrap();
        let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

        let result = executor
            .edit_file(&serde_json::json!({
                "path": "test.txt",
                "old_string": "foo",
                "new_string": "qux"
            }))
            .await
            .unwrap();

        match result {
            ToolResult::Error(msg) => {
                assert!(
                    msg.contains("3 matches"),
                    "should report match count, got: {msg}"
                );
            }
            other => panic!("Expected Error, got {:?}", other),
        }
    }

    // =========================================================================
    // 10.3.3: grep enrichment
    // =========================================================================

    #[tokio::test]
    async fn test_grep_output_mode_files_with_matches() {
        let temp_dir = TempDir::new().unwrap();
        std::fs::write(temp_dir.path().join("a.txt"), "hello world\n").unwrap();
        std::fs::write(temp_dir.path().join("b.txt"), "goodbye world\n").unwrap();
        std::fs::write(temp_dir.path().join("c.txt"), "no match here\n").unwrap();
        let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

        let result = executor
            .grep_content(&serde_json::json!({
                "pattern": "world",
                "output_mode": "files_with_matches"
            }))
            .await
            .unwrap();

        match result {
            ToolResult::Success(output) => {
                assert!(
                    output.contains("a.txt"),
                    "should contain a.txt, got: {output}"
                );
                assert!(output.contains("b.txt"), "should contain b.txt");
                assert!(!output.contains("c.txt"), "should not contain c.txt");
                // files_with_matches should NOT include line numbers or content
                assert!(!output.contains("hello"), "should not contain line content");
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_grep_output_mode_count() {
        let temp_dir = TempDir::new().unwrap();
        std::fs::write(temp_dir.path().join("a.txt"), "foo\nfoo\nbar\n").unwrap();
        std::fs::write(temp_dir.path().join("b.txt"), "foo\nbaz\n").unwrap();
        let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

        let result = executor
            .grep_content(&serde_json::json!({
                "pattern": "foo",
                "output_mode": "count"
            }))
            .await
            .unwrap();

        match result {
            ToolResult::Success(output) => {
                assert!(
                    output.contains("a.txt:2"),
                    "should show a.txt:2, got: {output}"
                );
                assert!(output.contains("b.txt:1"), "should show b.txt:1");
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_grep_context_lines() {
        let temp_dir = TempDir::new().unwrap();
        std::fs::write(
            temp_dir.path().join("test.txt"),
            "aaa\nbbb\nccc\nTARGET\nddd\neee\nfff\n",
        )
        .unwrap();
        let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

        let result = executor
            .grep_content(&serde_json::json!({
                "pattern": "TARGET",
                "context_lines": 2
            }))
            .await
            .unwrap();

        match result {
            ToolResult::Success(output) => {
                assert!(
                    output.contains("bbb"),
                    "should contain context before, got: {output}"
                );
                assert!(output.contains("ccc"), "should contain context before");
                assert!(output.contains("TARGET"), "should contain match");
                assert!(output.contains("ddd"), "should contain context after");
                assert!(output.contains("eee"), "should contain context after");
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_grep_file_type_filter() {
        let temp_dir = TempDir::new().unwrap();
        std::fs::write(temp_dir.path().join("code.rs"), "fn hello() {}\n").unwrap();
        std::fs::write(temp_dir.path().join("notes.txt"), "fn hello() {}\n").unwrap();
        let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

        let result = executor
            .grep_content(&serde_json::json!({
                "pattern": "hello",
                "file_type": "rust"
            }))
            .await
            .unwrap();

        match result {
            ToolResult::Success(output) => {
                assert!(
                    output.contains("code.rs"),
                    "should find in .rs file, got: {output}"
                );
                assert!(
                    !output.contains("notes.txt"),
                    "should not find in .txt file"
                );
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_grep_head_limit() {
        let temp_dir = TempDir::new().unwrap();
        // Create many matching lines
        let content = (1..=20)
            .map(|i| format!("match_{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(temp_dir.path().join("test.txt"), &content).unwrap();
        let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

        let result = executor
            .grep_content(&serde_json::json!({
                "pattern": "match_",
                "head_limit": 5
            }))
            .await
            .unwrap();

        match result {
            ToolResult::Success(output) => {
                let lines: Vec<&str> = output.lines().collect();
                assert!(
                    lines.len() <= 5,
                    "should return at most 5 results, got {}",
                    lines.len()
                );
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_grep_path_restricts_search() {
        let temp_dir = TempDir::new().unwrap();
        std::fs::create_dir_all(temp_dir.path().join("src")).unwrap();
        std::fs::create_dir_all(temp_dir.path().join("tests")).unwrap();
        std::fs::write(temp_dir.path().join("src/code.rs"), "fn target() {}\n").unwrap();
        std::fs::write(temp_dir.path().join("tests/test.rs"), "fn target() {}\n").unwrap();
        let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

        let result = executor
            .grep_content(&serde_json::json!({
                "pattern": "target",
                "path": "src"
            }))
            .await
            .unwrap();

        match result {
            ToolResult::Success(output) => {
                assert!(
                    output.contains("src/code.rs"),
                    "should find in src/, got: {output}"
                );
                assert!(!output.contains("tests/"), "should not find in tests/");
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    // =========================================================================
    // 10.3.4: bash with timeout and description
    // =========================================================================

    #[tokio::test]
    async fn test_bash_with_description_runs_normally() {
        let temp_dir = TempDir::new().unwrap();
        let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

        let result = executor
            .execute_bash(&serde_json::json!({
                "command": "echo hello",
                "description": "Print a greeting"
            }))
            .await
            .unwrap();

        match result {
            ToolResult::Success(output) => {
                assert!(
                    output.contains("hello"),
                    "should run normally, got: {output}"
                );
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_bash_with_timeout_caps_at_max() {
        let temp_dir = TempDir::new().unwrap();
        let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

        // Should not panic or error when specifying a valid timeout
        let result = executor
            .execute_bash(&serde_json::json!({
                "command": "echo fast",
                "timeout": 5000
            }))
            .await
            .unwrap();

        assert!(matches!(result, ToolResult::Success(_)));
    }

    // =========================================================================
    // 11.3.2 / 11.3.7: execute_bash decomposition & validate+read pattern tests
    // =========================================================================

    #[test]
    fn test_validate_bash_command_blocks_dangerous_pattern() {
        let executor = ToolExecutor::new(PathBuf::from("/tmp"));
        let result = executor.validate_bash_command("rm -rf /");
        assert!(result.is_err(), "dangerous command should be rejected");
        match result.unwrap_err() {
            ToolResult::Error(msg) => {
                assert!(
                    msg.contains("Command blocked by security policy"),
                    "should mention security policy, got: {msg}"
                );
            }
            other => panic!("Expected ToolResult::Error, got {:?}", other),
        }
    }

    #[test]
    fn test_validate_bash_command_allows_safe_command() {
        let executor = ToolExecutor::new(PathBuf::from("/tmp"));
        let result = executor.validate_bash_command("echo hello");
        assert!(
            result.is_ok(),
            "safe command should be allowed: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_validate_bash_command_allowlist_mode_blocks_unlisted() {
        let policy = ToolExecutionPolicy {
            allowlist_mode: true,
            allowed_commands: vec![Regex::new(r"^echo ").expect("valid regex")],
            ..ToolExecutionPolicy::default()
        };
        let executor = ToolExecutor::new(PathBuf::from("/tmp")).with_policy(policy);

        let result = executor.validate_bash_command("ls -la");
        assert!(result.is_err(), "unlisted command should be blocked");
        match result.unwrap_err() {
            ToolResult::Error(msg) => {
                assert!(
                    msg.contains("not in allowlist"),
                    "should mention allowlist, got: {msg}"
                );
            }
            other => panic!("Expected ToolResult::Error, got {:?}", other),
        }
    }

    #[test]
    fn test_validate_bash_command_allowlist_mode_allows_listed() {
        let policy = ToolExecutionPolicy {
            allowlist_mode: true,
            allowed_commands: vec![Regex::new(r"^echo ").expect("valid regex")],
            ..ToolExecutionPolicy::default()
        };
        let executor = ToolExecutor::new(PathBuf::from("/tmp")).with_policy(policy);

        let result = executor.validate_bash_command("echo hello world");
        assert!(
            result.is_ok(),
            "listed command should be allowed: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_validate_bash_command_detects_escape_bypasses() {
        let executor = ToolExecutor::new(PathBuf::from("/tmp"));
        // r\m -rf / should be normalized to rm -rf / and blocked
        let result = executor.validate_bash_command(r"r\m -rf /");
        assert!(
            result.is_err(),
            "escaped dangerous command should be rejected"
        );
    }

    #[test]
    fn test_build_shell_command_without_sandbox() {
        let executor = ToolExecutor::new(PathBuf::from("/tmp"));
        let (program, args) = executor.build_shell_command("echo hello");

        // Without sandbox, should use the default shell
        #[cfg(unix)]
        assert_eq!(program, "sh", "should use sh on unix");
        #[cfg(windows)]
        assert_eq!(program, "cmd.exe", "should use cmd.exe on windows");

        assert!(
            args.last().map(|a| a.as_str()) == Some("echo hello"),
            "last arg should be the command, got: {args:?}"
        );
    }

    #[test]
    fn test_build_shell_command_with_sandbox() {
        let policy = ToolExecutionPolicy {
            sandbox_enabled: true,
            ..ToolExecutionPolicy::default()
        };
        let executor = ToolExecutor::new(PathBuf::from("/tmp")).with_policy(policy);
        let (program, args) = executor.build_shell_command("echo hello");

        // With sandbox enabled, the program/args should differ from the default shell
        // The exact output depends on platform, but it should wrap the command
        assert!(
            !program.is_empty(),
            "sandbox should produce a non-empty program"
        );
        assert!(
            !args.is_empty(),
            "sandbox should produce non-empty arguments"
        );
    }

    #[test]
    fn test_format_bash_output_success_no_truncation() {
        let output = std::process::Output {
            status: std::process::ExitStatus::default(),
            stdout: b"hello world\n".to_vec(),
            stderr: Vec::new(),
        };
        let result = format_bash_output(output, 1024);
        match result {
            ToolResult::Success(s) => {
                assert!(s.contains("hello world"), "should contain output, got: {s}");
                assert!(
                    !s.contains("truncated"),
                    "should not mention truncation, got: {s}"
                );
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[test]
    fn test_format_bash_output_success_with_truncation() {
        let output = std::process::Output {
            status: std::process::ExitStatus::default(),
            stdout: b"abcdefghij".to_vec(),
            stderr: Vec::new(),
        };
        let result = format_bash_output(output, 5);
        match result {
            ToolResult::Success(s) => {
                assert!(s.starts_with("abcde"), "should start with truncated output");
                assert!(
                    s.contains("Output truncated"),
                    "should mention truncation, got: {s}"
                );
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_validate_file_path_rejects_traversal() {
        let temp_dir = TempDir::new().unwrap();
        let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

        let result = executor.validate_file_path("../../etc/passwd").await;
        assert!(result.is_err(), "path traversal should be rejected");
        match result.unwrap_err() {
            ToolResult::Error(msg) => {
                assert!(
                    msg.contains("traversal") || msg.contains("Absolute"),
                    "should mention path issue, got: {msg}"
                );
            }
            other => panic!("Expected ToolResult::Error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_validate_file_path_accepts_valid_path() {
        let temp_dir = TempDir::new().unwrap();
        std::fs::write(temp_dir.path().join("test.txt"), "content").unwrap();
        let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

        let result = executor.validate_file_path("test.txt").await;
        assert!(
            result.is_ok(),
            "valid path should be accepted: {:?}",
            result.err()
        );
        let path = result.unwrap();
        assert!(
            path.ends_with("test.txt"),
            "should return the path, got: {path:?}"
        );
    }

    #[tokio::test]
    async fn test_validate_file_write_path_rejects_protected() {
        let temp_dir = TempDir::new().unwrap();
        let protected = temp_dir.path().join("protected");
        std::fs::create_dir_all(&protected).unwrap();
        std::fs::write(protected.join("secret.txt"), "secret").unwrap();

        let canonical_protected = protected.canonicalize().unwrap();
        let policy = ToolExecutionPolicy {
            protected_paths: vec![canonical_protected],
            ..ToolExecutionPolicy::default()
        };
        let executor = ToolExecutor::new(temp_dir.path().to_path_buf()).with_policy(policy);

        let result = executor
            .validate_file_write_path("protected/secret.txt")
            .await;
        assert!(result.is_err(), "protected path should be rejected");
        match result.unwrap_err() {
            ToolResult::Error(msg) => {
                assert!(
                    msg.contains("protected"),
                    "should mention protected, got: {msg}"
                );
            }
            other => panic!("Expected ToolResult::Error, got {:?}", other),
        }
    }

    // =========================================================================
    // 11.3.1 / 11.3.6: grep decomposition & shared walk pattern tests
    // =========================================================================

    #[tokio::test]
    async fn test_grep_invalid_regex_returns_error() {
        let temp_dir = TempDir::new().unwrap();
        std::fs::write(temp_dir.path().join("a.txt"), "hello\n").unwrap();
        let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

        let result = executor
            .grep_content(&serde_json::json!({"pattern": "[invalid("}))
            .await
            .unwrap();

        match result {
            ToolResult::Error(msg) => {
                assert!(
                    msg.contains("Invalid regex pattern"),
                    "should report invalid regex, got: {msg}"
                );
            }
            other => panic!("Expected Error for invalid regex, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_grep_binary_file_skipped() {
        let temp_dir = TempDir::new().unwrap();
        std::fs::write(temp_dir.path().join("binary.bin"), [0xFF, 0xFE, 0x00, 0x01]).unwrap();
        std::fs::write(temp_dir.path().join("text.txt"), "findme here\n").unwrap();
        let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

        let result = executor
            .grep_content(&serde_json::json!({
                "pattern": "findme",
                "output_mode": "files_with_matches"
            }))
            .await
            .unwrap();

        match result {
            ToolResult::Success(output) => {
                assert!(
                    output.contains("text.txt"),
                    "should find match in text file, got: {output}"
                );
                assert!(!output.contains("binary.bin"), "should skip binary file");
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_grep_empty_search_root() {
        let temp_dir = TempDir::new().unwrap();
        std::fs::create_dir_all(temp_dir.path().join("empty")).unwrap();
        let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

        let result = executor
            .grep_content(&serde_json::json!({
                "pattern": "anything",
                "path": "empty"
            }))
            .await
            .unwrap();

        match result {
            ToolResult::Success(output) => {
                assert!(
                    output.is_empty(),
                    "should return empty for empty directory, got: {output}"
                );
            }
            other => panic!("Expected Success (empty), got {:?}", other),
        }
    }

    #[test]
    fn test_walk_relative_files_returns_files_only() {
        let temp_dir = TempDir::new().unwrap();
        std::fs::create_dir_all(temp_dir.path().join("sub")).unwrap();
        std::fs::write(temp_dir.path().join("root.txt"), "r").unwrap();
        std::fs::write(temp_dir.path().join("sub/nested.txt"), "n").unwrap();

        let files: Vec<(PathBuf, PathBuf)> =
            ToolExecutor::walk_relative_files(temp_dir.path(), temp_dir.path());
        let rel_strs: Vec<String> = files
            .iter()
            .map(|(_, rel)| rel.to_string_lossy().to_string())
            .collect();

        assert!(
            rel_strs.contains(&"root.txt".to_string()),
            "should contain root.txt, got: {rel_strs:?}"
        );
        assert!(
            rel_strs.iter().any(|s| s.contains("nested.txt")),
            "should contain nested.txt, got: {rel_strs:?}"
        );
        for (abs, _rel) in &files {
            assert!(abs.is_file(), "walk should only return files: {:?}", abs);
        }
    }

    #[test]
    fn test_parse_grep_params_missing_pattern() {
        let input = serde_json::json!({"output_mode": "count"});
        let result = ToolExecutor::parse_grep_params(&input, Path::new("/tmp"));
        assert!(result.is_err(), "should error when pattern is missing");
    }

    #[test]
    fn test_parse_grep_params_invalid_regex() {
        let input = serde_json::json!({"pattern": "(unclosed"});
        let result = ToolExecutor::parse_grep_params(&input, Path::new("/tmp"));
        let inner = result.unwrap();
        assert!(
            matches!(inner, Err(ToolResult::Error(ref msg)) if msg.contains("Invalid regex")),
            "expected invalid regex error, got: {:?}",
            inner
        );
    }

    #[test]
    fn test_parse_grep_params_all_defaults() {
        let input = serde_json::json!({"pattern": "hello"});
        let result = ToolExecutor::parse_grep_params(&input, Path::new("/tmp"));
        let params = result.unwrap().unwrap();
        assert_eq!(params.output_mode, "content");
        assert!(params.context_lines.is_none());
        assert!(params.head_limit.is_none());
        assert!(params.file_glob.is_none());
        assert!(params.type_extensions.is_none());
        assert_eq!(params.search_root, Path::new("/tmp"));
    }

    #[test]
    fn test_format_match_without_context() {
        let lines = vec!["aaa", "bbb", "ccc"];
        let result = ToolExecutor::format_match_with_context(&lines, 1, None, "file.rs");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "file.rs:2: bbb");
    }

    #[test]
    fn test_format_match_with_context_lines() {
        let lines = vec!["aaa", "bbb", "ccc", "ddd", "eee"];
        let result = ToolExecutor::format_match_with_context(&lines, 2, Some(1), "file.rs");
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "file.rs-2- bbb");
        assert_eq!(result[1], "file.rs:3:ccc");
        assert_eq!(result[2], "file.rs-4- ddd");
    }

    #[tokio::test]
    async fn test_grep_case_insensitive() {
        let temp_dir = TempDir::new().unwrap();
        std::fs::write(temp_dir.path().join("a.txt"), "Hello World\nhello world\n").unwrap();
        let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

        let result = executor
            .grep_content(&serde_json::json!({
                "pattern": "HELLO",
                "case_insensitive": true,
                "output_mode": "count"
            }))
            .await
            .unwrap();

        match result {
            ToolResult::Success(output) => {
                assert!(
                    output.contains("a.txt:2"),
                    "case insensitive should match both lines, got: {output}"
                );
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_glob_files_uses_shared_walk() {
        let temp_dir = TempDir::new().unwrap();
        std::fs::create_dir_all(temp_dir.path().join("src")).unwrap();
        std::fs::write(temp_dir.path().join("src/main.rs"), "fn main()").unwrap();
        std::fs::write(temp_dir.path().join("src/lib.rs"), "pub mod").unwrap();
        std::fs::write(temp_dir.path().join("readme.md"), "# hi").unwrap();
        let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

        let result = executor
            .glob_files(&serde_json::json!({"pattern": "src/*.rs"}))
            .await
            .unwrap();

        match result {
            ToolResult::Success(output) => {
                assert!(
                    output.contains("src/main.rs"),
                    "should match src/main.rs, got: {output}"
                );
                assert!(output.contains("src/lib.rs"), "should match src/lib.rs");
                assert!(!output.contains("readme.md"), "should not match readme.md");
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_multi_edit_applies_two_edits() {
        let dir = TempDir::new().unwrap();
        let file1 = dir.path().join("a.txt");
        let file2 = dir.path().join("b.txt");
        std::fs::write(&file1, "hello world").unwrap();
        std::fs::write(&file2, "foo bar").unwrap();

        let executor = ToolExecutor::new(dir.path().to_path_buf());
        let input = serde_json::json!({
            "edits": [
                { "path": "a.txt", "old_string": "hello", "new_string": "hi" },
                { "path": "b.txt", "old_string": "foo", "new_string": "baz" }
            ]
        });
        let result = executor.multi_edit(&input).await.unwrap();
        match result {
            ToolResult::Success(msg) => {
                assert!(msg.contains("2 succeeded"));
                assert!(msg.contains("0 failed"));
            }
            other => panic!("Expected Success, got {other:?}"),
        }
        assert_eq!(std::fs::read_to_string(&file1).unwrap(), "hi world");
        assert_eq!(std::fs::read_to_string(&file2).unwrap(), "baz bar");
    }

    #[tokio::test]
    async fn test_multi_edit_partial_failure() {
        let dir = TempDir::new().unwrap();
        let file1 = dir.path().join("a.txt");
        std::fs::write(&file1, "hello world").unwrap();

        let executor = ToolExecutor::new(dir.path().to_path_buf());
        let input = serde_json::json!({
            "edits": [
                { "path": "a.txt", "old_string": "hello", "new_string": "hi" },
                { "path": "nonexistent.txt", "old_string": "x", "new_string": "y" }
            ]
        });
        let result = executor.multi_edit(&input).await.unwrap();
        match result {
            ToolResult::Success(msg) => {
                assert!(msg.contains("1 succeeded"));
                assert!(msg.contains("1 failed"));
            }
            other => panic!("Expected Success (partial), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_multi_edit_empty_edits() {
        let dir = TempDir::new().unwrap();
        let executor = ToolExecutor::new(dir.path().to_path_buf());
        let input = serde_json::json!({ "edits": [] });
        let result = executor.multi_edit(&input).await.unwrap();
        assert!(matches!(result, ToolResult::Error(_)));
    }
}
