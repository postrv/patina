//! Compaction progress widget for displaying context compaction status.
//!
//! This widget shows the progress of context compaction operations, including:
//! - A visual progress bar
//! - Token counts (before, after, target)
//! - Savings calculation
//!
//! # Example
//!
//! ```rust,ignore
//! use patina::tui::widgets::compaction_progress::{
//!     CompactionProgressState, CompactionProgressWidget, CompactionStatus,
//! };
//!
//! let mut state = CompactionProgressState::new(10_000, 50_000);
//! state.set_status(CompactionStatus::Compacting);
//! state.set_progress(0.5);
//!
//! let widget = CompactionProgressWidget::new(&state);
//! // frame.render_widget(widget, area);
//! ```

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};
use std::fmt;

use crate::tui::theme::PatinaTheme;

/// Status of the compaction operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompactionStatus {
    /// No compaction in progress.
    #[default]
    Idle,
    /// Compaction is currently running.
    Compacting,
    /// Compaction completed successfully.
    Complete,
    /// Compaction failed.
    Failed,
}

impl fmt::Display for CompactionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => write!(f, "Idle"),
            Self::Compacting => write!(f, "Compacting..."),
            Self::Complete => write!(f, "Complete"),
            Self::Failed => write!(f, "Failed"),
        }
    }
}

/// State for the compaction progress widget.
///
/// Tracks the progress of a context compaction operation.
#[derive(Debug, Clone)]
pub struct CompactionProgressState {
    /// Target token count after compaction.
    target_tokens: usize,
    /// Token count before compaction.
    before_tokens: usize,
    /// Token count after compaction (set when complete).
    after_tokens: Option<usize>,
    /// Current progress (0.0 to 1.0).
    progress: f64,
    /// Current status of the compaction.
    status: CompactionStatus,
}

impl CompactionProgressState {
    /// Creates a new compaction progress state.
    ///
    /// # Arguments
    ///
    /// * `target_tokens` - Target token count after compaction
    /// * `before_tokens` - Token count before compaction
    #[must_use]
    pub fn new(target_tokens: usize, before_tokens: usize) -> Self {
        Self {
            target_tokens,
            before_tokens,
            after_tokens: None,
            progress: 0.0,
            status: CompactionStatus::Idle,
        }
    }

    /// Returns the target token count.
    #[must_use]
    pub fn target_tokens(&self) -> usize {
        self.target_tokens
    }

    /// Returns the token count before compaction.
    #[must_use]
    pub fn before_tokens(&self) -> usize {
        self.before_tokens
    }

    /// Returns the token count after compaction, if available.
    #[must_use]
    pub fn after_tokens(&self) -> Option<usize> {
        self.after_tokens
    }

    /// Sets the token count after compaction.
    pub fn set_after_tokens(&mut self, tokens: usize) {
        self.after_tokens = Some(tokens);
    }

    /// Returns the number of tokens saved by compaction.
    ///
    /// Returns `None` if compaction has not completed.
    #[must_use]
    pub fn saved_tokens(&self) -> Option<usize> {
        self.after_tokens
            .map(|after| self.before_tokens.saturating_sub(after))
    }

    /// Returns the current progress (0.0 to 1.0).
    #[must_use]
    pub fn progress(&self) -> f64 {
        self.progress
    }

    /// Sets the current progress.
    ///
    /// The value is clamped to the range [0.0, 1.0].
    pub fn set_progress(&mut self, progress: f64) {
        self.progress = progress.clamp(0.0, 1.0);
    }

    /// Returns the current status.
    #[must_use]
    pub fn status(&self) -> CompactionStatus {
        self.status
    }

    /// Sets the current status.
    pub fn set_status(&mut self, status: CompactionStatus) {
        self.status = status;
    }
}

/// Widget for displaying compaction progress.
pub struct CompactionProgressWidget<'a> {
    /// The progress state to render.
    state: &'a CompactionProgressState,
}

impl<'a> CompactionProgressWidget<'a> {
    /// Creates a new compaction progress widget.
    ///
    /// # Arguments
    ///
    /// * `state` - The progress state to display
    #[must_use]
    pub fn new(state: &'a CompactionProgressState) -> Self {
        Self { state }
    }

    /// Formats a token count for display.
    ///
    /// Uses K/M suffixes for large numbers to keep the display compact.
    fn format_tokens(tokens: usize) -> String {
        if tokens >= 1_000_000 {
            format!("{}M", tokens / 1_000_000)
        } else if tokens >= 10_000 {
            format!("{}K", tokens / 1_000)
        } else {
            tokens.to_string()
        }
    }

    /// Renders the status line with icon and text.
    fn render_status_line(&self) -> Line<'static> {
        let (icon, status_text, style) = match self.state.status {
            CompactionStatus::Idle => ("○", "Idle", Style::default().fg(PatinaTheme::MUTED)),
            CompactionStatus::Compacting => (
                "◐",
                "Compacting...",
                Style::default().fg(PatinaTheme::WARNING),
            ),
            CompactionStatus::Complete => {
                ("✓", "Complete", Style::default().fg(PatinaTheme::SUCCESS))
            }
            CompactionStatus::Failed => ("✗", "Failed", Style::default().fg(PatinaTheme::ERROR)),
        };

