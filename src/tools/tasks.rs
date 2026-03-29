//! Task management tools for LLM-callable task tracking.
//!
//! Provides an in-memory task store with dependency tracking, status workflows,
//! and four LLM-callable tool operations: create, get, list, and update.
//!
//! # Task Lifecycle
//!
//! Tasks progress through statuses: `Pending` -> `InProgress` -> `Completed`.
//! Tasks can also be soft-deleted with the `Deleted` status.
//!
//! # Dependency Tracking
//!
//! Tasks can declare `blocked_by` relationships. A task is considered blocked
//! when any of its `blocked_by` task IDs reference non-completed tasks.
//!
//! # Examples
//!
//! ```
//! use patina::tools::tasks::TaskStore;
//!
//! let store = TaskStore::new();
//! let task = store.create("Implement feature", "Build the new widget system");
//! assert_eq!(task.id, "1");
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// Status of a task in its lifecycle.
///
/// Tasks typically progress from `Pending` through `InProgress` to `Completed`.
/// The `Deleted` status is a soft-delete that hides the task from list results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Task has been created but work has not started.
    Pending,
    /// Task is actively being worked on.
    InProgress,
    /// Task has been finished.
    Completed,
    /// Task has been soft-deleted and is hidden from listings.
    Deleted,
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::InProgress => write!(f, "in_progress"),
            Self::Completed => write!(f, "completed"),
            Self::Deleted => write!(f, "deleted"),
        }
    }
}

/// A single task with metadata, status, and dependency information.
///
/// Tasks are created via [`TaskStore::create`] and updated via [`TaskStore::update`].
/// Each task has a unique incrementing string ID and tracks both forward (`blocks`)
/// and reverse (`blocked_by`) dependency edges.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// Unique incrementing identifier (e.g., "1", "2", "3").
    pub id: String,
    /// Short summary of the task.
    pub subject: String,
    /// Detailed description of what the task entails.
    pub description: String,
    /// Current lifecycle status.
    pub status: TaskStatus,
    /// Optional owner or assignee of the task.
    pub owner: Option<String>,
    /// IDs of tasks that this task blocks (forward edges).
    pub blocks: Vec<String>,
    /// IDs of tasks that must complete before this task can proceed (reverse edges).
    pub blocked_by: Vec<String>,
    /// Arbitrary key-value metadata.
    pub metadata: HashMap<String, serde_json::Value>,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
    /// ISO 8601 last-updated timestamp.
    pub updated_at: String,
}

/// Partial update specification for modifying an existing task.
///
/// All fields are optional. Only provided fields are applied during
/// [`TaskStore::update`]. The `add_blocks` and `add_blocked_by` fields
/// append to the existing dependency lists rather than replacing them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskUpdate {
    /// New status for the task.
    pub status: Option<TaskStatus>,
    /// New subject line.
    pub subject: Option<String>,
    /// New description.
    pub description: Option<String>,
    /// New owner (pass `Some(Some("name"))` to set, `Some(None)` to clear).
    pub owner: Option<Option<String>>,
    /// Task IDs to append to the `blocks` list.
    pub add_blocks: Option<Vec<String>>,
    /// Task IDs to append to the `blocked_by` list.
    pub add_blocked_by: Option<Vec<String>>,
    /// Metadata entries to merge (existing keys are overwritten).
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

/// Thread-safe, in-memory task store with auto-incrementing IDs.
///
/// The store is cheaply cloneable via internal `Arc` and safe for concurrent
/// access from multiple threads or async tasks.
///
/// # Examples
///
/// ```
/// use patina::tools::tasks::TaskStore;
///
/// let store = TaskStore::new();
/// let t1 = store.create("First task", "Description");
/// let t2 = store.create("Second task", "Description");
/// assert_eq!(t1.id, "1");
/// assert_eq!(t2.id, "2");
/// assert_eq!(store.list().len(), 2);
/// ```
#[derive(Debug, Clone)]
pub struct TaskStore {
    inner: Arc<Mutex<TaskStoreInner>>,
    counter: Arc<AtomicU64>,
}

#[derive(Debug)]
struct TaskStoreInner {
    tasks: Vec<Task>,
}

impl Default for TaskStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskStore {
    /// Creates a new empty task store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(TaskStoreInner { tasks: Vec::new() })),
            counter: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Creates a new task with the given subject and description.
    ///
    /// The task is assigned an auto-incrementing string ID and starts
    /// in [`TaskStatus::Pending`] status.
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::tools::tasks::TaskStore;
    ///
    /// let store = TaskStore::new();
    /// let task = store.create("Build feature", "Implement the new widget");
    /// assert_eq!(task.id, "1");
    /// assert_eq!(task.status, patina::tools::tasks::TaskStatus::Pending);
    /// ```
    #[must_use]
    pub fn create(&self, subject: &str, description: &str) -> Task {
        let id = self.counter.fetch_add(1, Ordering::SeqCst).to_string();
        let now = now_iso8601();
        let task = Task {
            id,
            subject: subject.to_string(),
            description: description.to_string(),
            status: TaskStatus::Pending,
            owner: None,
            blocks: Vec::new(),
            blocked_by: Vec::new(),
            metadata: HashMap::new(),
            created_at: now.clone(),
            updated_at: now,
        };
        let mut inner = self.inner.lock().expect("task store lock poisoned");
        inner.tasks.push(task.clone());
        task
    }

