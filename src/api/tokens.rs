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

/// Default token estimate for images when dimensions are unknown.
///
/// Based on a typical 1024x1024 image which is approximately 1400 tokens.
/// We use 1500 as a conservative estimate.
pub const DEFAULT_IMAGE_TOKENS: usize = 1500;

/// Estimates token count for an image based on its dimensions.
///
/// Uses Claude's official image token formula: `(width × height) / 750`.
///
/// # Arguments
///
/// * `width` - Image width in pixels
/// * `height` - Image height in pixels
///
/// # Returns
///
/// Estimated token count for the image
///
/// # Examples
///
/// ```rust
/// use patina::api::tokens::estimate_image_tokens;
///
/// // 1024x1024 image (uses ceiling division to avoid underestimating)
/// let tokens = estimate_image_tokens(1024, 1024);
/// assert_eq!(tokens, 1399); // ceil(1024 * 1024 / 750) = 1399
///
/// // Small thumbnail
/// let tokens = estimate_image_tokens(100, 100);
/// assert_eq!(tokens, 14); // ceil(100 * 100 / 750) = 14
/// ```
#[must_use]
pub fn estimate_image_tokens(width: u32, height: u32) -> usize {
    let pixels = u64::from(width) * u64::from(height);
    // Use ceiling division to avoid underestimating
    pixels.div_ceil(750) as usize
}

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
            // Use estimate_image_tokens(width, height) if dimensions are known.
            DEFAULT_IMAGE_TOKENS
        }
    }
}

/// Token estimator with configurable safety buffer and advanced features.
///
/// Provides enhanced token estimation with:
/// - Configurable safety buffer (default 10%)
/// - Unicode-aware character counting
/// - Code block detection with adjusted ratios
///
/// # Examples
///
/// ```rust
/// use patina::api::tokens::TokenEstimator;
///
/// let estimator = TokenEstimator::default();
/// let tokens = estimator.estimate_with_buffer("Hello, world!");
/// ```
#[derive(Debug, Clone)]
pub struct TokenEstimator {
    safety_buffer_percent: u8,
}

impl Default for TokenEstimator {
    fn default() -> Self {
        Self {
            safety_buffer_percent: 10,
        }
    }
}

impl TokenEstimator {
    /// Creates a new `TokenEstimator` with a custom safety buffer percentage.
    ///
    /// # Arguments
    ///
    /// * `percent` - Safety buffer percentage (0-100)
    ///
    /// # Returns
    ///
    /// A new `TokenEstimator` with the specified safety buffer
    #[must_use]
    pub fn with_safety_buffer(percent: u8) -> Self {
        Self {
            safety_buffer_percent: percent.min(100),
        }
    }

    /// Returns the configured safety buffer percentage.
    #[must_use]
    pub fn safety_buffer_percent(&self) -> u8 {
        self.safety_buffer_percent
    }

    /// Estimates tokens for text without any adjustments.
    ///
    /// Uses the standard ~4 characters per token heuristic.
    #[must_use]
    pub fn estimate(&self, text: &str) -> usize {
        estimate_tokens(text)
    }

    /// Estimates tokens with the configured safety buffer applied.
    ///
    /// The safety buffer adds a percentage on top of the base estimate
    /// to avoid underestimation.
    ///
    /// # Example
    ///
    /// With a 10% safety buffer, 100 base tokens becomes 110 tokens.
    #[must_use]
    pub fn estimate_with_buffer(&self, text: &str) -> usize {
        let base = estimate_tokens(text);
        let buffer = base * usize::from(self.safety_buffer_percent) / 100;
        base + buffer
    }

