//! File read/write/edit/list_files tests.

use super::common::TestContext;
use patina::tools::{ToolCall, ToolExecutionPolicy, ToolExecutor, ToolResult};
use serde_json::json;

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
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
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
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
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
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
    }
}

/// Test that read_file handles large files gracefully.
#[tokio::test]
async fn test_read_file_large_file() {
    let ctx = TestContext::new();

    // Create a 1MB file
    let large_content = "x".repeat(1024 * 1024);
    ctx.create_file("large_file.txt", &large_content);

    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "read_file".to_string(),
        input: json!({ "path": "large_file.txt" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Success(content) => {
            assert_eq!(
                content.len(),
                1024 * 1024,
                "should read full 1MB file content"
            );
        }
        ToolResult::Error(e) => panic!("expected success for large file, got error: {e}"),
        ToolResult::Cancelled => panic!("expected success, got cancelled"),
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
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
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
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
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
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
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
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
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
    }
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

    // Content larger than the limit
    let large_content = "x".repeat(200);

    let call = ToolCall {
        name: "write_file".to_string(),
        input: json!({ "path": "too_large.txt", "content": large_content }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execution should not error");

    match result {
        ToolResult::Error(e) => {
            assert!(
                e.contains("exceeds limit") || e.contains("size"),
                "error should mention size limit, got: {e}"
            );
        }
        ToolResult::Success(s) => {
            panic!("write should fail due to size limit, got success: {s}")
        }
        ToolResult::Cancelled => panic!("expected error, got cancelled"),
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
    }

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
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
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
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
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
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
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
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
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
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
    }
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

    // Attempt to list the external directory via path traversal
    let call = ToolCall {
        name: "list_files".to_string(),
        input: json!({ "path": "../external_test_dir_traversal" }),
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
                    || e.contains("Absolute paths are not allowed"),
                "error should mention path traversal, got: {e}"
            );
        }
        ToolResult::Success(s) => {
            panic!("list_files should block path traversal, but listed contents: {s}")
        }
        ToolResult::Cancelled => panic!("expected error, got cancelled"),
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
    }
}

/// Test that list_files blocks absolute paths.
#[tokio::test]
async fn test_list_files_blocks_absolute_path() {
    let ctx = TestContext::new();
    let executor = ToolExecutor::new(ctx.path());

    // Attempt to list an absolute path outside working directory
    let call = ToolCall {
        name: "list_files".to_string(),
        input: json!({ "path": "/tmp" }),
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
                    || e.contains("Absolute paths are not allowed")
                    || e.contains("absolute"),
                "error should mention path traversal or absolute path, got: {e}"
            );
        }
        ToolResult::Success(s) => {
            panic!("list_files should block absolute paths, but listed contents: {s}")
        }
        ToolResult::Cancelled => panic!("expected error, got cancelled"),
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
    }
}

/// Test that list_files handles nonexistent directory gracefully.
#[tokio::test]
async fn test_list_files_nonexistent_directory() {
    let ctx = TestContext::new();
    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "list_files".to_string(),
        input: json!({ "path": "nonexistent_dir" }),
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
                    || e.contains("canonicalize")
                    || e.contains("Failed"),
                "error should indicate directory not found, got: {e}"
            );
        }
        ToolResult::Success(s) => {
            panic!("list_files should fail for nonexistent directory, got success: {s}")
        }
        ToolResult::Cancelled => panic!("expected error, got cancelled"),
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
    }
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

    // Attempt to escape via subdir/../../
    let call = ToolCall {
        name: "list_files".to_string(),
        input: json!({ "path": "subdir/../../external_test_dir_escape" }),
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
                    || e.contains("Absolute paths are not allowed"),
                "error should mention path traversal, got: {e}"
            );
        }
        ToolResult::Success(s) => {
            panic!("list_files should block parent escape, but listed contents: {s}")
        }
        ToolResult::Cancelled => panic!("expected error, got cancelled"),
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
    }
}
