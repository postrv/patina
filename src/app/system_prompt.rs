//! System prompt construction for API requests.
//!
//! Builds the base system prompt that instructs the model on its identity,
//! available tools, and project context. The prompt is injected via
//! [`AppState::set_system_prompt`] at startup and included in every API
//! request through [`AppState::build_request_options`].

use crate::context::ProjectContext;
use crate::rules::RuleEngine;
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
/// * `working_dir` - Project root directory for loading CLAUDE.md files
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
/// * `working_dir` - Project root directory for loading CLAUDE.md files
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

    // Narsil tool guidance (harmless if narsil is unavailable)
    parts.push(NARSIL_TOOL_GUIDANCE.to_string());

    // Project instructions from CLAUDE.md, .claude/CLAUDE.md, .patina/CLAUDE.md
    if let Some(project_instructions) = load_project_instructions(working_dir) {
        parts.push(format!(
            "# Project Instructions\n\n\
             The following instructions are from the project's CLAUDE.md files. \
             Follow them carefully.\n\n{}",
            project_instructions
        ));
        debug!("Loaded project CLAUDE.md from {}", working_dir.display());
    }

    // Unconditional rules from .claude/rules/, .patina/rules/, ~/.claude/rules/
    match RuleEngine::load_from_dirs(working_dir) {
        Ok(rule_engine) => {
            let unconditional = rule_engine.unconditional_rules();
            if !unconditional.is_empty() {
                let rules_text: Vec<&str> =
                    unconditional.iter().map(|r| r.content.as_str()).collect();
                parts.push(format!("# Project Rules\n\n{}", rules_text.join("\n\n")));
                debug!(
                    "Loaded {} unconditional rule(s) into system prompt",
                    unconditional.len()
                );
            }
        }
        Err(e) => {
            tracing::warn!("Failed to load rules: {}", e);
        }
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

    // Narsil code intelligence context (graceful: returns None if unavailable)
    if let Some(narsil_ctx) = load_narsil_context(working_dir) {
        parts.push(narsil_ctx);
        debug!("Injected narsil code intelligence context into system prompt");
    }

    parts.join("\n\n")
}

/// Loads project instructions from all CLAUDE.md locations.
///
/// Uses [`ProjectContext`] to discover and merge content from:
/// - `CLAUDE.md` at the project root
/// - `.claude/CLAUDE.md` (standard Claude Code location)
/// - `.patina/CLAUDE.md` (Patina-specific location)
///
/// Returns `None` if no CLAUDE.md files are found or all are empty.
fn load_project_instructions(working_dir: &Path) -> Option<String> {
    let mut ctx = ProjectContext::new(working_dir.to_path_buf());
    if let Err(e) = ctx.load() {
        tracing::warn!("Failed to load project context: {}", e);
        return None;
    }
    ctx.root_context()
        .map(String::from)
        .filter(|s| !s.is_empty())
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

/// Loads cached narsil code intelligence context for the system prompt.
///
/// Attempts to read a cached narsil context from the platform cache directory.
/// Returns `None` if no cache is available or the cache is stale (> 5 minutes).
fn load_narsil_context(working_dir: &Path) -> Option<String> {
    let cache_dir = dirs_for_narsil_cache()?;
    crate::narsil::system_prompt::build_narsil_context_cached(working_dir, &cache_dir)
}

/// Returns the platform cache directory for narsil context.
///
/// Uses `$XDG_CACHE_HOME` if set, otherwise falls back to `~/.cache` on
/// Linux/macOS or the platform equivalent.
fn dirs_for_narsil_cache() -> Option<std::path::PathBuf> {
    std::env::var("XDG_CACHE_HOME")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .or_else(|| std::env::var("USERPROFILE").ok())
                .map(|h| std::path::PathBuf::from(h).join(".cache"))
        })
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

/// Guidance for narsil-mcp code intelligence tools.
///
/// Conditionally included in the system prompt to teach the model when and
/// how to use structural code intelligence tools instead of text-based
/// search.
const NARSIL_TOOL_GUIDANCE: &str = "\
# Code Intelligence Tools (narsil)

When narsil-mcp code intelligence tools are available, prefer them for structural queries:

- **Before refactoring**: Use `find_callers`/`get_callers` to understand impact before changing a function
- **Security analysis**: Use `trace_taint`/`get_data_flow` to trace data flow for security review
- **Code review**: Use `get_complexity`/`get_function_hotspots` to identify risky high-complexity code
- **Architecture**: Use `find_circular_imports` to detect dependency cycles
- **Understanding code**: Use `get_call_graph` for function relationship maps
- **Finding usage**: Use `find_references`/`find_symbol_usages` before renaming or removing

