//! Slash commands - user-triggered workflows

use serde::Deserialize;
use std::path::PathBuf;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub args: Vec<CommandArg>,
    #[serde(skip)]
    pub content: String,
    #[serde(skip)]
    pub source_path: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommandArg {
    pub name: String,
    #[serde(default = "default_arg_type")]
    pub arg_type: String,
    #[serde(default)]
    pub required: bool,
    pub default: Option<String>,
    #[serde(default)]
    pub choices: Vec<String>,
}

fn default_arg_type() -> String {
    "string".to_string()
}

pub struct CommandExecutor {
    commands: HashMap<String, SlashCommand>,
}

impl CommandExecutor {
    pub fn new() -> Self {
        Self {
            commands: HashMap::new(),
        }
    }

    pub fn load_from_dir(&mut self, dir: &PathBuf) -> anyhow::Result<()> {
        if !dir.exists() {
            return Ok(());
        }

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().map_or(false, |e| e == "md") {
                if let Ok(cmd) = self.parse_command_file(&path) {
                    self.commands.insert(cmd.name.clone(), cmd);
                }
            }
        }

        Ok(())
    }

    fn parse_command_file(&self, path: &PathBuf) -> anyhow::Result<SlashCommand> {
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

        let mut cmd: SlashCommand = if !frontmatter.is_empty() {
            serde_yaml::from_str(frontmatter)?
        } else {
            SlashCommand {
                name: path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string(),
                description: String::new(),
                args: Vec::new(),
                content: String::new(),
                source_path: PathBuf::new(),
            }
        };

        cmd.content = body.to_string();
        cmd.source_path = path.clone();

        Ok(cmd)
    }

    pub fn execute(
        &self,
        name: &str,
        args: HashMap<String, String>,
    ) -> anyhow::Result<String> {
        let cmd = self.commands.get(name)
            .ok_or_else(|| anyhow::anyhow!("Unknown command: /{}", name))?;

        let mut result = cmd.content.clone();
        for (key, value) in args {
            result = result.replace(&format!("{{{{ {} }}}}", key), &value);
        }

        Ok(result)
    }

    pub fn list(&self) -> Vec<(&str, &str)> {
        self.commands.iter()
            .map(|(name, cmd)| (name.as_str(), cmd.description.as_str()))
            .collect()
    }
}

impl Default for CommandExecutor {
    fn default() -> Self {
        Self::new()
    }
}
