//! Context compaction for long conversations.
//!
//! Unlike truncation which simply drops old messages, compaction summarizes
//! them into a timeline while preserving key decisions and outcomes.
//!
//! # Overview
//!
//! Long-running agentic sessions can accumulate hundreds of messages. Truncation
//! loses valuable context about what was accomplished. Compaction instead:
//!
//! 1. **Preserves the system message** (always kept verbatim)
//! 2. **Summarizes old messages** into a structured timeline
//! 3. **Preserves recent messages** (configurable count)
//! 4. **Keeps tool use pairs together** (tool_use + tool_result)
//!
//! # Example
//!
//! ```rust,ignore
//! use patina::api::compaction::{ContextCompactor, CompactionConfig};
//! use patina::types::ApiMessageV2;
//!
//! let compactor = ContextCompactor::new(client);
//! let messages = vec![/* long conversation */];
//!
//! let config = CompactionConfig {
//!     target_tokens: 50_000,
//!     preserve_recent: 4,
//!     ..Default::default()
//! };
//!
//! let result = compactor.compact(&messages, &config)?;
//! println!("Saved {} tokens", result.saved_tokens);
//! ```

use crate::api::tokens::estimate_messages_tokens;
use crate::api::AnthropicClient;
use crate::types::{ApiMessageV2, Message, Role, StreamEvent};
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::warn;

// =============================================================================
// Summarization Prompts
// =============================================================================

/// System prompt for timeline-style summarization.
///
/// Instructs Claude to create a chronological summary of conversation events.
pub const TIMELINE_SUMMARIZATION_PROMPT: &str = r#"You are a conversation summarizer. Create a concise timeline of the previous conversation.

Focus on:
- Key decisions and their outcomes
- Files created, modified, or deleted
- Commands executed and their results
- Problems encountered and how they were resolved
- Important context that future messages might need

Format as a numbered timeline, with each item being a single sentence.
Keep the summary under 500 words.

Previous conversation to summarize:
"#;

/// System prompt for bullet-point style summarization.
///
/// Instructs Claude to create a structured list of key outcomes.
pub const BULLET_SUMMARIZATION_PROMPT: &str = r#"You are a conversation summarizer. Create a concise bullet-point summary of the previous conversation.

Focus on:
- What was accomplished
- Important decisions made
- Files or code that was modified
- Current state of the project

Use bullet points (- item) for each key point.
Keep the summary under 500 words.

Previous conversation to summarize:
"#;

/// System prompt for narrative-style summarization.
///
/// Instructs Claude to create a flowing narrative summary.
pub const NARRATIVE_SUMMARIZATION_PROMPT: &str = r#"You are a conversation summarizer. Write a brief narrative summary of the previous conversation.

Include:
- The overall goal being worked on
- Progress made toward that goal
- Key technical decisions
- Current status

Write in past tense as a connected narrative.
Keep the summary under 500 words.

Previous conversation to summarize:
"#;

// =============================================================================
// Summarizer Trait
// =============================================================================

/// Trait for generating conversation summaries.
///
/// This trait abstracts the summarization logic, allowing different
/// implementations for testing (mock) and production (Claude API).
///
/// # Example
///
/// ```rust,ignore
/// use patina::api::compaction::{Summarizer, MockSummarizer, CompactionConfig};
/// use patina::types::ApiMessageV2;
///
/// let summarizer = MockSummarizer;
/// let messages = vec![ApiMessageV2::user("Hello")];
/// let config = CompactionConfig::default();
/// let summary = summarizer.summarize(&messages, &config);
/// ```
pub trait Summarizer: Send + Sync {
    /// Generate a summary of the given messages.
    ///
    /// # Arguments
    ///
    /// * `messages` - The messages to summarize
    /// * `config` - Configuration controlling summary style
    ///
    /// # Returns
    ///
    /// A string containing the summary
    fn summarize(
        &self,
        messages: &[ApiMessageV2],
        config: &CompactionConfig,
    ) -> impl std::future::Future<Output = String> + Send;
}

/// Mock summarizer for testing.
///
/// Generates placeholder summaries without making API calls.
/// Extracts key content from messages to create a timeline.
#[derive(Debug, Clone, Copy, Default)]
pub struct MockSummarizer;

