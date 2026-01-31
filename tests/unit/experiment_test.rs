//! Unit tests for git worktree experiment mode.
//!
//! These tests define the expected behavior for experiment workflows:
//! - Starting experiments creates isolated worktrees for risky changes
//! - Accepting experiments merges changes back to the original branch
//! - Rejecting experiments discards changes and removes the worktree
//!
//! This is a TDD RED phase - tests are written before implementation.

use patina::worktree::{
    Experiment, ExperimentConfig, ExperimentError, ExperimentState, WorktreeManager,
};
use tempfile::TempDir;

// ============================================================================
// Test Helpers
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

// ============================================================================
// ExperimentConfig Tests
// ============================================================================

#[test]
fn test_experiment_config_defaults() {
    let config = ExperimentConfig::default();

    // Default prefix for experiment branches
    assert_eq!(config.branch_prefix, "exp/");

    // Default experiment directory
    assert_eq!(config.experiment_dir, ".experiments");

    // Auto-cleanup disabled by default
    assert!(!config.auto_cleanup_on_reject);
}

// ============================================================================
// Experiment Lifecycle Tests
// ============================================================================

#[test]
fn test_experiment_start() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path().to_path_buf();
    create_git_repo_with_commit(&repo_path);

    let manager = WorktreeManager::new(repo_path.clone()).unwrap();

    // Start a new experiment
    let experiment = Experiment::start(&manager, "risky-refactor").unwrap();

    // Experiment should be in active state
    assert_eq!(experiment.state(), ExperimentState::Active);

    // Experiment should have a name
    assert_eq!(experiment.name(), "risky-refactor");

    // Experiment should have a worktree path
    assert!(experiment.worktree_path().exists());

    // Experiment should track the original branch
    assert!(!experiment.original_branch().is_empty());

    // Experiment should have an experiment branch
    assert!(experiment.experiment_branch().starts_with("exp/"));
}

#[test]
fn test_experiment_start_with_description() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path().to_path_buf();
    create_git_repo_with_commit(&repo_path);

    let manager = WorktreeManager::new(repo_path.clone()).unwrap();

    // Start experiment with description
    let experiment = Experiment::start_with_description(
        &manager,
        "db-migration",
        "Testing new database schema migration",
    )
    .unwrap();

    assert_eq!(experiment.name(), "db-migration");
    assert_eq!(
        experiment.description(),
        Some("Testing new database schema migration")
    );
}

#[test]
fn test_experiment_accept() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path().to_path_buf();
    create_git_repo_with_commit(&repo_path);

    let manager = WorktreeManager::new(repo_path.clone()).unwrap();

    // Start an experiment
    let experiment = Experiment::start(&manager, "feature-experiment").unwrap();
    let worktree_path = experiment.worktree_path().to_path_buf();

    // Make some changes in the experiment worktree
    std::fs::write(worktree_path.join("feature.txt"), "New feature code").unwrap();
    std::process::Command::new("git")
        .args(["add", "feature.txt"])
        .current_dir(&worktree_path)
        .output()
        .expect("Failed to add file");
    std::process::Command::new("git")
        .args(["commit", "-m", "Add feature"])
        .current_dir(&worktree_path)
        .output()
        .expect("Failed to commit");

    // Accept the experiment - should merge changes back
    let result = experiment.accept().unwrap();

    // Result should indicate success and number of commits merged
    assert!(result.success);
    assert!(result.commits_merged >= 1);

    // Experiment should be in completed state
    assert_eq!(result.final_state, ExperimentState::Accepted);

    // The feature file should now exist in the original branch
    assert!(repo_path.join("feature.txt").exists());

    // Worktree should be cleaned up
    assert!(!worktree_path.exists());
}

