//! Integration tests for enterprise audit logging.
//!
//! Tests audit logging functionality including:
//! - Tool usage logging
//! - API call logging
//! - Log persistence and querying
//! - Audit configuration

use rct::enterprise::audit::{
    AuditConfig, AuditEntry, AuditEvent, AuditLevel, AuditLogger, AuditQuery,
};
use std::path::PathBuf;
use tempfile::TempDir;

// =============================================================================
// Helper functions
// =============================================================================

/// Creates a test audit logger with file output.
fn file_logger(dir: &TempDir) -> AuditLogger {
    let config = AuditConfig {
        enabled: true,
        level: AuditLevel::All,
        output_path: Some(dir.path().join("audit.log")),
        json_format: true,
        include_timestamps: true,
        include_session_id: true,
    };
    AuditLogger::new(config)
}

/// Creates a memory-only audit logger for testing.
fn memory_logger() -> AuditLogger {
    let config = AuditConfig {
        enabled: true,
        level: AuditLevel::All,
        output_path: None,
        json_format: false,
        include_timestamps: true,
        include_session_id: true,
    };
    AuditLogger::new(config)
}

// =============================================================================
// 7.4.1 Audit logging tests
// =============================================================================

/// Test that tool usage is logged.
#[tokio::test]
async fn test_audit_log_tool_use() {
    let mut logger = memory_logger();

    let entry = AuditEntry::tool_use(
        "session-123",
        "Bash",
        serde_json::json!({"command": "ls -la"}),
        Some(serde_json::json!({"output": "file1\nfile2"})),
        std::time::Duration::from_millis(150),
    );

    logger.log(entry).await.expect("Failed to log");

    let entries = logger.entries();
    assert_eq!(entries.len(), 1);

    match &entries[0].event {
        AuditEvent::ToolUse {
            tool_name,
            duration_ms,
            ..
        } => {
            assert_eq!(tool_name, "Bash");
            assert_eq!(*duration_ms, 150);
        }
        _ => panic!("Expected ToolUse event"),
    }
}

/// Test that API calls are logged.
#[tokio::test]
async fn test_audit_log_api_call() {
    let mut logger = memory_logger();

    let entry = AuditEntry::api_call(
        "session-123",
        "claude-3-opus",
        1500,
        750,
        std::time::Duration::from_millis(2500),
        true,
    );

    logger.log(entry).await.expect("Failed to log");

    let entries = logger.entries();
    assert_eq!(entries.len(), 1);

    match &entries[0].event {
        AuditEvent::ApiCall {
            model,
            input_tokens,
            output_tokens,
            success,
            ..
        } => {
            assert_eq!(model, "claude-3-opus");
            assert_eq!(*input_tokens, 1500);
            assert_eq!(*output_tokens, 750);
            assert!(*success);
        }
        _ => panic!("Expected ApiCall event"),
    }
}

/// Test that audit logs persist to file.
#[tokio::test]
async fn test_audit_log_persistence() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let mut logger = file_logger(&temp_dir);

    // Log some entries
    let entry1 = AuditEntry::tool_use(
        "session-1",
        "Read",
        serde_json::json!({"file": "/test/path"}),
        None,
        std::time::Duration::from_millis(10),
    );
    let entry2 = AuditEntry::api_call(
        "session-1",
        "claude-3-sonnet",
        500,
        200,
        std::time::Duration::from_millis(1000),
        true,
    );

    logger.log(entry1).await.expect("Failed to log entry 1");
    logger.log(entry2).await.expect("Failed to log entry 2");

    // Flush to disk
    logger.flush().await.expect("Failed to flush");

    // Verify file exists and has content
    let log_path = temp_dir.path().join("audit.log");
    assert!(log_path.exists(), "Audit log file should exist");

    let content = std::fs::read_to_string(&log_path).expect("Failed to read log file");
    assert!(content.contains("Read"), "Log should contain tool name");
    assert!(
        content.contains("claude-3-sonnet"),
        "Log should contain model name"
    );
}

/// Test querying audit logs by session.
#[tokio::test]
async fn test_audit_query_by_session() {
    let mut logger = memory_logger();

    // Log entries for different sessions
    let entry1 = AuditEntry::tool_use(
        "session-a",
        "Bash",
        serde_json::json!({}),
        None,
        std::time::Duration::from_millis(10),
    );
    let entry2 = AuditEntry::tool_use(
        "session-b",
        "Read",
        serde_json::json!({}),
        None,
        std::time::Duration::from_millis(10),
    );
    let entry3 = AuditEntry::api_call(
        "session-a",
        "model",
        100,
        50,
        std::time::Duration::from_millis(100),
        true,
    );

    logger.log(entry1).await.unwrap();
    logger.log(entry2).await.unwrap();
    logger.log(entry3).await.unwrap();

    // Query by session
    let query = AuditQuery::new().session("session-a");
    let results = logger.query(&query);

    assert_eq!(results.len(), 2);
    for entry in results {
        assert_eq!(entry.session_id, "session-a");
    }
}

