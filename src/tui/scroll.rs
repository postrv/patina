//! Smart auto-scroll state management for TUI.
//!
//! This module provides intelligent scrolling behavior that:
//! - Follows new content automatically (Follow mode)
//! - Preserves manual scroll position when user scrolls (Manual mode)
//! - Pauses auto-scroll during certain operations (Paused mode)
//!
//! The core state types ([`AutoScrollMode`] and [`ScrollState`]) are defined in
//! [`crate::types::ui_state`] to break the app<->tui dependency cycle.
//! They are re-exported here for backward compatibility.
//!
//! # Example
//!
//! ```
//! use patina::tui::scroll::{AutoScrollMode, ScrollState};
//!
//! let mut scroll = ScrollState::new();
//!
//! // Initial mode is Follow
//! assert!(matches!(scroll.mode(), AutoScrollMode::Follow));
//!
//! // User scrolls up - switches to Manual
//! scroll.scroll_up(10);
//! assert!(matches!(scroll.mode(), AutoScrollMode::Manual));
//!
//! // User scrolls to bottom - resumes Follow
//! scroll.scroll_to_bottom(100);
//! assert!(matches!(scroll.mode(), AutoScrollMode::Follow));
//! ```

// Re-export core types from the shared module
pub use crate::types::ui_state::{AutoScrollMode, ScrollState};

// =============================================================================
// Viewport Virtualization
// =============================================================================

/// Tracks the line range for a content item.
///
/// Used to efficiently determine which content items are visible
/// in the current viewport without iterating through all items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentRange {
    /// Index of the content item (e.g., message index).
    pub index: usize,
    /// Starting line number (inclusive).
    pub start_line: usize,
    /// Ending line number (exclusive).
    pub end_line: usize,
}

impl ContentRange {
    /// Creates a new content range.
    #[must_use]
    pub fn new(index: usize, start_line: usize, end_line: usize) -> Self {
        Self {
            index,
            start_line,
            end_line,
        }
    }

    /// Returns the number of lines in this range.
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.end_line.saturating_sub(self.start_line)
    }

    /// Returns true if this range overlaps with the given line range.
    #[must_use]
    pub fn overlaps(&self, start: usize, end: usize) -> bool {
        self.start_line < end && self.end_line > start
    }
}

/// Virtualized viewport for efficient rendering of large content.
///
/// This struct tracks content ranges and determines which items need
/// to be rendered for the current viewport position.
///
/// # Example
///
/// ```
/// use patina::tui::scroll::VirtualizedViewport;
///
/// let mut viewport = VirtualizedViewport::new();
///
/// // Register content items with their line counts
/// viewport.add_content(0, 5);  // Message 0: 5 lines
/// viewport.add_content(1, 10); // Message 1: 10 lines
/// viewport.add_content(2, 3);  // Message 2: 3 lines
///
/// // Get visible content indices for a viewport
/// let visible = viewport.visible_indices(0, 8); // Lines 0-8 visible
/// assert!(visible.contains(&0));
/// assert!(visible.contains(&1));
/// ```
#[derive(Debug, Clone, Default)]
pub struct VirtualizedViewport {
    /// Ranges for each content item.
    ranges: Vec<ContentRange>,
    /// Total line count across all content.
    total_lines: usize,
    /// Buffer lines to render beyond the visible area.
    buffer: usize,
}

impl VirtualizedViewport {
    /// Creates a new virtualized viewport.
    #[must_use]
    pub fn new() -> Self {
        Self {
            ranges: Vec::new(),
            total_lines: 0,
            buffer: 5, // Render 5 extra lines above/below for smooth scrolling
        }
    }

    /// Creates a virtualized viewport with a custom buffer size.
    #[must_use]
    pub fn with_buffer(buffer: usize) -> Self {
        Self {
            ranges: Vec::new(),
            total_lines: 0,
            buffer,
        }
    }

    /// Clears all content ranges.
    pub fn clear(&mut self) {
        self.ranges.clear();
        self.total_lines = 0;
    }