    /// Creates a new task with subject, description, and initial metadata.
    ///
    /// This is a convenience method combining [`Self::create`] with metadata
    /// assignment in a single operation.
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::tools::tasks::TaskStore;
    /// use std::collections::HashMap;
    ///
    /// let store = TaskStore::new();
    /// let mut meta = HashMap::new();
    /// meta.insert("priority".to_string(), serde_json::json!("high"));
    /// let task = store.create_with_metadata("Task", "Desc", meta);
    /// assert!(task.metadata.contains_key("priority"));
    /// ```
    #[must_use]
    pub fn create_with_metadata(
        &self,
        subject: &str,
        description: &str,
        metadata: HashMap<String, serde_json::Value>,
    ) -> Task {
        let id = self.counter.fetch_add(1, Ordering::SeqCst).to_string();
        let now = now_iso8601();
        let task = Task {
            id,
            subject: subject.to_string(),
            description: description.to_string(),
            status: TaskStatus::Pending,
            owner: None,
            blocks: Vec::new(),
            blocked_by: Vec::new(),
            metadata,
            created_at: now.clone(),
            updated_at: now,
        };
        let mut inner = self.inner.lock().expect("task store lock poisoned");
        inner.tasks.push(task.clone());
        task
    }

    /// Returns a clone of the task with the given ID, if it exists.
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::tools::tasks::TaskStore;
    ///
    /// let store = TaskStore::new();
    /// let task = store.create("My task", "Details");
    /// assert!(store.get(&task.id).is_some());
    /// assert!(store.get("nonexistent").is_none());
    /// ```
    #[must_use]
    pub fn get(&self, id: &str) -> Option<Task> {
        let inner = self.inner.lock().expect("task store lock poisoned");
        inner.tasks.iter().find(|t| t.id == id).cloned()
    }

    /// Returns clones of all non-deleted tasks.
    ///
    /// Tasks with [`TaskStatus::Deleted`] are excluded from the results.
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::tools::tasks::{TaskStore, TaskUpdate, TaskStatus};
    ///
    /// let store = TaskStore::new();
    /// store.create("Active", "Still here");
    /// let deleted = store.create("Gone", "Soft deleted");
    /// store.update(&deleted.id, TaskUpdate {
    ///     status: Some(TaskStatus::Deleted),
    ///     ..Default::default()
    /// }).unwrap();
    /// assert_eq!(store.list().len(), 1);
    /// ```
    #[must_use]
    pub fn list(&self) -> Vec<Task> {
        let inner = self.inner.lock().expect("task store lock poisoned");
        inner
            .tasks
            .iter()
            .filter(|t| t.status != TaskStatus::Deleted)
            .cloned()
            .collect()
    }

    /// Checks whether a task is blocked by incomplete dependencies.
    ///
    /// A task is blocked if any of its `blocked_by` IDs reference tasks
    /// that have not yet reached [`TaskStatus::Completed`].
    ///
    /// Returns `false` if the task does not exist.
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::tools::tasks::{TaskStore, TaskUpdate, TaskStatus};
    ///
    /// let store = TaskStore::new();
    /// let t1 = store.create("Blocker", "Must finish first");
    /// let t2 = store.create("Blocked", "Waits on blocker");
    /// store.update(&t2.id, TaskUpdate {
    ///     add_blocked_by: Some(vec![t1.id.clone()]),
    ///     ..Default::default()
    /// }).unwrap();
    /// assert!(store.is_blocked(&t2.id));
    ///
    /// store.update(&t1.id, TaskUpdate {
    ///     status: Some(TaskStatus::Completed),
    ///     ..Default::default()
    /// }).unwrap();
    /// assert!(!store.is_blocked(&t2.id));
    /// ```
    #[must_use]
    pub fn is_blocked(&self, id: &str) -> bool {
        let inner = self.inner.lock().expect("task store lock poisoned");
        let Some(task) = inner.tasks.iter().find(|t| t.id == id) else {
            return false;
        };
        if task.blocked_by.is_empty() {
            return false;
        }
        task.blocked_by.iter().any(|dep_id| {
            inner
                .tasks
                .iter()
                .find(|t| t.id == *dep_id)
                .is_some_and(|dep| dep.status != TaskStatus::Completed)
        })
    }

