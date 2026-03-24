//! Persistent task management for tracking work items.
//!
//! Provides a simple JSON-backed todo store that persists across sessions.
//! Used by Claude to track implementation progress, quality gates, and
//! pending work items.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Priority level for a todo item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    /// High priority — should be done first.
    High,
    /// Medium priority — default.
    #[default]
    Medium,
    /// Low priority — can be deferred.
    Low,
}

/// A single todo item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    /// Unique identifier (UUID).
    pub id: String,
    /// The task content.
    pub content: String,
    /// Whether the task is completed.
    pub completed: bool,
    /// Priority level.
    pub priority: Priority,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
}

/// Persistent todo store backed by a JSON file.
pub struct TodoStore {
    path: PathBuf,
}

impl TodoStore {
    /// Creates a new todo store at the given path.
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Loads all todo items from disk.
    ///
    /// Returns an empty list if the file does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be parsed.
    pub fn load(&self) -> Result<Vec<TodoItem>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let content = std::fs::read_to_string(&self.path)
            .with_context(|| format!("Failed to read todos from {}", self.path.display()))?;
        let items: Vec<TodoItem> =
            serde_json::from_str(&content).with_context(|| "Failed to parse todos JSON")?;
        Ok(items)
    }

    /// Saves todo items to disk.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn save(&self, items: &[TodoItem]) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create dir: {}", parent.display()))?;
        }
        let content = serde_json::to_string_pretty(items).context("Failed to serialize todos")?;
        std::fs::write(&self.path, content)
            .with_context(|| format!("Failed to write todos to {}", self.path.display()))
    }

    /// Adds a new todo item with the given content and priority.
    ///
    /// # Errors
    ///
    /// Returns an error if the store cannot be loaded or saved.
    pub fn add(&self, content: &str, priority: Priority) -> Result<TodoItem> {
        let mut items = self.load()?;
        let item = TodoItem {
            id: Uuid::new_v4().to_string(),
            content: content.to_string(),
            completed: false,
            priority,
            created_at: chrono_now_iso8601(),
        };
        items.push(item.clone());
        self.save(&items)?;
        Ok(item)
    }

    /// Marks a todo item as completed by ID prefix.
    ///
    /// # Errors
    ///
    /// Returns an error if the item is not found or the store cannot be saved.
    pub fn complete(&self, id_prefix: &str) -> Result<()> {
        let mut items = self.load()?;
        let found = items.iter_mut().find(|item| item.id.starts_with(id_prefix));
        match found {
            Some(item) => {
                item.completed = true;
                self.save(&items)?;
                Ok(())
            }
            None => anyhow::bail!("Todo item not found with ID prefix: {id_prefix}"),
        }
    }

    /// Removes a todo item by ID prefix.
    ///
    /// # Errors
    ///
    /// Returns an error if the item is not found or the store cannot be saved.
    pub fn remove(&self, id_prefix: &str) -> Result<()> {
        let mut items = self.load()?;
        let len_before = items.len();
        items.retain(|item| !item.id.starts_with(id_prefix));
        if items.len() == len_before {
            anyhow::bail!("Todo item not found with ID prefix: {id_prefix}");
        }
        self.save(&items)?;
        Ok(())
    }

    /// Lists all todo items, optionally filtered by completion status.
    ///
    /// # Errors
    ///
    /// Returns an error if the store cannot be loaded.
    pub fn list(&self, completed: Option<bool>) -> Result<Vec<TodoItem>> {
        let items = self.load()?;
        Ok(match completed {
            Some(c) => items.into_iter().filter(|i| i.completed == c).collect(),
            None => items,
        })
    }

    /// Formats the todo list as a human-readable string.
    ///
    /// # Errors
    ///
    /// Returns an error if the store cannot be loaded.
    pub fn format_list(&self) -> Result<String> {
        let items = self.load()?;
        if items.is_empty() {
            return Ok("No todo items.".to_string());
        }

        let mut output = String::new();
        for item in &items {
            let check = if item.completed { "x" } else { " " };
            let priority = match item.priority {
                Priority::High => "HIGH",
                Priority::Medium => "MED ",
                Priority::Low => "LOW ",
            };
            let id_short = &item.id[..8];
            output.push_str(&format!(
                "[{check}] {priority} {id_short} {}\n",
                item.content
            ));
        }
        Ok(output)
    }
}

