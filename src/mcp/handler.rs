//! MCP client handler for server notifications.
//!
//! Implements `rmcp::handler::client::ClientHandler` to handle notifications
//! from MCP servers, including tool list changes, progress updates, and log messages.
//!
//! # Architecture
//!
//! `PatinaClientHandler` is constructed per-connection with:
//! - A shared `Arc<RwLock<Vec<Tool>>>` for the tool list (updated on `on_tool_list_changed`)
//! - An `mpsc::UnboundedSender<McpEvent>` for forwarding events to the manager
//!
//! The handler is `Send + Sync + 'static` as required by the SDK.

use std::future::Future;

use rmcp::handler::client::ClientHandler;
use rmcp::model::{
    ClientCapabilities, ClientInfo, Implementation, LoggingMessageNotificationParam,
    ProgressNotificationParam, Tool,
};
use rmcp::service::{NotificationContext, RoleClient};
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

/// Events emitted by MCP servers via notifications.
///
/// These events are collected by `McpConnection::drain_events()` and can be
/// polled by the manager or surfaced to the TUI.
#[derive(Debug, Clone)]
pub enum McpEvent {
    /// Server's tool list changed; tools have been refreshed.
    ToolListChanged {
        /// Name of the server whose tools changed.
        server_name: String,
    },
    /// Server reported progress on an operation.
    ProgressUpdate {
        /// Name of the server.
        server_name: String,
        /// Current progress value.
        progress: f64,
        /// Total expected progress, if known.
        total: Option<f64>,
        /// Human-readable progress message.
        message: Option<String>,
    },
    /// Server emitted a log message.
    LogMessage {
        /// Name of the server.
        server_name: String,
        /// Log level (debug, info, warning, error, etc.).
        level: String,
        /// Log message data.
        data: String,
    },
    /// Server's resource list changed.
    ResourceListChanged {
        /// Name of the server.
        server_name: String,
    },
    /// Server's prompt list changed.
    PromptListChanged {
        /// Name of the server.
        server_name: String,
    },
}

/// Patina's implementation of the rmcp `ClientHandler` trait.
///
/// Handles server notifications by updating shared state and forwarding
/// events through an mpsc channel.
///
/// # Thread Safety
///
/// This handler is `Send + Sync + 'static`. The tool list uses `std::sync::RwLock`
/// (not `tokio::sync::RwLock`) for synchronous reads from non-async contexts.
pub struct PatinaClientHandler {
    /// Name of the MCP server this handler is attached to.
    server_name: String,
    /// Shared tool list, updated when `on_tool_list_changed` fires.
    tools: Arc<RwLock<Vec<Tool>>>,
    /// Channel for forwarding events to the manager.
    event_tx: mpsc::Sender<McpEvent>,
}

impl PatinaClientHandler {
    /// Creates a new handler for the given server.
    ///
    /// # Arguments
    ///
    /// * `server_name` - Identifier for the MCP server
    /// * `tools` - Shared tool list that will be updated on notifications
    /// * `event_tx` - Channel sender for forwarding events
    #[must_use]
    pub fn new(
        server_name: String,
        tools: Arc<RwLock<Vec<Tool>>>,
        event_tx: mpsc::Sender<McpEvent>,
    ) -> Self {
        Self {
            server_name,
            tools,
            event_tx,
        }
    }
}

impl ClientHandler for PatinaClientHandler {
    fn get_info(&self) -> ClientInfo {
        ClientInfo {
            protocol_version: Default::default(),
            capabilities: ClientCapabilities::default(),
            client_info: Implementation {
                name: "patina".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                title: None,
                description: None,
                icons: None,
                website_url: None,
            },
            meta: None,
        }
    }

