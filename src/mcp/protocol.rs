//! JSON-RPC 2.0 protocol types for MCP communication.
//!
//! This module implements the JSON-RPC 2.0 specification types used by the
//! Model Context Protocol (MCP) for communication between client and server.
//!
//! # Example
//!
//! ```
//! use rct::mcp::protocol::{JsonRpcRequest, JsonRpcResponse, JsonRpcError};
//! use serde_json::json;
//!
//! // Create a request
//! let request = JsonRpcRequest::new(1, "tools/list", json!({}));
//!
//! // Serialize to JSON
//! let json = serde_json::to_string(&request).unwrap();
//! assert!(json.contains("\"jsonrpc\":\"2.0\""));
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC 2.0 request ID, which can be a number, string, or null.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    /// Numeric request ID
    Number(i64),
    /// String request ID
    String(String),
}

impl From<i64> for RequestId {
    fn from(n: i64) -> Self {
        RequestId::Number(n)
    }
}

impl From<&str> for RequestId {
    fn from(s: &str) -> Self {
        RequestId::String(s.to_string())
    }
}

impl From<String> for RequestId {
    fn from(s: String) -> Self {
        RequestId::String(s)
    }
}

/// A JSON-RPC 2.0 request message.
///
/// Requests are sent from client to server to invoke methods.
/// A request without an ID is a notification (no response expected).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    /// JSON-RPC version, always "2.0"
    jsonrpc: String,

    /// Request ID (absent for notifications)
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<RequestId>,

    /// Method name to invoke
    method: String,

    /// Method parameters
    #[serde(default)]
    params: Value,
}

impl JsonRpcRequest {
    /// Creates a new JSON-RPC request with a numeric ID.
    ///
    /// # Arguments
    ///
    /// * `id` - Numeric request identifier
    /// * `method` - Method name to invoke
    /// * `params` - Method parameters as JSON value
    ///
    /// # Example
    ///
    /// ```
    /// use rct::mcp::protocol::JsonRpcRequest;
    /// use serde_json::json;
    ///
    /// let request = JsonRpcRequest::new(1, "tools/list", json!({}));
    /// ```
    #[must_use]
    pub fn new(id: i64, method: &str, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id: Some(RequestId::Number(id)),
            method: method.to_string(),
            params,
        }
    }

    /// Creates a new JSON-RPC request with a string ID.
    ///
    /// # Arguments
    ///
    /// * `id` - String request identifier
    /// * `method` - Method name to invoke
    /// * `params` - Method parameters as JSON value
    #[must_use]
    pub fn new_with_string_id(id: &str, method: &str, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id: Some(RequestId::String(id.to_string())),
            method: method.to_string(),
            params,
        }
    }

    /// Creates a notification (request without ID, no response expected).
    ///
    /// Notifications are one-way messages that don't expect a response.
    ///
    /// # Arguments
    ///
    /// * `method` - Method name to invoke
    /// * `params` - Method parameters as JSON value
    #[must_use]
    pub fn notification(method: &str, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id: None,
            method: method.to_string(),
            params,
        }
    }

    /// Returns the method name.
    #[must_use]
    pub fn method(&self) -> &str {
        &self.method
    }

    /// Returns the request parameters.
    #[must_use]
    pub fn params(&self) -> &Value {
        &self.params
    }

    /// Returns the request ID, if present.
    #[must_use]
    pub fn id(&self) -> Option<&RequestId> {
        self.id.as_ref()
    }

    /// Returns true if this is a notification (no ID).
    #[must_use]
    pub fn is_notification(&self) -> bool {
        self.id.is_none()
    }
}

