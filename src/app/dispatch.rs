//! Event dispatcher for the handler-based architecture.
//!
//! [`EventDispatcher`] dispatches [`AppEvent`] values to an ordered list of
//! [`EventHandler`] implementations. Handlers are tried in priority order;
//! the first handler to return [`Handled::CONSUMED`] wins and remaining
//! handlers are skipped.
//!
//! This module is a pure addition — no existing code is modified.

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use tracing::trace;

use crate::app::context::AppContext;
use crate::app::events::AppEvent;

/// Indicates whether an event handler consumed an event.
///
/// `Handled(true)` means the event was fully processed and should not
/// be passed to subsequent handlers. `Handled(false)` means the handler
/// did not process the event and dispatch should continue.
///
/// # Examples
///
/// ```rust,ignore
/// use patina::app::dispatch::Handled;
///
/// let consumed = Handled::CONSUMED;
/// assert!(consumed.0);
///
/// let ignored = Handled::IGNORED;
/// assert!(!ignored.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct Handled(pub bool);

impl Handled {
    /// The event was consumed by the handler.
    pub const CONSUMED: Self = Self(true);

    /// The event was not processed by the handler.
    pub const IGNORED: Self = Self(false);

    /// Returns `true` if the event was consumed.
    #[must_use]
    pub fn is_consumed(self) -> bool {
        self.0
    }

    /// Returns `true` if the event was not consumed.
    #[must_use]
    pub fn is_ignored(self) -> bool {
        !self.0
    }
}

/// Trait for event handlers in the dispatched event loop architecture.
///
/// Handlers receive an [`AppEvent`] and an [`AppContext`] and return
/// [`Handled::CONSUMED`] if they processed the event, or [`Handled::IGNORED`]
/// if the event should be passed to the next handler.
///
/// # Errors
///
/// Implementations should return errors only for fatal conditions that
/// should propagate to the event loop. Recoverable issues should be
/// logged and return `Ok(Handled::IGNORED)`.
///
/// # Examples
///
/// ```rust,ignore
/// use patina::app::dispatch::{EventHandler, Handled};
/// use patina::app::events::AppEvent;
/// use patina::app::context::AppContext;
/// use std::future::Future;
/// use std::pin::Pin;
///
/// struct TickHandler;
///
/// impl EventHandler for TickHandler {
///     fn handle<'a>(
///         &'a mut self,
///         event: &'a AppEvent,
///         ctx: &'a mut AppContext<'_>,
///     ) -> Pin<Box<dyn Future<Output = anyhow::Result<Handled>> + Send + 'a>> {
///         Box::pin(async move {
///             if matches!(event, AppEvent::Tick) {
///                 Ok(Handled::CONSUMED)
///             } else {
///                 Ok(Handled::IGNORED)
///             }
///         })
///     }
///
///     fn name(&self) -> &str {
///         "tick"
///     }
/// }
/// ```
pub trait EventHandler: Send {
    /// Handle an event, returning whether it was consumed.
    ///
    /// # Errors
    ///
    /// Returns an error if the handler encounters a fatal condition
    /// that should propagate to the event loop.
    fn handle<'a>(
        &'a mut self,
        event: &'a AppEvent,
        ctx: &'a mut AppContext<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<Handled>> + Send + 'a>>;

    /// Human-readable name for logging and debugging.
    fn name(&self) -> &str;
}

/// Dispatches events to an ordered list of handlers.
///
/// Handlers are tried in registration order. The first handler to return
/// [`Handled::CONSUMED`] wins; remaining handlers are not called.
/// If no handler consumes the event, [`Handled::IGNORED`] is returned.
///
/// **Observers** are a separate set of handlers that always run after
/// normal dispatch, regardless of whether the event was consumed. Use
/// observers for cross-cutting concerns like session persistence that
/// must react to every event.
///
/// # Examples
///
/// ```rust,ignore
/// use patina::app::dispatch::EventDispatcher;
///
/// let dispatcher = EventDispatcher::new(vec![
///     Box::new(permission_handler),
///     Box::new(keyboard_handler),
///     Box::new(stream_handler),
/// ]).with_observers(vec![
///     Box::new(session_handler), // always runs
/// ]);
/// ```
pub struct EventDispatcher {
    handlers: Vec<Box<dyn EventHandler>>,
    observers: Vec<Box<dyn EventHandler>>,
}

