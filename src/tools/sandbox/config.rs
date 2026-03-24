//! Sandbox configuration types.
//!
//! Defines the [`SandboxConfig`] struct controlling which filesystem paths
//! and network access are permitted inside the OS-level sandbox.

use std::path::PathBuf;

/// Configuration for OS-level command sandboxing.
///
/// Controls which filesystem paths and capabilities are available
/// to commands executed inside the sandbox.
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
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    /// Whether sandboxing is enabled.
    pub enabled: bool,
    /// Paths allowed for read access.
    pub allow_read: Vec<PathBuf>,
    /// Paths allowed for write access.
    pub allow_write: Vec<PathBuf>,
    /// Whether to allow network access.
    pub allow_network: bool,
}

impl SandboxConfig {
    /// Creates a sandbox config for the given working directory.
    ///
    /// Allows read and write access to the working directory, plus
    /// read access to common system paths. Network is enabled by default.
    #[must_use]
    pub fn for_working_dir(working_dir: PathBuf) -> Self {
        Self {
            enabled: true,
            allow_read: vec![working_dir.clone()],
            allow_write: vec![working_dir],
            allow_network: true,
        }
    }

    /// Creates a disabled sandbox config (no-op).
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            allow_read: Vec::new(),
            allow_write: Vec::new(),
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
    }

    #[test]
    fn test_sandbox_config_for_working_dir() {
        let config = SandboxConfig::for_working_dir(PathBuf::from("/tmp/project"));
        assert!(config.enabled);
        assert_eq!(config.allow_read.len(), 1);
        assert_eq!(config.allow_write.len(), 1);
        assert!(config.allow_network);
    }

    #[test]
    fn test_sandbox_config_disabled() {
        let config = SandboxConfig::disabled();
        assert!(!config.enabled);
    }
}
