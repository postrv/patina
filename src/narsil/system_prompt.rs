//! Narsil code intelligence context for the system prompt.
//!
//! Provides functions to build, cache, and format narsil intelligence data
//! for injection into the LLM system prompt. This gives the model awareness
//! of the codebase structure, complexity hotspots, and security posture.
//!
//! # Cache Strategy
//!
//! Narsil queries can be expensive. This module uses a file-based cache at
//! `<cache_dir>/patina/narsil-context.cache` with a configurable TTL (default
//! 5 minutes). When the cache is fresh, the cached content is returned
//! directly. When stale or missing, `None` is returned — the caller is
//! responsible for populating the cache via [`update_narsil_cache`].
//!
//! # Example
//!
//! ```ignore
//! use patina::narsil::system_prompt::{build_narsil_context_cached, update_narsil_cache};
//! use std::path::Path;
//!
//! // Check for cached context
//! let cache_dir = Path::new("/tmp/cache");
//! if let Some(ctx) = build_narsil_context_cached(Path::new("."), cache_dir) {
//!     println!("Narsil context: {}", ctx);
//! }
//!
//! // Update the cache after running narsil queries
//! update_narsil_cache(cache_dir, "# Code Intelligence (narsil)\n...").unwrap();
//! ```

use std::path::Path;
use std::time::Duration;

/// Default cache time-to-live: 5 minutes.
const DEFAULT_TTL: Duration = Duration::from_secs(300);

/// Relative path within the cache directory for the narsil context file.
const CACHE_RELATIVE_PATH: &str = "patina/narsil-context.cache";

/// Builds a compact narsil intelligence section for the system prompt.
///
/// Returns `None` if narsil is unavailable or if data cannot be fetched.
/// This function checks for a cache file and returns its content only if
/// the cache is fresh (less than [`DEFAULT_TTL`] old).
///
/// Target output size: 200-300 tokens maximum.
///
/// # Arguments
///
/// * `working_dir` - Project root directory (currently unused, reserved for
///   future direct narsil queries)
/// * `cache_dir` - Directory where cache files are stored
#[must_use]
pub fn build_narsil_context_cached(_working_dir: &Path, cache_dir: &Path) -> Option<String> {
    if !is_cache_fresh(cache_dir, DEFAULT_TTL) {
        return None;
    }

    let cache_path = cache_dir.join(CACHE_RELATIVE_PATH);
    std::fs::read_to_string(cache_path)
        .ok()
        .filter(|s| !s.is_empty())
}

/// Writes narsil context to the cache file.
///
/// Creates intermediate directories if they do not exist. The content is
/// written atomically (via a full overwrite) to the cache path at
/// `<cache_dir>/patina/narsil-context.cache`.
///
/// # Arguments
///
/// * `cache_dir` - Root cache directory
/// * `content` - The formatted narsil context string to cache
///
/// # Errors
///
/// Returns an error if the cache directory cannot be created or the file
/// cannot be written.
pub fn update_narsil_cache(cache_dir: &Path, content: &str) -> anyhow::Result<()> {
    let cache_path = cache_dir.join(CACHE_RELATIVE_PATH);
    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&cache_path, content)?;
    Ok(())
}

/// Checks if the narsil cache file exists and is within the given TTL.
///
/// Returns `true` if the cache file at `<cache_dir>/patina/narsil-context.cache`
/// exists and was modified less than `ttl` ago. Returns `false` if the file
/// is missing, stale, or if metadata cannot be read.
///
/// # Arguments
///
/// * `cache_dir` - Root cache directory
/// * `ttl` - Maximum age for the cache to be considered fresh
#[must_use]
pub fn is_cache_fresh(cache_dir: &Path, ttl: Duration) -> bool {
    let cache_path = cache_dir.join(CACHE_RELATIVE_PATH);
    let metadata = match std::fs::metadata(&cache_path) {
        Ok(m) => m,
        Err(_) => return false,
    };
    let modified = match metadata.modified() {
        Ok(t) => t,
        Err(_) => return false,
    };
    let elapsed = match modified.elapsed() {
        Ok(d) => d,
        Err(_) => return false,
    };
    elapsed < ttl
}

