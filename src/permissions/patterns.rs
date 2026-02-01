//! Pattern matching for permission rules.
//!
//! This module provides glob-style pattern matching for tool names and inputs.
//!
//! # Supported Patterns
//!
//! - `*` - Matches any sequence of characters (but not empty)
//! - `?` - Matches any single character
//! - Exact match - Matches the literal string
//!
//! # Examples
//!
//! ```
//! use patina::permissions::patterns::matches_pattern;
//!
//! // Exact match
//! assert!(matches_pattern("Bash", "Bash"));
//! assert!(!matches_pattern("Bash", "Read"));
//!
//! // Wildcard suffix
//! assert!(matches_pattern("git *", "git status"));
//! assert!(matches_pattern("mcp__*", "mcp__jetbrains__build"));
//!
//! // Single character wildcard
//! assert!(matches_pattern("test?", "test1"));
//! assert!(!matches_pattern("test?", "test12"));
//! ```

use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashMap;
use std::sync::Mutex;

/// Cache for compiled regex patterns to avoid repeated compilation.
static PATTERN_CACHE: Lazy<Mutex<HashMap<String, Regex>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Maximum number of cached patterns to prevent unbounded memory growth.
const MAX_CACHE_SIZE: usize = 100;

/// Checks if a value matches a glob-style pattern.
///
/// # Arguments
///
/// * `pattern` - The pattern to match against (supports `*` and `?` wildcards)
/// * `value` - The value to check
///
/// # Returns
///
/// `true` if the value matches the pattern, `false` otherwise.
///
/// # Examples
///
/// ```
/// use patina::permissions::patterns::matches_pattern;
///
/// assert!(matches_pattern("Bash", "Bash"));
/// assert!(matches_pattern("git *", "git status"));
/// assert!(matches_pattern("mcp__*", "mcp__jetbrains__build"));
/// assert!(!matches_pattern("Read", "Bash"));
/// ```
#[must_use]
pub fn matches_pattern(pattern: &str, value: &str) -> bool {
    // Fast path: exact match
    if !pattern.contains('*') && !pattern.contains('?') {
        return pattern == value;
    }

    // Check cache for compiled regex
    let cache = PATTERN_CACHE.lock().unwrap();
    if let Some(regex) = cache.get(pattern) {
        return regex.is_match(value);
    }
    drop(cache); // Release lock before potentially slow regex compilation

    // Compile the pattern to regex
    let regex = match compile_pattern(pattern) {
        Ok(r) => r,
        Err(_) => return false, // Invalid pattern never matches
    };

    let matches = regex.is_match(value);

    // Cache the compiled regex
    let mut cache = PATTERN_CACHE.lock().unwrap();
    if cache.len() < MAX_CACHE_SIZE {
        cache.insert(pattern.to_string(), regex);
    }

    matches
}

/// Compiles a glob pattern to a regex.
///
/// # Arguments
///
/// * `pattern` - The glob pattern to compile
///
/// # Returns
///
/// A compiled regex that matches the pattern.
///
/// # Errors
///
/// Returns an error if the resulting regex is invalid.
fn compile_pattern(pattern: &str) -> Result<Regex, regex::Error> {
    let mut regex_str = String::with_capacity(pattern.len() * 2 + 2);
    regex_str.push('^');

    for c in pattern.chars() {
        match c {
            '*' => regex_str.push_str(".*"),
            '?' => regex_str.push('.'),
            // Escape regex special characters
            '.' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '^' | '$' | '\\' => {
                regex_str.push('\\');
                regex_str.push(c);
            }
            _ => regex_str.push(c),
        }
    }

    regex_str.push('$');
    Regex::new(&regex_str)
}

