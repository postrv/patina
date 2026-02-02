//! Context extraction and suggestion for narsil integration.
//!
//! This module provides functionality to:
//! 1. Extract code references (functions, files) from conversation messages
//! 2. Suggest relevant context based on code references using narsil
//!
//! # Code References
//!
//! Code references are extracted from message text using pattern matching:
//! - Function references: `foo()`, `my_function()`, backtick-quoted identifiers
//! - File references: `src/main.rs`, paths ending in common extensions
//! - Line references: `file.rs:42`, `src/lib.rs:100-150`
//!
//! # Context Suggestions
//!
//! When references are found, narsil can provide relevant context:
//! - For functions: callers, callees, and related functions
//! - For files: imports, dependencies, and related modules
//!
//! # Example
//!
//! ```ignore
//! use patina::narsil::context::{extract_code_references, CodeReference};
//!
//! let text = "Look at the process_data function in src/handler.rs";
//! let refs = extract_code_references(text);
//!
//! assert!(refs.iter().any(|r| matches!(r, CodeReference::Function { name, .. } if name == "process_data")));
//! assert!(refs.iter().any(|r| matches!(r, CodeReference::File { path, .. } if path == "src/handler.rs")));
//! ```

use std::collections::HashSet;

/// A reference to code extracted from a message.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CodeReference {
    /// A function or method reference.
    Function {
        /// The function name (without parentheses).
        name: String,
    },
    /// A file path reference.
    File {
        /// The file path.
        path: String,
        /// Optional line number or range.
        line: Option<LineRef>,
    },
}

impl CodeReference {
    /// Creates a function reference.
    #[must_use]
    pub fn function(name: impl Into<String>) -> Self {
        Self::Function { name: name.into() }
    }

    /// Creates a file reference without line numbers.
    #[must_use]
    pub fn file(path: impl Into<String>) -> Self {
        Self::File {
            path: path.into(),
            line: None,
        }
    }

    /// Creates a file reference with a line number.
    #[must_use]
    pub fn file_with_line(path: impl Into<String>, line: u32) -> Self {
        Self::File {
            path: path.into(),
            line: Some(LineRef::Single(line)),
        }
    }

    /// Creates a file reference with a line range.
    #[must_use]
    pub fn file_with_range(path: impl Into<String>, start: u32, end: u32) -> Self {
        Self::File {
            path: path.into(),
            line: Some(LineRef::Range(start, end)),
        }
    }

    /// Returns the name if this is a function reference.
    #[must_use]
    pub fn as_function_name(&self) -> Option<&str> {
        match self {
            Self::Function { name } => Some(name),
            Self::File { .. } => None,
        }
    }

    /// Returns the path if this is a file reference.
    #[must_use]
    pub fn as_file_path(&self) -> Option<&str> {
        match self {
            Self::File { path, .. } => Some(path),
            Self::Function { .. } => None,
        }
    }
}

/// A line reference within a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LineRef {
    /// A single line number.
    Single(u32),
    /// A range of lines (start, end) inclusive.
    Range(u32, u32),
}

/// A suggestion for context to add to a conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextSuggestion {
    /// The code reference that triggered this suggestion.
    pub source: CodeReference,
    /// The type of context being suggested.
    pub kind: ContextKind,
    /// Human-readable description of the suggestion.
    pub description: String,
    /// The actual context content (code, callers list, etc.).
    pub content: String,
}

/// The kind of context being suggested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextKind {
    /// Functions that call the referenced function.
    Callers,
    /// Functions called by the referenced function.
    Callees,
    /// Files/modules that the referenced file imports.
    Imports,
    /// Files/modules that import the referenced file.
    ImportedBy,
    /// The definition/implementation of a symbol.
    Definition,
}

/// Extracts code references from text.
///
/// Scans the input text for patterns that look like code references:
/// - Function calls: `foo()`, `MyClass::method()`
/// - Backtick-quoted identifiers: \`process_data\`
/// - File paths: `src/main.rs`, `lib/handler.py`
/// - Line references: `file.rs:42`, `src/lib.rs:100-150`
///
/// # Arguments
///
/// * `text` - The text to scan for code references
///
/// # Returns
///
/// A vector of unique code references found in the text.
///
/// # Example
///
/// ```
/// use patina::narsil::context::{extract_code_references, CodeReference};
///
/// let refs = extract_code_references("Call the `process_data` function");
/// assert!(!refs.is_empty());
/// ```
#[must_use]
pub fn extract_code_references(text: &str) -> Vec<CodeReference> {
    let mut refs = HashSet::new();

    // Extract function references from backticks: `function_name`
    extract_backtick_functions(text, &mut refs);

    // Extract function calls: foo(), bar_baz()
    extract_function_calls(text, &mut refs);

    // Extract file paths with optional line numbers
    extract_file_paths(text, &mut refs);

    refs.into_iter().collect()
}

