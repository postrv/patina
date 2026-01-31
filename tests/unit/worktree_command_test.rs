//! Tests for /worktree slash command parsing.
//!
//! These tests define the expected parsing behavior for worktree commands
//! before implementation (TDD RED phase).

use patina::commands::worktree::{parse_worktree_command, WorktreeCommand};

/// Tests that `/worktree new <name>` parses correctly.
#[test]
fn test_parse_worktree_new() {
    let result = parse_worktree_command("new feature-branch");
    assert!(result.is_ok(), "Should parse 'new' subcommand");

    let cmd = result.unwrap();
    assert!(
        matches!(cmd, WorktreeCommand::New { name } if name == "feature-branch"),
        "Should parse name argument"
    );
}

/// Tests that `/worktree new` without a name returns an error.
#[test]
fn test_parse_worktree_new_requires_name() {
    let result = parse_worktree_command("new");
    assert!(result.is_err(), "Should require name argument");
}

/// Tests that `/worktree list` parses correctly.
#[test]
fn test_parse_worktree_list() {
    let result = parse_worktree_command("list");
    assert!(result.is_ok(), "Should parse 'list' subcommand");

    let cmd = result.unwrap();
    assert!(
        matches!(cmd, WorktreeCommand::List),
        "Should return List variant"
    );
}

/// Tests that `/worktree switch <name>` parses correctly.
#[test]
fn test_parse_worktree_switch() {
    let result = parse_worktree_command("switch my-worktree");
    assert!(result.is_ok(), "Should parse 'switch' subcommand");

    let cmd = result.unwrap();
    assert!(
        matches!(cmd, WorktreeCommand::Switch { name } if name == "my-worktree"),
        "Should parse name argument"
    );
}

/// Tests that `/worktree switch` without a name returns an error.
#[test]
fn test_parse_worktree_switch_requires_name() {
    let result = parse_worktree_command("switch");
    assert!(result.is_err(), "Should require name argument");
}

/// Tests that `/worktree remove <name>` parses correctly.
#[test]
fn test_parse_worktree_remove() {
    let result = parse_worktree_command("remove old-worktree");
    assert!(result.is_ok(), "Should parse 'remove' subcommand");

    let cmd = result.unwrap();
    assert!(
        matches!(cmd, WorktreeCommand::Remove { name } if name == "old-worktree"),
        "Should parse name argument"
    );
}

/// Tests that `/worktree remove` without a name returns an error.
#[test]
fn test_parse_worktree_remove_requires_name() {
    let result = parse_worktree_command("remove");
    assert!(result.is_err(), "Should require name argument");
}

/// Tests that `/worktree clean` parses correctly.
#[test]
fn test_parse_worktree_clean() {
    let result = parse_worktree_command("clean");
    assert!(result.is_ok(), "Should parse 'clean' subcommand");

    let cmd = result.unwrap();
    assert!(
        matches!(cmd, WorktreeCommand::Clean),
        "Should return Clean variant"
    );
}

/// Tests that `/worktree status` parses correctly.
#[test]
fn test_parse_worktree_status() {
    let result = parse_worktree_command("status");
    assert!(result.is_ok(), "Should parse 'status' subcommand");

    let cmd = result.unwrap();
    assert!(
        matches!(cmd, WorktreeCommand::Status),
        "Should return Status variant"
    );
}

/// Tests that unknown subcommands return an error.
#[test]
fn test_parse_worktree_unknown_subcommand() {
    let result = parse_worktree_command("unknown");
    assert!(result.is_err(), "Should reject unknown subcommand");
}

/// Tests that empty input returns an error.
#[test]
fn test_parse_worktree_empty() {
    let result = parse_worktree_command("");
    assert!(result.is_err(), "Should reject empty input");
}

/// Tests that whitespace-only input returns an error.
#[test]
fn test_parse_worktree_whitespace() {
    let result = parse_worktree_command("   ");
    assert!(result.is_err(), "Should reject whitespace-only input");
}

/// Tests that names with special characters are accepted.
#[test]
fn test_parse_worktree_name_with_special_chars() {
    let result = parse_worktree_command("new feature/my-branch_v2");
    assert!(
        result.is_ok(),
        "Should accept names with slashes, hyphens, underscores"
    );

    let cmd = result.unwrap();
    assert!(
        matches!(cmd, WorktreeCommand::New { name } if name == "feature/my-branch_v2"),
        "Should preserve special characters in name"
    );
}

/// Tests that extra whitespace is handled correctly.
#[test]
fn test_parse_worktree_extra_whitespace() {
    let result = parse_worktree_command("  new   my-worktree  ");
    assert!(result.is_ok(), "Should handle extra whitespace");

    let cmd = result.unwrap();
    assert!(
        matches!(cmd, WorktreeCommand::New { name } if name == "my-worktree"),
        "Should trim whitespace from arguments"
    );
}
