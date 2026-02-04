//! JSON-LD parsing and Markdown rendering for CCG responses.
//!
//! This module converts CCG tool responses (JSON-LD format) into compressed
//! Markdown suitable for context windows. It provides:
//!
//! - [`CcgManifest`]: Parsed Layer 0 manifest data
//! - [`CcgArchitecture`]: Parsed Layer 1 architecture data
//! - Markdown rendering functions for each data type
//!
//! # Example
//!
//! ```ignore
//! use patina::context::compression::render::{CcgManifest, parse_ccg_manifest, render_manifest_markdown};
//!
//! let manifest = parse_ccg_manifest(&json_response)?;
//! let markdown = render_manifest_markdown(&manifest);
//! ```

use serde::{Deserialize, Serialize};

/// Parsed CCG manifest (Layer 0 metadata).
///
/// Contains high-level repository information in approximately ~2KB.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CcgManifest {
    /// Repository name
    #[serde(default)]
    pub repo_name: String,
    /// Current commit hash
    #[serde(default)]
    pub commit: String,
    /// Languages detected with line counts
    #[serde(default)]
    pub languages: Vec<LanguageInfo>,
    /// Symbol count summary
    #[serde(default)]
    pub symbols: SymbolSummary,
    /// Security findings summary
    #[serde(default)]
    pub security: SecuritySummary,
    /// Quality metrics
    #[serde(default)]
    pub quality: QualitySummary,
}

/// Language information from manifest.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LanguageInfo {
    /// Language name (e.g., "Rust", "TypeScript")
    #[serde(default)]
    pub name: String,
    /// Number of lines
    #[serde(default)]
    pub lines: usize,
    /// Number of files
    #[serde(default)]
    pub files: usize,
}

/// Symbol count summary.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SymbolSummary {
    /// Total symbols
    #[serde(default)]
    pub total: usize,
    /// Functions/methods
    #[serde(default)]
    pub functions: usize,
    /// Classes/structs
    #[serde(default)]
    pub types: usize,
    /// Modules
    #[serde(default)]
    pub modules: usize,
}

/// Security findings summary.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SecuritySummary {
    /// Critical severity findings
    #[serde(default)]
    pub critical: usize,
    /// High severity findings
    #[serde(default)]
    pub high: usize,
    /// Medium severity findings
    #[serde(default)]
    pub medium: usize,
    /// Low severity findings
    #[serde(default)]
    pub low: usize,
}

impl SecuritySummary {
    /// Returns true if there are any findings.
    #[must_use]
    pub fn has_findings(&self) -> bool {
        self.critical > 0 || self.high > 0 || self.medium > 0 || self.low > 0
    }

    /// Returns the total number of findings.
    #[must_use]
    pub fn total(&self) -> usize {
        self.critical + self.high + self.medium + self.low
    }
}

/// Quality metrics summary.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QualitySummary {
    /// Average cyclomatic complexity
    #[serde(default)]
    pub avg_complexity: f64,
    /// Test coverage percentage (0-100)
    #[serde(default)]
    pub test_coverage: Option<f64>,
    /// Documentation coverage percentage (0-100)
    #[serde(default)]
    pub doc_coverage: Option<f64>,
}

/// Parsed CCG architecture (Layer 1).
///
/// Contains module structure and dependency information (~10-50KB).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CcgArchitecture {
    /// Module information
    #[serde(default)]
    pub modules: Vec<ModuleInfo>,
    /// Public API exports
    #[serde(default)]
    pub public_api: Vec<ExportInfo>,
    /// Dependency edges between modules
    #[serde(default)]
    pub dependency_graph: Vec<DependencyEdge>,
}

/// Module information from architecture.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModuleInfo {
    /// Module path (e.g., "src/api/mod.rs")
    #[serde(default)]
    pub path: String,
    /// Module name
    #[serde(default)]
    pub name: String,
    /// Number of public symbols
    #[serde(default)]
    pub public_symbols: usize,
    /// Number of private symbols
    #[serde(default)]
    pub private_symbols: usize,
}

