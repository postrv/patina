//! Web fetch tool for retrieving and converting web content.
//!
//! This module provides URL fetching capabilities with:
//! - HTML to markdown conversion using html2text
//! - URL validation (reject file://, localhost, private IPs)
//! - Content length limits
//! - Timeout handling
//! - Redirect limiting
//!
//! # Security
//!
//! The tool validates URLs to prevent:
//! - Local file access via file:// URLs
//! - SSRF attacks via localhost/private IP URLs
//! - Memory exhaustion via content length limits
//!
//! # Examples
//!
//! ```no_run
//! use patina::tools::web_fetch::{WebFetchTool, WebFetchConfig};
//!
//! # async fn example() -> anyhow::Result<()> {
//! let tool = WebFetchTool::new(WebFetchConfig::default());
//! let result = tool.fetch("https://example.com").await?;
//! println!("Content: {}", result.content);
//! # Ok(())
//! # }
//! ```

use anyhow::Result;
use std::time::Duration;

/// Configuration for the web fetch tool.
#[derive(Debug, Clone)]
pub struct WebFetchConfig {
    /// Request timeout duration.
    pub timeout: Duration,
    /// Maximum content length to fetch (in bytes).
    pub max_content_length: usize,
    /// Maximum number of redirects to follow.
    pub max_redirects: u8,
}

impl Default for WebFetchConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            max_content_length: 1_000_000, // 1MB
            max_redirects: 5,
        }
    }
}

/// Result of a web fetch operation.
#[derive(Debug, Clone)]
pub struct WebFetchResult {
    /// The fetched content (converted to markdown if HTML).
    pub content: String,
    /// The content type of the response.
    pub content_type: String,
    /// HTTP status code.
    pub status: u16,
}

/// Tool for fetching web content.
pub struct WebFetchTool {
    /// Configuration for the fetch tool.
    ///
    /// Prefixed with underscore during RED phase; will be used in GREEN phase.
    _config: WebFetchConfig,
}

impl WebFetchTool {
    /// Creates a new web fetch tool with the given configuration.
    #[must_use]
    pub fn new(config: WebFetchConfig) -> Self {
        Self { _config: config }
    }

    /// Fetches content from the given URL.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The URL is invalid
    /// - The URL uses a disallowed scheme (file://)
    /// - The URL points to localhost or private IP ranges
    /// - The request times out
    /// - The content exceeds the maximum length
    /// - Too many redirects are encountered
    pub async fn fetch(&self, _url: &str) -> Result<WebFetchResult> {
        // TODO: Implement in task 0.1.3
        Err(anyhow::anyhow!("WebFetchTool not yet implemented"))
    }
}
