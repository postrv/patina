//! Stream handler for API chunk processing and tool result handling.
//!
//! [`StreamHandler`] processes [`AppEvent::ApiChunk`] and [`AppEvent::ToolResult`]
//! events, delegating to [`AppState::append_chunk`] and
//! [`AppState::record_tool_result`] respectively. It detects message completion,
//! tool-use stop reasons, and all-tools-complete conditions to drive the
//! streaming / tool execution cycle.

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use tracing::debug;

use crate::api::StreamEvent;
use crate::app::context::AppContext;
use crate::app::dispatch::{EventHandler, Handled};
use crate::app::events::AppEvent;
use crate::app::tool_loop::ToolLoopState;

/// Handles API streaming chunks and tool execution results.
///
/// `StreamHandler` is responsible for the streaming / tool execution cycle:
///
/// 1. **API chunks** are delegated to [`AppState::append_chunk`], which
///    updates the timeline and tool loop state.
/// 2. **Message completion** marks the session dirty so
///    [`SessionHandler`](super::session::SessionHandler) persists the state.
/// 3. **Tool-use stop reason** triggers tool execution via
///    [`AppContext::start_tool_execution`](crate::app::context::AppContext::start_tool_execution).
/// 4. **Tool results** are recorded via [`AppState::record_tool_result`].
/// 5. **All tools complete** triggers continuation streaming via
///    [`AppContext::finish_tool_execution_and_continue`](crate::app::context::AppContext::finish_tool_execution_and_continue).
///
/// # Examples
///
/// ```rust,ignore
/// use patina::app::handlers::stream::StreamHandler;
/// use patina::app::dispatch::EventDispatcher;
///
/// let dispatcher = EventDispatcher::new(vec![
///     Box::new(StreamHandler),
///     Box::new(tick_handler),
///     Box::new(session_handler),
/// ]);
/// ```
pub struct StreamHandler;

impl EventHandler for StreamHandler {
    fn handle<'a>(
        &'a mut self,
        event: &'a AppEvent,
        ctx: &'a mut AppContext<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<Handled>> + Send + 'a>> {
        Box::pin(async move {
            match event {
                AppEvent::ApiChunk(chunk) => {
                    // Check completion flags before consuming the chunk.
                    let is_message_complete = matches!(
                        chunk,
                        StreamEvent::MessageStop | StreamEvent::MessageComplete { .. }
                    );
                    let is_tool_use_complete = matches!(
                        chunk,
                        StreamEvent::MessageComplete { stop_reason }
                            if stop_reason.needs_tool_execution()
                    );

                    // Delegate chunk processing to state.
                    ctx.state.append_chunk(chunk.clone())?;

                    // Mark session dirty after message completion so
                    // SessionHandler persists the conversation.
                    if is_message_complete {
                        ctx.state.session_tracking_mut().mark_dirty();
                    }

                    // Trigger tool execution if the stop reason requires it.
                    if is_tool_use_complete {
                        ctx.start_tool_execution()?;
                    }

                    Ok(Handled::CONSUMED)
                }
                AppEvent::ToolResult { tool_id, result } => {
                    debug!(tool_id = %tool_id, is_error = result.is_error, "Tool result received");

                    ctx.state.record_tool_result(tool_id, result.clone());

                    // When all tools have completed and the tool loop is in
                    // Executing state, set up continuation streaming.
                    let is_executing =
                        matches!(ctx.state.tool_loop_state(), ToolLoopState::Executing);
                    if is_executing && ctx.state.all_tools_complete() {
                        debug!("All tools complete, setting up continuation");
                        ctx.state.clear_tool_result_rx();
                        ctx.finish_tool_execution_and_continue().await?;
                    }

                    Ok(Handled::CONSUMED)
                }
                _ => Ok(Handled::IGNORED),
            }
        })
    }

