//! System prompt construction for API requests.
//!
//! Builds the base system prompt that instructs the model on its identity,
//! available tools, and project context. The prompt is injected via
//! [`AppState::set_system_prompt`] at startup and included in every API
//! request through [`AppState::build_request_options`].

use std::fmt::Write as _;
use std::path::Path;
use tracing::debug;

/// Builds the system prompt from project context.
///
/// Assembles identity, tool guidance, CLAUDE.md project instructions,
/// user-global instructions, matched skill context, and environment context.
///
/// # Arguments
///
/// * `working_dir` - Project root directory for loading `.claude/CLAUDE.md`
#[must_use]
pub fn build_system_prompt(working_dir: &Path) -> String {
    build_system_prompt_with_skills(working_dir, None)
}

/// Builds the system prompt with optional skill engine context.
///
/// When a `SkillEngine` is provided, always-active skills and task-matched
/// skills are injected into the system prompt between user instructions
/// and environment context.
///
/// # Arguments
///
/// * `working_dir` - Project root directory for loading `.claude/CLAUDE.md`
/// * `skill_engine` - Optional skill engine for context-aware skill injection
#[must_use]
pub fn build_system_prompt_with_skills(
    working_dir: &Path,
    skill_engine: Option<&crate::skills::SkillEngine>,
) -> String {
    let mut parts = Vec::new();

    // Identity
    parts.push(IDENTITY.to_string());

    // Tool guidance
    parts.push(TOOL_GUIDANCE.to_string());

    // Project instructions from .claude/CLAUDE.md
    if let Some(project_instructions) = load_claude_md(working_dir) {
        parts.push(format!(
            "# Project Instructions\n\n\
             The following instructions are from the project's .claude/CLAUDE.md file. \
             Follow them carefully.\n\n{}",
            project_instructions
        ));
        debug!(
            "Loaded project CLAUDE.md from {}",
            working_dir.join(".claude/CLAUDE.md").display()
        );
    }

    // User-global instructions from ~/.claude/CLAUDE.md
    if let Some(user_instructions) = load_user_claude_md() {
        parts.push(format!(
            "# User Instructions\n\n\
             The following are the user's global instructions from ~/.claude/CLAUDE.md.\n\n{}",
            user_instructions
        ));
        debug!("Loaded user-global CLAUDE.md");
    }

    // Skill context from loaded skills
    if let Some(engine) = skill_engine {
        let skills = engine.all_skills();
        if !skills.is_empty() {
            let skill_names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
            let skill_list = skill_names.join(", ");
            parts.push(format!(
                "# Available Skills\n\n\
                 The following skills are loaded and available: {skill_list}"
            ));
            debug!("Injected {} skill(s) into system prompt", skills.len());
        }
    }

    // Environment context
    parts.push(build_environment_context(working_dir));

    parts.join("\n\n")
}

/// Loads `.claude/CLAUDE.md` from the project directory.
fn load_claude_md(working_dir: &Path) -> Option<String> {
    let path = working_dir.join(".claude").join("CLAUDE.md");
    std::fs::read_to_string(path).ok().filter(|s| !s.is_empty())
}

/// Loads `~/.claude/CLAUDE.md` from the user's home directory.
fn load_user_claude_md() -> Option<String> {
    let home = std::env::var("HOME")
        .ok()
        .or_else(|| std::env::var("USERPROFILE").ok())
        .map(std::path::PathBuf::from)?;
    let path = home.join(".claude").join("CLAUDE.md");
    std::fs::read_to_string(path).ok().filter(|s| !s.is_empty())
}

/// Returns a compact summary of `git status --porcelain` output.
///
/// Categorises working-tree changes into modified, added, deleted, and
/// untracked counts. Returns `"Clean working tree"` when there are no
/// changes, or `None` when `working_dir` is not inside a git repository.
///
/// # Errors
///
/// Returns `None` if the `git` command fails (e.g. not a repo).
fn get_git_status_summary(working_dir: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(working_dir)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() {
        return Some("Clean working tree".to_string());
    }

    let mut modified = 0u32;
    let mut added = 0u32;
    let mut deleted = 0u32;
    let mut untracked = 0u32;

    for line in stdout.lines() {
        if line.len() < 2 {
            continue;
        }
        let index = line.as_bytes()[0];
        let worktree = line.as_bytes()[1];

        match (index, worktree) {
            (b'?', b'?') => untracked += 1,
            _ => {
                // Index status
                match index {
                    b'A' => added += 1,
                    b'D' => deleted += 1,
                    b'M' | b'R' | b'C' => modified += 1,
                    _ => {}
                }
                // Worktree status (unstaged changes)
                match worktree {
                    b'M' => modified += 1,
                    b'D' => deleted += 1,
                    _ => {}
                }
            }
        }
    }

    let mut parts = Vec::new();
    if modified > 0 {
        parts.push(format!("{modified} modified"));
    }
    if added > 0 {
        parts.push(format!("{added} added"));
    }
    if deleted > 0 {
        parts.push(format!("{deleted} deleted"));
    }
    if untracked > 0 {
        parts.push(format!("{untracked} untracked"));
    }

    if parts.is_empty() {
        Some("Clean working tree".to_string())
    } else {
        Some(parts.join(", "))
    }
}

