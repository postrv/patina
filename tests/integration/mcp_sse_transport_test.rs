//! Integration tests for MCP SSE transport.
//!
//! These tests verify that the SSE transport can:
//! - Connect to an SSE endpoint
//! - Send and receive JSON-RPC messages via HTTP
//! - Handle the MCP initialization protocol over SSE
//!
//! SSE (Server-Sent Events) transport uses:
//! - GET to /sse endpoint for server-to-client events
//! - POST to /message endpoint for client-to-server messages

use rct::mcp::protocol::JsonRpcRequest;
use rct::mcp::transport::{SseTransport, Transport};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

/// Helper to create a mock MCP server with SSE support.
async fn setup_mock_sse_server() -> MockServer {
    let server = MockServer::start().await;
    let request_counter = Arc::new(AtomicUsize::new(0));

    // SSE endpoint - returns server info and message endpoint URL
    // Return relative path /message which will be resolved against the SSE URL
    Mock::given(method("GET"))
        .and(path("/sse"))
        .and(header("accept", "text/event-stream"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .insert_header("cache-control", "no-cache")
                .set_body_string("event: endpoint\ndata: /message\n\n"),
        )
        .mount(&server)
        .await;

    // Message endpoint - handles JSON-RPC requests
    let counter = Arc::clone(&request_counter);
    Mock::given(method("POST"))
        .and(path("/message"))
        .and(header("content-type", "application/json"))
        .respond_with(move |req: &Request| {
            let id = counter.fetch_add(1, Ordering::SeqCst);

            // Parse request to get method and id
            if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&req.body) {
                let method = json.get("method").and_then(|m| m.as_str()).unwrap_or("");
                let req_id = json.get("id").cloned().unwrap_or(json!(id));

                let response = match method {
                    "initialize" => json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "result": {
                            "protocolVersion": "2024-11-05",
                            "capabilities": { "tools": {} },
                            "serverInfo": {
                                "name": "mock-sse-server",
                                "version": "1.0.0"
                            }
                        }
                    }),
                    "tools/list" => json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "result": {
                            "tools": [{
                                "name": "echo",
                                "description": "Echo input",
                                "inputSchema": { "type": "object" }
                            }]
                        }
                    }),
                    "tools/call" => json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "result": {
                            "content": [{
                                "type": "text",
                                "text": "Tool executed"
                            }]
                        }
                    }),
                    "ping" => json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "result": {}
                    }),
                    _ => json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "error": {
                            "code": -32601,
                            "message": "Method not found"
                        }
                    }),
                };

                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_json(response)
            } else {
                ResponseTemplate::new(400).set_body_string("Invalid JSON")
            }
        })
        .mount(&server)
        .await;

    server
}

// ============================================================================
// SSE Transport Connection Tests
// ============================================================================

/// Tests that SSE transport can connect to a server.
#[tokio::test]
async fn test_sse_transport_connection() {
    let server = setup_mock_sse_server().await;
    let sse_url = format!("{}/sse", server.uri());

    let mut transport = SseTransport::new(&sse_url);
    transport
        .start()
        .await
        .expect("SSE transport should connect");

    assert!(transport.is_connected(), "Transport should be connected");

    transport.stop().await.expect("Should stop cleanly");
}

/// Tests that SSE transport handles connection failure gracefully.
#[tokio::test]
async fn test_sse_transport_connection_failure() {
    // Invalid URL that won't connect
    let mut transport = SseTransport::new("http://127.0.0.1:1/nonexistent");
    let result = transport.start().await;

    assert!(result.is_err(), "Should fail to connect to invalid URL");
}

// ============================================================================
// SSE Transport Initialization Tests
// ============================================================================

/// Tests that SSE transport can initialize an MCP server.
#[tokio::test]
async fn test_sse_transport_initialization() {
    let server = setup_mock_sse_server().await;
    let sse_url = format!("{}/sse", server.uri());

    let mut transport = SseTransport::new(&sse_url);
    transport.start().await.expect("Should connect");

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
    assert!(result["serverInfo"].is_object());

    transport.stop().await.expect("Should stop");
}

// ============================================================================
// SSE Transport Bidirectional Tests
// ============================================================================

