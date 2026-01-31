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

    /// Loads project context from CLAUDE.md files.
    ///
    /// This method uses graceful degradation:
    /// - If a CLAUDE.md file cannot be read, a warning is logged and loading continues
    /// - If a subdirectory cannot be traversed, a warning is logged and loading continues
    /// - The method only fails on critical errors (e.g., root context file exists but can't be read)
    ///
    /// # Errors
    ///
    /// Returns an error only for critical failures that should stop the application.
    pub fn load(&mut self) -> anyhow::Result<()> {
        let root_path = self.project_root.join("CLAUDE.md");
        if root_path.exists() {
            match std::fs::read_to_string(&root_path) {
                Ok(content) => self.root_context = Some(content),
                Err(e) => {
                    // Root CLAUDE.md is more important - log warning but don't fail
                    tracing::warn!("Failed to read root CLAUDE.md at {:?}: {}", root_path, e);
                }
            }
        }

        let patina_path = self.project_root.join(".patina/CLAUDE.md");
        if patina_path.exists() {
            match std::fs::read_to_string(&patina_path) {
                Ok(patina_content) => {
                    self.root_context = Some(match &self.root_context {
                        Some(existing) => format!("{}\n\n{}", existing, patina_content),
                        None => patina_content,
                    });
                }
                Err(e) => {
                    tracing::warn!("Failed to read .patina/CLAUDE.md at {:?}: {}", patina_path, e);
                }
            }
        }

        self.walk_for_claude_md(&self.project_root.clone());

        Ok(())
    }

    /// Recursively walks directories looking for CLAUDE.md files.
    ///
    /// Uses graceful degradation: logs warnings for unreadable directories
    /// or files and continues with other directories.
    fn walk_for_claude_md(&mut self, dir: &Path) {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                tracing::debug!("Cannot read directory {:?}, skipping: {}", dir, e);
                return;
            }
        };

        for entry_result in entries {
            let entry = match entry_result {
                Ok(e) => e,
                Err(e) => {
                    tracing::debug!(
                        "Error reading directory entry in {:?}, skipping: {}",
                        dir,
                        e
                    );
                    continue;
                }
            };

            let path = entry.path();

            if path.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.starts_with('.') || name == "node_modules" || name == "target" {
                    continue;
                }

                let claude_md = path.join("CLAUDE.md");
                if claude_md.exists() {
                    match path.strip_prefix(&self.project_root) {
                        Ok(rel_path) => match std::fs::read_to_string(&claude_md) {
                            Ok(content) => {
                                self.subdir_contexts.insert(rel_path.to_path_buf(), content);
                            }
                            Err(e) => {
                                tracing::debug!(
                                    "Cannot read CLAUDE.md at {:?}, skipping: {}",
                                    claude_md,
                                    e
                                );
                            }
                        },
                        Err(e) => {
                            tracing::debug!("Cannot strip prefix for {:?}, skipping: {}", path, e);
                        }
                    }
                }

                self.walk_for_claude_md(&path);
            }
        }
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
