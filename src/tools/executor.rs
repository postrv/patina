//! Tool executor for agentic capabilities.
//!
//! This module provides the core tool execution engine with security policy enforcement.

use anyhow::Result;
use glob::Pattern;
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;
use tracing::{debug, warn};
use walkdir::WalkDir;

use super::security::{normalize_command, ToolExecutionPolicy};
use super::{vision, web_fetch, web_search};
use crate::permissions::PermissionRequest;
use crate::shell::ShellConfig;

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
    fn check_symlink(&self, path: &str) -> std::result::Result<(), String> {
        let full_path = self.working_dir.join(path);

        // Use symlink_metadata to check the path itself, not what it points to
        // fs::metadata follows symlinks, symlink_metadata does not
        match std::fs::symlink_metadata(&full_path) {
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

    pub async fn execute(&self, call: ToolCall) -> Result<ToolResult> {
        match call.name.as_str() {
            "bash" => self.execute_bash(&call.input).await,
            "read_file" => self.read_file(&call.input).await,
            "write_file" => self.write_file(&call.input).await,
            "edit" => self.edit_file(&call.input).await,
            "list_files" => self.list_files(&call.input).await,
            "glob" => self.glob_files(&call.input).await,
            "grep" => self.grep_content(&call.input).await,
            "web_fetch" => self.web_fetch(&call.input).await,
            "web_search" => self.web_search(&call.input).await,
            "analyze_image" => self.analyze_image(&call.input).await,
            _ => Ok(ToolResult::Error(format!("Unknown tool: {}", call.name))),
        }
    }

    async fn execute_bash(&self, input: &serde_json::Value) -> Result<ToolResult> {
        let command = input
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing command"))?;

        // Normalize the command to detect escape-based bypasses (e.g., r\m -> rm)
        let normalized = normalize_command(command);

        // Check both original and normalized command against dangerous patterns
        for pattern in &self.policy.dangerous_patterns {
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

        // In allowlist mode, only allow commands that match an allowed pattern
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
                return Ok(ToolResult::Error(
                    "Command blocked: not in allowlist".to_string(),
                ));
            }
        }

        // Spawn the command with kill_on_drop to ensure process cleanup on timeout
        // Use platform-agnostic shell configuration (sh -c on Unix, cmd.exe /C on Windows)
        let shell = ShellConfig::default();
        let child = Command::new(&shell.command)
            .args(&shell.args)
            .arg(command)
            .current_dir(&self.working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        // Wait for the child with timeout
        // When timeout occurs, the future (and child) is dropped, triggering kill_on_drop
        match tokio::time::timeout(self.policy.command_timeout, child.wait_with_output()).await {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let combined = format!("{}{}", stdout, stderr);

                // P0-3: Truncate output if it exceeds max_output_size to prevent memory issues
                let (final_output, truncated) = if combined.len() > self.policy.max_output_size {
                    let truncated_output = combined
                        .chars()
                        .take(self.policy.max_output_size)
                        .collect::<String>();
                    warn!(
                        original_size = combined.len(),
                        max_size = self.policy.max_output_size,
                        "Bash command output truncated"
                    );
                    (truncated_output, true)
                } else {
                    (combined, false)
                };

                if output.status.success() {
                    let result = if truncated {
                        format!(
                            "{}\n\n[Output truncated: {} bytes exceeded {} byte limit]",
                            final_output,
                            stdout.len() + stderr.len(),
                            self.policy.max_output_size
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
                // Timeout occurred - child is automatically killed by kill_on_drop
                warn!(
                    timeout_ms = %self.policy.command_timeout.as_millis(),
                    "Bash command timed out and was killed"
                );
                Err(anyhow::anyhow!(
                    "Command timed out after {:?}",
                    self.policy.command_timeout
                ))
            }
        }
    }

    async fn read_file(&self, input: &serde_json::Value) -> Result<ToolResult> {
        let path = input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing path"))?;

        // Check for symlinks BEFORE path validation to prevent TOCTOU attacks
        if let Err(e) = self.check_symlink(path) {
            return Ok(ToolResult::Error(e));
        }

        // Validate path is within working directory
        let full_path = match self.validate_path(path) {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::Error(e)),
        };

        match tokio::fs::read_to_string(&full_path).await {
            Ok(content) => Ok(ToolResult::Success(content)),
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

        // Check for symlinks BEFORE path validation to prevent TOCTOU attacks
        if let Err(e) = self.check_symlink(path) {
            return Ok(ToolResult::Error(e));
        }

        // Validate path is within working directory and not protected
        let full_path = match self.validate_write_path(path) {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::Error(e)),
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

        // Check for symlinks BEFORE path validation to prevent TOCTOU attacks
        if let Err(e) = self.check_symlink(path) {
            return Ok(ToolResult::Error(e));
        }

        // Validate path is within working directory
        let full_path = match self.validate_path(path) {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::Error(e)),
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

        if match_count > 1 {
            return Ok(ToolResult::Error(format!(
                "Multiple matches found: {match_count} matches. Edit requires a unique match to avoid ambiguity."
            )));
        }

        // Create backup before editing
        if let Err(e) = self.create_backup(&full_path).await {
            return Ok(ToolResult::Error(format!("Failed to create backup: {e}")));
        }

        // Perform the replacement
        let new_content = content.replacen(old_string, new_string, 1);

        // Write the modified content
        if let Err(e) = tokio::fs::write(&full_path, &new_content).await {
            return Ok(ToolResult::Error(format!("Failed to write file: {e}")));
        }

        // Generate diff output
        let diff = Self::generate_diff(old_string, new_string);

        Ok(ToolResult::Success(format!(
            "Successfully replaced in {path}:\n{diff}"
        )))
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

        let mut matches = Vec::new();

        // Walk the directory tree
        for entry in WalkDir::new(&self.working_dir)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();

            // Skip directories
            if path.is_dir() {
                continue;
            }

            // Get relative path
            let relative = match path.strip_prefix(&self.working_dir) {
                Ok(r) => r,
                Err(_) => continue,
            };

            let relative_str = relative.to_string_lossy();

            // Check gitignore patterns
            if respect_gitignore && self.is_gitignored(&relative_str, &gitignore_patterns) {
                continue;
            }

            // Check if path matches the glob pattern
            if glob_pattern.matches(&relative_str) {
                matches.push(relative_str.to_string());
            }
        }

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

    /// Searches file contents for a pattern.
    ///
    /// # Arguments
    ///
    /// * `pattern` - The regex pattern to search for
    /// * `case_insensitive` - Whether to perform case-insensitive search (optional)
    /// * `file_pattern` - Glob pattern to filter files (optional)
    ///
    /// # Errors
    ///
    /// Returns an error if the regex pattern is invalid.
    async fn grep_content(&self, input: &serde_json::Value) -> Result<ToolResult> {
        let pattern = input
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing pattern"))?;

        let case_insensitive = input
            .get("case_insensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let file_pattern = input
            .get("file_pattern")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Compile the regex pattern
        let regex = match if case_insensitive {
            regex::RegexBuilder::new(pattern)
                .case_insensitive(true)
                .build()
        } else {
            Regex::new(pattern)
        } {
            Ok(r) => r,
            Err(e) => {
                debug!(
                    pattern = %pattern,
                    error = %e,
                    "Invalid regex pattern"
                );
                return Ok(ToolResult::Error(format!("Invalid regex pattern: {e}")));
            }
        };

        // Compile file filter pattern if provided
        let file_glob = file_pattern.as_ref().and_then(|p| Pattern::new(p).ok());

        let mut results = Vec::new();

        // Walk the directory tree
        for entry in WalkDir::new(&self.working_dir)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();

            // Skip directories
            if path.is_dir() {
                continue;
            }

            // Get relative path
            let relative = match path.strip_prefix(&self.working_dir) {
                Ok(r) => r,
                Err(_) => continue,
            };

            let relative_str = relative.to_string_lossy();

            // Apply file pattern filter if provided
            if let Some(ref glob) = file_glob {
                let filename = relative
                    .file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_default();
                if !glob.matches(&filename) {
                    continue;
                }
            }

            // Read file content (skip binary files)
            let content = match fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue, // Skip files we can't read as text
            };

            // Search for matches
            for (line_num, line) in content.lines().enumerate() {
                if regex.is_match(line) {
                    results.push(format!("{}:{}: {}", relative_str, line_num + 1, line));
                }
            }
        }

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

        let tool = web_fetch::WebFetchTool::new(web_fetch::WebFetchConfig::default());

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

        // Check for symlinks BEFORE path validation to prevent TOCTOU attacks
        if let Err(e) = self.check_symlink(path) {
            return Ok(ToolResult::Error(e));
        }

        // Validate path is within working directory
        let full_path = match self.validate_path(path) {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::Error(e)),
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
}
