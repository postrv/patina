//! Export slash command for conversation export.
//!
//! This module provides parsing and formatting for `/export` commands that
//! export the current conversation in various formats: Markdown, JSON, or
//! plain text.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::Serialize;
use thiserror::Error;

use crate::types::ConversationEntry;

/// Supported export output formats.
///
/// Each variant produces a different representation of the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// Markdown with headers for roles, code blocks for tool results,
    /// and blockquotes for thinking.
    Markdown,
    /// Structured JSON array with all metadata.
    Json,
    /// Simple plain text with role prefixes.
    PlainText,
}

impl fmt::Display for ExportFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Markdown => write!(f, "markdown"),
            Self::Json => write!(f, "json"),
            Self::PlainText => write!(f, "text"),
        }
    }
}

impl FromStr for ExportFormat {
    type Err = ExportCommandError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "markdown" | "md" => Ok(Self::Markdown),
            "json" => Ok(Self::Json),
            "text" | "txt" | "plaintext" | "plain" => Ok(Self::PlainText),
            _ => Err(ExportCommandError::UnknownFormat(s.to_string())),
        }
    }
}

/// Options controlling how the conversation is exported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportOptions {
    /// Output format for the export.
    pub format: ExportFormat,
    /// File path to write the export to. If `None`, content is returned for display.
    pub output_path: Option<PathBuf>,
    /// Whether to include tool execution results in the output.
    pub include_tool_results: bool,
    /// Whether to include thinking/streaming blocks in the output.
    pub include_thinking: bool,
    /// Whether to include timestamps in the output.
    pub include_timestamps: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            format: ExportFormat::Markdown,
            output_path: None,
            include_tool_results: true,
            include_thinking: true,
            include_timestamps: true,
        }
    }
}

/// Errors that can occur when parsing or executing export commands.
#[derive(Debug, Error)]
pub enum ExportCommandError {
    /// An unknown export format was specified.
    #[error("unknown format '{0}'. Valid formats: markdown, json, text")]
    UnknownFormat(String),