/// Public API export information.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExportInfo {
    /// Symbol name
    #[serde(default)]
    pub name: String,
    /// Symbol kind (function, struct, enum, etc.)
    #[serde(default)]
    pub kind: String,
    /// Module where defined
    #[serde(default)]
    pub module: String,
    /// Optional signature
    #[serde(default)]
    pub signature: Option<String>,
}

/// Dependency edge between modules.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DependencyEdge {
    /// Source module
    #[serde(default)]
    pub from: String,
    /// Target module
    #[serde(default)]
    pub to: String,
    /// Import count
    #[serde(default)]
    pub weight: usize,
}

/// Parsed SPARQL symbol results.
#[derive(Debug, Clone, Default)]
pub struct SymbolResults {
    /// List of symbols with details
    pub symbols: Vec<SymbolDetail>,
}

/// Individual symbol detail from SPARQL query.
#[derive(Debug, Clone, Default)]
pub struct SymbolDetail {
    /// Symbol name
    pub name: String,
    /// Symbol kind
    pub kind: String,
    /// File path
    pub file: String,
    /// Line number
    pub line: usize,
    /// Optional signature
    pub signature: Option<String>,
}

/// Parses a CCG manifest from JSON response.
///
/// # Errors
///
/// Returns an error if the JSON is invalid or doesn't match the expected schema.
pub fn parse_ccg_manifest(json: &serde_json::Value) -> Result<CcgManifest, ParseError> {
    // Try to extract from common JSON-LD wrapper patterns
    let data = extract_data_from_jsonld(json);

    // Use serde to parse, with defaults for missing fields
    serde_json::from_value(data.clone()).map_err(|e| ParseError::InvalidJson(e.to_string()))
}

/// Parses a CCG architecture from JSON response.
///
/// # Errors
///
/// Returns an error if the JSON is invalid or doesn't match the expected schema.
pub fn parse_ccg_architecture(json: &serde_json::Value) -> Result<CcgArchitecture, ParseError> {
    let data = extract_data_from_jsonld(json);
    serde_json::from_value(data.clone()).map_err(|e| ParseError::InvalidJson(e.to_string()))
}

/// Extracts data from JSON-LD wrapper if present.
fn extract_data_from_jsonld(json: &serde_json::Value) -> &serde_json::Value {
    // Try common JSON-LD patterns
    if let Some(graph) = json.get("@graph") {
        if let Some(first) = graph.as_array().and_then(|arr| arr.first()) {
            return first;
        }
    }
    if let Some(data) = json.get("data") {
        return data;
    }
    if let Some(result) = json.get("result") {
        return result;
    }
    json
}

/// Error parsing CCG response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// Invalid JSON format
    InvalidJson(String),
    /// Missing required field
    MissingField(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidJson(msg) => write!(f, "invalid JSON: {msg}"),
            Self::MissingField(field) => write!(f, "missing required field: {field}"),
        }
    }
}

impl std::error::Error for ParseError {}