/// Extracts function names from backtick-quoted text.
fn extract_backtick_functions(text: &str, refs: &mut HashSet<CodeReference>) {
    // Match `identifier` patterns where identifier looks like a function name
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '`' {
            let mut content = String::new();
            for inner in chars.by_ref() {
                if inner == '`' {
                    break;
                }
                content.push(inner);
            }
            // Check if content looks like a function name (snake_case or camelCase identifier)
            if is_valid_function_name(&content) {
                refs.insert(CodeReference::function(content));
            }
        }
    }
}

/// Extracts function call patterns like foo() or bar_baz().
fn extract_function_calls(text: &str, refs: &mut HashSet<CodeReference>) {
    // Match identifier followed by ()
    let mut i = 0;
    let bytes = text.as_bytes();

    while i < bytes.len() {
        // Find start of potential identifier
        if is_ident_start(bytes[i]) {
            let start = i;
            while i < bytes.len() && is_ident_char(bytes[i]) {
                i += 1;
            }
            let name = &text[start..i];

            // Check if followed by (
            if i < bytes.len() && bytes[i] == b'(' {
                // Skip common keywords and very short names
                if name.len() >= 2 && !is_keyword(name) {
                    refs.insert(CodeReference::function(name));
                }
            }
        } else {
            i += 1;
        }
    }
}

/// Extracts file paths with optional line references.
fn extract_file_paths(text: &str, refs: &mut HashSet<CodeReference>) {
    // Common file extensions to look for (sorted longest first to prevent .js matching .json)
    let extensions = [
        ".swift", ".scala", ".java", ".toml", ".yaml", ".json", ".tsx", ".jsx", ".cpp", ".hpp",
        ".php", ".yml", ".rs", ".py", ".ts", ".js", ".go", ".rb", ".kt", ".md", ".c", ".h",
    ];

    // Match paths like src/foo.rs or ./bar/baz.py
    let words: Vec<&str> = text.split_whitespace().collect();

    for word in words {
        // Check if this looks like a file path (check before stripping to preserve extension)
        for ext in &extensions {
            if let Some(pos) = word.find(ext) {
                let end_of_ext = pos + ext.len();

                // Extract the path part (up to extension) and everything after
                let path_part = &word[..end_of_ext];
                let after_ext = if end_of_ext < word.len() {
                    &word[end_of_ext..]
                } else {
                    ""
                };

                // Strip leading punctuation from path
                let path = path_part.trim_start_matches(|c: char| {
                    c == ','
                        || c == '.'
                        || c == ';'
                        || c == '"'
                        || c == '\''
                        || c == '('
                        || c == '['
                });

                // Parse optional line reference (strip trailing punctuation from line ref)
                let line_ref = if let Some(stripped) = after_ext.strip_prefix(':') {
                    let line_part = stripped.trim_end_matches(|c: char| {
                        c == ','
                            || c == '.'
                            || c == ';'
                            || c == '"'
                            || c == '\''
                            || c == ')'
                            || c == ']'
                    });
                    parse_line_ref(line_part)
                } else {
                    None
                };

                // Validate path looks reasonable
                if is_valid_path(path) {
                    refs.insert(CodeReference::File {
                        path: path.to_string(),
                        line: line_ref,
                    });
                }
                break;
            }
        }
    }
}

/// Parses a line reference like "42" or "100-150".
fn parse_line_ref(s: &str) -> Option<LineRef> {
    if let Some(dash_pos) = s.find('-') {
        let start: u32 = s[..dash_pos].parse().ok()?;
        let end: u32 = s[dash_pos + 1..].parse().ok()?;
        Some(LineRef::Range(start, end))
    } else {
        let line: u32 = s.parse().ok()?;
        Some(LineRef::Single(line))
    }
}

/// Checks if a string is a valid function name.
fn is_valid_function_name(s: &str) -> bool {
    if s.is_empty() || s.len() > 100 {
        return false;
    }
    let bytes = s.as_bytes();
    // Must start with letter or underscore
    if !is_ident_start(bytes[0]) {
        return false;
    }
    // Rest must be alphanumeric or underscore
    bytes.iter().all(|&b| is_ident_char(b))
}

/// Checks if a byte can start an identifier.
fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

