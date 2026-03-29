//! Deferred tool loading via on-demand schema search.
//!
//! Instead of sending all MCP tool schemas upfront (which wastes context tokens),
//! tools can be registered as "deferred" with just a name and short description.
//! Their full schema is fetched on demand when the LLM calls `tool_search`.
//!
//! # Query Syntax
//!
//! - `select:Name1,Name2` — fetch exact tools by name
//! - `keyword1 keyword2` — keyword search ranked by relevance
//! - `+required keyword` — require a word in the tool name, rank by remaining terms
//!
//! # Examples
//!
//! ```
//! use patina::tools::tool_search::{ToolSearchRegistry, ToolSearchResult};
//!
//! let mut registry = ToolSearchRegistry::new();
//! registry.register_deferred(
//!     "my_tool".to_string(),
//!     "Does something useful".to_string(),
//!     serde_json::json!({"type": "object", "properties": {}}),
//! );
//!
//! let results = registry.search("my_tool", 5);
//! assert_eq!(results.len(), 1);
//! assert_eq!(results[0].name, "my_tool");
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::debug;

/// A deferred tool entry stored in the registry.
///
/// Contains the tool name, a short description for keyword matching,
/// and the full JSON schema that is returned on demand.
#[derive(Debug, Clone)]
struct DeferredToolEntry {
    /// Short human-readable description used for keyword matching.
    description: String,
    /// Full JSON schema for the tool (parameters, types, etc.).
    full_schema: serde_json::Value,
}

/// A single result from a tool search query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSearchResult {
    /// The tool name.
    pub name: String,
    /// Short description of the tool.
    pub description: String,
    /// Full JSON schema definition for the tool.
    pub full_schema: serde_json::Value,
}

/// Registry for deferred tools that supports on-demand schema retrieval.
///
/// Tools are registered with a name, short description, and full schema.
/// The LLM only sees tool names initially; full schemas are fetched via
/// the `search` method when needed.
#[derive(Debug, Clone, Default)]
pub struct ToolSearchRegistry {
    entries: HashMap<String, DeferredToolEntry>,
}

impl ToolSearchRegistry {
    /// Creates a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Registers a deferred tool with its name, description, and full schema.
    ///
    /// If a tool with the same name already exists, it is replaced.
    pub fn register_deferred(
        &mut self,
        name: String,
        description: String,
        full_schema: serde_json::Value,
    ) {
        debug!(tool_name = %name, "Registered deferred tool");
        self.entries.insert(
            name,
            DeferredToolEntry {
                description,
                full_schema,
            },
        );
    }

