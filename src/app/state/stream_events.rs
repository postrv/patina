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
        self.dirty.messages = true;
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
        self.dirty.messages = true;
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
        }
        self.tool_state.handle_message_complete(stop_reason)?;
        self.display.loading = false;
        self.streaming_rx = None;
        self.dirty.messages = true;
        Ok(())
    }

    /// Handles a stream `Error` event by logging the error and resetting
    /// the streaming state.
    fn handle_stream_error(&mut self, error: String) {
        tracing::error!("Stream error: {}", error);
        self.display.loading = false;
        self.streaming_rx = None;
        self.dirty.messages = true;
    }
}
