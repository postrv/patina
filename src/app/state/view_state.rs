use super::display::DisplayState;
use super::input::InputState;
use super::ui_selection::UISelectionState;
use super::worktree::WorktreeStatus;
use crate::tui::search::SearchState;

/// Presentation-layer state extracted from AppState.
///
/// Groups all view/display fields: display state (scroll, loading, throbber),
/// UI selection (mouse, clipboard), worktree status bar, and input buffer.
/// These fields are exclusively consumed by the render path and input handlers.
///
/// `AppState` retains thin delegation methods for dirty flag management
/// and cross-cutting orchestration.
pub struct ViewState {
    /// Display, scroll, and animation state.
    pub(crate) display: DisplayState,
    /// UI selection and copy state.
    pub(crate) ui_selection: UISelectionState,
    /// Git worktree status (branch, modified, ahead/behind).
    pub(crate) worktree: WorktreeStatus,
    /// Input buffer state (text, cursor, completion).
    pub(crate) input_state: InputState,
    /// Transcript search state (Ctrl+F).
    pub(crate) search_state: SearchState,
}

impl ViewState {
    /// Returns whether any view component needs a re-render.
    #[must_use]
    pub fn needs_render(&self) -> bool {
        self.display.is_dirty() || self.input_state.is_dirty() || self.worktree.is_dirty()
    }

    /// Clears dirty flags on all view components after rendering.
    pub fn mark_rendered(&mut self) {
        self.display.mark_clean();
        self.input_state.mark_clean();
        self.worktree.mark_clean();
    }

    // ========================================================================
    // Display Accessors
    // ========================================================================

    /// Returns a reference to the display state.
    #[must_use]
    pub fn display(&self) -> &DisplayState {
        &self.display
    }

    /// Returns a mutable reference to the display state.
    pub fn display_mut(&mut self) -> &mut DisplayState {
        &mut self.display
    }

    // ========================================================================
    // UI Selection Accessors
    // ========================================================================

    /// Returns a reference to the UI selection state.
    #[must_use]
    pub fn ui_selection(&self) -> &UISelectionState {
        &self.ui_selection
    }

    /// Returns a mutable reference to the UI selection state.
    pub fn ui_selection_mut(&mut self) -> &mut UISelectionState {
        &mut self.ui_selection
    }

    // ========================================================================
    // Worktree Accessors
    // ========================================================================

    /// Returns a reference to the worktree status.
    #[must_use]
    pub fn worktree(&self) -> &WorktreeStatus {
        &self.worktree
    }

    /// Returns a mutable reference to the worktree status.
    pub fn worktree_mut(&mut self) -> &mut WorktreeStatus {
        &mut self.worktree
    }

    // ========================================================================
    // Input State Accessors
    // ========================================================================

    /// Returns a reference to the input state.
    #[must_use]
    pub fn input_state(&self) -> &InputState {
        &self.input_state
    }

    /// Returns a mutable reference to the input state.
    pub fn input_state_mut(&mut self) -> &mut InputState {
        &mut self.input_state
    }

    // ========================================================================
    // Search State Accessors
    // ========================================================================

    /// Returns a reference to the search state.
    #[must_use]
    pub fn search_state(&self) -> &SearchState {
        &self.search_state
    }

    /// Returns a mutable reference to the search state.
    pub fn search_state_mut(&mut self) -> &mut SearchState {
        &mut self.search_state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ui_state::{FocusArea, SelectionState};

    fn make_view_state() -> ViewState {
        ViewState {
            display: DisplayState::new(),
            ui_selection: UISelectionState {
                selection: SelectionState::new(),
                copy_pending: false,
                rendered_lines_cache: Vec::new(),
                focus_area: FocusArea::Content,
            },
            worktree: WorktreeStatus::new(),
            input_state: InputState::new(),
            search_state: SearchState::new(),
        }
    }

    #[test]
    fn test_view_state_new_defaults() {
        let vs = make_view_state();
        assert!(!vs.needs_render());
        assert!(!vs.display().is_loading());
        assert_eq!(vs.input_state().text(), "");
        assert!(vs.worktree().branch().is_none());
    }

    #[test]
    fn test_view_state_needs_render_from_display() {
        let mut vs = make_view_state();
        vs.display_mut().mark_dirty();
        assert!(vs.needs_render());
        vs.mark_rendered();
        assert!(!vs.needs_render());
    }

    #[test]
    fn test_view_state_needs_render_from_input() {
        let mut vs = make_view_state();
        vs.input_state_mut().insert_char('a');
        assert!(vs.needs_render());
        vs.mark_rendered();
        assert!(!vs.needs_render());
    }

    #[test]
    fn test_view_state_needs_render_from_worktree() {
        let mut vs = make_view_state();
        vs.worktree_mut().set_branch("main".to_string());
        assert!(vs.needs_render());
        vs.mark_rendered();
        assert!(!vs.needs_render());
    }

    #[test]
    fn test_view_state_accessors() {
        let mut vs = make_view_state();
        vs.display_mut().set_loading(true);
        assert!(vs.display().is_loading());

        vs.worktree_mut().set_modified(3);
        assert_eq!(vs.worktree().modified(), 3);

        vs.ui_selection_mut().set_focus_area(FocusArea::Input);
        assert_eq!(vs.ui_selection().focus_area(), FocusArea::Input);
    }

    #[test]
    fn test_view_state_search_state_accessor() {
        let mut vs = make_view_state();
        assert!(!vs.search_state().is_active());
        vs.search_state_mut().set_active(true);
        assert!(vs.search_state().is_active());
    }

    #[test]
    fn test_view_state_search_state_query() {
        let mut vs = make_view_state();
        let entries = vec!["hello world"];
        vs.search_state_mut().set_query("hello", &entries);
        assert_eq!(vs.search_state().query(), "hello");
        assert_eq!(vs.search_state().match_count(), 1);
    }
}
