//! Centralized error types for RCT.
//!
//! This module provides a unified error type (`RctError`) that encompasses
//! all error conditions across the application. Using a single error type
//! provides:
//!
//! - Consistent error handling patterns
//! - Clear error categorization (security, retryable, etc.)
//! - Easy integration with `anyhow` for context
//!
//! # Example
//!
//! ```
//! use rct::error::{RctError, RctResult};
//!
//! fn validate_path(path: &str) -> RctResult<()> {
//!     if path.contains("..") {
//!         return Err(RctError::tool_path_traversal(path));
//!     }
//!     Ok(())
//! }
//!
//! fn main() {
//!     match validate_path("../etc/passwd") {
//!         Ok(()) => println!("Path is valid"),
//!         Err(e) => {
//!             println!("Error: {}", e);
//!             if e.is_security_related() {
//!                 println!("This is a security issue!");
//!             }
//!         }
//!     }
//! }
//! ```

use std::fmt;

/// Result type alias using `RctError`.
pub type RctResult<T> = Result<T, RctError>;

/// Centralized error type for RCT.
///
/// This enum provides variants for all error categories across modules,
/// enabling consistent error handling and categorization.
#[derive(Debug)]
pub enum RctError {
    // ============== Tool Errors ==============
    /// Path traversal attempt detected.
    ToolPathTraversal {
        /// The path that was attempted.
        path: String,
    },

    /// Permission denied when accessing a resource.
    ToolPermissionDenied {
        /// The path that access was denied for.
        path: String,
    },

    /// Command execution timed out.
    ToolTimeout {
        /// The command that timed out.
        command: String,
        /// Timeout in milliseconds.
        timeout_ms: u64,
    },

    /// Security policy violation.
    ToolSecurityViolation {
        /// The command that violated the policy.
        command: String,
        /// Description of the violation.
        reason: String,
    },

    // ============== API Errors ==============
    /// Network error during API communication.
    ApiNetwork {
        /// Description of the network error.
        message: String,
    },

    /// Rate limited by the API.
    ApiRateLimited {
        /// Retry after this many seconds.
        retry_after_secs: u64,
    },

    /// Authentication failure.
    ApiAuthentication {
        /// Description of the authentication error.
        message: String,
    },

    /// Invalid response from the API.
    ApiInvalidResponse {
        /// Description of the response issue.
        message: String,
    },

    // ============== MCP Errors ==============
    /// MCP transport error.
    McpTransport {
        /// Description of the transport error.
        message: String,
    },

    /// MCP command validation error.
    McpValidation {
        /// Description of the validation error.
        message: String,
    },

    /// MCP protocol error.
    McpProtocol {
        /// Description of the protocol error.
        message: String,
    },

    // ============== Session Errors ==============
    /// Session integrity check failure.
    SessionIntegrity {
        /// Description of the integrity error.
        message: String,
    },

    /// Session I/O error.
    SessionIo {
        /// Description of the I/O error.
        message: String,
    },

    /// Session validation error.
    SessionValidation {
        /// Description of the validation error.
        message: String,
    },

    // ============== Hook Errors ==============
    /// Hook validation error.
    HookValidation {
        /// Description of the validation error.
        message: String,
    },

    /// Hook execution error.
    HookExecution {
        /// Description of the execution error.
        message: String,
    },

    // ============== Plugin Errors ==============
    /// Plugin loading error.
    PluginLoad {
        /// The plugin name.
        plugin: String,
        /// Description of the load error.
        message: String,
    },

    /// Plugin execution error.
    PluginExecution {
        /// The plugin name.
        plugin: String,
        /// Description of the execution error.
        message: String,
    },

    // ============== Context Errors ==============
    /// Context I/O error.
    ContextIo {
        /// The path being accessed.
        path: String,
        /// Description of the I/O error.
        message: String,
    },

    // ============== Wrapped Errors ==============
    /// Error from anyhow or other sources.
    Other {
        /// The wrapped error message.
        message: String,
        /// The original error, if available.
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
}

// ============== Constructor Methods ==============

impl RctError {
    // Tool error constructors

    /// Creates a path traversal error.
    #[must_use]
    pub fn tool_path_traversal(path: impl Into<String>) -> Self {
        Self::ToolPathTraversal { path: path.into() }
    }

    /// Creates a permission denied error.
    #[must_use]
    pub fn tool_permission_denied(path: impl Into<String>) -> Self {
        Self::ToolPermissionDenied { path: path.into() }
    }

    /// Creates a timeout error.
    #[must_use]
    pub fn tool_timeout(command: impl Into<String>, timeout_ms: u64) -> Self {
        Self::ToolTimeout {
            command: command.into(),
            timeout_ms,
        }
    }

    /// Creates a security violation error.
    #[must_use]
    pub fn tool_security_violation(command: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::ToolSecurityViolation {
            command: command.into(),
            reason: reason.into(),
        }
    }