    fn name(&self) -> &str {
        "stream"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::StreamEvent;
    use crate::api::{AnthropicClient, LlmProvider};
    use crate::app::state::AppState;
    use crate::session::SessionManager;
    use crate::types::config::ParallelMode;
    use crate::types::{StopReason, ToolResultBlock};
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
    // StreamHandler::name
    // =========================================================================

    #[test]
    fn name_returns_stream() {
        let handler = StreamHandler;
        assert_eq!(handler.name(), "stream");
    }

    // =========================================================================
    // StreamHandler::handle — ApiChunk events must be consumed
    // =========================================================================

    #[tokio::test]
    async fn handle_api_chunk_content_delta_returns_consumed() {
        let mut handler = StreamHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::ApiChunk(StreamEvent::ContentDelta("hello".to_string()));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "StreamHandler must consume ApiChunk(ContentDelta) events"
        );
    }

    #[tokio::test]
    async fn handle_api_chunk_message_stop_returns_consumed() {
        let mut handler = StreamHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::ApiChunk(StreamEvent::MessageStop);
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "StreamHandler must consume ApiChunk(MessageStop) events"
        );
    }

    #[tokio::test]
    async fn handle_api_chunk_message_complete_end_turn_returns_consumed() {
        let mut handler = StreamHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::ApiChunk(StreamEvent::MessageComplete {
            stop_reason: StopReason::EndTurn,
        });
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "StreamHandler must consume ApiChunk(MessageComplete) events"
        );
    }

    #[tokio::test]
    async fn handle_api_chunk_error_returns_consumed() {
        let mut handler = StreamHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::ApiChunk(StreamEvent::Error("test error".to_string()));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "StreamHandler must consume ApiChunk(Error) events"
        );
    }

    #[tokio::test]
    async fn handle_api_chunk_tool_use_start_returns_consumed() {
        let mut handler = StreamHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::ApiChunk(StreamEvent::ToolUseStart {
            id: "toolu_123".to_string(),
            name: "bash".to_string(),
            index: 0,
        });
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "StreamHandler must consume ApiChunk(ToolUseStart) events"
        );
    }

    #[tokio::test]
    async fn handle_api_chunk_content_block_complete_returns_consumed() {
        let mut handler = StreamHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::ApiChunk(StreamEvent::ContentBlockComplete { index: 0 });
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "StreamHandler must consume ApiChunk(ContentBlockComplete) events"
        );
    }

    // =========================================================================
    // StreamHandler::handle — ToolResult events must be consumed
    // =========================================================================

    #[tokio::test]
    async fn handle_tool_result_returns_consumed() {
        let mut handler = StreamHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // Set up a tool as executing so record_tool_result has something to remove.
        state.mark_tool_executing("toolu_test");

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::ToolResult {
            tool_id: "toolu_test".to_string(),
            result: ToolResultBlock {
                tool_use_id: "toolu_test".to_string(),
                content: "output".to_string(),
                is_error: false,
            },
        };
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "StreamHandler must consume ToolResult events"
        );
    }

    // =========================================================================
    // StreamHandler::handle — non-streaming events must be ignored
    // =========================================================================

    #[tokio::test]
    async fn handle_key_returns_ignored() {
        let mut handler = StreamHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::IGNORED,
            "StreamHandler must ignore Key events"
        );
    }

    #[tokio::test]
    async fn handle_tick_returns_ignored() {
        let mut handler = StreamHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Tick;
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::IGNORED,
            "StreamHandler must ignore Tick events"
        );
    }

    #[tokio::test]
    async fn handle_quit_returns_ignored() {
        let mut handler = StreamHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Quit;
        let result = handler.handle(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::IGNORED,
            "StreamHandler must ignore Quit events"
        );
    }

    #[tokio::test]
    async fn handle_resize_returns_ignored() {
        let mut handler = StreamHandler;

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
            "StreamHandler must ignore Resize events"
        );
    }

    // =========================================================================
    // StreamHandler::handle — state mutations on message completion
    // =========================================================================

    #[tokio::test]
    async fn handle_api_chunk_message_stop_clears_loading() {
        let mut handler = StreamHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_loading(true);

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::ApiChunk(StreamEvent::MessageStop);
        let _result = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(
            !ctx.state.is_loading(),
            "MessageStop must clear the loading flag"
        );
    }

    #[tokio::test]
    async fn handle_api_chunk_message_complete_end_turn_clears_loading() {
        let mut handler = StreamHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_loading(true);

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::ApiChunk(StreamEvent::MessageComplete {
            stop_reason: StopReason::EndTurn,
        });
        let _result = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(
            !ctx.state.is_loading(),
            "MessageComplete(EndTurn) must clear the loading flag"
        );
    }

    #[tokio::test]
    async fn handle_api_chunk_error_clears_loading() {
        let mut handler = StreamHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.set_loading(true);

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::ApiChunk(StreamEvent::Error("test error".to_string()));
        let _result = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(!ctx.state.is_loading(), "Error must clear the loading flag");
    }

    #[tokio::test]
    async fn handle_api_chunk_message_stop_marks_session_dirty() {
        let mut handler = StreamHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::ApiChunk(StreamEvent::MessageStop);
        let _result = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(
            ctx.state.session_tracking_mut().take_dirty(),
            "MessageStop must mark the session as dirty for auto-save"
        );
    }

    #[tokio::test]
    async fn handle_api_chunk_message_complete_marks_session_dirty() {
        let mut handler = StreamHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::ApiChunk(StreamEvent::MessageComplete {
            stop_reason: StopReason::EndTurn,
        });
        let _result = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(
            ctx.state.session_tracking_mut().take_dirty(),
            "MessageComplete must mark the session as dirty for auto-save"
        );
    }

    // =========================================================================
    // StreamHandler::handle — tool result state mutations
    // =========================================================================

    #[tokio::test]
    async fn handle_tool_result_removes_from_executing() {
        let mut handler = StreamHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        state.mark_tool_executing("toolu_abc");
        assert!(state.has_executing_tools());

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::ToolResult {
            tool_id: "toolu_abc".to_string(),
            result: ToolResultBlock {
                tool_use_id: "toolu_abc".to_string(),
                content: "done".to_string(),
                is_error: false,
            },
        };
        let _result = handler.handle(&event, &mut ctx).await.unwrap();

        assert!(
            !ctx.state.has_executing_tools(),
            "ToolResult must remove the tool from the executing set"
        );
    }

    #[tokio::test]
    async fn handle_tool_result_does_not_crash_without_executing_tool() {
        let mut handler = StreamHandler;

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // No tools marked as executing — handler must still succeed.
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::ToolResult {
            tool_id: "toolu_orphan".to_string(),
            result: ToolResultBlock {
                tool_use_id: "toolu_orphan".to_string(),
                content: "orphan result".to_string(),
                is_error: false,
            },
        };
        let result = handler.handle(&event, &mut ctx).await;

        assert!(
            result.is_ok(),
            "StreamHandler must handle orphan tool results gracefully"
        );
    }

    // =========================================================================
    // Send bound verification
    // =========================================================================

    #[test]
    fn stream_handler_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<StreamHandler>();
    }

    // =========================================================================
    // Integration with EventDispatcher
    // =========================================================================

    #[tokio::test]
    async fn stream_handler_works_in_dispatcher() {
        use crate::app::dispatch::EventDispatcher;

        let handler = StreamHandler;
        let mut dispatcher = EventDispatcher::new(vec![Box::new(handler)]);

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();
        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        // ApiChunk should be consumed.
        let event = AppEvent::ApiChunk(StreamEvent::ContentDelta("test".to_string()));
        let result = dispatcher.dispatch(&event, &mut ctx).await.unwrap();
        assert_eq!(
            result,
            Handled::CONSUMED,
            "StreamHandler should consume ApiChunk in dispatcher"
        );

        // Key event should pass through.
        let key = AppEvent::Key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        let result = dispatcher.dispatch(&key, &mut ctx).await.unwrap();
        assert_eq!(
            result,
            Handled::IGNORED,
            "StreamHandler should ignore Key in dispatcher"
        );
    }
}
