//! Core types for smart context compression.
//!
//! This module provides the fundamental types used throughout the compression system:
//! - [`CompressionLevel`]: The granularity of context compression
//! - [`ContextSource`]: How the context was obtained
//! - [`CompressionResult`]: The result of a compression operation

use std::cmp::Ordering;

/// The granularity level of context compression.
///
/// Compression levels are ordered from most compressed (Manifest) to
/// least compressed (Full). Each level provides progressively more detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompressionLevel {
    /// Minimal metadata (~2KB): languages, symbol counts, security summary
    Manifest,
    /// Module structure (~10-50KB): modules, exports, dependencies
    Architecture,
    /// Symbol details (variable): specific symbol definitions and context
    SymbolDetail,
    /// Full file contents: no compression
    Full,
}

impl CompressionLevel {
    /// Returns the compression levels in order from most to least compressed.
    #[must_use]
    pub fn all() -> &'static [CompressionLevel] {
        &[
            Self::Manifest,
            Self::Architecture,
            Self::SymbolDetail,
            Self::Full,
        ]
    }

    /// Returns the ordering index (0 = most compressed, 3 = least compressed).
    #[must_use]
    pub fn order(&self) -> usize {
        match self {
            Self::Manifest => 0,
            Self::Architecture => 1,
            Self::SymbolDetail => 2,
            Self::Full => 3,
        }
    }
}

impl PartialOrd for CompressionLevel {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CompressionLevel {
    fn cmp(&self, other: &Self) -> Ordering {
        self.order().cmp(&other.order())
    }
}

/// How the context was obtained.
///
/// Tracks the source of context data for debugging and metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContextSource {
    /// Retrieved from cache
    Cache,
    /// Layer 0 CCG (manifest)
    CcgLayer0,
    /// Layer 1 CCG (architecture)
    CcgLayer1,
    /// CCG SPARQL query result
    CcgSparql,
    /// Constructed from fallback tools
    Constructed,
    /// Direct tool call (get_symbol_definition, etc.)
    DirectTool,
    /// Direct file read
    DirectFile,
}

impl std::fmt::Display for ContextSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cache => write!(f, "cache"),
            Self::CcgLayer0 => write!(f, "ccg-layer0"),
            Self::CcgLayer1 => write!(f, "ccg-layer1"),
            Self::CcgSparql => write!(f, "ccg-sparql"),
            Self::Constructed => write!(f, "constructed"),
            Self::DirectTool => write!(f, "direct-tool"),
            Self::DirectFile => write!(f, "direct-file"),
        }
    }
}

/// The result of a context compression operation.
#[derive(Debug, Clone)]
pub struct CompressionResult {
    /// The compressed/rendered content as Markdown
    content: String,
    /// The compression level used
    level: CompressionLevel,
    /// How the content was obtained
    source: ContextSource,
    /// Approximate token count (estimated at ~4 chars/token)
    tokens_approx: usize,
}

impl CompressionResult {
    /// Creates a new compression result.
    ///
    /// Token count is automatically estimated from content length.
    #[must_use]
    pub fn new(content: String, level: CompressionLevel, source: ContextSource) -> Self {
        let tokens_approx = estimate_tokens(&content);
        Self {
            content,
            level,
            source,
            tokens_approx,
        }
    }

    /// Creates a new compression result with an explicit token count.
    #[must_use]
    pub fn with_tokens(
        content: String,
        level: CompressionLevel,
        source: ContextSource,
        tokens_approx: usize,
    ) -> Self {
        Self {
            content,
            level,
            source,
            tokens_approx,
        }
    }

    /// Returns the compressed content.
    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }

    /// Returns the compression level.
    #[must_use]
    pub fn level(&self) -> CompressionLevel {
        self.level
    }

    /// Returns how the content was obtained.
    #[must_use]
    pub fn source(&self) -> ContextSource {
        self.source
    }

    /// Returns the approximate token count.
    #[must_use]
    pub fn tokens_approx(&self) -> usize {
        self.tokens_approx
    }

    /// Returns the content length in bytes.
    #[must_use]
    pub fn byte_len(&self) -> usize {
        self.content.len()
    }
}

/// Estimates token count from content.
///
/// Uses a simple heuristic of ~4 characters per token, which is
/// a reasonable approximation for English text and code.
#[must_use]
pub fn estimate_tokens(content: &str) -> usize {
    // ~4 chars per token is a reasonable approximation
    // Add 1 to avoid returning 0 for very short content
    content.len().saturating_add(3) / 4
}

