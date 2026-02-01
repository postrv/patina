//! Terminal UI rendering

pub mod scroll;
pub mod selection;
pub mod theme;
pub mod widgets;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::app::state::AppState;
use crate::permissions::PermissionRequest;
use crate::tui::theme::PatinaTheme;
use crate::tui::widgets::permission_prompt::{PermissionPromptState, PermissionPromptWidget};
use crate::types::{ConversationEntry, Timeline};

/// Calculates the total number of displayed lines after wrapping.
///
/// This accounts for line wrapping when content is wider than the viewport.
/// Each logical line may wrap to multiple displayed lines.
///
/// # Arguments
///
/// * `lines` - The logical lines to measure
/// * `width` - The available width for content (excluding borders)
///
/// # Returns
///
/// The total number of displayed lines after wrapping
fn calculate_wrapped_height(lines: &[Line], width: usize) -> usize {
    if width == 0 {
        return lines.len();
    }

    lines
        .iter()
        .map(|line| {
            let line_width = line.width();
            if line_width == 0 {
                1 // Empty lines still take 1 row
            } else {
                // Ceiling division: how many rows needed for this line
                line_width.div_ceil(width)
            }
        })
        .sum()
}

/// Renders a timeline to a vector of lines for display.
///
/// This function converts timeline entries into styled lines suitable for
/// terminal display. Tool blocks are rendered inline immediately after
/// their associated assistant message.
///
/// # Arguments
///
/// * `timeline` - The timeline containing conversation entries
/// * `_width` - Available width for content (reserved for future wrapping)
///
/// # Returns
///
/// A vector of styled `Line` objects ready for display.
#[must_use]
pub fn render_timeline_to_lines(timeline: &Timeline, _width: usize) -> Vec<Line<'static>> {
    render_timeline_with_throbber(timeline, '⠋')
}

/// Renders a timeline to a vector of lines with animated throbber.
///
/// This variant accepts a throbber character for animated streaming display.
///
/// # Arguments
///
/// * `timeline` - The timeline containing conversation entries
/// * `throbber` - Character to display for streaming animation
///
/// # Returns
///
/// A vector of styled `Line` objects ready for display.
#[must_use]
pub fn render_timeline_with_throbber(timeline: &Timeline, throbber: char) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    for entry in timeline.iter() {
        match entry {
            ConversationEntry::UserMessage(text) => {
                render_user_message(&mut lines, text);
            }
            ConversationEntry::AssistantMessage(text) => {
                // Skip rendering empty assistant messages (e.g., tool-use only responses)
                if !text.is_empty() {
                    render_assistant_message(&mut lines, text);
                }
            }
            ConversationEntry::Streaming { text, .. } => {
                render_streaming_entry_with_throbber(&mut lines, text, throbber);
            }
            ConversationEntry::ToolExecution {
                name,
                input,
                output,
                is_error,
                ..
            } => {
                render_tool_execution(&mut lines, name, input, output.as_deref(), *is_error);
            }
        }
    }

    lines
}

/// Renders a user message to lines.
fn render_user_message(lines: &mut Vec<Line<'static>>, text: &str) {
    lines.push(Line::from(vec![Span::styled(
        "You: ".to_string(),
        PatinaTheme::user_label(),
    )]));

    for line in text.lines() {
        lines.push(Line::from(Span::styled(
            line.to_string(),
            PatinaTheme::user_message(),
        )));
    }

    lines.push(Line::from(""));
}

/// Renders an assistant message to lines.
fn render_assistant_message(lines: &mut Vec<Line<'static>>, text: &str) {
    lines.push(Line::from(vec![Span::styled(
        "Patina: ".to_string(),
        PatinaTheme::assistant_label(),
    )]));

    for line in text.lines() {
        lines.push(Line::from(Span::styled(
            line.to_string(),
            PatinaTheme::assistant_message(),
        )));
    }

    lines.push(Line::from(""));
}

/// Renders a streaming entry to lines with a specified throbber character.
fn render_streaming_entry_with_throbber(
    lines: &mut Vec<Line<'static>>,
    text: &str,
    throbber: char,
) {
    lines.push(Line::from(vec![
        Span::styled("Patina: ".to_string(), PatinaTheme::assistant_label()),
        Span::styled(format!("{} ", throbber), PatinaTheme::streaming()),
    ]));

    for line in text.lines() {
        lines.push(Line::from(Span::styled(
            line.to_string(),
            PatinaTheme::assistant_message(),
        )));
    }
}

