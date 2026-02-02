//! IDE command handlers for processing IDE requests
//!
//! This module provides handler functions for each [`IdeRequest`] variant.
//! Handlers are designed to be testable by accepting minimal required context
//! rather than full application state.
//!
//! # Example
//!
//! ```ignore
//! use patina::ide::handlers::{handle_ping, handle_get_status, StatusContext};
//!
//! let pong = handle_ping();
//! let status = handle_get_status(&StatusContext {
//!     busy: false,
//!     turn_count: 0,
//!     active_tools: vec![],
//! });
//! ```

use super::protocol::{IdeRequest, IdeResponse};
use std::collections::HashSet;
use std::path::PathBuf;
use tokio::sync::mpsc;
use uuid::Uuid;

/// Context required for status queries
#[derive(Debug, Clone)]
pub struct StatusContext {
    /// Whether the application is currently processing a request
    pub busy: bool,
    /// Number of conversation turns
    pub turn_count: u32,
    /// Names of currently executing tools
    pub active_tools: Vec<String>,
}

/// Context required for queuing prompts
#[derive(Debug)]
pub struct PromptContext {
    /// Channel to send prompts to the main application
    pub prompt_tx: mpsc::UnboundedSender<QueuedPrompt>,
}

/// A prompt queued for processing
#[derive(Debug, Clone)]
pub struct QueuedPrompt {
    /// Unique request identifier
    pub request_id: String,
    /// The prompt text
    pub text: String,
    /// Optional file context
    pub file: Option<PathBuf>,
    /// Optional text selection
    pub selection: Option<super::protocol::TextSelection>,
}

