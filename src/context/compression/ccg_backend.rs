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
use crate::mcp::connection::McpConnection;
use std::fmt;

/// Default SPARQL result limit to prevent context flooding.
pub const DEFAULT_SPARQL_LIMIT: usize = 100;

/// Error type for CCG response parsing failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CcgResponseError {
    /// Response is missing the `content` field.
    MissingContent,
    /// Response has an empty `content` array.
    EmptyContent,
    /// Response content has an invalid format.
    InvalidFormat(String),
}

impl CcgResponseError {
    /// Creates a MissingContent error.
    #[must_use]
    pub fn missing_content() -> Self {
        Self::MissingContent
    }

    /// Creates an EmptyContent error.
    #[must_use]
    pub fn empty_content() -> Self {
        Self::EmptyContent
    }

    /// Creates an InvalidFormat error with details.
    #[must_use]
    pub fn invalid_format(details: String) -> Self {
        Self::InvalidFormat(details)
    }
}

impl fmt::Display for CcgResponseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingContent => write!(f, "MCP response missing 'content' field"),
            Self::EmptyContent => write!(f, "MCP response has empty 'content' array"),
            Self::InvalidFormat(details) => write!(f, "Invalid MCP response format: {}", details),
        }
    }
}

impl std::error::Error for CcgResponseError {}

/// Parses an MCP tool response and extracts the text content.
///
/// MCP tool responses have a standard structure:
/// ```json
/// {
///   "content": [
///     { "type": "text", "text": "..." }
///   ]
/// }
/// ```
///
/// This function extracts the text from the first content block.
///
/// # Errors
///
/// Returns `CcgResponseError` if:
/// - The `content` field is missing
/// - The `content` array is empty
/// - The content block doesn't have a `text` field
pub fn parse_mcp_tool_response(response: &serde_json::Value) -> Result<String, CcgResponseError> {
    let content = response
        .get("content")
        .ok_or(CcgResponseError::MissingContent)?;

    let content_array = content
        .as_array()
        .ok_or_else(|| CcgResponseError::InvalidFormat("content is not an array".to_string()))?;

    if content_array.is_empty() {
        return Err(CcgResponseError::EmptyContent);
    }

    // Extract text from the first content block
    let first_block = &content_array[0];

    first_block
        .get("text")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            CcgResponseError::InvalidFormat("content block missing 'text' field".to_string())
        })
}

