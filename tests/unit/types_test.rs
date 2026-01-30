//! Unit tests for core types module.
//!
//! These tests verify serialization and display behavior for core types.
//! Following TDD RED phase - these tests will fail until types are properly implemented.

use rct::{Message, Role};

/// Tests that Message can be serialized to JSON and deserialized back correctly.
///
/// Expected JSON format:
/// ```json
/// {
///   "role": "user",
///   "content": "Hello, Claude!"
/// }
/// ```
#[test]
fn test_message_serialization() {
    let message = Message {
        role: Role::User,
        content: "Hello, Claude!".to_string(),
    };

    // Test serialization to JSON
    let json = serde_json::to_string(&message).expect("Message should serialize to JSON");

    // Verify JSON structure
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["role"], "user");
    assert_eq!(parsed["content"], "Hello, Claude!");

    // Test deserialization from JSON
    let deserialized: Message =
        serde_json::from_str(&json).expect("Message should deserialize from JSON");
    assert_eq!(deserialized.content, "Hello, Claude!");
    assert_eq!(deserialized.role, Role::User);
}

/// Tests that Message with Assistant role serializes correctly.
#[test]
fn test_message_serialization_assistant() {
    let message = Message {
        role: Role::Assistant,
        content: "I'm here to help.".to_string(),
    };

    let json = serde_json::to_string(&message).expect("Message should serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["role"], "assistant");
    assert_eq!(parsed["content"], "I'm here to help.");
}

/// Tests that Role enum implements Display with lowercase API-compatible strings.
///
/// Expected display output:
/// - Role::User -> "user"
/// - Role::Assistant -> "assistant"
#[test]
fn test_role_display() {
    assert_eq!(format!("{}", Role::User), "user");
    assert_eq!(format!("{}", Role::Assistant), "assistant");
}

/// Tests that Role can be serialized as a lowercase string.
#[test]
fn test_role_serialization() {
    let user = Role::User;
    let assistant = Role::Assistant;

    let user_json = serde_json::to_string(&user).expect("Role::User should serialize");
    let assistant_json =
        serde_json::to_string(&assistant).expect("Role::Assistant should serialize");

    assert_eq!(user_json, "\"user\"");
    assert_eq!(assistant_json, "\"assistant\"");
}

/// Tests that Role can be deserialized from lowercase strings.
#[test]
fn test_role_deserialization() {
    let user: Role = serde_json::from_str("\"user\"").expect("Should deserialize 'user'");
    let assistant: Role =
        serde_json::from_str("\"assistant\"").expect("Should deserialize 'assistant'");

    assert_eq!(user, Role::User);
    assert_eq!(assistant, Role::Assistant);
}

/// Tests that Message handles empty content correctly.
#[test]
fn test_message_empty_content() {
    let message = Message {
        role: Role::User,
        content: String::new(),
    };

    let json = serde_json::to_string(&message).expect("Empty message should serialize");
    let deserialized: Message = serde_json::from_str(&json).expect("Should deserialize");

    assert!(deserialized.content.is_empty());
}

/// Tests that Message handles unicode content correctly.
#[test]
fn test_message_unicode_content() {
    let message = Message {
        role: Role::User,
        content: "Hello 世界! 🌍 Привет".to_string(),
    };

    let json = serde_json::to_string(&message).expect("Unicode message should serialize");
    let deserialized: Message = serde_json::from_str(&json).expect("Should deserialize");

    assert_eq!(deserialized.content, "Hello 世界! 🌍 Привет");
}
