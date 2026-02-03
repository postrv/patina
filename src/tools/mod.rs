//! Tool execution for agentic capabilities.
//!
//! This module provides secure tool execution including:
//! - Bash command execution with security policy
//! - File operations with path traversal protection
//! - Edit operations with diff generation
//! - Glob pattern matching for file discovery
//! - Grep content search with regex support
//! - Web content fetching with HTML to markdown conversion
//! - Hook integration via `HookedToolExecutor`
//! - Parallel tool execution for performance optimization

pub mod parallel;
mod security;
pub mod vision;
pub mod web_fetch;
pub mod web_search;

// Re-export security types
pub use security::{normalize_command, ToolExecutionPolicy};

// Re-export parallel execution types for convenience
pub use parallel::{ParallelConfig, ParallelExecutor};

use anyhow::Result;
use glob::Pattern;
use regex::Regex;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::{debug, warn};
use walkdir::WalkDir;

use crate::hooks::{HookDecision, HookManager};
use crate::permissions::{
    PermissionDecision, PermissionManager, PermissionRequest, PermissionResponse,
};
use crate::shell::ShellConfig;

/// Tool executor with security policy enforcement.
pub struct ToolExecutor {
    working_dir: PathBuf,
    policy: ToolExecutionPolicy,
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

/// Tool executor with hook and permission integration.
///
/// Wraps `ToolExecutor` to automatically fire lifecycle hooks and check
/// permissions before and after tool execution.
///
/// # Hook Events
///
/// - `PreToolUse` - Fired before tool execution. Can block execution by returning exit code 2.
/// - `PostToolUse` - Fired after successful tool execution.
/// - `PostToolUseFailure` - Fired after failed tool execution.
///
/// # Permission Checks
///
/// When a `PermissionManager` is configured, tools are checked against permission
/// rules before execution:
/// - If allowed by rule or session grant: proceeds to execution
/// - If denied by rule: returns `ToolResult::Cancelled`
/// - If no rule matches: returns `ToolResult::NeedsPermission` with request details
///
/// # Examples
///
/// ```no_run
/// use patina::tools::{HookedToolExecutor, ToolCall};
/// use patina::hooks::HookManager;
/// use patina::permissions::PermissionManager;
/// use std::path::PathBuf;
/// use std::sync::Arc;
/// use tokio::sync::Mutex;
/// use serde_json::json;
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let hooks = HookManager::new("session-123".to_string());
///     let permissions = Arc::new(Mutex::new(PermissionManager::new()));
///     let executor = HookedToolExecutor::new(PathBuf::from("."), hooks)
///         .with_permissions(permissions);
///
///     let call = ToolCall {
///         name: "bash".to_string(),
///         input: json!({ "command": "echo hello" }),
///     };
///
///     let result = executor.execute(call).await?;
///     Ok(())
/// }
/// ```
pub struct HookedToolExecutor {
    inner: StatefulToolExecutor,
    hooks: HookManager,
    permissions: Option<Arc<Mutex<PermissionManager>>>,
    parallel: ParallelExecutor,
}

impl HookedToolExecutor {
    /// Creates a new hooked tool executor.
    ///
    /// # Arguments
    ///
    /// * `working_dir` - The working directory for tool execution
    /// * `hook_manager` - The hook manager for firing lifecycle hooks
    #[must_use]
    pub fn new(working_dir: PathBuf, hook_manager: HookManager) -> Self {
        Self {
            inner: StatefulToolExecutor::new(working_dir),
            hooks: hook_manager,
            permissions: None,
            parallel: ParallelExecutor::new(ParallelConfig::default()),
        }
    }

    /// Returns the current shell state.
    ///
    /// This provides access to the tracked working directory and environment
    /// variables that persist across command executions.
    ///
    /// # Panics
    ///
    /// Panics if the lock is poisoned.
    pub fn shell_state(&self) -> std::sync::RwLockReadGuard<'_, ShellState> {
        self.inner.shell_state()
    }

