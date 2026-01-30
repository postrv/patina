//! Integration tests for session persistence.
//!
//! Tests session save/load functionality including:
//! - Saving session state to disk
//! - Resuming sessions from saved state
//! - Session metadata handling

use rct::session::{Session, SessionManager};
use rct::types::message::{Message, Role};
use std::path::PathBuf;
use tempfile::TempDir;
use uuid::Uuid;

// =============================================================================
// Helper functions
// =============================================================================

/// Creates a test message with the given role and content.
fn test_message(role: Role, content: &str) -> Message {
    Message {
        role,
        content: content.to_string(),
    }
}

/// Creates a sample conversation with multiple messages.
fn sample_conversation() -> Vec<Message> {
    vec![
        test_message(Role::User, "Hello, Claude!"),
        test_message(Role::Assistant, "Hello! How can I help you today?"),
        test_message(Role::User, "Can you explain Rust ownership?"),
        test_message(
            Role::Assistant,
            "Certainly! Ownership is one of Rust's most distinctive features...",
        ),
    ]
}

// =============================================================================
// 7.2.1 Session save tests
// =============================================================================

/// Test that a session can be saved to disk.
#[tokio::test]
async fn test_session_save() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let manager = SessionManager::new(temp_dir.path().to_path_buf());

    // Create a session with some messages
    let mut session = Session::new(PathBuf::from("/test/project"));
    for msg in sample_conversation() {
        session.add_message(msg);
    }

    // Save the session
    let session_id = manager
        .save(&session)
        .await
        .expect("Failed to save session");

    // Verify the session file exists
    let session_file = temp_dir.path().join(format!("{}.json", session_id));
    assert!(session_file.exists(), "Session file should exist");
}

/// Test that session metadata is preserved when saving.
#[tokio::test]
async fn test_session_save_metadata() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let manager = SessionManager::new(temp_dir.path().to_path_buf());

    let working_dir = PathBuf::from("/test/my-project");
    let mut session = Session::new(working_dir.clone());
    session.add_message(test_message(Role::User, "Test message"));

    let session_id = manager
        .save(&session)
        .await
        .expect("Failed to save session");

    // Load and verify metadata
    let loaded = manager
        .load(&session_id)
        .await
        .expect("Failed to load session");
    assert_eq!(loaded.working_dir(), &working_dir);
    assert!(loaded.created_at() <= loaded.updated_at());
}

/// Test that saving updates the session ID.
#[tokio::test]
async fn test_session_save_assigns_id() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let manager = SessionManager::new(temp_dir.path().to_path_buf());

    let session = Session::new(PathBuf::from("/test"));
    let session_id = manager
        .save(&session)
        .await
        .expect("Failed to save session");

    // Verify the ID is a valid UUID
    assert!(
        Uuid::parse_str(&session_id).is_ok(),
        "Session ID should be a valid UUID"
    );
}

/// Test that multiple sessions can be saved.
#[tokio::test]
async fn test_session_save_multiple() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let manager = SessionManager::new(temp_dir.path().to_path_buf());

    let session1 = Session::new(PathBuf::from("/project1"));
    let session2 = Session::new(PathBuf::from("/project2"));

    let id1 = manager
        .save(&session1)
        .await
        .expect("Failed to save session 1");
    let id2 = manager
        .save(&session2)
        .await
        .expect("Failed to save session 2");

    assert_ne!(id1, id2, "Session IDs should be unique");

    // Both sessions should be listable
    let sessions = manager.list().await.expect("Failed to list sessions");
    assert_eq!(sessions.len(), 2);
}

// =============================================================================
// 7.2.1 Session resume tests
// =============================================================================