    fn on_tool_list_changed(
        &self,
        context: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + Send + '_ {
        let server_name = self.server_name.clone();
        let tools = Arc::clone(&self.tools);
        let event_tx = self.event_tx.clone();

        async move {
            // Refresh tool list from the server
            match context.peer.list_all_tools().await {
                Ok(new_tools) => {
                    if let Ok(mut guard) = tools.write() {
                        *guard = new_tools;
                    }
                    tracing::debug!(
                        server = %server_name,
                        "Tool list refreshed via notification"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        server = %server_name,
                        error = %e,
                        "Failed to refresh tool list after notification"
                    );
                }
            }

            if event_tx
                .try_send(McpEvent::ToolListChanged { server_name })
                .is_err()
            {
                tracing::warn!("MCP event channel full, dropping ToolListChanged notification");
            }
        }
    }

    fn on_progress(
        &self,
        params: ProgressNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + Send + '_ {
        let event = McpEvent::ProgressUpdate {
            server_name: self.server_name.clone(),
            progress: params.progress,
            total: params.total,
            message: params.message,
        };
        if self.event_tx.try_send(event).is_err() {
            tracing::warn!("MCP event channel full, dropping ProgressUpdate notification");
        }
        std::future::ready(())
    }

    fn on_logging_message(
        &self,
        params: LoggingMessageNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + Send + '_ {
        let level_str = format!("{:?}", params.level);
        let data_str = params.data.to_string();

        tracing::debug!(
            server = %self.server_name,
            level = %level_str,
            "MCP server log: {}",
            data_str
        );

        let event = McpEvent::LogMessage {
            server_name: self.server_name.clone(),
            level: level_str,
            data: data_str,
        };
        if self.event_tx.try_send(event).is_err() {
            tracing::warn!("MCP event channel full, dropping LogMessage notification");
        }
        std::future::ready(())
    }

    fn on_resource_list_changed(
        &self,
        _context: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + Send + '_ {
        if self
            .event_tx
            .try_send(McpEvent::ResourceListChanged {
                server_name: self.server_name.clone(),
            })
            .is_err()
        {
            tracing::warn!("MCP event channel full, dropping ResourceListChanged notification");
        }
        std::future::ready(())
    }

    fn on_prompt_list_changed(
        &self,
        _context: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + Send + '_ {
        if self
            .event_tx
            .try_send(McpEvent::PromptListChanged {
                server_name: self.server_name.clone(),
            })
            .is_err()
        {
            tracing::warn!("MCP event channel full, dropping PromptListChanged notification");
        }
        std::future::ready(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_event_variants() {
        // Verify all enum variants can be constructed
        let _ = McpEvent::ToolListChanged {
            server_name: "test".to_string(),
        };
        let _ = McpEvent::ProgressUpdate {
            server_name: "test".to_string(),
            progress: 0.5,
            total: Some(1.0),
            message: Some("halfway".to_string()),
        };
        let _ = McpEvent::LogMessage {
            server_name: "test".to_string(),
            level: "Info".to_string(),
            data: "hello".to_string(),
        };
        let _ = McpEvent::ResourceListChanged {
            server_name: "test".to_string(),
        };
        let _ = McpEvent::PromptListChanged {
            server_name: "test".to_string(),
        };
    }

    #[test]
    fn test_handler_get_info() {
        let (tx, _rx) = mpsc::channel(256);
        let tools = Arc::new(RwLock::new(Vec::new()));
        let handler = PatinaClientHandler::new("test-server".to_string(), tools, tx);

        let info = ClientHandler::get_info(&handler);
        assert_eq!(info.client_info.name, "patina");
        assert!(!info.client_info.version.is_empty());
    }

    #[test]
    fn test_handler_construction() {
        let (tx, _rx) = mpsc::channel(256);
        let tools = Arc::new(RwLock::new(Vec::new()));
        let handler = PatinaClientHandler::new("my-server".to_string(), tools.clone(), tx);

        assert_eq!(handler.server_name, "my-server");
        assert!(tools.read().unwrap().is_empty());
    }

    #[test]
    fn test_handler_is_send_sync() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<PatinaClientHandler>();
    }

    #[test]
    fn test_mcp_event_clone() {
        let event = McpEvent::ProgressUpdate {
            server_name: "s".to_string(),
            progress: 1.0,
            total: None,
            message: None,
        };
        let _cloned = event.clone();
    }
}
