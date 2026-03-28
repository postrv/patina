//! Auto-compaction and session management methods for `AppState`.

use super::*;

impl AppState {
    // ========================================================================
    // Auto-Compaction (Phase 4.4)
    // ========================================================================

    /// Estimates the total tokens in the current conversation.
    ///
    /// Uses the token estimation utilities to calculate approximate token
    /// usage for all API messages. This is a heuristic estimate, not an
    /// exact count.
    ///
    /// # Returns
    ///
    /// Estimated token count for the conversation.
    #[must_use]
    pub fn estimate_conversation_tokens(&self) -> usize {
        use crate::api::tokens::estimate_messages_tokens;
        estimate_messages_tokens(&self.api_messages)
    }

    /// Updates the token budget based on the current conversation size.
    ///
    /// This synchronizes the token budget with the actual estimated token
    /// usage in the conversation. Call this after adding messages or after
    /// compaction to keep the budget accurate.
    pub fn sync_token_budget(&mut self) {
        let tokens = self.estimate_conversation_tokens();
        self.reset_token_budget();
        self.add_token_usage(tokens);
    }

    /// Checks if compaction should be triggered and performs it if needed.
    ///
    /// This method:
    /// 1. Estimates current conversation tokens
    /// 2. Checks if usage exceeds the auto-compaction threshold
    /// 3. If yes, performs compaction using the mock summarizer
    /// 4. Updates api_messages with compacted result
    /// 5. Syncs token budget
    ///
    /// # Arguments
    ///
    /// * `threshold` - Fraction of context window at which to trigger (0.0-1.0)
    /// * `context_limit` - Maximum context window size in tokens
    ///
    /// # Returns
    ///
    /// * `Ok(true)` - Compaction was performed
    /// * `Ok(false)` - No compaction needed
    /// * `Err(_)` - Compaction failed
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Trigger compaction at 80% of 200k context
    /// let compacted = state.maybe_compact(0.8, 200_000, None).await?;
    /// if compacted {
    ///     println!("Conversation was compacted");
    /// }
    /// ```
    pub async fn maybe_compact(
        &mut self,
        threshold: f32,
        context_limit: usize,
        provider: Option<&Arc<dyn crate::api::LlmProvider>>,
    ) -> Result<bool> {
        use crate::api::compaction::{
            CompactionConfig, ContextCompactor, NoOpSummarizer, ProviderSummarizer,
        };
        use std::time::Instant;

        // Estimate current usage
        let current_tokens = self.estimate_conversation_tokens();
        let threshold_tokens = (context_limit as f64 * f64::from(threshold)) as usize;

        // Check for a manual compaction request (bypasses threshold and circuit breaker)
        let forced = self.compression.take_compaction_request().is_some();

        // Manual compaction resets the circuit breaker
        if forced {
            self.compression.reset_circuit_breaker();
        }

        // Circuit breaker: skip auto-compaction after too many consecutive failures
        if !forced && self.compression.is_compaction_circuit_open() {
            tracing::warn!("Compaction circuit breaker open — skipping auto-compaction");
            return Ok(false);
        }

        // Check if we're under threshold and no forced compaction
        if !forced && current_tokens < threshold_tokens {
            tracing::debug!(
                current = current_tokens,
                threshold = threshold_tokens,
                "Compaction not needed"
            );
            return Ok(false);
        }

        let trigger = if forced { "manual" } else { "auto" };
        tracing::info!(
            current = current_tokens,
            threshold = threshold_tokens,
            trigger,
            "Starting compaction"
        );

        // Show compaction progress
        let target_tokens = context_limit / 2; // Target 50% of context
        self.start_compaction(target_tokens, current_tokens, !forced);

        // Fire PreCompact hook (non-blocking)
        let executor = Arc::clone(&self.tool_state.tool_executor);
        tokio::spawn(async move {
            let _ = executor.hooks().fire_pre_compact().await;
        });

        let config = CompactionConfig {
            target_tokens,
            preserve_recent: 4,
            ..Default::default()
        };

        // Start timing
        let start_time = Instant::now();

        // Use provider-backed summarizer when available, otherwise fall back to no-op
        let compact_result = if let Some(p) = provider {
            let summarizer = ProviderSummarizer::new(Arc::clone(p));
            let compactor = ContextCompactor::with_summarizer(summarizer);
            compactor.compact(&self.api_messages, &config).await
        } else {
            let compactor = ContextCompactor::<NoOpSummarizer>::new_noop();
            compactor.compact(&self.api_messages, &config).await
        };

        // Perform compaction
        match compact_result {
            Ok(result) => {
                let duration = start_time.elapsed();
                let after_tokens = crate::api::tokens::estimate_messages_tokens(&result.messages);
                let tokens_saved = result.saved_tokens;

                // Record compaction metrics
                self.compression
                    .compaction_metrics
                    .record_compaction(tokens_saved, duration);

                // Log metrics summary
                tracing::info!(
                    before = current_tokens,
                    after = after_tokens,
                    saved = tokens_saved,
                    duration_ms = duration.as_millis() as u64,
                    total_compactions = self.compression.compaction_metrics.compaction_count(),
                    total_tokens_saved = self.compression.compaction_metrics.total_tokens_saved(),
                    "Compaction complete"
                );

                // Update state with compacted messages
                self.api_messages = result.messages;

                // Sync token budget with new size
                self.sync_token_budget();

                // Update compaction UI
                self.complete_compaction(after_tokens);

                // Fire PostCompact hook (non-blocking)
                let executor = Arc::clone(&self.tool_state.tool_executor);
                tokio::spawn(async move {
                    let _ = executor.hooks().fire_post_compact().await;
                });

                // Reset circuit breaker on success
                self.compression.record_compaction_success();

                Ok(true)
            }
            Err(e) => {
                tracing::error!(error = %e, "Compaction failed");
                self.fail_compaction();

                // Record failure for circuit breaker
                self.compression.record_compaction_failure();

                // Return error but don't fail the operation
                Err(e)
            }
        }
    }

