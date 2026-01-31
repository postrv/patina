//! Worktree picker widget for selecting and managing git worktrees.
//!
//! This widget displays a list of available worktrees with status indicators
//! and provides keybindings for common operations.
//!
//! # Keybindings
//!
//! - `n` - Create new worktree
//! - `s` - Switch to selected worktree
//! - `d` - Delete selected worktree
//! - `c` - Clean up prunable worktrees
//!
//! # Example
//!
//! ```no_run
//! use patina::tui::widgets::worktree_picker::{WorktreePickerState, WorktreePickerWidget};
//! use patina::worktree::WorktreeInfo;
//!
//! let state = WorktreePickerState::default();
//! let widget = WorktreePickerWidget::new(&state);
//! // Render widget in a ratatui frame
//! ```

use crate::worktree::{WorktreeInfo, WorktreeStatus};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, Widget},
};
use std::collections::HashMap;

/// State for the worktree picker widget.
///
/// Manages the list of worktrees, their statuses, and the current selection.
#[derive(Debug, Default)]
pub struct WorktreePickerState {
    /// List of available worktrees.
    worktrees: Vec<WorktreeInfo>,

    /// Status for each worktree, keyed by name.
    statuses: HashMap<String, WorktreeStatus>,

    /// Currently selected index.
    selected: usize,
}

impl WorktreePickerState {
    /// Creates a new picker state with the given worktrees.
    ///
    /// # Example
    ///
    /// ```
    /// use patina::tui::widgets::worktree_picker::WorktreePickerState;
    ///
    /// let state = WorktreePickerState::new(vec![]);
    /// assert!(state.worktrees().is_empty());
    /// ```
    #[must_use]
    pub fn new(worktrees: Vec<WorktreeInfo>) -> Self {
        Self {
            worktrees,
            statuses: HashMap::new(),
            selected: 0,
        }
    }

    /// Returns the current list of worktrees.
    #[must_use]
    pub fn worktrees(&self) -> &[WorktreeInfo] {
        &self.worktrees
    }

    /// Returns the currently selected index.
    #[must_use]
    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// Returns the currently selected worktree, if any.
    #[must_use]
    pub fn selected_worktree(&self) -> Option<&WorktreeInfo> {
        self.worktrees.get(self.selected)
    }

    /// Moves selection to the next item.
    ///
    /// Stops at the last item (does not wrap).
    pub fn select_next(&mut self) {
        if !self.worktrees.is_empty() && self.selected < self.worktrees.len() - 1 {
            self.selected += 1;
        }
    }

    /// Moves selection to the previous item.
    ///
    /// Stops at the first item (does not wrap).
    pub fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Updates the status for a worktree.
    pub fn update_status(&mut self, name: &str, status: WorktreeStatus) {
        self.statuses.insert(name.to_string(), status);
    }

    /// Gets the status for a worktree.
    #[must_use]
    pub fn get_status(&self, name: &str) -> Option<&WorktreeStatus> {
        self.statuses.get(name)
    }

    /// Sets the list of worktrees.
    pub fn set_worktrees(&mut self, worktrees: Vec<WorktreeInfo>) {
        self.worktrees = worktrees;
        // Clamp selection to valid range
        if !self.worktrees.is_empty() && self.selected >= self.worktrees.len() {
            self.selected = self.worktrees.len() - 1;
        }
    }
}

/// Widget for displaying and selecting worktrees.
///
/// Renders a list of worktrees with status indicators and keybinding hints.
#[derive(Clone)]
pub struct WorktreePickerWidget<'a> {
    /// Reference to the picker state.
    state: &'a WorktreePickerState,

    /// Block decoration for the widget.
    block: Option<Block<'a>>,
}

impl<'a> WorktreePickerWidget<'a> {
    /// Creates a new worktree picker widget.
    ///
    /// # Example
    ///
    /// ```
    /// use patina::tui::widgets::worktree_picker::{WorktreePickerState, WorktreePickerWidget};
    ///
    /// let state = WorktreePickerState::default();
    /// let widget = WorktreePickerWidget::new(&state);
    /// ```
    #[must_use]
    pub fn new(state: &'a WorktreePickerState) -> Self {
        Self { state, block: None }
    }

    /// Sets the block decoration for the widget.
    #[must_use]
    pub fn block(mut self, block: Block<'a>) -> Self {
        self.block = Some(block);
        self
    }

    /// Formats a worktree entry with status indicators.
    fn format_worktree_line(&self, worktree: &WorktreeInfo, is_selected: bool) -> Line<'a> {
        let mut spans = Vec::new();