/// Handle a ping request - responds with version info
///
/// # Returns
///
/// Returns a [`IdeResponse::Pong`] with the current Patina version.
#[must_use]
pub fn handle_ping() -> IdeResponse {
    IdeResponse::Pong {
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

/// Handle a get_status request - returns current application status
///
/// # Arguments
///
/// * `ctx` - Status context containing busy state, turn count, and active tools
///
/// # Returns
///
/// Returns a [`IdeResponse::Status`] with the current application state.
#[must_use]
pub fn handle_get_status(ctx: &StatusContext) -> IdeResponse {
    IdeResponse::Status {
        busy: ctx.busy,
        turn_count: ctx.turn_count,
        active_tools: ctx.active_tools.clone(),
    }
}

/// Handle a send_prompt request - queues prompt for processing
///
/// # Arguments
///
/// * `ctx` - Prompt context containing the channel to queue prompts
/// * `text` - The prompt text
/// * `file` - Optional file context
/// * `selection` - Optional text selection
///
/// # Returns
///
/// Returns [`IdeResponse::PromptReceived`] on success, or [`IdeResponse::Error`] on failure.
pub fn handle_send_prompt(
    ctx: &PromptContext,
    text: String,
    file: Option<PathBuf>,
    selection: Option<super::protocol::TextSelection>,
) -> IdeResponse {
    let request_id = Uuid::new_v4().to_string();

    let prompt = QueuedPrompt {
        request_id: request_id.clone(),
        text,
        file,
        selection,
    };

    match ctx.prompt_tx.send(prompt) {
        Ok(()) => IdeResponse::PromptReceived { request_id },
        Err(_) => IdeResponse::Error {
            code: "QUEUE_FULL".to_string(),
            message: "Failed to queue prompt - channel closed".to_string(),
            request_id: None,
        },
    }
}

/// Handle a cancel request - attempts to cancel an in-progress operation
///
/// # Arguments
///
/// * `request_id` - The request ID to cancel
/// * `pending_requests` - Set of currently pending request IDs
///
/// # Returns
///
/// Returns [`IdeResponse::Cancelled`] if request was found and cancelled,
/// or [`IdeResponse::Error`] if not found.
pub fn handle_cancel(request_id: &str, pending_requests: &HashSet<String>) -> IdeResponse {
    if pending_requests.contains(request_id) {
        IdeResponse::Cancelled {
            request_id: request_id.to_string(),
        }
    } else {
        IdeResponse::Error {
            code: "NOT_FOUND".to_string(),
            message: format!("Request '{}' not found or already completed", request_id),
            request_id: Some(request_id.to_string()),
        }
    }
}

/// Handle an init request - registers a new IDE session
///
/// # Arguments
///
/// * `workspace` - The workspace root directory
/// * `capabilities` - Client capabilities
/// * `session_id` - Assigned session ID
///
/// # Returns
///
/// Returns [`IdeResponse::InitAck`] with server capabilities.
#[must_use]
pub fn handle_init(
    workspace: &PathBuf,
    capabilities: &[String],
    session_id: &str,
) -> IdeResponse {
    tracing::info!(
        "IDE session initialized: workspace={:?}, capabilities={:?}",
        workspace,
        capabilities
    );

    IdeResponse::InitAck {
        session_id: session_id.to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        capabilities: super::protocol::default_capabilities(),
    }
}

/// Route an incoming request to the appropriate handler
///
/// # Arguments
///
/// * `request` - The incoming IDE request
/// * `status_ctx` - Context for status queries
/// * `prompt_ctx` - Context for prompt handling
/// * `pending_requests` - Set of pending request IDs for cancellation
/// * `session_id` - Session ID for init acknowledgment
///
/// # Returns
///
/// Returns the appropriate [`IdeResponse`] for the request.
pub fn route_request(
    request: IdeRequest,
    status_ctx: &StatusContext,
    prompt_ctx: &PromptContext,
    pending_requests: &HashSet<String>,
    session_id: &str,
) -> IdeResponse {
    match request {
        IdeRequest::Ping => handle_ping(),
        IdeRequest::GetStatus => handle_get_status(status_ctx),
        IdeRequest::SendPrompt {
            text,
            file,
            selection,
        } => handle_send_prompt(prompt_ctx, text, file, selection),
        IdeRequest::Cancel { request_id } => handle_cancel(&request_id, pending_requests),
        IdeRequest::Init {
            workspace,
            capabilities,
        } => handle_init(&workspace, &capabilities, session_id),
        IdeRequest::ApplyEdit { file, diff } => {
            // Edit application is handled separately through the tool system
            tracing::info!("Apply edit request: {:?} with diff length {}", file, diff.len());
            IdeResponse::Error {
                code: "NOT_IMPLEMENTED".to_string(),
                message: "Edit application through IDE protocol is not yet implemented".to_string(),
                request_id: None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // handle_ping tests
    // =========================================================================

    #[test]
    fn test_handle_ping_returns_pong() {
        let response = handle_ping();
        match response {
            IdeResponse::Pong { version } => {
                assert!(!version.is_empty());
                // Version should be semver-ish
                assert!(version.contains('.'));
            }
            _ => panic!("Expected Pong response"),
        }
    }

    #[test]
    fn test_handle_ping_version_matches_cargo() {
        let response = handle_ping();
        match response {
            IdeResponse::Pong { version } => {
                assert_eq!(version, env!("CARGO_PKG_VERSION"));
            }
            _ => panic!("Expected Pong response"),
        }
    }

    // =========================================================================
    // handle_get_status tests
    // =========================================================================

    #[test]
    fn test_handle_get_status_idle() {
        let ctx = StatusContext {
            busy: false,
            turn_count: 0,
            active_tools: vec![],
        };
        let response = handle_get_status(&ctx);
        match response {
            IdeResponse::Status {
                busy,
                turn_count,
                active_tools,
            } => {
                assert!(!busy);
                assert_eq!(turn_count, 0);
                assert!(active_tools.is_empty());
            }
            _ => panic!("Expected Status response"),
        }
    }

    #[test]
    fn test_handle_get_status_busy() {
        let ctx = StatusContext {
            busy: true,
            turn_count: 5,
            active_tools: vec!["bash".to_string(), "read_file".to_string()],
        };
        let response = handle_get_status(&ctx);
        match response {
            IdeResponse::Status {
                busy,
                turn_count,
                active_tools,
            } => {
                assert!(busy);
                assert_eq!(turn_count, 5);
                assert_eq!(active_tools.len(), 2);
                assert!(active_tools.contains(&"bash".to_string()));
            }
            _ => panic!("Expected Status response"),
        }
    }

    // =========================================================================
    // handle_send_prompt tests
    // =========================================================================

    #[test]
    fn test_handle_send_prompt_success() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let ctx = PromptContext { prompt_tx: tx };

        let response = handle_send_prompt(&ctx, "Hello".to_string(), None, None);

        match response {
            IdeResponse::PromptReceived { request_id } => {
                assert!(!request_id.is_empty());
                // Should be valid UUID
                assert!(Uuid::parse_str(&request_id).is_ok());
            }
            _ => panic!("Expected PromptReceived response"),
        }

        // Verify prompt was queued
        let queued = rx.try_recv().expect("Should have queued prompt");
        assert_eq!(queued.text, "Hello");
        assert!(queued.file.is_none());
    }

    #[test]
    fn test_handle_send_prompt_with_file() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let ctx = PromptContext { prompt_tx: tx };

        let response = handle_send_prompt(
            &ctx,
            "Explain this".to_string(),
            Some(PathBuf::from("src/main.rs")),
            None,
        );

        match response {
            IdeResponse::PromptReceived { .. } => {}
            _ => panic!("Expected PromptReceived response"),
        }

        let queued = rx.try_recv().expect("Should have queued prompt");
        assert_eq!(queued.file, Some(PathBuf::from("src/main.rs")));
    }

    #[test]
    fn test_handle_send_prompt_channel_closed() {
        let (tx, rx) = mpsc::unbounded_channel::<QueuedPrompt>();
        drop(rx); // Close the receiver
        let ctx = PromptContext { prompt_tx: tx };

        let response = handle_send_prompt(&ctx, "Hello".to_string(), None, None);

        match response {
            IdeResponse::Error { code, .. } => {
                assert_eq!(code, "QUEUE_FULL");
            }
            _ => panic!("Expected Error response"),
        }
    }

    // =========================================================================
    // handle_cancel tests
    // =========================================================================

    #[test]
    fn test_handle_cancel_found() {
        let mut pending = HashSet::new();
        pending.insert("req-123".to_string());

        let response = handle_cancel("req-123", &pending);

        match response {
            IdeResponse::Cancelled { request_id } => {
                assert_eq!(request_id, "req-123");
            }
            _ => panic!("Expected Cancelled response"),
        }
    }

    #[test]
    fn test_handle_cancel_not_found() {
        let pending = HashSet::new();

        let response = handle_cancel("req-999", &pending);

        match response {
            IdeResponse::Error {
                code, request_id, ..
            } => {
                assert_eq!(code, "NOT_FOUND");
                assert_eq!(request_id, Some("req-999".to_string()));
            }
            _ => panic!("Expected Error response"),
        }
    }

    // =========================================================================
    // handle_init tests
    // =========================================================================

    #[test]
    fn test_handle_init_returns_ack() {
        let workspace = PathBuf::from("/home/user/project");
        let capabilities = vec!["streaming".to_string()];
        let session_id = "sess-001";

        let response = handle_init(&workspace, &capabilities, session_id);

        match response {
            IdeResponse::InitAck {
                session_id: sid,
                version,
                capabilities: caps,
            } => {
                assert_eq!(sid, "sess-001");
                assert!(!version.is_empty());
                assert!(!caps.is_empty());
                assert!(caps.contains(&"streaming".to_string()));
            }
            _ => panic!("Expected InitAck response"),
        }
    }

    // =========================================================================
    // route_request tests
    // =========================================================================

    #[test]
    fn test_route_ping_request() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let status_ctx = StatusContext {
            busy: false,
            turn_count: 0,
            active_tools: vec![],
        };
        let prompt_ctx = PromptContext { prompt_tx: tx };
        let pending = HashSet::new();

        let response = route_request(
            IdeRequest::Ping,
            &status_ctx,
            &prompt_ctx,
            &pending,
            "sess-001",
        );

        assert!(matches!(response, IdeResponse::Pong { .. }));
    }

    #[test]
    fn test_route_status_request() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let status_ctx = StatusContext {
            busy: true,
            turn_count: 3,
            active_tools: vec!["bash".to_string()],
        };
        let prompt_ctx = PromptContext { prompt_tx: tx };
        let pending = HashSet::new();

        let response = route_request(
            IdeRequest::GetStatus,
            &status_ctx,
            &prompt_ctx,
            &pending,
            "sess-001",
        );

        match response {
            IdeResponse::Status { busy, .. } => assert!(busy),
            _ => panic!("Expected Status response"),
        }
    }

    #[test]
    fn test_route_apply_edit_not_implemented() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let status_ctx = StatusContext {
            busy: false,
            turn_count: 0,
            active_tools: vec![],
        };
        let prompt_ctx = PromptContext { prompt_tx: tx };
        let pending = HashSet::new();

        let response = route_request(
            IdeRequest::ApplyEdit {
                file: PathBuf::from("test.rs"),
                diff: "-old\n+new".to_string(),
            },
            &status_ctx,
            &prompt_ctx,
            &pending,
            "sess-001",
        );

        match response {
            IdeResponse::Error { code, .. } => {
                assert_eq!(code, "NOT_IMPLEMENTED");
            }
            _ => panic!("Expected Error response"),
        }
    }
}
