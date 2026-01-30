//! Plugin discovery, loading, and management.
//!
//! This module provides:
//! - Plugin discovery from filesystem
//! - Plugin registry for managing loaded plugins
//! - Host API traits for plugin development

pub mod host;

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
        let manifest: PluginManifest =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path)?)?;

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

    /// Checks if a plugin is loaded.
    #[must_use]
    pub fn has_plugin(&self, name: &str) -> bool {
        self.plugins.contains_key(name)
    }

    /// Returns the number of loaded plugins.
    #[must_use]
    pub fn plugin_count(&self) -> usize {
        self.plugins.len()
    }

    /// Unloads a plugin and removes its commands and skills.
    ///
    /// Returns `true` if the plugin was found and unloaded, `false` otherwise.
    pub fn unload_plugin(&mut self, name: &str) -> bool {
        if self.plugins.remove(name).is_some() {
            // Remove commands from this plugin
            self.commands
                .retain(|_, (plugin_name, _)| plugin_name != name);

            // Remove skills from this plugin
            self.skills.retain(|(plugin_name, _)| plugin_name != name);

            true
        } else {
            false
        }
    }

    /// Reloads a plugin from its directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin cannot be loaded from the path.
    pub fn reload_plugin(&mut self, name: &str, plugin_dir: &Path) -> Result<()> {
        self.unload_plugin(name);

        let plugin = self.load_plugin(plugin_dir)?;
        let loaded_name = plugin.manifest.name.clone();

        for cmd in &plugin.commands {
            let key = format!("{}:{}", loaded_name, cmd.name);
            self.commands
                .insert(key, (loaded_name.clone(), cmd.clone()));
        }

        for skill in &plugin.skills {
            self.skills.push((loaded_name.clone(), skill.clone()));
        }

        self.plugins.insert(loaded_name, plugin);
        Ok(())
    }

    /// Returns the manifest for a plugin.
    #[must_use]
    pub fn get_manifest(&self, name: &str) -> Option<&PluginManifest> {
        self.plugins.get(name).map(|p| &p.manifest)
    }

    /// Returns the path for a plugin.
    #[must_use]
    pub fn get_plugin_path(&self, name: &str) -> Option<&Path> {
        self.plugins.get(name).map(|p| p.path.as_path())
    }

    /// Returns a list of all loaded plugin names.
    #[must_use]
    pub fn list_plugins(&self) -> Vec<String> {
        self.plugins.keys().cloned().collect()
    }

    /// Returns a list of all registered command names (fully qualified).
    #[must_use]
    pub fn list_commands(&self) -> Vec<String> {
        self.commands.keys().cloned().collect()
    }

    /// Returns the plugin name that provides a command.
    #[must_use]
    pub fn get_command_plugin(&self, name: &str) -> Option<String> {
        // Try exact match first
        if let Some((plugin_name, _)) = self.commands.get(name) {
            return Some(plugin_name.clone());
        }

        // Try short name match
        for (key, (plugin_name, _)) in &self.commands {
            if key.ends_with(&format!(":{}", name)) {
                return Some(plugin_name.clone());
            }
        }

        None
    }

    /// Returns the total number of registered commands.
    #[must_use]
    pub fn command_count(&self) -> usize {
        self.commands.len()
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
