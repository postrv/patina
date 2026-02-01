//! Permission prompt widget for tool approval.
//!
//! This widget displays a modal dialog asking the user to approve tool execution.
//!
//! # Keybindings
//!
//! - `y` or `Enter` - Allow once (session grant)
//! - `a` - Allow always (persistent rule)
//! - `n` or `Esc` - Deny
//!
//! # Example
//!
//! ```
//! use patina::tui::widgets::permission_prompt::{PermissionPromptState, PermissionPromptWidget};
//! use patina::permissions::{PermissionRequest, PermissionResponse};
//!
//! let request = PermissionRequest::new("Bash", Some("git status"), "Execute git status command");
//! let state = PermissionPromptState::new(request);
//! let widget = PermissionPromptWidget::new(&state);
//! // Render widget centered in the frame
//! ```

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap},
};

use crate::permissions::{PermissionRequest, PermissionResponse};

/// Represents which option is currently selected in the prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectedOption {
    /// "Allow Once" option selected.
    #[default]
    AllowOnce,
    /// "Allow Always" option selected.
    AllowAlways,
    /// "Deny" option selected.
    Deny,
}

impl SelectedOption {
    /// Moves to the next option (wraps around).
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::AllowOnce => Self::AllowAlways,
            Self::AllowAlways => Self::Deny,
            Self::Deny => Self::AllowOnce,
        }
    }

    /// Moves to the previous option (wraps around).
    #[must_use]
    pub fn prev(self) -> Self {
        match self {
            Self::AllowOnce => Self::Deny,
            Self::AllowAlways => Self::AllowOnce,
            Self::Deny => Self::AllowAlways,
        }
    }

    /// Converts the selected option to a permission response.
    #[must_use]
    pub fn to_response(self) -> PermissionResponse {
        match self {
            Self::AllowOnce => PermissionResponse::AllowOnce,
            Self::AllowAlways => PermissionResponse::AllowAlways,
            Self::Deny => PermissionResponse::Deny,
        }
    }
}

/// State for the permission prompt widget.
#[derive(Debug, Clone)]
pub struct PermissionPromptState {
    /// The permission request being displayed.
    request: PermissionRequest,

    /// Currently selected option.
    selected: SelectedOption,

    /// Whether the prompt has been answered.
    answered: bool,

    /// The response if answered.
    response: Option<PermissionResponse>,
}

impl PermissionPromptState {
    /// Creates a new permission prompt state for the given request.
    #[must_use]
    pub fn new(request: PermissionRequest) -> Self {
        Self {
            request,
            selected: SelectedOption::default(),
            answered: false,
            response: None,
        }
    }

    /// Returns the permission request.
    #[must_use]
    pub fn request(&self) -> &PermissionRequest {
        &self.request
    }

    /// Returns the currently selected option.
    #[must_use]
    pub fn selected(&self) -> SelectedOption {
        self.selected
    }

    /// Moves selection to the next option.
    pub fn select_next(&mut self) {
        self.selected = self.selected.next();
    }

    /// Moves selection to the previous option.
    pub fn select_previous(&mut self) {
        self.selected = self.selected.prev();
    }

    /// Selects "Allow Once" option.
    pub fn select_allow_once(&mut self) {
        self.selected = SelectedOption::AllowOnce;
    }

    /// Selects "Allow Always" option.
    pub fn select_allow_always(&mut self) {
        self.selected = SelectedOption::AllowAlways;
    }

    /// Selects "Deny" option.
    pub fn select_deny(&mut self) {
        self.selected = SelectedOption::Deny;
    }

    /// Confirms the currently selected option.
    pub fn confirm(&mut self) {
        self.answered = true;
        self.response = Some(self.selected.to_response());
    }

    /// Confirms with a specific response (bypasses selection).
    pub fn confirm_with(&mut self, response: PermissionResponse) {
        self.answered = true;
        self.response = Some(response);
    }

    /// Returns whether the prompt has been answered.
    #[must_use]
    pub fn is_answered(&self) -> bool {
        self.answered
    }

    /// Returns the response if the prompt has been answered.
    #[must_use]
    pub fn response(&self) -> Option<PermissionResponse> {
        self.response
    }

    /// Takes the response, consuming it (can only be called once).
    pub fn take_response(&mut self) -> Option<PermissionResponse> {
        self.response.take()
    }

