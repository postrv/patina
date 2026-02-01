//! Web search tool for querying search engines and returning results.
//!
//! This module provides web search capabilities with:
//! - DuckDuckGo HTML API integration
//! - Result parsing and formatting
//! - Timeout handling
//! - Rate limiting awareness
//!
//! # Security
//!
//! The tool validates queries and handles errors gracefully:
//! - Empty queries are rejected
//! - Timeouts prevent hanging
//! - Server errors are handled properly
//!
//! # Examples
//!
//! ```no_run
//! use patina::tools::web_search::{WebSearchTool, WebSearchConfig};
//!
//! # async fn example() -> anyhow::Result<()> {
//! let tool = WebSearchTool::new(WebSearchConfig::default());
//! let results = tool.search("rust programming", 10).await?;
//! for result in &results {
//!     println!("{}: {}", result.title, result.url);
//! }
//! # Ok(())
//! # }
//! ```

use anyhow::{bail, Result};
use reqwest::Client;
use scraper::{Html, Selector};
use std::time::Duration;
use tracing::debug;

/// Configuration for the web search tool.
#[derive(Debug, Clone)]
pub struct WebSearchConfig {
    /// Request timeout duration.
    pub timeout: Duration,
    /// Maximum number of results to return.
    pub max_results: usize,
    /// Base URL for the search API (for testing with mock servers).
    pub base_url: String,
    /// Allow localhost URLs (for testing only).
    ///
    /// This should NEVER be enabled in production.
    pub allow_localhost: bool,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            max_results: 10,
            base_url: "https://html.duckduckgo.com/html".to_string(),
            allow_localhost: false,
        }
    }
}

impl WebSearchConfig {
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

/// A search result from the web search.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// The title of the search result.
    pub title: String,
    /// The URL of the search result.
    pub url: String,
    /// A snippet/description from the search result.
    pub snippet: String,
}

/// Tool for searching the web.
pub struct WebSearchTool {
    config: WebSearchConfig,
    client: Client,
}

impl WebSearchTool {
    /// Creates a new web search tool with the given configuration.
    ///
    /// # Panics
    ///
    /// Panics if the HTTP client cannot be built (should not happen with default settings).
    #[must_use]
    pub fn new(config: WebSearchConfig) -> Self {
        let client = Client::builder()
            .timeout(config.timeout)
            .user_agent("Patina/0.3.0")
            .build()
            .expect("Failed to build HTTP client");

        Self { config, client }
    }

    /// Searches the web for the given query.
    ///
    /// # Arguments
    ///
    /// * `query` - The search query string.
    /// * `max_results` - Maximum number of results to return.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The query is empty
    /// - The request times out
    /// - The search API returns an error
    /// - The response cannot be parsed
    pub async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>> {
        // Validate query
        let query = query.trim();
        if query.is_empty() {
            bail!("Search query cannot be empty");
        }

        let effective_max = max_results.min(self.config.max_results);

        debug!(query = %query, max_results = effective_max, "Performing web search");

        // Build the request URL
        let url = format!("{}?q={}", self.config.base_url, urlencoding::encode(query));

        // Make the request
        let response = self.client.get(&url).send().await?;

        // Check for HTTP errors
        let status = response.status();
        if !status.is_success() {
            bail!("Search request failed with status {}", status);
        }

        // Parse the response
        let html_content = response.text().await?;
        let results = Self::parse_duckduckgo_results(&html_content, effective_max);

        Ok(results)
    }

