//! Hooked executor tests.

use super::common::TestContext;
use patina::hooks::HookManager;
use patina::tools::{ToolCall, ToolResult};
use serde_json::json;

// =============================================================================
// Tool Hooks Integration Tests (4.2.4)
// =============================================================================

/// Test that HookedToolExecutor fires PreToolUse hook before execution.
#[cfg(unix)]
#[tokio::test]
async fn test_hooked_executor_fires_pre_tool_use() {
    let ctx = TestContext::new();
    let mut manager = HookManager::new("test-tool-session".to_string());

    // Register a hook that writes to a marker file to prove it ran
    let marker_path = ctx.path().join("hook_marker.txt");
    manager.register_tool_hook(
        patina::hooks::HookEvent::PreToolUse,
        None, // No matcher - runs for all tools
        &format!("echo 'pre-tool executed' > {:?} && exit 0", marker_path),
    );

    let executor = patina::tools::HookedToolExecutor::new(ctx.path(), manager);

    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "echo hello" }),
    };

    let result = executor.execute(call).await.expect("should not error");

    // Tool should succeed
    assert!(matches!(result, ToolResult::Success(_)));

    // Hook should have created marker file
    assert!(
        marker_path.exists(),
        "PreToolUse hook should have created marker file"
    );
}

/// Test that HookedToolExecutor fires PostToolUse hook after successful execution.
#[cfg(unix)]
#[tokio::test]
async fn test_hooked_executor_fires_post_tool_use() {
    let ctx = TestContext::new();
    let mut manager = HookManager::new("test-post-tool".to_string());

    let marker_path = ctx.path().join("post_tool_marker.txt");
    manager.register_tool_hook(
        patina::hooks::HookEvent::PostToolUse,
        None,
        &format!("echo 'post-tool executed' > {:?} && exit 0", marker_path),
    );

    let executor = patina::tools::HookedToolExecutor::new(ctx.path(), manager);

    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "echo success" }),
    };

    let result = executor.execute(call).await.expect("should not error");

    assert!(matches!(result, ToolResult::Success(_)));
    assert!(
        marker_path.exists(),
        "PostToolUse hook should have created marker file"
    );
}

/// Test that HookedToolExecutor fires PostToolUseFailure hook after failed execution.
#[cfg(unix)]
#[tokio::test]
async fn test_hooked_executor_fires_post_tool_use_failure() {
    let ctx = TestContext::new();
    let mut manager = HookManager::new("test-failure-hook".to_string());

    let marker_path = ctx.path().join("failure_marker.txt");
    manager.register_tool_hook(
        patina::hooks::HookEvent::PostToolUseFailure,
        None,
        &format!("echo 'failure hook executed' > {:?} && exit 0", marker_path),
    );

    let executor = patina::tools::HookedToolExecutor::new(ctx.path(), manager);

    // Command that will fail
    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "exit 1" }),
    };

    let result = executor.execute(call).await.expect("should not error");

    assert!(matches!(result, ToolResult::Error(_)));
    assert!(
        marker_path.exists(),
        "PostToolUseFailure hook should have created marker file"
    );
}

/// Test that PreToolUse hook can block tool execution.
#[tokio::test]
async fn test_hooked_executor_pre_tool_use_blocks() {
    let ctx = TestContext::new();
    let mut manager = HookManager::new("test-block-tool".to_string());

    // Hook that blocks with exit code 2
    manager.register_tool_hook(
        patina::hooks::HookEvent::PreToolUse,
        Some("bash"),
        "echo 'Blocked: bash not allowed' && exit 2",
    );

    let executor = patina::tools::HookedToolExecutor::new(ctx.path(), manager);

    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "echo should not run" }),
    };

    let result = executor.execute(call).await.expect("should not error");

    match result {
        ToolResult::Cancelled => {
            // Expected - hook blocked execution
        }
        ToolResult::Error(e) => {
            assert!(
                e.contains("blocked") || e.contains("hook"),
                "error should indicate hook blocked execution, got: {e}"
            );
        }
        ToolResult::Success(s) => {
            panic!("tool should have been blocked by hook, got success: {s}")
        }
        ToolResult::NeedsPermission(_) => {
            panic!("unexpected needs permission")
        }
    }
}

