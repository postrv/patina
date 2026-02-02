//! Subagent status panel widget for displaying subagent activity.
//!
//! This widget renders a panel showing the status of running subagents:
//! - List of active/completed subagents
//! - Status indicator (spinner, checkmark, error)
//! - Progress (turns used / max turns)
//!
//! # Example
//!
//! ```rust,ignore
//! use patina::tui::widgets::subagent::{SubagentPanelState, SubagentPanelWidget, SubagentDisplayInfo, SubagentDisplayStatus};
//! use uuid::Uuid;
//!
//! let mut state = SubagentPanelState::new();
//! state.add_subagent(SubagentDisplayInfo {
//!     id: Uuid::new_v4(),
//!     name: "explorer".to_string(),
//!     status: SubagentDisplayStatus::Running,
//!     current_turn: 2,
//!     max_turns: 10,
//!     last_activity: Some("Reading files...".to_string()),
//! });
//!
//! let widget = SubagentPanelWidget::new(&state);
//! // Render widget in a frame
//! ```

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};
use uuid::Uuid;

use crate::tui::theme::PatinaTheme;

/// Status of a subagent for display purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubagentDisplayStatus {
    /// Agent is queued but not yet running.
    Pending,
    /// Agent is currently executing.
    Running,
    /// Agent completed successfully.
    Completed,
    /// Agent failed during execution.
    Failed,
}

impl SubagentDisplayStatus {
    /// Returns the display icon for this status.
    #[must_use]
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Pending => "○",
            Self::Running => "◐",
            Self::Completed => "✓",
            Self::Failed => "✗",
        }
    }

    /// Returns the status label.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Running => "Running",
            Self::Completed => "Done",
            Self::Failed => "Failed",
        }
    }
}

/// Information about a subagent for display.
#[derive(Debug, Clone)]
pub struct SubagentDisplayInfo {
    /// Unique identifier for the subagent.
    pub id: Uuid,
    /// Display name of the subagent.
    pub name: String,
    /// Current status.
    pub status: SubagentDisplayStatus,
    /// Number of turns completed.
    pub current_turn: usize,
    /// Maximum allowed turns.
    pub max_turns: usize,
    /// Last activity description (optional).
    pub last_activity: Option<String>,
}

impl SubagentDisplayInfo {
    /// Creates a new display info entry.
    #[must_use]
    pub fn new(id: Uuid, name: impl Into<String>, max_turns: usize) -> Self {
        Self {
            id,
            name: name.into(),
            status: SubagentDisplayStatus::Pending,
            current_turn: 0,
            max_turns,
            last_activity: None,
        }
    }

    /// Returns the progress ratio (0.0 to 1.0).
    #[must_use]
    pub fn progress(&self) -> f64 {
        if self.max_turns == 0 {
            0.0
        } else {
            (self.current_turn as f64 / self.max_turns as f64).min(1.0)
        }
    }

    /// Returns the progress as a percentage (0 to 100).
    #[must_use]
    pub fn progress_percent(&self) -> u16 {
        (self.progress() * 100.0) as u16
    }
}

/// State for the subagent panel widget.
#[derive(Debug, Clone, Default)]
pub struct SubagentPanelState {
    /// List of subagents to display.
    subagents: Vec<SubagentDisplayInfo>,
    /// Whether the panel is collapsed.
    collapsed: bool,
}

