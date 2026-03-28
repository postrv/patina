//! Rewind picker widget for selecting checkpoints to rewind to.
//!
//! This widget displays a list of session checkpoints with timestamps and
//! message previews, allowing users to select a checkpoint to rewind to.
//!
//! # Keybindings
//!
//! - `Up`/`Down` - Navigate between checkpoints
//! - `Enter` - Confirm selection
//! - `Esc` - Dismiss the picker
//!
//! # Example
//!
//! ```
//! use patina::tui::widgets::rewind_picker::{
//!     CheckpointEntry, RewindPickerState,
//! };
//!
//! let state = RewindPickerState::default();
//! assert!(!state.is_visible());
//! ```

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, Widget},
};
use std::time::SystemTime;

/// An entry in the rewind picker representing a session checkpoint.
#[derive(Debug, Clone)]
pub struct CheckpointEntry {
    /// The index of this checkpoint in the session's checkpoint list.
    pub index: usize,
    /// When the checkpoint was created.
    pub timestamp: SystemTime,
    /// Preview of the last message at this checkpoint (first ~40 chars).
    pub message_preview: String,
    /// Optional human-readable label.
    pub label: Option<String>,
}

/// State for the rewind picker widget.
///
/// Manages the list of checkpoints, current selection, and visibility.
#[derive(Debug, Default)]
pub struct RewindPickerState {
    /// Checkpoint entries to display.
    checkpoints: Vec<CheckpointEntry>,
    /// Currently selected index.
    selected: usize,
    /// Whether the picker is currently visible.
    visible: bool,
}

impl RewindPickerState {
    /// Creates a new picker state with the given checkpoint entries.
    ///
    /// The picker is initially not visible; call [`show`](Self::show) to display it.
    ///
    /// # Example
    ///
    /// ```
    /// use patina::tui::widgets::rewind_picker::RewindPickerState;
    ///
    /// let state = RewindPickerState::new(vec![]);
    /// assert!(state.checkpoint_entries().is_empty());
    /// assert!(!state.is_visible());
    /// ```
    #[must_use]
    pub fn new(checkpoints: Vec<CheckpointEntry>) -> Self {
        Self {
            checkpoints,
            selected: 0,
            visible: false,
        }
    }

    /// Returns the checkpoint entries.
    #[must_use]
    pub fn checkpoint_entries(&self) -> &[CheckpointEntry] {
        &self.checkpoints
    }

    /// Returns the currently selected index.
    #[must_use]
    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// Returns the currently selected checkpoint entry, if any.
    #[must_use]
    pub fn selected_entry(&self) -> Option<&CheckpointEntry> {
        self.checkpoints.get(self.selected)
    }

    /// Returns whether the picker is visible.
    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Shows the picker.
    pub fn show(&mut self) {
        self.visible = true;
    }

    /// Hides the picker.
    pub fn hide(&mut self) {
        self.visible = false;
    }

    /// Moves selection to the next item.
    ///
    /// Stops at the last item (does not wrap).
    pub fn select_next(&mut self) {
        if !self.checkpoints.is_empty() && self.selected < self.checkpoints.len() - 1 {
            self.selected += 1;
        }
    }

    /// Moves selection to the previous item.
    ///
    /// Stops at the first item (does not wrap).
    pub fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Sets the checkpoint entries and resets selection.
    pub fn set_checkpoints(&mut self, checkpoints: Vec<CheckpointEntry>) {
        self.checkpoints = checkpoints;
        self.selected = 0;
    }

    /// Confirms the current selection and returns the checkpoint index.
    ///
    /// Returns `None` if no checkpoints are available. Hides the picker on
    /// successful confirmation.
    #[must_use]
    pub fn confirm_selection(&mut self) -> Option<usize> {
        let entry = self.checkpoints.get(self.selected)?;
        let idx = entry.index;
        self.visible = false;
        Some(idx)
    }
}

/// Formats a duration as a human-readable relative time string.
///
/// # Examples
///
/// - "just now" for durations under 60 seconds
/// - "2m ago" for 2 minutes
/// - "1h ago" for 1 hour
/// - "3d ago" for 3 days
#[must_use]
fn format_relative_time(timestamp: SystemTime) -> String {
    let elapsed = timestamp.elapsed().unwrap_or_default();
    let secs = elapsed.as_secs();

    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}

/// Widget for displaying and selecting checkpoints for rewind.
///
/// Renders a list of checkpoints with timestamps, message previews, and
/// optional labels.
#[derive(Clone)]
pub struct RewindPickerWidget<'a> {
    /// Reference to the picker state.
    state: &'a RewindPickerState,

    /// Block decoration for the widget.
    block: Option<Block<'a>>,
}

impl<'a> RewindPickerWidget<'a> {
    /// Creates a new rewind picker widget.
    ///
    /// # Example
    ///
    /// ```
    /// use patina::tui::widgets::rewind_picker::{RewindPickerState, RewindPickerWidget};
    ///
    /// let state = RewindPickerState::default();
    /// let widget = RewindPickerWidget::new(&state);
    /// ```
    #[must_use]
    pub fn new(state: &'a RewindPickerState) -> Self {
        Self { state, block: None }
    }

    /// Sets the block decoration for the widget.
    #[must_use]
    pub fn block(mut self, block: Block<'a>) -> Self {
        self.block = Some(block);
        self
    }

    /// Formats a checkpoint entry as a display line.
    fn format_entry_line(&self, entry: &CheckpointEntry, is_selected: bool) -> Line<'a> {
        let mut spans = Vec::new();