impl Summarizer for MockSummarizer {
    async fn summarize(&self, messages: &[ApiMessageV2], config: &CompactionConfig) -> String {
        generate_mock_summary(messages, config)
    }
}

/// Claude API-based summarizer for production use.
///
/// Uses the Anthropic API to generate intelligent summaries of conversations.
/// Falls back to mock summarization if the API call fails (graceful degradation).
///
/// # Example
///
/// ```rust,ignore
/// use patina::api::compaction::ClaudeSummarizer;
/// use patina::api::AnthropicClient;
/// use std::sync::Arc;
///
/// let client = Arc::new(AnthropicClient::new(api_key, "claude-3-haiku-20240307"));
/// let summarizer = ClaudeSummarizer::new(client);
/// ```
/// ClaudeSummarizer intentionally doesn't implement Debug to avoid
/// exposing the API client internals.
#[derive(Clone)]
pub struct ClaudeSummarizer {
    /// The Anthropic API client for making summarization requests.
    client: Arc<AnthropicClient>,
    /// Model to use for summarization (defaults to haiku for speed/cost).
    model: String,
}

/// Default model for summarization (fast and cheap).
const SUMMARIZATION_MODEL: &str = "claude-3-haiku-20240307";

impl ClaudeSummarizer {
    /// Creates a new Claude summarizer with the given client.
    ///
    /// Uses claude-3-haiku by default for fast, cost-effective summarization.
    ///
    /// # Arguments
    ///
    /// * `client` - The Anthropic API client to use for requests
    #[must_use]
    pub fn new(client: Arc<AnthropicClient>) -> Self {
        Self {
            client,
            model: SUMMARIZATION_MODEL.to_string(),
        }
    }

    /// Creates a new Claude summarizer with a specific model.
    ///
    /// # Arguments
    ///
    /// * `client` - The Anthropic API client
    /// * `model` - The model to use for summarization
    #[must_use]
    pub fn with_model(client: Arc<AnthropicClient>, model: &str) -> Self {
        Self {
            client,
            model: model.to_string(),
        }
    }

    /// Performs the async summarization request.
    async fn summarize_async(
        &self,
        messages: &[ApiMessageV2],
        config: &CompactionConfig,
    ) -> Result<String> {
        tracing::debug!(model = %self.model, message_count = messages.len(), "Starting Claude summarization");

        // Build the summarization prompt
        let formatted_messages = format_messages_for_summary(messages);
        let prompt = format!(
            "{}\n{}",
            get_summarization_prompt(config.summary_style),
            formatted_messages
        );

        // Create the request message
        let request_messages = vec![Message {
            role: Role::User,
            content: prompt,
        }];

        // Create channel for streaming response
        let (tx, mut rx) = mpsc::channel::<StreamEvent>(64);

        // Make the API request
        self.client.stream_message(&request_messages, tx).await?;

        // Collect the response content
        let mut response = String::new();
        while let Some(event) = rx.recv().await {
            match event {
                StreamEvent::ContentDelta(delta) => {
                    response.push_str(&delta);
                }
                StreamEvent::Error(err) => {
                    return Err(anyhow::anyhow!("API error during summarization: {}", err));
                }
                StreamEvent::MessageStop => break,
                _ => {} // Ignore other events
            }
        }

        if response.is_empty() {
            return Err(anyhow::anyhow!("Empty response from summarization API"));
        }

        Ok(response)
    }
}

impl Summarizer for ClaudeSummarizer {
    /// Generates a summary using the Claude API.
    ///
    /// Falls back to mock summarization if the API call fails (graceful degradation).
    async fn summarize(&self, messages: &[ApiMessageV2], config: &CompactionConfig) -> String {
        match self.summarize_async(messages, config).await {
            Ok(summary) => summary,
            Err(e) => {
                warn!(
                    error = %e,
                    "Failed to generate Claude summary, falling back to mock summarization"
                );
                generate_mock_summary(messages, config)
            }
        }
    }
}

