//! Unit tests for the worktree picker widget.
//!
//! These tests verify the `WorktreePickerWidget` correctly displays
//! worktrees with status indicators and handles keybindings.

use patina::tui::widgets::worktree_picker::{WorktreePickerState, WorktreePickerWidget};
use patina::worktree::{WorktreeInfo, WorktreeStatus};
use ratatui::{backend::TestBackend, Terminal};
use std::path::PathBuf;

// ============================================================================
// WorktreePickerState Tests
// ============================================================================

#[test]
fn test_worktree_picker_state_creation() {
    let state = WorktreePickerState::default();

    // Default state should have no worktrees selected
    assert_eq!(state.selected_index(), 0);
    assert!(state.worktrees().is_empty());
}

#[test]
fn test_worktree_picker_state_with_worktrees() {
    let worktrees = vec![
        create_worktree_info("main", true),
        create_worktree_info("feature-a", false),
        create_worktree_info("feature-b", false),
    ];

    let state = WorktreePickerState::new(worktrees);

    assert_eq!(state.worktrees().len(), 3);
    assert_eq!(state.selected_index(), 0);
}

#[test]
fn test_worktree_picker_state_selection_navigation() {
    let worktrees = vec![
        create_worktree_info("main", true),
        create_worktree_info("feature-a", false),
        create_worktree_info("feature-b", false),
    ];

    let mut state = WorktreePickerState::new(worktrees);

    // Initially at 0
    assert_eq!(state.selected_index(), 0);

    // Move down
    state.select_next();
    assert_eq!(state.selected_index(), 1);

    state.select_next();
    assert_eq!(state.selected_index(), 2);

    // Should wrap or stop at end
    state.select_next();
    assert_eq!(state.selected_index(), 2); // Stops at end

    // Move up
    state.select_previous();
    assert_eq!(state.selected_index(), 1);

    state.select_previous();
    assert_eq!(state.selected_index(), 0);

    // Should stop at beginning
    state.select_previous();
    assert_eq!(state.selected_index(), 0);
}

#[test]
fn test_worktree_picker_state_selected_worktree() {
    let worktrees = vec![
        create_worktree_info("main", true),
        create_worktree_info("feature-a", false),
    ];

    let mut state = WorktreePickerState::new(worktrees);

    // Get selected worktree
    assert_eq!(
        state.selected_worktree().map(|w| w.name.as_str()),
        Some("main")
    );

    state.select_next();
    assert_eq!(
        state.selected_worktree().map(|w| w.name.as_str()),
        Some("feature-a")
    );
}

#[test]
fn test_worktree_picker_state_update_status() {
    let worktrees = vec![create_worktree_info("main", true)];

    let mut state = WorktreePickerState::new(worktrees);

    let new_status = WorktreeStatus {
        modified: 5,
        staged: 2,
        untracked: 3,
        ahead: 1,
        behind: 0,
    };

    state.update_status("main", new_status);

    let status = state.get_status("main");
    assert!(status.is_some());
    let status = status.unwrap();
    assert_eq!(status.modified, 5);
    assert_eq!(status.staged, 2);
}

// ============================================================================
// WorktreePickerWidget Rendering Tests
// ============================================================================

#[test]
fn test_worktree_picker_widget_renders_empty_state() {
    let state = WorktreePickerState::default();
    let widget = WorktreePickerWidget::new(&state);

    let output = render_widget(&widget, 60, 10);

    // Should show a message about no worktrees
    assert!(
        output.contains("No worktrees") || output.contains("empty"),
        "Empty state should indicate no worktrees"
    );
}

#[test]
fn test_worktree_picker_widget_renders_worktrees() {
    let worktrees = vec![
        create_worktree_info("main", true),
        create_worktree_info("feature-x", false),
    ];

    let state = WorktreePickerState::new(worktrees);
    let widget = WorktreePickerWidget::new(&state);

    let output = render_widget(&widget, 60, 10);

    // Should show worktree names
    assert!(output.contains("main"), "Should display 'main' worktree");
    assert!(
        output.contains("feature-x"),
        "Should display 'feature-x' worktree"
    );
}