/// CCG namespace URI as defined in CCG Spec v0.2 Section 3.
///
/// This is the canonical namespace for Code Context Graph ontology terms.
pub const CCG_NAMESPACE: &str = "https://codecontextgraph.com/ontology/v1#";

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
            r#"PREFIX ccg: <{CCG_NAMESPACE}>
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
            r#"PREFIX ccg: <{CCG_NAMESPACE}>
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
            r#"PREFIX ccg: <{CCG_NAMESPACE}>
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

    // =========================================================================
    // Async MCP fetch methods (R.3.2)
    // =========================================================================

    /// Fetches the CCG manifest from narsil-mcp.
    ///
    /// Calls the `get_ccg_manifest` tool and parses the response.
    ///
    /// # Arguments
    ///
    /// * `client` - An active MCP client connected to narsil-mcp
    ///
    /// # Returns
    ///
    /// A `CompressionResult` containing the manifest JSON-LD content.
    ///
    /// # Errors
    ///
    /// Returns an error if the MCP call fails or the response cannot be parsed.
    pub async fn fetch_manifest(
        &self,
        client: &McpConnection,
    ) -> Result<CompressionResult, CcgFetchError> {
        let args = self.manifest_args();
        let response = client
            .call_tool_json("get_ccg_manifest", args)
            .await
            .map_err(CcgFetchError::mcp_error)?;

        let content =
            parse_mcp_tool_response(&response).map_err(CcgFetchError::from_response_error)?;

        Ok(self.create_manifest_result(content))
    }

    /// Fetches the CCG architecture from narsil-mcp.
    ///
    /// Calls the `export_ccg_architecture` tool and parses the response.
    ///
    /// # Arguments
    ///
    /// * `client` - An active MCP client connected to narsil-mcp
    ///
    /// # Returns
    ///
    /// A `CompressionResult` containing the architecture JSON-LD content.
    ///
    /// # Errors
    ///
    /// Returns an error if the MCP call fails or the response cannot be parsed.
    pub async fn fetch_architecture(
        &self,
        client: &McpConnection,
    ) -> Result<CompressionResult, CcgFetchError> {
        let args = self.architecture_args();
        let response = client
            .call_tool_json("export_ccg_architecture", args)
            .await
            .map_err(CcgFetchError::mcp_error)?;

        let content =
            parse_mcp_tool_response(&response).map_err(CcgFetchError::from_response_error)?;

        Ok(self.create_architecture_result(content))
    }

    /// Executes a SPARQL query against the CCG.
    ///
    /// Calls the `query_ccg` tool with the provided SPARQL query.
    ///
    /// # Arguments
    ///
    /// * `client` - An active MCP client connected to narsil-mcp
    /// * `sparql` - The SPARQL query to execute (should include LIMIT clause)
    ///
    /// # Returns
    ///
    /// A `CompressionResult` containing the query results.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The SPARQL query lacks a LIMIT clause
    /// - The MCP call fails
    /// - The response cannot be parsed
    pub async fn execute_query(
        &self,
        client: &McpConnection,
        sparql: &str,
    ) -> Result<CompressionResult, CcgFetchError> {
        // Safety: ensure LIMIT clause is present
        if !has_limit_clause(sparql) {
            return Err(CcgFetchError::missing_limit());
        }

        let args = self.query_args(sparql);
        let response = client
            .call_tool_json("query_ccg", args)
            .await
            .map_err(CcgFetchError::mcp_error)?;

        let content =
            parse_mcp_tool_response(&response).map_err(CcgFetchError::from_response_error)?;

        Ok(self.create_symbol_result(content))
    }
}

/// Error type for CCG fetch operations.
#[derive(Debug)]
pub enum CcgFetchError {
    /// MCP tool call failed.
    McpError(anyhow::Error),
    /// MCP response could not be parsed.
    ResponseError(CcgResponseError),
    /// SPARQL query missing LIMIT clause.
    MissingLimit,
}

impl CcgFetchError {
    /// Creates an error from an MCP call failure.
    pub fn mcp_error(err: anyhow::Error) -> Self {
        Self::McpError(err)
    }

    /// Creates an error from a response parsing failure.
    pub fn from_response_error(err: CcgResponseError) -> Self {
        Self::ResponseError(err)
    }

    /// Creates a missing LIMIT clause error.
    #[must_use]
    pub fn missing_limit() -> Self {
        Self::MissingLimit
    }

    /// Returns true if this is an MCP connection/call error.
    #[must_use]
    pub fn is_mcp_error(&self) -> bool {
        matches!(self, Self::McpError(_))
    }

    /// Returns true if this is a response parsing error.
    #[must_use]
    pub fn is_response_error(&self) -> bool {
        matches!(self, Self::ResponseError(_))
    }
}

impl fmt::Display for CcgFetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::McpError(e) => write!(f, "MCP tool call failed: {}", e),
            Self::ResponseError(e) => write!(f, "Failed to parse CCG response: {}", e),
            Self::MissingLimit => write!(f, "SPARQL query must include a LIMIT clause"),
        }
    }
}

