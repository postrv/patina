//! AWS Bedrock provider for Anthropic Claude models.
//!
//! This module provides the [`BedrockProvider`] LLM provider implementation
//! that routes requests through AWS Bedrock. It translates Patina's internal
//! Anthropic-format messages into Bedrock's `converse-stream` API format and
//! handles AWS SigV4 request signing.
//!
//! # Authentication
//!
//! Credentials are loaded from environment variables:
//! - `AWS_ACCESS_KEY_ID` (required)
//! - `AWS_SECRET_ACCESS_KEY` (required)
//! - `AWS_SESSION_TOKEN` (optional, for temporary credentials)
//! - `AWS_REGION` (required, e.g., "us-east-1")
//!
//! # Model ID Mapping
//!
//! Anthropic model identifiers are mapped to Bedrock model IDs:
//! - `"claude-sonnet-4-20250514"` -> `"anthropic.claude-sonnet-4-20250514-v1:0"`
//! - `"claude-opus-4-20250514"` -> `"anthropic.claude-opus-4-20250514-v1:0"`
//!
//! # Examples
//!
//! ```rust,ignore
//! use patina::api::providers::bedrock::BedrockProvider;
//! use patina::api::provider::LlmProvider;
//!
//! let provider = BedrockProvider::from_env()?;
//! assert_eq!(provider.name(), "bedrock");
//! ```

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use hmac::{Hmac, Mac};
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

use crate::api::provider::{LlmProvider, RequestOptions};
use crate::api::tools::{ToolChoice, ToolDefinition};
use crate::api::{SystemBlock, ThinkingConfig};
use crate::types::{ApiMessageV2, StreamEvent};

/// HMAC-SHA256 type alias for AWS SigV4 signing.
type HmacSha256 = Hmac<Sha256>;

/// AWS Bedrock service name used in SigV4 signing.
const SERVICE: &str = "bedrock";

/// LLM provider for AWS Bedrock (Anthropic models via AWS infrastructure).
///
/// Sends requests to the Bedrock `converse-stream` endpoint with AWS SigV4
/// signed requests. The request body uses the same Anthropic Messages API
/// format, which Bedrock proxies transparently.
///
/// # Examples
///
/// ```rust,ignore
/// use patina::api::providers::bedrock::BedrockProvider;
/// use patina::api::provider::LlmProvider;
///
/// let provider = BedrockProvider::from_env()?;
/// assert_eq!(provider.name(), "bedrock");
/// ```
pub struct BedrockProvider {
    client: reqwest::Client,
    access_key_id: SecretString,
    secret_access_key: SecretString,
    session_token: Option<SecretString>,
    region: String,
    model: String,
    bedrock_model_id: String,
}

impl std::fmt::Debug for BedrockProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BedrockProvider")
            .field("region", &self.region)
            .field("model", &self.model)
            .field("bedrock_model_id", &self.bedrock_model_id)
            .field("access_key_id", &"[REDACTED]")
            .field("secret_access_key", &"[REDACTED]")
            .field(
                "session_token",
                &self.session_token.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

/// AWS SigV4 request body for Bedrock `converse-stream`.
///
/// This mirrors the Anthropic Messages API format, which Bedrock accepts
/// via its `invoke-model-with-response-stream` endpoint.
#[derive(serde::Serialize)]
struct BedrockRequest<'a> {
    anthropic_version: &'static str,
    max_tokens: u32,
    messages: &'a [ApiMessageV2],
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<&'a [SystemBlock]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<&'a ThinkingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a [ToolDefinition]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'a ToolChoice>,
}

impl BedrockProvider {
    /// Creates a new Bedrock provider from explicit credentials.
    ///
    /// # Arguments
    ///
    /// * `access_key_id` - AWS access key ID
    /// * `secret_access_key` - AWS secret access key
    /// * `session_token` - Optional AWS session token (for temporary credentials)
    /// * `region` - AWS region (e.g., "us-east-1")
    /// * `model` - Anthropic model identifier (e.g., "claude-sonnet-4-20250514")
    #[must_use]
    pub fn new(
        access_key_id: SecretString,
        secret_access_key: SecretString,
        session_token: Option<SecretString>,
        region: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        let model = model.into();
        let bedrock_model_id = map_to_bedrock_model_id(&model);
        Self {
            client: reqwest::Client::new(),
            access_key_id,
            secret_access_key,
            session_token,
            region: region.into(),
            model,
            bedrock_model_id,
        }
    }

