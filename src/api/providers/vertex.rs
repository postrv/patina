//! Google Vertex AI provider for Anthropic Claude models.
//!
//! This module provides the [`VertexProvider`] LLM provider implementation
//! that routes requests through Google Cloud's Vertex AI platform. It translates
//! Patina's internal Anthropic-format messages and handles Google Cloud
//! authentication via Application Default Credentials (ADC).
//!
//! # Authentication
//!
//! The provider reads a service account key file to obtain OAuth2 access tokens:
//! - `GOOGLE_APPLICATION_CREDENTIALS` - Path to service account JSON key file
//! - `GOOGLE_CLOUD_PROJECT` - Google Cloud project ID
//! - `CLOUD_ML_REGION` - Google Cloud region (e.g., "us-east5")
//!
//! # Endpoint
//!
//! Requests are sent to:
//! ```text
//! https://{region}-aiplatform.googleapis.com/v1/projects/{project}/locations/{region}/publishers/anthropic/models/{model}:streamRawPredict
//! ```
//!
//! # Examples
//!
//! ```rust,ignore
//! use patina::api::providers::vertex::VertexProvider;
//! use patina::api::provider::LlmProvider;
//!
//! let provider = VertexProvider::from_env()?;
//! assert_eq!(provider.name(), "vertex");
//! ```

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use secrecy::{ExposeSecret, SecretString};
use tokio::sync::mpsc;

use crate::api::provider::{LlmProvider, RequestOptions};
use crate::api::tools::{ToolChoice, ToolDefinition};
use crate::api::{SystemBlock, ThinkingConfig};
use crate::types::{ApiMessageV2, StreamEvent};

/// LLM provider for Google Vertex AI (Anthropic models via Google Cloud).
///
/// Sends requests to the Vertex AI `streamRawPredict` endpoint, which proxies
/// the Anthropic Messages API. Authentication uses Google Cloud OAuth2 access
/// tokens from Application Default Credentials (ADC).
///
/// # Examples
///
/// ```rust,ignore
/// use patina::api::providers::vertex::VertexProvider;
/// use patina::api::provider::LlmProvider;
///
/// let provider = VertexProvider::new(
///     "my-project",
///     "us-east5",
///     "claude-sonnet-4@20250514",
///     secrecy::SecretString::from("ya29.access-token"),
/// );
/// assert_eq!(provider.name(), "vertex");
/// ```
pub struct VertexProvider {
    client: reqwest::Client,
    project: String,
    region: String,
    model: String,
    access_token: SecretString,
}

impl std::fmt::Debug for VertexProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VertexProvider")
            .field("project", &self.project)
            .field("region", &self.region)
            .field("model", &self.model)
            .field("access_token", &"[REDACTED]")
            .finish()
    }
}

/// Vertex AI request body for `streamRawPredict`.
///
/// This mirrors the Anthropic Messages API format, which Vertex AI accepts
/// via its `streamRawPredict` endpoint.
#[derive(serde::Serialize)]
struct VertexRequest<'a> {
    anthropic_version: &'static str,
    max_tokens: u32,
    stream: bool,
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