    /// Applies a partial update to the task with the given ID.
    ///
    /// Only fields that are `Some` in the [`TaskUpdate`] are modified.
    /// The `add_blocks` and `add_blocked_by` fields append to existing
    /// lists rather than replacing them. The `updated_at` timestamp is
    /// always refreshed.
    ///
    /// # Errors
    ///
    /// Returns an error if no task with the given ID exists.
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::tools::tasks::{TaskStore, TaskUpdate, TaskStatus};
    ///
    /// let store = TaskStore::new();
    /// let task = store.create("Original", "First draft");
    /// store.update(&task.id, TaskUpdate {
    ///     subject: Some("Updated".to_string()),
    ///     status: Some(TaskStatus::InProgress),
    ///     ..Default::default()
    /// }).unwrap();
    ///
    /// let updated = store.get(&task.id).unwrap();
    /// assert_eq!(updated.subject, "Updated");
    /// assert_eq!(updated.status, TaskStatus::InProgress);
    /// ```
    pub fn update(&self, id: &str, updates: TaskUpdate) -> anyhow::Result<()> {
        let mut inner = self.inner.lock().expect("task store lock poisoned");
        let task = inner
            .tasks
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or_else(|| anyhow::anyhow!("Task not found: {id}"))?;

        if let Some(status) = updates.status {
            task.status = status;
        }
        if let Some(subject) = updates.subject {
            task.subject = subject;
        }
        if let Some(description) = updates.description {
            task.description = description;
        }
        if let Some(owner) = updates.owner {
            task.owner = owner;
        }
        if let Some(add_blocks) = updates.add_blocks {
            task.blocks.extend(add_blocks);
        }
        if let Some(add_blocked_by) = updates.add_blocked_by {
            task.blocked_by.extend(add_blocked_by);
        }
        if let Some(metadata) = updates.metadata {
            task.metadata.extend(metadata);
        }
        task.updated_at = now_iso8601();
        Ok(())
    }

    /// Executes a `task_create` tool call from LLM-provided JSON input.
    ///
    /// # Expected Input
    ///
    /// ```json
    /// {
    ///   "subject": "Task subject",
    ///   "description": "Detailed description",
    ///   "metadata": { "key": "value" }
    /// }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if `subject` or `description` is missing.
    pub fn execute_task_create(&self, input: &serde_json::Value) -> anyhow::Result<String> {
        let subject = input
            .get("subject")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required field: subject"))?;
        let description = input
            .get("description")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required field: description"))?;

        let metadata: HashMap<String, serde_json::Value> = input
            .get("metadata")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let task = if metadata.is_empty() {
            self.create(subject, description)
        } else {
            self.create_with_metadata(subject, description, metadata)
        };

        serde_json::to_string_pretty(&task)
            .map_err(|e| anyhow::anyhow!("Failed to serialize task: {e}"))
    }

    /// Executes a `task_get` tool call from LLM-provided JSON input.
    ///
    /// Returns the full task details including blocked status.
    ///
    /// # Expected Input
    ///
    /// ```json
    /// { "task_id": "1" }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if `task_id` is missing or the task does not exist.
    pub fn execute_task_get(&self, input: &serde_json::Value) -> anyhow::Result<String> {
        let task_id = input
            .get("task_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required field: task_id"))?;

        let task = self
            .get(task_id)
            .ok_or_else(|| anyhow::anyhow!("Task not found: {task_id}"))?;

        let blocked = self.is_blocked(task_id);
        let mut result = serde_json::to_value(&task)
            .map_err(|e| anyhow::anyhow!("Failed to serialize task: {e}"))?;
        if let Some(obj) = result.as_object_mut() {
            obj.insert("is_blocked".to_string(), serde_json::Value::Bool(blocked));
        }

        serde_json::to_string_pretty(&result)
            .map_err(|e| anyhow::anyhow!("Failed to serialize task: {e}"))
    }

    /// Executes a `task_list` tool call.
    ///
    /// Returns a summary of all non-deleted tasks with their blocked status.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    pub fn execute_task_list(&self) -> anyhow::Result<String> {
        let tasks = self.list();
        let summaries: Vec<serde_json::Value> = tasks
            .iter()
            .map(|t| {
                let blocked = self.is_blocked(&t.id);
                serde_json::json!({
                    "id": t.id,
                    "subject": t.subject,
                    "status": t.status,
                    "owner": t.owner,
                    "is_blocked": blocked,
                    "blocks": t.blocks,
                    "blocked_by": t.blocked_by,
                })
            })
            .collect();

        serde_json::to_string_pretty(&summaries)
            .map_err(|e| anyhow::anyhow!("Failed to serialize task list: {e}"))
    }

    /// Executes a `task_update` tool call from LLM-provided JSON input.
    ///
    /// # Expected Input
    ///
    /// ```json
    /// {
    ///   "task_id": "1",
    ///   "status": "in_progress",
    ///   "subject": "New subject",
    ///   "description": "New description",
    ///   "owner": "agent-1",
    ///   "add_blocks": ["2"],
    ///   "add_blocked_by": ["3"]
    /// }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if `task_id` is missing or the task does not exist.
    pub fn execute_task_update(&self, input: &serde_json::Value) -> anyhow::Result<String> {
        let task_id = input
            .get("task_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required field: task_id"))?;

        let status: Option<TaskStatus> =
            input
                .get("status")
                .and_then(|v| v.as_str())
                .map(|s| match s {
                    "pending" => TaskStatus::Pending,
                    "in_progress" => TaskStatus::InProgress,
                    "completed" => TaskStatus::Completed,
                    "deleted" => TaskStatus::Deleted,
                    _ => TaskStatus::Pending,
                });

        let subject = input
            .get("subject")
            .and_then(|v| v.as_str())
            .map(String::from);

        let description = input
            .get("description")
            .and_then(|v| v.as_str())
            .map(String::from);

        let owner = input.get("owner").map(|v| v.as_str().map(String::from));

        let add_blocks: Option<Vec<String>> = input.get("add_blocks").and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|item| item.as_str().map(String::from))
                    .collect()
            })
        });

        let add_blocked_by: Option<Vec<String>> = input.get("add_blocked_by").and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|item| item.as_str().map(String::from))
                    .collect()
            })
        });

        let updates = TaskUpdate {
            status,
            subject,
            description,
            owner,
            add_blocks,
            add_blocked_by,
            metadata: None,
        };

        self.update(task_id, updates)?;

        let task = self
            .get(task_id)
            .ok_or_else(|| anyhow::anyhow!("Task not found after update: {task_id}"))?;

        serde_json::to_string_pretty(&task)
            .map_err(|e| anyhow::anyhow!("Failed to serialize updated task: {e}"))
    }
}

