//! Plan review widget for displaying a plan approval modal.
//!
//! Renders a centered modal showing plan title, steps with selection highlight,
//! and keybinding hints for approve/reject/navigate.

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap},
};

use crate::types::ui_state::PlanState;

/// Widget for displaying a plan review modal.
///
/// Shows the plan title, a list of steps with the currently selected step
/// highlighted, and a footer with keybinding hints.
pub struct PlanReviewWidget<'a> {
    state: &'a PlanState,
}

impl<'a> PlanReviewWidget<'a> {
    /// Creates a new plan review widget.
    #[must_use]
    pub fn new(state: &'a PlanState) -> Self {
        Self { state }
    }

    /// Calculates the modal area centered in the given terminal area.
    ///
    /// The modal width is 60 chars and height scales with step count.
    #[must_use]
    pub fn modal_area(area: Rect, step_count: usize) -> Rect {
        let width = area.width.clamp(20, 70);
        // Title (1) + border (2) + steps + separator (1) + footer (1) + padding (2)
        let content_height = (step_count + 7) as u16;
        let min_h = 8u16.min(area.height);
        let max_h = area.height.saturating_sub(4).max(min_h);
        let height = content_height.clamp(min_h, max_h);

        let x = (area.width.saturating_sub(width)) / 2;
        let y = (area.height.saturating_sub(height)) / 2;

        Rect::new(x, y, width, height)
    }
}

impl Widget for PlanReviewWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" Plan: {} ", self.state.title))
            .title_alignment(Alignment::Center)
            .border_style(Style::default().fg(Color::Cyan))
            .style(Style::default().bg(Color::DarkGray));

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 3 || inner.width < 10 {
            return;
        }

        // Layout: steps list + separator + footer
        let footer_height = 2u16;
        let steps_height = inner.height.saturating_sub(footer_height);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(steps_height),
                Constraint::Length(footer_height),
            ])
            .split(inner);

        // Render steps
        let mut step_lines: Vec<Line<'_>> = Vec::new();
        for (i, step) in self.state.steps.iter().enumerate() {
            let is_selected = i == self.state.selected_index;
            let marker = if is_selected { ">" } else { " " };
            let num = format!("{:>2}.", i + 1);

            let style = if is_selected {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };

            let marker_style = if is_selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let mut spans = vec![
                Span::styled(marker.to_string(), marker_style),
                Span::styled(format!(" {num} "), Style::default().fg(Color::DarkGray)),
                Span::styled(step.description.clone(), style),
            ];

            if !step.tool_calls.is_empty() {
                let tools = step.tool_calls.join(", ");
                spans.push(Span::styled(
                    format!(" [{tools}]"),
                    Style::default().fg(Color::DarkGray),
                ));
            }

            step_lines.push(Line::from(spans));
        }

        let steps_para = Paragraph::new(step_lines).wrap(Wrap { trim: false });
        steps_para.render(chunks[0], buf);

        // Footer with keybindings
        let footer = Line::from(vec![
            Span::styled(" [Enter] ", Style::default().fg(Color::Green)),
            Span::styled("Approve  ", Style::default().fg(Color::White)),
            Span::styled("[Esc] ", Style::default().fg(Color::Red)),
            Span::styled("Reject  ", Style::default().fg(Color::White)),
            Span::styled("[Up/Down] ", Style::default().fg(Color::Yellow)),
            Span::styled("Navigate", Style::default().fg(Color::White)),
        ]);
        let footer_para = Paragraph::new(footer).alignment(Alignment::Center);
        footer_para.render(chunks[1], buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ui_state::{PlanState, PlanStep};
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    fn sample_plan() -> PlanState {
        PlanState::new(
            "toolu_1".into(),
            "Refactor Auth".into(),
            vec![
                PlanStep {
                    description: "Read auth module".into(),
                    tool_calls: vec!["read_file".into()],
                },
                PlanStep {
                    description: "Edit handler".into(),
                    tool_calls: vec!["edit".into()],
                },
                PlanStep {
                    description: "Run tests".into(),
                    tool_calls: vec!["bash".into()],
                },
            ],
        )
    }

    #[test]
    fn modal_area_centered() {
        let area = Rect::new(0, 0, 120, 40);
        let modal = PlanReviewWidget::modal_area(area, 3);
        assert!(modal.x > 0, "modal should be offset from left");
        assert!(modal.y > 0, "modal should be offset from top");
        assert!(modal.width <= 70, "modal should respect max width");
        assert!(modal.height >= 8, "modal should have minimum height");
    }

    #[test]
    fn modal_area_clamps_on_small_terminal() {
        let area = Rect::new(0, 0, 30, 10);
        let modal = PlanReviewWidget::modal_area(area, 3);
        assert!(modal.width >= 20, "width should be at least min");
        assert!(modal.height <= 10, "height should not exceed area");
    }

    #[test]
    fn widget_renders_without_panic() {
        let plan = sample_plan();
        let widget = PlanReviewWidget::new(&plan);
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        // Verify the plan title appears in the rendered output
        let content = buf_to_string(&buf);
        assert!(
            content.contains("Refactor Auth"),
            "Plan title should appear in output"
        );
    }

    #[test]
    fn widget_renders_step_descriptions() {
        let plan = sample_plan();
        let widget = PlanReviewWidget::new(&plan);
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        let content = buf_to_string(&buf);
        assert!(content.contains("Read auth module"), "Step 1 should appear");
        assert!(content.contains("Edit handler"), "Step 2 should appear");
        assert!(content.contains("Run tests"), "Step 3 should appear");
    }

    #[test]
    fn widget_renders_keybindings() {
        let plan = sample_plan();
        let widget = PlanReviewWidget::new(&plan);
        let area = Rect::new(0, 0, 70, 20);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        let content = buf_to_string(&buf);
        assert!(content.contains("Enter"), "Enter hint should appear");
        assert!(content.contains("Esc"), "Esc hint should appear");
    }

    #[test]
    fn widget_handles_tiny_area_without_panic() {
        let plan = sample_plan();
        let widget = PlanReviewWidget::new(&plan);
        let area = Rect::new(0, 0, 5, 3);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf); // should not panic
    }

    /// Helper to convert a buffer to a single string for content assertions.
    fn buf_to_string(buf: &Buffer) -> String {
        let mut output = String::new();
        for y in buf.area.y..buf.area.bottom() {
            for x in buf.area.x..buf.area.right() {
                output.push_str(buf.cell((x, y)).map_or(" ", |c| c.symbol()));
            }
            output.push('\n');
        }
        output
    }
}
