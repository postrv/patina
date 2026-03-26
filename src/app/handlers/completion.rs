//! Completion handler for slash command auto-completion.
//!
//! [`CompletionHandler`] intercepts key events when the completion popup is
//! active and converts them into navigation, accept, or dismiss actions.
//!
//! This handler **must** run after `PermissionHandler` (so permission modals
//! take priority) and before `KeyboardHandler` (so completion navigation keys
//! don't reach normal input handling).
//!
//! # Key Bindings (when completion is active)
//!
//! - **Down**: Select next entry
//! - **Up**: Select previous entry
//! - **Tab**: Accept selected entry
//! - **Enter**: Accept selected entry
//! - **Esc**: Dismiss completion popup
//! - **Other keys**: Pass through to `KeyboardHandler` (IGNORED)

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};

use crate::app::context::AppContext;
use crate::app::dispatch::{EventHandler, Handled};
use crate::app::events::AppEvent;
use crate::app::state::InputState;

/// Handles completion popup key events for slash command auto-completion.
///
/// When the completion popup is active, this handler intercepts navigation
/// and selection keys. All other keys pass through to the `KeyboardHandler`.
///
/// # Examples
///
/// ```rust,ignore
/// use patina::app::handlers::completion::CompletionHandler;
/// use patina::app::dispatch::EventDispatcher;
///
/// let dispatcher = EventDispatcher::new(vec![
///     Box::new(PermissionHandler),    // Priority 1
///     Box::new(CompletionHandler),    // Priority 2
///     Box::new(KeyboardHandler),      // Priority 3
/// ]);
/// ```
pub struct CompletionHandler;

impl CompletionHandler {
    /// Processes a key event against the completion popup state.
    ///
    /// This is the core logic extracted for independent testability.
    /// Takes only `InputState`, not the full `AppState`.
    fn handle_key(key: &KeyEvent, input: &mut InputState) -> Result<Handled> {
        if !input.has_completion() {
            return Ok(Handled::IGNORED);
        }
        match key.code {
            KeyCode::Down => {
                if let Some(completion) = input.completion_mut() {
                    completion.select_next();
                }
                Ok(Handled::CONSUMED)
            }
            KeyCode::Up => {
                if let Some(completion) = input.completion_mut() {
                    completion.select_previous();
                }
                Ok(Handled::CONSUMED)
            }
            KeyCode::Tab | KeyCode::Enter => {
                input.accept_completion();
                Ok(Handled::CONSUMED)
            }
            KeyCode::Esc => {
                input.dismiss_completion();
                Ok(Handled::CONSUMED)
            }
            _ => Ok(Handled::IGNORED),
        }
    }
}

impl EventHandler for CompletionHandler {
    fn handle<'a>(
        &'a mut self,
        event: &'a AppEvent,
        ctx: &'a mut AppContext<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<Handled>> + Send + 'a>> {
        Box::pin(async move {
            let AppEvent::Key(key) = event else {
                return Ok(Handled::IGNORED);
            };
            Self::handle_key(key, ctx.state.input_state_mut())
        })
    }

