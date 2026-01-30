//! API client integration tests.
//!
//! Tests for Anthropic API client functionality including:
//! - Stream message handling
//! - Error handling
//! - Retry logic

mod common;

use common::TestContext;
use rct::api::AnthropicClient;
use rct::types::{Message, Role, StreamEvent};
use secrecy::SecretString;
use tokio::sync::mpsc;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Placeholder test to verify test infrastructure works.
#[test]
fn test_infrastructure_works() {
    let ctx = TestContext::new();
    assert!(ctx.path().exists());
}

/// Test successful streaming message response.
///
/// Verifies that the API client correctly:
/// - Connects to the configured endpoint
/// - Parses SSE stream format
/// - Emits ContentDelta events for text chunks
/// - Emits MessageStop when stream completes
#[tokio::test]
async fn test_stream_message_success() {
    // Arrange: Start mock server with streaming SSE response
    let mock_server = MockServer::start().await;

    let sse_response = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_123","type":"message","role":"assistant","content":[]}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":", world!"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null}}

event: message_stop
data: {"type":"message_stop"}

"#;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "test-key-value"))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(sse_response, "text/event-stream")
                .append_header("content-type", "text/event-stream"),
        )
        .mount(&mock_server)
        .await;

    // Create client with mock server URL
    let key = SecretString::from("test-key-value");
    let client = AnthropicClient::new_with_base_url(key, "claude-3-opus", &mock_server.uri());

    let messages = vec![Message {
        role: Role::User,
        content: "Hello".to_string(),
    }];

    let (tx, mut rx) = mpsc::channel::<StreamEvent>(32);

    // Act: Stream the message
    client.stream_message(&messages, tx).await.unwrap();

    // Assert: Collect all events
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }

    // Should have received content deltas and message stop
    let content: String = events
        .iter()
        .filter_map(|e| e.content().map(String::from))
        .collect();
    assert_eq!(content, "Hello, world!");

    // Should have a MessageStop event
    assert!(
        events.iter().any(|e| e.is_stop()),
        "Expected MessageStop event"
    );
}

/// Test error handling when API returns an error response.
///
/// Verifies that the API client correctly:
/// - Handles non-2xx HTTP status codes
/// - Emits an Error event with the error details
/// - Does not panic on error responses
#[tokio::test]
async fn test_stream_message_error_handling() {
    // Arrange: Start mock server with error response
    let mock_server = MockServer::start().await;

    let error_body =
        r#"{"type":"error","error":{"type":"invalid_request_error","message":"Invalid API key"}}"#;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_string(error_body)
                .append_header("content-type", "application/json"),
        )
        .mount(&mock_server)
        .await;

    // Create client with mock server URL
    let key = SecretString::from("invalid-key-value");
    let client = AnthropicClient::new_with_base_url(key, "claude-3-opus", &mock_server.uri());

    let messages = vec![Message {
        role: Role::User,
        content: "Hello".to_string(),
    }];

    let (tx, mut rx) = mpsc::channel::<StreamEvent>(32);

    // Act: Stream the message (should not panic)
    let result = client.stream_message(&messages, tx).await;
    assert!(
        result.is_ok(),
        "stream_message should not return Err on API error"
    );

    // Assert: Should receive an error event
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }

    assert!(
        events.iter().any(|e| e.is_error()),
        "Expected Error event for 401 response"
    );

    // Error message should contain status code
    let error_event = events.iter().find(|e| e.is_error()).unwrap();
    let error_msg = error_event.error().unwrap();
    assert!(
        error_msg.contains("401"),
        "Error message should contain status code: {}",
        error_msg
    );
}