/// Checks if a byte can be part of an identifier.
fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Checks if a name is a common keyword to skip.
fn is_keyword(s: &str) -> bool {
    matches!(
        s,
        "if" | "else"
            | "for"
            | "while"
            | "match"
            | "fn"
            | "let"
            | "mut"
            | "pub"
            | "use"
            | "mod"
            | "impl"
            | "struct"
            | "enum"
            | "trait"
            | "type"
            | "const"
            | "static"
            | "return"
            | "break"
            | "continue"
            | "loop"
            | "async"
            | "await"
            | "move"
            | "ref"
            | "self"
            | "Self"
            | "super"
            | "crate"
            | "where"
            | "as"
            | "in"
            | "true"
            | "false"
    )
}

/// Checks if a path looks like a valid file path.
fn is_valid_path(s: &str) -> bool {
    if s.is_empty() || s.len() > 500 {
        return false;
    }
    // Must contain at least one path separator or just be a filename
    // Reject paths that are just extensions
    s.chars().any(|c| c.is_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    // =============================================================================
    // CodeReference type tests
    // =============================================================================

    #[test]
    fn test_code_reference_function_creation() {
        let r = CodeReference::function("process_data");
        assert_eq!(r.as_function_name(), Some("process_data"));
        assert_eq!(r.as_file_path(), None);
    }

    #[test]
    fn test_code_reference_file_creation() {
        let r = CodeReference::file("src/main.rs");
        assert_eq!(r.as_file_path(), Some("src/main.rs"));
        assert_eq!(r.as_function_name(), None);
    }

    #[test]
    fn test_code_reference_file_with_line() {
        let r = CodeReference::file_with_line("src/lib.rs", 42);
        match r {
            CodeReference::File { path, line } => {
                assert_eq!(path, "src/lib.rs");
                assert_eq!(line, Some(LineRef::Single(42)));
            }
            _ => panic!("Expected file reference"),
        }
    }

    #[test]
    fn test_code_reference_file_with_range() {
        let r = CodeReference::file_with_range("src/lib.rs", 100, 150);
        match r {
            CodeReference::File { path, line } => {
                assert_eq!(path, "src/lib.rs");
                assert_eq!(line, Some(LineRef::Range(100, 150)));
            }
            _ => panic!("Expected file reference"),
        }
    }

    // =============================================================================
    // Task 2.2.1 - extract_code_references function tests
    // =============================================================================

    #[test]
    fn test_extract_code_references_function_backticks() {
        // Should extract function names from backticks
        let refs = extract_code_references("Look at the `process_data` function");

        assert!(
            refs.iter()
                .any(|r| r.as_function_name() == Some("process_data")),
            "Should find backtick-quoted function: {:?}",
            refs
        );
    }

    #[test]
    fn test_extract_code_references_function_call() {
        // Should extract function call patterns
        let refs = extract_code_references("Call process_data() to handle the request");

        assert!(
            refs.iter()
                .any(|r| r.as_function_name() == Some("process_data")),
            "Should find function call: {:?}",
            refs
        );
    }

    #[test]
    fn test_extract_code_references_multiple_functions() {
        let refs = extract_code_references("Both `foo` and bar() are important");

        assert!(refs.iter().any(|r| r.as_function_name() == Some("foo")));
        assert!(refs.iter().any(|r| r.as_function_name() == Some("bar")));
    }

    #[test]
    fn test_extract_code_references_file() {
        // Should extract file paths
        let refs = extract_code_references("Check the code in src/handler.rs");

        assert!(
            refs.iter()
                .any(|r| r.as_file_path() == Some("src/handler.rs")),
            "Should find file path: {:?}",
            refs
        );
    }

    #[test]
    fn test_extract_code_references_file_with_line() {
        // Should extract file:line references
        let refs = extract_code_references("The bug is at src/main.rs:42");

        let file_ref = refs
            .iter()
            .find(|r| r.as_file_path() == Some("src/main.rs"))
            .expect("Should find file reference");

        match file_ref {
            CodeReference::File { line, .. } => {
                assert_eq!(*line, Some(LineRef::Single(42)));
            }
            _ => panic!("Expected file reference"),
        }
    }

    #[test]
    fn test_extract_code_references_file_with_range() {
        // Should extract file:start-end references
        let refs = extract_code_references("See src/lib.rs:100-150 for details");

        let file_ref = refs
            .iter()
            .find(|r| r.as_file_path() == Some("src/lib.rs"))
            .expect("Should find file reference");

        match file_ref {
            CodeReference::File { line, .. } => {
                assert_eq!(*line, Some(LineRef::Range(100, 150)));
            }
            _ => panic!("Expected file reference"),
        }
    }

    #[test]
    fn test_extract_code_references_mixed() {
        // Should handle mixed references
        let refs = extract_code_references(
            "The `handle_request` function in src/api/handler.rs:50 needs review",
        );

        assert!(
            refs.iter()
                .any(|r| r.as_function_name() == Some("handle_request")),
            "Should find function"
        );
        assert!(
            refs.iter()
                .any(|r| r.as_file_path() == Some("src/api/handler.rs")),
            "Should find file"
        );
    }

    #[test]
    fn test_extract_code_references_no_duplicates() {
        // Should not produce duplicate references
        let refs = extract_code_references("`foo` and `foo` and foo()");

        let foo_count = refs
            .iter()
            .filter(|r| r.as_function_name() == Some("foo"))
            .count();
        assert_eq!(foo_count, 1, "Should deduplicate function references");
    }

    #[test]
    fn test_extract_code_references_empty_text() {
        let refs = extract_code_references("");
        assert!(refs.is_empty());
    }

    #[test]
    fn test_extract_code_references_no_references() {
        let refs = extract_code_references("This is just plain text with no code references.");
        assert!(refs.is_empty());
    }

    #[test]
    fn test_extract_code_references_skips_keywords() {
        // Should not treat language keywords as function references
        let refs = extract_code_references("if() and for() are not functions");

        assert!(
            refs.iter().all(|r| r.as_function_name() != Some("if")),
            "Should skip 'if' keyword"
        );
        assert!(
            refs.iter().all(|r| r.as_function_name() != Some("for")),
            "Should skip 'for' keyword"
        );
    }

    #[test]
    fn test_extract_code_references_various_extensions() {
        let refs =
            extract_code_references("Check main.py, handler.js, config.toml, and schema.json");

        assert!(refs.iter().any(|r| r.as_file_path() == Some("main.py")));
        assert!(refs.iter().any(|r| r.as_file_path() == Some("handler.js")));
        assert!(refs.iter().any(|r| r.as_file_path() == Some("config.toml")));
        assert!(refs.iter().any(|r| r.as_file_path() == Some("schema.json")));
    }

    // =============================================================================
    // ContextSuggestion and ContextKind tests
    // =============================================================================

    #[test]
    fn test_context_kind_variants() {
        // Verify all variants exist
        let _callers = ContextKind::Callers;
        let _callees = ContextKind::Callees;
        let _imports = ContextKind::Imports;
        let _imported_by = ContextKind::ImportedBy;
        let _definition = ContextKind::Definition;
    }

    #[test]
    fn test_context_suggestion_creation() {
        let suggestion = ContextSuggestion {
            source: CodeReference::function("process_data"),
            kind: ContextKind::Callers,
            description: "Functions that call process_data".to_string(),
            content: "main() -> process_data()\nhandle_request() -> process_data()".to_string(),
        };

        assert_eq!(suggestion.source.as_function_name(), Some("process_data"));
        assert_eq!(suggestion.kind, ContextKind::Callers);
    }

    // =============================================================================
    // Helper function tests
    // =============================================================================

    #[test]
    fn test_is_valid_function_name() {
        assert!(is_valid_function_name("foo"));
        assert!(is_valid_function_name("process_data"));
        assert!(is_valid_function_name("handleRequest"));
        assert!(is_valid_function_name("_private"));
        assert!(is_valid_function_name("MyClass"));

        assert!(!is_valid_function_name(""));
        assert!(!is_valid_function_name("123"));
        assert!(!is_valid_function_name("foo-bar"));
        assert!(!is_valid_function_name("foo.bar"));
    }

    #[test]
    fn test_parse_line_ref_single() {
        assert_eq!(parse_line_ref("42"), Some(LineRef::Single(42)));
        assert_eq!(parse_line_ref("1"), Some(LineRef::Single(1)));
        assert_eq!(parse_line_ref("999"), Some(LineRef::Single(999)));
    }

    #[test]
    fn test_parse_line_ref_range() {
        assert_eq!(parse_line_ref("10-20"), Some(LineRef::Range(10, 20)));
        assert_eq!(parse_line_ref("100-150"), Some(LineRef::Range(100, 150)));
    }

    #[test]
    fn test_parse_line_ref_invalid() {
        assert_eq!(parse_line_ref("abc"), None);
        assert_eq!(parse_line_ref(""), None);
        assert_eq!(parse_line_ref("10-abc"), None);
    }
}