        Line::from(vec![
            Span::styled(format!(" {} ", icon), style),
            Span::styled(status_text.to_string(), style),
        ])
    }

    /// Renders the progress bar.
    fn render_progress_bar(&self, width: u16) -> Line<'static> {
        let percent = (self.state.progress * 100.0).round() as u8;
        let percent_str = format!("{:>3}%", percent);

        // Calculate bar width (leave room for brackets and percentage)
        let bar_width = width.saturating_sub(8) as usize;
        if bar_width == 0 {
            return Line::from(Span::styled(
                percent_str,
                Style::default().fg(PatinaTheme::VERDIGRIS),
            ));
        }

        let filled = (bar_width as f64 * self.state.progress).round() as usize;
        let empty = bar_width.saturating_sub(filled);

        let bar_style = match self.state.status {
            CompactionStatus::Complete => Style::default().fg(PatinaTheme::SUCCESS),
            CompactionStatus::Failed => Style::default().fg(PatinaTheme::ERROR),
            _ => Style::default().fg(PatinaTheme::VERDIGRIS),
        };

        let empty_style = Style::default().fg(PatinaTheme::MUTED);

        Line::from(vec![
            Span::raw(" ["),
            Span::styled("█".repeat(filled), bar_style),
            Span::styled("░".repeat(empty), empty_style),
            Span::raw("] "),
            Span::styled(percent_str, bar_style),
        ])
    }

    /// Renders the token counts line.
    fn render_token_counts(&self) -> Line<'static> {
        let before = Self::format_tokens(self.state.before_tokens);
        let target = Self::format_tokens(self.state.target_tokens);

        let label_style = Style::default().fg(PatinaTheme::MUTED);
        let value_style = Style::default().fg(PatinaTheme::VERDIGRIS_BRIGHT);

        Line::from(vec![
            Span::styled(" Before: ", label_style),
            Span::styled(before, value_style),
            Span::styled(" | Target: ", label_style),
            Span::styled(target, value_style),
        ])
    }

    /// Renders the after/savings line (when complete).
    fn render_savings_line(&self) -> Option<Line<'static>> {
        let after = self.state.after_tokens?;
        let saved = self.state.saved_tokens()?;

        let after_str = Self::format_tokens(after);
        let saved_str = Self::format_tokens(saved);

        let savings_percent = if self.state.before_tokens > 0 {
            (saved as f64 / self.state.before_tokens as f64 * 100.0).round() as usize
        } else {
            0
        };

        let label_style = Style::default().fg(PatinaTheme::MUTED);
        let value_style = Style::default().fg(PatinaTheme::VERDIGRIS_BRIGHT);
        let success_style = Style::default().fg(PatinaTheme::SUCCESS);

        Some(Line::from(vec![
            Span::styled(" After: ", label_style),
            Span::styled(after_str, value_style),
            Span::styled(" | Saved: ", label_style),
            Span::styled(
                format!("{} ({}%)", saved_str, savings_percent),
                success_style,
            ),
        ]))
    }
}

impl Widget for CompactionProgressWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 3 || area.height < 1 {
            return;
        }

        // Create the outer block
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(PatinaTheme::BORDER))
            .title(Span::styled(
                " Context Compaction ",
                Style::default().fg(PatinaTheme::BRONZE),
            ))
            .style(Style::default().bg(PatinaTheme::BG_SECONDARY));

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 1 {
            return;
        }

        // Determine layout based on available height
        let has_savings = self.state.after_tokens.is_some();
        let row_count = if has_savings { 4 } else { 3 };

        let constraints: Vec<Constraint> = (0..row_count)
            .map(|_| Constraint::Length(1))
            .chain(std::iter::once(Constraint::Min(0)))
            .collect();

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        // Render status line
        if !layout.is_empty() {
            let status_line = self.render_status_line();
            Paragraph::new(status_line).render(layout[0], buf);
        }

        // Render progress bar
        if layout.len() > 1 {
            let progress_line = self.render_progress_bar(inner.width);
            Paragraph::new(progress_line).render(layout[1], buf);
        }

        // Render token counts
        if layout.len() > 2 {
            let tokens_line = self.render_token_counts();
            Paragraph::new(tokens_line).render(layout[2], buf);
        }

        // Render savings (if complete)
        if has_savings && layout.len() > 3 {
            if let Some(savings_line) = self.render_savings_line() {
                Paragraph::new(savings_line).render(layout[3], buf);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compaction_status_default() {
        let status = CompactionStatus::default();
        assert_eq!(status, CompactionStatus::Idle);
    }

    #[test]
    fn test_compaction_progress_state_creation() {
        let state = CompactionProgressState::new(10_000, 50_000);
        assert_eq!(state.target_tokens(), 10_000);
        assert_eq!(state.before_tokens(), 50_000);
        assert_eq!(state.after_tokens(), None);
        assert!(state.progress().abs() < 0.001);
        assert_eq!(state.status(), CompactionStatus::Idle);
    }

    #[test]
    fn test_progress_clamp() {
        let mut state = CompactionProgressState::new(10_000, 50_000);

        state.set_progress(1.5);
        assert!((state.progress() - 1.0).abs() < 0.001);

        state.set_progress(-0.5);
        assert!(state.progress().abs() < 0.001);
    }

    #[test]
    fn test_format_tokens_small() {
        assert_eq!(CompactionProgressWidget::format_tokens(500), "500");
        assert_eq!(CompactionProgressWidget::format_tokens(9999), "9999");
    }

    #[test]
    fn test_format_tokens_thousands() {
        assert_eq!(CompactionProgressWidget::format_tokens(10_000), "10K");
        assert_eq!(CompactionProgressWidget::format_tokens(50_000), "50K");
        assert_eq!(CompactionProgressWidget::format_tokens(999_999), "999K");
    }

    #[test]
    fn test_format_tokens_millions() {
        assert_eq!(CompactionProgressWidget::format_tokens(1_000_000), "1M");
        assert_eq!(CompactionProgressWidget::format_tokens(10_000_000), "10M");
    }
}