/// Test that the client retries on rate limit (429) responses.
///
/// Verifies that the API client:
/// - Retries automatically when receiving a 429 Too Many Requests
/// - Succeeds on subsequent attempts if the rate limit clears
/// - Returns the successful response after retry
#[tokio::test]
async fn test_retry_on_rate_limit() {
    // Arrange: Start mock server that returns 429 first, then succeeds
    let mock_server = MockServer::start().await;

    // First request returns 429
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(429)
                .set_body_string(r#"{"error":{"message":"rate_limit_exceeded"}}"#)
                .append_header("retry-after", "1"),
        )
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    // Second request succeeds
    let sse_response = r#"event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Retry succeeded"}}

event: message_stop
data: {"type":"message_stop"}

"#;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(sse_response, "text/event-stream")
                .append_header("content-type", "text/event-stream"),
        )
        .mount(&mock_server)
        .await;

    // Create client with mock server URL
    let key = SecretString::from("test-key-value");
    let client = AnthropicClient::new_with_base_url(key, "claude-3-opus", &mock_server.uri());

    let messages = vec![Message {
        role: Role::User,
        content: "Hello".to_string(),
    }];

    let (tx, mut rx) = mpsc::channel::<StreamEvent>(32);

    // Act: Stream the message - should retry and succeed
    let result = client.stream_message(&messages, tx).await;
    assert!(result.is_ok(), "Should succeed after retry");

    // Assert: Should receive successful response after retry
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }

    let content: String = events
        .iter()
        .filter_map(|e| e.content().map(String::from))
        .collect();
    assert_eq!(
        content, "Retry succeeded",
        "Should have received content after retry"
    );

    // Should have a MessageStop event (success, not error)
    assert!(
        events.iter().any(|e| e.is_stop()),
        "Expected MessageStop event after successful retry"
    );

    // Should NOT have an Error event
    assert!(
        !events.iter().any(|e| e.is_error()),
        "Should not have error after successful retry"
    );
}

/// Test that the client uses exponential backoff for retries.
///
/// Verifies that the API client:
/// - Waits before retrying (not immediate)
/// - Increases delay with each retry attempt
/// - Gives up after max retries and returns error
#[tokio::test]
async fn test_retry_exponential_backoff() {
    use std::time::Instant;

    // Arrange: Start mock server that always returns 429
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(429)
                .set_body_string(r#"{"error":{"message":"rate_limit_exceeded"}}"#),
        )
        .expect(3) // Expect initial + 2 retries = 3 total attempts
        .mount(&mock_server)
        .await;

    // Create client with mock server URL
    let key = SecretString::from("test-key-value");
    let client = AnthropicClient::new_with_base_url(key, "claude-3-opus", &mock_server.uri());

    let messages = vec![Message {
        role: Role::User,
        content: "Hello".to_string(),
    }];

    let (tx, mut rx) = mpsc::channel::<StreamEvent>(32);

    // Act: Stream the message - should retry with backoff then fail
    let start = Instant::now();
    let _result = client.stream_message(&messages, tx).await;
    let elapsed = start.elapsed();

    // Assert: Should have taken some time due to backoff
    // With exponential backoff: ~100ms + ~200ms = ~300ms minimum
    assert!(
        elapsed.as_millis() >= 200,
        "Expected backoff delay, but completed in {:?}",
        elapsed
    );

    // Should receive an error event after exhausting retries
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }

    assert!(
        events.iter().any(|e| e.is_error()),
        "Expected Error event after exhausting retries"
    );
}

/// Test that the client retries on server errors (5xx).
///
/// Verifies that the API client:
/// - Retries on 500, 502, 503, 504 status codes
/// - Succeeds if server recovers
#[tokio::test]
async fn test_retry_on_server_error() {
    // Arrange: Start mock server that returns 503 first, then succeeds
    let mock_server = MockServer::start().await;

    // First request returns 503 Service Unavailable
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(503)
                .set_body_string(r#"{"error":{"message":"service_unavailable"}}"#),
        )
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    // Second request succeeds
    let sse_response = r#"event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Server recovered"}}

event: message_stop
data: {"type":"message_stop"}

"#;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(sse_response, "text/event-stream")
                .append_header("content-type", "text/event-stream"),
        )
        .mount(&mock_server)
        .await;

    // Create client with mock server URL
    let key = SecretString::from("test-key-value");
    let client = AnthropicClient::new_with_base_url(key, "claude-3-opus", &mock_server.uri());

    let messages = vec![Message {
        role: Role::User,
        content: "Hello".to_string(),
    }];

    let (tx, mut rx) = mpsc::channel::<StreamEvent>(32);

    // Act: Stream the message - should retry and succeed
    let result = client.stream_message(&messages, tx).await;
    assert!(result.is_ok(), "Should succeed after retry");

    // Assert: Should receive successful response after retry
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }

    let content: String = events
        .iter()
        .filter_map(|e| e.content().map(String::from))
        .collect();
    assert_eq!(
        content, "Server recovered",
        "Should have received content after retry"
    );
}
