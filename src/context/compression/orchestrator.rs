//! Compression orchestrator for routing to CCG or fallback backends.
//!
//! The orchestrator detects available narsil capabilities and routes
//! context requests to the appropriate backend:
//! - CCG backend when `CcgGraph` capability is available
//! - Fallback backend using core narsil tools otherwise
//!
//! # Example
//!
//! ```ignore
//! use patina::context::compression::{CompressionOrchestrator, CompressionLevel};
//! use patina::narsil::NarsilIntegration;
//!
//! let integration = NarsilIntegration::from_tool_names(&tools, &path);
//! let orchestrator = CompressionOrchestrator::new(&integration);
//!
//! // Get compressed context
//! let result = orchestrator.get_context(CompressionLevel::Manifest).await?;
//! println!("Got {} tokens from {}", result.tokens_approx(), result.source());
//! ```

use crate::context::compression::{
    CacheKey, CompressionLevel, CompressionMetrics, CompressionResult, ContextSource, ResultCache,
};
use crate::narsil::{NarsilCapabilities, NarsilCapability};
use std::sync::Arc;
use std::time::Duration;

/// Default TTL for orchestrator cache (5 minutes).
pub const DEFAULT_ORCHESTRATOR_TTL: Duration = Duration::from_secs(300);

/// Default maximum cache entries for orchestrator.
pub const DEFAULT_ORCHESTRATOR_MAX_ENTRIES: usize = 100;

/// Routes compression requests to CCG or fallback backends.
///
/// The orchestrator checks for the `CcgGraph` capability to determine
/// which backend to use. Results are cached with hash-based invalidation.
#[derive(Debug)]
pub struct CompressionOrchestrator {
    /// Detected narsil capabilities
    capabilities: NarsilCapabilities,
    /// Result cache with hash-based invalidation
    cache: Arc<ResultCache>,
    /// Metrics for monitoring
    metrics: Arc<CompressionMetrics>,
    /// Repository name for MCP calls
    repo_name: String,
}

impl CompressionOrchestrator {
    /// Creates a new orchestrator from narsil capabilities.
    ///
    /// Uses default TTL and cache size.
    #[must_use]
    pub fn new(capabilities: NarsilCapabilities, repo_name: impl Into<String>) -> Self {
        Self::with_config(
            capabilities,
            repo_name,
            DEFAULT_ORCHESTRATOR_TTL,
            DEFAULT_ORCHESTRATOR_MAX_ENTRIES,
        )
    }

    /// Creates a new orchestrator with custom configuration.
    #[must_use]
    pub fn with_config(
        capabilities: NarsilCapabilities,
        repo_name: impl Into<String>,
        ttl: Duration,
        max_entries: usize,
    ) -> Self {
        Self {
            capabilities,
            cache: Arc::new(ResultCache::with_config(ttl, max_entries)),
            metrics: Arc::new(CompressionMetrics::new()),
            repo_name: repo_name.into(),
        }
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

    /// Returns the metrics.
    #[must_use]
    pub fn metrics(&self) -> &CompressionMetrics {
        &self.metrics
    }

    /// Returns true if CCG backend should be used.
    #[must_use]
    pub fn should_use_ccg(&self) -> bool {
        self.capabilities.has(NarsilCapability::CcgGraph)
    }

    /// Returns true if fallback backend should be used.
    #[must_use]
    pub fn should_use_fallback(&self) -> bool {
        !self.should_use_ccg()
    }

    /// Gets cached context if available and valid.
    ///
    /// Returns `None` if no cached result exists or if the cache is invalid
    /// (hash changed or TTL expired).
    pub fn get_cached(&self, level: CompressionLevel, repo_hash: &str) -> Option<CompressionResult> {
        let key = CacheKey::global(level);
        if let Some(result) = self.cache.get(&key, repo_hash) {
            self.metrics.record_cache_hit();
            Some(result)
        } else {
            self.metrics.record_cache_miss();
            None
        }
    }

    /// Stores a result in the cache.
    pub fn cache_result(&self, level: CompressionLevel, result: &CompressionResult, repo_hash: &str) {
        use crate::context::compression::CachedResult;

        let key = CacheKey::global(level);
        let cached = CachedResult::new(
            result.content().to_string(),
            result.tokens_approx(),
            repo_hash.to_string(),
            level,
        );
        self.cache.set(key, cached);
    }

    /// Creates a compression result for the manifest level.
    ///
    /// Uses CCG if available, otherwise falls back to core tools.
    /// This is a placeholder - actual implementation will call backends.
    #[must_use]
    pub fn create_manifest_result(&self, content: String) -> CompressionResult {
        let source = if self.should_use_ccg() {
            self.metrics.record_ccg_call();
            ContextSource::CcgLayer0
        } else {
            self.metrics.record_fallback_call();
            ContextSource::Constructed
        };
        CompressionResult::new(content, CompressionLevel::Manifest, source)
    }

    /// Creates a compression result for the architecture level.
    ///
    /// Uses CCG if available, otherwise falls back to core tools.
    /// This is a placeholder - actual implementation will call backends.
    #[must_use]
    pub fn create_architecture_result(&self, content: String) -> CompressionResult {
        let source = if self.should_use_ccg() {
            self.metrics.record_ccg_call();
            ContextSource::CcgLayer1
        } else {
            self.metrics.record_fallback_call();
            ContextSource::Constructed
        };
        CompressionResult::new(content, CompressionLevel::Architecture, source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::narsil::NarsilCapabilities;

    fn create_capabilities_with_ccg() -> NarsilCapabilities {
        let tools = vec![
            "get_ccg_manifest".to_string(),
            "export_ccg_architecture".to_string(),
            "query_ccg".to_string(),
            "find_symbols".to_string(),
            "get_dependencies".to_string(),
        ];
        NarsilCapabilities::from_tools(&tools)
    }

    fn create_capabilities_without_ccg() -> NarsilCapabilities {
        let tools = vec![
            "find_symbols".to_string(),
            "get_dependencies".to_string(),
            "get_export_map".to_string(),
            "get_incremental_status".to_string(),
        ];
        NarsilCapabilities::from_tools(&tools)
    }

    #[test]
    fn test_orchestrator_new() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        assert_eq!(orchestrator.repo_name(), "test-repo");
        assert!(orchestrator.capabilities().has(NarsilCapability::CcgGraph));
    }

    #[test]
    fn test_orchestrator_with_config() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::with_config(
            caps,
            "my-repo",
            Duration::from_secs(60),
            50,
        );

        assert_eq!(orchestrator.repo_name(), "my-repo");
        assert!(!orchestrator.should_use_ccg());
    }

    #[test]
    fn test_routes_to_ccg_when_capability_present() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        assert!(orchestrator.should_use_ccg());
        assert!(!orchestrator.should_use_fallback());
    }

