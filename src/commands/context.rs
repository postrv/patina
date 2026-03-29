//! Context analysis slash command.
//!
//! This module provides the `/context` command that displays token usage
//! breakdown and optimization suggestions. It helps users understand how
//! their context window is being consumed and identifies potential issues
//! like near-capacity usage, memory bloat, or excessive tool definitions.

use std::fmt;

/// Input data for context analysis.
///
/// This struct decouples the analysis logic from application state,
/// making it independently testable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextAnalysisInput {
    /// Token count consumed by the system prompt.
    pub system_prompt_tokens: usize,
    /// Token count consumed by CLAUDE.md files.
    pub claude_md_tokens: usize,
    /// Token count consumed by tool definitions.
    pub tool_definitions_tokens: usize,
    /// Token count consumed by the conversation history.
    pub conversation_tokens: usize,
    /// Token count consumed by memory files.
    pub memory_tokens: usize,
    /// Maximum token capacity for the model.
    pub max_tokens: usize,
}

/// Token usage breakdown for the current context.
///
/// Contains per-category token counts and the model's maximum capacity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextBreakdown {
    /// Tokens consumed by the system prompt.
    pub system_prompt_tokens: usize,
    /// Tokens consumed by CLAUDE.md files.
    pub claude_md_tokens: usize,
    /// Tokens consumed by tool definitions.
    pub tool_definitions_tokens: usize,
    /// Tokens consumed by the conversation history.
    pub conversation_tokens: usize,
    /// Tokens consumed by memory files.
    pub memory_tokens: usize,
    /// Sum of all token categories.
    pub total_tokens: usize,
    /// Maximum token capacity for the model.
    pub max_tokens: usize,
}

/// Warnings about context usage patterns that may need attention.
///
/// Each variant represents a specific issue with actionable advice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextWarning {
    /// Total usage exceeds 80% of model capacity.
    NearCapacity {
        /// Current usage as a percentage of capacity.
        usage_percent: usize,
    },
    /// Memory tokens exceed 10% of total usage.
    MemoryBloat {
        /// Memory's share of total usage as a percentage.
        memory_percent: usize,
    },
    /// Tool definition tokens exceed 30% of total usage.
    TooManyTools {
        /// Tool definitions' share of total usage as a percentage.
        tools_percent: usize,
    },
    /// System prompt exceeds 5,000 tokens.
    LargeSystemPrompt {
        /// Number of tokens in the system prompt.
        token_count: usize,
    },
}

impl fmt::Display for ContextWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NearCapacity { usage_percent } => {
                write!(
                    f,
                    "Near capacity ({usage_percent}% used). Consider summarizing \
                     conversation history or starting a new session."
                )
            }
            Self::MemoryBloat { memory_percent } => {
                write!(
                    f,
                    "Memory bloat ({memory_percent}% of tokens). Consider pruning \
                     stale memory entries to free context space."
                )
            }
            Self::TooManyTools { tools_percent } => {
                write!(
                    f,
                    "Too many tools ({tools_percent}% of tokens). Consider disabling \
                     unused MCP servers or tool definitions."
                )
            }
            Self::LargeSystemPrompt { token_count } => {
                write!(
                    f,
                    "Large system prompt ({token_count} tokens). Consider trimming \
                     CLAUDE.md or project instructions."
                )
            }
        }
    }
}

/// Computes a token usage breakdown from the given analysis input.
///
/// Sums all individual token categories to produce the total. If `max_tokens`
/// is zero, the total may exceed it (callers should handle this edge case
/// via [`detect_warnings`]).
///
/// # Examples
///
/// ```
/// use patina::commands::context::{analyze_context, ContextAnalysisInput};
///
/// let input = ContextAnalysisInput {
///     system_prompt_tokens: 2_100,
///     claude_md_tokens: 1_500,
///     tool_definitions_tokens: 8_200,
///     conversation_tokens: 32_400,
///     memory_tokens: 1_000,
///     max_tokens: 200_000,
/// };
/// let breakdown = analyze_context(&input);
/// assert_eq!(breakdown.total_tokens, 45_200);
/// ```
#[must_use]
pub fn analyze_context(input: &ContextAnalysisInput) -> ContextBreakdown {
    let total = input.system_prompt_tokens
        + input.claude_md_tokens
        + input.tool_definitions_tokens
        + input.conversation_tokens
        + input.memory_tokens;

    ContextBreakdown {
        system_prompt_tokens: input.system_prompt_tokens,
        claude_md_tokens: input.claude_md_tokens,
        tool_definitions_tokens: input.tool_definitions_tokens,
        conversation_tokens: input.conversation_tokens,
        memory_tokens: input.memory_tokens,
        total_tokens: total,
        max_tokens: input.max_tokens,
    }
}