    /// Creates a new Bedrock provider by reading credentials from environment variables.
    ///
    /// Reads:
    /// - `AWS_ACCESS_KEY_ID` (required)
    /// - `AWS_SECRET_ACCESS_KEY` (required)
    /// - `AWS_SESSION_TOKEN` (optional)
    /// - `AWS_REGION` (required)
    /// - `AWS_BEDROCK_MODEL` (required)
    ///
    /// # Errors
    ///
    /// Returns an error if any required environment variable is not set.
    pub fn from_env() -> Result<Self> {
        let access_key_id = std::env::var("AWS_ACCESS_KEY_ID")
            .map(SecretString::from)
            .map_err(|_| anyhow::anyhow!("AWS_ACCESS_KEY_ID not set"))?;
        let secret_access_key = std::env::var("AWS_SECRET_ACCESS_KEY")
            .map(SecretString::from)
            .map_err(|_| anyhow::anyhow!("AWS_SECRET_ACCESS_KEY not set"))?;
        let session_token = std::env::var("AWS_SESSION_TOKEN")
            .ok()
            .map(SecretString::from);
        let region =
            std::env::var("AWS_REGION").map_err(|_| anyhow::anyhow!("AWS_REGION not set"))?;
        let model = std::env::var("AWS_BEDROCK_MODEL")
            .map_err(|_| anyhow::anyhow!("AWS_BEDROCK_MODEL not set"))?;

        Ok(Self::new(
            access_key_id,
            secret_access_key,
            session_token,
            region,
            model,
        ))
    }

    /// Constructs the Bedrock streaming endpoint URL.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let url = provider.endpoint_url();
    /// // "https://bedrock-runtime.us-east-1.amazonaws.com/model/anthropic.claude-sonnet-4-20250514-v1%3A0/invoke-with-response-stream"
    /// ```
    #[must_use]
    pub fn endpoint_url(&self) -> String {
        format!(
            "https://bedrock-runtime.{}.amazonaws.com/model/{}/invoke-with-response-stream",
            self.region,
            urlencoding::encode(&self.bedrock_model_id),
        )
    }

    /// Signs a request using AWS SigV4 and returns the required headers.
    ///
    /// Produces the `Authorization`, `x-amz-date`, `x-amz-content-sha256`,
    /// and optionally `x-amz-security-token` headers needed for authenticated
    /// requests to AWS Bedrock.
    ///
    /// # Arguments
    ///
    /// * `method` - HTTP method (e.g., "POST")
    /// * `url` - The full request URL
    /// * `body` - The request body bytes
    /// * `datetime` - The request timestamp in ISO 8601 format
    ///
    /// # Errors
    ///
    /// Returns an error if HMAC computation fails (should not happen with valid keys).
    fn sign_request(
        &self,
        method: &str,
        url: &str,
        body: &[u8],
        datetime: &str,
    ) -> Result<Vec<(String, String)>> {
        let date = &datetime[..8];
        let parsed_url = reqwest::Url::parse(url)
            .map_err(|e| anyhow::anyhow!("Failed to parse URL for signing: {e}"))?;
        let host = parsed_url
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("URL has no host"))?;
        let path = parsed_url.path();

        // Step 1: Create canonical request
        let payload_hash = hex::encode(Sha256::digest(body));

        let mut signed_headers = "content-type;host;x-amz-content-sha256;x-amz-date".to_string();
        let mut canonical_headers = format!(
            "content-type:application/json\nhost:{host}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{datetime}\n"
        );

        if let Some(ref token) = self.session_token {
            signed_headers.push_str(";x-amz-security-token");
            canonical_headers
                .push_str(&format!("x-amz-security-token:{}\n", token.expose_secret()));
        }

        let canonical_request =
            format!("{method}\n{path}\n\n{canonical_headers}\n{signed_headers}\n{payload_hash}");

        // Step 2: Create string to sign
        let credential_scope = format!("{date}/{}/{SERVICE}/aws4_request", self.region);
        let canonical_request_hash = hex::encode(Sha256::digest(canonical_request.as_bytes()));
        let string_to_sign =
            format!("AWS4-HMAC-SHA256\n{datetime}\n{credential_scope}\n{canonical_request_hash}");

        // Step 3: Calculate signing key
        let signing_key =
            derive_signing_key(self.secret_access_key.expose_secret(), date, &self.region)?;

