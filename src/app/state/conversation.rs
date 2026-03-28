//! Conversation lifecycle methods for [`AppState`].
//!
//! Extracted from `state/mod.rs` to consolidate conversation-related logic:
//! message management, API message building, context injection, streaming
//! setup, and background event reception.

use super::*;

impl AppState {
    // ========================================================================
    // Message Management
    // ========================================================================

    /// Returns the display messages from the timeline for clipboard copy.
    #[must_use]
    pub fn messages(&self) -> Vec<Message> {
        self.conversation
            .timeline
            .entries()
            .iter()
            .filter_map(|entry| {
                let role = if entry.is_user() {
                    Role::User
                } else if entry.is_assistant() {
                    Role::Assistant
                } else {
                    return None;
                };
                entry.text().map(|text| Message {
                    role,
                    content: text.to_string(),
                })
            })
            .collect()
    }

    /// Adds a message to the conversation timeline and display.
    ///
    /// This updates the unified timeline and sets the dirty flag so the UI
    /// will re-render. Note: This only adds to the display timeline, not
    /// the API messages.
    pub fn add_message(&mut self, message: Message) {
        // Add to unified timeline based on role
        match message.role {
            Role::User => self
                .conversation
                .timeline
                .push_user_message(&message.content),
            Role::Assistant => self
                .conversation
                .timeline
                .push_assistant_message(&message.content),
        }
        self.conversation.dirty = true;
    }

    /// Returns the API messages for continuation.
    ///
    /// These messages include full content blocks (tool_use, tool_result)
    /// and should be used when sending to the API.
    #[must_use]
    pub fn api_messages(&self) -> &[ApiMessageV2] {
        &self.conversation.api_messages
    }

    /// Returns a mutable reference to the API messages.
    pub fn api_messages_mut(&mut self) -> &mut Vec<ApiMessageV2> {
        &mut self.conversation.api_messages
    }

    /// Returns the count of API messages.
    #[must_use]
    pub fn api_messages_len(&self) -> usize {
        self.conversation.api_messages.len()
    }

    // ========================================================================
    // API Message Building
    // ========================================================================

    /// Returns API messages truncated to fit within the token budget.
    ///
    /// This should be used when sending messages to the API instead of
    /// `api_messages()` directly to prevent context overflow and control costs.
    ///
    /// The truncation:
    /// - Always preserves the first message (system/project context)
    /// - Prioritizes recent messages over older ones
    /// - Respects the `DEFAULT_MAX_INPUT_TOKENS` limit
    ///
    /// # Returns
    ///
    /// A new vector containing the truncated message history.
    #[must_use]
    pub fn api_messages_truncated(&self) -> Vec<ApiMessageV2> {
        use crate::api::{truncate_context, DEFAULT_MAX_INPUT_TOKENS};
        truncate_context(&self.conversation.api_messages, DEFAULT_MAX_INPUT_TOKENS)
    }

    /// Prepares API messages for sending by compacting and truncating.
    ///
    /// This is the canonical method for obtaining the message list before
    /// any API call (both user-submitted messages and tool continuations).
    /// It combines two steps:
    /// 1. `maybe_compact_graceful()` — compresses old messages if context usage
    ///    exceeds the threshold
    /// 2. `api_messages_truncated()` — caps the result at `DEFAULT_MAX_INPUT_TOKENS`
    ///
    /// Using this method instead of `api_messages().to_vec()` prevents token
    /// explosion and spiralling costs during tool continuation loops.
    ///
    /// # Arguments
    ///
    /// * `model` - The model name, used to determine the context window size
    ///
    /// # Returns
    ///
    /// A truncated, compacted message list safe to send to the API.
    pub async fn prepare_api_messages_for_send(
        &mut self,
        model: &str,
        provider: Option<&Arc<dyn LlmProvider>>,
    ) -> Vec<ApiMessageV2> {
        let context_limit = model_context_limit(model);
        if self
            .maybe_compact_graceful(DEFAULT_COMPACTION_THRESHOLD, context_limit, provider)
            .await
        {
            tracing::info!(
                threshold = DEFAULT_COMPACTION_THRESHOLD,
                context_limit,
                "Conversation compacted before tool continuation"
            );
        }

        let total = self.conversation.api_messages.len();
        let truncated = self.api_messages_truncated();
        let sent = truncated.len();
        if sent < total {
            tracing::info!(
                total,
                sending = sent,
                dropped = total - sent,
                "Context truncated for tool continuation"
            );
        }

        truncated
    }

