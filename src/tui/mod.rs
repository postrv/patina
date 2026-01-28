//! Terminal UI rendering

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::app::state::{AppState, Role};

pub fn render(frame: &mut Frame, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .split(frame.area());

    render_messages(frame, chunks[0], state);
    render_input(frame, chunks[1], state);
}

fn render_messages(frame: &mut Frame, area: Rect, state: &AppState) {
    let mut lines: Vec<Line> = Vec::new();

    for message in &state.messages {
        let (prefix, style) = match message.role {
            Role::User => ("You: ", Style::default().fg(Color::Cyan)),
            Role::Assistant => ("Claude: ", Style::default().fg(Color::Green)),
        };

        lines.push(Line::from(vec![
            Span::styled(prefix, style.add_modifier(Modifier::BOLD)),
        ]));

        for line in message.content.lines() {
            lines.push(Line::from(Span::styled(line, style)));
        }

        lines.push(Line::from(""));
    }

    if let Some(ref response) = state.current_response {
        if state.is_loading() {
            lines.push(Line::from(vec![
                Span::styled("Claude: ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{} ", state.throbber_char()), Style::default().fg(Color::Yellow)),
            ]));
        }

        for line in response.lines() {
            lines.push(Line::from(Span::styled(line, Style::default().fg(Color::Green))));
        }
    }

    let messages = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Messages "))
        .wrap(Wrap { trim: false })
        .scroll((state.scroll_offset as u16, 0));

    frame.render_widget(messages, area);
}

fn render_input(frame: &mut Frame, area: Rect, state: &AppState) {
    let input = Paragraph::new(state.input.as_str())
        .block(Block::default().borders(Borders::ALL).title(" Input (Enter to send, Ctrl+C to quit) "))
        .style(Style::default().fg(Color::White));

    frame.render_widget(input, area);

    frame.set_cursor_position((
        area.x + state.input.len() as u16 + 1,
        area.y + 1,
    ));
}