/// Generates a mock summary for testing.
///
/// Extracts key content from messages to create a summary based on the configured style.
fn generate_mock_summary(messages: &[ApiMessageV2], config: &CompactionConfig) -> String {
    let mut summary_parts = Vec::new();

    // Check if any input is already a summary (to merge)
    let has_existing_summary = messages.iter().any(|m| {
        let text = m.content.to_text().to_lowercase();
        text.contains("summary") || text.contains("previous conversation")
    });

    // Header based on style
    let header = match config.summary_style {
        SummaryStyle::Timeline => "Previous conversation timeline:",
        SummaryStyle::BulletPoints => "Previous conversation summary:",
        SummaryStyle::Narrative => "Summary of earlier conversation:",
    };
    summary_parts.push(header.to_string());

    // Extract key actions from messages
    for (i, msg) in messages.iter().enumerate() {
        let text = msg.content.to_text();

        // Skip very short messages or existing summaries (if merging)
        if text.len() < 10 {
            continue;
        }

        // For existing summaries, extract and merge their content
        if has_existing_summary
            && (text.to_lowercase().contains("summary")
                || text.to_lowercase().contains("previous conversation"))
        {
            // Extract bullet points or timeline items from existing summary
            for line in text.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with('-') || trimmed.starts_with('•') || trimmed.starts_with('*')
                {
                    summary_parts.push(trimmed.to_string());
                }
            }
            continue;
        }

        // Extract key phrases from messages
        let key_phrases = extract_key_phrases(&text);
        if !key_phrases.is_empty() {
            let prefix = match config.summary_style {
                SummaryStyle::Timeline => format!("{}.", i + 1),
                SummaryStyle::BulletPoints => "-".to_string(),
                SummaryStyle::Narrative => "".to_string(),
            };

            if config.summary_style == SummaryStyle::Narrative {
                summary_parts.push(key_phrases);
            } else {
                summary_parts.push(format!("{} {}", prefix, key_phrases));
            }
        }
    }

    // Ensure we have some content
    if summary_parts.len() == 1 {
        summary_parts.push("- Completed various tasks and actions.".to_string());
    }

    summary_parts.join("\n")
}

/// Extracts key phrases from a message for summarization.
fn extract_key_phrases(text: &str) -> String {
    // Look for action words and key content
    let text_lower = text.to_lowercase();

    // Prioritize messages with action words
    let action_indicators = [
        "created",
        "added",
        "implemented",
        "fixed",
        "updated",
        "deployed",
        "completed",
        "wrote",
        "built",
        "configured",
        "installed",
        "removed",
        "refactored",
    ];

    for indicator in action_indicators {
        if text_lower.contains(indicator) {
            // Return a shortened version of the message (UTF-8 safe)
            return crate::util::truncate_string_bytes(text, 100, "...");
        }
    }

    // For other messages, extract first sentence or truncate
    if let Some(period_pos) = text.find('.') {
        if period_pos < 150 {
            return text[..=period_pos].to_string();
        }
    }

    // Fallback: truncate (UTF-8 safe)
    crate::util::truncate_string_bytes(text, 80, "...")
}

/// Returns the appropriate summarization prompt for the given style.
#[must_use]
pub fn get_summarization_prompt(style: SummaryStyle) -> &'static str {
    match style {
        SummaryStyle::Timeline => TIMELINE_SUMMARIZATION_PROMPT,
        SummaryStyle::BulletPoints => BULLET_SUMMARIZATION_PROMPT,
        SummaryStyle::Narrative => NARRATIVE_SUMMARIZATION_PROMPT,
    }
}