        // Step 4: Calculate signature
        let signature = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes())?);

        // Step 5: Build Authorization header
        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}",
            self.access_key_id.expose_secret()
        );

        let mut headers = vec![
            ("Authorization".to_string(), authorization),
            ("x-amz-date".to_string(), datetime.to_string()),
            ("x-amz-content-sha256".to_string(), payload_hash),
        ];

        if let Some(ref token) = self.session_token {
            headers.push((
                "x-amz-security-token".to_string(),
                token.expose_secret().to_string(),
            ));
        }

        Ok(headers)
    }

    /// Processes the SSE stream from a Bedrock response.
    ///
    /// Bedrock streams events in the same SSE format as the direct Anthropic API,
    /// so we reuse the same parsing logic.
    ///
    /// # Errors
    ///
    /// Returns an error if reading the byte stream fails.
    async fn process_stream(
        &self,
        response: reqwest::Response,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<()> {
        // Bedrock uses the same SSE format as the Anthropic API.
        // We reuse the AnthropicClient's stream processing via the same approach.
        use futures::StreamExt;
        use std::time::Duration;

        const SSE_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut current_block_index: usize = 0;
        let mut in_tool_use_block = false;
        let mut in_thinking_block = false;

        loop {
            let chunk = match tokio::time::timeout(SSE_IDLE_TIMEOUT, stream.next()).await {
                Ok(Some(chunk)) => chunk?,
                Ok(None) => break,
                Err(_) => {
                    tracing::error!(
                        "Bedrock SSE stream idle timeout after {}s",
                        SSE_IDLE_TIMEOUT.as_secs()
                    );
                    tx.send(StreamEvent::Error(
                        "Bedrock SSE stream timed out waiting for data".to_string(),
                    ))
                    .await
                    .ok();
                    return Err(anyhow::anyhow!("Bedrock SSE stream idle timeout"));
                }
            };
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim().to_owned();
                drop(buffer.drain(..pos + 1));

                let Some(json_str) = line.strip_prefix("data: ") else {
                    continue;
                };
                if json_str == "[DONE]" {
                    return Ok(());
                }

                let parsed: serde_json::Value = match serde_json::from_str(json_str) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                let event_type = parsed.get("type").and_then(|v| v.as_str()).unwrap_or("");

                match event_type {
                    "message_start" => {
                        // Extract usage stats if present
                        if let Some(usage) = parsed
                            .get("message")
                            .and_then(|m| m.get("usage"))
                            .and_then(|u| {
                                serde_json::from_value::<crate::types::stream::ApiUsage>(u.clone())
                                    .ok()
                            })
                        {
                            tx.send(StreamEvent::Usage(usage)).await.ok();
                        }
                    }
                    "content_block_start" => {
                        if let Some(cb) = parsed.get("content_block") {
                            let block_type = cb.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            current_block_index =
                                parsed.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                            in_tool_use_block = block_type == "tool_use";
                            in_thinking_block = block_type == "thinking";

                            match block_type {
                                "tool_use" => {
                                    if let (Some(id), Some(name)) = (
                                        cb.get("id").and_then(|v| v.as_str()),
                                        cb.get("name").and_then(|v| v.as_str()),
                                    ) {
                                        tx.send(StreamEvent::ToolUseStart {
                                            id: id.to_string(),
                                            name: name.to_string(),
                                            index: current_block_index,
                                        })
                                        .await
                                        .ok();
                                    }
                                }
                                "thinking" => {
                                    tx.send(StreamEvent::ThinkingStart {
                                        index: current_block_index,
                                    })
                                    .await
                                    .ok();
                                }
                                _ => {}
                            }
                        }
                    }
                    "content_block_delta" => {
                        if let Some(delta) = parsed.get("delta") {
                            let block_index = parsed
                                .get("index")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(current_block_index as u64)
                                as usize;
                            let delta_type =
                                delta.get("type").and_then(|v| v.as_str()).unwrap_or("");

                            match delta_type {
                                "input_json_delta" => {
                                    if let Some(pj) =
                                        delta.get("partial_json").and_then(|v| v.as_str())
                                    {
                                        tx.send(StreamEvent::ToolUseInputDelta {
                                            index: block_index,
                                            partial_json: pj.to_string(),
                                        })
                                        .await
                                        .ok();
                                    }
                                }
                                "thinking_delta" => {
                                    if let Some(text) =
                                        delta.get("thinking").and_then(|v| v.as_str())
                                    {
                                        tx.send(StreamEvent::ThinkingDelta(text.to_string()))
                                            .await
                                            .ok();
                                    }
                                }
                                _ => {
                                    if let Some(text) = delta.get("text").and_then(|v| v.as_str()) {
                                        tx.send(StreamEvent::ContentDelta(text.to_string()))
                                            .await
                                            .ok();
                                    }
                                }
                            }
                        }
                    }
                    "content_block_stop" => {
                        let block_index = parsed
                            .get("index")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(current_block_index as u64)
                            as usize;
                        let event = if in_tool_use_block {
                            StreamEvent::ToolUseComplete { index: block_index }
                        } else if in_thinking_block {
                            StreamEvent::ThinkingComplete { index: block_index }
                        } else {
                            StreamEvent::ContentBlockComplete { index: block_index }
                        };
                        tx.send(event).await.ok();
                        in_tool_use_block = false;
                        in_thinking_block = false;
                    }
                    "message_delta" => {
                        if let Some(delta) = parsed.get("delta") {
                            if let Some(stop_reason) =
                                delta.get("stop_reason").and_then(|v| v.as_str())
                            {
                                let reason = match stop_reason {
                                    "tool_use" => crate::types::content::StopReason::ToolUse,
                                    "max_tokens" => crate::types::content::StopReason::MaxTokens,
                                    "stop_sequence" => {
                                        crate::types::content::StopReason::StopSequence
                                    }
                                    _ => crate::types::content::StopReason::EndTurn,
                                };
                                tx.send(StreamEvent::MessageComplete {
                                    stop_reason: reason,
                                })
                                .await
                                .ok();
                            }
                        }
                    }
                    "message_stop" => {
                        tx.send(StreamEvent::MessageStop).await.ok();
                    }
                    _ => {}
                }
            }
        }

        Ok(())
    }
}

