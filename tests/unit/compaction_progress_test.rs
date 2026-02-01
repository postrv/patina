//! Unit tests for the compaction progress widget.
//!
//! The compaction progress widget displays the status of context compaction,
//! including progress bar, token counts, and before/after comparison.
//!
//! These tests define the expected behavior for `CompactionProgressWidget`.

use patina::tui::widgets::compaction_progress::{
    CompactionProgressState, CompactionProgressWidget, CompactionStatus,
};
use ratatui::{backend::TestBackend, Terminal};

// ============================================================================
// Helper Functions
// ============================================================================

/// Creates a test terminal with the given dimensions.
fn test_terminal(width: u16, height: u16) -> Terminal<TestBackend> {
    let backend = TestBackend::new(width, height);
    Terminal::new(backend).expect("Failed to create test terminal")
}

/// Extracts the rendered content as a string from the terminal buffer.
fn buffer_to_string(terminal: &Terminal<TestBackend>) -> String {
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol().chars().next().unwrap_or(' '))
        .collect()
}

// ============================================================================
// CompactionProgressState Tests
// ============================================================================

/// Tests that CompactionProgressState can be created with initial values.
#[test]
fn test_compaction_progress_state_new() {
    let state = CompactionProgressState::new(10_000, 50_000);

    assert_eq!(state.before_tokens(), 50_000);
    assert_eq!(state.target_tokens(), 10_000);
    assert_eq!(state.status(), CompactionStatus::Idle);
}

/// Tests that progress can be updated.
#[test]
fn test_compaction_progress_state_update_progress() {
    let mut state = CompactionProgressState::new(10_000, 50_000);
    state.set_progress(0.5);

    assert!((state.progress() - 0.5).abs() < 0.001);
}

/// Tests that status can be changed.
#[test]
fn test_compaction_progress_state_set_status() {
    let mut state = CompactionProgressState::new(10_000, 50_000);
    state.set_status(CompactionStatus::Compacting);

    assert_eq!(state.status(), CompactionStatus::Compacting);
}

/// Tests that after_tokens can be set when compaction completes.
#[test]
fn test_compaction_progress_state_set_after_tokens() {
    let mut state = CompactionProgressState::new(10_000, 50_000);
    state.set_after_tokens(15_000);

    assert_eq!(state.after_tokens(), Some(15_000));
}

/// Tests that saved tokens are calculated correctly.
#[test]
fn test_compaction_progress_state_saved_tokens() {
    let mut state = CompactionProgressState::new(10_000, 50_000);
    state.set_after_tokens(15_000);

    assert_eq!(state.saved_tokens(), Some(35_000));
}

// ============================================================================
// CompactionStatus Tests
// ============================================================================

/// Tests that CompactionStatus has the expected variants.
#[test]
fn test_compaction_status_variants() {
    let idle = CompactionStatus::Idle;
    let compacting = CompactionStatus::Compacting;
    let complete = CompactionStatus::Complete;
    let failed = CompactionStatus::Failed;

    // Verify all variants exist
    assert_eq!(idle, CompactionStatus::Idle);
    assert_eq!(compacting, CompactionStatus::Compacting);
    assert_eq!(complete, CompactionStatus::Complete);
    assert_eq!(failed, CompactionStatus::Failed);
}

/// Tests that CompactionStatus implements Display.
#[test]
fn test_compaction_status_display() {
    assert_eq!(format!("{}", CompactionStatus::Idle), "Idle");
    assert_eq!(format!("{}", CompactionStatus::Compacting), "Compacting...");
    assert_eq!(format!("{}", CompactionStatus::Complete), "Complete");
    assert_eq!(format!("{}", CompactionStatus::Failed), "Failed");
}

// ============================================================================
// Progress Widget Rendering Tests
// ============================================================================

