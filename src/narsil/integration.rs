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
//! use patina::mcp::client::McpClient;
//!
//! // Connect using an MCP client
//! let mut client = McpClient::new("narsil", "/path/to/narsil-mcp", vec!["--repos", "."]);
//! client.start().await?;
//!
//! let integration = NarsilIntegration::from_mcp_client(&mut client, Path::new(".")).await?;
//! if integration.has_capability(NarsilCapability::SecurityScan) {
//!     println!("Security scanning available!");
//! }
//! ```

use crate::mcp::client::McpClient;
use crate::narsil::context::{CodeReference, ContextKind, ContextSuggestion};
use anyhow::Result;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

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

    /// Creates a NarsilIntegration by connecting to an existing MCP client.
    ///
    /// This method queries the MCP client for available tools and automatically
    /// detects which narsil capabilities are available.
    ///
    /// # Arguments
    ///
    /// * `client` - An already-started MCP client connected to narsil-mcp
    /// * `repo_path` - Path to the repository being analyzed
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The MCP client is not connected
    /// - The tools/list request fails
    ///
    /// # Example
    ///
    /// ```ignore
    /// use patina::narsil::NarsilIntegration;
    /// use patina::mcp::client::McpClient;
    /// use std::path::Path;
    ///
    /// let mut client = McpClient::new("narsil", "/path/to/narsil-mcp", vec![]);
    /// client.start().await?;
    ///
    /// let integration = NarsilIntegration::from_mcp_client(&mut client, Path::new(".")).await?;
    /// println!("Connected with {} capabilities", integration.capabilities().count());
    /// ```
    pub async fn from_mcp_client(client: &mut McpClient, repo_path: &Path) -> Result<Self> {
        // Query available tools from the MCP server
        let tools = client.list_tools().await?;
        let tool_names: Vec<String> = tools.iter().map(|t| t.name.clone()).collect();

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

    /// Suggests relevant context for the given code references.
    ///
    /// This method analyzes the provided code references and queries narsil-mcp
    /// for related context:
    /// - For function references: returns callers of the function
    /// - For file references: returns imports/dependencies of the file
    ///
    /// # Arguments
    ///
    /// * `client` - An active MCP client connected to narsil-mcp
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
    /// let suggestions = integration.suggest_context(&mut client, &refs).await?;
    /// ```
    pub async fn suggest_context(
        &self,
        client: &mut McpClient,
        references: &[CodeReference],
    ) -> Result<Vec<ContextSuggestion>> {
        let mut suggestions = Vec::new();

        for reference in references {
            match reference {
                CodeReference::Function { name } => {
                    // Query callers for function references if we have CallGraph capability
                    if self.has_capability(NarsilCapability::CallGraph) {
                        if let Ok(response) = client
                            .call_tool(
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
                            .call_tool(
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
        assert!(all.len() >= 7, "Should have at least 7 capabilities");
        assert!(all.contains(&NarsilCapability::CallGraph));
        assert!(all.contains(&NarsilCapability::SecurityScan));
        assert!(all.contains(&NarsilCapability::CodeSearch));
        assert!(all.contains(&NarsilCapability::SymbolAnalysis));
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
}
