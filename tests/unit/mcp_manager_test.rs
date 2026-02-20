//! Tests for MCP Manager

use patina::mcp::config::McpServerEntry;
use patina::mcp::manager::{is_mcp_tool, namespace_tool_name, parse_namespaced_tool, McpManager};
use std::collections::HashMap;
use std::time::Duration;

// =============================================================================
// Namespace helper tests
// =============================================================================

#[test]
fn test_namespace_tool_name() {
    assert_eq!(namespace_tool_name("fs", "read"), "fs__read");
    assert_eq!(
        namespace_tool_name("narsil", "scan_security"),
        "narsil__scan_security"
    );
}

#[test]
fn test_parse_namespaced_tool() {
    assert_eq!(parse_namespaced_tool("fs__read"), Some(("fs", "read")));
    assert_eq!(parse_namespaced_tool("a__b__c"), Some(("a", "b__c")));
    assert_eq!(parse_namespaced_tool("bash"), None);
    assert_eq!(parse_namespaced_tool("read_file"), None);
}

#[test]
fn test_is_mcp_tool() {
    assert!(is_mcp_tool("fs__read"));
    assert!(!is_mcp_tool("bash"));
    assert!(!is_mcp_tool("read_file"));
}

// =============================================================================
// Manager lifecycle tests
// =============================================================================

#[tokio::test]
async fn test_manager_empty_configs_yields_empty() {
    let manager = McpManager::start_all(HashMap::new(), Duration::from_secs(1)).await;
    assert!(manager.is_empty());
    assert_eq!(manager.connected_count(), 0);
    assert_eq!(manager.tool_count(), 0);
    assert!(manager.tool_definitions().is_empty());
}

#[tokio::test]
async fn test_manager_startup_skips_disabled() {
    let mut configs = HashMap::new();
    configs.insert(
        "disabled".to_string(),
        McpServerEntry {
            command: "echo".to_string(),
            args: vec![],
            env: HashMap::new(),
            url: None,
            headers: None,
            transport_type: None,
            disabled: true,
        },
    );

    let manager = McpManager::start_all(configs, Duration::from_secs(1)).await;
    assert!(manager.is_empty());
}

#[tokio::test]
async fn test_manager_startup_invalid_command_marks_failed() {
    let mut configs = HashMap::new();
    configs.insert(
        "bad-server".to_string(),
        McpServerEntry {
            command: "/nonexistent/bin/server".to_string(),
            args: vec![],
            env: HashMap::new(),
            url: None,
            headers: None,
            transport_type: None,
            disabled: false,
        },
    );

    let manager = McpManager::start_all(configs, Duration::from_secs(2)).await;
    assert_eq!(manager.connected_count(), 0);

    let statuses = manager.server_statuses();
    assert_eq!(statuses.len(), 1);
    assert!(!statuses[0].1.is_connected());
}

#[tokio::test]
async fn test_manager_sse_not_yet_supported() {
    let mut configs = HashMap::new();
    configs.insert(
        "sse".to_string(),
        McpServerEntry {
            command: String::new(),
            args: vec![],
            env: HashMap::new(),
            url: Some("http://localhost:8080/sse".to_string()),
            headers: None,
            transport_type: None,
            disabled: false,
        },
    );

    let manager = McpManager::start_all(configs, Duration::from_secs(1)).await;
    assert_eq!(manager.connected_count(), 0);
}

// =============================================================================
// Tool routing tests
// =============================================================================

#[tokio::test]
async fn test_call_tool_no_double_underscore_returns_error() {
    let manager = McpManager::start_all(HashMap::new(), Duration::from_secs(1)).await;
    let result = manager
        .call_tool("bash", serde_json::json!({"command": "ls"}))
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_call_tool_unknown_namespace_returns_error() {
    let manager = McpManager::start_all(HashMap::new(), Duration::from_secs(1)).await;
    let result = manager
        .call_tool("nonexistent__tool", serde_json::json!({}))
        .await;
    assert!(result.is_err());
}

// =============================================================================
// Shutdown tests
// =============================================================================

#[tokio::test]
async fn test_shutdown_handles_empty_manager() {
    let mut manager = McpManager::start_all(HashMap::new(), Duration::from_secs(1)).await;
    manager.shutdown_all().await;
    // Should not panic
}

#[tokio::test]
async fn test_is_empty_after_no_startup() {
    let manager = McpManager::start_all(HashMap::new(), Duration::from_secs(1)).await;
    assert!(manager.is_empty());
}
