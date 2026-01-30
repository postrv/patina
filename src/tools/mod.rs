//! Tool execution for agentic capabilities.
//!
//! This module provides secure tool execution including:
//! - Bash command execution with security policy
//! - File operations with path traversal protection
//! - Edit operations with diff generation
//! - Glob pattern matching for file discovery
//! - Grep content search with regex support
//! - Hook integration via `HookedToolExecutor`

use anyhow::Result;
use glob::Pattern;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use walkdir::WalkDir;

use crate::hooks::{HookDecision, HookManager};

/// Static collection of dangerous command patterns.
///
/// These patterns are compiled once on first access, ensuring:
/// - No runtime panics from invalid regex (patterns validated at initialization)
/// - No repeated compilation cost when creating new `ToolExecutionPolicy` instances
/// - Consistent pattern set across all policy instances
static DANGEROUS_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        // Destructive file operations
        Regex::new(r"rm\s+-rf\s+/").expect("invalid regex: rm -rf"),
        Regex::new(r"rm\s+-fr\s+/").expect("invalid regex: rm -fr"),
        Regex::new(r"rm\s+--no-preserve-root").expect("invalid regex: rm --no-preserve-root"),
        // Privilege escalation - comprehensive patterns
        Regex::new(r"sudo\s+").expect("invalid regex: sudo"),
        Regex::new(r"\bsu\s+-").expect("invalid regex: su -"),
        Regex::new(r"\bsu\s+root\b").expect("invalid regex: su root"),
        Regex::new(r"\bsu\s*$").expect("invalid regex: bare su"),
        Regex::new(r"doas\s+").expect("invalid regex: doas"),
        Regex::new(r"\bpkexec\b").expect("invalid regex: pkexec"),
        Regex::new(r"\brunuser\b").expect("invalid regex: runuser"),
        // Dangerous permissions
        Regex::new(r"chmod\s+777").expect("invalid regex: chmod 777"),
        Regex::new(r"chmod\s+-R\s+777").expect("invalid regex: chmod -R 777"),
        Regex::new(r"chmod\s+u\+s").expect("invalid regex: chmod setuid"),
        // Disk/filesystem destruction
        Regex::new(r"mkfs\.").expect("invalid regex: mkfs"),
        Regex::new(r"dd\s+if=.+of=/dev/").expect("invalid regex: dd to device"),
        Regex::new(r">\s*/dev/sd[a-z]").expect("invalid regex: redirect to sd"),
        Regex::new(r">\s*/dev/nvme").expect("invalid regex: redirect to nvme"),
        // Fork bombs and resource exhaustion
        Regex::new(r":\(\)\s*\{\s*:\|:&\s*\}\s*;").expect("invalid regex: fork bomb"),
        // Remote code execution patterns
        Regex::new(r"curl\s+.+\|\s*(ba)?sh").expect("invalid regex: curl pipe sh"),
        Regex::new(r"wget\s+.+\|\s*(ba)?sh").expect("invalid regex: wget pipe sh"),
        Regex::new(r"curl\s+.+\|\s*sudo").expect("invalid regex: curl pipe sudo"),
        Regex::new(r"wget\s+.+\|\s*sudo").expect("invalid regex: wget pipe sudo"),
        // System disruption
        Regex::new(r"\bshutdown\b").expect("invalid regex: shutdown"),
        Regex::new(r"\breboot\b").expect("invalid regex: reboot"),
        Regex::new(r"\bhalt\b").expect("invalid regex: halt"),
        Regex::new(r"\bpoweroff\b").expect("invalid regex: poweroff"),
        // History manipulation (hiding tracks)
        Regex::new(r"history\s+-c").expect("invalid regex: history clear"),
        Regex::new(r">\s*~/\.bash_history").expect("invalid regex: bash_history redirect"),
        // Dangerous eval patterns
        Regex::new(r"\beval\s+\$").expect("invalid regex: eval var"),
        Regex::new(r#"\beval\s+["'$]"#).expect("invalid regex: eval string"),
        // Command substitution patterns
        Regex::new(r"\$\(\s*which\s+").expect("invalid regex: which substitution"),
        Regex::new(r"`\s*which\s+").expect("invalid regex: which backtick"),
        Regex::new(r"\$\(\s*printf\s+").expect("invalid regex: printf substitution"),
        // Encoded command execution patterns
        Regex::new(r"base64\s+(-d|--decode).*\|\s*(ba)?sh").expect("invalid regex: base64 decode"),
        Regex::new(r"\|\s*base64\s+(-d|--decode).*\|\s*(ba)?sh")
            .expect("invalid regex: piped base64"),
        Regex::new(r#"printf\s+["']\\x[0-9a-fA-F]"#).expect("invalid regex: printf hex"),
    ]
});

/// Tool executor with security policy enforcement.
pub struct ToolExecutor {
    working_dir: PathBuf,
    policy: ToolExecutionPolicy,
}

/// Security policy for tool execution.
///
/// # Security Modes
///
/// The policy supports two security modes:
///
/// - **Blocklist mode** (default): Commands are allowed unless they match a dangerous pattern.
///   Good for general-purpose use where flexibility is needed.
///
/// - **Allowlist mode**: Commands are blocked unless they match an allowed pattern.
///   More restrictive, suitable for high-security environments. Enable by setting
///   `allowlist_mode = true` and providing patterns in `allowed_commands`.
///
/// In both modes, dangerous patterns are always checked and will block matching commands.
pub struct ToolExecutionPolicy {
    /// Patterns that match dangerous commands (always blocked).
    pub dangerous_patterns: Vec<Regex>,
    /// Paths that are protected from write operations.
    pub protected_paths: Vec<PathBuf>,
    /// Maximum allowed file size for write operations.
    pub max_file_size: usize,
    /// Timeout for command execution.
    pub command_timeout: Duration,
    /// Enable allowlist mode (default: false).
    ///
    /// When enabled, only commands matching `allowed_commands` will be permitted.
    /// Dangerous patterns are still enforced on top of the allowlist.
    pub allowlist_mode: bool,
    /// Patterns for commands that are allowed in allowlist mode.
    ///
    /// Only used when `allowlist_mode` is true.
    pub allowed_commands: Vec<Regex>,
}

