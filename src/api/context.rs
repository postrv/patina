//! Context window management for API requests.
//!
//! Provides smart truncation and compaction of conversation history to stay within
//! token budgets while preserving conversation coherence.
//!
//! # Overview
//!
//! Long conversations can accumulate hundreds of thousands of tokens,
//! causing API costs to skyrocket. This module provides utilities to
//! manage conversation history:
//!
//! 1. **Always preserving the first message** (system prompt/project context)
//! 2. **Prioritizing recent messages** (most relevant context)
//! 3. **Respecting token budgets** (configurable limits)
//! 4. **Maintaining conversation order** (coherent history)
//!
//! ## Truncation vs Compaction
//!
//! - **Truncation** (`truncate_context`): Simply drops old messages to fit budget.
//!   Fast but loses context.
//! - **Compaction** (`compact_or_truncate_context`): Summarizes old messages into
//!   a timeline before preserving recent messages. Slower but preserves more context.
//!
//! # Example
//!
//! ```rust
//! use patina::api::context::{truncate_context, DEFAULT_MAX_INPUT_TOKENS};
//! use patina::types::ApiMessageV2;
//!
//! let messages = vec![
//!     ApiMessageV2::user("Initial project context..."),
//!     ApiMessageV2::assistant("Understood."),
//!     // ... many more messages ...
//! ];
//!
//! // Truncate to fit within budget
//! let truncated = truncate_context(&messages, DEFAULT_MAX_INPUT_TOKENS);
//! assert!(truncated.len() <= messages.len());
//! ```

use crate::api::compaction::{CompactionConfig, ContextCompactor};
use crate::api::tokens::estimate_message_tokens;
use crate::types::ApiMessageV2;

/// Default maximum input tokens per request.
///
/// Set conservatively below the 200k context window limit to allow room for:
/// - Model response tokens
/// - Tool definitions
/// - Safety margin for estimation errors
///
/// At $3/million input tokens (Claude Sonnet), this limits per-request cost to ~$0.30.
pub const DEFAULT_MAX_INPUT_TOKENS: usize = 100_000;

/// Default maximum number of messages to include.
///
/// Provides a hard cap independent of token counting to ensure
/// reasonable context even if token estimation is off.
pub const DEFAULT_MAX_MESSAGES: usize = 30;

/// Truncates messages to fit within a token budget.
///
/// Uses the default message limit (`DEFAULT_MAX_MESSAGES`).
///
/// # Algorithm
///
/// 1. Always keep the first message (system/project context)
/// 2. Work backwards from most recent messages
/// 3. Include messages until budget is exhausted
/// 4. Return messages in chronological order
///
/// # Arguments
///
/// * `messages` - The full conversation history
/// * `max_tokens` - Maximum total tokens allowed
///
/// # Returns
///
/// A new vector with messages truncated to fit the budget.
/// The first message is always preserved if it exists.
///
/// # Example
///
/// ```rust
/// use patina::api::context::truncate_context;
/// use patina::types::ApiMessageV2;
///
/// let messages = vec![
///     ApiMessageV2::user("System prompt"),
///     ApiMessageV2::assistant("Response 1"),
///     ApiMessageV2::user("Query 2"),
/// ];
///
/// let truncated = truncate_context(&messages, 1000);
/// assert!(!truncated.is_empty());
/// ```
#[must_use]
pub fn truncate_context(messages: &[ApiMessageV2], max_tokens: usize) -> Vec<ApiMessageV2> {
    truncate_context_with_limits(messages, max_tokens, DEFAULT_MAX_MESSAGES)
}

