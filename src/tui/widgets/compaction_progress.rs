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

use crate::tui::theme::PatinaTheme;

// Re-export state types from the shared module
pub use crate::types::ui_state::{CompactionProgressState, CompactionStatus};

/// Widget for displaying compaction progress.
pub struct CompactionProgressWidget<'a> {
    /// The progress state to render.
    state: &'a CompactionProgressState,
    /// Optional throbber character for animation during compaction.
    throbber: Option<char>,
}

impl<'a> CompactionProgressWidget<'a> {
    /// Creates a new compaction progress widget.
    ///
    /// # Arguments
    ///
    /// * `state` - The progress state to display
    #[must_use]
    pub fn new(state: &'a CompactionProgressState) -> Self {
        Self {
            state,
            throbber: None,
        }
    }

    /// Sets the throbber character for animated display.
    ///
    /// The throbber is shown during the `Compacting` status to indicate
    /// that work is in progress. Use characters like `['⠋', '⠙', '⠹', '⠸']`
    /// and cycle through them for animation.
    #[must_use]
    pub fn with_throbber(mut self, throbber: char) -> Self {
        self.throbber = Some(throbber);
        self
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
    pub fn render_status_line(&self) -> Line<'static> {
        let (icon, status_text, style) = match self.state.status() {
            CompactionStatus::Idle => (
                "○".to_string(),
                "Idle",
                Style::default().fg(PatinaTheme::MUTED),
            ),
            CompactionStatus::Compacting => {
                // Use throbber if available, otherwise default icon
                let icon = self
                    .throbber
                    .map_or_else(|| "◐".to_string(), |c| c.to_string());
                // Show "Auto-compacting..." for auto-triggered compaction
                let text = if self.state.is_auto() {
                    "Auto-compacting context..."
                } else {
                    "Compacting..."
                };
                (icon, text, Style::default().fg(PatinaTheme::WARNING))
            }
            CompactionStatus::Complete => (
                "✓".to_string(),
                "Complete",
                Style::default().fg(PatinaTheme::SUCCESS),
            ),
            CompactionStatus::Failed => (
                "✗".to_string(),
                "Failed",
                Style::default().fg(PatinaTheme::ERROR),
            ),
        };

        Line::from(vec![
            Span::styled(format!(" {} ", icon), style),
            Span::styled(status_text.to_string(), style),
        ])
    }

    /// Renders the progress bar.
    fn render_progress_bar(&self, width: u16) -> Line<'static> {
        let percent = (self.state.progress() * 100.0).round() as u8;
        let percent_str = format!("{:>3}%", percent);

        // Calculate bar width (leave room for brackets and percentage)
        let bar_width = width.saturating_sub(8) as usize;
        if bar_width == 0 {
            return Line::from(Span::styled(
                percent_str,
                Style::default().fg(PatinaTheme::VERDIGRIS),
            ));
        }

        let filled = (bar_width as f64 * self.state.progress()).round() as usize;
        let empty = bar_width.saturating_sub(filled);

        let bar_style = match self.state.status() {
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
        let before = Self::format_tokens(self.state.before_tokens());
        let target = Self::format_tokens(self.state.target_tokens());

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
        let after = self.state.after_tokens()?;
        let saved = self.state.saved_tokens()?;

        let after_str = Self::format_tokens(after);
        let saved_str = Self::format_tokens(saved);

        let savings_percent = if self.state.before_tokens() > 0 {
            (saved as f64 / self.state.before_tokens() as f64 * 100.0).round() as usize
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
        let has_savings = self.state.after_tokens().is_some();
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

    // 4.4.4.1 Tests: is_auto field to distinguish manual vs auto compaction
    #[test]
    fn test_state_defaults_to_manual_compaction() {
        let state = CompactionProgressState::new(10_000, 50_000);
        assert!(
            !state.is_auto(),
            "New state should default to manual (not auto)"
        );
    }

    #[test]
    fn test_state_auto_compaction_can_be_set() {
        let mut state = CompactionProgressState::new(10_000, 50_000);
        state.set_auto(true);
        assert!(state.is_auto(), "State should be auto after set_auto(true)");
    }

    #[test]
    fn test_new_auto_constructor() {
        let state = CompactionProgressState::new_auto(10_000, 50_000);
        assert!(
            state.is_auto(),
            "new_auto should create auto-compaction state"
        );
    }

    // 4.4.4.2 Tests: Auto-compacting context message
    #[test]
    fn test_status_text_manual() {
        let mut state = CompactionProgressState::new(10_000, 50_000);
        state.set_status(CompactionStatus::Compacting);
        let widget = CompactionProgressWidget::new(&state).with_throbber('⠋');
        let line = widget.render_status_line();
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            text.contains("Compacting..."),
            "Manual compaction should show 'Compacting...'"
        );
        assert!(
            !text.contains("Auto-"),
            "Manual compaction should not show 'Auto-'"
        );
    }

    #[test]
    fn test_status_text_auto() {
        let mut state = CompactionProgressState::new_auto(10_000, 50_000);
        state.set_status(CompactionStatus::Compacting);
        let widget = CompactionProgressWidget::new(&state).with_throbber('⠋');
        let line = widget.render_status_line();
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            text.contains("Auto-compacting"),
            "Auto compaction should show 'Auto-compacting'"
        );
    }

    // 4.4.4.3 Tests: Throbber animation
    #[test]
    fn test_throbber_animation_renders() {
        let mut state = CompactionProgressState::new(10_000, 50_000);
        state.set_status(CompactionStatus::Compacting);
        let widget = CompactionProgressWidget::new(&state).with_throbber('⠙');
        let line = widget.render_status_line();
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            text.contains('⠙'),
            "Compacting status should show throbber character"
        );
    }

    #[test]
    fn test_throbber_different_frames() {
        let mut state = CompactionProgressState::new(10_000, 50_000);
        state.set_status(CompactionStatus::Compacting);

        for throbber_char in ['⠋', '⠙', '⠹', '⠸'] {
            let widget = CompactionProgressWidget::new(&state).with_throbber(throbber_char);
            let line = widget.render_status_line();
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert!(
                text.contains(throbber_char),
                "Throbber char '{}' should be visible",
                throbber_char
            );
        }
    }

    #[test]
    fn test_idle_no_throbber() {
        let state = CompactionProgressState::new(10_000, 50_000);
        let widget = CompactionProgressWidget::new(&state).with_throbber('⠋');
        let line = widget.render_status_line();
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        // Idle status should show circle, not throbber
        assert!(text.contains('○'), "Idle should show ○ icon");
        assert!(!text.contains('⠋'), "Idle should not show throbber");
    }
}