impl Default for ToolExecutionPolicy {
    fn default() -> Self {
        Self {
            dangerous_patterns: DANGEROUS_PATTERNS.clone(),
            protected_paths: vec![
                PathBuf::from("/etc"),
                PathBuf::from("/usr"),
                PathBuf::from("/bin"),
            ],
            max_file_size: 10 * 1024 * 1024,
            command_timeout: Duration::from_secs(300),
            allowlist_mode: false,
            allowed_commands: vec![],
        }
    }
}

/// Normalizes a command string by removing shell escape characters.
///
/// This helps detect bypass attempts where characters are escaped to avoid
/// pattern matching (e.g., `r\m` becoming `rm` after shell processing).
fn normalize_command(cmd: &str) -> String {
    let mut result = String::with_capacity(cmd.len());
    let mut chars = cmd.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            // Skip the backslash and include the next character literally
            // unless it's a special escape sequence we want to preserve
            if let Some(&next) = chars.peek() {
                match next {
                    // Preserve common escape sequences that don't affect command names
                    'n' | 't' | 'r' | '0' | 'x' => {
                        result.push(c);
                        result.push(chars.next().unwrap());
                    }
                    // For letters, the backslash is often used to bypass filters
                    // e.g., r\m -> rm, so we skip the backslash
                    'a'..='z' | 'A'..='Z' => {
                        result.push(chars.next().unwrap());
                    }
                    // For other characters, preserve both
                    _ => {
                        result.push(c);
                        result.push(chars.next().unwrap());
                    }
                }
            } else {
                result.push(c);
            }
        } else {
            result.push(c);
        }
    }

    result
}

#[derive(Debug)]
pub struct ToolCall {
    pub name: String,
    pub input: serde_json::Value,
}

#[derive(Debug)]
pub enum ToolResult {
    Success(String),
    Error(String),
    Cancelled,
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
                    return Err("Path traversal outside working directory".to_string());
                }
                full_path
            }
        };

        // Verify the canonical path starts with the working directory
        if !canonical_full_path.starts_with(&canonical_working_dir) {
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
                return Ok(ToolResult::Error(
                    "Command blocked: not in allowlist".to_string(),
                ));
            }
        }

        // Spawn the command with kill_on_drop to ensure process cleanup on timeout
        let child = Command::new("sh")
            .arg("-c")
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

                if output.status.success() {
                    Ok(ToolResult::Success(format!("{}{}", stdout, stderr)))
                } else {
                    Ok(ToolResult::Error(format!(
                        "Exit code {}: {}{}",
                        output.status.code().unwrap_or(-1),
                        stdout,
                        stderr
                    )))
                }
            }
            Ok(Err(e)) => Err(e.into()),
            Err(_) => {
                // Timeout occurred - child is automatically killed by kill_on_drop
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
            Err(e) => Ok(ToolResult::Error(format!("Failed to read file: {}", e))),
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
            Err(e) => Ok(ToolResult::Error(format!("Failed to write file: {}", e))),
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
                return Ok(ToolResult::Error(format!(
                    "Failed to list directory '{}': {}",
                    path, e
                )))
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
            Err(e) => return Ok(ToolResult::Error(format!("Invalid glob pattern: {e}"))),
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
            Err(e) => return Ok(ToolResult::Error(format!("Invalid regex pattern: {e}"))),
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
}

/// Tool executor with hook integration.
///
/// Wraps `ToolExecutor` to automatically fire lifecycle hooks before and after
/// tool execution. Use this when hooks need to be integrated into tool execution.
///
/// # Hook Events
///
/// - `PreToolUse` - Fired before tool execution. Can block execution by returning exit code 2.
/// - `PostToolUse` - Fired after successful tool execution.
/// - `PostToolUseFailure` - Fired after failed tool execution.
///
/// # Examples
///
/// ```no_run
/// use rct::tools::{HookedToolExecutor, ToolCall};
/// use rct::hooks::HookManager;
/// use std::path::PathBuf;
/// use serde_json::json;
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let manager = HookManager::new("session-123".to_string());
///     let executor = HookedToolExecutor::new(PathBuf::from("."), manager);
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
    inner: ToolExecutor,
    hooks: HookManager,
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
            inner: ToolExecutor::new(working_dir),
            hooks: hook_manager,
        }
    }

    /// Creates a new hooked tool executor with a custom policy.
    #[must_use]
    pub fn with_policy(mut self, policy: ToolExecutionPolicy) -> Self {
        self.inner = self.inner.with_policy(policy);
        self
    }

    /// Executes a tool call with hook integration.
    ///
    /// This method:
    /// 1. Fires `PreToolUse` hook - if it returns Block, returns `ToolResult::Cancelled`
    /// 2. Executes the actual tool
    /// 3. Fires `PostToolUse` on success or `PostToolUseFailure` on failure
    ///
    /// # Errors
    ///
    /// Returns an error if hook execution or tool execution fails.
    pub async fn execute(&self, call: ToolCall) -> Result<ToolResult> {
        let tool_input = call.input.clone();
        let tool_name = call.name.clone();

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
            ToolResult::Cancelled => {
                // No hook for cancelled - it was already blocked by PreToolUse
            }
        }

        Ok(result)
    }
}