/// Compacts or truncates messages to fit within a token budget.
///
/// This function first attempts to use compaction (summarizing old messages).
/// If compaction fails or is not applicable, it falls back to truncation.
///
/// # Arguments
///
/// * `messages` - The full conversation history
/// * `max_tokens` - Maximum total tokens allowed
/// * `preserve_recent` - Number of recent messages to preserve verbatim
///
/// # Returns
///
/// A new vector with messages compacted/truncated to fit the budget.
/// The first message is always preserved if it exists.
///
/// # Example
///
/// ```rust
/// use patina::api::context::compact_or_truncate_context;
/// use patina::types::ApiMessageV2;
///
/// let messages = vec![
///     ApiMessageV2::user("System prompt"),
///     ApiMessageV2::assistant("Response 1"),
///     ApiMessageV2::user("Query 2"),
///     ApiMessageV2::assistant("Response 2"),
/// ];
///
/// let result = compact_or_truncate_context(&messages, 1000, 2);
/// assert!(!result.is_empty());
/// ```
#[must_use]
pub fn compact_or_truncate_context(
    messages: &[ApiMessageV2],
    max_tokens: usize,
    preserve_recent: usize,
) -> Vec<ApiMessageV2> {
    // Try compaction first
    let compactor = ContextCompactor::new_mock();
    let config = CompactionConfig {
        target_tokens: max_tokens,
        preserve_recent,
        ..Default::default()
    };

    match compactor.compact(messages, &config) {
        Ok(result) => {
            if result.saved_tokens > 0 {
                tracing::info!(
                    saved_tokens = result.saved_tokens,
                    original_count = messages.len(),
                    compacted_count = result.messages.len(),
                    "Context compacted successfully"
                );
            }
            result.messages
        }
        Err(e) => {
            tracing::warn!(error = %e, "Compaction failed, falling back to truncation");
            truncate_context_with_limits(messages, max_tokens, preserve_recent)
        }
    }
}