impl EventDispatcher {
    /// Creates a new dispatcher with the given handlers in priority order.
    ///
    /// Handlers are called in the order provided. Earlier handlers have
    /// higher priority and can consume events before later handlers see them.
    #[must_use]
    pub fn new(handlers: Vec<Box<dyn EventHandler>>) -> Self {
        Self {
            handlers,
            observers: vec![],
        }
    }

    /// Adds observer handlers that run unconditionally after normal dispatch.
    ///
    /// Observers always run regardless of whether a handler consumed the
    /// event. They are intended for cross-cutting concerns like session
    /// persistence. Observer return values do not affect the dispatch result.
    #[must_use]
    pub fn with_observers(mut self, observers: Vec<Box<dyn EventHandler>>) -> Self {
        self.observers = observers;
        self
    }

    /// Returns the number of registered handlers (excludes observers).
    #[must_use]
    pub fn handler_count(&self) -> usize {
        self.handlers.len()
    }

    /// Returns the number of registered observers.
    #[must_use]
    pub fn observer_count(&self) -> usize {
        self.observers.len()
    }

    /// Dispatches an event to handlers in order until one consumes it,
    /// then unconditionally runs all observers.
    ///
    /// Returns [`Handled::CONSUMED`] if any handler consumed the event,
    /// or [`Handled::IGNORED`] if no handler processed it. Observer
    /// return values do not affect the result.
    ///
    /// # Errors
    ///
    /// Propagates any error returned by a handler or observer.
    pub async fn dispatch(
        &mut self,
        event: &AppEvent,
        ctx: &mut AppContext<'_>,
    ) -> Result<Handled> {
        let mut consumed = Handled::IGNORED;

        for handler in &mut self.handlers {
            let result = handler.handle(event, ctx).await?;
            if result.is_consumed() {
                trace!(handler = handler.name(), %event, "event consumed");
                consumed = Handled::CONSUMED;
                break;
            }
        }

        // Observers always run regardless of consumption.
        for observer in &mut self.observers {
            observer.handle(event, ctx).await?;
            trace!(observer = observer.name(), %event, "observer executed");
        }

        if consumed.is_ignored() {
            trace!(%event, "event unhandled");
        }

        Ok(consumed)
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
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

    /// A mock handler that tracks call count and returns a configurable result.
    struct MockHandler {
        handler_name: String,
        result: Handled,
        call_count: Arc<AtomicUsize>,
    }

    impl MockHandler {
        fn new(name: &str, result: Handled) -> (Self, Arc<AtomicUsize>) {
            let count = Arc::new(AtomicUsize::new(0));
            (
                Self {
                    handler_name: name.to_string(),
                    result,
                    call_count: count.clone(),
                },
                count,
            )
        }
    }

    impl EventHandler for MockHandler {
        fn handle<'a>(
            &'a mut self,
            _event: &'a AppEvent,
            _ctx: &'a mut AppContext<'_>,
        ) -> Pin<Box<dyn Future<Output = Result<Handled>> + Send + 'a>> {
            Box::pin(async move {
                self.call_count.fetch_add(1, Ordering::SeqCst);
                Ok(self.result)
            })
        }