    /// Estimates tokens with Unicode-aware grapheme counting.
    ///
    /// This method counts grapheme clusters instead of bytes, which provides
    /// more accurate estimates for non-ASCII text (e.g., CJK characters,
    /// emoji).
    ///
    /// CJK characters use ~1.5 tokens per grapheme (conservative estimate).
    /// ASCII characters use ~0.25 tokens per grapheme (~4 chars/token).
    #[must_use]
    pub fn estimate_unicode_aware(&self, text: &str) -> usize {
        if text.is_empty() {
            return 0;
        }

        // Count graphemes manually (avoiding external dependency)
        // For CJK detection, check if most chars are non-ASCII
        let char_count = text.chars().count();
        let ascii_count = text.chars().filter(|c| c.is_ascii()).count();
        let non_ascii_ratio = if char_count > 0 {
            (char_count - ascii_count) as f64 / char_count as f64
        } else {
            0.0
        };

        // If mostly non-ASCII (CJK, etc.), use ~1.5 tokens per char
        // Otherwise, use standard ~0.25 tokens per char
        if non_ascii_ratio > 0.5 {
            // CJK-heavy text: approximately 1.5 tokens per character
            (char_count as f64 * 1.5).ceil() as usize
        } else {
            // ASCII-heavy text: use standard estimation
            text.len().div_ceil(4)
        }
    }

    /// Estimates tokens with code block detection.
    ///
    /// Returns both the estimate and whether the text was detected as code.
    /// Code blocks use ~3 chars/token (more dense than prose).
    ///
    /// # Returns
    ///
    /// A tuple of (estimated_tokens, is_code)
    #[must_use]
    pub fn estimate_with_code_detection(&self, text: &str) -> (usize, bool) {
        let is_code = Self::detect_code(text);
        let estimate = if is_code {
            // Code is denser: ~3 chars per token
            text.len().div_ceil(3)
        } else {
            // Prose: ~4 chars per token
            text.len().div_ceil(4)
        };
        (estimate, is_code)
    }

    /// Detects if text appears to be code.
    ///
    /// Uses heuristics like presence of braces, semicolons, function keywords, etc.
    fn detect_code(text: &str) -> bool {
        if text.is_empty() {
            return false;
        }

        // Code indicators
        let code_patterns = [
            "fn ", "func ", "function ", "def ", "class ", "struct ", "impl ",
            "pub ", "private ", "public ", "const ", "let ", "var ",
            "if ", "else ", "for ", "while ", "match ", "switch ",
            "return ", "import ", "use ", "from ", "require(",
            " => ", " -> ", "->", "::", "&&", "||",
            "();", ");", "};", "{{", "}}",
        ];

        let matches = code_patterns
            .iter()
            .filter(|p| text.contains(*p))
            .count();

        // Also check for balanced braces and common syntax
        let has_braces = text.contains('{') && text.contains('}');
        let has_parens = text.contains('(') && text.contains(')');
        let has_semicolons = text.matches(';').count() >= 2;

        // Consider it code if multiple indicators match
        matches >= 2 || (has_braces && has_semicolons) || (has_parens && matches >= 1)
    }
}

/// Token budget tracker for monitoring API usage against limits.
///
/// Tracks token usage and provides warning/critical thresholds to help
/// avoid exceeding context limits.
///
/// # Default Thresholds
///
/// - Warning: 80% of limit
/// - Critical: 95% of limit
///
/// # Examples
///
/// ```rust
/// use patina::api::tokens::TokenBudget;
///
/// let mut budget = TokenBudget::new(100_000);
/// budget.add_usage(50_000);
/// assert_eq!(budget.remaining(), 50_000);
/// assert!(!budget.is_warning());
/// ```
#[derive(Debug, Clone)]
pub struct TokenBudget {
    limit: usize,
    used: usize,
    warning_threshold_percent: u8,
    critical_threshold_percent: u8,
}

