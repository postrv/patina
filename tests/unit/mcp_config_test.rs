//! Tests for MCP config file loading from the filesystem.

use patina::mcp::config::{load_project_config, McpConfigFile, McpServerEntry};
use std::fs;
use tempfile::TempDir;

#[test]
fn load_project_config_from_mcp_json() {
    let temp = TempDir::new().unwrap();
    let config_json = r#"{
        "mcpServers": {
            "test-server": {
                "command": "echo",
                "args": ["hello"]
            }
        }
    }"#;
    fs::write(temp.path().join(".mcp.json"), config_json).unwrap();

    let configs = load_project_config(temp.path()).unwrap();
    assert_eq!(configs.len(), 1);
    assert_eq!(configs["test-server"].command, "echo");
    assert_eq!(configs["test-server"].args, vec!["hello"]);
}

#[test]
fn load_missing_config_returns_empty() {
    let temp = TempDir::new().unwrap();
    let configs = load_project_config(temp.path()).unwrap();
    assert!(configs.is_empty());
}

#[test]
fn load_invalid_json_returns_error() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join(".mcp.json"), "not valid json{{{").unwrap();

    let result = load_project_config(temp.path());
    assert!(result.is_err());
}

#[test]
fn load_disabled_server_filtered_out() {
    let temp = TempDir::new().unwrap();
    let config_json = r#"{
        "mcpServers": {
            "enabled-server": {"command": "echo"},
            "disabled-server": {"command": "nope", "disabled": true}
        }
    }"#;
    fs::write(temp.path().join(".mcp.json"), config_json).unwrap();

    let configs = load_project_config(temp.path()).unwrap();
    assert_eq!(configs.len(), 1);
    assert!(configs.contains_key("enabled-server"));
    assert!(!configs.contains_key("disabled-server"));
}

#[test]
fn load_config_with_env_and_args() {
    let temp = TempDir::new().unwrap();
    let config_json = r#"{
        "mcpServers": {
            "narsil": {
                "command": "narsil-mcp",
                "args": ["--repo", "/path/to/repo", "--git"],
                "env": {"RUST_LOG": "info"}
            }
        }
    }"#;
    fs::write(temp.path().join(".mcp.json"), config_json).unwrap();

    let configs = load_project_config(temp.path()).unwrap();
    let entry = &configs["narsil"];
    assert_eq!(entry.command, "narsil-mcp");
    assert_eq!(entry.args.len(), 3);
    assert_eq!(entry.env.get("RUST_LOG"), Some(&"info".to_string()));
}

#[test]
fn load_empty_mcp_servers_returns_empty() {
    let temp = TempDir::new().unwrap();
    let config_json = r#"{"mcpServers": {}}"#;
    fs::write(temp.path().join(".mcp.json"), config_json).unwrap();

    let configs = load_project_config(temp.path()).unwrap();
    assert!(configs.is_empty());
}

#[test]
fn config_file_deserialization_roundtrip() {
    let json = r#"{
        "mcpServers": {
            "fs": {"command": "fs-server", "args": ["--root", "/tmp"]},
            "git": {"command": "git-mcp", "env": {"GIT_DIR": ".git"}}
        }
    }"#;
    let config: McpConfigFile = serde_json::from_str(json).unwrap();
    assert_eq!(config.mcp_servers.len(), 2);
    assert_eq!(config.mcp_servers["fs"].args, vec!["--root", "/tmp"]);
    assert_eq!(
        config.mcp_servers["git"].env.get("GIT_DIR"),
        Some(&".git".to_string())
    );
}

#[test]
fn server_entry_type_detection() {
    let stdio: McpServerEntry = serde_json::from_str(r#"{"command":"echo"}"#).unwrap();
    assert!(stdio.is_stdio());
    assert!(!stdio.is_sse());

    let sse: McpServerEntry = serde_json::from_str(r#"{"url":"http://localhost:8080"}"#).unwrap();
    assert!(sse.is_sse());
    assert!(!sse.is_stdio());
}