/// Detects context usage warnings from a breakdown.
///
/// Checks for:
/// - **`NearCapacity`**: total usage > 80% of `max_tokens`
/// - **`MemoryBloat`**: memory > 10% of total usage
/// - **`TooManyTools`**: tool definitions > 30% of total usage
/// - **`LargeSystemPrompt`**: system prompt > 5,000 tokens
///
/// Returns an empty vector when no issues are found or when `total_tokens`
/// or `max_tokens` is zero.
///
/// # Examples
///
/// ```
/// use patina::commands::context::{analyze_context, detect_warnings, ContextAnalysisInput, ContextWarning};
///
/// let input = ContextAnalysisInput {
///     system_prompt_tokens: 100,
///     claude_md_tokens: 100,
///     tool_definitions_tokens: 100,
///     conversation_tokens: 162_100,
///     memory_tokens: 100,
///     max_tokens: 200_000,
/// };
/// let breakdown = analyze_context(&input);
/// let warnings = detect_warnings(&breakdown);
/// assert!(warnings.iter().any(|w| matches!(w, ContextWarning::NearCapacity { .. })));
/// ```
#[must_use]
pub fn detect_warnings(breakdown: &ContextBreakdown) -> Vec<ContextWarning> {
    let mut warnings = Vec::new();

    if breakdown.max_tokens == 0 || breakdown.total_tokens == 0 {
        return warnings;
    }

    // Near capacity: total > 80% of max
    let usage_percent = (breakdown.total_tokens * 100) / breakdown.max_tokens;
    if usage_percent > 80 {
        warnings.push(ContextWarning::NearCapacity { usage_percent });
    }

    // Memory bloat: memory > 10% of total
    let memory_percent = (breakdown.memory_tokens * 100) / breakdown.total_tokens;
    if memory_percent > 10 {
        warnings.push(ContextWarning::MemoryBloat { memory_percent });
    }

    // Too many tools: tools > 30% of total
    let tools_percent = (breakdown.tool_definitions_tokens * 100) / breakdown.total_tokens;
    if tools_percent > 30 {
        warnings.push(ContextWarning::TooManyTools { tools_percent });
    }

    // Large system prompt: > 5,000 tokens
    const LARGE_SYSTEM_PROMPT_THRESHOLD: usize = 5_000;
    if breakdown.system_prompt_tokens > LARGE_SYSTEM_PROMPT_THRESHOLD {
        warnings.push(ContextWarning::LargeSystemPrompt {
            token_count: breakdown.system_prompt_tokens,
        });
    }

    warnings
}

/// Formats a human-readable context usage report.
///
/// Produces a tree-style breakdown showing each category's token count and
/// its percentage of total usage, followed by any warnings with actionable
/// suggestions.
///
/// # Examples
///
/// ```
/// use patina::commands::context::{analyze_context, detect_warnings, format_report, ContextAnalysisInput};
///
/// let input = ContextAnalysisInput {
///     system_prompt_tokens: 2_100,
///     claude_md_tokens: 1_500,
///     tool_definitions_tokens: 8_200,
///     conversation_tokens: 32_400,
///     memory_tokens: 1_000,
///     max_tokens: 200_000,
/// };
/// let breakdown = analyze_context(&input);
/// let warnings = detect_warnings(&breakdown);
/// let report = format_report(&breakdown, &warnings);
/// assert!(report.contains("Context Usage:"));
/// assert!(report.contains("System prompt:"));
/// ```
#[must_use]
pub fn format_report(breakdown: &ContextBreakdown, warnings: &[ContextWarning]) -> String {
    let mut report = String::new();

    // Header line: Context Usage: X / Y tokens (Z%)
    let capacity_percent = if breakdown.max_tokens > 0 {
        (breakdown.total_tokens as f64 / breakdown.max_tokens as f64) * 100.0
    } else {
        0.0
    };

    report.push_str(&format!(
        "Context Usage: {} / {} tokens ({:.1}%)\n",
        format_number(breakdown.total_tokens),
        format_number(breakdown.max_tokens),
        capacity_percent,
    ));

    // Category breakdown
    let categories: [(&str, usize); 5] = [
        ("System prompt", breakdown.system_prompt_tokens),
        ("CLAUDE.md", breakdown.claude_md_tokens),
        ("Tool definitions", breakdown.tool_definitions_tokens),
        ("Conversation", breakdown.conversation_tokens),
        ("Memory", breakdown.memory_tokens),
    ];

    for (i, (label, tokens)) in categories.iter().enumerate() {
        let is_last = i == categories.len() - 1;
        let connector = if is_last { "\u{2514}" } else { "\u{251c}" };
        let dash = "\u{2500}\u{2500}";

        let pct = if breakdown.total_tokens > 0 {
            (*tokens as f64 / breakdown.total_tokens as f64) * 100.0
        } else {
            0.0
        };

        report.push_str(&format!(
            "{connector}{dash} {label:<17} {tokens:>7} ({pct:.1}%)\n",
            connector = connector,
            dash = dash,
            label = format!("{}:", label),
            tokens = format_number(*tokens),
            pct = pct,
        ));
    }

    // Warnings
    if !warnings.is_empty() {
        report.push('\n');
        report.push_str("Warnings:\n");
        for warning in warnings {
            report.push_str(&format!("  ! {warning}\n"));
        }
    }

    report
}

