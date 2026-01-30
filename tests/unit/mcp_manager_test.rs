//! Tests for MCP Manager

use rct::mcp::{McpManager, McpServerConfig, McpTool, McpTransport};
use std::collections::HashMap;

#[test]
fn test_mcp_manager_new() {
    let manager = McpManager::new();
    assert!(manager.get_tools().is_empty());
}

#[test]
fn test_mcp_manager_default() {
    let manager = McpManager::default();
    assert!(manager.get_tools().is_empty());
}

#[tokio::test]
async fn test_mcp_manager_initialize_empty() {
    let mut manager = McpManager::new();
    let configs = HashMap::new();
    let result = manager.initialize(configs).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_mcp_manager_initialize_disabled_server() {
    let mut manager = McpManager::new();
    let mut configs = HashMap::new();
    configs.insert(
        "test-server".to_string(),
        McpServerConfig {
            transport: McpTransport::Stdio {
                command: "echo".to_string(),
                args: vec![],
                env: HashMap::new(),
            },
            enabled: false,
        },
    );
    let result = manager.initialize(configs).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_mcp_manager_initialize_stdio() {
    let mut manager = McpManager::new();
    let mut configs = HashMap::new();
    configs.insert(
        "test-server".to_string(),
        McpServerConfig {
            transport: McpTransport::Stdio {
                command: "echo".to_string(),
                args: vec!["hello".to_string()],
                env: HashMap::new(),
            },
            enabled: true,
        },
    );
    let result = manager.initialize(configs).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_mcp_manager_initialize_sse() {
    let mut manager = McpManager::new();
    let mut configs = HashMap::new();
    configs.insert(
        "sse-server".to_string(),
        McpServerConfig {
            transport: McpTransport::Sse {
                url: "http://localhost:8080/sse".to_string(),
                headers: HashMap::new(),
            },
            enabled: true,
        },
    );
    let result = manager.initialize(configs).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_mcp_manager_initialize_http() {
    let mut manager = McpManager::new();
    let mut configs = HashMap::new();
    configs.insert(
        "http-server".to_string(),
        McpServerConfig {
            transport: McpTransport::Http {
                url: "http://localhost:8080/api".to_string(),
                headers: HashMap::new(),
            },
            enabled: true,
        },
    );
    let result = manager.initialize(configs).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_mcp_manager_call_tool() {
    let manager = McpManager::new();
    let result = manager
        .call_tool("test-tool", serde_json::json!({"input": "value"}))
        .await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), serde_json::json!({}));
}

#[test]
fn test_mcp_tool_serialization() {
    let tool = McpTool {
        name: "test-tool".to_string(),
        description: "A test tool".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "input": {"type": "string"}
            }
        }),
    };

    let json = serde_json::to_string(&tool).unwrap();
    assert!(json.contains("test-tool"));
    assert!(json.contains("A test tool"));

    let deserialized: McpTool = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.name, "test-tool");
}

#[test]
fn test_mcp_server_config_deserialization_stdio() {
    let json = r#"{
        "transport": {
            "type": "stdio",
            "command": "node",
            "args": ["server.js"],
            "env": {"NODE_ENV": "production"}
        },
        "enabled": true
    }"#;

    let config: McpServerConfig = serde_json::from_str(json).unwrap();
    assert!(config.enabled);
    match config.transport {
        McpTransport::Stdio { command, args, env } => {
            assert_eq!(command, "node");
            assert_eq!(args, vec!["server.js"]);
            assert_eq!(env.get("NODE_ENV"), Some(&"production".to_string()));
        }
        _ => panic!("Expected Stdio transport"),
    }
}

#[test]
fn test_mcp_server_config_deserialization_sse() {
    let json = r#"{
        "transport": {
            "type": "sse",
            "url": "http://localhost:8080/events",
            "headers": {"Authorization": "Bearer token"}
        },
        "enabled": false
    }"#;

    let config: McpServerConfig = serde_json::from_str(json).unwrap();
    assert!(!config.enabled);
    match config.transport {
        McpTransport::Sse { url, headers } => {
            assert_eq!(url, "http://localhost:8080/events");
            assert_eq!(
                headers.get("Authorization"),
                Some(&"Bearer token".to_string())
            );
        }
        _ => panic!("Expected Sse transport"),
    }
}

#[test]
fn test_mcp_server_config_deserialization_http() {
    let json = r#"{
        "transport": {
            "type": "http",
            "url": "http://localhost:8080/api",
            "headers": {}
        },
        "enabled": true
    }"#;

    let config: McpServerConfig = serde_json::from_str(json).unwrap();
    assert!(config.enabled);
    match config.transport {
        McpTransport::Http { url, headers } => {
            assert_eq!(url, "http://localhost:8080/api");
            assert!(headers.is_empty());
        }
        _ => panic!("Expected Http transport"),
    }
}

#[test]
fn test_mcp_server_config_defaults() {
    // Test that enabled defaults to false
    let json = r#"{
        "transport": {
            "type": "stdio",
            "command": "echo"
        }
    }"#;

    let config: McpServerConfig = serde_json::from_str(json).unwrap();
    assert!(!config.enabled);
    match config.transport {
        McpTransport::Stdio { command, args, env } => {
            assert_eq!(command, "echo");
            assert!(args.is_empty());
            assert!(env.is_empty());
        }
        _ => panic!("Expected Stdio transport"),
    }
}
