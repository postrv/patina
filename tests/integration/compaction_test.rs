//! Integration tests for context compaction.
//!
//! These tests verify:
//! - End-to-end compaction workflow
//! - Tool use preservation during compaction
//! - Summary quality and structure
//! - Integration with the context module

use patina::api::compaction::{
    build_summarization_request, CompactionConfig, ContextCompactor, SummaryStyle,
};
use patina::api::context::compact_or_truncate_context;
use patina::api::tokens::estimate_messages_tokens;
use patina::types::{ApiMessageV2, ContentBlock, MessageContent, Role};
use serde_json::json;

// ============================================================================
// End-to-End Compaction Tests
// ============================================================================

/// Verifies complete compaction workflow from input to output.
#[test]
fn test_compaction_end_to_end() {
    // Create a realistic conversation that needs compaction
    let long_response = "I have completed the task. This response contains detailed information about what was done, including implementation details, code changes, and next steps. The implementation follows best practices and includes proper error handling.";

    let mut messages = vec![ApiMessageV2::user(
        "You are a helpful coding assistant for project X. Follow the coding standards.",
    )];

    // Add multiple exchanges to simulate a real session
    for i in 0..10 {
        messages.push(ApiMessageV2::user(format!(
            "Task {}: Please implement feature {}",
            i, i
        )));
        messages.push(ApiMessageV2::assistant(format!(
            "Completed task {}. {}",
            i, long_response
        )));
    }

    let original_count = messages.len();
    let original_tokens = estimate_messages_tokens(&messages);

    // Compact with aggressive settings
    let compactor = ContextCompactor::new_mock();
    let config = CompactionConfig {
        target_tokens: 500,
        preserve_recent: 2,
        summary_style: SummaryStyle::Timeline,
    };

    let result = compactor.compact(&messages, &config).unwrap();

    // Verify compaction results
    assert!(
        result.messages.len() < original_count,
        "Should have fewer messages after compaction"
    );
    assert!(
        result.saved_tokens > 0,
        "Should report token savings: original={}, result tokens={}",
        original_tokens,
        estimate_messages_tokens(&result.messages)
    );

    // Verify first message (system) is preserved
    assert_eq!(
        result.messages[0].role,
        Role::User,
        "First message should be user"
    );
    assert!(
        result.messages[0].content.to_text().contains("project X"),
        "System message content should be preserved"
    );

    // Verify last messages are preserved
    let last_msg = result.messages.last().unwrap();
    assert!(
        last_msg.content.to_text().contains("task"),
        "Recent context should be preserved"
    );
}

/// Verifies compaction works correctly via the context module integration.
#[test]
fn test_compaction_via_context_module() {
    let padding = "x".repeat(200);
    let mut messages = vec![ApiMessageV2::user(format!("System prompt {}", padding))];

    for i in 0..20 {
        messages.push(ApiMessageV2::user(format!("Question {} {}", i, padding)));
        messages.push(ApiMessageV2::assistant(format!("Answer {} {}", i, padding)));
    }

    let original_count = messages.len();

    // Use the integrated compact_or_truncate_context function
    let result = compact_or_truncate_context(&messages, 200, 2);

    // Should have compacted
    assert!(
        result.len() < original_count,
        "Should have fewer messages: {} vs {}",
        result.len(),
        original_count
    );

    // Should preserve first message
    assert!(result[0].content.to_text().contains("System prompt"));

    // Should preserve recent messages
    let last_original = messages.last().unwrap().content.to_text();
    let last_result = result.last().unwrap().content.to_text();
    assert_eq!(
        last_result, last_original,
        "Last message should be preserved"
    );
}

// ============================================================================
// Tool Use Preservation Tests
// ============================================================================

