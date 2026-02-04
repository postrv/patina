//! Fallback backend for compression when CCG is unavailable.
//!
//! Uses core narsil tools that work without the `--graph` flag:
//! - `get_incremental_status`: Repository hash and statistics
//! - `find_symbols`: Symbol discovery and counts
//! - `get_dependencies`: Module dependency information
//! - `get_export_map`: Public API exports
//! - `get_symbol_definition`: Individual symbol details
//!
//! This backend produces similar output to the CCG backend but with
//! less structured data and potentially more API calls.

use crate::context::compression::{CompressionLevel, CompressionResult, ContextSource};

/// Default maximum symbols to return in fallback queries.
pub const DEFAULT_FALLBACK_LIMIT: usize = 100;

/// Fallback backend for compression without CCG.
///
/// Uses core narsil tools to construct compressed context.
#[derive(Debug, Clone)]
pub struct FallbackBackend {
    /// Repository name for MCP calls
    repo_name: String,
    /// Maximum symbols to return
    symbol_limit: usize,
}

impl FallbackBackend {
    /// Creates a new fallback backend.
    #[must_use]
    pub fn new(repo_name: impl Into<String>) -> Self {
        Self {
            repo_name: repo_name.into(),
            symbol_limit: DEFAULT_FALLBACK_LIMIT,
        }
    }

    /// Creates a fallback backend with a custom symbol limit.
    #[must_use]
    pub fn with_limit(repo_name: impl Into<String>, limit: usize) -> Self {
        Self {
            repo_name: repo_name.into(),
            symbol_limit: limit,
        }
    }

    /// Returns the repository name.
    #[must_use]
    pub fn repo_name(&self) -> &str {
        &self.repo_name
    }

    /// Returns the symbol limit.
    #[must_use]
    pub fn symbol_limit(&self) -> usize {
        self.symbol_limit
    }

    /// Builds arguments for `get_incremental_status` to get repository hash.
    #[must_use]
    pub fn status_args(&self) -> serde_json::Value {
        serde_json::json!({
            "repo": self.repo_name
        })
    }

    /// Builds arguments for `find_symbols` for manifest generation.
    ///
    /// Returns counts only for minimal overhead.
    #[must_use]
    pub fn symbols_count_args(&self) -> serde_json::Value {
        serde_json::json!({
            "repo": self.repo_name,
            "symbol_type": "all",
            "exclude_tests": true
        })
    }

    /// Builds arguments for `find_symbols` for architecture generation.
    #[must_use]
    pub fn symbols_args(&self, symbol_type: &str) -> serde_json::Value {
        serde_json::json!({
            "repo": self.repo_name,
            "symbol_type": symbol_type,
            "exclude_tests": true
        })
    }

    /// Builds arguments for `get_dependencies`.
    #[must_use]
    pub fn dependencies_args(&self, path: &str) -> serde_json::Value {
        serde_json::json!({
            "repo": self.repo_name,
            "path": path,
            "direction": "both"
        })
    }

    /// Builds arguments for `get_export_map`.
    #[must_use]
    pub fn export_map_args(&self, path: &str) -> serde_json::Value {
        serde_json::json!({
            "repo": self.repo_name,
            "path": path
        })
    }

    /// Builds arguments for `get_symbol_definition`.
    #[must_use]
    pub fn symbol_definition_args(&self, symbol: &str) -> serde_json::Value {
        serde_json::json!({
            "repo": self.repo_name,
            "symbol": symbol
        })
    }

    /// Creates a manifest compression result from fallback data.
    #[must_use]
    pub fn create_manifest_result(&self, content: String) -> CompressionResult {
        CompressionResult::new(content, CompressionLevel::Manifest, ContextSource::Constructed)
    }

    /// Creates an architecture compression result from fallback data.
    #[must_use]
    pub fn create_architecture_result(&self, content: String) -> CompressionResult {
        CompressionResult::new(
            content,
            CompressionLevel::Architecture,
            ContextSource::Constructed,
        )
    }

    /// Creates a symbol detail compression result from fallback data.
    #[must_use]
    pub fn create_symbol_result(&self, content: String) -> CompressionResult {
        CompressionResult::new(content, CompressionLevel::SymbolDetail, ContextSource::DirectTool)
    }
}