#[cfg(test)]
mod tests {
    use super::*;

    // =============================================================================
    // CompressionLevel tests
    // =============================================================================

    #[test]
    fn test_compression_level_ordering() {
        assert!(CompressionLevel::Manifest < CompressionLevel::Architecture);
        assert!(CompressionLevel::Architecture < CompressionLevel::SymbolDetail);
        assert!(CompressionLevel::SymbolDetail < CompressionLevel::Full);
    }

    #[test]
    fn test_compression_level_order_values() {
        assert_eq!(CompressionLevel::Manifest.order(), 0);
        assert_eq!(CompressionLevel::Architecture.order(), 1);
        assert_eq!(CompressionLevel::SymbolDetail.order(), 2);
        assert_eq!(CompressionLevel::Full.order(), 3);
    }

    #[test]
    fn test_compression_level_all() {
        let all = CompressionLevel::all();
        assert_eq!(all.len(), 4);
        assert_eq!(all[0], CompressionLevel::Manifest);
        assert_eq!(all[3], CompressionLevel::Full);
    }

    #[test]
    fn test_compression_level_equality() {
        assert_eq!(CompressionLevel::Manifest, CompressionLevel::Manifest);
        assert_ne!(CompressionLevel::Manifest, CompressionLevel::Full);
    }

    // =============================================================================
    // ContextSource tests
    // =============================================================================

    #[test]
    fn test_context_source_debug_impl() {
        // Test that Debug is implemented and produces reasonable output
        let debug_str = format!("{:?}", ContextSource::Cache);
        assert!(debug_str.contains("Cache"));
    }

    #[test]
    fn test_context_source_display() {
        assert_eq!(ContextSource::Cache.to_string(), "cache");
        assert_eq!(ContextSource::CcgLayer0.to_string(), "ccg-layer0");
        assert_eq!(ContextSource::CcgLayer1.to_string(), "ccg-layer1");
        assert_eq!(ContextSource::CcgSparql.to_string(), "ccg-sparql");
        assert_eq!(ContextSource::Constructed.to_string(), "constructed");
        assert_eq!(ContextSource::DirectTool.to_string(), "direct-tool");
        assert_eq!(ContextSource::DirectFile.to_string(), "direct-file");
    }

    #[test]
    fn test_context_source_equality() {
        assert_eq!(ContextSource::Cache, ContextSource::Cache);
        assert_ne!(ContextSource::Cache, ContextSource::CcgLayer0);
    }

    // =============================================================================
    // CompressionResult tests
    // =============================================================================

    #[test]
    fn test_compression_result_creation() {
        let result = CompressionResult::new(
            "# Test Content\n\nSome markdown.".to_string(),
            CompressionLevel::Architecture,
            ContextSource::CcgLayer1,
        );

        assert!(!result.content().is_empty());
        assert_eq!(result.level(), CompressionLevel::Architecture);
        assert_eq!(result.source(), ContextSource::CcgLayer1);
        assert!(result.tokens_approx() > 0);
    }

    #[test]
    fn test_compression_result_with_tokens() {
        let result = CompressionResult::with_tokens(
            "content".to_string(),
            CompressionLevel::Manifest,
            ContextSource::Cache,
            100,
        );

        assert_eq!(result.tokens_approx(), 100);
    }

    #[test]
    fn test_compression_result_byte_len() {
        let content = "Hello, world!";
        let result = CompressionResult::new(
            content.to_string(),
            CompressionLevel::Full,
            ContextSource::DirectFile,
        );

        assert_eq!(result.byte_len(), content.len());
    }

    // =============================================================================
    // Token estimation tests
    // =============================================================================

    #[test]
    fn test_estimate_tokens_reasonable_for_code() {
        let code = r#"
fn main() {
    println!("Hello, world!");
}
"#;
        let tokens = estimate_tokens(code);
        // Should be roughly length / 4
        assert!(tokens > 0);
        assert!(tokens <= code.len()); // Can't have more tokens than chars
    }

    #[test]
    fn test_estimate_tokens_empty() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn test_estimate_tokens_short() {
        // Very short strings should still give reasonable estimates
        assert!(estimate_tokens("hi") > 0);
    }
}
