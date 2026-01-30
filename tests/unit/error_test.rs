//! Tests for RCT error types.
//!
//! These tests verify the behavior of the centralized error types module,
//! ensuring proper Display, Error trait implementations, and conversions.

use rct::error::{RctError, RctResult};

#[cfg(test)]
mod error_type_tests {
    use super::*;

    // ============== Construction Tests ==============

    #[test]
    fn test_tool_error_path_traversal() {
        let err = RctError::tool_path_traversal("/etc/passwd");
        assert!(err.to_string().contains("path traversal"));
        assert!(err.to_string().contains("/etc/passwd"));
    }

    #[test]
    fn test_tool_error_permission_denied() {
        let err = RctError::tool_permission_denied("/secret/file");
        assert!(err.to_string().contains("permission denied"));
        assert!(err.to_string().contains("/secret/file"));
    }

    #[test]
    fn test_tool_error_timeout() {
        let err = RctError::tool_timeout("bash", 30000);
        assert!(err.to_string().contains("timed out"));
        assert!(err.to_string().contains("30000"));
    }

    #[test]
    fn test_tool_error_security_violation() {
        let err = RctError::tool_security_violation("rm -rf /", "destructive command");
        assert!(err.to_string().contains("security violation"));
        assert!(err.to_string().contains("destructive command"));
    }

    #[test]
    fn test_api_error_network() {
        let err = RctError::api_network("connection refused");
        assert!(err.to_string().contains("network error"));
        assert!(err.to_string().contains("connection refused"));
    }

    #[test]
    fn test_api_error_rate_limited() {
        let err = RctError::api_rate_limited(60);
        assert!(err.to_string().contains("rate limited"));
        assert!(err.to_string().contains("60"));
    }

    #[test]
    fn test_api_error_authentication() {
        let err = RctError::api_authentication("invalid API key");
        assert!(err.to_string().contains("authentication"));
        assert!(err.to_string().contains("invalid API key"));
    }

    #[test]
    fn test_api_error_invalid_response() {
        let err = RctError::api_invalid_response("missing content field");
        assert!(err.to_string().contains("invalid response"));
        assert!(err.to_string().contains("missing content field"));
    }

    #[test]
    fn test_mcp_error_transport() {
        let err = RctError::mcp_transport("connection lost");
        assert!(err.to_string().contains("transport error"));
        assert!(err.to_string().contains("connection lost"));
    }

    #[test]
    fn test_mcp_error_validation() {
        let err = RctError::mcp_validation("invalid command: rm");
        assert!(err.to_string().contains("validation error"));
        assert!(err.to_string().contains("invalid command: rm"));
    }

    #[test]
    fn test_mcp_error_protocol() {
        let err = RctError::mcp_protocol("JSON-RPC error: method not found");
        assert!(err.to_string().contains("protocol error"));
        assert!(err.to_string().contains("method not found"));
    }

    #[test]
    fn test_session_error_integrity() {
        let err = RctError::session_integrity("checksum mismatch");
        assert!(err.to_string().contains("integrity"));
        assert!(err.to_string().contains("checksum mismatch"));
    }

    #[test]
    fn test_session_error_io() {
        let err = RctError::session_io("failed to read file");
        assert!(err.to_string().contains("I/O error"));
        assert!(err.to_string().contains("failed to read file"));
    }

    #[test]
    fn test_session_error_validation() {
        let err = RctError::session_validation("invalid session ID");
        assert!(err.to_string().contains("validation error"));
        assert!(err.to_string().contains("invalid session ID"));
    }

    #[test]
    fn test_hook_error_validation() {
        let err = RctError::hook_validation("dangerous command blocked");
        assert!(err.to_string().contains("validation error"));
        assert!(err.to_string().contains("dangerous command blocked"));
    }

    #[test]
    fn test_hook_error_execution() {
        let err = RctError::hook_execution("hook returned non-zero exit code");
        assert!(err.to_string().contains("execution error"));
        assert!(err.to_string().contains("non-zero exit code"));
    }

    #[test]
    fn test_plugin_error_load() {
        let err = RctError::plugin_load("my_plugin", "library not found");
        assert!(err.to_string().contains("load error"));
        assert!(err.to_string().contains("my_plugin"));
        assert!(err.to_string().contains("library not found"));
    }

    #[test]
    fn test_plugin_error_execution() {
        let err = RctError::plugin_execution("my_plugin", "panic in on_message");
        assert!(err.to_string().contains("execution error"));
        assert!(err.to_string().contains("my_plugin"));
        assert!(err.to_string().contains("panic in on_message"));
    }

