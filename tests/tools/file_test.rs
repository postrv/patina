//! File read/write/edit/list_files tests.

use super::common::TestContext;
use patina::tools::{ToolCall, ToolExecutionPolicy, ToolExecutor, ToolResult};
use serde_json::json;

// =============================================================================
// Test Helpers
// =============================================================================

/// Executes a tool call and returns the result, panicking on execution failure.
async fn run_tool(executor: &ToolExecutor, name: &str, input: serde_json::Value) -> ToolResult {
    let call = ToolCall {
        name: name.to_string(),
        input,
    };
    executor
        .execute(call)
        .await
        .expect("execution should not error")
}

/// Asserts the result is `ToolResult::Error` and the message contains at least one keyword.
fn assert_tool_error(result: ToolResult, expected_keywords: &[&str]) {
    match result {
        ToolResult::Error(e) => {
            assert!(
                expected_keywords.iter().any(|kw| e.contains(kw)),
                "error should contain one of {expected_keywords:?}, got: {e}"
            );
        }
        ToolResult::Success(s) => panic!("expected error, got success: {s}"),
        ToolResult::Cancelled => panic!("expected error, got cancelled"),
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
    }
}

/// Unwraps a `ToolResult::Success`, panicking on any other variant.
fn unwrap_success(result: ToolResult) -> String {
    match result {
        ToolResult::Success(s) => s,
        ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        ToolResult::Cancelled => panic!("expected success, got cancelled"),
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
    }
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

    let content =
        unwrap_success(run_tool(&executor, "read_file", json!({ "path": "readable.txt" })).await);

    // read_file returns cat -n style line-numbered output
    assert!(
        content.contains("file content here"),
        "should contain file content, got: {content}"
    );
    assert!(
        content.contains("1\t"),
        "should have line number prefix, got: {content}"
    );
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
    let result = run_tool(
        &executor,
        "read_file",
        json!({ "path": "../outside_workdir.txt" }),
    )
    .await;
    assert_tool_error(result, &["path traversal", "outside working directory"]);
}

/// Test that read_file returns appropriate error for nonexistent files.
#[tokio::test]
async fn test_file_read_nonexistent() {
    let ctx = TestContext::new();
    let executor = ToolExecutor::new(ctx.path());

    let result = run_tool(
        &executor,
        "read_file",
        json!({ "path": "does_not_exist.txt" }),
    )
    .await;
    assert_tool_error(result, &["not found", "No such file", "Failed to read"]);
}

/// Test that read_file handles large files gracefully.
#[tokio::test]
async fn test_read_file_large_file() {
    let ctx = TestContext::new();
    let large_content = "x".repeat(1024 * 1024);
    ctx.create_file("large_file.txt", &large_content);
    let executor = ToolExecutor::new(ctx.path());

    let content =
        unwrap_success(run_tool(&executor, "read_file", json!({ "path": "large_file.txt" })).await);

    // read_file returns cat -n style line-numbered output
    // The 1MB file is one long line, so output is "     1\t" + 1MB + "\n"
    assert!(
        content.len() > 1024 * 1024,
        "should read full file content plus line number prefix, got len: {}",
        content.len()
    );
    assert!(content.contains("xxxx"), "should contain file data");
}

// =============================================================================
// File Write Tests (2.2.2)
// =============================================================================

/// Test that write_file creates a file in the working directory.
#[tokio::test]
async fn test_file_write_creates_file() {
    let ctx = TestContext::new();
    let executor = ToolExecutor::new(ctx.path());

    let msg = unwrap_success(
        run_tool(
            &executor,
            "write_file",
            json!({ "path": "new_file.txt", "content": "written content" }),
        )
        .await,
    );

    assert!(
        msg.contains("Wrote") && msg.contains("bytes"),
        "should report bytes written, got: {msg}"
    );
    let written_path = ctx.path().join("new_file.txt");
    let content = std::fs::read_to_string(&written_path).expect("file should exist");
    assert_eq!(content, "written content");
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
    let result = run_tool(
        &executor,
        "write_file",
        json!({ "path": "../should_not_be_created.txt", "content": "malicious content" }),
    )
    .await;
    assert_tool_error(result, &["path traversal", "outside working directory"]);

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

    let result = run_tool(
        &executor,
        "write_file",
        json!({ "path": "/etc/test_file", "content": "should not write" }),
    )
    .await;
    assert_tool_error(
        result,
        &["protected", "outside working directory", "absolute"],
    );
}

