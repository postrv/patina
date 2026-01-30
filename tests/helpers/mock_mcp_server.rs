//! Mock MCP Server for Cross-Platform Testing
//!
//! # Architecture Design (Task 3.1.1)
//!
//! This is a standalone Rust binary that acts as an MCP server for testing purposes.
//! It replaces the bash-based mock server to enable Windows compatibility.
//!
//! ## Purpose
//!
//! The mock server provides a predictable MCP server implementation that:
//! - Reads JSON-RPC 2.0 messages from stdin
//! - Responds with configurable MCP protocol messages on stdout
//! - Supports multiple test scenarios via command-line flags
//!
//! ## Usage
//!
//! ```bash
//! # Normal mode - responds to all MCP methods
//! mock_mcp_server
//!
//! # Crash after N messages
//! mock_mcp_server --crash-after 2
//!
//! # Return invalid JSON for Nth message
//! mock_mcp_server --invalid-json-at 1
//!
//! # Add delay before responding (milliseconds)
//! mock_mcp_server --delay 1000
//!
//! # Never respond (for timeout testing)
//! mock_mcp_server --no-response
//!
//! # Exit immediately (for crash testing)
//! mock_mcp_server --exit-immediately
//! ```
//!
//! ## Supported MCP Methods
//!
//! | Method | Response |
//! |--------|----------|
//! | `initialize` | `{protocolVersion, capabilities, serverInfo}` |
//! | `initialized` | (notification - no response) |
//! | `tools/list` | `{tools: [{name, description, inputSchema}]}` |
//! | `tools/call` | `{content: [{type, text}]}` |
//! | `ping` | `{}` |
//! | Unknown | JSON-RPC error -32601 (Method not found) |
//!
//! ## Test Scenarios
//!
//! | Scenario | Flag | Behavior |
//! |----------|------|----------|
//! | Normal operation | (none) | Respond to all methods correctly |
//! | Crash after N | `--crash-after N` | Exit with code 1 after N messages |
//! | Invalid JSON | `--invalid-json-at N` | Output "not valid json" for message N |
//! | Slow response | `--delay MS` | Sleep before each response |
//! | No response | `--no-response` | Read input but never respond |
//! | Exit immediately | `--exit-immediately` | Exit with code 0 before reading |
//! | Custom exit code | `--exit-code N` | Exit with specified code |
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    Mock MCP Server                          │
//! ├─────────────────────────────────────────────────────────────┤
//! │                                                             │
//! │  ┌─────────────┐     ┌──────────────┐     ┌─────────────┐  │
//! │  │   Stdin     │────▶│  Message     │────▶│  Response   │  │
//! │  │   Reader    │     │  Router      │     │  Writer     │  │
//! │  └─────────────┘     └──────────────┘     └─────────────┘  │
//! │                             │                    │          │
//! │                             ▼                    ▼          │
//! │                      ┌──────────────┐     ┌─────────────┐  │
//! │                      │   Config     │     │   Stdout    │  │
//! │                      │   (flags)    │     │   Writer    │  │
//! │                      └──────────────┘     └─────────────┘  │
//! │                                                             │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Message Flow
//!
//! 1. Read line from stdin (blocking)
//! 2. Parse as JSON-RPC request
//! 3. Check configuration for special behavior:
//!    - If crash-after reached, exit
//!    - If invalid-json-at matches, output invalid string
//!    - If delay set, sleep
//!    - If no-response set, skip output
//! 4. Route to method handler
//! 5. Write JSON-RPC response to stdout
//! 6. Repeat
//!
//! ## Exit Codes
//!
//! | Code | Meaning |
//! |------|---------|
//! | 0 | Clean shutdown |
//! | 1 | Simulated crash |
//! | Custom | Via `--exit-code` flag |

use std::io::{self, BufRead, Write};

/// Configuration parsed from command-line arguments.
#[derive(Debug, Default)]
struct Config {
    /// Exit after processing this many messages (0 = unlimited).
    crash_after: usize,
    /// Return invalid JSON for this message number (0 = never).
    invalid_json_at: usize,
    /// Delay in milliseconds before responding.
    delay_ms: u64,
    /// Never respond to messages.
    no_response: bool,
    /// Exit immediately without reading any input.
    exit_immediately: bool,
    /// Exit code to use when exiting.
    exit_code: i32,
}

impl Config {
    /// Parse configuration from command-line arguments.
    fn from_args() -> Self {
        let args: Vec<String> = std::env::args().collect();
        let mut config = Config::default();

        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--crash-after" => {
                    i += 1;
                    if i < args.len() {
                        config.crash_after = args[i].parse().unwrap_or(0);
                    }
                }
                "--invalid-json-at" => {
                    i += 1;
                    if i < args.len() {
                        config.invalid_json_at = args[i].parse().unwrap_or(0);
                    }
                }
                "--delay" => {
                    i += 1;
                    if i < args.len() {
                        config.delay_ms = args[i].parse().unwrap_or(0);
                    }
                }
                "--no-response" => {
                    config.no_response = true;
                }
                "--exit-immediately" => {
                    config.exit_immediately = true;
                }
                "--exit-code" => {
                    i += 1;
                    if i < args.len() {
                        config.exit_code = args[i].parse().unwrap_or(1);
                    }
                }
                _ => {}
            }
            i += 1;
        }

        config
    }
}

/// JSON-RPC request structure.
#[derive(Debug)]
struct JsonRpcRequest {
    id: Option<serde_json::Value>,
    method: String,
    #[allow(dead_code)]
    params: serde_json::Value,
}

