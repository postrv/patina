//! NarsilIntegration - High-level wrapper for narsil-mcp code intelligence.
//!
//! Provides automatic capability detection and a unified interface for
//! interacting with narsil-mcp's code intelligence features.
//!
//! # Architecture
//!
//! `NarsilIntegration` wraps an MCP client connection to narsil-mcp and:
//! 1. Discovers available tools via MCP protocol
//! 2. Maps tools to high-level capabilities
//! 3. Provides ergonomic methods for common operations
//!
//! # Capabilities
//!
//! Capabilities are auto-detected by checking which tools are available:
//! - `CallGraph`: `get_call_graph`, `get_callers`, `get_callees`
//! - `SecurityScan`: `scan_security`, `check_owasp_top10`
//! - `CodeSearch`: `search_code`, `semantic_search`
//! - `SymbolAnalysis`: `find_symbols`, `get_symbol_definition`
//!
//! # Example
//!
//! ```ignore
//! use patina::narsil::{NarsilIntegration, NarsilCapability};
//! use patina::mcp::connection::McpConnection;
//! use std::time::Duration;
//!
//! // Connect using an MCP connection
//! let conn = McpConnection::connect_stdio(
//!     "narsil", "/path/to/narsil-mcp", &[], &Default::default(),
//!     Duration::from_secs(10),
//! ).await?;
//!
//! let integration = NarsilIntegration::from_mcp_client(&conn, Path::new(".")).await?;
//! if integration.has_capability(NarsilCapability::SecurityScan) {
//!     println!("Security scanning available!");
//! }
//! ```

use crate::context::compression::CompressionOrchestrator;
use crate::mcp::connection::McpConnection;
use crate::narsil::context::{CodeReference, ContextKind, ContextSuggestion};
use anyhow::Result;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Individual narsil-mcp capabilities that can be detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NarsilCapability {
    /// Call graph analysis (get_call_graph, get_callers, get_callees)
    CallGraph,
    /// Security scanning (scan_security, check_owasp_top10, check_cwe_top25)
    SecurityScan,
    /// Code search (search_code, semantic_search, hybrid_search)
    CodeSearch,
    /// Symbol analysis (find_symbols, get_symbol_definition, find_references)
    SymbolAnalysis,
    /// Git history integration (get_file_history, get_blame, get_recent_changes)
    GitHistory,
    /// Taint analysis (trace_taint, get_taint_sources, find_injection_vulnerabilities)
    TaintAnalysis,
    /// Dependency analysis (get_dependencies, check_dependencies, find_circular_imports)
    DependencyAnalysis,
    /// CCG (Code Comprehension Graph) access (get_ccg_manifest, export_ccg_architecture, query_ccg)
    /// Requires narsil-mcp to be started with --graph flag
    CcgGraph,
}

impl NarsilCapability {
    /// Returns the tool names that indicate this capability is available.
    #[must_use]
    pub fn required_tools(&self) -> &'static [&'static str] {
        match self {
            Self::CallGraph => &["get_call_graph", "get_callers", "get_callees"],
            Self::SecurityScan => &["scan_security", "check_owasp_top10", "check_cwe_top25"],
            Self::CodeSearch => &["search_code", "semantic_search"],
            Self::SymbolAnalysis => &["find_symbols", "get_symbol_definition", "find_references"],
            Self::GitHistory => &["get_file_history", "get_blame", "get_recent_changes"],
            Self::TaintAnalysis => &[
                "trace_taint",
                "get_taint_sources",
                "find_injection_vulnerabilities",
            ],
            Self::DependencyAnalysis => &[
                "get_dependencies",
                "check_dependencies",
                "find_circular_imports",
            ],
            Self::CcgGraph => &["get_ccg_manifest", "export_ccg_architecture", "query_ccg"],
        }
    }

    /// Returns all capability variants.
    #[must_use]
    pub fn all() -> &'static [NarsilCapability] {
        &[
            Self::CallGraph,
            Self::SecurityScan,
            Self::CodeSearch,
            Self::SymbolAnalysis,
            Self::GitHistory,
            Self::TaintAnalysis,
            Self::DependencyAnalysis,
            Self::CcgGraph,
        ]
    }
}

/// Collection of detected narsil capabilities.
#[derive(Debug, Clone, Default)]
pub struct NarsilCapabilities {
    capabilities: HashSet<NarsilCapability>,
    available_tools: HashSet<String>,
}

impl NarsilCapabilities {
    /// Creates a new empty capabilities set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates capabilities from a list of available tool names.
    ///
    /// Automatically detects which capabilities are available based on
    /// which tools are present.
    #[must_use]
    pub fn from_tools(tools: &[String]) -> Self {
        let available_tools: HashSet<String> = tools.iter().cloned().collect();
        let mut capabilities = HashSet::new();

        for capability in NarsilCapability::all() {
            // A capability is available if at least one of its required tools exists
            let has_any_tool = capability
                .required_tools()
                .iter()
                .any(|tool| available_tools.contains(*tool));

            if has_any_tool {
                capabilities.insert(*capability);
            }
        }

        Self {
            capabilities,
            available_tools,
        }
    }

    /// Returns true if the specified capability is available.
    #[must_use]
    pub fn has(&self, capability: NarsilCapability) -> bool {
        self.capabilities.contains(&capability)
    }

    /// Returns true if any capabilities are available.
    #[must_use]
    pub fn is_available(&self) -> bool {
        !self.capabilities.is_empty()
    }

    /// Returns all available capabilities.
    #[must_use]
    pub fn available(&self) -> Vec<NarsilCapability> {
        self.capabilities.iter().copied().collect()
    }

    /// Returns all available tool names.
    #[must_use]
    pub fn tools(&self) -> &HashSet<String> {
        &self.available_tools
    }

    /// Returns the number of available capabilities.
    #[must_use]
    pub fn count(&self) -> usize {
        self.capabilities.len()
    }
}

/// High-level integration with narsil-mcp for code intelligence.
///
/// Provides automatic capability detection and a unified interface for
/// interacting with narsil-mcp's features.
///
/// # Example
///
/// ```ignore
/// let integration = NarsilIntegration::new("/path/to/project").await?;
///
/// // Check available capabilities
/// if integration.has_capability(NarsilCapability::SecurityScan) {
///     // Use security scanning features
/// }
///
/// // Get all capabilities
/// for cap in integration.capabilities().available() {
///     println!("Available: {:?}", cap);
/// }
/// ```
#[derive(Debug)]
pub struct NarsilIntegration {
    /// Repository path being analyzed
    repo_path: PathBuf,
    /// Repository name (basename of repo_path)
    repo_name: String,
    /// Detected capabilities
    capabilities: NarsilCapabilities,
    /// Whether the integration is connected
    connected: bool,
}

