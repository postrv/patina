use crate::api::TokenBudget;
use crate::context::compression::{CompactionMetrics, CompressionOrchestrator};
use crate::mcp::connection::McpConnection;
use crate::narsil::context::ContextSuggestion;
use crate::types::ui_state::CompactionProgressState;
use std::sync::Arc;

/// Compression, context injection, and compaction state extracted from AppState.
///
/// Groups all fields related to: context compression (CCG orchestrator, caching),
/// narsil MCP client management, auto-context injection, token budgeting,
/// compaction progress, and compaction metrics.
pub struct CompressionState {
    /// Optional compression orchestrator for CCG context management.
    pub(crate) compression_orchestrator: Option<Arc<CompressionOrchestrator>>,

    /// Last repository hash used for CCG context injection.
    pub(crate) last_ccg_hash: Option<String>,

    /// Cached CCG context content for injection into messages.
    pub(crate) cached_ccg_context: Option<String>,

    /// Optional narsil MCP connection for CCG context fetching.
    pub(crate) narsil_client: Option<McpConnection>,

    /// Maximum tokens to include in auto-injected context.
    pub(crate) context_token_budget: usize,

    /// Tokens actually injected in the most recent context injection.
    pub(crate) context_tokens_injected: usize,

    /// Whether auto-context injection is enabled.
    pub(crate) auto_context_enabled: bool,

    /// Pending context suggestions to be injected into the next message.
    pub(crate) pending_context: Vec<ContextSuggestion>,

    /// Optional compaction progress state for displaying the compaction overlay.
    pub(crate) compaction_state: Option<CompactionProgressState>,

    /// Metrics tracking for compaction operations.
    pub(crate) compaction_metrics: Arc<CompactionMetrics>,

    /// Token budget tracking for the current session.
    pub(crate) token_budget: TokenBudget,

    /// Whether a manual compaction has been requested (e.g. via `/compact`).
    pub(crate) compaction_requested: bool,

    /// Optional custom instructions for the next compaction (set by `/compact`).
    pub(crate) compaction_custom_instructions: Option<String>,

    /// Consecutive compaction failure count for circuit breaker.
    /// After 3 failures, auto-compaction is skipped until reset.
    pub(crate) consecutive_compaction_failures: u8,

    /// Whether any render-visible mutation has occurred since last `mark_render_clean()`.
    pub(crate) render_dirty: bool,
}

impl CompressionState {
    /// Returns whether any render-visible mutation has occurred since last `mark_render_clean()`.
    #[must_use]
    pub fn is_render_dirty(&self) -> bool {
        self.render_dirty
    }

    /// Clears the render-dirty flag after rendering.
    pub fn mark_render_clean(&mut self) {
        self.render_dirty = false;
    }

    /// Marks render-visible state as changed.
    pub fn mark_render_dirty(&mut self) {
        self.render_dirty = true;
    }

    /// Returns whether auto-context injection is enabled.
    #[must_use]
    pub fn auto_context_enabled(&self) -> bool {
        self.auto_context_enabled
    }

    /// Sets whether auto-context injection is enabled.
    pub fn set_auto_context_enabled(&mut self, enabled: bool) {
        self.auto_context_enabled = enabled;
    }

    /// Returns whether there are pending context suggestions.
    #[must_use]
    pub fn has_pending_context(&self) -> bool {
        !self.pending_context.is_empty()
    }

    /// Returns a reference to the pending context suggestions.
    #[must_use]
    pub fn pending_context(&self) -> &[ContextSuggestion] {
        &self.pending_context
    }

    /// Sets the pending context suggestions.
    pub fn set_pending_context(&mut self, suggestions: Vec<ContextSuggestion>) {
        self.pending_context = suggestions;
    }

    /// Takes and returns the pending context suggestions, clearing them.
    #[must_use]
    pub fn take_pending_context(&mut self) -> Vec<ContextSuggestion> {
        std::mem::take(&mut self.pending_context)
    }

