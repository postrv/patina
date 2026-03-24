//! Parallel execution and shell state persistence tests.

use super::common::TestContext;
use patina::tools::{ToolCall, ToolExecutor, ToolResult};
use serde_json::json;
use std::sync::Arc;

/// Placeholder test to verify test infrastructure works.
#[test]
fn test_infrastructure_works() {
    let ctx = TestContext::new();
    assert!(ctx.path().exists());
}

// =============================================================================
// Concurrency Tests (3.6.1) - Parallel Tool Execution
// =============================================================================

/// Test that parallel file operations complete without race conditions.
#[tokio::test]
async fn test_parallel_file_operations() {
    let ctx = TestContext::new();

    // Create test files
    for i in 0..5 {
        ctx.create_file(
            &format!("file_{}.txt", i),
            &format!("initial content {}", i),
        );
    }

    let working_dir = ctx.path().to_path_buf();

    // Launch parallel read operations
    let mut handles = Vec::new();
    for i in 0..5 {
        let wd = working_dir.clone();
        let handle = tokio::spawn(async move {
            let executor = ToolExecutor::new(wd);
            let call = ToolCall {
                name: "read_file".to_string(),
                input: json!({ "path": format!("file_{}.txt", i) }),
            };
            executor.execute(call).await
        });
        handles.push(handle);
    }

    // Wait for all to complete
    let mut success_count = 0;
    for handle in handles {
        let result = handle.await.expect("task should not panic");
        match result {
            Ok(ToolResult::Success(_)) => success_count += 1,
            Ok(ToolResult::Error(e)) => panic!("Concurrent read failed: {}", e),
            _ => {}
        }
    }

    assert_eq!(success_count, 5, "All parallel reads should succeed");

    // Launch parallel write operations to DIFFERENT files
    let mut write_handles = Vec::new();
    for i in 0..5 {
        let wd = working_dir.clone();
        let handle = tokio::spawn(async move {
            let executor = ToolExecutor::new(wd);
            let call = ToolCall {
                name: "write_file".to_string(),
                input: json!({
                    "path": format!("new_file_{}.txt", i),
                    "content": format!("parallel write content {}", i)
                }),
            };
            executor.execute(call).await
        });
        write_handles.push(handle);
    }

    // Wait for all writes to complete
    let mut write_success = 0;
    for handle in write_handles {
        let result = handle.await.expect("task should not panic");
        match result {
            Ok(ToolResult::Success(_)) => write_success += 1,
            Ok(ToolResult::Error(e)) => panic!("Concurrent write failed: {}", e),
            _ => {}
        }
    }

    assert_eq!(write_success, 5, "All parallel writes should succeed");

    // Verify all files exist with correct content
    for i in 0..5 {
        let content = std::fs::read_to_string(working_dir.join(format!("new_file_{}.txt", i)))
            .expect("file should exist");
        assert!(
            content.contains(&format!("parallel write content {}", i)),
            "File {} should have correct content",
            i
        );
    }
}

/// Test that parallel bash commands complete without issues.
#[tokio::test]
async fn test_parallel_bash_commands() {
    let ctx = TestContext::new();
    let working_dir = ctx.path().to_path_buf();

    // Launch parallel bash commands
    let mut handles = Vec::new();
    for i in 0..5 {
        let wd = working_dir.clone();
        let handle = tokio::spawn(async move {
            let executor = ToolExecutor::new(wd);
            let call = ToolCall {
                name: "bash".to_string(),
                input: json!({ "command": format!("echo 'command {}' && sleep 0.1", i) }),
            };
            executor.execute(call).await
        });
        handles.push(handle);
    }

    // Measure time - parallel should be faster than sequential
    let start = std::time::Instant::now();

    // Wait for all to complete
    let mut success_count = 0;
    for (i, handle) in handles.into_iter().enumerate() {
        let result = handle.await.expect("task should not panic");
        match result {
            Ok(ToolResult::Success(output)) => {
                assert!(
                    output.contains(&format!("command {}", i)),
                    "Output should contain 'command {}'",
                    i
                );
                success_count += 1;
            }
            Ok(ToolResult::Error(e)) => panic!("Concurrent bash command {} failed: {}", i, e),
            _ => {}
        }
    }

    let elapsed = start.elapsed();

    assert_eq!(
        success_count, 5,
        "All parallel bash commands should succeed"
    );

    // If truly parallel, 5 commands with 0.1s each should take ~0.1-0.3s
    // If sequential, would take ~0.5s or more
    assert!(
        elapsed.as_millis() < 500,
        "Parallel execution should be faster than sequential (~500ms), took {:?}",
        elapsed
    );
}