impl NarsilIntegration {
    /// Creates a new NarsilIntegration for the specified repository.
    ///
    /// This constructor creates the integration in a disconnected state.
    /// Call `connect()` to establish the connection and detect capabilities.
    ///
    /// # Arguments
    ///
    /// * `repo_path` - Path to the repository to analyze
    ///
    /// # Example
    ///
    /// ```
    /// use patina::narsil::NarsilIntegration;
    /// use std::path::Path;
    ///
    /// let integration = NarsilIntegration::new(Path::new("/path/to/repo"));
    /// assert!(!integration.is_connected());
    /// ```
    #[must_use]
    pub fn new(repo_path: &Path) -> Self {
        let repo_name = repo_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        Self {
            repo_path: repo_path.to_path_buf(),
            repo_name,
            capabilities: NarsilCapabilities::new(),
            connected: false,
        }
    }

    /// Returns the repository path.
    #[must_use]
    pub fn repo_path(&self) -> &Path {
        &self.repo_path
    }

    /// Returns the repository name.
    #[must_use]
    pub fn repo_name(&self) -> &str {
        &self.repo_name
    }

    /// Returns the detected capabilities.
    #[must_use]
    pub fn capabilities(&self) -> &NarsilCapabilities {
        &self.capabilities
    }

    /// Returns true if the specified capability is available.
    #[must_use]
    pub fn has_capability(&self, capability: NarsilCapability) -> bool {
        self.capabilities.has(capability)
    }

    /// Returns true if connected to narsil-mcp.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// Creates a NarsilIntegration by connecting to an existing MCP connection.
    ///
    /// This method reads the connection's tool list and automatically
    /// detects which narsil capabilities are available.
    ///
    /// # Arguments
    ///
    /// * `client` - An active `McpConnection` connected to narsil-mcp
    /// * `repo_path` - Path to the repository being analyzed
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The MCP connection is not connected
    ///
    /// # Example
    ///
    /// ```ignore
    /// use patina::narsil::NarsilIntegration;
    /// use patina::mcp::connection::McpConnection;
    /// use std::path::Path;
    ///
    /// let conn = McpConnection::connect_stdio(
    ///     "narsil", "/path/to/narsil-mcp", &[], &Default::default(),
    ///     Duration::from_secs(10),
    /// ).await?;
    ///
    /// let integration = NarsilIntegration::from_mcp_client(&conn, Path::new(".")).await?;
    /// println!("Connected with {} capabilities", integration.capabilities().count());
    /// ```
    pub async fn from_mcp_client(client: &McpConnection, repo_path: &Path) -> Result<Self> {
        // Read available tools from the MCP connection
        let tools = client.tools();
        let tool_names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();

        // Detect capabilities from tool list
        let capabilities = NarsilCapabilities::from_tools(&tool_names);

        let repo_name = repo_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        Ok(Self {
            repo_path: repo_path.to_path_buf(),
            repo_name,
            capabilities,
            connected: true,
        })
    }

    /// Creates a NarsilIntegration from a pre-discovered list of tool names.
    ///
    /// This is useful when tools have already been discovered through another
    /// mechanism (e.g., app-level MCP management) and you want to create
    /// an integration without re-querying the server.
    ///
    /// # Arguments
    ///
    /// * `tool_names` - List of available tool names from narsil-mcp
    /// * `repo_path` - Path to the repository being analyzed
    ///
    /// # Example
    ///
    /// ```
    /// use patina::narsil::{NarsilIntegration, NarsilCapability};
    /// use std::path::Path;
    ///
    /// let tools = vec![
    ///     "scan_security".to_string(),
    ///     "get_call_graph".to_string(),
    /// ];
    /// let integration = NarsilIntegration::from_tool_names(&tools, Path::new("/my/repo"));
    ///
    /// assert!(integration.is_connected());
    /// assert!(integration.has_capability(NarsilCapability::SecurityScan));
    /// ```
    #[must_use]
    pub fn from_tool_names(tool_names: &[String], repo_path: &Path) -> Self {
        let capabilities = NarsilCapabilities::from_tools(tool_names);

        let repo_name = repo_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        Self {
            repo_path: repo_path.to_path_buf(),
            repo_name,
            capabilities,
            connected: true,
        }
    }

    /// Disconnects the integration.
    ///
    /// After calling this method, `is_connected()` will return false.
    /// The capabilities are preserved for inspection but the integration
    /// should not be used for further operations.
    pub fn disconnect(&mut self) {
        self.connected = false;
    }

    /// Creates a compression orchestrator for context management.
    ///
    /// The orchestrator automatically selects the appropriate backend:
    /// - CCG backend if `CcgGraph` capability is available
    /// - Fallback backend using core narsil tools otherwise
    ///
    /// # Example
    ///
    /// ```ignore
    /// use patina::narsil::NarsilIntegration;
    /// use patina::context::compression::CompressionLevel;
    ///
    /// let integration = NarsilIntegration::from_tool_names(&tools, &path);
    /// let orchestrator = integration.create_compression_orchestrator();
    ///
    /// // Use orchestrator for context compression
    /// let result = orchestrator.get_context(CompressionLevel::Manifest, "abc123", || {
    ///     // Fetch content if not cached...
    ///     "# Manifest content".to_string()
    /// });
    /// ```
    #[must_use]
    pub fn create_compression_orchestrator(&self) -> Arc<CompressionOrchestrator> {
        Arc::new(CompressionOrchestrator::new(
            self.capabilities.clone(),
            &self.repo_name,
        ))
    }

    /// Creates a compression orchestrator with custom configuration.
    ///
    /// # Arguments
    ///
    /// * `cache_ttl` - Cache time-to-live
    /// * `max_cache_entries` - Maximum number of cached results
    ///
    /// # Example
    ///
    /// ```ignore
    /// use std::time::Duration;
    ///
    /// // Create orchestrator with 10-minute cache TTL and 200 max entries
    /// let orchestrator = integration.create_compression_orchestrator_with_config(
    ///     Duration::from_secs(600),
    ///     200,
    /// );
    /// ```
    #[must_use]
    pub fn create_compression_orchestrator_with_config(
        &self,
        cache_ttl: Duration,
        max_cache_entries: usize,
    ) -> Arc<CompressionOrchestrator> {
        Arc::new(CompressionOrchestrator::with_config(
            self.capabilities.clone(),
            &self.repo_name,
            cache_ttl,
            max_cache_entries,
        ))
    }