/// Tests that the widget renders a progress bar.
///
/// The progress bar should visually indicate how far along compaction is,
/// using a visual representation like [=====>     ] or similar.
#[test]
fn test_progress_widget_renders_progress_bar() {
    let mut terminal = test_terminal(60, 10);

    let mut state = CompactionProgressState::new(10_000, 50_000);
    state.set_status(CompactionStatus::Compacting);
    state.set_progress(0.5); // 50% complete

    let widget = CompactionProgressWidget::new(&state);

    terminal
        .draw(|frame| {
            frame.render_widget(widget, frame.area());
        })
        .expect("Failed to draw");

    let content = buffer_to_string(&terminal);

    // Should show some progress indicator
    // Common progress bar characters: [, ], =, -, >, █, ░, ▓
    let has_progress_indicator = content.contains('[')
        || content.contains('█')
        || content.contains('=')
        || content.contains('▓')
        || content.contains('%')
        || content.to_lowercase().contains("progress");

    assert!(
        has_progress_indicator,
        "Should display a progress bar or progress indicator. Content: {}",
        content.trim()
    );

    // Should indicate partial completion (50%)
    assert!(
        content.contains("50") || content.contains("50%"),
        "Should show 50% progress. Content: {}",
        content.trim()
    );
}

/// Tests that the widget shows token counts.
///
/// The widget should display the before tokens, target tokens, and
/// optionally the current/after tokens.
#[test]
fn test_progress_widget_shows_token_counts() {
    let mut terminal = test_terminal(80, 10);

    let state = CompactionProgressState::new(10_000, 50_000);
    let widget = CompactionProgressWidget::new(&state);

    terminal
        .draw(|frame| {
            frame.render_widget(widget, frame.area());
        })
        .expect("Failed to draw");

    let content = buffer_to_string(&terminal);

    // Should show token counts (with or without commas/formatting)
    let shows_before = content.contains("50000")
        || content.contains("50,000")
        || content.contains("50k")
        || content.contains("50K");

    let shows_target = content.contains("10000")
        || content.contains("10,000")
        || content.contains("10k")
        || content.contains("10K");

    assert!(
        shows_before,
        "Should display before tokens (50,000). Content: {}",
        content.trim()
    );

    assert!(
        shows_target || content.to_lowercase().contains("target"),
        "Should display target tokens (10,000) or 'target'. Content: {}",
        content.trim()
    );
}

/// Tests that the widget shows before/after comparison.
///
/// When compaction is complete, the widget should show both the original
/// token count and the compacted token count for easy comparison.
#[test]
fn test_progress_widget_shows_before_after() {
    let mut terminal = test_terminal(80, 10);

    let mut state = CompactionProgressState::new(10_000, 50_000);
    state.set_status(CompactionStatus::Complete);
    state.set_after_tokens(15_000);
    state.set_progress(1.0);

    let widget = CompactionProgressWidget::new(&state);

    terminal
        .draw(|frame| {
            frame.render_widget(widget, frame.area());
        })
        .expect("Failed to draw");

    let content = buffer_to_string(&terminal);

    // Should show before tokens
    let shows_before = content.contains("50000")
        || content.contains("50,000")
        || content.contains("50k")
        || content.to_lowercase().contains("before");

    // Should show after tokens
    let shows_after = content.contains("15000")
        || content.contains("15,000")
        || content.contains("15k")
        || content.to_lowercase().contains("after");

    // Should show savings
    let shows_savings = content.contains("35000")
        || content.contains("35,000")
        || content.contains("35k")
        || content.contains("saved")
        || content.contains("70%"); // 35k saved from 50k = 70%

    assert!(
        shows_before,
        "Should display before tokens. Content: {}",
        content.trim()
    );

    assert!(
        shows_after,
        "Should display after tokens. Content: {}",
        content.trim()
    );

    // Either show absolute savings or percentage savings
    assert!(
        shows_savings || (shows_before && shows_after),
        "Should show savings or before/after comparison. Content: {}",
        content.trim()
    );
}

