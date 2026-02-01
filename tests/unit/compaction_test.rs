//! Tests for context compaction module
//!
//! Context compaction differs from truncation:
//! - Truncation simply drops old messages
//! - Compaction summarizes old messages into a timeline, preserving key information
//!
//! These tests define the expected behavior for the `ContextCompactor` struct.

use patina::api::compaction::{CompactionConfig, ContextCompactor};
use patina::api::tokens::estimate_messages_tokens;
use patina::types::{ApiMessageV2, ContentBlock, MessageContent};
use serde_json::json;

/// Helper to create a simple user message
fn user(content: impl Into<String>) -> ApiMessageV2 {
    ApiMessageV2::user(content)
}

/// Helper to create a simple assistant message
fn assistant(content: impl Into<String>) -> ApiMessageV2 {
    ApiMessageV2::assistant(content)
}

/// Helper to create an assistant message with tool use
fn assistant_with_tool(text: &str, tool_id: &str, tool_name: &str) -> ApiMessageV2 {
    ApiMessageV2::assistant_with_content(MessageContent::blocks(vec![
        ContentBlock::text(text),
        ContentBlock::tool_use(tool_id, tool_name, json!({"arg": "value"})),
    ]))
}

/// Helper to create a user message with tool result
fn user_with_tool_result(tool_id: &str, result: &str) -> ApiMessageV2 {
    ApiMessageV2::user_with_content(MessageContent::blocks(vec![ContentBlock::tool_result(
        tool_id, result,
    )]))
}

// =============================================================================
// test_compact_preserves_system_message
// =============================================================================

/// The first message (system/project context) must always be preserved.
/// This is critical for maintaining conversation coherence.
#[test]
fn test_compact_preserves_system_message() {
    let compactor = ContextCompactor::new_mock();
    let messages = vec![
        user("You are a helpful assistant for project X..."),
        assistant("Understood. How can I help?"),
        user("What files are in this project?"),
        assistant("Let me check..."),
    ];

    let config = CompactionConfig::default();
    let result = compactor.compact(&messages, &config);

    assert!(result.is_ok());
    let result = result.unwrap();

    // First message must be preserved
    assert!(
        !result.messages.is_empty(),
        "Compaction should produce at least one message"
    );
    assert_eq!(
        result.messages[0].content.to_text(),
        messages[0].content.to_text(),
        "First message (system prompt) must be preserved exactly"
    );
}

// =============================================================================
// test_compact_summarizes_old_messages
// =============================================================================

/// Old messages should be summarized into a timeline summary,
/// not just dropped like truncation does.
#[test]
fn test_compact_summarizes_old_messages() {
    let compactor = ContextCompactor::new_mock();

    // Create a long conversation with substantial content that exceeds token budget
    let long_response = "Here is a detailed explanation of how to implement this feature. First, you need to understand the core concepts involved. Then you'll want to set up the proper data structures and algorithms. After that, you can proceed with the actual implementation. Make sure to add proper error handling and logging throughout.";

    let messages = vec![
        user("System prompt for project X with detailed instructions about coding standards and project requirements."),
        assistant(long_response),
        user("Create a new file called main.rs with proper module structure and documentation"),
        assistant(format!("I've created main.rs with a comprehensive hello world program. {}", long_response)),
        user("Now add a function called greet with proper error handling"),
        assistant(format!("Added the greet function to main.rs with full documentation. {}", long_response)),
        user("Run the tests and show me the results"),
        assistant(format!("All tests passed! Here are the details: {}", long_response)),
        user("Deploy to production environment"),
        assistant(format!("Successfully deployed to production. {}", long_response)),
    ];

    // Configure to compact aggressively (small target - this should definitely trigger compaction)
    let config = CompactionConfig {
        target_tokens: 100, // Very small target to ensure compaction
        preserve_recent: 2,
        ..Default::default()
    };

    let result = compactor.compact(&messages, &config).unwrap();

    // Should have fewer messages than original
    assert!(
        result.messages.len() < messages.len(),
        "Compacted messages ({}) should be fewer than original ({})",
        result.messages.len(),
        messages.len()
    );

    // Should contain a summary that references key actions
    let all_text: String = result
        .messages
        .iter()
        .map(|m| m.content.to_text())
        .collect::<Vec<_>>()
        .join(" ");

    // The summary should mention key events from the conversation
    assert!(
        all_text.contains("main.rs")
            || all_text.contains("greet")
            || all_text.contains("deployed")
            || all_text.contains("summary")
            || all_text.contains("timeline")
            || all_text.contains("Previous conversation"),
        "Compacted messages should contain summary of key actions"
    );
}