impl LlmProvider for BedrockProvider {
    fn name(&self) -> &str {
        "bedrock"
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn stream_message<'a>(
        &'a self,
        messages: &'a [ApiMessageV2],
        tools: Option<&'a [ToolDefinition]>,
        tool_choice: Option<&'a ToolChoice>,
        options: &'a RequestOptions,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let request = BedrockRequest {
                anthropic_version: "bedrock-2023-05-31",
                max_tokens: 8192,
                messages,
                system: options.system.as_deref(),
                thinking: options.thinking.as_ref(),
                tools,
                tool_choice,
            };

            let body = serde_json::to_vec(&request)
                .map_err(|e| anyhow::anyhow!("Failed to serialize Bedrock request: {e}"))?;

            let url = self.endpoint_url();
            let datetime = chrono_free_datetime();

            let sign_headers = self.sign_request("POST", &url, &body, &datetime)?;

            crate::api::retry::send_with_retry(
                || async {
                    let mut req = self
                        .client
                        .post(&url)
                        .header("Content-Type", "application/json")
                        .header("Accept", "application/vnd.amazon.eventstream");

                    for (key, value) in &sign_headers {
                        req = req.header(key.as_str(), value.as_str());
                    }

                    req.body(body.clone()).send().await.map_err(Into::into)
                },
                |response, tx| async move { self.process_stream(response, tx).await },
                &tx,
            )
            .await
        })
    }
}

/// Maps an Anthropic model identifier to the AWS Bedrock model ID.
///
/// Bedrock model IDs follow the format `anthropic.{model}-v1:0`.
/// If the model already has the `anthropic.` prefix, it is returned as-is.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(
///     map_to_bedrock_model_id("claude-sonnet-4-20250514"),
///     "anthropic.claude-sonnet-4-20250514-v1:0"
/// );
/// assert_eq!(
///     map_to_bedrock_model_id("anthropic.claude-sonnet-4-20250514-v1:0"),
///     "anthropic.claude-sonnet-4-20250514-v1:0"
/// );
/// ```
#[must_use]
pub fn map_to_bedrock_model_id(model: &str) -> String {
    if model.starts_with("anthropic.") {
        return model.to_string();
    }
    format!("anthropic.{model}-v1:0")
}

