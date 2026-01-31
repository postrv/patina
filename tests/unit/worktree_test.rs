//! Unit tests for git worktree integration.
//!
//! These tests verify the `WorktreeManager` correctly detects git repositories
//! and manages worktrees for parallel development workflows.

use patina::worktree::{WorktreeConfig, WorktreeError, WorktreeInfo, WorktreeManager};
use std::path::PathBuf;
use tempfile::TempDir;

// ============================================================================
// WorktreeConfig Tests
// ============================================================================

#[test]
fn test_worktree_config_defaults() {
    let config = WorktreeConfig::default();

    // Default worktree directory should be .worktrees relative to repo root
    assert_eq!(config.worktree_dir, ".worktrees");

    // Default branch prefix should be "wt/"
    assert_eq!(config.branch_prefix, "wt/");

    // Auto-cleanup should be disabled by default
    assert!(!config.auto_cleanup);
}

#[test]
fn test_worktree_config_custom_values() {
    let config = WorktreeConfig {
        worktree_dir: "custom-trees".to_string(),
        branch_prefix: "feature/".to_string(),
        auto_cleanup: true,
    };

    assert_eq!(config.worktree_dir, "custom-trees");
    assert_eq!(config.branch_prefix, "feature/");
    assert!(config.auto_cleanup);
}

// ============================================================================
// WorktreeManager Detection Tests
// ============================================================================

#[test]
fn test_worktree_manager_detects_git_repo() {
    // Create a temp directory with a git repo
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path().to_path_buf();

    // Initialize a git repo
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to init git repo");

    let manager = WorktreeManager::new(repo_path.clone()).unwrap();
    assert!(manager.is_git_repo());
    // Canonicalize both paths for comparison (handles macOS /var -> /private/var symlink)
    let expected = repo_path.canonicalize().unwrap();
    let actual = manager.repo_root().canonicalize().unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn test_worktree_manager_rejects_non_git_directory() {
    // Create a temp directory without a git repo
    let temp_dir = TempDir::new().unwrap();
    let non_repo_path = temp_dir.path().to_path_buf();

    let result = WorktreeManager::new(non_repo_path);
    assert!(result.is_err());

    if let Err(WorktreeError::NotAGitRepository(path)) = result {
        assert!(path.exists());
    } else {
        panic!("Expected NotAGitRepository error");
    }
}

#[test]
fn test_worktree_manager_detects_nested_git_directory() {
    // Create a temp directory with a nested structure
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path().to_path_buf();

    // Initialize a git repo
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to init git repo");

    // Create a nested directory
    let nested_path = repo_path.join("src").join("deep");
    std::fs::create_dir_all(&nested_path).unwrap();

    // Manager should find the repo root from nested path
    let manager = WorktreeManager::new(nested_path).unwrap();
    assert!(manager.is_git_repo());
    // Canonicalize both paths for comparison (handles macOS /var -> /private/var symlink)
    let expected = repo_path.canonicalize().unwrap();
    let actual = manager.repo_root().canonicalize().unwrap();
    assert_eq!(actual, expected);
}

// ============================================================================
// WorktreeInfo Tests
// ============================================================================

#[test]
fn test_worktree_info_creation() {
    let info = WorktreeInfo {
        name: "feature-test".to_string(),
        path: PathBuf::from("/repo/.worktrees/feature-test"),
        branch: "wt/feature-test".to_string(),
        is_main: false,
        is_locked: false,
        is_prunable: false,
    };

    assert_eq!(info.name, "feature-test");
    assert!(!info.is_main);
    assert!(!info.is_locked);
}

#[test]
fn test_worktree_info_main_worktree() {
    let info = WorktreeInfo {
        name: "main".to_string(),
        path: PathBuf::from("/repo"),
        branch: "main".to_string(),
        is_main: true,
        is_locked: false,
        is_prunable: false,
    };

    assert!(info.is_main);
    assert_eq!(info.branch, "main");
}

// ============================================================================
// WorktreeError Tests
// ============================================================================

#[test]
fn test_worktree_error_display() {
    let error = WorktreeError::NotAGitRepository(PathBuf::from("/some/path"));
    let display = format!("{}", error);
    assert!(display.contains("/some/path"));

    let error = WorktreeError::WorktreeExists("feature-x".to_string());
    let display = format!("{}", error);
    assert!(display.contains("feature-x"));

    let error = WorktreeError::WorktreeNotFound("missing".to_string());
    let display = format!("{}", error);
    assert!(display.contains("missing"));
}

// ============================================================================
// Worktree CRUD Operations Tests
// ============================================================================

/// Helper to create a git repo with an initial commit.
fn create_git_repo_with_commit(path: &std::path::Path) {
    // Initialize git repo
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(path)
        .output()
        .expect("Failed to init git repo");

    // Configure git user for commits
    std::process::Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(path)
        .output()
        .expect("Failed to set git email");

    std::process::Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(path)
        .output()
        .expect("Failed to set git user");

    // Create a file and commit
    std::fs::write(path.join("README.md"), "# Test Repo").unwrap();

    std::process::Command::new("git")
        .args(["add", "README.md"])
        .current_dir(path)
        .output()
        .expect("Failed to add file");

    std::process::Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(path)
        .output()
        .expect("Failed to commit");
}

#[test]
fn test_list_worktrees_shows_main() {
    // A fresh git repo should list the main worktree
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path().to_path_buf();
    create_git_repo_with_commit(&repo_path);

    let manager = WorktreeManager::new(repo_path).unwrap();
    let worktrees = manager.list().unwrap();

    // Should have at least the main worktree
    assert!(
        !worktrees.is_empty(),
        "Should list at least the main worktree"
    );

    // Find the main worktree
    let main_worktree = worktrees.iter().find(|w| w.is_main);
    assert!(main_worktree.is_some(), "Should have a main worktree");
}

#[test]
fn test_create_worktree() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path().to_path_buf();
    create_git_repo_with_commit(&repo_path);

    let manager = WorktreeManager::new(repo_path.clone()).unwrap();

    // Create a new worktree
    let info = manager.create("feature-test").unwrap();

    assert_eq!(info.name, "feature-test");
    assert!(!info.is_main);
    assert!(info.path.exists(), "Worktree directory should exist");

    // The branch should have the configured prefix
    let config = manager.config();
    assert!(
        info.branch.starts_with(&config.branch_prefix),
        "Branch '{}' should start with prefix '{}'",
        info.branch,
        config.branch_prefix
    );

    // Verify the worktree was actually created by listing
    let worktrees = manager.list().unwrap();
    let created = worktrees.iter().find(|w| w.name == "feature-test");
    assert!(created.is_some(), "Created worktree should appear in list");
}

