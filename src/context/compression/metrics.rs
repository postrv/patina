//! Metrics for compression operations.
//!
//! Provides atomic counters for tracking cache performance and
//! backend usage. All operations are thread-safe using atomic primitives.
//!
//! # Example
//!
//! ```
//! use patina::context::compression::CompressionMetrics;
//!
//! let metrics = CompressionMetrics::new();
//!
//! // Record cache operations
//! metrics.record_cache_hit();
//! metrics.record_cache_hit();
//! metrics.record_cache_miss();
//!
//! // Check hit rate
//! let hit_rate = metrics.hit_rate();
//! assert!(hit_rate > 0.6); // 2/3 = 66.7% hit rate
//! ```

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Metrics for compression operations.
///
/// All counters use atomic operations for thread-safe access.
#[derive(Debug, Default)]
pub struct CompressionMetrics {
    /// Number of cache hits
    cache_hits: AtomicU64,
    /// Number of cache misses
    cache_misses: AtomicU64,
    /// Number of CCG backend calls
    ccg_calls: AtomicU64,
    /// Number of fallback backend calls
    fallback_calls: AtomicU64,
    /// Total latency in microseconds (for calculating average)
    total_latency_us: AtomicU64,
    /// Number of latency samples
    latency_samples: AtomicU64,
    /// Number of times degradation occurred
    degradations: AtomicU64,
}