impl JsonRpcRequest {
    /// Parse a JSON-RPC request from a JSON string.
    fn parse(line: &str) -> Option<Self> {
        let json: serde_json::Value = serde_json::from_str(line).ok()?;
        let obj = json.as_object()?;

        let method = obj.get("method")?.as_str()?.to_string();
        let id = obj.get("id").cloned();
        let params = obj.get("params").cloned().unwrap_or(serde_json::json!({}));

        Some(JsonRpcRequest { id, method, params })
    }

    /// Returns true if this is a notification (no id).
    fn is_notification(&self) -> bool {
        self.id.is_none()
    }
}

/// Generate JSON-RPC success response.
fn success_response(id: &serde_json::Value, result: serde_json::Value) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })
    .to_string()
}

/// Generate JSON-RPC error response.
fn error_response(id: &serde_json::Value, code: i32, message: &str) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message
        }
    })
    .to_string()
}

/// Handle the "initialize" method.
fn handle_initialize(id: &serde_json::Value) -> String {
    success_response(
        id,
        serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "mock-mcp-server",
                "version": "1.0.0"
            }
        }),
    )
}

/// Handle the "tools/list" method.
fn handle_tools_list(id: &serde_json::Value) -> String {
    success_response(
        id,
        serde_json::json!({
            "tools": [
                {
                    "name": "echo",
                    "description": "Echo input back",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "text": {
                                "type": "string",
                                "description": "Text to echo"
                            }
                        },
                        "required": ["text"]
                    }
                }
            ]
        }),
    )
}

/// Handle the "tools/call" method.
fn handle_tools_call(id: &serde_json::Value) -> String {
    success_response(
        id,
        serde_json::json!({
            "content": [
                {
                    "type": "text",
                    "text": "Tool executed successfully"
                }
            ]
        }),
    )
}

/// Handle the "ping" method.
fn handle_ping(id: &serde_json::Value) -> String {
    success_response(id, serde_json::json!({}))
}

/// Handle unknown methods.
fn handle_unknown(id: &serde_json::Value) -> String {
    error_response(id, -32601, "Method not found")
}

/// Route a request to the appropriate handler.
fn route_request(request: &JsonRpcRequest) -> Option<String> {
    // Notifications don't get responses
    if request.is_notification() {
        return None;
    }

    let id = request.id.as_ref()?;

    let response = match request.method.as_str() {
        "initialize" => handle_initialize(id),
        "tools/list" => handle_tools_list(id),
        "tools/call" => handle_tools_call(id),
        "ping" => handle_ping(id),
        _ => handle_unknown(id),
    };

    Some(response)
}

/// Main entry point for the mock MCP server.
fn main() {
    let config = Config::from_args();

    // Exit immediately if configured
    if config.exit_immediately {
        std::process::exit(config.exit_code);
    }

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut message_count = 0usize;

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        // Skip empty lines
        if line.trim().is_empty() {
            continue;
        }

        message_count += 1;

        // Check crash-after
        if config.crash_after > 0 && message_count > config.crash_after {
            std::process::exit(config.exit_code.max(1));
        }

        // Apply delay if configured
        if config.delay_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(config.delay_ms));
        }

        // Check no-response
        if config.no_response {
            continue;
        }

        // Check invalid-json-at
        if config.invalid_json_at > 0 && message_count == config.invalid_json_at {
            writeln!(stdout, "this is not valid json!!!").ok();
            stdout.flush().ok();
            continue;
        }

        // Parse and route the request
        if let Some(request) = JsonRpcRequest::parse(&line) {
            if let Some(response) = route_request(&request) {
                writeln!(stdout, "{}", response).ok();
                stdout.flush().ok();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_request_with_id() {
        let json = r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}"#;
        let request = JsonRpcRequest::parse(json).unwrap();
        assert_eq!(request.method, "ping");
        assert_eq!(request.id, Some(serde_json::json!(1)));
        assert!(!request.is_notification());
    }

    #[test]
    fn test_parse_notification() {
        let json = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
        let request = JsonRpcRequest::parse(json).unwrap();
        assert_eq!(request.method, "initialized");
        assert!(request.is_notification());
    }

    #[test]
    fn test_route_initialize() {
        let request = JsonRpcRequest {
            id: Some(serde_json::json!(1)),
            method: "initialize".to_string(),
            params: serde_json::json!({}),
        };
        let response = route_request(&request).unwrap();
        assert!(response.contains("protocolVersion"));
        assert!(response.contains("2024-11-05"));
    }

    #[test]
    fn test_route_tools_list() {
        let request = JsonRpcRequest {
            id: Some(serde_json::json!(1)),
            method: "tools/list".to_string(),
            params: serde_json::json!({}),
        };
        let response = route_request(&request).unwrap();
        assert!(response.contains("tools"));
        assert!(response.contains("echo"));
    }

    #[test]
    fn test_route_unknown_method() {
        let request = JsonRpcRequest {
            id: Some(serde_json::json!(1)),
            method: "unknown/method".to_string(),
            params: serde_json::json!({}),
        };
        let response = route_request(&request).unwrap();
        assert!(response.contains("-32601"));
        assert!(response.contains("Method not found"));
    }

    #[test]
    fn test_notification_no_response() {
        let request = JsonRpcRequest {
            id: None,
            method: "initialized".to_string(),
            params: serde_json::json!({}),
        };
        let response = route_request(&request);
        assert!(response.is_none());
    }

    #[test]
    fn test_config_default() {
        let config = Config::default();
        assert_eq!(config.crash_after, 0);
        assert_eq!(config.invalid_json_at, 0);
        assert_eq!(config.delay_ms, 0);
        assert!(!config.no_response);
        assert!(!config.exit_immediately);
        assert_eq!(config.exit_code, 0);
    }
}
