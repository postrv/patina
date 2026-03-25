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
//! let tool = WebFetchTool::new(WebFetchConfig::default())?;
//! let result = tool.fetch("https://example.com").await?;
//! println!("Content: {}", result.content);
//! # Ok(())
//! # }
//! ```

use anyhow::{bail, Context, Result};
use reqwest::redirect::Policy;
use reqwest::Client;
use std::time::Duration;
use tracing::debug;

use crate::util::url_validation;

/// Configuration for the web fetch tool.
#[derive(Debug, Clone)]
pub struct WebFetchConfig {
    /// Request timeout duration.
    pub timeout: Duration,
    /// Maximum content length to fetch (in bytes).
    pub max_content_length: usize,
    /// Maximum number of redirects to follow.
    pub max_redirects: u8,
    /// Allow localhost URLs (for testing only).
    ///
    /// This should NEVER be enabled in production as it enables SSRF attacks.
    pub allow_localhost: bool,
}

impl Default for WebFetchConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            max_content_length: 1_000_000, // 1MB
            max_redirects: 5,
            allow_localhost: false,
        }
    }
}

impl WebFetchConfig {
    /// Creates a test configuration that allows localhost URLs.
    ///
    /// # Security Warning
    ///
    /// This should ONLY be used in tests with mock servers.
    /// Never use this in production code.
    #[must_use]
    pub fn for_testing() -> Self {
        Self {
            allow_localhost: true,
            ..Default::default()
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
    config: WebFetchConfig,
    client: Client,
}

impl WebFetchTool {
    /// Creates a new web fetch tool with the given configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP client cannot be built.
    pub fn new(config: WebFetchConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(config.timeout)
            .redirect(Policy::limited(config.max_redirects as usize))
            .user_agent("Patina/0.3.0")
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self { config, client })
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
    pub async fn fetch(&self, url: &str) -> Result<WebFetchResult> {
        // Parse and validate the URL
        let parsed_url = url_validation::validate_url(url, self.config.allow_localhost)?;

        debug!(url = %parsed_url, "Fetching web content");

        // Make the request
        let response = self.client.get(parsed_url.as_str()).send().await?;

        // Post-connect DNS rebinding protection: validate the resolved IP address
        // even if the hostname passed pre-connect validation. A malicious DNS server
        // could return a public IP during validation and a private IP at connect time.
        if !self.config.allow_localhost {
            if let Some(remote_addr) = response.remote_addr() {
                let ip = remote_addr.ip();
                if ip.is_loopback() || url_validation::is_private_ip_addr(ip) {
                    bail!(
                        "DNS rebinding detected: {} resolved to private IP {}",
                        parsed_url.host_str().unwrap_or("unknown"),
                        ip
                    );
                }
            }
        }

        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("text/plain")
            .to_string();

        // Check content length from headers if available
        if let Some(content_length) = response.content_length() {
            if content_length as usize > self.config.max_content_length {
                bail!(
                    "Content too large: {} bytes exceeds {} byte limit",
                    content_length,
                    self.config.max_content_length
                );
            }
        }

        // Read the response body with size limit
        let bytes = response.bytes().await?;
        if bytes.len() > self.config.max_content_length {
            bail!(
                "Content too large: {} bytes exceeds {} byte limit",
                bytes.len(),
                self.config.max_content_length
            );
        }

        // Convert bytes to string
        let raw_content = String::from_utf8_lossy(&bytes).to_string();

        // Convert HTML to markdown if content type is HTML
        let content = if Self::is_html_content_type(&content_type) {
            Self::html_to_markdown(&raw_content)
        } else {
            raw_content
        };

        // Extract the base content type (before any ; charset=... etc)
        let base_content_type = content_type
            .split(';')
            .next()
            .unwrap_or(&content_type)
            .trim()
            .to_string();

        Ok(WebFetchResult {
            content,
            content_type: base_content_type,
            status,
        })
    }

    /// Checks if a content type indicates HTML content.
    fn is_html_content_type(content_type: &str) -> bool {
        let ct_lower = content_type.to_lowercase();
        ct_lower.contains("text/html") || ct_lower.contains("application/xhtml")
    }

    /// Converts HTML content to markdown.
    fn html_to_markdown(html: &str) -> String {
        // Use html2text to convert HTML to plain text with markdown-like formatting
        // The width of 80 characters is a reasonable default for readable output
        html2text::from_read(html.as_bytes(), 80)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_localhost() {
        assert!(url_validation::is_localhost("localhost"));
        assert!(url_validation::is_localhost("127.0.0.1"));
        assert!(url_validation::is_localhost("127.0.0.2"));
        assert!(url_validation::is_localhost("::1"));
        assert!(url_validation::is_localhost("[::1]"));
        assert!(!url_validation::is_localhost("example.com"));
        assert!(!url_validation::is_localhost("8.8.8.8"));
    }

    #[test]
    fn test_is_private_ip() {
        // Private ranges
        assert!(url_validation::is_private_ip("10.0.0.1"));
        assert!(url_validation::is_private_ip("10.255.255.255"));
        assert!(url_validation::is_private_ip("172.16.0.1"));
        assert!(url_validation::is_private_ip("172.31.255.255"));
        assert!(url_validation::is_private_ip("192.168.1.1"));
        assert!(url_validation::is_private_ip("192.168.0.1"));
        assert!(url_validation::is_private_ip("169.254.1.1"));

        // Not private
        assert!(!url_validation::is_private_ip("8.8.8.8"));
        assert!(!url_validation::is_private_ip("172.32.0.1")); // Just outside 172.16-31 range
        assert!(!url_validation::is_private_ip("example.com"));
    }

    #[test]
    fn test_is_html_content_type() {
        assert!(WebFetchTool::is_html_content_type("text/html"));
        assert!(WebFetchTool::is_html_content_type(
            "text/html; charset=utf-8"
        ));
        assert!(WebFetchTool::is_html_content_type("TEXT/HTML"));
        assert!(WebFetchTool::is_html_content_type("application/xhtml+xml"));
        assert!(!WebFetchTool::is_html_content_type("application/json"));
        assert!(!WebFetchTool::is_html_content_type("text/plain"));
    }

    #[test]
    fn test_html_to_markdown() {
        let html = "<html><body><h1>Title</h1><p>Paragraph</p></body></html>";
        let markdown = WebFetchTool::html_to_markdown(html);
        assert!(markdown.contains("Title"));
        assert!(markdown.contains("Paragraph"));
    }

    #[test]
    fn test_html_to_markdown_preserves_links() {
        let html =
            r#"<html><body><p>Visit <a href="https://example.com">Example</a></p></body></html>"#;
        let markdown = WebFetchTool::html_to_markdown(html);
        // html2text should preserve link text
        assert!(markdown.contains("Example"));
    }

    #[test]
    fn test_web_fetch_tool_new_returns_ok() {
        let result = WebFetchTool::new(WebFetchConfig::default());
        assert!(result.is_ok(), "Default config should produce Ok");
    }

    #[test]
    fn test_is_private_ip_addr_v4_ranges() {
        use std::net::{IpAddr, Ipv4Addr};

        // Private ranges
        assert!(url_validation::is_private_ip_addr(IpAddr::V4(
            Ipv4Addr::new(10, 0, 0, 1)
        )));
        assert!(url_validation::is_private_ip_addr(IpAddr::V4(
            Ipv4Addr::new(172, 16, 0, 1)
        )));
        assert!(url_validation::is_private_ip_addr(IpAddr::V4(
            Ipv4Addr::new(192, 168, 1, 1)
        )));
        assert!(url_validation::is_private_ip_addr(IpAddr::V4(
            Ipv4Addr::new(169, 254, 0, 1)
        )));

        // Public
        assert!(!url_validation::is_private_ip_addr(IpAddr::V4(
            Ipv4Addr::new(8, 8, 8, 8)
        )));
        assert!(!url_validation::is_private_ip_addr(IpAddr::V4(
            Ipv4Addr::new(1, 1, 1, 1)
        )));
    }

    #[test]
    fn test_is_private_ip_addr_v6_ranges() {
        use std::net::{IpAddr, Ipv6Addr};

        // Loopback
        assert!(url_validation::is_private_ip_addr(IpAddr::V6(
            Ipv6Addr::LOCALHOST
        )));
        // Link-local (fe80::/10)
        assert!(url_validation::is_private_ip_addr(IpAddr::V6(
            Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1)
        )));
        // Unique local (fc00::/7)
        assert!(url_validation::is_private_ip_addr(IpAddr::V6(
            Ipv6Addr::new(0xfc00, 0, 0, 0, 0, 0, 0, 1)
        )));

        // Public
        assert!(!url_validation::is_private_ip_addr(IpAddr::V6(
            Ipv6Addr::new(0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8888)
        )));
    }

    #[test]
    fn test_is_private_ip_addr_public() {
        use std::net::{IpAddr, Ipv4Addr};

        assert!(!url_validation::is_private_ip_addr(IpAddr::V4(
            Ipv4Addr::new(8, 8, 8, 8)
        )));
        assert!(!url_validation::is_private_ip_addr(IpAddr::V4(
            Ipv4Addr::new(93, 184, 216, 34)
        )));
    }
}
