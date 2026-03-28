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
                self.thinking_buffer.clear();
                tracing::debug!("Thinking block started");
            }
            StreamEvent::ThinkingDelta(text) => {
                self.thinking_buffer.push_str(&text);
            }
            StreamEvent::ThinkingComplete { .. } => {
                if !self.thinking_buffer.is_empty() {
                    let thinking_text = std::mem::take(&mut self.thinking_buffer);
                    let token_count = crate::api::tokens::estimate_tokens(&thinking_text);
                    tracing::info!("Thinking complete: {} tokens", token_count);
                    self.timeline
                        .push_assistant_message(format!("[Thought for ~{} tokens]", token_count));
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
        self.timeline.append_to_streaming(&text);
        self.tool_state.tool_loop.append_text(&text);
        self.dirty.full = true;
    }

    /// Handles a `MessageStop` stream event by finalizing the streaming entry
    /// and adding the assistant message to the API conversation history.
    ///
    /// Only processes if currently streaming, to prevent duplicates when
    /// `MessageComplete` has already handled finalization.
    fn handle_message_stop(&mut self) {
        if self.timeline.is_streaming() {
            self.timeline.finalize_streaming_as_message();
            if let Some(crate::types::ConversationEntry::AssistantMessage(text)) =
                self.timeline.entries().last()
            {
                self.api_messages.push(ApiMessageV2::assistant(text));
            }
        }
        self.display.loading = false;
        self.streaming_rx = None;
        self.dirty.full = true;

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
            self.timeline.finalize_streaming_for_tool_use();
            tracing::debug!("Tool use response - text stored in tool_loop, not adding to API yet");
        } else {
            self.timeline.finalize_streaming_as_message();
            if let Some(crate::types::ConversationEntry::AssistantMessage(text)) =
                self.timeline.entries().last()
            {
                self.api_messages.push(ApiMessageV2::assistant(text));
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
        self.display.loading = false;
        self.streaming_rx = None;
        self.dirty.full = true;

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
    /// `StopFailure` hook, and resetting the streaming state.
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

        self.display.loading = false;
        self.streaming_rx = None;
        self.dirty.full = true;
    }
}