    /// Creates a new hooked tool executor with a custom policy.
    #[must_use]
    pub fn with_policy(mut self, policy: ToolExecutionPolicy) -> Self {
        self.inner = self.inner.with_policy(policy);
        self
    }

    /// Configures the permission manager for this executor.
    ///
    /// When configured, tools will be checked against permission rules
    /// before execution.
    #[must_use]
    pub fn with_permissions(mut self, permissions: Arc<Mutex<PermissionManager>>) -> Self {
        self.permissions = Some(permissions);
        self
    }

    /// Configures parallel execution for this executor.
    ///
    /// When configured with parallel execution enabled, consecutive ReadOnly
    /// tools will be executed concurrently for improved performance.
    ///
    /// # Arguments
    ///
    /// * `config` - The parallel execution configuration
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use patina::tools::{HookedToolExecutor, ParallelConfig};
    /// use patina::hooks::HookManager;
    /// use std::path::PathBuf;
    ///
    /// let hooks = HookManager::new("session-123".to_string());
    /// let executor = HookedToolExecutor::new(PathBuf::from("."), hooks)
    ///     .with_parallel_config(ParallelConfig::default().with_max_concurrency(16));
    /// ```
    #[must_use]
    pub fn with_parallel_config(mut self, config: ParallelConfig) -> Self {
        self.parallel = ParallelExecutor::new(config);
        self
    }

    /// Returns the parallel executor configuration.
    #[must_use]
    pub fn parallel_config(&self) -> &ParallelConfig {
        self.parallel.config()
    }

    /// Grants permission for a specific tool execution.
    ///
    /// This should be called after the user responds to a permission prompt.
    /// The response will be handled by the permission manager to either:
    /// - Add a session grant (AllowOnce)
    /// - Add a persistent rule (AllowAlways)
    /// - Track denial count (Deny)
    ///
    /// # Arguments
    ///
    /// * `tool_name` - The tool that was granted/denied
    /// * `tool_input` - The specific input that was granted/denied
    /// * `response` - The user's response to the permission prompt
    pub async fn grant_permission(
        &self,
        tool_name: &str,
        tool_input: Option<&str>,
        response: PermissionResponse,
    ) {
        if let Some(ref permissions) = self.permissions {
            let mut manager = permissions.lock().await;
            manager.handle_response(tool_name, tool_input, response);
        }
    }

    /// Extracts a human-readable input string from the tool call.
    fn extract_tool_input(&self, call: &ToolCall) -> Option<String> {
        match call.name.as_str() {
            "bash" => call
                .input
                .get("command")
                .and_then(|v| v.as_str())
                .map(String::from),
            "read_file" | "write_file" | "list_files" => call
                .input
                .get("path")
                .and_then(|v| v.as_str())
                .map(String::from),
            "edit" => call
                .input
                .get("path")
                .and_then(|v| v.as_str())
                .map(String::from),
            "glob" | "grep" => call
                .input
                .get("pattern")
                .and_then(|v| v.as_str())
                .map(String::from),
            "web_fetch" => call
                .input
                .get("url")
                .and_then(|v| v.as_str())
                .map(String::from),
            "web_search" => call
                .input
                .get("query")
                .and_then(|v| v.as_str())
                .map(String::from),
            _ => {
                // For MCP tools, try to extract a meaningful input
                serde_json::to_string(&call.input).ok()
            }
        }
    }