    #[test]
    fn test_context_error_io() {
        let err = RctError::context_io("/project/.claude.md", "file not found");
        assert!(err.to_string().contains("I/O error"));
        assert!(err.to_string().contains("/project/.claude.md"));
    }

    // ============== Error Trait Tests ==============

    #[test]
    fn test_error_is_std_error() {
        let err = RctError::tool_timeout("bash", 30000);
        let std_err: &dyn std::error::Error = &err;
        assert!(!std_err.to_string().is_empty());
    }

    #[test]
    fn test_error_source_for_wrapped_error() {
        // When wrapping an anyhow error, the message is preserved
        let source_err = anyhow::anyhow!("original error");
        let err = RctError::from(source_err);
        // The error message should contain the original error text
        assert!(err.to_string().contains("original error"));
    }

    // ============== Display Tests ==============

    #[test]
    fn test_display_format_consistent() {
        // All errors should have a consistent format: "category: specific message"
        let errors = vec![
            RctError::tool_path_traversal("/etc/passwd"),
            RctError::api_network("timeout"),
            RctError::mcp_transport("disconnected"),
            RctError::session_integrity("corrupted"),
            RctError::hook_validation("blocked"),
            RctError::plugin_load("test", "missing"),
            RctError::context_io("/path", "error"),
        ];

        for err in errors {
            let msg = err.to_string();
            // Should contain a colon separating category from message
            assert!(
                msg.contains(':'),
                "Error message should contain colon: {}",
                msg
            );
        }
    }

    // ============== Conversion Tests ==============

    #[test]
    fn test_from_anyhow_error() {
        let anyhow_err = anyhow::anyhow!("something went wrong");
        let rct_err: RctError = anyhow_err.into();
        assert!(rct_err.to_string().contains("something went wrong"));
    }

    #[test]
    fn test_into_anyhow_error() {
        let rct_err = RctError::tool_timeout("test", 1000);
        let anyhow_err: anyhow::Error = rct_err.into();
        assert!(anyhow_err.to_string().contains("timed out"));
    }

    #[test]
    fn test_rct_result_type_alias() {
        fn returns_result() -> RctResult<i32> {
            Ok(42)
        }

        fn returns_error() -> RctResult<i32> {
            Err(RctError::tool_timeout("test", 1000))
        }

        assert_eq!(returns_result().unwrap(), 42);
        assert!(returns_error().is_err());
    }

    // ============== Category Tests ==============

    #[test]
    fn test_is_retryable() {
        // Network and rate limit errors should be retryable
        assert!(RctError::api_network("timeout").is_retryable());
        assert!(RctError::api_rate_limited(60).is_retryable());
        assert!(RctError::mcp_transport("connection lost").is_retryable());

        // Security violations should NOT be retryable
        assert!(!RctError::tool_security_violation("rm -rf", "dangerous").is_retryable());
        assert!(!RctError::api_authentication("invalid key").is_retryable());
        assert!(!RctError::session_integrity("checksum").is_retryable());
    }

    #[test]
    fn test_is_security_related() {
        // Security-related errors
        assert!(RctError::tool_path_traversal("/etc/passwd").is_security_related());
        assert!(RctError::tool_security_violation("rm -rf", "dangerous").is_security_related());
        assert!(RctError::mcp_validation("blocked command").is_security_related());
        assert!(RctError::hook_validation("dangerous").is_security_related());
        assert!(RctError::session_integrity("tampering").is_security_related());

        // Non-security errors
        assert!(!RctError::api_network("timeout").is_security_related());
        assert!(!RctError::tool_timeout("bash", 1000).is_security_related());
    }

    #[test]
    fn test_module_name() {
        assert_eq!(RctError::tool_timeout("test", 1000).module(), "tools");
        assert_eq!(RctError::api_network("timeout").module(), "api");
        assert_eq!(RctError::mcp_transport("lost").module(), "mcp");
        assert_eq!(RctError::session_integrity("bad").module(), "session");
        assert_eq!(RctError::hook_validation("blocked").module(), "hooks");
        assert_eq!(RctError::plugin_load("test", "missing").module(), "plugins");
        assert_eq!(RctError::context_io("/path", "error").module(), "context");
    }

    // ============== Debug Tests ==============

    #[test]
    fn test_debug_output() {
        let err = RctError::tool_timeout("bash", 30000);
        let debug = format!("{:?}", err);
        // Debug output should contain useful information
        assert!(debug.contains("Tool") || debug.contains("tool") || debug.contains("Timeout"));
    }
}