/// Test that write_file creates a backup when overwriting existing files.
#[tokio::test]
async fn test_file_write_creates_backup() {
    let ctx = TestContext::new();
    ctx.create_file("existing.txt", "original content");
    let executor = ToolExecutor::new(ctx.path());

    unwrap_success(
        run_tool(
            &executor,
            "write_file",
            json!({ "path": "existing.txt", "content": "new content" }),
        )
        .await,
    );

    // Check that original content was backed up
    let backup_dir = ctx.path().join(".rct_backups");
    assert!(
        backup_dir.exists(),
        "backup directory should be created at {backup_dir:?}"
    );

    let backups: Vec<_> = std::fs::read_dir(&backup_dir)
        .expect("should read backup dir")
        .filter_map(|e| e.ok())
        .collect();
    assert!(!backups.is_empty(), "at least one backup file should exist");

    let backup_content = std::fs::read_to_string(backups[0].path()).expect("should read backup");
    assert!(
        backup_content.contains("original content"),
        "backup should contain original content, got: {backup_content}"
    );
}

/// Test that write_file enforces max file size limit.
#[tokio::test]
async fn test_write_file_exceeds_size_limit() {
    let ctx = TestContext::new();
    let policy = ToolExecutionPolicy {
        max_file_size: 100, // Very small limit for testing
        ..Default::default()
    };
    let executor = ToolExecutor::new(ctx.path()).with_policy(policy);

    let large_content = "x".repeat(200);
    let result = run_tool(
        &executor,
        "write_file",
        json!({ "path": "too_large.txt", "content": large_content }),
    )
    .await;
    assert_tool_error(result, &["exceeds limit", "size"]);

    // Verify file was NOT created
    assert!(
        !ctx.path().join("too_large.txt").exists(),
        "file should not have been created"
    );
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

    let output = unwrap_success(
        run_tool(
            &executor,
            "edit",
            json!({
                "path": "target.txt",
                "old_string": "world",
                "new_string": "planet"
            }),
        )
        .await,
    );

    let content =
        std::fs::read_to_string(ctx.path().join("target.txt")).expect("file should exist");
    assert_eq!(
        content, "Hello planet, hello universe!",
        "should replace the matched string"
    );
    assert!(
        output.contains("replaced") || output.contains("edited") || output.contains("diff"),
        "output should indicate edit was made, got: {output}"
    );
}

/// Test that edit tool generates a diff output.
#[tokio::test]
async fn test_edit_generates_diff() {
    let ctx = TestContext::new();
    ctx.create_file("diff_target.txt", "line one\nline two\nline three\n");
    let executor = ToolExecutor::new(ctx.path());

    let output = unwrap_success(
        run_tool(
            &executor,
            "edit",
            json!({
                "path": "diff_target.txt",
                "old_string": "line two",
                "new_string": "line TWO modified"
            }),
        )
        .await,
    );

    assert!(
        output.contains("-") && output.contains("+")
            || output.contains("old") && output.contains("new")
            || output.contains("line two") && output.contains("line TWO modified"),
        "output should show diff, got: {output}"
    );
}

/// Test that edit tool requires a unique match.
#[tokio::test]
async fn test_edit_unique_match_required() {
    let ctx = TestContext::new();
    ctx.create_file("ambiguous.txt", "foo bar foo baz foo");
    let executor = ToolExecutor::new(ctx.path());

    let result = run_tool(
        &executor,
        "edit",
        json!({
            "path": "ambiguous.txt",
            "old_string": "foo",
            "new_string": "qux"
        }),
    )
    .await;
    assert_tool_error(result, &["unique", "multiple", "ambiguous", "3 matches"]);

    // Verify file was NOT modified
    let content =
        std::fs::read_to_string(ctx.path().join("ambiguous.txt")).expect("file should exist");
    assert_eq!(
        content, "foo bar foo baz foo",
        "file should not be modified when match is ambiguous"
    );
}

