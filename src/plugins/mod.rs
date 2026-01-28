//! Plugin discovery, loading, and management

use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub min_rct_version: Option<String>,
}

#[derive(Debug)]
pub struct Plugin {
    pub manifest: PluginManifest,
    pub path: PathBuf,
    pub commands: Vec<Command>,
    pub skills: Vec<Skill>,
    pub hooks: HooksConfig,
    pub agents: Vec<Agent>,
}

#[derive(Debug, Clone)]
pub struct Command {
    pub name: String,
    pub description: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub instructions: String,
}

#[derive(Debug, Clone)]
pub struct Agent {
    pub name: String,
    pub description: String,
    pub system_prompt: String,
}

#[derive(Debug, Default)]
pub struct HooksConfig {
    pub pre_tool_use: Vec<HookDef>,
    pub post_tool_use: Vec<HookDef>,
    pub session_start: Vec<HookDef>,
    pub session_end: Vec<HookDef>,
    pub user_prompt_submit: Vec<HookDef>,
    pub notification: Vec<HookDef>,
    pub stop: Vec<HookDef>,
    pub subagent_stop: Vec<HookDef>,
    pub pre_compact: Vec<HookDef>,
    pub permission_request: Vec<HookDef>,
}

#[derive(Debug, Clone)]
pub struct HookDef {
    pub matcher: Option<String>,
    pub command: String,
}

pub struct PluginRegistry {
    plugins: HashMap<String, Plugin>,
    commands: HashMap<String, (String, Command)>,
    skills: Vec<(String, Skill)>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
            commands: HashMap::new(),
            skills: Vec::new(),
        }
    }

    pub fn load_all(&mut self, search_paths: &[PathBuf]) -> Result<()> {
        for path in search_paths {
            self.discover_plugins(path)?;
        }
        Ok(())
    }

    fn discover_plugins(&mut self, dir: &Path) -> Result<()> {
        if !dir.exists() {
            return Ok(());
        }

        for entry in WalkDir::new(dir).max_depth(2) {
            let entry = entry?;
            let manifest_path = entry.path().join(".claude-plugin/plugin.json");

            if manifest_path.exists() {
                if let Ok(plugin) = self.load_plugin(entry.path()) {
                    let name = plugin.manifest.name.clone();

                    for cmd in &plugin.commands {
                        let key = format!("{}:{}", name, cmd.name);
                        self.commands.insert(key, (name.clone(), cmd.clone()));
                    }

                    for skill in &plugin.skills {
                        self.skills.push((name.clone(), skill.clone()));
                    }

                    self.plugins.insert(name, plugin);
                }
            }
        }

        Ok(())
    }

    fn load_plugin(&self, plugin_dir: &Path) -> Result<Plugin> {
        let manifest_path = plugin_dir.join(".claude-plugin/plugin.json");
        let manifest: PluginManifest = serde_json::from_str(
            &std::fs::read_to_string(&manifest_path)?
        )?;

        let commands = self.load_commands(plugin_dir)?;
        let skills = self.load_skills(plugin_dir)?;
        let agents = self.load_agents(plugin_dir)?;
        let hooks = self.load_hooks(plugin_dir)?;

        Ok(Plugin {
            manifest,
            path: plugin_dir.to_path_buf(),
            commands,
            skills,
            hooks,
            agents,
        })
    }

    fn load_commands(&self, plugin_dir: &Path) -> Result<Vec<Command>> {
        let commands_dir = plugin_dir.join("commands");
        let mut commands = Vec::new();

        if commands_dir.exists() {
            for entry in std::fs::read_dir(commands_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().map(|e| e == "md").unwrap_or(false) {
                    let name = path.file_stem().unwrap().to_string_lossy().to_string();
                    let content = std::fs::read_to_string(&path)?;
                    commands.push(Command {
                        name,
                        description: None,
                        content,
                    });
                }
            }
        }

        Ok(commands)
    }

    fn load_skills(&self, plugin_dir: &Path) -> Result<Vec<Skill>> {
        let skills_dir = plugin_dir.join("skills");
        let mut skills = Vec::new();

        if skills_dir.exists() {
            for entry in std::fs::read_dir(skills_dir)? {
                let entry = entry?;
                let skill_md = entry.path().join("SKILL.md");
                if skill_md.exists() {
                    let content = std::fs::read_to_string(&skill_md)?;
                    if let Some(skill) = parse_skill_md(&content) {
                        skills.push(skill);
                    }
                }
            }
        }

        Ok(skills)
    }

    fn load_agents(&self, _plugin_dir: &Path) -> Result<Vec<Agent>> {
        Ok(Vec::new())
    }

    fn load_hooks(&self, plugin_dir: &Path) -> Result<HooksConfig> {
        let hooks_json = plugin_dir.join("hooks/hooks.json");
        if hooks_json.exists() {
            let _content = std::fs::read_to_string(&hooks_json)?;
        }
        Ok(HooksConfig::default())
    }

    pub fn get_command(&self, name: &str) -> Option<&Command> {
        if let Some((_, cmd)) = self.commands.get(name) {
            return Some(cmd);
        }

        for (key, (_, cmd)) in &self.commands {
            if key.ends_with(&format!(":{}", name)) {
                return Some(cmd);
            }
        }

        None
    }

    pub fn all_skills(&self) -> impl Iterator<Item = &Skill> {
        self.skills.iter().map(|(_, s)| s)
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_skill_md(content: &str) -> Option<Skill> {
    let parts: Vec<&str> = content.splitn(3, "---").collect();
    if parts.len() < 3 {
        return None;
    }

    #[derive(Deserialize)]
    struct Frontmatter {
        name: String,
        description: String,
    }

    let frontmatter: Frontmatter = serde_yaml::from_str(parts[1].trim()).ok()?;

    Some(Skill {
        name: frontmatter.name,
        description: frontmatter.description,
        instructions: parts[2].trim().to_string(),
    })
}
