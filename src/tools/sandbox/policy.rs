//! Application-level sandbox policy enforcement.
//!
//! Provides [`SandboxPolicy`], which validates filesystem read/write and
//! command execution requests against the allowlist/denylist rules in
//! [`SandboxConfig`]. This operates at the application level, complementing
//! the OS-level kernel sandboxing in [`macos`](super::macos) and
//! [`linux`](super::linux).
//!
//! # Architecture
//!
//! ```text
//! Tool Request
//!     ↓
//! SandboxPolicy::check_read / check_write / check_execute
//!     ├─ Sandbox disabled → Allow
//!     ├─ Path in deny list → Deny (deny always wins)
//!     ├─ Path in allow list → Allow
//!     └─ Not in any list → Deny (default-deny when enabled)
//! ```
//!
//! # Example
//!
//! ```
//! use std::path::{Path, PathBuf};
//! use patina::tools::sandbox::{SandboxConfig, policy::SandboxPolicy};
//!
//! let config = SandboxConfig {
//!     enabled: true,
//!     allow_read: vec![PathBuf::from("/home/user/project")],
//!     deny_read: vec![PathBuf::from("/home/user/project/.env")],
//!     allow_write: vec![PathBuf::from("/home/user/project/src")],
//!     deny_write: vec![],
//!     allow_execute: vec!["git *".to_string()],
//!     deny_execute: vec!["git push *".to_string()],
//!     allow_network: true,
//! };
//!
//! let policy = SandboxPolicy::new(config);
//! assert!(policy.check_read(Path::new("/home/user/project/src/main.rs")).is_ok());
//! assert!(policy.check_read(Path::new("/home/user/project/.env")).is_err());
//! ```

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use glob::Pattern;

use super::config::SandboxConfig;

/// Application-level sandbox that enforces access control policies.
///
/// Validates read, write, and execute operations against configured
/// allowlists and denylists. Deny rules always take priority over allow
/// rules. When the sandbox is enabled and allow lists are empty, all
/// operations are denied by default.
///
/// # Examples
///
/// ```
/// use std::path::{Path, PathBuf};
/// use patina::tools::sandbox::{SandboxConfig, policy::SandboxPolicy};
///
/// // Disabled sandbox allows everything
/// let policy = SandboxPolicy::new(SandboxConfig::default());
/// assert!(policy.check_read(Path::new("/any/path")).is_ok());
///
/// // Enabled sandbox with empty lists denies everything
/// let config = SandboxConfig {
///     enabled: true,
///     ..SandboxConfig::default()
/// };
/// let policy = SandboxPolicy::new(config);
/// assert!(policy.check_read(Path::new("/any/path")).is_err());
/// ```
pub struct SandboxPolicy {
    config: SandboxConfig,
}

impl SandboxPolicy {
    /// Creates a new sandbox policy from the given configuration.
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::tools::sandbox::{SandboxConfig, policy::SandboxPolicy};
    ///
    /// let policy = SandboxPolicy::new(SandboxConfig::default());
    /// assert!(!policy.is_enabled());
    /// ```
    #[must_use]
    pub fn new(config: SandboxConfig) -> Self {
        Self { config }
    }

