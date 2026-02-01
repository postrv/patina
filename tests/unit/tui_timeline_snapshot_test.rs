//! Snapshot tests for timeline-based TUI rendering.
//!
//! Phase 4 (RED/GREEN): These tests verify that:
//! - Timeline entries render in correct order
//! - Tool blocks appear inline with their associated messages
//! - Streaming appears at the end

use patina::app::state::AppState;
use patina::tui::render_timeline_to_lines;
use patina::types::config::ParallelMode;
use patina::types::{ConversationEntry, Role};
use ratatui::text::Line;
use std::path::PathBuf;

/// Helper to convert rendered lines to a string for snapshot comparison.
fn lines_to_string(lines: &[Line]) -> String {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<Vec<_>>()
                .join("")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ============================================================================
// Timeline Ordering Tests
// ============================================================================

/// Tests that timeline entries maintain correct order during iteration.
#[test]
fn test_timeline_iteration_order() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true, ParallelMode::Enabled);

    // Add entries in a specific order
    state.add_message(patina::Message {
        role: Role::User,
        content: "Hello".to_string(),
    });
    state.add_message(patina::Message {
        role: Role::Assistant,
        content: "Hi there!".to_string(),
    });
    state.add_tool_block_with_result("bash", "ls", "file.txt", false);
    state.add_message(patina::Message {
        role: Role::User,
        content: "Thanks".to_string(),
    });

    // Verify order
    let entries: Vec<_> = state.timeline().iter().collect();
    assert_eq!(entries.len(), 4);
    assert!(entries[0].is_user());
    assert!(entries[1].is_assistant());
    assert!(entries[2].is_tool_execution());
    assert!(entries[3].is_user());
}

/// Tests that tool blocks track their associated message index.
#[test]
fn test_tool_block_follows_correct_message() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true, ParallelMode::Enabled);

    // User 0
    state.add_message(patina::Message {
        role: Role::User,
        content: "Run ls".to_string(),
    });

    // Assistant 1
    state.add_message(patina::Message {
        role: Role::Assistant,
        content: "Running command...".to_string(),
    });

    // Tool follows assistant at index 1
    state.add_tool_block_with_result("bash", "ls -la", "output", false);

    let entries: Vec<_> = state.timeline().iter().collect();

    // Tool block should track that it follows message index 1
    if let ConversationEntry::ToolExecution {
        follows_message_idx,
        ..
    } = &entries[2]
    {
        assert_eq!(
            *follows_message_idx,
            Some(1),
            "Tool should follow assistant at index 1"
        );
    } else {
        panic!("Expected tool execution at index 2");
    }
}

/// Tests timeline with streaming entry at the end.
#[test]
fn test_timeline_streaming_at_end() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true, ParallelMode::Enabled);

    state.add_message(patina::Message {
        role: Role::User,
        content: "Hello".to_string(),
    });

    // Start streaming
    state.set_streaming(true);
    state.append_streaming_text("Responding...");

    // Streaming should be the last entry
    let entries: Vec<_> = state.timeline().iter().collect();
    assert_eq!(entries.len(), 2);
    assert!(entries[0].is_user());
    assert!(entries[1].is_streaming());
}

/// Tests that timeline correctly orders multiple tools after same assistant message.
#[test]
fn test_multiple_tools_same_message() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true, ParallelMode::Enabled);

    state.add_message(patina::Message {
        role: Role::Assistant,
        content: "I'll run multiple commands.".to_string(),
    });

    state.add_tool_block_with_result("bash", "ls", "files", false);
    state.add_tool_block_with_result("bash", "pwd", "/home", false);
    state.add_tool_block_with_result("bash", "whoami", "user", false);

    let entries: Vec<_> = state.timeline().iter().collect();
    assert_eq!(entries.len(), 4);

    // All tools should follow the same assistant message (index 0)
    for entry in entries.iter().skip(1) {
        if let ConversationEntry::ToolExecution {
            follows_message_idx,
            ..
        } = entry
        {
            assert_eq!(*follows_message_idx, Some(0));
        }
    }
}

