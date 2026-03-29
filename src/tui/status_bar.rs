//! Status bar span builders for the TUI status bar.
//!
//! Each function produces a `Vec<Span<'static>>` from minimal data,
//! keeping the rendering logic pure and independently testable.

use ratatui::{
    style::{Modifier, Style},
    text::Span,
};

use crate::tui::scroll::AutoScrollMode;
use crate::tui::selection::FocusArea;
use crate::tui::theme::PatinaTheme;
use crate::types::render_view::RenderFeedback;
use crate::types::render_view::RenderView;

/// Produces git branch spans (icon + branch name).
///
/// # Examples
///
/// ```rust,ignore
/// let spans = branch_spans(Some("main"));
/// assert_eq!(spans.len(), 2);
/// ```
#[must_use]
pub fn branch_spans(branch: Option<&str>) -> Vec<Span<'static>> {
    let Some(branch) = branch else {
        return Vec::new();
    };
    vec![
        Span::styled(" ", Style::default().fg(PatinaTheme::VERDIGRIS)),
        Span::styled(
            branch.to_owned(),
            Style::default()
                .fg(PatinaTheme::VERDIGRIS_BRIGHT)
                .add_modifier(Modifier::BOLD),
        ),
    ]
}

/// Produces modified-file-count spans (dirty indicator).
///
/// Returns an empty vec when `count` is zero.
///
/// # Examples
///
/// ```rust,ignore
/// let spans = modified_spans(3);
/// assert_eq!(spans.len(), 2);
/// ```
#[must_use]
pub fn modified_spans(count: usize) -> Vec<Span<'static>> {
    if count == 0 {
        return Vec::new();
    }
    vec![
        Span::raw(" "),
        Span::styled(
            format!("●{count}"),
            Style::default().fg(PatinaTheme::WARNING),
        ),
    ]
}

/// Produces ahead/behind upstream indicator spans.
///
/// Only emits spans for non-zero values.
///
/// # Examples
///
/// ```rust,ignore
/// let spans = upstream_spans(2, 1);
/// assert_eq!(spans.len(), 4); // space+ahead + space+behind
/// ```
#[must_use]
pub fn upstream_spans(ahead: usize, behind: usize) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    if ahead > 0 {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!("↑{ahead}"),
            Style::default().fg(PatinaTheme::SUCCESS),
        ));
    }
    if behind > 0 {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!("↓{behind}"),
            Style::default().fg(PatinaTheme::ERROR),
        ));
    }
    spans
}

/// Produces focus-area indicator spans (`[CONTENT]` or `[INPUT]`).
///
/// # Examples
///
/// ```rust,ignore
/// let spans = focus_spans(FocusArea::Content);
/// assert_eq!(spans[1].content, "[CONTENT]");
/// ```
#[must_use]
pub fn focus_spans(area: FocusArea) -> Vec<Span<'static>> {
    let (label, color) = match area {
        FocusArea::Content => ("CONTENT", PatinaTheme::VERDIGRIS_BRIGHT),
        FocusArea::Input => ("INPUT", PatinaTheme::MUTED),
    };
    vec![
        Span::raw(" "),
        Span::styled(format!("[{label}]"), Style::default().fg(color)),
    ]
}

/// Produces selection range spans when a selection is active.
///
/// `selection` should be the result of `SelectionState::range()`.
///
/// # Examples
///
/// ```rust,ignore
/// let spans = selection_spans(Some((start, end)));
/// assert_eq!(spans.len(), 2);
/// ```
#[must_use]
pub fn selection_spans(selection: Option<(usize, usize)>) -> Vec<Span<'static>> {
    let Some((start_line, end_line)) = selection else {
        return Vec::new();
    };
    vec![
        Span::raw(" "),
        Span::styled(
            format!("SEL:L{start_line}-L{end_line}"),
            Style::default().fg(PatinaTheme::SUCCESS),
        ),
    ]
}

