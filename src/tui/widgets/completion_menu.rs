//! Completion menu widget for slash command auto-completion.
//!
//! Renders a popup list of matching commands above the input area.
//! The currently selected entry is highlighted with an inverted style.
//!
//! # Layout
//!
//! The popup appears directly above the input box, left-aligned, showing
//! up to [`MAX_VISIBLE_ENTRIES`] entries. Each entry shows the command name,
//! an optional source indicator (`[P]` for plugin, `[M]` for MCP), and
//! a truncated description.
//!
//! # Example
//!
//! ```rust,ignore
//! use patina::tui::widgets::completion_menu::CompletionMenuWidget;
//! use patina::app::completion::CompletionState;
//!
//! if let Some(completion) = state.completion() {
//!     let widget = CompletionMenuWidget::new(completion);
//!     let area = CompletionMenuWidget::popup_area(input_area, completion.filtered().len());
//!     frame.render_widget(widget, area);
//! }
//! ```

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Widget},
};

use crate::app::completion::CompletionState;

/// Maximum number of entries visible in the popup at once.
const MAX_VISIBLE_ENTRIES: usize = 8;

/// Widget for rendering the completion popup menu.
#[derive(Clone)]
pub struct CompletionMenuWidget<'a> {
    state: &'a CompletionState,
}

impl<'a> CompletionMenuWidget<'a> {
    /// Creates a new completion menu widget.
    #[must_use]
    pub fn new(state: &'a CompletionState) -> Self {
        Self { state }
    }

    /// Calculates the popup area positioned above the input area.
    ///
    /// The popup is left-aligned with the input area and sized to fit
    /// the number of entries (capped at [`MAX_VISIBLE_ENTRIES`]).
    /// Includes 2 extra rows for the border.
    ///
    /// The popup is clamped to fit within the terminal bounds.
    #[must_use]
    pub fn popup_area(input_area: Rect, entry_count: usize) -> Rect {
        let visible = entry_count.clamp(1, MAX_VISIBLE_ENTRIES);
        let height = (visible as u16) + 2; // +2 for top/bottom border
        let width = input_area.width.min(50);

        // Position above the input area
        let y = input_area.y.saturating_sub(height);
        let x = input_area.x;

        Rect::new(x, y, width, height)
    }
}

