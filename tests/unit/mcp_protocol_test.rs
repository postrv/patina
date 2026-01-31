//! Unit tests for MCP JSON-RPC protocol types.
//!
//! These tests verify serialization and parsing behavior for JSON-RPC 2.0 messages.
//! Following TDD RED phase - these tests will fail until types are implemented.

use patina::mcp::protocol::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};
use serde_json::json;

// ============================================================================
// Request Serialization Tests
// ============================================================================

/// Tests that a basic JSON-RPC request serializes correctly.
///
/// Expected JSON format per JSON-RPC 2.0 spec:
/// ```json
/// {
///   "jsonrpc": "2.0",
///   "id": 1,
///   "method": "tools/list",
///   "params": {}
/// }
/// ```
#[test]
fn test_jsonrpc_request_serialization() {
    let request = JsonRpcRequest::new(1, "tools/list", json!({}));

    let json = serde_json::to_string(&request).expect("Request should serialize to JSON");
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed["jsonrpc"], "2.0");
    assert_eq!(parsed["id"], 1);
    assert_eq!(parsed["method"], "tools/list");
    assert!(parsed["params"].is_object());
}

/// Tests that a request with complex params serializes correctly.
#[test]
fn test_jsonrpc_request_with_complex_params() {
    let params = json!({
        "name": "bash",
        "arguments": {
            "command": "ls -la"
        }
    });
    let request = JsonRpcRequest::new(42, "tools/call", params.clone());

    let json = serde_json::to_string(&request).expect("Request should serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed["id"], 42);
    assert_eq!(parsed["method"], "tools/call");
    assert_eq!(parsed["params"]["name"], "bash");
    assert_eq!(parsed["params"]["arguments"]["command"], "ls -la");
}

/// Tests that request with string ID serializes correctly.
#[test]
fn test_jsonrpc_request_string_id() {
    let request = JsonRpcRequest::new_with_string_id("abc-123", "initialize", json!({}));

    let json = serde_json::to_string(&request).expect("Request should serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed["id"], "abc-123");
}

/// Tests that notification (no id) serializes without id field.
#[test]
fn test_jsonrpc_notification_serialization() {
    let notification = JsonRpcRequest::notification("notifications/cancelled", json!({"id": 5}));

    let json = serde_json::to_string(&notification).expect("Notification should serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed["jsonrpc"], "2.0");
    assert_eq!(parsed["method"], "notifications/cancelled");
    assert!(parsed.get("id").is_none() || parsed["id"].is_null());
}

// ============================================================================
// Response Parsing Tests
// ============================================================================

/// Tests that a successful JSON-RPC response parses correctly.
///
/// Expected JSON format:
/// ```json
/// {
///   "jsonrpc": "2.0",
///   "id": 1,
///   "result": { "tools": [] }
/// }
/// ```
#[test]
fn test_jsonrpc_response_parsing() {
    let json_str = r#"{
        "jsonrpc": "2.0",
        "id": 1,
        "result": { "tools": [] }
    }"#;

    let response: JsonRpcResponse = serde_json::from_str(json_str).expect("Response should parse");

    assert!(response.is_success());
    assert!(!response.is_error());
    assert_eq!(response.id(), Some(&serde_json::json!(1)));

    let result = response.result().expect("Should have result");
    assert!(result.get("tools").unwrap().as_array().unwrap().is_empty());
}

/// Tests that a response with complex result parses correctly.
#[test]
fn test_jsonrpc_response_complex_result() {
    let json_str = r#"{
        "jsonrpc": "2.0",
        "id": 42,
        "result": {
            "tools": [
                {
                    "name": "bash",
                    "description": "Execute bash commands",
                    "inputSchema": { "type": "object" }
                }
            ]
        }
    }"#;

    let response: JsonRpcResponse = serde_json::from_str(json_str).expect("Should parse");

    assert!(response.is_success());
    let result = response.result().unwrap();
    let tools = result.get("tools").unwrap().as_array().unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "bash");
}

