//! Sandbox configuration types.
//!
//! Defines the [`SandboxConfig`] struct controlling which filesystem paths,
//! commands, and network access are permitted inside the sandbox.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Configuration for sandbox enforcement.
///
/// Controls which filesystem paths may be read/written, which commands
/// may be executed, and whether network access is permitted. Used by both
/// OS-level sandboxing (Seatbelt, Landlock) and application-level policy
/// enforcement ([`SandboxPolicy`](super::policy::SandboxPolicy)).
///
/// Deny lists always take priority over allow lists. When enabled with
/// empty allow lists, all operations are denied by default.
///
/// # Examples
///
/// ```rust
/// use patina::tools::sandbox::config::SandboxConfig;
/// use std::path::PathBuf;
///
/// let config = SandboxConfig::for_working_dir(PathBuf::from("/home/user/project"));
/// assert!(config.enabled);
/// assert!(config.allow_read.contains(&PathBuf::from("/home/user/project")));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    /// Whether sandboxing is enabled.
    pub enabled: bool,
    /// Paths (or glob patterns) allowed for read access.
    pub allow_read: Vec<PathBuf>,
    /// Paths (or glob patterns) denied for reading. Overrides `allow_read`.
    #[serde(default)]
    pub deny_read: Vec<PathBuf>,
    /// Paths (or glob patterns) allowed for write access.
    pub allow_write: Vec<PathBuf>,
    /// Paths (or glob patterns) denied for writing. Overrides `allow_write`.
    #[serde(default)]
    pub deny_write: Vec<PathBuf>,
    /// Command patterns allowed for execution (supports glob syntax).
    #[serde(default)]
    pub allow_execute: Vec<String>,
    /// Command patterns denied for execution. Overrides `allow_execute`.
    #[serde(default)]
    pub deny_execute: Vec<String>,
    /// Whether to allow network access.
    pub allow_network: bool,
}

impl SandboxConfig {
    /// Creates a sandbox config for the given working directory.
    ///
    /// Allows read and write access to the working directory, plus
    /// read access to common system paths. Network is enabled by default.
    /// All command execution is allowed.
    #[must_use]
    pub fn for_working_dir(working_dir: PathBuf) -> Self {
        Self {
            enabled: true,
            allow_read: vec![working_dir.clone()],
            deny_read: Vec::new(),
            allow_write: vec![working_dir],
            deny_write: Vec::new(),
            allow_execute: vec!["*".to_string()],
            deny_execute: Vec::new(),
            allow_network: true,
        }
    }

    /// Creates a disabled sandbox config (no-op).
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            allow_read: Vec::new(),
            deny_read: Vec::new(),
            allow_write: Vec::new(),
            deny_write: Vec::new(),
            allow_execute: Vec::new(),
            deny_execute: Vec::new(),
            allow_network: true,
        }
    }
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self::disabled()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sandbox_config_default_is_disabled() {
        let config = SandboxConfig::default();
        assert!(!config.enabled);
        assert!(config.allow_read.is_empty());
        assert!(config.deny_read.is_empty());
        assert!(config.allow_write.is_empty());
        assert!(config.deny_write.is_empty());
        assert!(config.allow_execute.is_empty());
        assert!(config.deny_execute.is_empty());
    }

    #[test]
    fn test_sandbox_config_for_working_dir() {
        let config = SandboxConfig::for_working_dir(PathBuf::from("/tmp/project"));
        assert!(config.enabled);
        assert_eq!(config.allow_read.len(), 1);
        assert_eq!(config.allow_write.len(), 1);
        assert!(config.deny_read.is_empty());
        assert!(config.deny_write.is_empty());
        assert_eq!(config.allow_execute, vec!["*".to_string()]);
        assert!(config.deny_execute.is_empty());
        assert!(config.allow_network);
    }

    #[test]
    fn test_sandbox_config_disabled() {
        let config = SandboxConfig::disabled();
        assert!(!config.enabled);
    }

    #[test]
    fn test_sandbox_config_serialization_roundtrip() {
        let config = SandboxConfig {
            enabled: true,
            allow_read: vec![PathBuf::from("/home/user")],
            deny_read: vec![PathBuf::from("/home/user/.ssh")],
            allow_write: vec![PathBuf::from("/tmp")],
            deny_write: vec![PathBuf::from("/tmp/protected")],
            allow_execute: vec!["git *".to_string()],
            deny_execute: vec!["rm *".to_string()],
            allow_network: false,
        };

        let serialized = serde_json::to_string(&config).expect("serialize");
        let deserialized: SandboxConfig = serde_json::from_str(&serialized).expect("deserialize");

        assert!(deserialized.enabled);
        assert_eq!(deserialized.allow_read.len(), 1);
        assert_eq!(deserialized.deny_read.len(), 1);
        assert_eq!(deserialized.allow_write.len(), 1);
        assert_eq!(deserialized.deny_write.len(), 1);
        assert_eq!(deserialized.allow_execute.len(), 1);
        assert_eq!(deserialized.deny_execute.len(), 1);
        assert!(!deserialized.allow_network);
    }

    #[test]
    fn test_sandbox_config_deserialize_with_defaults() {
        // Minimal JSON with only required fields - new fields default via #[serde(default)]
        let json = r#"{"enabled":true,"allow_read":[],"allow_write":[],"allow_network":true}"#;
        let config: SandboxConfig = serde_json::from_str(json).expect("deserialize");

        assert!(config.enabled);
        assert!(config.deny_read.is_empty());
        assert!(config.deny_write.is_empty());
        assert!(config.allow_execute.is_empty());
        assert!(config.deny_execute.is_empty());
    }
}
