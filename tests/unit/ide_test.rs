//! Tests for IDE integration protocol types
//!
//! The primary IDE protocol types live in `patina::ide::protocol`.
//! Comprehensive serialization and round-trip tests are in `protocol.rs`.
//! Controller integration tests live in the integration test suite.

use patina::ide::protocol::{
    default_capabilities, parse_request, serialize_response, IdeRequest, IdeResponse,
    TextSelection, PROTOCOL_VERSION,
};
use std::path::PathBuf;

#[test]
fn test_protocol_version_is_set() {
    assert!(!PROTOCOL_VERSION.is_empty());
}

#[test]
fn test_default_capabilities_non_empty() {
    let caps = default_capabilities();
    assert!(!caps.is_empty(), "Default capabilities should not be empty");
    assert!(caps.contains(&"streaming".to_string()));
}

#[test]
fn test_send_prompt_with_selection_roundtrip() {
    let request = IdeRequest::SendPrompt {
        text: "Explain this code".to_string(),
        file: Some(PathBuf::from("src/main.rs")),
        selection: Some(TextSelection {
            start_line: 10,
            start_column: 0,
            end_line: 20,
            end_column: 0,
            text: "fn main() {}".to_string(),
        }),
    };

    let json = serde_json::to_vec(&request).unwrap();
    let deserialized: IdeRequest = serde_json::from_slice(&json).unwrap();
    assert_eq!(request, deserialized);
}

#[test]
fn test_send_prompt_without_selection_roundtrip() {
    let request = IdeRequest::SendPrompt {
        text: "Hello".to_string(),
        file: None,
        selection: None,
    };

    let json = serde_json::to_vec(&request).unwrap();
    let deserialized: IdeRequest = serde_json::from_slice(&json).unwrap();
    assert_eq!(request, deserialized);
}

#[test]
fn test_init_request_serialization() {
    let json = r#"{"type":"init","workspace":"/project","capabilities":["edit","diff"]}"#;
    let request = parse_request(json.as_bytes()).unwrap();
    match request {
        IdeRequest::Init {
            workspace,
            capabilities,
        } => {
            assert_eq!(workspace, PathBuf::from("/project"));
            assert_eq!(capabilities, vec!["edit", "diff"]);
        }
        _ => panic!("Expected Init request"),
    }
}

#[test]
fn test_streaming_content_response_serialization() {
    let response = IdeResponse::StreamingContent {
        request_id: "req-1".to_string(),
        delta: "Hello ".to_string(),
        done: false,
    };

    let bytes = serialize_response(&response).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["type"], "streaming_content");
    assert_eq!(json["delta"], "Hello ");
}

#[test]
fn test_edit_proposal_response_serialization() {
    let response = IdeResponse::EditProposal {
        request_id: "req-2".to_string(),
        file: PathBuf::from("src/main.rs"),
        diff: "- old\n+ new".to_string(),
        description: "Fix bug".to_string(),
    };

    let bytes = serialize_response(&response).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["type"], "edit_proposal");
    assert_eq!(json["description"], "Fix bug");
}

#[test]
fn test_tool_execution_response_serialization() {
    let response = IdeResponse::ToolExecution {
        request_id: "req-3".to_string(),
        tool: "bash".to_string(),
        input: serde_json::json!({"command": "ls -la"}),
        output: None,
    };

    let bytes = serialize_response(&response).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["type"], "tool_execution");
    assert_eq!(json["tool"], "bash");
    assert!(json.get("output").is_none());
}

#[test]
fn test_text_selection_fields() {
    let selection = TextSelection {
        start_line: 1,
        start_column: 0,
        end_line: 10,
        end_column: 50,
        text: "selected text".to_string(),
    };

    let json = serde_json::to_string(&selection).unwrap();
    let deserialized: TextSelection = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.start_line, 1);
    assert_eq!(deserialized.end_column, 50);
    assert_eq!(deserialized.text, "selected text");
}

#[test]
fn test_authenticate_request_parsing() {
    let json = r#"{"type": "authenticate", "token": "test-auth-value"}"#;
    let request = parse_request(json.as_bytes()).unwrap();
    assert_eq!(
        request,
        IdeRequest::Authenticate {
            token: "test-auth-value".to_string(),
        }
    );
}