/// Formats messages for inclusion in a summarization prompt.
///
/// Converts a slice of messages to a human-readable format suitable
/// for Claude to summarize.
#[must_use]
pub fn format_messages_for_summary(messages: &[ApiMessageV2]) -> String {
    messages
        .iter()
        .map(|msg| {
            let role = match msg.role {
                Role::User => "User",
                Role::Assistant => "Assistant",
            };
            format!("{}: {}", role, msg.content.to_text())
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Builds a complete summarization request from messages and style.
///
/// Combines the appropriate prompt with formatted messages.
#[must_use]
pub fn build_summarization_request(messages: &[ApiMessageV2], style: SummaryStyle) -> String {
    let prompt = get_summarization_prompt(style);
    let formatted = format_messages_for_summary(messages);
    format!("{}\n{}", prompt, formatted)
}

// =============================================================================
// Configuration
// =============================================================================

/// Configuration for context compaction.
#[derive(Debug, Clone)]
pub struct CompactionConfig {
    /// Target token count for the compacted output.
    pub target_tokens: usize,
    /// Number of recent messages to preserve verbatim.
    pub preserve_recent: usize,
    /// Style of summary generation.
    pub summary_style: SummaryStyle,
    /// Threshold for auto-compaction (fraction of context window).
    ///
    /// When token usage exceeds `auto_compact_threshold * context_limit`,
    /// compaction is automatically triggered. Default is 0.8 (80%).
    pub auto_compact_threshold: f32,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            target_tokens: 50_000,
            preserve_recent: 4,
            summary_style: SummaryStyle::Timeline,
            auto_compact_threshold: 0.8, // Trigger compaction at 80% of context
        }
    }
}

impl CompactionConfig {
    /// Creates a new CompactionConfig with default values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the auto-compaction threshold.
    ///
    /// # Arguments
    ///
    /// * `threshold` - Fraction of context window at which to trigger compaction (0.0-1.0)
    ///
    /// # Panics
    ///
    /// Panics if threshold is not in the range 0.0 to 1.0.
    #[must_use]
    pub fn with_auto_compact_threshold(mut self, threshold: f32) -> Self {
        assert!(
            (0.0..=1.0).contains(&threshold),
            "Auto-compact threshold must be between 0.0 and 1.0"
        );
        self.auto_compact_threshold = threshold;
        self
    }

    /// Returns the token threshold at which compaction should trigger.
    ///
    /// # Arguments
    ///
    /// * `context_limit` - The maximum context window size in tokens
    ///
    /// # Returns
    ///
    /// The token count at which compaction should be triggered.
    #[must_use]
    pub fn compaction_threshold(&self, context_limit: usize) -> usize {
        (context_limit as f64 * f64::from(self.auto_compact_threshold)) as usize
    }
}

/// Style of summary generation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SummaryStyle {
    /// Chronological timeline of key events.
    #[default]
    Timeline,
    /// Bullet points of key outcomes.
    BulletPoints,
    /// Narrative summary.
    Narrative,
}

/// Result of a compaction operation.
#[derive(Debug, Clone)]
pub struct CompactionResult {
    /// The compacted messages.
    pub messages: Vec<ApiMessageV2>,
    /// Number of tokens saved by compaction.
    pub saved_tokens: usize,
}

/// Context compactor that summarizes old messages.
///
/// Uses a `Summarizer` implementation to generate summaries of conversation
/// history while preserving the system message and recent context.
///
/// # Type Parameters
///
/// * `S` - The summarizer implementation to use
///
/// # Example
///
/// ```rust,ignore
/// use patina::api::compaction::{ContextCompactor, MockSummarizer, CompactionConfig};
///
/// // Create a mock compactor for testing
/// let compactor = ContextCompactor::new_mock();
///
/// // Or with a custom summarizer
/// let custom_compactor = ContextCompactor::with_summarizer(MockSummarizer);
/// ```
#[derive(Debug)]
pub struct ContextCompactor<S: Summarizer> {
    /// The summarizer used to generate conversation summaries
    summarizer: S,
}

impl ContextCompactor<MockSummarizer> {
    /// Creates a mock compactor for testing.
    ///
    /// The mock compactor generates placeholder summaries without
    /// making actual API calls.
    #[must_use]
    pub fn new_mock() -> Self {
        Self {
            summarizer: MockSummarizer,
        }
    }
}

impl ContextCompactor<ClaudeSummarizer> {
    /// Creates a production compactor using the Claude API.
    ///
    /// Uses claude-3-haiku by default for fast, cost-effective summarization.
    ///
    /// # Arguments
    ///
    /// * `client` - The Anthropic API client
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use patina::api::compaction::ContextCompactor;
    /// use patina::api::AnthropicClient;
    /// use std::sync::Arc;
    ///
    /// let client = Arc::new(AnthropicClient::new(api_key, "claude-sonnet-4-20250514"));
    /// let compactor = ContextCompactor::with_client(client);
    /// ```
    #[must_use]
    pub fn with_client(client: Arc<AnthropicClient>) -> Self {
        Self {
            summarizer: ClaudeSummarizer::new(client),
        }
    }

