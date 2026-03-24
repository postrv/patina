//! Allowlist/denylist and permission tests.

use super::common::TestContext;
use patina::tools::{ToolCall, ToolExecutionPolicy, ToolExecutor, ToolResult};
use regex::Regex;
use serde_json::json;

// =============================================================================
// Allowlist Mode Tests (1.1.3) - Security Hardening
// =============================================================================

/// Test that allowlist mode blocks commands not in the allowlist.
#[tokio::test]
async fn test_allowlist_mode_blocks_unlisted_commands() {
    let ctx = TestContext::new();
    let policy = ToolExecutionPolicy {
        allowlist_mode: true,
        allowed_commands: vec![Regex::new(r"^echo\s+").unwrap()],
        ..Default::default()
    };
    let executor = ToolExecutor::new(ctx.path()).with_policy(policy);

    // Command not in allowlist
    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "ls -la" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Error(e) => {
            assert!(
                e.contains("not in allowlist") || e.contains("blocked"),
                "error should indicate command not in allowlist, got: {e}"
            );
        }
        ToolResult::Success(s) => {
            panic!("command should be blocked in allowlist mode, got success: {s}")
        }
        ToolResult::Cancelled => panic!("expected error, got cancelled"),
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
    }
}

/// Test that allowlist mode allows commands in the allowlist.
#[tokio::test]
async fn test_allowlist_mode_allows_listed_commands() {
    let ctx = TestContext::new();
    let policy = ToolExecutionPolicy {
        allowlist_mode: true,
        allowed_commands: vec![Regex::new(r"^echo\s+").unwrap()],
        ..Default::default()
    };
    let executor = ToolExecutor::new(ctx.path()).with_policy(policy);

    // Command in allowlist
    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "echo hello" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Success(output) => {
            assert!(output.contains("hello"), "output should contain 'hello'");
        }
        ToolResult::Error(e) => panic!("allowed command should succeed, got error: {e}"),
        ToolResult::Cancelled => panic!("expected success, got cancelled"),
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
    }
}

/// Test that allowlist mode still blocks dangerous commands even if they match allowlist.
#[cfg(unix)]
#[tokio::test]
async fn test_allowlist_mode_still_blocks_dangerous() {
    let ctx = TestContext::new();
    let policy = ToolExecutionPolicy {
        allowlist_mode: true,
        // Allowlist that would match dangerous command
        allowed_commands: vec![Regex::new(r".*").unwrap()],
        ..Default::default()
    };
    let executor = ToolExecutor::new(ctx.path()).with_policy(policy);

    // Dangerous command that matches allowlist but should still be blocked
    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "sudo rm -rf /" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Error(e) => {
            assert!(
                e.contains("blocked by security policy"),
                "error should mention security policy, got: {e}"
            );
        }
        ToolResult::Success(s) => {
            panic!("dangerous command should be blocked even with allowlist, got: {s}")
        }
        ToolResult::Cancelled => panic!("expected error, got cancelled"),
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
    }
}

/// Test that allowlist mode with empty allowlist blocks all commands.
#[tokio::test]
async fn test_allowlist_mode_empty_blocks_all() {
    let ctx = TestContext::new();
    let policy = ToolExecutionPolicy {
        allowlist_mode: true,
        allowed_commands: vec![], // Empty allowlist
        ..Default::default()
    };
    let executor = ToolExecutor::new(ctx.path()).with_policy(policy);

    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "echo test" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Error(e) => {
            assert!(
                e.contains("not in allowlist") || e.contains("blocked"),
                "error should indicate command blocked, got: {e}"
            );
        }
        ToolResult::Success(s) => {
            panic!("empty allowlist should block all commands, got success: {s}")
        }
        ToolResult::Cancelled => panic!("expected error, got cancelled"),
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
    }
}

/// Test that allowlist mode disabled (default) allows safe commands.
#[tokio::test]
async fn test_allowlist_mode_disabled_allows_safe() {
    let ctx = TestContext::new();
    // Default policy has allowlist_mode = false
    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "ls" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Success(_) => {
            // Expected - command executed successfully
        }
        ToolResult::Error(e) => {
            // ls might fail for other reasons but should not be blocked by policy
            if e.contains("blocked") || e.contains("allowlist") {
                panic!("safe command should not be blocked with allowlist disabled, got: {e}")
            }
        }
        ToolResult::Cancelled => panic!("expected success or non-policy error"),
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
    }
}

// =============================================================================
// Error Path Tests (3.1.1) - File Operation Error Handling
// =============================================================================

/// Test that read_file returns proper error when file permissions deny read access.
#[cfg(unix)]
#[tokio::test]
async fn test_read_file_permission_denied() {
    use std::os::unix::fs::PermissionsExt;

    let ctx = TestContext::new();
    let file_path = ctx.create_file("no_read_perms.txt", "secret content");

    // Remove read permissions (write-only)
    let mut perms = std::fs::metadata(&file_path)
        .expect("file should exist")
        .permissions();
    perms.set_mode(0o200); // Write-only
    std::fs::set_permissions(&file_path, perms).expect("failed to set permissions");

    // Ensure permissions are restored on cleanup
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(mut perms) = std::fs::metadata(&self.0).map(|m| m.permissions()) {
                perms.set_mode(0o644);
                let _ = std::fs::set_permissions(&self.0, perms);
            }
        }
    }
    let _cleanup = Cleanup(file_path);

    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "read_file".to_string(),
        input: json!({ "path": "no_read_perms.txt" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Error(e) => {
            assert!(
                e.contains("Permission denied")
                    || e.contains("permission denied")
                    || e.contains("Failed to read"),
                "error should indicate permission denied, got: {e}"
            );
        }
        ToolResult::Success(s) => {
            panic!("read_file should fail with permission denied, got success: {s}")
        }
        ToolResult::Cancelled => panic!("expected error, got cancelled"),
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
    }
}

