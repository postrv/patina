//! Tests for `app` module: dispatcher integration and orchestrator initialization.

use patina::api::{AnthropicClient, LlmProvider, StreamEvent};
use patina::app::context::AppContext;
use patina::app::dispatch::Handled;
use patina::app::events::AppEvent;
use patina::app::state::AppState;
use patina::app::{create_dispatcher, initialize_compression_orchestrator, Config};
use patina::permissions::PermissionRequest;
use patina::session::SessionManager;
use patina::types::config::{NarsilMode, ParallelMode};
use patina::types::StopReason;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
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
// Compression orchestrator initialization tests
// =========================================================================

#[test]
fn test_initialize_compression_orchestrator_disabled_mode() {
    let mut state = AppState::new(PathBuf::from("/tmp"), false, ParallelMode::Enabled);
    let config = Config::new(
        SecretString::new("test".into()),
        "model",
        PathBuf::from("/tmp"),
    )
    .with_narsil_mode(NarsilMode::Disabled);

    // Disabled mode should never set orchestrator
    initialize_compression_orchestrator(&mut state, &config);
    assert!(state.compression_orchestrator().is_none());
}

#[test]
fn test_initialize_compression_orchestrator_auto_mode_no_code_files() {
    use tempfile::tempdir;

    // Create a temp dir with no code files
    let temp = tempdir().unwrap();
    let mut state = AppState::new(temp.path().to_path_buf(), false, ParallelMode::Enabled);
    let config = Config::new(
        SecretString::new("test".into()),
        "model",
        temp.path().to_path_buf(),
    )
    .with_narsil_mode(NarsilMode::Auto);

    // Auto mode with no code files should not set orchestrator
    initialize_compression_orchestrator(&mut state, &config);
    // Result depends on is_narsil_available() AND has_supported_code_files()
    // Since temp dir has no code files, orchestrator should be None
    assert!(state.compression_orchestrator().is_none());
}

// =========================================================================
// create_dispatcher() tests — Phase 1.10 integration
// =========================================================================

#[test]
fn create_dispatcher_returns_seven_handlers_and_one_observer() {
    let dispatcher = create_dispatcher();
    assert_eq!(
        dispatcher.handler_count(),
        7,
        "Dispatcher must have 7 handlers: Permission, Completion, Keyboard, Stream, Agent, Continuous, Tick"
    );
    assert_eq!(
        dispatcher.observer_count(),
        1,
        "Dispatcher must have 1 observer: Session"
    );
}

// =========================================================================
// Full dispatcher integration tests — all handlers working together
// =========================================================================

#[tokio::test]
async fn dispatcher_quit_event_triggers_session_save() {
    let mut dispatcher = create_dispatcher();

    let client = test_client();
    let mut state = test_state();
    let (session_mgr, _dir) = test_session_manager();

    let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

    // Quit event should be dispatched to all handlers (none consume it
    // except SessionHandler which observes and returns IGNORED).
    let event = AppEvent::Quit;
    let result: Handled = dispatcher.dispatch(&event, &mut ctx).await.unwrap();

    // SessionHandler always returns IGNORED, so overall result is IGNORED.
    assert_eq!(
        result,
        Handled::IGNORED,
        "Quit event should not be consumed (SessionHandler returns IGNORED)"
    );

    // SessionHandler must have saved the session on Quit.
    assert!(
        ctx.state.session_id().is_some(),
        "SessionHandler must auto-save session on Quit event"
    );
}

#[tokio::test]
async fn dispatcher_char_input_inserts_into_state() {
    let mut dispatcher = create_dispatcher();

    let client = test_client();
    let mut state = test_state();
    let (session_mgr, _dir) = test_session_manager();

    let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

    let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
    let result: Handled = dispatcher.dispatch(&event, &mut ctx).await.unwrap();

    assert_eq!(result, Handled::CONSUMED);
    assert_eq!(
        ctx.state.input(),
        "x",
        "Character input must flow through to KeyboardHandler and insert into input"
    );
}

#[tokio::test]
async fn dispatcher_tick_advances_throbber() {
    let mut dispatcher = create_dispatcher();

    let client = test_client();
    let mut state = test_state();
    let (session_mgr, _dir) = test_session_manager();

    let before = state.throbber_char();

    let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

    let event = AppEvent::Tick;
    let result: Handled = dispatcher.dispatch(&event, &mut ctx).await.unwrap();

    assert_eq!(result, Handled::CONSUMED);
    assert_ne!(
        ctx.state.throbber_char(),
        before,
        "Tick event must advance throbber via TickHandler"
    );
}

#[tokio::test]
async fn dispatcher_api_chunk_content_delta_consumed() {
    let mut dispatcher = create_dispatcher();

    let client = test_client();
    let mut state = test_state();
    let (session_mgr, _dir) = test_session_manager();

    let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

    let event = AppEvent::ApiChunk(StreamEvent::ContentDelta("hello".to_string()));
    let result: Handled = dispatcher.dispatch(&event, &mut ctx).await.unwrap();

    assert_eq!(
        result,
        Handled::CONSUMED,
        "ApiChunk must be consumed by StreamHandler"
    );
}

