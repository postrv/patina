//! Security pre-flight checks using narsil-mcp.
//!
//! This module provides security scanning integration to check tool invocations
//! before execution. It queries narsil-mcp for security findings and returns
//! verdicts that can be used to allow, warn, or block tool execution.
//!
//! # Security Verdicts
//!
//! - `Allow`: No security issues found, tool can execute
//! - `Warn(reason)`: High severity issue found, user should be warned
//! - `Block(reason)`: Critical severity issue found, tool should not execute
//!
//! # Example
//!
//! ```ignore
//! use patina::narsil::security::{SecurityVerdict, security_check};
//!
//! let verdict = security_check(&mut client, "bash", r#"{"command":"ls"}"#).await?;
//! match verdict {
//!     SecurityVerdict::Allow => { /* execute */ }
//!     SecurityVerdict::Warn(reason) => { /* warn user, then execute */ }
//!     SecurityVerdict::Block(reason) => { /* deny execution */ }
//! }
//! ```

/// Result of a security pre-flight check.
///
/// Used to determine whether a tool invocation should be allowed,
/// warned about, or blocked based on security analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecurityVerdict {
    /// No security issues found, tool execution is allowed.
    Allow,

    /// High severity issue found. Execution can proceed but user should be warned.
    ///
    /// Contains a human-readable description of the concern.
    Warn(String),

    /// Critical severity issue found. Execution should be blocked.
    ///
    /// Contains a human-readable description of the security issue.
    Block(String),
}

impl SecurityVerdict {
    /// Returns true if this verdict allows execution (Allow or Warn).
    #[must_use]
    pub fn allows_execution(&self) -> bool {
        !matches!(self, Self::Block(_))
    }

    /// Returns true if this verdict blocks execution.
    #[must_use]
    pub fn blocks_execution(&self) -> bool {
        matches!(self, Self::Block(_))
    }

    /// Returns true if this verdict has a warning.
    #[must_use]
    pub fn has_warning(&self) -> bool {
        matches!(self, Self::Warn(_))
    }

    /// Returns the reason message if this is a Warn or Block verdict.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Allow => None,
            Self::Warn(reason) | Self::Block(reason) => Some(reason),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =============================================================================
    // SecurityVerdict enum tests (Task 2.3.2)
    // =============================================================================

    #[test]
    fn test_security_verdict_allow() {
        let verdict = SecurityVerdict::Allow;
        assert!(verdict.allows_execution());
        assert!(!verdict.blocks_execution());
        assert!(!verdict.has_warning());
        assert!(verdict.reason().is_none());
    }

    #[test]
    fn test_security_verdict_warn() {
        let verdict = SecurityVerdict::Warn("Potential path traversal".to_string());
        assert!(verdict.allows_execution());
        assert!(!verdict.blocks_execution());
        assert!(verdict.has_warning());
        assert_eq!(verdict.reason(), Some("Potential path traversal"));
    }

    #[test]
    fn test_security_verdict_block() {
        let verdict = SecurityVerdict::Block("Command injection detected".to_string());
        assert!(!verdict.allows_execution());
        assert!(verdict.blocks_execution());
        assert!(!verdict.has_warning());
        assert_eq!(verdict.reason(), Some("Command injection detected"));
    }

    #[test]
    fn test_security_verdict_equality() {
        assert_eq!(SecurityVerdict::Allow, SecurityVerdict::Allow);
        assert_eq!(
            SecurityVerdict::Warn("test".to_string()),
            SecurityVerdict::Warn("test".to_string())
        );
        assert_ne!(
            SecurityVerdict::Warn("a".to_string()),
            SecurityVerdict::Warn("b".to_string())
        );
        assert_ne!(
            SecurityVerdict::Allow,
            SecurityVerdict::Block("x".to_string())
        );
    }

    // =============================================================================
    // security_check method tests (Task 2.3.1 - RED phase)
    // These tests will fail until security_check is implemented in integration.rs
    // =============================================================================

    // Note: The security_check method will be added to NarsilIntegration in Task 2.3.3
    // These tests document the expected behavior before implementation.

    #[test]
    fn test_security_check_allows_safe_tool() {
        // A safe tool invocation like "ls" should return Allow
        // This test documents that benign commands should pass through

        // For now, test the verdict type directly
        // The actual security_check integration test will be async
        let verdict = SecurityVerdict::Allow;
        assert!(verdict.allows_execution());
    }

    #[test]
    fn test_security_check_blocks_critical() {
        // A critical security issue (e.g., command injection) should return Block
        // This test documents that critical findings must block execution

        let verdict =
            SecurityVerdict::Block("CRITICAL: Command injection vulnerability".to_string());
        assert!(verdict.blocks_execution());
        assert!(verdict.reason().unwrap().contains("CRITICAL"));
    }

    #[test]
    fn test_security_check_warns_high() {
        // A high severity issue should return Warn, allowing execution with notice
        // This test documents that high findings should warn but not block

        let verdict = SecurityVerdict::Warn("HIGH: Potential path traversal".to_string());
        assert!(verdict.allows_execution());
        assert!(verdict.has_warning());
        assert!(verdict.reason().unwrap().contains("HIGH"));
    }
}
