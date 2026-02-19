//! Shared application context for event handlers.
//!
//! `AppContext` bundles the references that event handlers need — terminal,
//! API client, application state, and session manager — into a single
//! borrow-friendly struct. This replaces the four separate parameters
//! previously threaded through the event loop.

use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io;

use crate::api::AnthropicClient;
use crate::app::state::AppState;
use crate::session::SessionManager;

/// Bundles shared references needed by event handlers.
///
/// `AppContext` provides a single access point for the terminal, API client,
/// application state, and session manager. Handlers receive `&mut AppContext`
/// instead of multiple individual references.
///
/// # Examples
///
/// ```rust,ignore
/// use patina::app::context::AppContext;
///
/// let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_manager);
/// assert!(ctx.needs_render() || !ctx.needs_render()); // delegates to state
/// ```
pub struct AppContext<'a> {
    /// The ratatui terminal for drawing.
    pub terminal: &'a mut Terminal<CrosstermBackend<io::Stdout>>,
    /// The API client for making requests.
    pub client: &'a AnthropicClient,
    /// Mutable application state.
    pub state: &'a mut AppState,
    /// Session persistence manager.
    pub session_manager: &'a SessionManager,
}

impl<'a> AppContext<'a> {
    /// Creates a new `AppContext` from the event loop's parameters.
    ///
    /// # Arguments
    ///
    /// * `terminal` - The ratatui terminal for rendering
    /// * `client` - The Anthropic API client
    /// * `state` - Mutable application state
    /// * `session_manager` - Session persistence manager
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
    /// ```
    #[must_use]
    pub fn new(
        terminal: &'a mut Terminal<CrosstermBackend<io::Stdout>>,
        client: &'a AnthropicClient,
        state: &'a mut AppState,
        session_manager: &'a SessionManager,
    ) -> Self {
        Self {
            terminal,
            client,
            state,
            session_manager,
        }
    }

    /// Returns `true` if the UI needs re-rendering.
    ///
    /// Delegates to [`AppState::needs_render`].
    #[must_use]
    pub fn needs_render(&self) -> bool {
        self.state.needs_render()
    }

    /// Clears all dirty flags after rendering.
    ///
    /// Delegates to [`AppState::mark_rendered`].
    pub fn mark_rendered(&mut self) {
        self.state.mark_rendered();
    }

    /// Returns `true` if the application is waiting for an API response.
    ///
    /// Delegates to [`AppState::is_loading`].
    #[must_use]
    pub fn is_loading(&self) -> bool {
        self.state.is_loading()
    }

    /// Returns `true` if there are active background tasks (streaming or tool execution).
    ///
    /// Delegates to [`AppState::has_background_work`].
    #[must_use]
    pub fn has_background_work(&self) -> bool {
        self.state.has_background_work()
    }

    /// Returns `true` if a permission prompt is waiting for user input.
    ///
    /// Delegates to [`AppState::has_pending_permission`].
    #[must_use]
    pub fn has_pending_permission(&self) -> bool {
        self.state.has_pending_permission()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::config::ParallelMode;
    use secrecy::SecretString;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Helper to create a real terminal for tests.
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

    fn test_session_manager() -> SessionManager {
        let dir = TempDir::new().expect("failed to create temp dir");
        SessionManager::new(dir.path().to_path_buf())
    }

    #[test]
    fn context_construction() {
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let session_mgr = test_session_manager();

        let _ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
    }

    #[test]
    fn context_exposes_state_accessors() {
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let session_mgr = test_session_manager();

        let ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);

        // Fresh state should not be loading
        assert!(!ctx.is_loading());
        // Fresh state should not have background work
        assert!(!ctx.has_background_work());
        // Fresh state should not have pending permission
        assert!(!ctx.has_pending_permission());
    }

    #[test]
    fn needs_render_delegates_to_state() {
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let session_mgr = test_session_manager();

        // Mark state dirty
        state.mark_full_redraw();

        let ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        assert!(ctx.needs_render());
    }

    #[test]
    fn mark_rendered_clears_dirty() {
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let session_mgr = test_session_manager();

        state.mark_full_redraw();

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        assert!(ctx.needs_render());

        ctx.mark_rendered();
        assert!(!ctx.needs_render());
    }

    #[test]
    fn context_provides_mutable_state_access() {
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let session_mgr = test_session_manager();

        let ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);

        // Should be able to mutate state through context
        ctx.state.mark_full_redraw();
        assert!(ctx.needs_render());
    }

    #[test]
    fn context_provides_client_access() {
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let session_mgr = test_session_manager();

        let ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);

        // Should be able to access client through context
        let _client_ref: &AnthropicClient = ctx.client;
    }

    #[test]
    fn context_provides_session_manager_access() {
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let session_mgr = test_session_manager();

        let ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);

        // Should be able to access session manager through context
        let _sm_ref: &SessionManager = ctx.session_manager;
    }
}
