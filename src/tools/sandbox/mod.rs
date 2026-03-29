//! OS-level sandboxing for command execution.
//!
//! Provides kernel-enforced filesystem and network restrictions for
//! bash commands, adding defense-in-depth beyond the application-level
//! command filtering in [`security`](super::security).
//!
//! # Platform Support
//!
//! - **macOS**: Seatbelt (`sandbox-exec`) profiles
//! - **Linux**: Landlock LSM (kernel 5.13+)
//! - **Other**: No-op fallback with warning
//!
//! # Usage
//!
//! ```rust,ignore
//! use patina::tools::sandbox::{create_sandbox, SandboxConfig};
//!
//! let config = SandboxConfig::for_working_dir(working_dir);
//! let sandbox = create_sandbox();
//! sandbox.apply(&mut cmd, &config)?;
//! ```

pub mod config;
pub mod linux;
pub mod macos;
pub mod noop;
pub mod policy;

pub use config::SandboxConfig;
pub use policy::SandboxPolicy;

use anyhow::Result;

/// Trait for OS-level sandbox implementations.
///
/// Implementors apply kernel-enforced restrictions to a [`Command`]
/// before it is spawned. This provides defense-in-depth beyond
/// the application-level command filtering.
pub trait Sandbox: Send + Sync {
    /// Applies sandbox constraints to a Command before spawning.
    ///
    /// # Errors
    ///
    /// Returns an error if the sandbox cannot be applied.
    fn apply(&self, cmd: &mut tokio::process::Command, config: &SandboxConfig) -> Result<()>;

    /// Wraps a shell command string with sandbox enforcement.
    ///
    /// On macOS, this prefixes with `sandbox-exec -p "<profile>"`.
    /// On other platforms, returns the command unchanged.
    ///
    /// # Returns
    ///
    /// A tuple of `(program, args)` — the new command to execute.
    fn wrap_command(&self, command: &str, config: &SandboxConfig) -> (String, Vec<String>);

    /// Returns whether this sandbox implementation is available on the current system.
    #[must_use]
    fn is_available(&self) -> bool;

    /// Returns a human-readable name for this sandbox.
    #[must_use]
    fn name(&self) -> &'static str;
}

/// Creates the appropriate sandbox for the current platform.
///
/// Returns:
/// - macOS: [`MacOsSandbox`](macos::MacOsSandbox) if `sandbox-exec` is available
/// - Linux: [`LinuxSandbox`](linux::LinuxSandbox) if Landlock is available
/// - Other: [`NoopSandbox`](noop::NoopSandbox) with no restrictions
///
/// # Examples
///
/// ```rust,ignore
/// let sandbox = patina::tools::sandbox::create_sandbox();
/// println!("Using sandbox: {}", sandbox.name());
/// ```
#[must_use]
pub fn create_sandbox() -> Box<dyn Sandbox> {
    #[cfg(target_os = "macos")]
    {
        let sandbox = macos::MacOsSandbox;
        if sandbox.is_available() {
            tracing::info!("Using macOS Seatbelt sandbox");
            return Box::new(sandbox);
        }
    }

    #[cfg(target_os = "linux")]
    {
        let sandbox = linux::LinuxSandbox;
        if sandbox.is_available() {
            tracing::info!("Using Linux Landlock sandbox");
            return Box::new(sandbox);
        }
    }

    tracing::debug!("No OS-level sandbox available, using no-op fallback");
    Box::new(noop::NoopSandbox)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_sandbox_returns_platform_impl() {
        let sandbox = create_sandbox();
        // On any platform, we should get a valid sandbox
        assert!(!sandbox.name().is_empty());
    }

    #[test]
    fn test_noop_sandbox_apply_succeeds() {
        let sandbox = noop::NoopSandbox;
        let config = SandboxConfig::disabled();
        let mut cmd = tokio::process::Command::new("echo");
        assert!(sandbox.apply(&mut cmd, &config).is_ok());
    }

    #[test]
    fn test_sandbox_config_for_working_dir_is_enabled() {
        let config = SandboxConfig::for_working_dir(std::path::PathBuf::from("/tmp"));
        assert!(config.enabled);
    }
}
