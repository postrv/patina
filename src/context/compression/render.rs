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

/// Parses a `get_project_structure` response into a [`CcgManifest`].
///
/// Extracts language, file count, and size information from the
/// narsil project structure tree output. Lines are estimated from
/// file sizes using ~40 bytes per line.
///
/// # Arguments
///
/// * `content` - The text output from narsil's `get_project_structure` tool
/// * `repo_name` - Repository name to set in the manifest
/// * `commit_hash` - Git commit hash for cache invalidation
///
/// # Returns
///
/// A `CcgManifest` populated with repository metadata extracted from the tree.
///
/// # Examples
///
/// ```ignore
/// let manifest = parse_project_structure_to_manifest(
///     "📁 repo/\n  🦀 main.rs (10.0KB)",
///     "my-repo",
///     "abc123",
/// );
/// assert_eq!(manifest.repo_name, "my-repo");
/// ```
#[must_use]
pub fn parse_project_structure_to_manifest(
    content: &str,
    repo_name: &str,
    commit_hash: &str,
) -> CcgManifest {
    let mut lang_counts: std::collections::HashMap<String, (usize, usize)> =
        std::collections::HashMap::new();

    for line in content.lines() {
        if let Some((ext, size_bytes)) = extract_file_info_from_tree_line(line) {
            let lang_name = language_name_from_extension(&ext);
            let entry = lang_counts.entry(lang_name).or_insert((0, 0));
            entry.0 += 1;
            entry.1 += size_bytes;
        }
    }

    let mut languages: Vec<LanguageInfo> = lang_counts
        .into_iter()
        .map(|(name, (files, total_size))| {
            let estimated_lines = if total_size > 0 { total_size / 40 } else { 0 };
            LanguageInfo {
                name,
                lines: estimated_lines,
                files,
            }
        })
        .collect();

    languages.sort_by(|a, b| b.files.cmp(&a.files));

    let total_files: usize = languages.iter().map(|l| l.files).sum();

    CcgManifest {
        repo_name: repo_name.to_string(),
        commit: commit_hash.to_string(),
        languages,
        symbols: SymbolSummary {
            total: total_files,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Extracts file extension and size from a narsil project structure tree line.
///
/// Lines have the format: `  🦀 main.rs (21.5KB)` or `  📝 README.md (12.5KB)`
///
/// Returns `None` for directory lines, headers, and non-file entries.
fn extract_file_info_from_tree_line(line: &str) -> Option<(String, usize)> {
    let trimmed = line.trim();

    // Skip directory entries, headers, code fences, and empty lines
    if trimmed.is_empty()
        || trimmed.starts_with("📁")
        || trimmed.starts_with('#')
        || trimmed.starts_with("```")
    {
        return None;
    }

    // Find file name: skip emoji prefix, look for filename with extension
    let after_emoji = skip_emoji_prefix(trimmed)?;

    // Extract filename (everything before the opening paren)
    let paren_pos = after_emoji.find('(')?;
    let filename = after_emoji[..paren_pos].trim();

    // Extract extension
    let dot_pos = filename.rfind('.')?;
    let ext = filename[dot_pos + 1..].to_lowercase();

    // Extract size from parentheses
    let close_paren = after_emoji[paren_pos..].find(')')?;
    let size_str = &after_emoji[paren_pos + 1..paren_pos + close_paren];
    let size_bytes = parse_size_str(size_str)?;

    Some((ext, size_bytes))
}

/// Skips emoji prefix characters to reach the filename.
///
/// Narsil uses various emoji prefixes: 🦀, 📝, 📋, ⚙️, 📄, 🌐, etc.
fn skip_emoji_prefix(s: &str) -> Option<&str> {
    // Find the first ASCII alphanumeric, dot, or underscore character
    let first_ascii = s.find(|c: char| c.is_ascii_alphanumeric() || c == '.' || c == '_')?;
    Some(&s[first_ascii..])
}

/// Parses a human-readable size string into bytes.
///
/// Supports formats: "21.5KB", "1.3MB", "911B"
fn parse_size_str(s: &str) -> Option<usize> {
    let s = s.trim();
    if let Some(num_str) = s.strip_suffix("KB") {
        let num: f64 = num_str.trim().parse().ok()?;
        Some((num * 1024.0) as usize)
    } else if let Some(num_str) = s.strip_suffix("MB") {
        let num: f64 = num_str.trim().parse().ok()?;
        Some((num * 1024.0 * 1024.0) as usize)
    } else if let Some(num_str) = s.strip_suffix('B') {
        let num: f64 = num_str.trim().parse().ok()?;
        Some(num as usize)
    } else {
        None
    }
}

/// Maps a file extension to a human-readable language name.
fn language_name_from_extension(ext: &str) -> String {
    match ext {
        "rs" => "Rust".to_string(),
        "toml" => "TOML".to_string(),
        "md" => "Markdown".to_string(),
        "json" => "JSON".to_string(),
        "sh" => "Shell".to_string(),
        "html" => "HTML".to_string(),
        "css" => "CSS".to_string(),
        "js" => "JavaScript".to_string(),
        "ts" => "TypeScript".to_string(),
        "tsx" => "TypeScript".to_string(),
        "jsx" => "JavaScript".to_string(),
        "py" => "Python".to_string(),
        "yml" | "yaml" => "YAML".to_string(),
        "rb" => "Ruby".to_string(),
        "go" => "Go".to_string(),
        "java" => "Java".to_string(),
        "c" | "h" => "C".to_string(),
        "cpp" | "cc" | "cxx" | "hpp" => "C++".to_string(),
        "lock" => "Lock".to_string(),
        "dot" => "Graphviz".to_string(),
        _ => ext.to_uppercase(),
    }
}

/// Parses a narsil `get_import_graph` response into a [`CcgArchitecture`].
///
/// Extracts module-level dependency information from the markdown table
/// returned by narsil's import graph tool. Each row becomes a [`ModuleInfo`]
/// with dependency counts mapped to symbol counts, and high-connectivity
/// files become [`DependencyEdge`] entries representing architectural weight.
///
/// # Arguments
///
/// * `content` - The text output from narsil's `get_import_graph` tool
/// * `repo_name` - Repository name for context
///
/// # Returns
///
/// A `CcgArchitecture` populated with module dependency information.
///
/// # Examples
///
/// ```ignore
/// let arch = parse_import_graph_to_architecture(
///     "| File | Dependencies | Dependents |\n|---|---|---|\n| src/main.rs | 5 | 0 |",
///     "my-repo",
/// );
/// assert!(!arch.modules.is_empty());
/// ```
#[must_use]
pub fn parse_import_graph_to_architecture(content: &str, repo_name: &str) -> CcgArchitecture {
    let mut modules = Vec::new();
    let mut dependency_graph = Vec::new();
    let mut seen_files: std::collections::HashSet<String> = std::collections::HashSet::new();

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip non-table rows: headers, separators, empty lines
        if trimmed.is_empty()
            || trimmed.starts_with('#')
            || trimmed.starts_with("|-")
            || !trimmed.starts_with('|')
        {
            continue;
        }

        let cols: Vec<&str> = trimmed.split('|').map(str::trim).collect();
        // Expected: ["", "File", "Dependencies", "Dependents", ""]
        // or data:  ["", "src/app/state.rs", "21", "8", ""]
        if cols.len() < 4 {
            continue;
        }

        let file_path = cols[1].trim();
        let deps_str = cols[2].trim();
        let dependents_str = cols[3].trim();

        // Skip header row
        if file_path == "File" || file_path.is_empty() {
            continue;
        }

        // Parse numeric columns
        let deps: usize = deps_str.parse().unwrap_or(0);
        let dependents: usize = dependents_str.parse().unwrap_or(0);

        // Deduplicate (narsil can return repeated entries)
        if !seen_files.insert(file_path.to_string()) {
            continue;
        }

        // Extract module name from file path
        let name = extract_module_name(file_path);

        modules.push(ModuleInfo {
            path: file_path.to_string(),
            name: name.clone(),
            public_symbols: dependents,
            private_symbols: deps,
        });

        // Create dependency edges for high-connectivity modules
        // (dependents > 0 means other modules import this one)
        if dependents > 0 {
            dependency_graph.push(DependencyEdge {
                from: format!("{repo_name} modules"),
                to: name,
                weight: dependents,
            });
        }
    }

    // Sort modules by total connectivity (most connected first)
    modules.sort_by(|a, b| {
        let total_a = a.public_symbols + a.private_symbols;
        let total_b = b.public_symbols + b.private_symbols;
        total_b.cmp(&total_a)
    });

    // Sort dependency edges by weight (most imported first)
    dependency_graph.sort_by(|a, b| b.weight.cmp(&a.weight));

    CcgArchitecture {
        modules,
        public_api: Vec::new(), // Not available from import graph alone
        dependency_graph,
    }
}

/// Extracts a module name from a file path.
///
/// Converts paths like `src/app/state.rs` to `app::state` and
/// `src/api/mod.rs` to `api`.
fn extract_module_name(path: &str) -> String {
    let path = path.strip_prefix("src/").unwrap_or(path);
    let path = path
        .strip_suffix(".rs")
        .or_else(|| path.strip_suffix('/'))
        .unwrap_or(path);

    // Remove mod.rs suffix to get the module name
    let path = path.strip_suffix("/mod").unwrap_or(path);

    path.replace('/', "::")
}

/// Renders a CCG architecture as Markdown within a token budget.
///
/// If the full architecture rendering exceeds the budget, modules are
/// progressively removed (least connected first) until the output fits.
/// The dependency graph section is dropped first if needed.
///
/// # Arguments
///
/// * `arch` - The architecture data to render
/// * `token_budget` - Maximum approximate tokens for the output
///
/// # Returns
///
/// Markdown string that fits within the token budget.
#[must_use]
pub fn render_architecture_within_budget(arch: &CcgArchitecture, token_budget: usize) -> String {
    use crate::context::compression::estimate_tokens;

    // Try full render first
    let full = render_architecture_markdown(arch);
    if estimate_tokens(&full) <= token_budget {
        return full;
    }

    // Try without dependency graph
    let reduced = CcgArchitecture {
        modules: arch.modules.clone(),
        public_api: arch.public_api.clone(),
        dependency_graph: Vec::new(),
    };
    let without_deps = render_architecture_markdown(&reduced);
    if estimate_tokens(&without_deps) <= token_budget {
        return without_deps;
    }

    // Progressively reduce modules (keep most connected)
    let mut module_count = arch.modules.len();
    while module_count > 0 {
        let truncated = CcgArchitecture {
            modules: arch.modules[..module_count].to_vec(),
            public_api: Vec::new(),
            dependency_graph: Vec::new(),
        };
        let rendered = render_architecture_markdown(&truncated);
        if estimate_tokens(&rendered) <= token_budget {
            return rendered;
        }
        module_count /= 2;
    }

    // Absolute minimum: just the header
    "# Architecture\n\n_Truncated: token budget too small._\n".to_string()
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

    // =============================================================================
    // parse_project_structure_to_manifest tests (Task 3.1)
    // =============================================================================

    #[test]
    fn test_parse_project_structure_sets_repo_name() {
        let manifest = parse_project_structure_to_manifest("", "my-repo", "abc123");
        assert_eq!(manifest.repo_name, "my-repo");
    }

    #[test]
    fn test_parse_project_structure_sets_commit_hash() {
        let manifest = parse_project_structure_to_manifest("", "repo", "abc123def456");
        assert_eq!(manifest.commit, "abc123def456");
    }

    #[test]
    fn test_parse_project_structure_handles_empty_content() {
        let manifest = parse_project_structure_to_manifest("", "repo", "hash");
        assert_eq!(manifest.repo_name, "repo");
        assert_eq!(manifest.commit, "hash");
        assert!(manifest.languages.is_empty());
    }

    #[test]
    fn test_parse_project_structure_extracts_rust_files() {
        let content = "# Project Structure: test-repo\n\n\
            ```\n\
            📁 test-repo/\n\
              📁 src/\n\
                🦀 main.rs (21.5KB)\n\
                🦀 lib.rs (911B)\n\
              ⚙️ Cargo.toml (2.0KB)\n\
            ```";
        let manifest = parse_project_structure_to_manifest(content, "test-repo", "abc123");

        let rust = manifest.languages.iter().find(|l| l.name == "Rust");
        assert!(rust.is_some(), "Should detect Rust language");
        assert_eq!(rust.unwrap().files, 2);
    }

    #[test]
    fn test_parse_project_structure_extracts_multiple_languages() {
        let content = "📁 test-repo/\n\
            🦀 main.rs (10.0KB)\n\
            📝 README.md (5.0KB)\n\
            📋 config.json (1.0KB)";
        let manifest = parse_project_structure_to_manifest(content, "test-repo", "abc123");
        assert!(
            manifest.languages.len() >= 3,
            "Should detect at least 3 languages, got {}",
            manifest.languages.len()
        );
    }

    #[test]
    fn test_parse_project_structure_estimates_lines_from_size() {
        let content = "📁 repo/\n  🦀 big_file.rs (40.0KB)";
        let manifest = parse_project_structure_to_manifest(content, "repo", "hash");

        let rust = manifest.languages.iter().find(|l| l.name == "Rust");
        assert!(rust.is_some());
        // 40KB at ~40 bytes/line ≈ 1024 lines
        let lines = rust.unwrap().lines;
        assert!(
            lines > 500,
            "Should estimate lines from file size, got {lines}"
        );
    }

    #[test]
    fn test_parse_project_structure_sorts_languages_by_file_count() {
        let content = "📁 repo/\n\
            🦀 a.rs (1.0KB)\n\
            🦀 b.rs (1.0KB)\n\
            🦀 c.rs (1.0KB)\n\
            📝 README.md (1.0KB)";
        let manifest = parse_project_structure_to_manifest(content, "repo", "hash");

        assert!(!manifest.languages.is_empty());
        assert_eq!(
            manifest.languages[0].name, "Rust",
            "Rust (3 files) should be first"
        );
    }

    #[test]
    fn test_parse_project_structure_sets_total_file_count() {
        let content = "📁 repo/\n\
            🦀 a.rs (1.0KB)\n\
            🦀 b.rs (1.0KB)\n\
            📝 README.md (1.0KB)";
        let manifest = parse_project_structure_to_manifest(content, "repo", "hash");
        assert_eq!(manifest.symbols.total, 3, "Total should count all files");
    }

    #[test]
    fn test_parse_project_structure_skips_directories() {
        let content = "📁 repo/\n  📁 src/\n  📁 tests/";
        let manifest = parse_project_structure_to_manifest(content, "repo", "hash");
        assert!(
            manifest.languages.is_empty(),
            "Directories should not be counted as files"
        );
    }

    // =============================================================================
    // Helper function tests (Task 3.1)
    // =============================================================================

    #[test]
    fn test_extract_file_info_from_rust_file() {
        let result = extract_file_info_from_tree_line("    🦀 main.rs (21.5KB)");
        assert!(result.is_some());
        let (ext, size) = result.unwrap();
        assert_eq!(ext, "rs");
        // 21.5 * 1024 = 22016
        assert!(
            size > 20000 && size < 23000,
            "Size should be ~22016, got {size}"
        );
    }

    #[test]
    fn test_extract_file_info_from_markdown_file() {
        let result = extract_file_info_from_tree_line("  📝 README.md (12.5KB)");
        assert!(result.is_some());
        let (ext, _) = result.unwrap();
        assert_eq!(ext, "md");
    }

    #[test]
    fn test_extract_file_info_from_byte_size() {
        let result = extract_file_info_from_tree_line("  🦀 lib.rs (911B)");
        assert!(result.is_some());
        let (ext, size) = result.unwrap();
        assert_eq!(ext, "rs");
        assert_eq!(size, 911);
    }

    #[test]
    fn test_extract_file_info_from_megabyte_size() {
        let result = extract_file_info_from_tree_line("  🌐 report.html (1.5MB)");
        assert!(result.is_some());
        let (ext, size) = result.unwrap();
        assert_eq!(ext, "html");
        // 1.5 * 1024 * 1024 = 1572864
        assert!(size > 1500000 && size < 1600000);
    }

    #[test]
    fn test_extract_file_info_skips_directory() {
        assert!(extract_file_info_from_tree_line("  📁 src/").is_none());
    }

    #[test]
    fn test_extract_file_info_skips_header() {
        assert!(extract_file_info_from_tree_line("# Project Structure: repo").is_none());
    }

    #[test]
    fn test_extract_file_info_skips_empty() {
        assert!(extract_file_info_from_tree_line("").is_none());
    }

    #[test]
    fn test_extract_file_info_skips_code_fence() {
        assert!(extract_file_info_from_tree_line("```").is_none());
    }

    #[test]
    fn test_parse_size_str_kilobytes() {
        assert_eq!(parse_size_str("21.5KB"), Some(22016));
        assert_eq!(parse_size_str("1.0KB"), Some(1024));
    }

    #[test]
    fn test_parse_size_str_bytes() {
        assert_eq!(parse_size_str("911B"), Some(911));
    }

    #[test]
    fn test_parse_size_str_megabytes() {
        let size = parse_size_str("1.0MB").unwrap();
        assert_eq!(size, 1048576);
    }

    #[test]
    fn test_parse_size_str_invalid() {
        assert!(parse_size_str("abc").is_none());
        assert!(parse_size_str("").is_none());
    }

    #[test]
    fn test_language_name_from_extension() {
        assert_eq!(language_name_from_extension("rs"), "Rust");
        assert_eq!(language_name_from_extension("py"), "Python");
        assert_eq!(language_name_from_extension("ts"), "TypeScript");
        assert_eq!(language_name_from_extension("md"), "Markdown");
        assert_eq!(language_name_from_extension("toml"), "TOML");
        assert_eq!(language_name_from_extension("xyz"), "XYZ");
    }

    // =============================================================================
    // ParseError tests
    // =============================================================================

    // =============================================================================
    // parse_import_graph_to_architecture tests (Task 3.2)
    // =============================================================================

    #[test]
    fn test_parse_import_graph_extracts_modules() {
        let content = "\
# Import Graph\n\
\n\
## Repository Import Summary\n\
\n\
| File | Dependencies | Dependents |\n\
|------|--------------|------------|\n\
| src/app/state.rs | 21 | 8 |\n\
| src/api/mod.rs | 10 | 15 |\n\
| src/main.rs | 5 | 0 |";

        let arch = parse_import_graph_to_architecture(content, "test-repo");

        assert_eq!(arch.modules.len(), 3, "Should extract 3 modules");

        let state_mod = arch.modules.iter().find(|m| m.path == "src/app/state.rs");
        assert!(state_mod.is_some(), "Should find app::state module");
        let state_mod = state_mod.unwrap();
        assert_eq!(
            state_mod.private_symbols, 21,
            "Dependencies should map to private_symbols"
        );
        assert_eq!(
            state_mod.public_symbols, 8,
            "Dependents should map to public_symbols"
        );
    }

    #[test]
    fn test_parse_import_graph_extracts_dependencies() {
        let content = "\
| File | Dependencies | Dependents |\n\
|------|--------------|------------|\n\
| src/app/state.rs | 21 | 8 |\n\
| src/api/mod.rs | 10 | 15 |";

        let arch = parse_import_graph_to_architecture(content, "test-repo");

        assert!(
            !arch.dependency_graph.is_empty(),
            "Should create dependency edges for modules with dependents > 0"
        );

        let api_edge = arch.dependency_graph.iter().find(|e| e.to == "api");
        assert!(api_edge.is_some(), "Should have edge for api module");
        assert_eq!(api_edge.unwrap().weight, 15);
    }

    #[test]
    fn test_parse_import_graph_handles_empty_content() {
        let arch = parse_import_graph_to_architecture("", "repo");
        assert!(arch.modules.is_empty());
        assert!(arch.dependency_graph.is_empty());
        assert!(arch.public_api.is_empty());
    }

    #[test]
    fn test_parse_import_graph_deduplicates_entries() {
        let content = "\
| File | Dependencies | Dependents |\n\
|------|--------------|------------|\n\
| src/app/state.rs | 21 | 8 |\n\
| src/app/state.rs | 21 | 8 |\n\
| src/app/state.rs | 21 | 8 |";

        let arch = parse_import_graph_to_architecture(content, "repo");

        assert_eq!(arch.modules.len(), 1, "Should deduplicate repeated entries");
    }

    #[test]
    fn test_parse_import_graph_sorts_by_connectivity() {
        let content = "\
| File | Dependencies | Dependents |\n\
|------|--------------|------------|\n\
| src/small.rs | 1 | 1 |\n\
| src/big.rs | 20 | 15 |\n\
| src/medium.rs | 5 | 5 |";

        let arch = parse_import_graph_to_architecture(content, "repo");

        assert_eq!(
            arch.modules[0].path, "src/big.rs",
            "Most connected module should be first"
        );
        assert_eq!(
            arch.modules[2].path, "src/small.rs",
            "Least connected module should be last"
        );
    }

    #[test]
    fn test_parse_import_graph_module_names() {
        let content = "\
| File | Dependencies | Dependents |\n\
|------|--------------|------------|\n\
| src/app/mod.rs | 5 | 3 |\n\
| src/api/client.rs | 2 | 1 |\n\
| src/main.rs | 10 | 0 |";

        let arch = parse_import_graph_to_architecture(content, "repo");

        let names: Vec<&str> = arch.modules.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"app"), "src/app/mod.rs should become 'app'");
        assert!(
            names.contains(&"api::client"),
            "src/api/client.rs should become 'api::client'"
        );
        assert!(names.contains(&"main"), "src/main.rs should become 'main'");
    }

    #[test]
    fn test_parse_import_graph_skips_header_row() {
        let content = "\
| File | Dependencies | Dependents |\n\
|------|--------------|------------|\n\
| src/main.rs | 5 | 0 |";

        let arch = parse_import_graph_to_architecture(content, "repo");

        assert_eq!(arch.modules.len(), 1);
        assert_eq!(arch.modules[0].path, "src/main.rs");
    }

    // =============================================================================
    // render_architecture_within_budget tests (Task 3.2 REFACTOR)
    // =============================================================================

    #[test]
    fn test_render_architecture_within_budget_returns_full_when_fits() {
        let arch = CcgArchitecture {
            modules: vec![ModuleInfo {
                path: "src/main.rs".to_string(),
                name: "main".to_string(),
                public_symbols: 0,
                private_symbols: 5,
            }],
            dependency_graph: vec![DependencyEdge {
                from: "a".to_string(),
                to: "b".to_string(),
                weight: 1,
            }],
            ..Default::default()
        };

        let result = render_architecture_within_budget(&arch, 10000);
        let full = render_architecture_markdown(&arch);
        assert_eq!(result, full, "Should return full render when within budget");
    }

    #[test]
    fn test_render_architecture_within_budget_truncates_when_over() {
        use crate::context::compression::estimate_tokens;

        // Create a large architecture
        let modules: Vec<ModuleInfo> = (0..100)
            .map(|i| ModuleInfo {
                path: format!("src/module_{i}/very_long_name_{i}.rs"),
                name: format!("module_{i}::very_long_name_{i}"),
                public_symbols: i,
                private_symbols: 100 - i,
            })
            .collect();

        let arch = CcgArchitecture {
            modules,
            dependency_graph: (0..50)
                .map(|i| DependencyEdge {
                    from: format!("module_{i}"),
                    to: format!("module_{}", i + 1),
                    weight: i,
                })
                .collect(),
            ..Default::default()
        };

        let full_tokens = estimate_tokens(&render_architecture_markdown(&arch));
        let budget = full_tokens / 4; // Quarter of what's needed

        let result = render_architecture_within_budget(&arch, budget);
        let result_tokens = estimate_tokens(&result);

        assert!(
            result_tokens <= budget,
            "Result ({result_tokens} tokens) should fit in budget ({budget} tokens)"
        );
        assert!(
            result.contains("Architecture"),
            "Truncated result should still have header"
        );
    }

    #[test]
    fn test_render_architecture_within_budget_tiny_budget() {
        let arch = CcgArchitecture {
            modules: vec![ModuleInfo {
                path: "src/main.rs".to_string(),
                name: "main".to_string(),
                public_symbols: 0,
                private_symbols: 5,
            }],
            ..Default::default()
        };

        let result = render_architecture_within_budget(&arch, 1); // Impossibly small
        assert!(
            result.contains("Truncated"),
            "Minimum output should indicate truncation"
        );
    }

    #[test]
    fn test_extract_module_name_from_path() {
        assert_eq!(extract_module_name("src/app/state.rs"), "app::state");
        assert_eq!(extract_module_name("src/api/mod.rs"), "api");
        assert_eq!(extract_module_name("src/main.rs"), "main");
        assert_eq!(extract_module_name("src/lib.rs"), "lib");
        assert_eq!(
            extract_module_name("src/app/handlers/keyboard.rs"),
            "app::handlers::keyboard"
        );
    }

    #[test]
    fn test_parse_error_display() {
        let err = ParseError::InvalidJson("test error".to_string());
        assert!(err.to_string().contains("test error"));

        let err = ParseError::MissingField("repo_name".to_string());
        assert!(err.to_string().contains("repo_name"));
    }
}
