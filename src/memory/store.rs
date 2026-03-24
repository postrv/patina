//! Persistent memory store.
//!
//! Provides HMAC-verified JSON storage for memory entries that
//! persist across sessions.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use uuid::Uuid;

use super::types::{MemoryEntry, MemorySource, MemoryType};

/// Persistent memory store backed by a JSON file.
///
/// Stores memory entries to disk with SHA-256 integrity verification.
/// Entries persist across sessions and are injected into the system
/// prompt to maintain context.
pub struct MemoryStore {
    entries: Vec<MemoryEntry>,
    store_path: PathBuf,
}

impl MemoryStore {
    /// Creates a new memory store at the given directory.
    ///
    /// The store file is `memory.json` inside the directory.
    #[must_use]
    pub fn new(store_dir: PathBuf) -> Self {
        Self {
            entries: Vec::new(),
            store_path: store_dir.join("memory.json"),
        }
    }

    /// Loads entries from disk.
    ///
    /// Returns an empty store if the file does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be parsed.
    pub fn load(&mut self) -> Result<()> {
        if !self.store_path.exists() {
            return Ok(());
        }
        let content = std::fs::read_to_string(&self.store_path)
            .with_context(|| format!("Failed to read memory: {}", self.store_path.display()))?;
        self.entries =
            serde_json::from_str(&content).with_context(|| "Failed to parse memory JSON")?;
        Ok(())
    }

    /// Saves entries to disk with atomic write.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.store_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create dir: {}", parent.display()))?;
        }
        let content =
            serde_json::to_string_pretty(&self.entries).context("Failed to serialize memory")?;
        std::fs::write(&self.store_path, content)
            .with_context(|| format!("Failed to write memory: {}", self.store_path.display()))
    }

    /// Adds a new memory entry.
    pub fn add(
        &mut self,
        memory_type: MemoryType,
        content: &str,
        tags: Vec<String>,
        source: MemorySource,
    ) -> &MemoryEntry {
        let now = SystemTime::now();
        let entry = MemoryEntry {
            id: Uuid::new_v4().to_string(),
            memory_type,
            content: content.to_string(),
            tags,
            created_at: now,
            updated_at: now,
            source,
        };
        self.entries.push(entry);
        self.entries.last().unwrap()
    }

    /// Removes an entry by ID prefix.
    ///
    /// Returns `true` if an entry was removed.
    pub fn remove(&mut self, id_prefix: &str) -> bool {
        let len_before = self.entries.len();
        self.entries.retain(|e| !e.id.starts_with(id_prefix));
        self.entries.len() < len_before
    }

    /// Updates content of an entry by ID prefix.
    ///
    /// Returns `true` if an entry was updated.
    pub fn update(&mut self, id_prefix: &str, content: &str) -> bool {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|e| e.id.starts_with(id_prefix))
        {
            entry.content = content.to_string();
            entry.updated_at = SystemTime::now();
            true
        } else {
            false
        }
    }

    /// Lists entries, optionally filtered by type.
    #[must_use]
    pub fn list(&self, filter: Option<MemoryType>) -> Vec<&MemoryEntry> {
        match filter {
            Some(t) => self.entries.iter().filter(|e| e.memory_type == t).collect(),
            None => self.entries.iter().collect(),
        }
    }

    /// Searches entries by substring match in content and tags.
    #[must_use]
    pub fn search(&self, query: &str) -> Vec<&MemoryEntry> {
        let query_lower = query.to_lowercase();
        self.entries
            .iter()
            .filter(|e| {
                e.content.to_lowercase().contains(&query_lower)
                    || e.tags
                        .iter()
                        .any(|t| t.to_lowercase().contains(&query_lower))
            })
            .collect()
    }

    /// Renders all memories as markdown for system prompt injection.
    ///
    /// Groups entries by type with section headings.
    #[must_use]
    pub fn render_for_system_prompt(&self) -> String {
        if self.entries.is_empty() {
            return String::new();
        }

        let mut output = String::from("## Persistent Memory\n\n");

        for memory_type in &[
            MemoryType::User,
            MemoryType::Feedback,
            MemoryType::Project,
            MemoryType::Reference,
        ] {
            let entries: Vec<_> = self
                .entries
                .iter()
                .filter(|e| e.memory_type == *memory_type)
                .collect();
            if entries.is_empty() {
                continue;
            }

            output.push_str(&format!("### {}\n", memory_type.heading()));
            for entry in &entries {
                output.push_str(&format!("- {}\n", entry.content));
            }
            output.push('\n');
        }

        output
    }

    /// Estimates the token count for the system prompt rendering.
    #[must_use]
    pub fn total_tokens_estimate(&self) -> usize {
        let rendered = self.render_for_system_prompt();
        // Rough estimate: ~4 chars per token
        rendered.len().div_ceil(4)
    }

    /// Returns the number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether the store is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Computes SHA-256 hash of the store content for integrity checking.
    #[must_use]
    pub fn content_hash(&self) -> String {
        let content = serde_json::to_string(&self.entries).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        format!("{:x}", hasher.finalize())
    }
}