impl SubagentPanelState {
    /// Creates a new empty panel state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            subagents: Vec::new(),
            collapsed: false,
        }
    }

    /// Adds a subagent to the display.
    pub fn add_subagent(&mut self, info: SubagentDisplayInfo) {
        self.subagents.push(info);
    }

    /// Updates a subagent's status by ID.
    ///
    /// Returns true if the subagent was found and updated.
    pub fn update_status(&mut self, id: Uuid, status: SubagentDisplayStatus) -> bool {
        if let Some(agent) = self.subagents.iter_mut().find(|a| a.id == id) {
            agent.status = status;
            true
        } else {
            false
        }
    }

    /// Updates a subagent's turn count by ID.
    ///
    /// Returns true if the subagent was found and updated.
    pub fn update_turn(&mut self, id: Uuid, turn: usize) -> bool {
        if let Some(agent) = self.subagents.iter_mut().find(|a| a.id == id) {
            agent.current_turn = turn;
            true
        } else {
            false
        }
    }

    /// Updates a subagent's last activity description.
    ///
    /// Returns true if the subagent was found and updated.
    pub fn update_activity(&mut self, id: Uuid, activity: impl Into<String>) -> bool {
        if let Some(agent) = self.subagents.iter_mut().find(|a| a.id == id) {
            agent.last_activity = Some(activity.into());
            true
        } else {
            false
        }
    }

    /// Removes a subagent from the display.
    ///
    /// Returns true if the subagent was found and removed.
    pub fn remove_subagent(&mut self, id: Uuid) -> bool {
        let len_before = self.subagents.len();
        self.subagents.retain(|a| a.id != id);
        self.subagents.len() < len_before
    }

    /// Clears all completed subagents from the display.
    pub fn clear_completed(&mut self) {
        self.subagents
            .retain(|a| a.status != SubagentDisplayStatus::Completed);
    }

    /// Returns the list of subagents.
    #[must_use]
    pub fn subagents(&self) -> &[SubagentDisplayInfo] {
        &self.subagents
    }

    /// Returns the number of subagents.
    #[must_use]
    pub fn len(&self) -> usize {
        self.subagents.len()
    }

    /// Returns whether there are no subagents.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.subagents.is_empty()
    }

    /// Returns the count of running subagents.
    #[must_use]
    pub fn running_count(&self) -> usize {
        self.subagents
            .iter()
            .filter(|a| a.status == SubagentDisplayStatus::Running)
            .count()
    }

    /// Returns whether the panel is collapsed.
    #[must_use]
    pub fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    /// Sets whether the panel is collapsed.
    pub fn set_collapsed(&mut self, collapsed: bool) {
        self.collapsed = collapsed;
    }

    /// Toggles the collapsed state.
    pub fn toggle_collapsed(&mut self) {
        self.collapsed = !self.collapsed;
    }

    /// Gets a subagent by ID.
    #[must_use]
    pub fn get(&self, id: Uuid) -> Option<&SubagentDisplayInfo> {
        self.subagents.iter().find(|a| a.id == id)
    }
}

/// Widget for rendering the subagent status panel.
pub struct SubagentPanelWidget<'a> {
    /// The state to render.
    state: &'a SubagentPanelState,
}

impl<'a> SubagentPanelWidget<'a> {
    /// Creates a new subagent panel widget.
    #[must_use]
    pub fn new(state: &'a SubagentPanelState) -> Self {
        Self { state }
    }

    /// Renders the header with title and count.
    fn render_header(&self) -> Line<'static> {
        let running = self.state.running_count();
        let total = self.state.len();

        let title_style = PatinaTheme::tool_header();
        let count_style = Style::default().fg(PatinaTheme::VERDIGRIS_MUTED);

        let collapse_indicator = if self.state.is_collapsed() {
            "▶"
        } else {
            "▼"
        };

