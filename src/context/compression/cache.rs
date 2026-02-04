//! Hash-based cache for compression results.
//!
//! The cache invalidates entries based on the repository's Merkle hash,
//! which changes whenever the indexed files change. This ensures cache
//! coherency without explicit invalidation signals.
//!
//! # Cache Key
//!
//! Cache entries are keyed by `(CompressionLevel, scope)` where:
//! - `CompressionLevel` is the compression granularity
//! - `scope` is an optional path or symbol identifier
//!
//! # Invalidation
//!
//! Entries are invalidated when:
//! - The repository hash changes (indicates code changes)
//! - The TTL expires (configurable, default 5 minutes)
//!
//! # Eviction
//!
//! When the cache reaches capacity, the oldest 10% of entries are evicted.

use crate::context::compression::{CompressionLevel, CompressionResult, ContextSource};
use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Default TTL for cache entries (5 minutes).
pub const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(300);

/// Default maximum cache entries.
pub const DEFAULT_MAX_ENTRIES: usize = 100;

/// A cached compression result with metadata for validation.
#[derive(Debug, Clone)]
pub struct CachedResult {
    /// The cached content
    content: String,
    /// Approximate token count
    tokens_approx: usize,
    /// When the result was computed
    computed_at: Instant,
    /// The repository hash when computed (for invalidation)
    repo_hash: String,
    /// The compression level
    level: CompressionLevel,
}

impl CachedResult {
    /// Creates a new cached result.
    #[must_use]
    pub fn new(
        content: String,
        tokens_approx: usize,
        repo_hash: String,
        level: CompressionLevel,
    ) -> Self {
        Self {
            content,
            tokens_approx,
            computed_at: Instant::now(),
            repo_hash,
            level,
        }
    }

    /// Returns true if the cached result is still valid.
    ///
    /// A result is valid if:
    /// - The repository hash matches the current hash
    /// - The TTL has not expired
    #[must_use]
    pub fn is_valid(&self, current_hash: &str, ttl: Duration) -> bool {
        self.repo_hash == current_hash && self.computed_at.elapsed() < ttl
    }

    /// Converts the cached result to a CompressionResult.
    #[must_use]
    pub fn to_result(&self) -> CompressionResult {
        CompressionResult::with_tokens(
            self.content.clone(),
            self.level,
            ContextSource::Cache,
            self.tokens_approx,
        )
    }

    /// Returns the content.
    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }

    /// Returns the token count.
    #[must_use]
    pub fn tokens_approx(&self) -> usize {
        self.tokens_approx
    }

    /// Returns the repository hash.
    #[must_use]
    pub fn repo_hash(&self) -> &str {
        &self.repo_hash
    }
}

/// Cache key combining compression level and scope.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct CacheKey {
    level: CompressionLevel,
    scope: String,
}

impl CacheKey {
    /// Creates a new cache key.
    #[must_use]
    pub fn new(level: CompressionLevel, scope: impl Into<String>) -> Self {
        Self {
            level,
            scope: scope.into(),
        }
    }

    /// Creates a key for repository-wide context (no scope).
    #[must_use]
    pub fn global(level: CompressionLevel) -> Self {
        Self::new(level, String::new())
    }
}

/// Thread-safe cache for compression results.
///
/// Uses DashMap for concurrent access without external locking.
#[derive(Debug)]
pub struct ResultCache {
    /// The underlying cache storage
    entries: DashMap<CacheKey, CachedResult>,
    /// Time-to-live for cache entries
    ttl: Duration,
    /// Maximum number of entries
    max_entries: usize,
    /// Counter for access ordering (for eviction)
    access_counter: AtomicU64,
}