    /// Clears the pending context suggestions.
    pub fn clear_pending_context(&mut self) {
        self.pending_context.clear();
    }

    /// Returns the last CCG hash used for context injection.
    #[must_use]
    pub fn last_ccg_hash(&self) -> Option<&str> {
        self.last_ccg_hash.as_deref()
    }

    /// Sets the last CCG hash.
    pub fn set_last_ccg_hash(&mut self, hash: String) {
        self.last_ccg_hash = Some(hash);
    }

    /// Returns whether there is cached CCG context.
    #[must_use]
    pub fn has_cached_ccg_context(&self) -> bool {
        self.cached_ccg_context.is_some()
    }

    /// Takes the cached CCG context.
    pub fn take_cached_ccg_context(&mut self) -> Option<String> {
        self.cached_ccg_context.take()
    }

    /// Returns context for injection if auto-context is enabled.
    #[must_use]
    pub fn context_for_injection(&self) -> Option<&str> {
        if self.auto_context_enabled {
            self.cached_ccg_context.as_deref()
        } else {
            None
        }
    }

    /// Returns whether a narsil MCP client is available.
    #[must_use]
    pub fn has_narsil_client(&self) -> bool {
        self.narsil_client.is_some()
    }

    /// Sets the narsil MCP connection.
    pub fn set_narsil_client(&mut self, client: McpConnection) {
        self.narsil_client = Some(client);
    }

    /// Returns the maximum token budget for auto-injected context.
    #[must_use]
    pub fn context_token_budget(&self) -> usize {
        self.context_token_budget
    }

    /// Sets the maximum token budget for auto-injected context.
    pub fn set_context_token_budget(&mut self, budget: usize) {
        self.context_token_budget = budget;
    }

    /// Returns the number of tokens injected in the most recent context injection.
    #[must_use]
    pub fn context_tokens_injected(&self) -> usize {
        self.context_tokens_injected
    }

    /// Sets the number of tokens injected.
    pub fn set_context_tokens_injected(&mut self, tokens: usize) {
        self.context_tokens_injected = tokens;
    }

    /// Returns the compression orchestrator if available.
    #[must_use]
    pub fn compression_orchestrator(&self) -> Option<&Arc<CompressionOrchestrator>> {
        self.compression_orchestrator.as_ref()
    }

    /// Sets the compression orchestrator.
    pub fn set_compression_orchestrator(&mut self, orchestrator: Arc<CompressionOrchestrator>) {
        self.compression_orchestrator = Some(orchestrator);
    }

    /// Returns true if the compression orchestrator supports CCG.
    #[must_use]
    pub fn has_ccg_support(&self) -> bool {
        self.compression_orchestrator
            .as_ref()
            .is_some_and(|o| o.should_use_ccg())
    }

    /// Returns a reference to the token budget.
    #[must_use]
    pub fn token_budget(&self) -> &TokenBudget {
        &self.token_budget
    }

    /// Returns a mutable reference to the token budget.
    pub fn token_budget_mut(&mut self) -> &mut TokenBudget {
        &mut self.token_budget
    }

    /// Returns the compaction progress state.
    #[must_use]
    pub fn compaction_state(&self) -> Option<&CompactionProgressState> {
        self.compaction_state.as_ref()
    }

    /// Returns a mutable reference to the compaction progress state.
    pub fn compaction_state_mut(&mut self) -> Option<&mut CompactionProgressState> {
        self.compaction_state.as_mut()
    }

    /// Starts a compaction operation.
    pub fn start_compaction(&mut self, target_tokens: usize, before_tokens: usize, is_auto: bool) {
        let mut state = if is_auto {
            CompactionProgressState::new_auto(target_tokens, before_tokens)
        } else {
            CompactionProgressState::new(target_tokens, before_tokens)
        };
        state.set_status(crate::tui::widgets::CompactionStatus::Compacting);
        self.compaction_state = Some(state);
        self.render_dirty = true;
    }