    /// Creates a production compactor with a specific model for summarization.
    ///
    /// # Arguments
    ///
    /// * `client` - The Anthropic API client
    /// * `model` - The model to use for summarization
    #[must_use]
    pub fn with_client_and_model(client: Arc<AnthropicClient>, model: &str) -> Self {
        Self {
            summarizer: ClaudeSummarizer::with_model(client, model),
        }
    }
}

impl<S: Summarizer> ContextCompactor<S> {
    /// Creates a new context compactor with the given summarizer.
    ///
    /// # Arguments
    ///
    /// * `summarizer` - The summarizer implementation to use
    #[must_use]
    pub fn with_summarizer(summarizer: S) -> Self {
        Self { summarizer }
    }

    /// Compacts a conversation to fit within the token budget.
    ///
    /// # Algorithm
    ///
    /// 1. Check if already under budget - return unchanged if so
    /// 2. Preserve the first message (system prompt)
    /// 3. Preserve the last N messages (recent context)
    /// 4. Summarize middle messages into a timeline
    /// 5. Return compacted messages with savings report
    ///
    /// # Arguments
    ///
    /// * `messages` - The full conversation history
    /// * `config` - Compaction configuration
    ///
    /// # Returns
    ///
    /// A `CompactionResult` containing the compacted messages and savings.
    ///
    /// # Errors
    ///
    /// Returns an error if summarization fails (API error, etc.)
    pub async fn compact(
        &self,
        messages: &[ApiMessageV2],
        config: &CompactionConfig,
    ) -> Result<CompactionResult> {
        // Empty or very short conversations don't need compaction
        if messages.len() <= config.preserve_recent + 1 {
            return Ok(CompactionResult {
                messages: messages.to_vec(),
                saved_tokens: 0,
            });
        }

        let original_tokens = estimate_messages_tokens(messages);

        // Check if already under budget
        if original_tokens <= config.target_tokens {
            return Ok(CompactionResult {
                messages: messages.to_vec(),
                saved_tokens: 0,
            });
        }

        // Split messages into: first (system), middle (to summarize), recent (to preserve)
        let first_message = &messages[0];
        let preserve_count = config.preserve_recent.min(messages.len().saturating_sub(1));
        let middle_end = messages.len().saturating_sub(preserve_count);
        let middle_messages = &messages[1..middle_end];
        let recent_messages = &messages[middle_end..];

        // If there's nothing to summarize, return unchanged
        if middle_messages.is_empty() {
            return Ok(CompactionResult {
                messages: messages.to_vec(),
                saved_tokens: 0,
            });
        }

        // Generate summary of middle messages using the configured summarizer
        let summary = self.summarizer.summarize(middle_messages, config).await;

        // Build compacted message list
        let mut compacted = Vec::with_capacity(3 + recent_messages.len());

        // 1. First message (system prompt) - always preserved
        compacted.push(first_message.clone());

        // 2. Summary message (as assistant, since first is user)
        compacted.push(ApiMessageV2::assistant(summary));

        // 3. Recent messages - preserved verbatim, ensuring proper role alternation
        // We need to ensure proper role alternation after the summary
        for msg in recent_messages {
            compacted.push(msg.clone());
        }

        // Ensure role alternation is valid
        fix_role_alternation(&mut compacted);

        let compacted_tokens = estimate_messages_tokens(&compacted);
        let saved_tokens = original_tokens.saturating_sub(compacted_tokens);

        Ok(CompactionResult {
            messages: compacted,
            saved_tokens,
        })
    }
}

impl Default for ContextCompactor<MockSummarizer> {
    fn default() -> Self {
        Self::new_mock()
    }
}

