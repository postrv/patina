//! Tick handler for throbber animation and MCP event polling.
//!
//! [`TickHandler`] processes [`AppEvent::Tick`] events by:
//! 1. Advancing the throbber animation frame
//! 2. Draining pending MCP server events (tool list changes, progress, logs)

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;

use crate::app::context::AppContext;
use crate::app::dispatch::{EventHandler, Handled};
use crate::app::events::AppEvent;
use crate::app::state::DisplayState;

/// Handles periodic tick events for animation and background polling.
///
/// On each [`AppEvent::Tick`]:
/// 1. Advances the throbber animation frame
/// 2. Drains MCP events from all connected servers, logging them
///
/// # Examples
///
/// ```rust,ignore
/// use patina::app::handlers::tick::TickHandler;
/// use patina::app::dispatch::{EventDispatcher, EventHandler};
///
/// let handler = TickHandler;
/// let dispatcher = EventDispatcher::new(vec![Box::new(handler)]);
/// ```
pub struct TickHandler;

impl TickHandler {
    /// Advances the throbber animation.
    ///
    /// The display state tracks its own dirty flag.
    fn handle_tick(display: &mut DisplayState) {
        display.tick_throbber();
    }
}

impl EventHandler for TickHandler {
    fn handle<'a>(
        &'a mut self,
        event: &'a AppEvent,
        ctx: &'a mut AppContext<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<Handled>> + Send + 'a>> {
        Box::pin(async move {
            if matches!(event, AppEvent::Tick) {
                Self::handle_tick(ctx.state.display_mut());
                process_mcp_events(ctx);
                Ok(Handled::CONSUMED)
            } else {
                Ok(Handled::IGNORED)
            }
        })
    }

    fn name(&self) -> &str {
        "tick"
    }
}

