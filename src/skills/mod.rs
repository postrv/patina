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

        let (frontmatter, body) = if content.starts_with("---") {
            let end = content[3..].find("---")
                .map(|i| i + 3)
                .unwrap_or(0);
            let yaml = &content[3..end];
            let body = content[end + 3..].trim();
            (yaml, body)
        } else {
            ("", content.as_str())
        };

        let config: SkillConfig = if !frontmatter.is_empty() {
            serde_yaml::from_str(frontmatter)?
        } else {
            SkillConfig {
                name: path.parent()
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

        self.skills.iter()
            .filter(|skill| {
                if skill.config.triggers.always_active {
                    return true;
                }

                for keyword in &skill.config.triggers.keywords {
                    if task_lower.contains(&keyword.to_lowercase()) {
                        return true;
                    }
                }

                skill.description.to_lowercase()
                    .split_whitespace()
                    .any(|word| task_lower.contains(word))
            })
            .collect()
    }

    pub fn all_skills(&self) -> &[Skill] {
        &self.skills
    }
}

impl Default for SkillEngine {
    fn default() -> Self {
        Self::new()
    }
}