impl Default for ResultCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ResultCache {
    /// Creates a new empty cache with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(DEFAULT_CACHE_TTL, DEFAULT_MAX_ENTRIES)
    }

    /// Creates a new cache with custom configuration.
    #[must_use]
    pub fn with_config(ttl: Duration, max_entries: usize) -> Self {
        Self {
            entries: DashMap::new(),
            ttl,
            max_entries,
            access_counter: AtomicU64::new(0),
        }
    }

    /// Returns true if the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the number of entries in the cache.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Gets a cached result if valid.
    ///
    /// Returns `None` if:
    /// - No entry exists for the key
    /// - The entry's hash doesn't match the current hash
    /// - The entry's TTL has expired
    pub fn get(&self, key: &CacheKey, current_hash: &str) -> Option<CompressionResult> {
        let entry = self.entries.get(key)?;
        if entry.is_valid(current_hash, self.ttl) {
            self.access_counter.fetch_add(1, Ordering::Relaxed);
            Some(entry.to_result())
        } else {
            // Remove invalid entry
            drop(entry);
            self.entries.remove(key);
            None
        }
    }

    /// Stores a result in the cache.
    ///
    /// If the cache is at capacity, evicts the oldest 10% of entries first.
    pub fn set(&self, key: CacheKey, result: CachedResult) {
        // Check capacity and evict if needed
        if self.entries.len() >= self.max_entries {
            self.evict();
        }

        self.entries.insert(key, result);
    }

    /// Clears all entries from the cache.
    pub fn clear(&self) {
        self.entries.clear();
    }

    /// Evicts the oldest 10% of entries.
    fn evict(&self) {
        let evict_count = self.max_entries / 10;
        if evict_count == 0 {
            return;
        }

        // Collect keys sorted by age (oldest first)
        let mut entries: Vec<_> = self
            .entries
            .iter()
            .map(|e| (e.key().clone(), e.value().computed_at))
            .collect();

        entries.sort_by(|a, b| a.1.cmp(&b.1));

        // Remove the oldest entries
        for (key, _) in entries.into_iter().take(evict_count) {
            self.entries.remove(&key);
        }
    }

    /// Removes entries that have expired based on TTL.
    pub fn cleanup_expired(&self, current_hash: &str) {
        self.entries
            .retain(|_, entry| entry.is_valid(current_hash, self.ttl));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    // =============================================================================
    // CachedResult tests
    // =============================================================================

    #[test]
    fn test_cached_result_is_valid_with_matching_hash() {
        let result = CachedResult::new(
            "test content".to_string(),
            10,
            "abc123".to_string(),
            CompressionLevel::Manifest,
        );

        assert!(result.is_valid("abc123", Duration::from_secs(60)));
    }

    #[test]
    fn test_cached_result_invalid_on_hash_change() {
        let result = CachedResult::new(
            "test content".to_string(),
            10,
            "abc123".to_string(),
            CompressionLevel::Manifest,
        );

        assert!(!result.is_valid("different_hash", Duration::from_secs(60)));
    }

    #[test]
    fn test_cached_result_invalid_after_ttl_expiry() {
        let result = CachedResult::new(
            "test content".to_string(),
            10,
            "abc123".to_string(),
            CompressionLevel::Manifest,
        );

        // Sleep briefly then check with a very short TTL
        thread::sleep(Duration::from_millis(10));
        assert!(!result.is_valid("abc123", Duration::from_nanos(1)));
    }

    #[test]
    fn test_cached_result_to_result() {
        let cached = CachedResult::new(
            "content".to_string(),
            25,
            "hash".to_string(),
            CompressionLevel::Architecture,
        );

        let result = cached.to_result();
        assert_eq!(result.content(), "content");
        assert_eq!(result.tokens_approx(), 25);
        assert_eq!(result.level(), CompressionLevel::Architecture);
        assert_eq!(result.source(), ContextSource::Cache);
    }

    // =============================================================================
    // CacheKey tests
    // =============================================================================

    #[test]
    fn test_cache_key_equality() {
        let key1 = CacheKey::new(CompressionLevel::Manifest, "scope1");
        let key2 = CacheKey::new(CompressionLevel::Manifest, "scope1");
        let key3 = CacheKey::new(CompressionLevel::Manifest, "scope2");

        assert_eq!(key1, key2);
        assert_ne!(key1, key3);
    }

    #[test]
    fn test_cache_key_global() {
        let key = CacheKey::global(CompressionLevel::Architecture);
        let expected = CacheKey::new(CompressionLevel::Architecture, "");

        assert_eq!(key, expected);
    }

    // =============================================================================
    // ResultCache tests
    // =============================================================================

    #[test]
    fn test_cache_new_is_empty() {
        let cache = ResultCache::new();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn test_cache_set_stores_result() {
        let cache = ResultCache::new();
        let key = CacheKey::global(CompressionLevel::Manifest);
        let result = CachedResult::new(
            "content".to_string(),
            10,
            "hash".to_string(),
            CompressionLevel::Manifest,
        );

        cache.set(key, result);

        assert!(!cache.is_empty());
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_cache_hit_returns_result() {
        let cache = ResultCache::new();
        let key = CacheKey::global(CompressionLevel::Manifest);
        let result = CachedResult::new(
            "test content".to_string(),
            10,
            "current_hash".to_string(),
            CompressionLevel::Manifest,
        );

        cache.set(key.clone(), result);

        let retrieved = cache.get(&key, "current_hash");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().content(), "test content");
    }

    #[test]
    fn test_cache_miss_returns_none() {
        let cache = ResultCache::new();
        let key = CacheKey::global(CompressionLevel::Manifest);

        let retrieved = cache.get(&key, "any_hash");
        assert!(retrieved.is_none());
    }

    #[test]
    fn test_cache_invalidation_on_hash_change() {
        let cache = ResultCache::new();
        let key = CacheKey::global(CompressionLevel::Manifest);
        let result = CachedResult::new(
            "content".to_string(),
            10,
            "old_hash".to_string(),
            CompressionLevel::Manifest,
        );

        cache.set(key.clone(), result);

        // With matching hash - should hit
        let hit = cache.get(&key, "old_hash");
        assert!(hit.is_some());

        // Store again with old hash
        let result2 = CachedResult::new(
            "content2".to_string(),
            10,
            "old_hash".to_string(),
            CompressionLevel::Manifest,
        );
        cache.set(key.clone(), result2);

        // With different hash - should miss (and remove entry)
        let miss = cache.get(&key, "new_hash");
        assert!(miss.is_none());
    }

    #[test]
    fn test_cache_clear() {
        let cache = ResultCache::new();
        let key1 = CacheKey::new(CompressionLevel::Manifest, "scope1");
        let key2 = CacheKey::new(CompressionLevel::Architecture, "scope2");

        cache.set(
            key1,
            CachedResult::new(
                "a".to_string(),
                1,
                "h".to_string(),
                CompressionLevel::Manifest,
            ),
        );
        cache.set(
            key2,
            CachedResult::new(
                "b".to_string(),
                1,
                "h".to_string(),
                CompressionLevel::Architecture,
            ),
        );

        assert_eq!(cache.len(), 2);

        cache.clear();

        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn test_cache_eviction_at_capacity() {
        // Create a small cache that will trigger eviction
        let cache = ResultCache::with_config(DEFAULT_CACHE_TTL, 10);

        // Fill the cache
        for i in 0..10 {
            let key = CacheKey::new(CompressionLevel::Manifest, format!("scope{}", i));
            // Add small delays to ensure different timestamps
            if i > 0 {
                thread::sleep(Duration::from_millis(1));
            }
            cache.set(
                key,
                CachedResult::new(
                    format!("content{}", i),
                    10,
                    "hash".to_string(),
                    CompressionLevel::Manifest,
                ),
            );
        }

        assert_eq!(cache.len(), 10);

        // Add one more - should trigger eviction
        let new_key = CacheKey::new(CompressionLevel::Manifest, "scope_new");
        cache.set(
            new_key,
            CachedResult::new(
                "new content".to_string(),
                10,
                "hash".to_string(),
                CompressionLevel::Manifest,
            ),
        );

        // Should have evicted 10% (1 entry) before adding the new one
        // So we should have 10 entries (10 - 1 + 1 = 10)
        assert!(cache.len() <= 10);
    }

    #[test]
    fn test_cache_with_custom_ttl() {
        let cache = ResultCache::with_config(Duration::from_millis(50), 100);
        let key = CacheKey::global(CompressionLevel::Manifest);
        let result = CachedResult::new(
            "content".to_string(),
            10,
            "hash".to_string(),
            CompressionLevel::Manifest,
        );

        cache.set(key.clone(), result);

        // Should hit immediately
        assert!(cache.get(&key, "hash").is_some());

        // Wait for TTL to expire
        thread::sleep(Duration::from_millis(60));

        // Should miss after expiry
        assert!(cache.get(&key, "hash").is_none());
    }

    #[test]
    fn test_cache_cleanup_expired() {
        let cache = ResultCache::with_config(Duration::from_millis(10), 100);
        let key1 = CacheKey::new(CompressionLevel::Manifest, "scope1");
        let key2 = CacheKey::new(CompressionLevel::Architecture, "scope2");

        cache.set(
            key1,
            CachedResult::new(
                "a".to_string(),
                1,
                "h".to_string(),
                CompressionLevel::Manifest,
            ),
        );
        cache.set(
            key2,
            CachedResult::new(
                "b".to_string(),
                1,
                "h".to_string(),
                CompressionLevel::Architecture,
            ),
        );

        assert_eq!(cache.len(), 2);

        // Wait for entries to expire
        thread::sleep(Duration::from_millis(20));

        // Cleanup should remove expired entries
        cache.cleanup_expired("h");

        assert!(cache.is_empty());
    }
}