/// Renders a tool execution entry to lines.
fn render_tool_execution(
    lines: &mut Vec<Line<'static>>,
    name: &str,
    input: &str,
    output: Option<&str>,
    is_error: bool,
) {
    // Tool block header
    let (icon, header_style) = if is_error {
        ("✗", PatinaTheme::error().add_modifier(Modifier::BOLD))
    } else if output.is_some() {
        ("✓", PatinaTheme::tool_header())
    } else {
        ("⚙", PatinaTheme::tool_header())
    };

    lines.push(Line::from(vec![
        Span::styled(format!("  {} ", icon), header_style),
        Span::styled(name.to_string(), header_style),
    ]));

    // Tool input line
    lines.push(Line::from(vec![
        Span::styled("    › ".to_string(), PatinaTheme::prompt()),
        Span::styled(
            input.to_string(),
            Style::default().fg(PatinaTheme::TOOL_CONTENT),
        ),
    ]));

    // Tool result (if complete) or pending status
    if let Some(result) = output {
        let result_style = if is_error {
            PatinaTheme::error()
        } else {
            Style::default().fg(PatinaTheme::TOOL_CONTENT)
        };

        // Show first few lines of result (truncate long output)
        let result_lines: Vec<&str> = result.lines().take(5).collect();
        for line in &result_lines {
            lines.push(Line::from(vec![
                Span::raw("    ".to_string()),
                Span::styled((*line).to_string(), result_style),
            ]));
        }

        // Indicate if output was truncated
        let total_lines = result.lines().count();
        if total_lines > 5 {
            lines.push(Line::from(vec![
                Span::raw("    ".to_string()),
                Span::styled(
                    format!("... ({} more lines)", total_lines - 5),
                    Style::default().fg(PatinaTheme::MUTED),
                ),
            ]));
        }
    } else {
        // Pending state
        lines.push(Line::from(vec![
            Span::raw("    ".to_string()),
            Span::styled("Running...".to_string(), PatinaTheme::streaming()),
        ]));
    }

    lines.push(Line::from("")); // Spacer between tool blocks
}

pub fn render(frame: &mut Frame, state: &mut AppState) {
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

    // Render permission modal overlay if there's a pending permission request
    if let Some(request) = state.pending_permission() {
        render_permission_modal(frame, request);
    }
}

/// Renders the permission prompt modal as an overlay.
///
/// This function renders a modal dialog asking the user to approve or deny
/// tool execution. The modal appears centered over the main UI.
///
/// # Arguments
///
/// * `frame` - The ratatui frame to render into
/// * `request` - The permission request to display
pub fn render_permission_modal(frame: &mut Frame, request: &PermissionRequest) {
    let area = frame.area();
    let modal_area = PermissionPromptWidget::modal_area(area);

    // Create the prompt state from the request
    let prompt_state = PermissionPromptState::new(request.clone());

    // Check if this is a dangerous command and add warning styling
    let is_dangerous = is_dangerous_command(request);

    // Render the widget
    if is_dangerous {
        render_permission_modal_dangerous(frame, modal_area, &prompt_state);
    } else {
        let widget = PermissionPromptWidget::new(&prompt_state);
        frame.render_widget(widget, modal_area);
    }
}