    /// Suggests relevant context for the given code references.
    ///
    /// This method analyzes the provided code references and queries narsil-mcp
    /// for related context:
    /// - For function references: returns callers of the function
    /// - For file references: returns imports/dependencies of the file
    ///
    /// # Library-Only
    ///
    /// This function is exposed for programmatic context building in custom tools.
    /// It is not called automatically by Patina to avoid adding narsil-mcp as a
    /// required dependency in performance-critical paths.
    ///
    /// Use cases:
    /// - Building IDE integrations that want proactive context suggestions
    /// - Creating automation tools that need code-aware context
    ///
    /// # Arguments
    ///
    /// * `client` - An active `McpConnection` connected to narsil-mcp
    /// * `references` - Code references to get context for
    ///
    /// # Returns
    ///
    /// A vector of context suggestions, one for each reference that could be resolved.
    ///
    /// # Errors
    ///
    /// Returns an error if MCP calls fail.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use patina::narsil::{NarsilIntegration, extract_code_references};
    ///
    /// let refs = extract_code_references("Look at the `process_data` function");
    /// let suggestions = integration.suggest_context(&conn, &refs).await?;
    /// ```
    pub async fn suggest_context(
        &self,
        client: &McpConnection,
        references: &[CodeReference],
    ) -> Result<Vec<ContextSuggestion>> {
        let mut suggestions = Vec::new();

        for reference in references {
            match reference {
                CodeReference::Function { name } => {
                    // Query callers for function references if we have CallGraph capability
                    if self.has_capability(NarsilCapability::CallGraph) {
                        if let Ok(response) = client
                            .call_tool_json(
                                "get_callers",
                                serde_json::json!({
                                    "repo": self.repo_path.to_string_lossy(),
                                    "function": name,
                                }),
                            )
                            .await
                        {
                            let callers = parse_callers_response(&response);
                            let suggestion =
                                build_context_suggestion_from_callers(reference.clone(), &callers);
                            suggestions.push(suggestion);
                        }
                    }
                }
                CodeReference::File { path, .. } => {
                    // Query dependencies for file references if we have DependencyAnalysis capability
                    if self.has_capability(NarsilCapability::DependencyAnalysis) {
                        if let Ok(response) = client
                            .call_tool_json(
                                "get_dependencies",
                                serde_json::json!({
                                    "repo": self.repo_path.to_string_lossy(),
                                    "path": path,
                                    "direction": "imports",
                                }),
                            )
                            .await
                        {
                            let deps = parse_dependencies_response(&response);
                            let suggestion = build_context_suggestion_from_dependencies(
                                reference.clone(),
                                &deps,
                            );
                            suggestions.push(suggestion);
                        }
                    }
                }
            }
        }

        Ok(suggestions)
    }
}

/// Information about a function that calls another function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallerInfo {
    /// Name of the calling function.
    pub function: String,
    /// File containing the caller.
    pub file: String,
    /// Line number of the call site.
    pub line: u32,
}

/// Information about a dependency/import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyInfo {
    /// The import path (e.g., "std::collections::HashMap").
    pub path: String,
    /// The kind of import (e.g., "use", "mod").
    pub kind: String,
}

/// Parses a narsil `get_callers` response into caller information.
///
/// # Arguments
///
/// * `response` - JSON response from narsil get_callers tool
///
/// # Returns
///
/// A vector of caller information extracted from the response.
#[must_use]
pub fn parse_callers_response(response: &serde_json::Value) -> Vec<CallerInfo> {
    let Some(callers) = response.get("callers").and_then(|c| c.as_array()) else {
        return Vec::new();
    };

    callers
        .iter()
        .filter_map(|caller| {
            let function = caller.get("function")?.as_str()?.to_string();
            let file = caller.get("file")?.as_str()?.to_string();
            let line = caller.get("line")?.as_u64()? as u32;
            Some(CallerInfo {
                function,
                file,
                line,
            })
        })
        .collect()
}

/// Parses a narsil `get_dependencies` response into dependency information.
///
/// # Arguments
///
/// * `response` - JSON response from narsil get_dependencies tool
///
/// # Returns
///
/// A vector of dependency information extracted from the response.
#[must_use]
pub fn parse_dependencies_response(response: &serde_json::Value) -> Vec<DependencyInfo> {
    let Some(imports) = response.get("imports").and_then(|i| i.as_array()) else {
        return Vec::new();
    };

    imports
        .iter()
        .filter_map(|dep| {
            let path = dep.get("path")?.as_str()?.to_string();
            let kind = dep
                .get("kind")
                .and_then(|k| k.as_str())
                .unwrap_or("use")
                .to_string();
            Some(DependencyInfo { path, kind })
        })
        .collect()
}

/// Builds a context suggestion from caller information.
///
/// # Arguments
///
/// * `source` - The code reference that was queried
/// * `callers` - The caller information from narsil
///
/// # Returns
///
/// A context suggestion describing the callers.
#[must_use]
pub fn build_context_suggestion_from_callers(
    source: CodeReference,
    callers: &[CallerInfo],
) -> ContextSuggestion {
    let function_name = source.as_function_name().unwrap_or("unknown");

    let description = format!("Functions that call `{}`", function_name);

    let content = if callers.is_empty() {
        "No callers found".to_string()
    } else {
        callers
            .iter()
            .map(|c| format!("{}() in {}:{}", c.function, c.file, c.line))
            .collect::<Vec<_>>()
            .join("\n")
    };

    ContextSuggestion {
        source,
        kind: ContextKind::Callers,
        description,
        content,
    }
}

/// Builds a context suggestion from dependency information.
///
/// # Arguments
///
/// * `source` - The code reference that was queried
/// * `deps` - The dependency information from narsil
///
/// # Returns
///
/// A context suggestion describing the imports.
#[must_use]
pub fn build_context_suggestion_from_dependencies(
    source: CodeReference,
    deps: &[DependencyInfo],
) -> ContextSuggestion {
    let file_path = source.as_file_path().unwrap_or("unknown");

    let description = format!("Imports in `{}`", file_path);

    let content = if deps.is_empty() {
        "No imports found".to_string()
    } else {
        deps.iter()
            .map(|d| format!("{} {}", d.kind, d.path))
            .collect::<Vec<_>>()
            .join("\n")
    };

    ContextSuggestion {
        source,
        kind: ContextKind::Imports,
        description,
        content,
    }
}

// =============================================================================
// Security pre-flight types and helpers (Task 2.3.3)
// =============================================================================

/// A security finding from narsil scan_security.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityFinding {
    /// Severity level: CRITICAL, HIGH, MEDIUM, LOW, INFO.
    pub severity: String,
    /// The rule that triggered this finding (e.g., CWE-78).
    pub rule: String,
    /// Human-readable description of the issue.
    pub message: String,
}

/// Parses a narsil `scan_security` response into security findings.
///
/// # Arguments
///
/// * `response` - JSON response from narsil scan_security tool
///
/// # Returns
///
/// A vector of security findings extracted from the response.
#[must_use]
pub fn parse_security_findings(response: &serde_json::Value) -> Vec<SecurityFinding> {
    let Some(findings) = response.get("findings").and_then(|f| f.as_array()) else {
        return Vec::new();
    };

    findings
        .iter()
        .filter_map(|finding| {
            let severity = finding.get("severity")?.as_str()?.to_string();
            let rule = finding.get("rule")?.as_str()?.to_string();
            let message = finding.get("message")?.as_str()?.to_string();
            Some(SecurityFinding {
                severity,
                rule,
                message,
            })
        })
        .collect()
}