        Line::from(vec![
            Span::styled(
                format!(" {} ", collapse_indicator),
                Style::default().fg(PatinaTheme::BRONZE_MUTED),
            ),
            Span::styled("Subagents ", title_style),
            Span::styled(format!("({}/{})", running, total), count_style),
        ])
    }

    /// Renders a single subagent entry.
    fn render_subagent(&self, info: &SubagentDisplayInfo) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        // Status icon and name
        let icon_style = match info.status {
            SubagentDisplayStatus::Pending => Style::default().fg(PatinaTheme::MUTED),
            SubagentDisplayStatus::Running => Style::default()
                .fg(PatinaTheme::VERDIGRIS_BRIGHT)
                .add_modifier(Modifier::SLOW_BLINK),
            SubagentDisplayStatus::Completed => Style::default().fg(PatinaTheme::SUCCESS),
            SubagentDisplayStatus::Failed => PatinaTheme::error(),
        };

        let name_style = Style::default().fg(PatinaTheme::VERDIGRIS);

        lines.push(Line::from(vec![
            Span::styled(format!("  {} ", info.status.icon()), icon_style),
            Span::styled(info.name.clone(), name_style),
            Span::styled(
                format!(" [{}/{}]", info.current_turn, info.max_turns),
                Style::default().fg(PatinaTheme::MUTED),
            ),
        ]));

        // Last activity (if any and not collapsed)
        if !self.state.is_collapsed() {
            if let Some(activity) = &info.last_activity {
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(
                        truncate_activity(activity, 40),
                        Style::default().fg(PatinaTheme::VERDIGRIS_MUTED),
                    ),
                ]));
            }
        }

        lines
    }

    /// Calculates the required height for rendering.
    #[must_use]
    pub fn required_height(&self) -> u16 {
        if self.state.is_empty() {
            return 0;
        }

        if self.state.is_collapsed() {
            // Just the header
            3
        } else {
            // Header + each subagent (1-2 lines each) + borders
            let agent_lines: usize = self
                .state
                .subagents()
                .iter()
                .map(|a| if a.last_activity.is_some() { 2 } else { 1 })
                .sum();
            (agent_lines + 2) as u16 // +2 for borders
        }
    }
}