/// Test that a session can be resumed from disk.
#[tokio::test]
async fn test_session_resume() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let manager = SessionManager::new(temp_dir.path().to_path_buf());

    // Create and save a session
    let mut original = Session::new(PathBuf::from("/test/project"));
    for msg in sample_conversation() {
        original.add_message(msg);
    }
    let session_id = manager
        .save(&original)
        .await
        .expect("Failed to save session");

    // Resume the session
    let resumed = manager
        .load(&session_id)
        .await
        .expect("Failed to resume session");

    // Verify messages are preserved
    assert_eq!(
        resumed.messages().len(),
        original.messages().len(),
        "Message count should match"
    );

    for (orig, resumed) in original.messages().iter().zip(resumed.messages().iter()) {
        assert_eq!(orig.role, resumed.role);
        assert_eq!(orig.content, resumed.content);
    }
}

/// Test that resuming a non-existent session returns an error.
#[tokio::test]
async fn test_session_resume_nonexistent() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let manager = SessionManager::new(temp_dir.path().to_path_buf());

    let result = manager.load("nonexistent-session-id").await;
    assert!(result.is_err(), "Loading nonexistent session should fail");
}

/// Test that session list returns all saved sessions.
#[tokio::test]
async fn test_session_list() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let manager = SessionManager::new(temp_dir.path().to_path_buf());

    // Initially empty
    let sessions = manager.list().await.expect("Failed to list sessions");
    assert!(sessions.is_empty());

    // Add some sessions
    let session1 = Session::new(PathBuf::from("/project1"));
    let session2 = Session::new(PathBuf::from("/project2"));
    manager
        .save(&session1)
        .await
        .expect("Failed to save session 1");
    manager
        .save(&session2)
        .await
        .expect("Failed to save session 2");

    // Should list both
    let sessions = manager.list().await.expect("Failed to list sessions");
    assert_eq!(sessions.len(), 2);
}

/// Test that session can be deleted.
#[tokio::test]
async fn test_session_delete() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let manager = SessionManager::new(temp_dir.path().to_path_buf());

    let session = Session::new(PathBuf::from("/test"));
    let session_id = manager
        .save(&session)
        .await
        .expect("Failed to save session");

    // Delete the session
    manager
        .delete(&session_id)
        .await
        .expect("Failed to delete session");

    // Should not be loadable anymore
    let result = manager.load(&session_id).await;
    assert!(result.is_err(), "Deleted session should not be loadable");
}

/// Test that session updates preserve the same ID.
#[tokio::test]
async fn test_session_update() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let manager = SessionManager::new(temp_dir.path().to_path_buf());

    // Create and save
    let mut session = Session::new(PathBuf::from("/test"));
    session.add_message(test_message(Role::User, "First message"));
    let session_id = manager
        .save(&session)
        .await
        .expect("Failed to save session");

    // Load, modify, and save again
    let mut loaded = manager
        .load(&session_id)
        .await
        .expect("Failed to load session");
    loaded.add_message(test_message(Role::Assistant, "Response"));
    manager
        .update(&session_id, &loaded)
        .await
        .expect("Failed to update session");

    // Verify the update persisted
    let final_session = manager
        .load(&session_id)
        .await
        .expect("Failed to load final session");
    assert_eq!(final_session.messages().len(), 2);
}

/// Test that session metadata includes working directory.
#[tokio::test]
async fn test_session_metadata_working_dir() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let manager = SessionManager::new(temp_dir.path().to_path_buf());

    let working_dir = PathBuf::from("/home/user/my-project");
    let session = Session::new(working_dir.clone());
    let session_id = manager
        .save(&session)
        .await
        .expect("Failed to save session");

    let metadata = manager
        .get_metadata(&session_id)
        .await
        .expect("Failed to get metadata");
    assert_eq!(metadata.working_dir, working_dir);
}

/// Test listing sessions with metadata.
#[tokio::test]
async fn test_session_list_with_metadata() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let manager = SessionManager::new(temp_dir.path().to_path_buf());

    let mut session = Session::new(PathBuf::from("/test"));
    session.add_message(test_message(Role::User, "Hello"));
    manager.save(&session).await.expect("Failed to save");

    let sessions = manager
        .list_with_metadata()
        .await
        .expect("Failed to list with metadata");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].1.message_count, 1);
}