/// Verifies tool use blocks are handled correctly during compaction.
#[test]
fn test_compaction_with_tool_use() {
    let messages =
        vec![
            ApiMessageV2::user("System prompt with project context"),
            ApiMessageV2::assistant("Ready to help."),
            ApiMessageV2::user("Run ls command"),
            ApiMessageV2::assistant_with_content(MessageContent::blocks(vec![
                ContentBlock::text("Running the ls command..."),
                ContentBlock::tool_use("toolu_1", "bash", json!({"command": "ls -la"})),
            ])),
            ApiMessageV2::user_with_content(MessageContent::blocks(vec![
                ContentBlock::tool_result("toolu_1", "file1.txt\nfile2.txt\ndir1/"),
            ])),
            ApiMessageV2::assistant("The directory contains file1.txt, file2.txt, and dir1/."),
            ApiMessageV2::user("Now create a new file"),
            ApiMessageV2::assistant_with_content(MessageContent::blocks(vec![
                ContentBlock::text("Creating the new file..."),
                ContentBlock::tool_use(
                    "toolu_2",
                    "write_file",
                    json!({"path": "newfile.txt", "content": "hello"}),
                ),
            ])),
            ApiMessageV2::user_with_content(MessageContent::blocks(vec![
                ContentBlock::tool_result("toolu_2", "File created successfully"),
            ])),
            ApiMessageV2::assistant("Created newfile.txt with the content 'hello'."),
        ];

    let compactor = ContextCompactor::new_mock();
    let config = CompactionConfig {
        target_tokens: 500,
        preserve_recent: 4,
        summary_style: SummaryStyle::Timeline,
    };

    let result = compactor.compact(&messages, &config).unwrap();

    // Verify role alternation is maintained
    for window in result.messages.windows(2) {
        assert_ne!(
            window[0].role, window[1].role,
            "Adjacent messages should have different roles"
        );
    }

    // Verify we have a reasonable number of messages
    assert!(
        result.messages.len() >= 3,
        "Should have at least system + summary + recent"
    );
}

/// Verifies tool pairs are kept together when they're in the recent messages.
#[test]
fn test_compaction_preserves_tool_pairs_in_recent() {
    let messages = vec![
        ApiMessageV2::user("System prompt"),
        ApiMessageV2::assistant("Ready."),
        ApiMessageV2::user("Old task 1"),
        ApiMessageV2::assistant("Done with old task 1."),
        ApiMessageV2::user("Run the test command"),
        ApiMessageV2::assistant_with_content(MessageContent::blocks(vec![
            ContentBlock::text("Running tests..."),
            ContentBlock::tool_use("toolu_test", "bash", json!({"command": "cargo test"})),
        ])),
        ApiMessageV2::user_with_content(MessageContent::blocks(vec![ContentBlock::tool_result(
            "toolu_test",
            "All 50 tests passed!",
        )])),
        ApiMessageV2::assistant("All tests passed successfully!"),
    ];

    let compactor = ContextCompactor::new_mock();
    let config = CompactionConfig {
        target_tokens: 300,
        preserve_recent: 4, // Should preserve the tool use pair
        summary_style: SummaryStyle::Timeline,
    };

    let result = compactor.compact(&messages, &config).unwrap();

    // Check if tool_use is in recent messages
    let has_tool_use = result.messages.iter().any(|m| {
        if let MessageContent::Blocks(blocks) = &m.content {
            blocks.iter().any(|b| matches!(b, ContentBlock::ToolUse(_)))
        } else {
            false
        }
    });

    // Check if corresponding tool_result is present
    let has_tool_result = result.messages.iter().any(|m| {
        if let MessageContent::Blocks(blocks) = &m.content {
            blocks
                .iter()
                .any(|b| matches!(b, ContentBlock::ToolResult(_)))
        } else {
            false
        }
    });

    // If tool_use is present, tool_result should also be present
    if has_tool_use {
        assert!(
            has_tool_result,
            "Tool use and tool result should be kept together"
        );
    }
}

// ============================================================================
// Summary Quality Tests
// ============================================================================