/// Returns the last 5 commits as a compact oneline summary.
///
/// # Errors
///
/// Returns `None` if the working directory is not a git repository or
/// has no commits.
fn get_recent_commits(working_dir: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["log", "--oneline", "-5"])
        .current_dir(working_dir)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Maximum number of output lines before the tree is truncated.
const PROJECT_TREE_MAX_LINES: usize = 30;

/// Directories that are excluded from the project tree because they are
/// noisy or auto-generated.
const NOISE_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".DS_Store",
    "__pycache__",
    ".mypy_cache",
    ".pytest_cache",
    "dist",
    "build",
    ".next",
    ".cache",
    "coverage",
    ".tox",
    ".eggs",
    ".venv",
    "venv",
];

/// Returns a compact directory tree for the given project root.
///
/// Walks the directory up to `max_depth` levels, skipping common noise
/// directories (`.git`, `target`, `node_modules`, etc.). Only top-level
/// files are shown; files inside subdirectories are omitted to keep the
/// output compact. Output is truncated to [`PROJECT_TREE_MAX_LINES`].
///
/// # Errors
///
/// Returns `None` if the directory cannot be read.
fn get_project_tree(working_dir: &Path, max_depth: usize) -> Option<String> {
    let mut tree = String::new();
    let mut line_count = 0usize;
    build_tree_recursive(working_dir, &mut tree, 0, max_depth, &mut line_count);

    if tree.is_empty() {
        return None;
    }

    Some(tree)
}

/// Recursively builds the tree string, counting lines and truncating when
/// the limit is exceeded.
fn build_tree_recursive(
    dir: &Path,
    output: &mut String,
    depth: usize,
    max_depth: usize,
    line_count: &mut usize,
) {
    if depth >= max_depth {
        return;
    }

    let mut entries: Vec<_> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(_) => return,
    };
    entries.sort_by_key(|e| e.file_name());

    // Count remaining entries that we haven't shown yet (for truncation msg)
    let mut remaining_after_limit = 0usize;

    for entry in &entries {
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip noise directories and hidden files (except at depth 0 for dirs)
        if NOISE_DIRS.contains(&name.as_str()) {
            continue;
        }

        let path = entry.path();

        if *line_count >= PROJECT_TREE_MAX_LINES {
            remaining_after_limit += 1;
            continue;
        }

        let indent = "  ".repeat(depth);

        if path.is_dir() {
            let _ = writeln!(output, "{indent}{name}/");
            *line_count += 1;
            build_tree_recursive(&path, output, depth + 1, max_depth, line_count);
        } else if depth == 0 {
            // Only show top-level files
            let _ = writeln!(output, "{indent}{name}");
            *line_count += 1;
        }
    }

    if depth == 0 && remaining_after_limit > 0 {
        let _ = writeln!(output, "... and {remaining_after_limit} more entries");
    }
}

/// Builds environment context (current date, git branch, status, tree).
fn build_environment_context(working_dir: &Path) -> String {
    let mut ctx = String::from("# Environment\n");

    // Current date (YYYY-MM-DD using std time)
    if let Ok(output) = std::process::Command::new("date").arg("+%Y-%m-%d").output() {
        if output.status.success() {
            let date = String::from_utf8_lossy(&output.stdout);
            ctx.push_str(&format!("\nCurrent date: {}\n", date.trim()));
        }
    }

    // Git branch
    if let Some(branch) = get_git_branch(working_dir) {
        ctx.push_str(&format!("Current git branch: {branch}\n"));
    }

    // Git status summary
    if let Some(status) = get_git_status_summary(working_dir) {
        ctx.push_str(&format!("Git status: {status}\n"));
    }

    // Recent commits
    if let Some(commits) = get_recent_commits(working_dir) {
        ctx.push_str(&format!("Recent commits:\n{commits}\n"));
    }

    // Working directory
    ctx.push_str(&format!("Working directory: {}\n", working_dir.display()));

    // Project structure
    if let Some(tree) = get_project_tree(working_dir, 2) {
        ctx.push_str(&format!("\nProject structure:\n{tree}"));
    }

    ctx
}

