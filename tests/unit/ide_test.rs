//! Tests for IDE integration

use rct::ide::{IdeMessage, IdeServer, Selection};
use std::path::PathBuf;
use uuid::Uuid;

#[test]
fn test_ide_server_new() {
    let server = IdeServer::new(9000);
    // Server should not be listening yet
    assert!(server.get_session(Uuid::new_v4()).is_none());
}

#[test]
fn test_ide_server_register_session() {
    let mut server = IdeServer::new(9000);
    let session_id = Uuid::new_v4();
    let workspace = PathBuf::from("/tmp/workspace");
    let capabilities = vec!["edit".to_string(), "diff".to_string()];

    server.register_session(session_id, workspace.clone(), capabilities.clone());

    let session = server.get_session(session_id);
    assert!(session.is_some());
    let session = session.unwrap();
    assert_eq!(session.id, session_id);
    assert_eq!(session.workspace, workspace);
    assert_eq!(session.capabilities, capabilities);
}

#[test]
fn test_ide_server_remove_session() {
    let mut server = IdeServer::new(9000);
    let session_id = Uuid::new_v4();
    let workspace = PathBuf::from("/tmp/workspace");

    server.register_session(session_id, workspace, vec![]);
    assert!(server.get_session(session_id).is_some());

    server.remove_session(session_id);
    assert!(server.get_session(session_id).is_none());
}

#[test]
fn test_ide_server_multiple_sessions() {
    let mut server = IdeServer::new(9000);
    let session1 = Uuid::new_v4();
    let session2 = Uuid::new_v4();

    server.register_session(
        session1,
        PathBuf::from("/workspace1"),
        vec!["edit".to_string()],
    );
    server.register_session(
        session2,
        PathBuf::from("/workspace2"),
        vec!["diff".to_string()],
    );

    let s1 = server.get_session(session1).unwrap();
    assert_eq!(s1.workspace, PathBuf::from("/workspace1"));

    let s2 = server.get_session(session2).unwrap();
    assert_eq!(s2.workspace, PathBuf::from("/workspace2"));
}

#[test]
fn test_ide_message_init_serialization() {
    let msg = IdeMessage::Init {
        workspace: PathBuf::from("/project"),
        capabilities: vec!["edit".to_string(), "diff".to_string()],
    };

    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("\"type\":\"init\""));
    assert!(json.contains("/project"));

    let deserialized: IdeMessage = serde_json::from_str(&json).unwrap();
    match deserialized {
        IdeMessage::Init {
            workspace,
            capabilities,
        } => {
            assert_eq!(workspace, PathBuf::from("/project"));
            assert_eq!(capabilities, vec!["edit", "diff"]);
        }
        _ => panic!("Expected Init message"),
    }
}

#[test]
fn test_ide_message_prompt_serialization() {
    let msg = IdeMessage::Prompt {
        text: "Explain this code".to_string(),
        selection: Some(Selection {
            file: PathBuf::from("src/main.rs"),
            start_line: 10,
            start_column: 0,
            end_line: 20,
            end_column: 0,
            text: "fn main() {}".to_string(),
        }),
    };

    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("\"type\":\"prompt\""));
    assert!(json.contains("Explain this code"));

    let deserialized: IdeMessage = serde_json::from_str(&json).unwrap();
    match deserialized {
        IdeMessage::Prompt { text, selection } => {
            assert_eq!(text, "Explain this code");
            assert!(selection.is_some());
            let sel = selection.unwrap();
            assert_eq!(sel.start_line, 10);
            assert_eq!(sel.end_line, 20);
        }
        _ => panic!("Expected Prompt message"),
    }
}

#[test]
fn test_ide_message_prompt_without_selection() {
    let msg = IdeMessage::Prompt {
        text: "Hello".to_string(),
        selection: None,
    };

    let json = serde_json::to_string(&msg).unwrap();
    let deserialized: IdeMessage = serde_json::from_str(&json).unwrap();
    match deserialized {
        IdeMessage::Prompt { text, selection } => {
            assert_eq!(text, "Hello");
            assert!(selection.is_none());
        }
        _ => panic!("Expected Prompt message"),
    }
}

#[test]
fn test_ide_message_apply_edit_serialization() {
    let msg = IdeMessage::ApplyEdit {
        file: PathBuf::from("src/lib.rs"),
        diff: "@@ -1,3 +1,4 @@".to_string(),
    };

    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("\"type\":\"apply_edit\""));

    let deserialized: IdeMessage = serde_json::from_str(&json).unwrap();
    match deserialized {
        IdeMessage::ApplyEdit { file, diff } => {
            assert_eq!(file, PathBuf::from("src/lib.rs"));
            assert!(diff.contains("@@"));
        }
        _ => panic!("Expected ApplyEdit message"),
    }
}

#[test]
fn test_ide_message_streaming_content_serialization() {
    let msg = IdeMessage::StreamingContent {
        delta: "Hello ".to_string(),
    };

    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("\"type\":\"streaming_content\""));

    let deserialized: IdeMessage = serde_json::from_str(&json).unwrap();
    match deserialized {
        IdeMessage::StreamingContent { delta } => {
            assert_eq!(delta, "Hello ");
        }
        _ => panic!("Expected StreamingContent message"),
    }
}

#[test]
fn test_ide_message_edit_proposal_serialization() {
    let msg = IdeMessage::EditProposal {
        file: PathBuf::from("src/main.rs"),
        diff: "- old\n+ new".to_string(),
        description: "Fix bug".to_string(),
    };

    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("\"type\":\"edit_proposal\""));
    assert!(json.contains("Fix bug"));

    let deserialized: IdeMessage = serde_json::from_str(&json).unwrap();
    match deserialized {
        IdeMessage::EditProposal {
            file,
            diff,
            description,
        } => {
            assert_eq!(file, PathBuf::from("src/main.rs"));
            assert_eq!(diff, "- old\n+ new");
            assert_eq!(description, "Fix bug");
        }
        _ => panic!("Expected EditProposal message"),
    }
}

#[test]
fn test_ide_message_tool_use_serialization() {
    let msg = IdeMessage::ToolUse {
        tool: "bash".to_string(),
        input: serde_json::json!({"command": "ls -la"}),
    };

    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("\"type\":\"tool_use\""));
    assert!(json.contains("bash"));

    let deserialized: IdeMessage = serde_json::from_str(&json).unwrap();
    match deserialized {
        IdeMessage::ToolUse { tool, input } => {
            assert_eq!(tool, "bash");
            assert_eq!(input["command"], "ls -la");
        }
        _ => panic!("Expected ToolUse message"),
    }
}

#[test]
fn test_selection_serialization() {
    let selection = Selection {
        file: PathBuf::from("test.rs"),
        start_line: 1,
        start_column: 0,
        end_line: 10,
        end_column: 50,
        text: "selected text".to_string(),
    };

    let json = serde_json::to_string(&selection).unwrap();
    let deserialized: Selection = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.file, PathBuf::from("test.rs"));
    assert_eq!(deserialized.start_line, 1);
    assert_eq!(deserialized.end_column, 50);
    assert_eq!(deserialized.text, "selected text");
}

#[tokio::test]
async fn test_ide_server_start() {
    let _server = IdeServer::new(0); // Port 0 = random available port
                                     // This test verifies basic construction; actual server start needs integration tests
}