    /// Returns whether the sandbox policy is enabled.
    ///
    /// When disabled, all operations are permitted without checks.
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::tools::sandbox::{SandboxConfig, policy::SandboxPolicy};
    ///
    /// let policy = SandboxPolicy::new(SandboxConfig {
    ///     enabled: true,
    ///     ..SandboxConfig::default()
    /// });
    /// assert!(policy.is_enabled());
    /// ```
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Validates that the given path is allowed for reading.
    ///
    /// When the sandbox is disabled, all reads are permitted. When enabled,
    /// the path is checked against deny and allow lists. Deny always wins.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The path matches a `deny_read` pattern
    /// - The path does not match any `allow_read` pattern (default-deny)
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::{Path, PathBuf};
    /// use patina::tools::sandbox::{SandboxConfig, policy::SandboxPolicy};
    ///
    /// let config = SandboxConfig {
    ///     enabled: true,
    ///     allow_read: vec![PathBuf::from("/home")],
    ///     ..SandboxConfig::default()
    /// };
    /// let policy = SandboxPolicy::new(config);
    /// assert!(policy.check_read(Path::new("/home/user/file.txt")).is_ok());
    /// assert!(policy.check_read(Path::new("/etc/passwd")).is_err());
    /// ```
    pub fn check_read(&self, path: &Path) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }

        let normalized = normalize_path(path);

        if path_matches_any(&normalized, &self.config.deny_read) {
            bail!(
                "Sandbox: read access denied for {} (matched deny_read pattern)",
                normalized.display()
            );
        }

        if path_matches_any(&normalized, &self.config.allow_read) {
            return Ok(());
        }

        bail!(
            "Sandbox: read access denied for {} (not in allow_read list)",
            normalized.display()
        );
    }

    /// Validates that the given path is allowed for writing.
    ///
    /// When the sandbox is disabled, all writes are permitted. When enabled,
    /// the path is checked against deny and allow lists. Deny always wins.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The path matches a `deny_write` pattern
    /// - The path does not match any `allow_write` pattern (default-deny)
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::{Path, PathBuf};
    /// use patina::tools::sandbox::{SandboxConfig, policy::SandboxPolicy};
    ///
    /// let config = SandboxConfig {
    ///     enabled: true,
    ///     allow_write: vec![PathBuf::from("/tmp/safe")],
    ///     deny_write: vec![PathBuf::from("/tmp/safe/secrets")],
    ///     ..SandboxConfig::default()
    /// };
    /// let policy = SandboxPolicy::new(config);
    /// assert!(policy.check_write(Path::new("/tmp/safe/output.txt")).is_ok());
    /// assert!(policy.check_write(Path::new("/tmp/safe/secrets/key")).is_err());
    /// ```
    pub fn check_write(&self, path: &Path) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }

        let normalized = normalize_path(path);

        if path_matches_any(&normalized, &self.config.deny_write) {
            bail!(
                "Sandbox: write access denied for {} (matched deny_write pattern)",
                normalized.display()
            );
        }

        if path_matches_any(&normalized, &self.config.allow_write) {
            return Ok(());
        }

        bail!(
            "Sandbox: write access denied for {} (not in allow_write list)",
            normalized.display()
        );
    }

    /// Validates that the given command is allowed for execution.
    ///
    /// When the sandbox is disabled, all commands are permitted. When enabled,
    /// the command is checked against deny and allow lists. Deny always wins.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The command matches a `deny_execute` pattern
    /// - The command does not match any `allow_execute` pattern (default-deny)
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::tools::sandbox::{SandboxConfig, policy::SandboxPolicy};
    ///
    /// let config = SandboxConfig {
    ///     enabled: true,
    ///     allow_execute: vec!["git *".to_string()],
    ///     deny_execute: vec!["git push --force *".to_string()],
    ///     ..SandboxConfig::default()
    /// };
    /// let policy = SandboxPolicy::new(config);
    /// assert!(policy.check_execute("git status").is_ok());
    /// assert!(policy.check_execute("git push --force origin main").is_err());
    /// ```
    pub fn check_execute(&self, command: &str) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }

        if command_matches_any(command, &self.config.deny_execute) {
            bail!(
                "Sandbox: execute access denied for '{}' (matched deny_execute pattern)",
                command
            );
        }

        if command_matches_any(command, &self.config.allow_execute) {
            return Ok(());
        }

        bail!(
            "Sandbox: execute access denied for '{}' (not in allow_execute list)",
            command
        );
    }
}

/// Normalizes a path by resolving `..` and `.` components lexically.
///
/// Unlike [`std::fs::canonicalize`], this does not touch the filesystem,
/// making it safe to use on paths that may not yet exist. This prevents
/// path traversal attacks like `../../../etc/passwd`.
fn normalize_path(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                components.pop();
            }
            std::path::Component::CurDir => {}
            other => components.push(other),
        }
    }
    components.iter().collect()
}

