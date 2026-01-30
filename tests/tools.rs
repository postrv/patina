//! Tool execution integration tests.
//!
//! Tests for tool execution including:
//! - Bash command execution
//! - Security policy enforcement
//! - File operations

mod common;

use common::TestContext;
use rct::tools::{ToolCall, ToolExecutionPolicy, ToolExecutor, ToolResult};
use serde_json::json;
use std::time::Duration;

/// Test that a simple bash command executes successfully.
#[tokio::test]
async fn test_bash_execution_success() {
    let ctx = TestContext::new();
    let executor = ToolExecutor::new(ctx.path());

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
            assert!(
                output.contains("hello"),
                "output should contain 'hello', got: {output}"
            );
        }
        ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        ToolResult::Cancelled => panic!("expected success, got cancelled"),
    }
}

/// Test that bash execution captures both stdout and stderr.
#[tokio::test]
async fn test_bash_captures_stdout_stderr() {
    let ctx = TestContext::new();
    let executor = ToolExecutor::new(ctx.path());

    // Command that writes to both stdout and stderr
    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "echo stdout_message && echo stderr_message >&2" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Success(output) => {
            assert!(
                output.contains("stdout_message"),
                "output should contain stdout, got: {output}"
            );
            assert!(
                output.contains("stderr_message"),
                "output should contain stderr, got: {output}"
            );
        }
        ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        ToolResult::Cancelled => panic!("expected success, got cancelled"),
    }
}

/// Test that bash execution returns error for failed commands.
#[tokio::test]
async fn test_bash_execution_failure() {
    let ctx = TestContext::new();
    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "exit 1" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Error(e) => {
            assert!(
                e.contains("Exit code 1"),
                "error should contain exit code, got: {e}"
            );
        }
        ToolResult::Success(s) => panic!("expected error, got success: {s}"),
        ToolResult::Cancelled => panic!("expected error, got cancelled"),
    }
}

/// Test that bash execution uses the working directory.
#[tokio::test]
async fn test_bash_uses_working_directory() {
    let ctx = TestContext::new();
    ctx.create_file("test_marker.txt", "marker content");
    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "cat test_marker.txt" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Success(output) => {
            assert!(
                output.contains("marker content"),
                "output should contain file content, got: {output}"
            );
        }
        ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        ToolResult::Cancelled => panic!("expected success, got cancelled"),
    }
}

/// Test that bash returns error when command field is missing.
#[tokio::test]
async fn test_bash_missing_command() {
    let ctx = TestContext::new();
    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({}),
    };

    let result = executor.execute(call).await;

    assert!(
        result.is_err(),
        "should return error for missing command field"
    );
}

// =============================================================================
// Security Blocking Tests (2.1.2)
// =============================================================================

/// Test that rm -rf / is blocked by security policy.
#[tokio::test]
async fn test_bash_blocks_rm_rf() {
    let ctx = TestContext::new();
    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "rm -rf /" }),
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
        ToolResult::Success(s) => panic!("rm -rf / should be blocked, got success: {s}"),
        ToolResult::Cancelled => panic!("expected error, got cancelled"),
    }
}

/// Test that sudo commands are blocked by security policy.
#[tokio::test]
async fn test_bash_blocks_sudo() {
    let ctx = TestContext::new();
    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "sudo rm file" }),
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
        ToolResult::Success(s) => panic!("sudo should be blocked, got success: {s}"),
        ToolResult::Cancelled => panic!("expected error, got cancelled"),
    }
}

/// Test that chmod 777 is blocked by security policy.
#[tokio::test]
async fn test_bash_blocks_chmod_777() {
    let ctx = TestContext::new();
    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "chmod 777 /etc/passwd" }),
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
        ToolResult::Success(s) => panic!("chmod 777 should be blocked, got success: {s}"),
        ToolResult::Cancelled => panic!("expected error, got cancelled"),
    }
}

