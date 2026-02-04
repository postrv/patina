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
}