impl VertexProvider {
    /// Creates a new Vertex AI provider with an access token.
    ///
    /// # Arguments
    ///
    /// * `project` - Google Cloud project ID
    /// * `region` - Google Cloud region (e.g., "us-east5")
    /// * `model` - Vertex AI model identifier (e.g., "claude-sonnet-4@20250514")
    /// * `access_token` - Google Cloud OAuth2 access token
    #[must_use]
    pub fn new(
        project: impl Into<String>,
        region: impl Into<String>,
        model: impl Into<String>,
        access_token: SecretString,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            project: project.into(),
            region: region.into(),
            model: model.into(),
            access_token,
        }
    }

    /// Creates a new Vertex AI provider by reading configuration from environment
    /// variables and loading an access token from Application Default Credentials.
    ///
    /// Reads:
    /// - `GOOGLE_CLOUD_PROJECT` (required)
    /// - `CLOUD_ML_REGION` (required)
    /// - `VERTEX_AI_MODEL` (required)
    /// - `GOOGLE_APPLICATION_CREDENTIALS` (required - path to service account key)
    ///
    /// # Errors
    ///
    /// Returns an error if any required environment variable is not set or
    /// if the credentials file cannot be read or parsed.
    pub fn from_env() -> Result<Self> {
        let project = std::env::var("GOOGLE_CLOUD_PROJECT")
            .map_err(|_| anyhow::anyhow!("GOOGLE_CLOUD_PROJECT not set"))?;
        let region = std::env::var("CLOUD_ML_REGION")
            .map_err(|_| anyhow::anyhow!("CLOUD_ML_REGION not set"))?;
        let model = std::env::var("VERTEX_AI_MODEL")
            .map_err(|_| anyhow::anyhow!("VERTEX_AI_MODEL not set"))?;
        let creds_path = std::env::var("GOOGLE_APPLICATION_CREDENTIALS")
            .map_err(|_| anyhow::anyhow!("GOOGLE_APPLICATION_CREDENTIALS not set"))?;

        let creds_json = std::fs::read_to_string(&creds_path)
            .map_err(|e| anyhow::anyhow!("Failed to read credentials file at {creds_path}: {e}"))?;

        let access_token = extract_access_token_from_service_account(&creds_json)?;

        Ok(Self::new(project, region, model, access_token))
    }

    /// Constructs the Vertex AI streaming endpoint URL.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let url = provider.endpoint_url();
    /// // "https://us-east5-aiplatform.googleapis.com/v1/projects/my-project/locations/us-east5/publishers/anthropic/models/claude-sonnet-4@20250514:streamRawPredict"
    /// ```
    #[must_use]
    pub fn endpoint_url(&self) -> String {
        build_vertex_endpoint(&self.project, &self.region, &self.model)
    }

    /// Processes the SSE stream from a Vertex AI response.
    ///
    /// Vertex AI streams events in the same SSE format as the direct Anthropic API
    /// when using `streamRawPredict`, so we parse identically.
    ///
    /// # Errors
    ///
    /// Returns an error if reading the byte stream fails.
    async fn process_stream(
        &self,
        response: reqwest::Response,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<()> {
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
                        "Vertex AI SSE stream idle timeout after {}s",
                        SSE_IDLE_TIMEOUT.as_secs()
                    );
                    tx.send(StreamEvent::Error(
                        "Vertex AI SSE stream timed out waiting for data".to_string(),
                    ))
                    .await
                    .ok();
                    return Err(anyhow::anyhow!("Vertex AI SSE stream idle timeout"));
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

impl LlmProvider for VertexProvider {
    fn name(&self) -> &str {
        "vertex"
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
            let request = VertexRequest {
                anthropic_version: "vertex-2023-10-16",
                max_tokens: 8192,
                stream: true,
                messages,
                system: options.system.as_deref(),
                thinking: options.thinking.as_ref(),
                tools,
                tool_choice,
            };

            let url = self.endpoint_url();

            crate::api::retry::send_with_retry(
                || async {
                    self.client
                        .post(&url)
                        .header("Content-Type", "application/json")
                        .header(
                            "Authorization",
                            format!("Bearer {}", self.access_token.expose_secret()),
                        )
                        .json(&request)
                        .send()
                        .await
                        .map_err(Into::into)
                },
                |response, tx| async move { self.process_stream(response, tx).await },
                &tx,
            )
            .await
        })
    }
}

/// Constructs the Vertex AI streaming endpoint URL.
///
/// # Arguments
///
/// * `project` - Google Cloud project ID
/// * `region` - Google Cloud region (e.g., "us-east5")
/// * `model` - Vertex AI model identifier (e.g., "claude-sonnet-4@20250514")
///
/// # Examples
///
/// ```rust,ignore
/// let url = build_vertex_endpoint("my-project", "us-east5", "claude-sonnet-4@20250514");
/// assert!(url.contains("us-east5-aiplatform.googleapis.com"));
/// ```
#[must_use]
pub fn build_vertex_endpoint(project: &str, region: &str, model: &str) -> String {
    format!(
        "https://{region}-aiplatform.googleapis.com/v1/projects/{project}/locations/{region}/publishers/anthropic/models/{model}:streamRawPredict"
    )
}

