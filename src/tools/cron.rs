//! Cron-style scheduling for recurring agent tasks.
//!
//! Provides an in-memory schedule store with JSON persistence for creating,
//! listing, deleting, and checking due schedules. Expressions use a
//! human-readable interval syntax (`"5m"`, `"1h"`, `"daily"`) rather than
//! traditional cron syntax.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use uuid::Uuid;

/// A single scheduled task definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CronSchedule {
    /// Unique identifier (UUID).
    pub id: String,
    /// Human-readable interval expression (e.g. `"5m"`, `"1h"`, `"daily"`).
    pub expression: String,
    /// The prompt to execute when the schedule fires.
    pub prompt: String,
    /// ISO 8601-ish creation timestamp.
    pub created_at: String,
    /// Timestamp of the most recent execution, if any.
    pub last_run: Option<String>,
    /// Timestamp of the next scheduled execution.
    pub next_run: String,
    /// Whether this schedule is active.
    pub enabled: bool,
}

/// Persistent store for cron schedules backed by a JSON file.
pub struct CronStore {
    path: PathBuf,
}

impl CronStore {
    /// Creates a new cron store at the given file path.
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Loads all schedules from disk.
    ///
    /// Returns an empty list if the backing file does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be read or parsed.
    pub fn load(&self) -> Result<Vec<CronSchedule>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let content = std::fs::read_to_string(&self.path)
            .with_context(|| format!("Failed to read schedules from {}", self.path.display()))?;
        let items: Vec<CronSchedule> =
            serde_json::from_str(&content).with_context(|| "Failed to parse schedules JSON")?;
        Ok(items)
    }

    /// Persists all schedules to disk.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn save(&self, items: &[CronSchedule]) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create dir: {}", parent.display()))?;
        }
        let content =
            serde_json::to_string_pretty(items).context("Failed to serialize schedules")?;
        std::fs::write(&self.path, content)
            .with_context(|| format!("Failed to write schedules to {}", self.path.display()))
    }

    /// Creates a new schedule with the given interval expression and prompt.
    ///
    /// Validates the expression, generates a unique ID, computes the first
    /// `next_run` timestamp, and persists the schedule.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The expression is invalid (see [`parse_interval`]).
    /// - The store cannot be loaded or saved.
    pub fn create(&self, expression: &str, prompt: &str) -> Result<CronSchedule> {
        // Validate expression before creating
        let _ = parse_interval(expression)?;

        let now = SystemTime::now();
        let schedule = CronSchedule {
            id: Uuid::new_v4().to_string(),
            expression: expression.to_string(),
            prompt: prompt.to_string(),
            created_at: system_time_to_timestamp(now),
            last_run: None,
            next_run: next_run_time(expression, now)?,
            enabled: true,
        };

        let mut items = self.load()?;
        items.push(schedule.clone());
        self.save(&items)?;
        Ok(schedule)
    }

    /// Deletes a schedule by its full ID.
    ///
    /// # Errors
    ///
    /// Returns an error if no schedule with the given ID exists, or if the
    /// store cannot be loaded or saved.
    pub fn delete(&self, id: &str) -> Result<()> {
        let mut items = self.load()?;
        let len_before = items.len();
        items.retain(|s| s.id != id);
        if items.len() == len_before {
            bail!("Schedule not found with ID: {id}");
        }
        self.save(&items)?;
        Ok(())
    }

    /// Returns all schedules (both enabled and disabled).
    ///
    /// # Errors
    ///
    /// Returns an error if the store cannot be loaded.
    #[must_use = "the returned schedule list should be used"]
    pub fn list(&self) -> Result<Vec<CronSchedule>> {
        self.load()
    }

    /// Returns the schedule with the given ID, if it exists.
    ///
    /// # Errors
    ///
    /// Returns an error if the store cannot be loaded.
    #[must_use = "the returned schedule should be used"]
    pub fn get(&self, id: &str) -> Result<Option<CronSchedule>> {
        let items = self.load()?;
        Ok(items.into_iter().find(|s| s.id == id))
    }

    /// Toggles the enabled state of a schedule by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the schedule is not found or the store cannot be
    /// loaded or saved.
    pub fn toggle(&self, id: &str) -> Result<CronSchedule> {
        let mut items = self.load()?;
        let schedule = items
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or_else(|| anyhow::anyhow!("Schedule not found with ID: {id}"))?;
        schedule.enabled = !schedule.enabled;
        let result = schedule.clone();
        self.save(&items)?;
        Ok(result)
    }

    /// Checks all enabled schedules and returns those that are due to fire.
    ///
    /// For each due schedule, `last_run` is updated to the current time and
    /// `next_run` is advanced by one interval. The updated state is persisted.
    ///
    /// # Errors
    ///
    /// Returns an error if the store cannot be loaded or saved, or if an
    /// expression cannot be parsed.
    pub fn check_due(&self) -> Result<Vec<CronSchedule>> {
        let mut items = self.load()?;
        let now = SystemTime::now();
        let now_ts = system_time_to_secs(now);

        let mut due = Vec::new();

        for schedule in &mut items {
            if !schedule.enabled {
                continue;
            }
            let next_secs = timestamp_to_secs(&schedule.next_run);
            if now_ts >= next_secs {
                schedule.last_run = Some(system_time_to_timestamp(now));
                schedule.next_run = next_run_time(&schedule.expression, now)?;
                due.push(schedule.clone());
            }
        }

        if !due.is_empty() {
            self.save(&items)?;
        }

        Ok(due)
    }

    /// Checks due schedules using a caller-supplied timestamp.
    ///
    /// This is identical to [`check_due`](Self::check_due) but uses `now`
    /// instead of the real system clock, making it suitable for testing.
    ///
    /// # Errors
    ///
    /// Returns an error if the store cannot be loaded or saved, or if an
    /// expression cannot be parsed.
    pub fn check_due_at(&self, now: SystemTime) -> Result<Vec<CronSchedule>> {
        let mut items = self.load()?;
        let now_ts = system_time_to_secs(now);

        let mut due = Vec::new();

        for schedule in &mut items {
            if !schedule.enabled {
                continue;
            }
            let next_secs = timestamp_to_secs(&schedule.next_run);
            if now_ts >= next_secs {
                schedule.last_run = Some(system_time_to_timestamp(now));
                schedule.next_run = next_run_time(&schedule.expression, now)?;
                due.push(schedule.clone());
            }
        }

        if !due.is_empty() {
            self.save(&items)?;
        }

        Ok(due)
    }
}