#[test]
fn test_worktree_picker_widget_shows_selection() {
    let worktrees = vec![
        create_worktree_info("main", true),
        create_worktree_info("feature-x", false),
    ];

    let mut state = WorktreePickerState::new(worktrees);
    state.select_next(); // Select feature-x

    let widget = WorktreePickerWidget::new(&state);
    let output = render_widget(&widget, 60, 10);

    // Selected item should have visual indicator (e.g., ">" or highlight)
    // The exact indicator depends on implementation
    assert!(
        output.contains(">") || output.contains("►") || output.contains("→"),
        "Should show selection indicator"
    );
}

#[test]
fn test_worktree_picker_widget_shows_status_indicators() {
    let worktrees = vec![create_worktree_info("feature-dirty", false)];

    let mut state = WorktreePickerState::new(worktrees);

    // Set dirty status
    state.update_status(
        "feature-dirty",
        WorktreeStatus {
            modified: 3,
            staged: 1,
            untracked: 2,
            ahead: 0,
            behind: 0,
        },
    );

    let widget = WorktreePickerWidget::new(&state);
    let output = render_widget(&widget, 80, 10);

    // Should show status indicators
    // Could be symbols like ● for dirty, ↑ for ahead, etc.
    assert!(
        output.contains("●")
            || output.contains("*")
            || output.contains("!")
            || output.contains("M")
            || output.contains("3"),
        "Should show dirty status indicator"
    );
}

#[test]
fn test_worktree_picker_widget_shows_keybinding_hints() {
    let state = WorktreePickerState::default();
    let widget = WorktreePickerWidget::new(&state);

    let output = render_widget(&widget, 80, 15);

    // Should show keybinding hints
    assert!(
        output.contains("n") || output.contains("new"),
        "Should show 'n' for new"
    );
    assert!(
        output.contains("s") || output.contains("switch"),
        "Should show 's' for switch"
    );
    assert!(
        output.contains("d") || output.contains("delete"),
        "Should show 'd' for delete"
    );
    assert!(
        output.contains("c") || output.contains("clean"),
        "Should show 'c' for clean"
    );
}

#[test]
fn test_worktree_picker_widget_shows_main_indicator() {
    let worktrees = vec![
        create_worktree_info("main", true),
        create_worktree_info("feature-x", false),
    ];

    let state = WorktreePickerState::new(worktrees);
    let widget = WorktreePickerWidget::new(&state);

    let output = render_widget(&widget, 60, 10);

    // Main worktree should have special indicator
    assert!(
        output.contains("★") || output.contains("●") || output.contains("[main]"),
        "Should indicate which is the main worktree"
    );
}

#[test]
fn test_worktree_picker_widget_shows_ahead_behind() {
    let worktrees = vec![create_worktree_info("feature-ahead", false)];

    let mut state = WorktreePickerState::new(worktrees);
    state.update_status(
        "feature-ahead",
        WorktreeStatus {
            modified: 0,
            staged: 0,
            untracked: 0,
            ahead: 3,
            behind: 1,
        },
    );

    let widget = WorktreePickerWidget::new(&state);
    let output = render_widget(&widget, 80, 10);

    // Should show ahead/behind indicators
    assert!(
        output.contains("↑3")
            || output.contains("+3")
            || output.contains("3↑")
            || output.contains("ahead"),
        "Should show ahead count"
    );
    assert!(
        output.contains("↓1")
            || output.contains("-1")
            || output.contains("1↓")
            || output.contains("behind"),
        "Should show behind count"
    );
}

// ============================================================================
// Helper Functions
// ============================================================================

fn create_worktree_info(name: &str, is_main: bool) -> WorktreeInfo {
    WorktreeInfo {
        name: name.to_string(),
        path: PathBuf::from(format!("/repo/.worktrees/{}", name)),
        branch: if is_main {
            "main".to_string()
        } else {
            format!("wt/{}", name)
        },
        is_main,
        is_locked: false,
        is_prunable: false,
    }
}

fn render_widget(widget: &WorktreePickerWidget, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("Failed to create terminal");

    terminal
        .draw(|frame| {
            frame.render_widget(widget.clone(), frame.area());
        })
        .expect("Failed to draw");

    // Convert buffer to string
    let buffer = terminal.backend().buffer();
    let mut output = String::new();

    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, y)];
            output.push_str(cell.symbol());
        }
        output.push('\n');
    }

    output
}
