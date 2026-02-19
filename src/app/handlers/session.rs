//! Session auto-save handler.
//!
//! [`SessionHandler`] observes events and persists the session when the
//! `session_dirty` flag is set on [`AppState`](crate::app::state::AppState),
//! or unconditionally on [`AppEvent::Quit`]. It always returns
//! [`Handled::IGNORED`] so that other handlers can still process the event.

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use tracing::{debug, warn};

use crate::app::context::AppContext;
use crate::app::dispatch::{EventHandler, Handled};
use crate::app::events::AppEvent;
use crate::session::SessionManager;

/// Handles session auto-save by observing events and persisting when needed.
///
/// `SessionHandler` is designed to run last in the dispatcher chain. It never
/// consumes events — it observes them and performs side effects (session saves)
/// when the application state has been marked dirty by other handlers.
///
/// # Save triggers
///
/// - **Dirty flag**: When [`AppState::take_session_dirty`] returns `true`,
///   the handler saves the session. Other handlers set this flag after
///   modifying conversation state.
/// - **Quit event**: On [`AppEvent::Quit`], the handler always saves
///   regardless of the dirty flag, as a final checkpoint before shutdown.
///
/// # Error handling
///
/// Save errors are logged at `warn` level but do not propagate. The handler
/// always returns `Ok(Handled::IGNORED)` to avoid crashing the application
/// due to transient I/O failures.
///
/// # Examples
///
/// ```rust,ignore
/// use patina::app::handlers::session::SessionHandler;
/// use patina::app::dispatch::EventDispatcher;
///
/// let dispatcher = EventDispatcher::new(vec![
///     Box::new(tick_handler),
///     Box::new(SessionHandler), // last in chain
/// ]);
/// ```
pub struct SessionHandler;

impl EventHandler for SessionHandler {
    fn handle<'a>(
        &'a mut self,
        event: &'a AppEvent,
        ctx: &'a mut AppContext<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<Handled>> + Send + 'a>> {
        Box::pin(async move {
            // Stub: always ignore — tests will define the expected behavior.
            let _ = event;
            let _ = ctx;
            Ok(Handled::IGNORED)
        })
    }

    fn name(&self) -> &str {
        "session"
    }
}

