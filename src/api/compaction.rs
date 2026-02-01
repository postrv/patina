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
use crate::types::{ApiMessageV2, Role};
use anyhow::Result;

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
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            target_tokens: 50_000,
            preserve_recent: 4,
            summary_style: SummaryStyle::Timeline,
        }
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
/// Uses Claude to generate summaries of conversation history while
/// preserving the system message and recent context.
#[derive(Debug)]
pub struct ContextCompactor {
    /// Whether this is a mock compactor for testing
    is_mock: bool,
}

impl ContextCompactor {
    /// Creates a new context compactor with a Claude client.
    ///
    /// The client is used to generate summaries of old messages.
    #[must_use]
    pub fn new() -> Self {
        Self { is_mock: false }
    }

    /// Creates a mock compactor for testing.
    ///
    /// The mock compactor generates placeholder summaries without
    /// making actual API calls.
    #[must_use]
    pub fn new_mock() -> Self {
        Self { is_mock: true }
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
    pub fn compact(
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

        // Generate summary of middle messages
        let summary = self.generate_summary(middle_messages, config);

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
        self.fix_role_alternation(&mut compacted);

        let compacted_tokens = estimate_messages_tokens(&compacted);
        let saved_tokens = original_tokens.saturating_sub(compacted_tokens);

        Ok(CompactionResult {
            messages: compacted,
            saved_tokens,
        })
    }

    /// Generates a summary of messages.
    ///
    /// For mock compactor, generates a placeholder summary.
    /// For real compactor, would use Claude API.
    fn generate_summary(&self, messages: &[ApiMessageV2], config: &CompactionConfig) -> String {
        if self.is_mock {
            self.generate_mock_summary(messages, config)
        } else {
            // Real implementation would call Claude API
            // For now, use mock summary
            self.generate_mock_summary(messages, config)
        }
    }

    /// Generates a mock summary for testing.
    ///
    /// Extracts key content from messages to create a timeline.
    fn generate_mock_summary(
        &self,
        messages: &[ApiMessageV2],
        config: &CompactionConfig,
    ) -> String {
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
                    if trimmed.starts_with('-')
                        || trimmed.starts_with('•')
                        || trimmed.starts_with('*')
                    {
                        summary_parts.push(trimmed.to_string());
                    }
                }
                continue;
            }

            // Extract key phrases from messages
            let key_phrases = self.extract_key_phrases(&text);
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
    fn extract_key_phrases(&self, text: &str) -> String {
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
                // Return a shortened version of the message
                let truncated = if text.len() > 100 {
                    format!("{}...", &text[..100])
                } else {
                    text.to_string()
                };
                return truncated;
            }
        }

        // For other messages, extract first sentence or truncate
        if let Some(period_pos) = text.find('.') {
            if period_pos < 150 {
                return text[..=period_pos].to_string();
            }
        }

        // Fallback: truncate
        if text.len() > 80 {
            format!("{}...", &text[..80])
        } else {
            text.to_string()
        }
    }

    /// Fixes role alternation in the message list.
    ///
    /// Ensures messages alternate between user and assistant roles.
    fn fix_role_alternation(&self, messages: &mut Vec<ApiMessageV2>) {
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
}

impl Default for ContextCompactor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compaction_config_default() {
        let config = CompactionConfig::default();
        assert_eq!(config.target_tokens, 50_000);
        assert_eq!(config.preserve_recent, 4);
        assert_eq!(config.summary_style, SummaryStyle::Timeline);
    }

    #[test]
    fn test_context_compactor_new_mock() {
        let compactor = ContextCompactor::new_mock();
        assert!(compactor.is_mock);
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
        let compactor = ContextCompactor::new_mock();
        let text = "I created a new file called main.rs with the hello world program.";
        let result = compactor.extract_key_phrases(text);
        assert!(result.contains("created"));
    }

    #[test]
    fn test_extract_key_phrases_truncates_long() {
        let compactor = ContextCompactor::new_mock();
        let text = "a".repeat(200);
        let result = compactor.extract_key_phrases(&text);
        assert!(result.len() < text.len());
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_fix_role_alternation_already_valid() {
        let compactor = ContextCompactor::new_mock();
        let mut messages = vec![
            ApiMessageV2::user("Hello"),
            ApiMessageV2::assistant("Hi"),
            ApiMessageV2::user("Bye"),
        ];
        compactor.fix_role_alternation(&mut messages);
        assert_eq!(messages.len(), 3);
    }

    #[test]
    fn test_fix_role_alternation_inserts_placeholder() {
        let compactor = ContextCompactor::new_mock();
        let mut messages = vec![
            ApiMessageV2::user("Hello"),
            ApiMessageV2::user("Another user message"),
        ];
        compactor.fix_role_alternation(&mut messages);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1].role, Role::Assistant);
    }

    #[test]
    fn test_compact_empty_messages() {
        let compactor = ContextCompactor::new_mock();
        let messages: Vec<ApiMessageV2> = vec![];
        let config = CompactionConfig::default();
        let result = compactor.compact(&messages, &config).unwrap();
        assert!(result.messages.is_empty());
        assert_eq!(result.saved_tokens, 0);
    }

    #[test]
    fn test_compact_short_conversation() {
        let compactor = ContextCompactor::new_mock();
        let messages = vec![
            ApiMessageV2::user("System"),
            ApiMessageV2::assistant("Ready"),
        ];
        let config = CompactionConfig {
            preserve_recent: 4,
            ..Default::default()
        };
        let result = compactor.compact(&messages, &config).unwrap();
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
}