    /// Builds the final API message list for sending to the LLM.
    ///
    /// Returns the truncated message history ready for an API call. Context
    /// injection is **not** performed here — callers that need CCG context
    /// should inject it into the user message before appending to
    /// `api_messages` (see `submit_message()` and `print::run_print_mode()`).
    ///
    /// This avoids double-injection: `submit_message()` already embeds
    /// context into the user message via `.take()`, so a second injection
    /// here would duplicate it.
    ///
    /// # Returns
    ///
    /// A new vector containing the truncated message history.
    #[must_use]
    pub fn build_api_messages(&self) -> Vec<ApiMessageV2> {
        self.api_messages_truncated()
    }

    // ========================================================================
    // Message Submission
    // ========================================================================

    /// Submits a user message to the API and starts streaming the response.
    ///
    /// This method orchestrates the full conversation submission lifecycle:
    /// 1. Refreshes CCG context if auto-context is enabled
    /// 2. Optionally injects context into the user message
    /// 3. Updates the timeline and API message history
    /// 4. Initializes the tool loop state machine
    /// 5. Creates a streaming channel and spawns the API call
    ///
    /// # Arguments
    ///
    /// * `client` - The LLM provider to stream the response from
    /// * `content` - The user message text
    ///
    /// # Errors
    ///
    /// Returns an error if the tool loop fails to initialize.
    pub async fn submit_message(
        &mut self,
        client: &std::sync::Arc<dyn LlmProvider>,
        content: String,
    ) -> Result<()> {
        // Refresh context from build_context() if auto-context is enabled.
        // This populates cached_ccg_context which is consumed below.
        self.refresh_build_context().await;

        // Build the API message content, optionally with CCG context
        let api_content = if self.compression.auto_context_enabled {
            if let Some(context) = self.compression.cached_ccg_context.take() {
                tracing::info!(
                    context_len = context.len(),
                    "Injecting CCG context into user message"
                );
                // Prepend context to user message for API
                format!("<context>\n{}\n</context>\n\n{}", context, content)
            } else {
                content.clone()
            }
        } else {
            content.clone()
        };

        // Timeline shows original user input (cleaner UI)
        self.conversation.timeline.push_user_message(&content);
        // API gets potentially context-augmented message
        let user_msg = ApiMessageV2::user(&api_content);
        self.conversation.api_messages.push(user_msg);

        self.view.display.loading = true;
        // Start streaming in timeline
        if self.conversation.timeline.try_push_streaming().is_err() {
            tracing::warn!("Timeline already streaming when submitting message");
        }

        // Initialize tool loop state machine for streaming
        // This must be called BEFORE the API stream starts so tool events are captured
        if let Err(e) = self.tool_state.tool_loop.start_streaming() {
            tracing::warn!("Failed to start tool loop streaming: {}", e);
            // Reset and try again - the loop might be in an unexpected state
            self.tool_state.tool_loop.reset();
            if let Err(e2) = self.tool_state.tool_loop.start_streaming() {
                tracing::error!(
                    "Tool loop start_streaming failed after reset: {}. Tool execution may be compromised.",
                    e2
                );
            }
        }

        let (tx, rx) = mpsc::channel(STREAMING_CHANNEL_BUFFER);
        self.conversation.streaming_rx = Some(rx);

        // Compact + truncate API messages for cost-controlled sending
        let api_messages = self
            .prepare_api_messages_for_send(client.model(), Some(client))
            .await;

        let client = std::sync::Arc::clone(client);
        let tools = self.all_tool_definitions();
        let options = self.build_request_options(client.model());
        tokio::spawn(async move {
            if let Err(e) = client
                .stream_message(
                    &api_messages,
                    Some(&tools),
                    Some(&ToolChoice::Auto),
                    &options,
                    tx,
                )
                .await
            {
                tracing::error!("API error: {}", e);
            }
        });

        Ok(())
    }

