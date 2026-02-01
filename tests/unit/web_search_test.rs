//! Unit tests for the WebSearch tool.
//!
//! These tests verify web search functionality, result parsing,
//! and error handling for the web search tool.

use patina::tools::web_search::{SearchResult, WebSearchConfig, WebSearchTool};

// ============================================================================
// Search Result Structure Tests
// ============================================================================

#[test]
fn test_search_result_has_title_url_snippet() {
    let result = SearchResult {
        title: "Example Page".to_string(),
        url: "https://example.com".to_string(),
        snippet: "This is an example snippet from the search result.".to_string(),
    };

    assert_eq!(result.title, "Example Page");
    assert_eq!(result.url, "https://example.com");
    assert!(!result.snippet.is_empty());
}

// ============================================================================
// Configuration Tests
// ============================================================================

#[test]
fn test_web_search_config_default() {
    let config = WebSearchConfig::default();

    assert!(
        config.timeout.as_secs() >= 10,
        "Default timeout should be reasonable"
    );
    assert!(
        config.max_results >= 5,
        "Default max results should be reasonable"
    );
}

#[test]
fn test_web_search_config_for_testing() {
    let config = WebSearchConfig::for_testing();

    assert!(
        config.allow_localhost,
        "Testing config should allow localhost"
    );
}

// ============================================================================
// Search Execution Tests (requires mock server)
// ============================================================================

#[tokio::test]
async fn test_search_returns_results() {
    use wiremock::matchers::{method, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // DuckDuckGo HTML API returns results in HTML format
    let html_response = r#"
        <html>
        <body>
            <div class="result">
                <a class="result__a" href="https://example.com/page1">Example Page 1</a>
                <a class="result__snippet">This is the first result snippet.</a>
            </div>
            <div class="result">
                <a class="result__a" href="https://example.com/page2">Example Page 2</a>
                <a class="result__snippet">This is the second result snippet.</a>
            </div>
        </body>
        </html>
    "#;

    Mock::given(method("GET"))
        .and(query_param("q", "test query"))
        .respond_with(ResponseTemplate::new(200).set_body_string(html_response))
        .mount(&mock_server)
        .await;

    let config = WebSearchConfig {
        base_url: mock_server.uri(),
        allow_localhost: true,
        ..Default::default()
    };
    let tool = WebSearchTool::new(config);
    let results = tool.search("test query", 10).await;

    assert!(results.is_ok(), "Search should succeed: {:?}", results);
    let results = results.unwrap();
    assert!(!results.is_empty(), "Should return some results");
}

#[tokio::test]
async fn test_search_empty_query_returns_error() {
    let tool = WebSearchTool::new(WebSearchConfig::default());

    let result = tool.search("", 10).await;

    assert!(result.is_err(), "Empty query should return error");
    let err = result.unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("empty")
            || err.to_string().to_lowercase().contains("query"),
        "Error should mention empty query: {}",
        err
    );
}

#[tokio::test]
async fn test_search_respects_max_results() {
    use wiremock::matchers::{method, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // Return more results than requested
    let html_response = r#"
        <html>
        <body>
            <div class="result">
                <a class="result__a" href="https://example.com/1">Result 1</a>
                <a class="result__snippet">Snippet 1</a>
            </div>
            <div class="result">
                <a class="result__a" href="https://example.com/2">Result 2</a>
                <a class="result__snippet">Snippet 2</a>
            </div>
            <div class="result">
                <a class="result__a" href="https://example.com/3">Result 3</a>
                <a class="result__snippet">Snippet 3</a>
            </div>
            <div class="result">
                <a class="result__a" href="https://example.com/4">Result 4</a>
                <a class="result__snippet">Snippet 4</a>
            </div>
            <div class="result">
                <a class="result__a" href="https://example.com/5">Result 5</a>
                <a class="result__snippet">Snippet 5</a>
            </div>
        </body>
        </html>
    "#;

    Mock::given(method("GET"))
        .and(query_param("q", "many results"))
        .respond_with(ResponseTemplate::new(200).set_body_string(html_response))
        .mount(&mock_server)
        .await;

    let config = WebSearchConfig {
        base_url: mock_server.uri(),
        allow_localhost: true,
        ..Default::default()
    };
    let tool = WebSearchTool::new(config);
    let results = tool.search("many results", 3).await;

    assert!(results.is_ok());
    let results = results.unwrap();
    assert!(
        results.len() <= 3,
        "Should respect max_results limit, got {}",
        results.len()
    );
}

#[tokio::test]
async fn test_search_timeout_returns_error() {
    use std::time::Duration;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // Respond with a delay longer than the timeout
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(10)))
        .mount(&mock_server)
        .await;

    let config = WebSearchConfig {
        base_url: mock_server.uri(),
        timeout: Duration::from_millis(100),
        allow_localhost: true,
        ..Default::default()
    };
    let tool = WebSearchTool::new(config);
    let result = tool.search("slow query", 10).await;

    assert!(result.is_err(), "Should timeout");
}

#[tokio::test]
async fn test_search_api_error_returns_error() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // Return a server error
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
        .mount(&mock_server)
        .await;

    let config = WebSearchConfig {
        base_url: mock_server.uri(),
        allow_localhost: true,
        ..Default::default()
    };
    let tool = WebSearchTool::new(config);
    let result = tool.search("error query", 10).await;

    assert!(result.is_err(), "Should return error for 5xx response");
}

#[tokio::test]
async fn test_search_formats_results_as_markdown() {
    use wiremock::matchers::{method, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    let html_response = r#"
        <html>
        <body>
            <div class="result">
                <a class="result__a" href="https://rust-lang.org">Rust Programming Language</a>
                <a class="result__snippet">A language empowering everyone to build reliable software.</a>
            </div>
        </body>
        </html>
    "#;

    Mock::given(method("GET"))
        .and(query_param("q", "rust programming"))
        .respond_with(ResponseTemplate::new(200).set_body_string(html_response))
        .mount(&mock_server)
        .await;

    let config = WebSearchConfig {
        base_url: mock_server.uri(),
        allow_localhost: true,
        ..Default::default()
    };
    let tool = WebSearchTool::new(config);
    let results = tool.search("rust programming", 10).await;

    assert!(results.is_ok());
    let results = results.unwrap();

    // Convert to markdown format
    let markdown = WebSearchTool::format_as_markdown(&results);

    // Verify markdown contains expected elements
    assert!(
        markdown.contains("Rust Programming Language") || markdown.contains("rust-lang.org"),
        "Markdown should contain result title or URL: {}",
        markdown
    );
}

// ============================================================================
// Markdown Formatting Tests
// ============================================================================

#[test]
fn test_format_as_markdown_empty_results() {
    let results: Vec<SearchResult> = vec![];
    let markdown = WebSearchTool::format_as_markdown(&results);

    assert!(
        markdown.contains("No results") || markdown.is_empty() || markdown.contains("0 results"),
        "Should indicate no results: {}",
        markdown
    );
}

#[test]
fn test_format_as_markdown_multiple_results() {
    let results = vec![
        SearchResult {
            title: "First Result".to_string(),
            url: "https://example.com/1".to_string(),
            snippet: "First snippet.".to_string(),
        },
        SearchResult {
            title: "Second Result".to_string(),
            url: "https://example.com/2".to_string(),
            snippet: "Second snippet.".to_string(),
        },
    ];

    let markdown = WebSearchTool::format_as_markdown(&results);

    assert!(markdown.contains("First Result"));
    assert!(markdown.contains("Second Result"));
    assert!(markdown.contains("https://example.com/1"));
    assert!(markdown.contains("https://example.com/2"));
}
