//! System prompt construction for API requests.
//!
//! Builds the base system prompt that instructs the model on its identity,
//! available tools, and project context. The prompt is injected via
//! [`AppState::set_system_prompt`] at startup and included in every API
//! request through [`AppState::build_request_options`].

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

/// Builds environment context (current date, git branch).
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
        ctx.push_str(&format!("Current git branch: {}\n", branch));
    }

    // Working directory
    ctx.push_str(&format!("Working directory: {}\n", working_dir.display()));

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
}