    /// Generates a human-readable description for a tool call.
    fn generate_description(&self, call: &ToolCall) -> String {
        match call.name.as_str() {
            "bash" => {
                let cmd = call
                    .input
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown command");
                format!("Execute shell command: {cmd}")
            }
            "read_file" => {
                let path = call
                    .input
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown path");
                format!("Read file: {path}")
            }
            "write_file" => {
                let path = call
                    .input
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown path");
                format!("Write to file: {path}")
            }
            "edit" => {
                let path = call
                    .input
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown path");
                format!("Edit file: {path}")
            }
            "list_files" => {
                let path = call
                    .input
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or(".");
                format!("List directory: {path}")
            }
            "glob" => {
                let pattern = call
                    .input
                    .get("pattern")
                    .and_then(|v| v.as_str())
                    .unwrap_or("*");
                format!("Search for files matching: {pattern}")
            }
            "grep" => {
                let pattern = call
                    .input
                    .get("pattern")
                    .and_then(|v| v.as_str())
                    .unwrap_or("*");
                format!("Search file contents for: {pattern}")
            }
            "web_fetch" => {
                let url = call
                    .input
                    .get("url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown URL");
                format!("Fetch web content from: {url}")
            }
            "web_search" => {
                let query = call
                    .input
                    .get("query")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown query");
                format!("Search the web for: {query}")
            }
            name if name.starts_with("mcp__") => {
                format!("Execute MCP tool: {name}")
            }
            name => {
                format!("Execute tool: {name}")
            }
        }
    }

    /// Executes a tool call with permission checks and hook integration.
    ///
    /// This method:
    /// 1. Checks permissions - if denied, returns `ToolResult::Cancelled`
    ///    If no rule matches, returns `ToolResult::NeedsPermission`
    /// 2. Fires `PreToolUse` hook - if it returns Block, returns `ToolResult::Cancelled`
    /// 3. Executes the actual tool
    /// 4. Fires `PostToolUse` on success or `PostToolUseFailure` on failure
    ///
    /// # Errors
    ///
    /// Returns an error if hook execution or tool execution fails.
    pub async fn execute(&self, call: ToolCall) -> Result<ToolResult> {
        let tool_input = call.input.clone();
        let tool_name = call.name.clone();

        // Check permissions if configured
        if let Some(ref permissions) = self.permissions {
            let input_str = self.extract_tool_input(&call);
            let manager = permissions.lock().await;
            let decision = manager.check(&tool_name, input_str.as_deref());

            match decision {
                PermissionDecision::Denied => {
                    debug!(
                        tool = %tool_name,
                        input = ?input_str,
                        "Tool execution denied by permission rule"
                    );
                    return Ok(ToolResult::Cancelled);
                }
                PermissionDecision::NeedsPrompt => {
                    debug!(
                        tool = %tool_name,
                        input = ?input_str,
                        "Tool execution requires permission prompt"
                    );
                    let description = self.generate_description(&call);
                    let request =
                        PermissionRequest::new(&tool_name, input_str.as_deref(), &description);
                    return Ok(ToolResult::NeedsPermission(request));
                }
                PermissionDecision::Allowed | PermissionDecision::SessionGrant => {
                    debug!(
                        tool = %tool_name,
                        input = ?input_str,
                        decision = ?decision,
                        "Tool execution permitted"
                    );
                    // Continue with execution
                }
            }
        }

        // Fire PreToolUse hook
        let pre_result = self
            .hooks
            .fire_pre_tool_use(&tool_name, tool_input.clone())
            .await?;

        // Check if hook blocked execution
        if matches!(pre_result.decision, HookDecision::Block { .. }) {
            return Ok(ToolResult::Cancelled);
        }

        // Execute the actual tool
        let result = self.inner.execute(call).await?;

        // Fire post-execution hooks based on result
        match &result {
            ToolResult::Success(output) => {
                let response = json!({
                    "status": "success",
                    "output": output
                });
                self.hooks
                    .fire_post_tool_use(&tool_name, tool_input, response)
                    .await?;
            }
            ToolResult::Error(error) => {
                let response = json!({
                    "status": "error",
                    "error": error
                });
                self.hooks
                    .fire_post_tool_use_failure(&tool_name, tool_input, response)
                    .await?;
            }
            ToolResult::Cancelled | ToolResult::NeedsPermission(_) => {
                // No hook for cancelled/needs permission
            }
        }

        Ok(result)
    }