/// Tests bidirectional communication over SSE transport.
#[tokio::test]
async fn test_sse_transport_bidirectional() {
    let server = setup_mock_sse_server().await;
    let sse_url = format!("{}/sse", server.uri());

    let mut transport = SseTransport::new(&sse_url);
    transport.start().await.expect("Should connect");

    // Initialize first
    let init_request = JsonRpcRequest::new(1, "initialize", json!({}));
    transport
        .send_request(init_request, Duration::from_secs(5))
        .await
        .expect("Initialize should succeed");

    // Test ping
    let ping = JsonRpcRequest::new(2, "ping", json!({}));
    let pong = transport
        .send_request(ping, Duration::from_secs(5))
        .await
        .expect("Ping should respond");
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

    transport.stop().await.expect("Should stop");
}

/// Tests that SSE transport handles unknown methods correctly.
#[tokio::test]
async fn test_sse_transport_method_not_found() {
    let server = setup_mock_sse_server().await;
    let sse_url = format!("{}/sse", server.uri());

    let mut transport = SseTransport::new(&sse_url);
    transport.start().await.expect("Should connect");

    let request = JsonRpcRequest::new(1, "nonexistent/method", json!({}));
    let response = transport
        .send_request(request, Duration::from_secs(5))
        .await
        .expect("Should receive error response");

    assert!(response.is_error());
    let error = response.error().expect("Should have error");
    assert_eq!(error.code(), -32601); // Method not found

    transport.stop().await.expect("Should stop");
}

// ============================================================================
// SSE Transport Timeout Tests
// ============================================================================

/// Tests that SSE transport handles request timeout.
#[tokio::test]
async fn test_sse_transport_timeout() {
    let server = MockServer::start().await;

    // Mount a slow endpoint that doesn't respond
    Mock::given(method("GET"))
        .and(path("/sse"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(60)))
        .mount(&server)
        .await;

    let sse_url = format!("{}/sse", server.uri());
    let mut transport = SseTransport::new(&sse_url);

    // Connection should timeout
    let result = tokio::time::timeout(Duration::from_millis(500), transport.start()).await;

    // Either the start times out or fails quickly
    assert!(
        result.is_err() || result.unwrap().is_err(),
        "Should timeout or fail"
    );
}

// ============================================================================
// SSE Transport Restart Tests
// ============================================================================

/// Tests that SSE transport can be stopped and restarted.
#[tokio::test]
async fn test_sse_transport_restart() {
    let server = setup_mock_sse_server().await;
    let sse_url = format!("{}/sse", server.uri());

    // First connection
    let mut transport = SseTransport::new(&sse_url);
    transport.start().await.expect("Should connect");

    let request = JsonRpcRequest::new(1, "ping", json!({}));
    let response = transport
        .send_request(request, Duration::from_secs(5))
        .await
        .expect("Should respond");
    assert!(response.is_success());

    transport.stop().await.expect("Should stop");

    // Second connection with new transport
    let mut transport2 = SseTransport::new(&sse_url);
    transport2.start().await.expect("Should reconnect");

    let request2 = JsonRpcRequest::new(1, "ping", json!({}));
    let response2 = transport2
        .send_request(request2, Duration::from_secs(5))
        .await
        .expect("Should respond again");
    assert!(response2.is_success());

    transport2.stop().await.expect("Should stop");
}

// ============================================================================
// SSE Transport Custom Headers Tests
// ============================================================================

// ============================================================================
// SSE Transport Error Path Tests (Task 3.3.1)
// ============================================================================