/// Formats a number with comma separators for readability.
///
/// # Examples
///
/// ```
/// use patina::commands::context::format_number;
///
/// assert_eq!(format_number(0), "0");
/// assert_eq!(format_number(1_000), "1,000");
/// assert_eq!(format_number(200_000), "200,000");
/// assert_eq!(format_number(1_234_567), "1,234,567");
/// ```
#[must_use]
pub fn format_number(n: usize) -> String {
    let s = n.to_string();
    let len = s.len();
    if len <= 3 {
        return s;
    }

    let mut result = String::with_capacity(len + (len - 1) / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            result.push(',');
        }
        result.push(ch);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // analyze_context: breakdown calculation
    // =========================================================================

    #[test]
    fn test_analyze_context_sums_all_categories() {
        let input = ContextAnalysisInput {
            system_prompt_tokens: 2_100,
            claude_md_tokens: 1_500,
            tool_definitions_tokens: 8_200,
            conversation_tokens: 32_400,
            memory_tokens: 1_000,
            max_tokens: 200_000,
        };

        let breakdown = analyze_context(&input);

        assert_eq!(breakdown.system_prompt_tokens, 2_100);
        assert_eq!(breakdown.claude_md_tokens, 1_500);
        assert_eq!(breakdown.tool_definitions_tokens, 8_200);
        assert_eq!(breakdown.conversation_tokens, 32_400);
        assert_eq!(breakdown.memory_tokens, 1_000);
        assert_eq!(breakdown.total_tokens, 45_200);
        assert_eq!(breakdown.max_tokens, 200_000);
    }

    #[test]
    fn test_analyze_context_zero_tokens() {
        let input = ContextAnalysisInput {
            system_prompt_tokens: 0,
            claude_md_tokens: 0,
            tool_definitions_tokens: 0,
            conversation_tokens: 0,
            memory_tokens: 0,
            max_tokens: 200_000,
        };

        let breakdown = analyze_context(&input);

        assert_eq!(breakdown.total_tokens, 0);
        assert_eq!(breakdown.max_tokens, 200_000);
    }

    #[test]
    fn test_analyze_context_zero_max_tokens() {
        let input = ContextAnalysisInput {
            system_prompt_tokens: 100,
            claude_md_tokens: 0,
            tool_definitions_tokens: 0,
            conversation_tokens: 0,
            memory_tokens: 0,
            max_tokens: 0,
        };

        let breakdown = analyze_context(&input);

        assert_eq!(breakdown.total_tokens, 100);
        assert_eq!(breakdown.max_tokens, 0);
    }

    #[test]
    fn test_analyze_context_at_capacity() {
        let input = ContextAnalysisInput {
            system_prompt_tokens: 50_000,
            claude_md_tokens: 50_000,
            tool_definitions_tokens: 50_000,
            conversation_tokens: 50_000,
            memory_tokens: 0,
            max_tokens: 200_000,
        };

        let breakdown = analyze_context(&input);

        assert_eq!(breakdown.total_tokens, 200_000);
        assert_eq!(breakdown.total_tokens, breakdown.max_tokens);
    }

    #[test]
    fn test_analyze_context_over_capacity() {
        let input = ContextAnalysisInput {
            system_prompt_tokens: 100_000,
            claude_md_tokens: 50_000,
            tool_definitions_tokens: 50_000,
            conversation_tokens: 50_000,
            memory_tokens: 0,
            max_tokens: 200_000,
        };

        let breakdown = analyze_context(&input);

        assert_eq!(breakdown.total_tokens, 250_000);
        assert!(breakdown.total_tokens > breakdown.max_tokens);
    }

    // =========================================================================
    // detect_warnings: NearCapacity
    // =========================================================================

    #[test]
    fn test_near_capacity_warning_at_81_percent() {
        let breakdown = ContextBreakdown {
            system_prompt_tokens: 1_000,
            claude_md_tokens: 0,
            tool_definitions_tokens: 0,
            conversation_tokens: 161_000,
            memory_tokens: 0,
            total_tokens: 162_000,
            max_tokens: 200_000,
        };

        let warnings = detect_warnings(&breakdown);

        assert!(warnings
            .iter()
            .any(|w| matches!(w, ContextWarning::NearCapacity { usage_percent: 81 })));
    }

    #[test]
    fn test_no_near_capacity_warning_at_80_percent() {
        let breakdown = ContextBreakdown {
            system_prompt_tokens: 1_000,
            claude_md_tokens: 0,
            tool_definitions_tokens: 0,
            conversation_tokens: 159_000,
            memory_tokens: 0,
            total_tokens: 160_000,
            max_tokens: 200_000,
        };

        let warnings = detect_warnings(&breakdown);

        assert!(!warnings
            .iter()
            .any(|w| matches!(w, ContextWarning::NearCapacity { .. })));
    }

    #[test]
    fn test_near_capacity_warning_at_100_percent() {
        let breakdown = ContextBreakdown {
            system_prompt_tokens: 0,
            claude_md_tokens: 0,
            tool_definitions_tokens: 0,
            conversation_tokens: 200_000,
            memory_tokens: 0,
            total_tokens: 200_000,
            max_tokens: 200_000,
        };

        let warnings = detect_warnings(&breakdown);

        assert!(warnings
            .iter()
            .any(|w| matches!(w, ContextWarning::NearCapacity { usage_percent: 100 })));
    }

    // =========================================================================
    // detect_warnings: MemoryBloat
    // =========================================================================

    #[test]
    fn test_memory_bloat_warning_above_10_percent() {
        let breakdown = ContextBreakdown {
            system_prompt_tokens: 0,
            claude_md_tokens: 0,
            tool_definitions_tokens: 0,
            conversation_tokens: 89_000,
            memory_tokens: 11_000,
            total_tokens: 100_000,
            max_tokens: 200_000,
        };

        let warnings = detect_warnings(&breakdown);

        assert!(warnings
            .iter()
            .any(|w| matches!(w, ContextWarning::MemoryBloat { memory_percent: 11 })));
    }

    #[test]
    fn test_no_memory_bloat_at_10_percent() {
        let breakdown = ContextBreakdown {
            system_prompt_tokens: 0,
            claude_md_tokens: 0,
            tool_definitions_tokens: 0,
            conversation_tokens: 90_000,
            memory_tokens: 10_000,
            total_tokens: 100_000,
            max_tokens: 200_000,
        };

        let warnings = detect_warnings(&breakdown);

        assert!(!warnings
            .iter()
            .any(|w| matches!(w, ContextWarning::MemoryBloat { .. })));
    }

    // =========================================================================
    // detect_warnings: TooManyTools
    // =========================================================================

    #[test]
    fn test_too_many_tools_warning_above_30_percent() {
        let breakdown = ContextBreakdown {
            system_prompt_tokens: 0,
            claude_md_tokens: 0,
            tool_definitions_tokens: 31_000,
            conversation_tokens: 69_000,
            memory_tokens: 0,
            total_tokens: 100_000,
            max_tokens: 200_000,
        };

        let warnings = detect_warnings(&breakdown);

        assert!(warnings
            .iter()
            .any(|w| matches!(w, ContextWarning::TooManyTools { tools_percent: 31 })));
    }

    #[test]
    fn test_no_too_many_tools_at_30_percent() {
        let breakdown = ContextBreakdown {
            system_prompt_tokens: 0,
            claude_md_tokens: 0,
            tool_definitions_tokens: 30_000,
            conversation_tokens: 70_000,
            memory_tokens: 0,
            total_tokens: 100_000,
            max_tokens: 200_000,
        };

        let warnings = detect_warnings(&breakdown);

        assert!(!warnings
            .iter()
            .any(|w| matches!(w, ContextWarning::TooManyTools { .. })));
    }

    // =========================================================================
    // detect_warnings: LargeSystemPrompt
    // =========================================================================

    #[test]
    fn test_large_system_prompt_warning_above_5000() {
        let breakdown = ContextBreakdown {
            system_prompt_tokens: 5_001,
            claude_md_tokens: 0,
            tool_definitions_tokens: 0,
            conversation_tokens: 94_999,
            memory_tokens: 0,
            total_tokens: 100_000,
            max_tokens: 200_000,
        };

        let warnings = detect_warnings(&breakdown);

        assert!(warnings
            .iter()
            .any(|w| matches!(w, ContextWarning::LargeSystemPrompt { token_count: 5_001 })));
    }

    #[test]
    fn test_no_large_system_prompt_at_5000() {
        let breakdown = ContextBreakdown {
            system_prompt_tokens: 5_000,
            claude_md_tokens: 0,
            tool_definitions_tokens: 0,
            conversation_tokens: 95_000,
            memory_tokens: 0,
            total_tokens: 100_000,
            max_tokens: 200_000,
        };

        let warnings = detect_warnings(&breakdown);

        assert!(!warnings
            .iter()
            .any(|w| matches!(w, ContextWarning::LargeSystemPrompt { .. })));
    }

    // =========================================================================
    // detect_warnings: edge cases
    // =========================================================================

    #[test]
    fn test_no_warnings_for_zero_total_tokens() {
        let breakdown = ContextBreakdown {
            system_prompt_tokens: 0,
            claude_md_tokens: 0,
            tool_definitions_tokens: 0,
            conversation_tokens: 0,
            memory_tokens: 0,
            total_tokens: 0,
            max_tokens: 200_000,
        };

        let warnings = detect_warnings(&breakdown);

        assert!(warnings.is_empty());
    }

    #[test]
    fn test_no_warnings_for_zero_max_tokens() {
        let breakdown = ContextBreakdown {
            system_prompt_tokens: 100,
            claude_md_tokens: 0,
            tool_definitions_tokens: 0,
            conversation_tokens: 0,
            memory_tokens: 0,
            total_tokens: 100,
            max_tokens: 0,
        };

        let warnings = detect_warnings(&breakdown);

        assert!(warnings.is_empty());
    }

    #[test]
    fn test_multiple_warnings_at_once() {
        let breakdown = ContextBreakdown {
            system_prompt_tokens: 6_000,
            claude_md_tokens: 0,
            tool_definitions_tokens: 40_000,
            conversation_tokens: 40_000,
            memory_tokens: 15_000,
            total_tokens: 101_000,
            max_tokens: 100_000,
        };

        let warnings = detect_warnings(&breakdown);

        // Should trigger NearCapacity, MemoryBloat, TooManyTools, and LargeSystemPrompt
        assert!(warnings
            .iter()
            .any(|w| matches!(w, ContextWarning::NearCapacity { .. })));
        assert!(warnings
            .iter()
            .any(|w| matches!(w, ContextWarning::MemoryBloat { .. })));
        assert!(warnings
            .iter()
            .any(|w| matches!(w, ContextWarning::TooManyTools { .. })));
        assert!(warnings
            .iter()
            .any(|w| matches!(w, ContextWarning::LargeSystemPrompt { .. })));
        assert_eq!(warnings.len(), 4);
    }

    // =========================================================================
    // format_report: output structure
    // =========================================================================

    #[test]
    fn test_format_report_contains_header() {
        let breakdown = ContextBreakdown {
            system_prompt_tokens: 2_100,
            claude_md_tokens: 1_500,
            tool_definitions_tokens: 8_200,
            conversation_tokens: 32_400,
            memory_tokens: 1_000,
            total_tokens: 45_200,
            max_tokens: 200_000,
        };

        let report = format_report(&breakdown, &[]);

        assert!(report.contains("Context Usage: 45,200 / 200,000 tokens (22.6%)"));
    }

    #[test]
    fn test_format_report_contains_all_categories() {
        let breakdown = ContextBreakdown {
            system_prompt_tokens: 2_100,
            claude_md_tokens: 1_500,
            tool_definitions_tokens: 8_200,
            conversation_tokens: 32_400,
            memory_tokens: 1_000,
            total_tokens: 45_200,
            max_tokens: 200_000,
        };

        let report = format_report(&breakdown, &[]);

        assert!(report.contains("System prompt:"));
        assert!(report.contains("CLAUDE.md:"));
        assert!(report.contains("Tool definitions:"));
        assert!(report.contains("Conversation:"));
        assert!(report.contains("Memory:"));
    }

    #[test]
    fn test_format_report_has_tree_connectors() {
        let breakdown = ContextBreakdown {
            system_prompt_tokens: 100,
            claude_md_tokens: 100,
            tool_definitions_tokens: 100,
            conversation_tokens: 100,
            memory_tokens: 100,
            total_tokens: 500,
            max_tokens: 200_000,
        };

        let report = format_report(&breakdown, &[]);

        // First 4 categories use branch connector
        assert!(report.contains("\u{251c}\u{2500}\u{2500}"));
        // Last category uses end connector
        assert!(report.contains("\u{2514}\u{2500}\u{2500}"));
    }

    #[test]
    fn test_format_report_includes_warnings() {
        let breakdown = ContextBreakdown {
            system_prompt_tokens: 100,
            claude_md_tokens: 0,
            tool_definitions_tokens: 0,
            conversation_tokens: 161_900,
            memory_tokens: 0,
            total_tokens: 162_000,
            max_tokens: 200_000,
        };
        let warnings = vec![ContextWarning::NearCapacity { usage_percent: 81 }];

        let report = format_report(&breakdown, &warnings);

        assert!(report.contains("Warnings:"));
        assert!(report.contains("Near capacity (81% used)"));
    }

    #[test]
    fn test_format_report_no_warnings_section_when_empty() {
        let breakdown = ContextBreakdown {
            system_prompt_tokens: 100,
            claude_md_tokens: 100,
            tool_definitions_tokens: 100,
            conversation_tokens: 100,
            memory_tokens: 100,
            total_tokens: 500,
            max_tokens: 200_000,
        };

        let report = format_report(&breakdown, &[]);

        assert!(!report.contains("Warnings:"));
    }

    #[test]
    fn test_format_report_zero_tokens() {
        let breakdown = ContextBreakdown {
            system_prompt_tokens: 0,
            claude_md_tokens: 0,
            tool_definitions_tokens: 0,
            conversation_tokens: 0,
            memory_tokens: 0,
            total_tokens: 0,
            max_tokens: 200_000,
        };

        let report = format_report(&breakdown, &[]);

        assert!(report.contains("Context Usage: 0 / 200,000 tokens (0.0%)"));
        // All categories should show 0.0%
        assert!(report.contains("0 (0.0%)"));
    }

    #[test]
    fn test_format_report_zero_max_tokens() {
        let breakdown = ContextBreakdown {
            system_prompt_tokens: 100,
            claude_md_tokens: 0,
            tool_definitions_tokens: 0,
            conversation_tokens: 0,
            memory_tokens: 0,
            total_tokens: 100,
            max_tokens: 0,
        };

        let report = format_report(&breakdown, &[]);

        assert!(report.contains("Context Usage: 100 / 0 tokens (0.0%)"));
    }

    #[test]
    fn test_format_report_at_exact_capacity() {
        let breakdown = ContextBreakdown {
            system_prompt_tokens: 50_000,
            claude_md_tokens: 50_000,
            tool_definitions_tokens: 50_000,
            conversation_tokens: 50_000,
            memory_tokens: 0,
            total_tokens: 200_000,
            max_tokens: 200_000,
        };
        let warnings = vec![ContextWarning::NearCapacity { usage_percent: 100 }];

        let report = format_report(&breakdown, &warnings);

        assert!(report.contains("200,000 / 200,000 tokens (100.0%)"));
        assert!(report.contains("Near capacity (100% used)"));
    }

    // =========================================================================
    // format_number
    // =========================================================================

    #[test]
    fn test_format_number_zero() {
        assert_eq!(format_number(0), "0");
    }

    #[test]
    fn test_format_number_small() {
        assert_eq!(format_number(42), "42");
        assert_eq!(format_number(999), "999");
    }

    #[test]
    fn test_format_number_thousands() {
        assert_eq!(format_number(1_000), "1,000");
        assert_eq!(format_number(45_200), "45,200");
        assert_eq!(format_number(200_000), "200,000");
    }

    #[test]
    fn test_format_number_millions() {
        assert_eq!(format_number(1_000_000), "1,000,000");
        assert_eq!(format_number(1_234_567), "1,234,567");
    }

    // =========================================================================
    // ContextWarning Display
    // =========================================================================

    #[test]
    fn test_near_capacity_display() {
        let warning = ContextWarning::NearCapacity { usage_percent: 85 };
        let msg = warning.to_string();
        assert!(msg.contains("85% used"));
        assert!(msg.contains("summarizing"));
    }

    #[test]
    fn test_memory_bloat_display() {
        let warning = ContextWarning::MemoryBloat { memory_percent: 15 };
        let msg = warning.to_string();
        assert!(msg.contains("15% of tokens"));
        assert!(msg.contains("pruning"));
    }

    #[test]
    fn test_too_many_tools_display() {
        let warning = ContextWarning::TooManyTools { tools_percent: 35 };
        let msg = warning.to_string();
        assert!(msg.contains("35% of tokens"));
        assert!(msg.contains("disabling"));
    }

    #[test]
    fn test_large_system_prompt_display() {
        let warning = ContextWarning::LargeSystemPrompt { token_count: 7_000 };
        let msg = warning.to_string();
        assert!(msg.contains("7000 tokens"));
        assert!(msg.contains("trimming"));
    }

    // =========================================================================
    // ContextAnalysisInput: Clone and Debug
    // =========================================================================

    #[test]
    fn test_context_analysis_input_clone() {
        let input = ContextAnalysisInput {
            system_prompt_tokens: 100,
            claude_md_tokens: 200,
            tool_definitions_tokens: 300,
            conversation_tokens: 400,
            memory_tokens: 500,
            max_tokens: 200_000,
        };
        let cloned = input.clone();
        assert_eq!(input, cloned);
    }

    #[test]
    fn test_context_breakdown_debug() {
        let breakdown = ContextBreakdown {
            system_prompt_tokens: 0,
            claude_md_tokens: 0,
            tool_definitions_tokens: 0,
            conversation_tokens: 0,
            memory_tokens: 0,
            total_tokens: 0,
            max_tokens: 0,
        };
        let debug = format!("{:?}", breakdown);
        assert!(debug.contains("ContextBreakdown"));
    }

    #[test]
    fn test_context_warning_debug() {
        let warning = ContextWarning::NearCapacity { usage_percent: 90 };
        let debug = format!("{:?}", warning);
        assert!(debug.contains("NearCapacity"));
    }

    // =========================================================================
    // Integration: full pipeline
    // =========================================================================

    #[test]
    fn test_full_pipeline_typical_usage() {
        let input = ContextAnalysisInput {
            system_prompt_tokens: 2_100,
            claude_md_tokens: 1_500,
            tool_definitions_tokens: 8_200,
            conversation_tokens: 32_400,
            memory_tokens: 1_000,
            max_tokens: 200_000,
        };

        let breakdown = analyze_context(&input);
        let warnings = detect_warnings(&breakdown);
        let report = format_report(&breakdown, &warnings);

        assert_eq!(breakdown.total_tokens, 45_200);
        assert!(warnings.is_empty());
        assert!(report.contains("45,200 / 200,000"));
    }

    #[test]
    fn test_full_pipeline_problematic_usage() {
        let input = ContextAnalysisInput {
            system_prompt_tokens: 6_000,
            claude_md_tokens: 2_000,
            tool_definitions_tokens: 60_000,
            conversation_tokens: 100_000,
            memory_tokens: 22_000,
            max_tokens: 200_000,
        };

        let breakdown = analyze_context(&input);
        let warnings = detect_warnings(&breakdown);
        let report = format_report(&breakdown, &warnings);

        assert_eq!(breakdown.total_tokens, 190_000);
        // Should have NearCapacity (95%), MemoryBloat (11.6%), TooManyTools (31.6%), LargeSystemPrompt (6000)
        assert_eq!(warnings.len(), 4);
        assert!(report.contains("Warnings:"));
    }
}