/// Checks whether a normalized path matches any entry in a path list.
///
/// A path matches an entry if:
/// - The entry is a glob pattern and it matches the path string
/// - The path starts with the entry (directory prefix match)
/// - The path equals the entry exactly
fn path_matches_any(path: &Path, patterns: &[PathBuf]) -> bool {
    let path_str = path.to_string_lossy();

    for pattern_path in patterns {
        let pattern_str = pattern_path.to_string_lossy();

        // Try glob matching first
        if let Ok(glob) = Pattern::new(&pattern_str) {
            if glob.matches(&path_str) {
                return true;
            }
        }

        // Directory prefix match: /home allows /home/user/file
        if path.starts_with(pattern_path) {
            return true;
        }
    }

    false
}

/// Checks whether a command matches any entry in a command pattern list.
///
/// Uses glob pattern matching against the full command string.
fn command_matches_any(command: &str, patterns: &[String]) -> bool {
    for pattern_str in patterns {
        if let Ok(glob) = Pattern::new(pattern_str) {
            if glob.matches(command) {
                return true;
            }
        }

        // Exact match fallback
        if command == pattern_str {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Disabled sandbox tests
    // =========================================================================

    #[test]
    fn test_disabled_sandbox_allows_all_reads() {
        let policy = SandboxPolicy::new(SandboxConfig::default());
        assert!(policy.check_read(Path::new("/etc/passwd")).is_ok());
        assert!(policy.check_read(Path::new("/root/.ssh/id_rsa")).is_ok());
        assert!(policy.check_read(Path::new("/any/path")).is_ok());
    }

    #[test]
    fn test_disabled_sandbox_allows_all_writes() {
        let policy = SandboxPolicy::new(SandboxConfig::default());
        assert!(policy.check_write(Path::new("/etc/shadow")).is_ok());
        assert!(policy.check_write(Path::new("/tmp/output")).is_ok());
    }

    #[test]
    fn test_disabled_sandbox_allows_all_execution() {
        let policy = SandboxPolicy::new(SandboxConfig::default());
        assert!(policy.check_execute("rm -rf /").is_ok());
        assert!(policy.check_execute("sudo reboot").is_ok());
    }

    #[test]
    fn test_disabled_sandbox_is_not_enabled() {
        let policy = SandboxPolicy::new(SandboxConfig::default());
        assert!(!policy.is_enabled());
    }

    // =========================================================================
    // Enabled sandbox with empty lists (default-deny)
    // =========================================================================

    #[test]
    fn test_enabled_empty_lists_denies_all_reads() {
        let config = SandboxConfig {
            enabled: true,
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        assert!(policy.check_read(Path::new("/any/path")).is_err());
        assert!(policy.check_read(Path::new("/home/user")).is_err());
    }

    #[test]
    fn test_enabled_empty_lists_denies_all_writes() {
        let config = SandboxConfig {
            enabled: true,
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        assert!(policy.check_write(Path::new("/tmp/file")).is_err());
    }

    #[test]
    fn test_enabled_empty_lists_denies_all_execution() {
        let config = SandboxConfig {
            enabled: true,
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        assert!(policy.check_execute("ls").is_err());
    }

    #[test]
    fn test_enabled_sandbox_is_enabled() {
        let config = SandboxConfig {
            enabled: true,
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);
        assert!(policy.is_enabled());
    }

    // =========================================================================
    // Allow read tests
    // =========================================================================

    #[test]
    fn test_allow_read_permits_listed_paths() {
        let config = SandboxConfig {
            enabled: true,
            allow_read: vec![PathBuf::from("/home/user/project")],
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        assert!(policy
            .check_read(Path::new("/home/user/project/src/main.rs"))
            .is_ok());
        assert!(policy.check_read(Path::new("/home/user/project")).is_ok());
    }

    #[test]
    fn test_allow_read_denies_unlisted_paths() {
        let config = SandboxConfig {
            enabled: true,
            allow_read: vec![PathBuf::from("/home/user/project")],
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        assert!(policy.check_read(Path::new("/etc/passwd")).is_err());
        assert!(policy.check_read(Path::new("/home/user/other")).is_err());
    }

    // =========================================================================
    // Deny read overrides allow read
    // =========================================================================

    #[test]
    fn test_deny_read_overrides_allow_read() {
        let config = SandboxConfig {
            enabled: true,
            allow_read: vec![PathBuf::from("/home/user")],
            deny_read: vec![PathBuf::from("/home/user/.ssh")],
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        // Allowed path works
        assert!(policy
            .check_read(Path::new("/home/user/project/file.rs"))
            .is_ok());

        // Denied path is blocked even though parent is allowed
        assert!(policy
            .check_read(Path::new("/home/user/.ssh/id_rsa"))
            .is_err());
    }

    // =========================================================================
    // Allow write tests
    // =========================================================================

    #[test]
    fn test_allow_write_permits_listed_paths() {
        let config = SandboxConfig {
            enabled: true,
            allow_write: vec![PathBuf::from("/tmp/output")],
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        assert!(policy
            .check_write(Path::new("/tmp/output/result.json"))
            .is_ok());
        assert!(policy.check_write(Path::new("/tmp/output")).is_ok());
    }

    #[test]
    fn test_allow_write_denies_unlisted_paths() {
        let config = SandboxConfig {
            enabled: true,
            allow_write: vec![PathBuf::from("/tmp/safe")],
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        assert!(policy.check_write(Path::new("/etc/passwd")).is_err());
        assert!(policy.check_write(Path::new("/tmp/unsafe")).is_err());
    }

    // =========================================================================
    // Deny write overrides allow write
    // =========================================================================

    #[test]
    fn test_deny_write_overrides_allow_write() {
        let config = SandboxConfig {
            enabled: true,
            allow_write: vec![PathBuf::from("/home/user/project")],
            deny_write: vec![PathBuf::from("/home/user/project/.env")],
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        assert!(policy
            .check_write(Path::new("/home/user/project/src/lib.rs"))
            .is_ok());
        assert!(policy
            .check_write(Path::new("/home/user/project/.env"))
            .is_err());
    }

    // =========================================================================
    // Glob pattern matching
    // =========================================================================

    #[test]
    fn test_glob_pattern_in_allow_read() {
        let config = SandboxConfig {
            enabled: true,
            allow_read: vec![PathBuf::from("/home/*/project")],
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        assert!(policy.check_read(Path::new("/home/alice/project")).is_ok());
        assert!(policy.check_read(Path::new("/home/bob/project")).is_ok());
    }

    #[test]
    fn test_glob_pattern_in_deny_read() {
        let config = SandboxConfig {
            enabled: true,
            allow_read: vec![PathBuf::from("/home/user")],
            deny_read: vec![PathBuf::from("/home/user/*.key")],
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        assert!(policy.check_read(Path::new("/home/user/readme.md")).is_ok());
        assert!(policy
            .check_read(Path::new("/home/user/secret.key"))
            .is_err());
    }

    #[test]
    fn test_glob_pattern_in_allow_write() {
        let config = SandboxConfig {
            enabled: true,
            allow_write: vec![PathBuf::from("/tmp/build-*")],
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        assert!(policy.check_write(Path::new("/tmp/build-123")).is_ok());
        assert!(policy.check_write(Path::new("/tmp/build-abc")).is_ok());
        assert!(policy.check_write(Path::new("/tmp/other")).is_err());
    }

    #[test]
    fn test_glob_pattern_in_execute() {
        let config = SandboxConfig {
            enabled: true,
            allow_execute: vec!["cargo *".to_string()],
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        assert!(policy.check_execute("cargo build").is_ok());
        assert!(policy.check_execute("cargo test --lib").is_ok());
        assert!(policy.check_execute("npm install").is_err());
    }

    // =========================================================================
    // Subdirectory access tests
    // =========================================================================

    #[test]
    fn test_allow_read_includes_subdirectories() {
        let config = SandboxConfig {
            enabled: true,
            allow_read: vec![PathBuf::from("/home")],
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        assert!(policy
            .check_read(Path::new("/home/user/deeply/nested/file.txt"))
            .is_ok());
    }

    #[test]
    fn test_allow_write_includes_subdirectories() {
        let config = SandboxConfig {
            enabled: true,
            allow_write: vec![PathBuf::from("/tmp")],
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        assert!(policy.check_write(Path::new("/tmp/a/b/c/d/file")).is_ok());
    }

    // =========================================================================
    // Path canonicalization / traversal tests
    // =========================================================================

    #[test]
    fn test_path_traversal_normalization() {
        let config = SandboxConfig {
            enabled: true,
            allow_read: vec![PathBuf::from("/home/user/project")],
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        // Path traversal attempt: /home/user/project/../../../etc/passwd
        // Normalizes to /etc/passwd which is not allowed
        assert!(policy
            .check_read(Path::new("/home/user/project/../../../etc/passwd"))
            .is_err());
    }

    #[test]
    fn test_path_traversal_within_allowed_dir() {
        let config = SandboxConfig {
            enabled: true,
            allow_read: vec![PathBuf::from("/home/user")],
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        // /home/user/project/../documents/file normalizes to /home/user/documents/file
        assert!(policy
            .check_read(Path::new("/home/user/project/../documents/file"))
            .is_ok());
    }

    #[test]
    fn test_dot_component_normalization() {
        let config = SandboxConfig {
            enabled: true,
            allow_read: vec![PathBuf::from("/home/user")],
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        // Current-dir components should be stripped
        assert!(policy
            .check_read(Path::new("/home/user/./project/./file.rs"))
            .is_ok());
    }

    // =========================================================================
    // Execute allow/deny tests
    // =========================================================================

    #[test]
    fn test_allow_execute_permits_listed_commands() {
        let config = SandboxConfig {
            enabled: true,
            allow_execute: vec!["git *".to_string(), "cargo *".to_string()],
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        assert!(policy.check_execute("git status").is_ok());
        assert!(policy.check_execute("cargo build").is_ok());
        assert!(policy.check_execute("rm -rf /").is_err());
    }

    #[test]
    fn test_deny_execute_overrides_allow_execute() {
        let config = SandboxConfig {
            enabled: true,
            allow_execute: vec!["git *".to_string()],
            deny_execute: vec!["git push --force *".to_string()],
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        assert!(policy.check_execute("git status").is_ok());
        assert!(policy.check_execute("git commit -m 'test'").is_ok());
        assert!(policy
            .check_execute("git push --force origin main")
            .is_err());
    }

    #[test]
    fn test_exact_command_match() {
        let config = SandboxConfig {
            enabled: true,
            allow_execute: vec!["ls".to_string()],
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        assert!(policy.check_execute("ls").is_ok());
        assert!(policy.check_execute("ls -la").is_err());
    }

    // =========================================================================
    // Error message tests
    // =========================================================================

    #[test]
    fn test_read_deny_error_message_mentions_deny_pattern() {
        let config = SandboxConfig {
            enabled: true,
            allow_read: vec![PathBuf::from("/home")],
            deny_read: vec![PathBuf::from("/home/user/.ssh")],
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        let err = policy
            .check_read(Path::new("/home/user/.ssh/id_rsa"))
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("deny_read"),
            "Error should mention deny_read: {msg}"
        );
        assert!(
            msg.contains("Sandbox"),
            "Error should mention Sandbox: {msg}"
        );
    }

    #[test]
    fn test_read_not_allowed_error_message() {
        let config = SandboxConfig {
            enabled: true,
            allow_read: vec![PathBuf::from("/home/user")],
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        let err = policy.check_read(Path::new("/etc/passwd")).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("not in allow_read list"),
            "Error should mention allow_read: {msg}"
        );
        assert!(
            msg.contains("/etc/passwd"),
            "Error should contain the path: {msg}"
        );
    }

    #[test]
    fn test_write_deny_error_message() {
        let config = SandboxConfig {
            enabled: true,
            allow_write: vec![PathBuf::from("/tmp")],
            deny_write: vec![PathBuf::from("/tmp/evil")],
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        let err = policy.check_write(Path::new("/tmp/evil")).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("deny_write"),
            "Error should mention deny_write: {msg}"
        );
    }

    #[test]
    fn test_write_not_allowed_error_message() {
        let config = SandboxConfig {
            enabled: true,
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        let err = policy.check_write(Path::new("/tmp/file")).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("not in allow_write list"),
            "Error should mention allow_write: {msg}"
        );
    }

    #[test]
    fn test_execute_deny_error_message() {
        let config = SandboxConfig {
            enabled: true,
            allow_execute: vec!["*".to_string()],
            deny_execute: vec!["rm *".to_string()],
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        let err = policy.check_execute("rm -rf /").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("deny_execute"),
            "Error should mention deny_execute: {msg}"
        );
    }

    #[test]
    fn test_execute_not_allowed_error_message() {
        let config = SandboxConfig {
            enabled: true,
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        let err = policy.check_execute("evil_command").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("not in allow_execute list"),
            "Error should mention allow_execute: {msg}"
        );
    }

    // =========================================================================
    // normalize_path unit tests
    // =========================================================================

    #[test]
    fn test_normalize_path_resolves_parent_dir() {
        let result = normalize_path(Path::new("/a/b/../c"));
        assert_eq!(result, PathBuf::from("/a/c"));
    }

    #[test]
    fn test_normalize_path_resolves_current_dir() {
        let result = normalize_path(Path::new("/a/./b/./c"));
        assert_eq!(result, PathBuf::from("/a/b/c"));
    }

    #[test]
    fn test_normalize_path_multiple_parent_dirs() {
        let result = normalize_path(Path::new("/a/b/c/../../d"));
        assert_eq!(result, PathBuf::from("/a/d"));
    }

    #[test]
    fn test_normalize_path_no_special_components() {
        let result = normalize_path(Path::new("/a/b/c"));
        assert_eq!(result, PathBuf::from("/a/b/c"));
    }

    // =========================================================================
    // Edge case tests
    // =========================================================================

    #[test]
    fn test_multiple_allow_read_paths() {
        let config = SandboxConfig {
            enabled: true,
            allow_read: vec![
                PathBuf::from("/home/user/project"),
                PathBuf::from("/usr/share"),
                PathBuf::from("/tmp"),
            ],
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        assert!(policy
            .check_read(Path::new("/home/user/project/file"))
            .is_ok());
        assert!(policy
            .check_read(Path::new("/usr/share/doc/readme"))
            .is_ok());
        assert!(policy.check_read(Path::new("/tmp/scratch")).is_ok());
        assert!(policy.check_read(Path::new("/var/log/syslog")).is_err());
    }

    #[test]
    fn test_deny_exact_file_in_allowed_directory() {
        let config = SandboxConfig {
            enabled: true,
            allow_read: vec![PathBuf::from("/home/user/project")],
            deny_read: vec![PathBuf::from("/home/user/project/secrets.json")],
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        assert!(policy
            .check_read(Path::new("/home/user/project/src/main.rs"))
            .is_ok());
        assert!(policy
            .check_read(Path::new("/home/user/project/secrets.json"))
            .is_err());
    }

    #[test]
    fn test_wildcard_allow_all_execute() {
        let config = SandboxConfig {
            enabled: true,
            allow_execute: vec!["*".to_string()],
            deny_execute: vec!["sudo *".to_string()],
            ..SandboxConfig::default()
        };
        let policy = SandboxPolicy::new(config);

        assert!(policy.check_execute("ls -la").is_ok());
        assert!(policy.check_execute("cargo build").is_ok());
        assert!(policy.check_execute("sudo rm -rf /").is_err());
    }
}