/// Tests that the widget shows completion status.
#[test]
fn test_progress_widget_shows_complete_status() {
    let mut terminal = test_terminal(60, 10);

    let mut state = CompactionProgressState::new(10_000, 50_000);
    state.set_status(CompactionStatus::Complete);
    state.set_after_tokens(15_000);
    state.set_progress(1.0);

    let widget = CompactionProgressWidget::new(&state);

    terminal
        .draw(|frame| {
            frame.render_widget(widget, frame.area());
        })
        .expect("Failed to draw");

    let content = buffer_to_string(&terminal);

    // Should show complete status
    let shows_complete = content.to_lowercase().contains("complete")
        || content.contains("100%")
        || content.contains("done")
        || content.contains('✓')
        || content.contains("✔");

    assert!(
        shows_complete,
        "Should indicate compaction is complete. Content: {}",
        content.trim()
    );
}

/// Tests that the widget shows error status.
#[test]
fn test_progress_widget_shows_failed_status() {
    let mut terminal = test_terminal(60, 10);

    let mut state = CompactionProgressState::new(10_000, 50_000);
    state.set_status(CompactionStatus::Failed);

    let widget = CompactionProgressWidget::new(&state);

    terminal
        .draw(|frame| {
            frame.render_widget(widget, frame.area());
        })
        .expect("Failed to draw");

    let content = buffer_to_string(&terminal);

    // Should show failed/error status
    let shows_failed = content.to_lowercase().contains("fail")
        || content.to_lowercase().contains("error")
        || content.contains('✗')
        || content.contains('✘')
        || content.contains('×');

    assert!(
        shows_failed,
        "Should indicate compaction failed. Content: {}",
        content.trim()
    );
}

// ============================================================================
// Edge Case Tests
// ============================================================================

/// Tests that the widget handles zero tokens gracefully.
#[test]
fn test_progress_widget_handles_zero_tokens() {
    let mut terminal = test_terminal(60, 10);

    let state = CompactionProgressState::new(0, 0);
    let widget = CompactionProgressWidget::new(&state);

    // Should not panic
    terminal
        .draw(|frame| {
            frame.render_widget(widget, frame.area());
        })
        .expect("Drawing with zero tokens should not panic");
}

/// Tests that the widget handles very large token counts.
#[test]
fn test_progress_widget_handles_large_tokens() {
    let mut terminal = test_terminal(80, 10);

    let state = CompactionProgressState::new(1_000_000, 10_000_000);
    let widget = CompactionProgressWidget::new(&state);

    // Should render without panic or overflow
    terminal
        .draw(|frame| {
            frame.render_widget(widget, frame.area());
        })
        .expect("Drawing with large tokens should not panic");

    let content = buffer_to_string(&terminal);

    // Should show some representation of the large numbers
    let has_numbers = content.chars().any(|c| c.is_ascii_digit())
        || content.contains("M")
        || content.contains("K");

    assert!(
        has_numbers,
        "Should display token counts. Content: {}",
        content
    );
}

/// Tests that the widget renders in small terminal sizes.
#[test]
fn test_progress_widget_renders_in_small_terminal() {
    let mut terminal = test_terminal(20, 3);

    let mut state = CompactionProgressState::new(10_000, 50_000);
    state.set_status(CompactionStatus::Compacting);
    state.set_progress(0.3);

    let widget = CompactionProgressWidget::new(&state);

    // Should not panic even in very constrained space
    terminal
        .draw(|frame| {
            frame.render_widget(widget, frame.area());
        })
        .expect("Drawing in small terminal should not panic");

    // Should produce some output
    let content = buffer_to_string(&terminal);
    assert!(!content.trim().is_empty(), "Should render something");
}

/// Tests that progress is clamped to valid range.
#[test]
fn test_progress_widget_clamps_progress() {
    let mut state = CompactionProgressState::new(10_000, 50_000);

    // Setting progress above 1.0 should clamp to 1.0
    state.set_progress(1.5);
    assert!((state.progress() - 1.0).abs() < 0.001);

    // Setting progress below 0.0 should clamp to 0.0
    state.set_progress(-0.5);
    assert!(state.progress().abs() < 0.001);
}
