//! IDE message protocol for VS Code and JetBrains extensions
//!
//! This module defines the message types for bidirectional communication
//! between Patina and IDE extensions via TCP/JSON protocol.
//!
//! # Protocol Overview
//!
//! Messages are length-prefixed JSON objects. Each message has a `type` field
//! that determines its variant.
//!
//! # Examples
//!
//! ```ignore
//! // Ping request
//! {"type": "ping"}
//!
//! // Pong response
//! {"type": "pong", "version": "0.5.0"}
//! ```

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Messages sent FROM the IDE to Patina
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IdeRequest {
    /// Health check / keep-alive
    Ping,

    /// Request current application status
    GetStatus,

    /// Send a prompt for processing
    SendPrompt {
        /// The prompt text
        text: String,
        /// Optional file context
        #[serde(skip_serializing_if = "Option::is_none")]
        file: Option<PathBuf>,
        /// Optional selected text range
        #[serde(skip_serializing_if = "Option::is_none")]
        selection: Option<TextSelection>,
    },

    /// Cancel an in-progress operation
    Cancel {
        /// Request ID to cancel
        request_id: String,
    },

    /// Apply a code edit
    ApplyEdit {
        /// Target file
        file: PathBuf,
        /// Unified diff format
        diff: String,
    },

    /// Initialize session with workspace info
    Init {
        /// Workspace root directory
        workspace: PathBuf,
        /// Supported capabilities
        capabilities: Vec<String>,
    },
}

/// Selected text range in a file
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextSelection {
    /// Start line (0-indexed)
    pub start_line: u32,
    /// Start column (0-indexed)
    pub start_column: u32,
    /// End line (0-indexed)
    pub end_line: u32,
    /// End column (0-indexed)
    pub end_column: u32,
    /// The selected text content
    pub text: String,
}

/// Messages sent FROM Patina TO the IDE
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IdeResponse {
    /// Response to Ping
    Pong {
        /// Patina version
        version: String,
    },

    /// Current application status
    Status {
        /// Whether Patina is currently processing
        busy: bool,
        /// Current conversation turn count
        turn_count: u32,
        /// Active tool executions
        active_tools: Vec<String>,
    },

    /// Acknowledgment that prompt was received
    PromptReceived {
        /// Assigned request ID for tracking
        request_id: String,
    },

    /// Streaming content update
    StreamingContent {
        /// Request ID this content belongs to
        request_id: String,
        /// Content delta
        delta: String,
        /// Whether this is the final chunk
        done: bool,
    },

    /// Tool execution notification
    ToolExecution {
        /// Request ID this belongs to
        request_id: String,
        /// Tool name
        tool: String,
        /// Tool input (JSON)
        input: serde_json::Value,
        /// Tool output (if completed)
        #[serde(skip_serializing_if = "Option::is_none")]
        output: Option<String>,
    },

    /// Edit proposal for user review
    EditProposal {
        /// Request ID this belongs to
        request_id: String,
        /// Target file
        file: PathBuf,
        /// Unified diff
        diff: String,
        /// Human-readable description
        description: String,
    },

    /// Error response
    Error {
        /// Error code
        code: String,
        /// Human-readable message
        message: String,
        /// Optional request ID if error relates to specific request
        #[serde(skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
    },

    /// Cancellation acknowledgment
    Cancelled {
        /// Request ID that was cancelled
        request_id: String,
    },

    /// Session initialized successfully
    InitAck {
        /// Assigned session ID
        session_id: String,
        /// Patina version
        version: String,
        /// Server capabilities
        capabilities: Vec<String>,
    },
}

/// Parse an IDE request from JSON bytes
///
/// # Errors
///
/// Returns an error if the JSON is malformed or missing required fields.
pub fn parse_request(data: &[u8]) -> Result<IdeRequest, serde_json::Error> {
    serde_json::from_slice(data)
}

/// Serialize an IDE response to JSON bytes
///
/// # Errors
///
/// Returns an error if serialization fails (should not happen for valid data).
pub fn serialize_response(response: &IdeResponse) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(response)
}

/// Protocol version for compatibility checks
pub const PROTOCOL_VERSION: &str = "1.0";