/// Produces token budget spans, color-coded by usage level.
///
/// Returns an empty vec when `used` is zero.
///
/// # Examples
///
/// ```rust,ignore
/// let spans = budget_spans(50_000, 200_000, false, false);
/// assert_eq!(spans.len(), 2);
/// ```
#[must_use]
pub fn budget_spans(
    used: usize,
    limit: usize,
    is_critical: bool,
    is_warning: bool,
) -> Vec<Span<'static>> {
    if used == 0 {
        return Vec::new();
    }
    let color = if is_critical {
        PatinaTheme::ERROR
    } else if is_warning {
        PatinaTheme::WARNING
    } else {
        PatinaTheme::SUCCESS
    };
    vec![
        Span::raw(" "),
        Span::styled(
            format!("{}k/{}k", used / 1000, limit / 1000),
            Style::default().fg(color),
        ),
    ]
}

/// Produces context-tokens-injected spans.
///
/// Returns an empty vec when `ctx_tokens` is zero.
///
/// # Examples
///
/// ```rust,ignore
/// let spans = context_spans(8000);
/// assert_eq!(spans.len(), 2);
/// ```
#[must_use]
pub fn context_spans(ctx_tokens: usize) -> Vec<Span<'static>> {
    if ctx_tokens == 0 {
        return Vec::new();
    }
    vec![
        Span::raw(" "),
        Span::styled(
            format!("CTX:{}k", ctx_tokens / 1000),
            Style::default().fg(PatinaTheme::VERDIGRIS),
        ),
    ]
}

/// Produces session cost spans.
///
/// Returns an empty vec when cost is below the display threshold ($0.001).
/// Uses 3 decimal places for sub-cent amounts, 2 decimal places otherwise.
///
/// # Examples
///
/// ```rust,ignore
/// let spans = cost_spans(0.123);
/// assert_eq!(spans.len(), 2);
/// assert_eq!(spans[1].content, "$0.12");
/// ```
#[must_use]
pub fn cost_spans(cost_usd: f64) -> Vec<Span<'static>> {
    if cost_usd < 0.001 {
        return Vec::new();
    }
    let cost_str = if cost_usd < 0.01 {
        format!("${:.3}", cost_usd)
    } else {
        format!("${:.2}", cost_usd)
    };
    vec![
        Span::raw(" "),
        Span::styled(cost_str, Style::default().fg(PatinaTheme::VERDIGRIS)),
    ]
}

/// Produces scroll mode and position spans.
///
/// # Examples
///
/// ```rust,ignore
/// let spans = scroll_spans(AutoScrollMode::Follow, 0, 40, 120);
/// assert_eq!(spans.len(), 1);
/// ```
#[must_use]
pub fn scroll_spans(
    mode: AutoScrollMode,
    offset: usize,
    viewport_height: usize,
    content_height: usize,
) -> Vec<Span<'static>> {
    let mode_char = match mode {
        AutoScrollMode::Follow => 'F',
        AutoScrollMode::Manual => 'M',
        AutoScrollMode::Paused => 'P',
    };
    vec![Span::styled(
        format!(" [{mode_char}:{offset}↑ {viewport_height}/{content_height}]"),
        Style::default().fg(PatinaTheme::MUTED),
    )]
}

/// Produces an update-available notification span.
///
/// Returns an empty vec when no update is available.
///
/// # Examples
///
/// ```rust,ignore
/// let spans = update_spans(Some("2.0.0"));
/// assert_eq!(spans.len(), 2);
/// ```
#[must_use]
pub fn update_spans(version: Option<&str>) -> Vec<Span<'static>> {
    let Some(version) = version else {
        return Vec::new();
    };
    vec![
        Span::raw(" "),
        Span::styled(
            format!("UPD:{version}"),
            Style::default().fg(PatinaTheme::WARNING),
        ),
    ]
}

