//! Integration tests for wired tools: tool_search, task CRUD, and cron scheduling.

use super::common::TestContext;
use patina::tools::cron::CronStore;
use patina::tools::tasks::TaskStore;
use patina::tools::tool_search::ToolSearchRegistry;
use patina::tools::{ToolCall, ToolExecutor, ToolResult};
use serde_json::json;
use std::sync::Arc;

// =============================================================================
// ToolSearch through Executor
// =============================================================================

/// Verify that tool_search returns results when invoked through the executor.
#[tokio::test]
async fn test_tool_search_via_executor() {
    let ctx = TestContext::new();
    let mut registry = ToolSearchRegistry::new();
    registry.register_deferred(
        "my_fancy_tool".to_string(),
        "A fancy tool for testing".to_string(),
        json!({"type": "object", "properties": {"x": {"type": "string"}}}),
    );

    let executor = ToolExecutor::new(ctx.path()).with_tool_search_registry(Arc::new(registry));

    let call = ToolCall {
        name: "tool_search".to_string(),
        input: json!({ "query": "select:my_fancy_tool", "max_results": 5 }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execute should not error");
    match result {
        ToolResult::Success(output) => {
            assert!(
                output.contains("my_fancy_tool"),
                "should find registered tool, got: {output}"
            );
            assert!(
                output.contains("A fancy tool"),
                "should include description, got: {output}"
            );
        }
        ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        _ => panic!("unexpected result variant"),
    }
}

/// Verify that tool_search returns an error when no registry is configured.
#[tokio::test]
async fn test_tool_search_without_registry_returns_error() {
    let ctx = TestContext::new();
    let executor = ToolExecutor::new(ctx.path());

    let call = ToolCall {
        name: "tool_search".to_string(),
        input: json!({ "query": "select:anything" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("execute should not error");
    match result {
        ToolResult::Error(e) => {
            assert!(
                e.contains("not available"),
                "should indicate unavailability, got: {e}"
            );
        }
        _ => panic!("expected error when registry not configured"),
    }
}

// =============================================================================
// TaskStore through Executor
// =============================================================================

/// Verify task_create and task_get through the executor.
#[tokio::test]
async fn test_task_create_and_get_via_executor() {
    let ctx = TestContext::new();
    let store = Arc::new(TaskStore::new());
    let executor = ToolExecutor::new(ctx.path()).with_task_store(Arc::clone(&store));

    // Create a task
    let create_call = ToolCall {
        name: "task_create".to_string(),
        input: json!({
            "subject": "Write tests",
            "description": "Write integration tests for wired tools"
        }),
    };

    let result = executor
        .execute(create_call)
        .await
        .expect("execute should not error");
    let task_json: serde_json::Value = match result {
        ToolResult::Success(output) => serde_json::from_str(&output).expect("should be valid JSON"),
        ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        _ => panic!("unexpected result variant"),
    };
    let task_id = task_json["id"].as_str().expect("should have id");

    // Get the task
    let get_call = ToolCall {
        name: "task_get".to_string(),
        input: json!({ "task_id": task_id }),
    };

    let result = executor
        .execute(get_call)
        .await
        .expect("execute should not error");
    match result {
        ToolResult::Success(output) => {
            assert!(
                output.contains("Write tests"),
                "should contain task subject, got: {output}"
            );
            assert!(
                output.contains("is_blocked"),
                "should contain blocked status, got: {output}"
            );
        }
        ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        _ => panic!("unexpected result variant"),
    }
}

/// Verify task_list through the executor.
#[tokio::test]
async fn test_task_list_via_executor() {
    let ctx = TestContext::new();
    let store = Arc::new(TaskStore::new());
    let executor = ToolExecutor::new(ctx.path()).with_task_store(Arc::clone(&store));

    // Create two tasks
    let _ = store.create("Task A", "First task");
    let _ = store.create("Task B", "Second task");

    let list_call = ToolCall {
        name: "task_list".to_string(),
        input: json!({}),
    };

    let result = executor
        .execute(list_call)
        .await
        .expect("execute should not error");
    match result {
        ToolResult::Success(output) => {
            assert!(
                output.contains("Task A"),
                "should list first task, got: {output}"
            );
            assert!(
                output.contains("Task B"),
                "should list second task, got: {output}"
            );
        }
        ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        _ => panic!("unexpected result variant"),
    }
}

/// Verify task_update through the executor.
#[tokio::test]
async fn test_task_update_via_executor() {
    let ctx = TestContext::new();
    let store = Arc::new(TaskStore::new());
    let executor = ToolExecutor::new(ctx.path()).with_task_store(Arc::clone(&store));

    let task = store.create("Original", "Original description");

    let update_call = ToolCall {
        name: "task_update".to_string(),
        input: json!({
            "task_id": task.id,
            "status": "in_progress",
            "subject": "Updated Subject"
        }),
    };

    let result = executor
        .execute(update_call)
        .await
        .expect("execute should not error");
    match result {
        ToolResult::Success(output) => {
            assert!(
                output.contains("Updated Subject"),
                "should contain updated subject, got: {output}"
            );
            assert!(
                output.contains("in_progress"),
                "should contain updated status, got: {output}"
            );
        }
        ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        _ => panic!("unexpected result variant"),
    }
}

// =============================================================================
// CronStore through Executor
// =============================================================================

/// Verify cron_create and cron_list through the executor.
#[tokio::test]
async fn test_cron_create_and_list_via_executor() {
    let ctx = TestContext::new();
    let cron_path = ctx.path().join("cron.json");
    let store = Arc::new(CronStore::new(cron_path));
    let executor = ToolExecutor::new(ctx.path()).with_cron_store(Arc::clone(&store));

    // Create a schedule
    let create_call = ToolCall {
        name: "cron_create".to_string(),
        input: json!({
            "expression": "5m",
            "prompt": "Run health check"
        }),
    };

    let result = executor
        .execute(create_call)
        .await
        .expect("execute should not error");
    match &result {
        ToolResult::Success(output) => {
            assert!(
                output.contains("Run health check"),
                "should contain prompt, got: {output}"
            );
            assert!(
                output.contains("5m"),
                "should contain expression, got: {output}"
            );
        }
        ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        _ => panic!("unexpected result variant"),
    }

    // List schedules
    let list_call = ToolCall {
        name: "cron_list".to_string(),
        input: json!({}),
    };

    let result = executor
        .execute(list_call)
        .await
        .expect("execute should not error");
    match result {
        ToolResult::Success(output) => {
            assert!(
                output.contains("Run health check"),
                "should list created schedule, got: {output}"
            );
        }
        ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        _ => panic!("unexpected result variant"),
    }
}

/// Verify cron_delete through the executor.
#[tokio::test]
async fn test_cron_delete_via_executor() {
    let ctx = TestContext::new();
    let cron_path = ctx.path().join("cron.json");
    let store = Arc::new(CronStore::new(cron_path));
    let executor = ToolExecutor::new(ctx.path()).with_cron_store(Arc::clone(&store));

    // Create a schedule first
    let schedule = store
        .create("1h", "Hourly task")
        .expect("create should work");

    // Delete it
    let delete_call = ToolCall {
        name: "cron_delete".to_string(),
        input: json!({ "id": schedule.id }),
    };

    let result = executor
        .execute(delete_call)
        .await
        .expect("execute should not error");
    match result {
        ToolResult::Success(output) => {
            assert!(
                output.contains("deleted successfully"),
                "should confirm deletion, got: {output}"
            );
        }
        ToolResult::Error(e) => panic!("expected success, got error: {e}"),
        _ => panic!("unexpected result variant"),
    }

    // Verify it's gone
    let items = store.list().expect("list should work");
    assert!(items.is_empty(), "schedule should be deleted");
}
