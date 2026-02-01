//! Tests for async (non-blocking) tool execution.
//!
//! These tests verify that tool execution doesn't block the UI event loop.

use patina::app::state::AppState;
use patina::types::{ConversationEntry, ToolResultBlock, ToolUseBlock};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::timeout;

/// Helper to create a new AppState for testing.
fn new_state() -> AppState {
    AppState::new(PathBuf::from("/tmp/test"), false)
}

/// Helper to create a mock tool use block.
fn mock_tool_use(name: &str, input: &str) -> ToolUseBlock {
    ToolUseBlock {
        id: format!("tool_{}", name),
        name: name.to_string(),
        input: serde_json::from_str(input).unwrap_or_default(),
    }
}

/// Helper to create a mock tool result.
fn mock_tool_result(tool_use_id: &str, content: &str, is_error: bool) -> ToolResultBlock {
    ToolResultBlock {
        tool_use_id: tool_use_id.to_string(),
        content: content.to_string(),
        is_error,
    }
}

// ============================================================================
// 5.1.1 Async Tool Execution Tests
// ============================================================================

/// Tests that tool execution channel can be set on AppState.
#[test]
fn test_tool_result_channel_setup() {
    let mut state = new_state();

    // Create a channel for tool results
    let (tx, rx) = mpsc::unbounded_channel::<(String, ToolResultBlock)>();

    // Should be able to set the receiver on state
    state.set_tool_result_rx(rx);

    // Verify channel is set
    assert!(state.has_tool_result_rx());

    // Send a result through the channel
    let result = mock_tool_result("tool_bash", "/home/user", false);
    tx.send(("tool_bash".to_string(), result)).unwrap();

    // Should be able to receive (non-blocking check)
    assert!(state.try_recv_tool_result().is_some());
}

