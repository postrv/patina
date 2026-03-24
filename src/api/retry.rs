//! Retry logic for HTTP streaming requests.
//!
//! Provides exponential backoff retry for transient API errors (429, 5xx).
//! Used by both `AnthropicClient` and `OpenRouterProvider` to avoid duplicating
//! the retry loop in every streaming method.

use std::future::Future;
use std::time::Duration;

use anyhow::Result;
use reqwest::StatusCode;
use tokio::sync::mpsc;

use crate::types::StreamEvent;

/// Maximum number of retry attempts for retryable errors.
const MAX_RETRIES: u32 = 2;

/// Base delay for exponential backoff in milliseconds.
const BASE_BACKOFF_MS: u64 = 100;

/// Checks if an HTTP status code should trigger a retry.
///
/// Retries on:
/// - 429 Too Many Requests (rate limit)
/// - 5xx Server Errors (500, 502, 503, 504, etc.)
#[must_use]
pub fn is_retryable_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

/// Computes the backoff delay for a given attempt (0-indexed).
///
/// Uses exponential backoff: 100ms, 200ms, 400ms, ...
#[must_use]
pub fn backoff_delay(attempt: u32) -> Duration {
    Duration::from_millis(BASE_BACKOFF_MS * (1 << attempt))
}

/// Executes an HTTP streaming request with retry on transient errors.
///
/// Calls `make_request` to build and send the HTTP request. On success,
/// delegates to `on_success` to process the response stream. On transient
/// errors (429, 5xx), retries with exponential backoff. On non-retryable
/// errors or exhausted retries, sends `StreamEvent::Error` on the channel.
///
/// # Arguments
///
/// * `make_request` - Async closure that sends the HTTP request and returns the response
/// * `on_success` - Async closure that processes a successful response stream
/// * `tx` - Channel sender for streaming events (used to report errors)
///
/// # Errors
///
/// Returns `Err` only for irrecoverable transport errors (DNS failure, connection refused).
/// API-level errors (4xx, 5xx) are sent as `StreamEvent::Error` on the channel.
pub async fn send_with_retry<MakeReq, MakeFut, OnSuccess, OnSuccessFut>(
    mut make_request: MakeReq,
    on_success: OnSuccess,
    tx: &mpsc::Sender<StreamEvent>,
) -> Result<()>
where
    MakeReq: FnMut() -> MakeFut,
    MakeFut: Future<Output = Result<reqwest::Response>>,
    OnSuccess: FnOnce(reqwest::Response, mpsc::Sender<StreamEvent>) -> OnSuccessFut,
    OnSuccessFut: Future<Output = Result<()>>,
{
    let mut last_error: Option<(StatusCode, String)> = None;
    let mut on_success = Some(on_success);

    for attempt in 0..=MAX_RETRIES {
        let response = make_request().await?;
        let status = response.status();

        if status.is_success() {
            // Take the on_success closure (it's FnOnce)
            let handler = on_success.take().expect("on_success called only once");
            return handler(response, tx.clone()).await;
        }

        if is_retryable_status(status) && attempt < MAX_RETRIES {
            let body = response.text().await.unwrap_or_default();
            last_error = Some((status, body));
            tokio::time::sleep(backoff_delay(attempt)).await;
            continue;
        }

        // Non-retryable error or exhausted retries
        let body = response.text().await.unwrap_or_default();
        tx.send(StreamEvent::Error(format!("{status}: {body}")))
            .await
            .ok();
        return Ok(());
    }

    // Exhausted retries — send the last error
    if let Some((status, body)) = last_error {
        tx.send(StreamEvent::Error(format!("{status}: {body}")))
            .await
            .ok();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_retryable_429() {
        assert!(is_retryable_status(StatusCode::TOO_MANY_REQUESTS));
    }

    #[test]
    fn test_is_retryable_5xx() {
        assert!(is_retryable_status(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(is_retryable_status(StatusCode::BAD_GATEWAY));
        assert!(is_retryable_status(StatusCode::SERVICE_UNAVAILABLE));
        assert!(is_retryable_status(StatusCode::GATEWAY_TIMEOUT));
    }

    #[test]
    fn test_not_retryable_4xx() {
        assert!(!is_retryable_status(StatusCode::BAD_REQUEST));
        assert!(!is_retryable_status(StatusCode::UNAUTHORIZED));
        assert!(!is_retryable_status(StatusCode::FORBIDDEN));
        assert!(!is_retryable_status(StatusCode::NOT_FOUND));
        assert!(!is_retryable_status(StatusCode::UNPROCESSABLE_ENTITY));
    }

    #[test]
    fn test_not_retryable_2xx() {
        assert!(!is_retryable_status(StatusCode::OK));
        assert!(!is_retryable_status(StatusCode::CREATED));
    }

    #[test]
    fn test_backoff_delay_exponential() {
        assert_eq!(backoff_delay(0), Duration::from_millis(100));
        assert_eq!(backoff_delay(1), Duration::from_millis(200));
        assert_eq!(backoff_delay(2), Duration::from_millis(400));
    }

    #[test]
    fn test_max_retries_value() {
        // Ensure the constant matches the documented behavior
        assert_eq!(MAX_RETRIES, 2);
    }

    #[test]
    fn test_base_backoff_value() {
        assert_eq!(BASE_BACKOFF_MS, 100);
    }

    // ── send_with_retry integration tests ──

    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_send_with_retry_success_first_try() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .expect(1)
            .mount(&mock_server)
            .await;

        let (tx, mut rx) = mpsc::channel(10);
        let url = mock_server.uri();
        let client = reqwest::Client::new();
        send_with_retry(
            || {
                let c = client.clone();
                let u = url.clone();
                async move { Ok(c.get(&u).send().await?) }
            },
            |_response, _tx| async move { Ok(()) },
            &tx,
        )
        .await
        .unwrap();

        // No error events should have been sent
        drop(tx);
        assert!(rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn test_send_with_retry_retries_on_429() {
        let mock_server = MockServer::start().await;
        // First call: 429, second call: 200
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(429).set_body_string("rate limited"))
            .up_to_n_times(1)
            .expect(1)
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .expect(1)
            .mount(&mock_server)
            .await;

        let (tx, mut rx) = mpsc::channel(10);
        let url = mock_server.uri();
        let client = reqwest::Client::new();

        send_with_retry(
            || {
                let c = client.clone();
                let u = url.clone();
                async move { Ok(c.get(&u).send().await?) }
            },
            |_response, _tx| async { Ok(()) },
            &tx,
        )
        .await
        .unwrap();

        // No error events — retry succeeded
        drop(tx);
        assert!(rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn test_send_with_retry_retries_on_5xx() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500).set_body_string("server error"))
            .up_to_n_times(1)
            .expect(1)
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .expect(1)
            .mount(&mock_server)
            .await;

        let (tx, mut rx) = mpsc::channel(10);
        let url = mock_server.uri();
        let client = reqwest::Client::new();

        send_with_retry(
            || {
                let c = client.clone();
                let u = url.clone();
                async move { Ok(c.get(&u).send().await?) }
            },
            |_response, _tx| async { Ok(()) },
            &tx,
        )
        .await
        .unwrap();

        drop(tx);
        assert!(rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn test_send_with_retry_non_retryable_4xx() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(400).set_body_string("bad request"))
            .expect(1)
            .mount(&mock_server)
            .await;

        let (tx, mut rx) = mpsc::channel(10);
        let url = mock_server.uri();
        let client = reqwest::Client::new();

        send_with_retry(
            || {
                let c = client.clone();
                let u = url.clone();
                async move { Ok(c.get(&u).send().await?) }
            },
            |_response, _tx| async { Ok(()) },
            &tx,
        )
        .await
        .unwrap();

        // Should get exactly one error event, no retries
        let event = rx.recv().await.expect("should receive error event");
        assert!(matches!(event, StreamEvent::Error(msg) if msg.contains("400")));
    }

    #[tokio::test]
    async fn test_send_with_retry_exhausted() {
        let mock_server = MockServer::start().await;
        // All 3 attempts (initial + 2 retries) return 429
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(429).set_body_string("rate limited"))
            .expect(3)
            .mount(&mock_server)
            .await;

        let (tx, mut rx) = mpsc::channel(10);
        let url = mock_server.uri();
        let client = reqwest::Client::new();

        send_with_retry(
            || {
                let c = client.clone();
                let u = url.clone();
                async move { Ok(c.get(&u).send().await?) }
            },
            |_response, _tx| async { Ok(()) },
            &tx,
        )
        .await
        .unwrap();

        // Should get error after exhausting retries
        let event = rx.recv().await.expect("should receive error event");
        assert!(matches!(event, StreamEvent::Error(msg) if msg.contains("429")));
    }
}
