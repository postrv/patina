//! Integration tests for the web_fetch tool.
//!
//! These tests verify:
//! - End-to-end tool execution through the ToolExecutor
//! - Tool schema validity for the Anthropic API
//! - Security validations work through the full execution path
//!
//! # Note on Mock Server Testing
//!
//! The default `ToolExecutor` uses `WebFetchConfig::default()` which blocks localhost
//! URLs for security (SSRF protection). The unit tests in `tests/unit/web_fetch_test.rs`
//! use `WebFetchConfig::for_testing()` to test with mock servers. These integration
//! tests focus on verifying the integration between components.

use patina::api::tools::{default_tools, web_fetch_tool};
use patina::tools::{ToolCall, ToolExecutor, ToolResult};
use serde_json::json;

// ============================================================================
// Tool Schema Validation Tests
// ============================================================================

/// Verifies the web_fetch tool schema is valid for the Anthropic API.
///
/// The API requires:
/// - `name`: Tool identifier
/// - `description`: Human-readable description
/// - `input_schema`: JSON Schema object with properties and required fields
#[test]
fn test_web_fetch_tool_schema_valid() {
    let tool = web_fetch_tool();

    // Verify basic structure
    assert_eq!(tool.name, "web_fetch");
    assert!(
        !tool.description.is_empty(),
        "Tool description should not be empty"
    );

    // Verify input schema is a valid JSON Schema
    let schema = &tool.input_schema;
    assert_eq!(
        schema["type"], "object",
        "Input schema must have type: object"
    );

    // Verify properties exist and url is defined
    assert!(
        schema["properties"].is_object(),
        "Input schema must have properties object"
    );
    assert!(
        schema["properties"]["url"].is_object(),
        "Input schema must define url property"
    );

    // Verify url property has correct schema
    assert_eq!(
        schema["properties"]["url"]["type"], "string",
        "url property must be a string"
    );
    assert!(
        schema["properties"]["url"]["description"]
            .as_str()
            .is_some(),
        "url property should have a description"
    );

    // Verify required field includes url
    assert!(
        schema["required"].is_array(),
        "Input schema must have required array"
    );
    let required = schema["required"]
        .as_array()
        .expect("required should be an array");
    assert!(
        required.iter().any(|v| v == "url"),
        "url should be a required field"
    );
}

/// Verifies web_fetch is included in the default tools list.
#[test]
fn test_web_fetch_in_default_tools() {
    let tools = default_tools();

    let web_fetch = tools.iter().find(|t| t.name == "web_fetch");
    assert!(
        web_fetch.is_some(),
        "web_fetch should be in default tools list"
    );

    let tool = web_fetch.unwrap();
    assert!(
        tool.description.contains("URL"),
        "Description should mention URL"
    );
    assert!(
        tool.description.contains("markdown"),
        "Description should mention markdown conversion"
    );
}

/// Verifies all default tools have consistent schema structure.
#[test]
fn test_all_tool_schemas_consistent() {
    let tools = default_tools();

    for tool in &tools {
        // Every tool must have a non-empty name and description
        assert!(!tool.name.is_empty(), "Tool {} has empty name", tool.name);
        assert!(
            !tool.description.is_empty(),
            "Tool {} has empty description",
            tool.name
        );

        // Every tool must have a valid input schema
        let schema = &tool.input_schema;
        assert_eq!(
            schema["type"], "object",
            "Tool {} schema must have type: object",
            tool.name
        );
        assert!(
            schema["properties"].is_object(),
            "Tool {} schema must have properties",
            tool.name
        );
        assert!(
            schema["required"].is_array(),
            "Tool {} schema must have required array",
            tool.name
        );
    }
}

// ============================================================================
// Tool Executor Integration Tests
// ============================================================================

