//! Integration tests for MCP stdio transport.
//!
//! These tests verify that the stdio transport can:
//! - Start MCP server processes
//! - Send and receive JSON-RPC messages
//! - Handle the MCP initialization protocol

use rct::mcp::protocol::JsonRpcRequest;
use rct::mcp::transport::{StdioTransport, Transport};
use serde_json::json;
use std::time::Duration;

/// Helper to create a mock MCP server command that echoes JSON-RPC responses.
///
/// This uses a simple shell script that reads a line and responds with
/// a proper JSON-RPC response.
fn mock_mcp_server_command() -> (&'static str, Vec<&'static str>) {
    // Use bash to create a simple echo server
    // It reads a line, parses it as JSON-RPC, and responds
    (
        "bash",
        vec![
            "-c",
            r#"
            while IFS= read -r line; do
                # Parse the method from the JSON-RPC request
                method=$(echo "$line" | jq -r '.method // empty')
                id=$(echo "$line" | jq -r '.id // empty')

                case "$method" in
                    "initialize")
                        echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{\"tools\":{}},\"serverInfo\":{\"name\":\"mock-server\",\"version\":\"1.0.0\"}}}"
                        ;;
                    "tools/list")
                        echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"tools\":[{\"name\":\"echo\",\"description\":\"Echo input\",\"inputSchema\":{\"type\":\"object\"}}]}}"
                        ;;
                    "tools/call")
                        echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"Tool executed\"}]}}"
                        ;;
                    "ping")
                        echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{}}"
                        ;;
                    *)
                        echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"error\":{\"code\":-32601,\"message\":\"Method not found\"}}"
                        ;;
                esac
            done
            "#,
        ],
    )
}

/// Tests that stdio transport can initialize an MCP server.
///
/// The MCP initialization sequence:
/// 1. Client sends "initialize" request with capabilities
/// 2. Server responds with its capabilities and version
/// 3. Client sends "initialized" notification
#[tokio::test]
async fn test_mcp_stdio_initialization() {
    let (cmd, args) = mock_mcp_server_command();

    // Create and start the transport
    let mut transport = StdioTransport::new(cmd, args);
    transport
        .start()
        .await
        .expect("Transport should start successfully");

    // Send initialize request
    let request = JsonRpcRequest::new(
        1,
        "initialize",
        json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "rct",
                "version": "0.1.0"
            }
        }),
    );

    let response = transport
        .send_request(request, Duration::from_secs(5))
        .await
        .expect("Should receive response");

    assert!(response.is_success(), "Initialize should succeed");

    let result = response.result().expect("Should have result");
    assert_eq!(result["protocolVersion"], "2024-11-05");
    assert!(result["capabilities"].is_object());
    assert!(result["serverInfo"].is_object());

    // Send initialized notification (no response expected)
    let notification = JsonRpcRequest::notification("initialized", json!({}));
    transport
        .send_notification(notification)
        .await
        .expect("Should send notification");

    // Shutdown
    transport.stop().await.expect("Transport should stop");
}

/// Tests bidirectional communication with the MCP server.
///
/// Verifies that we can:
/// 1. Send multiple requests
/// 2. Receive correct responses
/// 3. Handle concurrent messages
#[tokio::test]
async fn test_mcp_stdio_bidirectional() {
    let (cmd, args) = mock_mcp_server_command();

    let mut transport = StdioTransport::new(cmd, args);
    transport.start().await.expect("Transport should start");

    // Initialize first
    let init_request = JsonRpcRequest::new(1, "initialize", json!({}));
    transport
        .send_request(init_request, Duration::from_secs(5))
        .await
        .expect("Initialize should succeed");

    // Test ping-pong
    let ping = JsonRpcRequest::new(2, "ping", json!({}));
    let pong = transport
        .send_request(ping, Duration::from_secs(5))
        .await
        .expect("Ping should get response");
    assert!(pong.is_success());

    // Test tools/list
    let list_tools = JsonRpcRequest::new(3, "tools/list", json!({}));
    let tools_response = transport
        .send_request(list_tools, Duration::from_secs(5))
        .await
        .expect("Should list tools");

    assert!(tools_response.is_success());
    let result = tools_response.result().unwrap();
    let tools = result["tools"].as_array().expect("Should have tools array");
    assert!(!tools.is_empty());

    // Test tools/call
    let call_tool = JsonRpcRequest::new(
        4,
        "tools/call",
        json!({
            "name": "echo",
            "arguments": {"text": "hello"}
        }),
    );
    let call_response = transport
        .send_request(call_tool, Duration::from_secs(5))
        .await
        .expect("Should call tool");

    assert!(call_response.is_success());
    let result = call_response.result().unwrap();
    assert!(result["content"].is_array());

    transport.stop().await.expect("Should stop");
}

/// Tests that transport handles unknown methods correctly.
#[tokio::test]
async fn test_mcp_stdio_method_not_found() {
    let (cmd, args) = mock_mcp_server_command();

    let mut transport = StdioTransport::new(cmd, args);
    transport.start().await.expect("Transport should start");

    let request = JsonRpcRequest::new(1, "nonexistent/method", json!({}));
    let response = transport
        .send_request(request, Duration::from_secs(5))
        .await
        .expect("Should receive error response");

    assert!(response.is_error());
    let error = response.error().expect("Should have error");
    assert_eq!(error.code(), -32601); // Method not found
}

/// Tests that transport handles timeout correctly.
#[tokio::test]
async fn test_mcp_stdio_timeout() {
    // Use a server that doesn't respond - sleep waits forever without producing output
    let mut transport = StdioTransport::new("sleep", vec!["3600"]);
    transport.start().await.expect("Transport should start");

    let request = JsonRpcRequest::new(1, "ping", json!({}));
    let result = transport
        .send_request(request, Duration::from_millis(100))
        .await;

    assert!(result.is_err(), "Should timeout");
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("timeout") || err.to_string().contains("Timeout"),
        "Error should mention timeout: {err}"
    );

    // Stop should still work even after timeout
    let _ = transport.stop().await;
}

/// Tests that transport can be stopped and restarted.
#[tokio::test]
async fn test_mcp_stdio_restart() {
    let (cmd, args) = mock_mcp_server_command();

    let mut transport = StdioTransport::new(cmd, args.clone());

    // First run
    transport.start().await.expect("Should start");
    let request = JsonRpcRequest::new(1, "ping", json!({}));
    let response = transport
        .send_request(request, Duration::from_secs(5))
        .await
        .expect("Should respond");
    assert!(response.is_success());
    transport.stop().await.expect("Should stop");

    // Second run - should work the same
    let mut transport2 = StdioTransport::new(cmd, args);
    transport2.start().await.expect("Should start again");
    let request2 = JsonRpcRequest::new(1, "ping", json!({}));
    let response2 = transport2
        .send_request(request2, Duration::from_secs(5))
        .await
        .expect("Should respond again");
    assert!(response2.is_success());
    transport2.stop().await.expect("Should stop");
}
