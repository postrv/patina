//! Context window analysis for token usage breakdown.
//!
//! Provides per-message token estimation and context utilization analysis
//! to help users understand and optimize their context window usage.

use crate::api::tokens::estimate_tokens;
use crate::types::Message;

/// Token usage information for a single message.
#[derive(Debug, Clone)]
pub struct MessageTokenInfo {
    /// Index of the message in the conversation.
    pub index: usize,
    /// Role (user/assistant).
    pub role: String,
    /// Estimated token count.
    pub tokens: usize,
    /// First 80 characters of the message for identification.
    pub preview: String,
}

/// Suggestion for reducing context usage.
#[derive(Debug, Clone)]
pub struct ContextSuggestion {
    /// Human-readable suggestion text.
    pub text: String,
    /// Estimated tokens that could be saved.
    pub potential_savings: usize,
}

/// Analysis of current context window utilization.
#[derive(Debug, Clone)]
pub struct ContextAnalysis {
    /// Total tokens estimated for all messages.
    pub total_tokens_used: usize,
    /// Maximum context window size for the model.
    pub context_limit: usize,
    /// Per-message token breakdown.
    pub message_breakdown: Vec<MessageTokenInfo>,
    /// Suggestions for reducing context usage.
    pub suggestions: Vec<ContextSuggestion>,
}

impl ContextAnalysis {
    /// Analyzes the context usage of a conversation.
    ///
    /// Estimates token counts per message and identifies heavy messages
    /// that may benefit from compression.
    ///
    /// # Arguments
    ///
    /// * `messages` - The conversation messages to analyze.
    /// * `context_limit` - The model's context window size in tokens.
    #[must_use]
    pub fn analyze(messages: &[Message], context_limit: usize) -> Self {
        let mut message_breakdown = Vec::with_capacity(messages.len());
        let mut total_tokens_used = 0;

        for (index, msg) in messages.iter().enumerate() {
            let tokens = estimate_tokens(&msg.content) + 4; // +4 for role/separator overhead
            total_tokens_used += tokens;

            let preview = if msg.content.len() > 80 {
                format!("{}...", &msg.content[..77])
            } else {
                msg.content.clone()
            };

            message_breakdown.push(MessageTokenInfo {
                index,
                role: format!("{:?}", msg.role),
                tokens,
                preview,
            });
        }

        let suggestions =
            Self::generate_suggestions(&message_breakdown, total_tokens_used, context_limit);

        Self {
            total_tokens_used,
            context_limit,
            message_breakdown,
            suggestions,
        }
    }

    /// Generates suggestions based on context analysis.
    fn generate_suggestions(
        breakdown: &[MessageTokenInfo],
        total: usize,
        limit: usize,
    ) -> Vec<ContextSuggestion> {
        let mut suggestions = Vec::new();

        // Flag messages using > 20% of context
        let threshold = limit / 5;
        for info in breakdown {
            if info.tokens > threshold {
                suggestions.push(ContextSuggestion {
                    text: format!(
                        "Message {} ({}) uses {} tokens ({:.0}% of context). Consider summarizing.",
                        info.index,
                        info.role,
                        info.tokens,
                        (info.tokens as f64 / limit as f64) * 100.0,
                    ),
                    potential_savings: info.tokens / 2,
                });
            }
        }

        // Overall usage warning
        if total > limit * 3 / 4 {
            suggestions.push(ContextSuggestion {
                text: format!(
                    "Context is {:.0}% full ({}/{} tokens). Consider starting a new session.",
                    (total as f64 / limit as f64) * 100.0,
                    total,
                    limit,
                ),
                potential_savings: 0,
            });
        }

        suggestions
    }

    /// Formats the analysis as a human-readable string.
    #[must_use]
    pub fn format(&self) -> String {
        let mut output = String::new();

        output.push_str(&format!(
            "Context Usage: {}/{} tokens ({:.0}%)\n\n",
            self.total_tokens_used,
            self.context_limit,
            if self.context_limit > 0 {
                (self.total_tokens_used as f64 / self.context_limit as f64) * 100.0
            } else {
                0.0
            },
        ));

        if !self.message_breakdown.is_empty() {
            output.push_str("Message Breakdown:\n");
            for info in &self.message_breakdown {
                output.push_str(&format!(
                    "  [{:>3}] {:>9} {:>6} tokens  {}\n",
                    info.index, info.role, info.tokens, info.preview,
                ));
            }
        }

        if !self.suggestions.is_empty() {
            output.push_str("\nSuggestions:\n");
            for suggestion in &self.suggestions {
                output.push_str(&format!("  - {}\n", suggestion.text));
            }
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Role;

    fn test_msg(role: Role, content: &str) -> Message {
        Message {
            role,
            content: content.to_string(),
        }
    }

    #[test]
    fn test_context_analysis_empty() {
        let analysis = ContextAnalysis::analyze(&[], 200_000);
        assert_eq!(analysis.total_tokens_used, 0);
        assert_eq!(analysis.context_limit, 200_000);
        assert!(analysis.message_breakdown.is_empty());
        assert!(analysis.suggestions.is_empty());
    }

    #[test]
    fn test_context_analysis_basic() {
        let messages = vec![
            test_msg(Role::User, "Hello, how are you?"),
            test_msg(Role::Assistant, "I'm doing well, thanks!"),
        ];
        let analysis = ContextAnalysis::analyze(&messages, 200_000);
        assert!(analysis.total_tokens_used > 0);
        assert_eq!(analysis.message_breakdown.len(), 2);
        assert_eq!(analysis.message_breakdown[0].index, 0);
        assert_eq!(analysis.message_breakdown[1].index, 1);
    }

    #[test]
    fn test_context_analysis_heavy_message_suggestion() {
        // Create a message that's > 20% of a small context
        let long_content = "x ".repeat(5000); // ~2500 tokens
        let messages = vec![test_msg(Role::User, &long_content)];
        let analysis = ContextAnalysis::analyze(&messages, 1000);
        // The message uses way more than 20% of 1000 tokens
        assert!(
            !analysis.suggestions.is_empty(),
            "Should suggest compression for heavy messages"
        );
    }

    #[test]
    fn test_context_analysis_high_usage_warning() {
        // Create enough messages to exceed 75% of a tiny context
        let long_content = "word ".repeat(2000);
        let messages = vec![test_msg(Role::User, &long_content)];
        let analysis = ContextAnalysis::analyze(&messages, 100);
        let has_full_warning = analysis.suggestions.iter().any(|s| s.text.contains("full"));
        assert!(has_full_warning, "Should warn about high context usage");
    }

    #[test]
    fn test_context_analysis_format() {
        let messages = vec![
            test_msg(Role::User, "Hello"),
            test_msg(Role::Assistant, "Hi"),
        ];
        let analysis = ContextAnalysis::analyze(&messages, 200_000);
        let formatted = analysis.format();
        assert!(formatted.contains("Context Usage:"));
        assert!(formatted.contains("200000"));
        assert!(formatted.contains("Message Breakdown:"));
    }

    #[test]
    fn test_context_analysis_preview_truncation() {
        let long_msg = "a".repeat(200);
        let messages = vec![test_msg(Role::User, &long_msg)];
        let analysis = ContextAnalysis::analyze(&messages, 200_000);
        assert!(analysis.message_breakdown[0].preview.ends_with("..."));
        assert!(analysis.message_breakdown[0].preview.len() <= 83);
    }
}