    /// Resets the prompt state (for reuse).
    pub fn reset(&mut self) {
        self.answered = false;
        self.response = None;
        self.selected = SelectedOption::default();
    }
}

/// Widget for displaying the permission prompt.
///
/// Renders a centered modal dialog with the permission request details
/// and option buttons.
#[derive(Clone)]
pub struct PermissionPromptWidget<'a> {
    /// Reference to the prompt state.
    state: &'a PermissionPromptState,
}

impl<'a> PermissionPromptWidget<'a> {
    /// Creates a new permission prompt widget.
    #[must_use]
    pub fn new(state: &'a PermissionPromptState) -> Self {
        Self { state }
    }

    /// Calculates the area for the modal dialog.
    ///
    /// Returns a centered rectangle sized appropriately for the content.
    #[must_use]
    pub fn modal_area(area: Rect) -> Rect {
        // Modal should be about 60 chars wide and 12 lines tall
        let width = area.width.clamp(40, 60);
        let height = area.height.clamp(10, 14);

        let x = (area.width.saturating_sub(width)) / 2;
        let y = (area.height.saturating_sub(height)) / 2;

        Rect::new(x, y, width, height)
    }

    /// Renders an option button.
    fn render_option(&self, label: &str, hotkey: char, is_selected: bool) -> Line<'a> {
        let style = if is_selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };

        let hotkey_style = if is_selected {
            Style::default()
                .fg(Color::Yellow)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::UNDERLINED)
        };

        Line::from(vec![
            Span::raw(" "),
            Span::styled(format!("[{hotkey}]"), hotkey_style),
            Span::raw(" "),
            Span::styled(label.to_string(), style),
            Span::raw(" "),
        ])
    }
}