    /// Performs compaction if needed, logging but not failing on errors.
    ///
    /// This is a convenience wrapper around `maybe_compact` that handles
    /// errors gracefully. Use this in the message send flow where compaction
    /// failure shouldn't block the API call.
    ///
    /// # Arguments
    ///
    /// * `threshold` - Fraction of context window at which to trigger
    /// * `context_limit` - Maximum context window size in tokens
    ///
    /// # Returns
    ///
    /// `true` if compaction was performed successfully, `false` otherwise.
    pub async fn maybe_compact_graceful(
        &mut self,
        threshold: f32,
        context_limit: usize,
        provider: Option<&Arc<dyn crate::api::LlmProvider>>,
    ) -> bool {
        match self.maybe_compact(threshold, context_limit, provider).await {
            Ok(compacted) => compacted,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "Compaction failed, continuing without compaction"
                );
                // Clear failed compaction state
                self.clear_compaction();
                false
            }
        }
    }

    // ========================================================================
    // Session Restoration and Auto-Save (delegates to SessionTracking)
    // ========================================================================

    /// Returns a reference to the session tracking state.
    #[must_use]
    pub fn session_tracking(&self) -> &SessionTracking {
        &self.session
    }

    /// Creates a `Session` from the current application state.
    ///
    /// The resulting session includes:
    /// - All conversation messages (converted from timeline)
    /// - Current UI state (scroll position, input buffer, cursor position)
    /// - Working directory
    ///
    /// This is used for auto-save functionality.
    #[must_use]
    pub fn to_session(&self) -> Session {
        use crate::session::UiState;

        let mut session = Session::new(self.working_dir.clone());

        // Convert timeline entries to messages for session persistence
        for entry in self.timeline.iter() {
            match entry {
                crate::types::ConversationEntry::UserMessage(text) => {
                    session.add_message(Message {
                        role: Role::User,
                        content: text.clone(),
                    });
                }
                crate::types::ConversationEntry::AssistantMessage(text) => {
                    session.add_message(Message {
                        role: Role::Assistant,
                        content: text.clone(),
                    });
                }
                // Skip streaming and tool execution entries for session persistence
                _ => {}
            }
        }

        // Capture UI state (use scroll offset for backward compatibility)
        let ui_state = UiState::with_state(
            self.display.scroll.offset(),
            self.input_state.text().to_string(),
            self.input_state.cursor_position(),
        );
        session.set_ui_state(Some(ui_state));

        // Copy checkpoints from session tracking
        for cp in self.session.checkpoints() {
            session.add_checkpoint_raw(cp.clone());
        }

        session
    }

    /// Restores application state from a saved session.
    ///
    /// This restores:
    /// - Message history (to timeline)
    /// - UI state (scroll position, input buffer, cursor position) if saved
    /// - Session ID for subsequent saves
    ///
    /// # Arguments
    ///
    /// * `session` - The session to restore from.
    pub fn restore_from_session(&mut self, session: &Session) {
        // Clear and rebuild timeline from session messages
        self.timeline = Timeline::new();
        for message in session.messages() {
            match message.role {
                Role::User => self.timeline.push_user_message(&message.content),
                Role::Assistant => self.timeline.push_assistant_message(&message.content),
            }
        }

        // Restore UI state if available
        if let Some(ui_state) = session.ui_state() {
            self.display.scroll.restore_offset(ui_state.scroll_offset());
            self.input_state
                .set_text(ui_state.input_buffer().to_string());
            self.input_state
                .set_cursor_position(ui_state.cursor_position());
        }

        // Restore session ID if available
        if let Some(id) = session.id() {
            self.session.set_id(id.to_string());
        }

        // Restore checkpoints
        self.session.set_checkpoints(session.checkpoints().to_vec());

        // Mark for full redraw
        self.dirty.full = true;
    }
}
