//! Tests for error handling and logging throughout the application.
//!
//! These tests verify that error paths return appropriate errors.
//! Note: Log output verification is limited in test environments due to
//! tracing-test crate limitations with async code and cross-crate logging.
//! The actual logging implementation can be verified with RUST_LOG=debug.

use rct::tools::{ToolCall, ToolExecutionPolicy, ToolExecutor, ToolResult};
use serde_json::json;
use std::time::Duration;
use tempfile::TempDir;

// =============================================================================
// Tools Module - Security Violation Handling
// =============================================================================

/// Security violations from dangerous command patterns should return errors.
#[tokio::test]
async fn test_bash_security_violation_returns_error() {
    let temp_dir = TempDir::new().unwrap();
    let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "sudo rm -rf /" }),
    };

    let result = executor.execute(call).await.unwrap();

    // Verify security violation is caught and returns error
    match result {
        ToolResult::Error(msg) => {
            assert!(
                msg.contains("blocked by security policy"),
                "Expected security policy error, got: {}",
                msg
            );
        }
        _ => panic!("Expected ToolResult::Error for dangerous command"),
    }
}

/// Path traversal attempts should return errors.
#[tokio::test]
async fn test_path_traversal_returns_error() {
    let temp_dir = TempDir::new().unwrap();
    let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

    let call = ToolCall {
        name: "read_file".to_string(),
        input: json!({ "path": "../../../etc/passwd" }),
    };

    let result = executor.execute(call).await.unwrap();

    match result {
        ToolResult::Error(msg) => {
            assert!(
                msg.contains("traversal") || msg.contains("outside working directory"),
                "Expected path traversal error, got: {}",
                msg
            );
        }
        _ => panic!("Expected ToolResult::Error for path traversal"),
    }
}

/// Symlink rejections should return errors for TOCTOU security.
#[cfg(unix)]
#[tokio::test]
async fn test_symlink_rejection_returns_error() {
    let temp_dir = TempDir::new().unwrap();
    let temp_path = temp_dir.path();

    // Create a symlink
    let link_path = temp_path.join("link_to_file");
    let target_path = temp_path.join("target.txt");
    std::fs::write(&target_path, "content").unwrap();
    std::os::unix::fs::symlink(&target_path, &link_path).unwrap();

    let executor = ToolExecutor::new(temp_path.to_path_buf());

    let call = ToolCall {
        name: "read_file".to_string(),
        input: json!({ "path": "link_to_file" }),
    };

    let result = executor.execute(call).await.unwrap();

    match result {
        ToolResult::Error(msg) => {
            assert!(
                msg.contains("Symlink") || msg.contains("symlink") || msg.contains("TOCTOU"),
                "Expected symlink rejection error, got: {}",
                msg
            );
        }
        _ => panic!("Expected ToolResult::Error for symlink"),
    }
}

/// Command timeouts should return errors.
#[tokio::test]
async fn test_command_timeout_returns_error() {
    let temp_dir = TempDir::new().unwrap();
    let policy = ToolExecutionPolicy {
        command_timeout: Duration::from_millis(50),
        ..Default::default()
    };
    let executor = ToolExecutor::new(temp_dir.path().to_path_buf()).with_policy(policy);

    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "sleep 10" }),
    };

    let result = executor.execute(call).await;

    // Command should timeout and return an error
    assert!(result.is_err(), "Expected error from timeout");
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("timed out"),
        "Expected timeout error, got: {}",
        err
    );
}

/// Allowlist mode blocking should return errors.
#[tokio::test]
async fn test_allowlist_block_returns_error() {
    let temp_dir = TempDir::new().unwrap();
    let policy = ToolExecutionPolicy {
        allowlist_mode: true,
        allowed_commands: vec![regex::Regex::new(r"^echo\s").unwrap()],
        ..Default::default()
    };
    let executor = ToolExecutor::new(temp_dir.path().to_path_buf()).with_policy(policy);

    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "ls -la" }),
    };

    let result = executor.execute(call).await.unwrap();

    match result {
        ToolResult::Error(msg) => {
            assert!(
                msg.contains("not in allowlist"),
                "Expected allowlist error, got: {}",
                msg
            );
        }
        _ => panic!("Expected ToolResult::Error for non-allowed command"),
    }
}

// =============================================================================
// Tools Module - File Operation Error Handling
// =============================================================================

/// File not found errors should return errors.
#[tokio::test]
async fn test_file_not_found_returns_error() {
    let temp_dir = TempDir::new().unwrap();
    let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

    let call = ToolCall {
        name: "read_file".to_string(),
        input: json!({ "path": "nonexistent_file.txt" }),
    };

    let result = executor.execute(call).await.unwrap();

    match result {
        ToolResult::Error(msg) => {
            assert!(
                msg.contains("Failed to read file") || msg.contains("No such file"),
                "Expected file not found error, got: {}",
                msg
            );
        }
        _ => panic!("Expected ToolResult::Error for missing file"),
    }
}

/// Write to read-only directory should return error.
#[tokio::test]
async fn test_write_permission_denied_returns_error() {
    let temp_dir = TempDir::new().unwrap();
    let temp_path = temp_dir.path();

    // Create a read-only directory
    let readonly_dir = temp_path.join("readonly");
    std::fs::create_dir(&readonly_dir).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&readonly_dir, std::fs::Permissions::from_mode(0o444)).unwrap();
    }

    let executor = ToolExecutor::new(temp_path.to_path_buf());

    let call = ToolCall {
        name: "write_file".to_string(),
        input: json!({
            "path": "readonly/test.txt",
            "content": "test content"
        }),
    };

    let result = executor.execute(call).await.unwrap();

    // Clean up permissions
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&readonly_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    match result {
        ToolResult::Error(msg) => {
            assert!(
                msg.contains("Failed to write") || msg.contains("Permission denied"),
                "Expected write error, got: {}",
                msg
            );
        }
        _ => panic!("Expected ToolResult::Error for permission denied"),
    }
}

