//! Basic bash execution tests.

use super::common::TestContext;
use patina::tools::{ToolCall, ToolExecutionPolicy, ToolExecutor, ToolResult};
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
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
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
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
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
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
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
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
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
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
    }
}

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
        err.to_string().contains("deadline")
            || err.to_string().contains("elapsed")
            || err.to_string().contains("timed out"),
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
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
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

/// Test that bash output is truncated when it exceeds max_output_size.
#[tokio::test]
async fn test_bash_output_truncated_when_exceeds_limit() {
    let ctx = TestContext::new();

    // Create a policy with a small output size limit for testing (10KB)
    let policy = ToolExecutionPolicy {
        max_output_size: 10 * 1024, // 10KB
        ..Default::default()
    };
    let executor = ToolExecutor::new(ctx.path()).with_policy(policy);

    // Generate output larger than the limit
    // seq 1 5000 produces about 24KB of output on most systems
    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "seq 1 5000" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Success(output) => {
            // Output should be truncated to around max_output_size
            assert!(
                output.len() <= 15 * 1024, // Allow some buffer for truncation message
                "output should be truncated, got {} bytes",
                output.len()
            );
            // Should contain truncation notice
            assert!(
                output.contains("truncated") || output.contains("Output"),
                "output should mention truncation"
            );
        }
        ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        ToolResult::Cancelled => panic!("expected success, got cancelled"),
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
    }
}

/// Test that bash output is NOT truncated when within limit.
#[tokio::test]
async fn test_bash_output_not_truncated_when_within_limit() {
    let ctx = TestContext::new();

    // Create a policy with a reasonable limit
    let policy = ToolExecutionPolicy {
        max_output_size: 1024 * 1024, // 1MB
        ..Default::default()
    };
    let executor = ToolExecutor::new(ctx.path()).with_policy(policy);

    // Generate small output
    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "echo 'small output'" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Success(output) => {
            assert!(
                !output.contains("truncated"),
                "small output should not be truncated"
            );
            assert!(
                output.contains("small output"),
                "output should contain expected content"
            );
        }
        ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        ToolResult::Cancelled => panic!("expected success, got cancelled"),
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
    }
}

/// Test that bash timeout actually kills the spawned process.
///
/// This test verifies not just that the command times out, but that the
/// underlying process is actually terminated and not left running as a
/// zombie or orphan process.
#[cfg(unix)]
#[tokio::test]
async fn test_bash_timeout_kills_process() {
    let ctx = TestContext::new();
    let marker_file = ctx.path().join("timeout_marker.txt");
    let counter_file = ctx.path().join("timeout_counter.txt");

    let policy = ToolExecutionPolicy {
        command_timeout: Duration::from_millis(200),
        ..Default::default()
    };
    let executor = ToolExecutor::new(ctx.path()).with_policy(policy);

    // Command that writes to a file every 100ms in a loop
    // If the process isn't killed, it would keep incrementing
    let command = format!(
        r#"echo "started" > {:?}; for i in 1 2 3 4 5 6 7 8 9 10; do echo $i >> {:?}; sleep 0.1; done"#,
        marker_file, counter_file
    );

    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": command }),
    };

    let result = executor.execute(call).await;

    // Should error due to timeout
    assert!(result.is_err(), "command should timeout");

    // Wait a moment for any lingering process activity
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Marker file should exist (process started)
    assert!(
        marker_file.exists(),
        "process should have started and created marker file"
    );

    // Counter file might exist with some entries, but should have stopped incrementing
    if counter_file.exists() {
        let initial_content = std::fs::read_to_string(&counter_file).unwrap_or_default();
        let initial_lines = initial_content.lines().count();

        // Wait a bit more to ensure process is truly dead
        tokio::time::sleep(Duration::from_millis(200)).await;

        let final_content = std::fs::read_to_string(&counter_file).unwrap_or_default();
        let final_lines = final_content.lines().count();

        // If process was killed, line count should not have increased
        assert_eq!(
            initial_lines, final_lines,
            "process should be killed - counter file should stop growing (initial: {}, final: {})",
            initial_lines, final_lines
        );

        // Should not have completed all 10 iterations (that would take 1 second)
        assert!(
            final_lines < 10,
            "process should have been killed before completing all iterations, got {} lines",
            final_lines
        );
    }
}