// ============================================================================
// Rendering Content Tests
// ============================================================================

/// Tests that timeline produces expected content for rendering.
#[test]
fn test_timeline_produces_render_content() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true, ParallelMode::Enabled);

    state.add_message(patina::Message {
        role: Role::User,
        content: "What files are here?".to_string(),
    });

    state.add_message(patina::Message {
        role: Role::Assistant,
        content: "Let me check.".to_string(),
    });

    state.add_tool_block_with_result("bash", "ls", "README.md\nCargo.toml", false);

    // Verify all content is in timeline
    let mut has_user = false;
    let mut has_assistant = false;
    let mut has_tool = false;

    for entry in state.timeline().iter() {
        match entry {
            ConversationEntry::UserMessage(text) => {
                assert_eq!(text, "What files are here?");
                has_user = true;
            }
            ConversationEntry::AssistantMessage(text) => {
                assert_eq!(text, "Let me check.");
                has_assistant = true;
            }
            ConversationEntry::ToolExecution {
                name,
                input,
                output,
                ..
            } => {
                assert_eq!(name, "bash");
                assert_eq!(input, "ls");
                assert_eq!(output.as_deref(), Some("README.md\nCargo.toml"));
                has_tool = true;
            }
            ConversationEntry::Streaming { .. } => {}
            ConversationEntry::ImageDisplay { .. } => {}
        }
    }

    assert!(has_user, "Should have user message");
    assert!(has_assistant, "Should have assistant message");
    assert!(has_tool, "Should have tool execution");
}

// ============================================================================
// Edge Cases
// ============================================================================

/// Tests empty timeline.
#[test]
fn test_empty_timeline() {
    let state = AppState::new(PathBuf::from("/tmp"), true, ParallelMode::Enabled);
    assert!(state.timeline().is_empty());
    assert_eq!(state.timeline().len(), 0);
}

/// Tests timeline with only streaming (no complete messages).
#[test]
fn test_timeline_only_streaming() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true, ParallelMode::Enabled);

    state.set_streaming(true);
    state.append_streaming_text("Starting...");

    let entries: Vec<_> = state.timeline().iter().collect();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].is_streaming());
}

/// Tests that empty assistant messages don't render "Patina:" label.
///
/// When Claude sends only tool_use blocks (no text), the assistant message
/// is empty and should not be rendered to avoid spurious "Patina:" lines.
#[test]
fn test_empty_assistant_message_not_rendered() {
    use patina::types::Timeline;

    let mut timeline = Timeline::new();

    // Add a user message
    timeline.push_user_message("Run some commands");

    // Add an empty assistant message (simulates tool_use only response)
    timeline.push_assistant_message("");

    // Add a tool execution using push_tool_execution
    timeline.push_tool_execution("bash", "ls", Some("file1.txt".to_string()), false);

    // Render the timeline
    let lines = render_timeline_to_lines(&timeline, 80);
    let output = lines_to_string(&lines);

    // Should NOT contain "Patina:" followed immediately by nothing useful
    // The output should have "You:" for user message and tool block, but no empty "Patina:"
    assert!(
        output.contains("You:"),
        "Output should contain user message: {}",
        output
    );

    // Count occurrences of "Patina:"
    let patina_count = output.matches("Patina:").count();
    assert_eq!(
        patina_count, 0,
        "Empty assistant message should not render 'Patina:' label, but found {} occurrences in:\n{}",
        patina_count, output
    );
}

/// Tests that tool error status is preserved.
#[test]
fn test_tool_error_status() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true, ParallelMode::Enabled);

    state.add_message(patina::Message {
        role: Role::Assistant,
        content: "Running...".to_string(),
    });

    state.add_tool_block_with_result("bash", "bad_cmd", "command not found", true);

    let entries: Vec<_> = state.timeline().iter().collect();

    if let ConversationEntry::ToolExecution { is_error, .. } = &entries[1] {
        assert!(*is_error, "Tool should be marked as error");
    } else {
        panic!("Expected tool execution");
    }
}