/// Tests that response with string ID parses correctly.
#[test]
fn test_jsonrpc_response_string_id() {
    let json_str = r#"{
        "jsonrpc": "2.0",
        "id": "request-abc",
        "result": null
    }"#;

    let response: JsonRpcResponse = serde_json::from_str(json_str).expect("Should parse");

    assert_eq!(response.id(), Some(&json!("request-abc")));
}

// ============================================================================
// Error Response Tests
// ============================================================================

/// Tests that a JSON-RPC error response parses correctly.
///
/// Expected JSON format:
/// ```json
/// {
///   "jsonrpc": "2.0",
///   "id": 1,
///   "error": {
///     "code": -32600,
///     "message": "Invalid Request"
///   }
/// }
/// ```
#[test]
fn test_jsonrpc_error_parsing() {
    let json_str = r#"{
        "jsonrpc": "2.0",
        "id": 1,
        "error": {
            "code": -32600,
            "message": "Invalid Request"
        }
    }"#;

    let response: JsonRpcResponse = serde_json::from_str(json_str).expect("Should parse");

    assert!(!response.is_success());
    assert!(response.is_error());

    let error = response.error().expect("Should have error");
    assert_eq!(error.code(), -32600);
    assert_eq!(error.message(), "Invalid Request");
    assert!(error.data().is_none());
}

/// Tests that error with additional data parses correctly.
#[test]
fn test_jsonrpc_error_with_data() {
    let json_str = r#"{
        "jsonrpc": "2.0",
        "id": 2,
        "error": {
            "code": -32602,
            "message": "Invalid params",
            "data": {
                "expected": "string",
                "received": "number"
            }
        }
    }"#;

    let response: JsonRpcResponse = serde_json::from_str(json_str).expect("Should parse");

    let error = response.error().unwrap();
    assert_eq!(error.code(), -32602);

    let data = error.data().expect("Should have data");
    assert_eq!(data["expected"], "string");
}

/// Tests that standard JSON-RPC error codes are correctly identified.
#[test]
fn test_jsonrpc_standard_error_codes() {
    // Parse error
    let error = JsonRpcError::parse_error();
    assert_eq!(error.code(), -32700);
    assert!(error.message().contains("Parse"));

    // Invalid request
    let error = JsonRpcError::invalid_request();
    assert_eq!(error.code(), -32600);

    // Method not found
    let error = JsonRpcError::method_not_found();
    assert_eq!(error.code(), -32601);

    // Invalid params
    let error = JsonRpcError::invalid_params("Missing field 'name'");
    assert_eq!(error.code(), -32602);

    // Internal error
    let error = JsonRpcError::internal_error();
    assert_eq!(error.code(), -32603);
}

/// Tests that error can be serialized back to JSON.
#[test]
fn test_jsonrpc_error_serialization() {
    let error = JsonRpcError::new(
        -32602,
        "Invalid params".to_string(),
        Some(json!({"field": "name"})),
    );

    let json = serde_json::to_string(&error).expect("Error should serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed["code"], -32602);
    assert_eq!(parsed["message"], "Invalid params");
    assert_eq!(parsed["data"]["field"], "name");
}

/// Tests that response with null id (error for notification) parses.
#[test]
fn test_jsonrpc_error_null_id() {
    let json_str = r#"{
        "jsonrpc": "2.0",
        "id": null,
        "error": {
            "code": -32600,
            "message": "Invalid Request"
        }
    }"#;

    let response: JsonRpcResponse = serde_json::from_str(json_str).expect("Should parse");

    // null id should be preserved
    assert!(response.id().is_none() || response.id() == Some(&serde_json::Value::Null));
    assert!(response.is_error());
}

// ============================================================================
// Round-trip Tests
// ============================================================================

/// Tests that request can be serialized and deserialized back.
#[test]
fn test_request_round_trip() {
    let original = JsonRpcRequest::new(999, "test/method", json!({"key": "value"}));

    let json = serde_json::to_string(&original).expect("Should serialize");
    let parsed: JsonRpcRequest = serde_json::from_str(&json).expect("Should deserialize");

    assert_eq!(parsed.method(), "test/method");
    assert_eq!(parsed.params()["key"], "value");
}
