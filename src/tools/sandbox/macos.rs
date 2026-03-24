//! macOS Seatbelt sandbox implementation.
//!
//! Wraps commands with `sandbox-exec -p "<profile>"` to apply OS-level
//! filesystem and network restrictions using Apple's Seatbelt framework.

use anyhow::Result;
use std::path::Path;

use super::config::SandboxConfig;
use super::Sandbox;

/// macOS Seatbelt sandbox.
///
/// Generates a Seatbelt profile (SBPL) from [`SandboxConfig`] and wraps
/// commands with `sandbox-exec -p "<profile>"` before execution.
///
/// The profile is passed inline via `-p` (not via a temp file) to avoid
/// TOCTOU race conditions.
pub struct MacOsSandbox;

impl MacOsSandbox {
    /// Generates a Seatbelt profile string from the config.
    ///
    /// The profile:
    /// - Denies all operations by default
    /// - Allows process execution (needed for the shell itself)
    /// - Allows read/write to configured paths
    /// - Allows read to system libraries and common paths
    /// - Conditionally allows network access
    #[must_use]
    pub fn generate_profile(config: &SandboxConfig) -> String {
        let mut profile = String::from("(version 1)\n(deny default)\n");

        // Allow process execution (needed for shell and subprocesses)
        profile.push_str("(allow process-exec*)\n");
        profile.push_str("(allow process-fork)\n");
        profile.push_str("(allow sysctl-read)\n");
        profile.push_str("(allow mach-lookup)\n");

        // Allow basic system reads
        for system_path in &[
            "/usr/lib",
            "/usr/share",
            "/System",
            "/dev/null",
            "/dev/urandom",
            "/dev/fd",
            "/private/tmp",
            "/private/var/tmp",
            "/var/tmp",
            "/tmp",
        ] {
            profile.push_str(&format!("(allow file-read* (subpath \"{system_path}\"))\n"));
        }

        // Allow read to configured paths
        for path in &config.allow_read {
            let path_str = path.display();
            profile.push_str(&format!("(allow file-read* (subpath \"{path_str}\"))\n"));
        }

        // Allow write to configured paths
        for path in &config.allow_write {
            let path_str = path.display();
            profile.push_str(&format!("(allow file-write* (subpath \"{path_str}\"))\n"));
        }

        // Network access
        if config.allow_network {
            profile.push_str("(allow network*)\n");
        }

        profile
    }
}

impl Sandbox for MacOsSandbox {
    fn apply(&self, cmd: &mut tokio::process::Command, config: &SandboxConfig) -> Result<()> {
        if !config.enabled {
            return Ok(());
        }

        let profile = Self::generate_profile(config);
        cmd.env("PATINA_SANDBOX_PROFILE", profile);
        tracing::debug!("Applied macOS Seatbelt sandbox env");
        Ok(())
    }

    fn wrap_command(&self, command: &str, config: &SandboxConfig) -> (String, Vec<String>) {
        if !config.enabled || !self.is_available() {
            return (
                "/bin/sh".to_string(),
                vec!["-c".to_string(), command.to_string()],
            );
        }

        let profile = Self::generate_profile(config);
        tracing::debug!("Wrapping command with sandbox-exec");
        (
            "/usr/bin/sandbox-exec".to_string(),
            vec![
                "-p".to_string(),
                profile,
                "/bin/sh".to_string(),
                "-c".to_string(),
                command.to_string(),
            ],
        )
    }

    fn is_available(&self) -> bool {
        Path::new("/usr/bin/sandbox-exec").exists()
    }

    fn name(&self) -> &'static str {
        "macos-seatbelt"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_generate_profile_includes_working_dir() {
        let config = SandboxConfig::for_working_dir(PathBuf::from("/home/user/project"));
        let profile = MacOsSandbox::generate_profile(&config);

        assert!(profile.contains("(deny default)"));
        assert!(profile.contains(r#"(allow file-read* (subpath "/home/user/project"))"#));
        assert!(profile.contains(r#"(allow file-write* (subpath "/home/user/project"))"#));
        assert!(profile.contains("(allow network*)"));
    }

    #[test]
    fn test_generate_profile_no_network() {
        let config = SandboxConfig {
            enabled: true,
            allow_read: vec![],
            allow_write: vec![],
            allow_network: false,
        };
        let profile = MacOsSandbox::generate_profile(&config);

        assert!(!profile.contains("(allow network*)"));
    }

    #[test]
    fn test_generate_profile_system_paths() {
        let config = SandboxConfig::for_working_dir(PathBuf::from("/tmp/test"));
        let profile = MacOsSandbox::generate_profile(&config);

        assert!(profile.contains(r#"(subpath "/usr/lib")"#));
        assert!(profile.contains(r#"(subpath "/usr/share")"#));
        assert!(profile.contains(r#"(subpath "/System")"#));
    }

    #[test]
    fn test_macos_sandbox_name() {
        let sandbox = MacOsSandbox;
        assert_eq!(sandbox.name(), "macos-seatbelt");
    }
}