/// Test that matcher patterns filter which tools hooks apply to.
#[cfg(unix)]
#[tokio::test]
async fn test_hooked_executor_matcher_filters_tools() {
    let ctx = TestContext::new();
    let mut manager = HookManager::new("test-matcher-filter".to_string());

    // Hook that only matches "read_file" tool
    let marker_path = ctx.path().join("read_marker.txt");
    manager.register_tool_hook(
        patina::hooks::HookEvent::PreToolUse,
        Some("read_file"),
        &format!("echo 'read hook ran' > {:?} && exit 0", marker_path),
    );

    let executor = patina::tools::HookedToolExecutor::new(ctx.path(), manager);

    // Execute bash - hook should NOT run
    let bash_call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "echo hello" }),
    };
    let _ = executor.execute(bash_call).await.expect("should not error");

    assert!(
        !marker_path.exists(),
        "hook should not have run for bash tool"
    );

    // Execute read_file - hook SHOULD run
    ctx.create_file("test.txt", "content");
    let read_call = ToolCall {
        name: "read_file".to_string(),
        input: json!({ "path": "test.txt" }),
    };
    let _ = executor.execute(read_call).await.expect("should not error");

    assert!(
        marker_path.exists(),
        "hook should have run for read_file tool"
    );
}

/// Test that PreToolUse hook receives tool context.
#[cfg(unix)]
#[tokio::test]
async fn test_hooked_executor_pre_tool_use_receives_context() {
    let ctx = TestContext::new();
    let mut manager = HookManager::new("test-tool-context".to_string());

    // Hook that checks for tool_name in context and blocks if found
    manager.register_tool_hook(
        patina::hooks::HookEvent::PreToolUse,
        None,
        r#"input=$(cat); echo "$input" | grep -q '"tool_name":"bash"' && exit 2 || exit 0"#,
    );

    let executor = patina::tools::HookedToolExecutor::new(ctx.path(), manager);

    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "echo test" }),
    };

    let result = executor.execute(call).await.expect("should not error");

    // Hook should have blocked because tool_name was in context
    assert!(
        matches!(result, ToolResult::Cancelled | ToolResult::Error(_)),
        "hook should block when context contains tool_name"
    );
}

/// Test that PostToolUse hook receives tool response.
#[cfg(unix)]
#[tokio::test]
async fn test_hooked_executor_post_tool_use_receives_response() {
    let ctx = TestContext::new();
    let mut manager = HookManager::new("test-response-context".to_string());

    // Hook that checks for tool_response in context
    let marker_path = ctx.path().join("response_marker.txt");
    manager.register_tool_hook(
        patina::hooks::HookEvent::PostToolUse,
        None,
        &format!(
            r#"input=$(cat); echo "$input" | grep -q '"tool_response"' && echo 'found response' > {:?} && exit 0 || exit 1"#,
            marker_path
        ),
    );

    let executor = patina::tools::HookedToolExecutor::new(ctx.path(), manager);

    let call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "echo output_text" }),
    };

    let _ = executor.execute(call).await.expect("should not error");

    // If hook found tool_response, marker file should exist
    assert!(
        marker_path.exists(),
        "PostToolUse hook should receive tool_response in context"
    );
}

/// Test that HookedToolExecutor exposes shell state via shell_state().
#[tokio::test]
async fn test_hooked_executor_exposes_shell_state() {
    use patina::tools::HookedToolExecutor;

    let ctx = TestContext::new();
    ctx.create_file("mydir/test.txt", "content");

    let hooks = HookManager::new("test-session".to_string());
    let executor = HookedToolExecutor::new(ctx.path(), hooks);

    // Initial shell state should have cwd = ctx.path()
    {
        let state = executor.shell_state();
        assert_eq!(
            state.cwd().canonicalize().unwrap(),
            ctx.path().canonicalize().unwrap()
        );
        assert!(state.env().is_empty());
    }

    // cd into mydir
    let cd_call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "cd mydir" }),
    };
    executor.execute(cd_call).await.expect("cd should succeed");

    // Shell state should reflect new cwd
    {
        let state = executor.shell_state();
        assert!(
            state.cwd().ends_with("mydir"),
            "cwd should end with mydir, got: {}",
            state.cwd().display()
        );
    }
}

/// Test that HookedToolExecutor persists environment variables via shell state.
#[tokio::test]
async fn test_hooked_executor_shell_state_env() {
    use patina::tools::HookedToolExecutor;

    let ctx = TestContext::new();
    let hooks = HookManager::new("test-session".to_string());
    let executor = HookedToolExecutor::new(ctx.path(), hooks);

    // Export an environment variable
    let export_call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "export HOOKED_TEST=hello" }),
    };
    executor
        .execute(export_call)
        .await
        .expect("export should succeed");

    // Shell state should have the env var
    {
        let state = executor.shell_state();
        assert_eq!(
            state.env().get("HOOKED_TEST"),
            Some(&"hello".to_string()),
            "env should contain HOOKED_TEST=hello"
        );
    }

    // Echo should use the persisted env var
    let echo_call = ToolCall {
        name: "bash".to_string(),
        input: json!({ "command": "echo $HOOKED_TEST" }),
    };
    let result = executor
        .execute(echo_call)
        .await
        .expect("echo should succeed");

    match result {
        ToolResult::Success(output) => {
            assert!(
                output.contains("hello"),
                "expected 'hello' in output, got: {output}"
            );
        }
        _ => panic!("expected success"),
    }
}