/// Produces a plan-pending indicator span when a plan review modal is active.
///
/// Returns an empty vec when no plan is pending.
///
/// # Examples
///
/// ```rust,ignore
/// let spans = plan_spans(true);
/// assert_eq!(spans.len(), 2);
/// ```
#[must_use]
pub fn plan_spans(has_plan: bool) -> Vec<Span<'static>> {
    if !has_plan {
        return Vec::new();
    }
    vec![
        Span::raw(" "),
        Span::styled(
            "[PLAN]".to_string(),
            Style::default()
                .fg(PatinaTheme::WARNING)
                .add_modifier(Modifier::BOLD),
        ),
    ]
}

/// Collects all status bar spans from a [`RenderView`] by delegating to pure sub-functions.
///
/// The `feedback` parameter provides freshly-computed viewport and content
/// heights from the current render pass, ensuring the scroll indicator in the
/// status bar reflects the actual layout rather than stale values from the
/// previous frame.
///
/// # Errors
///
/// This function does not return errors.
#[must_use]
pub fn build_status_bar_spans(view: &RenderView, feedback: &RenderFeedback) -> Vec<Span<'static>> {
    let mut spans = Vec::new();

    spans.extend(branch_spans(view.worktree_branch));
    spans.extend(modified_spans(view.worktree_modified));
    spans.extend(upstream_spans(view.worktree_ahead, view.worktree_behind));
    spans.extend(focus_spans(view.focus_area));
    spans.extend(plan_spans(view.pending_plan.is_some()));

    let selection = view
        .selection
        .range()
        .map(|(start, end)| (start.line, end.line));
    spans.extend(selection_spans(selection));

    let budget = view.token_budget;
    spans.extend(budget_spans(
        budget.used(),
        budget.limit(),
        budget.is_critical(),
        budget.is_warning(),
    ));

    spans.extend(context_spans(view.context_tokens_injected));
    spans.extend(cost_spans(view.session_cost_usd));
    spans.extend(update_spans(view.update_available));

    // Use feedback-provided viewport/content heights so the scroll indicator
    // reflects the layout computed in *this* frame, not the previous one.
    spans.extend(scroll_spans(
        view.scroll_state.mode(),
        view.scroll_state.offset(),
        feedback.viewport_height,
        feedback.content_height,
    ));

    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_spans_with_branch_returns_two_spans() {
        let spans = branch_spans(Some("main"));
        assert_eq!(spans.len(), 2);
        // First span is the icon
        assert_eq!(spans[0].content, " ");
        // Second span is the branch name
        assert_eq!(spans[1].content, "main");
    }

    #[test]
    fn branch_spans_none_returns_empty() {
        let spans = branch_spans(None);
        assert!(spans.is_empty());
    }

    #[test]
    fn modified_spans_zero_returns_empty() {
        let spans = modified_spans(0);
        assert!(spans.is_empty());
    }

    #[test]
    fn modified_spans_nonzero_returns_indicator() {
        let spans = modified_spans(5);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[1].content, "●5");
    }

    #[test]
    fn upstream_spans_both_nonzero() {
        let spans = upstream_spans(3, 2);
        assert_eq!(spans.len(), 4);
        assert_eq!(spans[1].content, "↑3");
        assert_eq!(spans[3].content, "↓2");
    }

    #[test]
    fn upstream_spans_both_zero_returns_empty() {
        let spans = upstream_spans(0, 0);
        assert!(spans.is_empty());
    }

    #[test]
    fn upstream_spans_only_ahead() {
        let spans = upstream_spans(1, 0);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[1].content, "↑1");
    }

    #[test]
    fn upstream_spans_only_behind() {
        let spans = upstream_spans(0, 4);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[1].content, "↓4");
    }

    #[test]
    fn focus_spans_content() {
        let spans = focus_spans(FocusArea::Content);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[1].content, "[CONTENT]");
    }

    #[test]
    fn focus_spans_input() {
        let spans = focus_spans(FocusArea::Input);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[1].content, "[INPUT]");
    }

    #[test]
    fn selection_spans_none_returns_empty() {
        let spans = selection_spans(None);
        assert!(spans.is_empty());
    }

    #[test]
    fn selection_spans_some_returns_range() {
        let spans = selection_spans(Some((5, 12)));
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[1].content, "SEL:L5-L12");
    }

    #[test]
    fn budget_spans_zero_used_returns_empty() {
        let spans = budget_spans(0, 200_000, false, false);
        assert!(spans.is_empty());
    }

    #[test]
    fn budget_spans_normal_usage() {
        let spans = budget_spans(50_000, 200_000, false, false);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[1].content, "50k/200k");
    }

    #[test]
    fn budget_spans_critical_color() {
        let spans = budget_spans(190_000, 200_000, true, true);
        assert_eq!(spans.len(), 2);
        // Critical takes priority — verify span contains the right text
        assert_eq!(spans[1].content, "190k/200k");
        assert_eq!(spans[1].style.fg, Some(PatinaTheme::ERROR));
    }

    #[test]
    fn budget_spans_warning_color() {
        let spans = budget_spans(150_000, 200_000, false, true);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[1].style.fg, Some(PatinaTheme::WARNING));
    }

    #[test]
    fn context_spans_zero_returns_empty() {
        let spans = context_spans(0);
        assert!(spans.is_empty());
    }

    #[test]
    fn context_spans_nonzero_returns_indicator() {
        let spans = context_spans(8000);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[1].content, "CTX:8k");
    }

    #[test]
    fn scroll_spans_follow_mode() {
        let spans = scroll_spans(AutoScrollMode::Follow, 0, 40, 120);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, " [F:0↑ 40/120]");
    }

    #[test]
    fn scroll_spans_manual_mode() {
        let spans = scroll_spans(AutoScrollMode::Manual, 15, 40, 120);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, " [M:15↑ 40/120]");
    }

    #[test]
    fn scroll_spans_paused_mode() {
        let spans = scroll_spans(AutoScrollMode::Paused, 5, 30, 80);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, " [P:5↑ 30/80]");
    }

    #[test]
    fn update_spans_none_returns_empty() {
        let spans = update_spans(None);
        assert!(spans.is_empty());
    }

    #[test]
    fn update_spans_some_returns_indicator() {
        let spans = update_spans(Some("2.0.0"));
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[1].content, "UPD:2.0.0");
        assert_eq!(spans[1].style.fg, Some(PatinaTheme::WARNING));
    }

    // =========================================================================
    // C2: cost_spans tests
    // =========================================================================

    // =========================================================================
    // plan_spans tests
    // =========================================================================

    #[test]
    fn plan_spans_active_returns_indicator() {
        let spans = plan_spans(true);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[1].content, "[PLAN]");
        assert_eq!(spans[1].style.fg, Some(PatinaTheme::WARNING));
    }

    #[test]
    fn plan_spans_inactive_returns_empty() {
        let spans = plan_spans(false);
        assert!(spans.is_empty());
    }

    // =========================================================================
    // C2: cost_spans tests
    // =========================================================================

    #[test]
    fn cost_spans_below_threshold_returns_empty() {
        let spans = cost_spans(0.0);
        assert!(spans.is_empty());
        let spans = cost_spans(0.0005);
        assert!(spans.is_empty());
    }

    #[test]
    fn cost_spans_sub_cent_uses_three_decimals() {
        let spans = cost_spans(0.005);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[1].content, "$0.005");
    }

    #[test]
    fn cost_spans_above_cent_uses_two_decimals() {
        let spans = cost_spans(0.12);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[1].content, "$0.12");
    }

    #[test]
    fn cost_spans_dollar_amount() {
        let spans = cost_spans(1.50);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[1].content, "$1.50");
        assert_eq!(spans[1].style.fg, Some(PatinaTheme::VERDIGRIS));
    }
}
