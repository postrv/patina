//! Audit logging for enterprise compliance.
//!
//! This module provides comprehensive audit logging for tool usage,
//! API calls, and session lifecycle events.
//!
//! # Example
//!
//! ```
//! use rct::enterprise::audit::{AuditConfig, AuditLevel, AuditLogger, AuditEntry};
//! use std::path::PathBuf;
//!
//! // Create audit logger
//! let config = AuditConfig {
//!     enabled: true,
//!     level: AuditLevel::All,
//!     output_path: Some(PathBuf::from("/var/log/rct/audit.log")),
//!     json_format: true,
//!     include_timestamps: true,
//!     include_session_id: true,
//! };
//!
//! let mut logger = AuditLogger::new(config);
//!
//! // Log a tool use event
//! let entry = AuditEntry::tool_use(
//!     "session-123",
//!     "Bash",
//!     serde_json::json!({"command": "ls"}),
//!     Some(serde_json::json!({"output": "files"})),
//!     std::time::Duration::from_millis(50),
//! );
//!
//! // logger.log(entry).await.unwrap();
//! ```

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use tokio::fs::{File, OpenOptions};
use tokio::io::AsyncWriteExt;

/// Audit logging level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuditLevel {
    /// Log all events.
    #[default]
    All,
    /// Log only API calls (for cost tracking).
    ApiOnly,
    /// Log only tool usage.
    ToolsOnly,
    /// Log only session lifecycle events.
    SessionOnly,
}

/// Configuration for audit logging.
#[derive(Debug, Clone)]
pub struct AuditConfig {
    /// Whether audit logging is enabled.
    pub enabled: bool,
    /// What level of events to log.
    pub level: AuditLevel,
    /// Path to write audit logs (None for memory-only).
    pub output_path: Option<PathBuf>,
    /// Whether to format output as JSON.
    pub json_format: bool,
    /// Whether to include timestamps in entries.
    pub include_timestamps: bool,
    /// Whether to include session ID in entries.
    pub include_session_id: bool,
}

impl AuditConfig {
    /// Validates the audit configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration is invalid.
    pub fn validate(&self) -> Result<()> {
        // Currently all configurations are valid
        Ok(())
    }
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            level: AuditLevel::All,
            output_path: None,
            json_format: true,
            include_timestamps: true,
            include_session_id: true,
        }
    }
}

/// Type of audit event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuditEvent {
    /// Tool was used.
    ToolUse {
        /// Name of the tool.
        tool_name: String,
        /// Input parameters.
        input: serde_json::Value,
        /// Output result (if available).
        output: Option<serde_json::Value>,
        /// Duration in milliseconds.
        duration_ms: u64,
    },
    /// API call was made.
    ApiCall {
        /// Model used.
        model: String,
        /// Input tokens used.
        input_tokens: u32,
        /// Output tokens generated.
        output_tokens: u32,
        /// Duration in milliseconds.
        duration_ms: u64,
        /// Whether the call succeeded.
        success: bool,
    },
    /// Session started.
    SessionStart {
        /// Working directory.
        working_dir: PathBuf,
    },
    /// Session ended.
    SessionEnd {
        /// Session duration in seconds.
        duration_secs: u64,
    },
}

/// A single audit log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Session identifier.
    pub session_id: String,
    /// Timestamp of the event.
    pub timestamp: SystemTime,
    /// The audit event.
    pub event: AuditEvent,
}

impl AuditEntry {
    /// Creates a tool use audit entry.
    #[must_use]
    pub fn tool_use(
        session_id: &str,
        tool_name: &str,
        input: serde_json::Value,
        output: Option<serde_json::Value>,
        duration: Duration,
    ) -> Self {
        Self {
            session_id: session_id.to_string(),
            timestamp: SystemTime::now(),
            event: AuditEvent::ToolUse {
                tool_name: tool_name.to_string(),
                input,
                output,
                duration_ms: duration.as_millis() as u64,
            },
        }
    }

    /// Creates an API call audit entry.
    #[must_use]
    pub fn api_call(
        session_id: &str,
        model: &str,
        input_tokens: u32,
        output_tokens: u32,
        duration: Duration,
        success: bool,
    ) -> Self {
        Self {
            session_id: session_id.to_string(),
            timestamp: SystemTime::now(),
            event: AuditEvent::ApiCall {
                model: model.to_string(),
                input_tokens,
                output_tokens,
                duration_ms: duration.as_millis() as u64,
                success,
            },
        }
    }

