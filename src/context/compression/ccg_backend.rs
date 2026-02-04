//! CCG (Code Comprehension Graph) backend for compression.
//!
//! Uses CCG-specific narsil tools when the `--graph` flag is enabled:
//! - `get_ccg_manifest`: Layer 0 metadata (~2KB)
//! - `export_ccg_architecture`: Layer 1 architecture (~10-50KB)
//! - `query_ccg`: SPARQL queries for symbol details
//!
//! # SPARQL Safety
//!
//! All SPARQL queries include `LIMIT 100` to prevent context flooding.
//! The CCG tools require narsil-mcp to be started with `--graph` flag.

use crate::context::compression::{CompressionLevel, CompressionResult, ContextSource};

/// Default SPARQL result limit to prevent context flooding.
pub const DEFAULT_SPARQL_LIMIT: usize = 100;

/// CCG backend for accessing Code Comprehension Graph data.
///
/// Wraps CCG-specific MCP tool calls and ensures SPARQL safety.
#[derive(Debug, Clone)]
pub struct CcgBackend {
    /// Repository name for MCP calls
    repo_name: String,
    /// Maximum results per SPARQL query
    sparql_limit: usize,
}

impl CcgBackend {
    /// Creates a new CCG backend.
    #[must_use]
    pub fn new(repo_name: impl Into<String>) -> Self {
        Self {
            repo_name: repo_name.into(),
            sparql_limit: DEFAULT_SPARQL_LIMIT,
        }
    }

    /// Creates a CCG backend with a custom SPARQL limit.
    #[must_use]
    pub fn with_limit(repo_name: impl Into<String>, limit: usize) -> Self {
        Self {
            repo_name: repo_name.into(),
            sparql_limit: limit,
        }
    }

    /// Returns the repository name.
    #[must_use]
    pub fn repo_name(&self) -> &str {
        &self.repo_name
    }

    /// Returns the SPARQL limit.
    #[must_use]
    pub fn sparql_limit(&self) -> usize {
        self.sparql_limit
    }

    /// Builds the MCP tool call arguments for `get_ccg_manifest`.
    #[must_use]
    pub fn manifest_args(&self) -> serde_json::Value {
        serde_json::json!({
            "repo": self.repo_name
        })
    }

    /// Builds the MCP tool call arguments for `export_ccg_architecture`.
    #[must_use]
    pub fn architecture_args(&self) -> serde_json::Value {
        serde_json::json!({
            "repo": self.repo_name
        })
    }

    /// Builds the MCP tool call arguments for `query_ccg` with a SPARQL query.
    ///
    /// # Important
    ///
    /// The SPARQL query MUST include a LIMIT clause. This method does NOT
    /// automatically add a limit - use the helper methods or ensure your
    /// query includes `LIMIT`.
    #[must_use]
    pub fn query_args(&self, sparql: &str) -> serde_json::Value {
        serde_json::json!({
            "repo": self.repo_name,
            "query": sparql
        })
    }

    /// Builds a scoped SPARQL query with the configured limit.
    ///
    /// The query retrieves symbols within a specific scope (file or module).
    #[must_use]
    pub fn build_scope_query(&self, scope: &str) -> String {
        format!(
            r#"PREFIX ccg: <http://example.org/ccg#>
SELECT ?symbol ?kind ?file ?line
WHERE {{
    ?symbol ccg:inScope "{scope}" ;
            ccg:kind ?kind ;
            ccg:file ?file ;
            ccg:line ?line .
}}
LIMIT {}"#,
            self.sparql_limit
        )
    }

    /// Builds a callers SPARQL query with the configured limit.
    ///
    /// The query retrieves functions that call the specified symbol.
    #[must_use]
    pub fn build_callers_query(&self, symbol: &str) -> String {
        format!(
            r#"PREFIX ccg: <http://example.org/ccg#>
SELECT ?caller ?file ?line
WHERE {{
    ?caller ccg:calls <{symbol}> ;
            ccg:file ?file ;
            ccg:line ?line .
}}
LIMIT {}"#,
            self.sparql_limit
        )
    }

    /// Builds a security findings SPARQL query with severity filter.
    #[must_use]
    pub fn build_security_query(&self, min_severity: &str) -> String {
        format!(
            r#"PREFIX ccg: <http://example.org/ccg#>
SELECT ?finding ?severity ?rule ?file ?line
WHERE {{
    ?finding a ccg:SecurityFinding ;
             ccg:severity ?severity ;
             ccg:rule ?rule ;
             ccg:file ?file ;
             ccg:line ?line .
    FILTER (?severity IN ("{min_severity}", "CRITICAL", "HIGH"))
}}
LIMIT {}"#,
            self.sparql_limit
        )
    }

    /// Creates a manifest compression result from CCG response.
    #[must_use]
    pub fn create_manifest_result(&self, content: String) -> CompressionResult {
        CompressionResult::new(
            content,
            CompressionLevel::Manifest,
            ContextSource::CcgLayer0,
        )
    }

    /// Creates an architecture compression result from CCG response.
    #[must_use]
    pub fn create_architecture_result(&self, content: String) -> CompressionResult {
        CompressionResult::new(
            content,
            CompressionLevel::Architecture,
            ContextSource::CcgLayer1,
        )
    }