    /// Adds a content item with the specified line count.
    ///
    /// Returns the starting line number for this content.
    pub fn add_content(&mut self, index: usize, line_count: usize) -> usize {
        let start_line = self.total_lines;
        let end_line = start_line + line_count;

        self.ranges
            .push(ContentRange::new(index, start_line, end_line));
        self.total_lines = end_line;

        start_line
    }

    /// Returns the total number of lines.
    #[must_use]
    pub fn total_lines(&self) -> usize {
        self.total_lines
    }

    /// Returns the number of content items.
    #[must_use]
    pub fn content_count(&self) -> usize {
        self.ranges.len()
    }

    /// Returns the line range for a specific content index.
    #[must_use]
    pub fn range_for(&self, index: usize) -> Option<&ContentRange> {
        self.ranges.iter().find(|r| r.index == index)
    }

    /// Returns indices of content items that are visible in the given line range.
    ///
    /// The range is inclusive of `start_line` and exclusive of `end_line`.
    /// A buffer is added to include items just outside the visible range
    /// for smoother scrolling.
    #[must_use]
    pub fn visible_indices(&self, start_line: usize, end_line: usize) -> Vec<usize> {
        // Expand the range by the buffer
        let buffered_start = start_line.saturating_sub(self.buffer);
        let buffered_end = end_line.saturating_add(self.buffer);

        self.ranges
            .iter()
            .filter(|r| r.overlaps(buffered_start, buffered_end))
            .map(|r| r.index)
            .collect()
    }

    /// Returns content ranges that are visible in the given line range.
    #[must_use]
    pub fn visible_ranges(&self, start_line: usize, end_line: usize) -> Vec<&ContentRange> {
        let buffered_start = start_line.saturating_sub(self.buffer);
        let buffered_end = end_line.saturating_add(self.buffer);

        self.ranges
            .iter()
            .filter(|r| r.overlaps(buffered_start, buffered_end))
            .collect()
    }

    /// Returns true if the given content index is visible.
    #[must_use]
    pub fn is_visible(&self, index: usize, start_line: usize, end_line: usize) -> bool {
        if let Some(range) = self.range_for(index) {
            let buffered_start = start_line.saturating_sub(self.buffer);
            let buffered_end = end_line.saturating_add(self.buffer);
            range.overlaps(buffered_start, buffered_end)
        } else {
            false
        }
    }

