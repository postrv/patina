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
mod metrics;
mod orchestrator;
mod types;

pub use cache::{CacheKey, CachedResult, ResultCache, DEFAULT_CACHE_TTL, DEFAULT_MAX_ENTRIES};
pub use metrics::{CompressionMetrics, MetricsSummary};
pub use orchestrator::{
    CompressionOrchestrator, DEFAULT_ORCHESTRATOR_MAX_ENTRIES, DEFAULT_ORCHESTRATOR_TTL,
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