/// Tests that tool execution returns immediately when spawned in background.
#[tokio::test]
async fn test_tool_execution_returns_immediately() {
    let mut state = new_state();

    // Add a pending tool
    state.add_pending_tool(mock_tool_use("bash", r#"{"command": "sleep 5"}"#));

    // Start tool execution in background
    let start = std::time::Instant::now();
    let handle = state.spawn_tool_execution();

    // Should return immediately (not wait for 5 second sleep)
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_millis(100),
        "spawn_tool_execution should return immediately, took {:?}",
        elapsed
    );

    // Handle should be valid
    assert!(handle.is_some());
}

/// Tests that tool results are streamed back through the channel.
#[tokio::test]
async fn test_tool_results_streamed() {
    let mut state = new_state();

    // Set up channel
    let (tx, rx) = mpsc::unbounded_channel::<(String, ToolResultBlock)>();
    state.set_tool_result_rx(rx);

    // Simulate tool execution sending results
    let result1 = mock_tool_result("tool_1", "result 1", false);
    let result2 = mock_tool_result("tool_2", "result 2", false);

    tx.send(("tool_1".to_string(), result1.clone())).unwrap();
    tx.send(("tool_2".to_string(), result2.clone())).unwrap();

    // Results should be receivable
    let received1 = state.try_recv_tool_result();
    assert!(received1.is_some());
    let (id1, r1) = received1.unwrap();
    assert_eq!(id1, "tool_1");
    assert_eq!(r1.content, "result 1");

    let received2 = state.try_recv_tool_result();
    assert!(received2.is_some());
    let (id2, r2) = received2.unwrap();
    assert_eq!(id2, "tool_2");
    assert_eq!(r2.content, "result 2");

    // No more results
    assert!(state.try_recv_tool_result().is_none());
}

/// Tests that UI remains responsive during tool execution.
/// This simulates the event loop being able to process events while tools run.
#[tokio::test]
async fn test_ui_responsive_during_tools() {
    let mut state = new_state();

    // Set up channel
    let (tx, rx) = mpsc::unbounded_channel::<(String, ToolResultBlock)>();
    state.set_tool_result_rx(rx);

    // Spawn a task that simulates slow tool execution
    let tx_clone = tx.clone();
    let tool_task = tokio::spawn(async move {
        // Simulate slow tool (100ms)
        tokio::time::sleep(Duration::from_millis(100)).await;
        let result = mock_tool_result("slow_tool", "done", false);
        tx_clone.send(("slow_tool".to_string(), result)).ok();
    });

    // Meanwhile, UI should be able to process events
    // Simulate several "tick" events that would be blocked if tools were synchronous
    let mut ticks = 0;
    let start = std::time::Instant::now();

    while start.elapsed() < Duration::from_millis(150) {
        // This simulates the event loop continuing to run
        state.tick_throbber();
        ticks += 1;

        // Check for tool results without blocking
        if let Some((id, result)) = state.try_recv_tool_result() {
            state.record_tool_result(&id, result);
        }

        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Should have processed multiple ticks while tool was running
    assert!(
        ticks >= 10,
        "Should have processed at least 10 ticks, got {}",
        ticks
    );

    // Tool should have completed
    tool_task.await.unwrap();
}

/// Tests that tool execution state is tracked correctly.
#[test]
fn test_tool_execution_state_tracking() {
    let mut state = new_state();

    // Initially no tools executing
    assert!(!state.has_executing_tools());

    // Add pending tool
    state.add_pending_tool(mock_tool_use("bash", r#"{"command": "pwd"}"#));

    // Mark tool as executing
    state.mark_tool_executing("tool_bash");
    assert!(state.has_executing_tools());

    // Record result
    let result = mock_tool_result("tool_bash", "/home/user", false);
    state.record_tool_result("tool_bash", result);

    // Tool should now be complete
    assert!(!state.has_executing_tools());
    assert!(state.all_tools_complete());
}

/// Tests that tool progress is visible in timeline during execution.
#[test]
fn test_tool_progress_in_timeline() {
    let mut state = new_state();

    // Add an assistant message that will use a tool
    state
        .timeline_mut()
        .push_assistant_message("Let me check that.");

    // Add tool execution (in progress)
    state.add_tool_to_timeline_executing("bash", "pwd");

    // Timeline should show the tool as executing (no output yet)
    let entries: Vec<_> = state.timeline().iter().collect();
    assert_eq!(entries.len(), 2);

    match &entries[1] {
        ConversationEntry::ToolExecution { name, output, .. } => {
            assert_eq!(name, "bash");
            assert!(output.is_none(), "Executing tool should have no output yet");
        }
        _ => panic!("Expected ToolExecution entry"),
    }

    // Complete the tool
    state.update_tool_in_timeline("bash", Some("/home/user".to_string()), false);

    // Now timeline should show result
    let entries: Vec<_> = state.timeline().iter().collect();
    match &entries[1] {
        ConversationEntry::ToolExecution { output, .. } => {
            assert_eq!(output.as_deref(), Some("/home/user"));
        }
        _ => panic!("Expected ToolExecution entry"),
    }
}

/// Tests that multiple tools can execute and report results independently.
#[tokio::test]
async fn test_multiple_tools_independent_results() {
    let mut state = new_state();

    // Set up channel
    let (tx, rx) = mpsc::unbounded_channel::<(String, ToolResultBlock)>();
    state.set_tool_result_rx(rx);

    // Spawn multiple tool executions
    let tx1 = tx.clone();
    let tx2 = tx.clone();

    let task1 = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let result = mock_tool_result("tool_1", "result 1", false);
        tx1.send(("tool_1".to_string(), result)).ok();
    });

    let task2 = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(30)).await;
        let result = mock_tool_result("tool_2", "result 2", false);
        tx2.send(("tool_2".to_string(), result)).ok();
    });

    // Collect results (tool_2 should arrive first due to shorter delay)
    let mut results = Vec::new();

    let collect_timeout = timeout(Duration::from_millis(200), async {
        while results.len() < 2 {
            if let Some(result) = state.try_recv_tool_result() {
                results.push(result);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });

    collect_timeout
        .await
        .expect("Should receive both results within timeout");

    task1.await.unwrap();
    task2.await.unwrap();

    // Both results received
    assert_eq!(results.len(), 2);

    // Order should be tool_2 first (faster)
    assert_eq!(results[0].0, "tool_2");
    assert_eq!(results[1].0, "tool_1");
}

/// Tests error handling when tool execution fails.
#[test]
fn test_tool_error_handling() {
    let mut state = new_state();

    // Add tool to timeline as executing
    state.add_tool_to_timeline_executing("bash", "invalid_command");

    // Record error result
    let error_result = mock_tool_result("tool_bash", "command not found", true);
    state.record_tool_result("tool_bash", error_result);

    // Timeline should show error
    let entries: Vec<_> = state.timeline().iter().collect();
    match &entries[0] {
        ConversationEntry::ToolExecution {
            is_error, output, ..
        } => {
            assert!(is_error, "Tool should be marked as error");
            assert_eq!(output.as_deref(), Some("command not found"));
        }
        _ => panic!("Expected ToolExecution entry"),
    }
}
