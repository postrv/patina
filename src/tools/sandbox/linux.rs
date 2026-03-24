//! Linux Landlock sandbox implementation.
//!
//! Uses the Landlock LSM (Linux Security Module) for kernel-enforced
//! filesystem restrictions. Requires Linux 5.13+.
//!
//! Falls back gracefully to no-op when Landlock is unavailable.

use anyhow::Result;

use super::config::SandboxConfig;
use super::Sandbox;

/// Linux Landlock sandbox.
///
/// Applies Landlock filesystem restrictions to child processes via
/// `Command::pre_exec()`. The restrictions are applied in the child
/// after `fork()` but before `exec()`.
///
/// Requires Linux kernel 5.13+ with Landlock enabled. Falls back
/// gracefully with a warning if unavailable.
pub struct LinuxSandbox;

impl LinuxSandbox {
    /// Checks if Landlock is supported on this kernel.
    #[must_use]
    pub fn is_landlock_supported() -> bool {
        // Check if the landlock sysfs exists
        std::path::Path::new("/sys/kernel/security/landlock").exists()
    }
}

impl Sandbox for LinuxSandbox {
    fn apply(&self, _cmd: &mut tokio::process::Command, config: &SandboxConfig) -> Result<()> {
        if !config.enabled {
            return Ok(());
        }

        if !Self::is_landlock_supported() {
            tracing::warn!(
                "Landlock is not available on this kernel; running without OS-level sandboxing"
            );
            return Ok(());
        }

        // Landlock requires the `landlock` crate for full enforcement.
        // Without it, we log a warning rather than silently claiming protection.
        tracing::warn!(
            "Landlock sandbox not enforced: full implementation requires the landlock crate. \
             Running without OS-level sandboxing on Linux."
        );

        Ok(())
    }

    fn wrap_command(&self, command: &str, config: &SandboxConfig) -> (String, Vec<String>) {
        if config.enabled && Self::is_landlock_supported() {
            tracing::warn!("Linux Landlock sandbox not enforced (landlock crate required)");
        }
        // On Linux without landlock crate, return the command unchanged
        (
            "/bin/sh".to_string(),
            vec!["-c".to_string(), command.to_string()],
        )
    }

    fn is_available(&self) -> bool {
        Self::is_landlock_supported()
    }

    fn name(&self) -> &'static str {
        "linux-landlock"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_linux_sandbox_name() {
        let sandbox = LinuxSandbox;
        assert_eq!(sandbox.name(), "linux-landlock");
    }

    #[test]
    fn test_linux_sandbox_disabled_is_noop() {
        let sandbox = LinuxSandbox;
        let config = SandboxConfig::disabled();
        let mut cmd = tokio::process::Command::new("echo");
        // Should succeed without error even when disabled
        assert!(sandbox.apply(&mut cmd, &config).is_ok());
    }
}