impl Widget for PermissionPromptWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Clear the background
        Clear.render(area, buf);

        // Draw the main block
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Permission Required ")
            .title_alignment(Alignment::Center)
            .border_style(Style::default().fg(Color::Yellow))
            .style(Style::default().bg(Color::DarkGray));

        let inner = block.inner(area);
        block.render(area, buf);

        // Layout the content vertically
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Tool name
                Constraint::Length(1), // Separator
                Constraint::Length(2), // Input (may wrap)
                Constraint::Length(1), // Separator
                Constraint::Min(2),    // Description
                Constraint::Length(1), // Separator
                Constraint::Length(1), // Options
                Constraint::Length(1), // Keybinding hints
            ])
            .split(inner);

        // Tool name
        let tool_line = Line::from(vec![
            Span::styled("Tool: ", Style::default().fg(Color::White)),
            Span::styled(
                self.state.request.tool_name.clone(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
        buf.set_line(chunks[0].x, chunks[0].y, &tool_line, chunks[0].width);

        // Tool input (if present)
        if let Some(ref input) = self.state.request.tool_input {
            let input_text = if input.len() > (inner.width as usize).saturating_sub(8) {
                // Truncate long inputs
                let max_len = (inner.width as usize).saturating_sub(11);
                format!("{}...", &input[..max_len.min(input.len())])
            } else {
                input.clone()
            };

            let input_para = Paragraph::new(Line::from(vec![
                Span::styled("Input: ", Style::default().fg(Color::White)),
                Span::styled(input_text, Style::default().fg(Color::Yellow)),
            ]))
            .wrap(Wrap { trim: true });
            input_para.render(chunks[2], buf);
        }

        // Description
        let desc_para = Paragraph::new(self.state.request.description.clone())
            .style(Style::default().fg(Color::White))
            .wrap(Wrap { trim: true });
        desc_para.render(chunks[4], buf);

        // Options (horizontal layout)
        let options_area = chunks[6];
        let option_width = options_area.width / 3;

        // Allow Once
        let allow_once = self.render_option(
            "Allow Once",
            'y',
            self.state.selected == SelectedOption::AllowOnce,
        );
        buf.set_line(options_area.x, options_area.y, &allow_once, option_width);

        // Allow Always
        let allow_always = self.render_option(
            "Allow Always",
            'a',
            self.state.selected == SelectedOption::AllowAlways,
        );
        buf.set_line(
            options_area.x + option_width,
            options_area.y,
            &allow_always,
            option_width,
        );

        // Deny
        let deny = self.render_option("Deny", 'n', self.state.selected == SelectedOption::Deny);
        buf.set_line(
            options_area.x + option_width * 2,
            options_area.y,
            &deny,
            option_width,
        );

        // Keybinding hints
        let hints = Line::from(vec![
            Span::styled("←→", Style::default().fg(Color::Cyan)),
            Span::raw(":select "),
            Span::styled("Enter", Style::default().fg(Color::Cyan)),
            Span::raw(":confirm "),
            Span::styled("Esc", Style::default().fg(Color::Cyan)),
            Span::raw(":deny"),
        ]);
        buf.set_line(chunks[7].x, chunks[7].y, &hints, chunks[7].width);
    }
}

/// Handles keyboard input for the permission prompt.
///
/// Returns `Some(response)` if a decision was made, `None` if the input
/// was handled but no decision was made.
///
/// # Arguments
///
/// * `state` - The prompt state to update
/// * `key` - The key that was pressed (as char or special key indicator)
///
/// # Returns
///
/// The permission response if the user made a decision.
pub fn handle_key_input(
    state: &mut PermissionPromptState,
    key: char,
) -> Option<PermissionResponse> {
    match key {
        'y' | 'Y' => {
            state.confirm_with(PermissionResponse::AllowOnce);
            state.response()
        }
        'a' | 'A' => {
            state.confirm_with(PermissionResponse::AllowAlways);
            state.response()
        }
        'n' | 'N' => {
            state.confirm_with(PermissionResponse::Deny);
            state.response()
        }
        '\r' | '\n' => {
            // Enter - confirm current selection
            state.confirm();
            state.response()
        }
        '\x1b' => {
            // Escape - deny
            state.confirm_with(PermissionResponse::Deny);
            state.response()
        }
        'h' | '\x08' => {
            // Left arrow (h in vim, or backspace for left)
            state.select_previous();
            None
        }
        'l' | '\t' => {
            // Right arrow (l in vim, or tab for right)
            state.select_next();
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // SelectedOption tests
    // =========================================================================

    #[test]
    fn test_selected_option_default() {
        assert_eq!(SelectedOption::default(), SelectedOption::AllowOnce);
    }

    #[test]
    fn test_selected_option_next() {
        assert_eq!(
            SelectedOption::AllowOnce.next(),
            SelectedOption::AllowAlways
        );
        assert_eq!(SelectedOption::AllowAlways.next(), SelectedOption::Deny);
        assert_eq!(SelectedOption::Deny.next(), SelectedOption::AllowOnce);
    }

    #[test]
    fn test_selected_option_prev() {
        assert_eq!(SelectedOption::AllowOnce.prev(), SelectedOption::Deny);
        assert_eq!(
            SelectedOption::AllowAlways.prev(),
            SelectedOption::AllowOnce
        );
        assert_eq!(SelectedOption::Deny.prev(), SelectedOption::AllowAlways);
    }

    #[test]
    fn test_selected_option_to_response() {
        assert_eq!(
            SelectedOption::AllowOnce.to_response(),
            PermissionResponse::AllowOnce
        );
        assert_eq!(
            SelectedOption::AllowAlways.to_response(),
            PermissionResponse::AllowAlways
        );
        assert_eq!(SelectedOption::Deny.to_response(), PermissionResponse::Deny);
    }

    // =========================================================================
    // PermissionPromptState tests
    // =========================================================================

    #[test]
    fn test_state_new() {
        let request =
            PermissionRequest::new("Bash", Some("git status"), "Execute git status command");
        let state = PermissionPromptState::new(request);

        assert_eq!(state.request().tool_name, "Bash");
        assert_eq!(state.selected(), SelectedOption::AllowOnce);
        assert!(!state.is_answered());
        assert!(state.response().is_none());
    }

    #[test]
    fn test_state_navigation() {
        let request = PermissionRequest::new("Bash", None, "Test command");
        let mut state = PermissionPromptState::new(request);

        assert_eq!(state.selected(), SelectedOption::AllowOnce);

        state.select_next();
        assert_eq!(state.selected(), SelectedOption::AllowAlways);

        state.select_next();
        assert_eq!(state.selected(), SelectedOption::Deny);

        state.select_previous();
        assert_eq!(state.selected(), SelectedOption::AllowAlways);
    }

    #[test]
    fn test_state_direct_selection() {
        let request = PermissionRequest::new("Bash", None, "Test");
        let mut state = PermissionPromptState::new(request);

        state.select_deny();
        assert_eq!(state.selected(), SelectedOption::Deny);

        state.select_allow_always();
        assert_eq!(state.selected(), SelectedOption::AllowAlways);

        state.select_allow_once();
        assert_eq!(state.selected(), SelectedOption::AllowOnce);
    }

    #[test]
    fn test_state_confirm() {
        let request = PermissionRequest::new("Bash", None, "Test");
        let mut state = PermissionPromptState::new(request);

        state.select_allow_always();
        state.confirm();

        assert!(state.is_answered());
        assert_eq!(state.response(), Some(PermissionResponse::AllowAlways));
    }

    #[test]
    fn test_state_confirm_with() {
        let request = PermissionRequest::new("Bash", None, "Test");
        let mut state = PermissionPromptState::new(request);

        state.confirm_with(PermissionResponse::Deny);

        assert!(state.is_answered());
        assert_eq!(state.response(), Some(PermissionResponse::Deny));
    }

    #[test]
    fn test_state_take_response() {
        let request = PermissionRequest::new("Bash", None, "Test");
        let mut state = PermissionPromptState::new(request);

        state.confirm_with(PermissionResponse::AllowOnce);

        let response = state.take_response();
        assert_eq!(response, Some(PermissionResponse::AllowOnce));

        // Second take should return None
        assert!(state.take_response().is_none());
    }

    #[test]
    fn test_state_reset() {
        let request = PermissionRequest::new("Bash", None, "Test");
        let mut state = PermissionPromptState::new(request);

        state.select_deny();
        state.confirm();

        state.reset();

        assert!(!state.is_answered());
        assert!(state.response().is_none());
        assert_eq!(state.selected(), SelectedOption::AllowOnce);
    }

    // =========================================================================
    // Key input handling tests
    // =========================================================================

    #[test]
    fn test_key_input_y_allows_once() {
        let request = PermissionRequest::new("Bash", None, "Test");
        let mut state = PermissionPromptState::new(request);

        let response = handle_key_input(&mut state, 'y');
        assert_eq!(response, Some(PermissionResponse::AllowOnce));
    }

    #[test]
    fn test_key_input_a_allows_always() {
        let request = PermissionRequest::new("Bash", None, "Test");
        let mut state = PermissionPromptState::new(request);

        let response = handle_key_input(&mut state, 'a');
        assert_eq!(response, Some(PermissionResponse::AllowAlways));
    }

    #[test]
    fn test_key_input_n_denies() {
        let request = PermissionRequest::new("Bash", None, "Test");
        let mut state = PermissionPromptState::new(request);

        let response = handle_key_input(&mut state, 'n');
        assert_eq!(response, Some(PermissionResponse::Deny));
    }

    #[test]
    fn test_key_input_enter_confirms_selection() {
        let request = PermissionRequest::new("Bash", None, "Test");
        let mut state = PermissionPromptState::new(request);

        state.select_allow_always();
        let response = handle_key_input(&mut state, '\r');
        assert_eq!(response, Some(PermissionResponse::AllowAlways));
    }

    #[test]
    fn test_key_input_escape_denies() {
        let request = PermissionRequest::new("Bash", None, "Test");
        let mut state = PermissionPromptState::new(request);

        let response = handle_key_input(&mut state, '\x1b');
        assert_eq!(response, Some(PermissionResponse::Deny));
    }

    #[test]
    fn test_key_input_navigation() {
        let request = PermissionRequest::new("Bash", None, "Test");
        let mut state = PermissionPromptState::new(request);

        // Tab moves right
        let response = handle_key_input(&mut state, '\t');
        assert!(response.is_none());
        assert_eq!(state.selected(), SelectedOption::AllowAlways);

        // h moves left (vim style)
        let response = handle_key_input(&mut state, 'h');
        assert!(response.is_none());
        assert_eq!(state.selected(), SelectedOption::AllowOnce);
    }

    // =========================================================================
    // Modal area calculation tests
    // =========================================================================

    #[test]
    fn test_modal_area_centered() {
        let area = Rect::new(0, 0, 100, 50);
        let modal = PermissionPromptWidget::modal_area(area);

        // Should be centered
        assert!(modal.x > 0);
        assert!(modal.y > 0);
        assert!(modal.x + modal.width <= area.width);
        assert!(modal.y + modal.height <= area.height);
    }

    #[test]
    fn test_modal_area_small_terminal() {
        let area = Rect::new(0, 0, 40, 10);
        let modal = PermissionPromptWidget::modal_area(area);

        // Should fit within the area
        assert!(modal.width <= area.width);
        assert!(modal.height <= area.height);
    }
}