/// Default server capabilities
pub fn default_capabilities() -> Vec<String> {
    vec![
        "streaming".to_string(),
        "tool_execution".to_string(),
        "edit_proposal".to_string(),
        "cancel".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // IdeRequest parsing tests
    // =========================================================================

    #[test]
    fn test_parse_ping_request() {
        let json = r#"{"type": "ping"}"#;
        let request = parse_request(json.as_bytes()).unwrap();
        assert_eq!(request, IdeRequest::Ping);
    }

    #[test]
    fn test_parse_get_status_request() {
        let json = r#"{"type": "get_status"}"#;
        let request = parse_request(json.as_bytes()).unwrap();
        assert_eq!(request, IdeRequest::GetStatus);
    }

    #[test]
    fn test_parse_send_prompt_minimal() {
        let json = r#"{"type": "send_prompt", "text": "Hello, Claude!"}"#;
        let request = parse_request(json.as_bytes()).unwrap();
        assert_eq!(
            request,
            IdeRequest::SendPrompt {
                text: "Hello, Claude!".to_string(),
                file: None,
                selection: None,
            }
        );
    }

    #[test]
    fn test_parse_send_prompt_with_file() {
        let json = r#"{"type": "send_prompt", "text": "Explain this", "file": "src/main.rs"}"#;
        let request = parse_request(json.as_bytes()).unwrap();
        assert_eq!(
            request,
            IdeRequest::SendPrompt {
                text: "Explain this".to_string(),
                file: Some(PathBuf::from("src/main.rs")),
                selection: None,
            }
        );
    }

    #[test]
    fn test_parse_send_prompt_with_selection() {
        let json = r#"{
            "type": "send_prompt",
            "text": "Refactor this function",
            "file": "src/lib.rs",
            "selection": {
                "start_line": 10,
                "start_column": 0,
                "end_line": 20,
                "end_column": 1,
                "text": "fn foo() {}"
            }
        }"#;
        let request = parse_request(json.as_bytes()).unwrap();
        match request {
            IdeRequest::SendPrompt {
                text,
                file,
                selection,
            } => {
                assert_eq!(text, "Refactor this function");
                assert_eq!(file, Some(PathBuf::from("src/lib.rs")));
                let sel = selection.unwrap();
                assert_eq!(sel.start_line, 10);
                assert_eq!(sel.end_line, 20);
                assert_eq!(sel.text, "fn foo() {}");
            }
            _ => panic!("Expected SendPrompt"),
        }
    }

    #[test]
    fn test_parse_cancel_request() {
        let json = r#"{"type": "cancel", "request_id": "req-123"}"#;
        let request = parse_request(json.as_bytes()).unwrap();
        assert_eq!(
            request,
            IdeRequest::Cancel {
                request_id: "req-123".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_apply_edit_request() {
        let json = r#"{"type": "apply_edit", "file": "src/foo.rs", "diff": "@@ -1,3 +1,3 @@\n-old\n+new"}"#;
        let request = parse_request(json.as_bytes()).unwrap();
        assert_eq!(
            request,
            IdeRequest::ApplyEdit {
                file: PathBuf::from("src/foo.rs"),
                diff: "@@ -1,3 +1,3 @@\n-old\n+new".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_init_request() {
        let json =
            r#"{"type": "init", "workspace": "/home/user/project", "capabilities": ["streaming"]}"#;
        let request = parse_request(json.as_bytes()).unwrap();
        assert_eq!(
            request,
            IdeRequest::Init {
                workspace: PathBuf::from("/home/user/project"),
                capabilities: vec!["streaming".to_string()],
            }
        );
    }

    #[test]
    fn test_parse_invalid_json() {
        let json = r#"{"type": "ping""#; // Missing closing brace
        let result = parse_request(json.as_bytes());
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_unknown_type() {
        let json = r#"{"type": "unknown_message_type"}"#;
        let result = parse_request(json.as_bytes());
        assert!(result.is_err());
    }

    // =========================================================================
    // IdeResponse serialization tests
    // =========================================================================

    #[test]
    fn test_serialize_pong_response() {
        let response = IdeResponse::Pong {
            version: "0.5.0".to_string(),
        };
        let bytes = serialize_response(&response).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["type"], "pong");
        assert_eq!(json["version"], "0.5.0");
    }

    #[test]
    fn test_serialize_status_response() {
        let response = IdeResponse::Status {
            busy: true,
            turn_count: 5,
            active_tools: vec!["bash".to_string(), "read_file".to_string()],
        };
        let bytes = serialize_response(&response).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["type"], "status");
        assert_eq!(json["busy"], true);
        assert_eq!(json["turn_count"], 5);
        assert_eq!(json["active_tools"][0], "bash");
        assert_eq!(json["active_tools"][1], "read_file");
    }

    #[test]
    fn test_serialize_prompt_received_response() {
        let response = IdeResponse::PromptReceived {
            request_id: "req-abc".to_string(),
        };
        let bytes = serialize_response(&response).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["type"], "prompt_received");
        assert_eq!(json["request_id"], "req-abc");
    }

    #[test]
    fn test_serialize_streaming_content_response() {
        let response = IdeResponse::StreamingContent {
            request_id: "req-123".to_string(),
            delta: "Hello ".to_string(),
            done: false,
        };
        let bytes = serialize_response(&response).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["type"], "streaming_content");
        assert_eq!(json["request_id"], "req-123");
        assert_eq!(json["delta"], "Hello ");
        assert_eq!(json["done"], false);
    }

    #[test]
    fn test_serialize_error_response() {
        let response = IdeResponse::Error {
            code: "INVALID_REQUEST".to_string(),
            message: "Missing required field".to_string(),
            request_id: Some("req-456".to_string()),
        };
        let bytes = serialize_response(&response).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["type"], "error");
        assert_eq!(json["code"], "INVALID_REQUEST");
        assert_eq!(json["message"], "Missing required field");
        assert_eq!(json["request_id"], "req-456");
    }

    #[test]
    fn test_serialize_error_response_without_request_id() {
        let response = IdeResponse::Error {
            code: "SERVER_ERROR".to_string(),
            message: "Internal error".to_string(),
            request_id: None,
        };
        let bytes = serialize_response(&response).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["type"], "error");
        assert!(json.get("request_id").is_none());
    }

    #[test]
    fn test_serialize_cancelled_response() {
        let response = IdeResponse::Cancelled {
            request_id: "req-789".to_string(),
        };
        let bytes = serialize_response(&response).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["type"], "cancelled");
        assert_eq!(json["request_id"], "req-789");
    }

    #[test]
    fn test_serialize_init_ack_response() {
        let response = IdeResponse::InitAck {
            session_id: "sess-001".to_string(),
            version: "0.5.0".to_string(),
            capabilities: vec!["streaming".to_string(), "cancel".to_string()],
        };
        let bytes = serialize_response(&response).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["type"], "init_ack");
        assert_eq!(json["session_id"], "sess-001");
        assert_eq!(json["version"], "0.5.0");
        assert_eq!(json["capabilities"][0], "streaming");
    }

    #[test]
    fn test_serialize_edit_proposal_response() {
        let response = IdeResponse::EditProposal {
            request_id: "req-edit".to_string(),
            file: PathBuf::from("src/lib.rs"),
            diff: "-old\n+new".to_string(),
            description: "Rename variable".to_string(),
        };
        let bytes = serialize_response(&response).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["type"], "edit_proposal");
        assert_eq!(json["file"], "src/lib.rs");
        assert_eq!(json["description"], "Rename variable");
    }

    #[test]
    fn test_serialize_tool_execution_response() {
        let response = IdeResponse::ToolExecution {
            request_id: "req-tool".to_string(),
            tool: "bash".to_string(),
            input: serde_json::json!({"command": "ls -la"}),
            output: Some("file1.rs\nfile2.rs".to_string()),
        };
        let bytes = serialize_response(&response).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["type"], "tool_execution");
        assert_eq!(json["tool"], "bash");
        assert_eq!(json["input"]["command"], "ls -la");
        assert_eq!(json["output"], "file1.rs\nfile2.rs");
    }

    // =========================================================================
    // Round-trip tests
    // =========================================================================

    #[test]
    fn test_request_roundtrip() {
        let original = IdeRequest::SendPrompt {
            text: "Test prompt".to_string(),
            file: Some(PathBuf::from("test.rs")),
            selection: None,
        };
        let bytes = serde_json::to_vec(&original).unwrap();
        let parsed: IdeRequest = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(original, parsed);
    }

    #[test]
    fn test_response_roundtrip() {
        let original = IdeResponse::Status {
            busy: false,
            turn_count: 0,
            active_tools: vec![],
        };
        let bytes = serialize_response(&original).unwrap();
        let parsed: IdeResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(original, parsed);
    }

    // =========================================================================
    // Helper function tests
    // =========================================================================

    #[test]
    fn test_default_capabilities() {
        let caps = default_capabilities();
        assert!(caps.contains(&"streaming".to_string()));
        assert!(caps.contains(&"tool_execution".to_string()));
        assert!(caps.contains(&"cancel".to_string()));
    }

    #[test]
    fn test_protocol_version() {
        assert_eq!(PROTOCOL_VERSION, "1.0");
    }
}