Prefer narsil structural tools over grep/glob for queries about callers, callees, \
data flow, and dependencies. They understand the code graph, not just text patterns.";

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
        let prompt = build_system_prompt(dir.path());
        assert!(!prompt.contains("Project Instructions"));
        assert!(prompt.contains("Patina"));
    }

    #[test]
    fn test_build_system_prompt_contains_environment() {
        let dir = TempDir::new().unwrap();
        let prompt = build_system_prompt(dir.path());
        assert!(prompt.contains("Current date:"));
        assert!(prompt.contains("Working directory:"));
    }

    #[test]
    fn test_build_system_prompt_loads_patina_claude_md() {
        let dir = TempDir::new().unwrap();
        let patina_dir = dir.path().join(".patina");
        fs::create_dir_all(&patina_dir).unwrap();
        fs::write(patina_dir.join("CLAUDE.md"), "Patina-specific rules.").unwrap();

        let prompt = build_system_prompt(dir.path());
        assert!(prompt.contains("Patina-specific rules."));
        assert!(prompt.contains("Project Instructions"));
    }

    #[test]
    fn test_build_system_prompt_loads_root_claude_md() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("CLAUDE.md"), "Root project rules.").unwrap();

        let prompt = build_system_prompt(dir.path());
        assert!(prompt.contains("Root project rules."));
        assert!(prompt.contains("Project Instructions"));
    }

    #[test]
    fn test_build_system_prompt_merges_multiple_claude_md() {
        let dir = TempDir::new().unwrap();

        fs::write(dir.path().join("CLAUDE.md"), "FROM_ROOT").unwrap();

        let claude_dir = dir.path().join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        fs::write(claude_dir.join("CLAUDE.md"), "FROM_DOT_CLAUDE").unwrap();

        let patina_dir = dir.path().join(".patina");
        fs::create_dir_all(&patina_dir).unwrap();
        fs::write(patina_dir.join("CLAUDE.md"), "FROM_PATINA").unwrap();

        let prompt = build_system_prompt(dir.path());
        assert!(prompt.contains("FROM_ROOT"));
        assert!(prompt.contains("FROM_DOT_CLAUDE"));
        assert!(prompt.contains("FROM_PATINA"));
    }

    #[test]
    fn test_load_project_instructions_returns_none_for_missing() {
        let dir = TempDir::new().unwrap();
        assert!(load_project_instructions(dir.path()).is_none());
    }

    #[test]
    fn test_load_project_instructions_returns_none_for_empty() {
        let dir = TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        fs::write(claude_dir.join("CLAUDE.md"), "").unwrap();
        assert!(load_project_instructions(dir.path()).is_none());
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

    #[test]
    fn test_get_git_status_summary_clean() {
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

        fs::write(dir.path().join("file.txt"), "changed").unwrap();
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
        assert!(tree.contains("a/"));
        assert!(tree.contains("b/"));
        assert!(!tree.contains("c/"));
    }

    #[test]
    fn test_get_project_tree_truncates_long_output() {
        let dir = TempDir::new().unwrap();
        for i in 0..40 {
            fs::create_dir_all(dir.path().join(format!("dir_{i:03}"))).unwrap();
        }

        let tree = get_project_tree(dir.path(), 2).unwrap();
        assert!(tree.contains("... and"));
        assert!(tree.contains("more"));
    }

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

    #[test]
    fn test_system_prompt_includes_unconditional_rules() {
        let dir = TempDir::new().unwrap();
        let rules_dir = dir.path().join(".claude").join("rules");
        fs::create_dir_all(&rules_dir).unwrap();
        fs::write(
            rules_dir.join("style.md"),
            "Always use snake_case for variables.",
        )
        .unwrap();
        let prompt = build_system_prompt(dir.path());
        assert!(
            prompt.contains("Project Rules"),
            "System prompt should contain Project Rules header"
        );
        assert!(prompt.contains("Always use snake_case for variables."));
    }

    #[test]
    fn test_system_prompt_excludes_path_scoped_rules() {
        let dir = TempDir::new().unwrap();
        let rules_dir = dir.path().join(".claude").join("rules");
        fs::create_dir_all(&rules_dir).unwrap();
        fs::write(
            rules_dir.join("scoped.md"),
            "---\npaths:\n  - \"src/api/**/*.rs\"\n---\nAPI-specific rule.",
        )
        .unwrap();
        let prompt = build_system_prompt(dir.path());
        assert!(!prompt.contains("API-specific rule."));
    }

    #[test]
    fn test_system_prompt_no_rules_section_when_no_rules() {
        let dir = TempDir::new().unwrap();
        let prompt = build_system_prompt(dir.path());
        assert!(!prompt.contains("Project Rules"));
    }

    #[test]
    fn test_system_prompt_includes_narsil_tool_guidance() {
        let dir = TempDir::new().unwrap();
        let prompt = build_system_prompt(dir.path());
        assert!(
            prompt.contains("Code Intelligence Tools (narsil)"),
            "System prompt should contain narsil tool guidance"
        );
        assert!(
            prompt.contains("find_callers"),
            "Narsil guidance should mention find_callers"
        );
        assert!(
            prompt.contains("get_call_graph"),
            "Narsil guidance should mention get_call_graph"
        );
    }
}