    /// Returns the number of deferred tools in the registry.
    #[must_use]
    pub fn deferred_count(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if a tool with the given name is registered as deferred.
    #[must_use]
    pub fn is_deferred(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    /// Searches for tools matching the given query.
    ///
    /// # Query Syntax
    ///
    /// - `select:Name1,Name2` — exact name lookup (comma-separated)
    /// - `+required keyword` — require `required` in the tool name, rank by remaining terms
    /// - `keyword1 keyword2` — free-text keyword search ranked by relevance
    ///
    /// Results are ordered by relevance score (descending) and capped at `max_results`.
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::tools::tool_search::ToolSearchRegistry;
    ///
    /// let mut reg = ToolSearchRegistry::new();
    /// reg.register_deferred(
    ///     "read_file".to_string(),
    ///     "Read a file from disk".to_string(),
    ///     serde_json::json!({}),
    /// );
    /// let results = reg.search("select:read_file", 5);
    /// assert_eq!(results.len(), 1);
    /// ```
    #[must_use]
    pub fn search(&self, query: &str, max_results: usize) -> Vec<ToolSearchResult> {
        let query = query.trim();
        if query.is_empty() {
            return Vec::new();
        }

        if let Some(names_str) = query.strip_prefix("select:") {
            return self.search_exact(names_str, max_results);
        }

        self.search_keywords(query, max_results)
    }

    /// Performs exact name matching for `select:Name1,Name2` queries.
    fn search_exact(&self, names_str: &str, max_results: usize) -> Vec<ToolSearchResult> {
        let names: Vec<&str> = names_str.split(',').map(str::trim).collect();

        let mut results = Vec::new();
        for name in names {
            if name.is_empty() {
                continue;
            }
            if let Some(entry) = self.entries.get(name) {
                results.push(ToolSearchResult {
                    name: name.to_string(),
                    description: entry.description.clone(),
                    full_schema: entry.full_schema.clone(),
                });
                if results.len() >= max_results {
                    break;
                }
            }
        }

        results
    }

    /// Performs keyword-based search with relevance ranking.
    ///
    /// Scoring rules:
    /// - Exact name match: 100 points
    /// - Name contains keyword: 10 points per keyword
    /// - Description contains keyword: 5 points per keyword
    /// - `+keyword` prefix requires that keyword in the tool name (filters out non-matches)
    fn search_keywords(&self, query: &str, max_results: usize) -> Vec<ToolSearchResult> {
        let tokens: Vec<&str> = query.split_whitespace().collect();
        if tokens.is_empty() {
            return Vec::new();
        }

        // Separate required (prefixed with +) from ranking keywords
        let mut required: Vec<&str> = Vec::new();
        let mut ranking: Vec<&str> = Vec::new();
        for token in &tokens {
            if let Some(req) = token.strip_prefix('+') {
                if !req.is_empty() {
                    required.push(req);
                }
            } else {
                ranking.push(token);
            }
        }

        let mut scored: Vec<(i64, &str, &DeferredToolEntry)> = self
            .entries
            .iter()
            .filter_map(|(name, entry)| {
                let name_lower = name.to_lowercase();

                // Check required keywords — all must appear in the name
                for req in &required {
                    if !name_lower.contains(&req.to_lowercase()) {
                        return None;
                    }
                }

                let mut score: i64 = 0;

                // Score ranking keywords
                let all_keywords: Vec<&str> = required
                    .iter()
                    .copied()
                    .chain(ranking.iter().copied())
                    .collect();

                for keyword in &all_keywords {
                    let kw_lower = keyword.to_lowercase();

                    // Exact full name match
                    if name_lower == kw_lower {
                        score += 100;
                    } else if name_lower.contains(&kw_lower) {
                        score += 10;
                    }

                    // Description match
                    if entry.description.to_lowercase().contains(&kw_lower) {
                        score += 5;
                    }
                }

                if score > 0 {
                    Some((score, name.as_str(), entry))
                } else {
                    None
                }
            })
            .collect();

        // Sort by score descending, then name ascending for stability
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));

        scored
            .into_iter()
            .take(max_results)
            .map(|(_, name, entry)| ToolSearchResult {
                name: name.to_string(),
                description: entry.description.clone(),
                full_schema: entry.full_schema.clone(),
            })
            .collect()
    }

    /// Returns the tool definition that describes `tool_search` to the LLM.
    ///
    /// This JSON value can be included in the list of available tools sent
    /// to the Claude API, enabling the LLM to call `tool_search` at runtime.
    #[must_use]
    pub fn tool_definition() -> serde_json::Value {
        serde_json::json!({
            "name": "tool_search",
            "description": "Fetches full schema definitions for deferred tools so they can be called. \
                Deferred tools appear by name in system messages. Until fetched, only the name is known \
                - there is no parameter schema, so the tool cannot be invoked. This tool takes a query, \
                matches it against the deferred tool list, and returns the matched tools' complete \
                JSONSchema definitions. Once a tool's schema appears in the result, it is callable.\n\n\
                Query forms:\n\
                - \"select:Read,Edit,Grep\" - fetch exact tools by name\n\
                - \"notebook jupyter\" - keyword search, up to max_results best matches\n\
                - \"+slack send\" - require \"slack\" in the name, rank by remaining terms",
            "input_schema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Query to find deferred tools. Use \"select:<tool_name>\" \
                            for direct selection, or keywords to search."
                    },
                    "max_results": {
                        "type": "number",
                        "description": "Maximum number of results to return (default: 5)",
                        "default": 5
                    }
                },
                "required": ["query"]
            }
        })
    }

    /// Executes a `tool_search` tool call with the given input.
    ///
    /// # Errors
    ///
    /// Returns an error if the `query` parameter is missing or not a string.
    pub fn execute(&self, input: &serde_json::Value) -> Result<String, ToolSearchError> {
        let query = input
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or(ToolSearchError::MissingQuery)?;

        let max_results = input
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map_or(5, |v| v as usize);

        let results = self.search(query, max_results);

        debug!(
            query = query,
            max_results = max_results,
            found = results.len(),
            "Tool search executed"
        );

        serde_json::to_string_pretty(&results)
            .map_err(|e| ToolSearchError::Serialization(e.to_string()))
    }
}

/// Errors that can occur during tool search execution.
#[derive(Debug, thiserror::Error)]
pub enum ToolSearchError {
    /// The required `query` parameter was missing or not a string.
    #[error("missing required parameter: query")]
    MissingQuery,