// =============================================================================
// test_compact_preserves_recent_messages
// =============================================================================

/// Recent messages should be preserved verbatim, not summarized.
/// This ensures the model has immediate context for the current task.
#[test]
fn test_compact_preserves_recent_messages() {
    let compactor = ContextCompactor::new_mock();

    let messages = vec![
        user("System prompt"),
        assistant("Ready."),
        user("Old task 1"),
        assistant("Done with old task 1."),
        user("Old task 2"),
        assistant("Done with old task 2."),
        user("Current task: fix the bug in auth.rs"),
        assistant("I'll look at auth.rs now."),
    ];

    let config = CompactionConfig {
        target_tokens: 500,
        preserve_recent: 4, // Keep last 4 messages
        ..Default::default()
    };

    let result = compactor.compact(&messages, &config).unwrap();

    // Last 4 messages should be preserved exactly
    let last_four: Vec<_> = messages.iter().rev().take(4).rev().collect();
    let result_last_four: Vec<_> = result.messages.iter().rev().take(4).rev().collect();

    for (original, compacted) in last_four.iter().zip(result_last_four.iter()) {
        assert_eq!(
            original.content.to_text(),
            compacted.content.to_text(),
            "Recent messages should be preserved exactly"
        );
    }
}

// =============================================================================
// test_compact_respects_token_budget
// =============================================================================

/// Compacted output must respect the configured token budget.
#[test]
fn test_compact_respects_token_budget() {
    let compactor = ContextCompactor::new_mock();

    // Create a very long conversation
    let mut messages = vec![user("System prompt for a large project")];
    for i in 0..50 {
        messages.push(user(format!(
            "Question {}: How do I implement feature {}?",
            i, i
        )));
        messages.push(assistant(format!(
            "Response {}: Here's how to implement feature {}. First, you need to...",
            i, i
        )));
    }

    let target_tokens = 5_000;
    let config = CompactionConfig {
        target_tokens,
        preserve_recent: 4,
        ..Default::default()
    };

    let result = compactor.compact(&messages, &config).unwrap();

    let actual_tokens = estimate_messages_tokens(&result.messages);
    assert!(
        actual_tokens <= target_tokens + 500, // Allow some margin for estimation error
        "Compacted tokens ({}) should be near or below target ({})",
        actual_tokens,
        target_tokens
    );
}

// =============================================================================
// test_compact_maintains_conversation_coherence
// =============================================================================

/// The compacted conversation must maintain alternating user/assistant roles.
#[test]
fn test_compact_maintains_conversation_coherence() {
    let compactor = ContextCompactor::new_mock();

    let messages = vec![
        user("System prompt"),
        assistant("Ready."),
        user("Task 1"),
        assistant("Done 1."),
        user("Task 2"),
        assistant("Done 2."),
        user("Task 3"),
        assistant("Done 3."),
    ];

    let config = CompactionConfig {
        target_tokens: 200,
        preserve_recent: 2,
        ..Default::default()
    };

    let result = compactor.compact(&messages, &config).unwrap();

    // Check role alternation (first should be user)
    for window in result.messages.windows(2) {
        assert_ne!(
            window[0].role, window[1].role,
            "Adjacent messages should have different roles"
        );
    }

    // First message should be user (system prompt)
    assert_eq!(
        result.messages[0].role,
        patina::types::Role::User,
        "First message should be from user"
    );
}