        // Selection indicator
        if is_selected {
            spans.push(Span::styled(
                "> ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::raw("  "));
        }

        // Checkpoint index
        spans.push(Span::styled(
            format!("#{} ", entry.index),
            Style::default().fg(Color::Cyan),
        ));

        // Relative time
        let time_str = format_relative_time(entry.timestamp);
        spans.push(Span::styled(
            format!("[{}] ", time_str),
            Style::default().fg(Color::DarkGray),
        ));

        // Label (if present)
        if let Some(ref label) = entry.label {
            spans.push(Span::styled(
                format!("({}) ", label),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::ITALIC),
            ));
        }

        // Message preview
        let preview = if entry.message_preview.len() > 40 {
            format!("{}...", &entry.message_preview[..40])
        } else {
            entry.message_preview.clone()
        };

        let preview_style = if is_selected {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        spans.push(Span::styled(preview, preview_style));

        Line::from(spans)
    }

    /// Creates the keybinding hints line.
    fn keybinding_hints() -> Line<'static> {
        Line::from(vec![
            Span::styled("Up/Down", Style::default().fg(Color::Cyan)),
            Span::raw(":navigate "),
            Span::styled("Enter", Style::default().fg(Color::Cyan)),
            Span::raw(":rewind "),
            Span::styled("Esc", Style::default().fg(Color::Cyan)),
            Span::raw(":cancel"),
        ])
    }
}

impl Widget for RewindPickerWidget<'_> {
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

        // Render empty state or checkpoint list
        if self.state.checkpoints.is_empty() {
            let empty_text = Line::from(Span::styled(
                "No checkpoints available.",
                Style::default().fg(Color::DarkGray),
            ));
            buf.set_line(list_area.x, list_area.y, &empty_text, list_area.width);
        } else {
            // Build list items
            let items: Vec<ListItem> = self
                .state
                .checkpoints
                .iter()
                .enumerate()
                .map(|(i, entry)| {
                    let is_selected = i == self.state.selected;
                    let line = self.format_entry_line(entry, is_selected);
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

    fn make_entries(count: usize) -> Vec<CheckpointEntry> {
        (0..count)
            .map(|i| CheckpointEntry {
                index: i,
                timestamp: SystemTime::now(),
                message_preview: format!("Message at checkpoint {i}"),
                label: if i % 2 == 0 {
                    Some(format!("label-{i}"))
                } else {
                    None
                },
            })
            .collect()
    }

    #[test]
    fn test_rewind_picker_state_default() {
        let state = RewindPickerState::default();
        assert!(state.checkpoint_entries().is_empty());
        assert_eq!(state.selected_index(), 0);
        assert!(!state.is_visible());
    }

    #[test]
    fn test_rewind_picker_state_new() {
        let entries = make_entries(3);
        let state = RewindPickerState::new(entries);
        assert_eq!(state.checkpoint_entries().len(), 3);
        assert_eq!(state.selected_index(), 0);
        assert!(!state.is_visible());
    }

    #[test]
    fn test_rewind_picker_visibility_toggle() {
        let mut state = RewindPickerState::new(make_entries(2));
        assert!(!state.is_visible());

        state.show();
        assert!(state.is_visible());

        state.hide();
        assert!(!state.is_visible());
    }

    #[test]
    fn test_rewind_picker_navigation_down() {
        let mut state = RewindPickerState::new(make_entries(3));
        assert_eq!(state.selected_index(), 0);

        state.select_next();
        assert_eq!(state.selected_index(), 1);

        state.select_next();
        assert_eq!(state.selected_index(), 2);

        // Should not go past the end
        state.select_next();
        assert_eq!(state.selected_index(), 2);
    }

    #[test]
    fn test_rewind_picker_navigation_up() {
        let mut state = RewindPickerState::new(make_entries(3));
        state.select_next();
        state.select_next();
        assert_eq!(state.selected_index(), 2);

        state.select_previous();
        assert_eq!(state.selected_index(), 1);

        state.select_previous();
        assert_eq!(state.selected_index(), 0);

        // Should not go below zero
        state.select_previous();
        assert_eq!(state.selected_index(), 0);
    }

    #[test]
    fn test_rewind_picker_confirm_selection() {
        let mut state = RewindPickerState::new(make_entries(3));
        state.show();
        state.select_next(); // index 1

        let result = state.confirm_selection();
        assert_eq!(result, Some(1));
        assert!(!state.is_visible(), "Picker should be hidden after confirm");
    }

    #[test]
    fn test_rewind_picker_confirm_empty() {
        let mut state = RewindPickerState::new(vec![]);
        state.show();

        let result = state.confirm_selection();
        assert!(result.is_none());
    }

    #[test]
    fn test_rewind_picker_selected_entry() {
        let state = RewindPickerState::new(make_entries(3));
        let entry = state.selected_entry().unwrap();
        assert_eq!(entry.index, 0);
    }

    #[test]
    fn test_rewind_picker_set_checkpoints_resets_selection() {
        let mut state = RewindPickerState::new(make_entries(5));
        state.select_next();
        state.select_next();
        assert_eq!(state.selected_index(), 2);

        state.set_checkpoints(make_entries(2));
        assert_eq!(state.selected_index(), 0);
        assert_eq!(state.checkpoint_entries().len(), 2);
    }

    #[test]
    fn test_format_relative_time_just_now() {
        let now = SystemTime::now();
        let result = format_relative_time(now);
        assert_eq!(result, "just now");
    }

    #[test]
    fn test_rewind_picker_navigation_empty_list() {
        let mut state = RewindPickerState::new(vec![]);
        // These should not panic
        state.select_next();
        state.select_previous();
        assert_eq!(state.selected_index(), 0);
    }
}