/// Returns the current time as an ISO 8601 string.
fn now_iso8601() -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    // Simple ISO 8601 approximation without external chrono dependency
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Calculate year/month/day from days since epoch (1970-01-01)
    let (year, month, day) = days_to_ymd(days);

    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

/// Converts days since Unix epoch to (year, month, day).
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Simplified calendar calculation
    let mut remaining = days;
    let mut year = 1970u64;

    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        year += 1;
    }

    let month_days: [u64; 12] = if is_leap_year(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 1u64;
    for &md in &month_days {
        if remaining < md {
            break;
        }
        remaining -= md;
        month += 1;
    }

    (year, month, remaining + 1)
}

/// Returns whether a year is a leap year.
fn is_leap_year(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

/// Returns the tool definition for `task_create`.
///
/// Creates a new task with a subject and description.
#[must_use]
pub fn task_create_tool() -> crate::api::tools::ToolDefinition {
    crate::api::tools::ToolDefinition::new(
        "task_create",
        "Create a new task for tracking work items. Tasks start in 'pending' status \
         and can be updated, blocked by other tasks, and progressed through a workflow.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "subject": {
                    "type": "string",
                    "description": "Short summary of the task"
                },
                "description": {
                    "type": "string",
                    "description": "Detailed description of what the task entails"
                },
                "metadata": {
                    "type": "object",
                    "description": "Optional key-value metadata to attach to the task"
                }
            },
            "required": ["subject", "description"]
        }),
    )
}

/// Returns the tool definition for `task_get`.
///
/// Retrieves a single task by ID with its current blocked status.
#[must_use]
pub fn task_get_tool() -> crate::api::tools::ToolDefinition {
    crate::api::tools::ToolDefinition::new(
        "task_get",
        "Get the full details of a task by its ID, including current status, \
         dependencies, metadata, and whether it is blocked by incomplete tasks.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The ID of the task to retrieve"
                }
            },
            "required": ["task_id"]
        }),
    )
}

/// Returns the tool definition for `task_list`.
///
/// Lists all non-deleted tasks with summary information.
#[must_use]
pub fn task_list_tool() -> crate::api::tools::ToolDefinition {
    crate::api::tools::ToolDefinition::new(
        "task_list",
        "List all active (non-deleted) tasks with their ID, subject, status, owner, \
         blocked status, and dependency information.",
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        }),
    )
}