    // API error constructors

    /// Creates a network error.
    #[must_use]
    pub fn api_network(message: impl Into<String>) -> Self {
        Self::ApiNetwork {
            message: message.into(),
        }
    }

    /// Creates a rate limited error.
    #[must_use]
    pub fn api_rate_limited(retry_after_secs: u64) -> Self {
        Self::ApiRateLimited { retry_after_secs }
    }

    /// Creates an authentication error.
    #[must_use]
    pub fn api_authentication(message: impl Into<String>) -> Self {
        Self::ApiAuthentication {
            message: message.into(),
        }
    }

    /// Creates an invalid response error.
    #[must_use]
    pub fn api_invalid_response(message: impl Into<String>) -> Self {
        Self::ApiInvalidResponse {
            message: message.into(),
        }
    }

    // MCP error constructors

    /// Creates a transport error.
    #[must_use]
    pub fn mcp_transport(message: impl Into<String>) -> Self {
        Self::McpTransport {
            message: message.into(),
        }
    }

    /// Creates a validation error.
    #[must_use]
    pub fn mcp_validation(message: impl Into<String>) -> Self {
        Self::McpValidation {
            message: message.into(),
        }
    }

    /// Creates a protocol error.
    #[must_use]
    pub fn mcp_protocol(message: impl Into<String>) -> Self {
        Self::McpProtocol {
            message: message.into(),
        }
    }

    // Session error constructors

    /// Creates a session integrity error.
    #[must_use]
    pub fn session_integrity(message: impl Into<String>) -> Self {
        Self::SessionIntegrity {
            message: message.into(),
        }
    }

    /// Creates a session I/O error.
    #[must_use]
    pub fn session_io(message: impl Into<String>) -> Self {
        Self::SessionIo {
            message: message.into(),
        }
    }

    /// Creates a session validation error.
    #[must_use]
    pub fn session_validation(message: impl Into<String>) -> Self {
        Self::SessionValidation {
            message: message.into(),
        }
    }

    // Hook error constructors

    /// Creates a hook validation error.
    #[must_use]
    pub fn hook_validation(message: impl Into<String>) -> Self {
        Self::HookValidation {
            message: message.into(),
        }
    }

    /// Creates a hook execution error.
    #[must_use]
    pub fn hook_execution(message: impl Into<String>) -> Self {
        Self::HookExecution {
            message: message.into(),
        }
    }

    // Plugin error constructors

    /// Creates a plugin load error.
    #[must_use]
    pub fn plugin_load(plugin: impl Into<String>, message: impl Into<String>) -> Self {
        Self::PluginLoad {
            plugin: plugin.into(),
            message: message.into(),
        }
    }

    /// Creates a plugin execution error.
    #[must_use]
    pub fn plugin_execution(plugin: impl Into<String>, message: impl Into<String>) -> Self {
        Self::PluginExecution {
            plugin: plugin.into(),
            message: message.into(),
        }
    }

    // Context error constructors

    /// Creates a context I/O error.
    #[must_use]
    pub fn context_io(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self::ContextIo {
            path: path.into(),
            message: message.into(),
        }
    }
}

// ============== Category Methods ==============

impl RctError {
    /// Returns `true` if this error is potentially retryable.
    ///
    /// Network errors, rate limits, and transport errors are typically retryable.
    /// Security violations and validation errors are not.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::ApiNetwork { .. }
                | Self::ApiRateLimited { .. }
                | Self::McpTransport { .. }
                | Self::ToolTimeout { .. }
        )
    }

    /// Returns `true` if this error is security-related.
    ///
    /// Security errors should be logged and may require additional handling.
    #[must_use]
    pub fn is_security_related(&self) -> bool {
        matches!(
            self,
            Self::ToolPathTraversal { .. }
                | Self::ToolSecurityViolation { .. }
                | Self::McpValidation { .. }
                | Self::HookValidation { .. }
                | Self::SessionIntegrity { .. }
        )
    }

    /// Returns the module name where this error originated.
    #[must_use]
    pub fn module(&self) -> &'static str {
        match self {
            Self::ToolPathTraversal { .. }
            | Self::ToolPermissionDenied { .. }
            | Self::ToolTimeout { .. }
            | Self::ToolSecurityViolation { .. } => "tools",

            Self::ApiNetwork { .. }
            | Self::ApiRateLimited { .. }
            | Self::ApiAuthentication { .. }
            | Self::ApiInvalidResponse { .. } => "api",

            Self::McpTransport { .. } | Self::McpValidation { .. } | Self::McpProtocol { .. } => {
                "mcp"
            }

            Self::SessionIntegrity { .. }
            | Self::SessionIo { .. }
            | Self::SessionValidation { .. } => "session",

            Self::HookValidation { .. } | Self::HookExecution { .. } => "hooks",

            Self::PluginLoad { .. } | Self::PluginExecution { .. } => "plugins",

            Self::ContextIo { .. } => "context",

            Self::Other { .. } => "unknown",
        }
    }
}

