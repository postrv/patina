//! Tests for project context loading

use patina::context::ProjectContext;
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
fn test_project_context_load_patina_claude_md() {
    let temp = setup_temp_project();
    let patina_dir = temp.path().join(".patina");
    fs::create_dir_all(&patina_dir).unwrap();
    let claude_md = patina_dir.join("CLAUDE.md");
    fs::write(
        &claude_md,
        "# Patina Specific\n\nPatina configuration context.",
    )
    .unwrap();

    let mut ctx = ProjectContext::new(temp.path().to_path_buf());
    ctx.load().unwrap();

    let content = ctx.get_context(temp.path());
    assert!(content.contains("Patina Specific"));
}

#[test]
fn test_project_context_load_both_root_and_patina() {
    let temp = setup_temp_project();

    // Root CLAUDE.md
    let root_claude = temp.path().join("CLAUDE.md");
    fs::write(&root_claude, "Root context content").unwrap();

    // .patina/CLAUDE.md
    let patina_dir = temp.path().join(".patina");
    fs::create_dir_all(&patina_dir).unwrap();
    let patina_claude = patina_dir.join("CLAUDE.md");
    fs::write(&patina_claude, "Patina context content").unwrap();

    let mut ctx = ProjectContext::new(temp.path().to_path_buf());
    ctx.load().unwrap();

    let content = ctx.get_context(temp.path());
    assert!(content.contains("Root context"));
    assert!(content.contains("Patina context"));
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

// =============================================================================
// Graceful Degradation Tests (4.2.1)
// =============================================================================

/// Test that context loading continues when a CLAUDE.md file is unreadable.
///
/// This tests graceful degradation: if a CLAUDE.md file in a subdirectory
/// cannot be read (e.g., permission denied), loading should log a warning
/// and continue, rather than failing the entire load operation.
#[test]
fn test_context_continues_on_unreadable_claude_md() {
    let temp = setup_temp_project();

    // Create valid root CLAUDE.md
    let root_claude = temp.path().join("CLAUDE.md");
    fs::write(&root_claude, "Root context content").unwrap();

    // Create a readable subdir with CLAUDE.md
    let readable_dir = temp.path().join("readable");
    fs::create_dir_all(&readable_dir).unwrap();
    let readable_claude = readable_dir.join("CLAUDE.md");
    fs::write(&readable_claude, "Readable subdir context").unwrap();

    // Create another subdir whose CLAUDE.md has invalid UTF-8
    // This tests the graceful handling of read failures
    let invalid_dir = temp.path().join("invalid");
    fs::create_dir_all(&invalid_dir).unwrap();
    let invalid_claude = invalid_dir.join("CLAUDE.md");
    // Write invalid UTF-8 bytes - this will cause read_to_string to fail
    fs::write(&invalid_claude, [0xFF, 0xFE, 0x00, 0x01]).unwrap();

    let mut ctx = ProjectContext::new(temp.path().to_path_buf());

    // Load should succeed despite the unreadable CLAUDE.md
    let result = ctx.load();
    assert!(
        result.is_ok(),
        "Context loading should succeed even with unreadable subdir CLAUDE.md: {:?}",
        result.err()
    );

    // Root context should still be loaded
    let content = ctx.get_context(temp.path());
    assert!(
        content.contains("Root context"),
        "Root context should be loaded"
    );

    // Readable subdir context should be loaded
    let readable_content = ctx.get_context(&readable_dir);
    assert!(
        readable_content.contains("Readable subdir context"),
        "Readable subdir context should be loaded"
    );
}

/// Test that context loading continues when a subdirectory cannot be traversed.
///
/// This tests graceful degradation: if a subdirectory cannot be read
/// (e.g., permission denied on the directory itself), loading should
/// log a warning and continue with other directories.
#[cfg(unix)]
#[test]
fn test_context_continues_on_unreadable_subdir() {
    use std::os::unix::fs::PermissionsExt;

    let temp = setup_temp_project();

    // Create valid root CLAUDE.md
    let root_claude = temp.path().join("CLAUDE.md");
    fs::write(&root_claude, "Root context content").unwrap();

    // Create a readable subdir with CLAUDE.md
    let readable_dir = temp.path().join("readable");
    fs::create_dir_all(&readable_dir).unwrap();
    let readable_claude = readable_dir.join("CLAUDE.md");
    fs::write(&readable_claude, "Readable content").unwrap();

    // Create an unreadable subdir (no read permission)
    let unreadable_dir = temp.path().join("unreadable");
    fs::create_dir_all(&unreadable_dir).unwrap();
    // Write a CLAUDE.md in it before making it unreadable
    let unreadable_claude = unreadable_dir.join("CLAUDE.md");
    fs::write(&unreadable_claude, "Unreadable content").unwrap();

    // Remove read permission from the directory
    let mut perms = fs::metadata(&unreadable_dir).unwrap().permissions();
    perms.set_mode(0o000);
    fs::set_permissions(&unreadable_dir, perms).unwrap();

    // Ensure we restore permissions on cleanup
    struct RestorePerms {
        path: std::path::PathBuf,
    }
    impl Drop for RestorePerms {
        fn drop(&mut self) {
            let mut perms = fs::metadata(&self.path)
                .map(|m| m.permissions())
                .unwrap_or_else(|_| std::fs::Permissions::from_mode(0o755));
            perms.set_mode(0o755);
            let _ = fs::set_permissions(&self.path, perms);
        }
    }
    let _guard = RestorePerms {
        path: unreadable_dir.clone(),
    };

    let mut ctx = ProjectContext::new(temp.path().to_path_buf());

    // Load should succeed despite the unreadable subdirectory
    let result = ctx.load();
    assert!(
        result.is_ok(),
        "Context loading should succeed even with unreadable subdir: {:?}",
        result.err()
    );

    // Root context should still be loaded
    let content = ctx.get_context(temp.path());
    assert!(
        content.contains("Root context"),
        "Root context should be loaded"
    );

    // Readable subdir context should be loaded
    let readable_content = ctx.get_context(&readable_dir);
    assert!(
        readable_content.contains("Readable content"),
        "Readable subdir context should be loaded"
    );
}