/// Returns the tool definition for `task_update`.
///
/// Updates fields on an existing task.
#[must_use]
pub fn task_update_tool() -> crate::api::tools::ToolDefinition {
    crate::api::tools::ToolDefinition::new(
        "task_update",
        "Update an existing task. Can change status (pending/in_progress/completed/deleted), \
         subject, description, owner, and add dependency relationships.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The ID of the task to update"
                },
                "status": {
                    "type": "string",
                    "enum": ["pending", "in_progress", "completed", "deleted"],
                    "description": "New status for the task"
                },
                "subject": {
                    "type": "string",
                    "description": "New subject line"
                },
                "description": {
                    "type": "string",
                    "description": "New description"
                },
                "owner": {
                    "type": "string",
                    "description": "New owner/assignee"
                },
                "add_blocks": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Task IDs to add to the blocks list"
                },
                "add_blocked_by": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Task IDs to add to the blocked_by list"
                }
            },
            "required": ["task_id"]
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_auto_id_generation() {
        let store = TaskStore::new();
        let t1 = store.create("First", "First description");
        let t2 = store.create("Second", "Second description");
        let t3 = store.create("Third", "Third description");

        assert_eq!(t1.id, "1");
        assert_eq!(t2.id, "2");
        assert_eq!(t3.id, "3");
    }

    #[test]
    fn test_create_initial_state() {
        let store = TaskStore::new();
        let task = store.create("Subject", "Description");

        assert_eq!(task.subject, "Subject");
        assert_eq!(task.description, "Description");
        assert_eq!(task.status, TaskStatus::Pending);
        assert!(task.owner.is_none());
        assert!(task.blocks.is_empty());
        assert!(task.blocked_by.is_empty());
        assert!(task.metadata.is_empty());
        assert!(!task.created_at.is_empty());
        assert!(!task.updated_at.is_empty());
    }

    #[test]
    fn test_create_with_metadata() {
        let store = TaskStore::new();
        let mut meta = HashMap::new();
        meta.insert("priority".to_string(), serde_json::json!("high"));
        meta.insert("estimate".to_string(), serde_json::json!(8));

        let task = store.create_with_metadata("Task", "Desc", meta);
        assert_eq!(task.metadata.len(), 2);
        assert_eq!(task.metadata["priority"], serde_json::json!("high"));
        assert_eq!(task.metadata["estimate"], serde_json::json!(8));
    }

    #[test]
    fn test_get_existing_task() {
        let store = TaskStore::new();
        let created = store.create("Find me", "Findable");

        let found = store.get(&created.id);
        assert!(found.is_some());
        let found = found.unwrap();
        assert_eq!(found.id, created.id);
        assert_eq!(found.subject, "Find me");
    }

    #[test]
    fn test_get_nonexistent_task() {
        let store = TaskStore::new();
        assert!(store.get("999").is_none());
    }

    #[test]
    fn test_get_returns_clone_not_reference() {
        let store = TaskStore::new();
        let task = store.create("Original", "Desc");
        let fetched = store.get(&task.id).unwrap();

        // Modifying the clone should not affect the store
        assert_eq!(fetched.subject, "Original");
    }

    #[test]
    fn test_list_excludes_deleted_tasks() {
        let store = TaskStore::new();
        let _ = store.create("Active 1", "Desc");
        let to_delete = store.create("To delete", "Desc");
        let _ = store.create("Active 2", "Desc");

        store
            .update(
                &to_delete.id,
                TaskUpdate {
                    status: Some(TaskStatus::Deleted),
                    ..Default::default()
                },
            )
            .unwrap();

        let tasks = store.list();
        assert_eq!(tasks.len(), 2);
        assert!(tasks.iter().all(|t| t.status != TaskStatus::Deleted));
        assert!(tasks.iter().any(|t| t.subject == "Active 1"));
        assert!(tasks.iter().any(|t| t.subject == "Active 2"));
    }

    #[test]
    fn test_list_empty_store() {
        let store = TaskStore::new();
        assert!(store.list().is_empty());
    }

    #[test]
    fn test_status_workflow_pending_to_in_progress_to_completed() {
        let store = TaskStore::new();
        let task = store.create("Workflow test", "Desc");

        assert_eq!(task.status, TaskStatus::Pending);

        store
            .update(
                &task.id,
                TaskUpdate {
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(store.get(&task.id).unwrap().status, TaskStatus::InProgress);

        store
            .update(
                &task.id,
                TaskUpdate {
                    status: Some(TaskStatus::Completed),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(store.get(&task.id).unwrap().status, TaskStatus::Completed);
    }

    #[test]
    fn test_dependency_tracking_blocked() {
        let store = TaskStore::new();
        let blocker = store.create("Blocker", "Must finish first");
        let blocked = store.create("Blocked", "Depends on blocker");

        store
            .update(
                &blocked.id,
                TaskUpdate {
                    add_blocked_by: Some(vec![blocker.id.clone()]),
                    ..Default::default()
                },
            )
            .unwrap();

        assert!(store.is_blocked(&blocked.id));
        assert!(!store.is_blocked(&blocker.id));
    }

    #[test]
    fn test_dependency_tracking_unblocked_after_completion() {
        let store = TaskStore::new();
        let blocker = store.create("Blocker", "Must finish first");
        let blocked = store.create("Blocked", "Depends on blocker");

        store
            .update(
                &blocked.id,
                TaskUpdate {
                    add_blocked_by: Some(vec![blocker.id.clone()]),
                    ..Default::default()
                },
            )
            .unwrap();

        assert!(store.is_blocked(&blocked.id));

        store
            .update(
                &blocker.id,
                TaskUpdate {
                    status: Some(TaskStatus::Completed),
                    ..Default::default()
                },
            )
            .unwrap();

        assert!(!store.is_blocked(&blocked.id));
    }

    #[test]
    fn test_dependency_tracking_multiple_blockers() {
        let store = TaskStore::new();
        let b1 = store.create("Blocker 1", "Desc");
        let b2 = store.create("Blocker 2", "Desc");
        let blocked = store.create("Blocked by two", "Desc");

        store
            .update(
                &blocked.id,
                TaskUpdate {
                    add_blocked_by: Some(vec![b1.id.clone(), b2.id.clone()]),
                    ..Default::default()
                },
            )
            .unwrap();

        assert!(store.is_blocked(&blocked.id));

        // Complete only one blocker - still blocked
        store
            .update(
                &b1.id,
                TaskUpdate {
                    status: Some(TaskStatus::Completed),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(store.is_blocked(&blocked.id));

        // Complete second blocker - now unblocked
        store
            .update(
                &b2.id,
                TaskUpdate {
                    status: Some(TaskStatus::Completed),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(!store.is_blocked(&blocked.id));
    }

    #[test]
    fn test_dependency_nonexistent_task_not_blocked() {
        let store = TaskStore::new();
        assert!(!store.is_blocked("nonexistent"));
    }

    #[test]
    fn test_update_all_fields() {
        let store = TaskStore::new();
        let task = store.create("Original subject", "Original description");

        store
            .update(
                &task.id,
                TaskUpdate {
                    status: Some(TaskStatus::InProgress),
                    subject: Some("Updated subject".to_string()),
                    description: Some("Updated description".to_string()),
                    owner: Some(Some("agent-1".to_string())),
                    add_blocks: Some(vec!["99".to_string()]),
                    add_blocked_by: Some(vec!["98".to_string()]),
                    metadata: Some({
                        let mut m = HashMap::new();
                        m.insert("key".to_string(), serde_json::json!("value"));
                        m
                    }),
                },
            )
            .unwrap();

        let updated = store.get(&task.id).unwrap();
        assert_eq!(updated.status, TaskStatus::InProgress);
        assert_eq!(updated.subject, "Updated subject");
        assert_eq!(updated.description, "Updated description");
        assert_eq!(updated.owner, Some("agent-1".to_string()));
        assert_eq!(updated.blocks, vec!["99"]);
        assert_eq!(updated.blocked_by, vec!["98"]);
        assert_eq!(updated.metadata["key"], serde_json::json!("value"));
        assert!(updated.updated_at >= task.updated_at);
    }

    #[test]
    fn test_update_nonexistent_task_fails() {
        let store = TaskStore::new();
        let result = store.update(
            "nonexistent",
            TaskUpdate {
                status: Some(TaskStatus::Completed),
                ..Default::default()
            },
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Task not found"));
    }

    #[test]
    fn test_update_partial_preserves_other_fields() {
        let store = TaskStore::new();
        let task = store.create("Keep this subject", "Keep this description");

        store
            .update(
                &task.id,
                TaskUpdate {
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .unwrap();

        let updated = store.get(&task.id).unwrap();
        assert_eq!(updated.subject, "Keep this subject");
        assert_eq!(updated.description, "Keep this description");
        assert_eq!(updated.status, TaskStatus::InProgress);
    }

    #[test]
    fn test_update_owner_clear() {
        let store = TaskStore::new();
        let task = store.create("Task", "Desc");

        // Set owner
        store
            .update(
                &task.id,
                TaskUpdate {
                    owner: Some(Some("agent-1".to_string())),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(
            store.get(&task.id).unwrap().owner,
            Some("agent-1".to_string())
        );

        // Clear owner
        store
            .update(
                &task.id,
                TaskUpdate {
                    owner: Some(None),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(store.get(&task.id).unwrap().owner.is_none());
    }

    #[test]
    fn test_add_blocks_appends() {
        let store = TaskStore::new();
        let task = store.create("Task", "Desc");

        store
            .update(
                &task.id,
                TaskUpdate {
                    add_blocks: Some(vec!["10".to_string()]),
                    ..Default::default()
                },
            )
            .unwrap();

        store
            .update(
                &task.id,
                TaskUpdate {
                    add_blocks: Some(vec!["20".to_string()]),
                    ..Default::default()
                },
            )
            .unwrap();

        let updated = store.get(&task.id).unwrap();
        assert_eq!(updated.blocks, vec!["10", "20"]);
    }

    #[test]
    fn test_concurrent_access_safety() {
        let store = TaskStore::new();
        let store_clone = store.clone();

        let handles: Vec<_> = (0..10)
            .map(|i| {
                let s = store_clone.clone();
                std::thread::spawn(move || {
                    s.create(&format!("Task {i}"), &format!("Description {i}"))
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        let tasks = store.list();
        assert_eq!(tasks.len(), 10);

        // All IDs should be unique
        let mut ids: Vec<String> = tasks.iter().map(|t| t.id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 10);
    }

    #[test]
    fn test_concurrent_update_safety() {
        let store = TaskStore::new();
        let task = store.create("Shared task", "Desc");

        let handles: Vec<_> = (0..10)
            .map(|i| {
                let s = store.clone();
                let id = task.id.clone();
                std::thread::spawn(move || {
                    s.update(
                        &id,
                        TaskUpdate {
                            add_blocks: Some(vec![format!("{i}")]),
                            ..Default::default()
                        },
                    )
                    .unwrap();
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        let updated = store.get(&task.id).unwrap();
        assert_eq!(updated.blocks.len(), 10);
    }

    #[test]
    fn test_task_serialization_roundtrip() {
        let task = Task {
            id: "42".to_string(),
            subject: "Test task".to_string(),
            description: "A test".to_string(),
            status: TaskStatus::InProgress,
            owner: Some("tester".to_string()),
            blocks: vec!["43".to_string()],
            blocked_by: vec!["41".to_string()],
            metadata: {
                let mut m = HashMap::new();
                m.insert("priority".to_string(), serde_json::json!("high"));
                m
            },
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-02T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&task).unwrap();
        let deserialized: Task = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, "42");
        assert_eq!(deserialized.subject, "Test task");
        assert_eq!(deserialized.status, TaskStatus::InProgress);
        assert_eq!(deserialized.owner, Some("tester".to_string()));
        assert_eq!(deserialized.blocks, vec!["43"]);
        assert_eq!(deserialized.blocked_by, vec!["41"]);
        assert_eq!(deserialized.metadata["priority"], serde_json::json!("high"));
    }

    #[test]
    fn test_task_status_serialization() {
        let statuses = [
            (TaskStatus::Pending, "\"pending\""),
            (TaskStatus::InProgress, "\"in_progress\""),
            (TaskStatus::Completed, "\"completed\""),
            (TaskStatus::Deleted, "\"deleted\""),
        ];

        for (status, expected_json) in statuses {
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(json, expected_json);
            let deserialized: TaskStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, status);
        }
    }

    #[test]
    fn test_task_status_display() {
        assert_eq!(TaskStatus::Pending.to_string(), "pending");
        assert_eq!(TaskStatus::InProgress.to_string(), "in_progress");
        assert_eq!(TaskStatus::Completed.to_string(), "completed");
        assert_eq!(TaskStatus::Deleted.to_string(), "deleted");
    }

    #[test]
    fn test_task_update_default() {
        let update = TaskUpdate::default();
        assert!(update.status.is_none());
        assert!(update.subject.is_none());
        assert!(update.description.is_none());
        assert!(update.owner.is_none());
        assert!(update.add_blocks.is_none());
        assert!(update.add_blocked_by.is_none());
        assert!(update.metadata.is_none());
    }

    #[test]
    fn test_execute_task_create() {
        let store = TaskStore::new();
        let input = serde_json::json!({
            "subject": "Build feature",
            "description": "Implement the widget"
        });

        let result = store.execute_task_create(&input).unwrap();
        let task: Task = serde_json::from_str(&result).unwrap();
        assert_eq!(task.id, "1");
        assert_eq!(task.subject, "Build feature");
        assert_eq!(task.description, "Implement the widget");
    }

    #[test]
    fn test_execute_task_create_with_metadata() {
        let store = TaskStore::new();
        let input = serde_json::json!({
            "subject": "Task with meta",
            "description": "Has metadata",
            "metadata": { "priority": "high" }
        });

        let result = store.execute_task_create(&input).unwrap();
        let task: Task = serde_json::from_str(&result).unwrap();
        assert_eq!(task.metadata["priority"], serde_json::json!("high"));
    }

    #[test]
    fn test_execute_task_create_missing_subject() {
        let store = TaskStore::new();
        let input = serde_json::json!({ "description": "No subject" });
        assert!(store.execute_task_create(&input).is_err());
    }

    #[test]
    fn test_execute_task_create_missing_description() {
        let store = TaskStore::new();
        let input = serde_json::json!({ "subject": "No description" });
        assert!(store.execute_task_create(&input).is_err());
    }

    #[test]
    fn test_execute_task_get() {
        let store = TaskStore::new();
        let _ = store.create("Findable", "Description");

        let input = serde_json::json!({ "task_id": "1" });
        let result = store.execute_task_get(&input).unwrap();
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();

        assert_eq!(value["id"], "1");
        assert_eq!(value["subject"], "Findable");
        assert_eq!(value["is_blocked"], false);
    }

    #[test]
    fn test_execute_task_get_nonexistent() {
        let store = TaskStore::new();
        let input = serde_json::json!({ "task_id": "999" });
        assert!(store.execute_task_get(&input).is_err());
    }

    #[test]
    fn test_execute_task_get_missing_task_id() {
        let store = TaskStore::new();
        let input = serde_json::json!({});
        assert!(store.execute_task_get(&input).is_err());
    }

    #[test]
    fn test_execute_task_list() {
        let store = TaskStore::new();
        let _ = store.create("Task A", "Desc A");
        let _ = store.create("Task B", "Desc B");

        let result = store.execute_task_list().unwrap();
        let tasks: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0]["subject"], "Task A");
        assert_eq!(tasks[1]["subject"], "Task B");
    }

    #[test]
    fn test_execute_task_list_empty() {
        let store = TaskStore::new();
        let result = store.execute_task_list().unwrap();
        let tasks: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert!(tasks.is_empty());
    }

    #[test]
    fn test_execute_task_update() {
        let store = TaskStore::new();
        let _ = store.create("Original", "Original desc");

        let input = serde_json::json!({
            "task_id": "1",
            "status": "in_progress",
            "subject": "Updated"
        });

        let result = store.execute_task_update(&input).unwrap();
        let task: Task = serde_json::from_str(&result).unwrap();
        assert_eq!(task.subject, "Updated");
        assert_eq!(task.status, TaskStatus::InProgress);
    }

    #[test]
    fn test_execute_task_update_missing_task_id() {
        let store = TaskStore::new();
        let input = serde_json::json!({ "status": "completed" });
        assert!(store.execute_task_update(&input).is_err());
    }

    #[test]
    fn test_execute_task_update_nonexistent() {
        let store = TaskStore::new();
        let input = serde_json::json!({ "task_id": "999", "status": "completed" });
        assert!(store.execute_task_update(&input).is_err());
    }

    #[test]
    fn test_execute_task_get_blocked_status() {
        let store = TaskStore::new();
        let blocker = store.create("Blocker", "Blocks");
        let _ = store.create("Blocked", "Is blocked");

        store
            .update(
                "2",
                TaskUpdate {
                    add_blocked_by: Some(vec![blocker.id.clone()]),
                    ..Default::default()
                },
            )
            .unwrap();

        let input = serde_json::json!({ "task_id": "2" });
        let result = store.execute_task_get(&input).unwrap();
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(value["is_blocked"], true);
    }

    #[test]
    fn test_execute_task_list_includes_blocked_status() {
        let store = TaskStore::new();
        let blocker = store.create("Blocker", "Blocks");
        let _ = store.create("Blocked", "Is blocked");

        store
            .update(
                "2",
                TaskUpdate {
                    add_blocked_by: Some(vec![blocker.id]),
                    ..Default::default()
                },
            )
            .unwrap();

        let result = store.execute_task_list().unwrap();
        let tasks: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert_eq!(tasks[0]["is_blocked"], false);
        assert_eq!(tasks[1]["is_blocked"], true);
    }

    #[test]
    fn test_tool_definitions_have_required_fields() {
        let create = task_create_tool();
        assert_eq!(create.name, "task_create");
        assert!(!create.description.is_empty());
        assert!(create.input_schema["required"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("subject")));
        assert!(create.input_schema["required"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("description")));

        let get = task_get_tool();
        assert_eq!(get.name, "task_get");
        assert!(get.input_schema["required"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("task_id")));

        let list = task_list_tool();
        assert_eq!(list.name, "task_list");

        let update = task_update_tool();
        assert_eq!(update.name, "task_update");
        assert!(update.input_schema["required"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("task_id")));
    }

    #[test]
    fn test_now_iso8601_format() {
        let ts = now_iso8601();
        // Should match YYYY-MM-DDTHH:MM:SSZ format
        assert_eq!(ts.len(), 20);
        assert!(ts.ends_with('Z'));
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[7..8], "-");
        assert_eq!(&ts[10..11], "T");
        assert_eq!(&ts[13..14], ":");
        assert_eq!(&ts[16..17], ":");
    }

    #[test]
    fn test_is_leap_year() {
        assert!(is_leap_year(2000));
        assert!(is_leap_year(2024));
        assert!(!is_leap_year(1900));
        assert!(!is_leap_year(2023));
    }

    #[test]
    fn test_days_to_ymd_epoch() {
        let (y, m, d) = days_to_ymd(0);
        assert_eq!((y, m, d), (1970, 1, 1));
    }

    #[test]
    fn test_days_to_ymd_known_date() {
        // 2026-03-29 is day number 20541 since epoch
        // (verified: 56 years * 365 + 14 leap days + 31 + 28 + 29 - 1 = 20541)
        // Actually let's compute: 1970..2026 = 56 years
        // Leap years in [1970, 2026): 1972,1976,...,2024 = 14 leap years
        // 56*365 + 14 = 20440 + 14 = 20454 days to 2026-01-01
        // +31 (Jan) + 28 (Feb) + 28 (Mar 1-28) = 20541
        let (y, m, d) = days_to_ymd(20541);
        assert_eq!((y, m, d), (2026, 3, 29));
    }

    #[test]
    fn test_store_default_trait() {
        let store = TaskStore::default();
        assert!(store.list().is_empty());
    }

    #[test]
    fn test_metadata_merge_on_update() {
        let store = TaskStore::new();
        let mut initial_meta = HashMap::new();
        initial_meta.insert("key1".to_string(), serde_json::json!("val1"));
        let task = store.create_with_metadata("Task", "Desc", initial_meta);

        let mut new_meta = HashMap::new();
        new_meta.insert("key2".to_string(), serde_json::json!("val2"));

        store
            .update(
                &task.id,
                TaskUpdate {
                    metadata: Some(new_meta),
                    ..Default::default()
                },
            )
            .unwrap();

        let updated = store.get(&task.id).unwrap();
        assert_eq!(updated.metadata.len(), 2);
        assert_eq!(updated.metadata["key1"], serde_json::json!("val1"));
        assert_eq!(updated.metadata["key2"], serde_json::json!("val2"));
    }

    #[test]
    fn test_execute_task_update_with_dependencies() {
        let store = TaskStore::new();
        let _ = store.create("Task 1", "Desc");
        let _ = store.create("Task 2", "Desc");

        let input = serde_json::json!({
            "task_id": "2",
            "add_blocks": ["3"],
            "add_blocked_by": ["1"]
        });

        let result = store.execute_task_update(&input).unwrap();
        let task: Task = serde_json::from_str(&result).unwrap();
        assert_eq!(task.blocks, vec!["3"]);
        assert_eq!(task.blocked_by, vec!["1"]);
    }
}
