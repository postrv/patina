//! Tool execution for agentic capabilities

use anyhow::Result;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use regex::Regex;

pub struct ToolExecutor {
    working_dir: PathBuf,
    policy: ToolExecutionPolicy,
}

pub struct ToolExecutionPolicy {
    pub dangerous_patterns: Vec<Regex>,
    pub protected_paths: Vec<PathBuf>,
    pub max_file_size: usize,
    pub command_timeout: Duration,
}

impl Default for ToolExecutionPolicy {
    fn default() -> Self {
        Self {
            dangerous_patterns: vec![
                Regex::new(r"rm\s+-rf\s+/").unwrap(),
                Regex::new(r"sudo\s+").unwrap(),
                Regex::new(r"chmod\s+777").unwrap(),
            ],
            protected_paths: vec![
                PathBuf::from("/etc"),
                PathBuf::from("/usr"),
                PathBuf::from("/bin"),
            ],
            max_file_size: 10 * 1024 * 1024,
            command_timeout: Duration::from_secs(300),
        }
    }
}

#[derive(Debug)]
pub struct ToolCall {
    pub name: String,
    pub input: serde_json::Value,
}

#[derive(Debug)]
pub enum ToolResult {
    Success(String),
    Error(String),
    Cancelled,
}

impl ToolExecutor {
    pub fn new(working_dir: PathBuf) -> Self {
        Self {
            working_dir,
            policy: ToolExecutionPolicy::default(),
        }
    }

    pub fn with_policy(mut self, policy: ToolExecutionPolicy) -> Self {
        self.policy = policy;
        self
    }

    pub async fn execute(&self, call: ToolCall) -> Result<ToolResult> {
        match call.name.as_str() {
            "bash" => self.execute_bash(&call.input).await,
            "read_file" => self.read_file(&call.input).await,
            "write_file" => self.write_file(&call.input).await,
            "list_files" => self.list_files(&call.input).await,
            _ => Ok(ToolResult::Error(format!("Unknown tool: {}", call.name))),
        }
    }

    async fn execute_bash(&self, input: &serde_json::Value) -> Result<ToolResult> {
        let command = input.get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing command"))?;

        for pattern in &self.policy.dangerous_patterns {
            if pattern.is_match(command) {
                return Ok(ToolResult::Error(format!(
                    "Command blocked by security policy: matches {:?}",
                    pattern.as_str()
                )));
            }
        }

        let output = tokio::time::timeout(
            self.policy.command_timeout,
            Command::new("sh")
                .arg("-c")
                .arg(command)
                .current_dir(&self.working_dir)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
        ).await??;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if output.status.success() {
            Ok(ToolResult::Success(format!("{}{}", stdout, stderr)))
        } else {
            Ok(ToolResult::Error(format!(
                "Exit code {}: {}{}",
                output.status.code().unwrap_or(-1),
                stdout,
                stderr
            )))
        }
    }

    async fn read_file(&self, input: &serde_json::Value) -> Result<ToolResult> {
        let path = input.get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing path"))?;

        let full_path = self.working_dir.join(path);

        match tokio::fs::read_to_string(&full_path).await {
            Ok(content) => Ok(ToolResult::Success(content)),
            Err(e) => Ok(ToolResult::Error(format!("Failed to read file: {}", e))),
        }
    }

    async fn write_file(&self, input: &serde_json::Value) -> Result<ToolResult> {
        let path = input.get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing path"))?;

        let content = input.get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing content"))?;

        if content.len() > self.policy.max_file_size {
            return Ok(ToolResult::Error(format!(
                "File size {} exceeds limit {}",
                content.len(),
                self.policy.max_file_size
            )));
        }

        let full_path = self.working_dir.join(path);

        if let Some(parent) = full_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        match tokio::fs::write(&full_path, content).await {
            Ok(()) => Ok(ToolResult::Success(format!("Wrote {} bytes to {}", content.len(), path))),
            Err(e) => Ok(ToolResult::Error(format!("Failed to write file: {}", e))),
        }
    }

    async fn list_files(&self, input: &serde_json::Value) -> Result<ToolResult> {
        let path = input.get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        let full_path = self.working_dir.join(path);

        let mut entries = Vec::new();
        let mut dir = tokio::fs::read_dir(&full_path).await?;

        while let Some(entry) = dir.next_entry().await? {
            let name = entry.file_name().to_string_lossy().to_string();
            let file_type = entry.file_type().await?;
            let prefix = if file_type.is_dir() { "d " } else { "- " };
            entries.push(format!("{}{}", prefix, name));
        }

        entries.sort();
        Ok(ToolResult::Success(entries.join("\n")))
    }
}
