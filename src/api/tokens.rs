//! Token estimation utilities.
//!
//! Provides heuristic-based token counting for API request budgeting.
//! These estimates are intentionally conservative to avoid exceeding limits.
//!
//! # Token Estimation
//!
//! The Claude API charges based on tokens, which roughly correspond to ~4 characters
//! for English text. This module provides utilities to estimate token counts for:
//!
//! - Raw text strings
//! - API messages (with role overhead)
//! - Content blocks (text, tool_use, tool_result)
//!
//! # Example
//!
//! ```rust
//! use patina::api::tokens::{estimate_tokens, estimate_messages_tokens};
//! use patina::types::ApiMessageV2;
//!
//! // Estimate tokens for raw text
//! let tokens = estimate_tokens("Hello, how are you?");
//! assert!(tokens >= 4 && tokens <= 10);
//!
//! // Estimate tokens for messages
//! let messages = vec![
//!     ApiMessageV2::user("Hello"),
//!     ApiMessageV2::assistant("Hi there!"),
//! ];
//! let total = estimate_messages_tokens(&messages);
//! ```

use crate::types::{ApiMessageV2, ContentBlock, MessageContent};

/// Estimates token count for a string.
///
/// Uses the heuristic of ~4 characters per token, which is reasonably
/// accurate for English text and code. This intentionally overestimates
/// slightly to provide safety margin.
///
/// # Arguments
///
/// * `text` - The text to estimate tokens for
///
/// # Returns
///
/// Estimated token count (uses ceiling division to never underestimate)
///
/// # Examples
///
/// ```rust
/// use patina::api::tokens::estimate_tokens;
///
/// assert_eq!(estimate_tokens(""), 0);
/// assert_eq!(estimate_tokens("Hello"), 2); // 5 chars / 4 = 1.25, ceiling = 2
/// ```
#[must_use]
pub fn estimate_tokens(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    // Use byte length for consistency with Unicode
    // Ceiling division rounds up to avoid underestimation
    text.len().div_ceil(4)
}

/// Estimates total tokens for a slice of API messages.
///
/// Sums the token estimates for all message content plus per-message overhead.
///
/// # Arguments
///
/// * `messages` - The messages to estimate tokens for
///
/// # Returns
///
/// Total estimated token count across all messages
#[must_use]
pub fn estimate_messages_tokens(messages: &[ApiMessageV2]) -> usize {
    messages.iter().map(estimate_message_tokens).sum()
}

/// Estimates tokens for a single API message.
///
/// Includes overhead for message structure (role, etc.) plus content tokens.
///
/// # Arguments
///
/// * `message` - The message to estimate tokens for
///
/// # Returns
///
/// Estimated token count for the message
#[must_use]
pub fn estimate_message_tokens(message: &ApiMessageV2) -> usize {
    // Base overhead for message structure (~4 tokens for role, separators, etc.)
    let overhead = 4;
    let content_tokens = estimate_content_tokens(&message.content);
    overhead + content_tokens
}

/// Estimates tokens for message content.
///
/// Handles both text content and content block arrays.
#[must_use]
fn estimate_content_tokens(content: &MessageContent) -> usize {
    match content {
        MessageContent::Text(text) => estimate_tokens(text),
        MessageContent::Blocks(blocks) => blocks.iter().map(estimate_block_tokens).sum(),
    }
}