impl CompressionMetrics {
    /// Creates a new metrics instance with all counters at zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a cache hit.
    pub fn record_cache_hit(&self) {
        self.cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a cache miss.
    pub fn record_cache_miss(&self) {
        self.cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a CCG backend call.
    pub fn record_ccg_call(&self) {
        self.ccg_calls.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a fallback backend call.
    pub fn record_fallback_call(&self) {
        self.fallback_calls.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a latency sample.
    pub fn record_latency(&self, duration: Duration) {
        let us = duration.as_micros() as u64;
        self.total_latency_us.fetch_add(us, Ordering::Relaxed);
        self.latency_samples.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a degradation event.
    pub fn record_degradation(&self) {
        self.degradations.fetch_add(1, Ordering::Relaxed);
    }

    /// Returns the number of degradation events.
    #[must_use]
    pub fn degradations(&self) -> u64 {
        self.degradations.load(Ordering::Relaxed)
    }

    /// Returns the number of cache hits.
    #[must_use]
    pub fn cache_hits(&self) -> u64 {
        self.cache_hits.load(Ordering::Relaxed)
    }

    /// Returns the number of cache misses.
    #[must_use]
    pub fn cache_misses(&self) -> u64 {
        self.cache_misses.load(Ordering::Relaxed)
    }

    /// Returns the number of CCG backend calls.
    #[must_use]
    pub fn ccg_calls(&self) -> u64 {
        self.ccg_calls.load(Ordering::Relaxed)
    }

    /// Returns the number of fallback backend calls.
    #[must_use]
    pub fn fallback_calls(&self) -> u64 {
        self.fallback_calls.load(Ordering::Relaxed)
    }

    /// Returns the cache hit rate as a value between 0.0 and 1.0.
    ///
    /// Returns 0.0 if no cache operations have been recorded.
    #[must_use]
    pub fn hit_rate(&self) -> f64 {
        let hits = self.cache_hits.load(Ordering::Relaxed);
        let misses = self.cache_misses.load(Ordering::Relaxed);
        let total = hits + misses;

        if total == 0 {
            0.0
        } else {
            hits as f64 / total as f64
        }
    }

    /// Returns the average latency.
    ///
    /// Returns `None` if no latency samples have been recorded.
    #[must_use]
    pub fn average_latency(&self) -> Option<Duration> {
        let samples = self.latency_samples.load(Ordering::Relaxed);
        if samples == 0 {
            return None;
        }

        let total_us = self.total_latency_us.load(Ordering::Relaxed);
        let avg_us = total_us / samples;
        Some(Duration::from_micros(avg_us))
    }

    /// Returns a summary of all metrics.
    #[must_use]
    pub fn summary(&self) -> MetricsSummary {
        MetricsSummary {
            cache_hits: self.cache_hits.load(Ordering::Relaxed),
            cache_misses: self.cache_misses.load(Ordering::Relaxed),
            ccg_calls: self.ccg_calls.load(Ordering::Relaxed),
            fallback_calls: self.fallback_calls.load(Ordering::Relaxed),
            degradations: self.degradations.load(Ordering::Relaxed),
            hit_rate: self.hit_rate(),
            average_latency: self.average_latency(),
        }
    }

    /// Resets all counters to zero.
    pub fn reset(&self) {
        self.cache_hits.store(0, Ordering::Relaxed);
        self.cache_misses.store(0, Ordering::Relaxed);
        self.ccg_calls.store(0, Ordering::Relaxed);
        self.fallback_calls.store(0, Ordering::Relaxed);
        self.total_latency_us.store(0, Ordering::Relaxed);
        self.latency_samples.store(0, Ordering::Relaxed);
        self.degradations.store(0, Ordering::Relaxed);
    }
}

/// Summary of compression metrics.
#[derive(Debug, Clone)]
pub struct MetricsSummary {
    /// Number of cache hits
    pub cache_hits: u64,
    /// Number of cache misses
    pub cache_misses: u64,
    /// Number of CCG backend calls
    pub ccg_calls: u64,
    /// Number of fallback backend calls
    pub fallback_calls: u64,
    /// Number of degradation events
    pub degradations: u64,
    /// Cache hit rate (0.0 to 1.0)
    pub hit_rate: f64,
    /// Average latency (if any samples recorded)
    pub average_latency: Option<Duration>,
}

impl std::fmt::Display for MetricsSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "hits={} misses={} hit_rate={:.1}% ccg={} fallback={}",
            self.cache_hits,
            self.cache_misses,
            self.hit_rate * 100.0,
            self.ccg_calls,
            self.fallback_calls
        )?;

        if let Some(latency) = &self.average_latency {
            write!(f, " avg_latency={:?}", latency)?;
        }

        Ok(())
    }
}

/// Metrics for context compaction operations.
///
/// Tracks the number of compactions performed, total tokens saved,
/// and total time spent compacting. All operations are thread-safe
/// using atomic primitives.
///
/// # Example
///
/// ```
/// use patina::context::compression::CompactionMetrics;
/// use std::time::Duration;
///
/// let metrics = CompactionMetrics::new();
///
/// // Record a compaction operation
/// metrics.record_compaction(5000, Duration::from_millis(100));
///
/// // Check metrics
/// assert_eq!(metrics.compaction_count(), 1);
/// assert_eq!(metrics.total_tokens_saved(), 5000);
/// ```
#[derive(Debug, Default)]
pub struct CompactionMetrics {
    /// Number of compaction operations performed
    compaction_count: AtomicU64,
    /// Total tokens saved across all compactions
    total_tokens_saved: AtomicU64,
    /// Total time spent compacting in milliseconds
    total_time_ms: AtomicU64,
}

impl CompactionMetrics {
    /// Creates a new compaction metrics instance with all counters at zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a compaction operation.
    ///
    /// # Arguments
    ///
    /// * `tokens_saved` - Number of tokens saved by this compaction
    /// * `duration` - How long the compaction took
    pub fn record_compaction(&self, tokens_saved: usize, duration: Duration) {
        self.compaction_count.fetch_add(1, Ordering::Relaxed);
        self.total_tokens_saved
            .fetch_add(tokens_saved as u64, Ordering::Relaxed);
        self.total_time_ms
            .fetch_add(duration.as_millis() as u64, Ordering::Relaxed);
    }

    /// Returns the number of compactions performed.
    #[must_use]
    pub fn compaction_count(&self) -> u64 {
        self.compaction_count.load(Ordering::Relaxed)
    }

    /// Returns the total tokens saved across all compactions.
    #[must_use]
    pub fn total_tokens_saved(&self) -> u64 {
        self.total_tokens_saved.load(Ordering::Relaxed)
    }

    /// Returns the total time spent compacting in milliseconds.
    #[must_use]
    pub fn total_time_ms(&self) -> u64 {
        self.total_time_ms.load(Ordering::Relaxed)
    }

    /// Returns the average tokens saved per compaction.
    ///
    /// Returns 0 if no compactions have been recorded.
    #[must_use]
    pub fn average_tokens_saved(&self) -> u64 {
        let count = self.compaction_count.load(Ordering::Relaxed);
        if count == 0 {
            return 0;
        }
        self.total_tokens_saved.load(Ordering::Relaxed) / count
    }

    /// Returns the average time per compaction in milliseconds.
    ///
    /// Returns 0 if no compactions have been recorded.
    #[must_use]
    pub fn average_time_ms(&self) -> u64 {
        let count = self.compaction_count.load(Ordering::Relaxed);
        if count == 0 {
            return 0;
        }
        self.total_time_ms.load(Ordering::Relaxed) / count
    }

    /// Returns a summary of all compaction metrics.
    #[must_use]
    pub fn summary(&self) -> CompactionMetricsSummary {
        let count = self.compaction_count.load(Ordering::Relaxed);
        CompactionMetricsSummary {
            compaction_count: count,
            total_tokens_saved: self.total_tokens_saved.load(Ordering::Relaxed),
            total_time_ms: self.total_time_ms.load(Ordering::Relaxed),
            average_tokens_saved: self.average_tokens_saved(),
            average_time_ms: self.average_time_ms(),
        }
    }

    /// Resets all counters to zero.
    pub fn reset(&self) {
        self.compaction_count.store(0, Ordering::Relaxed);
        self.total_tokens_saved.store(0, Ordering::Relaxed);
        self.total_time_ms.store(0, Ordering::Relaxed);
    }
}

/// Summary of compaction metrics.
#[derive(Debug, Clone)]
pub struct CompactionMetricsSummary {
    /// Number of compactions performed
    pub compaction_count: u64,
    /// Total tokens saved
    pub total_tokens_saved: u64,
    /// Total time spent in milliseconds
    pub total_time_ms: u64,
    /// Average tokens saved per compaction
    pub average_tokens_saved: u64,
    /// Average time per compaction in milliseconds
    pub average_time_ms: u64,
}

impl std::fmt::Display for CompactionMetricsSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "compactions={} tokens_saved={} time={}ms avg_saved={} avg_time={}ms",
            self.compaction_count,
            self.total_tokens_saved,
            self.total_time_ms,
            self.average_tokens_saved,
            self.average_time_ms
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_initial_values_zero() {
        let metrics = CompressionMetrics::new();

        assert_eq!(metrics.cache_hits(), 0);
        assert_eq!(metrics.cache_misses(), 0);
        assert_eq!(metrics.ccg_calls(), 0);
        assert_eq!(metrics.fallback_calls(), 0);
    }

    #[test]
    fn test_metrics_record_cache_hit() {
        let metrics = CompressionMetrics::new();

        metrics.record_cache_hit();
        metrics.record_cache_hit();

        assert_eq!(metrics.cache_hits(), 2);
        assert_eq!(metrics.cache_misses(), 0);
    }

    #[test]
    fn test_metrics_record_cache_miss() {
        let metrics = CompressionMetrics::new();

        metrics.record_cache_miss();
        metrics.record_cache_miss();
        metrics.record_cache_miss();

        assert_eq!(metrics.cache_misses(), 3);
        assert_eq!(metrics.cache_hits(), 0);
    }

    #[test]
    fn test_hit_rate_calculation() {
        let metrics = CompressionMetrics::new();

        metrics.record_cache_hit();
        metrics.record_cache_hit();
        metrics.record_cache_miss();

        let rate = metrics.hit_rate();
        // 2 hits / 3 total = 0.666...
        assert!((rate - 0.666).abs() < 0.01);
    }

    #[test]
    fn test_hit_rate_zero_on_empty() {
        let metrics = CompressionMetrics::new();

        assert_eq!(metrics.hit_rate(), 0.0);
    }

    #[test]
    fn test_hit_rate_100_percent() {
        let metrics = CompressionMetrics::new();

        metrics.record_cache_hit();
        metrics.record_cache_hit();
        metrics.record_cache_hit();

        assert_eq!(metrics.hit_rate(), 1.0);
    }

    #[test]
    fn test_metrics_record_ccg_call() {
        let metrics = CompressionMetrics::new();

        metrics.record_ccg_call();
        assert_eq!(metrics.ccg_calls(), 1);
    }

    #[test]
    fn test_metrics_record_fallback_call() {
        let metrics = CompressionMetrics::new();

        metrics.record_fallback_call();
        assert_eq!(metrics.fallback_calls(), 1);
    }

    #[test]
    fn test_metrics_record_latency() {
        let metrics = CompressionMetrics::new();

        metrics.record_latency(Duration::from_millis(10));
        metrics.record_latency(Duration::from_millis(20));

        let avg = metrics.average_latency().unwrap();
        // Average of 10ms and 20ms = 15ms
        assert!((avg.as_millis() as i64 - 15).abs() < 2);
    }

    #[test]
    fn test_metrics_no_latency_samples() {
        let metrics = CompressionMetrics::new();

        assert!(metrics.average_latency().is_none());
    }

    #[test]
    fn test_summary_includes_all_fields() {
        let metrics = CompressionMetrics::new();

        metrics.record_cache_hit();
        metrics.record_cache_hit();
        metrics.record_cache_miss();
        metrics.record_ccg_call();
        metrics.record_fallback_call();
        metrics.record_latency(Duration::from_millis(5));

        let summary = metrics.summary();

        assert_eq!(summary.cache_hits, 2);
        assert_eq!(summary.cache_misses, 1);
        assert_eq!(summary.ccg_calls, 1);
        assert_eq!(summary.fallback_calls, 1);
        assert!((summary.hit_rate - 0.666).abs() < 0.01);
        assert!(summary.average_latency.is_some());
    }

    #[test]
    fn test_summary_display() {
        let metrics = CompressionMetrics::new();

        metrics.record_cache_hit();
        metrics.record_cache_miss();

        let summary = metrics.summary();
        let display = format!("{}", summary);

        assert!(display.contains("hits=1"));
        assert!(display.contains("misses=1"));
        assert!(display.contains("50.0%"));
    }

    #[test]
    fn test_metrics_reset() {
        let metrics = CompressionMetrics::new();

        metrics.record_cache_hit();
        metrics.record_cache_miss();
        metrics.record_ccg_call();

        metrics.reset();

        assert_eq!(metrics.cache_hits(), 0);
        assert_eq!(metrics.cache_misses(), 0);
        assert_eq!(metrics.ccg_calls(), 0);
    }

    #[test]
    fn test_metrics_thread_safety() {
        use std::sync::Arc;
        use std::thread;

        let metrics = Arc::new(CompressionMetrics::new());
        let mut handles = vec![];

        // Spawn multiple threads incrementing counters
        for _ in 0..10 {
            let metrics_clone = Arc::clone(&metrics);
            handles.push(thread::spawn(move || {
                for _ in 0..100 {
                    metrics_clone.record_cache_hit();
                    metrics_clone.record_cache_miss();
                }
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        // 10 threads * 100 iterations = 1000 each
        assert_eq!(metrics.cache_hits(), 1000);
        assert_eq!(metrics.cache_misses(), 1000);
    }

    // 4.4.5 Tests: CompactionMetrics
    #[test]
    fn test_compaction_metrics_initial_values_zero() {
        let metrics = CompactionMetrics::new();
        assert_eq!(metrics.compaction_count(), 0);
        assert_eq!(metrics.total_tokens_saved(), 0);
        assert_eq!(metrics.total_time_ms(), 0);
    }

    #[test]
    fn test_compaction_metrics_record_compaction() {
        let metrics = CompactionMetrics::new();
        metrics.record_compaction(1000, Duration::from_millis(50));
        metrics.record_compaction(500, Duration::from_millis(30));

        assert_eq!(metrics.compaction_count(), 2);
        assert_eq!(metrics.total_tokens_saved(), 1500);
        assert_eq!(metrics.total_time_ms(), 80);
    }

    #[test]
    fn test_compaction_metrics_average_tokens_saved() {
        let metrics = CompactionMetrics::new();

        // No compactions - should return 0
        assert_eq!(metrics.average_tokens_saved(), 0);

        // Record some compactions
        metrics.record_compaction(1000, Duration::from_millis(10));
        metrics.record_compaction(2000, Duration::from_millis(20));

        // Average = (1000 + 2000) / 2 = 1500
        assert_eq!(metrics.average_tokens_saved(), 1500);
    }

    #[test]
    fn test_compaction_metrics_average_time_ms() {
        let metrics = CompactionMetrics::new();

        // No compactions - should return 0
        assert_eq!(metrics.average_time_ms(), 0);

        // Record some compactions
        metrics.record_compaction(100, Duration::from_millis(10));
        metrics.record_compaction(100, Duration::from_millis(30));

        // Average = (10 + 30) / 2 = 20
        assert_eq!(metrics.average_time_ms(), 20);
    }

    #[test]
    fn test_compaction_metrics_summary() {
        let metrics = CompactionMetrics::new();
        metrics.record_compaction(1000, Duration::from_millis(50));
        metrics.record_compaction(500, Duration::from_millis(25));

        let summary = metrics.summary();
        assert_eq!(summary.compaction_count, 2);
        assert_eq!(summary.total_tokens_saved, 1500);
        assert_eq!(summary.total_time_ms, 75);
        assert_eq!(summary.average_tokens_saved, 750);
        assert_eq!(summary.average_time_ms, 37);
    }

    #[test]
    fn test_compaction_metrics_summary_display() {
        let metrics = CompactionMetrics::new();
        metrics.record_compaction(10_000, Duration::from_millis(100));

        let summary = metrics.summary();
        let display = format!("{}", summary);

        assert!(display.contains("compactions=1"));
        assert!(display.contains("tokens_saved=10000"));
        assert!(display.contains("time=100ms"));
    }

    #[test]
    fn test_compaction_metrics_reset() {
        let metrics = CompactionMetrics::new();
        metrics.record_compaction(500, Duration::from_millis(25));

        metrics.reset();

        assert_eq!(metrics.compaction_count(), 0);
        assert_eq!(metrics.total_tokens_saved(), 0);
        assert_eq!(metrics.total_time_ms(), 0);
    }
}