/// Test that write_file returns proper error when writing to a read-only directory.
#[cfg(unix)]
#[tokio::test]
async fn test_write_file_to_readonly_directory() {
    use std::os::unix::fs::PermissionsExt;

    let ctx = TestContext::new();

    // Create a subdirectory and make it read-only
    let readonly_dir = ctx.path().join("readonly_subdir");
    std::fs::create_dir(&readonly_dir).expect("failed to create directory");

    let mut perms = std::fs::metadata(&readonly_dir)
        .expect("dir should exist")
        .permissions();
    perms.set_mode(0o555); // Read + execute only, no write
    std::fs::set_permissions(&readonly_dir, perms.clone()).expect("failed to set permissions");

    // Ensure permissions are restored on cleanup
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(mut perms) = std::fs::metadata(&self.0).map(|m| m.permissions()) {
                perms.set_mode(0o755);
                let _ = std::fs::set_permissions(&self.0, perms);
            }
        }
    }
    let _cleanup = Cleanup(readonly_dir.clone());

    let executor = ToolExecutor::new(ctx.path());

    // Attempt to write to the read-only directory
    let call = ToolCall {
        name: "write_file".to_string(),
        input: json!({ "path": "readonly_subdir/new_file.txt", "content": "should fail" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Error(e) => {
            assert!(
                e.contains("Permission denied")
                    || e.contains("permission denied")
                    || e.contains("Failed to write")
                    || e.contains("Read-only"),
                "error should indicate write failure, got: {e}"
            );
        }
        ToolResult::Success(s) => {
            panic!("write_file should fail on read-only directory, got success: {s}")
        }
        ToolResult::Cancelled => panic!("expected error, got cancelled"),
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
    }
}

/// Test that edit tool returns proper error when file has no read permission.
#[cfg(unix)]
#[tokio::test]
async fn test_edit_file_no_read_permission() {
    use std::os::unix::fs::PermissionsExt;

    let ctx = TestContext::new();
    let file_path = ctx.create_file("no_edit_perms.txt", "original content");

    // Remove read permissions
    let mut perms = std::fs::metadata(&file_path)
        .expect("file should exist")
        .permissions();
    perms.set_mode(0o200); // Write-only
    std::fs::set_permissions(&file_path, perms).expect("failed to set permissions");

    // Ensure permissions are restored on cleanup
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(mut perms) = std::fs::metadata(&self.0).map(|m| m.permissions()) {
                perms.set_mode(0o644);
                let _ = std::fs::set_permissions(&self.0, perms);
            }
        }
    }
    let _cleanup = Cleanup(file_path);

    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "edit".to_string(),
        input: json!({
            "path": "no_edit_perms.txt",
            "old_string": "original",
            "new_string": "modified"
        }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Error(e) => {
            assert!(
                e.contains("Permission denied")
                    || e.contains("permission denied")
                    || e.contains("Failed to read"),
                "error should indicate permission denied, got: {e}"
            );
        }
        ToolResult::Success(s) => {
            panic!("edit should fail with permission denied, got success: {s}")
        }
        ToolResult::Cancelled => panic!("expected error, got cancelled"),
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
    }
}

/// Test that edit tool returns proper error when file has no write permission.
#[cfg(unix)]
#[tokio::test]
async fn test_edit_file_no_write_permission() {
    use std::os::unix::fs::PermissionsExt;

    let ctx = TestContext::new();
    let file_path = ctx.create_file("no_write_perms.txt", "original content here");

    // Make file read-only
    let mut perms = std::fs::metadata(&file_path)
        .expect("file should exist")
        .permissions();
    perms.set_mode(0o444); // Read-only
    std::fs::set_permissions(&file_path, perms).expect("failed to set permissions");

    // Ensure permissions are restored on cleanup
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(mut perms) = std::fs::metadata(&self.0).map(|m| m.permissions()) {
                perms.set_mode(0o644);
                let _ = std::fs::set_permissions(&self.0, perms);
            }
        }
    }
    let _cleanup = Cleanup(file_path);

    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "edit".to_string(),
        input: json!({
            "path": "no_write_perms.txt",
            "old_string": "original",
            "new_string": "modified"
        }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Error(e) => {
            assert!(
                e.contains("Permission denied")
                    || e.contains("permission denied")
                    || e.contains("Failed to write")
                    || e.contains("Failed to create backup"),
                "error should indicate write failure, got: {e}"
            );
        }
        ToolResult::Success(s) => {
            panic!("edit should fail with permission denied, got success: {s}")
        }
        ToolResult::Cancelled => panic!("expected error, got cancelled"),
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
    }
}