/// Converts security findings into a security verdict.
///
/// # Arguments
///
/// * `findings` - Security findings from narsil scan
///
/// # Returns
///
/// - `Block` if any CRITICAL findings exist
/// - `Warn` if any HIGH findings exist (and no CRITICAL)
/// - `Allow` if only MEDIUM/LOW/INFO findings or no findings
#[must_use]
pub fn security_verdict_from_findings(
    findings: &[SecurityFinding],
) -> crate::narsil::SecurityVerdict {
    use crate::narsil::SecurityVerdict;

    // Check for CRITICAL first (highest priority)
    if let Some(critical) = findings
        .iter()
        .find(|f| f.severity.eq_ignore_ascii_case("critical"))
    {
        return SecurityVerdict::Block(format!(
            "CRITICAL: {} ({})",
            critical.message, critical.rule
        ));
    }

    // Check for HIGH (warning, but allows execution)
    if let Some(high) = findings
        .iter()
        .find(|f| f.severity.eq_ignore_ascii_case("high"))
    {
        return SecurityVerdict::Warn(format!("HIGH: {} ({})", high.message, high.rule));
    }

    // MEDIUM, LOW, INFO are allowed without warning
    SecurityVerdict::Allow
}

// =============================================================================
// Parallel MCP capability detection (Phase 5.2)
// =============================================================================

/// Result of detecting capabilities from a single MCP server.
///
/// Contains the discovered tool names and metadata about the detection process,
/// including timing and error information for servers that failed or timed out.
#[derive(Debug, Clone)]
pub struct ServerCapabilityResult {
    /// Name of the MCP server that was queried.
    pub server_name: String,
    /// Tool names discovered from the server (empty if detection failed).
    pub tools: Vec<String>,
    /// Detected narsil capabilities.
    pub capabilities: NarsilCapabilities,
    /// Whether capability detection succeeded.
    pub success: bool,
    /// Error message if detection failed (timeout, connection error, etc.).
    pub error: Option<String>,
    /// Time taken for the detection attempt.
    pub duration: Duration,
}

impl ServerCapabilityResult {
    /// Returns the tool names discovered from this server.
    #[must_use]
    pub fn tool_names(&self) -> Vec<String> {
        self.tools.clone()
    }

    /// Returns true if the server timed out during detection.
    #[must_use]
    pub fn is_timeout(&self) -> bool {
        self.error.as_ref().is_some_and(|e| e.contains("timed out"))
    }
}

/// Result of parallel capability detection across multiple MCP servers.
///
/// Aggregates individual server results and provides convenience methods
/// for building `NarsilCapabilities` from the combined tool lists.
///
/// # Example
///
/// ```ignore
/// let results = detect_all_capabilities(&connections, Duration::from_secs(10)).await;
/// println!("Detected {} servers, {} succeeded", results.total(), results.successful_count());
///
/// let capabilities = results.merged_capabilities();
/// if capabilities.has(NarsilCapability::SecurityScan) {
///     println!("Security scanning available!");
/// }
/// ```
#[derive(Debug, Clone)]
pub struct ParallelCapabilityResult {
    /// Per-server results.
    results: Vec<ServerCapabilityResult>,
}

impl ParallelCapabilityResult {
    /// Returns the total number of servers queried.
    #[must_use]
    pub fn total(&self) -> usize {
        self.results.len()
    }

    /// Returns the number of servers that responded successfully.
    #[must_use]
    pub fn successful_count(&self) -> usize {
        self.results.iter().filter(|r| r.success).count()
    }

    /// Returns the number of servers that failed (error or timeout).
    #[must_use]
    pub fn failed_count(&self) -> usize {
        self.results.iter().filter(|r| !r.success).count()
    }

    /// Returns the number of servers that timed out.
    #[must_use]
    pub fn timeout_count(&self) -> usize {
        self.results.iter().filter(|r| r.is_timeout()).count()
    }

    /// Returns the per-server results.
    #[must_use]
    pub fn server_results(&self) -> &[ServerCapabilityResult] {
        &self.results
    }

    /// Merges all successful tool lists into a single `NarsilCapabilities`.
    ///
    /// Tools from all successful servers are combined. Duplicate tool names
    /// are deduplicated by the `NarsilCapabilities::from_tools` constructor.
    #[must_use]
    pub fn merged_capabilities(&self) -> NarsilCapabilities {
        let all_tools: Vec<String> = self
            .results
            .iter()
            .filter(|r| r.success)
            .flat_map(|r| r.tool_names())
            .collect();
        NarsilCapabilities::from_tools(&all_tools)
    }

    /// Returns the names of servers that failed.
    #[must_use]
    pub fn failed_servers(&self) -> Vec<&str> {
        self.results
            .iter()
            .filter(|r| !r.success)
            .map(|r| r.server_name.as_str())
            .collect()
    }

    /// Returns true if all servers were detected successfully.
    #[must_use]
    pub fn all_succeeded(&self) -> bool {
        self.results.iter().all(|r| r.success)
    }
}

