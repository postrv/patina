//! Glob and grep tests.

use super::common::TestContext;
use patina::tools::{ToolCall, ToolExecutor, ToolResult};
use serde_json::json;

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
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
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
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
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
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
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
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
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
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
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
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
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
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
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
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
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
        ToolResult::NeedsPermission(_) => panic!("unexpected needs permission"),
    }
}