/// A JSON-RPC 2.0 error object.
///
/// Error objects are included in responses when a request fails.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    /// Error code (negative for predefined errors)
    code: i32,

    /// Human-readable error message
    message: String,

    /// Additional error data (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl JsonRpcError {
    /// Creates a new JSON-RPC error.
    ///
    /// # Arguments
    ///
    /// * `code` - Error code
    /// * `message` - Human-readable error message
    /// * `data` - Optional additional error data
    #[must_use]
    pub fn new(code: i32, message: String, data: Option<Value>) -> Self {
        Self {
            code,
            message,
            data,
        }
    }

    /// Returns the error code.
    #[must_use]
    pub fn code(&self) -> i32 {
        self.code
    }

    /// Returns the error message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the additional error data, if present.
    #[must_use]
    pub fn data(&self) -> Option<&Value> {
        self.data.as_ref()
    }

    /// Creates a Parse error (-32700).
    ///
    /// Invalid JSON was received by the server.
    #[must_use]
    pub fn parse_error() -> Self {
        Self::new(-32700, "Parse error".to_string(), None)
    }

    /// Creates an Invalid Request error (-32600).
    ///
    /// The JSON sent is not a valid Request object.
    #[must_use]
    pub fn invalid_request() -> Self {
        Self::new(-32600, "Invalid Request".to_string(), None)
    }

    /// Creates a Method not found error (-32601).
    ///
    /// The method does not exist or is not available.
    #[must_use]
    pub fn method_not_found() -> Self {
        Self::new(-32601, "Method not found".to_string(), None)
    }

    /// Creates an Invalid params error (-32602).
    ///
    /// Invalid method parameter(s).
    ///
    /// # Arguments
    ///
    /// * `details` - Additional details about the invalid parameters
    #[must_use]
    pub fn invalid_params(details: &str) -> Self {
        Self::new(-32602, format!("Invalid params: {details}"), None)
    }

    /// Creates an Internal error (-32603).
    ///
    /// Internal JSON-RPC error.
    #[must_use]
    pub fn internal_error() -> Self {
        Self::new(-32603, "Internal error".to_string(), None)
    }
}

impl std::fmt::Display for JsonRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for JsonRpcError {}

/// A JSON-RPC 2.0 response message.
///
/// Responses are sent from server to client in response to requests.
/// A response contains either a result (success) or an error (failure).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    /// JSON-RPC version, always "2.0"
    jsonrpc: String,

    /// Request ID that this response corresponds to
    #[serde(default)]
    id: Option<Value>,

    /// Result value (present on success)
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,

    /// Error object (present on failure)
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    /// Creates a successful response.
    ///
    /// # Arguments
    ///
    /// * `id` - Request ID this response corresponds to
    /// * `result` - Result value
    #[must_use]
    pub fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id: Some(id),
            result: Some(result),
            error: None,
        }
    }

    /// Creates an error response.
    ///
    /// # Arguments
    ///
    /// * `id` - Request ID this response corresponds to (may be null)
    /// * `error` - Error object
    #[must_use]
    pub fn new_error(id: Option<Value>, error: JsonRpcError) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(error),
        }
    }

    /// Returns true if this response indicates success.
    #[must_use]
    pub fn is_success(&self) -> bool {
        self.result.is_some() && self.error.is_none()
    }

    /// Returns true if this response indicates an error.
    #[must_use]
    pub fn is_error(&self) -> bool {
        self.error.is_some()
    }

    /// Returns the request ID.
    #[must_use]
    pub fn id(&self) -> Option<&Value> {
        self.id.as_ref()
    }

    /// Returns the result value, if present.
    #[must_use]
    pub fn result(&self) -> Option<&Value> {
        self.result.as_ref()
    }

    /// Returns the error object, if present.
    #[must_use]
    pub fn error(&self) -> Option<&JsonRpcError> {
        self.error.as_ref()
    }

    /// Consumes the response and returns the result or error.
    ///
    /// # Errors
    ///
    /// Returns the `JsonRpcError` if this is an error response.
    pub fn into_result(self) -> Result<Value, JsonRpcError> {
        if let Some(error) = self.error {
            Err(error)
        } else {
            Ok(self.result.unwrap_or(Value::Null))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_request_id_from_number() {
        let id: RequestId = 42i64.into();
        assert_eq!(id, RequestId::Number(42));
    }

    #[test]
    fn test_request_id_from_string() {
        let id: RequestId = "abc".into();
        assert_eq!(id, RequestId::String("abc".to_string()));
    }

    #[test]
    fn test_error_display() {
        let error = JsonRpcError::new(-32600, "Invalid Request".to_string(), None);
        assert_eq!(format!("{error}"), "[-32600] Invalid Request");
    }

    #[test]
    fn test_response_into_result_success() {
        let response = JsonRpcResponse::success(json!(1), json!({"ok": true}));
        let result = response.into_result().expect("Should be success");
        assert_eq!(result["ok"], true);
    }

    #[test]
    fn test_response_into_result_error() {
        let response = JsonRpcResponse::new_error(Some(json!(1)), JsonRpcError::invalid_request());
        let err = response.into_result().expect_err("Should be error");
        assert_eq!(err.code(), -32600);
    }
}