/// Checks if a permission request is for a dangerous command.
///
/// A command is considered dangerous if it matches any of the patterns
/// in the security policy's dangerous pattern list.
#[must_use]
pub fn is_dangerous_command(request: &PermissionRequest) -> bool {
    use once_cell::sync::Lazy;
    use regex::Regex;

    // Pattern list for dangerous commands (subset of tools/mod.rs patterns)
    static DANGEROUS_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
        vec![
            // Destructive file operations
            Regex::new(r"rm\s+-rf").expect("invalid regex"),
            Regex::new(r"rm\s+--no-preserve-root").expect("invalid regex"),
            // Privilege escalation
            Regex::new(r"\bsudo\s+").expect("invalid regex"),
            Regex::new(r"\bsu\s+-").expect("invalid regex"),
            // Disk destruction
            Regex::new(r"\bmkfs\.").expect("invalid regex"),
            Regex::new(r"\bdd\s+if=.+of=/dev/").expect("invalid regex"),
            // Fork bombs
            Regex::new(r":\(\)\s*\{").expect("invalid regex"),
            // Remote code execution
            Regex::new(r"curl\s+.+\|\s*(ba)?sh").expect("invalid regex"),
            Regex::new(r"wget\s+.+\|\s*(ba)?sh").expect("invalid regex"),
            // System disruption
            Regex::new(r"\bshutdown\b").expect("invalid regex"),
            Regex::new(r"\breboot\b").expect("invalid regex"),
            // Dangerous eval
            Regex::new(r"\beval\s+\$").expect("invalid regex"),
        ]
    });

    // Only check Bash tool calls
    if request.tool_name != "Bash" && request.tool_name != "bash" {
        return false;
    }

    // Check the tool input against dangerous patterns
    if let Some(ref input) = request.tool_input {
        for pattern in DANGEROUS_PATTERNS.iter() {
            if pattern.is_match(input) {
                return true;
            }
        }
    }

    false
}

/// Renders a permission modal with dangerous command warning.
///
/// This variant of the modal includes a red warning border and
/// additional warning text to alert the user.
fn render_permission_modal_dangerous(frame: &mut Frame, area: Rect, state: &PermissionPromptState) {
    use ratatui::widgets::Clear;

    // Clear the background
    frame.render_widget(Clear, area);

    // Draw a warning-styled block (red border instead of yellow)
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" ⚠ DANGEROUS - Permission Required ⚠ ")
        .title_alignment(ratatui::layout::Alignment::Center)
        .border_style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
        .style(Style::default().bg(Color::DarkGray));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Layout the content vertically
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Warning text
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

    // Warning text
    let warning_line = Line::from(vec![Span::styled(
        "This command may be destructive!",
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    )]);
    frame.render_widget(Paragraph::new(warning_line), chunks[0]);

    // Tool name
    let tool_line = Line::from(vec![
        Span::styled("Tool: ", Style::default().fg(Color::White)),
        Span::styled(
            state.request().tool_name.clone(),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
    ]);
    frame.render_widget(Paragraph::new(tool_line), chunks[1]);

    // Tool input (if present)
    if let Some(ref input) = state.request().tool_input {
        let input_text = if input.len() > (inner.width as usize).saturating_sub(8) {
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
        frame.render_widget(input_para, chunks[3]);
    }

    // Description
    let desc_para = Paragraph::new(state.request().description.clone())
        .style(Style::default().fg(Color::White))
        .wrap(Wrap { trim: true });
    frame.render_widget(desc_para, chunks[5]);

    // Options line (using simpler rendering)
    let options_line = Line::from(vec![
        Span::styled("[y]", Style::default().fg(Color::Cyan)),
        Span::raw(" Allow Once  "),
        Span::styled("[a]", Style::default().fg(Color::Cyan)),
        Span::raw(" Allow Always  "),
        Span::styled("[n]", Style::default().fg(Color::Red)),
        Span::raw(" Deny"),
    ]);
    frame.render_widget(Paragraph::new(options_line), chunks[7]);

    // Keybinding hints
    let hints = Line::from(vec![
        Span::styled("Enter", Style::default().fg(Color::Cyan)),
        Span::raw(":confirm "),
        Span::styled("Esc", Style::default().fg(Color::Cyan)),
        Span::raw(":deny"),
    ]);
    frame.render_widget(Paragraph::new(hints), chunks[8]);
}

fn render_messages(frame: &mut Frame, area: Rect, state: &mut AppState) {
    // Render using unified timeline
    let throbber = state.throbber_char();
    let lines = render_timeline_with_throbber(state.timeline(), throbber);

    // Update cached lines for copy/paste operations
    state.update_rendered_lines_cache(&lines);

    // Update scroll state with content dimensions
    // Subtract 2 for borders (top and bottom)
    let viewport_height = area.height.saturating_sub(2) as usize;
    // Subtract 2 for borders (left and right)
    let content_width = area.width.saturating_sub(2) as usize;

    // Calculate actual wrapped content height
    // Each Line may wrap to multiple displayed lines based on content width
    let wrapped_height = calculate_wrapped_height(&lines, content_width);

    state.set_viewport_height(viewport_height);
    state.update_content_height(wrapped_height);

    // Convert scroll offset: our model uses "offset from bottom" (0 = at bottom),
    // but ratatui Paragraph uses "offset from top" (0 = at top).
    //
    // The scroll offset may exceed max_scroll if the user scrolled up and then
    // content height decreased (e.g., wrapping calculation changed). We clamp
    // to ensure valid scroll position.
    let max_scroll = wrapped_height.saturating_sub(viewport_height);
    let clamped_offset = state.scroll_offset().min(max_scroll);
    let scroll_from_top = max_scroll.saturating_sub(clamped_offset);

    tracing::debug!(
        logical_lines = lines.len(),
        wrapped_height,
        viewport_height,
        content_width,
        max_scroll,
        raw_offset = state.scroll_offset(),
        clamped_offset,
        scroll_from_top,
        mode = ?state.scroll_state().mode(),
        "scroll calculation"
    );

    let messages = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Messages ")
                .border_style(PatinaTheme::border()),
        )
        .wrap(Wrap { trim: false })
        .scroll((scroll_from_top as u16, 0));

    frame.render_widget(messages, area);
}