/// Test querying audit logs by event type.
#[tokio::test]
async fn test_audit_query_by_event_type() {
    let mut logger = memory_logger();

    // Log mixed events
    let entry1 = AuditEntry::tool_use(
        "session-1",
        "Bash",
        serde_json::json!({}),
        None,
        std::time::Duration::from_millis(10),
    );
    let entry2 = AuditEntry::api_call(
        "session-1",
        "model",
        100,
        50,
        std::time::Duration::from_millis(100),
        true,
    );

    logger.log(entry1).await.unwrap();
    logger.log(entry2).await.unwrap();

    // Query for tool use only
    let query = AuditQuery::new().event_type("tool_use");
    let results = logger.query(&query);

    assert_eq!(results.len(), 1);
    assert!(matches!(results[0].event, AuditEvent::ToolUse { .. }));
}

/// Test audit logger disabled state.
#[test]
fn test_audit_logger_disabled() {
    let config = AuditConfig {
        enabled: false,
        level: AuditLevel::All,
        output_path: None,
        json_format: false,
        include_timestamps: true,
        include_session_id: true,
    };
    let logger = AuditLogger::new(config);

    assert!(!logger.is_enabled());
}

/// Test audit level filtering.
#[tokio::test]
async fn test_audit_level_filtering() {
    let config = AuditConfig {
        enabled: true,
        level: AuditLevel::ApiOnly,
        output_path: None,
        json_format: false,
        include_timestamps: true,
        include_session_id: true,
    };
    let mut logger = AuditLogger::new(config);

    // Log both tool use and API call
    let tool_entry = AuditEntry::tool_use(
        "session-1",
        "Bash",
        serde_json::json!({}),
        None,
        std::time::Duration::from_millis(10),
    );
    let api_entry = AuditEntry::api_call(
        "session-1",
        "model",
        100,
        50,
        std::time::Duration::from_millis(100),
        true,
    );

    logger.log(tool_entry).await.unwrap();
    logger.log(api_entry).await.unwrap();

    // Should only have API entry due to level filtering
    let entries = logger.entries();
    assert_eq!(entries.len(), 1);
    assert!(matches!(entries[0].event, AuditEvent::ApiCall { .. }));
}

/// Test computing audit statistics.
#[tokio::test]
async fn test_audit_statistics() {
    let mut logger = memory_logger();

    // Log multiple API calls
    for i in 0..5 {
        let entry = AuditEntry::api_call(
            "session-1",
            "claude-3-opus",
            100 * (i + 1) as u32,
            50 * (i + 1) as u32,
            std::time::Duration::from_millis(100 * (i + 1)),
            i != 2, // One failure
        );
        logger.log(entry).await.unwrap();
    }

    let stats = logger.statistics();

    assert_eq!(stats.total_api_calls, 5);
    assert_eq!(stats.successful_api_calls, 4);
    assert_eq!(stats.total_input_tokens, 100 + 200 + 300 + 400 + 500);
    assert_eq!(stats.total_output_tokens, 50 + 100 + 150 + 200 + 250);
}

/// Test session start/end logging.
#[tokio::test]
async fn test_audit_session_lifecycle() {
    let mut logger = memory_logger();

    // Log session start
    let start_entry = AuditEntry::session_start("session-new", PathBuf::from("/project"));
    logger.log(start_entry).await.unwrap();

    // Log session end
    let end_entry = AuditEntry::session_end("session-new", std::time::Duration::from_secs(300));
    logger.log(end_entry).await.unwrap();

    let entries = logger.entries();
    assert_eq!(entries.len(), 2);

    assert!(matches!(entries[0].event, AuditEvent::SessionStart { .. }));
    assert!(matches!(entries[1].event, AuditEvent::SessionEnd { .. }));
}

/// Test audit configuration validation.
#[test]
fn test_audit_config_validation() {
    let valid_config = AuditConfig {
        enabled: true,
        level: AuditLevel::All,
        output_path: Some(PathBuf::from("/tmp/audit.log")),
        json_format: true,
        include_timestamps: true,
        include_session_id: true,
    };

    assert!(valid_config.validate().is_ok());
}

/// Test JSON serialization of audit entries.
#[test]
fn test_audit_entry_json_serialization() {
    let entry = AuditEntry::tool_use(
        "session-json",
        "Write",
        serde_json::json!({"file": "/test.txt", "content": "hello"}),
        Some(serde_json::json!({"success": true})),
        std::time::Duration::from_millis(25),
    );

    let json = serde_json::to_string(&entry).expect("Failed to serialize");
    assert!(json.contains("Write"));
    assert!(json.contains("session-json"));

    let deserialized: AuditEntry = serde_json::from_str(&json).expect("Failed to deserialize");
    assert_eq!(deserialized.session_id, "session-json");
}