    /// Calculates the visible line range from scroll state.
    ///
    /// # Arguments
    ///
    /// * `scroll_offset` - Lines scrolled from bottom (0 = at bottom)
    /// * `viewport_height` - Height of the visible viewport
    ///
    /// # Returns
    ///
    /// A tuple of (start_line, end_line) for the visible range.
    #[must_use]
    pub fn visible_line_range(
        &self,
        scroll_offset: usize,
        viewport_height: usize,
    ) -> (usize, usize) {
        // scroll_offset is "lines from bottom"
        // start_line = total_lines - viewport_height - scroll_offset
        // end_line = total_lines - scroll_offset
        let end_line = self.total_lines.saturating_sub(scroll_offset);
        let start_line = end_line.saturating_sub(viewport_height);

        (start_line, end_line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // AutoScrollMode tests
    // =========================================================================

    #[test]
    fn test_auto_scroll_mode_default() {
        assert_eq!(AutoScrollMode::default(), AutoScrollMode::Follow);
    }

    #[test]
    fn test_auto_scroll_mode_should_auto_scroll() {
        assert!(AutoScrollMode::Follow.should_auto_scroll());
        assert!(!AutoScrollMode::Manual.should_auto_scroll());
        assert!(!AutoScrollMode::Paused.should_auto_scroll());
    }

    #[test]
    fn test_auto_scroll_mode_is_user_controlled() {
        assert!(!AutoScrollMode::Follow.is_user_controlled());
        assert!(AutoScrollMode::Manual.is_user_controlled());
        assert!(AutoScrollMode::Paused.is_user_controlled());
    }

    // =========================================================================
    // ScrollState construction tests
    // =========================================================================

    #[test]
    fn test_scroll_state_new() {
        let state = ScrollState::new();
        assert_eq!(state.offset(), 0);
        assert_eq!(state.mode(), AutoScrollMode::Follow);
        assert_eq!(state.content_height(), 0);
        assert_eq!(state.viewport_height(), 0);
    }

    #[test]
    fn test_scroll_state_default() {
        let state = ScrollState::default();
        assert_eq!(state.offset(), 0);
        assert_eq!(state.mode(), AutoScrollMode::Follow);
    }

    // =========================================================================
    // Auto-scroll behavior tests (from implementation plan)
    // =========================================================================

    #[test]
    fn test_auto_scroll_follows_new_content() {
        let mut state = ScrollState::new();
        state.set_viewport_height(20);

        // Initial content
        state.set_content_height(30);
        assert_eq!(state.offset(), 0); // At bottom

        // More content added
        state.set_content_height(50);

        // Should stay at bottom (offset 0) in Follow mode
        assert_eq!(state.offset(), 0);
        assert!(state.is_at_bottom());
        assert_eq!(state.mode(), AutoScrollMode::Follow);
    }

    #[test]
    fn test_user_scroll_up_switches_to_manual() {
        let mut state = ScrollState::new();
        state.set_viewport_height(20);
        state.set_content_height(100);

        // Initially in Follow mode at bottom
        assert_eq!(state.mode(), AutoScrollMode::Follow);
        assert_eq!(state.offset(), 0);

        // User scrolls up
        state.scroll_up(10);

        // Should switch to Manual mode
        assert_eq!(state.mode(), AutoScrollMode::Manual);
        assert_eq!(state.offset(), 10);
    }

    #[test]
    fn test_scroll_to_bottom_resumes_follow() {
        let mut state = ScrollState::new();
        state.set_viewport_height(20);
        state.set_content_height(100);

        // User scrolls up (switches to Manual)
        state.scroll_up(30);
        assert_eq!(state.mode(), AutoScrollMode::Manual);

        // Scroll back to bottom
        state.scroll_to_bottom(100);

        // Should resume Follow mode
        assert_eq!(state.mode(), AutoScrollMode::Follow);
        assert_eq!(state.offset(), 0);
        assert!(state.is_at_bottom());
    }

    // =========================================================================
    // Scroll operations tests
    // =========================================================================

    #[test]
    fn test_scroll_up_increases_offset() {
        let mut state = ScrollState::new();
        state.set_viewport_height(20);
        state.set_content_height(100);

        state.scroll_up(15);
        assert_eq!(state.offset(), 15);

        state.scroll_up(10);
        assert_eq!(state.offset(), 25);
    }

    #[test]
    fn test_scroll_down_decreases_offset() {
        let mut state = ScrollState::new();
        state.set_viewport_height(20);
        state.set_content_height(100);

        state.scroll_up(50);
        assert_eq!(state.offset(), 50);

        state.scroll_down(20);
        assert_eq!(state.offset(), 30);
    }

    #[test]
    fn test_scroll_down_to_bottom_resumes_follow() {
        let mut state = ScrollState::new();
        state.set_viewport_height(20);
        state.set_content_height(100);

        // Scroll up (manual mode)
        state.scroll_up(30);
        assert_eq!(state.mode(), AutoScrollMode::Manual);

        // Scroll all the way down
        state.scroll_down(30);
        assert_eq!(state.offset(), 0);
        assert_eq!(state.mode(), AutoScrollMode::Follow);
    }

    #[test]
    fn test_scroll_up_clamps_to_max() {
        let mut state = ScrollState::new();
        state.set_viewport_height(20);
        state.set_content_height(50);

        // Try to scroll way past the top
        state.scroll_up(1000);

        // Should be clamped to max offset (50 - 20 = 30)
        assert_eq!(state.offset(), 30);
        assert!(state.is_at_top());
    }

    #[test]
    fn test_scroll_down_clamps_to_zero() {
        let mut state = ScrollState::new();
        state.set_viewport_height(20);
        state.set_content_height(50);

        state.scroll_up(20);
        state.scroll_down(1000);

        assert_eq!(state.offset(), 0);
        assert!(state.is_at_bottom());
    }

    #[test]
    fn test_scroll_to_top() {
        let mut state = ScrollState::new();
        state.set_viewport_height(20);
        state.set_content_height(100);

        state.scroll_to_top();

        assert_eq!(state.offset(), 80); // 100 - 20
        assert!(state.is_at_top());
        assert_eq!(state.mode(), AutoScrollMode::Manual);
    }

    // =========================================================================
    // First visible line tests
    // =========================================================================

    #[test]
    fn test_first_visible_line_at_bottom() {
        let mut state = ScrollState::new();
        state.set_viewport_height(20);
        state.set_content_height(100);

        // At bottom (offset=0), first visible line should be 80 (100-20)
        assert_eq!(state.first_visible_line(), 80);
    }

    #[test]
    fn test_first_visible_line_at_top() {
        let mut state = ScrollState::new();
        state.set_viewport_height(20);
        state.set_content_height(100);

        state.scroll_to_top();

        // At top (offset=80), first visible line should be 0
        assert_eq!(state.first_visible_line(), 0);
    }

    #[test]
    fn test_first_visible_line_middle() {
        let mut state = ScrollState::new();
        state.set_viewport_height(20);
        state.set_content_height(100);

        // Scroll up by 40 (offset=40)
        // max_offset = 100-20 = 80
        // first_visible = 80 - 40 = 40
        state.scroll_up(40);

        assert_eq!(state.first_visible_line(), 40);
    }

    #[test]
    fn test_first_visible_line_content_smaller_than_viewport() {
        let mut state = ScrollState::new();
        state.set_viewport_height(100);
        state.set_content_height(50);

        // Content smaller than viewport, should always show from line 0
        assert_eq!(state.first_visible_line(), 0);
    }

    // =========================================================================
    // Pause/Resume tests
    // =========================================================================

    #[test]
    fn test_pause_switches_to_paused() {
        let mut state = ScrollState::new();
        state.pause();
        assert_eq!(state.mode(), AutoScrollMode::Paused);
    }

    #[test]
    fn test_pause_only_from_follow() {
        let mut state = ScrollState::new();
        state.set_viewport_height(20);
        state.set_content_height(100);

        // Already in Manual mode
        state.scroll_up(10);
        assert_eq!(state.mode(), AutoScrollMode::Manual);

        // Pause should not change Manual mode
        state.pause();
        assert_eq!(state.mode(), AutoScrollMode::Manual);
    }

    #[test]
    fn test_resume_from_paused() {
        let mut state = ScrollState::new();
        state.set_viewport_height(20);
        state.set_content_height(100);

        state.pause();
        state.scroll_up(30);
        assert_eq!(state.offset(), 30);

        state.resume();

        // Should be back in Follow mode at bottom
        assert_eq!(state.mode(), AutoScrollMode::Follow);
        assert_eq!(state.offset(), 0);
    }

    #[test]
    fn test_resume_only_from_paused() {
        let mut state = ScrollState::new();
        state.set_viewport_height(20);
        state.set_content_height(100);

        // In Manual mode
        state.scroll_up(30);
        assert_eq!(state.mode(), AutoScrollMode::Manual);

        // Resume should not change Manual mode
        state.resume();
        assert_eq!(state.mode(), AutoScrollMode::Manual);
    }

    #[test]
    fn test_force_follow() {
        let mut state = ScrollState::new();
        state.set_viewport_height(20);
        state.set_content_height(100);

        // In Manual mode, scrolled up
        state.scroll_up(50);
        assert_eq!(state.mode(), AutoScrollMode::Manual);
        assert_eq!(state.offset(), 50);

        // Force follow
        state.force_follow();

        // Should be in Follow mode at bottom
        assert_eq!(state.mode(), AutoScrollMode::Follow);
        assert_eq!(state.offset(), 0);
    }

    // =========================================================================
    // Viewport resize tests
    // =========================================================================

    #[test]
    fn test_set_viewport_height_clamps_offset() {
        let mut state = ScrollState::new();
        state.set_viewport_height(20);
        state.set_content_height(100);

        state.scroll_up(80); // Max offset

        // Increase viewport size
        state.set_viewport_height(50);

        // Offset should be clamped to new max (100 - 50 = 50)
        assert_eq!(state.offset(), 50);
    }

    #[test]
    fn test_viewport_larger_than_content() {
        let mut state = ScrollState::new();
        state.set_viewport_height(100);
        state.set_content_height(50);

        // Try to scroll
        state.scroll_up(50);

        // Offset should be 0 since viewport is larger than content
        assert_eq!(state.offset(), 0);
    }

    // =========================================================================
    // Content growth tests (Follow mode behavior)
    // =========================================================================

    #[test]
    fn test_content_growth_in_manual_mode() {
        let mut state = ScrollState::new();
        state.set_viewport_height(20);
        state.set_content_height(50);

        // Switch to Manual mode
        state.scroll_up(20);
        assert_eq!(state.mode(), AutoScrollMode::Manual);
        assert_eq!(state.offset(), 20);

        // Content grows
        state.set_content_height(100);

        // Offset should be preserved in Manual mode
        assert_eq!(state.offset(), 20);
        assert_eq!(state.mode(), AutoScrollMode::Manual);
    }

    #[test]
    fn test_is_at_bottom_and_top() {
        let mut state = ScrollState::new();
        state.set_viewport_height(20);
        state.set_content_height(100);

        // Initially at bottom
        assert!(state.is_at_bottom());
        assert!(!state.is_at_top());

        // Scroll to top
        state.scroll_to_top();
        assert!(!state.is_at_bottom());
        assert!(state.is_at_top());
    }

    // =========================================================================
    // VirtualizedViewport tests
    // =========================================================================

    #[test]
    fn test_virtualized_viewport_new() {
        let viewport = VirtualizedViewport::new();
        assert_eq!(viewport.total_lines(), 0);
        assert_eq!(viewport.content_count(), 0);
    }

    #[test]
    fn test_virtualized_viewport_add_content() {
        let mut viewport = VirtualizedViewport::new();

        let start0 = viewport.add_content(0, 5);
        assert_eq!(start0, 0);
        assert_eq!(viewport.total_lines(), 5);

        let start1 = viewport.add_content(1, 10);
        assert_eq!(start1, 5);
        assert_eq!(viewport.total_lines(), 15);

        let start2 = viewport.add_content(2, 3);
        assert_eq!(start2, 15);
        assert_eq!(viewport.total_lines(), 18);

        assert_eq!(viewport.content_count(), 3);
    }

    #[test]
    fn test_content_range_new() {
        let range = ContentRange::new(0, 10, 20);
        assert_eq!(range.index, 0);
        assert_eq!(range.start_line, 10);
        assert_eq!(range.end_line, 20);
        assert_eq!(range.line_count(), 10);
    }

    #[test]
    fn test_content_range_overlaps() {
        let range = ContentRange::new(0, 10, 20);

        // Overlapping cases
        assert!(range.overlaps(5, 15)); // Starts before, ends inside
        assert!(range.overlaps(15, 25)); // Starts inside, ends after
        assert!(range.overlaps(12, 18)); // Fully inside
        assert!(range.overlaps(5, 25)); // Fully contains

        // Non-overlapping cases
        assert!(!range.overlaps(0, 10)); // Just before (exclusive)
        assert!(!range.overlaps(20, 30)); // Just after (exclusive)
        assert!(!range.overlaps(0, 5)); // Well before
        assert!(!range.overlaps(25, 30)); // Well after
    }

    #[test]
    fn test_virtualized_viewport_visible_indices() {
        let mut viewport = VirtualizedViewport::with_buffer(0); // No buffer for clearer testing

        // Add content: 0-5, 5-15, 15-18, 18-25
        viewport.add_content(0, 5);
        viewport.add_content(1, 10);
        viewport.add_content(2, 3);
        viewport.add_content(3, 7);

        // Lines 0-10 visible -> should include indices 0 and 1
        let visible = viewport.visible_indices(0, 10);
        assert!(visible.contains(&0));
        assert!(visible.contains(&1));
        assert!(!visible.contains(&2));
        assert!(!visible.contains(&3));

        // Lines 10-20 visible -> should include indices 1 and 2
        let visible = viewport.visible_indices(10, 20);
        assert!(!visible.contains(&0));
        assert!(visible.contains(&1));
        assert!(visible.contains(&2));
        assert!(visible.contains(&3));
    }

    #[test]
    fn test_virtualized_viewport_with_buffer() {
        let mut viewport = VirtualizedViewport::with_buffer(5);

        viewport.add_content(0, 10); // 0-10
        viewport.add_content(1, 10); // 10-20
        viewport.add_content(2, 10); // 20-30

        // Lines 12-18 visible, buffer extends to 7-23
        let visible = viewport.visible_indices(12, 18);
        assert!(visible.contains(&0)); // 0-10 overlaps with 7
        assert!(visible.contains(&1)); // 10-20 fully visible
        assert!(visible.contains(&2)); // 20-30 overlaps with 23
    }

    #[test]
    fn test_virtualized_viewport_range_for() {
        let mut viewport = VirtualizedViewport::new();
        viewport.add_content(0, 5);
        viewport.add_content(1, 10);

        let range = viewport.range_for(0).unwrap();
        assert_eq!(range.start_line, 0);
        assert_eq!(range.end_line, 5);

        let range = viewport.range_for(1).unwrap();
        assert_eq!(range.start_line, 5);
        assert_eq!(range.end_line, 15);

        assert!(viewport.range_for(2).is_none());
    }

    #[test]
    fn test_virtualized_viewport_is_visible() {
        let mut viewport = VirtualizedViewport::with_buffer(0);
        viewport.add_content(0, 10); // 0-10
        viewport.add_content(1, 10); // 10-20
        viewport.add_content(2, 10); // 20-30

        assert!(viewport.is_visible(0, 5, 15)); // 0-10 overlaps 5-15
        assert!(viewport.is_visible(1, 5, 15)); // 10-20 overlaps 5-15
        assert!(!viewport.is_visible(2, 5, 15)); // 20-30 doesn't overlap 5-15
    }

    #[test]
    fn test_virtualized_viewport_visible_line_range() {
        let mut viewport = VirtualizedViewport::new();
        viewport.add_content(0, 100); // 100 total lines

        // At bottom (offset 0), viewport 20 lines
        let (start, end) = viewport.visible_line_range(0, 20);
        assert_eq!(start, 80);
        assert_eq!(end, 100);

        // Scrolled up 30 lines
        let (start, end) = viewport.visible_line_range(30, 20);
        assert_eq!(start, 50);
        assert_eq!(end, 70);

        // At top (offset = total - viewport = 80)
        let (start, end) = viewport.visible_line_range(80, 20);
        assert_eq!(start, 0);
        assert_eq!(end, 20);
    }

    #[test]
    fn test_virtualized_viewport_clear() {
        let mut viewport = VirtualizedViewport::new();
        viewport.add_content(0, 10);
        viewport.add_content(1, 10);

        assert_eq!(viewport.total_lines(), 20);
        assert_eq!(viewport.content_count(), 2);

        viewport.clear();

        assert_eq!(viewport.total_lines(), 0);
        assert_eq!(viewport.content_count(), 0);
    }

    #[test]
    fn test_virtualized_viewport_visible_ranges() {
        let mut viewport = VirtualizedViewport::with_buffer(0);
        viewport.add_content(0, 10);
        viewport.add_content(1, 10);
        viewport.add_content(2, 10);

        let ranges = viewport.visible_ranges(5, 15);
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0].index, 0);
        assert_eq!(ranges[1].index, 1);
    }
}