/// Test that parallel MCP tool calls would work (simulated with tool executor).
#[tokio::test]
async fn test_parallel_tool_calls() {
    let ctx = TestContext::new();

    // Create test files for glob/grep to search
    for i in 0..10 {
        ctx.create_file(
            &format!("search_{}.txt", i),
            &format!("searchable content number {} here", i),
        );
    }

    let working_dir = Arc::new(ctx.path().to_path_buf());

    // Launch parallel search operations (grep)
    let mut handles = Vec::new();
    for i in 0..5 {
        let wd = Arc::clone(&working_dir);
        let handle = tokio::spawn(async move {
            let executor = ToolExecutor::new(wd.as_ref().clone());
            let call = ToolCall {
                name: "grep".to_string(),
                input: json!({ "pattern": format!("number {}", i) }),
            };
            executor.execute(call).await
        });
        handles.push(handle);
    }

    // Also launch parallel glob operations
    for _ in 0..5 {
        let wd = Arc::clone(&working_dir);
        let handle = tokio::spawn(async move {
            let executor = ToolExecutor::new(wd.as_ref().clone());
            let call = ToolCall {
                name: "glob".to_string(),
                input: json!({ "pattern": "**/*.txt" }),
            };
            executor.execute(call).await
        });
        handles.push(handle);
    }

    // Wait for all to complete - should not deadlock or panic
    let mut completed = 0;
    for handle in handles {
        let result = handle.await;
        assert!(result.is_ok(), "Task should not panic");
        let tool_result = result.unwrap();
        assert!(tool_result.is_ok(), "Tool should not error");
        completed += 1;
    }

    assert_eq!(completed, 10, "All 10 parallel operations should complete");
}

// =============================================================================
// P1-1: Shell State Persistence Tests
// =============================================================================

/// Test that cd command updates the effective working directory.
#[tokio::test]
async fn test_shell_state_cd_updates_cwd() {
    use patina::tools::StatefulToolExecutor;

    let ctx = TestContext::new();
    // create_file creates parent directories automatically
    ctx.create_file("subdir/marker.txt", "subdir marker");

    let executor = StatefulToolExecutor::new(ctx.path());

    // First, cd into subdir
    let cd_call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "cd subdir" }),
    };
    executor.execute(cd_call).await.expect("cd should succeed");

    // Then list files - should see marker.txt since we're in subdir
    let ls_call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "ls" }),
    };
    let result = executor.execute(ls_call).await.expect("ls should succeed");

    match result {
        ToolResult::Success(output) => {
            assert!(
                output.contains("marker.txt"),
                "ls should show marker.txt since we're in subdir, got: {output}"
            );
        }
        ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        _ => panic!("unexpected result"),
    }
}

/// Test that export command persists environment variables.
#[tokio::test]
async fn test_shell_state_export_persists_env() {
    use patina::tools::StatefulToolExecutor;

    let ctx = TestContext::new();
    let executor = StatefulToolExecutor::new(ctx.path());

    // Set an environment variable
    let export_call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "export MY_TEST_VAR=test_value" }),
    };
    executor
        .execute(export_call)
        .await
        .expect("export should succeed");

    // Echo the variable
    let echo_call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "echo $MY_TEST_VAR" }),
    };
    let result = executor
        .execute(echo_call)
        .await
        .expect("echo should succeed");

    match result {
        ToolResult::Success(output) => {
            assert!(
                output.contains("test_value"),
                "exported variable should be accessible, got: {output}"
            );
        }
        ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        _ => panic!("unexpected result"),
    }
}

/// Test that cd with compound command (cd foo && ls) tracks directory.
#[tokio::test]
async fn test_shell_state_cd_compound_command() {
    use patina::tools::StatefulToolExecutor;

    let ctx = TestContext::new();
    // create_file creates parent directories automatically
    ctx.create_file("testdir/test.txt", "content");

    let executor = StatefulToolExecutor::new(ctx.path());

    // cd && ls in one command
    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "cd testdir && ls" }),
    };
    let result = executor
        .execute(call)
        .await
        .expect("cd && ls should succeed");

    // The output should contain test.txt
    match &result {
        ToolResult::Success(output) => {
            assert!(
                output.contains("test.txt"),
                "compound cd && ls should show test.txt, got: {output}"
            );
        }
        _ => panic!("unexpected result: {:?}", result),
    }

    // Subsequent command should still be in testdir
    let ls_call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "pwd" }),
    };
    let result = executor.execute(ls_call).await.expect("pwd should succeed");

    match result {
        ToolResult::Success(output) => {
            assert!(
                output.contains("testdir"),
                "pwd should show testdir as cwd, got: {output}"
            );
        }
        _ => panic!("unexpected result"),
    }
}