    /// Failed to serialize conversation to JSON.
    #[error("JSON serialization failed: {0}")]
    JsonSerialize(#[from] serde_json::Error),

    /// Failed to write the export file.
    #[error("failed to write export file: {0}")]
    WriteFile(#[from] std::io::Error),
}

/// JSON representation of a single conversation entry for export.
#[derive(Debug, Serialize)]
struct JsonEntry {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_input: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_error: Option<bool>,
}

/// Parses a `/export` argument string into [`ExportOptions`].
///
/// Expected usage: `/export [format] [path] [--no-tools] [--no-thinking]`
///
/// - `format` defaults to `markdown` if omitted
/// - `path` is optional; if absent content is returned for display
/// - `--no-tools` excludes tool execution blocks
/// - `--no-thinking` excludes streaming/thinking blocks
///
/// # Errors
///
/// Returns [`ExportCommandError::UnknownFormat`] if the format string is unrecognised.
///
/// # Examples
///
/// ```
/// use patina::commands::export::{parse_export_args, ExportFormat};
///
/// let opts = parse_export_args("json /tmp/out.json --no-tools").unwrap();
/// assert_eq!(opts.format, ExportFormat::Json);
/// assert_eq!(opts.output_path.unwrap().to_str().unwrap(), "/tmp/out.json");
/// assert!(!opts.include_tool_results);
/// assert!(opts.include_thinking);
/// ```
#[must_use = "returns parsed export options that should be used"]
pub fn parse_export_args(args: &str) -> Result<ExportOptions, ExportCommandError> {
    let trimmed = args.trim();
    let mut options = ExportOptions::default();

    if trimmed.is_empty() {
        return Ok(options);
    }

    let mut positional: Vec<&str> = Vec::new();

    for token in trimmed.split_whitespace() {
        match token {
            "--no-tools" => options.include_tool_results = false,
            "--no-thinking" => options.include_thinking = false,
            "--no-timestamps" => options.include_timestamps = false,
            _ => positional.push(token),
        }
    }

    // First positional: format (or path if it looks like a path)
    if let Some(&first) = positional.first() {
        if looks_like_path(first) {
            // No explicit format, just a path
            options.output_path = Some(PathBuf::from(first));
        } else {
            options.format = ExportFormat::from_str(first)?;
        }
    }

    // Second positional: path (only if first was a format)
    if positional.len() >= 2 && !looks_like_path(positional[0]) {
        options.output_path = Some(PathBuf::from(positional[1]));
    }

    Ok(options)
}

/// Heuristic: a token looks like a file path if it contains a path separator
/// or a dot-extension pattern.
fn looks_like_path(token: &str) -> bool {
    token.contains('/') || token.contains('\\') || token.contains('.')
}

/// Exports conversation entries to a string in the specified format.
///
/// Filters entries based on `options.include_tool_results` and
/// `options.include_thinking`, then formats them according to
/// `options.format`.
///
/// # Errors
///
/// Returns [`ExportCommandError::JsonSerialize`] if JSON serialization fails.
///
/// # Examples
///
/// ```
/// use patina::commands::export::{export_conversation, ExportOptions, ExportFormat};
/// use patina::types::ConversationEntry;
///
/// let entries = vec![
///     ConversationEntry::UserMessage("Hello".to_string()),
///     ConversationEntry::AssistantMessage("Hi there!".to_string()),
/// ];
/// let opts = ExportOptions { format: ExportFormat::PlainText, ..Default::default() };
/// let output = export_conversation(&entries, &opts).unwrap();
/// assert!(output.contains("User: Hello"));
/// ```
#[must_use = "returns the exported conversation content"]
pub fn export_conversation(
    entries: &[ConversationEntry],
    options: &ExportOptions,
) -> Result<String, ExportCommandError> {
    match options.format {
        ExportFormat::Markdown => Ok(export_markdown(entries, options)),
        ExportFormat::Json => export_json(entries, options),
        ExportFormat::PlainText => Ok(export_plain_text(entries, options)),
    }
}

/// Writes the exported content to a file.
///
/// Creates parent directories if they do not exist.
///
/// # Errors
///
/// Returns [`ExportCommandError::WriteFile`] if the file cannot be written.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// use patina::commands::export::write_export;
///
/// write_export("# Conversation\n\nHello!", Path::new("/tmp/export.md")).unwrap();
/// ```
pub fn write_export(content: &str, path: &Path) -> Result<(), ExportCommandError> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(path, content)?;
    Ok(())
}

/// Formats conversation entries as Markdown.
///
/// - User messages: `## User\n\n{text}`
/// - Assistant messages: `## Assistant\n\n{text}`
/// - Tool executions: `### Tool: {name}\n\n\`\`\`\n{input}\n\`\`\`\n\n{output}`
/// - Streaming entries (thinking): `> {text}` blockquotes
fn export_markdown(entries: &[ConversationEntry], options: &ExportOptions) -> String {
    let mut output = String::from("# Conversation Export\n");

    for entry in entries {
        match entry {
            ConversationEntry::UserMessage(text) => {
                output.push_str("\n## User\n\n");
                output.push_str(text);
                output.push('\n');
            }
            ConversationEntry::AssistantMessage(text) => {
                output.push_str("\n## Assistant\n\n");
                output.push_str(text);
                output.push('\n');
            }
            ConversationEntry::Streaming { text, .. } => {
                if !options.include_thinking {
                    continue;
                }
                output.push_str("\n> ");
                output.push_str(&text.replace('\n', "\n> "));
                output.push('\n');
            }
            ConversationEntry::ToolExecution {
                name,
                input,
                output: tool_output,
                is_error,
                ..
            } => {
                if !options.include_tool_results {
                    continue;
                }
                output.push_str("\n### Tool: ");
                output.push_str(name);
                if *is_error {
                    output.push_str(" (error)");
                }
                output.push_str("\n\n```\n");
                output.push_str(input);
                output.push_str("\n```\n");
                if let Some(result) = tool_output {
                    output.push_str("\n```\n");
                    output.push_str(result);
                    output.push_str("\n```\n");
                }
            }
            ConversationEntry::ImageDisplay { alt_text, .. } => {
                output.push_str("\n*[Image");
                if let Some(alt) = alt_text {
                    output.push_str(": ");
                    output.push_str(alt);
                }
                output.push_str("]*\n");
            }
        }
    }

    output
}

/// Formats conversation entries as a JSON array.
fn export_json(
    entries: &[ConversationEntry],
    options: &ExportOptions,
) -> Result<String, ExportCommandError> {
    let json_entries: Vec<JsonEntry> = entries
        .iter()
        .filter_map(|entry| match entry {
            ConversationEntry::UserMessage(text) => Some(JsonEntry {
                role: "user".to_string(),
                content: text.clone(),
                tool_name: None,
                tool_input: None,
                tool_output: None,
                is_error: None,
            }),
            ConversationEntry::AssistantMessage(text) => Some(JsonEntry {
                role: "assistant".to_string(),
                content: text.clone(),
                tool_name: None,
                tool_input: None,
                tool_output: None,
                is_error: None,
            }),
            ConversationEntry::Streaming { text, .. } => {
                if !options.include_thinking {
                    return None;
                }
                Some(JsonEntry {
                    role: "thinking".to_string(),
                    content: text.clone(),
                    tool_name: None,
                    tool_input: None,
                    tool_output: None,
                    is_error: None,
                })
            }
            ConversationEntry::ToolExecution {
                name,
                input,
                output,
                is_error,
                ..
            } => {
                if !options.include_tool_results {
                    return None;
                }
                Some(JsonEntry {
                    role: "tool".to_string(),
                    content: String::new(),
                    tool_name: Some(name.clone()),
                    tool_input: Some(input.clone()),
                    tool_output: output.clone(),
                    is_error: Some(*is_error),
                })
            }
            ConversationEntry::ImageDisplay { alt_text, .. } => Some(JsonEntry {
                role: "image".to_string(),
                content: alt_text.clone().unwrap_or_default(),
                tool_name: None,
                tool_input: None,
                tool_output: None,
                is_error: None,
            }),
        })
        .collect();

    Ok(serde_json::to_string_pretty(&json_entries)?)
}

/// Formats conversation entries as plain text.
fn export_plain_text(entries: &[ConversationEntry], options: &ExportOptions) -> String {
    let mut output = String::new();

    for entry in entries {
        match entry {
            ConversationEntry::UserMessage(text) => {
                output.push_str("User: ");
                output.push_str(text);
                output.push_str("\n\n");
            }
            ConversationEntry::AssistantMessage(text) => {
                output.push_str("Assistant: ");
                output.push_str(text);
                output.push_str("\n\n");
            }
            ConversationEntry::Streaming { text, .. } => {
                if !options.include_thinking {
                    continue;
                }
                output.push_str("(thinking): ");
                output.push_str(text);
                output.push_str("\n\n");
            }
            ConversationEntry::ToolExecution {
                name,
                input,
                output: tool_output,
                is_error,
                ..
            } => {
                if !options.include_tool_results {
                    continue;
                }
                output.push_str("Tool[");
                output.push_str(name);
                output.push(']');
                if *is_error {
                    output.push_str(" (error)");
                }
                output.push_str(": ");
                output.push_str(input);
                if let Some(result) = tool_output {
                    output.push_str(" -> ");
                    output.push_str(result);
                }
                output.push_str("\n\n");
            }
            ConversationEntry::ImageDisplay { alt_text, .. } => {
                output.push_str("[Image");
                if let Some(alt) = alt_text {
                    output.push_str(": ");
                    output.push_str(alt);
                }
                output.push_str("]\n\n");
            }
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // ExportFormat: Display and FromStr
    // =========================================================================

    #[test]
    fn format_display_markdown() {
        assert_eq!(ExportFormat::Markdown.to_string(), "markdown");
    }

    #[test]
    fn format_display_json() {
        assert_eq!(ExportFormat::Json.to_string(), "json");
    }

    #[test]
    fn format_display_plain_text() {
        assert_eq!(ExportFormat::PlainText.to_string(), "text");
    }

    #[test]
    fn format_from_str_markdown() {
        assert_eq!(
            ExportFormat::from_str("markdown").unwrap(),
            ExportFormat::Markdown
        );
        assert_eq!(
            ExportFormat::from_str("md").unwrap(),
            ExportFormat::Markdown
        );
        assert_eq!(
            ExportFormat::from_str("MARKDOWN").unwrap(),
            ExportFormat::Markdown
        );
    }

    #[test]
    fn format_from_str_json() {
        assert_eq!(ExportFormat::from_str("json").unwrap(), ExportFormat::Json);
        assert_eq!(ExportFormat::from_str("JSON").unwrap(), ExportFormat::Json);
    }

    #[test]
    fn format_from_str_plain_text() {
        assert_eq!(
            ExportFormat::from_str("text").unwrap(),
            ExportFormat::PlainText
        );
        assert_eq!(
            ExportFormat::from_str("txt").unwrap(),
            ExportFormat::PlainText
        );
        assert_eq!(
            ExportFormat::from_str("plaintext").unwrap(),
            ExportFormat::PlainText
        );
        assert_eq!(
            ExportFormat::from_str("plain").unwrap(),
            ExportFormat::PlainText
        );
    }

    #[test]
    fn format_from_str_unknown() {
        let err = ExportFormat::from_str("docx").unwrap_err();
        assert!(err.to_string().contains("docx"));
    }

    // =========================================================================
    // parse_export_args
    // =========================================================================

    #[test]
    fn parse_args_empty() {
        let opts = parse_export_args("").unwrap();
        assert_eq!(opts.format, ExportFormat::Markdown);
        assert!(opts.output_path.is_none());
        assert!(opts.include_tool_results);
        assert!(opts.include_thinking);
    }

    #[test]
    fn parse_args_format_only() {
        let opts = parse_export_args("json").unwrap();
        assert_eq!(opts.format, ExportFormat::Json);
        assert!(opts.output_path.is_none());
    }

    #[test]
    fn parse_args_format_and_path() {
        let opts = parse_export_args("markdown /tmp/export.md").unwrap();
        assert_eq!(opts.format, ExportFormat::Markdown);
        assert_eq!(opts.output_path.unwrap(), PathBuf::from("/tmp/export.md"));
    }

    #[test]
    fn parse_args_path_only() {
        let opts = parse_export_args("/tmp/conversation.md").unwrap();
        assert_eq!(opts.format, ExportFormat::Markdown);
        assert_eq!(
            opts.output_path.unwrap(),
            PathBuf::from("/tmp/conversation.md")
        );
    }

    #[test]
    fn parse_args_no_tools_flag() {
        let opts = parse_export_args("json --no-tools").unwrap();
        assert_eq!(opts.format, ExportFormat::Json);
        assert!(!opts.include_tool_results);
        assert!(opts.include_thinking);
    }

    #[test]
    fn parse_args_no_thinking_flag() {
        let opts = parse_export_args("text --no-thinking").unwrap();
        assert_eq!(opts.format, ExportFormat::PlainText);
        assert!(opts.include_tool_results);
        assert!(!opts.include_thinking);
    }

    #[test]
    fn parse_args_both_flags() {
        let opts = parse_export_args("json /tmp/out.json --no-tools --no-thinking").unwrap();
        assert_eq!(opts.format, ExportFormat::Json);
        assert_eq!(opts.output_path.unwrap(), PathBuf::from("/tmp/out.json"));
        assert!(!opts.include_tool_results);
        assert!(!opts.include_thinking);
    }

    #[test]
    fn parse_args_unknown_format() {
        let err = parse_export_args("docx").unwrap_err();
        assert!(matches!(err, ExportCommandError::UnknownFormat(_)));
    }

    #[test]
    fn parse_args_whitespace_only() {
        let opts = parse_export_args("   ").unwrap();
        assert_eq!(opts.format, ExportFormat::Markdown);
    }

    #[test]
    fn parse_args_no_timestamps_flag() {
        let opts = parse_export_args("json --no-timestamps").unwrap();
        assert!(!opts.include_timestamps);
    }

    // =========================================================================
    // Helper: sample conversation entries for tests
    // =========================================================================

    fn sample_entries() -> Vec<ConversationEntry> {
        vec![
            ConversationEntry::UserMessage("Hello, Claude!".to_string()),
            ConversationEntry::AssistantMessage("Hi! How can I help?".to_string()),
            ConversationEntry::ToolExecution {
                name: "bash".to_string(),
                input: "ls -la".to_string(),
                output: Some("total 42\nfile1.rs\nfile2.rs".to_string()),
                is_error: false,
                follows_message_idx: Some(1),
                tool_id: Some("toolu_123".to_string()),
            },
            ConversationEntry::Streaming {
                text: "Let me think...".to_string(),
                complete: false,
            },
            ConversationEntry::AssistantMessage("Here are the files.".to_string()),
        ]
    }

    // =========================================================================
    // export_conversation: Markdown format
    // =========================================================================

    #[test]
    fn export_markdown_format() {
        let entries = sample_entries();
        let opts = ExportOptions {
            format: ExportFormat::Markdown,
            ..Default::default()
        };
        let result = export_conversation(&entries, &opts).unwrap();

        assert!(result.starts_with("# Conversation Export\n"));
        assert!(result.contains("## User\n\nHello, Claude!"));
        assert!(result.contains("## Assistant\n\nHi! How can I help?"));
        assert!(result.contains("### Tool: bash"));
        assert!(result.contains("```\nls -la\n```"));
        assert!(result.contains("```\ntotal 42\nfile1.rs\nfile2.rs\n```"));
        assert!(result.contains("> Let me think..."));
    }

    #[test]
    fn export_markdown_tool_error() {
        let entries = vec![ConversationEntry::ToolExecution {
            name: "bash".to_string(),
            input: "bad-command".to_string(),
            output: Some("command not found".to_string()),
            is_error: true,
            follows_message_idx: None,
            tool_id: None,
        }];
        let opts = ExportOptions {
            format: ExportFormat::Markdown,
            ..Default::default()
        };
        let result = export_conversation(&entries, &opts).unwrap();
        assert!(result.contains("### Tool: bash (error)"));
    }

    #[test]
    fn export_markdown_image_entry() {
        let entries = vec![ConversationEntry::ImageDisplay {
            width: 100,
            height: 50,
            pixels: vec![],
            alt_text: Some("screenshot".to_string()),
        }];
        let opts = ExportOptions {
            format: ExportFormat::Markdown,
            ..Default::default()
        };
        let result = export_conversation(&entries, &opts).unwrap();
        assert!(result.contains("*[Image: screenshot]*"));
    }

    // =========================================================================
    // export_conversation: JSON format
    // =========================================================================

    #[test]
    fn export_json_format() {
        let entries = sample_entries();
        let opts = ExportOptions {
            format: ExportFormat::Json,
            ..Default::default()
        };
        let result = export_conversation(&entries, &opts).unwrap();

        let parsed: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed.len(), 5);

        assert_eq!(parsed[0]["role"], "user");
        assert_eq!(parsed[0]["content"], "Hello, Claude!");

        assert_eq!(parsed[1]["role"], "assistant");
        assert_eq!(parsed[1]["content"], "Hi! How can I help?");

        assert_eq!(parsed[2]["role"], "tool");
        assert_eq!(parsed[2]["tool_name"], "bash");
        assert_eq!(parsed[2]["tool_input"], "ls -la");

        assert_eq!(parsed[3]["role"], "thinking");
        assert_eq!(parsed[3]["content"], "Let me think...");

        assert_eq!(parsed[4]["role"], "assistant");
    }

    #[test]
    fn export_json_is_valid_json() {
        let entries = sample_entries();
        let opts = ExportOptions {
            format: ExportFormat::Json,
            ..Default::default()
        };
        let result = export_conversation(&entries, &opts).unwrap();
        assert!(serde_json::from_str::<serde_json::Value>(&result).is_ok());
    }

    // =========================================================================
    // export_conversation: PlainText format
    // =========================================================================

    #[test]
    fn export_plain_text_format() {
        let entries = sample_entries();
        let opts = ExportOptions {
            format: ExportFormat::PlainText,
            ..Default::default()
        };
        let result = export_conversation(&entries, &opts).unwrap();

        assert!(result.contains("User: Hello, Claude!"));
        assert!(result.contains("Assistant: Hi! How can I help?"));
        assert!(result.contains("Tool[bash]: ls -la -> total 42\nfile1.rs\nfile2.rs"));
        assert!(result.contains("(thinking): Let me think..."));
        assert!(result.contains("Assistant: Here are the files."));
    }

    #[test]
    fn export_plain_text_tool_error() {
        let entries = vec![ConversationEntry::ToolExecution {
            name: "bash".to_string(),
            input: "bad".to_string(),
            output: Some("error".to_string()),
            is_error: true,
            follows_message_idx: None,
            tool_id: None,
        }];
        let opts = ExportOptions {
            format: ExportFormat::PlainText,
            ..Default::default()
        };
        let result = export_conversation(&entries, &opts).unwrap();
        assert!(result.contains("Tool[bash] (error): bad -> error"));
    }

    #[test]
    fn export_plain_text_image_entry() {
        let entries = vec![ConversationEntry::ImageDisplay {
            width: 100,
            height: 50,
            pixels: vec![],
            alt_text: Some("diagram".to_string()),
        }];
        let opts = ExportOptions {
            format: ExportFormat::PlainText,
            ..Default::default()
        };
        let result = export_conversation(&entries, &opts).unwrap();
        assert!(result.contains("[Image: diagram]"));
    }

    // =========================================================================
    // --no-tools filtering
    // =========================================================================

    #[test]
    fn export_no_tools_excludes_tool_results_markdown() {
        let entries = sample_entries();
        let opts = ExportOptions {
            format: ExportFormat::Markdown,
            include_tool_results: false,
            ..Default::default()
        };
        let result = export_conversation(&entries, &opts).unwrap();

        assert!(result.contains("## User"));
        assert!(result.contains("## Assistant"));
        assert!(!result.contains("### Tool"));
        assert!(!result.contains("bash"));
    }

    #[test]
    fn export_no_tools_excludes_tool_results_json() {
        let entries = sample_entries();
        let opts = ExportOptions {
            format: ExportFormat::Json,
            include_tool_results: false,
            ..Default::default()
        };
        let result = export_conversation(&entries, &opts).unwrap();
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();

        // 5 entries minus 1 tool = 4
        assert_eq!(parsed.len(), 4);
        assert!(parsed.iter().all(|e| e["role"] != "tool"));
    }

    #[test]
    fn export_no_tools_excludes_tool_results_plain_text() {
        let entries = sample_entries();
        let opts = ExportOptions {
            format: ExportFormat::PlainText,
            include_tool_results: false,
            ..Default::default()
        };
        let result = export_conversation(&entries, &opts).unwrap();

        assert!(result.contains("User:"));
        assert!(result.contains("Assistant:"));
        assert!(!result.contains("Tool["));
    }

    // =========================================================================
    // --no-thinking filtering
    // =========================================================================

    #[test]
    fn export_no_thinking_excludes_streaming_markdown() {
        let entries = sample_entries();
        let opts = ExportOptions {
            format: ExportFormat::Markdown,
            include_thinking: false,
            ..Default::default()
        };
        let result = export_conversation(&entries, &opts).unwrap();

        assert!(result.contains("## User"));
        assert!(result.contains("## Assistant"));
        assert!(!result.contains("Let me think"));
    }

    #[test]
    fn export_no_thinking_excludes_streaming_json() {
        let entries = sample_entries();
        let opts = ExportOptions {
            format: ExportFormat::Json,
            include_thinking: false,
            ..Default::default()
        };
        let result = export_conversation(&entries, &opts).unwrap();
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();

        // 5 entries minus 1 streaming = 4
        assert_eq!(parsed.len(), 4);
        assert!(parsed.iter().all(|e| e["role"] != "thinking"));
    }

    #[test]
    fn export_no_thinking_excludes_streaming_plain_text() {
        let entries = sample_entries();
        let opts = ExportOptions {
            format: ExportFormat::PlainText,
            include_thinking: false,
            ..Default::default()
        };
        let result = export_conversation(&entries, &opts).unwrap();

        assert!(!result.contains("(thinking)"));
    }

    // =========================================================================
    // Empty conversation
    // =========================================================================

    #[test]
    fn export_empty_conversation_markdown() {
        let entries: Vec<ConversationEntry> = vec![];
        let opts = ExportOptions {
            format: ExportFormat::Markdown,
            ..Default::default()
        };
        let result = export_conversation(&entries, &opts).unwrap();
        assert_eq!(result, "# Conversation Export\n");
    }

    #[test]
    fn export_empty_conversation_json() {
        let entries: Vec<ConversationEntry> = vec![];
        let opts = ExportOptions {
            format: ExportFormat::Json,
            ..Default::default()
        };
        let result = export_conversation(&entries, &opts).unwrap();
        assert_eq!(result, "[]");
    }

    #[test]
    fn export_empty_conversation_plain_text() {
        let entries: Vec<ConversationEntry> = vec![];
        let opts = ExportOptions {
            format: ExportFormat::PlainText,
            ..Default::default()
        };
        let result = export_conversation(&entries, &opts).unwrap();
        assert!(result.is_empty());
    }

    // =========================================================================
    // write_export
    // =========================================================================

    #[test]
    fn write_export_creates_file() {
        let dir = std::env::temp_dir().join("patina_export_test");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("test_export.md");

        write_export("# Test Export\n\nHello!", &path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "# Test Export\n\nHello!");

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_export_creates_parent_dirs() {
        let dir = std::env::temp_dir().join("patina_export_test_nested");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("sub").join("dir").join("export.json");

        write_export("{}", &path).unwrap();

        assert!(path.exists());

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    // =========================================================================
    // ExportOptions default
    // =========================================================================

    #[test]
    fn export_options_default() {
        let opts = ExportOptions::default();
        assert_eq!(opts.format, ExportFormat::Markdown);
        assert!(opts.output_path.is_none());
        assert!(opts.include_tool_results);
        assert!(opts.include_thinking);
        assert!(opts.include_timestamps);
    }

    // =========================================================================
    // Error display
    // =========================================================================

    #[test]
    fn error_display_unknown_format() {
        let err = ExportCommandError::UnknownFormat("docx".to_string());
        let msg = err.to_string();
        assert!(msg.contains("docx"));
        assert!(msg.contains("unknown format"));
    }

    // =========================================================================
    // ExportFormat debug and clone
    // =========================================================================

    #[test]
    fn format_debug() {
        assert!(format!("{:?}", ExportFormat::Markdown).contains("Markdown"));
        assert!(format!("{:?}", ExportFormat::Json).contains("Json"));
        assert!(format!("{:?}", ExportFormat::PlainText).contains("PlainText"));
    }

    #[test]
    fn format_clone_eq() {
        let a = ExportFormat::Json;
        let b = a;
        assert_eq!(a, b);
    }

    // =========================================================================
    // Tool execution with no output (pending)
    // =========================================================================

    #[test]
    fn export_markdown_tool_pending_output() {
        let entries = vec![ConversationEntry::ToolExecution {
            name: "bash".to_string(),
            input: "sleep 10".to_string(),
            output: None,
            is_error: false,
            follows_message_idx: None,
            tool_id: None,
        }];
        let opts = ExportOptions {
            format: ExportFormat::Markdown,
            ..Default::default()
        };
        let result = export_conversation(&entries, &opts).unwrap();
        assert!(result.contains("### Tool: bash"));
        assert!(result.contains("```\nsleep 10\n```"));
        // No output block when output is None
        let count = result.matches("```").count();
        // Header code block only (open + close = 2)
        assert_eq!(count, 2);
    }

    #[test]
    fn export_plain_text_tool_pending_output() {
        let entries = vec![ConversationEntry::ToolExecution {
            name: "bash".to_string(),
            input: "sleep 10".to_string(),
            output: None,
            is_error: false,
            follows_message_idx: None,
            tool_id: None,
        }];
        let opts = ExportOptions {
            format: ExportFormat::PlainText,
            ..Default::default()
        };
        let result = export_conversation(&entries, &opts).unwrap();
        assert!(result.contains("Tool[bash]: sleep 10"));
        assert!(!result.contains("->"));
    }

    // =========================================================================
    // Image without alt text
    // =========================================================================

    #[test]
    fn export_markdown_image_no_alt() {
        let entries = vec![ConversationEntry::ImageDisplay {
            width: 100,
            height: 50,
            pixels: vec![],
            alt_text: None,
        }];
        let opts = ExportOptions {
            format: ExportFormat::Markdown,
            ..Default::default()
        };
        let result = export_conversation(&entries, &opts).unwrap();
        assert!(result.contains("*[Image]*"));
    }

    #[test]
    fn export_plain_text_image_no_alt() {
        let entries = vec![ConversationEntry::ImageDisplay {
            width: 100,
            height: 50,
            pixels: vec![],
            alt_text: None,
        }];
        let opts = ExportOptions {
            format: ExportFormat::PlainText,
            ..Default::default()
        };
        let result = export_conversation(&entries, &opts).unwrap();
        assert!(result.contains("[Image]"));
    }
}