/// Gets the current git branch name, if in a git repository.
fn get_git_branch(working_dir: &Path) -> Option<String> {
    std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(working_dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

const IDENTITY: &str = "\
You are Patina, a high-performance AI coding assistant powered by Claude. \
You help users with software engineering tasks including writing code, debugging, \
refactoring, explaining code, and navigating codebases. You have access to tools \
for reading and writing files, running commands, and searching code.";

const TOOL_GUIDANCE: &str = "\
# Tool Usage

Use the appropriate tool for each task:
- **bash**: Run shell commands, build projects, run tests
- **read_file**: Read file contents (prefer over bash cat/head/tail)
- **write_file**: Create new files
- **edit**: Modify existing files with precise string replacements
- **multi_edit**: Apply edits to multiple files in one operation
- **notebook_edit**: Edit Jupyter notebook cells by index
- **send_message**: Send messages to other active agents
- **grep**: Search file contents with regex patterns
- **glob**: Find files by name patterns
- **list_files**: List directory contents
- **web_fetch**: Fetch content from URLs
- **web_search**: Search the web for information
- **lsp**: Use Language Server Protocol for code intelligence (go-to-definition, find references)
- **todo_write**: Manage task lists

Prefer dedicated tools over bash equivalents (e.g., use read_file instead of cat, \
grep tool instead of grep command). Break complex tasks into smaller steps. \
Read files before modifying them to understand existing code.";

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_build_system_prompt_contains_identity() {
        let dir = TempDir::new().unwrap();
        let prompt = build_system_prompt(dir.path());
        assert!(prompt.contains("Patina"));
        assert!(prompt.contains("coding assistant"));
    }

    #[test]
    fn test_build_system_prompt_contains_tool_guidance() {
        let dir = TempDir::new().unwrap();
        let prompt = build_system_prompt(dir.path());
        assert!(prompt.contains("bash"));
        assert!(prompt.contains("read_file"));
        assert!(prompt.contains("grep"));
    }

    #[test]
    fn test_build_system_prompt_loads_project_claude_md() {
        let dir = TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        fs::write(claude_dir.join("CLAUDE.md"), "Always use snake_case.").unwrap();

        let prompt = build_system_prompt(dir.path());
        assert!(prompt.contains("Always use snake_case."));
        assert!(prompt.contains("Project Instructions"));
    }

    #[test]
    fn test_build_system_prompt_works_without_claude_md() {
        let dir = TempDir::new().unwrap();
        // No .claude/CLAUDE.md exists
        let prompt = build_system_prompt(dir.path());
        assert!(!prompt.contains("Project Instructions"));
        assert!(prompt.contains("Patina")); // identity still present
    }

    #[test]
    fn test_build_system_prompt_contains_environment() {
        let dir = TempDir::new().unwrap();
        let prompt = build_system_prompt(dir.path());
        assert!(prompt.contains("Current date:"));
        assert!(prompt.contains("Working directory:"));
    }

    #[test]
    fn test_load_claude_md_returns_none_for_missing() {
        let dir = TempDir::new().unwrap();
        assert!(load_claude_md(dir.path()).is_none());
    }

    #[test]
    fn test_load_claude_md_returns_none_for_empty() {
        let dir = TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        fs::write(claude_dir.join("CLAUDE.md"), "").unwrap();
        assert!(load_claude_md(dir.path()).is_none());
    }

    #[test]
    fn test_get_git_branch_returns_none_for_non_repo() {
        let dir = TempDir::new().unwrap();
        assert!(get_git_branch(dir.path()).is_none());
    }

    #[test]
    fn test_build_system_prompt_with_skills_none() {
        let dir = TempDir::new().unwrap();
        let prompt = build_system_prompt_with_skills(dir.path(), None);
        assert!(prompt.contains("Patina"));
        assert!(!prompt.contains("Available Skills"));
    }

    #[test]
    fn test_build_system_prompt_with_skills_empty_engine() {
        let dir = TempDir::new().unwrap();
        let engine = crate::skills::SkillEngine::new();
        let prompt = build_system_prompt_with_skills(dir.path(), Some(&engine));
        assert!(!prompt.contains("Available Skills"));
    }

    #[test]
    fn test_build_system_prompt_with_skills_loaded() {
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join("skills");
        let skill_subdir = skills_dir.join("test-skill");
        fs::create_dir_all(&skill_subdir).unwrap();
        fs::write(
            skill_subdir.join("SKILL.md"),
            "---\nname: test-skill\ndescription: A test skill\n---\nDo the thing.",
        )
        .unwrap();

        let mut engine = crate::skills::SkillEngine::new();
        engine.load_from_dir(&skills_dir).unwrap();

        let prompt = build_system_prompt_with_skills(dir.path(), Some(&engine));
        assert!(prompt.contains("Available Skills"));
        assert!(prompt.contains("test-skill"));
    }

    // B3 tests: git status summary

    #[test]
    fn test_get_git_status_summary_clean() {
        let dir = TempDir::new().unwrap();
        // Initialize a git repo and make an initial commit so it's clean
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        fs::write(dir.path().join("README.md"), "hello").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        let result = get_git_status_summary(dir.path());
        assert_eq!(result, Some("Clean working tree".to_string()));
    }

    #[test]
    fn test_get_git_status_summary_modified_files() {
        let dir = TempDir::new().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        fs::write(dir.path().join("file.txt"), "hello").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // Now modify the file
        fs::write(dir.path().join("file.txt"), "changed").unwrap();
        // Add an untracked file
        fs::write(dir.path().join("new.txt"), "new").unwrap();

        let result = get_git_status_summary(dir.path()).unwrap();
        assert!(result.contains("1 modified"));
        assert!(result.contains("1 untracked"));
    }

    #[test]
    fn test_get_git_status_summary_non_repo() {
        let dir = TempDir::new().unwrap();
        assert!(get_git_status_summary(dir.path()).is_none());
    }

    // B3 tests: project tree

    #[test]
    fn test_get_project_tree_basic() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();

        let tree = get_project_tree(dir.path(), 2).unwrap();
        assert!(tree.contains("src/"));
        assert!(tree.contains("Cargo.toml"));
    }

    #[test]
    fn test_get_project_tree_skips_noise() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        fs::create_dir_all(dir.path().join("target")).unwrap();
        fs::create_dir_all(dir.path().join("node_modules")).unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/lib.rs"), "").unwrap();

        let tree = get_project_tree(dir.path(), 2).unwrap();
        assert!(!tree.contains(".git"));
        assert!(!tree.contains("target"));
        assert!(!tree.contains("node_modules"));
        assert!(tree.contains("src/"));
    }

    #[test]
    fn test_get_project_tree_respects_max_depth() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("a/b/c/d")).unwrap();
        fs::write(dir.path().join("a/b/c/d/deep.txt"), "deep").unwrap();

        let tree = get_project_tree(dir.path(), 2).unwrap();
        // depth 0 = "a/", depth 1 = "b/", depth 2 would be stopped
        assert!(tree.contains("a/"));
        assert!(tree.contains("b/"));
        // "c/" is at depth 2 which is >= max_depth, so not shown
        assert!(!tree.contains("c/"));
    }

    #[test]
    fn test_get_project_tree_truncates_long_output() {
        let dir = TempDir::new().unwrap();
        // Create many top-level directories to exceed the line limit
        for i in 0..40 {
            fs::create_dir_all(dir.path().join(format!("dir_{i:03}"))).unwrap();
        }

        let tree = get_project_tree(dir.path(), 2).unwrap();
        assert!(tree.contains("... and"));
        assert!(tree.contains("more"));
    }

    // B3 tests: recent commits

    #[test]
    fn test_get_recent_commits_in_repo() {
        let dir = TempDir::new().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        fs::write(dir.path().join("file.txt"), "hello").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "Initial commit"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        let result = get_recent_commits(dir.path()).unwrap();
        assert!(result.contains("Initial commit"));
    }

    #[test]
    fn test_get_recent_commits_non_repo() {
        let dir = TempDir::new().unwrap();
        assert!(get_recent_commits(dir.path()).is_none());
    }

    // B3 tests: environment context integration

    #[test]
    fn test_build_environment_context_includes_git_status() {
        let dir = TempDir::new().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        fs::write(dir.path().join("file.txt"), "hello").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        let ctx = build_environment_context(dir.path());
        assert!(ctx.contains("Git status:"));
        assert!(ctx.contains("Clean working tree"));
    }

    #[test]
    fn test_build_environment_context_includes_project_structure() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();

        let ctx = build_environment_context(dir.path());
        assert!(ctx.contains("Project structure:"));
        assert!(ctx.contains("src/"));
    }
}