/// Renders a CCG manifest as Markdown.
///
/// Produces a compact ~500 byte summary from the ~2KB JSON-LD.
#[must_use]
pub fn render_manifest_markdown(manifest: &CcgManifest) -> String {
    let mut output = String::with_capacity(512);

    // Header
    output.push_str("# Repository Manifest\n\n");

    // Basic info
    if !manifest.repo_name.is_empty() {
        output.push_str(&format!("**Repository:** {}\n", manifest.repo_name));
    }
    if !manifest.commit.is_empty() {
        let short_commit = if manifest.commit.len() > 7 {
            &manifest.commit[..7]
        } else {
            &manifest.commit
        };
        output.push_str(&format!("**Commit:** {short_commit}\n"));
    }

    // Languages
    if !manifest.languages.is_empty() {
        output.push_str("\n## Languages\n\n");
        for lang in &manifest.languages {
            output.push_str(&format!(
                "- **{}**: {} lines ({} files)\n",
                lang.name, lang.lines, lang.files
            ));
        }
    }

    // Symbol counts
    output.push_str("\n## Symbols\n\n");
    output.push_str(&format!("- **Total:** {}\n", manifest.symbols.total));
    output.push_str(&format!(
        "- **Functions:** {}\n",
        manifest.symbols.functions
    ));
    output.push_str(&format!("- **Types:** {}\n", manifest.symbols.types));
    output.push_str(&format!("- **Modules:** {}\n", manifest.symbols.modules));

    // Security (only if non-zero)
    if manifest.security.has_findings() {
        output.push_str("\n## Security Findings\n\n");
        if manifest.security.critical > 0 {
            output.push_str(&format!("- **Critical:** {}\n", manifest.security.critical));
        }
        if manifest.security.high > 0 {
            output.push_str(&format!("- **High:** {}\n", manifest.security.high));
        }
        if manifest.security.medium > 0 {
            output.push_str(&format!("- **Medium:** {}\n", manifest.security.medium));
        }
        if manifest.security.low > 0 {
            output.push_str(&format!("- **Low:** {}\n", manifest.security.low));
        }
    }

    // Quality (optional)
    if manifest.quality.avg_complexity > 0.0
        || manifest.quality.test_coverage.is_some()
        || manifest.quality.doc_coverage.is_some()
    {
        output.push_str("\n## Quality\n\n");
        if manifest.quality.avg_complexity > 0.0 {
            output.push_str(&format!(
                "- **Avg Complexity:** {:.1}\n",
                manifest.quality.avg_complexity
            ));
        }
        if let Some(coverage) = manifest.quality.test_coverage {
            output.push_str(&format!("- **Test Coverage:** {coverage:.1}%\n"));
        }
        if let Some(coverage) = manifest.quality.doc_coverage {
            output.push_str(&format!("- **Doc Coverage:** {coverage:.1}%\n"));
        }
    }

    output
}

/// Renders a CCG architecture as Markdown.
///
/// Produces a structured view of modules, exports, and dependencies.
#[must_use]
pub fn render_architecture_markdown(arch: &CcgArchitecture) -> String {
    let mut output = String::with_capacity(2048);

    output.push_str("# Architecture\n\n");

    // Modules section
    if !arch.modules.is_empty() {
        output.push_str("## Modules\n\n");
        for module in &arch.modules {
            let symbols = module.public_symbols + module.private_symbols;
            output.push_str(&format!(
                "- **{}** (`{}`): {} symbols ({} public)\n",
                module.name, module.path, symbols, module.public_symbols
            ));
        }
    }

    // Public API / Exports section
    if !arch.public_api.is_empty() {
        output.push_str("\n## Public API\n\n");
        for export in &arch.public_api {
            if let Some(ref sig) = export.signature {
                output.push_str(&format!(
                    "- `{}` ({}): `{}`\n",
                    export.name, export.kind, sig
                ));
            } else {
                output.push_str(&format!("- `{}` ({})\n", export.name, export.kind));
            }
        }
    }

    // Dependencies section
    if !arch.dependency_graph.is_empty() {
        output.push_str("\n## Dependencies\n\n");
        for edge in &arch.dependency_graph {
            output.push_str(&format!("- {} → {}\n", edge.from, edge.to));
        }
    }

    output
}

/// Renders SPARQL symbol results as Markdown.
#[must_use]
pub fn render_symbols_markdown(results: &SymbolResults) -> String {
    let mut output = String::with_capacity(1024);

    output.push_str("# Symbols\n\n");

    if results.symbols.is_empty() {
        output.push_str("_No symbols found._\n");
        return output;
    }

    for symbol in &results.symbols {
        if let Some(ref sig) = symbol.signature {
            output.push_str(&format!(
                "- **{}** ({}): `{}`\n  - {}:{}\n",
                symbol.name, symbol.kind, sig, symbol.file, symbol.line
            ));
        } else {
            output.push_str(&format!(
                "- **{}** ({})\n  - {}:{}\n",
                symbol.name, symbol.kind, symbol.file, symbol.line
            ));
        }
    }

    output
}

