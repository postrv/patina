//! View model for TUI rendering, decoupling the TUI layer from `AppState`.
//!
//! [`RenderView`] contains borrowed references to everything the TUI needs to
//! render a single frame. By accepting `&RenderView` instead of `&AppState`,
//! the TUI module no longer imports from `app::state`, breaking the circular
//! dependency chain: `state -> app -> tui -> state`.
//!
//! The mutation path is handled separately: [`RenderFeedback`] captures computed
//! layout metrics, and the caller applies them to `AppState` after rendering.

use crate::api::tokens::TokenBudget;
use crate::permissions::PermissionRequest;
use crate::types::ui_state::{
    CompactionProgressState, CompletionState, ContinuousLoopStatus, FocusArea, GateResult,
    PlanState, QuestionState, ScrollState, SelectionState,
};
use crate::types::Timeline;

/// A read-only view of application state for TUI rendering.
///
/// This struct borrows every piece of data the TUI layer needs to render a
/// complete frame. It is constructed by [`AppState::as_render_view`] and
/// consumed by [`tui::render`].
///
/// # Design rationale
///
/// Previously, `tui::render()` accepted `&mut AppState`, coupling the TUI
/// directly to the application state module and creating a circular import
/// chain. `RenderView` breaks this by:
///
/// 1. Providing only the data the TUI actually reads (no mutation access).
/// 2. Living in `types/` which both `app` and `tui` can import freely.
/// 3. Keeping the mutation path explicit via [`RenderFeedback`].
///
/// [`AppState::as_render_view`]: crate::app::state::AppState::as_render_view
/// [`tui::render`]: crate::tui::render
#[derive(Debug)]
pub struct RenderView<'a> {
    // -- Message timeline --
    /// The conversation timeline to render.
    pub timeline: &'a Timeline,

    // -- Animation --
    /// Current throbber animation character for streaming indicators.
    pub throbber_char: char,

    // -- Scroll & selection --
    /// Current scroll offset from bottom (0 = at bottom).
    pub scroll_offset: usize,
    /// Full scroll state (mode, viewport, content height).
    pub scroll_state: &'a ScrollState,
    /// Text selection state for copy/paste.
    pub selection: &'a SelectionState,
    /// Which UI area currently has focus (Content vs Input).
    pub focus_area: FocusArea,

    // -- Input --
    /// Current text in the input buffer.
    pub input: &'a str,
    /// Active completion popup state, if any.
    pub completion: Option<&'a CompletionState>,

    // -- Status bar: git --
    /// Current git branch name.
    pub worktree_branch: Option<&'a str>,
    /// Number of modified files in the worktree.
    pub worktree_modified: usize,
    /// Commits ahead of upstream.
    pub worktree_ahead: usize,
    /// Commits behind upstream.
    pub worktree_behind: usize,

    // -- Status bar: tokens --
    /// Token budget (used/limit/thresholds).
    pub token_budget: &'a TokenBudget,
    /// Number of context tokens injected (e.g., from CLAUDE.md).
    pub context_tokens_injected: usize,

    // -- Status bar: cost --
    /// Cumulative session cost in USD (from CostTracker).
    pub session_cost_usd: f64,

    // -- Status bar: update --
    /// Available update version string, if any.
    pub update_available: Option<&'a str>,

    // -- Overlays: continuous loop --
    /// Current continuous loop status.
    pub continuous_status: &'a ContinuousLoopStatus,
    /// Total completed iterations in the continuous loop.
    pub continuous_iterations_completed: u32,
    /// Quality gate results for the current iteration.
    pub continuous_gate_results: &'a [GateResult],
    /// Gate currently being checked (name).
    pub continuous_checking_gate: Option<&'a str>,
    /// Duration of the last iteration in milliseconds.
    pub continuous_last_duration_ms: Option<u64>,

    // -- Overlays: compaction --
    /// Active compaction progress state, if any.
    pub compaction_state: Option<&'a CompactionProgressState>,

    // -- Overlays: modals --
    /// Pending permission request for tool approval.
    pub pending_permission: Option<&'a PermissionRequest>,
    /// Pending plan for user review.
    pub pending_plan: Option<&'a PlanState>,
    /// Pending question for user response.
    pub pending_question: Option<&'a QuestionState>,

    // -- Dirty flags --
    /// When `true`, only the throbber character changed since last render.
    /// The render path may skip the full timeline rebuild.
    pub throbber_only: bool,
}

/// Feedback from the render pass that must be applied to AppState.
///
/// `render_messages` computes layout metrics that the scroll and selection
/// systems need. Instead of mutating `AppState` directly during rendering,
/// this struct captures the computed values so they can be applied afterwards.
#[derive(Debug, Clone)]
pub struct RenderFeedback {
    /// Wrapped lines from the timeline (for copy/paste line cache).
    pub wrapped_lines: Vec<String>,
    /// The viewport height in rows (excluding borders).
    pub viewport_height: usize,
    /// The total content height in wrapped rows.
    pub content_height: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tokens::TokenBudget;
    use crate::types::ui_state::{ContinuousLoopStatus, FocusArea, ScrollState, SelectionState};
    use crate::types::Timeline;