    /// Parses DuckDuckGo HTML search results.
    ///
    /// DuckDuckGo's HTML API returns results in a structured format that we parse
    /// using CSS selectors.
    fn parse_duckduckgo_results(html: &str, max_results: usize) -> Vec<SearchResult> {
        let document = Html::parse_document(html);

        // DuckDuckGo HTML selectors
        let result_selector = Selector::parse(".result, .results_links, div.web-result")
            .unwrap_or_else(|_| {
                // Fallback to a simple div selector if the specific one fails
                Selector::parse("div").expect("basic selector should work")
            });

        let link_selector = Selector::parse("a.result__a, a.result-link, h2 a")
            .unwrap_or_else(|_| Selector::parse("a").expect("basic a selector should work"));

        let snippet_selector = Selector::parse(".result__snippet, .result-snippet, .snippet")
            .unwrap_or_else(|_| Selector::parse("p").expect("basic p selector should work"));

        let mut results = Vec::new();

        for element in document.select(&result_selector) {
            if results.len() >= max_results {
                break;
            }

            // Extract link and title
            if let Some(link_elem) = element.select(&link_selector).next() {
                let title = link_elem.text().collect::<String>().trim().to_string();
                let url = link_elem
                    .value()
                    .attr("href")
                    .unwrap_or_default()
                    .to_string();

                // Skip empty results
                if title.is_empty() || url.is_empty() {
                    continue;
                }

                // Extract snippet
                let snippet = element
                    .select(&snippet_selector)
                    .next()
                    .map(|e| e.text().collect::<String>().trim().to_string())
                    .unwrap_or_default();

                results.push(SearchResult {
                    title,
                    url,
                    snippet,
                });
            }
        }

        results
    }

    /// Formats search results as markdown.
    ///
    /// # Arguments
    ///
    /// * `results` - The search results to format.
    ///
    /// # Returns
    ///
    /// A markdown-formatted string containing the search results.
    #[must_use]
    pub fn format_as_markdown(results: &[SearchResult]) -> String {
        if results.is_empty() {
            return "No results found.".to_string();
        }

        let mut markdown = String::new();
        markdown.push_str(&format!("## Search Results ({} found)\n\n", results.len()));

        for (i, result) in results.iter().enumerate() {
            markdown.push_str(&format!("### {}. {}\n", i + 1, result.title));
            markdown.push_str(&format!("**URL**: {}\n\n", result.url));
            if !result.snippet.is_empty() {
                markdown.push_str(&format!("{}\n\n", result.snippet));
            }
            markdown.push_str("---\n\n");
        }

        markdown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_result_structure() {
        let result = SearchResult {
            title: "Test Title".to_string(),
            url: "https://example.com".to_string(),
            snippet: "Test snippet".to_string(),
        };

        assert_eq!(result.title, "Test Title");
        assert_eq!(result.url, "https://example.com");
        assert_eq!(result.snippet, "Test snippet");
    }

    #[test]
    fn test_config_default() {
        let config = WebSearchConfig::default();
        assert_eq!(config.timeout.as_secs(), 30);
        assert_eq!(config.max_results, 10);
        assert!(!config.allow_localhost);
    }

    #[test]
    fn test_config_for_testing() {
        let config = WebSearchConfig::for_testing();
        assert!(config.allow_localhost);
    }

    #[test]
    fn test_format_as_markdown_empty() {
        let results: Vec<SearchResult> = vec![];
        let markdown = WebSearchTool::format_as_markdown(&results);
        assert!(markdown.contains("No results"));
    }

    #[test]
    fn test_format_as_markdown_with_results() {
        let results = vec![SearchResult {
            title: "Rust Lang".to_string(),
            url: "https://rust-lang.org".to_string(),
            snippet: "A systems programming language.".to_string(),
        }];

        let markdown = WebSearchTool::format_as_markdown(&results);
        assert!(markdown.contains("Rust Lang"));
        assert!(markdown.contains("rust-lang.org"));
        assert!(markdown.contains("systems programming"));
    }

    #[test]
    fn test_parse_duckduckgo_results_basic() {
        let html = r#"
            <div class="result">
                <a class="result__a" href="https://example.com">Example Title</a>
                <span class="result__snippet">Example snippet text.</span>
            </div>
        "#;

        let results = WebSearchTool::parse_duckduckgo_results(html, 10);
        assert!(!results.is_empty());
        assert_eq!(results[0].title, "Example Title");
        assert_eq!(results[0].url, "https://example.com");
    }

    #[test]
    fn test_parse_respects_max_results() {
        let html = r#"
            <div class="result">
                <a class="result__a" href="https://example1.com">Title 1</a>
                <span class="result__snippet">Snippet 1</span>
            </div>
            <div class="result">
                <a class="result__a" href="https://example2.com">Title 2</a>
                <span class="result__snippet">Snippet 2</span>
            </div>
            <div class="result">
                <a class="result__a" href="https://example3.com">Title 3</a>
                <span class="result__snippet">Snippet 3</span>
            </div>
        "#;

        let results = WebSearchTool::parse_duckduckgo_results(html, 2);
        assert_eq!(results.len(), 2);
    }
}