/// Constructs the Bedrock streaming endpoint URL for a given region and model ID.
///
/// # Arguments
///
/// * `region` - AWS region (e.g., "us-east-1")
/// * `bedrock_model_id` - The full Bedrock model ID (e.g., "anthropic.claude-sonnet-4-20250514-v1:0")
///
/// # Examples
///
/// ```rust,ignore
/// let url = build_bedrock_endpoint("us-east-1", "anthropic.claude-sonnet-4-20250514-v1:0");
/// assert!(url.contains("bedrock-runtime.us-east-1.amazonaws.com"));
/// ```
#[must_use]
pub fn build_bedrock_endpoint(region: &str, bedrock_model_id: &str) -> String {
    format!(
        "https://bedrock-runtime.{region}.amazonaws.com/model/{}/invoke-with-response-stream",
        urlencoding::encode(bedrock_model_id),
    )
}

/// Derives the AWS SigV4 signing key from the secret access key, date, region, and service.
///
/// Implements the key derivation: `HMAC(HMAC(HMAC(HMAC("AWS4" + secret, date), region), service), "aws4_request")`
///
/// # Errors
///
/// Returns an error if HMAC computation fails.
fn derive_signing_key(secret: &str, date: &str, region: &str) -> Result<Vec<u8>> {
    let k_date = hmac_sha256(format!("AWS4{secret}").as_bytes(), date.as_bytes())?;
    let k_region = hmac_sha256(&k_date, region.as_bytes())?;
    let k_service = hmac_sha256(&k_region, SERVICE.as_bytes())?;
    hmac_sha256(&k_service, b"aws4_request")
}

/// Computes HMAC-SHA256.
///
/// # Errors
///
/// Returns an error if the HMAC key is invalid (should not happen with valid inputs).
fn hmac_sha256(key: &[u8], data: &[u8]) -> Result<Vec<u8>> {
    let mut mac =
        HmacSha256::new_from_slice(key).map_err(|e| anyhow::anyhow!("HMAC key error: {e}"))?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().to_vec())
}

/// Generates an ISO 8601 datetime string without depending on chrono.
///
/// Format: `YYYYMMDDTHHMMSSZ` (e.g., `"20260329T120000Z"`).
#[must_use]
fn chrono_free_datetime() -> String {
    use std::time::SystemTime;
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("system time before Unix epoch");
    let secs = duration.as_secs();

    // Calculate date components from epoch seconds
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Convert days since epoch to year/month/day
    let (year, month, day) = days_to_ymd(days);

    format!("{year:04}{month:02}{day:02}T{hours:02}{minutes:02}{seconds:02}Z")
}