// =============================================================================
// 2.3 Session Integrity Tests
// =============================================================================

/// Test that session detects tampering (modified content).
#[tokio::test]
async fn test_session_detects_tampering() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let manager = SessionManager::new(temp_dir.path().to_path_buf());

    // Create and save a valid session
    let mut session = Session::new(PathBuf::from("/test"));
    session.add_message(test_message(Role::User, "Original message"));
    let session_id = manager
        .save(&session)
        .await
        .expect("Failed to save session");

    // Manually tamper with the session file
    let session_file = temp_dir.path().join(format!("{}.json", session_id));
    let original_content = std::fs::read_to_string(&session_file).expect("Failed to read file");

    // Modify the message content
    let tampered_content = original_content.replace("Original message", "TAMPERED MESSAGE");
    std::fs::write(&session_file, tampered_content).expect("Failed to write tampered file");

    // Loading should fail due to integrity check
    let result = manager.load(&session_id).await;
    assert!(
        result.is_err(),
        "Loading tampered session should fail integrity check"
    );
}

/// Test that session validates schema (rejects invalid JSON structure).
#[tokio::test]
async fn test_session_validates_schema() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let manager = SessionManager::new(temp_dir.path().to_path_buf());

    // Create a session to get a valid ID
    let session = Session::new(PathBuf::from("/test"));
    let session_id = manager
        .save(&session)
        .await
        .expect("Failed to save session");

    // Overwrite with invalid schema (missing required fields)
    let session_file = temp_dir.path().join(format!("{}.json", session_id));
    let invalid_json = r#"{"invalid": "schema", "no_messages": true}"#;
    std::fs::write(&session_file, invalid_json).expect("Failed to write invalid file");

    // Loading should fail due to schema validation
    let result = manager.load(&session_id).await;
    assert!(
        result.is_err(),
        "Loading session with invalid schema should fail"
    );
}

// =============================================================================
// 3.4.1 Session Error Path Tests
// =============================================================================

/// Test that loading corrupted JSON returns an appropriate error.
#[tokio::test]
async fn test_session_load_corrupted_json() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let manager = SessionManager::new(temp_dir.path().to_path_buf());

    // Create a session to get a valid path
    let session = Session::new(PathBuf::from("/test"));
    let session_id = manager
        .save(&session)
        .await
        .expect("Failed to save session");

    // Overwrite with completely corrupted (non-JSON) content
    let session_file = temp_dir.path().join(format!("{}.json", session_id));
    std::fs::write(&session_file, "this is not json at all {{{ invalid").expect("Failed to write");

    // Loading should fail with deserialization error
    let result = manager.load(&session_id).await;
    assert!(result.is_err(), "Loading corrupted JSON should fail");

    let err = result.unwrap_err();
    let err_str = err.to_string().to_lowercase();
    assert!(
        err_str.contains("deserialize")
            || err_str.contains("json")
            || err_str.contains("parse")
            || err_str.contains("expected"),
        "Error should indicate JSON parsing failure: {}",
        err
    );
}

/// Test that saving to a read-only directory returns an appropriate error.
#[tokio::test]
#[cfg(unix)]
async fn test_session_save_permission_denied() {
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let readonly_dir = temp_dir.path().join("readonly");
    std::fs::create_dir(&readonly_dir).expect("Failed to create dir");

    // Make the directory read-only
    let mut perms = std::fs::metadata(&readonly_dir)
        .expect("Failed to get metadata")
        .permissions();
    perms.set_mode(0o444); // r--r--r--
    std::fs::set_permissions(&readonly_dir, perms).expect("Failed to set permissions");

    let manager = SessionManager::new(readonly_dir.clone());
    let session = Session::new(PathBuf::from("/test"));

    // Saving should fail due to permission denied
    let result = manager.save(&session).await;

    // Restore permissions for cleanup
    let mut perms = std::fs::metadata(&readonly_dir)
        .expect("Failed to get metadata")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&readonly_dir, perms).ok();

    assert!(result.is_err(), "Saving to read-only directory should fail");

    let err = result.unwrap_err();
    let err_str = err.to_string().to_lowercase();
    assert!(
        err_str.contains("permission")
            || err_str.contains("denied")
            || err_str.contains("failed")
            || err_str.contains("write"),
        "Error should indicate permission issue: {}",
        err
    );
}