    #[test]
    fn test_routes_to_fallback_when_capability_absent() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        assert!(!orchestrator.should_use_ccg());
        assert!(orchestrator.should_use_fallback());
    }

    #[test]
    fn test_create_manifest_result_ccg_path() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        let result = orchestrator.create_manifest_result("# Manifest".to_string());

        assert_eq!(result.level(), CompressionLevel::Manifest);
        assert_eq!(result.source(), ContextSource::CcgLayer0);
        assert_eq!(orchestrator.metrics().ccg_calls(), 1);
    }

    #[test]
    fn test_create_manifest_result_fallback_path() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        let result = orchestrator.create_manifest_result("# Manifest".to_string());

        assert_eq!(result.level(), CompressionLevel::Manifest);
        assert_eq!(result.source(), ContextSource::Constructed);
        assert_eq!(orchestrator.metrics().fallback_calls(), 1);
    }

    #[test]
    fn test_create_architecture_result_ccg_path() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        let result = orchestrator.create_architecture_result("# Architecture".to_string());

        assert_eq!(result.level(), CompressionLevel::Architecture);
        assert_eq!(result.source(), ContextSource::CcgLayer1);
    }

    #[test]
    fn test_create_architecture_result_fallback_path() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        let result = orchestrator.create_architecture_result("# Architecture".to_string());

        assert_eq!(result.level(), CompressionLevel::Architecture);
        assert_eq!(result.source(), ContextSource::Constructed);
    }

    #[test]
    fn test_cache_hit() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        // Store a result
        let content = "# Cached content".to_string();
        let result = CompressionResult::new(
            content.clone(),
            CompressionLevel::Manifest,
            ContextSource::CcgLayer0,
        );
        orchestrator.cache_result(CompressionLevel::Manifest, &result, "hash123");

        // Retrieve it
        let cached = orchestrator.get_cached(CompressionLevel::Manifest, "hash123");
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().content(), content);
        assert_eq!(orchestrator.metrics().cache_hits(), 1);
    }

    #[test]
    fn test_cache_miss() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        let cached = orchestrator.get_cached(CompressionLevel::Manifest, "hash123");
        assert!(cached.is_none());
        assert_eq!(orchestrator.metrics().cache_misses(), 1);
    }

    #[test]
    fn test_cache_invalidation_on_hash_change() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        // Store with old hash
        let result = CompressionResult::new(
            "content".to_string(),
            CompressionLevel::Manifest,
            ContextSource::CcgLayer0,
        );
        orchestrator.cache_result(CompressionLevel::Manifest, &result, "old_hash");

        // Try to retrieve with new hash
        let cached = orchestrator.get_cached(CompressionLevel::Manifest, "new_hash");
        assert!(cached.is_none());
    }

    #[test]
    fn test_metrics_accessible() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        // Verify metrics are accessible
        assert_eq!(orchestrator.metrics().cache_hits(), 0);
        assert_eq!(orchestrator.metrics().cache_misses(), 0);
    }
}
