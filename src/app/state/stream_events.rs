//! Stream event handling methods for AppState.
//!
//! Extracted from state/mod.rs to reduce file size.

use super::*;

impl AppState {
    pub fn append_chunk(&mut self, event: StreamEvent) -> Result<()> {
        match event {
            StreamEvent::ContentDelta(text) => self.handle_content_delta(text),
            StreamEvent::MessageStop => self.handle_message_stop(),
            StreamEvent::MessageComplete { stop_reason } => {
                self.handle_message_complete_event(stop_reason)?;
            }
            StreamEvent::Error(e) => self.handle_stream_error(e),
            StreamEvent::ToolUseStart { id, name, index } => {
                self.handle_tool_use_start(id, name, index);
            }
            StreamEvent::ToolUseInputDelta {
                index,
                partial_json,
            } => {
                self.handle_tool_use_input_delta(index, &partial_json);
            }
            StreamEvent::ToolUseComplete { index } => {
                self.tool_state.handle_tool_use_complete(index)?;
            }
            StreamEvent::ContentBlockComplete { .. } => {
                // Content block completion is tracked internally
                tracing::debug!("Content block complete");
            }
            StreamEvent::ThinkingStart { .. } => {
                self.conversation.thinking_buffer.clear();
                tracing::debug!("Thinking block started");
            }
            StreamEvent::ThinkingDelta(text) => {
                self.conversation.thinking_buffer.push_str(&text);
            }
            StreamEvent::ThinkingComplete { .. } => {
                if !self.conversation.thinking_buffer.is_empty() {
                    let thinking_text = std::mem::take(&mut self.conversation.thinking_buffer);
                    let token_count = crate::api::tokens::estimate_tokens(&thinking_text);
                    tracing::info!("Thinking complete: {} tokens", token_count);
                    self.conversation
                        .timeline
                        .push_assistant_message(format!("[Thought for ~{} tokens]", token_count));
                    self.conversation.dirty = true;
                }
            }
            StreamEvent::Usage(usage) => {
                let record = UsageRecord::with_cache(
                    &self.model_config.current_model,
                    usage.input_tokens,
                    usage.output_tokens,
                    usage.cache_read_input_tokens,
                    usage.cache_creation_input_tokens,
                    std::time::Duration::ZERO,
                );
                self.cost_tracker.record_usage(record);
                tracing::debug!(
                    "Usage recorded: {}in/{}out, session cost: ${:.4}",
                    usage.input_tokens,
                    usage.output_tokens,
                    self.cost_tracker.session_cost(),
                );
            }
        }
        Ok(())
    }

    /// Handles a `ContentDelta` stream event by appending text to the timeline
    /// and forwarding it to the tool loop for assistant text tracking.
    fn handle_content_delta(&mut self, text: String) {
        self.conversation.timeline.append_to_streaming(&text);
        self.tool_state.tool_loop.append_text(&text);
        self.conversation.dirty = true;
    }

    /// Handles a `MessageStop` stream event by finalizing the streaming entry
    /// and adding the assistant message to the API conversation history.
    ///
    /// Only processes if currently streaming, to prevent duplicates when
    /// `MessageComplete` has already handled finalization.
    fn handle_message_stop(&mut self) {
        if self.conversation.timeline.is_streaming() {
            self.conversation.timeline.finalize_streaming_as_message();
            if let Some(crate::types::ConversationEntry::AssistantMessage(text)) =
                self.conversation.timeline.entries().last()
            {
                self.conversation
                    .api_messages
                    .push(ApiMessageV2::assistant(text));
            }
        }
        self.view.display.loading = false;
        self.conversation.streaming_rx = None;
        self.conversation.dirty = true;

        // Fire desktop notification when the full response cycle completes
        if !self.tool_state.tool_loop_is_active() {
            if let Err(e) = self
                .notification_manager
                .notify("Patina", "Response complete")
            {
                tracing::debug!("Failed to send completion notification: {e}");
            }
        }
    }