/// Converts days since Unix epoch to (year, month, day).
///
/// Uses a civil calendar algorithm. This avoids pulling in the `chrono` or `time` crate
/// just for timestamp formatting.
fn days_to_ymd(days_since_epoch: u64) -> (u64, u64, u64) {
    // Algorithm from https://howardhinnant.github.io/date_algorithms.html
    let z = days_since_epoch + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::SecretString;

    #[test]
    fn test_map_to_bedrock_model_id_standard() {
        assert_eq!(
            map_to_bedrock_model_id("claude-sonnet-4-20250514"),
            "anthropic.claude-sonnet-4-20250514-v1:0"
        );
    }

    #[test]
    fn test_map_to_bedrock_model_id_opus() {
        assert_eq!(
            map_to_bedrock_model_id("claude-opus-4-20250514"),
            "anthropic.claude-opus-4-20250514-v1:0"
        );
    }

    #[test]
    fn test_map_to_bedrock_model_id_haiku() {
        assert_eq!(
            map_to_bedrock_model_id("claude-3-5-haiku-20241022"),
            "anthropic.claude-3-5-haiku-20241022-v1:0"
        );
    }

    #[test]
    fn test_map_to_bedrock_model_id_already_prefixed() {
        let id = "anthropic.claude-sonnet-4-20250514-v1:0";
        assert_eq!(map_to_bedrock_model_id(id), id);
    }

    #[test]
    fn test_build_bedrock_endpoint() {
        let url = build_bedrock_endpoint("us-east-1", "anthropic.claude-sonnet-4-20250514-v1:0");
        assert_eq!(
            url,
            "https://bedrock-runtime.us-east-1.amazonaws.com/model/anthropic.claude-sonnet-4-20250514-v1%3A0/invoke-with-response-stream"
        );
    }

    #[test]
    fn test_build_bedrock_endpoint_different_region() {
        let url = build_bedrock_endpoint("eu-west-1", "anthropic.claude-opus-4-20250514-v1:0");
        assert!(url.contains("bedrock-runtime.eu-west-1.amazonaws.com"));
        assert!(url.contains("anthropic.claude-opus-4-20250514-v1%3A0"));
    }

    #[test]
    fn test_bedrock_provider_endpoint_url() {
        let provider = BedrockProvider::new(
            SecretString::from("AKIAIOSFODNN7EXAMPLE"),
            SecretString::from("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"),
            None,
            "us-west-2",
            "claude-sonnet-4-20250514",
        );
        let url = provider.endpoint_url();
        assert!(url.contains("bedrock-runtime.us-west-2.amazonaws.com"));
        assert!(url.contains("anthropic.claude-sonnet-4-20250514-v1%3A0"));
    }

    #[test]
    fn test_bedrock_provider_name() {
        let provider = BedrockProvider::new(
            SecretString::from("AKIAIOSFODNN7EXAMPLE"),
            SecretString::from("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"),
            None,
            "us-east-1",
            "claude-sonnet-4-20250514",
        );
        assert_eq!(provider.name(), "bedrock");
    }

    #[test]
    fn test_bedrock_provider_model() {
        let provider = BedrockProvider::new(
            SecretString::from("AKIAIOSFODNN7EXAMPLE"),
            SecretString::from("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"),
            None,
            "us-east-1",
            "claude-sonnet-4-20250514",
        );
        assert_eq!(provider.model(), "claude-sonnet-4-20250514");
    }

    #[test]
    fn test_bedrock_provider_with_session_token() {
        let provider = BedrockProvider::new(
            SecretString::from("AKIAIOSFODNN7EXAMPLE"),
            SecretString::from("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"),
            Some(SecretString::from("FwoGZXIvYXdzEBY...")),
            "us-east-1",
            "claude-sonnet-4-20250514",
        );
        assert!(provider.session_token.is_some());
    }

    #[test]
    fn test_sign_request_produces_authorization_header() {
        let provider = BedrockProvider::new(
            SecretString::from("AKIAIOSFODNN7EXAMPLE"),
            SecretString::from("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"),
            None,
            "us-east-1",
            "claude-sonnet-4-20250514",
        );
        let body = b"{}";
        let datetime = "20260329T120000Z";
        let url = provider.endpoint_url();

        let headers = provider
            .sign_request("POST", &url, body, datetime)
            .expect("signing should succeed");

        // Should contain Authorization, x-amz-date, x-amz-content-sha256
        let header_names: Vec<&str> = headers.iter().map(|(k, _)| k.as_str()).collect();
        assert!(header_names.contains(&"Authorization"));
        assert!(header_names.contains(&"x-amz-date"));
        assert!(header_names.contains(&"x-amz-content-sha256"));
        assert!(!header_names.contains(&"x-amz-security-token"));

        // Authorization should have correct format
        let auth = headers
            .iter()
            .find(|(k, _)| k == "Authorization")
            .map(|(_, v)| v)
            .expect("Authorization header present");
        assert!(auth.starts_with("AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/"));
        assert!(auth.contains("SignedHeaders="));
        assert!(auth.contains("Signature="));
    }

    #[test]
    fn test_sign_request_with_session_token() {
        let provider = BedrockProvider::new(
            SecretString::from("AKIAIOSFODNN7EXAMPLE"),
            SecretString::from("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"),
            Some(SecretString::from("session-token-value")),
            "us-east-1",
            "claude-sonnet-4-20250514",
        );
        let body = b"{}";
        let datetime = "20260329T120000Z";
        let url = provider.endpoint_url();

        let headers = provider
            .sign_request("POST", &url, body, datetime)
            .expect("signing should succeed");

        let header_names: Vec<&str> = headers.iter().map(|(k, _)| k.as_str()).collect();
        assert!(header_names.contains(&"x-amz-security-token"));

        let token = headers
            .iter()
            .find(|(k, _)| k == "x-amz-security-token")
            .map(|(_, v)| v)
            .expect("security token header present");
        assert_eq!(token, "session-token-value");
    }

    #[test]
    fn test_derive_signing_key_deterministic() {
        let key1 = derive_signing_key("secret", "20260329", "us-east-1").expect("should succeed");
        let key2 = derive_signing_key("secret", "20260329", "us-east-1").expect("should succeed");
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_derive_signing_key_different_dates_differ() {
        let key1 = derive_signing_key("secret", "20260329", "us-east-1").expect("should succeed");
        let key2 = derive_signing_key("secret", "20260330", "us-east-1").expect("should succeed");
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_hmac_sha256_basic() {
        let result = hmac_sha256(b"key", b"data").expect("should succeed");
        assert_eq!(result.len(), 32); // SHA-256 produces 32 bytes
    }

    #[test]
    fn test_chrono_free_datetime_format() {
        let dt = chrono_free_datetime();
        // Format: YYYYMMDDTHHMMSSZ
        assert_eq!(dt.len(), 16);
        assert_eq!(&dt[8..9], "T");
        assert!(dt.ends_with('Z'));
    }

    #[test]
    fn test_days_to_ymd_epoch() {
        let (y, m, d) = days_to_ymd(0);
        assert_eq!((y, m, d), (1970, 1, 1));
    }

    #[test]
    fn test_days_to_ymd_known_date() {
        // 2026-03-29 is day 20541 since epoch
        let (y, m, d) = days_to_ymd(20_541);
        assert_eq!((y, m, d), (2026, 3, 29));
    }

    #[test]
    fn test_bedrock_request_serialization() {
        let messages = vec![ApiMessageV2::user("Hello!")];
        let request = BedrockRequest {
            anthropic_version: "bedrock-2023-05-31",
            max_tokens: 8192,
            messages: &messages,
            system: None,
            thinking: None,
            tools: None,
            tool_choice: None,
        };
        let json = serde_json::to_value(&request).expect("should serialize");
        assert_eq!(json["anthropic_version"], "bedrock-2023-05-31");
        assert_eq!(json["max_tokens"], 8192);
        assert!(json.get("system").is_none());
        assert!(json.get("tools").is_none());
    }

    #[test]
    fn test_bedrock_request_with_system() {
        let messages = vec![ApiMessageV2::user("Hello!")];
        let system = vec![SystemBlock {
            block_type: "text".to_string(),
            text: "You are helpful.".to_string(),
            cache_control: None,
        }];
        let request = BedrockRequest {
            anthropic_version: "bedrock-2023-05-31",
            max_tokens: 8192,
            messages: &messages,
            system: Some(&system),
            thinking: None,
            tools: None,
            tool_choice: None,
        };
        let json = serde_json::to_value(&request).expect("should serialize");
        assert!(json.get("system").is_some());
        assert_eq!(json["system"][0]["text"], "You are helpful.");
    }

    #[test]
    #[serial_test::serial]
    fn test_bedrock_from_env_missing_access_key() {
        // Ensure the env vars are not set
        std::env::remove_var("AWS_ACCESS_KEY_ID");
        std::env::remove_var("AWS_SECRET_ACCESS_KEY");
        std::env::remove_var("AWS_SESSION_TOKEN");
        std::env::remove_var("AWS_REGION");
        std::env::remove_var("AWS_BEDROCK_MODEL");
        let result = BedrockProvider::from_env();
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("AWS_ACCESS_KEY_ID"),
            "Error should mention AWS_ACCESS_KEY_ID, got: {err}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_bedrock_from_env_missing_secret_key() {
        std::env::set_var("AWS_ACCESS_KEY_ID", "test-key");
        std::env::remove_var("AWS_SECRET_ACCESS_KEY");
        std::env::remove_var("AWS_SESSION_TOKEN");
        std::env::remove_var("AWS_REGION");
        std::env::remove_var("AWS_BEDROCK_MODEL");
        let result = BedrockProvider::from_env();
        std::env::remove_var("AWS_ACCESS_KEY_ID");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("AWS_SECRET_ACCESS_KEY"),
            "Error should mention AWS_SECRET_ACCESS_KEY, got: {err}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_bedrock_from_env_missing_region() {
        std::env::set_var("AWS_ACCESS_KEY_ID", "test-key");
        std::env::set_var("AWS_SECRET_ACCESS_KEY", "test-secret");
        std::env::remove_var("AWS_SESSION_TOKEN");
        std::env::remove_var("AWS_REGION");
        std::env::remove_var("AWS_BEDROCK_MODEL");
        let result = BedrockProvider::from_env();
        std::env::remove_var("AWS_ACCESS_KEY_ID");
        std::env::remove_var("AWS_SECRET_ACCESS_KEY");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("AWS_REGION"),
            "Error should mention AWS_REGION, got: {err}"
        );
    }
}