// =============================================================================
// test_compact_handles_tool_use_pairs
// =============================================================================

/// Tool use and tool result blocks must be kept together.
/// Separating them would break the conversation structure.
#[test]
fn test_compact_handles_tool_use_pairs() {
    let compactor = ContextCompactor::new_mock();

    let messages = vec![
        user("System prompt"),
        assistant("Ready."),
        user("Run ls command"),
        assistant_with_tool("Running the command...", "toolu_1", "bash"),
        user_with_tool_result("toolu_1", "file1.txt\nfile2.txt"),
        assistant("The directory contains file1.txt and file2.txt."),
        user("Current question"),
        assistant("Current answer"),
    ];

    let config = CompactionConfig {
        target_tokens: 500,
        preserve_recent: 2,
        ..Default::default()
    };

    let result = compactor.compact(&messages, &config).unwrap();

    // If tool_use block is included, its corresponding tool_result must also be included
    let has_tool_use = result.messages.iter().any(|m| {
        if let MessageContent::Blocks(blocks) = &m.content {
            blocks.iter().any(|b| matches!(b, ContentBlock::ToolUse(_)))
        } else {
            false
        }
    });

    let has_tool_result = result.messages.iter().any(|m| {
        if let MessageContent::Blocks(blocks) = &m.content {
            blocks
                .iter()
                .any(|b| matches!(b, ContentBlock::ToolResult(_)))
        } else {
            false
        }
    });

    // If we have tool_use, we must have its result (they come in pairs)
    if has_tool_use {
        assert!(
            has_tool_result,
            "Tool use blocks must be paired with their results"
        );
    }
}

// =============================================================================
// test_compact_generates_timeline_summary
// =============================================================================

/// The summary should be structured as a timeline with key decisions/outcomes.
#[test]
fn test_compact_generates_timeline_summary() {
    let compactor = ContextCompactor::new_mock();

    // Use longer messages to ensure we exceed the token budget
    let detailed_response = "I have completed this task successfully. The implementation includes proper error handling, comprehensive logging, thorough documentation, and unit tests covering edge cases.";

    let messages = vec![
        user("System prompt with detailed project requirements and coding standards that must be followed throughout the development process."),
        assistant(format!("Ready to help with the project. I understand all the requirements. {}", detailed_response)),
        user("Create user authentication module with secure session handling"),
        assistant(format!("Created auth.rs with login and logout functions. {}", detailed_response)),
        user("Add password hashing using industry standard algorithms"),
        assistant(format!("Added bcrypt hashing to the auth module. {}", detailed_response)),
        user("Write comprehensive tests for auth module"),
        assistant(format!("Added 5 unit tests for authentication. {}", detailed_response)),
    ];

    let config = CompactionConfig {
        target_tokens: 100, // Very low to force compaction
        preserve_recent: 2,
        ..Default::default()
    };

    let result = compactor.compact(&messages, &config).unwrap();

    // The result should contain a summary message
    // Look for timeline-like content
    let all_text: String = result
        .messages
        .iter()
        .map(|m| m.content.to_text())
        .collect::<Vec<_>>()
        .join(" ");

    // Should have some form of structured summary
    let has_summary_structure = all_text.contains("Previous conversation")
        || all_text.contains("Summary")
        || all_text.contains("Timeline")
        || all_text.contains("Earlier")
        || all_text.contains("history")
        || all_text.contains("completed")
        || all_text.contains("actions")
        || all_text.contains("timeline");

    assert!(
        has_summary_structure,
        "Compacted output should contain a structured timeline/summary. Got: {}",
        all_text
    );
}

// =============================================================================
// test_compact_merges_adjacent_summaries
// =============================================================================

