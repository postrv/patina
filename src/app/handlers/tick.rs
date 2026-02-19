//! Tick handler for throbber animation.
//!
//! [`TickHandler`] processes [`AppEvent::Tick`] events by advancing the
//! throbber animation frame. This is the simplest handler — one line of
//! logic — extracted from the event loop to validate the handler pattern.

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;

use crate::app::context::AppContext;
use crate::app::dispatch::{EventHandler, Handled};
use crate::app::events::AppEvent;

/// Handles periodic tick events for the throbber animation.
///
/// When the application is loading or executing tools, a periodic timer
/// fires [`AppEvent::Tick`]. This handler advances the throbber animation
/// frame via [`AppState::tick_throbber`](crate::app::state::AppState::tick_throbber).
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

impl EventHandler for TickHandler {
    fn handle<'a>(
        &'a mut self,
        event: &'a AppEvent,
        ctx: &'a mut AppContext<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<Handled>> + Send + 'a>> {
        Box::pin(async move {
            if matches!(event, AppEvent::Tick) {
                ctx.state.tick_throbber();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{AnthropicClient, LlmProvider};
    use crate::app::state::AppState;
    use crate::session::SessionManager;
    use crate::types::config::ParallelMode;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::CrosstermBackend;
    use ratatui::Terminal;
    use secrecy::SecretString;
    use std::io;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::TempDir;

    // =========================================================================
    // Test helpers
    // =========================================================================

    fn test_terminal() -> Terminal<CrosstermBackend<io::Stdout>> {
        let backend = CrosstermBackend::new(io::stdout());
        Terminal::new(backend).expect("failed to create test terminal")
    }

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
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(&mut terminal, Arc::clone(&client), &mut state, &session_mgr);

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
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // Record initial throbber character (frame 0 = '⠋').
        let before = state.throbber_char();

        let mut ctx = AppContext::new(&mut terminal, Arc::clone(&client), &mut state, &session_mgr);
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
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // Clear any initial dirty flags.
        state.mark_rendered();

        let mut ctx = AppContext::new(&mut terminal, Arc::clone(&client), &mut state, &session_mgr);
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
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(&mut terminal, Arc::clone(&client), &mut state, &session_mgr);

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
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(&mut terminal, Arc::clone(&client), &mut state, &session_mgr);

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
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(&mut terminal, Arc::clone(&client), &mut state, &session_mgr);

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
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        let before = state.throbber_char();
        state.mark_rendered();

        let mut ctx = AppContext::new(&mut terminal, Arc::clone(&client), &mut state, &session_mgr);
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
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(&mut terminal, Arc::clone(&client), &mut state, &session_mgr);

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
}