    /// Failed to serialize search results.
    #[error("failed to serialize results: {0}")]
    Serialization(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a registry pre-populated with test tools.
    fn test_registry() -> ToolSearchRegistry {
        let mut reg = ToolSearchRegistry::new();
        reg.register_deferred(
            "read_file".to_string(),
            "Read a file from the filesystem".to_string(),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "File path to read"}
                },
                "required": ["path"]
            }),
        );
        reg.register_deferred(
            "write_file".to_string(),
            "Write content to a file".to_string(),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "content": {"type": "string"}
                },
                "required": ["path", "content"]
            }),
        );
        reg.register_deferred(
            "bash".to_string(),
            "Execute a bash command".to_string(),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string"}
                },
                "required": ["command"]
            }),
        );
        reg.register_deferred(
            "mcp__narsil__search_code".to_string(),
            "Search code using narsil semantic search".to_string(),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "repo": {"type": "string"}
                }
            }),
        );
        reg.register_deferred(
            "mcp__narsil__scan_security".to_string(),
            "Run security scan on indexed repository".to_string(),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "repo": {"type": "string"}
                }
            }),
        );
        reg
    }

    #[test]
    fn test_registration_and_deferred_count() {
        let reg = test_registry();
        assert_eq!(reg.deferred_count(), 5);
    }

    #[test]
    fn test_is_deferred() {
        let reg = test_registry();
        assert!(reg.is_deferred("read_file"));
        assert!(reg.is_deferred("bash"));
        assert!(!reg.is_deferred("nonexistent_tool"));
    }

    #[test]
    fn test_empty_registry() {
        let reg = ToolSearchRegistry::new();
        assert_eq!(reg.deferred_count(), 0);
        assert!(!reg.is_deferred("anything"));
    }

    #[test]
    fn test_select_exact_single() {
        let reg = test_registry();
        let results = reg.search("select:read_file", 5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "read_file");
        assert_eq!(results[0].description, "Read a file from the filesystem");
        assert!(results[0].full_schema.get("properties").is_some());
    }

    #[test]
    fn test_select_exact_multiple() {
        let reg = test_registry();
        let results = reg.search("select:read_file,write_file,bash", 10);
        assert_eq!(results.len(), 3);
        let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"write_file"));
        assert!(names.contains(&"bash"));
    }

    #[test]
    fn test_select_exact_nonexistent() {
        let reg = test_registry();
        let results = reg.search("select:does_not_exist", 5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_select_exact_partial_match() {
        let reg = test_registry();
        let results = reg.search("select:read_file,nonexistent,bash", 10);
        assert_eq!(results.len(), 2);
        let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"bash"));
    }

    #[test]
    fn test_select_respects_max_results() {
        let reg = test_registry();
        let results = reg.search("select:read_file,write_file,bash", 2);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_keyword_search_exact_name() {
        let reg = test_registry();
        let results = reg.search("bash", 5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "bash");
    }

    #[test]
    fn test_keyword_search_partial_name() {
        let reg = test_registry();
        let results = reg.search("file", 5);
        assert_eq!(results.len(), 2);
        let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"write_file"));
    }

    #[test]
    fn test_keyword_search_description() {
        let reg = test_registry();
        let results = reg.search("security", 5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "mcp__narsil__scan_security");
    }

    #[test]
    fn test_keyword_search_ranking_exact_over_partial() {
        let reg = test_registry();
        // "bash" should rank higher than tools that merely contain "bash" in description
        let results = reg.search("bash", 5);
        assert_eq!(results[0].name, "bash");
    }

    #[test]
    fn test_keyword_search_max_results() {
        let reg = test_registry();
        let results = reg.search("narsil", 1);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_required_keyword_filter() {
        let reg = test_registry();
        // +narsil requires "narsil" in the name
        let results = reg.search("+narsil search", 5);
        // Both narsil tools contain "narsil" in name, but "search" should boost search_code
        assert!(!results.is_empty());
        assert!(results
            .iter()
            .all(|r| r.name.to_lowercase().contains("narsil")));
        // search_code should rank first because it also matches "search" in name
        assert_eq!(results[0].name, "mcp__narsil__search_code");
    }

    #[test]
    fn test_required_keyword_filters_out_non_matches() {
        let reg = test_registry();
        let results = reg.search("+narsil bash", 5);
        // "bash" tool doesn't have "narsil" in its name, so it's excluded
        assert!(results
            .iter()
            .all(|r| r.name.to_lowercase().contains("narsil")));
    }

    #[test]
    fn test_empty_query_returns_empty() {
        let reg = test_registry();
        let results = reg.search("", 5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_whitespace_only_query_returns_empty() {
        let reg = test_registry();
        let results = reg.search("   ", 5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_no_matches_returns_empty() {
        let reg = test_registry();
        let results = reg.search("zzzznonexistent", 5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_special_characters_in_query() {
        let reg = test_registry();
        // Underscores, dots, etc. should not crash
        let results = reg.search("mcp__narsil", 5);
        assert_eq!(results.len(), 2);

        // Brackets and special chars should not panic
        let results = reg.search("(test)[brackets]{braces}", 5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_case_insensitive_search() {
        let reg = test_registry();
        let results = reg.search("BASH", 5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "bash");
    }

    #[test]
    fn test_case_insensitive_description_search() {
        let reg = test_registry();
        let results_lower = reg.search("filesystem", 5);
        let results_upper = reg.search("FILESYSTEM", 5);
        assert_eq!(results_lower.len(), results_upper.len());
    }

    #[test]
    fn test_select_with_spaces_around_names() {
        let reg = test_registry();
        let results = reg.search("select: read_file , write_file ", 5);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_select_with_empty_segments() {
        let reg = test_registry();
        let results = reg.search("select:read_file,,bash,", 5);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_replace_existing_deferred_tool() {
        let mut reg = ToolSearchRegistry::new();
        reg.register_deferred(
            "my_tool".to_string(),
            "Version 1".to_string(),
            serde_json::json!({"v": 1}),
        );
        reg.register_deferred(
            "my_tool".to_string(),
            "Version 2".to_string(),
            serde_json::json!({"v": 2}),
        );
        assert_eq!(reg.deferred_count(), 1);
        let results = reg.search("select:my_tool", 5);
        assert_eq!(results[0].description, "Version 2");
        assert_eq!(results[0].full_schema, serde_json::json!({"v": 2}));
    }

    #[test]
    fn test_execute_valid_input() {
        let reg = test_registry();
        let input = serde_json::json!({
            "query": "select:bash",
            "max_results": 5
        });
        let result = reg.execute(&input).unwrap();
        let parsed: Vec<ToolSearchResult> = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "bash");
    }

    #[test]
    fn test_execute_default_max_results() {
        let reg = test_registry();
        let input = serde_json::json!({
            "query": "file"
        });
        let result = reg.execute(&input).unwrap();
        let parsed: Vec<ToolSearchResult> = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn test_execute_missing_query() {
        let reg = test_registry();
        let input = serde_json::json!({});
        let err = reg.execute(&input).unwrap_err();
        assert!(matches!(err, ToolSearchError::MissingQuery));
    }

    #[test]
    fn test_execute_query_not_string() {
        let reg = test_registry();
        let input = serde_json::json!({"query": 42});
        let err = reg.execute(&input).unwrap_err();
        assert!(matches!(err, ToolSearchError::MissingQuery));
    }

    #[test]
    fn test_tool_definition_has_required_fields() {
        let def = ToolSearchRegistry::tool_definition();
        assert_eq!(def["name"], "tool_search");
        assert!(def.get("description").is_some());
        let schema = &def["input_schema"];
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"].get("query").is_some());
        assert!(schema["properties"].get("max_results").is_some());
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&serde_json::json!("query")));
    }

    #[test]
    fn test_default_impl() {
        let reg = ToolSearchRegistry::default();
        assert_eq!(reg.deferred_count(), 0);
    }

    #[test]
    fn test_result_contains_full_schema() {
        let reg = test_registry();
        let results = reg.search("select:read_file", 1);
        let schema = &results[0].full_schema;
        assert_eq!(schema["type"], "object");
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("path"));
    }

    #[test]
    fn test_multiple_keywords_combine_scores() {
        let reg = test_registry();
        // "narsil search" matches both narsil tools, but search_code
        // should score higher due to "search" in both name and description
        let results = reg.search("narsil search", 5);
        assert!(!results.is_empty());
        assert_eq!(results[0].name, "mcp__narsil__search_code");
    }

    #[test]
    fn test_zero_max_results() {
        let reg = test_registry();
        let results = reg.search("bash", 0);
        assert!(results.is_empty());
    }

    #[test]
    fn test_unicode_query() {
        let reg = test_registry();
        let results = reg.search("日本語クエリ", 5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_tool_search_error_display() {
        let err = ToolSearchError::MissingQuery;
        assert_eq!(err.to_string(), "missing required parameter: query");

        let err = ToolSearchError::Serialization("bad json".to_string());
        assert_eq!(err.to_string(), "failed to serialize results: bad json");
    }
}