    /// Updates the compaction progress (0.0 to 1.0).
    pub fn update_compaction_progress(&mut self, progress: f64) {
        if let Some(state) = &mut self.compaction_state {
            state.set_progress(progress);
            self.render_dirty = true;
        }
    }

    /// Completes the compaction operation with the final token count.
    pub fn complete_compaction(&mut self, after_tokens: usize) {
        if let Some(state) = &mut self.compaction_state {
            state.set_after_tokens(after_tokens);
            state.set_status(crate::tui::widgets::CompactionStatus::Complete);
            state.set_progress(1.0);
            self.render_dirty = true;
        }
    }

    /// Marks the compaction operation as failed.
    pub fn fail_compaction(&mut self) {
        if let Some(state) = &mut self.compaction_state {
            state.set_status(crate::tui::widgets::CompactionStatus::Failed);
            self.render_dirty = true;
        }
    }

    /// Clears the compaction state (closes the overlay).
    pub fn clear_compaction(&mut self) {
        self.compaction_state = None;
        self.render_dirty = true;
    }

    /// Returns a reference to the compaction metrics.
    #[must_use]
    pub fn compaction_metrics(&self) -> &CompactionMetrics {
        &self.compaction_metrics
    }

    /// Returns whether a manual compaction has been requested.
    #[must_use]
    pub fn is_compaction_requested(&self) -> bool {
        self.compaction_requested
    }

    /// Requests a manual compaction, optionally with custom instructions.
    ///
    /// The next call to `maybe_compact` (or similar) in the event loop
    /// should check this flag and force compaction regardless of threshold.
    pub fn request_compaction(&mut self, custom_instructions: Option<String>) {
        self.compaction_requested = true;
        self.compaction_custom_instructions = custom_instructions;
        self.render_dirty = true;
    }

    /// Takes the compaction request, clearing the flag and returning any
    /// custom instructions that were provided.
    ///
    /// Returns `None` if no compaction was requested.
    #[must_use]
    pub fn take_compaction_request(&mut self) -> Option<Option<String>> {
        if self.compaction_requested {
            self.compaction_requested = false;
            Some(self.compaction_custom_instructions.take())
        } else {
            None
        }
    }

    /// Maximum consecutive compaction failures before the circuit breaker trips.
    const CIRCUIT_BREAKER_THRESHOLD: u8 = 3;

    /// Returns whether the compaction circuit breaker has tripped.
    ///
    /// After 3 consecutive compaction failures, auto-compaction is disabled
    /// to prevent infinite retry loops. Manual `/compact` resets the breaker.
    #[must_use]
    pub fn is_compaction_circuit_open(&self) -> bool {
        self.consecutive_compaction_failures >= Self::CIRCUIT_BREAKER_THRESHOLD
    }

    /// Records a compaction failure, incrementing the failure counter.
    pub fn record_compaction_failure(&mut self) {
        self.consecutive_compaction_failures =
            self.consecutive_compaction_failures.saturating_add(1);
        if self.is_compaction_circuit_open() {
            tracing::warn!(
                "Compaction circuit breaker tripped after {} consecutive failures",
                self.consecutive_compaction_failures
            );
        }
    }

    /// Records a compaction success, resetting the failure counter.
    pub fn record_compaction_success(&mut self) {
        self.consecutive_compaction_failures = 0;
    }