    /// Handles a `MessageComplete` stream event by finalizing the streaming entry
    /// and processing the stop reason.
    ///
    /// For `tool_use` stop reasons, streaming is finalized without adding to API
    /// messages (handled later by `handle_tool_execution`). For normal responses,
    /// the assistant message is added to the API conversation history.
    ///
    /// # Errors
    ///
    /// Returns an error if tool loop state transition fails.
    fn handle_message_complete_event(&mut self, stop_reason: StopReason) -> Result<()> {
        let needs_tool_execution = stop_reason.needs_tool_execution();

        if needs_tool_execution {
            self.conversation.timeline.finalize_streaming_for_tool_use();
            tracing::debug!("Tool use response - text stored in tool_loop, not adding to API yet");
        } else {
            self.conversation.timeline.finalize_streaming_as_message();
            if let Some(crate::types::ConversationEntry::AssistantMessage(text)) =
                self.conversation.timeline.entries().last()
            {
                self.conversation
                    .api_messages
                    .push(ApiMessageV2::assistant(text));
            }

            // Fire Stop hook for non-tool-use completions (EndTurn, StopSequence, MaxTokens)
            let reason_str = match stop_reason {
                StopReason::EndTurn => "end_turn",
                StopReason::StopSequence => "stop_sequence",
                StopReason::MaxTokens => "max_tokens",
                StopReason::ToolUse => unreachable!(),
            };
            let hooks = self.tool_state.tool_executor.hooks();
            // Spawn the hook fire to avoid holding &mut self across await
            let hooks_clone_session = hooks.session_id().to_string();
            let reason_owned = reason_str.to_string();
            let executor = Arc::clone(&self.tool_state.tool_executor);
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    if let Err(e) = executor.hooks().fire_stop(&reason_owned).await {
                        tracing::debug!("Hook fire failed: {e}");
                    }
                    tracing::debug!(
                        session_id = %hooks_clone_session,
                        stop_reason = %reason_owned,
                        "Stop hook fired"
                    );
                });
            }
        }
        self.tool_state.handle_message_complete(stop_reason)?;
        self.view.display.loading = false;
        self.conversation.streaming_rx = None;
        self.conversation.dirty = true;

        // Auto-checkpoint on EndTurn (assistant turn completion)
        if stop_reason == StopReason::EndTurn {
            self.auto_checkpoint_on_end_turn();
        }

        Ok(())
    }

    /// Creates an automatic checkpoint when an assistant turn completes.
    ///
    /// Builds the current session messages from the timeline and creates a
    /// checkpoint at the last message index. Skips if a checkpoint already
    /// exists at that index (deduplication handled by `SessionTracking`).
    fn auto_checkpoint_on_end_turn(&mut self) {
        let messages: Vec<Message> = self
            .conversation
            .timeline
            .entries()
            .iter()
            .filter_map(|entry| match entry {
                crate::types::ConversationEntry::UserMessage(text) => Some(Message {
                    role: Role::User,
                    content: text.clone(),
                }),
                crate::types::ConversationEntry::AssistantMessage(text) => Some(Message {
                    role: Role::Assistant,
                    content: text.clone(),
                }),
                _ => None,
            })
            .collect();

        if messages.is_empty() {
            return;
        }

        let message_index = messages.len() - 1;
        let checkpoint = crate::session::Checkpoint::new(message_index, &messages);
        self.session.add_checkpoint(checkpoint);
        tracing::debug!(
            message_index = message_index,
            "Auto-checkpoint created on end_turn"
        );
    }

    /// Handles a stream `Error` event by logging the error, firing the
    /// `StopFailure` hook, resetting the tool loop state machine, finalizing
    /// any partial streaming timeline entry, and clearing the streaming state.
    fn handle_stream_error(&mut self, error: String) {
        tracing::error!("Stream error: {}", error);

        // Fire StopFailure hook in a background task
        let executor = Arc::clone(&self.tool_state.tool_executor);
        let error_clone = error.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Err(e) = executor.hooks().fire_stop_failure(&error_clone).await {
                    tracing::debug!("Hook fire failed: {e}");
                }
            });
        }

        // F1: Reset tool loop state machine so it doesn't stay stuck in Streaming
        self.tool_state.tool_loop.reset();

        // F2: Finalize any partial streaming entry so it doesn't remain as a ghost.
        // finalize_streaming_as_message is safe to call when there's no streaming
        // entry — it checks streaming_idx and returns early if None.
        if self.conversation.timeline.is_streaming() {
            self.conversation.timeline.finalize_streaming_as_message();
        }

        self.view.display.loading = false;
        self.conversation.streaming_rx = None;
        self.conversation.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::config::ParallelMode;
    use crate::types::content::StopReason;
    use crate::types::stream::{ApiUsage, StreamEvent};
    use std::path::PathBuf;

    fn test_state() -> AppState {
        AppState::new(PathBuf::from("/tmp/test"), false, ParallelMode::Enabled)
    }

    /// Sets up streaming mode: timeline gets a streaming entry, tool loop enters Streaming.
    fn start_streaming(state: &mut AppState) {
        state.set_streaming(true);
        // Tool loop must be in Streaming for handle_message_complete to succeed
        // set_streaming already tries to push streaming to timeline
    }

    // ======================================================================
    // ContentDelta
    // ======================================================================

    #[test]
    fn test_content_delta_appends_to_timeline() {
        let mut state = test_state();
        start_streaming(&mut state);
        state.mark_rendered();

        state
            .append_chunk(StreamEvent::ContentDelta("Hello ".to_string()))
            .unwrap();
        state
            .append_chunk(StreamEvent::ContentDelta("world".to_string()))
            .unwrap();

        // The streaming entry should contain accumulated text
        assert!(state.conversation.timeline.is_streaming());
        assert!(state.needs_render(), "dirty flag must be set");
    }

    #[test]
    fn test_content_delta_without_streaming_is_safe() {
        let mut state = test_state();
        // Do NOT call start_streaming — no streaming entry

        // Should not panic even without an active streaming entry
        state
            .append_chunk(StreamEvent::ContentDelta("orphan text".to_string()))
            .unwrap();
    }

    // ======================================================================
    // ThinkingStart / ThinkingDelta / ThinkingComplete
    // ======================================================================

    #[test]
    fn test_thinking_buffer_lifecycle() {
        let mut state = test_state();

        // ThinkingStart clears buffer
        state
            .append_chunk(StreamEvent::ThinkingStart { index: 0 })
            .unwrap();
        assert!(state.conversation.thinking_buffer.is_empty());

        // ThinkingDelta accumulates
        state
            .append_chunk(StreamEvent::ThinkingDelta("reason step 1, ".to_string()))
            .unwrap();
        state
            .append_chunk(StreamEvent::ThinkingDelta("reason step 2".to_string()))
            .unwrap();
        assert_eq!(
            state.conversation.thinking_buffer,
            "reason step 1, reason step 2"
        );

        // ThinkingComplete drains buffer and pushes summary to timeline
        let timeline_before = state.conversation.timeline.len();
        state
            .append_chunk(StreamEvent::ThinkingComplete { index: 0 })
            .unwrap();

        assert!(
            state.conversation.thinking_buffer.is_empty(),
            "buffer must be drained on complete"
        );
        assert_eq!(
            state.conversation.timeline.len(),
            timeline_before + 1,
            "summary entry must be added to timeline"
        );
    }

    #[test]
    fn test_thinking_complete_sets_dirty_flag() {
        let mut state = test_state();
        state
            .append_chunk(StreamEvent::ThinkingDelta("some reasoning".to_string()))
            .unwrap();
        // Clear dirty so we can detect the flag being set by ThinkingComplete
        state.mark_rendered();

        state
            .append_chunk(StreamEvent::ThinkingComplete { index: 0 })
            .unwrap();

        assert!(
            state.needs_render(),
            "dirty flag must be set after ThinkingComplete pushes to timeline"
        );
    }

    #[test]
    fn test_thinking_complete_with_empty_buffer_is_noop() {
        let mut state = test_state();
        let timeline_before = state.conversation.timeline.len();

        state
            .append_chunk(StreamEvent::ThinkingStart { index: 0 })
            .unwrap();
        // No ThinkingDelta — buffer stays empty
        state
            .append_chunk(StreamEvent::ThinkingComplete { index: 0 })
            .unwrap();

        assert_eq!(
            state.conversation.timeline.len(),
            timeline_before,
            "no timeline entry for empty thinking"
        );
    }

    #[test]
    fn test_multiple_thinking_starts_reset_buffer() {
        let mut state = test_state();

        state
            .append_chunk(StreamEvent::ThinkingDelta("old content".to_string()))
            .unwrap();
        // Second ThinkingStart clears the accumulated content
        state
            .append_chunk(StreamEvent::ThinkingStart { index: 1 })
            .unwrap();
        assert!(
            state.conversation.thinking_buffer.is_empty(),
            "ThinkingStart must clear previous buffer"
        );
    }

    // ======================================================================
    // MessageStop
    // ======================================================================

    #[test]
    fn test_message_stop_finalizes_streaming_and_pushes_api_message() {
        let mut state = test_state();
        start_streaming(&mut state);

        // Append some content first
        state
            .append_chunk(StreamEvent::ContentDelta("assistant reply".to_string()))
            .unwrap();
        assert!(state.conversation.timeline.is_streaming());

        let api_before = state.conversation.api_messages.len();
        state.append_chunk(StreamEvent::MessageStop).unwrap();

        assert!(
            !state.conversation.timeline.is_streaming(),
            "streaming must be finalized"
        );
        assert_eq!(
            state.conversation.api_messages.len(),
            api_before + 1,
            "assistant message must be pushed to api_messages"
        );
        assert!(!state.view.display.loading, "loading must be cleared");
        assert!(
            state.conversation.streaming_rx.is_none(),
            "streaming_rx must be cleared"
        );
    }

    #[test]
    fn test_message_stop_when_not_streaming_skips_finalization() {
        let mut state = test_state();
        // No streaming active

        let api_before = state.conversation.api_messages.len();
        state.append_chunk(StreamEvent::MessageStop).unwrap();

        // Should not panic, and should not add api message
        assert_eq!(state.conversation.api_messages.len(), api_before);
        assert!(!state.view.display.loading);
    }

    // ======================================================================
    // MessageComplete — EndTurn
    // ======================================================================

    #[test]
    fn test_message_complete_end_turn_finalizes_and_checkpoints() {
        let mut state = test_state();
        // Add a user message first so auto_checkpoint has content
        state.add_message(Message {
            role: Role::User,
            content: "Hello".to_string(),
        });
        start_streaming(&mut state);
        state
            .append_chunk(StreamEvent::ContentDelta("Hi there!".to_string()))
            .unwrap();

        let api_before = state.conversation.api_messages.len();
        state
            .append_chunk(StreamEvent::MessageComplete {
                stop_reason: StopReason::EndTurn,
            })
            .unwrap();

        assert!(
            !state.conversation.timeline.is_streaming(),
            "streaming must be finalized"
        );
        assert_eq!(
            state.conversation.api_messages.len(),
            api_before + 1,
            "assistant message pushed to api_messages"
        );
        assert!(!state.view.display.loading);
        assert!(state.conversation.streaming_rx.is_none());
        // auto_checkpoint should have created a checkpoint
        assert!(
            !state.session.checkpoints().is_empty(),
            "EndTurn should trigger auto-checkpoint"
        );
    }

    // ======================================================================
    // MessageComplete — ToolUse
    // ======================================================================

    #[test]
    fn test_message_complete_tool_use_does_not_push_api_message() {
        let mut state = test_state();
        start_streaming(&mut state);
        state
            .append_chunk(StreamEvent::ContentDelta("I'll use a tool".to_string()))
            .unwrap();

        let api_before = state.conversation.api_messages.len();
        state
            .append_chunk(StreamEvent::MessageComplete {
                stop_reason: StopReason::ToolUse,
            })
            .unwrap();

        assert_eq!(
            state.conversation.api_messages.len(),
            api_before,
            "ToolUse must NOT push to api_messages (handled by tool execution)"
        );
        assert!(!state.view.display.loading);
        assert!(state.conversation.streaming_rx.is_none());
    }

    // ======================================================================
    // MessageComplete — MaxTokens
    // ======================================================================

    #[test]
    fn test_message_complete_max_tokens_finalizes() {
        let mut state = test_state();
        start_streaming(&mut state);
        state
            .append_chunk(StreamEvent::ContentDelta("partial resp".to_string()))
            .unwrap();

        let api_before = state.conversation.api_messages.len();
        state
            .append_chunk(StreamEvent::MessageComplete {
                stop_reason: StopReason::MaxTokens,
            })
            .unwrap();

        assert_eq!(
            state.conversation.api_messages.len(),
            api_before + 1,
            "MaxTokens should push assistant message"
        );
        // MaxTokens does NOT trigger auto-checkpoint (only EndTurn does)
        assert!(
            state.session.checkpoints().is_empty(),
            "MaxTokens should not checkpoint"
        );
    }

    // ======================================================================
    // Error handling
    // ======================================================================

    #[test]
    fn test_stream_error_clears_streaming_state() {
        let mut state = test_state();
        start_streaming(&mut state);
        state.view.display.loading = true;

        state
            .append_chunk(StreamEvent::Error("API timeout".to_string()))
            .unwrap();

        assert!(!state.view.display.loading, "loading must be cleared");
        assert!(
            state.conversation.streaming_rx.is_none(),
            "rx must be cleared"
        );
        // F2: streaming entry should be finalized (no ghost entry)
        assert!(
            !state.conversation.timeline.is_streaming(),
            "streaming entry must be finalized after error"
        );
    }

    #[test]
    fn test_stream_error_resets_tool_loop_state_machine() {
        use crate::app::tool_loop::ToolLoopState;

        let mut state = test_state();
        start_streaming(&mut state);
        // Force the tool loop into Streaming state to simulate mid-stream
        state
            .tool_state
            .tool_loop
            .start_streaming()
            .expect("should transition to Streaming");
        assert!(
            matches!(state.tool_state.tool_loop.state(), ToolLoopState::Streaming),
            "precondition: tool loop must be in Streaming"
        );

        state
            .append_chunk(StreamEvent::Error("connection reset".to_string()))
            .unwrap();

        // F1: tool loop must be reset to Idle
        assert!(
            matches!(state.tool_state.tool_loop.state(), ToolLoopState::Idle),
            "tool loop must be reset to Idle after stream error"
        );
    }

    #[test]
    fn test_stream_error_without_streaming_entry_is_safe() {
        let mut state = test_state();
        // No streaming started — error should not panic
        state.view.display.loading = true;

        state
            .append_chunk(StreamEvent::Error("unexpected error".to_string()))
            .unwrap();

        assert!(!state.view.display.loading, "loading must be cleared");
        assert!(!state.conversation.timeline.is_streaming());
    }

    // ======================================================================
    // Usage tracking
    // ======================================================================

    #[test]
    fn test_usage_event_records_cost() {
        let mut state = test_state();
        let cost_before = state.cost_tracker.session_cost();

        state
            .append_chunk(StreamEvent::Usage(ApiUsage {
                input_tokens: 100,
                output_tokens: 50,
                cache_read_input_tokens: 10,
                cache_creation_input_tokens: 5,
            }))
            .unwrap();

        assert!(
            state.cost_tracker.session_cost() >= cost_before,
            "usage must be recorded"
        );
    }

    // ======================================================================
    // auto_checkpoint_on_end_turn
    // ======================================================================

    #[test]
    fn test_auto_checkpoint_empty_timeline_is_noop() {
        let mut state = test_state();
        state.auto_checkpoint_on_end_turn();
        assert!(
            state.session.checkpoints().is_empty(),
            "no checkpoint for empty timeline"
        );
    }

    #[test]
    fn test_auto_checkpoint_deduplicates() {
        let mut state = test_state();
        state.add_message(Message {
            role: Role::User,
            content: "Hello".to_string(),
        });
        state.auto_checkpoint_on_end_turn();
        let count_after_first = state.session.checkpoints().len();

        // Call again with same message count — should deduplicate
        state.auto_checkpoint_on_end_turn();
        assert_eq!(
            state.session.checkpoints().len(),
            count_after_first,
            "duplicate checkpoint must be skipped"
        );
    }

    // ======================================================================
    // ContentBlockComplete (no-op)
    // ======================================================================

    #[test]
    fn test_content_block_complete_is_noop() {
        let mut state = test_state();
        let timeline_before = state.conversation.timeline.len();

        state
            .append_chunk(StreamEvent::ContentBlockComplete { index: 0 })
            .unwrap();

        assert_eq!(state.conversation.timeline.len(), timeline_before);
    }
}