/// Formats raw narsil data into a compact system prompt section.
///
/// Assembles available fields into a structured markdown section suitable
/// for injection into the LLM system prompt. Fields that are `None` are
/// given sensible defaults (e.g., "No active findings" for security).
///
/// # Arguments
///
/// * `metrics` - Summary line like "42 modules, 1200 tests, 102K LOC"
/// * `hotspots` - Top complexity hotspots, one per line
/// * `security` - Security finding summary or status
/// * `circular` - Circular import count or description
#[must_use]
pub fn format_narsil_context(
    metrics: Option<&str>,
    hotspots: Option<&str>,
    security: Option<&str>,
    circular: Option<&str>,
) -> String {
    let mut out = String::from("# Code Intelligence (narsil)\n");
    out.push_str(&format!("Architecture: {}\n", metrics.unwrap_or("Unknown")));
    out.push_str(&format!(
        "Hotspots: {}\n",
        hotspots.unwrap_or("None identified")
    ));
    out.push_str(&format!(
        "Security: {}\n",
        security.unwrap_or("No active findings")
    ));
    out.push_str(&format!("Circular imports: {}", circular.unwrap_or("None")));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_format_narsil_context_all_fields_present() {
        let result = format_narsil_context(
            Some("42 modules, 3313 tests, 102K LOC"),
            Some("handle_message (35), process_tool (28), render_ui (22)"),
            Some("No active findings"),
            Some("None"),
        );

        assert!(result.contains("# Code Intelligence (narsil)"));
        assert!(result.contains("42 modules, 3313 tests, 102K LOC"));
        assert!(result.contains("handle_message (35)"));
        assert!(result.contains("No active findings"));
        assert!(result.contains("Circular imports: None"));
    }

    #[test]
    fn test_format_narsil_context_some_fields_none() {
        let result = format_narsil_context(
            Some("10 modules, 50 tests, 5K LOC"),
            None,
            None,
            Some("2 cycles detected"),
        );

        assert!(result.contains("# Code Intelligence (narsil)"));
        assert!(result.contains("10 modules, 50 tests, 5K LOC"));
        assert!(result.contains("Hotspots: None identified"));
        assert!(result.contains("Security: No active findings"));
        assert!(result.contains("Circular imports: 2 cycles detected"));
    }

    #[test]
    fn test_format_narsil_context_all_fields_none() {
        let result = format_narsil_context(None, None, None, None);

        assert!(result.contains("Architecture: Unknown"));
        assert!(result.contains("Hotspots: None identified"));
        assert!(result.contains("Security: No active findings"));
        assert!(result.contains("Circular imports: None"));
    }

    #[test]
    fn test_cache_write_and_read_roundtrip() {
        let cache_dir = TempDir::new().unwrap();
        let content = "# Code Intelligence (narsil)\nArchitecture: 5 modules, 100 tests, 10K LOC";

        update_narsil_cache(cache_dir.path(), content).unwrap();

        let read_back =
            std::fs::read_to_string(cache_dir.path().join(CACHE_RELATIVE_PATH)).unwrap();
        assert_eq!(read_back, content);
    }

    #[test]
    fn test_is_cache_fresh_returns_true_for_fresh_cache() {
        let cache_dir = TempDir::new().unwrap();
        update_narsil_cache(cache_dir.path(), "fresh content").unwrap();

        assert!(is_cache_fresh(cache_dir.path(), Duration::from_secs(60)));
    }

    #[test]
    fn test_is_cache_fresh_returns_false_for_stale_cache() {
        let cache_dir = TempDir::new().unwrap();
        update_narsil_cache(cache_dir.path(), "stale content").unwrap();

        // A TTL of 0 seconds means any existing file is stale
        assert!(!is_cache_fresh(cache_dir.path(), Duration::ZERO));
    }

    #[test]
    fn test_is_cache_fresh_returns_false_when_no_cache() {
        let cache_dir = TempDir::new().unwrap();

        assert!(!is_cache_fresh(cache_dir.path(), Duration::from_secs(300)));
    }

    #[test]
    fn test_build_narsil_context_cached_returns_none_when_no_cache() {
        let working_dir = TempDir::new().unwrap();
        let cache_dir = TempDir::new().unwrap();

        let result = build_narsil_context_cached(working_dir.path(), cache_dir.path());
        assert!(result.is_none());
    }

    #[test]
    fn test_build_narsil_context_cached_returns_content_when_fresh() {
        let working_dir = TempDir::new().unwrap();
        let cache_dir = TempDir::new().unwrap();
        let content = "# Code Intelligence (narsil)\nArchitecture: 3 modules";

        update_narsil_cache(cache_dir.path(), content).unwrap();

        let result = build_narsil_context_cached(working_dir.path(), cache_dir.path());
        assert_eq!(result, Some(content.to_string()));
    }

    #[test]
    fn test_build_narsil_context_cached_returns_none_for_empty_cache() {
        let working_dir = TempDir::new().unwrap();
        let cache_dir = TempDir::new().unwrap();

        update_narsil_cache(cache_dir.path(), "").unwrap();

        let result = build_narsil_context_cached(working_dir.path(), cache_dir.path());
        assert!(result.is_none());
    }

    #[test]
    fn test_update_narsil_cache_creates_directories() {
        let cache_dir = TempDir::new().unwrap();
        let deep_cache = cache_dir.path().join("nested");

        update_narsil_cache(&deep_cache, "test content").unwrap();

        let path = deep_cache.join(CACHE_RELATIVE_PATH);
        assert!(path.exists());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "test content");
    }

    #[test]
    fn test_update_narsil_cache_overwrites_existing() {
        let cache_dir = TempDir::new().unwrap();

        update_narsil_cache(cache_dir.path(), "first").unwrap();
        update_narsil_cache(cache_dir.path(), "second").unwrap();

        let content = std::fs::read_to_string(cache_dir.path().join(CACHE_RELATIVE_PATH)).unwrap();
        assert_eq!(content, "second");
    }
}