    fn name(&self) -> &str {
        "completion"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{AnthropicClient, LlmProvider};
    use crate::app::context::AppContext;
    use crate::app::dispatch::EventHandler;
    use crate::app::events::AppEvent;
    use crate::app::state::AppState;
    use crate::session::SessionManager;
    use crate::types::config::ParallelMode;
    use crate::types::ui_state::{CompletionEntry, CompletionSource, CompletionState};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use secrecy::SecretString;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn make_key(code: KeyCode) -> AppEvent {
        AppEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn make_state() -> AppState {
        AppState::new(PathBuf::from("/test"), true, ParallelMode::Disabled)
    }

    fn test_client() -> Arc<dyn LlmProvider> {
        Arc::new(AnthropicClient::new(
            SecretString::from("test-key"),
            "claude-test",
        ))
    }

    fn test_session_manager() -> (SessionManager, TempDir) {
        let dir = TempDir::new().expect("failed to create temp dir");
        let mgr = SessionManager::new(dir.path().to_path_buf());
        (mgr, dir)
    }

    // 8.4.1 - Handler basics

    #[test]
    fn handler_name() {
        let handler = CompletionHandler;
        assert_eq!(handler.name(), "completion");
    }

    #[tokio::test]
    async fn handler_ignores_all_events_when_no_completion() {
        let mut handler = CompletionHandler;
        let mut state = make_state();
        let client = test_client();
        let (sm, _dir) = test_session_manager();
        let mut ctx = AppContext {
            client: Arc::clone(&client),
            state: &mut state,
            session_manager: &sm,
        };

        let event = make_key(KeyCode::Down);
        let result = handler.handle(&event, &mut ctx).await.unwrap();
        assert!(result.is_ignored());
    }

    #[test]
    fn handler_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<CompletionHandler>();
    }

    // 8.4.2 - Navigation

    #[tokio::test]
    async fn handler_down_selects_next() {
        let mut handler = CompletionHandler;
        let mut state = make_state();
        state.insert_char('/');
        assert!(state.has_completion());
        let initial = state.completion().unwrap().selected_index();

        let client = test_client();
        let (sm, _dir) = test_session_manager();
        let mut ctx = AppContext {
            client: Arc::clone(&client),
            state: &mut state,
            session_manager: &sm,
        };

        let event = make_key(KeyCode::Down);
        let result = handler.handle(&event, &mut ctx).await.unwrap();
        assert!(result.is_consumed());
        assert_eq!(
            ctx.state.completion().unwrap().selected_index(),
            initial + 1
        );
    }

    #[tokio::test]
    async fn handler_up_selects_previous() {
        let mut handler = CompletionHandler;
        let mut state = make_state();
        state.insert_char('/');
        // Move down first so we can go up
        state.completion_mut().unwrap().select_next();
        assert_eq!(state.completion().unwrap().selected_index(), 1);

        let client = test_client();
        let (sm, _dir) = test_session_manager();
        let mut ctx = AppContext {
            client: Arc::clone(&client),
            state: &mut state,
            session_manager: &sm,
        };

        let event = make_key(KeyCode::Up);
        let result = handler.handle(&event, &mut ctx).await.unwrap();
        assert!(result.is_consumed());
        assert_eq!(ctx.state.completion().unwrap().selected_index(), 0);
    }

    #[tokio::test]
    async fn handler_esc_dismisses() {
        let mut handler = CompletionHandler;
        let mut state = make_state();
        state.insert_char('/');
        assert!(state.has_completion());

        let client = test_client();
        let (sm, _dir) = test_session_manager();
        let mut ctx = AppContext {
            client: Arc::clone(&client),
            state: &mut state,
            session_manager: &sm,
        };

        let event = make_key(KeyCode::Esc);
        let result = handler.handle(&event, &mut ctx).await.unwrap();
        assert!(result.is_consumed());
        assert!(!ctx.state.has_completion());
    }

    // 8.4.3 - Accept

    #[tokio::test]
    async fn handler_tab_accepts() {
        let mut handler = CompletionHandler;
        let mut state = make_state();
        state.insert_char('/');
        let expected_name = state
            .completion()
            .unwrap()
            .selected_entry()
            .unwrap()
            .name
            .clone();

        let client = test_client();
        let (sm, _dir) = test_session_manager();
        let mut ctx = AppContext {
            client: Arc::clone(&client),
            state: &mut state,
            session_manager: &sm,
        };

        let event = make_key(KeyCode::Tab);
        let result = handler.handle(&event, &mut ctx).await.unwrap();
        assert!(result.is_consumed());
        assert!(!ctx.state.has_completion());
        assert_eq!(ctx.state.input_state().text(), format!("/{expected_name} "));
    }

    #[tokio::test]
    async fn handler_enter_accepts() {
        let mut handler = CompletionHandler;
        let mut state = make_state();
        state.insert_char('/');

        let client = test_client();
        let (sm, _dir) = test_session_manager();
        let mut ctx = AppContext {
            client: Arc::clone(&client),
            state: &mut state,
            session_manager: &sm,
        };

        let event = make_key(KeyCode::Enter);
        let result = handler.handle(&event, &mut ctx).await.unwrap();
        assert!(result.is_consumed());
        assert!(!ctx.state.has_completion());
        assert!(ctx.state.input_state().text().starts_with('/'));
    }

    #[tokio::test]
    async fn handler_char_keys_pass_through() {
        let mut handler = CompletionHandler;
        let mut state = make_state();
        state.insert_char('/');

        let client = test_client();
        let (sm, _dir) = test_session_manager();
        let mut ctx = AppContext {
            client: Arc::clone(&client),
            state: &mut state,
            session_manager: &sm,
        };

        let event = make_key(KeyCode::Char('a'));
        let result = handler.handle(&event, &mut ctx).await.unwrap();
        assert!(result.is_ignored()); // passes through to KeyboardHandler
    }

    #[tokio::test]
    async fn handler_non_key_events_ignored() {
        let mut handler = CompletionHandler;
        let mut state = make_state();
        state.insert_char('/');

        let client = test_client();
        let (sm, _dir) = test_session_manager();
        let mut ctx = AppContext {
            client: Arc::clone(&client),
            state: &mut state,
            session_manager: &sm,
        };

        let event = AppEvent::Tick;
        let result = handler.handle(&event, &mut ctx).await.unwrap();
        assert!(result.is_ignored());
    }

    // =========================================================================
    // Narrow tests: handle_key with InputState only (no AppState)
    // =========================================================================

    /// Helper: builds a sample `CompletionState` with two entries.
    fn sample_completion() -> CompletionState {
        CompletionState::new(vec![
            CompletionEntry::new("help", "Show help", CompletionSource::Builtin),
            CompletionEntry::new("mcp", "MCP status", CompletionSource::Builtin),
        ])
    }

    #[test]
    fn handle_key_ignores_when_no_completion() {
        let mut input = InputState::new();
        let key = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        let result = CompletionHandler::handle_key(&key, &mut input).unwrap();
        assert!(result.is_ignored());
    }

    #[test]
    fn handle_key_down_selects_next() {
        let mut input = InputState::new();
        input.set_completion(sample_completion());
        let initial = input.completion().unwrap().selected_index();

        let key = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        let result = CompletionHandler::handle_key(&key, &mut input).unwrap();

        assert!(result.is_consumed());
        assert_eq!(input.completion().unwrap().selected_index(), initial + 1);
    }

    #[test]
    fn handle_key_esc_dismisses_completion() {
        let mut input = InputState::new();
        input.set_completion(sample_completion());
        assert!(input.has_completion());

        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        let result = CompletionHandler::handle_key(&key, &mut input).unwrap();

        assert!(result.is_consumed());
        assert!(!input.has_completion());
    }

    #[test]
    fn handle_key_tab_accepts_completion() {
        let mut input = InputState::new();
        input.set_text("/".to_string());
        input.set_completion(sample_completion());
        assert!(input.has_completion());

        let key = KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);
        let result = CompletionHandler::handle_key(&key, &mut input).unwrap();

        assert!(result.is_consumed());
        assert!(!input.has_completion());
        assert!(input.text().starts_with('/'));
    }

    #[test]
    fn handle_key_char_passes_through_with_completion() {
        let mut input = InputState::new();
        input.set_completion(sample_completion());

        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        let result = CompletionHandler::handle_key(&key, &mut input).unwrap();

        assert!(result.is_ignored());
    }
}