        fn name(&self) -> &str {
            &self.handler_name
        }
    }

    /// A mock handler that returns an error.
    struct ErrorHandler {
        handler_name: String,
        call_count: Arc<AtomicUsize>,
    }

    impl ErrorHandler {
        fn new(name: &str) -> (Self, Arc<AtomicUsize>) {
            let count = Arc::new(AtomicUsize::new(0));
            (
                Self {
                    handler_name: name.to_string(),
                    call_count: count.clone(),
                },
                count,
            )
        }
    }

    impl EventHandler for ErrorHandler {
        fn handle<'a>(
            &'a mut self,
            _event: &'a AppEvent,
            _ctx: &'a mut AppContext<'_>,
        ) -> Pin<Box<dyn Future<Output = Result<Handled>> + Send + 'a>> {
            Box::pin(async move {
                self.call_count.fetch_add(1, Ordering::SeqCst);
                Err(anyhow::anyhow!("handler error"))
            })
        }

        fn name(&self) -> &str {
            &self.handler_name
        }
    }

    fn tick_event() -> AppEvent {
        AppEvent::Tick
    }

    fn key_event() -> AppEvent {
        AppEvent::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
    }

    // =========================================================================
    // Handled tests
    // =========================================================================

    #[test]
    fn handled_consumed_is_true() {
        let consumed = Handled::CONSUMED;
        assert!(consumed.0);
        assert_eq!(consumed, Handled(true));
    }

    #[test]
    fn handled_ignored_is_false() {
        let ignored = Handled::IGNORED;
        assert!(!ignored.0);
        assert_eq!(ignored, Handled(false));
    }

    #[test]
    fn handled_is_copy() {
        let h = Handled::CONSUMED;
        let h2 = h;
        assert_eq!(h, h2);
    }

    #[test]
    fn handled_debug_format() {
        let s = format!("{:?}", Handled::CONSUMED);
        assert!(s.contains("Handled"));
        assert!(s.contains("true"));
    }

    #[test]
    fn handled_is_consumed_method() {
        assert!(Handled::CONSUMED.is_consumed());
        assert!(!Handled::IGNORED.is_consumed());
    }

    #[test]
    fn handled_is_ignored_method() {
        assert!(Handled::IGNORED.is_ignored());
        assert!(!Handled::CONSUMED.is_ignored());
    }

    // =========================================================================
    // EventHandler trait tests (via MockHandler)
    // =========================================================================

    #[tokio::test]
    async fn mock_handler_tracks_calls() {
        let (mut handler, count) = MockHandler::new("test", Handled::CONSUMED);
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);

        let event = tick_event();
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(result, Handled::CONSUMED);
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn mock_handler_returns_name() {
        let (handler, _) = MockHandler::new("my_handler", Handled::IGNORED);
        assert_eq!(handler.name(), "my_handler");
    }

    // =========================================================================
    // EventDispatcher construction tests
    // =========================================================================

    #[test]
    fn dispatcher_new_with_empty_handlers() {
        let dispatcher = EventDispatcher::new(vec![]);
        assert_eq!(dispatcher.handler_count(), 0);
    }

    #[test]
    fn dispatcher_handler_count() {
        let (h1, _) = MockHandler::new("a", Handled::IGNORED);
        let (h2, _) = MockHandler::new("b", Handled::IGNORED);
        let dispatcher = EventDispatcher::new(vec![Box::new(h1), Box::new(h2)]);
        assert_eq!(dispatcher.handler_count(), 2);
    }

    // =========================================================================
    // EventDispatcher::dispatch tests
    // =========================================================================

    #[tokio::test]
    async fn dispatch_empty_returns_ignored() {
        let mut dispatcher = EventDispatcher::new(vec![]);
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);

        let event = tick_event();
        let result = dispatcher.dispatch(&event, &mut ctx).await.unwrap();
        assert_eq!(result, Handled::IGNORED);
    }

    #[tokio::test]
    async fn dispatch_single_consuming_handler_returns_consumed() {
        let (handler, count) = MockHandler::new("consumer", Handled::CONSUMED);
        let mut dispatcher = EventDispatcher::new(vec![Box::new(handler)]);
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);

        let event = tick_event();
        let result = dispatcher.dispatch(&event, &mut ctx).await.unwrap();

        assert_eq!(result, Handled::CONSUMED);
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn dispatch_single_ignoring_handler_returns_ignored() {
        let (handler, count) = MockHandler::new("ignorer", Handled::IGNORED);
        let mut dispatcher = EventDispatcher::new(vec![Box::new(handler)]);
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);

        let event = tick_event();
        let result = dispatcher.dispatch(&event, &mut ctx).await.unwrap();

        assert_eq!(result, Handled::IGNORED);
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn dispatch_first_handler_wins() {
        // First handler consumes — second should never be called
        let (h1, count1) = MockHandler::new("first", Handled::CONSUMED);
        let (h2, count2) = MockHandler::new("second", Handled::CONSUMED);
        let mut dispatcher = EventDispatcher::new(vec![Box::new(h1), Box::new(h2)]);
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);

        let event = key_event();
        let result = dispatcher.dispatch(&event, &mut ctx).await.unwrap();

        assert_eq!(result, Handled::CONSUMED);
        assert_eq!(count1.load(Ordering::SeqCst), 1);
        assert_eq!(
            count2.load(Ordering::SeqCst),
            0,
            "second handler should not be called"
        );
    }

    #[tokio::test]
    async fn dispatch_falls_through_to_second_handler() {
        // First handler ignores — second should be called
        let (h1, count1) = MockHandler::new("ignorer", Handled::IGNORED);
        let (h2, count2) = MockHandler::new("consumer", Handled::CONSUMED);
        let mut dispatcher = EventDispatcher::new(vec![Box::new(h1), Box::new(h2)]);
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);

        let event = key_event();
        let result = dispatcher.dispatch(&event, &mut ctx).await.unwrap();

        assert_eq!(result, Handled::CONSUMED);
        assert_eq!(count1.load(Ordering::SeqCst), 1);
        assert_eq!(count2.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn dispatch_all_handlers_ignore_returns_ignored() {
        let (h1, count1) = MockHandler::new("a", Handled::IGNORED);
        let (h2, count2) = MockHandler::new("b", Handled::IGNORED);
        let (h3, count3) = MockHandler::new("c", Handled::IGNORED);
        let mut dispatcher = EventDispatcher::new(vec![Box::new(h1), Box::new(h2), Box::new(h3)]);
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);

        let event = tick_event();
        let result = dispatcher.dispatch(&event, &mut ctx).await.unwrap();

        assert_eq!(result, Handled::IGNORED);
        // All handlers should have been called
        assert_eq!(count1.load(Ordering::SeqCst), 1);
        assert_eq!(count2.load(Ordering::SeqCst), 1);
        assert_eq!(count3.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn dispatch_error_propagates() {
        let (handler, count) = ErrorHandler::new("failing");
        let mut dispatcher = EventDispatcher::new(vec![Box::new(handler)]);
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);

        let event = tick_event();
        let result = dispatcher.dispatch(&event, &mut ctx).await;

        assert!(result.is_err());
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn dispatch_error_stops_processing() {
        // Error handler first — second should never be called
        let (h1, count1) = ErrorHandler::new("error");
        let (h2, count2) = MockHandler::new("never", Handled::CONSUMED);
        let mut dispatcher = EventDispatcher::new(vec![Box::new(h1), Box::new(h2)]);
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);

        let event = tick_event();
        let result = dispatcher.dispatch(&event, &mut ctx).await;

        assert!(result.is_err());
        assert_eq!(count1.load(Ordering::SeqCst), 1);
        assert_eq!(
            count2.load(Ordering::SeqCst),
            0,
            "handler after error should not be called"
        );
    }

    #[tokio::test]
    async fn dispatch_handler_ordering_preserved() {
        // Three handlers — only second consumes. Verify call order.
        let (h1, count1) = MockHandler::new("first", Handled::IGNORED);
        let (h2, count2) = MockHandler::new("second", Handled::CONSUMED);
        let (h3, count3) = MockHandler::new("third", Handled::IGNORED);
        let mut dispatcher = EventDispatcher::new(vec![Box::new(h1), Box::new(h2), Box::new(h3)]);
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);

        let event = tick_event();
        let result = dispatcher.dispatch(&event, &mut ctx).await.unwrap();

        assert_eq!(result, Handled::CONSUMED);
        assert_eq!(
            count1.load(Ordering::SeqCst),
            1,
            "first handler should be called"
        );
        assert_eq!(
            count2.load(Ordering::SeqCst),
            1,
            "second handler should be called"
        );
        assert_eq!(
            count3.load(Ordering::SeqCst),
            0,
            "third handler should NOT be called"
        );
    }

    #[tokio::test]
    async fn dispatch_multiple_events_independently() {
        let (handler, count) = MockHandler::new("counter", Handled::CONSUMED);
        let mut dispatcher = EventDispatcher::new(vec![Box::new(handler)]);
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);

        // Dispatch three events
        for _ in 0..3 {
            let event = tick_event();
            let _result = dispatcher.dispatch(&event, &mut ctx).await.unwrap();
        }

        assert_eq!(count.load(Ordering::SeqCst), 3);
    }

    // =========================================================================
    // Observer tests
    // =========================================================================

    #[test]
    fn dispatcher_observer_count_default_zero() {
        let dispatcher = EventDispatcher::new(vec![]);
        assert_eq!(dispatcher.observer_count(), 0);
    }

    #[test]
    fn dispatcher_with_observers_sets_count() {
        let (obs, _) = MockHandler::new("obs", Handled::IGNORED);
        let dispatcher =
            EventDispatcher::new(vec![]).with_observers(vec![Box::new(obs)]);
        assert_eq!(dispatcher.observer_count(), 1);
        assert_eq!(dispatcher.handler_count(), 0);
    }

    #[tokio::test]
    async fn observer_runs_even_when_handler_consumes() {
        // Handler consumes the event — observer must still run.
        let (handler, h_count) = MockHandler::new("handler", Handled::CONSUMED);
        let (obs, o_count) = MockHandler::new("observer", Handled::IGNORED);
        let mut dispatcher =
            EventDispatcher::new(vec![Box::new(handler)]).with_observers(vec![Box::new(obs)]);
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);

        let event = tick_event();
        let result = dispatcher.dispatch(&event, &mut ctx).await.unwrap();

        assert_eq!(result, Handled::CONSUMED);
        assert_eq!(h_count.load(Ordering::SeqCst), 1, "handler should be called");
        assert_eq!(
            o_count.load(Ordering::SeqCst),
            1,
            "observer must run even when handler consumed the event"
        );
    }

    #[tokio::test]
    async fn observer_runs_when_no_handler_consumes() {
        let (handler, h_count) = MockHandler::new("handler", Handled::IGNORED);
        let (obs, o_count) = MockHandler::new("observer", Handled::IGNORED);
        let mut dispatcher =
            EventDispatcher::new(vec![Box::new(handler)]).with_observers(vec![Box::new(obs)]);
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);

        let event = tick_event();
        let result = dispatcher.dispatch(&event, &mut ctx).await.unwrap();

        assert_eq!(result, Handled::IGNORED);
        assert_eq!(h_count.load(Ordering::SeqCst), 1);
        assert_eq!(o_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn observer_result_does_not_affect_dispatch_result() {
        // No handlers — event is unhandled. Observer runs but its result
        // does not change the dispatch result to CONSUMED.
        let (obs, _) = MockHandler::new("observer", Handled::CONSUMED);
        let mut dispatcher =
            EventDispatcher::new(vec![]).with_observers(vec![Box::new(obs)]);
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);

        let event = tick_event();
        let result = dispatcher.dispatch(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::IGNORED,
            "Observer returning CONSUMED must NOT change the dispatch result"
        );
    }

    #[tokio::test]
    async fn observer_error_propagates() {
        let (obs, _) = ErrorHandler::new("failing_observer");
        let mut dispatcher =
            EventDispatcher::new(vec![]).with_observers(vec![Box::new(obs)]);
        let mut terminal = test_terminal();
        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(&mut terminal, &client, &mut state, &session_mgr);

        let event = tick_event();
        let result = dispatcher.dispatch(&event, &mut ctx).await;

        assert!(result.is_err(), "Observer errors must propagate");
    }

    // =========================================================================
    // Send + Sync assertions
    // =========================================================================

    #[test]
    fn handled_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Handled>();
    }

    #[test]
    fn handled_is_sync() {
        fn assert_sync<T: Sync>() {}
        assert_sync::<Handled>();
    }

    #[test]
    fn dispatcher_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<EventDispatcher>();
    }
}