/// Verifies timeline-style summary has proper structure.
#[test]
fn test_compaction_summary_quality_timeline() {
    let messages = vec![
        ApiMessageV2::user("System prompt for development project"),
        ApiMessageV2::assistant("Ready to help with development."),
        ApiMessageV2::user("Create authentication module"),
        ApiMessageV2::assistant("Created auth.rs with login and logout functions."),
        ApiMessageV2::user("Add password hashing"),
        ApiMessageV2::assistant("Added bcrypt password hashing to the auth module."),
        ApiMessageV2::user("Write unit tests"),
        ApiMessageV2::assistant("Added 10 unit tests for the authentication module."),
        ApiMessageV2::user("Current task: deploy"),
        ApiMessageV2::assistant("Preparing deployment."),
    ];

    let compactor = ContextCompactor::new_mock();
    let config = CompactionConfig {
        target_tokens: 200,
        preserve_recent: 2,
        summary_style: SummaryStyle::Timeline,
    };

    let result = compactor.compact(&messages, &config).unwrap();

    // Find the summary message (should be after first message)
    assert!(
        result.messages.len() >= 2,
        "Should have at least first message and summary"
    );

    let all_text: String = result
        .messages
        .iter()
        .map(|m| m.content.to_text())
        .collect::<Vec<_>>()
        .join(" ");

    // Verify summary mentions key actions
    let has_timeline_indicators = all_text.contains("timeline")
        || all_text.contains("Previous conversation")
        || all_text.contains("1.")
        || all_text.contains("Created")
        || all_text.contains("Added");

    assert!(
        has_timeline_indicators,
        "Timeline summary should have structured content"
    );
}

/// Verifies summarization request format is correct.
#[test]
fn test_summarization_request_format() {
    let messages = vec![
        ApiMessageV2::user("Hello"),
        ApiMessageV2::assistant("Hi there!"),
        ApiMessageV2::user("Help me with code"),
        ApiMessageV2::assistant("Sure, I'd be happy to help."),
    ];

    let request = build_summarization_request(&messages, SummaryStyle::Timeline);

    // Verify request contains the prompt
    assert!(
        request.contains("timeline"),
        "Should contain timeline instruction"
    );
    assert!(
        request.contains("Key decisions"),
        "Should contain summary guidelines"
    );

    // Verify request contains the messages
    assert!(request.contains("User:"), "Should contain User role prefix");
    assert!(
        request.contains("Assistant:"),
        "Should contain Assistant role prefix"
    );
    assert!(request.contains("Hello"), "Should contain message content");
}

/// Verifies different summary styles produce appropriate output.
#[test]
fn test_summary_styles() {
    let messages = vec![
        ApiMessageV2::user("Project setup"),
        ApiMessageV2::assistant("Ready."),
        ApiMessageV2::user("Create main.rs"),
        ApiMessageV2::assistant("Created main.rs with hello world."),
    ];

    for style in [
        SummaryStyle::Timeline,
        SummaryStyle::BulletPoints,
        SummaryStyle::Narrative,
    ] {
        let compactor = ContextCompactor::new_mock();
        let config = CompactionConfig {
            target_tokens: 100,
            preserve_recent: 0,
            summary_style: style,
        };

        let result = compactor.compact(&messages, &config);
        assert!(result.is_ok(), "Should succeed with style {:?}", style);

        let result = result.unwrap();
        assert!(!result.messages.is_empty(), "Should produce output");
    }
}

// ============================================================================
// Edge Case Tests
// ============================================================================

/// Verifies compaction handles empty conversations.
#[test]
fn test_compaction_empty_conversation() {
    let messages: Vec<ApiMessageV2> = vec![];
    let result = compact_or_truncate_context(&messages, 1000, 2);
    assert!(result.is_empty());
}

/// Verifies compaction handles single message.
#[test]
fn test_compaction_single_message() {
    let messages = vec![ApiMessageV2::user("Just the system prompt")];
    let result = compact_or_truncate_context(&messages, 1000, 2);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].content.to_text(), "Just the system prompt");
}

/// Verifies compaction with very low token budget.
#[test]
fn test_compaction_very_low_budget() {
    let padding = "x".repeat(500);
    let messages = vec![
        ApiMessageV2::user(format!("System {}", padding)),
        ApiMessageV2::assistant(format!("Response {}", padding)),
        ApiMessageV2::user(format!("Query {}", padding)),
        ApiMessageV2::assistant(format!("Answer {}", padding)),
    ];

    // Even with very low budget, should still produce valid output
    let result = compact_or_truncate_context(&messages, 10, 1);

    // Should have at least the first message
    assert!(!result.is_empty(), "Should produce some output");
    assert!(
        result[0].content.to_text().contains("System"),
        "Should preserve first message"
    );
}