/// Checks if content appears to be raw JSON rather than Markdown.
#[must_use]
pub fn looks_like_json(content: &str) -> bool {
    let trimmed = content.trim();
    (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
}

#[cfg(test)]
mod tests {
    use super::*;

    // =============================================================================
    // Module existence test
    // =============================================================================

    #[test]
    fn test_render_module_exists() {
        // Verify basic types are accessible
        let _manifest = CcgManifest::default();
        let _arch = CcgArchitecture::default();
    }

    // =============================================================================
    // CcgManifest parsing tests
    // =============================================================================

    #[test]
    fn test_parse_ccg_manifest_valid_json() {
        let json = serde_json::json!({
            "repo_name": "test-repo",
            "commit": "abc123def456",
            "languages": [
                {"name": "Rust", "lines": 10000, "files": 50}
            ],
            "symbols": {
                "total": 500,
                "functions": 300,
                "types": 100,
                "modules": 20
            },
            "security": {
                "critical": 0,
                "high": 1,
                "medium": 2,
                "low": 5
            },
            "quality": {
                "avg_complexity": 8.5,
                "test_coverage": 85.0
            }
        });

        let manifest = parse_ccg_manifest(&json).unwrap();

        assert_eq!(manifest.repo_name, "test-repo");
        assert_eq!(manifest.commit, "abc123def456");
        assert_eq!(manifest.languages.len(), 1);
        assert_eq!(manifest.languages[0].name, "Rust");
        assert_eq!(manifest.symbols.total, 500);
        assert_eq!(manifest.security.high, 1);
        assert!((manifest.quality.avg_complexity - 8.5).abs() < 0.01);
    }

    #[test]
    fn test_parse_ccg_manifest_handles_missing_fields() {
        let json = serde_json::json!({
            "repo_name": "minimal"
        });

        let manifest = parse_ccg_manifest(&json).unwrap();

        assert_eq!(manifest.repo_name, "minimal");
        assert!(manifest.commit.is_empty());
        assert!(manifest.languages.is_empty());
        assert_eq!(manifest.symbols.total, 0);
    }

    #[test]
    fn test_parse_ccg_manifest_handles_jsonld_wrapper() {
        let json = serde_json::json!({
            "@graph": [{
                "repo_name": "wrapped-repo",
                "commit": "xyz789"
            }]
        });

        let manifest = parse_ccg_manifest(&json).unwrap();
        assert_eq!(manifest.repo_name, "wrapped-repo");
    }

    // =============================================================================
    // CcgArchitecture parsing tests
    // =============================================================================

    #[test]
    fn test_parse_ccg_architecture_valid_json() {
        let json = serde_json::json!({
            "modules": [
                {"path": "src/lib.rs", "name": "lib", "public_symbols": 10, "private_symbols": 20}
            ],
            "public_api": [
                {"name": "run", "kind": "function", "module": "lib", "signature": "fn run() -> Result<()>"}
            ],
            "dependency_graph": [
                {"from": "api", "to": "core", "weight": 5}
            ]
        });

        let arch = parse_ccg_architecture(&json).unwrap();

        assert_eq!(arch.modules.len(), 1);
        assert_eq!(arch.modules[0].name, "lib");
        assert_eq!(arch.public_api.len(), 1);
        assert_eq!(arch.public_api[0].name, "run");
        assert_eq!(arch.dependency_graph.len(), 1);
        assert_eq!(arch.dependency_graph[0].from, "api");
    }

    #[test]
    fn test_parse_ccg_architecture_handles_missing_fields() {
        let json = serde_json::json!({
            "modules": []
        });

        let arch = parse_ccg_architecture(&json).unwrap();

        assert!(arch.modules.is_empty());
        assert!(arch.public_api.is_empty());
        assert!(arch.dependency_graph.is_empty());
    }

    // =============================================================================
    // Manifest Markdown rendering tests
    // =============================================================================

    #[test]
    fn test_manifest_markdown_includes_languages() {
        let manifest = CcgManifest {
            languages: vec![
                LanguageInfo {
                    name: "Rust".to_string(),
                    lines: 5000,
                    files: 25,
                },
                LanguageInfo {
                    name: "TOML".to_string(),
                    lines: 100,
                    files: 2,
                },
            ],
            ..Default::default()
        };

        let md = render_manifest_markdown(&manifest);

        assert!(md.contains("## Languages"));
        assert!(md.contains("**Rust**"));
        assert!(md.contains("5000 lines"));
        assert!(md.contains("TOML"));
    }

    #[test]
    fn test_manifest_markdown_includes_symbol_counts() {
        let manifest = CcgManifest {
            symbols: SymbolSummary {
                total: 100,
                functions: 60,
                types: 30,
                modules: 10,
            },
            ..Default::default()
        };

        let md = render_manifest_markdown(&manifest);

        assert!(md.contains("## Symbols"));
        assert!(md.contains("**Total:** 100"));
        assert!(md.contains("**Functions:** 60"));
        assert!(md.contains("**Types:** 30"));
        assert!(md.contains("**Modules:** 10"));
    }

    #[test]
    fn test_manifest_markdown_includes_security_if_nonzero() {
        let manifest = CcgManifest {
            security: SecuritySummary {
                critical: 0,
                high: 2,
                medium: 3,
                low: 5,
            },
            ..Default::default()
        };

        let md = render_manifest_markdown(&manifest);

        assert!(md.contains("## Security Findings"));
        assert!(md.contains("**High:** 2"));
        assert!(md.contains("**Medium:** 3"));
        assert!(md.contains("**Low:** 5"));
        // Critical is 0, so should not appear
        assert!(!md.contains("**Critical:**"));
    }

    #[test]
    fn test_manifest_markdown_excludes_security_if_zero() {
        let manifest = CcgManifest {
            security: SecuritySummary {
                critical: 0,
                high: 0,
                medium: 0,
                low: 0,
            },
            ..Default::default()
        };

        let md = render_manifest_markdown(&manifest);

        assert!(!md.contains("## Security Findings"));
    }

    // =============================================================================
    // Architecture Markdown rendering tests
    // =============================================================================

    #[test]
    fn test_architecture_markdown_includes_modules() {
        let arch = CcgArchitecture {
            modules: vec![
                ModuleInfo {
                    path: "src/api/mod.rs".to_string(),
                    name: "api".to_string(),
                    public_symbols: 15,
                    private_symbols: 10,
                },
                ModuleInfo {
                    path: "src/core/mod.rs".to_string(),
                    name: "core".to_string(),
                    public_symbols: 5,
                    private_symbols: 20,
                },
            ],
            ..Default::default()
        };

        let md = render_architecture_markdown(&arch);

        assert!(md.contains("## Modules"));
        assert!(md.contains("**api**"));
        assert!(md.contains("src/api/mod.rs"));
        assert!(md.contains("15 public"));
    }

    #[test]
    fn test_architecture_markdown_includes_exports() {
        let arch = CcgArchitecture {
            public_api: vec![
                ExportInfo {
                    name: "run".to_string(),
                    kind: "function".to_string(),
                    module: "lib".to_string(),
                    signature: Some("fn run() -> Result<()>".to_string()),
                },
                ExportInfo {
                    name: "Config".to_string(),
                    kind: "struct".to_string(),
                    module: "config".to_string(),
                    signature: None,
                },
            ],
            ..Default::default()
        };

        let md = render_architecture_markdown(&arch);

        assert!(md.contains("## Public API"));
        assert!(md.contains("`run` (function)"));
        assert!(md.contains("fn run() -> Result<()>"));
        assert!(md.contains("`Config` (struct)"));
    }

    #[test]
    fn test_architecture_markdown_includes_dependency_edges() {
        let arch = CcgArchitecture {
            dependency_graph: vec![
                DependencyEdge {
                    from: "api".to_string(),
                    to: "core".to_string(),
                    weight: 5,
                },
                DependencyEdge {
                    from: "core".to_string(),
                    to: "util".to_string(),
                    weight: 3,
                },
            ],
            ..Default::default()
        };

        let md = render_architecture_markdown(&arch);

        assert!(md.contains("## Dependencies"));
        assert!(md.contains("api → core"));
        assert!(md.contains("core → util"));
    }

    // =============================================================================
    // Symbols Markdown rendering tests
    // =============================================================================

    #[test]
    fn test_symbols_markdown_includes_signatures() {
        let results = SymbolResults {
            symbols: vec![
                SymbolDetail {
                    name: "process".to_string(),
                    kind: "function".to_string(),
                    file: "src/main.rs".to_string(),
                    line: 42,
                    signature: Some("fn process(input: &str) -> String".to_string()),
                },
                SymbolDetail {
                    name: "Handler".to_string(),
                    kind: "struct".to_string(),
                    file: "src/handler.rs".to_string(),
                    line: 10,
                    signature: None,
                },
            ],
        };

        let md = render_symbols_markdown(&results);

        assert!(md.contains("**process** (function)"));
        assert!(md.contains("fn process(input: &str) -> String"));
        assert!(md.contains("src/main.rs:42"));
        assert!(md.contains("**Handler** (struct)"));
    }

    #[test]
    fn test_symbols_markdown_empty() {
        let results = SymbolResults { symbols: vec![] };
        let md = render_symbols_markdown(&results);

        assert!(md.contains("_No symbols found._"));
    }

    // =============================================================================
    // JSON detection tests
    // =============================================================================

    #[test]
    fn test_manifest_rendered_as_markdown_not_json() {
        let manifest = CcgManifest {
            repo_name: "test".to_string(),
            ..Default::default()
        };

        let md = render_manifest_markdown(&manifest);

        assert!(!looks_like_json(&md), "Output should be Markdown, not JSON");
        assert!(md.starts_with('#'), "Markdown should start with heading");
    }

    #[test]
    fn test_architecture_rendered_as_markdown_not_json() {
        let arch = CcgArchitecture {
            modules: vec![ModuleInfo {
                name: "test".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };

        let md = render_architecture_markdown(&arch);

        assert!(!looks_like_json(&md), "Output should be Markdown, not JSON");
        assert!(md.starts_with('#'), "Markdown should start with heading");
    }

    #[test]
    fn test_looks_like_json() {
        assert!(looks_like_json(r#"{"key": "value"}"#));
        assert!(looks_like_json(r#"[1, 2, 3]"#));
        assert!(looks_like_json("  { } "));
        assert!(!looks_like_json("# Heading"));
        assert!(!looks_like_json("Some text"));
    }

    // =============================================================================
    // SecuritySummary tests
    // =============================================================================

    #[test]
    fn test_security_summary_has_findings() {
        let empty = SecuritySummary::default();
        assert!(!empty.has_findings());

        let with_low = SecuritySummary {
            low: 1,
            ..Default::default()
        };
        assert!(with_low.has_findings());

        let with_critical = SecuritySummary {
            critical: 1,
            ..Default::default()
        };
        assert!(with_critical.has_findings());
    }

    #[test]
    fn test_security_summary_total() {
        let summary = SecuritySummary {
            critical: 1,
            high: 2,
            medium: 3,
            low: 4,
        };
        assert_eq!(summary.total(), 10);
    }

    // =============================================================================
    // ParseError tests
    // =============================================================================

    #[test]
    fn test_parse_error_display() {
        let err = ParseError::InvalidJson("test error".to_string());
        assert!(err.to_string().contains("test error"));

        let err = ParseError::MissingField("repo_name".to_string());
        assert!(err.to_string().contains("repo_name"));
    }
}
