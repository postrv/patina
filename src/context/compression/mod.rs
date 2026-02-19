//! Smart context compression for optimal token usage.
//!
//! This module provides intelligent context compression that adapts to available
//! narsil-mcp capabilities. When CCG (Code Comprehension Graph) tools are available,
//! it uses the rich semantic graph. Otherwise, it falls back to core narsil tools.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────┐
//! │            CompressionOrchestrator                   │
//! │                                                      │
//! │  has_capability(CcgGraph)?                          │
//! │       ├── YES ──→ CCG Backend                       │
//! │       │           get_ccg_manifest (~2KB)            │
//! │       │           export_ccg_architecture (~50KB)    │
//! │       │           query_ccg (SPARQL, scoped)         │
//! │       │                                              │
//! │       └── NO  ──→ Fallback Backend                  │
//! │                   find_symbols + get_dependencies    │
//! │                   + get_export_map + get_file        │
//! │                                                      │
//! │  Both paths ──→ Cache ──→ Markdown Rendering        │
//! └─────────────────────────────────────────────────────┘
//! ```
//!
//! # Compression Levels
//!
//! | Level | CCG Path | Fallback Path | Size |
//! |-------|----------|---------------|------|
//! | Manifest | `get_ccg_manifest` | `find_symbols` + status | ~2 KB |
//! | Architecture | `export_ccg_architecture` | `find_symbols` + deps | ~10-50 KB |
//! | SymbolDetail | `query_ccg` (SPARQL) | `get_symbol_definition` | Variable |
//! | Full | `get_file` | `get_file` | Full size |
//!
//! # Example
//!
//! ```ignore
//! use patina::context::compression::{CompressionLevel, CompressionResult, ContextSource};
//!
//! // Get manifest-level context (most compressed)
//! let result = orchestrator.get_context(CompressionLevel::Manifest).await?;
//! println!("Got {} tokens from {}", result.tokens_approx(), result.source());
//! ```

mod cache;
mod ccg_backend;
mod fallback_backend;
mod metrics;
mod orchestrator;
mod render;
mod types;

pub use cache::{CacheKey, CachedResult, ResultCache, DEFAULT_CACHE_TTL, DEFAULT_MAX_ENTRIES};
pub use ccg_backend::{
    has_limit_clause, parse_mcp_tool_response, CcgBackend, CcgFetchError, CcgResponseError,
    DEFAULT_SPARQL_LIMIT,
};
pub use fallback_backend::{
    extract_repo_hash, extract_symbol_counts, FallbackBackend, SymbolCounts, DEFAULT_FALLBACK_LIMIT,
};
pub use metrics::{
    CompactionMetrics, CompactionMetricsSummary, CompressionMetrics, MetricsSummary,
};
pub use orchestrator::{
    BudgetAllocation, BuildContextResult, CompressionOrchestrator,
    DEFAULT_ARCHITECTURE_TOKEN_BUDGET, DEFAULT_MANIFEST_TOKEN_BUDGET,
    DEFAULT_ORCHESTRATOR_MAX_ENTRIES, DEFAULT_ORCHESTRATOR_TTL, DEFAULT_SYMBOL_TOKEN_BUDGET,
};
pub use render::{
    looks_like_json, parse_ccg_architecture, parse_ccg_manifest, parse_find_symbols_response,
    parse_import_graph_to_architecture, parse_project_structure_to_manifest,
    render_architecture_markdown, render_architecture_within_budget, render_manifest_markdown,
    render_symbols_markdown, render_symbols_within_budget, CcgArchitecture, CcgManifest,
    DependencyEdge, ExportInfo, LanguageInfo, ModuleInfo, ParseError, QualitySummary,
    SecuritySummary, SymbolDetail, SymbolResults, SymbolSummary,
};
pub use types::{estimate_tokens, CompressionLevel, CompressionResult, ContextSource};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compression_module_exports() {
        // Verify all public types are accessible
        let _level = CompressionLevel::Manifest;
        let _source = ContextSource::Cache;
        let _result = CompressionResult::new(
            "test".to_string(),
            CompressionLevel::Manifest,
            ContextSource::Cache,
        );

        // Verify estimate_tokens is exported
        let _tokens = estimate_tokens("test");
    }
}