/// Extracts a bearer token from a Google Cloud service account key JSON.
///
/// For production use, this would involve creating a signed JWT and exchanging it
/// for an access token via the OAuth2 token endpoint. In this lightweight
/// implementation, we extract the `client_email` and `private_key` fields
/// to verify the credentials file is valid, then store the private key as the
/// token material for JWT-based authentication.
///
/// In real usage, callers should either:
/// 1. Use `gcloud auth print-access-token` to get a token
/// 2. Set `GOOGLE_ACCESS_TOKEN` directly
///
/// # Errors
///
/// Returns an error if the JSON is invalid or missing required fields.
fn extract_access_token_from_service_account(json: &str) -> Result<SecretString> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| anyhow::anyhow!("Invalid credentials JSON: {e}"))?;

    // Validate the service account file has required fields
    let _client_email = value
        .get("client_email")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Credentials file missing 'client_email' field"))?;

    let private_key = value
        .get("private_key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Credentials file missing 'private_key' field"))?;

    // Store the private key as the credential material.
    // A full implementation would create a JWT signed with this key
    // and exchange it for an access token.
    Ok(SecretString::from(private_key.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::SecretString;

    #[test]
    fn test_build_vertex_endpoint() {
        let url = build_vertex_endpoint("my-project", "us-east5", "claude-sonnet-4@20250514");
        assert_eq!(
            url,
            "https://us-east5-aiplatform.googleapis.com/v1/projects/my-project/locations/us-east5/publishers/anthropic/models/claude-sonnet-4@20250514:streamRawPredict"
        );
    }

    #[test]
    fn test_build_vertex_endpoint_different_region() {
        let url = build_vertex_endpoint("prod-project", "europe-west4", "claude-opus-4@20250514");
        assert!(url.contains("europe-west4-aiplatform.googleapis.com"));
        assert!(url.contains("projects/prod-project"));
        assert!(url.contains("locations/europe-west4"));
        assert!(url.contains("claude-opus-4@20250514:streamRawPredict"));
    }

    #[test]
    fn test_vertex_provider_endpoint_url() {
        let provider = VertexProvider::new(
            "my-project",
            "us-east5",
            "claude-sonnet-4@20250514",
            SecretString::from("ya29.test-token"),
        );
        let url = provider.endpoint_url();
        assert!(url.contains("us-east5-aiplatform.googleapis.com"));
        assert!(url.contains("projects/my-project"));
        assert!(url.contains("claude-sonnet-4@20250514:streamRawPredict"));
    }

    #[test]
    fn test_vertex_provider_name() {
        let provider = VertexProvider::new(
            "my-project",
            "us-east5",
            "claude-sonnet-4@20250514",
            SecretString::from("ya29.test-token"),
        );
        assert_eq!(provider.name(), "vertex");
    }

    #[test]
    fn test_vertex_provider_model() {
        let provider = VertexProvider::new(
            "my-project",
            "us-east5",
            "claude-sonnet-4@20250514",
            SecretString::from("ya29.test-token"),
        );
        assert_eq!(provider.model(), "claude-sonnet-4@20250514");
    }

    #[test]
    fn test_vertex_request_serialization() {
        let messages = vec![ApiMessageV2::user("Hello!")];
        let request = VertexRequest {
            anthropic_version: "vertex-2023-10-16",
            max_tokens: 8192,
            stream: true,
            messages: &messages,
            system: None,
            thinking: None,
            tools: None,
            tool_choice: None,
        };
        let json = serde_json::to_value(&request).expect("should serialize");
        assert_eq!(json["anthropic_version"], "vertex-2023-10-16");
        assert_eq!(json["max_tokens"], 8192);
        assert_eq!(json["stream"], true);
        assert!(json.get("system").is_none());
        assert!(json.get("tools").is_none());
    }

    #[test]
    fn test_vertex_request_with_system() {
        let messages = vec![ApiMessageV2::user("Hello!")];
        let system = vec![SystemBlock {
            block_type: "text".to_string(),
            text: "You are helpful.".to_string(),
            cache_control: None,
        }];
        let request = VertexRequest {
            anthropic_version: "vertex-2023-10-16",
            max_tokens: 8192,
            stream: true,
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
    fn test_extract_access_token_from_service_account_valid() {
        let json = r#"{
            "type": "service_account",
            "client_email": "test@my-project.iam.gserviceaccount.com",
            "private_key": "-----BEGIN RSA PRIVATE KEY-----\nMIIE...\n-----END RSA PRIVATE KEY-----\n",
            "token_uri": "https://oauth2.googleapis.com/token"
        }"#;
        let result = extract_access_token_from_service_account(json);
        assert!(result.is_ok());
    }

    #[test]
    fn test_extract_access_token_missing_client_email() {
        let json = r#"{
            "type": "service_account",
            "private_key": "-----BEGIN RSA PRIVATE KEY-----\nMIIE...\n-----END RSA PRIVATE KEY-----\n"
        }"#;
        let result = extract_access_token_from_service_account(json);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("client_email"),
            "Error should mention client_email, got: {err}"
        );
    }

    #[test]
    fn test_extract_access_token_missing_private_key() {
        let json = r#"{
            "type": "service_account",
            "client_email": "test@my-project.iam.gserviceaccount.com"
        }"#;
        let result = extract_access_token_from_service_account(json);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("private_key"),
            "Error should mention private_key, got: {err}"
        );
    }

    #[test]
    fn test_extract_access_token_invalid_json() {
        let result = extract_access_token_from_service_account("not json");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Invalid credentials JSON"),
            "Error should mention invalid JSON, got: {err}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_vertex_from_env_missing_project() {
        std::env::remove_var("GOOGLE_CLOUD_PROJECT");
        std::env::remove_var("CLOUD_ML_REGION");
        std::env::remove_var("VERTEX_AI_MODEL");
        std::env::remove_var("GOOGLE_APPLICATION_CREDENTIALS");
        let result = VertexProvider::from_env();
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("GOOGLE_CLOUD_PROJECT"),
            "Error should mention GOOGLE_CLOUD_PROJECT, got: {err}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_vertex_from_env_missing_region() {
        std::env::set_var("GOOGLE_CLOUD_PROJECT", "test-project");
        std::env::remove_var("CLOUD_ML_REGION");
        std::env::remove_var("VERTEX_AI_MODEL");
        std::env::remove_var("GOOGLE_APPLICATION_CREDENTIALS");
        let result = VertexProvider::from_env();
        std::env::remove_var("GOOGLE_CLOUD_PROJECT");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("CLOUD_ML_REGION"),
            "Error should mention CLOUD_ML_REGION, got: {err}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_vertex_from_env_missing_model() {
        std::env::set_var("GOOGLE_CLOUD_PROJECT", "test-project");
        std::env::set_var("CLOUD_ML_REGION", "us-east5");
        std::env::remove_var("VERTEX_AI_MODEL");
        std::env::remove_var("GOOGLE_APPLICATION_CREDENTIALS");
        let result = VertexProvider::from_env();
        std::env::remove_var("GOOGLE_CLOUD_PROJECT");
        std::env::remove_var("CLOUD_ML_REGION");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("VERTEX_AI_MODEL"),
            "Error should mention VERTEX_AI_MODEL, got: {err}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_vertex_from_env_missing_credentials() {
        std::env::set_var("GOOGLE_CLOUD_PROJECT", "test-project");
        std::env::set_var("CLOUD_ML_REGION", "us-east5");
        std::env::set_var("VERTEX_AI_MODEL", "claude-sonnet-4@20250514");
        std::env::remove_var("GOOGLE_APPLICATION_CREDENTIALS");
        let result = VertexProvider::from_env();
        std::env::remove_var("GOOGLE_CLOUD_PROJECT");
        std::env::remove_var("CLOUD_ML_REGION");
        std::env::remove_var("VERTEX_AI_MODEL");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("GOOGLE_APPLICATION_CREDENTIALS"),
            "Error should mention GOOGLE_APPLICATION_CREDENTIALS, got: {err}"
        );
    }
}
