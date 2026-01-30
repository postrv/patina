//! Project context management (CLAUDE.md support)

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct ProjectContext {
    root_context: Option<String>,
    subdir_contexts: HashMap<PathBuf, String>,
    project_root: PathBuf,
}

impl ProjectContext {
    pub fn new(project_root: PathBuf) -> Self {
        Self {
            root_context: None,
            subdir_contexts: HashMap::new(),
            project_root,
        }
    }

    pub fn load(&mut self) -> anyhow::Result<()> {
        let root_path = self.project_root.join("CLAUDE.md");
        if root_path.exists() {
            self.root_context = Some(std::fs::read_to_string(&root_path)?);
        }

        let rct_path = self.project_root.join(".rct/CLAUDE.md");
        if rct_path.exists() {
            let rct_content = std::fs::read_to_string(&rct_path)?;
            self.root_context = Some(match &self.root_context {
                Some(existing) => format!("{}\n\n{}", existing, rct_content),
                None => rct_content,
            });
        }

        self.walk_for_claude_md(&self.project_root.clone())?;

        Ok(())
    }

    fn walk_for_claude_md(&mut self, dir: &Path) -> anyhow::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.starts_with('.') || name == "node_modules" || name == "target" {
                    continue;
                }

                let claude_md = path.join("CLAUDE.md");
                if claude_md.exists() {
                    let rel_path = path.strip_prefix(&self.project_root)?.to_path_buf();
                    let content = std::fs::read_to_string(&claude_md)?;
                    self.subdir_contexts.insert(rel_path, content);
                }

                self.walk_for_claude_md(&path)?;
            }
        }

        Ok(())
    }

    pub fn get_context(&self, cwd: &Path) -> String {
        let mut context = String::new();

        if let Some(root) = &self.root_context {
            context.push_str(root);
        }

        if let Ok(rel_cwd) = cwd.strip_prefix(&self.project_root) {
            for (subdir, content) in &self.subdir_contexts {
                if rel_cwd.starts_with(subdir) {
                    context.push_str("\n\n## Context: ");
                    context.push_str(&subdir.display().to_string());
                    context.push_str("\n\n");
                    context.push_str(content);
                }
            }
        }

        context
    }
}