/// Drains and processes pending MCP events from all connected servers.
///
/// Handles each event type:
/// - `ToolListChanged` — logged; tools are already refreshed by the handler
/// - `ResourceListChanged` / `PromptListChanged` — logged for awareness
/// - `ProgressUpdate` — logged at debug level
/// - `LogMessage` — forwarded to tracing at the appropriate level
fn process_mcp_events(ctx: &mut AppContext<'_>) {
    let Some(manager) = ctx.state.mcp_manager() else {
        return;
    };

    let events = manager.drain_all_events();
    if events.is_empty() {
        return;
    }

    for event in events {
        match event {
            crate::mcp::handler::McpEvent::ToolListChanged { server_name } => {
                tracing::info!(server = %server_name, "MCP server tool list changed");
            }
            crate::mcp::handler::McpEvent::ResourceListChanged { server_name } => {
                tracing::debug!(server = %server_name, "MCP server resource list changed");
            }
            crate::mcp::handler::McpEvent::PromptListChanged { server_name } => {
                tracing::debug!(server = %server_name, "MCP server prompt list changed");
            }
            crate::mcp::handler::McpEvent::ProgressUpdate {
                server_name,
                progress,
                total,
                message,
            } => {
                tracing::debug!(
                    server = %server_name,
                    progress,
                    ?total,
                    ?message,
                    "MCP server progress"
                );
            }
            crate::mcp::handler::McpEvent::LogMessage {
                server_name,
                level,
                data,
            } => {
                tracing::debug!(
                    server = %server_name,
                    level = %level,
                    "MCP server log: {}",
                    data
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{AnthropicClient, LlmProvider};
    use crate::app::state::AppState;
    use crate::session::SessionManager;
    use crate::types::config::ParallelMode;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use secrecy::SecretString;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::TempDir;

    // =========================================================================
    // Test helpers
    // =========================================================================

    fn test_client() -> Arc<dyn LlmProvider> {
        Arc::new(AnthropicClient::new(
            SecretString::from("test-key"),
            "claude-test",
        ))
    }

    fn test_state() -> AppState {
        AppState::new(PathBuf::from("/tmp/test"), true, ParallelMode::Disabled)
    }

    fn test_session_manager() -> (SessionManager, TempDir) {
        let dir = TempDir::new().expect("failed to create temp dir");
        let mgr = SessionManager::new(dir.path().to_path_buf());
        (mgr, dir)
    }

    // =========================================================================
    // TickHandler::handle tests
    // =========================================================================

    #[tokio::test]
    async fn handle_tick_returns_consumed() {
        let mut handler = TickHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Tick;
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "TickHandler must consume Tick events"
        );
    }

    #[tokio::test]
    async fn handle_tick_advances_throbber_frame() {
        let mut handler = TickHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // Record initial throbber character (frame 0 = '⠋').
        let before = state.throbber_char();

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Tick;
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        // After one tick, throbber should advance to frame 1 = '⠙'.
        assert_ne!(
            ctx.state.throbber_char(),
            before,
            "tick_throbber() must advance the throbber frame"
        );
        assert_eq!(ctx.state.throbber_char(), '⠙');
    }

    #[tokio::test]
    async fn handle_tick_marks_dirty() {
        let mut handler = TickHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // Clear any initial dirty flags.
        state.mark_rendered();

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Tick;
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(
            ctx.needs_render(),
            "tick_throbber() must mark the display dirty"
        );
    }

    #[tokio::test]
    async fn handle_key_returns_ignored() {
        let mut handler = TickHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::IGNORED,
            "TickHandler must ignore Key events"
        );
    }

    #[tokio::test]
    async fn handle_quit_returns_ignored() {
        let mut handler = TickHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Quit;
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::IGNORED,
            "TickHandler must ignore Quit events"
        );
    }

    #[tokio::test]
    async fn handle_resize_returns_ignored() {
        let mut handler = TickHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Resize {
            width: 80,
            height: 24,
        };
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::IGNORED,
            "TickHandler must ignore Resize events"
        );
    }

    #[tokio::test]
    async fn handle_does_not_modify_state_for_non_tick() {
        let mut handler = TickHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        let before = state.throbber_char();
        state.mark_rendered();

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        let _ = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            ctx.state.throbber_char(),
            before,
            "non-Tick events must not advance the throbber"
        );
    }

    // =========================================================================
    // TickHandler::name test
    // =========================================================================

    #[test]
    fn name_returns_tick() {
        let handler = TickHandler;
        assert_eq!(handler.name(), "tick");
    }

    // =========================================================================
    // Send bound verification
    // =========================================================================

    #[test]
    fn tick_handler_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<TickHandler>();
    }

    // =========================================================================
    // Integration with EventDispatcher
    // =========================================================================

    #[tokio::test]
    async fn tick_handler_works_in_dispatcher() {
        use crate::app::dispatch::EventDispatcher;

        let handler = TickHandler;
        let mut dispatcher = EventDispatcher::new(vec![Box::new(handler)]);

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        // Tick event should be consumed.
        let result = dispatcher
            .dispatch(&AppEvent::Tick, &mut ctx)
            .await
            .unwrap();
        assert_eq!(result, Handled::CONSUMED);

        // Key event should pass through.
        let key = AppEvent::Key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        let result = dispatcher.dispatch(&key, &mut ctx).await.unwrap();
        assert_eq!(result, Handled::IGNORED);
    }

    // =========================================================================
    // Narrow tests: handle_tick with DisplayState only (no AppState)
    // =========================================================================

    #[test]
    fn handle_tick_advances_throbber_on_display_state() {
        let mut display = DisplayState::new();
        assert_eq!(display.throbber_char(), '\u{280B}'); // ⠋

        TickHandler::handle_tick(&mut display);

        assert_eq!(display.throbber_char(), '\u{2819}'); // ⠙
    }

    #[test]
    fn handle_tick_sets_dirty_flag_on_display_state() {
        let mut display = DisplayState::new();
        display.mark_clean();
        assert!(!display.is_dirty());

        TickHandler::handle_tick(&mut display);

        assert!(display.is_dirty());
    }
}
