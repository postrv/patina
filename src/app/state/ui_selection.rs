use crate::tui::selection::{FocusArea, SelectionState};
use anyhow::Result;

/// UI selection and copy state extracted from AppState.
///
/// Groups fields related to text selection, clipboard copy operations,
/// and UI focus tracking.
pub struct UISelectionState {
    /// Text selection state for copy/paste functionality.
    pub(crate) selection: SelectionState,

    /// Flag indicating a copy operation was requested.
    pub(crate) copy_pending: bool,

    /// Cached rendered lines for copy operations.
    pub(crate) rendered_lines_cache: Vec<String>,

    /// Which area of the UI currently has focus.
    pub(crate) focus_area: FocusArea,
}

impl UISelectionState {
    /// Returns the selection state for read access.
    #[must_use]
    pub fn selection(&self) -> &SelectionState {
        &self.selection
    }

    /// Returns the selection state for modification.
    pub fn selection_mut(&mut self) -> &mut SelectionState {
        &mut self.selection
    }

    /// Returns the current focus area.
    #[must_use]
    pub fn focus_area(&self) -> FocusArea {
        self.focus_area
    }

    /// Sets the focus area, clearing selection if focus changes.
    pub fn set_focus_area(&mut self, area: FocusArea) {
        if self.focus_area != area {
            self.selection.clear();
            self.focus_area = area;
        }
    }

    /// Requests a copy operation to be performed during the next render.
    pub fn request_copy(&mut self) {
        self.copy_pending = true;
    }

    /// Checks and clears the copy pending flag. Returns `true` if a copy was requested.
    pub fn take_copy_pending(&mut self) -> bool {
        std::mem::take(&mut self.copy_pending)
    }

    /// Returns the total number of rendered lines in the cache.
    #[must_use]
    pub fn rendered_line_count(&self) -> usize {
        self.rendered_lines_cache.len()
    }

    /// Updates the cached rendered lines for copy operations.
    pub fn update_rendered_lines_cache(&mut self, lines: &[ratatui::text::Line<'_>], width: usize) {
        self.rendered_lines_cache = crate::tui::wrap_lines_to_strings(lines, width);
    }

    /// Updates the cached rendered lines from pre-wrapped strings.
    ///
    /// This accepts lines already wrapped by the render pass (via `RenderFeedback`),
    /// avoiding redundant wrapping computation.
    pub fn update_rendered_lines_from_strings(&mut self, wrapped: &[String]) {
        self.rendered_lines_cache = wrapped.to_vec();
    }

    /// Extracts the selected text from the cached rendered lines.
    ///
    /// Returns `None` if there is no selection or the cache is empty.
    #[must_use]
    pub fn extract_selected_text(&self) -> Option<String> {
        let (start, end) = self.selection.range()?;

        if self.rendered_lines_cache.is_empty() {
            return None;
        }

        let mut result = String::new();
        for (line_idx, line_text) in self.rendered_lines_cache.iter().enumerate() {
            if line_idx < start.line {
                continue;
            }
            if line_idx > end.line {
                break;
            }

            let (col_start, col_end) = if line_idx == start.line && line_idx == end.line {
                (start.col, end.col.min(line_text.len()))
            } else if line_idx == start.line {
                (start.col, line_text.len())
            } else if line_idx == end.line {
                (0, end.col.min(line_text.len()))
            } else {
                (0, line_text.len())
            };

            let col_start = col_start.min(line_text.len());
            let col_end = col_end.min(line_text.len());

            if col_start <= col_end {
                if !result.is_empty() {
                    result.push('\n');
                }
                result.push_str(&line_text[col_start..col_end]);
            }
        }

        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    /// Copies the current selection to clipboard using cached lines.
    ///
    /// # Errors
    ///
    /// Returns an error if clipboard access fails.
    pub fn copy_from_cache(&self) -> Result<bool> {
        let Some(text) = self.extract_selected_text() else {
            tracing::debug!("copy_from_cache: no text to copy");
            return Ok(false);
        };

        tracing::debug!(
            result_len = text.len(),
            result_lines = text.lines().count(),
            "copy_from_cache: copying to clipboard"
        );

        crate::tui::clipboard::copy_to_clipboard(&text)?;
        Ok(true)
    }

    /// Copies the current selection to the system clipboard from live lines.
    ///
    /// # Errors
    ///
    /// Returns an error if all clipboard methods fail.
    pub fn copy_selection_to_clipboard(&self, lines: &[ratatui::text::Line<'_>]) -> Result<bool> {
        let text = self.selection.extract_text(lines);
        if text.is_empty() {
            return Ok(false);
        }

        crate::tui::clipboard::copy_to_clipboard(&text)?;
        Ok(true)
    }

    /// Determines which focus area a screen row belongs to.
    #[must_use]
    pub fn focus_area_for_row(row: u16, terminal_height: u16) -> FocusArea {
        let input_start = terminal_height.saturating_sub(3);
        if row >= input_start {
            FocusArea::Input
        } else {
            FocusArea::Content
        }
    }
}