fn render_status_bar(frame: &mut Frame, area: Rect, state: &AppState) {
    let mut spans = Vec::new();

    // Branch name (using verdigris for git branch indicator)
    if let Some(branch) = state.worktree_branch() {
        spans.push(Span::styled(
            " ",
            Style::default().fg(PatinaTheme::VERDIGRIS),
        ));
        spans.push(Span::styled(
            branch,
            Style::default()
                .fg(PatinaTheme::VERDIGRIS_BRIGHT)
                .add_modifier(Modifier::BOLD),
        ));
    }

    // Modified count (dirty indicator - using warning color)
    let modified = state.worktree_modified();
    if modified > 0 {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!("●{}", modified),
            Style::default().fg(PatinaTheme::WARNING),
        ));
    }

    // Ahead indicator (using success/verdigris)
    let ahead = state.worktree_ahead();
    if ahead > 0 {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!("↑{}", ahead),
            Style::default().fg(PatinaTheme::SUCCESS),
        ));
    }

    // Behind indicator (using error color)
    let behind = state.worktree_behind();
    if behind > 0 {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!("↓{}", behind),
            Style::default().fg(PatinaTheme::ERROR),
        ));
    }

    // Scroll indicator (right side)
    let scroll = state.scroll_state();
    let mode_char = match scroll.mode() {
        crate::tui::scroll::AutoScrollMode::Follow => 'F',
        crate::tui::scroll::AutoScrollMode::Manual => 'M',
        crate::tui::scroll::AutoScrollMode::Paused => 'P',
    };
    let scroll_info = format!(
        " [{}:{}↑ {}/{}]",
        mode_char,
        scroll.offset(),
        scroll.viewport_height(),
        scroll.content_height()
    );
    spans.push(Span::styled(
        scroll_info,
        Style::default().fg(PatinaTheme::MUTED),
    ));

    let line = Line::from(spans);
    let status_bar = Paragraph::new(line).style(PatinaTheme::status_bar());

    frame.render_widget(status_bar, area);
}