/// Test that concurrent saves to the same session don't corrupt data.
///
/// This test verifies that when multiple saves happen concurrently,
/// all operations complete without panics and the final state is consistent.
#[tokio::test]
async fn test_session_concurrent_access() {
    use std::sync::Arc;
    use tokio::sync::Barrier;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let sessions_dir = temp_dir.path().to_path_buf();

    // Create an initial session
    let manager = SessionManager::new(sessions_dir.clone());
    let mut session = Session::new(PathBuf::from("/test"));
    session.add_message(test_message(Role::User, "Initial message"));
    let session_id = manager
        .save(&session)
        .await
        .expect("Failed to save initial session");

    // Launch multiple concurrent updates
    let num_concurrent = 5;
    let barrier = Arc::new(Barrier::new(num_concurrent));
    let mut handles = Vec::new();

    for i in 0..num_concurrent {
        let sessions_dir = sessions_dir.clone();
        let session_id = session_id.clone();
        let barrier = Arc::clone(&barrier);

        let handle = tokio::spawn(async move {
            let manager = SessionManager::new(sessions_dir);

            // Wait for all tasks to be ready
            barrier.wait().await;

            // Load, modify, and save
            let loaded = manager.load(&session_id).await;
            if let Ok(mut loaded) = loaded {
                loaded.add_message(test_message(
                    Role::Assistant,
                    &format!("Concurrent update {}", i),
                ));
                // Update may fail due to concurrent access, that's ok
                let _ = manager.update(&session_id, &loaded).await;
            }
        });
        handles.push(handle);
    }

    // Wait for all tasks to complete
    for handle in handles {
        let result = handle.await;
        assert!(result.is_ok(), "Concurrent task should not panic");
    }

    // Verify the session is still loadable and not corrupted
    let final_session = manager
        .load(&session_id)
        .await
        .expect("Session should be loadable after concurrent access");

    // Should have at least the initial message
    assert!(
        !final_session.messages().is_empty(),
        "Session should have at least one message"
    );

    // The session ID should be preserved
    assert_eq!(
        final_session.id(),
        Some(session_id.as_str()),
        "Session ID should be preserved"
    );
}

/// Test that concurrent reads don't cause issues.
#[tokio::test]
async fn test_session_concurrent_reads() {
    use std::sync::Arc;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let sessions_dir = temp_dir.path().to_path_buf();

    // Create a session with some content
    let manager = SessionManager::new(sessions_dir.clone());
    let mut session = Session::new(PathBuf::from("/test"));
    for i in 0..10 {
        session.add_message(test_message(Role::User, &format!("Message {}", i)));
    }
    let session_id = manager
        .save(&session)
        .await
        .expect("Failed to save session");

    // Launch multiple concurrent reads
    let num_concurrent = 10;
    let session_id = Arc::new(session_id);
    let mut handles = Vec::new();

    for _ in 0..num_concurrent {
        let sessions_dir = sessions_dir.clone();
        let session_id = Arc::clone(&session_id);

        let handle = tokio::spawn(async move {
            let manager = SessionManager::new(sessions_dir);
            manager.load(&session_id).await
        });
        handles.push(handle);
    }

    // All reads should succeed
    for handle in handles {
        let result = handle.await.expect("Task should not panic");
        assert!(
            result.is_ok(),
            "Concurrent read should succeed: {:?}",
            result
        );

        let loaded = result.unwrap();
        assert_eq!(loaded.messages().len(), 10, "Should have 10 messages");
    }
}