    // ========================================================================
    // Streaming and Background Events
    // ========================================================================

    /// Receives the next API streaming chunk, if available.
    ///
    /// Returns `None` immediately when no streaming is active. This is critical
    /// for event loop responsiveness - returning `pending()` would cause the
    /// `tokio::select!` to wait indefinitely, blocking keyboard input processing.
    ///
    /// # Returns
    ///
    /// - `Some(chunk)` - Next streaming event from the API
    /// - `None` - Channel closed or no active streaming
    pub async fn recv_api_chunk(&mut self) -> Option<StreamEvent> {
        match &mut self.conversation.streaming_rx {
            Some(rx) => rx.recv().await,
            None => None, // Return immediately - don't block with pending()
        }
    }

    /// Returns true if there is an active streaming receiver.
    ///
    /// Used for guard conditions in the event loop to skip polling
    /// when no streaming is active.
    #[must_use]
    pub fn has_streaming(&self) -> bool {
        self.conversation.streaming_rx.is_some()
    }

    /// Returns true if there are any active background channels (API streaming or tool results).
    ///
    /// Used for guard conditions in the event loop.
    #[must_use]
    pub fn has_background_work(&self) -> bool {
        self.conversation.streaming_rx.is_some() || self.tool_state.tool_result_rx.is_some()
    }

    /// Receives the next background event from either API streaming or tool execution.
    ///
    /// This combines both channels into a single async receive to avoid borrow checker
    /// issues in the event loop's `tokio::select!`. Returns immediately if both
    /// channels are `None`.
    ///
    /// # Returns
    ///
    /// - `Some(BackgroundEvent::ApiChunk(chunk))` - API streaming event
    /// - `Some(BackgroundEvent::ToolResult(id, result))` - Tool execution completed
    /// - `None` - Both channels closed or not set
    pub async fn recv_background_event(&mut self) -> Option<BackgroundEvent> {
        // Use tokio::select! to receive from whichever channel is ready first
        // Since this is a single method, there's no borrow conflict
        tokio::select! {
            biased;

            // Prioritize tool results to update UI quickly
            result = async {
                match &mut self.tool_state.tool_result_rx {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            }, if self.tool_state.tool_result_rx.is_some() => {
                match result {
                    Some((id, r)) => Some(BackgroundEvent::ToolResult(id, r)),
                    None => {
                        // Sender dropped (task panicked/completed) — clear to prevent busy-loop
                        self.tool_state.tool_result_rx = None;
                        None
                    }
                }
            }

            // Then API streaming chunks
            chunk = async {
                match &mut self.conversation.streaming_rx {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            }, if self.conversation.streaming_rx.is_some() => {
                match chunk {
                    Some(event) => Some(BackgroundEvent::ApiChunk(event)),
                    None => {
                        // Sender dropped (task panicked/completed) — clear to prevent busy-loop
                        self.conversation.streaming_rx = None;
                        None
                    }
                }
            }

            // If neither channel is active, return None immediately
            else => None
        }
    }

    /// Sets the streaming receiver for API response chunks.
    ///
    /// This is used by the tool execution flow to set up continuation streaming
    /// without blocking the event loop.
    pub fn set_streaming_rx(&mut self, rx: mpsc::Receiver<StreamEvent>) {
        self.conversation.streaming_rx = Some(rx);
    }

    /// Sets the loading state.
    ///
    /// When loading is true, the throbber animates and content accumulates.
    pub fn set_loading(&mut self, loading: bool) {
        self.view.display.loading = loading;
        self.conversation.dirty = true;
    }

    /// Initializes the streaming buffer for continuation streaming.
    ///
    /// This starts a new streaming entry in the timeline with optional initial content.
    pub fn set_current_response(&mut self, response: String) {
        // Start streaming in timeline if not already streaming
        if self.conversation.timeline.try_push_streaming().is_ok() && !response.is_empty() {
            self.conversation.timeline.append_to_streaming(&response);
        }
        self.conversation.dirty = true;
    }
}

#[cfg(test)]
impl AppState {
    /// Adds a full API message with content blocks (test-only).
    pub fn add_api_message(&mut self, message: ApiMessageV2) {
        let legacy = message.to_legacy();
        match legacy.role {
            Role::User => self
                .conversation
                .timeline
                .push_user_message(&legacy.content),
            Role::Assistant => self
                .conversation
                .timeline
                .push_assistant_message(&legacy.content),
        }
        self.conversation.api_messages.push(message);
        self.conversation.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_state() -> AppState {
        AppState::new(
            std::path::PathBuf::from("/test"),
            false,
            crate::types::config::ParallelMode::Enabled,
        )
    }

    #[test]
    fn messages_returns_timeline_entries() {
        let mut state = new_state();
        // Initially empty
        assert!(state.messages().is_empty());

        // After adding a message, it appears in messages()
        state.add_message(Message {
            role: Role::User,
            content: "hello".to_string(),
        });
        let msgs = state.messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, Role::User);
        assert_eq!(msgs[0].content, "hello");
    }

    #[test]
    fn add_message_updates_timeline_and_dirty_flag() {
        let mut state = new_state();
        state.dirty.clear();

        state.add_message(Message {
            role: Role::Assistant,
            content: "I can help.".to_string(),
        });

        // Dirty flag should be set (timeline mutation sets conversation.dirty)
        assert!(state.conversation.dirty);
        assert!(state.needs_render());

        // Timeline should contain the assistant message
        let entries: Vec<_> = state.timeline().iter().collect();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].is_assistant());
    }

