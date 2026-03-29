//! Wiring tests for AgentRegistry and SandboxPolicy.

use std::path::PathBuf;
use std::sync::Arc;

use patina::agents::AgentRegistry;
use patina::tools::sandbox::policy::SandboxPolicy;
use patina::tools::sandbox::SandboxConfig;
use patina::tools::{ToolCall, ToolExecutor, ToolResult};

fn json_map(pairs: &[(&str, &str)]) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for (k, v) in pairs {
        map.insert(
            (*k).to_string(),
            serde_json::Value::String((*v).to_string()),
        );
    }
    serde_json::Value::Object(map)
}

#[tokio::test]
async fn test_wired_registry_routes() {
    let reg = Arc::new(AgentRegistry::new());
    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    reg.register("worker-1".to_string(), tx);

    let dir = tempfile::TempDir::new().unwrap();
    let executor = ToolExecutor::new(dir.path().to_path_buf());
    let executor = executor.with_agent_registry(Arc::clone(&reg));

    let input = json_map(&[("to", "worker-1"), ("message", "go")]);
    let call = ToolCall {
        name: "send_message".to_string(),
        input,
    };
    let result = executor.execute(call).await.unwrap();
    assert!(matches!(result, ToolResult::Success(_)));

    let received = rx.recv().await.unwrap();
    assert_eq!(received.content, "go");
}

#[tokio::test]
async fn test_no_registry_error() {
    let dir = tempfile::TempDir::new().unwrap();
    let executor = ToolExecutor::new(dir.path().to_path_buf());
    let input = json_map(&[("to", "x"), ("message", "y")]);
    let call = ToolCall {
        name: "send_message".to_string(),
        input,
    };
    let result = executor.execute(call).await.unwrap();
    assert!(matches!(result, ToolResult::Error(_)));
}

#[tokio::test]
async fn test_sandbox_allows_when_disabled() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("a.txt"), "data").unwrap();

    let policy = Arc::new(SandboxPolicy::new(SandboxConfig::default()));
    let executor = ToolExecutor::new(dir.path().to_path_buf());
    let executor = executor.with_sandbox_policy(policy);

    let call = ToolCall {
        name: "read_file".to_string(),
        input: json_map(&[("path", "a.txt")]),
    };
    let result = executor.execute(call).await.unwrap();
    assert!(matches!(result, ToolResult::Success(_)));
}

#[tokio::test]
async fn test_sandbox_denies_read() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("a.txt"), "x").unwrap();

    let cfg = SandboxConfig {
        enabled: true,
        allow_read: vec![PathBuf::from("/nope")],
        ..SandboxConfig::default()
    };
    let policy = Arc::new(SandboxPolicy::new(cfg));
    let executor = ToolExecutor::new(dir.path().to_path_buf());
    let executor = executor.with_sandbox_policy(policy);

    let call = ToolCall {
        name: "read_file".to_string(),
        input: json_map(&[("path", "a.txt")]),
    };
    let result = executor.execute(call).await.unwrap();
    assert!(matches!(result, ToolResult::Error(_)));
}
