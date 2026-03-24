//! No-op sandbox implementation.
//!
//! Used when sandboxing is disabled or unavailable on the current platform.

use anyhow::Result;

use super::config::SandboxConfig;
use super::Sandbox;

/// A no-op sandbox that applies no restrictions.
///
/// Used as a fallback when:
/// - Sandboxing is explicitly disabled (`--no-sandbox`)
/// - The platform does not support sandboxing
/// - The sandbox implementation is unavailable
pub struct NoopSandbox;

impl Sandbox for NoopSandbox {
    fn apply(&self, _cmd: &mut tokio::process::Command, _config: &SandboxConfig) -> Result<()> {
        Ok(())
    }

    fn wrap_command(&self, command: &str, _config: &SandboxConfig) -> (String, Vec<String>) {
        (
            "/bin/sh".to_string(),
            vec!["-c".to_string(), command.to_string()],
        )
    }

    fn is_available(&self) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "noop"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_noop_sandbox_is_available() {
        let sandbox = NoopSandbox;
        assert!(sandbox.is_available());
        assert_eq!(sandbox.name(), "noop");
    }
}