/// Performs the auto-save, creating or updating the session as appropriate.
///
/// # Errors
///
/// Returns `Ok(())` even on save failure — errors are logged but not propagated.
async fn auto_save(state: &mut crate::app::state::AppState, session_manager: &SessionManager) {
    let session = state.to_session();

    let result = if let Some(existing_id) = state.session_id() {
        session_manager
            .update(existing_id, &session)
            .await
            .map(|()| existing_id.to_string())
    } else {
        session_manager.save(&session).await
    };

    match result {
        Ok(id) => {
            if state.session_id().is_none() {
                debug!(session_id = %id, "Created new session");
                state.set_session_id(id);
            } else {
                debug!(session_id = %id, "Updated session");
            }
        }
        Err(e) => {
            warn!(error = %e, "Failed to auto-save session");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::AnthropicClient;
    use crate::app::state::AppState;
    use crate::session::SessionManager;
    use crate::types::config::ParallelMode;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::CrosstermBackend;
    use ratatui::Terminal;
    use secrecy::SecretString;
    use std::io;
    use std::path::PathBuf;
    use tempfile::TempDir;

    // =========================================================================
    // Test helpers
    // =========================================================================

    fn test_terminal() -> Terminal<CrosstermBackend<io::Stdout>> {
        let backend = CrosstermBackend::new(io::stdout());
        Terminal::new(backend).expect("failed to create test terminal")
    }

    fn test_client() -> AnthropicClient {
        AnthropicClient::new(SecretString::from("test-key"), "claude-test")
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
    // SessionHandler::name
    // =========================================================================

    #[test]
    fn name_returns_session() {
        let handler = SessionHandler;
        assert_eq!(handler.name(), "session");
    }

    // =========================================================================
    // SessionHandler::handle — dirty flag triggers save
    // =========================================================================

    #[tokio::test]
    async fn handle_saves_when_session_dirty() {
        let mut handler = SessionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // Mark the session as needing a save.
        state.mark_session_dirty();

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = AppEvent::Tick;
        let _result = handler.handle(&event, &mut ctx).await.unwrap();

        // After handling, a new session should have been created.
        assert!(
            ctx.state.session_id().is_some(),
            "SessionHandler must create a session when dirty flag is set"
        );
    }

    #[tokio::test]
    async fn handle_clears_dirty_flag_after_save() {
        let mut handler = SessionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.mark_session_dirty();

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = AppEvent::Tick;
        let _result = handler.handle(&event, &mut ctx).await.unwrap();

        // The dirty flag should have been consumed.
        assert!(
            !ctx.state.take_session_dirty(),
            "SessionHandler must clear the dirty flag after saving"
        );
    }

    #[tokio::test]
    async fn handle_does_not_save_when_not_dirty() {
        let mut handler = SessionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // No dirty flag set — session should NOT be saved.
        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = AppEvent::Tick;
        let _result = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(
            ctx.state.session_id().is_none(),
            "SessionHandler must not save when session is not dirty"
        );
    }

    // =========================================================================
    // SessionHandler::handle — Quit event triggers unconditional save
    // =========================================================================

    #[tokio::test]
    async fn handle_saves_on_quit_even_when_not_dirty() {
        let mut handler = SessionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // Not dirty, but Quit should still trigger a save.
        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = AppEvent::Quit;
        let _result = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(
            ctx.state.session_id().is_some(),
            "SessionHandler must save on Quit even when not dirty"
        );
    }

    #[tokio::test]
    async fn handle_returns_ignored_on_quit() {
        let mut handler = SessionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = AppEvent::Quit;
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::IGNORED,
            "SessionHandler must return IGNORED for Quit so the event propagates"
        );
    }

    // =========================================================================
    // SessionHandler::handle — always returns IGNORED
    // =========================================================================

    #[tokio::test]
    async fn handle_returns_ignored_for_tick() {
        let mut handler = SessionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = AppEvent::Tick;
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::IGNORED,
            "SessionHandler must always return IGNORED"
        );
    }

    #[tokio::test]
    async fn handle_returns_ignored_for_key() {
        let mut handler = SessionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::IGNORED,
            "SessionHandler must always return IGNORED"
        );
    }

    #[tokio::test]
    async fn handle_returns_ignored_even_when_dirty() {
        let mut handler = SessionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.mark_session_dirty();

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = AppEvent::Tick;
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::IGNORED,
            "SessionHandler must return IGNORED even when saving"
        );
    }

    // =========================================================================
    // Session creation vs update
    // =========================================================================

    #[tokio::test]
    async fn handle_creates_new_session_when_no_id() {
        let mut handler = SessionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // No session_id — save should create a new session.
        assert!(state.session_id().is_none());
        state.mark_session_dirty();

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = AppEvent::Tick;
        let _result = handler.handle(&event, &mut ctx).await.unwrap();

        let id = ctx.state.session_id().expect("session_id should be set");
        assert!(!id.is_empty(), "session_id should be a non-empty string");
    }

    #[tokio::test]
    async fn handle_updates_existing_session() {
        let mut handler = SessionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // First, create a session.
        state.mark_session_dirty();
        {
            let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
            let _result = handler.handle(&AppEvent::Tick, &mut ctx).await.unwrap();
        }
        let first_id = state
            .session_id()
            .expect("should have session_id")
            .to_string();

        // Now mark dirty again — should update, not create.
        state.mark_session_dirty();
        {
            let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
            let _result = handler.handle(&AppEvent::Tick, &mut ctx).await.unwrap();
        }
        let second_id = state
            .session_id()
            .expect("should still have session_id")
            .to_string();

        assert_eq!(
            first_id, second_id,
            "Session ID must remain the same on update"
        );
    }

    // =========================================================================
    // Error handling — save failures must not crash
    // =========================================================================

    #[tokio::test]
    async fn handle_does_not_crash_on_save_error() {
        let mut handler = SessionHandler;
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();

        // Create a SessionManager pointing to an invalid path.
        let mgr = SessionManager::new(PathBuf::from("/nonexistent/impossible/path"));

        state.mark_session_dirty();

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &mgr);
        let event = AppEvent::Tick;

        // This must not return Err — save failures are logged, not propagated.
        let result = handler.handle(&event, &mut ctx).await;
        assert!(
            result.is_ok(),
            "SessionHandler must handle save errors gracefully"
        );
    }

    // =========================================================================
    // Send bound verification
    // =========================================================================

    #[test]
    fn session_handler_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<SessionHandler>();
    }

    // =========================================================================
    // Integration with EventDispatcher
    // =========================================================================

    #[tokio::test]
    async fn session_handler_works_in_dispatcher() {
        use crate::app::dispatch::EventDispatcher;

        let handler = SessionHandler;
        let mut dispatcher = EventDispatcher::new(vec![Box::new(handler)]);
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.mark_session_dirty();

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);

        // Dispatch a Tick — SessionHandler should save but return IGNORED.
        let result = dispatcher
            .dispatch(&AppEvent::Tick, &mut ctx)
            .await
            .unwrap();
        assert_eq!(
            result,
            Handled::IGNORED,
            "SessionHandler returns IGNORED in dispatcher"
        );

        // Session should have been created.
        assert!(
            ctx.state.session_id().is_some(),
            "Session should be saved via dispatcher"
        );
    }

    // =========================================================================
    // auto_save helper function tests
    // =========================================================================

    #[tokio::test]
    async fn auto_save_creates_session_file() {
        let mut state = test_state();
        let (session_mgr, dir) = test_session_manager();

        auto_save(&mut state, &session_mgr).await;

        let id = state.session_id().expect("session_id should be set");

        // Verify the session file exists on disk.
        let session_path = dir.path().join(format!("{id}.json"));
        assert!(
            session_path.exists(),
            "Session file should exist at {session_path:?}"
        );
    }

    #[tokio::test]
    async fn auto_save_updates_existing_session() {
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // Create initial session.
        auto_save(&mut state, &session_mgr).await;
        let id = state.session_id().unwrap().to_string();

        // Modify state and save again.
        state.input = "updated input".to_string();
        auto_save(&mut state, &session_mgr).await;

        // ID should remain the same.
        assert_eq!(state.session_id().unwrap(), id);
    }

    #[tokio::test]
    async fn auto_save_handles_invalid_path_gracefully() {
        let mut state = test_state();
        let mgr = SessionManager::new(PathBuf::from("/nonexistent/impossible/path"));

        // Should not panic or return error via state — just logs a warning.
        auto_save(&mut state, &mgr).await;
        assert!(
            state.session_id().is_none(),
            "session_id should remain None on save failure"
        );
    }
}