fn render_input(frame: &mut Frame, area: Rect, state: &AppState) {
    let input = Paragraph::new(state.input.as_str())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Input (Enter to send, Ctrl+C to quit) ")
                .border_style(PatinaTheme::border_focused()),
        )
        .style(Style::default().fg(PatinaTheme::USER_TEXT));

    frame.render_widget(input, area);

    frame.set_cursor_position((area.x + state.input.len() as u16 + 1, area.y + 1));
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    /// Creates a test terminal with the given dimensions.
    fn test_terminal(width: u16, height: u16) -> Terminal<TestBackend> {
        let backend = TestBackend::new(width, height);
        Terminal::new(backend).expect("Failed to create test terminal")
    }

    // =========================================================================
    // is_dangerous_command tests
    // =========================================================================

    #[test]
    fn test_is_dangerous_rm_rf() {
        let request = PermissionRequest::new("Bash", Some("rm -rf /tmp/test"), "Delete files");
        assert!(is_dangerous_command(&request));
    }

    #[test]
    fn test_is_dangerous_sudo() {
        let request =
            PermissionRequest::new("Bash", Some("sudo apt install vim"), "Install package");
        assert!(is_dangerous_command(&request));
    }

    #[test]
    fn test_is_dangerous_curl_pipe_sh() {
        let request = PermissionRequest::new(
            "Bash",
            Some("curl https://example.com/script.sh | sh"),
            "Download and run script",
        );
        assert!(is_dangerous_command(&request));
    }

    #[test]
    fn test_is_dangerous_reboot() {
        let request = PermissionRequest::new("Bash", Some("reboot"), "Restart system");
        assert!(is_dangerous_command(&request));
    }

    #[test]
    fn test_is_not_dangerous_ls() {
        let request = PermissionRequest::new("Bash", Some("ls -la"), "List files");
        assert!(!is_dangerous_command(&request));
    }

    #[test]
    fn test_is_not_dangerous_git() {
        let request = PermissionRequest::new("Bash", Some("git status"), "Check git status");
        assert!(!is_dangerous_command(&request));
    }

    #[test]
    fn test_is_not_dangerous_read_tool() {
        // Read tool is never dangerous, even if the path looks suspicious
        let request = PermissionRequest::new("Read", Some("/etc/passwd"), "Read file");
        assert!(!is_dangerous_command(&request));
    }

    #[test]
    fn test_is_not_dangerous_no_input() {
        let request = PermissionRequest::new("Bash", None, "No input");
        assert!(!is_dangerous_command(&request));
    }

    // =========================================================================
    // Permission modal rendering tests
    // =========================================================================

    #[test]
    fn test_permission_modal_renders() {
        let mut terminal = test_terminal(80, 24);

        let request = PermissionRequest::new("Bash", Some("echo hello"), "Print hello");

        terminal
            .draw(|frame| {
                render_permission_modal(frame, &request);
            })
            .expect("Failed to draw");

        // Get the buffer and check for expected content
        let buffer = terminal.backend().buffer();

        // The modal should contain "Permission Required" in the title
        let content: String = buffer
            .content()
            .iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();

        assert!(
            content.contains("Permission Required"),
            "Modal should contain 'Permission Required' title"
        );
        assert!(
            content.contains("Bash"),
            "Modal should show tool name 'Bash'"
        );
    }

    #[test]
    fn test_permission_modal_shows_dangerous_warning() {
        let mut terminal = test_terminal(80, 24);

        let request = PermissionRequest::new("Bash", Some("sudo rm -rf /"), "Dangerous command");

        terminal
            .draw(|frame| {
                render_permission_modal(frame, &request);
            })
            .expect("Failed to draw");

        // Get the buffer and check for expected content
        let buffer = terminal.backend().buffer();

        let content: String = buffer
            .content()
            .iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();

        // Should show DANGEROUS warning in title
        assert!(
            content.contains("DANGEROUS"),
            "Dangerous modal should contain 'DANGEROUS' warning"
        );
        assert!(
            content.contains("destructive"),
            "Dangerous modal should warn about destructive command"
        );
    }

    #[test]
    fn test_permission_modal_safe_command_no_dangerous_warning() {
        let mut terminal = test_terminal(80, 24);

        let request = PermissionRequest::new("Bash", Some("ls -la"), "List files");

        terminal
            .draw(|frame| {
                render_permission_modal(frame, &request);
            })
            .expect("Failed to draw");

        // Get the buffer and check for expected content
        let buffer = terminal.backend().buffer();

        let content: String = buffer
            .content()
            .iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();

        // Should NOT show DANGEROUS warning
        assert!(
            !content.contains("DANGEROUS"),
            "Safe command modal should NOT contain 'DANGEROUS' warning"
        );
    }

    #[test]
    fn test_permission_modal_displays_tool_input() {
        let mut terminal = test_terminal(80, 24);

        let request = PermissionRequest::new("Bash", Some("git status"), "Check git status");

        terminal
            .draw(|frame| {
                render_permission_modal(frame, &request);
            })
            .expect("Failed to draw");

        let buffer = terminal.backend().buffer();
        let content: String = buffer
            .content()
            .iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();

        assert!(
            content.contains("git status"),
            "Modal should display the tool input"
        );
    }

    #[test]
    fn test_permission_modal_displays_keybinding_hints() {
        let mut terminal = test_terminal(80, 24);

        let request = PermissionRequest::new("Read", Some("/tmp/test.txt"), "Read file");

        terminal
            .draw(|frame| {
                render_permission_modal(frame, &request);
            })
            .expect("Failed to draw");

        let buffer = terminal.backend().buffer();
        let content: String = buffer
            .content()
            .iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();

        // Should show keybinding hints
        assert!(
            content.contains("[y]") || content.contains("Allow"),
            "Modal should display keybinding hints"
        );
    }

    // =========================================================================
    // calculate_wrapped_height tests
    // =========================================================================

    #[test]
    fn test_wrapped_height_empty_lines() {
        let lines: Vec<Line> = vec![];
        assert_eq!(calculate_wrapped_height(&lines, 80), 0);
    }

    #[test]
    fn test_wrapped_height_single_short_line() {
        let lines = vec![Line::from("Hello")];
        // "Hello" is 5 chars, fits in 80 width = 1 line
        assert_eq!(calculate_wrapped_height(&lines, 80), 1);
    }

    #[test]
    fn test_wrapped_height_single_long_line() {
        // Create a line that's 100 chars wide
        let long_text = "x".repeat(100);
        let lines = vec![Line::from(long_text)];
        // 100 chars in 80 width = 2 lines (100/80 = 1.25, ceiling = 2)
        assert_eq!(calculate_wrapped_height(&lines, 80), 2);
    }

    #[test]
    fn test_wrapped_height_exact_fit() {
        // Exactly 80 chars in 80 width = 1 line
        let exact_text = "x".repeat(80);
        let lines = vec![Line::from(exact_text)];
        assert_eq!(calculate_wrapped_height(&lines, 80), 1);
    }

    #[test]
    fn test_wrapped_height_just_over() {
        // 81 chars in 80 width = 2 lines
        let over_text = "x".repeat(81);
        let lines = vec![Line::from(over_text)];
        assert_eq!(calculate_wrapped_height(&lines, 80), 2);
    }

    #[test]
    fn test_wrapped_height_empty_line() {
        // Empty lines should still count as 1 displayed line
        let lines = vec![Line::from("")];
        assert_eq!(calculate_wrapped_height(&lines, 80), 1);
    }

    #[test]
    fn test_wrapped_height_multiple_lines() {
        let lines = vec![
            Line::from("Short line"),    // 10 chars = 1 line
            Line::from("x".repeat(100)), // 100 chars = 2 lines
            Line::from(""),              // empty = 1 line
            Line::from("x".repeat(200)), // 200 chars = 3 lines
        ];
        // Total: 1 + 2 + 1 + 3 = 7 lines
        assert_eq!(calculate_wrapped_height(&lines, 80), 7);
    }

    #[test]
    fn test_wrapped_height_narrow_width() {
        // 100 chars in width 10 = 10 lines
        let lines = vec![Line::from("x".repeat(100))];
        assert_eq!(calculate_wrapped_height(&lines, 10), 10);
    }

    #[test]
    fn test_wrapped_height_zero_width_returns_line_count() {
        // Zero width is a degenerate case - return line count
        let lines = vec![Line::from("Hello"), Line::from("World")];
        assert_eq!(calculate_wrapped_height(&lines, 0), 2);
    }

    #[test]
    fn test_wrapped_height_styled_spans() {
        // Lines with styled spans should measure correctly
        let lines = vec![Line::from(vec![
            Span::styled("Hello ", Style::default().fg(Color::Red)),
            Span::styled("World", Style::default().fg(Color::Blue)),
        ])];
        // "Hello World" = 11 chars = 1 line in width 80
        assert_eq!(calculate_wrapped_height(&lines, 80), 1);
    }

    #[test]
    fn test_wrapped_height_realistic_tool_output() {
        // Simulate tool output with file paths that might be long
        let lines = vec![
            Line::from("✓ Bash"),
            Line::from("    › ls -la"),
            Line::from("    drwxr-xr-x  15 user  staff    480 Jan 31 14:00 ."),
            Line::from("    drwxr-xr-x   5 user  staff    160 Jan 31 13:00 .."),
            Line::from("/Users/very/long/path/to/some/deeply/nested/directory/structure/that/might/wrap/when/displayed/in/terminal".to_string()),
        ];
        // First 4 lines fit in 80
        // Last line is ~100+ chars = 2 lines
        let height = calculate_wrapped_height(&lines, 80);
        assert!(height >= 5, "Should have at least 5 lines, got {}", height);
    }
}
