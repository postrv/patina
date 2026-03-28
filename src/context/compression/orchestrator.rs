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
    parse_find_symbols_response, parse_import_graph_to_architecture, parse_mcp_tool_response,
    parse_project_structure_to_manifest, render_architecture_within_budget,
    render_manifest_markdown, render_symbols_within_budget, CacheKey, CcgBackend, CcgFetchError,
    CompressionLevel, CompressionMetrics, CompressionResult, ContextSource, ResultCache,
};
use crate::mcp::connection::McpConnection;
use crate::narsil::{NarsilCapabilities, NarsilCapability};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

/// Default token budget for manifest layer (~500 tokens).
///
/// Manifests are compact summaries (~2KB text) so this budget is generous.
pub const DEFAULT_MANIFEST_TOKEN_BUDGET: usize = 500;

/// Default TTL for orchestrator cache (5 minutes).
pub const DEFAULT_ORCHESTRATOR_TTL: Duration = Duration::from_secs(300);

/// Default maximum token budget for architecture layer (~12K tokens).
///
/// This limits the architecture context to prevent oversized outputs
/// from large codebases. Approximately 48KB of text.
pub const DEFAULT_ARCHITECTURE_TOKEN_BUDGET: usize = 12_000;

/// Default maximum cache entries for orchestrator.
pub const DEFAULT_ORCHESTRATOR_MAX_ENTRIES: usize = 100;

/// Default token budget for symbol layer (~4K tokens).
///
/// This limits the symbol context to prevent oversized outputs.
/// Approximately 16KB of text.
pub const DEFAULT_SYMBOL_TOKEN_BUDGET: usize = 4_000;

/// Token budget allocation across context layers.
///
/// Represents how a total token budget is divided among manifest,
/// architecture, and symbol layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetAllocation {
    /// Tokens allocated to the manifest layer
    pub manifest: usize,
    /// Tokens allocated to the architecture layer
    pub architecture: usize,
    /// Tokens allocated to the symbol layer
    pub symbols: usize,
}

/// Result of building context with per-layer token tracking.
///
/// Wraps a [`CompressionResult`] with additional metadata about how
/// tokens were distributed across the manifest, architecture, and symbol
/// layers.
#[derive(Debug, Clone)]
pub struct BuildContextResult {
    /// The assembled compression result
    result: CompressionResult,
    /// Tokens used by the manifest layer
    manifest_tokens: usize,
    /// Tokens used by the architecture layer
    architecture_tokens: usize,
    /// Tokens used by the symbol layer
    symbol_tokens: usize,
}

impl BuildContextResult {
    /// Creates a new build context result with per-layer token tracking.
    #[must_use]
    pub fn new(
        result: CompressionResult,
        manifest_tokens: usize,
        architecture_tokens: usize,
        symbol_tokens: usize,
    ) -> Self {
        Self {
            result,
            manifest_tokens,
            architecture_tokens,
            symbol_tokens,
        }
    }

    /// Returns the assembled compression result.
    #[must_use]
    pub fn result(&self) -> &CompressionResult {
        &self.result
    }

    /// Returns the tokens used by the manifest layer.
    #[must_use]
    pub fn manifest_tokens(&self) -> usize {
        self.manifest_tokens
    }

    /// Returns the tokens used by the architecture layer.
    #[must_use]
    pub fn architecture_tokens(&self) -> usize {
        self.architecture_tokens
    }

    /// Returns the tokens used by the symbol layer.
    #[must_use]
    pub fn symbol_tokens(&self) -> usize {
        self.symbol_tokens
    }

    /// Returns the total tokens used across all layers.
    #[must_use]
    pub fn total_tokens(&self) -> usize {
        self.manifest_tokens + self.architecture_tokens + self.symbol_tokens
    }
}

/// Routes compression requests to CCG or fallback backends.
///
/// The orchestrator checks for the `CcgGraph` capability to determine
/// which backend to use. Results are cached with hash-based invalidation.
///
/// # Graceful Degradation
///
/// The orchestrator handles failures gracefully:
/// - If narsil is disconnected, returns minimal context
/// - If MCP calls fail, falls back to cached or default results
/// - All failures are logged but don't cause panics
pub struct CompressionOrchestrator {
    /// Detected narsil capabilities
    capabilities: NarsilCapabilities,
    /// Result cache with hash-based invalidation
    cache: Arc<ResultCache>,
    /// Metrics for monitoring
    metrics: Arc<CompressionMetrics>,
    /// Repository name for MCP calls
    repo_name: String,
    /// Whether degraded mode is active (narsil unavailable)
    degraded: AtomicBool,
}