#[tokio::test]
async fn dispatcher_message_complete_marks_dirty_and_saves() {
    let mut dispatcher = create_dispatcher();

    let client = test_client();
    let mut state = test_state();
    let (session_mgr, _dir) = test_session_manager();

    let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

    // MessageComplete should: StreamHandler marks dirty -> SessionHandler saves.
    let event = AppEvent::ApiChunk(StreamEvent::MessageComplete {
        stop_reason: StopReason::EndTurn,
    });
    let result: Handled = dispatcher.dispatch(&event, &mut ctx).await.unwrap();

    assert_eq!(result, Handled::CONSUMED);

    // SessionHandler should have observed the dirty flag and saved.
    assert!(
        ctx.state.session_id().is_some(),
        "MessageComplete -> StreamHandler marks dirty -> SessionHandler saves"
    );
}

#[tokio::test]
async fn dispatcher_permission_key_consumed_before_keyboard() {
    let mut dispatcher = create_dispatcher();

    let client = test_client();
    let mut state = test_state();
    let (session_mgr, _dir) = test_session_manager();

    // Set up a pending permission.
    state.set_pending_permission(PermissionRequest::new("Bash", Some("ls"), "List"));

    let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

    // Send 'z' key — should be consumed by PermissionHandler, NOT
    // reach KeyboardHandler (which would insert 'z' into input).
    let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
    let result: Handled = dispatcher.dispatch(&event, &mut ctx).await.unwrap();

    assert_eq!(
        result,
        Handled::CONSUMED,
        "Key must be consumed by PermissionHandler when permission is pending"
    );
    assert_eq!(
        ctx.state.input(),
        "",
        "Key must NOT reach KeyboardHandler when permission is pending"
    );
}

#[tokio::test]
async fn dispatcher_resize_marks_redraw() {
    let mut dispatcher = create_dispatcher();

    let client = test_client();
    let mut state = test_state();
    let (session_mgr, _dir) = test_session_manager();

    // Clear initial render flags.
    state.mark_rendered();

    let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

    let event = AppEvent::Resize {
        width: 120,
        height: 40,
    };
    let result: Handled = dispatcher.dispatch(&event, &mut ctx).await.unwrap();

    assert_eq!(result, Handled::CONSUMED);
    assert!(
        ctx.state.needs_render(),
        "Resize must mark UI for redraw via KeyboardHandler"
    );
}

#[tokio::test]
async fn dispatcher_mouse_scroll_consumed() {
    let mut dispatcher = create_dispatcher();

    let client = test_client();
    let mut state = test_state();
    let (session_mgr, _dir) = test_session_manager();

    let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

    let event = AppEvent::Mouse(MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: 0,
        row: 0,
        modifiers: KeyModifiers::NONE,
    });
    let result: Handled = dispatcher.dispatch(&event, &mut ctx).await.unwrap();

    assert_eq!(
        result,
        Handled::CONSUMED,
        "Mouse events must be consumed by KeyboardHandler"
    );
}

#[tokio::test]
async fn dispatcher_ctrl_c_as_quit_saves_and_is_detectable() {
    let mut dispatcher = create_dispatcher();

    let client = test_client();
    let mut state = test_state();
    let (session_mgr, _dir) = test_session_manager();

    let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

    // In the real event loop, Ctrl+C is mapped to AppEvent::Quit by recv_event().
    // Dispatching Quit should save the session (via SessionHandler).
    let event = AppEvent::Quit;
    let is_quit = event.is_quit();
    let _result: Handled = dispatcher.dispatch(&event, &mut ctx).await.unwrap();

    assert!(is_quit, "AppEvent::Quit must report is_quit() == true");
    assert!(
        ctx.state.session_id().is_some(),
        "Quit dispatch must trigger session save"
    );
}

#[tokio::test]
async fn dispatcher_keyboard_quit_sets_wants_quit() {
    let mut dispatcher = create_dispatcher();

    let client = test_client();
    let mut state = test_state();
    let (session_mgr, _dir) = test_session_manager();

    let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

    // If a Key event for Ctrl+C reaches KeyboardHandler (bypassing recv_event's
    // Quit mapping), it should set wants_quit on state.
    let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    let result: Handled = dispatcher.dispatch(&event, &mut ctx).await.unwrap();

    assert_eq!(result, Handled::CONSUMED);
    assert!(
        ctx.state.wants_quit(),
        "Ctrl+C Key event must set wants_quit via KeyboardHandler"
    );
}

#[tokio::test]
async fn dispatcher_multiple_events_sequence() {
    let mut dispatcher = create_dispatcher();

    let client = test_client();
    let mut state = test_state();
    let (session_mgr, _dir) = test_session_manager();

    // Simulate a sequence: type "hi", then get an API chunk, then tick.
    let events = vec![
        AppEvent::Key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE)),
        AppEvent::Key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE)),
        AppEvent::ApiChunk(StreamEvent::ContentDelta("response".to_string())),
        AppEvent::Tick,
    ];

    for event in &events {
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
        let result: Handled = dispatcher.dispatch(event, &mut ctx).await.unwrap();
        assert_eq!(result, Handled::CONSUMED, "Event {event} must be consumed");
    }

    assert_eq!(state.input(), "hi", "Both characters must be inserted");
}
