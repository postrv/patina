//! Background task registry for long-running bash commands.
//!
//! [`BackgroundTaskRegistry`] manages tasks spawned via `bash` with
//! `run_in_background: true`. Each task captures stdout/stderr into
//! a shared buffer and can be queried or stopped.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

/// A single background task.
pub struct BackgroundTask {
    /// Unique identifier for this task.
    pub task_id: String,
    /// The command that was executed.
    pub command: String,
    /// Shared buffer for captured output.
    pub output_buffer: Arc<Mutex<String>>,
    /// Whether the task has completed.
    pub completed: Arc<AtomicBool>,
    /// When the task was started.
    pub started_at: Instant,
    /// Handle to abort the task.
    handle: Option<tokio::task::JoinHandle<()>>,
}

/// Summary information about a background task.
#[derive(Debug, Clone)]
pub struct TaskInfo {
    /// Unique task identifier.
    pub task_id: String,
    /// The command that was executed.
    pub command: String,
    /// Whether the task has completed.
    pub completed: bool,
    /// How long the task has been running (seconds).
    pub elapsed_secs: u64,
}

/// Registry for managing background tasks.
pub struct BackgroundTaskRegistry {
    tasks: HashMap<String, BackgroundTask>,
    next_id: u32,
}

impl BackgroundTaskRegistry {
    /// Creates a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
            next_id: 1,
        }
    }

    /// Spawns a new background task running the given command.
    ///
    /// Returns the task ID assigned to this task.
    pub fn spawn(&mut self, command: String, working_dir: &std::path::Path) -> String {
        let task_id = format!("bg_{}", self.next_id);
        self.next_id += 1;

        let output_buffer = Arc::new(Mutex::new(String::new()));
        let completed = Arc::new(AtomicBool::new(false));

        let buf_clone = Arc::clone(&output_buffer);
        let done_clone = Arc::clone(&completed);
        let cmd_clone = command.clone();
        let dir = working_dir.to_path_buf();

        let handle = tokio::spawn(async move {
            let result = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(&cmd_clone)
                .current_dir(&dir)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output()
                .await;

            match result {
                Ok(output) => {
                    let mut buf = buf_clone.lock().await;
                    if !output.stdout.is_empty() {
                        if let Ok(s) = String::from_utf8(output.stdout) {
                            buf.push_str(&s);
                        }
                    }
                    if !output.stderr.is_empty() {
                        if let Ok(s) = String::from_utf8(output.stderr) {
                            if !buf.is_empty() {
                                buf.push('\n');
                            }
                            buf.push_str(&s);
                        }
                    }
                }
                Err(e) => {
                    let mut buf = buf_clone.lock().await;
                    buf.push_str(&format!("Error: {e}"));
                }
            }
            done_clone.store(true, Ordering::Release);
        });

        self.tasks.insert(
            task_id.clone(),
            BackgroundTask {
                task_id: task_id.clone(),
                command,
                output_buffer,
                completed,
                started_at: Instant::now(),
                handle: Some(handle),
            },
        );

        task_id
    }

    /// Gets the current output of a background task.
    ///
    /// Returns the buffered output or an error if the task doesn't exist.
    pub async fn get_output(&self, task_id: &str) -> Result<String, String> {
        match self.tasks.get(task_id) {
            Some(task) => {
                let buf = task.output_buffer.lock().await;
                let completed = task.completed.load(Ordering::Acquire);
                let status = if completed { "completed" } else { "running" };
                Ok(format!("[Task {task_id} ({status})]\n{}", *buf))
            }
            None => Err(format!("No task with ID '{task_id}'")),
        }
    }

    /// Stops a running background task.
    ///
    /// Returns Ok if the task was found and aborted.
    pub fn stop(&mut self, task_id: &str) -> Result<String, String> {
        match self.tasks.get_mut(task_id) {
            Some(task) => {
                if let Some(handle) = task.handle.take() {
                    handle.abort();
                    task.completed.store(true, Ordering::Release);
                    Ok(format!("Task {task_id} stopped"))
                } else {
                    Ok(format!("Task {task_id} already finished"))
                }
            }
            None => Err(format!("No task with ID '{task_id}'")),
        }
    }

    /// Lists all tracked tasks with their status.
    #[must_use]
    pub fn list(&self) -> Vec<TaskInfo> {
        self.tasks
            .values()
            .map(|task| TaskInfo {
                task_id: task.task_id.clone(),
                command: task.command.clone(),
                completed: task.completed.load(Ordering::Acquire),
                elapsed_secs: task.started_at.elapsed().as_secs(),
            })
            .collect()
    }

    /// Removes completed tasks from the registry.
    pub fn cleanup_completed(&mut self) {
        self.tasks
            .retain(|_, task| !task.completed.load(Ordering::Acquire));
    }

    /// Returns a reference to the internal task map.
    ///
    /// Used by the tool intercept to clone Arc handles for async access.
    #[must_use]
    pub fn tasks_ref(&self) -> &HashMap<String, BackgroundTask> {
        &self.tasks
    }

    /// Returns the number of tracked tasks.
    #[must_use]
    pub fn count(&self) -> usize {
        self.tasks.len()
    }
}

impl Default for BackgroundTaskRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_dir() -> PathBuf {
        PathBuf::from("/tmp")
    }

    #[test]
    fn new_registry_is_empty() {
        let reg = BackgroundTaskRegistry::new();
        assert_eq!(reg.count(), 0);
        assert!(reg.list().is_empty());
    }

    #[tokio::test]
    async fn spawn_returns_unique_ids() {
        let mut reg = BackgroundTaskRegistry::new();
        let id1 = reg.spawn("echo hi".into(), &test_dir());
        let id2 = reg.spawn("echo bye".into(), &test_dir());
        assert_ne!(id1, id2);
        assert_eq!(reg.count(), 2);
    }

    #[tokio::test]
    async fn spawn_and_get_output() {
        let mut reg = BackgroundTaskRegistry::new();
        let id = reg.spawn("echo hello".into(), &test_dir());

        // Wait a bit for the task to complete
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let output = reg.get_output(&id).await.unwrap();
        assert!(output.contains("hello"), "output should contain 'hello'");
        assert!(output.contains("completed"), "task should be completed");
    }

    #[tokio::test]
    async fn get_output_unknown_id_returns_error() {
        let reg = BackgroundTaskRegistry::new();
        let result = reg.get_output("bg_999").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn stop_aborts_task() {
        let mut reg = BackgroundTaskRegistry::new();
        let id = reg.spawn("sleep 60".into(), &test_dir());

        let result = reg.stop(&id);
        assert!(result.is_ok());
        assert!(result.unwrap().contains("stopped"));
    }

    #[test]
    fn stop_unknown_id_returns_error() {
        let mut reg = BackgroundTaskRegistry::new();
        let result = reg.stop("bg_999");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn list_shows_task_info() {
        let mut reg = BackgroundTaskRegistry::new();
        reg.spawn("echo test".into(), &test_dir());

        let list = reg.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].command, "echo test");
        assert!(list[0].task_id.starts_with("bg_"));
    }

    #[tokio::test]
    async fn cleanup_removes_completed() {
        let mut reg = BackgroundTaskRegistry::new();
        reg.spawn("echo fast".into(), &test_dir());

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        reg.cleanup_completed();
        assert_eq!(reg.count(), 0, "completed task should be cleaned up");
    }

    #[test]
    fn default_creates_empty() {
        let reg = BackgroundTaskRegistry::default();
        assert_eq!(reg.count(), 0);
    }

    #[test]
    fn task_info_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<TaskInfo>();
    }
}
