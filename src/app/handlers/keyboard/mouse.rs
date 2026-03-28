//! Mouse event handling: click, drag, scroll.

use crossterm::event::{MouseButton, MouseEventKind};
use tracing::debug;

use crate::app::context::AppContext;
use crate::app::state::UISelectionState;
use crate::types::ui_state::{ContentPosition, FocusArea};

/// Processes a mouse event (click, drag, scroll).
pub(super) fn handle_mouse(
    ctx: &mut AppContext<'_>,
    kind: MouseEventKind,
    row: u16,
    column: u16,
    terminal_height: u16,
) {
    match kind {
        MouseEventKind::Down(MouseButton::Left) => {
            let clicked_area = UISelectionState::focus_area_for_row(row, terminal_height);
            ctx.state.ui_selection_mut().set_focus_area(clicked_area);

            if clicked_area == FocusArea::Content {
                let first_visible = ctx.state.display().scroll_state().first_visible_line();
                let content_row = row.saturating_sub(1) as usize;
                let pos = ContentPosition::new(
                    first_visible + content_row,
                    column.saturating_sub(1) as usize,
                );
                ctx.state.ui_selection_mut().selection_mut().start(pos);
            }
            ctx.state.mark_full_redraw();
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if ctx.state.ui_selection().focus_area() == FocusArea::Content {
                let first_visible = ctx.state.display().scroll_state().first_visible_line();
                let content_row = row.saturating_sub(1) as usize;
                let pos = ContentPosition::new(
                    first_visible + content_row,
                    column.saturating_sub(1) as usize,
                );
                ctx.state.ui_selection_mut().selection_mut().update(pos);
                ctx.state.mark_full_redraw();
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            if ctx.state.ui_selection().focus_area() == FocusArea::Content {
                ctx.state.ui_selection_mut().selection_mut().end();
                ctx.state.mark_full_redraw();
            }
        }
        MouseEventKind::ScrollUp => {
            debug!("mouse scroll up");
            ctx.state.scroll_up(3);
        }
        MouseEventKind::ScrollDown => {
            debug!("mouse scroll down");
            ctx.state.scroll_down(3);
        }
        _ => {}
    }
}