        // Selection indicator
        if is_selected {
            spans.push(Span::styled(
                "► ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::raw("  "));
        }

        // Main worktree indicator
        if worktree.is_main {
            spans.push(Span::styled("★ ", Style::default().fg(Color::Cyan)));
        } else {
            spans.push(Span::raw("  "));
        }

        // Worktree name
        let name_style = if is_selected {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        spans.push(Span::styled(worktree.name.clone(), name_style));

        // Branch name (for non-main worktrees)
        if !worktree.is_main {
            spans.push(Span::styled(
                format!(" ({})", worktree.branch),
                Style::default().fg(Color::DarkGray),
            ));
        }

        // Status indicators
        if let Some(status) = self.state.get_status(&worktree.name) {
            spans.push(Span::raw(" "));

            // Dirty indicator (modified/staged/untracked)
            if !status.is_clean() {
                let dirty_count = status.modified + status.staged + status.untracked;
                spans.push(Span::styled(
                    format!("●{}", dirty_count),
                    Style::default().fg(Color::Yellow),
                ));
                spans.push(Span::raw(" "));
            }

            // Ahead indicator
            if status.ahead > 0 {
                spans.push(Span::styled(
                    format!("↑{}", status.ahead),
                    Style::default().fg(Color::Green),
                ));
                spans.push(Span::raw(" "));
            }

            // Behind indicator
            if status.behind > 0 {
                spans.push(Span::styled(
                    format!("↓{}", status.behind),
                    Style::default().fg(Color::Red),
                ));
            }
        }

        // Locked indicator
        if worktree.is_locked {
            spans.push(Span::styled(
                " [locked]",
                Style::default().fg(Color::Magenta),
            ));
        }

        // Prunable indicator
        if worktree.is_prunable {
            spans.push(Span::styled(
                " [prunable]",
                Style::default().fg(Color::DarkGray),
            ));
        }

        Line::from(spans)
    }

    /// Creates the keybinding hints line.
    fn keybinding_hints() -> Line<'static> {
        Line::from(vec![
            Span::styled("n", Style::default().fg(Color::Cyan)),
            Span::raw(":new "),
            Span::styled("s", Style::default().fg(Color::Cyan)),
            Span::raw(":switch "),
            Span::styled("d", Style::default().fg(Color::Cyan)),
            Span::raw(":delete "),
            Span::styled("c", Style::default().fg(Color::Cyan)),
            Span::raw(":clean"),
        ])
    }
}

impl Widget for WorktreePickerWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Calculate inner area if we have a block
        let inner_area = if let Some(ref block) = self.block {
            let inner = block.inner(area);
            block.clone().render(area, buf);
            inner
        } else {
            area
        };

        // Reserve space for keybinding hints at the bottom
        let (list_area, hints_area) = if inner_area.height > 2 {
            (
                Rect {
                    height: inner_area.height - 1,
                    ..inner_area
                },
                Rect {
                    y: inner_area.y + inner_area.height - 1,
                    height: 1,
                    ..inner_area
                },
            )
        } else {
            (inner_area, Rect::default())
        };

        // Render empty state or worktree list
        if self.state.worktrees.is_empty() {
            let empty_text = Line::from(Span::styled(
                "No worktrees found. Press 'n' to create one.",
                Style::default().fg(Color::DarkGray),
            ));
            buf.set_line(list_area.x, list_area.y, &empty_text, list_area.width);
        } else {
            // Build list items
            let items: Vec<ListItem> = self
                .state
                .worktrees
                .iter()
                .enumerate()
                .map(|(i, worktree)| {
                    let is_selected = i == self.state.selected;
                    let line = self.format_worktree_line(worktree, is_selected);
                    ListItem::new(line)
                })
                .collect();

            let list = List::new(items);
            list.render(list_area, buf);
        }

        // Render keybinding hints
        if hints_area.height > 0 {
            let hints = Self::keybinding_hints();
            buf.set_line(hints_area.x, hints_area.y, &hints, hints_area.width);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_picker_state_new() {
        let state = WorktreePickerState::new(vec![]);
        assert!(state.worktrees().is_empty());
        assert_eq!(state.selected_index(), 0);
    }

    #[test]
    fn test_picker_state_navigation() {
        let worktrees = vec![
            WorktreeInfo {
                name: "a".to_string(),
                path: "/a".into(),
                branch: "main".to_string(),
                is_main: true,
                is_locked: false,
                is_prunable: false,
            },
            WorktreeInfo {
                name: "b".to_string(),
                path: "/b".into(),
                branch: "wt/b".to_string(),
                is_main: false,
                is_locked: false,
                is_prunable: false,
            },
        ];

        let mut state = WorktreePickerState::new(worktrees);

        assert_eq!(state.selected_index(), 0);
        state.select_next();
        assert_eq!(state.selected_index(), 1);
        state.select_next();
        assert_eq!(state.selected_index(), 1); // Should stay at 1
        state.select_previous();
        assert_eq!(state.selected_index(), 0);
        state.select_previous();
        assert_eq!(state.selected_index(), 0); // Should stay at 0
    }

    #[test]
    fn test_picker_state_status_update() {
        let mut state = WorktreePickerState::new(vec![WorktreeInfo {
            name: "test".to_string(),
            path: "/test".into(),
            branch: "main".to_string(),
            is_main: true,
            is_locked: false,
            is_prunable: false,
        }]);

        state.update_status(
            "test",
            WorktreeStatus {
                modified: 2,
                staged: 1,
                untracked: 0,
                ahead: 3,
                behind: 0,
            },
        );

        let status = state.get_status("test").unwrap();
        assert_eq!(status.modified, 2);
        assert_eq!(status.ahead, 3);
    }
}
