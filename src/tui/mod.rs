//! Terminal UI rendering

pub mod widgets;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::app::state::AppState;
use crate::types::Role;

pub fn render(frame: &mut Frame, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),    // Messages
            Constraint::Length(1), // Status bar
            Constraint::Length(3), // Input
        ])
        .split(frame.area());

    render_messages(frame, chunks[0], state);
    render_status_bar(frame, chunks[1], state);
    render_input(frame, chunks[2], state);
}

fn render_messages(frame: &mut Frame, area: Rect, state: &AppState) {
    let mut lines: Vec<Line> = Vec::new();

    for message in &state.messages {
        let (prefix, style) = match message.role {
            Role::User => ("You: ", Style::default().fg(Color::Cyan)),
            Role::Assistant => ("Claude: ", Style::default().fg(Color::Green)),
        };

        lines.push(Line::from(vec![Span::styled(
            prefix,
            style.add_modifier(Modifier::BOLD),
        )]));

        for line in message.content.lines() {
            lines.push(Line::from(Span::styled(line, style)));
        }

        lines.push(Line::from(""));
    }

    if let Some(ref response) = state.current_response {
        if state.is_loading() {
            lines.push(Line::from(vec![
                Span::styled(
                    "Claude: ",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{} ", state.throbber_char()),
                    Style::default().fg(Color::Yellow),
                ),
            ]));
        }

        for line in response.lines() {
            lines.push(Line::from(Span::styled(
                line,
                Style::default().fg(Color::Green),
            )));
        }
    }

    let messages = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Messages "))
        .wrap(Wrap { trim: false })
        .scroll((state.scroll_offset as u16, 0));

    frame.render_widget(messages, area);
}

fn render_status_bar(frame: &mut Frame, area: Rect, state: &AppState) {
    let mut spans = Vec::new();

    // Branch name
    if let Some(branch) = state.worktree_branch() {
        spans.push(Span::styled(" ", Style::default().fg(Color::Cyan)));
        spans.push(Span::styled(
            branch,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
    }

    // Modified count (dirty indicator)
    let modified = state.worktree_modified();
    if modified > 0 {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!("●{}", modified),
            Style::default().fg(Color::Yellow),
        ));
    }

    // Ahead indicator
    let ahead = state.worktree_ahead();
    if ahead > 0 {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!("↑{}", ahead),
            Style::default().fg(Color::Green),
        ));
    }

    // Behind indicator
    let behind = state.worktree_behind();
    if behind > 0 {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!("↓{}", behind),
            Style::default().fg(Color::Red),
        ));
    }

    let line = Line::from(spans);
    let status_bar = Paragraph::new(line).style(Style::default().bg(Color::DarkGray));

    frame.render_widget(status_bar, area);
}

fn render_input(frame: &mut Frame, area: Rect, state: &AppState) {
    let input = Paragraph::new(state.input.as_str())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Input (Enter to send, Ctrl+C to quit) "),
        )
        .style(Style::default().fg(Color::White));

    frame.render_widget(input, area);

    frame.set_cursor_position((area.x + state.input.len() as u16 + 1, area.y + 1));
}