impl TokenBudget {
    /// Creates a new token budget with the given limit.
    ///
    /// Uses default thresholds: 80% warning, 95% critical.
    #[must_use]
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            used: 0,
            warning_threshold_percent: 80,
            critical_threshold_percent: 95,
        }
    }

    /// Creates a new token budget with custom thresholds.
    ///
    /// # Arguments
    ///
    /// * `limit` - Maximum token count
    /// * `warning_percent` - Percentage at which to show warning (0-100)
    /// * `critical_percent` - Percentage at which to show critical (0-100)
    #[must_use]
    pub fn with_thresholds(limit: usize, warning_percent: u8, critical_percent: u8) -> Self {
        Self {
            limit,
            used: 0,
            warning_threshold_percent: warning_percent.min(100),
            critical_threshold_percent: critical_percent.min(100),
        }
    }

    /// Returns the token limit.
    #[must_use]
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Returns the number of tokens used.
    #[must_use]
    pub fn used(&self) -> usize {
        self.used
    }

    /// Returns the number of tokens remaining.
    ///
    /// Never returns a negative value; returns 0 if exceeded.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.limit.saturating_sub(self.used)
    }

    /// Adds to the token usage count.
    pub fn add_usage(&mut self, tokens: usize) {
        self.used = self.used.saturating_add(tokens);
    }

    /// Returns the percentage of budget used (0-100+).
    #[must_use]
    pub fn percentage_used(&self) -> usize {
        if self.limit == 0 {
            return 100;
        }
        (self.used * 100) / self.limit
    }

    /// Returns true if usage is at or above the warning threshold.
    #[must_use]
    pub fn is_warning(&self) -> bool {
        self.percentage_used() >= usize::from(self.warning_threshold_percent)
    }

    /// Returns true if usage is at or above the critical threshold.
    #[must_use]
    pub fn is_critical(&self) -> bool {
        self.percentage_used() >= usize::from(self.critical_threshold_percent)
    }

    /// Returns true if usage exceeds the limit.
    #[must_use]
    pub fn is_exceeded(&self) -> bool {
        self.used > self.limit
    }

    /// Resets usage to zero.
    pub fn reset(&mut self) {
        self.used = 0;
    }
}