/// Truncates messages with explicit token and message limits.
///
/// # Arguments
///
/// * `messages` - The full conversation history
/// * `max_tokens` - Maximum total tokens allowed
/// * `max_messages` - Maximum number of messages (excluding first)
///
/// # Returns
///
/// A new vector with messages truncated to fit both limits.
/// The first message is always preserved if it exists.
#[must_use]
pub fn truncate_context_with_limits(
    messages: &[ApiMessageV2],
    max_tokens: usize,
    max_messages: usize,
) -> Vec<ApiMessageV2> {
    if messages.is_empty() {
        return Vec::new();
    }

    // Always keep the first message (system/project context)
    let first_message = &messages[0];
    let first_tokens = estimate_message_tokens(first_message);

    if first_tokens >= max_tokens {
        // Even first message exceeds budget - return it anyway with warning
        tracing::warn!(
            first_tokens,
            max_tokens,
            "First message exceeds token budget"
        );
        return vec![first_message.clone()];
    }

    let mut result = vec![first_message.clone()];
    let mut remaining_budget = max_tokens.saturating_sub(first_tokens);
    let mut messages_to_add = Vec::new();

    // Work backwards from most recent, collecting messages that fit
    for msg in messages[1..].iter().rev() {
        if messages_to_add.len() >= max_messages {
            break;
        }

        let msg_tokens = estimate_message_tokens(msg);
        if msg_tokens > remaining_budget {
            // This message doesn't fit - stop here
            // (We could skip and continue, but that risks breaking conversation flow)
            break;
        }

        remaining_budget = remaining_budget.saturating_sub(msg_tokens);
        messages_to_add.push(msg.clone());
    }

    // Reverse to restore chronological order
    messages_to_add.reverse();
    result.extend(messages_to_add);

    let truncated_count = messages.len().saturating_sub(result.len());
    if truncated_count > 0 {
        tracing::info!(
            original_count = messages.len(),
            truncated_count = result.len(),
            messages_dropped = truncated_count,
            estimated_tokens = max_tokens - remaining_budget,
            "Context truncated to fit token budget"
        );
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ContentBlock, MessageContent};
    use serde_json::json;

    fn make_message(role: &str, content: &str) -> ApiMessageV2 {
        match role {
            "user" => ApiMessageV2::user(content),
            "assistant" => ApiMessageV2::assistant(content),
            _ => panic!("Invalid role"),
        }
    }

    // =========================================================================
    // truncate_context tests
    // =========================================================================

    #[test]
    fn test_truncate_empty_messages() {
        let messages: Vec<ApiMessageV2> = vec![];
        let truncated = truncate_context(&messages, 1000);
        assert!(truncated.is_empty());
    }

    #[test]
    fn test_truncate_single_message() {
        let messages = vec![make_message("user", "Hello")];
        let truncated = truncate_context(&messages, 1000);
        assert_eq!(truncated.len(), 1);
    }

    #[test]
    fn test_truncate_under_budget_unchanged() {
        let messages = vec![
            make_message("user", "Hello"),
            make_message("assistant", "Hi there!"),
        ];
        let truncated = truncate_context(&messages, 10_000);
        assert_eq!(truncated.len(), 2);
    }

    #[test]
    fn test_truncate_preserves_first_message() {
        // First message is system/project context - must always keep
        let messages = vec![
            make_message("user", "System prompt with project context..."),
            make_message("assistant", "Response 1"),
            make_message("user", "Query 2"),
            make_message("assistant", "Response 2"),
            make_message("user", "Query 3"),
        ];

        // Budget that can only fit first + last 2
        let truncated = truncate_context(&messages, 100);

        // First message always preserved
        assert_eq!(
            truncated[0].content.to_text(),
            messages[0].content.to_text()
        );
        // Most recent messages preserved
        assert!(truncated.len() >= 2);
        assert_eq!(
            truncated.last().unwrap().content.to_text(),
            messages.last().unwrap().content.to_text()
        );
    }

    #[test]
    fn test_truncate_respects_token_budget() {
        let large_content = "x".repeat(10_000); // ~2500 tokens
        let messages = vec![
            make_message("user", "System"),
            make_message("assistant", &large_content),
            make_message("user", &large_content),
            make_message("assistant", &large_content),
        ];

        let truncated = truncate_context(&messages, 5_000); // ~5k token budget
        let total_tokens: usize = truncated.iter().map(estimate_message_tokens).sum();

        // Should be under or around budget (within margin for overhead)
        assert!(
            total_tokens <= 6_000,
            "Should be near budget, got {} tokens",
            total_tokens
        );
    }

    #[test]
    fn test_truncate_maintains_conversation_order() {
        let messages = vec![
            make_message("user", "First"),
            make_message("assistant", "Second"),
            make_message("user", "Third"),
            make_message("assistant", "Fourth"),
        ];

        let truncated = truncate_context(&messages, 10_000);

        // Verify order is preserved
        let texts: Vec<_> = truncated.iter().map(|m| m.content.to_text()).collect();
        assert_eq!(texts, vec!["First", "Second", "Third", "Fourth"]);
    }

    #[test]
    fn test_truncate_with_tool_blocks() {
        // Ensure tool_use and tool_result blocks are handled
        let messages = vec![
            make_message("user", "Run ls"),
            ApiMessageV2::assistant_with_content(MessageContent::blocks(vec![
                ContentBlock::text("Running command..."),
                ContentBlock::tool_use("tool_1", "bash", json!({"command": "ls"})),
            ])),
            ApiMessageV2::user_with_content(MessageContent::blocks(vec![
                ContentBlock::tool_result("tool_1", "file1.txt\nfile2.txt"),
            ])),
        ];

        let truncated = truncate_context(&messages, 10_000);
        assert_eq!(truncated.len(), 3);
    }

    // =========================================================================
    // truncate_context_with_limits tests
    // =========================================================================

    #[test]
    fn test_truncate_max_messages_limit() {
        // Even if under token budget, limit message count
        let messages: Vec<_> = (0..100)
            .map(|i| make_message("user", &format!("Message {}", i)))
            .collect();

        let truncated = truncate_context_with_limits(&messages, 1_000_000, 20);
        // First message + up to 20 recent messages
        assert!(truncated.len() <= 21);
    }

    #[test]
    fn test_truncate_preserves_first_even_if_over_budget() {
        // If first message alone exceeds budget, still return it
        let large_first = "x".repeat(100_000); // ~25k tokens
        let messages = vec![
            make_message("user", &large_first),
            make_message("assistant", "Short response"),
        ];

        let truncated = truncate_context(&messages, 100); // Very small budget

        // First message should still be included
        assert!(!truncated.is_empty());
        assert_eq!(truncated[0].content.to_text(), large_first);
    }

    #[test]
    fn test_truncate_drops_middle_messages() {
        let messages = vec![
            make_message("user", "First (system)"),
            make_message("assistant", "Old response 1"),
            make_message("user", "Old query 2"),
            make_message("assistant", "Old response 2"),
            make_message("user", "Recent query"),
            make_message("assistant", "Recent response"),
        ];

        // Budget that fits first + last 2 messages only
        let truncated = truncate_context_with_limits(&messages, 200, 2);

        // Should have first + last 2
        assert!(truncated.len() <= 3);
        // First is preserved
        assert_eq!(truncated[0].content.to_text(), "First (system)");
        // Most recent is preserved
        assert_eq!(
            truncated.last().unwrap().content.to_text(),
            "Recent response"
        );
    }

    #[test]
    fn test_truncate_with_zero_max_messages() {
        let messages = vec![
            make_message("user", "First"),
            make_message("assistant", "Second"),
        ];

        let truncated = truncate_context_with_limits(&messages, 10_000, 0);

        // Should only have first message
        assert_eq!(truncated.len(), 1);
        assert_eq!(truncated[0].content.to_text(), "First");
    }

    #[test]
    fn test_truncate_realistic_conversation() {
        // Simulate a realistic conversation with varied message lengths
        let mut messages = vec![make_message(
            "user",
            "You are a helpful assistant for project X...",
        )];

        for i in 0..20 {
            messages.push(make_message(
                "user",
                &format!("Question {}: How do I do something?", i),
            ));
            messages.push(make_message(
                "assistant",
                &format!(
                    "Response {}: Here's how you can do that. First, you need to...",
                    i
                ),
            ));
        }

        // Truncate to fit 50k tokens
        let truncated = truncate_context(&messages, 50_000);

        // Should have fewer messages
        assert!(truncated.len() < messages.len());
        // But should still have useful context
        assert!(truncated.len() > 5);
        // First message preserved
        assert!(truncated[0].content.to_text().contains("project X"));
    }

    #[test]
    fn test_truncate_alternating_roles() {
        let messages = vec![
            make_message("user", "U1"),
            make_message("assistant", "A1"),
            make_message("user", "U2"),
            make_message("assistant", "A2"),
            make_message("user", "U3"),
            make_message("assistant", "A3"),
        ];

        let truncated = truncate_context(&messages, 10_000);

        // Should maintain alternating user/assistant pattern
        for (i, msg) in truncated.iter().enumerate() {
            if i == 0 {
                assert_eq!(msg.role, crate::types::Role::User);
            } else if i % 2 == 1 {
                // Odd indices (1, 3, 5) should be assistant
                assert_eq!(msg.role, crate::types::Role::Assistant);
            } else {
                // Even indices (2, 4) should be user
                assert_eq!(msg.role, crate::types::Role::User);
            }
        }
    }

    // =========================================================================
    // compact_or_truncate_context tests
    // =========================================================================

    #[test]
    fn test_compact_or_truncate_empty() {
        let messages: Vec<ApiMessageV2> = vec![];
        let result = compact_or_truncate_context(&messages, 1000, 2);
        assert!(result.is_empty());
    }

    #[test]
    fn test_compact_or_truncate_under_budget_unchanged() {
        let messages = vec![
            make_message("user", "Hello"),
            make_message("assistant", "Hi there!"),
        ];
        let result = compact_or_truncate_context(&messages, 10_000, 2);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_compact_or_truncate_preserves_first_message() {
        let messages = vec![
            make_message("user", "System prompt with project context"),
            make_message("assistant", "Ready to help"),
            make_message("user", "Task 1"),
            make_message("assistant", "Done 1"),
        ];
        let result = compact_or_truncate_context(&messages, 100, 2);
        assert!(!result.is_empty());
        assert_eq!(
            result[0].content.to_text(),
            "System prompt with project context"
        );
    }

    #[test]
    fn test_compact_or_truncate_preserves_recent() {
        // Create a long conversation that needs compaction
        let long_padding = "x".repeat(200);
        let mut messages = vec![make_message("user", &format!("System {}", long_padding))];
        for i in 0..10 {
            messages.push(make_message("user", &format!("Q{} {}", i, long_padding)));
            messages.push(make_message(
                "assistant",
                &format!("A{} {}", i, long_padding),
            ));
        }

        let result = compact_or_truncate_context(&messages, 200, 2);

        // Last message should be preserved
        let last_original = messages.last().unwrap().content.to_text();
        let last_result = result.last().unwrap().content.to_text();
        assert_eq!(last_result, last_original);
    }
}
