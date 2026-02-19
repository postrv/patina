//! Shared application context for event handlers.
//!
//! `AppContext` bundles the references that event handlers need — terminal,
//! API client, application state, and session manager — into a single
//! borrow-friendly struct. This replaces the four separate parameters
//! previously threaded through the event loop.

use crossterm::event::Event;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io;

use crate::api::AnthropicClient;
use crate::app::events::AppEvent;
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

    /// Receives the next application event from all event sources.
    ///
    /// Merges crossterm terminal events, background channels (API streaming,
    /// tool results), and the tick timer into a single [`AppEvent`]. Uses
    /// `biased` select to prioritize user input over background events.
    ///
    /// # Key behaviors
    ///
    /// - Crossterm key presses map to [`AppEvent::Key`]; releases are filtered
    /// - Ctrl+C and Ctrl+D map to [`AppEvent::Quit`]
    /// - Mouse and resize events map to their respective variants
    /// - FocusGained/FocusLost/Paste crossterm events are silently skipped
    /// - Background API chunks map to [`AppEvent::ApiChunk`] (guarded by
    ///   [`AppState::has_background_work`])
    /// - Tool results map to [`AppEvent::ToolResult`]
    /// - Tick fires when [`AppState::is_loading`] or
    ///   [`AppState::has_executing_tools`] is true
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use patina::app::context::AppContext;
    ///
    /// let event = ctx.recv_event(&mut crossterm_stream, &mut tick_interval).await;
    /// match event {
    ///     AppEvent::Key(key) => { /* handle key */ }
    ///     AppEvent::Quit => { /* exit loop */ }
    ///     _ => {}
    /// }
    /// ```
    pub async fn recv_event<S>(
        &mut self,
        _crossterm_events: &mut S,
        _tick_interval: &mut tokio::time::Interval,
    ) -> AppEvent
    where
        S: futures::Stream<Item = Result<Event, io::Error>> + Unpin,
    {
        // Stub: RED phase — tests define the expected behavior.
        // GREEN phase will replace this with the real tokio::select! implementation.
        AppEvent::Tick
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::StreamEvent;
    use crate::types::config::ParallelMode;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use secrecy::SecretString;
    use std::path::PathBuf;
    use std::time::Duration;
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

    /// Creates a crossterm event stream from a vec of events for testing.
    fn crossterm_stream(
        events: Vec<Event>,
    ) -> impl futures::Stream<Item = Result<Event, io::Error>> + Unpin {
        futures::stream::iter(events.into_iter().map(Ok))
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

    // =========================================================================
    // recv_event tests
    // =========================================================================

    #[tokio::test]
    async fn recv_event_returns_key_from_crossterm() {
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let session_mgr = test_session_manager();

        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        let mut stream = crossterm_stream(vec![Event::Key(key)]);
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.tick().await; // consume first immediate tick

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = ctx.recv_event(&mut stream, &mut interval).await;
        assert!(
            matches!(event, AppEvent::Key(k) if k.code == KeyCode::Char('a')),
            "Expected Key('a'), got {event}"
        );
    }

    #[tokio::test]
    async fn recv_event_returns_mouse_from_crossterm() {
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let session_mgr = test_session_manager();

        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        let mut stream = crossterm_stream(vec![Event::Mouse(mouse)]);
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.tick().await;

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = ctx.recv_event(&mut stream, &mut interval).await;
        assert!(
            matches!(event, AppEvent::Mouse(_)),
            "Expected Mouse event, got {event}"
        );
    }

    #[tokio::test]
    async fn recv_event_returns_resize_from_crossterm() {
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let session_mgr = test_session_manager();

        let mut stream = crossterm_stream(vec![Event::Resize(120, 40)]);
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.tick().await;

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = ctx.recv_event(&mut stream, &mut interval).await;
        match event {
            AppEvent::Resize { width, height } => {
                assert_eq!(width, 120);
                assert_eq!(height, 40);
            }
            other => panic!("Expected Resize(120, 40), got {other}"),
        }
    }

    #[tokio::test]
    async fn recv_event_maps_ctrl_c_to_quit() {
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let session_mgr = test_session_manager();

        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let mut stream = crossterm_stream(vec![Event::Key(key)]);
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.tick().await;

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = ctx.recv_event(&mut stream, &mut interval).await;
        assert!(
            matches!(event, AppEvent::Quit),
            "Expected Quit for Ctrl+C, got {event}"
        );
    }

    #[tokio::test]
    async fn recv_event_maps_ctrl_d_to_quit() {
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let session_mgr = test_session_manager();

        let key = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL);
        let mut stream = crossterm_stream(vec![Event::Key(key)]);
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.tick().await;

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = ctx.recv_event(&mut stream, &mut interval).await;
        assert!(
            matches!(event, AppEvent::Quit),
            "Expected Quit for Ctrl+D, got {event}"
        );
    }

    #[tokio::test]
    async fn recv_event_filters_key_release_events() {
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let session_mgr = test_session_manager();

        // Send a Release event followed by a Press event.
        // recv_event should skip the Release and return the Press.
        let release = KeyEvent {
            code: KeyCode::Char('x'),
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Release,
            state: crossterm::event::KeyEventState::NONE,
        };
        let press = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
        let mut stream = crossterm_stream(vec![Event::Key(release), Event::Key(press)]);
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.tick().await;

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = ctx.recv_event(&mut stream, &mut interval).await;
        assert!(
            matches!(event, AppEvent::Key(k) if k.code == KeyCode::Char('y')),
            "Expected Key('y') after filtering release, got {event}"
        );
    }

    #[tokio::test]
    async fn recv_event_filters_focus_events() {
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let session_mgr = test_session_manager();

        // FocusGained should be silently skipped; the Key after it should be returned.
        let key = KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE);
        let mut stream = crossterm_stream(vec![Event::FocusGained, Event::Key(key)]);
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.tick().await;

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = ctx.recv_event(&mut stream, &mut interval).await;
        assert!(
            matches!(event, AppEvent::Key(k) if k.code == KeyCode::Char('z')),
            "Expected Key('z') after filtering FocusGained, got {event}"
        );
    }

    #[tokio::test]
    async fn recv_event_returns_api_chunk_from_background() {
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let session_mgr = test_session_manager();

        // Set up a streaming channel with a chunk ready.
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        state.set_streaming_rx(rx);
        tx.send(StreamEvent::ContentDelta("hello".to_string()))
            .await
            .unwrap();

        // Use a pending stream so the crossterm branch never fires.
        let mut stream = futures::stream::pending::<Result<Event, io::Error>>();
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.tick().await;

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = ctx.recv_event(&mut stream, &mut interval).await;
        assert!(
            matches!(&event, AppEvent::ApiChunk(StreamEvent::ContentDelta(s)) if s == "hello"),
            "Expected ApiChunk(ContentDelta(\"hello\")), got {event}"
        );
    }

    #[tokio::test]
    async fn recv_event_returns_tool_result_from_background() {
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let session_mgr = test_session_manager();

        // Set up a tool result channel with a result ready.
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        state.set_tool_result_rx(rx);
        let result = crate::types::ToolResultBlock {
            tool_use_id: "toolu_abc".to_string(),
            content: "output".to_string(),
            is_error: false,
        };
        tx.send(("toolu_abc".to_string(), result)).unwrap();

        let mut stream = futures::stream::pending::<Result<Event, io::Error>>();
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.tick().await;

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = ctx.recv_event(&mut stream, &mut interval).await;
        assert!(
            matches!(&event, AppEvent::ToolResult { tool_id, .. } if tool_id == "toolu_abc"),
            "Expected ToolResult(toolu_abc), got {event}"
        );
    }

    #[tokio::test]
    async fn recv_event_returns_tick_when_loading() {
        tokio::time::pause();

        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let session_mgr = test_session_manager();

        state.set_loading(true);

        let mut stream = futures::stream::pending::<Result<Event, io::Error>>();
        let mut interval = tokio::time::interval(Duration::from_millis(100));
        interval.tick().await; // consume first immediate tick

        tokio::time::advance(Duration::from_millis(100)).await;

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = ctx.recv_event(&mut stream, &mut interval).await;
        assert!(
            matches!(event, AppEvent::Tick),
            "Expected Tick when loading, got {event}"
        );
    }

    #[tokio::test]
    async fn recv_event_returns_tick_when_executing_tools() {
        tokio::time::pause();

        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let session_mgr = test_session_manager();

        // Mark a tool as executing so has_executing_tools() returns true.
        state.mark_tool_executing("toolu_test");

        let mut stream = futures::stream::pending::<Result<Event, io::Error>>();
        let mut interval = tokio::time::interval(Duration::from_millis(100));
        interval.tick().await;

        tokio::time::advance(Duration::from_millis(100)).await;

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        let event = ctx.recv_event(&mut stream, &mut interval).await;
        assert!(
            matches!(event, AppEvent::Tick),
            "Expected Tick when executing tools, got {event}"
        );
    }

    #[tokio::test]
    async fn recv_event_prioritizes_input_over_background() {
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let session_mgr = test_session_manager();

        // Set up a background channel with data ready.
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        state.set_streaming_rx(rx);
        tx.send(StreamEvent::ContentDelta("bg".to_string()))
            .await
            .unwrap();

        // Also provide a crossterm key event.
        let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
        let mut stream = crossterm_stream(vec![Event::Key(key)]);
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.tick().await;

        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);
        // With biased select, input (first branch) should win.
        let event = ctx.recv_event(&mut stream, &mut interval).await;
        assert!(
            matches!(event, AppEvent::Key(k) if k.code == KeyCode::Char('x')),
            "Expected Key('x') to have priority over background, got {event}"
        );
    }
}
