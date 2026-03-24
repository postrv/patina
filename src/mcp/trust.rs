//! MCP configuration trust store.
//!
//! Tracks which project-local `.mcp.json` files have been explicitly trusted
//! by the user, using SHA-256 content hashes. This prevents untrusted
//! repositories from silently executing MCP server commands.
//!
//! User-global configuration (`~/.claude.json`) is always trusted.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Stores trust decisions for project-local MCP configurations.
///
/// Each entry maps a canonical project path to the SHA-256 hash of the
/// `.mcp.json` content that was approved by the user.
#[derive(Debug, Clone)]
pub struct McpTrustStore {
    /// Map of project directory → SHA-256 hex hash of trusted config content.
    entries: HashMap<String, String>,
    /// Path where the trust store is persisted.
    store_path: PathBuf,
}

impl McpTrustStore {
    /// Loads the trust store from disk.
    ///
    /// Returns an empty store if the file does not exist or cannot be parsed.
    ///
    /// # Arguments
    ///
    /// * `data_dir` - Directory where `mcp_trust.json` is stored.
    #[must_use]
    pub fn load(data_dir: &Path) -> Self {
        let store_path = data_dir.join("mcp_trust.json");
        let entries = if store_path.exists() {
            std::fs::read_to_string(&store_path)
                .ok()
                .and_then(|content| serde_json::from_str(&content).ok())
                .unwrap_or_default()
        } else {
            HashMap::new()
        };

        Self {
            entries,
            store_path,
        }
    }

    /// Creates an empty trust store for testing.
    #[cfg(test)]
    fn empty(store_path: PathBuf) -> Self {
        Self {
            entries: HashMap::new(),
            store_path,
        }
    }

    /// Checks whether the given project config content is trusted.
    ///
    /// # Arguments
    ///
    /// * `project_path` - Canonical path to the project directory.
    /// * `config_content` - The raw content of `.mcp.json`.
    #[must_use]
    pub fn is_trusted(&self, project_path: &Path, config_content: &str) -> bool {
        let key = project_path.to_string_lossy().to_string();
        let hash = Self::hash_content(config_content);
        self.entries.get(&key).is_some_and(|stored| *stored == hash)
    }

    /// Records trust for the given project config content.
    ///
    /// # Arguments
    ///
    /// * `project_path` - Canonical path to the project directory.
    /// * `config_content` - The raw content of `.mcp.json` to trust.
    pub fn trust(&mut self, project_path: &Path, config_content: &str) {
        let key = project_path.to_string_lossy().to_string();
        let hash = Self::hash_content(config_content);
        self.entries.insert(key, hash);
    }

    /// Persists the trust store to disk.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.store_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create dir: {}", parent.display()))?;
        }
        let content = serde_json::to_string_pretty(&self.entries)
            .context("Failed to serialize trust store")?;
        std::fs::write(&self.store_path, content)
            .with_context(|| format!("Failed to write trust store: {}", self.store_path.display()))
    }

    /// Computes the SHA-256 hex digest of config content.
    fn hash_content(content: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        format!("{:x}", hasher.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_trust_store_empty_is_untrusted() {
        let dir = TempDir::new().unwrap();
        let store = McpTrustStore::empty(dir.path().join("mcp_trust.json"));

        assert!(
            !store.is_trusted(Path::new("/some/project"), r#"{"mcpServers": {}}"#),
            "Fresh store should not trust anything"
        );
    }

    #[test]
    fn test_trust_store_trust_and_verify() {
        let dir = TempDir::new().unwrap();
        let mut store = McpTrustStore::empty(dir.path().join("mcp_trust.json"));

        let project = Path::new("/my/project");
        let config = r#"{"mcpServers": {"test": {"command": "echo"}}}"#;

        store.trust(project, config);
        assert!(
            store.is_trusted(project, config),
            "Trusted config should be recognized"
        );
    }

    #[test]
    fn test_trust_store_hash_mismatch() {
        let dir = TempDir::new().unwrap();
        let mut store = McpTrustStore::empty(dir.path().join("mcp_trust.json"));

        let project = Path::new("/my/project");
        let config_v1 = r#"{"mcpServers": {"safe": {"command": "echo"}}}"#;
        let config_v2 = r#"{"mcpServers": {"evil": {"command": "rm -rf /"}}}"#;

        store.trust(project, config_v1);
        assert!(
            !store.is_trusted(project, config_v2),
            "Modified config should not be trusted"
        );
    }

    #[test]
    fn test_trust_store_save_and_load() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path();

        let project = Path::new("/my/project");
        let config = r#"{"mcpServers": {"test": {"command": "echo"}}}"#;

        // Save
        {
            let mut store = McpTrustStore::load(data_dir);
            store.trust(project, config);
            store.save().unwrap();
        }

        // Reload
        {
            let store = McpTrustStore::load(data_dir);
            assert!(
                store.is_trusted(project, config),
                "Trust should persist across load/save"
            );
        }
    }
}