/// Verifies the ToolExecutor recognizes and routes the web_fetch tool.
#[tokio::test]
async fn test_web_fetch_tool_recognized_by_executor() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp directory");
    let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

    // Create a tool call for web_fetch
    let call = ToolCall {
        name: "web_fetch".to_string(),
        input: json!({ "url": "https://example.com" }),
    };

    // Execute should not return "Unknown tool" error
    let result = executor.execute(call).await;
    assert!(result.is_ok(), "Executor should handle web_fetch tool");

    let tool_result = result.unwrap();
    // The result should be either Success or Error (from actual fetch), not "Unknown tool"
    match &tool_result {
        ToolResult::Error(msg) => {
            assert!(
                !msg.contains("Unknown tool"),
                "web_fetch should be recognized, got: {msg}"
            );
        }
        ToolResult::Success(_) => {
            // Success is also valid (though unlikely without network)
        }
        _ => {
            // Other results are fine
        }
    }
}

/// Verifies web_fetch properly validates URL parameter.
#[tokio::test]
async fn test_web_fetch_missing_url_returns_error() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp directory");
    let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

    // Missing url parameter
    let call = ToolCall {
        name: "web_fetch".to_string(),
        input: json!({}),
    };

    let result = executor.execute(call).await;
    // Should error due to missing url
    assert!(
        result.is_err(),
        "Should error when url parameter is missing"
    );
}

/// Verifies security validation through the executor for localhost URLs.
///
/// The default ToolExecutor blocks localhost URLs for SSRF protection.
#[tokio::test]
async fn test_web_fetch_executor_blocks_localhost() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp directory");
    let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

    // Try to fetch localhost (should be blocked)
    let call = ToolCall {
        name: "web_fetch".to_string(),
        input: json!({ "url": "http://localhost:8080/test" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("Executor should not panic");

    match result {
        ToolResult::Error(msg) => {
            assert!(
                msg.to_lowercase().contains("localhost")
                    || msg.to_lowercase().contains("not allowed")
                    || msg.to_lowercase().contains("security"),
                "Error should mention localhost is blocked: {msg}"
            );
        }
        ToolResult::Success(_) => {
            panic!("Localhost fetch should be blocked by security policy");
        }
        _ => {
            panic!("Expected error result for localhost URL");
        }
    }
}

/// Verifies security validation through the executor for file:// URLs.
#[tokio::test]
async fn test_web_fetch_executor_blocks_file_urls() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp directory");
    let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

    // Try to fetch file URL (should be blocked)
    let call = ToolCall {
        name: "web_fetch".to_string(),
        input: json!({ "url": "file:///etc/passwd" }),
    };

    let result = executor
        .execute(call)
        .await
        .expect("Executor should not panic");

    match result {
        ToolResult::Error(msg) => {
            assert!(
                msg.to_lowercase().contains("file://")
                    || msg.to_lowercase().contains("not allowed")
                    || msg.to_lowercase().contains("security"),
                "Error should mention file:// is blocked: {msg}"
            );
        }
        ToolResult::Success(_) => {
            panic!("file:// fetch should be blocked by security policy");
        }
        _ => {
            panic!("Expected error result for file:// URL");
        }
    }
}

/// Verifies security validation through the executor for private IP addresses.
#[tokio::test]
async fn test_web_fetch_executor_blocks_private_ips() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp directory");
    let executor = ToolExecutor::new(temp_dir.path().to_path_buf());

    // Test various private IP ranges
    let private_urls = [
        "http://10.0.0.1/test",
        "http://192.168.1.1/test",
        "http://172.16.0.1/test",
    ];

    for url in private_urls {
        let call = ToolCall {
            name: "web_fetch".to_string(),
            input: json!({ "url": url }),
        };

        let result = executor
            .execute(call)
            .await
            .expect("Executor should not panic");

        match result {
            ToolResult::Error(msg) => {
                assert!(
                    msg.to_lowercase().contains("private")
                        || msg.to_lowercase().contains("not allowed")
                        || msg.to_lowercase().contains("security"),
                    "Error for {} should mention private IPs are blocked: {}",
                    url,
                    msg
                );
            }
            ToolResult::Success(_) => {
                panic!(
                    "Private IP fetch should be blocked by security policy: {}",
                    url
                );
            }
            _ => {
                panic!("Expected error result for private IP URL: {}", url);
            }
        }
    }
}