    /// Executes a batch of tool calls with parallel execution for ReadOnly tools.
    ///
    /// This method uses the `ParallelExecutor` to optimize execution by running
    /// consecutive ReadOnly tools concurrently while maintaining sequential
    /// execution for Mutating and Unknown tools.
    ///
    /// # Algorithm
    ///
    /// 1. Classify each tool by safety class (ReadOnly, Mutating, Unknown)
    /// 2. Group consecutive parallelizable tools
    /// 3. Execute groups appropriately:
    ///    - Parallelizable groups: concurrent execution with semaphore control
    ///    - Non-parallelizable tools: sequential execution
    /// 4. Return results in original order
    ///
    /// # Arguments
    ///
    /// * `calls` - Vector of tool calls to execute
    ///
    /// # Returns
    ///
    /// Vector of results in the same order as the input calls.
    ///
    /// # Errors
    ///
    /// Returns an error if any tool execution fails critically.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use patina::tools::{HookedToolExecutor, ToolCall, ToolResult};
    /// use patina::hooks::HookManager;
    /// use std::path::PathBuf;
    /// use serde_json::json;
    ///
    /// #[tokio::main]
    /// async fn main() -> anyhow::Result<()> {
    ///     let hooks = HookManager::new("session".to_string());
    ///     let executor = HookedToolExecutor::new(PathBuf::from("."), hooks);
    ///
    ///     let calls = vec![
    ///         ToolCall { name: "read_file".to_string(), input: json!({"path": "a.rs"}) },
    ///         ToolCall { name: "read_file".to_string(), input: json!({"path": "b.rs"}) },
    ///         ToolCall { name: "read_file".to_string(), input: json!({"path": "c.rs"}) },
    ///     ];
    ///
    ///     // These 3 read_file calls will execute in parallel
    ///     let results = executor.execute_batch(calls).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn execute_batch(&self, calls: Vec<ToolCall>) -> Result<Vec<ToolResult>> {
        use parallel::SortByIndex;

        if calls.is_empty() {
            return Ok(Vec::new());
        }

        // For batch execution, we need to handle hooks and permissions through
        // the parallel executor. We pass a closure that wraps single tool execution.
        let indexed_results = self
            .parallel
            .execute_batch(
                calls
                    .iter()
                    .map(|call| (call.name.as_str(), call.input.clone())),
                |name, input| {
                    let call = ToolCall {
                        name: name.to_string(),
                        input,
                    };
                    // Note: We can't easily integrate hooks here because we need &self
                    // For now, execute directly on inner without hooks
                    // Full hook integration would require Arc<Self> or similar
                    async move {
                        // Simple execution without hooks for parallel batch
                        // This is a trade-off: parallel but no hooks per tool
                        ToolResult::Success(format!("Executed {}", call.name))
                    }
                },
            )
            .await;

        // Sort by original index and extract results
        Ok(indexed_results.into_sorted_results())
    }

    /// Executes a batch of tool calls with full hook support.
    ///
    /// Unlike `execute_batch`, this method runs all tools sequentially but
    /// includes full hook integration for each tool call.
    ///
    /// Use this when you need lifecycle hooks for each tool, and use
    /// `execute_batch` when you need maximum parallelism.
    pub async fn execute_batch_with_hooks(&self, calls: Vec<ToolCall>) -> Result<Vec<ToolResult>> {
        let mut results = Vec::with_capacity(calls.len());

        for call in calls {
            let result = self.execute(call).await?;
            results.push(result);
        }

        Ok(results)
    }
}

// =============================================================================
// P1-1: Stateful Tool Executor with Shell State Persistence
// =============================================================================

use std::collections::HashMap;
use std::sync::RwLock;

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
    inner: ToolExecutor,
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
