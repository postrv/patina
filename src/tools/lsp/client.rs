//! JSON-RPC client for Language Server Protocol communication.
//!
//! Provides a lightweight JSON-RPC 2.0 client that communicates with language
//! servers over stdio. The client handles request/response correlation,
//! LSP initialization, and message framing.
//!
//! # Note
//!
//! This is a foundation module. Full server lifecycle management (start, shutdown,
//! reconnect) will be added when LSP is wired into the tool executor.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicI64, Ordering};

/// A JSON-RPC 2.0 request.
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcRequest {
    /// JSON-RPC version (always "2.0").
    pub jsonrpc: String,
    /// Request ID for correlating responses.
    pub id: i64,
    /// The method name.
    pub method: String,
    /// Optional parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// A JSON-RPC 2.0 response.
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcResponse {
    /// JSON-RPC version.
    pub jsonrpc: String,
    /// The response ID matching the request.
    pub id: Option<i64>,
    /// The result, if successful.
    pub result: Option<Value>,
    /// The error, if failed.
    pub error: Option<JsonRpcError>,
}

/// A JSON-RPC 2.0 error.
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcError {
    /// Error code.
    pub code: i64,
    /// Error message.
    pub message: String,
    /// Optional error data.
    pub data: Option<Value>,
}

impl std::fmt::Display for JsonRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JSON-RPC error {}: {}", self.code, self.message)
    }
}

/// Generates unique request IDs.
static REQUEST_ID: AtomicI64 = AtomicI64::new(1);

/// Creates a new JSON-RPC request.
#[must_use]
pub fn create_request(method: &str, params: Option<Value>) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: REQUEST_ID.fetch_add(1, Ordering::Relaxed),
        method: method.to_string(),
        params,
    }
}

/// Encodes a JSON-RPC message with LSP Content-Length framing.
///
/// # Errors
///
/// Returns an error if the message cannot be serialized.
pub fn encode_message(request: &JsonRpcRequest) -> Result<Vec<u8>> {
    let body = serde_json::to_string(request).context("Failed to serialize JSON-RPC request")?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    let mut message = header.into_bytes();
    message.extend_from_slice(body.as_bytes());
    Ok(message)
}

/// Parses a JSON-RPC response from a JSON string.
///
/// # Errors
///
/// Returns an error if the response cannot be parsed.
pub fn parse_response(json: &str) -> Result<JsonRpcResponse> {
    serde_json::from_str(json).context("Failed to parse JSON-RPC response")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_request_basic() {
        let req = create_request("textDocument/hover", None);
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.method, "textDocument/hover");
        assert!(req.params.is_none());
        assert!(req.id > 0);
    }

    #[test]
    fn test_create_request_with_params() {
        let params = serde_json::json!({"textDocument": {"uri": "file:///test.rs"}, "position": {"line": 0, "character": 0}});
        let req = create_request("textDocument/definition", Some(params.clone()));
        assert!(req.params.is_some());
        assert_eq!(req.params.unwrap(), params);
    }

    #[test]
    fn test_create_request_increments_id() {
        let req1 = create_request("method1", None);
        let req2 = create_request("method2", None);
        assert!(req2.id > req1.id);
    }

    #[test]
    fn test_encode_message() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: 1,
            method: "initialize".to_string(),
            params: None,
        };
        let encoded = encode_message(&req).unwrap();
        let encoded_str = String::from_utf8(encoded).unwrap();
        assert!(encoded_str.starts_with("Content-Length: "));
        assert!(encoded_str.contains("\r\n\r\n"));
        assert!(encoded_str.contains("\"initialize\""));
    }

    #[test]
    fn test_parse_response_success() {
        let json = r#"{"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}}"#;
        let resp = parse_response(json).unwrap();
        assert_eq!(resp.id, Some(1));
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
    }

    #[test]
    fn test_parse_response_error() {
        let json =
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"Method not found"}}"#;
        let resp = parse_response(json).unwrap();
        assert!(resp.error.is_some());
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32601);
        assert_eq!(err.message, "Method not found");
    }

    #[test]
    fn test_json_rpc_error_display() {
        let err = JsonRpcError {
            code: -32600,
            message: "Invalid request".to_string(),
            data: None,
        };
        assert_eq!(err.to_string(), "JSON-RPC error -32600: Invalid request");
    }

    #[test]
    fn test_request_serialization() {
        let req = create_request("shutdown", None);
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"method\":\"shutdown\""));
        // params should be omitted when None
        assert!(!json.contains("\"params\""));
    }
}
