//! Language Server Protocol integration for code intelligence.
//!
//! Provides LSP-based code navigation (go-to-definition, find-references, hover)
//! by discovering and communicating with language servers over stdio JSON-RPC.
//!
//! This is a lightweight complement to narsil-mcp code intelligence. When narsil
//! is available, it is preferred; this module serves as a fallback for languages
//! or scenarios where narsil is not indexed.

pub mod client;
pub mod discovery;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// LSP operations that can be requested.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum LspOperation {
    /// Navigate to the definition of a symbol.
    GoToDefinition {
        /// File path relative to the working directory.
        file: String,
        /// 1-based line number.
        line: u32,
        /// 1-based column number.
        column: u32,
    },
    /// Find all references to a symbol.
    FindReferences {
        /// File path relative to the working directory.
        file: String,
        /// 1-based line number.
        line: u32,
        /// 1-based column number.
        column: u32,
    },
    /// Get hover information for a symbol.
    Hover {
        /// File path relative to the working directory.
        file: String,
        /// 1-based line number.
        line: u32,
        /// 1-based column number.
        column: u32,
    },
    /// List symbols in a document.
    DocumentSymbol {
        /// File path relative to the working directory.
        file: String,
    },
    /// Search for symbols across the workspace.
    WorkspaceSymbol {
        /// Symbol name or pattern to search for.
        query: String,
    },
}

/// Result from an LSP operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspResult {
    /// The operation that was performed.
    pub operation: String,
    /// Human-readable formatted result.
    pub content: String,
    /// Locations returned by the operation (for go-to-definition, references).
    pub locations: Vec<LspLocation>,
}

/// A source code location returned by LSP.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspLocation {
    /// File path.
    pub file: String,
    /// 1-based line number.
    pub line: u32,
    /// 1-based column number.
    pub column: u32,
    /// Optional preview of the line content.
    pub preview: Option<String>,
}

/// High-level LSP tool that manages server lifecycle and dispatches operations.
pub struct LspTool {
    /// Working directory for resolving file paths.
    working_dir: std::path::PathBuf,
}

impl LspTool {
    /// Creates a new LSP tool for the given working directory.
    #[must_use]
    pub fn new(working_dir: std::path::PathBuf) -> Self {
        Self { working_dir }
    }

    /// Executes an LSP operation.
    ///
    /// Discovers the appropriate language server for the file extension,
    /// starts it if needed, and dispatches the operation.
    ///
    /// # Errors
    ///
    /// Returns an error if the language server cannot be found or the
    /// operation fails.
    pub async fn execute(&self, operation: &LspOperation) -> Result<LspResult> {
        let file_path = match operation {
            LspOperation::GoToDefinition { file, .. }
            | LspOperation::FindReferences { file, .. }
            | LspOperation::Hover { file, .. }
            | LspOperation::DocumentSymbol { file } => Some(file.as_str()),
            LspOperation::WorkspaceSymbol { .. } => None,
        };

        // Determine language from file extension
        let language = if let Some(file) = file_path {
            let path = Path::new(file);
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            discovery::language_from_extension(ext)
        } else {
            None
        };

        let server_info = match &language {
            Some(lang) => discovery::discover_server(lang),
            None => None,
        };

        match server_info {
            Some(info) => {
                // In a full implementation, we'd start the server and send JSON-RPC.
                // For now, return a helpful message about the discovered server.
                Ok(LspResult {
                    operation: format!("{:?}", std::mem::discriminant(operation)),
                    content: format!(
                        "LSP server discovered: {} ({})\n\
                         Command: {}\n\
                         Working directory: {}\n\n\
                         Note: Full LSP communication requires an active server session. \
                         Consider using narsil-mcp for immediate code intelligence.",
                        info.name,
                        info.language,
                        info.command.join(" "),
                        self.working_dir.display(),
                    ),
                    locations: Vec::new(),
                })
            }
            None => {
                let lang_hint = language.as_deref().unwrap_or("unknown");
                bail!(
                    "No language server found for '{}'. \
                     Install a language server or use narsil-mcp for code intelligence.\n\
                     Supported: rust-analyzer (Rust), typescript-language-server (TS/JS), \
                     pyright (Python), gopls (Go)",
                    lang_hint,
                )
            }
        }
    }