/// File size limit exceeded should return error.
#[tokio::test]
async fn test_file_size_limit_returns_error() {
    let temp_dir = TempDir::new().unwrap();
    let policy = ToolExecutionPolicy {
        max_file_size: 100, // Very small limit
        ..Default::default()
    };
    let executor = ToolExecutor::new(temp_dir.path().to_path_buf()).with_policy(policy);

    let large_content = "x".repeat(200);
    let call = ToolCall {
        name: "write_file".to_string(),
        input: json!({
            "path": "large_file.txt",
            "content": large_content
        }),
    };

    let result = executor.execute(call).await.unwrap();

    match result {
        ToolResult::Error(msg) => {
            assert!(
                msg.contains("exceeds limit") || msg.contains("size"),
                "Expected size limit error, got: {}",
                msg
            );
        }
        _ => panic!("Expected ToolResult::Error for size limit"),
    }
}

/// Directory listing errors should return errors.
#[tokio::test]
async fn test_list_directory_error_returns_error() {
    let temp_dir = TempDir::new().unwrap();
    let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

    let call = ToolCall {
        name: "list_files".to_string(),
        input: json!({ "path": "nonexistent_directory" }),
    };

    let result = executor.execute(call).await.unwrap();

    match result {
        ToolResult::Error(msg) => {
            assert!(
                msg.contains("Failed to list") || msg.contains("directory"),
                "Expected directory error, got: {}",
                msg
            );
        }
        _ => panic!("Expected ToolResult::Error for missing directory"),
    }
}

/// Invalid glob pattern should return error.
#[tokio::test]
async fn test_invalid_glob_pattern_returns_error() {
    let temp_dir = TempDir::new().unwrap();
    let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

    let call = ToolCall {
        name: "glob".to_string(),
        input: json!({ "pattern": "[invalid" }),
    };

    let result = executor.execute(call).await.unwrap();

    match result {
        ToolResult::Error(msg) => {
            assert!(
                msg.contains("Invalid glob pattern"),
                "Expected glob pattern error, got: {}",
                msg
            );
        }
        _ => panic!("Expected ToolResult::Error for invalid glob"),
    }
}

/// Invalid regex pattern in grep should return error.
#[tokio::test]
async fn test_invalid_regex_pattern_returns_error() {
    let temp_dir = TempDir::new().unwrap();
    let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

    let call = ToolCall {
        name: "grep".to_string(),
        input: json!({ "pattern": "[invalid" }),
    };

    let result = executor.execute(call).await.unwrap();

    match result {
        ToolResult::Error(msg) => {
            assert!(
                msg.contains("Invalid regex pattern"),
                "Expected regex pattern error, got: {}",
                msg
            );
        }
        _ => panic!("Expected ToolResult::Error for invalid regex"),
    }
}

// =============================================================================
// Session Module - Integrity and Validation Handling
// =============================================================================

/// Session integrity violations should return errors.
#[tokio::test]
async fn test_session_integrity_violation_returns_error() {
    use rct::session::SessionManager;

    let temp_dir = TempDir::new().unwrap();
    let session_path = temp_dir.path().join("tampered.json");

    // Write a tampered session file with invalid checksum
    let tampered_json = r#"{
        "session": {
            "id": "test-id",
            "messages": [],
            "working_dir": "/test",
            "created_at": {"secs_since_epoch": 0, "nanos_since_epoch": 0},
            "updated_at": {"secs_since_epoch": 0, "nanos_since_epoch": 0}
        },
        "checksum": "invalid_checksum_value"
    }"#;
    std::fs::write(&session_path, tampered_json).unwrap();

    let manager = SessionManager::new(temp_dir.path().to_path_buf());
    let result = manager.load("tampered").await;

    assert!(result.is_err(), "Expected error for tampered session");
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("checksum") || err.to_string().contains("integrity"),
        "Expected integrity error, got: {}",
        err
    );
}

/// Session validation failures should return errors.
#[tokio::test]
async fn test_session_validation_returns_error() {
    use rct::session::SessionManager;

    let temp_dir = TempDir::new().unwrap();
    let manager = SessionManager::new(temp_dir.path().to_path_buf());

    // Attempt path traversal in session ID
    let result = manager.load("../../../etc/passwd").await;

    assert!(result.is_err(), "Expected error for invalid session ID");
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("invalid session ID") || err.to_string().contains("alphanumeric"),
        "Expected validation error, got: {}",
        err
    );
}

// =============================================================================
// Note on Log Verification
// =============================================================================
//
// The logging implementation adds tracing::warn! and tracing::debug! calls
// to all error paths in the tools and session modules. To verify logging
// in development/production:
//
//   RUST_LOG=rct=debug cargo run
//
// Log messages include:
// - "Security violation: command blocked by dangerous pattern"
// - "Security: command blocked by allowlist policy"
// - "Security: path traversal attempt"
// - "Security: symlink rejected"
// - "Bash command timed out and was killed"
// - "File read failed" / "File write failed"
// - "File write blocked: size exceeds limit"
// - "Directory listing failed"
// - "Invalid glob pattern" / "Invalid regex pattern"
// - "Security: session integrity check failed"
// - "Security: session validation failed"
