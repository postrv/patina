//! Path-scoped rules engine for project and user rule files.
//!
//! Rules are markdown files stored in `.claude/rules/`, `.patina/rules/`, or
//! `~/.claude/rules/` directories. Each rule file may contain YAML frontmatter
//! between `---` delimiters with an optional `paths:` field specifying glob
//! patterns. Rules without a `paths:` field are unconditional and always apply.
//! Rules with `paths:` only apply when working with files matching those globs.
//!
//! # Example rule file
//!
//! ```markdown
//! ---
//! paths:
//!   - "src/api/**/*.rs"
//!   - "src/tools/**/*.rs"
//! ---
//! Always validate user input before passing to shell commands.
//! Use `validate_path()` for all file paths.
//! ```
//!
//! # Architecture
//!
//! - [`Rule`] — a single rule with content, optional path globs, and source
//! - [`RuleSource`] — discriminates project vs user origin
//! - [`RuleEngine`] — loads and queries rules from all configured directories

use anyhow::Result;
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// A single rule loaded from a `.md` file.
///
/// Rules without `paths` are unconditional (always injected into system prompt).
/// Rules with `paths` are path-scoped and only apply to matching files.
#[derive(Debug, Clone)]
pub struct Rule {
    /// The rule body content (everything after frontmatter).
    pub content: String,
    /// Optional glob patterns; `None` means this rule is unconditional.
    pub paths: Option<Vec<String>>,
    /// Where this rule was loaded from.
    pub source: RuleSource,
}

/// Where a rule was loaded from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleSource {
    /// Loaded from a project-local directory (`.claude/rules/` or `.patina/rules/`).
    Project(PathBuf),
    /// Loaded from the user's global directory (`~/.claude/rules/`).
    User(PathBuf),
}

/// YAML frontmatter structure for rule files.
#[derive(Debug, Deserialize, Default)]
struct RuleFrontmatter {
    /// Optional list of glob patterns that scope this rule.
    #[serde(default)]
    paths: Option<Vec<String>>,
}

/// Engine that loads and queries path-scoped rules.
///
/// Rules are loaded from three directories:
/// - `{project_root}/.claude/rules/*.md` (project rules)
/// - `{project_root}/.patina/rules/*.md` (project rules, alternative)
/// - `~/.claude/rules/*.md` (user-global rules)
pub struct RuleEngine {
    unconditional: Vec<Rule>,
    path_scoped: Vec<Rule>,
}

impl RuleEngine {
    /// Loads rules from all configured directories under `project_root`.
    ///
    /// Scans `.claude/rules/`, `.patina/rules/`, and `~/.claude/rules/`
    /// for `.md` files. Unreadable files are logged as warnings and skipped.
    ///
    /// # Errors
    ///
    /// Returns an error only for critical failures. Individual file read
    /// failures are logged and skipped.
    pub fn load_from_dirs(project_root: &Path) -> Result<Self> {
        let mut unconditional = Vec::new();
        let mut path_scoped = Vec::new();

        // Project rules from .claude/rules/
        let claude_rules = project_root.join(".claude").join("rules");
        Self::load_rules_from_dir(&claude_rules, true, &mut unconditional, &mut path_scoped);

        // Project rules from .patina/rules/
        let patina_rules = project_root.join(".patina").join("rules");
        Self::load_rules_from_dir(&patina_rules, true, &mut unconditional, &mut path_scoped);

        // User-global rules from ~/.claude/rules/
        if let Some(home) = home_dir() {
            let user_rules = home.join(".claude").join("rules");
            Self::load_rules_from_dir(&user_rules, false, &mut unconditional, &mut path_scoped);
        }

        Ok(Self {
            unconditional,
            path_scoped,
        })
    }

    /// Returns all unconditional rules (no `paths` field).
    #[must_use]
    pub fn unconditional_rules(&self) -> &[Rule] {
        &self.unconditional
    }