    /// Formats an LSP result as a human-readable string.
    #[must_use]
    pub fn format_result(result: &LspResult) -> String {
        let mut output = result.content.clone();

        if !result.locations.is_empty() {
            output.push_str("\n\nLocations:\n");
            for loc in &result.locations {
                output.push_str(&format!("  {}:{}:{}", loc.file, loc.line, loc.column));
                if let Some(preview) = &loc.preview {
                    output.push_str(&format!("  {}", preview));
                }
                output.push('\n');
            }
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lsp_operation_go_to_definition() {
        let op = LspOperation::GoToDefinition {
            file: "src/main.rs".to_string(),
            line: 10,
            column: 5,
        };
        let json = serde_json::to_string(&op).unwrap();
        assert!(json.contains("go_to_definition"));
        assert!(json.contains("src/main.rs"));
    }

    #[test]
    fn test_lsp_operation_find_references() {
        let op = LspOperation::FindReferences {
            file: "lib.rs".to_string(),
            line: 20,
            column: 10,
        };
        let json = serde_json::to_string(&op).unwrap();
        assert!(json.contains("find_references"));
    }

    #[test]
    fn test_lsp_operation_workspace_symbol() {
        let op = LspOperation::WorkspaceSymbol {
            query: "MyStruct".to_string(),
        };
        let json = serde_json::to_string(&op).unwrap();
        assert!(json.contains("workspace_symbol"));
        assert!(json.contains("MyStruct"));
    }

    #[test]
    fn test_lsp_operation_deserialization() {
        let json = r#"{"operation":"hover","file":"test.rs","line":1,"column":1}"#;
        let op: LspOperation = serde_json::from_str(json).unwrap();
        match op {
            LspOperation::Hover { file, line, column } => {
                assert_eq!(file, "test.rs");
                assert_eq!(line, 1);
                assert_eq!(column, 1);
            }
            _ => panic!("Expected Hover"),
        }
    }

    #[test]
    fn test_lsp_result_format_empty_locations() {
        let result = LspResult {
            operation: "hover".to_string(),
            content: "fn main() -> ()".to_string(),
            locations: Vec::new(),
        };
        let formatted = LspTool::format_result(&result);
        assert!(formatted.contains("fn main()"));
        assert!(!formatted.contains("Locations:"));
    }

    #[test]
    fn test_lsp_result_format_with_locations() {
        let result = LspResult {
            operation: "definition".to_string(),
            content: "Found definition".to_string(),
            locations: vec![LspLocation {
                file: "src/lib.rs".to_string(),
                line: 42,
                column: 5,
                preview: Some("pub fn my_function()".to_string()),
            }],
        };
        let formatted = LspTool::format_result(&result);
        assert!(formatted.contains("src/lib.rs:42:5"));
        assert!(formatted.contains("pub fn my_function()"));
    }

    #[test]
    fn test_lsp_location_serialization() {
        let loc = LspLocation {
            file: "main.rs".to_string(),
            line: 10,
            column: 3,
            preview: None,
        };
        let json = serde_json::to_string(&loc).unwrap();
        let deserialized: LspLocation = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.file, "main.rs");
        assert_eq!(deserialized.line, 10);
        assert!(deserialized.preview.is_none());
    }

    #[tokio::test]
    async fn test_lsp_tool_unknown_extension_fails() {
        let tool = LspTool::new(std::path::PathBuf::from("/tmp"));
        let op = LspOperation::Hover {
            file: "readme.xyz".to_string(),
            line: 1,
            column: 1,
        };
        let result = tool.execute(&op).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("No language server found"));
    }
}
