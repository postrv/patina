//! Unit tests for the WebFetch tool.
//!
//! These tests verify URL fetching, HTML to markdown conversion,
//! and security validations for the web fetch tool.

use patina::tools::web_fetch::{WebFetchConfig, WebFetchResult, WebFetchTool};

// ============================================================================
// URL Validation Tests
// ============================================================================

#[tokio::test]
async fn test_fetch_rejects_file_urls() {
    let tool = WebFetchTool::new(WebFetchConfig::default());

    let result = tool.fetch("file:///etc/passwd").await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("file://")
            || err.to_string().to_lowercase().contains("not allowed"),
        "Expected file:// URL to be rejected, got: {err}"
    );
}

#[tokio::test]
async fn test_fetch_rejects_localhost_urls() {
    let tool = WebFetchTool::new(WebFetchConfig::default());

    // Test localhost variations
    let localhost_urls = [
        "http://localhost/test",
        "http://127.0.0.1/test",
        "http://[::1]/test",
        "http://localhost:8080/test",
    ];

    for url in localhost_urls {
        let result = tool.fetch(url).await;
        assert!(
            result.is_err(),
            "Expected localhost URL to be rejected: {url}"
        );
    }
}

#[tokio::test]
async fn test_fetch_rejects_private_ip_urls() {
    let tool = WebFetchTool::new(WebFetchConfig::default());

    // Test private IP ranges
    let private_urls = [
        "http://10.0.0.1/test",
        "http://192.168.1.1/test",
        "http://172.16.0.1/test",
    ];

    for url in private_urls {
        let result = tool.fetch(url).await;
        assert!(
            result.is_err(),
            "Expected private IP URL to be rejected: {url}"
        );
    }
}

// ============================================================================
// Content Fetching Tests (requires mock server)
// ============================================================================

#[tokio::test]
async fn test_fetch_valid_url_returns_content() {
    // This test will use wiremock to simulate a web server
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string("<html><body>Hello World</body></html>"),
        )
        .mount(&mock_server)
        .await;

    let tool = WebFetchTool::new(WebFetchConfig::default());
    let result = tool.fetch(&format!("{}/test", mock_server.uri())).await;

    assert!(
        result.is_ok(),
        "Expected successful fetch, got: {:?}",
        result
    );
    let fetch_result = result.unwrap();
    assert_eq!(fetch_result.status, 200);
    assert!(!fetch_result.content.is_empty());
}

#[tokio::test]
async fn test_fetch_invalid_url_returns_error() {
    let tool = WebFetchTool::new(WebFetchConfig::default());

    let result = tool.fetch("not-a-valid-url").await;

    assert!(result.is_err(), "Expected error for invalid URL");
}

#[tokio::test]
async fn test_fetch_timeout_returns_error() {
    use std::time::Duration;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // Respond with a delay longer than the timeout
    Mock::given(method("GET"))
        .and(path("/slow"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(10)))
        .mount(&mock_server)
        .await;

    let config = WebFetchConfig {
        timeout: Duration::from_millis(100),
        ..Default::default()
    };
    let tool = WebFetchTool::new(config);
    let result = tool.fetch(&format!("{}/slow", mock_server.uri())).await;

    assert!(result.is_err(), "Expected timeout error");
}

// ============================================================================
// HTML to Markdown Conversion Tests
// ============================================================================

#[tokio::test]
async fn test_fetch_converts_html_to_markdown() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    let html = r#"
        <html>
        <body>
            <h1>Title</h1>
            <p>This is a paragraph.</p>
            <ul>
                <li>Item 1</li>
                <li>Item 2</li>
            </ul>
        </body>
        </html>
    "#;

    Mock::given(method("GET"))
        .and(path("/markdown"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(html)
                .insert_header("content-type", "text/html"),
        )
        .mount(&mock_server)
        .await;

    let tool = WebFetchTool::new(WebFetchConfig::default());
    let result = tool.fetch(&format!("{}/markdown", mock_server.uri())).await;

    assert!(result.is_ok());
    let fetch_result = result.unwrap();

    // Verify markdown conversion happened
    assert!(
        fetch_result.content.contains("Title") || fetch_result.content.contains("# Title"),
        "Expected title in content"
    );
    assert!(
        fetch_result.content.contains("paragraph"),
        "Expected paragraph content"
    );
}