#[test]
fn test_create_worktree_already_exists() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path().to_path_buf();
    create_git_repo_with_commit(&repo_path);

    let manager = WorktreeManager::new(repo_path).unwrap();

    // Create first worktree
    manager.create("duplicate").unwrap();

    // Try to create another with the same name - should fail
    let result = manager.create("duplicate");
    assert!(
        matches!(result, Err(WorktreeError::WorktreeExists(_))),
        "Should return WorktreeExists error"
    );
}

#[test]
fn test_list_worktrees() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path().to_path_buf();
    create_git_repo_with_commit(&repo_path);

    let manager = WorktreeManager::new(repo_path).unwrap();

    // Create multiple worktrees
    manager.create("feature-a").unwrap();
    manager.create("feature-b").unwrap();

    let worktrees = manager.list().unwrap();

    // Should have main + 2 created = 3 worktrees
    assert_eq!(worktrees.len(), 3, "Should have 3 worktrees (main + 2)");

    // Verify all expected worktrees exist
    let names: Vec<&str> = worktrees.iter().map(|w| w.name.as_str()).collect();
    assert!(
        names.contains(&"feature-a"),
        "Should contain feature-a worktree"
    );
    assert!(
        names.contains(&"feature-b"),
        "Should contain feature-b worktree"
    );
}

#[test]
fn test_remove_worktree() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path().to_path_buf();
    create_git_repo_with_commit(&repo_path);

    let manager = WorktreeManager::new(repo_path).unwrap();

    // Create a worktree
    let info = manager.create("to-remove").unwrap();
    let worktree_path = info.path.clone();

    // Verify it exists
    assert!(worktree_path.exists());

    // Remove it
    manager.remove("to-remove").unwrap();

    // Verify it's gone from the list
    let worktrees = manager.list().unwrap();
    let found = worktrees.iter().find(|w| w.name == "to-remove");
    assert!(
        found.is_none(),
        "Removed worktree should not appear in list"
    );
}

#[test]
fn test_remove_worktree_not_found() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path().to_path_buf();
    create_git_repo_with_commit(&repo_path);

    let manager = WorktreeManager::new(repo_path).unwrap();

    // Try to remove a non-existent worktree
    let result = manager.remove("nonexistent");
    assert!(
        matches!(result, Err(WorktreeError::WorktreeNotFound(_))),
        "Should return WorktreeNotFound error"
    );
}