    /// Creates a symbol detail compression result from SPARQL response.
    #[must_use]
    pub fn create_symbol_result(&self, content: String) -> CompressionResult {
        CompressionResult::new(
            content,
            CompressionLevel::SymbolDetail,
            ContextSource::CcgSparql,
        )
    }
}

/// Validates that a SPARQL query includes a LIMIT clause.
#[must_use]
pub fn has_limit_clause(query: &str) -> bool {
    let upper = query.to_uppercase();
    upper.contains("LIMIT ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ccg_backend_module_exists() {
        // Verify the module compiles and basic types are accessible
        let backend = CcgBackend::new("test-repo");
        assert_eq!(backend.repo_name(), "test-repo");
    }

    #[test]
    fn test_ccg_backend_with_limit() {
        let backend = CcgBackend::with_limit("repo", 50);
        assert_eq!(backend.sparql_limit(), 50);
    }

    #[test]
    fn test_manifest_args() {
        let backend = CcgBackend::new("my-repo");
        let args = backend.manifest_args();

        assert_eq!(args["repo"], "my-repo");
    }

    #[test]
    fn test_architecture_args() {
        let backend = CcgBackend::new("my-repo");
        let args = backend.architecture_args();

        assert_eq!(args["repo"], "my-repo");
    }

    #[test]
    fn test_query_args() {
        let backend = CcgBackend::new("my-repo");
        let sparql = "SELECT ?s WHERE { ?s ?p ?o } LIMIT 10";
        let args = backend.query_args(sparql);

        assert_eq!(args["repo"], "my-repo");
        assert_eq!(args["query"], sparql);
    }

    #[test]
    fn test_scope_query_includes_filter() {
        let backend = CcgBackend::new("repo");
        let query = backend.build_scope_query("src/main.rs");

        assert!(query.contains("src/main.rs"));
        assert!(query.contains("ccg:inScope"));
    }

    #[test]
    fn test_scope_query_includes_limit() {
        let backend = CcgBackend::new("repo");
        let query = backend.build_scope_query("src/main.rs");

        assert!(has_limit_clause(&query));
        assert!(query.contains("LIMIT 100"));
    }

    #[test]
    fn test_callers_query_targets_correct_symbol() {
        let backend = CcgBackend::new("repo");
        let query = backend.build_callers_query("my_function");

        assert!(query.contains("my_function"));
        assert!(query.contains("ccg:calls"));
    }

    #[test]
    fn test_security_query_filters_by_severity() {
        let backend = CcgBackend::new("repo");
        let query = backend.build_security_query("HIGH");

        assert!(query.contains("HIGH"));
        assert!(query.contains("CRITICAL"));
        assert!(query.contains("ccg:SecurityFinding"));
    }

    #[test]
    fn test_all_queries_include_limit_clause() {
        let backend = CcgBackend::new("repo");

        let scope_query = backend.build_scope_query("test");
        let callers_query = backend.build_callers_query("func");
        let security_query = backend.build_security_query("HIGH");

        assert!(
            has_limit_clause(&scope_query),
            "Scope query missing LIMIT: {}",
            scope_query
        );
        assert!(
            has_limit_clause(&callers_query),
            "Callers query missing LIMIT: {}",
            callers_query
        );
        assert!(
            has_limit_clause(&security_query),
            "Security query missing LIMIT: {}",
            security_query
        );
    }

    #[test]
    fn test_has_limit_clause_positive() {
        assert!(has_limit_clause("SELECT ?s WHERE { ?s ?p ?o } LIMIT 100"));
        assert!(has_limit_clause("SELECT ?s WHERE { ?s ?p ?o } limit 50"));
        assert!(has_limit_clause("LIMIT 10"));
    }

    #[test]
    fn test_has_limit_clause_negative() {
        assert!(!has_limit_clause("SELECT ?s WHERE { ?s ?p ?o }"));
        assert!(!has_limit_clause("SELECT limited FROM table"));
    }

    #[test]
    fn test_create_manifest_result() {
        let backend = CcgBackend::new("repo");
        let result = backend.create_manifest_result("# Manifest content".to_string());

        assert_eq!(result.level(), CompressionLevel::Manifest);
        assert_eq!(result.source(), ContextSource::CcgLayer0);
    }

    #[test]
    fn test_create_architecture_result() {
        let backend = CcgBackend::new("repo");
        let result = backend.create_architecture_result("# Architecture".to_string());

        assert_eq!(result.level(), CompressionLevel::Architecture);
        assert_eq!(result.source(), ContextSource::CcgLayer1);
    }

    #[test]
    fn test_create_symbol_result() {
        let backend = CcgBackend::new("repo");
        let result = backend.create_symbol_result("Symbol details".to_string());

        assert_eq!(result.level(), CompressionLevel::SymbolDetail);
        assert_eq!(result.source(), ContextSource::CcgSparql);
    }

    #[test]
    fn test_custom_limit_in_queries() {
        let backend = CcgBackend::with_limit("repo", 25);

        let query = backend.build_scope_query("test");
        assert!(query.contains("LIMIT 25"));

        let callers = backend.build_callers_query("func");
        assert!(callers.contains("LIMIT 25"));
    }
}