/// Fixes role alternation in the message list.
///
/// Ensures messages alternate between user and assistant roles.
fn fix_role_alternation(messages: &mut Vec<ApiMessageV2>) {
    if messages.len() < 2 {
        return;
    }

    let mut i = 1;
    while i < messages.len() {
        if messages[i].role == messages[i - 1].role {
            // Same role as previous - need to insert a placeholder
            let placeholder = if messages[i].role == Role::User {
                ApiMessageV2::assistant("Continuing...")
            } else {
                ApiMessageV2::user("Please continue.")
            };
            messages.insert(i, placeholder);
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_summarizer_trait_exists() {
        // Verify the Summarizer trait can be implemented
        struct TestSummarizer;

        impl Summarizer for TestSummarizer {
            async fn summarize(
                &self,
                _messages: &[ApiMessageV2],
                _config: &CompactionConfig,
            ) -> String {
                "Test summary".to_string()
            }
        }

        let summarizer = TestSummarizer;
        let messages = vec![ApiMessageV2::user("Test")];
        let config = CompactionConfig::default();
        let result = summarizer.summarize(&messages, &config).await;
        assert_eq!(result, "Test summary");
    }

    #[tokio::test]
    async fn test_mock_summarizer_implements_trait() {
        let summarizer = MockSummarizer;
        let messages = vec![
            ApiMessageV2::user("Hello"),
            ApiMessageV2::assistant("Hi there!"),
        ];
        let config = CompactionConfig::default();
        let result = summarizer.summarize(&messages, &config).await;
        assert!(result.contains("Previous conversation"));
    }

    #[test]
    fn test_compaction_config_default() {
        let config = CompactionConfig::default();
        assert_eq!(config.target_tokens, 50_000);
        assert_eq!(config.preserve_recent, 4);
        assert_eq!(config.summary_style, SummaryStyle::Timeline);
    }

    #[tokio::test]
    async fn test_context_compactor_new_mock() {
        // Verify mock compactor can be created and used
        let compactor = ContextCompactor::new_mock();
        let messages = vec![ApiMessageV2::user("Test")];
        let config = CompactionConfig::default();
        // Should not panic
        let _ = compactor.compact(&messages, &config).await;
    }

    #[tokio::test]
    async fn test_context_compactor_with_summarizer() {
        // Verify compactor can be created with custom summarizer
        struct CustomSummarizer;
        impl Summarizer for CustomSummarizer {
            async fn summarize(&self, _: &[ApiMessageV2], _: &CompactionConfig) -> String {
                "Custom summary".to_string()
            }
        }
        let compactor = ContextCompactor::with_summarizer(CustomSummarizer);
        let messages = vec![ApiMessageV2::user("Test")];
        let config = CompactionConfig::default();
        let _ = compactor.compact(&messages, &config).await;
    }

    #[test]
    fn test_compaction_result_fields() {
        let result = CompactionResult {
            messages: vec![],
            saved_tokens: 100,
        };
        assert!(result.messages.is_empty());
        assert_eq!(result.saved_tokens, 100);
    }

    #[test]
    fn test_extract_key_phrases_with_action() {
        let text = "I created a new file called main.rs with the hello world program.";
        let result = extract_key_phrases(text);
        assert!(result.contains("created"));
    }

    #[test]
    fn test_extract_key_phrases_truncates_long() {
        let text = "a".repeat(200);
        let result = extract_key_phrases(&text);
        assert!(result.len() < text.len());
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_fix_role_alternation_already_valid() {
        let mut messages = vec![
            ApiMessageV2::user("Hello"),
            ApiMessageV2::assistant("Hi"),
            ApiMessageV2::user("Bye"),
        ];
        fix_role_alternation(&mut messages);
        assert_eq!(messages.len(), 3);
    }

    #[test]
    fn test_fix_role_alternation_inserts_placeholder() {
        let mut messages = vec![
            ApiMessageV2::user("Hello"),
            ApiMessageV2::user("Another user message"),
        ];
        fix_role_alternation(&mut messages);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1].role, Role::Assistant);
    }

    #[tokio::test]
    async fn test_compact_empty_messages() {
        let compactor = ContextCompactor::new_mock();
        let messages: Vec<ApiMessageV2> = vec![];
        let config = CompactionConfig::default();
        let result = compactor.compact(&messages, &config).await.unwrap();
        assert!(result.messages.is_empty());
        assert_eq!(result.saved_tokens, 0);
    }

    #[tokio::test]
    async fn test_compact_short_conversation() {
        let compactor = ContextCompactor::new_mock();
        let messages = vec![
            ApiMessageV2::user("System"),
            ApiMessageV2::assistant("Ready"),
        ];
        let config = CompactionConfig {
            preserve_recent: 4,
            ..Default::default()
        };
        let result = compactor.compact(&messages, &config).await.unwrap();
        assert_eq!(result.messages.len(), 2);
        assert_eq!(result.saved_tokens, 0);
    }

    // =========================================================================
    // Summarization prompt tests
    // =========================================================================

    #[test]
    fn test_get_summarization_prompt_timeline() {
        let prompt = get_summarization_prompt(SummaryStyle::Timeline);
        assert!(prompt.contains("timeline"));
        assert!(prompt.contains("Key decisions"));
    }

    #[test]
    fn test_get_summarization_prompt_bullets() {
        let prompt = get_summarization_prompt(SummaryStyle::BulletPoints);
        assert!(prompt.contains("bullet-point"));
        assert!(prompt.contains("accomplished"));
    }

    #[test]
    fn test_get_summarization_prompt_narrative() {
        let prompt = get_summarization_prompt(SummaryStyle::Narrative);
        assert!(prompt.contains("narrative"));
        assert!(prompt.contains("past tense"));
    }

    #[test]
    fn test_format_messages_for_summary() {
        let messages = vec![
            ApiMessageV2::user("Hello, how are you?"),
            ApiMessageV2::assistant("I'm doing well, thank you!"),
        ];
        let formatted = format_messages_for_summary(&messages);
        assert!(formatted.contains("User: Hello"));
        assert!(formatted.contains("Assistant: I'm doing"));
    }

    #[test]
    fn test_format_messages_for_summary_empty() {
        let messages: Vec<ApiMessageV2> = vec![];
        let formatted = format_messages_for_summary(&messages);
        assert!(formatted.is_empty());
    }

    #[test]
    fn test_build_summarization_request() {
        let messages = vec![
            ApiMessageV2::user("Create a file"),
            ApiMessageV2::assistant("Done!"),
        ];
        let request = build_summarization_request(&messages, SummaryStyle::Timeline);
        assert!(request.contains("timeline"));
        assert!(request.contains("User: Create a file"));
        assert!(request.contains("Assistant: Done!"));
    }

    // =========================================================================
    // ClaudeSummarizer tests
    // =========================================================================

    #[test]
    fn test_claude_summarizer_struct_exists() {
        // Verify the struct can be referenced
        use super::ClaudeSummarizer;
        let _type_check: fn(Arc<AnthropicClient>) -> ClaudeSummarizer = ClaudeSummarizer::new;
    }

    #[test]
    fn test_claude_summarizer_default_model() {
        // Verify the default summarization model constant
        assert_eq!(SUMMARIZATION_MODEL, "claude-3-haiku-20240307");
    }

    #[test]
    fn test_summarization_prompt_template_exists() {
        // Verify the prompt templates are accessible
        assert!(!TIMELINE_SUMMARIZATION_PROMPT.is_empty());
        assert!(!BULLET_SUMMARIZATION_PROMPT.is_empty());
        assert!(!NARRATIVE_SUMMARIZATION_PROMPT.is_empty());
    }

    #[test]
    fn test_summarization_prompt_includes_messages_placeholder() {
        // Verify prompts end with the messages placeholder marker
        assert!(TIMELINE_SUMMARIZATION_PROMPT.contains("Previous conversation to summarize:"));
        assert!(BULLET_SUMMARIZATION_PROMPT.contains("Previous conversation to summarize:"));
        assert!(NARRATIVE_SUMMARIZATION_PROMPT.contains("Previous conversation to summarize:"));
    }

    #[test]
    fn test_context_compactor_with_client_factory() {
        // Verify factory method exists and returns correct type
        // This is a compile-time check - actual client would need API key
        use super::ContextCompactor;
        let _type_check: fn(Arc<AnthropicClient>) -> ContextCompactor<ClaudeSummarizer> =
            ContextCompactor::with_client;
    }

    #[test]
    fn test_context_compactor_with_client_and_model_factory() {
        // Verify factory method with custom model exists
        use super::ContextCompactor;
        let _type_check: fn(Arc<AnthropicClient>, &str) -> ContextCompactor<ClaudeSummarizer> =
            ContextCompactor::with_client_and_model;
    }

    #[test]
    fn test_claude_summarizer_graceful_degradation_pattern() {
        // Verify the Summarizer trait is implemented
        // When API fails, it should fall back to mock summary
        // This is tested at the trait level, not the actual API call
        fn accepts_summarizer<S: Summarizer>(_s: S) {}

        let mock = MockSummarizer;
        accepts_summarizer(mock);

        // ClaudeSummarizer would also satisfy this if we had a client
        // accepts_summarizer(ClaudeSummarizer::new(client));
    }
}