/// Extracts the Merkle hash from a `get_incremental_status` response.
///
/// Returns `None` if the hash is not present in the response.
#[must_use]
pub fn extract_repo_hash(response: &serde_json::Value) -> Option<String> {
    response
        .get("merkle_root")
        .or_else(|| response.get("root_hash"))
        .or_else(|| response.get("hash"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Extracts symbol counts from a `find_symbols` response.
#[must_use]
pub fn extract_symbol_counts(response: &serde_json::Value) -> SymbolCounts {
    let symbols = response
        .get("symbols")
        .and_then(|s| s.as_array())
        .map(|arr| arr.len())
        .unwrap_or(0);

    let functions = response
        .get("functions")
        .and_then(|f| f.as_array())
        .map(|arr| arr.len())
        .or_else(|| {
            // Count function symbols
            response.get("symbols").and_then(|s| s.as_array()).map(|arr| {
                arr.iter()
                    .filter(|s| {
                        s.get("kind")
                            .and_then(|k| k.as_str())
                            .is_some_and(|k| k == "function")
                    })
                    .count()
            })
        })
        .unwrap_or(0);

    let classes = response
        .get("classes")
        .and_then(|c| c.as_array())
        .map(|arr| arr.len())
        .or_else(|| {
            response.get("symbols").and_then(|s| s.as_array()).map(|arr| {
                arr.iter()
                    .filter(|s| {
                        s.get("kind")
                            .and_then(|k| k.as_str())
                            .is_some_and(|k| k == "class" || k == "struct")
                    })
                    .count()
            })
        })
        .unwrap_or(0);

    SymbolCounts {
        total: symbols,
        functions,
        classes,
    }
}

/// Summary of symbol counts from find_symbols.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SymbolCounts {
    /// Total symbols found
    pub total: usize,
    /// Number of functions
    pub functions: usize,
    /// Number of classes/structs
    pub classes: usize,
}

impl SymbolCounts {
    /// Creates empty symbol counts.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            total: 0,
            functions: 0,
            classes: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fallback_backend_module_exists() {
        let backend = FallbackBackend::new("test-repo");
        assert_eq!(backend.repo_name(), "test-repo");
    }

    #[test]
    fn test_fallback_backend_with_limit() {
        let backend = FallbackBackend::with_limit("repo", 50);
        assert_eq!(backend.symbol_limit(), 50);
    }

    #[test]
    fn test_status_args() {
        let backend = FallbackBackend::new("my-repo");
        let args = backend.status_args();

        assert_eq!(args["repo"], "my-repo");
    }

    #[test]
    fn test_symbols_count_args() {
        let backend = FallbackBackend::new("my-repo");
        let args = backend.symbols_count_args();

        assert_eq!(args["repo"], "my-repo");
        assert_eq!(args["symbol_type"], "all");
        assert_eq!(args["exclude_tests"], true);
    }

    #[test]
    fn test_symbols_args() {
        let backend = FallbackBackend::new("my-repo");
        let args = backend.symbols_args("function");

        assert_eq!(args["repo"], "my-repo");
        assert_eq!(args["symbol_type"], "function");
    }

    #[test]
    fn test_dependencies_args() {
        let backend = FallbackBackend::new("my-repo");
        let args = backend.dependencies_args("src/main.rs");

        assert_eq!(args["repo"], "my-repo");
        assert_eq!(args["path"], "src/main.rs");
        assert_eq!(args["direction"], "both");
    }

    #[test]
    fn test_export_map_args() {
        let backend = FallbackBackend::new("my-repo");
        let args = backend.export_map_args("src/lib.rs");

        assert_eq!(args["repo"], "my-repo");
        assert_eq!(args["path"], "src/lib.rs");
    }

    #[test]
    fn test_symbol_definition_args() {
        let backend = FallbackBackend::new("my-repo");
        let args = backend.symbol_definition_args("MyStruct");

        assert_eq!(args["repo"], "my-repo");
        assert_eq!(args["symbol"], "MyStruct");
    }

    #[test]
    fn test_create_manifest_result() {
        let backend = FallbackBackend::new("repo");
        let result = backend.create_manifest_result("# Manifest".to_string());

        assert_eq!(result.level(), CompressionLevel::Manifest);
        assert_eq!(result.source(), ContextSource::Constructed);
    }

    #[test]
    fn test_create_architecture_result() {
        let backend = FallbackBackend::new("repo");
        let result = backend.create_architecture_result("# Architecture".to_string());

        assert_eq!(result.level(), CompressionLevel::Architecture);
        assert_eq!(result.source(), ContextSource::Constructed);
    }

    #[test]
    fn test_create_symbol_result() {
        let backend = FallbackBackend::new("repo");
        let result = backend.create_symbol_result("Symbol details".to_string());

        assert_eq!(result.level(), CompressionLevel::SymbolDetail);
        assert_eq!(result.source(), ContextSource::DirectTool);
    }

    #[test]
    fn test_extract_repo_hash_merkle_root() {
        let response = serde_json::json!({
            "merkle_root": "abc123def456"
        });

        let hash = extract_repo_hash(&response);
        assert_eq!(hash, Some("abc123def456".to_string()));
    }

    #[test]
    fn test_extract_repo_hash_root_hash() {
        let response = serde_json::json!({
            "root_hash": "xyz789"
        });

        let hash = extract_repo_hash(&response);
        assert_eq!(hash, Some("xyz789".to_string()));
    }

    #[test]
    fn test_extract_repo_hash_missing() {
        let response = serde_json::json!({
            "other": "data"
        });

        let hash = extract_repo_hash(&response);
        assert!(hash.is_none());
    }

    #[test]
    fn test_extract_symbol_counts_with_symbols() {
        let response = serde_json::json!({
            "symbols": [
                {"name": "foo", "kind": "function"},
                {"name": "bar", "kind": "function"},
                {"name": "Baz", "kind": "class"}
            ]
        });

        let counts = extract_symbol_counts(&response);
        assert_eq!(counts.total, 3);
        assert_eq!(counts.functions, 2);
        assert_eq!(counts.classes, 1);
    }

    #[test]
    fn test_extract_symbol_counts_empty() {
        let response = serde_json::json!({});

        let counts = extract_symbol_counts(&response);
        assert_eq!(counts.total, 0);
        assert_eq!(counts.functions, 0);
        assert_eq!(counts.classes, 0);
    }

    #[test]
    fn test_symbol_counts_empty() {
        let counts = SymbolCounts::empty();
        assert_eq!(counts.total, 0);
        assert_eq!(counts.functions, 0);
        assert_eq!(counts.classes, 0);
    }
}