impl std::error::Error for CcgFetchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::McpError(e) => Some(e.as_ref()),
            Self::ResponseError(e) => Some(e),
            Self::MissingLimit => None,
        }
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

    #[test]
    fn test_sparql_uses_spec_namespace() {
        // CCG Spec v0.2 Section 3 defines the canonical namespace URI
        const SPEC_NAMESPACE: &str = "https://codecontextgraph.com/ontology/v1#";

        let backend = CcgBackend::new("repo");

        let scope_query = backend.build_scope_query("test");
        assert!(
            scope_query.contains(SPEC_NAMESPACE),
            "Scope query should use CCG spec namespace. Got:\n{}",
            scope_query
        );

        let callers_query = backend.build_callers_query("func");
        assert!(
            callers_query.contains(SPEC_NAMESPACE),
            "Callers query should use CCG spec namespace. Got:\n{}",
            callers_query
        );

        let security_query = backend.build_security_query("HIGH");
        assert!(
            security_query.contains(SPEC_NAMESPACE),
            "Security query should use CCG spec namespace. Got:\n{}",
            security_query
        );
    }

    #[test]
    fn test_sparql_queries_do_not_use_placeholder_namespace() {
        // Ensure we don't use the example.org placeholder
        let backend = CcgBackend::new("repo");

        let scope_query = backend.build_scope_query("test");
        assert!(
            !scope_query.contains("example.org"),
            "Scope query should not use placeholder namespace"
        );

        let callers_query = backend.build_callers_query("func");
        assert!(
            !callers_query.contains("example.org"),
            "Callers query should not use placeholder namespace"
        );

        let security_query = backend.build_security_query("HIGH");
        assert!(
            !security_query.contains("example.org"),
            "Security query should not use placeholder namespace"
        );
    }

    #[test]
    fn test_ccg_namespace_constant_matches_spec() {
        // Verify CCG namespace constant matches CCG Spec v0.2 Section 3
        assert_eq!(
            CCG_NAMESPACE, "https://codecontextgraph.com/ontology/v1#",
            "CCG namespace should match spec"
        );
    }

    // =============================================================================
    // R.3.2 - Async MCP fetch method tests (TDD RED phase)
    // =============================================================================

    #[test]
    fn test_parse_manifest_response_valid_json() {
        let response = serde_json::json!({
            "content": [{
                "type": "text",
                "text": r#"{"@context": "https://codecontextgraph.com/ontology/v1#", "repository": "test-repo", "languages": ["Rust"]}"#
            }]
        });

        let result = parse_mcp_tool_response(&response);
        assert!(result.is_ok(), "Should parse valid MCP response");
        assert!(result.unwrap().contains("test-repo"));
    }

    #[test]
    fn test_parse_manifest_response_missing_content() {
        let response = serde_json::json!({});

        let result = parse_mcp_tool_response(&response);
        assert!(result.is_err(), "Should fail on missing content field");
    }

    #[test]
    fn test_parse_manifest_response_empty_content() {
        let response = serde_json::json!({
            "content": []
        });

        let result = parse_mcp_tool_response(&response);
        assert!(result.is_err(), "Should fail on empty content array");
    }

    #[test]
    fn test_ccg_response_error_contains_details() {
        let err = CcgResponseError::missing_content();
        assert!(err.to_string().contains("missing"));

        let err = CcgResponseError::empty_content();
        assert!(err.to_string().contains("empty"));

        let err = CcgResponseError::invalid_format("expected text".to_string());
        assert!(err.to_string().contains("expected text"));
    }

    #[test]
    fn test_ccg_fetch_error_types() {
        let mcp_err = CcgFetchError::mcp_error(anyhow::anyhow!("connection failed"));
        assert!(mcp_err.is_mcp_error());
        assert!(!mcp_err.is_response_error());
        assert!(mcp_err.to_string().contains("connection failed"));

        let response_err = CcgFetchError::from_response_error(CcgResponseError::missing_content());
        assert!(!response_err.is_mcp_error());
        assert!(response_err.is_response_error());

        let limit_err = CcgFetchError::missing_limit();
        assert!(!limit_err.is_mcp_error());
        assert!(!limit_err.is_response_error());
        assert!(limit_err.to_string().contains("LIMIT"));
    }

    #[test]
    fn test_parse_response_extracts_text_from_array() {
        let response = serde_json::json!({
            "content": [
                {"type": "text", "text": "first content"},
                {"type": "text", "text": "second content"}
            ]
        });

        let result = parse_mcp_tool_response(&response);
        assert!(result.is_ok());
        // Should extract the first text block
        assert_eq!(result.unwrap(), "first content");
    }

    #[test]
    fn test_parse_response_handles_non_array_content() {
        let response = serde_json::json!({
            "content": "not an array"
        });

        let result = parse_mcp_tool_response(&response);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, CcgResponseError::InvalidFormat(_)));
    }
}