// ============== Display Implementation ==============

impl fmt::Display for RctError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Tool errors
            Self::ToolPathTraversal { path } => {
                write!(f, "tools: path traversal detected for '{}'", path)
            }
            Self::ToolPermissionDenied { path } => {
                write!(f, "tools: permission denied for '{}'", path)
            }
            Self::ToolTimeout {
                command,
                timeout_ms,
            } => {
                write!(
                    f,
                    "tools: command '{}' timed out after {} ms",
                    command, timeout_ms
                )
            }
            Self::ToolSecurityViolation { command, reason } => {
                write!(f, "tools: security violation for '{}': {}", command, reason)
            }

            // API errors
            Self::ApiNetwork { message } => {
                write!(f, "api: network error: {}", message)
            }
            Self::ApiRateLimited { retry_after_secs } => {
                write!(
                    f,
                    "api: rate limited, retry after {} seconds",
                    retry_after_secs
                )
            }
            Self::ApiAuthentication { message } => {
                write!(f, "api: authentication failed: {}", message)
            }
            Self::ApiInvalidResponse { message } => {
                write!(f, "api: invalid response: {}", message)
            }

            // MCP errors
            Self::McpTransport { message } => {
                write!(f, "mcp: transport error: {}", message)
            }
            Self::McpValidation { message } => {
                write!(f, "mcp: validation error: {}", message)
            }
            Self::McpProtocol { message } => {
                write!(f, "mcp: protocol error: {}", message)
            }

            // Session errors
            Self::SessionIntegrity { message } => {
                write!(f, "session: integrity check failed: {}", message)
            }
            Self::SessionIo { message } => {
                write!(f, "session: I/O error: {}", message)
            }
            Self::SessionValidation { message } => {
                write!(f, "session: validation error: {}", message)
            }

            // Hook errors
            Self::HookValidation { message } => {
                write!(f, "hooks: validation error: {}", message)
            }
            Self::HookExecution { message } => {
                write!(f, "hooks: execution error: {}", message)
            }

            // Plugin errors
            Self::PluginLoad { plugin, message } => {
                write!(f, "plugins: load error for '{}': {}", plugin, message)
            }
            Self::PluginExecution { plugin, message } => {
                write!(f, "plugins: execution error for '{}': {}", plugin, message)
            }

            // Context errors
            Self::ContextIo { path, message } => {
                write!(f, "context: I/O error for '{}': {}", path, message)
            }

            // Other errors
            Self::Other { message, .. } => {
                write!(f, "error: {}", message)
            }
        }
    }
}

// ============== Error Implementation ==============

impl std::error::Error for RctError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Other {
                source: Some(src), ..
            } => Some(src.as_ref()),
            _ => None,
        }
    }
}

// ============== Conversion Implementations ==============

impl From<anyhow::Error> for RctError {
    fn from(err: anyhow::Error) -> Self {
        // Convert the chain of errors to a descriptive string
        // Note: We store the message only since anyhow::Error doesn't implement std::error::Error
        Self::Other {
            message: format!("{:#}", err),
            source: None,
        }
    }
}

// Note: We don't implement `From<RctError> for anyhow::Error` because:
// 1. anyhow already has a blanket impl `From<E> for anyhow::Error where E: std::error::Error`
// 2. Since RctError implements std::error::Error, we can use `anyhow::Error::from(rct_err)` or `rct_err.into()`

// ============== Unit Tests ==============

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_error_display() {
        let err = RctError::tool_path_traversal("/etc/passwd");
        assert!(err.to_string().contains("path traversal"));
        assert!(err.to_string().contains("/etc/passwd"));
    }

    #[test]
    fn test_is_retryable() {
        assert!(RctError::api_network("timeout").is_retryable());
        assert!(RctError::api_rate_limited(60).is_retryable());
        assert!(!RctError::tool_security_violation("rm", "dangerous").is_retryable());
    }

    #[test]
    fn test_is_security_related() {
        assert!(RctError::tool_path_traversal("/etc").is_security_related());
        assert!(RctError::mcp_validation("blocked").is_security_related());
        assert!(!RctError::api_network("timeout").is_security_related());
    }

    #[test]
    fn test_module() {
        assert_eq!(RctError::tool_timeout("test", 1000).module(), "tools");
        assert_eq!(RctError::api_network("err").module(), "api");
        assert_eq!(RctError::mcp_transport("err").module(), "mcp");
    }

    #[test]
    fn test_from_anyhow() {
        let anyhow_err = anyhow::anyhow!("test error");
        let rct_err: RctError = anyhow_err.into();
        assert!(rct_err.to_string().contains("test error"));
    }
}