    /// Resets the circuit breaker, re-enabling auto-compaction.
    ///
    /// Called when the user manually triggers `/compact` to override.
    pub fn reset_circuit_breaker(&mut self) {
        self.consecutive_compaction_failures = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ui_state::CompactionStatus;

    /// Helper to build a default `CompressionState` for testing.
    fn make_state() -> CompressionState {
        CompressionState {
            compression_orchestrator: None,
            last_ccg_hash: None,
            cached_ccg_context: None,
            narsil_client: None,
            context_token_budget: 10_000,
            context_tokens_injected: 0,
            auto_context_enabled: false,
            pending_context: Vec::new(),
            compaction_state: None,
            compaction_metrics: Arc::new(CompactionMetrics::new()),
            token_budget: TokenBudget::new(100_000),
            compaction_requested: false,
            compaction_custom_instructions: None,
            consecutive_compaction_failures: 0,
            render_dirty: false,
        }
    }

    #[test]
    fn test_start_compaction_manual() {
        let mut state = make_state();
        state.start_compaction(8_000, 50_000, false);

        let cs = state
            .compaction_state()
            .expect("compaction_state should be Some");
        assert_eq!(cs.status(), CompactionStatus::Compacting);
        assert!(!cs.is_auto());
        assert_eq!(cs.target_tokens(), 8_000);
        assert_eq!(cs.before_tokens(), 50_000);
    }

    #[test]
    fn test_start_compaction_auto() {
        let mut state = make_state();
        state.start_compaction(8_000, 50_000, true);

        let cs = state
            .compaction_state()
            .expect("compaction_state should be Some");
        assert!(
            cs.is_auto(),
            "is_auto flag should be true for auto compaction"
        );
        assert_eq!(cs.status(), CompactionStatus::Compacting);
    }

    #[test]
    fn test_update_compaction_progress() {
        let mut state = make_state();
        state.start_compaction(8_000, 50_000, false);
        state.update_compaction_progress(0.42);

        let cs = state
            .compaction_state()
            .expect("compaction_state should be Some");
        assert!((cs.progress() - 0.42).abs() < f64::EPSILON);
    }

    #[test]
    fn test_complete_compaction() {
        let mut state = make_state();
        state.start_compaction(8_000, 50_000, false);
        state.complete_compaction(12_000);

        let cs = state
            .compaction_state()
            .expect("compaction_state should be Some");
        assert_eq!(cs.status(), CompactionStatus::Complete);
        assert_eq!(cs.after_tokens(), Some(12_000));
        assert!((cs.progress() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_fail_compaction() {
        let mut state = make_state();
        state.start_compaction(8_000, 50_000, false);
        state.fail_compaction();

        let cs = state
            .compaction_state()
            .expect("compaction_state should be Some");
        assert_eq!(cs.status(), CompactionStatus::Failed);
    }

    #[test]
    fn test_clear_compaction() {
        let mut state = make_state();
        state.start_compaction(8_000, 50_000, false);
        assert!(state.compaction_state().is_some());

        state.clear_compaction();
        assert!(state.compaction_state().is_none());
    }

    #[test]
    fn test_compaction_lifecycle_full() {
        let mut state = make_state();

        // start
        state.start_compaction(8_000, 50_000, false);
        assert_eq!(
            state.compaction_state().unwrap().status(),
            CompactionStatus::Compacting,
        );

        // update progress
        state.update_compaction_progress(0.5);
        assert!((state.compaction_state().unwrap().progress() - 0.5).abs() < f64::EPSILON);

        // complete
        state.complete_compaction(10_000);
        let cs = state.compaction_state().unwrap();
        assert_eq!(cs.status(), CompactionStatus::Complete);
        assert_eq!(cs.after_tokens(), Some(10_000));
        assert!((cs.progress() - 1.0).abs() < f64::EPSILON);

        // clear
        state.clear_compaction();
        assert!(state.compaction_state().is_none());
    }

    #[test]
    fn test_context_for_injection_enabled_cached() {
        let mut state = make_state();
        state.auto_context_enabled = true;
        state.cached_ccg_context = Some("cached context data".to_string());

        assert_eq!(state.context_for_injection(), Some("cached context data"));
    }

    #[test]
    fn test_context_for_injection_disabled_cached() {
        let mut state = make_state();
        state.auto_context_enabled = false;
        state.cached_ccg_context = Some("cached context data".to_string());

        assert_eq!(
            state.context_for_injection(),
            None,
            "Should return None when auto_context_enabled is false even with cached context"
        );
    }

    #[test]
    fn test_update_progress_noop_without_state() {
        let mut state = make_state();
        assert!(state.compaction_state().is_none());

        // Must not panic when compaction_state is None.
        state.update_compaction_progress(0.75);
        assert!(state.compaction_state().is_none());
    }

    #[test]
    fn test_compression_state_request_compaction() {
        let mut state = make_state();
        assert!(!state.is_compaction_requested());

        state.request_compaction(Some("focus on code".to_string()));
        assert!(state.is_compaction_requested());

        let instructions = state.take_compaction_request();
        assert_eq!(instructions, Some(Some("focus on code".to_string())));
        assert!(
            !state.is_compaction_requested(),
            "Flag should be cleared after take"
        );
    }

    #[test]
    fn test_compression_state_request_compaction_no_instructions() {
        let mut state = make_state();
        state.request_compaction(None);
        assert!(state.is_compaction_requested());

        let instructions = state.take_compaction_request();
        assert_eq!(instructions, Some(None));
        assert!(!state.is_compaction_requested());
    }

    #[test]
    fn test_take_compaction_request_returns_none_when_not_requested() {
        let mut state = make_state();
        assert_eq!(state.take_compaction_request(), None);
    }

    #[test]
    fn test_circuit_breaker_initially_closed() {
        let state = make_state();
        assert!(!state.is_compaction_circuit_open());
    }

    #[test]
    fn test_circuit_breaker_trips_after_three_failures() {
        let mut state = make_state();
        assert!(!state.is_compaction_circuit_open());

        state.record_compaction_failure();
        assert!(!state.is_compaction_circuit_open());

        state.record_compaction_failure();
        assert!(!state.is_compaction_circuit_open());

        state.record_compaction_failure();
        assert!(state.is_compaction_circuit_open());
    }

    #[test]
    fn test_circuit_breaker_success_resets_counter() {
        let mut state = make_state();
        state.record_compaction_failure();
        state.record_compaction_failure();
        assert!(!state.is_compaction_circuit_open());

        state.record_compaction_success();
        // Two more failures should not trip it since counter was reset
        state.record_compaction_failure();
        state.record_compaction_failure();
        assert!(!state.is_compaction_circuit_open());
    }

    #[test]
    fn test_circuit_breaker_manual_reset() {
        let mut state = make_state();
        state.record_compaction_failure();
        state.record_compaction_failure();
        state.record_compaction_failure();
        assert!(state.is_compaction_circuit_open());

        state.reset_circuit_breaker();
        assert!(!state.is_compaction_circuit_open());
    }

    #[test]
    fn test_circuit_breaker_saturating_add() {
        let mut state = make_state();
        // Should not overflow even with many failures
        for _ in 0..300 {
            state.record_compaction_failure();
        }
        assert!(state.is_compaction_circuit_open());
        assert_eq!(state.consecutive_compaction_failures, 255);
    }

    #[test]
    fn test_render_dirty_on_start_compaction() {
        let mut state = make_state();
        assert!(!state.is_render_dirty());
        state.start_compaction(8_000, 50_000, false);
        assert!(state.is_render_dirty());
    }

    #[test]
    fn test_render_dirty_on_request_compaction() {
        let mut state = make_state();
        state.request_compaction(None);
        assert!(state.is_render_dirty());
    }

    #[test]
    fn test_render_clean_after_mark() {
        let mut state = make_state();
        state.start_compaction(8_000, 50_000, false);
        assert!(state.is_render_dirty());
        state.mark_render_clean();
        assert!(!state.is_render_dirty());
    }

    #[test]
    fn test_render_dirty_through_compaction_lifecycle() {
        let mut state = make_state();
        state.start_compaction(8_000, 50_000, false);
        state.mark_render_clean();

        state.update_compaction_progress(0.5);
        assert!(state.is_render_dirty());
        state.mark_render_clean();

        state.complete_compaction(10_000);
        assert!(state.is_render_dirty());
        state.mark_render_clean();

        state.clear_compaction();
        assert!(state.is_render_dirty());
    }
}