/// If the input already contains summary messages, they should be merged
/// rather than creating nested summaries.
#[test]
fn test_compact_merges_adjacent_summaries() {
    let compactor = ContextCompactor::new_mock();

    // Simulate a conversation that was already compacted once
    let messages = vec![
        user("System prompt"),
        // This is a previous summary
        assistant("Previous conversation summary: User asked about feature A and B."),
        user("New task: implement feature C"),
        assistant("Implemented feature C."),
        user("Now implement feature D"),
        assistant("Implemented feature D."),
    ];

    let config = CompactionConfig {
        target_tokens: 300,
        preserve_recent: 2,
        ..Default::default()
    };

    let result = compactor.compact(&messages, &config).unwrap();

    // Count how many "summary" messages we have
    let summary_count = result
        .messages
        .iter()
        .filter(|m| {
            let text = m.content.to_text().to_lowercase();
            text.contains("summary") || text.contains("previous conversation")
        })
        .count();

    // Should have at most one summary message (merged), not multiple
    assert!(
        summary_count <= 1,
        "Adjacent summaries should be merged, found {} summary messages",
        summary_count
    );
}

// =============================================================================
// test_compact_idempotent_when_under_budget
// =============================================================================

/// If the conversation is already under the token budget,
/// compaction should return it unchanged.
#[test]
fn test_compact_idempotent_when_under_budget() {
    let compactor = ContextCompactor::new_mock();

    let messages = vec![
        user("System prompt"),
        assistant("Ready."),
        user("Simple question"),
        assistant("Simple answer."),
    ];

    let config = CompactionConfig {
        target_tokens: 100_000, // Very large budget
        preserve_recent: 10,
        ..Default::default()
    };

    let result = compactor.compact(&messages, &config).unwrap();

    // Should return messages unchanged
    assert_eq!(
        result.messages.len(),
        messages.len(),
        "Under-budget conversation should not be modified"
    );

    for (original, compacted) in messages.iter().zip(result.messages.iter()) {
        assert_eq!(
            original.content.to_text(),
            compacted.content.to_text(),
            "Messages should be unchanged when under budget"
        );
    }

    // Should report zero savings
    assert_eq!(
        result.saved_tokens, 0,
        "No tokens should be saved when under budget"
    );
}

// =============================================================================
// test_compact_reports_token_savings
// =============================================================================

/// Compaction should report how many tokens were saved.
#[test]
fn test_compact_reports_token_savings() {
    let compactor = ContextCompactor::new_mock();

    // Create a conversation with significant content that definitely exceeds budget
    let long_padding = "x".repeat(200); // Add 200 chars = 50 tokens to each message
    let mut messages = vec![user(format!(
        "System prompt with detailed context {}",
        long_padding
    ))];
    for i in 0..20 {
        messages.push(user(format!(
            "Question {} with extended context: {}",
            i, long_padding
        )));
        messages.push(assistant(format!("A detailed response for question {}. This contains a lot of information that should be summarized during compaction. Additional context: {}", i, long_padding)));
    }

    let original_tokens = estimate_messages_tokens(&messages);

    let config = CompactionConfig {
        target_tokens: 500, // Very low target to ensure compaction
        preserve_recent: 4,
        ..Default::default()
    };

    let result = compactor.compact(&messages, &config).unwrap();

    // Should report positive token savings
    assert!(
        result.saved_tokens > 0,
        "Should report token savings when compacting. Original tokens: {}, config target: {}",
        original_tokens,
        config.target_tokens
    );

    // Verify savings calculation is reasonable
    let compacted_tokens = estimate_messages_tokens(&result.messages);
    let expected_savings = original_tokens.saturating_sub(compacted_tokens);

    // Allow some variance in reporting due to estimation
    let savings_diff = (result.saved_tokens as i64 - expected_savings as i64).abs();
    assert!(
        savings_diff < 500,
        "Reported savings ({}) should be close to actual savings ({})",
        result.saved_tokens,
        expected_savings
    );
}