    /// Returns all rules whose glob patterns match `file_path`.
    ///
    /// Checks each path-scoped rule's glob patterns against the given file
    /// path. Returns references to all matching rules.
    #[must_use]
    pub fn rules_for_path(&self, file_path: &Path) -> Vec<&Rule> {
        let path_str = file_path.to_string_lossy();

        self.path_scoped
            .iter()
            .filter(|rule| {
                if let Some(ref patterns) = rule.paths {
                    patterns.iter().any(|pattern| {
                        glob::Pattern::new(pattern)
                            .map(|p| p.matches(&path_str))
                            .unwrap_or(false)
                    })
                } else {
                    false
                }
            })
            .collect()
    }

    /// Loads `.md` files from a single directory.
    ///
    /// When `is_project` is true, rules are tagged as [`RuleSource::Project`];
    /// otherwise as [`RuleSource::User`].
    fn load_rules_from_dir(
        dir: &Path,
        is_project: bool,
        unconditional: &mut Vec<Rule>,
        path_scoped: &mut Vec<Rule>,
    ) {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => return, // Directory doesn't exist or unreadable
        };

        for entry_result in entries {
            let entry = match entry_result {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!("Error reading rules directory entry in {:?}: {}", dir, e);
                    continue;
                }
            };

            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            // Only process .md files
            let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if extension != "md" {
                continue;
            }

            match parse_rule_file(&path, is_project) {
                Ok(rule) => {
                    if rule.paths.is_some() {
                        path_scoped.push(rule);
                    } else {
                        unconditional.push(rule);
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to parse rule file {:?}: {}", path, e);
                }
            }
        }
    }
}

/// Parses a single rule `.md` file.
///
/// Extracts optional YAML frontmatter (between `---` delimiters) and the
/// body content. The frontmatter may contain a `paths:` field with glob
/// patterns.
///
/// # Errors
///
/// Returns an error if the file cannot be read or the frontmatter YAML
/// is malformed.
pub fn parse_rule_file(path: &Path, is_project: bool) -> Result<Rule> {
    let content = std::fs::read_to_string(path)?;
    parse_rule_content(&content, path, is_project)
}

/// Parses rule content from a string (used for testing without file I/O).
///
/// # Errors
///
/// Returns an error if the YAML frontmatter is malformed.
fn parse_rule_content(content: &str, source_path: &Path, is_project: bool) -> Result<Rule> {
    let (frontmatter_str, body) = if let Some(after_open) = content.strip_prefix("---") {
        if let Some(end) = after_open.find("---") {
            let yaml = &after_open[..end];
            let body = after_open[end..].strip_prefix("---").unwrap_or("").trim();
            (yaml, body)
        } else {
            // No closing ---, treat entire content as body
            ("", content.trim())
        }
    } else {
        ("", content.trim())
    };

    let frontmatter: RuleFrontmatter = if !frontmatter_str.trim().is_empty() {
        serde_yaml::from_str(frontmatter_str)?
    } else {
        RuleFrontmatter::default()
    };

    let source = if is_project {
        RuleSource::Project(source_path.to_path_buf())
    } else {
        RuleSource::User(source_path.to_path_buf())
    };

    Ok(Rule {
        content: body.to_string(),
        paths: frontmatter.paths,
        source,
    })
}