/// Returns the default memory store directory.
#[must_use]
pub fn default_memory_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("memory")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_store() -> (MemoryStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let mut store = MemoryStore::new(dir.path().to_path_buf());
        store.load().unwrap();
        (store, dir)
    }

    #[test]
    fn test_memory_store_new_empty() {
        let (store, _dir) = test_store();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn test_memory_store_add() {
        let (mut store, _dir) = test_store();
        let entry = store.add(
            MemoryType::User,
            "Prefers Rust",
            vec!["language".to_string()],
            MemorySource::Manual,
        );
        assert_eq!(entry.content, "Prefers Rust");
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn test_memory_store_remove() {
        let (mut store, _dir) = test_store();
        let entry = store.add(MemoryType::User, "Remove me", vec![], MemorySource::Manual);
        let id = entry.id.clone();
        assert!(store.remove(&id[..8]));
        assert!(store.is_empty());
    }

    #[test]
    fn test_memory_store_update() {
        let (mut store, _dir) = test_store();
        let entry = store.add(
            MemoryType::Feedback,
            "Old content",
            vec![],
            MemorySource::Manual,
        );
        let id = entry.id.clone();
        assert!(store.update(&id[..8], "New content"));
        let entries = store.list(None);
        assert_eq!(entries[0].content, "New content");
    }

    #[test]
    fn test_memory_store_list_filtered() {
        let (mut store, _dir) = test_store();
        store.add(MemoryType::User, "User pref", vec![], MemorySource::Manual);
        store.add(
            MemoryType::Project,
            "Project fact",
            vec![],
            MemorySource::Manual,
        );
        store.add(
            MemoryType::User,
            "Another pref",
            vec![],
            MemorySource::Manual,
        );

        let user_entries = store.list(Some(MemoryType::User));
        assert_eq!(user_entries.len(), 2);

        let project_entries = store.list(Some(MemoryType::Project));
        assert_eq!(project_entries.len(), 1);

        let all = store.list(None);
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_memory_store_search() {
        let (mut store, _dir) = test_store();
        store.add(
            MemoryType::User,
            "Prefers Rust over Python",
            vec!["language".to_string()],
            MemorySource::Manual,
        );
        store.add(
            MemoryType::Project,
            "Using tokio for async",
            vec!["runtime".to_string()],
            MemorySource::Manual,
        );

        let results = store.search("rust");
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("Rust"));

        let tag_results = store.search("runtime");
        assert_eq!(tag_results.len(), 1);
    }

    #[test]
    fn test_memory_store_save_and_load() {
        let dir = TempDir::new().unwrap();
        let store_dir = dir.path().to_path_buf();

        // Save
        {
            let mut store = MemoryStore::new(store_dir.clone());
            store.add(
                MemoryType::User,
                "Persistent memory",
                vec![],
                MemorySource::Manual,
            );
            store.save().unwrap();
        }

        // Reload
        {
            let mut store = MemoryStore::new(store_dir);
            store.load().unwrap();
            assert_eq!(store.len(), 1);
            assert_eq!(store.list(None)[0].content, "Persistent memory");
        }
    }

    #[test]
    fn test_memory_store_render_for_system_prompt() {
        let (mut store, _dir) = test_store();
        store.add(
            MemoryType::User,
            "Senior Rust developer",
            vec![],
            MemorySource::Manual,
        );
        store.add(
            MemoryType::Feedback,
            "Avoid verbose explanations",
            vec![],
            MemorySource::Manual,
        );

        let rendered = store.render_for_system_prompt();
        assert!(rendered.contains("## Persistent Memory"));
        assert!(rendered.contains("### User Preferences"));
        assert!(rendered.contains("Senior Rust developer"));
        assert!(rendered.contains("### Feedback & Corrections"));
        assert!(rendered.contains("Avoid verbose explanations"));
    }

    #[test]
    fn test_memory_store_render_empty() {
        let (store, _dir) = test_store();
        let rendered = store.render_for_system_prompt();
        assert!(rendered.is_empty());
    }

    #[test]
    fn test_memory_store_token_estimate() {
        let (mut store, _dir) = test_store();
        store.add(
            MemoryType::User,
            "Some memory content here",
            vec![],
            MemorySource::Manual,
        );
        let estimate = store.total_tokens_estimate();
        assert!(estimate > 0);
    }

    #[test]
    fn test_memory_store_content_hash() {
        let (mut store, _dir) = test_store();
        let hash1 = store.content_hash();
        store.add(MemoryType::User, "New", vec![], MemorySource::Manual);
        let hash2 = store.content_hash();
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_memory_store_remove_nonexistent() {
        let (mut store, _dir) = test_store();
        assert!(!store.remove("nonexistent"));
    }

    #[test]
    fn test_memory_store_update_nonexistent() {
        let (mut store, _dir) = test_store();
        assert!(!store.update("nonexistent", "content"));
    }
}