/// Detects capabilities from multiple MCP connections.
///
/// Reads the tool list from each connection and builds capability results.
/// Connections that are not connected are reported as failures.
///
/// # Arguments
///
/// * `connections` - Slice of active `McpConnection` instances to query
/// * `_per_server_timeout` - Reserved for future use (connections already have tools cached)
///
/// # Returns
///
/// A `ParallelCapabilityResult` containing per-server results and methods
/// to merge capabilities across servers.
///
/// # Example
///
/// ```ignore
/// let result = detect_all_capabilities(&connections, Duration::from_secs(10)).await;
///
/// // Check overall status
/// tracing::info!(
///     total = result.total(),
///     success = result.successful_count(),
///     failed = result.failed_count(),
///     "MCP capability detection complete"
/// );
///
/// // Build capabilities from all successful servers
/// let capabilities = result.merged_capabilities();
/// ```
pub async fn detect_all_capabilities(
    connections: &[McpConnection],
    _per_server_timeout: Duration,
) -> ParallelCapabilityResult {
    let mut results = Vec::new();
    for conn in connections {
        let start = std::time::Instant::now();
        let tools = conn.tools();
        let tool_names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
        let capabilities = NarsilCapabilities::from_tools(&tool_names);
        let connected = conn.is_connected();

        let result = ServerCapabilityResult {
            server_name: conn.name().to_string(),
            tools: tool_names,
            capabilities,
            success: connected,
            error: if connected {
                None
            } else {
                Some("Not connected".to_string())
            },
            duration: start.elapsed(),
        };

        if result.success {
            tracing::info!(
                server = %result.server_name,
                tools = result.tools.len(),
                duration_ms = result.duration.as_millis() as u64,
                "MCP server capabilities detected"
            );
        } else {
            tracing::warn!(
                server = %result.server_name,
                error = result.error.as_deref().unwrap_or("unknown"),
                duration_ms = result.duration.as_millis() as u64,
                "MCP server capability detection failed"
            );
        }

        results.push(result);
    }

    ParallelCapabilityResult { results }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::narsil::SecurityVerdict;
    use std::path::PathBuf;

    // =============================================================================
    // NarsilCapability tests
    // =============================================================================

    #[test]
    fn test_capability_required_tools_not_empty() {
        for capability in NarsilCapability::all() {
            assert!(
                !capability.required_tools().is_empty(),
                "{:?} should have required tools",
                capability
            );
        }
    }

    #[test]
    fn test_capability_call_graph_tools() {
        let tools = NarsilCapability::CallGraph.required_tools();
        assert!(tools.contains(&"get_call_graph"));
        assert!(tools.contains(&"get_callers"));
        assert!(tools.contains(&"get_callees"));
    }

    #[test]
    fn test_capability_security_scan_tools() {
        let tools = NarsilCapability::SecurityScan.required_tools();
        assert!(tools.contains(&"scan_security"));
        assert!(tools.contains(&"check_owasp_top10"));
    }

    #[test]
    fn test_capability_all_returns_all_variants() {
        let all = NarsilCapability::all();
        assert!(all.len() >= 8, "Should have at least 8 capabilities");
        assert!(all.contains(&NarsilCapability::CallGraph));
        assert!(all.contains(&NarsilCapability::SecurityScan));
        assert!(all.contains(&NarsilCapability::CodeSearch));
        assert!(all.contains(&NarsilCapability::SymbolAnalysis));
        assert!(all.contains(&NarsilCapability::CcgGraph));
    }

    // =============================================================================
    // CcgGraph capability tests (Task 3.1.1)
    // =============================================================================

    #[test]
    fn test_ccg_graph_capability_detected_when_tools_present() {
        let tools = vec![
            "get_ccg_manifest".to_string(),
            "export_ccg_architecture".to_string(),
            "query_ccg".to_string(),
        ];
        let caps = NarsilCapabilities::from_tools(&tools);

        assert!(caps.has(NarsilCapability::CcgGraph));
    }

    #[test]
    fn test_ccg_graph_required_tools_returns_correct_tools() {
        let tools = NarsilCapability::CcgGraph.required_tools();
        assert!(tools.contains(&"get_ccg_manifest"));
        assert!(tools.contains(&"export_ccg_architecture"));
        assert!(tools.contains(&"query_ccg"));
    }

    #[test]
    fn test_all_capabilities_includes_ccg_graph() {
        let all = NarsilCapability::all();
        assert!(all.contains(&NarsilCapability::CcgGraph));
    }

    #[test]
    fn test_ccg_graph_capability_absent_when_no_graph_flag() {
        // These are the core tools that work without --graph flag
        let tools = vec![
            "find_symbols".to_string(),
            "get_dependencies".to_string(),
            "get_export_map".to_string(),
            "get_incremental_status".to_string(),
        ];
        let caps = NarsilCapabilities::from_tools(&tools);

        // Should NOT have CcgGraph without the CCG-specific tools
        assert!(!caps.has(NarsilCapability::CcgGraph));
        // But should have other capabilities
        assert!(caps.has(NarsilCapability::SymbolAnalysis));
        assert!(caps.has(NarsilCapability::DependencyAnalysis));
    }

    // =============================================================================
    // NarsilCapabilities tests
    // =============================================================================

    #[test]
    fn test_capabilities_new_is_empty() {
        let caps = NarsilCapabilities::new();
        assert!(!caps.is_available());
        assert_eq!(caps.count(), 0);
        assert!(caps.available().is_empty());
    }

    #[test]
    fn test_capabilities_from_tools_detects_call_graph() {
        let tools = vec![
            "get_call_graph".to_string(),
            "get_callers".to_string(),
            "other_tool".to_string(),
        ];
        let caps = NarsilCapabilities::from_tools(&tools);

        assert!(caps.has(NarsilCapability::CallGraph));
        assert!(!caps.has(NarsilCapability::SecurityScan));
        assert!(caps.is_available());
    }

    #[test]
    fn test_capabilities_from_tools_detects_security_scan() {
        let tools = vec!["scan_security".to_string(), "check_owasp_top10".to_string()];
        let caps = NarsilCapabilities::from_tools(&tools);

        assert!(caps.has(NarsilCapability::SecurityScan));
        assert!(!caps.has(NarsilCapability::CallGraph));
    }

    #[test]
    fn test_capabilities_from_tools_detects_multiple() {
        let tools = vec![
            "get_call_graph".to_string(),
            "scan_security".to_string(),
            "search_code".to_string(),
            "find_symbols".to_string(),
        ];
        let caps = NarsilCapabilities::from_tools(&tools);

        assert!(caps.has(NarsilCapability::CallGraph));
        assert!(caps.has(NarsilCapability::SecurityScan));
        assert!(caps.has(NarsilCapability::CodeSearch));
        assert!(caps.has(NarsilCapability::SymbolAnalysis));
        assert_eq!(caps.count(), 4);
    }

    #[test]
    fn test_capabilities_tools_returns_available_tools() {
        let tools = vec!["scan_security".to_string(), "search_code".to_string()];
        let caps = NarsilCapabilities::from_tools(&tools);

        assert!(caps.tools().contains("scan_security"));
        assert!(caps.tools().contains("search_code"));
        assert!(!caps.tools().contains("nonexistent"));
    }

    // =============================================================================
    // NarsilIntegration tests - Task 2.1.1
    // =============================================================================

    #[test]
    fn test_narsil_integration_new() {
        let path = PathBuf::from("/tmp/test-repo");
        let integration = NarsilIntegration::new(&path);

        assert_eq!(integration.repo_path(), path.as_path());
        assert_eq!(integration.repo_name(), "test-repo");
        assert!(!integration.is_connected());
        assert!(!integration.capabilities().is_available());
    }

    #[test]
    fn test_narsil_integration_new_extracts_repo_name() {
        let path = PathBuf::from("/home/user/projects/my-awesome-project");
        let integration = NarsilIntegration::new(&path);

        assert_eq!(integration.repo_name(), "my-awesome-project");
    }

    #[test]
    fn test_narsil_integration_has_capability_initially_false() {
        let path = PathBuf::from("/tmp/test-repo");
        let integration = NarsilIntegration::new(&path);

        assert!(!integration.has_capability(NarsilCapability::CallGraph));
        assert!(!integration.has_capability(NarsilCapability::SecurityScan));
        assert!(!integration.has_capability(NarsilCapability::CodeSearch));
    }

    #[test]
    fn test_narsil_capabilities_detection() {
        // Simulate detecting capabilities from tool list
        let tools = vec![
            "get_call_graph".to_string(),
            "get_callers".to_string(),
            "get_callees".to_string(),
            "scan_security".to_string(),
            "check_owasp_top10".to_string(),
            "check_cwe_top25".to_string(),
            "search_code".to_string(),
            "semantic_search".to_string(),
        ];

        let path = PathBuf::from("/tmp/test-repo");

        // Use from_tool_names to create a connected integration with capabilities
        let integration = NarsilIntegration::from_tool_names(&tools, &path);

        // Verify capabilities were detected
        assert!(integration.is_connected());
        assert!(integration.has_capability(NarsilCapability::CallGraph));
        assert!(integration.has_capability(NarsilCapability::SecurityScan));
        assert!(integration.has_capability(NarsilCapability::CodeSearch));
        assert!(!integration.has_capability(NarsilCapability::GitHistory));
        assert!(!integration.has_capability(NarsilCapability::TaintAnalysis));
    }

    #[test]
    fn test_narsil_integration_from_tool_names() {
        let path = PathBuf::from("/tmp/test-repo");

        let tools = vec!["trace_taint".to_string(), "get_taint_sources".to_string()];
        let integration = NarsilIntegration::from_tool_names(&tools, &path);

        assert!(integration.is_connected());
        assert!(integration.has_capability(NarsilCapability::TaintAnalysis));
        assert!(!integration.has_capability(NarsilCapability::CallGraph));
    }

    #[test]
    fn test_narsil_integration_root_path() {
        let path = PathBuf::from("/");
        let integration = NarsilIntegration::new(&path);

        // Root path should still work, repo_name becomes empty or "/"
        assert_eq!(integration.repo_path(), Path::new("/"));
    }

    #[test]
    fn test_narsil_integration_disconnect() {
        let tools = vec!["scan_security".to_string()];
        let path = PathBuf::from("/tmp/test-repo");

        let mut integration = NarsilIntegration::from_tool_names(&tools, &path);
        assert!(integration.is_connected());

        integration.disconnect();
        assert!(!integration.is_connected());

        // Capabilities should still be accessible after disconnect
        assert!(integration.has_capability(NarsilCapability::SecurityScan));
    }

    #[test]
    fn test_narsil_integration_from_tool_names_extracts_repo_name() {
        let tools = vec!["search_code".to_string()];
        let path = PathBuf::from("/home/user/my-project");

        let integration = NarsilIntegration::from_tool_names(&tools, &path);

        assert_eq!(integration.repo_name(), "my-project");
        assert!(integration.is_connected());
    }

    #[test]
    fn test_narsil_integration_from_empty_tools() {
        let tools: Vec<String> = vec![];
        let path = PathBuf::from("/tmp/test-repo");

        let integration = NarsilIntegration::from_tool_names(&tools, &path);

        // Should be connected but have no capabilities
        assert!(integration.is_connected());
        assert!(!integration.capabilities().is_available());
        assert_eq!(integration.capabilities().count(), 0);
    }

    // =============================================================================
    // Task 2.2.3 - suggest_context tests
    // =============================================================================

    #[test]
    fn test_parse_callers_response_valid() {
        // Simulated response from narsil get_callers
        let response = serde_json::json!({
            "callers": [
                {"function": "main", "file": "src/main.rs", "line": 10},
                {"function": "handle_request", "file": "src/handler.rs", "line": 42}
            ]
        });

        let callers = parse_callers_response(&response);
        assert_eq!(callers.len(), 2);
        assert_eq!(callers[0].function, "main");
        assert_eq!(callers[0].file, "src/main.rs");
        assert_eq!(callers[0].line, 10);
    }

    #[test]
    fn test_parse_callers_response_empty() {
        let response = serde_json::json!({
            "callers": []
        });

        let callers = parse_callers_response(&response);
        assert!(callers.is_empty());
    }

    #[test]
    fn test_parse_callers_response_missing_field() {
        // Response without callers field
        let response = serde_json::json!({
            "other": "data"
        });

        let callers = parse_callers_response(&response);
        assert!(callers.is_empty());
    }

    #[test]
    fn test_parse_dependencies_response_valid() {
        // Simulated response from narsil get_dependencies
        let response = serde_json::json!({
            "imports": [
                {"path": "std::collections::HashMap", "kind": "use"},
                {"path": "crate::utils", "kind": "mod"}
            ]
        });

        let deps = parse_dependencies_response(&response);
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].path, "std::collections::HashMap");
        assert_eq!(deps[0].kind, "use");
    }

    #[test]
    fn test_parse_dependencies_response_empty() {
        let response = serde_json::json!({
            "imports": []
        });

        let deps = parse_dependencies_response(&response);
        assert!(deps.is_empty());
    }

    #[test]
    fn test_parse_dependencies_response_missing_field() {
        let response = serde_json::json!({
            "other": "data"
        });

        let deps = parse_dependencies_response(&response);
        assert!(deps.is_empty());
    }

    #[test]
    fn test_build_context_suggestion_from_callers() {
        use crate::narsil::context::{CodeReference, ContextKind};

        let callers = vec![
            CallerInfo {
                function: "main".to_string(),
                file: "src/main.rs".to_string(),
                line: 10,
            },
            CallerInfo {
                function: "handle_request".to_string(),
                file: "src/handler.rs".to_string(),
                line: 42,
            },
        ];

        let source = CodeReference::function("process_data");
        let suggestion = build_context_suggestion_from_callers(source, &callers);

        assert_eq!(suggestion.kind, ContextKind::Callers);
        assert!(suggestion.description.contains("process_data"));
        assert!(suggestion.content.contains("main"));
        assert!(suggestion.content.contains("handle_request"));
    }

    #[test]
    fn test_build_context_suggestion_from_dependencies() {
        use crate::narsil::context::{CodeReference, ContextKind};

        let deps = vec![
            DependencyInfo {
                path: "std::collections::HashMap".to_string(),
                kind: "use".to_string(),
            },
            DependencyInfo {
                path: "crate::utils".to_string(),
                kind: "mod".to_string(),
            },
        ];

        let source = CodeReference::file("src/handler.rs");
        let suggestion = build_context_suggestion_from_dependencies(source, &deps);

        assert_eq!(suggestion.kind, ContextKind::Imports);
        assert!(suggestion.description.contains("src/handler.rs"));
        assert!(suggestion.content.contains("std::collections::HashMap"));
        assert!(suggestion.content.contains("crate::utils"));
    }

    #[test]
    fn test_build_context_suggestion_from_callers_empty() {
        use crate::narsil::context::{CodeReference, ContextKind};

        let callers: Vec<CallerInfo> = vec![];
        let source = CodeReference::function("unused_function");
        let suggestion = build_context_suggestion_from_callers(source, &callers);

        assert_eq!(suggestion.kind, ContextKind::Callers);
        assert!(
            suggestion.content.contains("No callers found")
                || suggestion.content.is_empty()
                || suggestion.content == "None"
        );
    }

    #[test]
    fn test_build_context_suggestion_from_dependencies_empty() {
        use crate::narsil::context::{CodeReference, ContextKind};

        let deps: Vec<DependencyInfo> = vec![];
        let source = CodeReference::file("isolated_file.rs");
        let suggestion = build_context_suggestion_from_dependencies(source, &deps);

        assert_eq!(suggestion.kind, ContextKind::Imports);
        assert!(
            suggestion.content.contains("No imports found")
                || suggestion.content.is_empty()
                || suggestion.content == "None"
        );
    }

    #[test]
    fn test_caller_info_struct() {
        let info = CallerInfo {
            function: "test_fn".to_string(),
            file: "src/test.rs".to_string(),
            line: 100,
        };

        assert_eq!(info.function, "test_fn");
        assert_eq!(info.file, "src/test.rs");
        assert_eq!(info.line, 100);
    }

    #[test]
    fn test_dependency_info_struct() {
        let info = DependencyInfo {
            path: "std::io".to_string(),
            kind: "use".to_string(),
        };

        assert_eq!(info.path, "std::io");
        assert_eq!(info.kind, "use");
    }

    // =============================================================================
    // Task 2.3.3 - security_check tests
    // =============================================================================

    #[test]
    fn test_parse_security_findings_no_findings() {
        let response = serde_json::json!({
            "findings": []
        });

        let findings = parse_security_findings(&response);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_parse_security_findings_with_findings() {
        let response = serde_json::json!({
            "findings": [
                {"severity": "CRITICAL", "rule": "CWE-78", "message": "Command injection"},
                {"severity": "HIGH", "rule": "CWE-22", "message": "Path traversal"}
            ]
        });

        let findings = parse_security_findings(&response);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].severity, "CRITICAL");
        assert_eq!(findings[0].rule, "CWE-78");
        assert_eq!(findings[1].severity, "HIGH");
    }

    #[test]
    fn test_parse_security_findings_missing_field() {
        let response = serde_json::json!({
            "other": "data"
        });

        let findings = parse_security_findings(&response);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_security_verdict_from_findings_empty() {
        use crate::narsil::SecurityVerdict;

        let findings: Vec<SecurityFinding> = vec![];
        let verdict = security_verdict_from_findings(&findings);
        assert_eq!(verdict, SecurityVerdict::Allow);
    }

    #[test]
    fn test_security_verdict_from_findings_critical() {
        let findings = vec![SecurityFinding {
            severity: "CRITICAL".to_string(),
            rule: "CWE-78".to_string(),
            message: "Command injection".to_string(),
        }];
        let verdict = security_verdict_from_findings(&findings);
        assert!(verdict.blocks_execution());
        assert!(verdict.reason().unwrap().contains("CRITICAL"));
    }

    #[test]
    fn test_security_verdict_from_findings_high() {
        let findings = vec![SecurityFinding {
            severity: "HIGH".to_string(),
            rule: "CWE-22".to_string(),
            message: "Path traversal".to_string(),
        }];
        let verdict = security_verdict_from_findings(&findings);
        assert!(verdict.has_warning());
        assert!(verdict.allows_execution());
    }

    #[test]
    fn test_security_verdict_from_findings_low_allows() {
        let findings = vec![SecurityFinding {
            severity: "LOW".to_string(),
            rule: "STYLE-001".to_string(),
            message: "Formatting issue".to_string(),
        }];
        let verdict = security_verdict_from_findings(&findings);
        assert_eq!(verdict, SecurityVerdict::Allow);
    }

    #[test]
    fn test_security_verdict_critical_takes_precedence() {
        // If both CRITICAL and HIGH exist, CRITICAL takes precedence (blocks)
        let findings = vec![
            SecurityFinding {
                severity: "HIGH".to_string(),
                rule: "CWE-22".to_string(),
                message: "Path traversal".to_string(),
            },
            SecurityFinding {
                severity: "CRITICAL".to_string(),
                rule: "CWE-78".to_string(),
                message: "Command injection".to_string(),
            },
        ];
        let verdict = security_verdict_from_findings(&findings);
        assert!(verdict.blocks_execution());
    }

    // =============================================================================
    // Task 3.3.4 - Compression orchestrator integration tests
    // =============================================================================

    #[test]
    fn test_create_compression_orchestrator_from_integration() {
        let tools = vec!["find_symbols".to_string(), "get_dependencies".to_string()];
        let path = PathBuf::from("/tmp/test-repo");
        let integration = NarsilIntegration::from_tool_names(&tools, &path);

        let orchestrator = integration.create_compression_orchestrator();

        // Orchestrator should be created successfully
        assert_eq!(orchestrator.repo_name(), "test-repo");
        // Should NOT use CCG (no CCG tools provided)
        assert!(!orchestrator.should_use_ccg());
        assert!(orchestrator.should_use_fallback());
    }

    #[test]
    fn test_create_compression_orchestrator_with_ccg() {
        let tools = vec![
            "get_ccg_manifest".to_string(),
            "export_ccg_architecture".to_string(),
            "query_ccg".to_string(),
        ];
        let path = PathBuf::from("/my/project");
        let integration = NarsilIntegration::from_tool_names(&tools, &path);

        let orchestrator = integration.create_compression_orchestrator();

        // Should use CCG
        assert!(orchestrator.should_use_ccg());
        assert!(!orchestrator.should_use_fallback());
        assert_eq!(orchestrator.repo_name(), "project");
    }

    #[test]
    fn test_create_compression_orchestrator_with_config() {
        let tools = vec!["find_symbols".to_string()];
        let path = PathBuf::from("/tmp/test-repo");
        let integration = NarsilIntegration::from_tool_names(&tools, &path);

        let orchestrator =
            integration.create_compression_orchestrator_with_config(Duration::from_secs(600), 200);

        // Verify config is applied (indirectly through the orchestrator)
        assert_eq!(orchestrator.repo_name(), "test-repo");
    }

    #[test]
    fn test_context_building_uses_compression_when_available() {
        use crate::context::compression::CompressionLevel;

        let tools = vec!["find_symbols".to_string(), "get_dependencies".to_string()];
        let path = PathBuf::from("/tmp/test-repo");
        let integration = NarsilIntegration::from_tool_names(&tools, &path);

        // Create orchestrator
        let orchestrator = integration.create_compression_orchestrator();

        // Use get_context (should work with fallback path)
        let result = orchestrator.get_context(CompressionLevel::Manifest, "test-hash", || {
            "# Test Manifest\n\nFallback content".to_string()
        });

        assert!(!result.content().is_empty());
        assert_eq!(result.level(), CompressionLevel::Manifest);
    }

    // =============================================================================
    // Phase 5.2 - Parallel MCP capability detection tests
    // =============================================================================

    mod parallel_capability_tests {
        use super::*;

        fn make_success_result(name: &str, tool_names: &[&str]) -> ServerCapabilityResult {
            let tools: Vec<String> = tool_names.iter().map(|n| n.to_string()).collect();
            let capabilities = NarsilCapabilities::from_tools(&tools);
            ServerCapabilityResult {
                server_name: name.to_string(),
                tools,
                capabilities,
                success: true,
                error: None,
                duration: Duration::from_millis(50),
            }
        }

        fn make_failure_result(name: &str, error: &str) -> ServerCapabilityResult {
            ServerCapabilityResult {
                server_name: name.to_string(),
                tools: Vec::new(),
                capabilities: NarsilCapabilities::new(),
                success: false,
                error: Some(error.to_string()),
                duration: Duration::from_millis(5),
            }
        }

        fn make_timeout_result(name: &str) -> ServerCapabilityResult {
            ServerCapabilityResult {
                server_name: name.to_string(),
                tools: Vec::new(),
                capabilities: NarsilCapabilities::new(),
                success: false,
                error: Some("capability detection timed out after 10s".to_string()),
                duration: Duration::from_secs(10),
            }
        }

        #[test]
        fn test_parallel_result_all_succeed() {
            let results = vec![
                make_success_result("server-a", &["scan_security", "get_call_graph"]),
                make_success_result("server-b", &["search_code", "find_symbols"]),
            ];

            let parallel = ParallelCapabilityResult { results };

            assert_eq!(parallel.total(), 2);
            assert_eq!(parallel.successful_count(), 2);
            assert_eq!(parallel.failed_count(), 0);
            assert_eq!(parallel.timeout_count(), 0);
            assert!(parallel.all_succeeded());
            assert!(parallel.failed_servers().is_empty());
        }

        #[test]
        fn test_parallel_result_mixed_success_and_failure() {
            let results = vec![
                make_success_result("narsil", &["scan_security", "get_call_graph"]),
                make_failure_result("broken", "connection refused"),
                make_success_result("other", &["search_code"]),
            ];

            let parallel = ParallelCapabilityResult { results };

            assert_eq!(parallel.total(), 3);
            assert_eq!(parallel.successful_count(), 2);
            assert_eq!(parallel.failed_count(), 1);
            assert!(!parallel.all_succeeded());
            assert_eq!(parallel.failed_servers(), vec!["broken"]);
        }

        #[test]
        fn test_parallel_result_with_timeouts() {
            let results = vec![
                make_success_result("fast-server", &["get_call_graph"]),
                make_timeout_result("slow-server"),
                make_timeout_result("dead-server"),
            ];

            let parallel = ParallelCapabilityResult { results };

            assert_eq!(parallel.total(), 3);
            assert_eq!(parallel.successful_count(), 1);
            assert_eq!(parallel.failed_count(), 2);
            assert_eq!(parallel.timeout_count(), 2);
        }

        #[test]
        fn test_parallel_result_all_fail() {
            let results = vec![
                make_failure_result("server-a", "connection refused"),
                make_timeout_result("server-b"),
            ];

            let parallel = ParallelCapabilityResult { results };

            assert_eq!(parallel.total(), 2);
            assert_eq!(parallel.successful_count(), 0);
            assert_eq!(parallel.failed_count(), 2);
            assert!(!parallel.all_succeeded());
        }

        #[test]
        fn test_parallel_result_empty() {
            let parallel = ParallelCapabilityResult {
                results: Vec::new(),
            };

            assert_eq!(parallel.total(), 0);
            assert_eq!(parallel.successful_count(), 0);
            assert_eq!(parallel.failed_count(), 0);
            assert!(parallel.all_succeeded()); // vacuously true
        }

        #[test]
        fn test_merged_capabilities_combines_tools_from_all_servers() {
            let results = vec![
                make_success_result(
                    "narsil",
                    &["scan_security", "get_call_graph", "get_callers"],
                ),
                make_success_result("other", &["search_code", "semantic_search"]),
            ];

            let parallel = ParallelCapabilityResult { results };
            let caps = parallel.merged_capabilities();

            // Should detect capabilities from both servers
            assert!(caps.has(NarsilCapability::SecurityScan));
            assert!(caps.has(NarsilCapability::CallGraph));
            assert!(caps.has(NarsilCapability::CodeSearch));
            assert!(caps.is_available());
        }

        #[test]
        fn test_merged_capabilities_excludes_failed_servers() {
            let results = vec![
                make_success_result("narsil", &["scan_security"]),
                make_failure_result("broken", "connection refused"),
            ];

            let parallel = ParallelCapabilityResult { results };
            let caps = parallel.merged_capabilities();

            // Only tools from successful servers should be included
            assert!(caps.has(NarsilCapability::SecurityScan));
            assert_eq!(caps.tools().len(), 1);
        }

        #[test]
        fn test_merged_capabilities_empty_when_all_fail() {
            let results = vec![
                make_failure_result("server-a", "error"),
                make_timeout_result("server-b"),
            ];

            let parallel = ParallelCapabilityResult { results };
            let caps = parallel.merged_capabilities();

            assert!(!caps.is_available());
            assert_eq!(caps.count(), 0);
        }

        #[test]
        fn test_merged_capabilities_deduplicates_tools() {
            // Both servers provide the same tool - should not double-count
            let results = vec![
                make_success_result("server-a", &["scan_security", "search_code"]),
                make_success_result("server-b", &["scan_security", "find_symbols"]),
            ];

            let parallel = ParallelCapabilityResult { results };
            let caps = parallel.merged_capabilities();

            // scan_security from both should still be one capability
            assert!(caps.has(NarsilCapability::SecurityScan));
            assert!(caps.has(NarsilCapability::CodeSearch));
            assert!(caps.has(NarsilCapability::SymbolAnalysis));
            // Three unique tool names total
            assert_eq!(caps.tools().len(), 3);
        }

        #[test]
        fn test_server_results_preserves_order() {
            let results = vec![
                make_success_result("first", &["tool_a"]),
                make_failure_result("second", "error"),
                make_success_result("third", &["tool_b"]),
            ];

            let parallel = ParallelCapabilityResult { results };
            let server_results = parallel.server_results();

            assert_eq!(server_results[0].server_name, "first");
            assert_eq!(server_results[1].server_name, "second");
            assert_eq!(server_results[2].server_name, "third");
        }

        #[test]
        fn test_parallel_result_timeout_detection_specific() {
            let results = vec![
                make_success_result("fast", &["tool_a"]),
                make_timeout_result("slow"),
                make_failure_result("broken", "connection refused"),
            ];

            let parallel = ParallelCapabilityResult { results };

            // Only the timeout server should be counted
            assert_eq!(parallel.timeout_count(), 1);
            // Both non-success should be failed
            assert_eq!(parallel.failed_count(), 2);
        }
    }
}