/// Parses a human-readable interval expression into a [`Duration`].
///
/// Supported formats:
/// - `"30s"` — seconds
/// - `"5m"` — minutes
/// - `"1h"` — hours
/// - `"hourly"` — alias for `"1h"`
/// - `"daily"` — alias for `"24h"`
///
/// # Errors
///
/// Returns an error if the expression is empty, uses an unknown suffix, or
/// contains a non-numeric value before the suffix.
///
/// # Examples
///
/// ```
/// use patina::tools::cron::parse_interval;
///
/// let dur = parse_interval("5m").unwrap();
/// assert_eq!(dur.as_secs(), 300);
/// ```
#[must_use = "the returned Duration should be used"]
pub fn parse_interval(expr: &str) -> Result<Duration> {
    let expr = expr.trim();
    if expr.is_empty() {
        bail!("Interval expression cannot be empty");
    }

    match expr {
        "daily" => return Ok(Duration::from_secs(86_400)),
        "hourly" => return Ok(Duration::from_secs(3_600)),
        _ => {}
    }

    if let Some(num) = expr.strip_suffix('s') {
        let val: u64 = num
            .parse()
            .with_context(|| format!("Invalid seconds value in expression: {expr}"))?;
        if val == 0 {
            bail!("Interval must be greater than zero");
        }
        return Ok(Duration::from_secs(val));
    }

    if let Some(num) = expr.strip_suffix('m') {
        let val: u64 = num
            .parse()
            .with_context(|| format!("Invalid minutes value in expression: {expr}"))?;
        if val == 0 {
            bail!("Interval must be greater than zero");
        }
        return Ok(Duration::from_secs(val * 60));
    }

    if let Some(num) = expr.strip_suffix('h') {
        let val: u64 = num
            .parse()
            .with_context(|| format!("Invalid hours value in expression: {expr}"))?;
        if val == 0 {
            bail!("Interval must be greater than zero");
        }
        return Ok(Duration::from_secs(val * 3_600));
    }

    bail!("Unknown interval expression: {expr}. Use <N>s, <N>m, <N>h, 'daily', or 'hourly'")
}