// ============================================================================
// Timeline Rendering Snapshot Tests (Phase 4.1.1)
// ============================================================================

/// Tests that timeline renders tool blocks inline with messages.
///
/// Expected output order:
/// - User message
/// - Assistant message
/// - Tool block (immediately after assistant)
/// - NOT: tool blocks gathered at the end
#[test]
fn test_timeline_renders_tool_blocks_inline() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true, ParallelMode::Enabled);

    state.add_message(patina::Message {
        role: Role::User,
        content: "List files".to_string(),
    });

    state.add_message(patina::Message {
        role: Role::Assistant,
        content: "I'll check for you.".to_string(),
    });

    state.add_tool_block_with_result("bash", "ls -la", "README.md\nCargo.toml", false);

    let lines = render_timeline_to_lines(state.timeline(), 80);
    let output = lines_to_string(&lines);

    insta::assert_snapshot!(output);
}

/// Tests timeline rendering with multiple interleaved tool blocks.
///
/// Scenario: User asks two questions, each answered with a tool call.
/// Tool blocks should appear immediately after their producing assistant message.
#[test]
fn test_timeline_renders_interleaved_tools() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true, ParallelMode::Enabled);

    // First Q&A with tool
    state.add_message(patina::Message {
        role: Role::User,
        content: "What directory am I in?".to_string(),
    });
    state.add_message(patina::Message {
        role: Role::Assistant,
        content: "Let me check.".to_string(),
    });
    state.add_tool_block_with_result("bash", "pwd", "/home/user/project", false);

    // Second Q&A with tool
    state.add_message(patina::Message {
        role: Role::User,
        content: "What files are here?".to_string(),
    });
    state.add_message(patina::Message {
        role: Role::Assistant,
        content: "I'll list them.".to_string(),
    });
    state.add_tool_block_with_result("bash", "ls", "src/\ntests/\nCargo.toml", false);

    let lines = render_timeline_to_lines(state.timeline(), 80);
    let output = lines_to_string(&lines);

    insta::assert_snapshot!(output);
}

/// Tests that streaming always appears at the end of timeline.
#[test]
fn test_timeline_renders_streaming_at_end() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true, ParallelMode::Enabled);

    state.add_message(patina::Message {
        role: Role::User,
        content: "Tell me about Rust".to_string(),
    });

    // Start streaming
    state.set_streaming(true);
    state.append_streaming_text("Rust is a systems programming language...");

    let lines = render_timeline_to_lines(state.timeline(), 80);
    let output = lines_to_string(&lines);

    insta::assert_snapshot!(output);
}

/// Tests timeline rendering with tool errors.
#[test]
fn test_timeline_renders_tool_error() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true, ParallelMode::Enabled);

    state.add_message(patina::Message {
        role: Role::Assistant,
        content: "Running command...".to_string(),
    });

    state.add_tool_block_with_result("bash", "nonexistent_cmd", "command not found", true);

    let lines = render_timeline_to_lines(state.timeline(), 80);
    let output = lines_to_string(&lines);

    insta::assert_snapshot!(output);
}

/// Tests timeline rendering with multiple tools after same message.
#[test]
fn test_timeline_renders_multiple_tools_same_message() {
    let mut state = AppState::new(PathBuf::from("/tmp"), true, ParallelMode::Enabled);

    state.add_message(patina::Message {
        role: Role::Assistant,
        content: "I'll run several commands.".to_string(),
    });

    state.add_tool_block_with_result("bash", "ls", "file1.txt\nfile2.txt", false);
    state.add_tool_block_with_result("bash", "pwd", "/home/user", false);
    state.add_tool_block_with_result("bash", "whoami", "user", false);

    let lines = render_timeline_to_lines(state.timeline(), 80);
    let output = lines_to_string(&lines);

    insta::assert_snapshot!(output);
}
