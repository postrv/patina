//! Clipboard operations: select all, copy, and paste.

use tracing::{debug, info, warn};

use crate::app::context::AppContext;
use crate::types::ui_state::FocusArea;

/// Selects all content and sets focus to the content area.
pub(super) fn handle_select_all(ctx: &mut AppContext<'_>) {
    let line_count = ctx.state.ui_selection().rendered_line_count();
    let timeline_len = ctx.state.timeline().len();
    debug!(
        line_count,
        timeline_len,
        focus_area = ?ctx.state.ui_selection().focus_area(),
        "select_all triggered",
    );
    if line_count == 0 {
        debug!(
            "select_all: no content to select (cache empty, timeline_len={})",
            timeline_len
        );
    } else {
        ctx.state
            .ui_selection_mut()
            .set_focus_area(FocusArea::Content);
        ctx.state
            .ui_selection_mut()
            .selection_mut()
            .select_all(line_count);
        ctx.state.mark_full_redraw();
        info!(line_count, "Selected all {} lines", line_count);
    }
}

/// Copies the current selection to the system clipboard.
pub(super) fn handle_copy(state: &crate::app::state::AppState) {
    let selection = state.ui_selection().selection();
    let cache_len = state.ui_selection().rendered_line_count();

    if let Some((start, end)) = selection.range() {
        let selected_lines = end.line.saturating_sub(start.line) + 1;
        debug!(
            start_line = start.line,
            end_line = end.line,
            selected_lines,
            cache_len,
            "copy: attempting to copy {} lines from cache of {} lines",
            selected_lines,
            cache_len
        );

        match state.ui_selection().copy_from_cache() {
            Ok(true) => {
                info!("Copied {} lines to clipboard", selected_lines);
            }
            Ok(false) => {
                warn!(
                    "copy: no text extracted (cache_len={}, selection=L{}-L{})",
                    cache_len, start.line, end.line
                );
            }
            Err(e) => {
                warn!("copy: clipboard error: {}", e);
            }
        }
    } else {
        debug!(
            "copy: no selection (has_selection={})",
            selection.has_selection()
        );
    }
}

/// Pastes text from the system clipboard into the input buffer.
pub(super) fn handle_paste(state: &mut crate::app::state::AppState) {
    match crate::tui::clipboard::paste_from_clipboard() {
        Ok(text) => {
            for c in text.chars() {
                // Skip control characters except newlines
                if c == '\n' || (!c.is_control()) {
                    state.insert_char(c);
                }
            }
            info!(len = text.len(), "Pasted from clipboard");
        }
        Err(e) => {
            warn!("paste: clipboard error: {}", e);
        }
    }
}