    /// Creates a minimal `RenderView` for testing.
    ///
    /// All fields use default/empty values so tests can override only what
    /// they need via struct update syntax.
    fn minimal_view<'a>(
        timeline: &'a Timeline,
        scroll_state: &'a ScrollState,
        selection: &'a SelectionState,
        token_budget: &'a TokenBudget,
        continuous_status: &'a ContinuousLoopStatus,
    ) -> RenderView<'a> {
        RenderView {
            timeline,
            throbber_char: ' ',
            scroll_offset: 0,
            scroll_state,
            selection,
            focus_area: FocusArea::Input,
            input: "",
            completion: None,
            worktree_branch: None,
            worktree_modified: 0,
            worktree_ahead: 0,
            worktree_behind: 0,
            token_budget,
            context_tokens_injected: 0,
            session_cost_usd: 0.0,
            update_available: None,
            continuous_status,
            continuous_iterations_completed: 0,
            continuous_gate_results: &[],
            continuous_checking_gate: None,
            continuous_last_duration_ms: None,
            compaction_state: None,
            pending_permission: None,
            pending_plan: None,
            pending_question: None,
            throbber_only: false,
        }
    }

    #[test]
    fn render_view_borrows_timeline() {
        let timeline = Timeline::new();
        let scroll = ScrollState::new();
        let selection = SelectionState::new();
        let budget = TokenBudget::new(200_000);
        let status = ContinuousLoopStatus::Inactive;

        let view = minimal_view(&timeline, &scroll, &selection, &budget, &status);
        assert!(view.timeline.is_empty());
    }

    #[test]
    fn render_view_exposes_input() {
        let timeline = Timeline::new();
        let scroll = ScrollState::new();
        let selection = SelectionState::new();
        let budget = TokenBudget::new(200_000);
        let status = ContinuousLoopStatus::Inactive;

        let view = RenderView {
            input: "hello",
            ..minimal_view(&timeline, &scroll, &selection, &budget, &status)
        };
        assert_eq!(view.input, "hello");
    }

    #[test]
    fn render_view_default_overlays_are_none() {
        let timeline = Timeline::new();
        let scroll = ScrollState::new();
        let selection = SelectionState::new();
        let budget = TokenBudget::new(200_000);
        let status = ContinuousLoopStatus::Inactive;

        let view = minimal_view(&timeline, &scroll, &selection, &budget, &status);
        assert!(view.completion.is_none());
        assert!(view.compaction_state.is_none());
        assert!(view.pending_permission.is_none());
        assert!(view.pending_plan.is_none());
        assert!(view.pending_question.is_none());
    }

    #[test]
    fn render_view_exposes_continuous_status() {
        let timeline = Timeline::new();
        let scroll = ScrollState::new();
        let selection = SelectionState::new();
        let budget = TokenBudget::new(200_000);
        let status = ContinuousLoopStatus::Running { iteration: 3 };

        let view = minimal_view(&timeline, &scroll, &selection, &budget, &status);
        assert_eq!(
            *view.continuous_status,
            ContinuousLoopStatus::Running { iteration: 3 }
        );
    }

    #[test]
    fn render_view_exposes_git_info() {
        let timeline = Timeline::new();
        let scroll = ScrollState::new();
        let selection = SelectionState::new();
        let budget = TokenBudget::new(200_000);
        let status = ContinuousLoopStatus::Inactive;

        let view = RenderView {
            worktree_branch: Some("main"),
            worktree_modified: 3,
            worktree_ahead: 1,
            worktree_behind: 2,
            ..minimal_view(&timeline, &scroll, &selection, &budget, &status)
        };
        assert_eq!(view.worktree_branch, Some("main"));
        assert_eq!(view.worktree_modified, 3);
        assert_eq!(view.worktree_ahead, 1);
        assert_eq!(view.worktree_behind, 2);
    }

    #[test]
    fn render_view_includes_cost() {
        let timeline = Timeline::new();
        let scroll = ScrollState::new();
        let selection = SelectionState::new();
        let budget = TokenBudget::new(200_000);
        let status = ContinuousLoopStatus::Inactive;

        let view = RenderView {
            session_cost_usd: 1.234,
            ..minimal_view(&timeline, &scroll, &selection, &budget, &status)
        };
        assert!((view.session_cost_usd - 1.234).abs() < f64::EPSILON);
    }

    #[test]
    fn render_view_cost_defaults_to_zero() {
        let timeline = Timeline::new();
        let scroll = ScrollState::new();
        let selection = SelectionState::new();
        let budget = TokenBudget::new(200_000);
        let status = ContinuousLoopStatus::Inactive;

        let view = minimal_view(&timeline, &scroll, &selection, &budget, &status);
        assert!((view.session_cost_usd - 0.0).abs() < f64::EPSILON);
    }
}