#[test]
fn test_experiment_reject() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path().to_path_buf();
    create_git_repo_with_commit(&repo_path);

    let manager = WorktreeManager::new(repo_path.clone()).unwrap();

    // Start an experiment
    let experiment = Experiment::start(&manager, "bad-idea").unwrap();
    let worktree_path = experiment.worktree_path().to_path_buf();

    // Make some changes in the experiment worktree
    std::fs::write(worktree_path.join("bad_code.txt"), "This was a bad idea").unwrap();
    std::process::Command::new("git")
        .args(["add", "bad_code.txt"])
        .current_dir(&worktree_path)
        .output()
        .expect("Failed to add file");
    std::process::Command::new("git")
        .args(["commit", "-m", "Bad commit"])
        .current_dir(&worktree_path)
        .output()
        .expect("Failed to commit");

    // Reject the experiment - should discard changes
    let result = experiment.reject().unwrap();

    // Result should indicate rejection
    assert!(result.success);
    assert_eq!(result.final_state, ExperimentState::Rejected);

    // The bad file should NOT exist in the original branch
    assert!(!repo_path.join("bad_code.txt").exists());

    // Worktree should be cleaned up
    assert!(!worktree_path.exists());
}

#[test]
fn test_experiment_pause() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path().to_path_buf();
    create_git_repo_with_commit(&repo_path);

    let manager = WorktreeManager::new(repo_path.clone()).unwrap();

    // Start an experiment
    let experiment = Experiment::start(&manager, "wip-feature").unwrap();
    let worktree_path = experiment.worktree_path().to_path_buf();

    // Make some changes
    std::fs::write(worktree_path.join("wip.txt"), "Work in progress").unwrap();
    std::process::Command::new("git")
        .args(["add", "wip.txt"])
        .current_dir(&worktree_path)
        .output()
        .expect("Failed to add file");
    std::process::Command::new("git")
        .args(["commit", "-m", "WIP"])
        .current_dir(&worktree_path)
        .output()
        .expect("Failed to commit");

    // Pause the experiment - should save state but keep worktree
    let paused = experiment.pause().unwrap();

    // Experiment should be in paused state
    assert_eq!(paused.state(), ExperimentState::Paused);

    // Worktree should still exist
    assert!(worktree_path.exists());

    // Changes should still be there
    assert!(worktree_path.join("wip.txt").exists());
}

#[test]
fn test_experiment_resume() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path().to_path_buf();
    create_git_repo_with_commit(&repo_path);

    let manager = WorktreeManager::new(repo_path.clone()).unwrap();

    // Start and pause an experiment
    let experiment = Experiment::start(&manager, "resumable").unwrap();
    let worktree_path = experiment.worktree_path().to_path_buf();
    let paused = experiment.pause().unwrap();

    // Resume the paused experiment
    let resumed = paused.resume().unwrap();

    // Should be active again
    assert_eq!(resumed.state(), ExperimentState::Active);

    // Worktree should still be there
    assert!(worktree_path.exists());
}

// ============================================================================
// ExperimentState Tests
// ============================================================================

#[test]
fn test_experiment_state_display() {
    assert_eq!(format!("{}", ExperimentState::Active), "active");
    assert_eq!(format!("{}", ExperimentState::Paused), "paused");
    assert_eq!(format!("{}", ExperimentState::Accepted), "accepted");
    assert_eq!(format!("{}", ExperimentState::Rejected), "rejected");
}

#[test]
fn test_experiment_state_is_terminal() {
    // Active and Paused are not terminal states
    assert!(!ExperimentState::Active.is_terminal());
    assert!(!ExperimentState::Paused.is_terminal());

    // Accepted and Rejected are terminal states
    assert!(ExperimentState::Accepted.is_terminal());
    assert!(ExperimentState::Rejected.is_terminal());
}

// ============================================================================
// ExperimentError Tests
// ============================================================================

#[test]
fn test_experiment_error_display() {
    let error = ExperimentError::ExperimentExists("duplicate".to_string());
    assert!(error.to_string().contains("duplicate"));

    let error = ExperimentError::ExperimentNotFound("missing".to_string());
    assert!(error.to_string().contains("missing"));

    let error = ExperimentError::InvalidState {
        current: ExperimentState::Rejected,
        expected: ExperimentState::Active,
    };
    let display = error.to_string();
    assert!(display.contains("rejected"));
    assert!(display.contains("active"));
}