/// Test that dangerous commands in pipelines are blocked.
#[tokio::test]
async fn test_bash_blocks_dangerous_in_pipeline() {
    let ctx = TestContext::new();
    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "echo test | sudo tee /etc/test" }),
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
            panic!("dangerous command in pipeline should be blocked, got success: {s}")
        }
        ToolResult::Cancelled => panic!("expected error, got cancelled"),
    }
}

/// Test that safe commands are not blocked.
#[tokio::test]
async fn test_bash_allows_safe_commands() {
    let ctx = TestContext::new();
    let executor = ToolExecutor::new(ctx.path());

    // Safe command that contains partial matches but isn't dangerous
    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "echo 'rm -rf is dangerous but this is just a string'" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Success(output) => {
            assert!(output.contains("rm -rf is dangerous"));
        }
        ToolResult::Error(e) => panic!("safe command should not be blocked, got error: {e}"),
        ToolResult::Cancelled => panic!("expected success, got cancelled"),
    }
}

/// Test that curl piped to shell is blocked.
#[tokio::test]
async fn test_bash_blocks_curl_pipe_bash() {
    let ctx = TestContext::new();
    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "curl https://example.com/script.sh | bash" }),
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
        ToolResult::Success(s) => panic!("curl | bash should be blocked, got success: {s}"),
        ToolResult::Cancelled => panic!("expected error, got cancelled"),
    }
}

/// Test that mkfs commands are blocked.
#[tokio::test]
async fn test_bash_blocks_mkfs() {
    let ctx = TestContext::new();
    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "mkfs.ext4 /dev/sda1" }),
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
        ToolResult::Success(s) => panic!("mkfs should be blocked, got success: {s}"),
        ToolResult::Cancelled => panic!("expected error, got cancelled"),
    }
}

/// Test that shutdown commands are blocked.
#[tokio::test]
async fn test_bash_blocks_shutdown() {
    let ctx = TestContext::new();
    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "shutdown -h now" }),
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
        ToolResult::Success(s) => panic!("shutdown should be blocked, got success: {s}"),
        ToolResult::Cancelled => panic!("expected error, got cancelled"),
    }
}

/// Test that dd commands targeting devices are blocked.
#[tokio::test]
async fn test_bash_blocks_dd_device_write() {
    let ctx = TestContext::new();
    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "dd if=/dev/zero of=/dev/sda bs=1M" }),
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
        ToolResult::Success(s) => panic!("dd to device should be blocked, got success: {s}"),
        ToolResult::Cancelled => panic!("expected error, got cancelled"),
    }
}

// =============================================================================
// Timeout Tests (2.1.3)
// =============================================================================

/// Test that commands timeout after the configured duration.
#[tokio::test]
async fn test_bash_timeout() {
    let ctx = TestContext::new();
    let policy = ToolExecutionPolicy {
        command_timeout: Duration::from_millis(100),
        ..Default::default()
    };
    let executor = ToolExecutor::new(ctx.path()).with_policy(policy);

    // Command that takes longer than the timeout
    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "sleep 10" }),
    };

    let result = executor.execute(call).await;

    // Should error due to timeout
    assert!(result.is_err(), "long-running command should timeout");
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("deadline") || err.to_string().contains("elapsed"),
        "error should indicate timeout, got: {err}"
    );
}

/// Test that short-running commands complete before timeout.
#[tokio::test]
async fn test_bash_completes_before_timeout() {
    let ctx = TestContext::new();
    let policy = ToolExecutionPolicy {
        command_timeout: Duration::from_secs(5),
        ..Default::default()
    };
    let executor = ToolExecutor::new(ctx.path()).with_policy(policy);

    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "echo fast" }),
    };

    let result = executor.execute(call).await.expect("should not timeout");

    match result {
        ToolResult::Success(output) => {
            assert!(output.contains("fast"));
        }
        ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        ToolResult::Cancelled => panic!("expected success, got cancelled"),
    }
}