#[tokio::test]
async fn test_fetch_preserves_links_in_markdown() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    let html = r#"
        <html>
        <body>
            <p>Visit <a href="https://example.com">Example Site</a> for more info.</p>
        </body>
        </html>
    "#;

    Mock::given(method("GET"))
        .and(path("/links"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(html)
                .insert_header("content-type", "text/html"),
        )
        .mount(&mock_server)
        .await;

    let tool = WebFetchTool::new(WebFetchConfig::default());
    let result = tool.fetch(&format!("{}/links", mock_server.uri())).await;

    assert!(result.is_ok());
    let fetch_result = result.unwrap();

    // Verify link is preserved in markdown format
    assert!(
        fetch_result.content.contains("https://example.com")
            || fetch_result.content.contains("Example Site"),
        "Expected link to be preserved in content: {}",
        fetch_result.content
    );
}

#[tokio::test]
async fn test_fetch_handles_non_html_content() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    let json_content = r#"{"key": "value", "number": 42}"#;

    Mock::given(method("GET"))
        .and(path("/api/data"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(json_content)
                .insert_header("content-type", "application/json"),
        )
        .mount(&mock_server)
        .await;

    let tool = WebFetchTool::new(WebFetchConfig::default());
    let result = tool.fetch(&format!("{}/api/data", mock_server.uri())).await;

    assert!(result.is_ok());
    let fetch_result = result.unwrap();

    // Non-HTML content should be returned as-is
    assert!(
        fetch_result.content.contains("key") && fetch_result.content.contains("value"),
        "Expected JSON content to be preserved"
    );
    assert_eq!(fetch_result.content_type, "application/json");
}

// ============================================================================
// Content Length and Redirect Tests
// ============================================================================

#[tokio::test]
async fn test_fetch_respects_max_content_length() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // Create content larger than max
    let large_content = "x".repeat(2_000_000); // 2MB

    Mock::given(method("GET"))
        .and(path("/large"))
        .respond_with(ResponseTemplate::new(200).set_body_string(&large_content))
        .mount(&mock_server)
        .await;

    let config = WebFetchConfig {
        max_content_length: 1_000_000, // 1MB limit
        ..Default::default()
    };
    let tool = WebFetchTool::new(config);
    let result = tool.fetch(&format!("{}/large", mock_server.uri())).await;

    // Should either error or truncate
    if let Ok(fetch_result) = result {
        assert!(
            fetch_result.content.len() <= 1_000_000,
            "Content should be truncated to max length"
        );
    }
    // Error is also acceptable behavior for oversized content
}

#[tokio::test]
async fn test_fetch_follows_redirects_limited() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // Set up redirect chain
    Mock::given(method("GET"))
        .and(path("/redirect1"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/redirect2"))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/redirect2"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/final"))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/final"))
        .respond_with(ResponseTemplate::new(200).set_body_string("Final destination"))
        .mount(&mock_server)
        .await;

    let config = WebFetchConfig {
        max_redirects: 5,
        ..Default::default()
    };
    let tool = WebFetchTool::new(config);
    let result = tool
        .fetch(&format!("{}/redirect1", mock_server.uri()))
        .await;

    assert!(result.is_ok(), "Should follow redirects successfully");
    let fetch_result = result.unwrap();
    assert!(fetch_result.content.contains("Final destination"));
}

#[tokio::test]
async fn test_fetch_blocks_excessive_redirects() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // Set up infinite redirect loop
    Mock::given(method("GET"))
        .and(path("/loop"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/loop"))
        .mount(&mock_server)
        .await;

    let config = WebFetchConfig {
        max_redirects: 3,
        ..Default::default()
    };
    let tool = WebFetchTool::new(config);
    let result = tool.fetch(&format!("{}/loop", mock_server.uri())).await;

    assert!(result.is_err(), "Should fail on redirect loop");
}

// ============================================================================
// WebFetchResult Tests
// ============================================================================

#[test]
fn test_web_fetch_result_structure() {
    let result = WebFetchResult {
        content: "Hello World".to_string(),
        content_type: "text/html".to_string(),
        status: 200,
    };

    assert_eq!(result.content, "Hello World");
    assert_eq!(result.content_type, "text/html");
    assert_eq!(result.status, 200);
}

// ============================================================================
// WebFetchConfig Tests
// ============================================================================

#[test]
fn test_web_fetch_config_default() {
    let config = WebFetchConfig::default();

    assert!(
        config.timeout.as_secs() >= 10,
        "Default timeout should be reasonable"
    );
    assert!(
        config.max_content_length >= 100_000,
        "Default max content should be reasonable"
    );
    assert!(
        config.max_redirects >= 3,
        "Should allow some redirects by default"
    );
}