/// Test that edit tool handles nonexistent files.
#[tokio::test]
async fn test_edit_nonexistent_file() {
    let ctx = TestContext::new();
    let executor = ToolExecutor::new(ctx.path());

    let result = run_tool(
        &executor,
        "edit",
        json!({
            "path": "nonexistent.txt",
            "old_string": "foo",
            "new_string": "bar"
        }),
    )
    .await;
    assert_tool_error(result, &["not found", "No such file", "Failed"]);
}

/// Test that edit tool handles no match found.
#[tokio::test]
async fn test_edit_no_match() {
    let ctx = TestContext::new();
    ctx.create_file("no_match.txt", "hello world");
    let executor = ToolExecutor::new(ctx.path());

    let result = run_tool(
        &executor,
        "edit",
        json!({
            "path": "no_match.txt",
            "old_string": "xyz",
            "new_string": "abc"
        }),
    )
    .await;
    assert_tool_error(result, &["not found", "no match", "0 matches"]);
}

// =============================================================================
// list_files Path Traversal Security Tests (0.1.1)
// =============================================================================

/// Test that list_files blocks path traversal via `..` escape.
#[tokio::test]
async fn test_list_files_blocks_path_traversal() {
    let ctx = TestContext::new();
    let working_dir = ctx.path();

    // Create a file in the parent directory (outside working dir)
    let parent_dir = working_dir.parent().expect("temp dir should have parent");
    let external_dir = parent_dir.join("external_test_dir_traversal");
    std::fs::create_dir_all(&external_dir).expect("failed to create external test dir");
    std::fs::write(external_dir.join("secret.txt"), "secret content")
        .expect("failed to create test file");

    // Ensure cleanup on drop
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(external_dir.clone());

    let executor = ToolExecutor::new(working_dir);
    let result = run_tool(
        &executor,
        "list_files",
        json!({ "path": "../external_test_dir_traversal" }),
    )
    .await;
    assert_tool_error(
        result,
        &[
            "path traversal",
            "outside working directory",
            "Absolute paths are not allowed",
        ],
    );
}

/// Test that list_files blocks absolute paths.
#[tokio::test]
async fn test_list_files_blocks_absolute_path() {
    let ctx = TestContext::new();
    let executor = ToolExecutor::new(ctx.path());

    let result = run_tool(&executor, "list_files", json!({ "path": "/tmp" })).await;
    assert_tool_error(
        result,
        &[
            "path traversal",
            "outside working directory",
            "Absolute paths are not allowed",
            "absolute",
        ],
    );
}

/// Test that list_files handles nonexistent directory gracefully.
#[tokio::test]
async fn test_list_files_nonexistent_directory() {
    let ctx = TestContext::new();
    let executor = ToolExecutor::new(ctx.path());

    let result = run_tool(
        &executor,
        "list_files",
        json!({ "path": "nonexistent_dir" }),
    )
    .await;
    assert_tool_error(
        result,
        &["not found", "No such file", "canonicalize", "Failed"],
    );
}

/// Test that list_files blocks complex parent directory escapes.
#[tokio::test]
async fn test_list_files_blocks_parent_escape() {
    let ctx = TestContext::new();
    let working_dir = ctx.path();

    // Create a subdirectory to enable the escape pattern
    ctx.create_file("subdir/file.txt", "content");

    // Create a file in the parent directory (outside working dir)
    let parent_dir = working_dir.parent().expect("temp dir should have parent");
    let external_dir = parent_dir.join("external_test_dir_escape");
    std::fs::create_dir_all(&external_dir).expect("failed to create external test dir");
    std::fs::write(external_dir.join("secret.txt"), "secret content")
        .expect("failed to create test file");

    // Ensure cleanup on drop
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(external_dir.clone());

    let executor = ToolExecutor::new(working_dir);
    let result = run_tool(
        &executor,
        "list_files",
        json!({ "path": "subdir/../../external_test_dir_escape" }),
    )
    .await;
    assert_tool_error(
        result,
        &[
            "path traversal",
            "outside working directory",
            "Absolute paths are not allowed",
        ],
    );
}
