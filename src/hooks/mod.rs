//! Hook execution engine for lifecycle events

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookEvent {
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
    PermissionRequest,
    UserPromptSubmit,
    SessionStart,
    SessionEnd,
    Notification,
    Stop,
    SubagentStop,
    PreCompact,
}

impl HookEvent {
    pub fn as_str(&self) -> &'static str {
        match self {
            HookEvent::PreToolUse => "PreToolUse",
            HookEvent::PostToolUse => "PostToolUse",
            HookEvent::PostToolUseFailure => "PostToolUseFailure",
            HookEvent::PermissionRequest => "PermissionRequest",
            HookEvent::UserPromptSubmit => "UserPromptSubmit",
            HookEvent::SessionStart => "SessionStart",
            HookEvent::SessionEnd => "SessionEnd",
            HookEvent::Notification => "Notification",
            HookEvent::Stop => "Stop",
            HookEvent::SubagentStop => "SubagentStop",
            HookEvent::PreCompact => "PreCompact",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct HookContext {
    pub hook_event_name: String,
    pub session_id: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_input: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_response: Option<serde_json::Value>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct HookDefinition {
    pub matcher: Option<String>,
    pub hooks: Vec<HookCommand>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct HookCommand {
    #[serde(rename = "type")]
    pub hook_type: String,
    pub command: String,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug)]
pub struct HookResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub decision: HookDecision,
}

#[derive(Debug, Default)]
pub enum HookDecision {
    #[default]
    Continue,
    Block {
        reason: String,
    },
    Allow,
    Deny,
}

pub struct HookExecutor {
    hooks: HashMap<HookEvent, Vec<HookDefinition>>,
}

/// Checks if a tool name matches a matcher pattern.
///
/// Supports three pattern types:
/// - Pipe-separated: "Bash|Read|Write" matches any listed tool
/// - Glob patterns: "mcp__*" matches tools starting with "mcp__"
/// - Exact match: "Bash" matches only "Bash"
///
/// # Errors
///
/// Returns an error if a glob pattern is invalid.
fn matches_pattern(matcher: &str, tool_name: &str) -> Result<bool> {
    // Check for pipe-separated pattern first
    if matcher.contains('|') {
        return Ok(matcher.split('|').any(|part| {
            let trimmed = part.trim();
            if trimmed.contains('*') || trimmed.contains('?') || trimmed.contains('[') {
                // Part is a glob pattern
                glob::Pattern::new(trimmed)
                    .map(|p| p.matches(tool_name))
                    .unwrap_or(false)
            } else {
                // Exact match
                trimmed == tool_name
            }
        }));
    }

    // Single pattern - could be glob or exact
    let pattern = glob::Pattern::new(matcher)?;
    Ok(pattern.matches(tool_name))
}

impl HookExecutor {
    pub fn new() -> Self {
        Self {
            hooks: HashMap::new(),
        }
    }

    pub fn register(&mut self, event: HookEvent, hooks: Vec<HookDefinition>) {
        self.hooks.entry(event).or_default().extend(hooks);
    }

    pub async fn execute(&self, event: HookEvent, context: &HookContext) -> Result<HookResult> {
        let definitions = match self.hooks.get(&event) {
            Some(defs) => defs,
            None => {
                return Ok(HookResult {
                    exit_code: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                    decision: HookDecision::Continue,
                })
            }
        };

        let context_json = serde_json::to_string(context)?;

        for def in definitions {
            if let Some(ref matcher) = def.matcher {
                if let Some(ref tool_name) = context.tool_name {
                    if !matches_pattern(matcher, tool_name)? {
                        continue;
                    }
                }
            }

            for hook in &def.hooks {
                let result = self.run_hook_command(&hook.command, &context_json).await?;

                match result.exit_code {
                    0 => continue,
                    2 => {
                        return Ok(HookResult {
                            decision: HookDecision::Block {
                                reason: result.stdout.clone(),
                            },
                            ..result
                        })
                    }
                    _ => {
                        tracing::warn!(
                            "Hook exited with code {}: {}",
                            result.exit_code,
                            result.stderr
                        );
                    }
                }
            }
        }

        Ok(HookResult {
            exit_code: 0,
            stdout: String::new(),
            stderr: String::new(),
            decision: HookDecision::Continue,
        })
    }

    async fn run_hook_command(&self, command: &str, stdin_data: &str) -> Result<HookResult> {
        // Validate command is not empty or whitespace-only
        let trimmed = command.trim();
        if trimmed.is_empty() {
            return Ok(HookResult {
                exit_code: 1,
                stdout: String::new(),
                stderr: "Hook command is empty".to_string(),
                decision: HookDecision::Continue,
            });
        }

        // Log hook execution for audit trail
        tracing::info!(command = %trimmed, "Executing hook command");

        let mut child = Command::new("sh")
            .arg("-c")
            .arg(trimmed)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(stdin_data.as_bytes()).await?;
        }

        let output = child.wait_with_output().await?;

        Ok(HookResult {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            decision: HookDecision::Continue,
        })
    }
}

impl Default for HookExecutor {
    fn default() -> Self {
        Self::new()
    }
}