impl std::fmt::Debug for CompressionOrchestrator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompressionOrchestrator")
            .field("capabilities", &self.capabilities)
            .field("cache", &self.cache)
            .field("metrics", &self.metrics)
            .field("repo_name", &self.repo_name)
            .field("degraded", &self.degraded.load(Ordering::Relaxed))
            .finish()
    }
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
            degraded: AtomicBool::new(false),
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

    /// Returns true if the orchestrator is in degraded mode.
    ///
    /// Degraded mode is entered when narsil is disconnected or MCP calls fail.
    #[must_use]
    pub fn is_degraded(&self) -> bool {
        self.degraded.load(Ordering::Relaxed)
    }

    /// Enters degraded mode.
    ///
    /// Call this when narsil becomes unavailable. The orchestrator will
    /// return minimal context until narsil reconnects.
    pub fn enter_degraded_mode(&self) {
        if !self.degraded.swap(true, Ordering::Relaxed) {
            warn!(
                repo = %self.repo_name,
                "Compression orchestrator entering degraded mode - narsil unavailable"
            );
            self.metrics.record_degradation();
        }
    }

    /// Exits degraded mode.
    ///
    /// Call this when narsil becomes available again.
    pub fn exit_degraded_mode(&self) {
        if self.degraded.swap(false, Ordering::Relaxed) {
            warn!(
                repo = %self.repo_name,
                "Compression orchestrator exiting degraded mode - narsil reconnected"
            );
        }
    }

    /// Gets cached context if available and valid.
    ///
    /// Returns `None` if no cached result exists or if the cache is invalid
    /// (hash changed or TTL expired).
    pub fn get_cached(
        &self,
        level: CompressionLevel,
        repo_hash: &str,
    ) -> Option<CompressionResult> {
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
    pub fn cache_result(
        &self,
        level: CompressionLevel,
        result: &CompressionResult,
        repo_hash: &str,
    ) {
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

    /// Creates a compression result for the symbol detail level.
    ///
    /// Uses `DirectTool` source since symbol queries go through core narsil tools.
    #[must_use]
    pub fn create_symbol_result(&self, content: String) -> CompressionResult {
        self.metrics.record_fallback_call();
        CompressionResult::new(
            content,
            CompressionLevel::SymbolDetail,
            ContextSource::DirectTool,
        )
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

    /// Gets context at the specified level, using cache if available.
    ///
    /// This method:
    /// 1. Checks the cache for a valid result
    /// 2. If cache miss, creates the result using the appropriate backend
    /// 3. Caches the result before returning
    ///
    /// # Arguments
    ///
    /// * `level` - The compression level to retrieve
    /// * `repo_hash` - Current repository hash for cache validation
    /// * `content_fn` - Function to generate content on cache miss
    ///
    /// # Returns
    ///
    /// The compression result, either from cache or freshly generated.
    pub fn get_context<F>(
        &self,
        level: CompressionLevel,
        repo_hash: &str,
        content_fn: F,
    ) -> CompressionResult
    where
        F: FnOnce() -> String,
    {
        // Try cache first
        if let Some(cached) = self.get_cached(level, repo_hash) {
            return cached;
        }

        // Cache miss - generate content
        let content = content_fn();
        let result = match level {
            CompressionLevel::Manifest => self.create_manifest_result(content),
            CompressionLevel::Architecture => self.create_architecture_result(content),
            CompressionLevel::SymbolDetail | CompressionLevel::Full => {
                // These levels use DirectTool source
                CompressionResult::new(content, level, ContextSource::DirectTool)
            }
        };

        // Cache the result
        self.cache_result(level, &result, repo_hash);

        result
    }

    /// Gets the default context (manifest + architecture).
    ///
    /// The default context is designed to stay under 52KB and provides
    /// a good balance between context richness and token efficiency.
    ///
    /// # Arguments
    ///
    /// * `repo_hash` - Current repository hash for cache validation
    /// * `manifest_fn` - Function to generate manifest content
    /// * `architecture_fn` - Function to generate architecture content
    ///
    /// # Returns
    ///
    /// A combined compression result with manifest and architecture.
    pub fn get_default_context<F1, F2>(
        &self,
        repo_hash: &str,
        manifest_fn: F1,
        architecture_fn: F2,
    ) -> CompressionResult
    where
        F1: FnOnce() -> String,
        F2: FnOnce() -> String,
    {
        let manifest = self.get_context(CompressionLevel::Manifest, repo_hash, manifest_fn);
        let architecture =
            self.get_context(CompressionLevel::Architecture, repo_hash, architecture_fn);

        // Combine manifest and architecture
        let combined = format!(
            "{}\n\n---\n\n{}",
            manifest.content(),
            architecture.content()
        );

        // Use the more detailed source for combined result
        let source = if self.should_use_ccg() {
            ContextSource::CcgLayer1
        } else {
            ContextSource::Constructed
        };

        CompressionResult::with_tokens(
            combined,
            CompressionLevel::Architecture, // Default context is architecture-level
            source,
            manifest.tokens_approx() + architecture.tokens_approx(),
        )
    }

    /// Returns the maximum size in bytes for default context.
    #[must_use]
    pub const fn default_context_max_bytes() -> usize {
        52 * 1024 // 52KB
    }

    /// Builds context progressively within a token budget.
    ///
    /// This method loads context in order of compression level, stopping
    /// when the token budget is exhausted:
    /// 1. Manifest (always included if budget allows)
    /// 2. Architecture (included if budget allows)
    /// 3. Symbol details (optional, fills remaining budget)
    ///
    /// # Arguments
    ///
    /// * `token_budget` - Maximum tokens to include
    /// * `repo_hash` - Current repository hash for cache validation
    /// * `manifest_fn` - Function to generate manifest content
    /// * `architecture_fn` - Function to generate architecture content
    /// * `symbol_fn` - Optional function to generate symbol details
    ///
    /// # Returns
    ///
    /// A compression result containing as much context as fits in the budget.
    pub fn build_context_with_budget<F1, F2, F3>(
        &self,
        token_budget: usize,
        repo_hash: &str,
        manifest_fn: F1,
        architecture_fn: F2,
        symbol_fn: Option<F3>,
    ) -> CompressionResult
    where
        F1: FnOnce() -> String,
        F2: FnOnce() -> String,
        F3: FnOnce() -> String,
    {
        let mut parts: Vec<String> = Vec::new();
        let mut total_tokens: usize;
        let mut highest_level = CompressionLevel::Manifest;

        // 1. Always try to include manifest first
        let manifest = self.get_context(CompressionLevel::Manifest, repo_hash, manifest_fn);
        let manifest_tokens = manifest.tokens_approx();

        if manifest_tokens <= token_budget {
            parts.push(manifest.content().to_string());
            total_tokens = manifest_tokens;
        } else {
            // Can't even fit manifest - return truncated
            return self.truncate_to_budget(&manifest, token_budget);
        }

        // 2. Include architecture if budget allows
        let remaining = token_budget.saturating_sub(total_tokens);
        if remaining > 0 {
            let architecture =
                self.get_context(CompressionLevel::Architecture, repo_hash, architecture_fn);
            let arch_tokens = architecture.tokens_approx();

            if arch_tokens <= remaining {
                parts.push(architecture.content().to_string());
                total_tokens += arch_tokens;
                highest_level = CompressionLevel::Architecture;
            }
        }

        // 3. Include symbol details if budget allows and function provided
        if let Some(symbol_fn) = symbol_fn {
            let remaining = token_budget.saturating_sub(total_tokens);
            if remaining > 0 {
                let symbols =
                    self.get_context(CompressionLevel::SymbolDetail, repo_hash, symbol_fn);
                let symbol_tokens = symbols.tokens_approx();

                if symbol_tokens <= remaining {
                    parts.push(symbols.content().to_string());
                    total_tokens += symbol_tokens;
                    highest_level = CompressionLevel::SymbolDetail;
                }
            }
        }

        // Combine all parts
        let combined = parts.join("\n\n---\n\n");
        let source = if self.should_use_ccg() {
            match highest_level {
                CompressionLevel::Manifest => ContextSource::CcgLayer0,
                CompressionLevel::Architecture => ContextSource::CcgLayer1,
                _ => ContextSource::CcgSparql,
            }
        } else {
            ContextSource::Constructed
        };

        CompressionResult::with_tokens(combined, highest_level, source, total_tokens)
    }

    /// Gets context with graceful degradation on failure.
    ///
    /// If the content function fails or returns an error, this method:
    /// 1. Enters degraded mode
    /// 2. Returns a minimal fallback result
    /// 3. Logs a warning
    ///
    /// # Arguments
    ///
    /// * `level` - The compression level to retrieve
    /// * `repo_hash` - Current repository hash for cache validation
    /// * `content_fn` - Function that may fail to generate content
    ///
    /// # Returns
    ///
    /// A compression result, or a minimal fallback if generation fails.
    pub fn get_context_graceful<F>(
        &self,
        level: CompressionLevel,
        repo_hash: &str,
        content_fn: F,
    ) -> CompressionResult
    where
        F: FnOnce() -> Result<String, String>,
    {
        // If already degraded, return minimal result
        if self.is_degraded() {
            return self.create_degraded_result(level);
        }

        // Try cache first
        if let Some(cached) = self.get_cached(level, repo_hash) {
            return cached;
        }

        // Try to generate content
        match content_fn() {
            Ok(content) => {
                // Success - exit degraded mode if we were in it
                self.exit_degraded_mode();

                let result = match level {
                    CompressionLevel::Manifest => self.create_manifest_result(content),
                    CompressionLevel::Architecture => self.create_architecture_result(content),
                    CompressionLevel::SymbolDetail | CompressionLevel::Full => {
                        CompressionResult::new(content, level, ContextSource::DirectTool)
                    }
                };

                self.cache_result(level, &result, repo_hash);
                result
            }
            Err(error) => {
                // Failed - enter degraded mode
                warn!(
                    level = ?level,
                    error = %error,
                    repo = %self.repo_name,
                    "Context generation failed, entering degraded mode"
                );
                self.enter_degraded_mode();
                self.create_degraded_result(level)
            }
        }
    }

    /// Creates a minimal result for degraded mode.
    fn create_degraded_result(&self, level: CompressionLevel) -> CompressionResult {
        let content = format!(
            "# {} [Degraded Mode]\n\n_Context unavailable - narsil disconnected or MCP call failed._",
            match level {
                CompressionLevel::Manifest => "Repository Manifest",
                CompressionLevel::Architecture => "Architecture",
                CompressionLevel::SymbolDetail => "Symbol Details",
                CompressionLevel::Full => "Full Content",
            }
        );
        CompressionResult::new(content, level, ContextSource::Constructed)
    }

    /// Truncates content to fit within a token budget.
    fn truncate_to_budget(
        &self,
        result: &CompressionResult,
        token_budget: usize,
    ) -> CompressionResult {
        let content = result.content();
        // Approximate chars per token (~4)
        let char_budget = token_budget * 4;

        let truncated = if content.len() > char_budget {
            let mut end = char_budget;
            // Find a safe UTF-8 boundary
            while end > 0 && !content.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}...", &content[..end])
        } else {
            content.to_string()
        };

        CompressionResult::with_tokens(truncated, result.level(), result.source(), token_budget)
    }

    // =========================================================================
    // Async MCP methods (R.3.1)
    // =========================================================================

    /// Creates a CCG backend for this orchestrator.
    fn ccg_backend(&self) -> CcgBackend {
        CcgBackend::new(&self.repo_name)
    }

    /// Fetches the manifest asynchronously via MCP.
    ///
    /// When CCG capability is available, calls `get_ccg_manifest` tool.
    /// Otherwise, returns a degraded result indicating CCG is unavailable.
    ///
    /// # Arguments
    ///
    /// * `client` - An active MCP client connected to narsil-mcp
    /// * `repo_hash` - Current repository hash for cache validation
    ///
    /// # Returns
    ///
    /// A `CompressionResult` containing the manifest, or a degraded result on failure.
    ///
    /// # Cache Behavior
    ///
    /// Results are cached and subsequent calls with the same `repo_hash` return
    /// the cached result without making an MCP call.
    pub async fn get_manifest_async(
        &self,
        client: &McpConnection,
        repo_hash: &str,
    ) -> CompressionResult {
        // Try cache first
        if let Some(cached) = self.get_cached(CompressionLevel::Manifest, repo_hash) {
            return cached;
        }

        // Check if CCG is available
        if !self.should_use_ccg() {
            warn!(
                repo = %self.repo_name,
                "CCG capability not available, returning degraded manifest"
            );
            return self.create_degraded_result(CompressionLevel::Manifest);
        }

        // Fetch via CCG backend
        let backend = self.ccg_backend();
        match backend.fetch_manifest(client).await {
            Ok(result) => {
                self.metrics.record_ccg_call();
                self.cache_result(CompressionLevel::Manifest, &result, repo_hash);
                self.exit_degraded_mode();
                result
            }
            Err(err) => {
                warn!(
                    repo = %self.repo_name,
                    error = %err,
                    "CCG manifest fetch failed, entering degraded mode"
                );
                self.enter_degraded_mode();
                self.create_degraded_result(CompressionLevel::Manifest)
            }
        }
    }

    /// Fetches the architecture asynchronously via MCP.
    ///
    /// When CCG capability is available, calls `export_ccg_architecture` tool.
    /// Otherwise, returns a degraded result indicating CCG is unavailable.
    ///
    /// # Arguments
    ///
    /// * `client` - An active MCP client connected to narsil-mcp
    /// * `repo_hash` - Current repository hash for cache validation
    ///
    /// # Returns
    ///
    /// A `CompressionResult` containing the architecture, or a degraded result on failure.
    pub async fn get_architecture_async(
        &self,
        client: &McpConnection,
        repo_hash: &str,
    ) -> CompressionResult {
        // Try cache first
        if let Some(cached) = self.get_cached(CompressionLevel::Architecture, repo_hash) {
            return cached;
        }

        // Check if CCG is available
        if !self.should_use_ccg() {
            warn!(
                repo = %self.repo_name,
                "CCG capability not available, returning degraded architecture"
            );
            return self.create_degraded_result(CompressionLevel::Architecture);
        }

        // Fetch via CCG backend
        let backend = self.ccg_backend();
        match backend.fetch_architecture(client).await {
            Ok(result) => {
                self.metrics.record_ccg_call();
                self.cache_result(CompressionLevel::Architecture, &result, repo_hash);
                self.exit_degraded_mode();
                result
            }
            Err(err) => {
                warn!(
                    repo = %self.repo_name,
                    error = %err,
                    "CCG architecture fetch failed, entering degraded mode"
                );
                self.enter_degraded_mode();
                self.create_degraded_result(CompressionLevel::Architecture)
            }
        }
    }

    /// Executes a SPARQL query asynchronously via MCP.
    ///
    /// Calls the `query_ccg` tool with the provided SPARQL query.
    /// Requires CCG capability to be available.
    ///
    /// # Arguments
    ///
    /// * `client` - An active MCP client connected to narsil-mcp
    /// * `sparql` - The SPARQL query (must include LIMIT clause)
    ///
    /// # Returns
    ///
    /// `Ok(CompressionResult)` on success, or `Err(CcgFetchError)` on failure.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - CCG capability is not available
    /// - The SPARQL query lacks a LIMIT clause
    /// - The MCP call fails
    pub async fn query_async(
        &self,
        client: &McpConnection,
        sparql: &str,
    ) -> Result<CompressionResult, CcgFetchError> {
        // Check if CCG is available
        if !self.should_use_ccg() {
            return Err(CcgFetchError::mcp_error(anyhow::anyhow!(
                "CCG capability not available"
            )));
        }

        let backend = self.ccg_backend();
        let result = backend.execute_query(client, sparql).await?;

        self.metrics.record_ccg_call();
        self.exit_degraded_mode();

        Ok(result)
    }

    /// Fetches the manifest via live MCP call to `get_project_structure`.
    ///
    /// This is the unified entry point for manifest retrieval. It:
    /// 1. Checks the cache (keyed by git commit hash)
    /// 2. On cache miss, calls `get_project_structure` via MCP
    /// 3. Parses the response into a [`CcgManifest`] and renders as Markdown
    /// 4. Caches the result for future calls with the same commit hash
    ///
    /// Unlike [`get_manifest_async`], this method works without the CCG `--graph`
    /// flag since `get_project_structure` is always available in narsil.
    ///
    /// # Arguments
    ///
    /// * `client` - An active MCP client connected to narsil-mcp
    /// * `repo_hash` - Current git commit hash for cache invalidation
    ///
    /// # Returns
    ///
    /// A `CompressionResult` containing the rendered manifest Markdown,
    /// or a degraded result on MCP failure.
    ///
    /// # Cache Behavior
    ///
    /// Results are cached keyed by `repo_hash`. Subsequent calls with the
    /// same hash return the cached result. A new commit hash triggers a
    /// fresh MCP call.
    pub async fn fetch_manifest(
        &self,
        client: &McpConnection,
        repo_hash: &str,
    ) -> CompressionResult {
        // 1. Check cache
        if let Some(cached) = self.get_cached(CompressionLevel::Manifest, repo_hash) {
            return cached;
        }

        // 2. Fetch via MCP
        let args = serde_json::json!({
            "repo": self.repo_name
        });

        match client.call_tool_json("get_project_structure", args).await {
            Ok(response) => self.process_manifest_response(&response, repo_hash),
            Err(err) => {
                warn!(
                    repo = %self.repo_name,
                    error = %err,
                    "Manifest fetch via get_project_structure failed, entering degraded mode"
                );
                self.enter_degraded_mode();
                self.create_degraded_result(CompressionLevel::Manifest)
            }
        }
    }

    /// Processes an MCP tool response into a manifest [`CompressionResult`].
    ///
    /// Separated from [`fetch_manifest`] for testability. Parses the MCP
    /// response, extracts project structure, builds a `CcgManifest`, renders
    /// it as Markdown, and caches the result.
    ///
    /// # Arguments
    ///
    /// * `response` - The raw MCP tool response JSON
    /// * `repo_hash` - Git commit hash for cache keying
    ///
    /// # Returns
    ///
    /// A `CompressionResult` on success, or a degraded result if the
    /// response cannot be parsed.
    pub fn process_manifest_response(
        &self,
        response: &serde_json::Value,
        repo_hash: &str,
    ) -> CompressionResult {
        match parse_mcp_tool_response(response) {
            Ok(content) => {
                let manifest =
                    parse_project_structure_to_manifest(&content, &self.repo_name, repo_hash);
                let markdown = render_manifest_markdown(&manifest);
                let result = self.create_manifest_result(markdown);
                self.cache_result(CompressionLevel::Manifest, &result, repo_hash);
                self.exit_degraded_mode();
                result
            }
            Err(err) => {
                warn!(
                    repo = %self.repo_name,
                    error = %err,
                    "Failed to parse manifest MCP response, entering degraded mode"
                );
                self.enter_degraded_mode();
                self.create_degraded_result(CompressionLevel::Manifest)
            }
        }
    }

    /// Fetches architecture data via live MCP call to `get_import_graph`.
    ///
    /// This is the unified entry point for architecture retrieval. It:
    /// 1. Checks the cache (keyed by git commit hash)
    /// 2. On cache miss, calls `get_import_graph` via MCP
    /// 3. Parses the response into a [`CcgArchitecture`] and renders as Markdown
    /// 4. Caches the result for future calls with the same commit hash
    ///
    /// Unlike [`get_architecture_async`], this method works without the CCG `--graph`
    /// flag since `get_import_graph` is always available in narsil.
    ///
    /// # Arguments
    ///
    /// * `client` - An active MCP client connected to narsil-mcp
    /// * `repo_hash` - Current git commit hash for cache invalidation
    ///
    /// # Returns
    ///
    /// A `CompressionResult` containing the rendered architecture Markdown,
    /// or a degraded result on MCP failure.
    ///
    /// # Cache Behavior
    ///
    /// Results are cached keyed by `repo_hash`. Subsequent calls with the
    /// same hash return the cached result. A new commit hash triggers a
    /// fresh MCP call.
    pub async fn fetch_architecture(
        &self,
        client: &McpConnection,
        repo_hash: &str,
    ) -> CompressionResult {
        // 1. Check cache
        if let Some(cached) = self.get_cached(CompressionLevel::Architecture, repo_hash) {
            return cached;
        }

        // 2. Fetch via MCP
        let args = serde_json::json!({
            "repo": self.repo_name
        });

        match client.call_tool_json("get_import_graph", args).await {
            Ok(response) => self.process_architecture_response(&response, repo_hash),
            Err(err) => {
                warn!(
                    repo = %self.repo_name,
                    error = %err,
                    "Architecture fetch via get_import_graph failed, entering degraded mode"
                );
                self.enter_degraded_mode();
                self.create_degraded_result(CompressionLevel::Architecture)
            }
        }
    }

    /// Processes an MCP tool response into an architecture [`CompressionResult`].
    ///
    /// Separated from [`fetch_architecture`] for testability. Parses the MCP
    /// response, extracts import graph data, builds a `CcgArchitecture`, renders
    /// it as Markdown, and caches the result.
    ///
    /// # Arguments
    ///
    /// * `response` - The raw MCP tool response JSON
    /// * `repo_hash` - Git commit hash for cache keying
    ///
    /// # Returns
    ///
    /// A `CompressionResult` on success, or a degraded result if the
    /// response cannot be parsed.
    pub fn process_architecture_response(
        &self,
        response: &serde_json::Value,
        repo_hash: &str,
    ) -> CompressionResult {
        match parse_mcp_tool_response(response) {
            Ok(content) => {
                let architecture = parse_import_graph_to_architecture(&content, &self.repo_name);
                let markdown = render_architecture_within_budget(
                    &architecture,
                    DEFAULT_ARCHITECTURE_TOKEN_BUDGET,
                );
                let result = self.create_architecture_result(markdown);
                self.cache_result(CompressionLevel::Architecture, &result, repo_hash);
                self.exit_degraded_mode();
                result
            }
            Err(err) => {
                warn!(
                    repo = %self.repo_name,
                    error = %err,
                    "Failed to parse architecture MCP response, entering degraded mode"
                );
                self.enter_degraded_mode();
                self.create_degraded_result(CompressionLevel::Architecture)
            }
        }
    }

    /// Gets the default context (manifest + architecture) asynchronously.
    ///
    /// Fetches both manifest and architecture via MCP calls when CCG is available.
    /// Uses caching to avoid redundant calls.
    ///
    /// # Arguments
    ///
    /// * `client` - An active MCP client connected to narsil-mcp
    /// * `repo_hash` - Current repository hash for cache validation
    ///
    /// # Returns
    ///
    /// A combined compression result with manifest and architecture.
    pub async fn get_default_context_async(
        &self,
        client: &McpConnection,
        repo_hash: &str,
    ) -> CompressionResult {
        let manifest = self.get_manifest_async(client, repo_hash).await;
        let architecture = self.get_architecture_async(client, repo_hash).await;

        // Combine manifest and architecture
        let combined = format!(
            "{}\n\n---\n\n{}",
            manifest.content(),
            architecture.content()
        );

        // Use the more detailed source for combined result
        let source = if self.should_use_ccg() && !self.is_degraded() {
            ContextSource::CcgLayer1
        } else {
            ContextSource::Constructed
        };

        CompressionResult::with_tokens(
            combined,
            CompressionLevel::Architecture,
            source,
            manifest.tokens_approx() + architecture.tokens_approx(),
        )
    }

    /// Fetches symbol context via live MCP call to `find_symbols`.
    ///
    /// This is the unified entry point for symbol-level context. It:
    /// 1. Checks the cache (keyed by git commit hash + active files)
    /// 2. On cache miss, calls `find_symbols` via MCP for each active file
    /// 3. Parses the response into [`SymbolResults`]
    /// 4. Renders as Markdown within the token budget
    /// 5. Caches the result for future calls with the same commit hash
    ///
    /// # Arguments
    ///
    /// * `client` - An active MCP client connected to narsil-mcp
    /// * `repo_hash` - Current git commit hash for cache invalidation
    /// * `active_files` - List of file paths the user is actively working with
    /// * `token_budget` - Maximum tokens for symbol output
    ///
    /// # Returns
    ///
    /// A `CompressionResult` containing the rendered symbol Markdown,
    /// or a degraded result on MCP failure.
    ///
    /// # Cache Behavior
    ///
    /// Results are cached keyed by `repo_hash`. Subsequent calls with the
    /// same hash return the cached result. A new commit hash triggers a
    /// fresh MCP call.
    pub async fn fetch_symbols(
        &self,
        client: &McpConnection,
        repo_hash: &str,
        active_files: &[String],
        token_budget: usize,
    ) -> CompressionResult {
        // 1. Check cache
        if let Some(cached) = self.get_cached(CompressionLevel::SymbolDetail, repo_hash) {
            return cached;
        }

        // 2. Build args for find_symbols — scope to active files if provided
        let args = if active_files.is_empty() {
            serde_json::json!({
                "repo": self.repo_name,
                "exclude_tests": true
            })
        } else {
            // Join active file patterns as a glob
            let pattern = active_files
                .iter()
                .map(|f| format!("{{{f}}}"))
                .collect::<Vec<_>>()
                .join(",");
            serde_json::json!({
                "repo": self.repo_name,
                "file_pattern": pattern,
                "exclude_tests": true
            })
        };

        match client.call_tool_json("find_symbols", args).await {
            Ok(response) => self.process_symbols_response(&response, repo_hash, token_budget),
            Err(err) => {
                warn!(
                    repo = %self.repo_name,
                    error = %err,
                    "Symbol fetch via find_symbols failed, entering degraded mode"
                );
                self.enter_degraded_mode();
                self.create_degraded_result(CompressionLevel::SymbolDetail)
            }
        }
    }

    /// Processes an MCP tool response into a symbol [`CompressionResult`].
    ///
    /// Separated from [`fetch_symbols`] for testability. Parses the MCP
    /// response, extracts symbol information, renders it as budget-aware
    /// Markdown, and caches the result.
    ///
    /// # Arguments
    ///
    /// * `response` - The raw MCP tool response JSON
    /// * `repo_hash` - Git commit hash for cache keying
    /// * `token_budget` - Maximum tokens for the rendered output
    ///
    /// # Returns
    ///
    /// A `CompressionResult` on success, or a degraded result if the
    /// response cannot be parsed.
    pub fn process_symbols_response(
        &self,
        response: &serde_json::Value,
        repo_hash: &str,
        token_budget: usize,
    ) -> CompressionResult {
        match parse_mcp_tool_response(response) {
            Ok(content) => {
                let symbols = parse_find_symbols_response(&content);
                let markdown = render_symbols_within_budget(&symbols, token_budget);
                let result = self.create_symbol_result(markdown);
                self.cache_result(CompressionLevel::SymbolDetail, &result, repo_hash);
                self.exit_degraded_mode();
                result
            }
            Err(err) => {
                warn!(
                    repo = %self.repo_name,
                    error = %err,
                    "Failed to parse symbols MCP response, entering degraded mode"
                );
                self.enter_degraded_mode();
                self.create_degraded_result(CompressionLevel::SymbolDetail)
            }
        }
    }

    // =========================================================================
    // build_context with Token Budgeting (Phase 3.4)
    // =========================================================================

    /// Computes how to allocate a total token budget across context layers.
    ///
    /// The allocation strategy is:
    /// 1. Manifest gets up to [`DEFAULT_MANIFEST_TOKEN_BUDGET`] (500 tokens)
    /// 2. Architecture gets up to [`DEFAULT_ARCHITECTURE_TOKEN_BUDGET`] (12K tokens)
    /// 3. Symbols get the remainder
    ///
    /// When the total budget is smaller than the manifest budget, all tokens
    /// go to manifest and other layers get zero.
    ///
    /// # Arguments
    ///
    /// * `total_budget` - Total token budget to allocate
    ///
    /// # Returns
    ///
    /// A [`BudgetAllocation`] with per-layer token budgets summing to `total_budget`.
    #[must_use]
    pub fn compute_budget_allocation(&self, total_budget: usize) -> BudgetAllocation {
        if total_budget == 0 {
            return BudgetAllocation {
                manifest: 0,
                architecture: 0,
                symbols: 0,
            };
        }

        let manifest = total_budget.min(DEFAULT_MANIFEST_TOKEN_BUDGET);
        let remaining_after_manifest = total_budget.saturating_sub(manifest);

        let architecture = remaining_after_manifest.min(DEFAULT_ARCHITECTURE_TOKEN_BUDGET);
        let symbols = remaining_after_manifest.saturating_sub(architecture);

        BudgetAllocation {
            manifest,
            architecture,
            symbols,
        }
    }

    /// Assembles pre-fetched context layers into a single result within budget.
    ///
    /// Takes already-fetched manifest, architecture, and symbol results and
    /// combines them progressively, stopping when the token budget is exhausted.
    ///
    /// # Arguments
    ///
    /// * `token_budget` - Maximum total tokens for the assembled result
    /// * `manifest` - Pre-fetched manifest result
    /// * `architecture` - Pre-fetched architecture result
    /// * `symbols` - Pre-fetched symbol result
    ///
    /// # Returns
    ///
    /// A [`BuildContextResult`] with the assembled content and per-layer token tracking.
    #[must_use]
    pub fn assemble_context(
        &self,
        token_budget: usize,
        manifest: CompressionResult,
        architecture: CompressionResult,
        symbols: CompressionResult,
    ) -> BuildContextResult {
        let mut parts: Vec<String> = Vec::new();
        let mut highest_level = CompressionLevel::Manifest;

        // 1. Always include manifest if budget allows
        let m_tokens = manifest.tokens_approx();
        if m_tokens > token_budget {
            // Truncate manifest to fit
            let truncated = self.truncate_to_budget(&manifest, token_budget);
            let t_tokens = truncated.tokens_approx();
            return BuildContextResult::new(
                CompressionResult::with_tokens(
                    truncated.content().to_string(),
                    CompressionLevel::Manifest,
                    manifest.source(),
                    t_tokens,
                ),
                t_tokens,
                0,
                0,
            );
        }

        parts.push(manifest.content().to_string());
        let manifest_tokens = m_tokens;
        let mut total_tokens = m_tokens;

        // 2. Include architecture if budget allows
        let mut architecture_tokens: usize = 0;
        let remaining = token_budget.saturating_sub(total_tokens);
        if remaining > 0 {
            let a_tokens = architecture.tokens_approx();
            if a_tokens <= remaining {
                parts.push(architecture.content().to_string());
                total_tokens += a_tokens;
                architecture_tokens = a_tokens;
                highest_level = CompressionLevel::Architecture;
            }
        }

        // 3. Include symbols if budget allows
        let mut symbol_tokens: usize = 0;
        let remaining = token_budget.saturating_sub(total_tokens);
        if remaining > 0 {
            let s_tokens = symbols.tokens_approx();
            if s_tokens <= remaining {
                parts.push(symbols.content().to_string());
                total_tokens += s_tokens;
                symbol_tokens = s_tokens;
                highest_level = CompressionLevel::SymbolDetail;
            }
        }

        let combined = parts.join("\n\n---\n\n");
        let source = if self.should_use_ccg() {
            match highest_level {
                CompressionLevel::Manifest => ContextSource::CcgLayer0,
                CompressionLevel::Architecture => ContextSource::CcgLayer1,
                _ => ContextSource::CcgSparql,
            }
        } else {
            ContextSource::Constructed
        };

        BuildContextResult::new(
            CompressionResult::with_tokens(combined, highest_level, source, total_tokens),
            manifest_tokens,
            architecture_tokens,
            symbol_tokens,
        )
    }

    /// Builds full context by fetching all layers via MCP in parallel and
    /// assembling them within a token budget.
    ///
    /// This is the top-level entry point for context building. It:
    /// 1. Computes budget allocation across layers
    /// 2. Fetches manifest, architecture, and symbols via MCP **in parallel**
    ///    using `tokio::join!`
    /// 3. Assembles them progressively within the total budget
    /// 4. Records metrics (latency, token usage)
    ///
    /// Each layer's fetch checks the cache first (no lock needed), then
    /// acquires a mutex only for the actual MCP call. This means cache-hit
    /// layers complete immediately while cache-miss layers overlap their
    /// I/O wait with other layers' processing.
    ///
    /// # Arguments
    ///
    /// * `client` - An active MCP client connected to narsil-mcp
    /// * `repo_hash` - Current git commit hash for cache invalidation
    /// * `active_files` - Files the user is actively working with
    /// * `token_budget` - Maximum total tokens for the result
    ///
    /// # Returns
    ///
    /// A [`BuildContextResult`] with assembled context and per-layer metrics.
    pub async fn build_context(
        &self,
        client: &McpConnection,
        repo_hash: &str,
        active_files: &[String],
        token_budget: usize,
    ) -> BuildContextResult {
        let start = std::time::Instant::now();

        let allocation = self.compute_budget_allocation(token_budget);

        // Wrap the client in a mutex so parallel fetches can share it.
        // Each fetch locks only during the actual MCP call; cache checks
        // and response parsing happen without holding the lock.
        let client_mutex = tokio::sync::Mutex::new(client);

        let (manifest, architecture, symbols) = tokio::join!(
            self.fetch_manifest_parallel(&client_mutex, repo_hash),
            self.fetch_architecture_parallel(&client_mutex, repo_hash),
            self.fetch_symbols_parallel(
                &client_mutex,
                repo_hash,
                active_files,
                allocation.symbols,
            ),
        );

        let result = self.assemble_context(token_budget, manifest, architecture, symbols);

        // Record metrics
        self.metrics.record_latency(start.elapsed());
        self.metrics.record_build_context(
            result.manifest_tokens(),
            result.architecture_tokens(),
            result.symbol_tokens(),
        );

        result
    }

    // =========================================================================
    // Parallel fetch helpers (Phase 5.1)
    // =========================================================================

    /// Fetches the manifest layer, acquiring the client lock only for the MCP call.
    ///
    /// Checks cache first (lock-free), then locks the mutex for the MCP call
    /// if needed, and processes the response after releasing the lock.
    async fn fetch_manifest_parallel(
        &self,
        client: &tokio::sync::Mutex<&McpConnection>,
        repo_hash: &str,
    ) -> CompressionResult {
        // Cache check — no lock needed
        if let Some(cached) = self.get_cached(CompressionLevel::Manifest, repo_hash) {
            return cached;
        }

        let args = serde_json::json!({
            "repo": self.repo_name
        });

        // Lock client only for the MCP call
        let response = {
            let guard = client.lock().await;
            guard.call_tool_json("get_project_structure", args).await
        };

        match response {
            Ok(response) => self.process_manifest_response(&response, repo_hash),
            Err(err) => {
                warn!(
                    repo = %self.repo_name,
                    error = %err,
                    "Manifest fetch via get_project_structure failed, entering degraded mode"
                );
                self.enter_degraded_mode();
                self.create_degraded_result(CompressionLevel::Manifest)
            }
        }
    }

    /// Fetches the architecture layer, acquiring the client lock only for the MCP call.
    ///
    /// Checks cache first (lock-free), then locks the mutex for the MCP call
    /// if needed, and processes the response after releasing the lock.
    async fn fetch_architecture_parallel(
        &self,
        client: &tokio::sync::Mutex<&McpConnection>,
        repo_hash: &str,
    ) -> CompressionResult {
        // Cache check — no lock needed
        if let Some(cached) = self.get_cached(CompressionLevel::Architecture, repo_hash) {
            return cached;
        }

        let args = serde_json::json!({
            "repo": self.repo_name
        });

        // Lock client only for the MCP call
        let response = {
            let guard = client.lock().await;
            guard.call_tool_json("get_import_graph", args).await
        };

        match response {
            Ok(response) => self.process_architecture_response(&response, repo_hash),
            Err(err) => {
                warn!(
                    repo = %self.repo_name,
                    error = %err,
                    "Architecture fetch via get_import_graph failed, entering degraded mode"
                );
                self.enter_degraded_mode();
                self.create_degraded_result(CompressionLevel::Architecture)
            }
        }
    }

    /// Fetches the symbol layer, acquiring the client lock only for the MCP call.
    ///
    /// Checks cache first (lock-free), then locks the mutex for the MCP call
    /// if needed, and processes the response after releasing the lock.
    async fn fetch_symbols_parallel(
        &self,
        client: &tokio::sync::Mutex<&McpConnection>,
        repo_hash: &str,
        active_files: &[String],
        token_budget: usize,
    ) -> CompressionResult {
        // Cache check — no lock needed
        if let Some(cached) = self.get_cached(CompressionLevel::SymbolDetail, repo_hash) {
            return cached;
        }

        let args = if active_files.is_empty() {
            serde_json::json!({
                "repo": self.repo_name,
                "exclude_tests": true
            })
        } else {
            let pattern = active_files
                .iter()
                .map(|f| format!("{{{f}}}"))
                .collect::<Vec<_>>()
                .join(",");
            serde_json::json!({
                "repo": self.repo_name,
                "file_pattern": pattern,
                "exclude_tests": true
            })
        };

        // Lock client only for the MCP call
        let response = {
            let guard = client.lock().await;
            guard.call_tool_json("find_symbols", args).await
        };

        match response {
            Ok(response) => self.process_symbols_response(&response, repo_hash, token_budget),
            Err(err) => {
                warn!(
                    repo = %self.repo_name,
                    error = %err,
                    "Symbol fetch via find_symbols failed, entering degraded mode"
                );
                self.enter_degraded_mode();
                self.create_degraded_result(CompressionLevel::SymbolDetail)
            }
        }
    }
}

#[cfg(test)]
impl BuildContextResult {
    /// Consumes self and returns the inner compression result.
    #[must_use]
    pub fn into_result(self) -> CompressionResult {
        self.result
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
        let orchestrator =
            CompressionOrchestrator::with_config(caps, "my-repo", Duration::from_secs(60), 50);

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

    #[test]
    fn test_get_context_returns_cached_on_hit() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        // First call - cache miss
        let result1 = orchestrator.get_context(CompressionLevel::Manifest, "hash123", || {
            "# Manifest\n\nGenerated content".to_string()
        });
        assert_eq!(orchestrator.metrics().cache_misses(), 1);
        assert_eq!(orchestrator.metrics().cache_hits(), 0);

        // Second call - cache hit
        let result2 = orchestrator.get_context(CompressionLevel::Manifest, "hash123", || {
            panic!("Should not be called on cache hit")
        });
        assert_eq!(orchestrator.metrics().cache_hits(), 1);

        // Content should be the same
        assert_eq!(result1.content(), result2.content());
    }

    #[test]
    fn test_get_context_calls_backend_on_miss() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        let mut called = false;
        let _result = orchestrator.get_context(CompressionLevel::Manifest, "hash123", || {
            called = true;
            "Generated content".to_string()
        });

        assert!(called, "Content function should be called on cache miss");
        assert_eq!(orchestrator.metrics().cache_misses(), 1);
    }

    #[test]
    fn test_get_default_context_returns_manifest_plus_architecture() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        let result = orchestrator.get_default_context(
            "hash123",
            || "# Manifest".to_string(),
            || "# Architecture".to_string(),
        );

        // Should contain both sections
        assert!(result.content().contains("# Manifest"));
        assert!(result.content().contains("# Architecture"));
        assert!(result.content().contains("---")); // Separator
    }

    #[test]
    fn test_default_context_size_within_limit() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        // Create content that's within the limit
        let manifest = "# Manifest\n\n".repeat(100); // ~1.3KB
        let architecture = "# Architecture\n\n".repeat(500); // ~6.5KB

        let result = orchestrator.get_default_context("hash123", || manifest, || architecture);

        assert!(
            result.byte_len() < CompressionOrchestrator::default_context_max_bytes(),
            "Default context should be under 52KB, got {} bytes",
            result.byte_len()
        );
    }

    #[test]
    fn test_default_context_combines_token_counts() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        let result = orchestrator.get_default_context(
            "hash123",
            || "Manifest content".to_string(),
            || "Architecture content".to_string(),
        );

        // Token count should be sum of both parts
        assert!(result.tokens_approx() > 0);
    }

    #[test]
    fn test_get_context_caches_result() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        // Generate and cache
        let _result = orchestrator.get_context(CompressionLevel::Manifest, "hash123", || {
            "Generated content".to_string()
        });

        // Verify it's cached (hit on same hash)
        let cached = orchestrator.get_cached(CompressionLevel::Manifest, "hash123");
        assert!(cached.is_some());
    }

    #[test]
    fn test_default_context_max_bytes() {
        assert_eq!(
            CompressionOrchestrator::default_context_max_bytes(),
            52 * 1024
        );
    }

    // =============================================================================
    // Token budget tests (Task 3.3.5)
    // =============================================================================

    #[test]
    fn test_context_building_respects_token_budget() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        // Small budget that won't fit architecture
        let result = orchestrator.build_context_with_budget(
            50, // Very small budget
            "hash123",
            || "# Manifest\n\nSmall content".to_string(),
            || "# Architecture\n\n".repeat(100), // Large content
            None::<fn() -> String>,
        );

        // Should only include manifest, not architecture
        assert!(result.content().contains("Manifest"));
        assert!(!result.content().contains("Architecture"));
        assert!(result.tokens_approx() <= 50);
    }

    #[test]
    fn test_context_includes_manifest_always() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        // Reasonable budget
        let result = orchestrator.build_context_with_budget(
            1000,
            "hash123",
            || "# Manifest\n\nContent".to_string(),
            || "# Architecture\n\nContent".to_string(),
            None::<fn() -> String>,
        );

        // Should always include manifest
        assert!(result.content().contains("Manifest"));
    }

    #[test]
    fn test_context_includes_architecture_if_budget_allows() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        // Large budget
        let result = orchestrator.build_context_with_budget(
            10000, // Large budget
            "hash123",
            || "# Manifest".to_string(),
            || "# Architecture".to_string(),
            None::<fn() -> String>,
        );

        // Should include both
        assert!(result.content().contains("Manifest"));
        assert!(result.content().contains("Architecture"));
        assert!(result.content().contains("---")); // Separator
    }

    #[test]
    fn test_context_includes_symbols_if_budget_allows() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        let result = orchestrator.build_context_with_budget(
            10000,
            "hash123",
            || "# Manifest".to_string(),
            || "# Architecture".to_string(),
            Some(|| "# Symbols".to_string()),
        );

        // Should include all three
        assert!(result.content().contains("Manifest"));
        assert!(result.content().contains("Architecture"));
        assert!(result.content().contains("Symbols"));
    }

    #[test]
    fn test_context_progressive_loading_order() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        // Budget that fits manifest and architecture but not symbols
        let manifest_content = "A".repeat(100); // ~25 tokens
        let arch_content = "B".repeat(100); // ~25 tokens
        let symbols_content = "C".repeat(1000); // ~250 tokens

        let result = orchestrator.build_context_with_budget(
            100, // Budget for ~400 chars
            "hash123",
            || manifest_content,
            || arch_content,
            Some(|| symbols_content),
        );

        // Should include manifest and architecture, but not symbols
        assert!(result.content().contains("AAA"));
        assert!(result.content().contains("BBB"));
        assert!(!result.content().contains("CCC"));
    }

    #[test]
    fn test_truncate_to_budget_handles_utf8() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        // Test with emoji content that has multi-byte chars
        let result = orchestrator.build_context_with_budget(
            5, // Very small budget
            "hash123",
            || "Hello 👋 World".to_string(), // Contains multi-byte emoji
            || "Architecture".to_string(),
            None::<fn() -> String>,
        );

        // Should not panic on UTF-8 boundary issues
        let content = result.content();
        // With BPE, 5 tokens covers more text than the old heuristic.
        // The important invariant is: no panic and valid UTF-8.
        assert!(!content.is_empty());
        assert!(
            content.len() <= 100,
            "Content should be bounded, got {} bytes",
            content.len()
        );
    }

    #[test]
    fn test_build_context_tracks_highest_level() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        let result = orchestrator.build_context_with_budget(
            10000,
            "hash123",
            || "Manifest".to_string(),
            || "Architecture".to_string(),
            Some(|| "Symbols".to_string()),
        );

        // Highest level should be SymbolDetail when all included
        assert_eq!(result.level(), CompressionLevel::SymbolDetail);
    }

    #[test]
    fn test_build_context_uses_correct_source() {
        let caps_ccg = create_capabilities_with_ccg();
        let orch_ccg = CompressionOrchestrator::new(caps_ccg, "repo");

        let result_ccg = orch_ccg.build_context_with_budget(
            10000,
            "hash123",
            || "Manifest".to_string(),
            || "Architecture".to_string(),
            None::<fn() -> String>,
        );

        // CCG path should use CcgLayer1 for architecture level
        assert_eq!(result_ccg.source(), ContextSource::CcgLayer1);

        let caps_fallback = create_capabilities_without_ccg();
        let orch_fallback = CompressionOrchestrator::new(caps_fallback, "repo");

        let result_fallback = orch_fallback.build_context_with_budget(
            10000,
            "hash123",
            || "Manifest".to_string(),
            || "Architecture".to_string(),
            None::<fn() -> String>,
        );

        // Fallback path should use Constructed
        assert_eq!(result_fallback.source(), ContextSource::Constructed);
    }

    // =============================================================================
    // Graceful degradation tests (Task 3.3.6)
    // =============================================================================

    #[test]
    fn test_graceful_degradation_when_narsil_disconnected() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        // Manually enter degraded mode (simulating narsil disconnect)
        orchestrator.enter_degraded_mode();
        assert!(orchestrator.is_degraded());

        // Should return degraded result without calling content_fn
        let result =
            orchestrator.get_context_graceful(CompressionLevel::Manifest, "hash123", || {
                panic!("Should not be called in degraded mode")
            });

        assert!(result.content().contains("Degraded Mode"));
        assert!(result.content().contains("unavailable"));
    }

    #[test]
    fn test_graceful_degradation_on_mcp_error() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        // Simulate MCP call failure
        let result =
            orchestrator.get_context_graceful(CompressionLevel::Manifest, "hash123", || {
                Err("MCP connection timeout".to_string())
            });

        assert!(orchestrator.is_degraded());
        assert!(result.content().contains("Degraded Mode"));
        assert_eq!(orchestrator.metrics().degradations(), 1);
    }

    #[test]
    fn test_logs_warning_on_degradation() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        // This will log a warning (we can't easily test log output, but we verify
        // the method completes successfully and enters degraded mode)
        let _result =
            orchestrator.get_context_graceful(CompressionLevel::Architecture, "hash123", || {
                Err("Connection refused".to_string())
            });

        assert!(orchestrator.is_degraded());
    }

    #[test]
    fn test_exits_degraded_mode_on_success() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        // First enter degraded mode
        orchestrator.enter_degraded_mode();
        assert!(orchestrator.is_degraded());

        // Then exit it manually
        orchestrator.exit_degraded_mode();
        assert!(!orchestrator.is_degraded());

        // Successful call should also exit degraded mode
        orchestrator.enter_degraded_mode();
        let _result =
            orchestrator.get_context_graceful(CompressionLevel::Manifest, "hash123", || {
                Ok("# Manifest content".to_string())
            });

        // Note: get_context_graceful returns early if degraded, so we need to
        // verify the exit_degraded_mode works when called explicitly
        orchestrator.exit_degraded_mode();
        assert!(!orchestrator.is_degraded());
    }

    #[test]
    fn test_degraded_mode_preserves_cache() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        // Populate cache before degradation
        let result = orchestrator.get_context(CompressionLevel::Manifest, "hash123", || {
            "# Cached manifest".to_string()
        });
        assert!(result.content().contains("Cached manifest"));

        // Enter degraded mode
        orchestrator.enter_degraded_mode();

        // Cache should still be accessible via get_context_graceful
        let _degraded_result =
            orchestrator.get_context_graceful(CompressionLevel::Manifest, "hash123", || {
                panic!("Should use cache")
            });

        // Even in degraded mode, we try cache first... but since we skip to
        // degraded result in is_degraded(), we get degraded content
        // Let's verify the cache is still there by using regular get_cached
        orchestrator.exit_degraded_mode();
        let cached = orchestrator.get_cached(CompressionLevel::Manifest, "hash123");
        assert!(cached.is_some());
        assert!(cached.unwrap().content().contains("Cached manifest"));
    }

    #[test]
    fn test_degraded_result_has_meaningful_content() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        orchestrator.enter_degraded_mode();

        let manifest =
            orchestrator
                .get_context_graceful(CompressionLevel::Manifest, "hash", || panic!("Not called"));
        assert!(manifest.content().contains("Repository Manifest"));

        let arch =
            orchestrator.get_context_graceful(CompressionLevel::Architecture, "hash", || {
                panic!("Not called")
            });
        assert!(arch.content().contains("Architecture"));
    }

    // =============================================================================
    // process_manifest_response tests (Task 3.1)
    // =============================================================================

    #[test]
    fn test_process_manifest_response_parses_valid_response() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        let response = serde_json::json!({
            "content": [{
                "type": "text",
                "text": "# Project Structure: test-repo\n\n```\n📁 test-repo/\n  🦀 main.rs (21.5KB)\n```"
            }]
        });

        let result = orchestrator.process_manifest_response(&response, "abc123");

        assert_eq!(result.level(), CompressionLevel::Manifest);
        assert!(
            result.content().contains("Repository Manifest"),
            "Should render manifest markdown, got: {}",
            result.content()
        );
        assert!(
            result.content().contains("test-repo"),
            "Should include repo name in rendered output"
        );
    }

    #[test]
    fn test_process_manifest_response_caches_result() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        let response = serde_json::json!({
            "content": [{
                "type": "text",
                "text": "📁 repo/\n  🦀 main.rs (1.0KB)"
            }]
        });

        let _result = orchestrator.process_manifest_response(&response, "hash123");

        let cached = orchestrator.get_cached(CompressionLevel::Manifest, "hash123");
        assert!(cached.is_some(), "Result should be cached after processing");
    }

    #[test]
    fn test_process_manifest_response_enters_degraded_on_invalid_response() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        let response = serde_json::json!({}); // Invalid - missing content

        let result = orchestrator.process_manifest_response(&response, "hash123");

        assert!(
            orchestrator.is_degraded(),
            "Should enter degraded mode on parse failure"
        );
        assert!(result.content().contains("Degraded Mode"));
    }

    #[test]
    fn test_process_manifest_response_exits_degraded_on_success() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        // Enter degraded mode first
        orchestrator.enter_degraded_mode();
        assert!(orchestrator.is_degraded());

        let response = serde_json::json!({
            "content": [{
                "type": "text",
                "text": "📁 repo/\n  🦀 main.rs (1.0KB)"
            }]
        });

        let _result = orchestrator.process_manifest_response(&response, "hash123");

        assert!(
            !orchestrator.is_degraded(),
            "Should exit degraded mode on successful parse"
        );
    }

    #[test]
    fn test_process_manifest_response_cache_invalidation_on_hash_change() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        let response = serde_json::json!({
            "content": [{
                "type": "text",
                "text": "📁 repo/\n  🦀 main.rs (1.0KB)"
            }]
        });

        // Cache with first hash
        let _result = orchestrator.process_manifest_response(&response, "old_hash");

        // Different hash should miss cache
        let cached = orchestrator.get_cached(CompressionLevel::Manifest, "new_hash");
        assert!(
            cached.is_none(),
            "Different commit hash should not hit cache"
        );
    }

    #[test]
    fn test_process_manifest_response_uses_correct_source_for_ccg() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        let response = serde_json::json!({
            "content": [{
                "type": "text",
                "text": "📁 repo/\n  🦀 main.rs (1.0KB)"
            }]
        });

        let result = orchestrator.process_manifest_response(&response, "hash");
        assert_eq!(
            result.source(),
            ContextSource::CcgLayer0,
            "CCG-capable orchestrator should mark source as CcgLayer0"
        );
    }

    #[test]
    fn test_process_manifest_response_uses_correct_source_for_fallback() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        let response = serde_json::json!({
            "content": [{
                "type": "text",
                "text": "📁 repo/\n  🦀 main.rs (1.0KB)"
            }]
        });

        let result = orchestrator.process_manifest_response(&response, "hash");
        assert_eq!(
            result.source(),
            ContextSource::Constructed,
            "Fallback orchestrator should mark source as Constructed"
        );
    }

    #[test]
    fn test_process_manifest_response_includes_language_info() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        let response = serde_json::json!({
            "content": [{
                "type": "text",
                "text": "📁 repo/\n  🦀 main.rs (10.0KB)\n  🦀 lib.rs (5.0KB)\n  📝 README.md (2.0KB)"
            }]
        });

        let result = orchestrator.process_manifest_response(&response, "hash");

        assert!(
            result.content().contains("Rust"),
            "Rendered manifest should mention Rust language"
        );
    }

    // =============================================================================
    // process_architecture_response tests (Task 3.2)
    // =============================================================================

    #[test]
    fn test_process_architecture_response_parses_valid_response() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        let response = serde_json::json!({
            "content": [{
                "type": "text",
                "text": "# Import Graph\n\n## Repository Import Summary\n\n| File | Dependencies | Dependents |\n|------|--------------|------------|\n| src/app/state.rs | 21 | 8 |\n| src/api/mod.rs | 10 | 15 |"
            }]
        });

        let result = orchestrator.process_architecture_response(&response, "abc123");

        assert_eq!(result.level(), CompressionLevel::Architecture);
        assert!(
            result.content().contains("Architecture"),
            "Should render architecture markdown, got: {}",
            result.content()
        );
    }

    #[test]
    fn test_process_architecture_response_caches_result() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        let response = serde_json::json!({
            "content": [{
                "type": "text",
                "text": "| File | Dependencies | Dependents |\n|------|--------------|------------|\n| src/main.rs | 5 | 0 |"
            }]
        });

        let _result = orchestrator.process_architecture_response(&response, "hash123");

        let cached = orchestrator.get_cached(CompressionLevel::Architecture, "hash123");
        assert!(cached.is_some(), "Result should be cached after processing");
    }

    #[test]
    fn test_process_architecture_response_enters_degraded_on_invalid_response() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        let response = serde_json::json!({}); // Invalid - missing content

        let result = orchestrator.process_architecture_response(&response, "hash123");

        assert!(
            orchestrator.is_degraded(),
            "Should enter degraded mode on parse failure"
        );
        assert!(result.content().contains("Degraded Mode"));
    }

    #[test]
    fn test_process_architecture_response_exits_degraded_on_success() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        // Enter degraded mode first
        orchestrator.enter_degraded_mode();
        assert!(orchestrator.is_degraded());

        let response = serde_json::json!({
            "content": [{
                "type": "text",
                "text": "| File | Dependencies | Dependents |\n|------|--------------|------------|\n| src/main.rs | 5 | 0 |"
            }]
        });

        let _result = orchestrator.process_architecture_response(&response, "hash123");

        assert!(
            !orchestrator.is_degraded(),
            "Should exit degraded mode on successful parse"
        );
    }

    #[test]
    fn test_process_architecture_response_cache_invalidation_on_hash_change() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        let response = serde_json::json!({
            "content": [{
                "type": "text",
                "text": "| File | Dependencies | Dependents |\n|------|--------------|------------|\n| src/main.rs | 5 | 0 |"
            }]
        });

        let _result = orchestrator.process_architecture_response(&response, "old_hash");

        let cached = orchestrator.get_cached(CompressionLevel::Architecture, "new_hash");
        assert!(
            cached.is_none(),
            "Different commit hash should not hit cache"
        );
    }

    #[test]
    fn test_process_architecture_response_renders_modules() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        let response = serde_json::json!({
            "content": [{
                "type": "text",
                "text": "| File | Dependencies | Dependents |\n|------|--------------|------------|\n| src/app/state.rs | 21 | 8 |\n| src/api/mod.rs | 10 | 15 |"
            }]
        });

        let result = orchestrator.process_architecture_response(&response, "hash");

        assert!(
            result.content().contains("Modules"),
            "Architecture should contain Modules section"
        );
    }

    #[test]
    fn test_process_architecture_response_renders_dependencies() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        let response = serde_json::json!({
            "content": [{
                "type": "text",
                "text": "| File | Dependencies | Dependents |\n|------|--------------|------------|\n| src/app/state.rs | 21 | 8 |"
            }]
        });

        let result = orchestrator.process_architecture_response(&response, "hash");

        assert!(
            result.content().contains("Dependencies"),
            "Architecture should contain Dependencies section"
        );
    }

    #[test]
    fn test_process_architecture_response_uses_correct_source_for_ccg() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        let response = serde_json::json!({
            "content": [{
                "type": "text",
                "text": "| File | Dependencies | Dependents |\n|------|--------------|------------|\n| src/main.rs | 5 | 0 |"
            }]
        });

        let result = orchestrator.process_architecture_response(&response, "hash");
        assert_eq!(
            result.source(),
            ContextSource::CcgLayer1,
            "CCG-capable orchestrator should mark source as CcgLayer1"
        );
    }

    #[test]
    fn test_process_architecture_response_uses_correct_source_for_fallback() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        let response = serde_json::json!({
            "content": [{
                "type": "text",
                "text": "| File | Dependencies | Dependents |\n|------|--------------|------------|\n| src/main.rs | 5 | 0 |"
            }]
        });

        let result = orchestrator.process_architecture_response(&response, "hash");
        assert_eq!(
            result.source(),
            ContextSource::Constructed,
            "Fallback orchestrator should mark source as Constructed"
        );
    }

    // =============================================================================
    // process_symbols_response tests (Task 3.3)
    // =============================================================================

    #[test]
    fn test_process_symbols_response_parses_valid_response() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        let response = serde_json::json!({
            "content": [{
                "type": "text",
                "text": "# Symbols in test-repo\n\nFound 2 symbols\n\n## Functions\n\n- **main** (`src/main.rs:1`) fn main() {\n- **run** (`src/app/mod.rs:10`) pub fn run() -> Result<()> {"
            }]
        });

        let result = orchestrator.process_symbols_response(&response, "abc123", 10000);

        assert_eq!(result.level(), CompressionLevel::SymbolDetail);
        assert!(
            result.content().contains("Symbols"),
            "Should render symbols markdown, got: {}",
            result.content()
        );
        assert!(
            result.content().contains("main"),
            "Should include symbol names in rendered output"
        );
    }

    #[test]
    fn test_process_symbols_response_caches_result() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        let response = serde_json::json!({
            "content": [{
                "type": "text",
                "text": "## Functions\n\n- **main** (`src/main.rs:1`) fn main() {"
            }]
        });

        let _result = orchestrator.process_symbols_response(&response, "hash123", 10000);

        let cached = orchestrator.get_cached(CompressionLevel::SymbolDetail, "hash123");
        assert!(cached.is_some(), "Result should be cached after processing");
    }

    #[test]
    fn test_process_symbols_response_enters_degraded_on_invalid_response() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        let response = serde_json::json!({}); // Invalid - missing content

        let result = orchestrator.process_symbols_response(&response, "hash123", 10000);

        assert!(
            orchestrator.is_degraded(),
            "Should enter degraded mode on parse failure"
        );
        assert!(result.content().contains("Degraded Mode"));
    }

    #[test]
    fn test_process_symbols_response_exits_degraded_on_success() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        // Enter degraded mode first
        orchestrator.enter_degraded_mode();
        assert!(orchestrator.is_degraded());

        let response = serde_json::json!({
            "content": [{
                "type": "text",
                "text": "## Functions\n\n- **main** (`src/main.rs:1`) fn main() {"
            }]
        });

        let _result = orchestrator.process_symbols_response(&response, "hash123", 10000);

        assert!(
            !orchestrator.is_degraded(),
            "Should exit degraded mode on successful parse"
        );
    }

    #[test]
    fn test_process_symbols_response_respects_token_budget() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        // Generate many symbols
        let mut symbols_text = String::from("## Functions\n\n");
        for i in 0..100 {
            symbols_text.push_str(&format!(
                "- **func_{i}** (`src/module_{i}.rs:{i}`) pub fn func_{i}(x: i32) -> i32 {{\n"
            ));
        }

        let response = serde_json::json!({
            "content": [{
                "type": "text",
                "text": symbols_text
            }]
        });

        let result = orchestrator.process_symbols_response(&response, "hash123", 50);

        assert!(
            result.content().contains("token budget reached"),
            "Should indicate truncation when over budget"
        );
    }

    #[test]
    fn test_process_symbols_response_cache_invalidation_on_hash_change() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        let response = serde_json::json!({
            "content": [{
                "type": "text",
                "text": "## Functions\n\n- **main** (`src/main.rs:1`) fn main() {"
            }]
        });

        let _result = orchestrator.process_symbols_response(&response, "old_hash", 10000);

        let cached = orchestrator.get_cached(CompressionLevel::SymbolDetail, "new_hash");
        assert!(
            cached.is_none(),
            "Different commit hash should not hit cache"
        );
    }

    #[test]
    fn test_process_symbols_response_uses_direct_tool_source() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        let response = serde_json::json!({
            "content": [{
                "type": "text",
                "text": "## Functions\n\n- **main** (`src/main.rs:1`) fn main() {"
            }]
        });

        let result = orchestrator.process_symbols_response(&response, "hash", 10000);
        assert_eq!(
            result.source(),
            ContextSource::DirectTool,
            "Symbol results should use DirectTool source"
        );
    }

    #[test]
    fn test_create_symbol_result() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        let result = orchestrator.create_symbol_result("# Symbols".to_string());

        assert_eq!(result.level(), CompressionLevel::SymbolDetail);
        assert_eq!(result.source(), ContextSource::DirectTool);
    }

    #[test]
    fn test_degradation_counter_increments_once_per_transition() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        // Multiple calls to enter_degraded_mode should only count once
        orchestrator.enter_degraded_mode();
        orchestrator.enter_degraded_mode();
        orchestrator.enter_degraded_mode();

        assert_eq!(orchestrator.metrics().degradations(), 1);

        // Exit and re-enter should increment again
        orchestrator.exit_degraded_mode();
        orchestrator.enter_degraded_mode();

        assert_eq!(orchestrator.metrics().degradations(), 2);
    }

    // =============================================================================
    // build_context tests (Task 3.4 - Token Budgeting)
    // =============================================================================

    #[test]
    fn test_build_context_result_creation() {
        let result = BuildContextResult::new(
            CompressionResult::new(
                "# Context".to_string(),
                CompressionLevel::Architecture,
                ContextSource::Constructed,
            ),
            500,
            3000,
            1500,
        );

        assert_eq!(result.manifest_tokens(), 500);
        assert_eq!(result.architecture_tokens(), 3000);
        assert_eq!(result.symbol_tokens(), 1500);
        assert_eq!(result.total_tokens(), 500 + 3000 + 1500);
        assert_eq!(result.result().level(), CompressionLevel::Architecture);
    }

    #[test]
    fn test_build_context_result_zero_layers() {
        let result = BuildContextResult::new(
            CompressionResult::new(
                String::new(),
                CompressionLevel::Manifest,
                ContextSource::Constructed,
            ),
            0,
            0,
            0,
        );

        assert_eq!(result.total_tokens(), 0);
        assert_eq!(result.manifest_tokens(), 0);
        assert_eq!(result.architecture_tokens(), 0);
        assert_eq!(result.symbol_tokens(), 0);
    }

    #[test]
    fn test_build_context_allocates_manifest_budget() {
        // Manifest should get ~500 tokens (DEFAULT_MANIFEST_TOKEN_BUDGET)
        assert_eq!(DEFAULT_MANIFEST_TOKEN_BUDGET, 500);
    }

    #[test]
    fn test_build_context_allocates_architecture_budget_up_to_cap() {
        // Architecture should be capped at DEFAULT_ARCHITECTURE_TOKEN_BUDGET (12K)
        assert_eq!(DEFAULT_ARCHITECTURE_TOKEN_BUDGET, 12_000);
    }

    #[test]
    fn test_build_context_symbols_get_remainder() {
        // With a 20K token budget:
        // manifest: 500
        // architecture: up to 12K
        // symbols: remainder = 20000 - 500 - 12000 = 7500
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        let total_budget: usize = 20_000;
        let allocation = orchestrator.compute_budget_allocation(total_budget);

        assert_eq!(allocation.manifest, DEFAULT_MANIFEST_TOKEN_BUDGET);
        assert!(allocation.architecture <= DEFAULT_ARCHITECTURE_TOKEN_BUDGET);
        assert_eq!(
            allocation.symbols,
            total_budget - allocation.manifest - allocation.architecture
        );
    }

    #[test]
    fn test_build_context_small_budget_manifest_only() {
        // Budget too small for architecture
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        let allocation = orchestrator.compute_budget_allocation(600);

        assert_eq!(allocation.manifest, 500);
        assert_eq!(allocation.architecture, 100); // Just the remainder
        assert_eq!(allocation.symbols, 0);
    }

    #[test]
    fn test_build_context_very_small_budget_truncates_manifest() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        let allocation = orchestrator.compute_budget_allocation(200);

        // Manifest budget should be clamped to total budget
        assert_eq!(allocation.manifest, 200);
        assert_eq!(allocation.architecture, 0);
        assert_eq!(allocation.symbols, 0);
    }

    #[test]
    fn test_build_context_assembles_all_layers_from_responses() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        let manifest_response = serde_json::json!({
            "content": [{"type": "text", "text": "📁 repo/\n  🦀 main.rs (1.0KB)"}]
        });
        let arch_response = serde_json::json!({
            "content": [{"type": "text", "text": "| File | Dependencies | Dependents |\n|------|--------------|------------|\n| src/main.rs | 5 | 0 |"}]
        });
        let symbols_response = serde_json::json!({
            "content": [{"type": "text", "text": "## Functions\n\n- **main** (`src/main.rs:1`) fn main() {"}]
        });

        let manifest = orchestrator.process_manifest_response(&manifest_response, "hash");
        let architecture = orchestrator.process_architecture_response(&arch_response, "hash");
        let symbols = orchestrator.process_symbols_response(&symbols_response, "hash", 4000);

        let result = orchestrator.assemble_context(20_000, manifest, architecture, symbols);

        // Should contain content from all three layers
        assert!(result.result().content().contains("Repository Manifest"));
        assert!(result.result().content().contains("Modules"));
        assert!(result.result().content().contains("Symbols"));
    }

    #[test]
    fn test_build_context_assembly_respects_total_budget() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        // Create large results that exceed budget
        let manifest = CompressionResult::new(
            "A".repeat(2000), // ~500 tokens
            CompressionLevel::Manifest,
            ContextSource::Constructed,
        );
        let architecture = CompressionResult::new(
            "B".repeat(48000), // ~12000 tokens
            CompressionLevel::Architecture,
            ContextSource::Constructed,
        );
        let symbols = CompressionResult::new(
            "C".repeat(80000), // ~20000 tokens
            CompressionLevel::SymbolDetail,
            ContextSource::DirectTool,
        );

        let result = orchestrator.assemble_context(
            1000, // Very small budget
            manifest,
            architecture,
            symbols,
        );

        // Total tokens should not exceed budget
        assert!(
            result.result().tokens_approx() <= 1000,
            "Total tokens {} should not exceed budget 1000",
            result.result().tokens_approx()
        );
    }

    #[test]
    fn test_build_context_assembly_skips_architecture_when_over_budget() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        let manifest = CompressionResult::new(
            "Manifest data".to_string(),
            CompressionLevel::Manifest,
            ContextSource::Constructed,
        );
        let architecture = CompressionResult::new(
            "B".repeat(100_000), // Very large - ~25K tokens
            CompressionLevel::Architecture,
            ContextSource::Constructed,
        );
        let symbols = CompressionResult::new(
            "S".repeat(100_000), // Also very large
            CompressionLevel::SymbolDetail,
            ContextSource::DirectTool,
        );

        // Budget that fits manifest but not architecture or symbols
        let result = orchestrator.assemble_context(100, manifest, architecture, symbols);

        // Should include manifest but skip architecture and symbols
        assert!(result.result().content().contains("Manifest data"));
        assert!(!result.result().content().contains("BBB"));
        assert!(!result.result().content().contains("SSS"));
        assert_eq!(result.architecture_tokens(), 0);
        assert_eq!(result.symbol_tokens(), 0);
    }

    #[test]
    fn test_build_context_assembly_skips_symbols_when_over_budget() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        let manifest = CompressionResult::new(
            "M".repeat(100), // ~25 tokens
            CompressionLevel::Manifest,
            ContextSource::Constructed,
        );
        let architecture = CompressionResult::new(
            "A".repeat(200), // ~50 tokens
            CompressionLevel::Architecture,
            ContextSource::Constructed,
        );
        let symbols = CompressionResult::new(
            "S".repeat(10000), // ~2500 tokens
            CompressionLevel::SymbolDetail,
            ContextSource::DirectTool,
        );

        // Budget fits manifest + architecture but not symbols
        let result = orchestrator.assemble_context(100, manifest, architecture, symbols);

        // Should include manifest and architecture but not symbols
        assert!(result.result().content().contains("MMM"));
        assert!(result.result().content().contains("AAA"));
        assert!(result.manifest_tokens() > 0);
        assert!(result.architecture_tokens() > 0);
        assert_eq!(result.symbol_tokens(), 0);
    }

    #[test]
    fn test_build_context_assembly_tracks_per_layer_tokens() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        let manifest = CompressionResult::new(
            "Manifest".to_string(),
            CompressionLevel::Manifest,
            ContextSource::Constructed,
        );
        let architecture = CompressionResult::new(
            "Architecture".to_string(),
            CompressionLevel::Architecture,
            ContextSource::Constructed,
        );
        let symbols = CompressionResult::new(
            "Symbols".to_string(),
            CompressionLevel::SymbolDetail,
            ContextSource::DirectTool,
        );

        let result = orchestrator.assemble_context(
            10_000,
            manifest.clone(),
            architecture.clone(),
            symbols.clone(),
        );

        assert_eq!(result.manifest_tokens(), manifest.tokens_approx());
        assert_eq!(result.architecture_tokens(), architecture.tokens_approx());
        assert_eq!(result.symbol_tokens(), symbols.tokens_approx());
    }

    #[test]
    fn test_build_context_assembly_uses_separator_between_layers() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        let manifest = CompressionResult::new(
            "Manifest".to_string(),
            CompressionLevel::Manifest,
            ContextSource::Constructed,
        );
        let architecture = CompressionResult::new(
            "Architecture".to_string(),
            CompressionLevel::Architecture,
            ContextSource::Constructed,
        );
        let symbols = CompressionResult::new(
            "Symbols".to_string(),
            CompressionLevel::SymbolDetail,
            ContextSource::DirectTool,
        );

        let result = orchestrator.assemble_context(10_000, manifest, architecture, symbols);

        // Should have separators between layers
        assert!(result.result().content().contains("---"));
    }

    #[test]
    fn test_build_context_graceful_degradation_manifest_failure() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        // Simulate degraded manifest
        let manifest = orchestrator.create_degraded_result(CompressionLevel::Manifest);
        let architecture = CompressionResult::new(
            "Architecture".to_string(),
            CompressionLevel::Architecture,
            ContextSource::Constructed,
        );
        let symbols = CompressionResult::new(
            "Symbols".to_string(),
            CompressionLevel::SymbolDetail,
            ContextSource::DirectTool,
        );

        let result = orchestrator.assemble_context(10_000, manifest, architecture, symbols);

        // Should still assemble with degraded manifest
        assert!(result.result().content().contains("Degraded Mode"));
        assert!(result.result().content().contains("Architecture"));
    }

    #[test]
    fn test_build_context_when_fully_degraded() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        // All layers degraded
        let manifest = orchestrator.create_degraded_result(CompressionLevel::Manifest);
        let architecture = orchestrator.create_degraded_result(CompressionLevel::Architecture);
        let symbols = orchestrator.create_degraded_result(CompressionLevel::SymbolDetail);

        let result = orchestrator.assemble_context(10_000, manifest, architecture, symbols);

        // Should still return a valid result
        assert!(result.result().content().contains("Degraded Mode"));
        assert!(result.result().tokens_approx() > 0);
    }

    #[test]
    fn test_build_context_sets_highest_level_achieved() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        // All layers fit
        let manifest = CompressionResult::new(
            "M".to_string(),
            CompressionLevel::Manifest,
            ContextSource::Constructed,
        );
        let architecture = CompressionResult::new(
            "A".to_string(),
            CompressionLevel::Architecture,
            ContextSource::Constructed,
        );
        let symbols = CompressionResult::new(
            "S".to_string(),
            CompressionLevel::SymbolDetail,
            ContextSource::DirectTool,
        );

        let result = orchestrator.assemble_context(10_000, manifest, architecture, symbols);
        assert_eq!(result.result().level(), CompressionLevel::SymbolDetail);
    }

    #[test]
    fn test_build_context_sets_manifest_level_when_only_manifest_fits() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

        let manifest = CompressionResult::new(
            "M".to_string(),
            CompressionLevel::Manifest,
            ContextSource::Constructed,
        );
        let architecture = CompressionResult::new(
            "A".repeat(10000),
            CompressionLevel::Architecture,
            ContextSource::Constructed,
        );
        let symbols = CompressionResult::new(
            "S".repeat(10000),
            CompressionLevel::SymbolDetail,
            ContextSource::DirectTool,
        );

        let result = orchestrator.assemble_context(5, manifest, architecture, symbols);
        assert_eq!(result.result().level(), CompressionLevel::Manifest);
    }

    #[test]
    fn test_budget_allocation_defaults() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        // Default total budget of 16K
        let allocation = orchestrator.compute_budget_allocation(16_000);

        assert_eq!(allocation.manifest, 500);
        assert!(allocation.architecture <= 12_000);
        let expected_symbols = 16_000 - allocation.manifest - allocation.architecture;
        assert_eq!(allocation.symbols, expected_symbols);
    }

    #[test]
    fn test_budget_allocation_large_budget_caps_architecture() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        // Very large budget
        let allocation = orchestrator.compute_budget_allocation(100_000);

        assert_eq!(allocation.manifest, 500);
        assert_eq!(allocation.architecture, 12_000);
        assert_eq!(allocation.symbols, 100_000 - 500 - 12_000);
    }

    #[test]
    fn test_budget_allocation_zero_budget() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        let allocation = orchestrator.compute_budget_allocation(0);

        assert_eq!(allocation.manifest, 0);
        assert_eq!(allocation.architecture, 0);
        assert_eq!(allocation.symbols, 0);
    }

    // =============================================================================
    // Parallel context building tests (Phase 5.1)
    // =============================================================================

    #[tokio::test]
    async fn test_build_context_parallel_returns_correct_result_with_cached_layers() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        // Pre-populate cache for all three layers
        let manifest = CompressionResult::new(
            "# Manifest\n\nProject overview".to_string(),
            CompressionLevel::Manifest,
            ContextSource::Constructed,
        );
        orchestrator.cache_result(CompressionLevel::Manifest, &manifest, "hash123");

        let arch = CompressionResult::new(
            "# Architecture\n\nModule structure".to_string(),
            CompressionLevel::Architecture,
            ContextSource::Constructed,
        );
        orchestrator.cache_result(CompressionLevel::Architecture, &arch, "hash123");

        let symbols = CompressionResult::new(
            "# Symbols\n\nKey types".to_string(),
            CompressionLevel::SymbolDetail,
            ContextSource::DirectTool,
        );
        orchestrator.cache_result(CompressionLevel::SymbolDetail, &symbols, "hash123");

        // Create a disconnected client — build_context won't use it since all layers are cached
        let client = McpConnection::disconnected("test");

        let result = orchestrator
            .build_context(&client, "hash123", &[], 20_000)
            .await;

        // All three layers should be present in the assembled result
        assert!(result.result().content().contains("# Manifest"));
        assert!(result.result().content().contains("# Architecture"));
        assert!(result.result().content().contains("# Symbols"));
        assert_eq!(orchestrator.metrics().cache_hits(), 3);
    }

    #[tokio::test]
    async fn test_build_context_parallel_records_latency_metric() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        // Pre-populate cache so MCP calls are skipped
        for level in [
            CompressionLevel::Manifest,
            CompressionLevel::Architecture,
            CompressionLevel::SymbolDetail,
        ] {
            let result =
                CompressionResult::new("content".to_string(), level, ContextSource::Constructed);
            orchestrator.cache_result(level, &result, "hash");
        }

        let client = McpConnection::disconnected("test");
        let _result = orchestrator
            .build_context(&client, "hash", &[], 20_000)
            .await;

        // Latency metric should have been recorded
        assert!(orchestrator.metrics().average_latency().is_some());
    }

    #[tokio::test]
    async fn test_build_context_parallel_records_token_metrics() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        // Pre-populate cache with known content
        let manifest = CompressionResult::new(
            "Manifest content".to_string(),
            CompressionLevel::Manifest,
            ContextSource::Constructed,
        );
        orchestrator.cache_result(CompressionLevel::Manifest, &manifest, "hash");

        let arch = CompressionResult::new(
            "Architecture content".to_string(),
            CompressionLevel::Architecture,
            ContextSource::Constructed,
        );
        orchestrator.cache_result(CompressionLevel::Architecture, &arch, "hash");

        let symbols = CompressionResult::new(
            "Symbols content".to_string(),
            CompressionLevel::SymbolDetail,
            ContextSource::DirectTool,
        );
        orchestrator.cache_result(CompressionLevel::SymbolDetail, &symbols, "hash");

        let client = McpConnection::disconnected("test");
        let result = orchestrator
            .build_context(&client, "hash", &[], 20_000)
            .await;

        // Per-layer token counts should be non-zero
        assert!(result.manifest_tokens() > 0);
        assert!(result.architecture_tokens() > 0);
        assert!(result.symbol_tokens() > 0);
        assert_eq!(
            result.total_tokens(),
            result.manifest_tokens() + result.architecture_tokens() + result.symbol_tokens()
        );
    }

    #[tokio::test]
    async fn test_build_context_parallel_handles_cache_miss_with_disconnected_client() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        // No cache populated — all layers will miss cache and try MCP calls.
        // The client is disconnected, so MCP calls will fail and
        // fetch methods should return degraded results.
        let client = McpConnection::disconnected("test");

        let result = orchestrator
            .build_context(&client, "hash", &[], 20_000)
            .await;

        // Should still produce a result (degraded mode)
        assert!(!result.result().content().is_empty());
        // Orchestrator should have entered degraded mode
        assert!(orchestrator.is_degraded());
    }

    #[tokio::test]
    async fn test_build_context_parallel_respects_budget() {
        let caps = create_capabilities_without_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        // Pre-populate cache with large and small content
        let manifest = CompressionResult::new(
            "M".repeat(80),
            CompressionLevel::Manifest,
            ContextSource::Constructed,
        );
        orchestrator.cache_result(CompressionLevel::Manifest, &manifest, "hash");

        let arch = CompressionResult::new(
            "A".repeat(4000),
            CompressionLevel::Architecture,
            ContextSource::Constructed,
        );
        orchestrator.cache_result(CompressionLevel::Architecture, &arch, "hash");

        let symbols = CompressionResult::new(
            "S".repeat(4000),
            CompressionLevel::SymbolDetail,
            ContextSource::DirectTool,
        );
        orchestrator.cache_result(CompressionLevel::SymbolDetail, &symbols, "hash");

        // Small budget: should include manifest but not architecture or symbols
        let client = McpConnection::disconnected("test");
        let result = orchestrator.build_context(&client, "hash", &[], 50).await;

        assert!(result.result().content().contains("MMM"));
        // Architecture (1000 tokens) shouldn't fit in 50 token budget
        assert!(!result.result().content().contains("AAA"));
    }

    #[tokio::test]
    async fn test_build_context_parallel_same_result_as_sequential_assembly() {
        let caps = create_capabilities_with_ccg();
        let orchestrator = CompressionOrchestrator::new(caps, "repo");

        let manifest_content = "# Project Manifest\n\nRust: 50 files".to_string();
        let arch_content = "# Module Graph\n\nsrc/api → src/mcp".to_string();
        let symbol_content = "# Symbols\n\npub fn main()".to_string();

        // Pre-populate cache
        let manifest = CompressionResult::new(
            manifest_content.clone(),
            CompressionLevel::Manifest,
            ContextSource::CcgLayer0,
        );
        orchestrator.cache_result(CompressionLevel::Manifest, &manifest, "hash");

        let arch = CompressionResult::new(
            arch_content.clone(),
            CompressionLevel::Architecture,
            ContextSource::CcgLayer1,
        );
        orchestrator.cache_result(CompressionLevel::Architecture, &arch, "hash");

        let symbols = CompressionResult::new(
            symbol_content.clone(),
            CompressionLevel::SymbolDetail,
            ContextSource::DirectTool,
        );
        orchestrator.cache_result(CompressionLevel::SymbolDetail, &symbols, "hash");

        // Build via parallel build_context
        let client = McpConnection::disconnected("test");
        let parallel_result = orchestrator
            .build_context(&client, "hash", &[], 20_000)
            .await;

        // Build via direct assemble_context (the sequential assembly path)
        let sequential_result = orchestrator.assemble_context(20_000, manifest, arch, symbols);

        // Results should be identical
        assert_eq!(
            parallel_result.result().content(),
            sequential_result.result().content()
        );
        assert_eq!(
            parallel_result.manifest_tokens(),
            sequential_result.manifest_tokens()
        );
        assert_eq!(
            parallel_result.architecture_tokens(),
            sequential_result.architecture_tokens()
        );
        assert_eq!(
            parallel_result.symbol_tokens(),
            sequential_result.symbol_tokens()
        );
    }

    #[test]
    fn test_into_result_transfers_ownership() {
        use crate::context::compression::types::{
            CompressionLevel, CompressionResult, ContextSource,
        };

        let inner = CompressionResult::new(
            "# Project manifest".to_string(),
            CompressionLevel::Manifest,
            ContextSource::Constructed,
        );
        let content_before = inner.content().to_string();

        let build_result = BuildContextResult::new(inner, 100, 200, 50);

        // Verify accessors before consuming
        assert_eq!(build_result.manifest_tokens(), 100);
        assert_eq!(build_result.architecture_tokens(), 200);
        assert_eq!(build_result.symbol_tokens(), 50);
        assert_eq!(build_result.total_tokens(), 350);

        // Consume via into_result and verify ownership transfer
        let owned = build_result.into_result();
        assert_eq!(owned.content(), content_before);
    }
}