// ============================================================================
// End-to-End Tests with Mock Server
// ============================================================================

/// End-to-end test verifying the complete flow through WebFetchTool.
///
/// Since ToolExecutor uses default config (localhost blocked), this test
/// directly uses WebFetchTool with testing config to verify the full fetch flow.
/// The unit tests also cover this, but this integration test verifies the
/// components work together properly.
#[tokio::test]
async fn test_web_fetch_end_to_end_with_mock_server() {
    use patina::tools::web_fetch::{WebFetchConfig, WebFetchTool};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // Start mock server
    let mock_server = MockServer::start().await;

    // Set up mock endpoint with HTML content
    let html_content = r#"
        <!DOCTYPE html>
        <html>
        <head><title>Test Page</title></head>
        <body>
            <h1>Welcome</h1>
            <p>This is a test page for integration testing.</p>
            <a href="https://example.com">Example Link</a>
        </body>
        </html>
    "#;

    Mock::given(method("GET"))
        .and(path("/integration-test"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(html_content, "text/html"))
        .mount(&mock_server)
        .await;

    // Create tool with testing config (allows localhost)
    let tool = WebFetchTool::new(WebFetchConfig::for_testing());

    // Fetch the mock endpoint
    let url = format!("{}/integration-test", mock_server.uri());
    let result = tool.fetch(&url).await;

    // Verify successful fetch
    assert!(result.is_ok(), "Fetch should succeed: {:?}", result);

    let fetch_result = result.unwrap();

    // Verify response metadata
    assert_eq!(fetch_result.status, 200, "Status should be 200");
    assert_eq!(
        fetch_result.content_type, "text/html",
        "Content type should be text/html"
    );

    // Verify HTML was converted to markdown-like text
    let content = &fetch_result.content;
    assert!(
        content.contains("Welcome"),
        "Content should contain heading text"
    );
    assert!(
        content.contains("integration testing"),
        "Content should contain paragraph text"
    );
    // html2text should preserve link text
    assert!(
        content.contains("Example Link") || content.contains("example.com"),
        "Content should preserve link information"
    );
}

/// Tests that JSON responses are preserved without HTML conversion.
#[tokio::test]
async fn test_web_fetch_preserves_json_content() {
    use patina::tools::web_fetch::{WebFetchConfig, WebFetchTool};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    let json_content = r#"{"status":"ok","data":{"id":123,"name":"test"}}"#;

    Mock::given(method("GET"))
        .and(path("/api/data"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(json_content, "application/json"))
        .mount(&mock_server)
        .await;

    let tool = WebFetchTool::new(WebFetchConfig::for_testing());
    let url = format!("{}/api/data", mock_server.uri());
    let result = tool.fetch(&url).await;

    assert!(result.is_ok(), "JSON fetch should succeed");

    let fetch_result = result.unwrap();
    assert_eq!(
        fetch_result.content_type, "application/json",
        "Content type should be application/json"
    );

    // JSON should be preserved as-is, not HTML-converted
    assert!(
        fetch_result.content.contains("status") && fetch_result.content.contains("ok"),
        "JSON content should be preserved: {}",
        fetch_result.content
    );
}

/// Tests redirect handling through the complete flow.
#[tokio::test]
async fn test_web_fetch_follows_redirects() {
    use patina::tools::web_fetch::{WebFetchConfig, WebFetchTool};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // Set up redirect chain
    Mock::given(method("GET"))
        .and(path("/redirect-start"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/final-destination"))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/final-destination"))
        .respond_with(ResponseTemplate::new(200).set_body_string("You made it!"))
        .mount(&mock_server)
        .await;

    let tool = WebFetchTool::new(WebFetchConfig::for_testing());
    let url = format!("{}/redirect-start", mock_server.uri());
    let result = tool.fetch(&url).await;

    assert!(result.is_ok(), "Redirect should be followed: {:?}", result);

    let fetch_result = result.unwrap();
    assert_eq!(fetch_result.status, 200);
    assert!(
        fetch_result.content.contains("You made it!"),
        "Should reach final destination"
    );
}
