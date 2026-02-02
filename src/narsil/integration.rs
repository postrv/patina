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
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
