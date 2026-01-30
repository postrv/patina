//! Skills system - auto-invoked context providers

use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct SkillConfig {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub triggers: SkillTriggers,
}

#[derive(Debug, Default, Deserialize)]
pub struct SkillTriggers {
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub file_patterns: Vec<String>,
    #[serde(default)]
    pub always_active: bool,
}

#[derive(Debug)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub config: SkillConfig,
    pub source_path: PathBuf,
}

pub struct SkillEngine {
    skills: Vec<Skill>,
}

impl SkillEngine {
    pub fn new() -> Self {
        Self { skills: Vec::new() }
    }

    pub fn load_from_dir(&mut self, dir: &PathBuf) -> anyhow::Result<()> {
        if !dir.exists() {
            return Ok(());
        }

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let skill_md = entry.path().join("SKILL.md");

            if skill_md.exists() {
                if let Ok(skill) = self.parse_skill_file(&skill_md) {
                    self.skills.push(skill);
                }
            }
        }

        Ok(())
    }

    fn parse_skill_file(&self, path: &PathBuf) -> anyhow::Result<Skill> {
        let content = std::fs::read_to_string(path)?;

        let (frontmatter, body) = if let Some(after_open) = content.strip_prefix("---") {
            let end = after_open.find("---").unwrap_or(after_open.len());
            let yaml = &after_open[..end];
            let body = after_open[end..].strip_prefix("---").unwrap_or("").trim();
            (yaml, body)
        } else {
            ("", content.as_str())
        };

        let config: SkillConfig = if !frontmatter.is_empty() {
            serde_yaml::from_str(frontmatter)?
        } else {
            SkillConfig {
                name: path
                    .parent()
                    .and_then(|p| p.file_name())
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string(),
                description: String::new(),
                allowed_tools: Vec::new(),
                triggers: SkillTriggers::default(),
            }
        };

        Ok(Skill {
            name: config.name.clone(),
            description: config.description.clone(),
            instructions: body.to_string(),
            config,
            source_path: path.clone(),
        })
    }

    pub fn match_skills(&self, task_description: &str) -> Vec<&Skill> {
        let task_lower = task_description.to_lowercase();

        self.skills
            .iter()
            .filter(|skill| {
                if skill.config.triggers.always_active {
                    return true;
                }

                for keyword in &skill.config.triggers.keywords {
                    if task_lower.contains(&keyword.to_lowercase()) {
                        return true;
                    }
                }

                skill
                    .description
                    .to_lowercase()
                    .split_whitespace()
                    .any(|word| task_lower.contains(word))
            })
            .collect()
    }

    pub fn all_skills(&self) -> &[Skill] {
        &self.skills
    }

    /// Matches skills based on file path patterns.
    ///
    /// Returns skills whose file_patterns match the given file path.
    ///
    /// # Arguments
    ///
    /// * `file_path` - The file path to match against skill file patterns
    ///
    /// # Examples
    ///
    /// ```
    /// use rct::skills::SkillEngine;
    ///
    /// let engine = SkillEngine::new();
    /// let matches = engine.match_skills_for_file("src/main.rs");
    /// ```
    #[must_use]
    pub fn match_skills_for_file(&self, file_path: &str) -> Vec<&Skill> {
        use glob::Pattern;
        use std::path::Path;

        let file_path = Path::new(file_path);
        let file_name = file_path.file_name().and_then(|s| s.to_str());

        self.skills
            .iter()
            .filter(|skill| {
                for pattern_str in &skill.config.triggers.file_patterns {
                    if let Ok(pattern) = Pattern::new(pattern_str) {
                        // Try matching against full path
                        if pattern.matches_path(file_path) {
                            return true;
                        }
                        // Try matching against just the file name
                        if let Some(name) = file_name {
                            if pattern.matches(name) {
                                return true;
                            }
                        }
                    }
                }
                false
            })
            .collect()
    }

    /// Generates context string from skills matched by task description.
    ///
    /// Returns a formatted string containing instructions from all matched skills,
    /// suitable for injection into a conversation context.
    ///
    /// # Arguments
    ///
    /// * `task_description` - The task description to match against skill triggers
    ///
    /// # Examples
    ///
    /// ```
    /// use rct::skills::SkillEngine;
    ///
    /// let engine = SkillEngine::new();
    /// let context = engine.get_context_for_task("Help me write rust code");
    /// ```
    #[must_use]
    pub fn get_context_for_task(&self, task_description: &str) -> String {
        let matched_skills = self.match_skills(task_description);
        self.format_context(&matched_skills)
    }

    /// Generates context string from skills matched by file path.
    ///
    /// Returns a formatted string containing instructions from all matched skills
    /// based on file pattern matching.
    ///
    /// # Arguments
    ///
    /// * `file_path` - The file path to match against skill file patterns
    ///
    /// # Examples
    ///
    /// ```
    /// use rct::skills::SkillEngine;
    ///
    /// let engine = SkillEngine::new();
    /// let context = engine.get_context_for_file("Cargo.toml");
    /// ```
    #[must_use]
    pub fn get_context_for_file(&self, file_path: &str) -> String {
        let matched_skills = self.match_skills_for_file(file_path);
        self.format_context(&matched_skills)
    }

    /// Formats a list of matched skills into a context string.
    fn format_context(&self, skills: &[&Skill]) -> String {
        if skills.is_empty() {
            return String::new();
        }

        let mut context = String::new();

        for skill in skills {
            context.push_str(&format!("## Skill: {}\n\n", skill.name));

            if !skill.config.allowed_tools.is_empty() {
                context.push_str("**Allowed tools:** ");
                context.push_str(&skill.config.allowed_tools.join(", "));
                context.push_str("\n\n");
            }

            context.push_str(&skill.instructions);
            context.push_str("\n\n");
        }

        context.trim_end().to_string()
    }
}

impl Default for SkillEngine {
    fn default() -> Self {
        Self::new()
    }
}