    /// Creates a session start audit entry.
    #[must_use]
    pub fn session_start(session_id: &str, working_dir: PathBuf) -> Self {
        Self {
            session_id: session_id.to_string(),
            timestamp: SystemTime::now(),
            event: AuditEvent::SessionStart { working_dir },
        }
    }

    /// Creates a session end audit entry.
    #[must_use]
    pub fn session_end(session_id: &str, duration: Duration) -> Self {
        Self {
            session_id: session_id.to_string(),
            timestamp: SystemTime::now(),
            event: AuditEvent::SessionEnd {
                duration_secs: duration.as_secs(),
            },
        }
    }

    /// Returns the event type as a string.
    #[must_use]
    pub fn event_type(&self) -> &'static str {
        match &self.event {
            AuditEvent::ToolUse { .. } => "tool_use",
            AuditEvent::ApiCall { .. } => "api_call",
            AuditEvent::SessionStart { .. } => "session_start",
            AuditEvent::SessionEnd { .. } => "session_end",
        }
    }
}

/// Query for filtering audit entries.
#[derive(Debug, Clone, Default)]
pub struct AuditQuery {
    /// Filter by session ID.
    session_id: Option<String>,
    /// Filter by event type.
    event_type: Option<String>,
    /// Filter by start time.
    start_time: Option<SystemTime>,
    /// Filter by end time.
    end_time: Option<SystemTime>,
}

impl AuditQuery {
    /// Creates a new empty query.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Filters by session ID.
    #[must_use]
    pub fn session(mut self, session_id: &str) -> Self {
        self.session_id = Some(session_id.to_string());
        self
    }

    /// Filters by event type.
    #[must_use]
    pub fn event_type(mut self, event_type: &str) -> Self {
        self.event_type = Some(event_type.to_string());
        self
    }

    /// Filters by start time.
    #[must_use]
    pub fn after(mut self, time: SystemTime) -> Self {
        self.start_time = Some(time);
        self
    }

    /// Filters by end time.
    #[must_use]
    pub fn before(mut self, time: SystemTime) -> Self {
        self.end_time = Some(time);
        self
    }

    /// Checks if an entry matches this query.
    fn matches(&self, entry: &AuditEntry) -> bool {
        if let Some(ref session) = self.session_id {
            if entry.session_id != *session {
                return false;
            }
        }

        if let Some(ref event_type) = self.event_type {
            if entry.event_type() != event_type {
                return false;
            }
        }

        if let Some(start) = self.start_time {
            if entry.timestamp < start {
                return false;
            }
        }

        if let Some(end) = self.end_time {
            if entry.timestamp > end {
                return false;
            }
        }

        true
    }
}

/// Statistics computed from audit entries.
#[derive(Debug, Clone, Default)]
pub struct AuditStatistics {
    /// Total API calls.
    pub total_api_calls: usize,
    /// Successful API calls.
    pub successful_api_calls: usize,
    /// Total input tokens.
    pub total_input_tokens: u32,
    /// Total output tokens.
    pub total_output_tokens: u32,
    /// Total tool uses.
    pub total_tool_uses: usize,
    /// Total sessions.
    pub total_sessions: usize,
}

/// Audit logger for tracking events.
#[derive(Debug)]
pub struct AuditLogger {
    /// Configuration.
    config: AuditConfig,
    /// In-memory entries.
    entries: Vec<AuditEntry>,
    /// File handle for persistent logging.
    file: Option<File>,
}

impl AuditLogger {
    /// Creates a new audit logger.
    #[must_use]
    pub fn new(config: AuditConfig) -> Self {
        Self {
            config,
            entries: Vec::new(),
            file: None,
        }
    }