/// Truncates an activity string to the specified length.
fn truncate_activity(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

impl Widget for SubagentPanelWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Don't render if empty
        if self.state.is_empty() {
            return;
        }

        // Create the outer block
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(PatinaTheme::BORDER))
            .style(Style::default().bg(PatinaTheme::BG_SECONDARY));

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height == 0 {
            return;
        }

        // Render header
        let header = self.render_header();
        let header_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: 1,
        };
        Paragraph::new(header).render(header_area, buf);

        // If collapsed, we're done
        if self.state.is_collapsed() {
            return;
        }

        // Render each subagent
        let mut y_offset = 1;
        for agent in self.state.subagents() {
            let lines = self.render_subagent(agent);
            for line in lines {
                if y_offset >= inner.height {
                    break;
                }
                let line_area = Rect {
                    x: inner.x,
                    y: inner.y + y_offset,
                    width: inner.width,
                    height: 1,
                };
                Paragraph::new(line).render(line_area, buf);
                y_offset += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============================================================================
    // SubagentDisplayStatus tests
    // ============================================================================

    #[test]
    fn test_status_icons_are_distinct() {
        let pending = SubagentDisplayStatus::Pending.icon();
        let running = SubagentDisplayStatus::Running.icon();
        let completed = SubagentDisplayStatus::Completed.icon();
        let failed = SubagentDisplayStatus::Failed.icon();

        // All icons should be different
        assert_ne!(pending, running);
        assert_ne!(pending, completed);
        assert_ne!(pending, failed);
        assert_ne!(running, completed);
        assert_ne!(running, failed);
        assert_ne!(completed, failed);
    }

    #[test]
    fn test_status_labels() {
        assert_eq!(SubagentDisplayStatus::Pending.label(), "Pending");
        assert_eq!(SubagentDisplayStatus::Running.label(), "Running");
        assert_eq!(SubagentDisplayStatus::Completed.label(), "Done");
        assert_eq!(SubagentDisplayStatus::Failed.label(), "Failed");
    }

    // ============================================================================
    // SubagentDisplayInfo tests
    // ============================================================================

    #[test]
    fn test_display_info_new() {
        let id = Uuid::new_v4();
        let info = SubagentDisplayInfo::new(id, "explorer", 10);

        assert_eq!(info.id, id);
        assert_eq!(info.name, "explorer");
        assert_eq!(info.status, SubagentDisplayStatus::Pending);
        assert_eq!(info.current_turn, 0);
        assert_eq!(info.max_turns, 10);
        assert!(info.last_activity.is_none());
    }

    #[test]
    fn test_display_info_progress() {
        let mut info = SubagentDisplayInfo::new(Uuid::new_v4(), "test", 10);

        // Initial progress is 0
        assert_eq!(info.progress(), 0.0);
        assert_eq!(info.progress_percent(), 0);

        // 50% progress
        info.current_turn = 5;
        assert!((info.progress() - 0.5).abs() < 0.001);
        assert_eq!(info.progress_percent(), 50);

        // 100% progress
        info.current_turn = 10;
        assert!((info.progress() - 1.0).abs() < 0.001);
        assert_eq!(info.progress_percent(), 100);
    }

    #[test]
    fn test_display_info_progress_capped_at_100() {
        let mut info = SubagentDisplayInfo::new(Uuid::new_v4(), "test", 10);
        info.current_turn = 15; // Over max

        assert!((info.progress() - 1.0).abs() < 0.001);
        assert_eq!(info.progress_percent(), 100);
    }

    #[test]
    fn test_display_info_progress_zero_max_turns() {
        let info = SubagentDisplayInfo::new(Uuid::new_v4(), "test", 0);

        // Should not panic, returns 0
        assert_eq!(info.progress(), 0.0);
        assert_eq!(info.progress_percent(), 0);
    }

    // ============================================================================
    // SubagentPanelState tests
    // ============================================================================

    #[test]
    fn test_panel_state_new_is_empty() {
        let state = SubagentPanelState::new();

        assert!(state.is_empty());
        assert_eq!(state.len(), 0);
        assert_eq!(state.running_count(), 0);
        assert!(!state.is_collapsed());
    }

    #[test]
    fn test_panel_state_add_subagent() {
        let mut state = SubagentPanelState::new();
        let id = Uuid::new_v4();

        state.add_subagent(SubagentDisplayInfo::new(id, "explorer", 10));

        assert!(!state.is_empty());
        assert_eq!(state.len(), 1);
        assert!(state.get(id).is_some());
    }

    #[test]
    fn test_panel_state_update_status() {
        let mut state = SubagentPanelState::new();
        let id = Uuid::new_v4();

        state.add_subagent(SubagentDisplayInfo::new(id, "worker", 5));

        // Update to running
        assert!(state.update_status(id, SubagentDisplayStatus::Running));
        assert_eq!(state.get(id).unwrap().status, SubagentDisplayStatus::Running);

        // Verify running count
        assert_eq!(state.running_count(), 1);

        // Update to completed
        assert!(state.update_status(id, SubagentDisplayStatus::Completed));
        assert_eq!(
            state.get(id).unwrap().status,
            SubagentDisplayStatus::Completed
        );
        assert_eq!(state.running_count(), 0);
    }

    #[test]
    fn test_panel_state_update_status_unknown_id() {
        let mut state = SubagentPanelState::new();
        let id = Uuid::new_v4();

        // Update non-existent subagent
        assert!(!state.update_status(id, SubagentDisplayStatus::Running));
    }

    #[test]
    fn test_panel_state_update_turn() {
        let mut state = SubagentPanelState::new();
        let id = Uuid::new_v4();

        state.add_subagent(SubagentDisplayInfo::new(id, "runner", 10));

        assert!(state.update_turn(id, 5));
        assert_eq!(state.get(id).unwrap().current_turn, 5);

        // Non-existent ID
        assert!(!state.update_turn(Uuid::new_v4(), 3));
    }

    #[test]
    fn test_panel_state_update_activity() {
        let mut state = SubagentPanelState::new();
        let id = Uuid::new_v4();

        state.add_subagent(SubagentDisplayInfo::new(id, "reader", 5));

        assert!(state.update_activity(id, "Reading src/main.rs..."));
        assert_eq!(
            state.get(id).unwrap().last_activity,
            Some("Reading src/main.rs...".to_string())
        );

        // Non-existent ID
        assert!(!state.update_activity(Uuid::new_v4(), "Nothing"));
    }

    #[test]
    fn test_panel_state_remove_subagent() {
        let mut state = SubagentPanelState::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();

        state.add_subagent(SubagentDisplayInfo::new(id1, "agent1", 5));
        state.add_subagent(SubagentDisplayInfo::new(id2, "agent2", 5));

        assert_eq!(state.len(), 2);

        assert!(state.remove_subagent(id1));
        assert_eq!(state.len(), 1);
        assert!(state.get(id1).is_none());
        assert!(state.get(id2).is_some());

        // Remove non-existent
        assert!(!state.remove_subagent(id1));
    }

    #[test]
    fn test_panel_state_clear_completed() {
        let mut state = SubagentPanelState::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();

        state.add_subagent(SubagentDisplayInfo::new(id1, "running", 5));
        state.add_subagent(SubagentDisplayInfo::new(id2, "completed", 5));
        state.add_subagent(SubagentDisplayInfo::new(id3, "failed", 5));

        state.update_status(id1, SubagentDisplayStatus::Running);
        state.update_status(id2, SubagentDisplayStatus::Completed);
        state.update_status(id3, SubagentDisplayStatus::Failed);

        state.clear_completed();

        assert_eq!(state.len(), 2);
        assert!(state.get(id1).is_some()); // Running - kept
        assert!(state.get(id2).is_none()); // Completed - removed
        assert!(state.get(id3).is_some()); // Failed - kept
    }

    #[test]
    fn test_panel_state_collapse() {
        let mut state = SubagentPanelState::new();

        assert!(!state.is_collapsed());

        state.set_collapsed(true);
        assert!(state.is_collapsed());

        state.toggle_collapsed();
        assert!(!state.is_collapsed());

        state.toggle_collapsed();
        assert!(state.is_collapsed());
    }

    #[test]
    fn test_panel_state_running_count() {
        let mut state = SubagentPanelState::new();

        // Add three subagents in different states
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();

        state.add_subagent(SubagentDisplayInfo::new(id1, "a", 5));
        state.add_subagent(SubagentDisplayInfo::new(id2, "b", 5));
        state.add_subagent(SubagentDisplayInfo::new(id3, "c", 5));

        state.update_status(id1, SubagentDisplayStatus::Running);
        state.update_status(id2, SubagentDisplayStatus::Running);
        state.update_status(id3, SubagentDisplayStatus::Completed);

        assert_eq!(state.running_count(), 2);
        assert_eq!(state.len(), 3);
    }

    // ============================================================================
    // SubagentPanelWidget tests
    // ============================================================================

    #[test]
    fn test_widget_creation() {
        let state = SubagentPanelState::new();
        let _widget = SubagentPanelWidget::new(&state);
        // Just verify it compiles and doesn't panic
    }

    #[test]
    fn test_widget_required_height_empty() {
        let state = SubagentPanelState::new();
        let widget = SubagentPanelWidget::new(&state);

        assert_eq!(widget.required_height(), 0);
    }

    #[test]
    fn test_widget_required_height_collapsed() {
        let mut state = SubagentPanelState::new();
        state.add_subagent(SubagentDisplayInfo::new(Uuid::new_v4(), "agent", 10));
        state.set_collapsed(true);

        let widget = SubagentPanelWidget::new(&state);

        assert_eq!(widget.required_height(), 3); // Header + borders
    }

    #[test]
    fn test_widget_required_height_expanded() {
        let mut state = SubagentPanelState::new();
        state.add_subagent(SubagentDisplayInfo::new(Uuid::new_v4(), "agent1", 10));
        state.add_subagent(SubagentDisplayInfo::new(Uuid::new_v4(), "agent2", 10));

        let widget = SubagentPanelWidget::new(&state);

        // 2 agents * 1 line each + 2 borders = 4
        assert_eq!(widget.required_height(), 4);
    }

    #[test]
    fn test_widget_required_height_with_activity() {
        let mut state = SubagentPanelState::new();
        let id = Uuid::new_v4();
        state.add_subagent(SubagentDisplayInfo::new(id, "agent", 10));
        state.update_activity(id, "Doing something...");

        let widget = SubagentPanelWidget::new(&state);

        // 1 agent * 2 lines (name + activity) + 2 borders = 4
        assert_eq!(widget.required_height(), 4);
    }

    #[test]
    fn test_render_header_shows_count() {
        let mut state = SubagentPanelState::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();

        state.add_subagent(SubagentDisplayInfo::new(id1, "a", 5));
        state.add_subagent(SubagentDisplayInfo::new(id2, "b", 5));
        state.update_status(id1, SubagentDisplayStatus::Running);

        let widget = SubagentPanelWidget::new(&state);
        let header = widget.render_header();

        // Header should contain count (1/2 - 1 running, 2 total)
        let content: String = header.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(content.contains("1/2"));
    }

    #[test]
    fn test_render_subagent_shows_name_and_progress() {
        let state = SubagentPanelState::new();
        let widget = SubagentPanelWidget::new(&state);

        let mut info = SubagentDisplayInfo::new(Uuid::new_v4(), "explorer", 10);
        info.current_turn = 3;
        info.status = SubagentDisplayStatus::Running;

        let lines = widget.render_subagent(&info);

        assert!(!lines.is_empty());

        // First line should contain name and progress
        let content: String = lines[0]
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        assert!(content.contains("explorer"));
        assert!(content.contains("3/10"));
    }

    #[test]
    fn test_render_subagent_shows_activity() {
        let state = SubagentPanelState::new();
        let widget = SubagentPanelWidget::new(&state);

        let mut info = SubagentDisplayInfo::new(Uuid::new_v4(), "reader", 5);
        info.last_activity = Some("Reading file.rs".to_string());

        let lines = widget.render_subagent(&info);

        // Should have 2 lines: name + activity
        assert_eq!(lines.len(), 2);

        let activity_content: String = lines[1]
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        assert!(activity_content.contains("Reading file.rs"));
    }

    // ============================================================================
    // Helper function tests
    // ============================================================================

    #[test]
    fn test_truncate_activity_short() {
        assert_eq!(truncate_activity("short", 10), "short");
    }

    #[test]
    fn test_truncate_activity_exact() {
        assert_eq!(truncate_activity("exactly10!", 10), "exactly10!");
    }

    #[test]
    fn test_truncate_activity_long() {
        let result = truncate_activity("this is a very long activity string", 20);
        assert!(result.ends_with("..."));
        assert!(result.len() <= 20);
    }

    // ============================================================================
    // Widget rendering tests (verify no panics)
    // ============================================================================

    #[test]
    fn test_widget_render_empty_state() {
        let state = SubagentPanelState::new();
        let widget = SubagentPanelWidget::new(&state);

        let mut buf = Buffer::empty(Rect::new(0, 0, 50, 10));
        widget.render(Rect::new(0, 0, 50, 10), &mut buf);

        // Should not panic, buffer should be unchanged (empty state renders nothing)
    }

    #[test]
    fn test_widget_render_with_subagents() {
        let mut state = SubagentPanelState::new();
        let id = Uuid::new_v4();
        state.add_subagent(SubagentDisplayInfo::new(id, "test-agent", 10));
        state.update_status(id, SubagentDisplayStatus::Running);
        state.update_turn(id, 3);

        let widget = SubagentPanelWidget::new(&state);

        let mut buf = Buffer::empty(Rect::new(0, 0, 50, 10));
        widget.render(Rect::new(0, 0, 50, 10), &mut buf);

        // Verify something was rendered (check for border character)
        let content = buf.content.iter().map(|c| c.symbol()).collect::<String>();
        assert!(
            content.contains("Subagents") || content.contains("▼") || content.contains("│"),
            "Buffer should contain panel content"
        );
    }

    #[test]
    fn test_widget_render_collapsed() {
        let mut state = SubagentPanelState::new();
        state.add_subagent(SubagentDisplayInfo::new(
            Uuid::new_v4(),
            "hidden-agent",
            10,
        ));
        state.set_collapsed(true);

        let widget = SubagentPanelWidget::new(&state);

        let mut buf = Buffer::empty(Rect::new(0, 0, 50, 5));
        widget.render(Rect::new(0, 0, 50, 5), &mut buf);

        // Should render header but not agent details
        let content = buf.content.iter().map(|c| c.symbol()).collect::<String>();
        assert!(content.contains("▶"), "Should show collapse indicator");
    }

    #[test]
    fn test_widget_render_zero_height() {
        let mut state = SubagentPanelState::new();
        state.add_subagent(SubagentDisplayInfo::new(Uuid::new_v4(), "agent", 5));

        let widget = SubagentPanelWidget::new(&state);

        let mut buf = Buffer::empty(Rect::new(0, 0, 50, 0));
        widget.render(Rect::new(0, 0, 50, 0), &mut buf);

        // Should not panic with zero height
    }
}