impl std::fmt::Display for TokenBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{} tokens ({}%)", self.used, self.limit, self.percentage_used())
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

    // =========================================================================
    // estimate_image_tokens tests
    // =========================================================================

    #[test]
    fn test_estimate_image_tokens_standard_1024x1024() {
        // Claude's formula: (width * height) / 750
        // 1024 * 1024 = 1,048,576 / 750 = 1398.1 -> 1399 (ceiling)
        let tokens = super::estimate_image_tokens(1024, 1024);
        assert_eq!(tokens, 1399);
    }

    #[test]
    fn test_estimate_image_tokens_small_100x100() {
        // 100 * 100 = 10,000 / 750 = 13.3 -> 14 (ceiling)
        let tokens = super::estimate_image_tokens(100, 100);
        assert_eq!(tokens, 14);
    }

    #[test]
    fn test_estimate_image_tokens_wide_1920x1080() {
        // HD image: 1920 * 1080 = 2,073,600 / 750 = 2764.8 -> 2765 (ceiling)
        let tokens = super::estimate_image_tokens(1920, 1080);
        assert_eq!(tokens, 2765);
    }

    #[test]
    fn test_estimate_image_tokens_tall_1080x1920() {
        // Portrait HD: same pixel count as wide
        let tokens = super::estimate_image_tokens(1080, 1920);
        assert_eq!(tokens, 2765);
    }

    #[test]
    fn test_estimate_image_tokens_tiny_10x10() {
        // Very small: 10 * 10 = 100 / 750 = 0.13 -> 1 (ceiling, never zero)
        let tokens = super::estimate_image_tokens(10, 10);
        assert_eq!(tokens, 1);
    }

    #[test]
    fn test_estimate_image_tokens_single_pixel() {
        // 1x1 = 1 / 750 = 0.001 -> 1 (ceiling)
        let tokens = super::estimate_image_tokens(1, 1);
        assert_eq!(tokens, 1);
    }

    #[test]
    fn test_estimate_image_tokens_4k_3840x2160() {
        // 4K image: 3840 * 2160 = 8,294,400 / 750 = 11,059.2 -> 11060
        let tokens = super::estimate_image_tokens(3840, 2160);
        assert_eq!(tokens, 11060);
    }

    #[test]
    fn test_estimate_image_tokens_exact_multiple_750x1() {
        // Edge case: exactly divisible
        // 750 * 1 = 750 / 750 = 1
        let tokens = super::estimate_image_tokens(750, 1);
        assert_eq!(tokens, 1);
    }

    #[test]
    fn test_estimate_image_tokens_zero_dimension() {
        // Edge case: zero dimension results in zero tokens
        let tokens = super::estimate_image_tokens(0, 1000);
        assert_eq!(tokens, 0);
        let tokens = super::estimate_image_tokens(1000, 0);
        assert_eq!(tokens, 0);
        let tokens = super::estimate_image_tokens(0, 0);
        assert_eq!(tokens, 0);
    }

    #[test]
    fn test_estimate_image_tokens_large_no_overflow() {
        // Large image: ensure no overflow with u32::MAX values
        // Using 65535 x 65535 (max reasonable image size)
        // 65535 * 65535 = 4,294,836,225 / 750 = 5,726,448.3 -> 5726449
        let tokens = super::estimate_image_tokens(65535, 65535);
        assert_eq!(tokens, 5726449);
    }

    #[test]
    fn test_default_image_tokens_constant() {
        // Verify the default constant is reasonable for unknown dimensions
        assert_eq!(super::DEFAULT_IMAGE_TOKENS, 1500);
        // Should be close to a 1024x1024 image estimate
        let calculated = super::estimate_image_tokens(1024, 1024);
        assert!(
            (calculated as i64 - super::DEFAULT_IMAGE_TOKENS as i64).abs() < 200,
            "DEFAULT_IMAGE_TOKENS should be close to 1024x1024 estimate"
        );
    }

    // =========================================================================
    // TokenEstimator tests (0.5.1 - Enhanced token estimation)
    // =========================================================================

    #[test]
    fn test_token_estimator_default_safety_buffer() {
        // Default safety buffer should be 10%
        let estimator = TokenEstimator::default();
        assert_eq!(estimator.safety_buffer_percent(), 10);
    }

    #[test]
    fn test_token_estimator_custom_safety_buffer() {
        let estimator = TokenEstimator::with_safety_buffer(15);
        assert_eq!(estimator.safety_buffer_percent(), 15);
    }

    #[test]
    fn test_estimate_tokens_with_safety_buffer() {
        let estimator = TokenEstimator::with_safety_buffer(10);
        // 100 base tokens + 10% = 110 tokens
        let text = "x".repeat(400); // 100 base tokens at 4 chars/token
        let estimate = estimator.estimate_with_buffer(&text);
        assert_eq!(estimate, 110);
    }

    #[test]
    fn test_estimate_tokens_unicode_aware() {
        let estimator = TokenEstimator::default();
        // Unicode characters should be counted by grapheme clusters, not bytes
        // "こんにちは" is 5 graphemes but 15 bytes in UTF-8
        let japanese = "こんにちは";
        let estimate = estimator.estimate_unicode_aware(japanese);
        // Should estimate based on grapheme count, not byte count
        // 5 graphemes * ~1.5 tokens/grapheme (conservative for CJK) = ~8 tokens
        assert!(
            (5..=10).contains(&estimate),
            "Expected 5-10 tokens for 5 Japanese characters, got {}",
            estimate
        );
    }

    #[test]
    fn test_estimate_tokens_code_blocks_detected() {
        let estimator = TokenEstimator::default();
        // Code blocks should use ~3 chars/token ratio (more tokens per char)
        let code = "fn main() { println!(\"Hello\"); }";
        let (base_estimate, is_code) = estimator.estimate_with_code_detection(code);
        assert!(is_code, "Should detect as code");
        // 33 chars at 3 chars/token = 11 tokens
        assert!(
            (10..=15).contains(&base_estimate),
            "Expected 10-15 tokens for code, got {}",
            base_estimate
        );
    }

    #[test]
    fn test_estimate_tokens_prose_not_detected_as_code() {
        let estimator = TokenEstimator::default();
        let prose = "Hello, how are you doing today? I hope you're well.";
        let (_, is_code) = estimator.estimate_with_code_detection(prose);
        assert!(!is_code, "Prose should not be detected as code");
    }

    #[test]
    fn test_estimate_tokens_matches_claude_approximation() {
        let estimator = TokenEstimator::default();
        // Claude's approximation is ~4 chars per token for English
        // Test with known text to verify we're in the ballpark
        let text = "The quick brown fox jumps over the lazy dog."; // 44 chars
        let estimate = estimator.estimate(text);
        // Expected: 44 / 4 = 11, with ceiling = 11
        assert!(
            (10..=15).contains(&estimate),
            "Expected ~11 tokens (Claude approximation), got {}",
            estimate
        );
    }

    // =========================================================================
    // TokenBudget tests (0.5.1 - Token budget tracking)
    // =========================================================================

    #[test]
    fn test_token_budget_new() {
        let budget = TokenBudget::new(100_000);
        assert_eq!(budget.limit(), 100_000);
        assert_eq!(budget.used(), 0);
        assert_eq!(budget.remaining(), 100_000);
    }

    #[test]
    fn test_token_budget_add_usage() {
        let mut budget = TokenBudget::new(100_000);
        budget.add_usage(5_000);
        assert_eq!(budget.used(), 5_000);
        assert_eq!(budget.remaining(), 95_000);
    }

    #[test]
    fn test_token_budget_tracking_accumulates() {
        let mut budget = TokenBudget::new(100_000);
        budget.add_usage(1_000);
        budget.add_usage(2_000);
        budget.add_usage(3_000);
        assert_eq!(budget.used(), 6_000);
        assert_eq!(budget.remaining(), 94_000);
    }

    #[test]
    fn test_token_budget_percentage_used() {
        let mut budget = TokenBudget::new(100_000);
        budget.add_usage(75_000);
        assert_eq!(budget.percentage_used(), 75);
    }

    #[test]
    fn test_token_budget_warns_near_limit() {
        let mut budget = TokenBudget::new(100_000);

        // Under 80% - no warning
        budget.add_usage(79_000);
        assert!(!budget.is_warning(), "Should not warn at 79%");

        // At 80% - warning
        budget.add_usage(1_000); // Now at 80%
        assert!(budget.is_warning(), "Should warn at 80%");
    }

    #[test]
    fn test_token_budget_critical_near_limit() {
        let mut budget = TokenBudget::new(100_000);
        budget.add_usage(95_000);
        assert!(budget.is_critical(), "Should be critical at 95%");
    }

    #[test]
    fn test_token_budget_exceeds_limit() {
        let mut budget = TokenBudget::new(100_000);
        budget.add_usage(100_001);
        assert!(budget.is_exceeded(), "Should be exceeded when over limit");
        assert_eq!(budget.remaining(), 0); // Remaining should not go negative
    }

    #[test]
    fn test_token_budget_reset() {
        let mut budget = TokenBudget::new(100_000);
        budget.add_usage(50_000);
        budget.reset();
        assert_eq!(budget.used(), 0);
        assert_eq!(budget.remaining(), 100_000);
    }

    #[test]
    fn test_token_budget_custom_thresholds() {
        let budget = TokenBudget::with_thresholds(100_000, 70, 90);
        let mut budget = budget;

        budget.add_usage(70_000);
        assert!(budget.is_warning(), "Should warn at custom 70% threshold");

        budget.add_usage(20_000); // Now at 90%
        assert!(budget.is_critical(), "Should be critical at custom 90% threshold");
    }

    #[test]
    fn test_token_budget_display_format() {
        let mut budget = TokenBudget::new(100_000);
        budget.add_usage(25_000);
        let display = format!("{}", budget);
        assert!(
            display.contains("25,000") || display.contains("25000"),
            "Display should show used tokens"
        );
        assert!(
            display.contains("100,000") || display.contains("100000"),
            "Display should show limit"
        );
    }
}