/// Estimates tokens for a content block.
///
/// Different block types have different overhead:
/// - Text blocks: just the text content
/// - Tool use blocks: name + ID + JSON input
/// - Tool result blocks: ID + content + is_error flag
/// - Image blocks: estimated based on Claude's image token formula
#[must_use]
fn estimate_block_tokens(block: &ContentBlock) -> usize {
    match block {
        ContentBlock::Text { text } => estimate_tokens(text),
        ContentBlock::ToolUse(tool_use) => {
            // Tool name + ID + JSON input structure + overhead
            let overhead = 10; // For structure: {"type":"tool_use",...}
            overhead
                + estimate_tokens(&tool_use.name)
                + estimate_tokens(&tool_use.id)
                + estimate_tokens(&tool_use.input.to_string())
        }
        ContentBlock::ToolResult(tool_result) => {
            // Tool use ID + content + structure overhead
            let overhead = 10; // For structure: {"type":"tool_result",...}
            overhead
                + estimate_tokens(&tool_result.tool_use_id)
                + estimate_tokens(&tool_result.content)
        }
        ContentBlock::Image { .. } => {
            // Image tokens are calculated as (width × height) / 750 by Claude.
            // Without decoding the image, we use a conservative estimate based on
            // a typical image size. A 1024x1024 image ≈ 1400 tokens.
            // We use 1500 as a safe default estimate.
            1500
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // =========================================================================
    // estimate_tokens tests
    // =========================================================================

    #[test]
    fn test_estimate_tokens_empty_string() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn test_estimate_tokens_english_text() {
        // ~4 chars per token for English
        let text = "Hello, how are you doing today?"; // 31 chars
        let estimate = estimate_tokens(text);
        // 31 chars -> 31.div_ceil(4) = 8 tokens
        assert!(
            (6..=10).contains(&estimate),
            "Expected 6-10 tokens, got {}",
            estimate
        );
    }

    #[test]
    fn test_estimate_tokens_code() {
        // Code tends to have more tokens per character
        let code = "fn main() { println!(\"Hello\"); }";
        let estimate = estimate_tokens(code);
        // 33 chars -> 33.div_ceil(4) = 9 tokens
        assert!(
            (8..=15).contains(&estimate),
            "Expected 8-15 tokens, got {}",
            estimate
        );
    }

    #[test]
    fn test_estimate_tokens_json() {
        let json_str = r#"{"name": "test", "value": 42}"#;
        let estimate = estimate_tokens(json_str);
        // 28 chars -> 28.div_ceil(4) = 7 tokens
        assert!(
            (7..=12).contains(&estimate),
            "Expected 7-12 tokens, got {}",
            estimate
        );
    }

    #[test]
    fn test_estimate_tokens_unicode() {
        // Unicode should still work (counts bytes, not chars)
        let unicode = "こんにちは世界"; // Japanese
        let estimate = estimate_tokens(unicode);
        // Japanese characters are multi-byte, so this will be larger
        assert!(estimate > 0);
    }

    #[test]
    fn test_estimate_tokens_large_content() {
        let large = "x".repeat(100_000);
        let estimate = estimate_tokens(&large);
        // 100k chars -> (100000 + 3) / 4 = 25000 tokens (approximately)
        assert_eq!(estimate, 25_000);
    }

    #[test]
    fn test_estimate_tokens_single_char() {
        // Single character should be 1 token (ceiling of 1/4)
        assert_eq!(estimate_tokens("x"), 1);
    }

    #[test]
    fn test_estimate_tokens_four_chars() {
        // Exactly 4 chars should be 1 token
        assert_eq!(estimate_tokens("test"), 1);
    }

    #[test]
    fn test_estimate_tokens_five_chars() {
        // 5 chars should be 2 tokens (ceiling of 5/4)
        assert_eq!(estimate_tokens("hello"), 2);
    }

    // =========================================================================
    // estimate_message_tokens tests
    // =========================================================================

    #[test]
    fn test_estimate_message_tokens_user_text() {
        let msg = ApiMessageV2::user("Hello");
        let estimate = estimate_message_tokens(&msg);
        // 4 overhead + ~2 content tokens
        assert!(
            (5..=10).contains(&estimate),
            "Expected 5-10 tokens, got {}",
            estimate
        );
    }

    #[test]
    fn test_estimate_message_tokens_assistant_text() {
        let msg = ApiMessageV2::assistant("Hi there, how can I help?");
        let estimate = estimate_message_tokens(&msg);
        // 4 overhead + content tokens
        assert!(estimate >= 8);
    }

    #[test]
    fn test_estimate_message_tokens_with_tool_use() {
        let msg = ApiMessageV2::assistant_with_content(MessageContent::blocks(vec![
            ContentBlock::text("Let me run that command."),
            ContentBlock::tool_use("toolu_123", "bash", json!({"command": "ls -la"})),
        ]));
        let estimate = estimate_message_tokens(&msg);
        // Should include overhead for both blocks
        assert!(estimate > 20);
    }

    #[test]
    fn test_estimate_message_tokens_with_tool_result() {
        let msg = ApiMessageV2::user_with_content(MessageContent::blocks(vec![
            ContentBlock::tool_result("toolu_123", "file1.txt\nfile2.txt\nfile3.txt"),
        ]));
        let estimate = estimate_message_tokens(&msg);
        // Should include tool result overhead
        assert!(estimate > 10);
    }

    // =========================================================================
    // estimate_messages_tokens tests
    // =========================================================================

    #[test]
    fn test_estimate_messages_tokens_empty() {
        let messages: Vec<ApiMessageV2> = vec![];
        assert_eq!(estimate_messages_tokens(&messages), 0);
    }

    #[test]
    fn test_estimate_messages_tokens_single() {
        let messages = vec![ApiMessageV2::user("Hello")];
        let estimate = estimate_messages_tokens(&messages);
        assert!(estimate > 0);
    }

    #[test]
    fn test_estimate_messages_tokens_multiple() {
        let messages = vec![
            ApiMessageV2::user("What files are in this directory?"),
            ApiMessageV2::assistant("I'll check for you."),
            ApiMessageV2::user_with_content(MessageContent::blocks(vec![
                ContentBlock::tool_result("toolu_1", "file1.txt\nfile2.txt"),
            ])),
            ApiMessageV2::assistant("There are two files."),
        ];
        let estimate = estimate_messages_tokens(&messages);
        // Should be sum of all messages
        assert!(estimate > 30);
    }

    #[test]
    fn test_estimate_messages_tokens_large_conversation() {
        // Simulate a conversation with many messages
        let mut messages = Vec::new();
        for i in 0..50 {
            messages.push(ApiMessageV2::user(format!(
                "Message {} with some content",
                i
            )));
            messages.push(ApiMessageV2::assistant(format!(
                "Response {} with more content here",
                i
            )));
        }
        let estimate = estimate_messages_tokens(&messages);
        // 100 messages * ~15 tokens each = ~1500 tokens
        assert!(
            (1000..=3000).contains(&estimate),
            "Expected 1000-3000 tokens, got {}",
            estimate
        );
    }

    // =========================================================================
    // estimate_block_tokens tests
    // =========================================================================

    #[test]
    fn test_estimate_block_tokens_text() {
        let block = ContentBlock::text("Hello, world!");
        let estimate = estimate_block_tokens(&block);
        // Just the text, no extra overhead
        assert!((3..=5).contains(&estimate));
    }

    #[test]
    fn test_estimate_block_tokens_tool_use() {
        let block = ContentBlock::tool_use("toolu_abc123", "bash", json!({"command": "ls -la"}));
        let estimate = estimate_block_tokens(&block);
        // Overhead + name + id + input
        assert!(estimate > 15);
    }

    #[test]
    fn test_estimate_block_tokens_tool_result() {
        let block = ContentBlock::tool_result("toolu_abc123", "output here");
        let estimate = estimate_block_tokens(&block);
        // Overhead + id + content
        assert!(estimate > 10);
    }

    #[test]
    fn test_estimate_block_tokens_large_tool_result() {
        let large_output = "x".repeat(10_000);
        let block = ContentBlock::tool_result("toolu_1", &large_output);
        let estimate = estimate_block_tokens(&block);
        // Should account for large content
        assert!(estimate > 2500);
    }
}