/// Extracts the command prefix from a bash command for pattern matching.
///
/// This is useful for matching bash commands by their primary command name.
///
/// # Examples
///
/// ```
/// use patina::permissions::patterns::extract_command_prefix;
///
/// assert_eq!(extract_command_prefix("git status"), Some("git"));
/// assert_eq!(extract_command_prefix("npm install lodash"), Some("npm"));
/// assert_eq!(extract_command_prefix("ls -la"), Some("ls"));
/// ```
#[must_use]
pub fn extract_command_prefix(command: &str) -> Option<&str> {
    command.split_whitespace().next()
}

/// Creates a pattern that matches a specific command and any arguments.
///
/// # Examples
///
/// ```
/// use patina::permissions::patterns::command_pattern;
///
/// assert_eq!(command_pattern("git"), "git *");
/// assert_eq!(command_pattern("npm"), "npm *");
/// ```
#[must_use]
pub fn command_pattern(command: &str) -> String {
    format!("{command} *")
}

/// Normalizes a tool input for consistent pattern matching.
///
/// This function:
/// - Trims whitespace
/// - Collapses multiple spaces to single space
///
/// # Examples
///
/// ```
/// use patina::permissions::patterns::normalize_input;
///
/// assert_eq!(normalize_input("  git   status  "), "git status");
/// ```
#[must_use]
pub fn normalize_input(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Common permission patterns for well-known tools.
pub mod common {
    /// Pattern for all git commands.
    pub const GIT_ALL: &str = "git *";

    /// Pattern for read-only git commands.
    pub const GIT_READONLY: &str = "git status|git log|git diff|git show|git branch";

    /// Pattern for npm commands.
    pub const NPM_ALL: &str = "npm *";

    /// Pattern for cargo commands.
    pub const CARGO_ALL: &str = "cargo *";

    /// Pattern for all MCP tools.
    pub const MCP_ALL: &str = "mcp__*";

    /// Pattern for JetBrains MCP tools.
    pub const MCP_JETBRAINS: &str = "mcp__jetbrains__*";

    /// Pattern for narsil MCP tools.
    pub const MCP_NARSIL: &str = "mcp__narsil__*";
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Basic pattern matching tests
    // =========================================================================

    #[test]
    fn test_exact_match() {
        assert!(matches_pattern("Bash", "Bash"));
        assert!(matches_pattern("Read", "Read"));
        assert!(!matches_pattern("Bash", "Read"));
        assert!(!matches_pattern("Read", "Bash"));
    }

    #[test]
    fn test_case_sensitive() {
        assert!(!matches_pattern("bash", "Bash"));
        assert!(!matches_pattern("Bash", "bash"));
        assert!(!matches_pattern("BASH", "Bash"));
    }

    #[test]
    fn test_empty_pattern_and_value() {
        assert!(matches_pattern("", ""));
        assert!(!matches_pattern("", "something"));
        assert!(!matches_pattern("something", ""));
    }

    // =========================================================================
    // Wildcard pattern tests
    // =========================================================================

    #[test]
    fn test_star_wildcard_suffix() {
        assert!(matches_pattern("git *", "git status"));
        assert!(matches_pattern("git *", "git commit -m 'message'"));
        assert!(matches_pattern("mcp__*", "mcp__jetbrains__build"));
        assert!(matches_pattern("mcp__*", "mcp__narsil__scan"));
    }

    #[test]
    fn test_star_wildcard_prefix() {
        assert!(matches_pattern("*.rs", "main.rs"));
        assert!(matches_pattern("*.rs", "lib.rs"));
        assert!(!matches_pattern("*.rs", "main.py"));
    }

    #[test]
    fn test_star_wildcard_middle() {
        assert!(matches_pattern("git*status", "gitstatus"));
        assert!(matches_pattern("git*status", "git status"));
        assert!(matches_pattern("git*status", "git --no-pager status"));
    }

    #[test]
    fn test_multiple_star_wildcards() {
        assert!(matches_pattern("*git*", "git"));
        assert!(matches_pattern("*git*", "mygitrepo"));
        assert!(matches_pattern("*git*", "this is git command"));
    }

    #[test]
    fn test_star_matches_empty() {
        // Star matches zero or more characters
        assert!(matches_pattern("git*", "git"));
        assert!(matches_pattern("*git", "git"));
    }

    // =========================================================================
    // Question mark wildcard tests
    // =========================================================================

    #[test]
    fn test_question_wildcard() {
        assert!(matches_pattern("test?", "test1"));
        assert!(matches_pattern("test?", "testa"));
        assert!(!matches_pattern("test?", "test"));
        assert!(!matches_pattern("test?", "test12"));
    }

    #[test]
    fn test_multiple_question_wildcards() {
        assert!(matches_pattern("t??t", "test"));
        assert!(matches_pattern("t??t", "toot"));
        assert!(!matches_pattern("t??t", "tt"));
        assert!(!matches_pattern("t??t", "toast"));
    }

    // =========================================================================
    // Special character escaping tests
    // =========================================================================

    #[test]
    fn test_special_chars_escaped() {
        // These regex special chars should be treated literally
        assert!(matches_pattern("file.txt", "file.txt"));
        assert!(!matches_pattern("file.txt", "filetxt")); // . should not match any char

        assert!(matches_pattern("path/to/file", "path/to/file"));
        assert!(matches_pattern("[test]", "[test]"));
        assert!(matches_pattern("a+b", "a+b"));
    }

    // =========================================================================
    // Real-world pattern tests
    // =========================================================================

    #[test]
    fn test_tool_patterns() {
        // MCP tools
        assert!(matches_pattern("mcp__*", "mcp__jetbrains__build_project"));
        assert!(matches_pattern(
            "mcp__jetbrains__*",
            "mcp__jetbrains__build_project"
        ));
        assert!(!matches_pattern("mcp__jetbrains__*", "mcp__narsil__scan"));

        // Bash commands
        assert!(matches_pattern("git *", "git status"));
        assert!(matches_pattern("npm *", "npm install lodash"));
        assert!(matches_pattern("cargo *", "cargo build --release"));
    }

    #[test]
    fn test_common_patterns() {
        assert!(matches_pattern(common::GIT_ALL, "git status"));
        assert!(matches_pattern(common::NPM_ALL, "npm install"));
        assert!(matches_pattern(common::CARGO_ALL, "cargo test"));
        assert!(matches_pattern(common::MCP_ALL, "mcp__jetbrains__build"));
        assert!(matches_pattern(
            common::MCP_JETBRAINS,
            "mcp__jetbrains__build"
        ));
        assert!(matches_pattern(common::MCP_NARSIL, "mcp__narsil__scan"));
    }

    // =========================================================================
    // Helper function tests
    // =========================================================================

    #[test]
    fn test_extract_command_prefix() {
        assert_eq!(extract_command_prefix("git status"), Some("git"));
        assert_eq!(extract_command_prefix("npm install lodash"), Some("npm"));
        assert_eq!(extract_command_prefix("ls"), Some("ls"));
        assert_eq!(extract_command_prefix(""), None);
        assert_eq!(extract_command_prefix("   "), None);
    }

    #[test]
    fn test_command_pattern() {
        assert_eq!(command_pattern("git"), "git *");
        assert_eq!(command_pattern("npm"), "npm *");
    }

    #[test]
    fn test_normalize_input() {
        assert_eq!(normalize_input("git status"), "git status");
        assert_eq!(normalize_input("  git   status  "), "git status");
        assert_eq!(normalize_input("git\tstatus"), "git status");
        assert_eq!(normalize_input(""), "");
    }

    // =========================================================================
    // Cache tests
    // =========================================================================

    #[test]
    fn test_pattern_caching() {
        // Call multiple times with same pattern - should use cache
        for _ in 0..10 {
            assert!(matches_pattern("test*pattern", "test123pattern"));
        }

        // Verify cache has the pattern
        let cache = PATTERN_CACHE.lock().unwrap();
        assert!(cache.contains_key("test*pattern"));
    }
}