/// Test that custom timeout policy is respected.
#[tokio::test]
async fn test_bash_custom_timeout_policy() {
    let ctx = TestContext::new();
    let policy = ToolExecutionPolicy {
        command_timeout: Duration::from_millis(50),
        ..Default::default()
    };
    let executor = ToolExecutor::new(ctx.path()).with_policy(policy);

    // Even a relatively short sleep should timeout with 50ms limit
    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "sleep 1" }),
    };

    let result = executor.execute(call).await;
    assert!(result.is_err(), "should timeout with 50ms limit");
}

/// Placeholder test to verify test infrastructure works.
#[test]
fn test_infrastructure_works() {
    let ctx = TestContext::new();
    assert!(ctx.path().exists());
}

// =============================================================================
// File Read Tests (2.2.1)
// =============================================================================

/// Test that read_file reads a file within the working directory.
#[tokio::test]
async fn test_file_read_within_working_dir() {
    let ctx = TestContext::new();
    ctx.create_file("readable.txt", "file content here");
    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "read_file".to_string(),
        input: json!({ "path": "readable.txt" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Success(content) => {
            assert_eq!(content, "file content here", "should read exact content");
        }
        ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        ToolResult::Cancelled => panic!("expected success, got cancelled"),
    }
}

/// Test that read_file blocks path traversal attacks.
#[tokio::test]
async fn test_file_read_blocks_path_traversal() {
    let ctx = TestContext::new();
    let working_dir = ctx.path();

    // Create a file in the parent directory (outside working dir)
    let parent_dir = working_dir.parent().expect("temp dir should have parent");
    let external_file = parent_dir.join("outside_workdir.txt");
    std::fs::write(&external_file, "external content").expect("failed to create test file");

    // Ensure cleanup on drop
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _cleanup = Cleanup(external_file);

    let executor = ToolExecutor::new(working_dir);

    // Attempt to read the external file via path traversal
    let call = ToolCall {
        name: "read_file".to_string(),
        input: json!({ "path": "../outside_workdir.txt" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Error(e) => {
            assert!(
                e.contains("path traversal") || e.contains("outside working directory"),
                "error should mention path traversal, got: {e}"
            );
        }
        ToolResult::Success(s) => {
            panic!("path traversal should be blocked, but read content: {s}")
        }
        ToolResult::Cancelled => panic!("expected error, got cancelled"),
    }
}

/// Test that read_file returns appropriate error for nonexistent files.
#[tokio::test]
async fn test_file_read_nonexistent() {
    let ctx = TestContext::new();
    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "read_file".to_string(),
        input: json!({ "path": "does_not_exist.txt" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Error(e) => {
            assert!(
                e.contains("not found")
                    || e.contains("No such file")
                    || e.contains("Failed to read"),
                "error should indicate file not found, got: {e}"
            );
        }
        ToolResult::Success(s) => panic!("expected error for nonexistent file, got success: {s}"),
        ToolResult::Cancelled => panic!("expected error, got cancelled"),
    }
}

// =============================================================================
// File Write Tests (2.2.2)
// =============================================================================

/// Test that write_file creates a file in the working directory.
#[tokio::test]
async fn test_file_write_creates_file() {
    let ctx = TestContext::new();
    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "write_file".to_string(),
        input: json!({ "path": "new_file.txt", "content": "written content" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Success(msg) => {
            assert!(
                msg.contains("Wrote") && msg.contains("bytes"),
                "should report bytes written, got: {msg}"
            );
            // Verify file was actually created
            let written_path = ctx.path().join("new_file.txt");
            let content = std::fs::read_to_string(&written_path).expect("file should exist");
            assert_eq!(content, "written content");
        }
        ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        ToolResult::Cancelled => panic!("expected success, got cancelled"),
    }
}

/// Test that write_file blocks path traversal attacks.
#[tokio::test]
async fn test_file_write_blocks_path_traversal() {
    let ctx = TestContext::new();
    let working_dir = ctx.path();
    let parent_dir = working_dir.parent().expect("temp dir should have parent");
    let escaped_file = parent_dir.join("should_not_be_created.txt");

    // Ensure cleanup
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _cleanup = Cleanup(escaped_file.clone());

    let executor = ToolExecutor::new(working_dir);

    let call = ToolCall {
        name: "write_file".to_string(),
        input: json!({ "path": "../should_not_be_created.txt", "content": "malicious content" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Error(e) => {
            assert!(
                e.contains("path traversal") || e.contains("outside working directory"),
                "error should mention path traversal, got: {e}"
            );
        }
        ToolResult::Success(s) => {
            panic!("path traversal should be blocked, but wrote file: {s}")
        }
        ToolResult::Cancelled => panic!("expected error, got cancelled"),
    }

    // Verify file was NOT created
    assert!(
        !escaped_file.exists(),
        "file should not have been created outside working directory"
    );
}

/// Test that write_file blocks writes to protected system paths.
#[tokio::test]
async fn test_file_write_blocks_protected_paths() {
    let ctx = TestContext::new();
    let executor = ToolExecutor::new(ctx.path());

    // Attempt to write to a protected path pattern (absolute path to /etc)
    let call = ToolCall {
        name: "write_file".to_string(),
        input: json!({ "path": "/etc/test_file", "content": "should not write" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Error(e) => {
            assert!(
                e.contains("protected")
                    || e.contains("outside working directory")
                    || e.contains("absolute"),
                "error should mention protected path, got: {e}"
            );
        }
        ToolResult::Success(s) => {
            panic!("writing to protected path should be blocked, got success: {s}")
        }
        ToolResult::Cancelled => panic!("expected error, got cancelled"),
    }
}

/// Test that write_file creates a backup when overwriting existing files.
#[tokio::test]
async fn test_file_write_creates_backup() {
    let ctx = TestContext::new();
    ctx.create_file("existing.txt", "original content");
    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "write_file".to_string(),
        input: json!({ "path": "existing.txt", "content": "new content" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Success(_) => {
            // Check that original content was backed up
            let backup_dir = ctx.path().join(".rct_backups");
            assert!(
                backup_dir.exists(),
                "backup directory should be created at {backup_dir:?}"
            );

            // Find backup file
            let backups: Vec<_> = std::fs::read_dir(&backup_dir)
                .expect("should read backup dir")
                .filter_map(|e| e.ok())
                .collect();
            assert!(!backups.is_empty(), "at least one backup file should exist");

            // Verify backup contains original content
            let backup_content =
                std::fs::read_to_string(backups[0].path()).expect("should read backup");
            assert!(
                backup_content.contains("original content"),
                "backup should contain original content, got: {backup_content}"
            );
        }
        ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        ToolResult::Cancelled => panic!("expected success, got cancelled"),
    }
}

// =============================================================================
// Edit Tool Tests (2.2.3)
// =============================================================================

/// Test that edit tool replaces a string in a file.
#[tokio::test]
async fn test_edit_replaces_string() {
    let ctx = TestContext::new();
    ctx.create_file("target.txt", "Hello world, hello universe!");
    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "edit".to_string(),
        input: json!({
            "path": "target.txt",
            "old_string": "world",
            "new_string": "planet"
        }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Success(output) => {
            // Verify the file was modified
            let content =
                std::fs::read_to_string(ctx.path().join("target.txt")).expect("file should exist");
            assert_eq!(
                content, "Hello planet, hello universe!",
                "should replace the matched string"
            );
            // Output should indicate success
            assert!(
                output.contains("replaced") || output.contains("edited") || output.contains("diff"),
                "output should indicate edit was made, got: {output}"
            );
        }
        ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        ToolResult::Cancelled => panic!("expected success, got cancelled"),
    }
}

/// Test that edit tool generates a diff output.
#[tokio::test]
async fn test_edit_generates_diff() {
    let ctx = TestContext::new();
    ctx.create_file("diff_target.txt", "line one\nline two\nline three\n");
    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "edit".to_string(),
        input: json!({
            "path": "diff_target.txt",
            "old_string": "line two",
            "new_string": "line TWO modified"
        }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Success(output) => {
            // Output should contain diff-like markers
            assert!(
                output.contains("-") && output.contains("+")
                    || output.contains("old") && output.contains("new")
                    || output.contains("line two") && output.contains("line TWO modified"),
                "output should show diff, got: {output}"
            );
        }
        ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        ToolResult::Cancelled => panic!("expected success, got cancelled"),
    }
}

/// Test that edit tool requires a unique match.
#[tokio::test]
async fn test_edit_unique_match_required() {
    let ctx = TestContext::new();
    ctx.create_file("ambiguous.txt", "foo bar foo baz foo");
    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "edit".to_string(),
        input: json!({
            "path": "ambiguous.txt",
            "old_string": "foo",
            "new_string": "qux"
        }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Error(e) => {
            assert!(
                e.contains("unique")
                    || e.contains("multiple")
                    || e.contains("ambiguous")
                    || e.contains("3 matches"),
                "error should mention non-unique match, got: {e}"
            );
            // Verify file was NOT modified
            let content = std::fs::read_to_string(ctx.path().join("ambiguous.txt"))
                .expect("file should exist");
            assert_eq!(
                content, "foo bar foo baz foo",
                "file should not be modified when match is ambiguous"
            );
        }
        ToolResult::Success(s) => {
            panic!("expected error for ambiguous match, got success: {s}")
        }
        ToolResult::Cancelled => panic!("expected error, got cancelled"),
    }
}

/// Test that edit tool handles nonexistent files.
#[tokio::test]
async fn test_edit_nonexistent_file() {
    let ctx = TestContext::new();
    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "edit".to_string(),
        input: json!({
            "path": "nonexistent.txt",
            "old_string": "foo",
            "new_string": "bar"
        }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Error(e) => {
            assert!(
                e.contains("not found") || e.contains("No such file") || e.contains("Failed"),
                "error should indicate file not found, got: {e}"
            );
        }
        ToolResult::Success(s) => panic!("expected error for nonexistent file, got success: {s}"),
        ToolResult::Cancelled => panic!("expected error, got cancelled"),
    }
}

/// Test that edit tool handles no match found.
#[tokio::test]
async fn test_edit_no_match() {
    let ctx = TestContext::new();
    ctx.create_file("no_match.txt", "hello world");
    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "edit".to_string(),
        input: json!({
            "path": "no_match.txt",
            "old_string": "xyz",
            "new_string": "abc"
        }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Error(e) => {
            assert!(
                e.contains("not found") || e.contains("no match") || e.contains("0 matches"),
                "error should indicate no match, got: {e}"
            );
        }
        ToolResult::Success(s) => panic!("expected error for no match, got success: {s}"),
        ToolResult::Cancelled => panic!("expected error, got cancelled"),
    }
}

// =============================================================================
// Glob Tool Tests (2.3.1)
// =============================================================================

/// Test that glob finds files matching a pattern.
#[tokio::test]
async fn test_glob_finds_files() {
    let ctx = TestContext::new();
    // Create test file structure
    ctx.create_file("src/main.rs", "fn main() {}");
    ctx.create_file("src/lib.rs", "pub fn lib() {}");
    ctx.create_file("src/utils/helpers.rs", "pub fn help() {}");
    ctx.create_file("tests/test.rs", "fn test() {}");
    ctx.create_file("README.md", "# Readme");

    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "glob".to_string(),
        input: json!({ "pattern": "**/*.rs" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Success(output) => {
            // Should find all .rs files
            assert!(
                output.contains("main.rs"),
                "should find main.rs, got: {output}"
            );
            assert!(
                output.contains("lib.rs"),
                "should find lib.rs, got: {output}"
            );
            assert!(
                output.contains("helpers.rs"),
                "should find helpers.rs, got: {output}"
            );
            assert!(
                output.contains("test.rs"),
                "should find test.rs, got: {output}"
            );
            // Should NOT find non-.rs files
            assert!(
                !output.contains("README.md"),
                "should not find README.md, got: {output}"
            );
        }
        ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        ToolResult::Cancelled => panic!("expected success, got cancelled"),
    }
}

/// Test that glob respects .gitignore patterns.
#[tokio::test]
async fn test_glob_respects_gitignore() {
    let ctx = TestContext::new();
    // Create test file structure with ignored files
    ctx.create_file(".gitignore", "target/\n*.log\n");
    ctx.create_file("src/main.rs", "fn main() {}");
    ctx.create_file("target/debug/app", "binary");
    ctx.create_file("debug.log", "log content");
    ctx.create_file("app.rs", "fn app() {}");

    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "glob".to_string(),
        input: json!({ "pattern": "**/*", "respect_gitignore": true }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Success(output) => {
            // Should find non-ignored files
            assert!(
                output.contains("main.rs") || output.contains("app.rs"),
                "should find non-ignored .rs files, got: {output}"
            );
            // Should NOT find ignored files
            assert!(
                !output.contains("target/debug"),
                "should respect .gitignore for target/, got: {output}"
            );
            assert!(
                !output.contains("debug.log"),
                "should respect .gitignore for *.log, got: {output}"
            );
        }
        ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        ToolResult::Cancelled => panic!("expected success, got cancelled"),
    }
}

/// Test that glob handles patterns with no matches.
#[tokio::test]
async fn test_glob_no_matches() {
    let ctx = TestContext::new();
    ctx.create_file("file.txt", "content");

    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "glob".to_string(),
        input: json!({ "pattern": "**/*.xyz" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Success(output) => {
            // Should return empty or indicate no matches
            assert!(
                output.is_empty() || output.contains("No matches") || output.trim().is_empty(),
                "should indicate no matches found, got: {output}"
            );
        }
        ToolResult::Error(e) => {
            // Also acceptable to return error for no matches
            assert!(
                e.contains("no match") || e.contains("No files"),
                "error should indicate no matches, got: {e}"
            );
        }
        ToolResult::Cancelled => panic!("expected success or no-match error, got cancelled"),
    }
}

/// Test that glob validates patterns within working directory.
#[tokio::test]
async fn test_glob_blocks_path_traversal() {
    let ctx = TestContext::new();
    let executor = ToolExecutor::new(ctx.path());

    // Attempt to glob outside working directory
    let call = ToolCall {
        name: "glob".to_string(),
        input: json!({ "pattern": "../**/*.rs" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Error(e) => {
            assert!(
                e.contains("path traversal")
                    || e.contains("outside working directory")
                    || e.contains("invalid pattern"),
                "error should mention path traversal, got: {e}"
            );
        }
        ToolResult::Success(output) => {
            // If it succeeds, it should not contain files from outside working directory
            // This is an acceptable outcome if the implementation sanitizes the pattern
            assert!(
                !output.contains("/Users/") && !output.contains("/home/"),
                "should not return files from outside working directory, got: {output}"
            );
        }
        ToolResult::Cancelled => panic!("expected error or sanitized success, got cancelled"),
    }
}

// =============================================================================
// Grep Tool Tests (2.3.2)
// =============================================================================

/// Test that grep finds content matching a pattern.
#[tokio::test]
async fn test_grep_finds_content() {
    let ctx = TestContext::new();
    ctx.create_file("file1.rs", "fn hello_world() {}\nfn goodbye() {}");
    ctx.create_file("file2.rs", "fn hello_universe() {}\nfn test() {}");
    ctx.create_file("file3.txt", "no functions here");

    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "grep".to_string(),
        input: json!({ "pattern": "hello" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Success(output) => {
            // Should find lines containing "hello"
            assert!(
                output.contains("hello_world") || output.contains("file1.rs"),
                "should find hello_world, got: {output}"
            );
            assert!(
                output.contains("hello_universe") || output.contains("file2.rs"),
                "should find hello_universe, got: {output}"
            );
            // Should NOT match file without "hello"
            assert!(
                !output.contains("no functions here"),
                "should not include non-matching content, got: {output}"
            );
        }
        ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        ToolResult::Cancelled => panic!("expected success, got cancelled"),
    }
}

/// Test that grep supports regex patterns.
#[tokio::test]
async fn test_grep_regex_support() {
    let ctx = TestContext::new();
    ctx.create_file(
        "code.rs",
        "fn test_one() {}\nfn test_two() {}\nfn other() {}",
    );

    let executor = ToolExecutor::new(ctx.path());

    // Use regex pattern to match test_* functions
    let call = ToolCall {
        name: "grep".to_string(),
        input: json!({ "pattern": r"fn test_\w+" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Success(output) => {
            assert!(
                output.contains("test_one"),
                "should match test_one with regex, got: {output}"
            );
            assert!(
                output.contains("test_two"),
                "should match test_two with regex, got: {output}"
            );
            // Should NOT match "other" which doesn't match the pattern
            assert!(
                !output.contains("fn other"),
                "should not match 'other' which doesn't fit pattern, got: {output}"
            );
        }
        ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        ToolResult::Cancelled => panic!("expected success, got cancelled"),
    }
}

/// Test that grep supports case-insensitive search.
#[tokio::test]
async fn test_grep_case_insensitive() {
    let ctx = TestContext::new();
    ctx.create_file("mixed.txt", "Hello World\nHELLO AGAIN\nhello there");

    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "grep".to_string(),
        input: json!({ "pattern": "hello", "case_insensitive": true }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Success(output) => {
            // Should match all variants
            assert!(
                output.contains("Hello") || output.contains("hello"),
                "should find case-insensitive matches, got: {output}"
            );
            // Count matches (should be 3 lines)
            let line_count = output.lines().filter(|l| !l.is_empty()).count();
            assert!(
                line_count >= 2,
                "should find multiple case variations, found {line_count} lines"
            );
        }
        ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        ToolResult::Cancelled => panic!("expected success, got cancelled"),
    }
}

/// Test that grep handles no matches.
#[tokio::test]
async fn test_grep_no_matches() {
    let ctx = TestContext::new();
    ctx.create_file("file.txt", "some content here");

    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "grep".to_string(),
        input: json!({ "pattern": "xyz123notfound" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Success(output) => {
            // Should return empty or indicate no matches
            assert!(
                output.is_empty() || output.contains("No matches") || output.trim().is_empty(),
                "should indicate no matches found, got: {output}"
            );
        }
        ToolResult::Error(e) => {
            // Also acceptable to return error for no matches
            assert!(
                e.contains("no match") || e.contains("No matches"),
                "error should indicate no matches, got: {e}"
            );
        }
        ToolResult::Cancelled => panic!("expected success or no-match error, got cancelled"),
    }
}

/// Test that grep can filter by file pattern.
#[tokio::test]
async fn test_grep_file_filter() {
    let ctx = TestContext::new();
    ctx.create_file("code.rs", "fn hello() {}");
    ctx.create_file("code.py", "def hello(): pass");
    ctx.create_file("code.txt", "hello text");

    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "grep".to_string(),
        input: json!({ "pattern": "hello", "file_pattern": "*.rs" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Success(output) => {
            // Should only find match in .rs file
            assert!(
                output.contains("code.rs") || output.contains("fn hello"),
                "should find match in .rs file, got: {output}"
            );
            // Should NOT find matches in other file types
            assert!(
                !output.contains("code.py") && !output.contains("def hello"),
                "should not include .py file, got: {output}"
            );
            assert!(
                !output.contains("code.txt") && !output.contains("hello text"),
                "should not include .txt file, got: {output}"
            );
        }
        ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        ToolResult::Cancelled => panic!("expected success, got cancelled"),
    }
}
