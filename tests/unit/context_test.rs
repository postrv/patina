//! Tests for project context loading

use rct::context::ProjectContext;
use std::fs;
use tempfile::TempDir;

fn setup_temp_project() -> TempDir {
    TempDir::new().expect("Failed to create temp dir")
}

#[test]
fn test_project_context_new() {
    let temp = setup_temp_project();
    let ctx = ProjectContext::new(temp.path().to_path_buf());
    let content = ctx.get_context(temp.path());
    assert!(content.is_empty());
}

#[test]
fn test_project_context_load_no_claude_md() {
    let temp = setup_temp_project();
    let mut ctx = ProjectContext::new(temp.path().to_path_buf());
    let result = ctx.load();
    assert!(result.is_ok());
    assert!(ctx.get_context(temp.path()).is_empty());
}

#[test]
fn test_project_context_load_root_claude_md() {
    let temp = setup_temp_project();
    let claude_md = temp.path().join("CLAUDE.md");
    fs::write(&claude_md, "# Project Context\n\nThis is the root context.").unwrap();

    let mut ctx = ProjectContext::new(temp.path().to_path_buf());
    ctx.load().unwrap();

    let content = ctx.get_context(temp.path());
    assert!(content.contains("Project Context"));
    assert!(content.contains("root context"));
}

#[test]
fn test_project_context_load_rct_claude_md() {
    let temp = setup_temp_project();
    let rct_dir = temp.path().join(".rct");
    fs::create_dir_all(&rct_dir).unwrap();
    let claude_md = rct_dir.join("CLAUDE.md");
    fs::write(&claude_md, "# RCT Specific\n\nRCT configuration context.").unwrap();

    let mut ctx = ProjectContext::new(temp.path().to_path_buf());
    ctx.load().unwrap();

    let content = ctx.get_context(temp.path());
    assert!(content.contains("RCT Specific"));
}

#[test]
fn test_project_context_load_both_root_and_rct() {
    let temp = setup_temp_project();

    // Root CLAUDE.md
    let root_claude = temp.path().join("CLAUDE.md");
    fs::write(&root_claude, "Root context content").unwrap();

    // .rct/CLAUDE.md
    let rct_dir = temp.path().join(".rct");
    fs::create_dir_all(&rct_dir).unwrap();
    let rct_claude = rct_dir.join("CLAUDE.md");
    fs::write(&rct_claude, "RCT context content").unwrap();

    let mut ctx = ProjectContext::new(temp.path().to_path_buf());
    ctx.load().unwrap();

    let content = ctx.get_context(temp.path());
    assert!(content.contains("Root context"));
    assert!(content.contains("RCT context"));
}

#[test]
fn test_project_context_subdir_claude_md() {
    let temp = setup_temp_project();

    // Create subdir with CLAUDE.md
    let subdir = temp.path().join("src").join("api");
    fs::create_dir_all(&subdir).unwrap();
    let subdir_claude = subdir.join("CLAUDE.md");
    fs::write(&subdir_claude, "# API Module\n\nAPI-specific context.").unwrap();

    let mut ctx = ProjectContext::new(temp.path().to_path_buf());
    ctx.load().unwrap();

    // Context from root should not include subdir context
    let root_content = ctx.get_context(temp.path());
    assert!(!root_content.contains("API Module"));

    // Context from subdir should include it
    let subdir_content = ctx.get_context(&subdir);
    assert!(subdir_content.contains("API Module"));
}

#[test]
fn test_project_context_ignores_hidden_dirs() {
    let temp = setup_temp_project();

    // Create hidden dir with CLAUDE.md (should be ignored)
    let hidden_dir = temp.path().join(".hidden");
    fs::create_dir_all(&hidden_dir).unwrap();
    let hidden_claude = hidden_dir.join("CLAUDE.md");
    fs::write(&hidden_claude, "Hidden content").unwrap();

    let mut ctx = ProjectContext::new(temp.path().to_path_buf());
    ctx.load().unwrap();

    let content = ctx.get_context(&hidden_dir);
    assert!(!content.contains("Hidden content"));
}

#[test]
fn test_project_context_ignores_node_modules() {
    let temp = setup_temp_project();

    // Create node_modules with CLAUDE.md (should be ignored)
    let node_modules = temp.path().join("node_modules");
    fs::create_dir_all(&node_modules).unwrap();
    let nm_claude = node_modules.join("CLAUDE.md");
    fs::write(&nm_claude, "Node modules content").unwrap();

    let mut ctx = ProjectContext::new(temp.path().to_path_buf());
    ctx.load().unwrap();

    let content = ctx.get_context(&node_modules);
    assert!(!content.contains("Node modules"));
}

#[test]
fn test_project_context_ignores_target() {
    let temp = setup_temp_project();

    // Create target with CLAUDE.md (should be ignored)
    let target = temp.path().join("target");
    fs::create_dir_all(&target).unwrap();
    let target_claude = target.join("CLAUDE.md");
    fs::write(&target_claude, "Target content").unwrap();

    let mut ctx = ProjectContext::new(temp.path().to_path_buf());
    ctx.load().unwrap();

    let content = ctx.get_context(&target);
    assert!(!content.contains("Target content"));
}

#[test]
fn test_project_context_nested_subdirs() {
    let temp = setup_temp_project();

    // Create nested subdirs
    let level1 = temp.path().join("src");
    let level2 = level1.join("api");
    let level3 = level2.join("handlers");
    fs::create_dir_all(&level3).unwrap();

    // CLAUDE.md at level2
    let l2_claude = level2.join("CLAUDE.md");
    fs::write(&l2_claude, "Level 2 context").unwrap();

    let mut ctx = ProjectContext::new(temp.path().to_path_buf());
    ctx.load().unwrap();

    // Level3 should inherit level2 context
    let l3_content = ctx.get_context(&level3);
    assert!(l3_content.contains("Level 2 context"));
}

#[test]
fn test_project_context_outside_project_root() {
    let temp = setup_temp_project();
    let claude_md = temp.path().join("CLAUDE.md");
    fs::write(&claude_md, "Project context").unwrap();

    let mut ctx = ProjectContext::new(temp.path().to_path_buf());
    ctx.load().unwrap();

    // Path outside project root should just get root context
    let outside_path = std::path::Path::new("/tmp/outside");
    let content = ctx.get_context(outside_path);
    assert!(content.contains("Project context"));
}