/// Tests that SSE transport handles connection loss gracefully.
///
/// Verifies that:
/// - After connection, if message endpoint fails, error is returned
/// - The transport can be stopped cleanly after connection issues
#[tokio::test]
async fn test_sse_connection_lost() {
    let server = MockServer::start().await;

    // SSE endpoint works initially
    Mock::given(method("GET"))
        .and(path("/sse"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string("event: endpoint\ndata: /message\n\n"),
        )
        .mount(&server)
        .await;

    // Message endpoint returns 503 Service Unavailable (simulating connection loss)
    Mock::given(method("POST"))
        .and(path("/message"))
        .respond_with(ResponseTemplate::new(503).set_body_string("Service Unavailable"))
        .mount(&server)
        .await;

    let sse_url = format!("{}/sse", server.uri());
    let mut transport = SseTransport::new(&sse_url);

    // Connection should succeed (SSE endpoint works)
    transport.start().await.expect("Should connect");
    assert!(transport.is_connected());

    // Request should fail because message endpoint returns 503
    let request = JsonRpcRequest::new(1, "ping", json!({}));
    let result = transport
        .send_request(request, Duration::from_secs(5))
        .await;

    assert!(
        result.is_err(),
        "Request should fail when message endpoint is unavailable"
    );

    let err = result.unwrap_err();
    // The error message may vary - just verify we got an error
    assert!(
        !err.to_string().is_empty(),
        "Error should have a message: {}",
        err
    );

    // Stop should still work
    transport.stop().await.expect("Should stop cleanly");
}

/// Tests that SSE transport handles HTTP timeout on POST requests.
///
/// Verifies that:
/// - Slow POST responses trigger timeout
/// - Timeout error is returned appropriately
#[tokio::test]
async fn test_http_timeout() {
    let server = MockServer::start().await;

    // SSE endpoint responds immediately
    Mock::given(method("GET"))
        .and(path("/sse"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string("event: endpoint\ndata: /message\n\n"),
        )
        .mount(&server)
        .await;

    // Message endpoint is very slow (5 second delay)
    Mock::given(method("POST"))
        .and(path("/message"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(5))
                .set_body_json(json!({"jsonrpc": "2.0", "id": 1, "result": {}})),
        )
        .mount(&server)
        .await;

    let sse_url = format!("{}/sse", server.uri());
    let mut transport = SseTransport::new(&sse_url);

    // Connection should succeed
    transport.start().await.expect("Should connect");

    // Request with short timeout should fail
    let request = JsonRpcRequest::new(1, "ping", json!({}));
    let result = transport
        .send_request(request, Duration::from_millis(200))
        .await;

    assert!(result.is_err(), "Request should timeout");

    let err = result.unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("timeout"),
        "Error should mention timeout: {}",
        err
    );

    // Stop should still work
    transport.stop().await.expect("Should stop cleanly");
}

/// Tests that SSE transport handles invalid JSON response from message endpoint.
///
/// Verifies that:
/// - Invalid JSON response causes an error
/// - The transport doesn't crash
#[tokio::test]
async fn test_sse_invalid_json_response() {
    let server = MockServer::start().await;

    // SSE endpoint works
    Mock::given(method("GET"))
        .and(path("/sse"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string("event: endpoint\ndata: /message\n\n"),
        )
        .mount(&server)
        .await;

    // Message endpoint returns invalid JSON
    Mock::given(method("POST"))
        .and(path("/message"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string("this is not valid json at all!!!"),
        )
        .mount(&server)
        .await;

    let sse_url = format!("{}/sse", server.uri());
    let mut transport = SseTransport::new(&sse_url);

    transport.start().await.expect("Should connect");

    let request = JsonRpcRequest::new(1, "ping", json!({}));
    let result = transport
        .send_request(request, Duration::from_secs(5))
        .await;

    assert!(
        result.is_err(),
        "Request should fail on invalid JSON response"
    );

    let err = result.unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("parse")
            || err.to_string().to_lowercase().contains("json")
            || err.to_string().to_lowercase().contains("failed"),
        "Error should indicate JSON parsing failure: {}",
        err
    );

    transport.stop().await.expect("Should stop cleanly");
}

/// Tests that SSE transport sends custom headers.
#[tokio::test]
async fn test_sse_transport_custom_headers() {
    let server = MockServer::start().await;

    // Verify custom header is sent
    Mock::given(method("GET"))
        .and(path("/sse"))
        .and(header("x-api-key", "test-key"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string("event: endpoint\ndata: /message\n\n"),
        )
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/message"))
        .and(header("x-api-key", "test-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {}
        })))
        .mount(&server)
        .await;

    let sse_url = format!("{}/sse", server.uri());
    let mut headers = std::collections::HashMap::new();
    headers.insert("x-api-key".to_string(), "test-key".to_string());

    let mut transport = SseTransport::with_headers(&sse_url, headers);
    transport
        .start()
        .await
        .expect("Should connect with headers");

    let request = JsonRpcRequest::new(1, "ping", json!({}));
    let response = transport
        .send_request(request, Duration::from_secs(5))
        .await
        .expect("Should respond");

    assert!(response.is_success());
    transport.stop().await.expect("Should stop");
}