/// Returns the user's home directory.
fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .or_else(|| std::env::var("USERPROFILE").ok())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // ---- parse_rule_file / parse_rule_content tests ----

    #[test]
    fn parse_rule_file_with_yaml_frontmatter_paths() {
        let dir = TempDir::new().unwrap();
        let rule_path = dir.path().join("security.md");
        fs::write(
            &rule_path,
            "---\npaths:\n  - \"src/api/**/*.rs\"\n  - \"src/tools/**/*.rs\"\n---\nValidate all inputs.",
        )
        .unwrap();

        let rule = parse_rule_file(&rule_path, true).unwrap();
        assert_eq!(rule.content, "Validate all inputs.");
        let paths = rule.paths.unwrap();
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0], "src/api/**/*.rs");
        assert_eq!(paths[1], "src/tools/**/*.rs");
        assert!(matches!(rule.source, RuleSource::Project(_)));
    }

    #[test]
    fn parse_rule_file_without_frontmatter_is_unconditional() {
        let dir = TempDir::new().unwrap();
        let rule_path = dir.path().join("style.md");
        fs::write(&rule_path, "Always use snake_case for variables.").unwrap();

        let rule = parse_rule_file(&rule_path, true).unwrap();
        assert_eq!(rule.content, "Always use snake_case for variables.");
        assert!(rule.paths.is_none());
    }

    #[test]
    fn parse_rule_file_with_empty_paths_list() {
        let dir = TempDir::new().unwrap();
        let rule_path = dir.path().join("empty_paths.md");
        fs::write(&rule_path, "---\npaths: []\n---\nSome rule text.").unwrap();

        let rule = parse_rule_file(&rule_path, false).unwrap();
        assert_eq!(rule.content, "Some rule text.");
        // Empty vec is still Some([])
        let paths = rule.paths.unwrap();
        assert!(paths.is_empty());
        assert!(matches!(rule.source, RuleSource::User(_)));
    }

    #[test]
    fn parse_rule_content_no_closing_delimiter() {
        let content = "---\npaths:\n  - \"*.rs\"\nSome body without closing delimiter.";
        let rule = parse_rule_content(content, Path::new("test.md"), true).unwrap();
        // Without closing ---, entire content is treated as body
        assert!(rule.paths.is_none());
        assert_eq!(rule.content, content.trim());
    }

    // ---- load_from_dirs tests ----

    #[test]
    fn load_from_dirs_with_mixed_rule_files() {
        let dir = TempDir::new().unwrap();
        let rules_dir = dir.path().join(".claude").join("rules");
        fs::create_dir_all(&rules_dir).unwrap();

        // Unconditional rule (no frontmatter)
        fs::write(rules_dir.join("style.md"), "Use consistent formatting.").unwrap();

        // Path-scoped rule
        fs::write(
            rules_dir.join("api.md"),
            "---\npaths:\n  - \"src/api/**/*.rs\"\n---\nValidate API inputs.",
        )
        .unwrap();

        // Non-md file should be skipped
        fs::write(rules_dir.join("notes.txt"), "This is not a rule.").unwrap();

        let engine = RuleEngine::load_from_dirs(dir.path()).unwrap();
        assert_eq!(engine.unconditional_rules().len(), 1);
        assert_eq!(
            engine.unconditional_rules()[0].content,
            "Use consistent formatting."
        );
        assert_eq!(engine.path_scoped.len(), 1);
        assert_eq!(engine.path_scoped[0].content, "Validate API inputs.");
    }

    #[test]
    fn load_from_dirs_with_patina_rules() {
        let dir = TempDir::new().unwrap();
        let rules_dir = dir.path().join(".patina").join("rules");
        fs::create_dir_all(&rules_dir).unwrap();

        fs::write(rules_dir.join("custom.md"), "Patina-specific rule.").unwrap();

        let engine = RuleEngine::load_from_dirs(dir.path()).unwrap();
        assert_eq!(engine.unconditional_rules().len(), 1);
        assert_eq!(
            engine.unconditional_rules()[0].content,
            "Patina-specific rule."
        );
    }

    #[test]
    fn load_from_dirs_empty_directories() {
        let dir = TempDir::new().unwrap();
        // No rules directories at all
        let engine = RuleEngine::load_from_dirs(dir.path()).unwrap();
        assert!(engine.unconditional_rules().is_empty());
        assert!(engine.path_scoped.is_empty());
    }

    // ---- rules_for_path tests ----

    #[test]
    fn rules_for_path_matches_correct_rules() {
        let dir = TempDir::new().unwrap();
        let rules_dir = dir.path().join(".claude").join("rules");
        fs::create_dir_all(&rules_dir).unwrap();

        fs::write(
            rules_dir.join("api.md"),
            "---\npaths:\n  - \"src/api/**/*.rs\"\n---\nAPI rule.",
        )
        .unwrap();
        fs::write(
            rules_dir.join("tools.md"),
            "---\npaths:\n  - \"src/tools/**/*.rs\"\n---\nTools rule.",
        )
        .unwrap();

        let engine = RuleEngine::load_from_dirs(dir.path()).unwrap();

        let matches = engine.rules_for_path(Path::new("src/api/client.rs"));
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].content, "API rule.");
    }

    #[test]
    fn rules_for_path_returns_empty_for_non_matching() {
        let dir = TempDir::new().unwrap();
        let rules_dir = dir.path().join(".claude").join("rules");
        fs::create_dir_all(&rules_dir).unwrap();

        fs::write(
            rules_dir.join("api.md"),
            "---\npaths:\n  - \"src/api/**/*.rs\"\n---\nAPI rule.",
        )
        .unwrap();

        let engine = RuleEngine::load_from_dirs(dir.path()).unwrap();

        let matches = engine.rules_for_path(Path::new("src/tui/widgets.rs"));
        assert!(matches.is_empty());
    }

    #[test]
    fn unconditional_rules_returns_only_rules_without_paths() {
        let dir = TempDir::new().unwrap();
        let rules_dir = dir.path().join(".claude").join("rules");
        fs::create_dir_all(&rules_dir).unwrap();

        fs::write(rules_dir.join("always.md"), "Always applies.").unwrap();
        fs::write(
            rules_dir.join("scoped.md"),
            "---\npaths:\n  - \"*.rs\"\n---\nOnly for Rust.",
        )
        .unwrap();

        let engine = RuleEngine::load_from_dirs(dir.path()).unwrap();

        let unconditional = engine.unconditional_rules();
        assert_eq!(unconditional.len(), 1);
        assert_eq!(unconditional[0].content, "Always applies.");
        assert!(unconditional[0].paths.is_none());
    }

    #[test]
    fn rule_source_project_vs_user() {
        let project_path = Path::new("/project/.claude/rules/test.md");
        let user_path = Path::new("/home/user/.claude/rules/test.md");

        let project_source = RuleSource::Project(project_path.to_path_buf());
        let user_source = RuleSource::User(user_path.to_path_buf());

        assert_ne!(project_source, user_source);
        assert_eq!(
            project_source,
            RuleSource::Project(project_path.to_path_buf())
        );
    }

    #[test]
    fn parse_rule_file_multiline_content() {
        let dir = TempDir::new().unwrap();
        let rule_path = dir.path().join("multi.md");
        fs::write(
            &rule_path,
            "---\npaths:\n  - \"**/*.rs\"\n---\nLine one.\n\nLine two.\n\nLine three.",
        )
        .unwrap();

        let rule = parse_rule_file(&rule_path, true).unwrap();
        assert!(rule.content.contains("Line one."));
        assert!(rule.content.contains("Line two."));
        assert!(rule.content.contains("Line three."));
    }

    #[test]
    fn rules_for_path_matches_multiple_rules() {
        let dir = TempDir::new().unwrap();
        let rules_dir = dir.path().join(".claude").join("rules");
        fs::create_dir_all(&rules_dir).unwrap();

        fs::write(
            rules_dir.join("rust.md"),
            "---\npaths:\n  - \"**/*.rs\"\n---\nRust rule.",
        )
        .unwrap();
        fs::write(
            rules_dir.join("api.md"),
            "---\npaths:\n  - \"src/api/**/*.rs\"\n---\nAPI rule.",
        )
        .unwrap();

        let engine = RuleEngine::load_from_dirs(dir.path()).unwrap();

        let matches = engine.rules_for_path(Path::new("src/api/client.rs"));
        assert_eq!(matches.len(), 2);
    }
}
