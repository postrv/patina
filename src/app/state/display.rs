/// Display and animation state extracted from AppState.
///
/// Groups fields related to scroll position, loading indicators,
/// throbber animation, and terminal dimensions.
use crate::types::ui_state::ScrollState;

/// Throbber animation characters (Braille pattern spinner).
const THROBBER_CHARS: [char; 4] = ['⠋', '⠙', '⠹', '⠸'];

pub struct DisplayState {
    /// Smart scroll state with auto-follow behavior.
    pub(crate) scroll: ScrollState,

    /// Whether an API request is in progress.
    pub(crate) loading: bool,

    /// Current throbber animation frame index (0..3).
    pub(crate) throbber_frame: usize,

    /// Cached terminal height for scroll calculations.
    /// Updated on resize events; defaults to 24 for headless/test environments.
    pub(crate) terminal_height: u16,

    /// Latest available version from background update check, if newer.
    pub(crate) update_available: Option<String>,

    /// Custom prompt bar color set by `/color`.
    pub(crate) prompt_color: Option<String>,

    /// Whether any mutation has occurred since last `mark_clean()`.
    dirty: bool,
}

impl DisplayState {
    /// Creates a new display state with sensible defaults.
    #[must_use]
    pub fn new() -> Self {
        Self {
            scroll: ScrollState::new(),
            loading: false,
            throbber_frame: 0,
            terminal_height: 24,
            update_available: None,
            prompt_color: None,
            dirty: false,
        }
    }

    /// Returns whether any mutation has occurred since last `mark_clean()`.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Clears the dirty flag after rendering.
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// Marks the display as needing a re-render.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Returns the current scroll offset.
    #[must_use]
    pub fn scroll_offset(&self) -> usize {
        self.scroll.offset()
    }

    /// Returns a reference to the scroll state.
    #[must_use]
    pub fn scroll_state(&self) -> &ScrollState {
        &self.scroll
    }

    /// Returns a mutable reference to the scroll state.
    pub fn scroll_state_mut(&mut self) -> &mut ScrollState {
        &mut self.scroll
    }

    /// Returns whether the application is loading (API request in progress).
    #[must_use]
    pub fn is_loading(&self) -> bool {
        self.loading
    }

    /// Sets the loading state.
    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
        self.dirty = true;
    }

    /// Advances the throbber animation frame.
    pub fn tick_throbber(&mut self) {
        self.dirty = true;
        self.throbber_frame = (self.throbber_frame + 1) % THROBBER_CHARS.len();
    }

    /// Returns the current throbber character.
    #[must_use]
    pub fn throbber_char(&self) -> char {
        THROBBER_CHARS[self.throbber_frame]
    }

    /// Returns the cached terminal height.
    #[must_use]
    pub fn terminal_height(&self) -> u16 {
        self.terminal_height
    }

    /// Sets the cached terminal height.
    pub fn set_terminal_height(&mut self, height: u16) {
        self.terminal_height = height;
        self.dirty = true;
    }

    /// Returns the available update version, if any.
    #[must_use]
    pub fn update_available(&self) -> Option<&str> {
        self.update_available.as_deref()
    }

    /// Sets the available update version from background check.
    pub fn set_update_available(&mut self, version: String) {
        self.update_available = Some(version);
        self.dirty = true;
    }

    /// Returns the custom prompt bar color, if set.
    #[must_use]
    pub fn prompt_color(&self) -> Option<&str> {
        self.prompt_color.as_deref()
    }

    /// Sets or clears the custom prompt bar color.
    pub fn set_prompt_color(&mut self, color: Option<String>) {
        self.prompt_color = color;
        self.dirty = true;
    }
}

impl Default for DisplayState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_state_defaults() {
        let state = DisplayState::new();
        assert_eq!(state.scroll_offset(), 0);
        assert!(!state.is_loading());
        assert_eq!(state.throbber_char(), '⠋');
        assert_eq!(state.terminal_height(), 24);
    }

    #[test]
    fn test_loading_toggle() {
        let mut state = DisplayState::new();
        assert!(!state.is_loading());
        state.set_loading(true);
        assert!(state.is_loading());
        state.set_loading(false);
        assert!(!state.is_loading());
    }

    #[test]
    fn test_set_loading_marks_dirty() {
        let mut state = DisplayState::new();
        state.mark_clean();
        assert!(!state.is_dirty());
        state.set_loading(true);
        assert!(state.is_dirty(), "set_loading should mark dirty");
    }

    #[test]
    fn test_set_update_available_marks_dirty() {
        let mut state = DisplayState::new();
        state.mark_clean();
        assert!(!state.is_dirty());
        state.set_update_available("2.0.0".to_string());
        assert!(state.is_dirty(), "set_update_available should mark dirty");
    }

    #[test]
    fn test_set_prompt_color_marks_dirty() {
        let mut state = DisplayState::new();
        state.mark_clean();
        assert!(!state.is_dirty());
        state.set_prompt_color(Some("red".to_string()));
        assert!(state.is_dirty(), "set_prompt_color should mark dirty");
    }

    #[test]
    fn test_set_terminal_height_marks_dirty() {
        let mut state = DisplayState::new();
        state.mark_clean();
        assert!(!state.is_dirty());
        state.set_terminal_height(50);
        assert!(state.is_dirty(), "set_terminal_height should mark dirty");
    }

    #[test]
    fn test_throbber_cycles() {
        let mut state = DisplayState::new();
        assert_eq!(state.throbber_char(), '⠋');
        state.tick_throbber();
        assert_eq!(state.throbber_char(), '⠙');
        state.tick_throbber();
        assert_eq!(state.throbber_char(), '⠹');
        state.tick_throbber();
        assert_eq!(state.throbber_char(), '⠸');
        state.tick_throbber();
        assert_eq!(state.throbber_char(), '⠋'); // wraps
    }

    #[test]
    fn test_terminal_height() {
        let mut state = DisplayState::new();
        assert_eq!(state.terminal_height(), 24);
        state.set_terminal_height(50);
        assert_eq!(state.terminal_height(), 50);
    }

    #[test]
    fn test_scroll_state_access() {
        let state = DisplayState::new();
        let scroll = state.scroll_state();
        assert_eq!(scroll.offset(), 0);
    }

    #[test]
    fn test_update_available_defaults_to_none() {
        let state = DisplayState::new();
        assert!(state.update_available().is_none());
    }

    #[test]
    fn test_set_update_available() {
        let mut state = DisplayState::new();
        state.set_update_available("2.0.0".to_string());
        assert_eq!(state.update_available(), Some("2.0.0"));
    }
}