    /// Returns whether audit logging is enabled.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Logs an audit entry.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to the file fails.
    pub async fn log(&mut self, entry: AuditEntry) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }

        // Check if entry matches the configured level
        if !self.should_log(&entry) {
            return Ok(());
        }

        // Store in memory
        self.entries.push(entry.clone());

        // Write to file if configured
        if let Some(path) = self.config.output_path.clone() {
            self.ensure_file_open(&path).await?;
            self.write_entry(&entry).await?;
        }

        Ok(())
    }

    /// Returns all entries.
    #[must_use]
    pub fn entries(&self) -> &[AuditEntry] {
        &self.entries
    }

    /// Queries entries matching the given criteria.
    #[must_use]
    pub fn query(&self, query: &AuditQuery) -> Vec<&AuditEntry> {
        self.entries
            .iter()
            .filter(|entry| query.matches(entry))
            .collect()
    }

    /// Computes statistics from all entries.
    #[must_use]
    pub fn statistics(&self) -> AuditStatistics {
        let mut stats = AuditStatistics::default();

        for entry in &self.entries {
            match &entry.event {
                AuditEvent::ApiCall {
                    input_tokens,
                    output_tokens,
                    success,
                    ..
                } => {
                    stats.total_api_calls += 1;
                    if *success {
                        stats.successful_api_calls += 1;
                    }
                    stats.total_input_tokens += input_tokens;
                    stats.total_output_tokens += output_tokens;
                }
                AuditEvent::ToolUse { .. } => {
                    stats.total_tool_uses += 1;
                }
                AuditEvent::SessionStart { .. } => {
                    stats.total_sessions += 1;
                }
                AuditEvent::SessionEnd { .. } => {}
            }
        }

        stats
    }

    /// Flushes any buffered data to disk.
    ///
    /// # Errors
    ///
    /// Returns an error if flushing fails.
    pub async fn flush(&mut self) -> Result<()> {
        if let Some(ref mut file) = self.file {
            file.flush().await.context("Failed to flush audit log")?;
        }
        Ok(())
    }

    /// Checks if an entry should be logged based on the configured level.
    fn should_log(&self, entry: &AuditEntry) -> bool {
        match self.config.level {
            AuditLevel::All => true,
            AuditLevel::ApiOnly => matches!(entry.event, AuditEvent::ApiCall { .. }),
            AuditLevel::ToolsOnly => matches!(entry.event, AuditEvent::ToolUse { .. }),
            AuditLevel::SessionOnly => matches!(
                entry.event,
                AuditEvent::SessionStart { .. } | AuditEvent::SessionEnd { .. }
            ),
        }
    }

    /// Ensures the file is open for writing.
    async fn ensure_file_open(&mut self, path: &PathBuf) -> Result<()> {
        if self.file.is_none() {
            // Create parent directories if needed
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .context("Failed to create audit log directory")?;
            }

            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .await
                .context("Failed to open audit log file")?;

            self.file = Some(file);
        }
        Ok(())
    }

    /// Writes an entry to the file.
    async fn write_entry(&mut self, entry: &AuditEntry) -> Result<()> {
        if let Some(ref mut file) = self.file {
            let line = if self.config.json_format {
                serde_json::to_string(entry).context("Failed to serialize entry")?
            } else {
                format!(
                    "[{}] {} - {:?}",
                    humantime::format_rfc3339(entry.timestamp),
                    entry.session_id,
                    entry.event
                )
            };

            file.write_all(line.as_bytes())
                .await
                .context("Failed to write entry")?;
            file.write_all(b"\n")
                .await
                .context("Failed to write newline")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_config_default() {
        let config = AuditConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.level, AuditLevel::All);
    }

    #[test]
    fn test_audit_entry_tool_use() {
        let entry = AuditEntry::tool_use(
            "test",
            "Bash",
            serde_json::json!({"cmd": "ls"}),
            None,
            Duration::from_millis(100),
        );
        assert_eq!(entry.session_id, "test");
        assert_eq!(entry.event_type(), "tool_use");
    }

    #[test]
    fn test_audit_entry_api_call() {
        let entry = AuditEntry::api_call(
            "test",
            "claude-3-opus",
            1000,
            500,
            Duration::from_secs(2),
            true,
        );
        assert_eq!(entry.event_type(), "api_call");
    }

    #[test]
    fn test_audit_query_matches() {
        let entry = AuditEntry::tool_use(
            "session-1",
            "Read",
            serde_json::json!({}),
            None,
            Duration::from_millis(10),
        );

        let query = AuditQuery::new().session("session-1");
        assert!(query.matches(&entry));

        let query = AuditQuery::new().session("session-2");
        assert!(!query.matches(&entry));
    }

    #[test]
    fn test_audit_level_variants() {
        assert_eq!(AuditLevel::default(), AuditLevel::All);
    }
}