/// Returns the current time as an ISO 8601 string.
fn chrono_now_iso8601() -> String {
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}s", now.as_secs())
}

/// Creates a default todo store path for the given data directory.
#[must_use]
pub fn default_todo_path(data_dir: &Path) -> PathBuf {
    data_dir.join("todos.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_store() -> (TodoStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = TodoStore::new(dir.path().join("todos.json"));
        (store, dir)
    }

    #[test]
    fn test_todo_store_empty() {
        let (store, _dir) = test_store();
        let items = store.load().unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn test_todo_store_add() {
        let (store, _dir) = test_store();
        let item = store.add("Write tests", Priority::High).unwrap();
        assert_eq!(item.content, "Write tests");
        assert_eq!(item.priority, Priority::High);
        assert!(!item.completed);

        let items = store.load().unwrap();
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn test_todo_store_complete() {
        let (store, _dir) = test_store();
        let item = store.add("Do thing", Priority::Medium).unwrap();
        store.complete(&item.id[..8]).unwrap();

        let items = store.load().unwrap();
        assert!(items[0].completed);
    }

    #[test]
    fn test_todo_store_remove() {
        let (store, _dir) = test_store();
        let item = store.add("Remove me", Priority::Low).unwrap();
        store.remove(&item.id[..8]).unwrap();

        let items = store.load().unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn test_todo_store_list_filtered() {
        let (store, _dir) = test_store();
        store.add("Task 1", Priority::High).unwrap();
        let item2 = store.add("Task 2", Priority::Medium).unwrap();
        store.complete(&item2.id[..8]).unwrap();

        let pending = store.list(Some(false)).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].content, "Task 1");

        let done = store.list(Some(true)).unwrap();
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].content, "Task 2");
    }

    #[test]
    fn test_todo_store_persistence() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("todos.json");

        // Save
        {
            let store = TodoStore::new(path.clone());
            store.add("Persistent task", Priority::High).unwrap();
        }

        // Reload
        {
            let store = TodoStore::new(path);
            let items = store.load().unwrap();
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].content, "Persistent task");
        }
    }

    #[test]
    fn test_todo_item_serialization() {
        let item = TodoItem {
            id: "test-id".to_string(),
            content: "Do something".to_string(),
            completed: false,
            priority: Priority::High,
            created_at: "1234s".to_string(),
        };
        let json = serde_json::to_string(&item).unwrap();
        let deserialized: TodoItem = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.content, "Do something");
        assert_eq!(deserialized.priority, Priority::High);
    }

    #[test]
    fn test_priority_default_is_medium() {
        assert_eq!(Priority::default(), Priority::Medium);
    }

    #[test]
    fn test_format_list_empty() {
        let (store, _dir) = test_store();
        let formatted = store.format_list().unwrap();
        assert_eq!(formatted, "No todo items.");
    }

    #[test]
    fn test_format_list_with_items() {
        let (store, _dir) = test_store();
        store.add("Task A", Priority::High).unwrap();
        let formatted = store.format_list().unwrap();
        assert!(formatted.contains("HIGH"));
        assert!(formatted.contains("Task A"));
        assert!(formatted.contains("[ ]"));
    }

    #[test]
    fn test_remove_nonexistent_fails() {
        let (store, _dir) = test_store();
        assert!(store.remove("nonexistent").is_err());
    }

    #[test]
    fn test_complete_nonexistent_fails() {
        let (store, _dir) = test_store();
        assert!(store.complete("nonexistent").is_err());
    }
}