/// Calculates the next run timestamp for the given expression, starting from `from`.
///
/// Returns a string timestamp in the `"<secs>s"` format used throughout the store.
///
/// # Errors
///
/// Returns an error if the expression cannot be parsed (see [`parse_interval`]).
///
/// # Examples
///
/// ```
/// use std::time::{SystemTime, Duration, UNIX_EPOCH};
/// use patina::tools::cron::next_run_time;
///
/// let from = UNIX_EPOCH + Duration::from_secs(1000);
/// let next = next_run_time("5m", from).unwrap();
/// assert_eq!(next, "1300s");
/// ```
#[must_use = "the returned timestamp should be used"]
pub fn next_run_time(expression: &str, from: SystemTime) -> Result<String> {
    let interval = parse_interval(expression)?;
    let next = from + interval;
    Ok(system_time_to_timestamp(next))
}

/// Creates a default cron store path for the given data directory.
#[must_use]
pub fn default_cron_path(data_dir: &Path) -> PathBuf {
    data_dir.join("cron_schedules.json")
}

/// Formats a `SystemTime` as `"<secs>s"` relative to the Unix epoch.
fn system_time_to_timestamp(t: SystemTime) -> String {
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{secs}s")
}

/// Extracts seconds from the store's `"<secs>s"` timestamp format.
fn system_time_to_secs(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Parses a `"<secs>s"` timestamp string back into raw seconds.
fn timestamp_to_secs(ts: &str) -> u64 {
    ts.strip_suffix('s')
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};
    use tempfile::TempDir;

    fn test_store() -> (CronStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = CronStore::new(dir.path().join("cron.json"));
        (store, dir)
    }

    // ─── parse_interval tests ───────────────────────────────────────

    #[test]
    fn test_parse_interval_seconds() {
        let dur = parse_interval("30s").unwrap();
        assert_eq!(dur.as_secs(), 30);
    }

    #[test]
    fn test_parse_interval_minutes() {
        let dur = parse_interval("5m").unwrap();
        assert_eq!(dur.as_secs(), 300);
    }

    #[test]
    fn test_parse_interval_hours() {
        let dur = parse_interval("1h").unwrap();
        assert_eq!(dur.as_secs(), 3_600);
    }

    #[test]
    fn test_parse_interval_daily() {
        let dur = parse_interval("daily").unwrap();
        assert_eq!(dur.as_secs(), 86_400);
    }

    #[test]
    fn test_parse_interval_hourly() {
        let dur = parse_interval("hourly").unwrap();
        assert_eq!(dur.as_secs(), 3_600);
    }

    #[test]
    fn test_parse_interval_invalid_empty() {
        assert!(parse_interval("").is_err());
    }

    #[test]
    fn test_parse_interval_invalid_unknown_suffix() {
        assert!(parse_interval("5x").is_err());
    }

    #[test]
    fn test_parse_interval_invalid_non_numeric() {
        assert!(parse_interval("abcm").is_err());
    }

    #[test]
    fn test_parse_interval_zero_rejected() {
        assert!(parse_interval("0s").is_err());
        assert!(parse_interval("0m").is_err());
        assert!(parse_interval("0h").is_err());
    }

    // ─── next_run_time tests ────────────────────────────────────────

    #[test]
    fn test_next_run_time_5m() {
        let from = UNIX_EPOCH + Duration::from_secs(1000);
        let next = next_run_time("5m", from).unwrap();
        assert_eq!(next, "1300s");
    }

    #[test]
    fn test_next_run_time_1h() {
        let from = UNIX_EPOCH + Duration::from_secs(0);
        let next = next_run_time("1h", from).unwrap();
        assert_eq!(next, "3600s");
    }

    #[test]
    fn test_next_run_time_daily() {
        let from = UNIX_EPOCH + Duration::from_secs(100);
        let next = next_run_time("daily", from).unwrap();
        assert_eq!(next, "86500s");
    }

    #[test]
    fn test_next_run_time_invalid_expression() {
        let from = UNIX_EPOCH;
        assert!(next_run_time("bad", from).is_err());
    }

    // ─── CronStore::create and list ─────────────────────────────────

    #[test]
    fn test_create_and_list() {
        let (store, _dir) = test_store();
        let schedule = store.create("5m", "Run tests").unwrap();

        assert_eq!(schedule.expression, "5m");
        assert_eq!(schedule.prompt, "Run tests");
        assert!(schedule.enabled);
        assert!(schedule.last_run.is_none());

        let all = store.list().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, schedule.id);
    }

    #[test]
    fn test_create_invalid_expression_rejected() {
        let (store, _dir) = test_store();
        assert!(store.create("bad", "prompt").is_err());

        // Store should be empty — invalid create should not persist anything
        let all = store.list().unwrap();
        assert!(all.is_empty());
    }

    // ─── CronStore::delete ──────────────────────────────────────────

    #[test]
    fn test_delete_removes_schedule() {
        let (store, _dir) = test_store();
        let schedule = store.create("10m", "Cleanup").unwrap();
        store.delete(&schedule.id).unwrap();

        let all = store.list().unwrap();
        assert!(all.is_empty());
    }

    #[test]
    fn test_delete_nonexistent_returns_error() {
        let (store, _dir) = test_store();
        assert!(store.delete("nonexistent-id").is_err());
    }

    // ─── CronStore::get ─────────────────────────────────────────────

    #[test]
    fn test_get_existing() {
        let (store, _dir) = test_store();
        let schedule = store.create("1h", "Hourly check").unwrap();
        let found = store.get(&schedule.id).unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().prompt, "Hourly check");
    }

    #[test]
    fn test_get_missing_returns_none() {
        let (store, _dir) = test_store();
        let found = store.get("no-such-id").unwrap();
        assert!(found.is_none());
    }

    // ─── CronStore::toggle ──────────────────────────────────────────

    #[test]
    fn test_toggle_enabled_disabled() {
        let (store, _dir) = test_store();
        let schedule = store.create("5m", "Toggle me").unwrap();
        assert!(schedule.enabled);

        let toggled = store.toggle(&schedule.id).unwrap();
        assert!(!toggled.enabled);

        let toggled_back = store.toggle(&schedule.id).unwrap();
        assert!(toggled_back.enabled);
    }

    #[test]
    fn test_toggle_nonexistent_returns_error() {
        let (store, _dir) = test_store();
        assert!(store.toggle("nope").is_err());
    }

    // ─── CronStore::check_due ───────────────────────────────────────

    #[test]
    fn test_check_due_fires_at_correct_time() {
        let (store, _dir) = test_store();

        // Create a schedule with a known base time
        let base = UNIX_EPOCH + Duration::from_secs(10_000);
        let schedule = CronSchedule {
            id: Uuid::new_v4().to_string(),
            expression: "30s".to_string(),
            prompt: "Check health".to_string(),
            created_at: system_time_to_timestamp(base),
            last_run: None,
            next_run: "10030s".to_string(), // due at base + 30s
            enabled: true,
        };
        store.save(std::slice::from_ref(&schedule)).unwrap();

        // Not yet due (at base + 20s)
        let before_due = UNIX_EPOCH + Duration::from_secs(10_020);
        let due = store.check_due_at(before_due).unwrap();
        assert!(due.is_empty());

        // Exactly due (at base + 30s)
        let at_due = UNIX_EPOCH + Duration::from_secs(10_030);
        let due = store.check_due_at(at_due).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].prompt, "Check health");

        // Verify the schedule was advanced
        let updated = store.get(&schedule.id).unwrap().unwrap();
        assert!(updated.last_run.is_some());
        assert_eq!(updated.next_run, "10060s"); // 10030 + 30
    }

    #[test]
    fn test_check_due_skips_disabled() {
        let (store, _dir) = test_store();

        let base = UNIX_EPOCH + Duration::from_secs(10_000);
        let schedule = CronSchedule {
            id: Uuid::new_v4().to_string(),
            expression: "30s".to_string(),
            prompt: "Disabled task".to_string(),
            created_at: system_time_to_timestamp(base),
            last_run: None,
            next_run: "10030s".to_string(),
            enabled: false,
        };
        store.save(&[schedule]).unwrap();

        let at_due = UNIX_EPOCH + Duration::from_secs(10_030);
        let due = store.check_due_at(at_due).unwrap();
        assert!(due.is_empty());
    }

    // ─── Persistence ────────────────────────────────────────────────

    #[test]
    fn test_persistence_across_instances() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("cron.json");

        let id = {
            let store = CronStore::new(path.clone());
            let schedule = store.create("1h", "Persistent job").unwrap();
            schedule.id
        };

        {
            let store = CronStore::new(path);
            let items = store.list().unwrap();
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].id, id);
            assert_eq!(items[0].prompt, "Persistent job");
        }
    }

    // ─── Serialization ─────────────────────────────────────────────

    #[test]
    fn test_cron_schedule_serialization() {
        let schedule = CronSchedule {
            id: "test-id".to_string(),
            expression: "5m".to_string(),
            prompt: "Do work".to_string(),
            created_at: "1000s".to_string(),
            last_run: None,
            next_run: "1300s".to_string(),
            enabled: true,
        };
        let json = serde_json::to_string(&schedule).unwrap();
        let deserialized: CronSchedule = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, schedule);
    }

    // ─── Helper functions ───────────────────────────────────────────

    #[test]
    fn test_system_time_to_timestamp() {
        let t = UNIX_EPOCH + Duration::from_secs(42);
        assert_eq!(system_time_to_timestamp(t), "42s");
    }

    #[test]
    fn test_timestamp_to_secs() {
        assert_eq!(timestamp_to_secs("12345s"), 12345);
        assert_eq!(timestamp_to_secs("invalid"), 0);
    }

    #[test]
    fn test_default_cron_path() {
        let path = default_cron_path(Path::new("/data"));
        assert_eq!(path, PathBuf::from("/data/cron_schedules.json"));
    }

    // ─── Edge cases ─────────────────────────────────────────────────

    #[test]
    fn test_parse_interval_whitespace_trimmed() {
        let dur = parse_interval("  5m  ").unwrap();
        assert_eq!(dur.as_secs(), 300);
    }

    #[test]
    fn test_multiple_schedules() {
        let (store, _dir) = test_store();
        store.create("5m", "Task A").unwrap();
        store.create("1h", "Task B").unwrap();
        store.create("daily", "Task C").unwrap();

        let all = store.list().unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_delete_one_of_many() {
        let (store, _dir) = test_store();
        store.create("5m", "Keep me").unwrap();
        let to_delete = store.create("1h", "Delete me").unwrap();
        store.create("daily", "Keep me too").unwrap();

        store.delete(&to_delete.id).unwrap();

        let all = store.list().unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().all(|s| s.id != to_delete.id));
    }
}
