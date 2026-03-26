//! Question prompt widget for displaying an interactive question modal.
//!
//! Renders a centered modal showing the question, optional choices with
//! radio-button style selection, an optional free-text input, and
//! keybinding hints.

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap},
};

use crate::types::ui_state::QuestionState;

/// Widget for displaying a user question prompt modal.
///
/// Shows the question text, a list of options (if any) with the selected
/// option highlighted, a free-text input field (if allowed), and footer
/// with keybinding hints.
pub struct QuestionPromptWidget<'a> {
    state: &'a QuestionState,
}

impl<'a> QuestionPromptWidget<'a> {
    /// Creates a new question prompt widget.
    #[must_use]
    pub fn new(state: &'a QuestionState) -> Self {
        Self { state }
    }

    /// Calculates the modal area centered in the given terminal area.
    #[must_use]
    pub fn modal_area(area: Rect, option_count: usize, has_free_text: bool) -> Rect {
        let width = area.width.clamp(40, 65);
        // Question (2) + border (2) + options + free-text (2) + footer (1) + padding (2)
        let text_rows = if has_free_text { 3 } else { 0 };
        let content_height = (option_count + text_rows + 7) as u16;
        let height = content_height.clamp(8, area.height.saturating_sub(4));

        let x = (area.width.saturating_sub(width)) / 2;
        let y = (area.height.saturating_sub(height)) / 2;

        Rect::new(x, y, width, height)
    }
}

impl Widget for QuestionPromptWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Question ")
            .title_alignment(Alignment::Center)
            .border_style(Style::default().fg(Color::Magenta))
            .style(Style::default().bg(Color::DarkGray));

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 3 || inner.width < 10 {
            return;
        }

        // Build content lines
        let mut lines: Vec<Line<'_>> = Vec::new();

        // Question text
        lines.push(Line::from(Span::styled(
            self.state.question.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));

        // Options
        for (i, option) in self.state.options.iter().enumerate() {
            let is_selected = i == self.state.selected_option && !self.state.in_free_text;
            let bullet = if is_selected { "(*)" } else { "( )" };

            let style = if is_selected {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };

            let bullet_style = if is_selected {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            lines.push(Line::from(vec![
                Span::styled(format!("  {bullet} "), bullet_style),
                Span::styled(option.clone(), style),
            ]));
        }

        // Free-text input
        if self.state.allow_free_text {
            if !self.state.options.is_empty() {
                lines.push(Line::from(""));
            }

            let input_style = if self.state.in_free_text {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let cursor = if self.state.in_free_text { "_" } else { "" };
            lines.push(Line::from(vec![
                Span::styled("  > ", input_style),
                Span::styled(
                    format!("{}{cursor}", self.state.free_text_input),
                    input_style,
                ),
            ]));
        }

        lines.push(Line::from(""));

        // Footer with keybindings
        let mut footer_spans = vec![
            Span::styled(" [Enter] ", Style::default().fg(Color::Green)),
            Span::styled("Submit  ", Style::default().fg(Color::White)),
            Span::styled("[Esc] ", Style::default().fg(Color::Red)),
            Span::styled("Cancel", Style::default().fg(Color::White)),
        ];

        if !self.state.options.is_empty() {
            footer_spans.push(Span::styled(
                "  [Up/Down] ",
                Style::default().fg(Color::Yellow),
            ));
            footer_spans.push(Span::styled("Navigate", Style::default().fg(Color::White)));
        }

        if self.state.allow_free_text && !self.state.options.is_empty() {
            footer_spans.push(Span::styled("  [Tab] ", Style::default().fg(Color::Yellow)));
            footer_spans.push(Span::styled("Toggle", Style::default().fg(Color::White)));
        }

        lines.push(Line::from(footer_spans));

        let para = Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .alignment(Alignment::Left);
        para.render(inner, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ui_state::QuestionState;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    fn question_with_options() -> QuestionState {
        QuestionState::new(
            "toolu_1".into(),
            "Which framework?".into(),
            vec!["React".into(), "Vue".into(), "Svelte".into()],
            true,
        )
    }

    fn question_free_text_only() -> QuestionState {
        QuestionState::new("toolu_1".into(), "What name?".into(), vec![], true)
    }

    #[test]
    fn modal_area_centered() {
        let area = Rect::new(0, 0, 120, 40);
        let modal = QuestionPromptWidget::modal_area(area, 3, true);
        assert!(modal.x > 0);
        assert!(modal.y > 0);
        assert!(modal.width <= 65);
    }

    #[test]
    fn widget_renders_question_text() {
        let q = question_with_options();
        let widget = QuestionPromptWidget::new(&q);
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        let content = buf_to_string(&buf);
        assert!(
            content.contains("Which framework"),
            "Question text should appear"
        );
    }

    #[test]
    fn widget_renders_options() {
        let q = question_with_options();
        let widget = QuestionPromptWidget::new(&q);
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        let content = buf_to_string(&buf);
        assert!(content.contains("React"), "Option 1 should appear");
        assert!(content.contains("Vue"), "Option 2 should appear");
        assert!(content.contains("Svelte"), "Option 3 should appear");
    }

    #[test]
    fn widget_renders_free_text_input() {
        let mut q = question_free_text_only();
        q.free_text_input = "my answer".into();
        let widget = QuestionPromptWidget::new(&q);
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        let content = buf_to_string(&buf);
        assert!(content.contains("my answer"), "Free text should appear");
    }

    #[test]
    fn widget_renders_keybindings() {
        let q = question_with_options();
        let widget = QuestionPromptWidget::new(&q);
        let area = Rect::new(0, 0, 70, 20);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        let content = buf_to_string(&buf);
        assert!(content.contains("Enter"), "Enter hint should appear");
        assert!(content.contains("Esc"), "Esc hint should appear");
        assert!(
            content.contains("Tab"),
            "Tab hint should appear for mixed mode"
        );
    }

    #[test]
    fn widget_handles_tiny_area_without_panic() {
        let q = question_with_options();
        let widget = QuestionPromptWidget::new(&q);
        let area = Rect::new(0, 0, 5, 3);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);
    }

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