    #[test]
    fn api_messages_start_empty_and_grow_on_add() {
        let mut state = new_state();
        assert!(state.api_messages().is_empty());
        assert_eq!(state.api_messages_len(), 0);

        let msg = ApiMessageV2::user("test prompt");
        state.add_api_message(msg);

        assert_eq!(state.api_messages_len(), 1);
        assert_eq!(state.api_messages()[0].to_legacy().content, "test prompt");
    }

    #[tokio::test]
    async fn recv_background_event_clears_streaming_rx_on_sender_drop() {
        let mut state = new_state();
        let (tx, rx) = mpsc::channel::<StreamEvent>(1);
        state.conversation.streaming_rx = Some(rx);

        assert!(state.has_background_work(), "streaming_rx is set");

        // Drop the sender to simulate a task panic / completion without explicit close
        drop(tx);

        // recv_background_event should return None and clear streaming_rx
        let event = state.recv_background_event().await;
        assert!(event.is_none(), "closed channel should yield None");
        assert!(
            state.conversation.streaming_rx.is_none(),
            "streaming_rx must be cleared when sender is dropped"
        );
        assert!(
            !state.has_background_work(),
            "no background work after cleanup"
        );
    }

    #[tokio::test]
    async fn recv_background_event_clears_tool_result_rx_on_sender_drop() {
        let mut state = new_state();
        let (tx, rx) = mpsc::channel::<(String, crate::types::ToolResultBlock)>(1);
        state.tool_state.tool_result_rx = Some(rx);

        assert!(state.has_background_work(), "tool_result_rx is set");

        // Drop the sender
        drop(tx);

        let event = state.recv_background_event().await;
        assert!(event.is_none(), "closed channel should yield None");
        assert!(
            state.tool_state.tool_result_rx.is_none(),
            "tool_result_rx must be cleared when sender is dropped"
        );
        assert!(
            !state.has_background_work(),
            "no background work after cleanup"
        );
    }

    #[tokio::test]
    async fn build_api_messages_does_not_inject_context() {
        // build_api_messages is now a pure passthrough — context injection
        // is handled by submit_message / print mode callers.
        let mut state = new_state();
        state.compression_mut().set_auto_context_enabled(true);
        state.compression.cached_ccg_context = Some("## Context Data".to_string());

        state
            .conversation
            .api_messages
            .push(ApiMessageV2::user("Hello"));

        let messages = state.build_api_messages();
        assert_eq!(messages.len(), 1, "no context message should be injected");
        assert_eq!(messages[0].content.to_text(), "Hello");

        // Context should remain unconsumed
        assert!(state.compression().has_cached_ccg_context());
    }
}