impl Widget for CompletionMenuWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Clear background
        Clear.render(area, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Commands ")
            .title_alignment(Alignment::Left)
            .border_style(Style::default().fg(Color::Cyan))
            .style(Style::default().bg(Color::DarkGray));

        let inner = block.inner(area);
        block.render(area, buf);

        let filtered = self.state.filtered();
        let selected = self.state.selected_index();

        // Calculate scroll window to keep selection visible
        let visible_count = inner.height as usize;
        let scroll_offset = if selected >= visible_count {
            selected - visible_count + 1
        } else {
            0
        };

        for (i, entry) in filtered
            .iter()
            .skip(scroll_offset)
            .take(visible_count)
            .enumerate()
        {
            let absolute_idx = scroll_offset + i;
            let y = inner.y + i as u16;

            if y >= inner.y + inner.height {
                break;
            }

            let is_selected = absolute_idx == selected;

            let style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White).bg(Color::DarkGray)
            };

            let indicator = entry.source.indicator();
            let name_width =
                entry.name.len() + indicator.len() + if indicator.is_empty() { 0 } else { 1 };
            let desc_space = (inner.width as usize).saturating_sub(name_width + 3);

            let description = if entry.description.len() > desc_space {
                format!("{}…", &entry.description[..desc_space.saturating_sub(1)])
            } else {
                entry.description.clone()
            };

            let mut spans = vec![Span::styled(format!("/{}", entry.name), style)];

            if !indicator.is_empty() {
                let indicator_style = if is_selected {
                    Style::default()
                        .fg(Color::Yellow)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Yellow).bg(Color::DarkGray)
                };
                spans.push(Span::styled(format!(" {indicator}"), indicator_style));
            }

            let desc_style = if is_selected {
                Style::default().fg(Color::DarkGray).bg(Color::Cyan)
            } else {
                Style::default().fg(Color::Gray).bg(Color::DarkGray)
            };
            spans.push(Span::styled(format!(" {description}"), desc_style));

            let line = Line::from(spans);
            buf.set_line(inner.x, y, &line, inner.width);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::completion::{CompletionEntry, CompletionSource, CompletionState};

    fn sample_entries() -> Vec<CompletionEntry> {
        vec![
            CompletionEntry::new("agent", "Manage agents", CompletionSource::Builtin),
            CompletionEntry::new("continuous", "Continuous loop", CompletionSource::Builtin),
            CompletionEntry::new("help", "Show help", CompletionSource::Builtin),
            CompletionEntry::new("plugins", "List plugins", CompletionSource::Builtin),
            CompletionEntry::new(
                "terminal-setup",
                "Terminal config",
                CompletionSource::Builtin,
            ),
            CompletionEntry::new("worktree", "Git worktrees", CompletionSource::Builtin),
        ]
    }

    // 8.5.1 - Layout

    #[test]
    fn popup_area_positions_above_input() {
        let input_area = Rect::new(0, 20, 80, 3);
        let area = CompletionMenuWidget::popup_area(input_area, 6);

        // Should be above the input area
        assert!(area.y + area.height <= input_area.y);
        // Left-aligned
        assert_eq!(area.x, input_area.x);
    }

    #[test]
    fn popup_area_clamps_to_terminal() {
        // Input at the very top
        let input_area = Rect::new(0, 2, 80, 3);
        let area = CompletionMenuWidget::popup_area(input_area, 10);

        // Should clamp y to 0 (saturating_sub)
        assert_eq!(area.y, 0);
    }

    #[test]
    fn popup_area_scales_with_entry_count() {
        let input_area = Rect::new(0, 30, 80, 3);

        let area_3 = CompletionMenuWidget::popup_area(input_area, 3);
        let area_6 = CompletionMenuWidget::popup_area(input_area, 6);
        let area_10 = CompletionMenuWidget::popup_area(input_area, 10);

        assert_eq!(area_3.height, 5); // 3 + 2 border
        assert_eq!(area_6.height, 8); // 6 + 2 border
        assert_eq!(area_10.height, 10); // capped at MAX_VISIBLE_ENTRIES (8) + 2 border
    }

    #[test]
    fn popup_area_max_width() {
        let input_area = Rect::new(0, 20, 100, 3);
        let area = CompletionMenuWidget::popup_area(input_area, 6);
        assert_eq!(area.width, 50); // capped at 50
    }

    // 8.5.2 - Rendering

    #[test]
    fn renders_without_panic() {
        let state = CompletionState::new(sample_entries());
        let widget = CompletionMenuWidget::new(&state);

        let area = Rect::new(0, 0, 50, 10);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);
        // No panic = success
    }

    #[test]
    fn renders_entry_names() {
        let state = CompletionState::new(sample_entries());
        let widget = CompletionMenuWidget::new(&state);

        let area = Rect::new(0, 0, 50, 10);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        // Check that "/agent" text appears in the buffer
        let content: String = (0..buf.area.width)
            .map(|x| {
                buf.cell((x, 1))
                    .map(|c| c.symbol().chars().next().unwrap_or(' '))
                    .unwrap_or(' ')
            })
            .collect();
        assert!(
            content.contains("/agent"),
            "Expected '/agent' in: {content}"
        );
    }

    #[test]
    fn selected_entry_has_different_style() {
        let state = CompletionState::new(sample_entries());
        let widget = CompletionMenuWidget::new(&state);

        let area = Rect::new(0, 0, 50, 10);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        // First entry (selected) should have Cyan background
        let selected_cell = buf.cell((1, 1)).unwrap();
        assert_eq!(selected_cell.bg, Color::Cyan);

        // Second entry (not selected) should have DarkGray background
        let unselected_cell = buf.cell((1, 2)).unwrap();
        assert_eq!(unselected_cell.bg, Color::DarkGray);
    }

    #[test]
    fn plugin_entry_shows_indicator() {
        let entries = vec![CompletionEntry::new(
            "my-cmd",
            "A plugin command",
            CompletionSource::Plugin("test-plugin".to_string()),
        )];
        let state = CompletionState::new(entries);
        let widget = CompletionMenuWidget::new(&state);

        let area = Rect::new(0, 0, 50, 5);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        let content: String = (0..buf.area.width)
            .map(|x| {
                buf.cell((x, 1))
                    .map(|c| c.symbol().chars().next().unwrap_or(' '))
                    .unwrap_or(' ')
            })
            .collect();
        assert!(content.contains("[P]"), "Expected '[P]' in: {content}");
    }

    #[test]
    fn renders_border() {
        let state = CompletionState::new(sample_entries());
        let widget = CompletionMenuWidget::new(&state);

        let area = Rect::new(0, 0, 50, 10);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        // Top-left corner should be a border character
        let corner = buf.cell((0, 0)).unwrap().symbol();
        assert_eq!(corner, "┌");
    }

    #[test]
    fn empty_state_renders_without_panic() {
        let mut state = CompletionState::new(sample_entries());
        state.set_filter("nonexistent_garbage");
        assert!(state.filtered().is_empty());

        let widget = CompletionMenuWidget::new(&state);
        let area = Rect::new(0, 0, 50, 5);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);
    }
}
