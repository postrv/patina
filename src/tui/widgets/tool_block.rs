//! Tool block rendering widget for displaying tool execution results.
//!
//! This widget renders tool executions with a distinctive style:
//! - Bronze header with ⚙ icon showing tool name
//! - Verdigris content with tool input and output
//! - Code background styling
//!
//! # Example
//!
//! ```rust,ignore
//! use patina::tui::widgets::tool_block::{ToolBlockState, ToolBlockWidget};
//!
//! let mut state = ToolBlockState::new("bash", "git status");
//! state.set_result("On branch main\nnothing to commit");
//!
//! let widget = ToolBlockWidget::new(&state);
//! // Render widget in a frame
//! ```

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget, Wrap},
};

use crate::tui::theme::PatinaTheme;

/// State for the tool block widget.
#[derive(Debug, Clone)]
pub struct ToolBlockState {
    /// Name of the tool being executed (e.g., "bash", "read_file").
    tool_name: String,

    /// Input provided to the tool (e.g., command, file path).
    tool_input: String,

    /// Result of tool execution, if complete.
    result: Option<String>,

    /// Whether the result represents an error.
    is_error: bool,
}

impl ToolBlockState {
    /// Creates a new tool block state.
    ///
    /// # Arguments
    ///
    /// * `tool_name` - Name of the tool (e.g., "bash", "read_file")
    /// * `tool_input` - Input provided to the tool
    #[must_use]
    pub fn new(tool_name: impl Into<String>, tool_input: impl Into<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            tool_input: tool_input.into(),
            result: None,
            is_error: false,
        }
    }

    /// Returns the tool name.
    #[must_use]
    pub fn tool_name(&self) -> &str {
        &self.tool_name
    }

    /// Returns the tool input.
    #[must_use]
    pub fn tool_input(&self) -> &str {
        &self.tool_input
    }

    /// Returns the result, if any.
    #[must_use]
    pub fn result(&self) -> Option<&str> {
        self.result.as_deref()
    }

    /// Returns whether this is an error result.
    #[must_use]
    pub fn is_error(&self) -> bool {
        self.is_error
    }

    /// Returns whether the tool execution is complete.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.result.is_some()
    }

    /// Sets the result of tool execution.
    pub fn set_result(&mut self, result: impl Into<String>) {
        self.result = Some(result.into());
        self.is_error = false;
    }

    /// Sets an error result.
    pub fn set_error(&mut self, error: impl Into<String>) {
        self.result = Some(error.into());
        self.is_error = true;
    }
}

/// Widget for rendering a tool execution block.
pub struct ToolBlockWidget<'a> {
    /// The state to render.
    state: &'a ToolBlockState,
}

impl<'a> ToolBlockWidget<'a> {
    /// Creates a new tool block widget.
    #[must_use]
    pub fn new(state: &'a ToolBlockState) -> Self {
        Self { state }
    }

    /// Renders the header line with tool icon and name.
    fn render_header(&self) -> Line<'static> {
        let icon = if self.state.is_error { "✗" } else { "⚙" };

        let header_style = if self.state.is_error {
            PatinaTheme::error().add_modifier(Modifier::BOLD)
        } else {
            PatinaTheme::tool_header()
        };

        Line::from(vec![
            Span::styled(format!(" {} ", icon), header_style),
            Span::styled(self.state.tool_name.clone(), header_style),
        ])
    }

    /// Renders the input line showing the tool input.
    fn render_input(&self) -> Line<'static> {
        Line::from(vec![
            Span::styled("  › ", PatinaTheme::prompt()),
            Span::styled(
                self.state.tool_input.clone(),
                Style::default().fg(PatinaTheme::TOOL_CONTENT),
            ),
        ])
    }

    /// Renders the result section.
    fn render_result(&self) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        if let Some(result) = &self.state.result {
            // Separator
            lines.push(Line::from(""));

            let result_style = if self.state.is_error {
                PatinaTheme::error()
            } else {
                Style::default().fg(PatinaTheme::TOOL_CONTENT)
            };

            // Result lines with indentation
            for line in result.lines() {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(line.to_string(), result_style),
                ]));
            }
        } else {
            // Pending state
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("Running...", PatinaTheme::streaming()),
            ]));
        }

        lines
    }
}

impl Widget for ToolBlockWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Create the outer block with theme styling
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(PatinaTheme::BORDER))
            .style(Style::default().bg(PatinaTheme::BG_CODE));

        let inner = block.inner(area);
        block.render(area, buf);

        // Layout: header, input, result
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Header
                Constraint::Length(1), // Input
                Constraint::Min(1),    // Result
            ])
            .split(inner);

        // Render header
        let header = self.render_header();
        Paragraph::new(header).render(layout[0], buf);

        // Render input
        let input = self.render_input();
        Paragraph::new(input).render(layout[1], buf);

        // Render result
        let result_lines = self.render_result();
        Paragraph::new(result_lines)
            .wrap(Wrap { trim: false })
            .render(layout[2], buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_creation() {
        let state = ToolBlockState::new("bash", "ls -la");
        assert_eq!(state.tool_name(), "bash");
        assert_eq!(state.tool_input(), "ls -la");
        assert!(state.result().is_none());
        assert!(!state.is_error());
        assert!(!state.is_complete());
    }

    #[test]
    fn test_state_with_result() {
        let mut state = ToolBlockState::new("bash", "echo hello");
        state.set_result("hello");

        assert_eq!(state.result(), Some("hello"));
        assert!(!state.is_error());
        assert!(state.is_complete());
    }

    #[test]
    fn test_state_with_error() {
        let mut state = ToolBlockState::new("bash", "bad-command");
        state.set_error("Command not found");

        assert_eq!(state.result(), Some("Command not found"));
        assert!(state.is_error());
        assert!(state.is_complete());
    }

    #[test]
    fn test_render_header_normal() {
        let state = ToolBlockState::new("bash", "ls");
        let widget = ToolBlockWidget::new(&state);
        let header = widget.render_header();

        let spans: Vec<_> = header.spans.iter().collect();
        assert!(!spans.is_empty());
    }

    #[test]
    fn test_render_header_error() {
        let mut state = ToolBlockState::new("bash", "bad");
        state.set_error("Error");
        let widget = ToolBlockWidget::new(&state);
        let header = widget.render_header();

        // Error header should contain error icon
        let content: String = header.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(content.contains("✗"));
    }

    #[test]
    fn test_render_pending() {
        let state = ToolBlockState::new("bash", "long-command");
        let widget = ToolBlockWidget::new(&state);
        let result = widget.render_result();

        // Should show "Running..."
        let content: String = result
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.to_string())
            .collect();
        assert!(content.contains("Running"));
    }
}