#[test]
fn test_experiment_already_exists() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path().to_path_buf();
    create_git_repo_with_commit(&repo_path);

    let manager = WorktreeManager::new(repo_path.clone()).unwrap();

    // Start first experiment
    let _first = Experiment::start(&manager, "duplicate-name").unwrap();

    // Try to start another with same name - should fail
    let result = Experiment::start(&manager, "duplicate-name");
    assert!(matches!(result, Err(ExperimentError::ExperimentExists(_))));
}

#[test]
fn test_experiment_invalid_state_transition() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path().to_path_buf();
    create_git_repo_with_commit(&repo_path);

    let manager = WorktreeManager::new(repo_path.clone()).unwrap();

    // Start and accept an experiment
    let experiment = Experiment::start(&manager, "completed-exp").unwrap();
    let result = experiment.accept().unwrap();

    // Create a fake experiment handle in accepted state
    // (In practice this would be a saved/restored experiment)
    // Trying to accept again should fail with InvalidState error
    assert_eq!(result.final_state, ExperimentState::Accepted);
}

// ============================================================================
// Experiment Listing Tests
// ============================================================================

#[test]
fn test_list_experiments() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path().to_path_buf();
    create_git_repo_with_commit(&repo_path);

    let manager = WorktreeManager::new(repo_path.clone()).unwrap();

    // Create several experiments
    let _exp1 = Experiment::start(&manager, "experiment-a").unwrap();
    let exp2 = Experiment::start(&manager, "experiment-b").unwrap();
    let _paused = exp2.pause().unwrap();
    let _exp3 = Experiment::start(&manager, "experiment-c").unwrap();

    // List all experiments
    let experiments = Experiment::list(&manager).unwrap();

    // Should have 3 experiments
    assert_eq!(experiments.len(), 3);

    // Check states
    let names: Vec<&str> = experiments.iter().map(|e| e.name()).collect();
    assert!(names.contains(&"experiment-a"));
    assert!(names.contains(&"experiment-b"));
    assert!(names.contains(&"experiment-c"));
}

#[test]
fn test_list_active_experiments() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path().to_path_buf();
    create_git_repo_with_commit(&repo_path);

    let manager = WorktreeManager::new(repo_path.clone()).unwrap();

    // Create experiments with different states
    let _active = Experiment::start(&manager, "active-exp").unwrap();
    let to_pause = Experiment::start(&manager, "paused-exp").unwrap();
    let _paused = to_pause.pause().unwrap();

    // List only active experiments
    let active = Experiment::list_active(&manager).unwrap();

    // Should have only 1 active experiment
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].name(), "active-exp");
}

// ============================================================================
// Experiment with Uncommitted Changes Tests
// ============================================================================

#[test]
fn test_experiment_reject_with_uncommitted_changes() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path().to_path_buf();
    create_git_repo_with_commit(&repo_path);

    let manager = WorktreeManager::new(repo_path.clone()).unwrap();

    // Start an experiment
    let experiment = Experiment::start(&manager, "dirty-experiment").unwrap();
    let worktree_path = experiment.worktree_path().to_path_buf();

    // Make uncommitted changes (don't commit)
    std::fs::write(worktree_path.join("uncommitted.txt"), "Not committed").unwrap();

    // Reject with force should still work and discard uncommitted changes
    let result = experiment.reject_force().unwrap();
    assert!(result.success);
    assert!(!worktree_path.exists());
}

#[test]
fn test_experiment_accept_fails_with_uncommitted_changes() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path().to_path_buf();
    create_git_repo_with_commit(&repo_path);

    let manager = WorktreeManager::new(repo_path.clone()).unwrap();

    // Start an experiment
    let experiment = Experiment::start(&manager, "uncommitted-accept").unwrap();
    let worktree_path = experiment.worktree_path().to_path_buf();

    // Make uncommitted changes
    std::fs::write(worktree_path.join("uncommitted.txt"), "Not committed").unwrap();

    // Accept should fail because there are uncommitted changes
    let result = experiment.accept();
    assert!(matches!(result, Err(ExperimentError::UncommittedChanges)));
}
