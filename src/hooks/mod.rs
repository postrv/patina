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

/// High-level hook manager for application lifecycle integration.
///
/// Provides convenience methods for firing all 11 hook events with appropriate
/// context construction. This is the primary interface for integrating hooks
/// into the application.
///
/// # Examples
///
/// ```no_run
/// use rct::hooks::{HookManager, HookEvent};
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let mut manager = HookManager::new("session-123".to_string());
///
///     // Fire session start
///     let result = manager.fire_session_start().await?;
///
///     // Check if hook blocked session start
///     if matches!(result.decision, rct::hooks::HookDecision::Block { .. }) {
///         eprintln!("Session start blocked by hook");
///         return Ok(());
///     }
///
///     Ok(())
/// }
/// ```
pub struct HookManager {
    executor: HookExecutor,
    session_id: String,
}

impl HookManager {
    /// Creates a new hook manager with the given session ID.
    #[must_use]
    pub fn new(session_id: String) -> Self {
        Self {
            executor: HookExecutor::new(),
            session_id,
        }
    }

    /// Returns the session ID.
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Loads hook configuration from a TOML file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed.
    pub fn load_config(&mut self, path: &std::path::Path) -> Result<()> {
        let content = std::fs::read_to_string(path)?;
        let config: HooksConfig = toml::from_str(&content)?;

        // Register hooks from configuration
        if let Some(hooks) = config.pre_tool_use {
            self.executor.register(HookEvent::PreToolUse, hooks);
        }
        if let Some(hooks) = config.post_tool_use {
            self.executor.register(HookEvent::PostToolUse, hooks);
        }
        if let Some(hooks) = config.post_tool_use_failure {
            self.executor.register(HookEvent::PostToolUseFailure, hooks);
        }
        if let Some(hooks) = config.permission_request {
            self.executor.register(HookEvent::PermissionRequest, hooks);
        }
        if let Some(hooks) = config.user_prompt_submit {
            self.executor.register(HookEvent::UserPromptSubmit, hooks);
        }
        if let Some(hooks) = config.session_start {
            self.executor.register(HookEvent::SessionStart, hooks);
        }
        if let Some(hooks) = config.session_end {
            self.executor.register(HookEvent::SessionEnd, hooks);
        }
        if let Some(hooks) = config.notification {
            self.executor.register(HookEvent::Notification, hooks);
        }
        if let Some(hooks) = config.stop {
            self.executor.register(HookEvent::Stop, hooks);
        }
        if let Some(hooks) = config.subagent_stop {
            self.executor.register(HookEvent::SubagentStop, hooks);
        }
        if let Some(hooks) = config.pre_compact {
            self.executor.register(HookEvent::PreCompact, hooks);
        }

        Ok(())
    }

    /// Registers a hook definition for a specific event.
    pub fn register_hook(&mut self, event: HookEvent, definition: HookDefinition) {
        self.executor.register(event, vec![definition]);
    }

    /// Registers a tool hook with optional matcher pattern.
    ///
    /// This is a convenience method for registering tool-related hooks.
    pub fn register_tool_hook(&mut self, event: HookEvent, matcher: Option<&str>, command: &str) {
        let definition = HookDefinition {
            matcher: matcher.map(String::from),
            hooks: vec![HookCommand {
                hook_type: "command".to_string(),
                command: command.to_string(),
                timeout_ms: Some(30000),
            }],
        };
        self.executor.register(event, vec![definition]);
    }

    /// Fires the SessionStart event.
    ///
    /// Called when the application session begins.
    pub async fn fire_session_start(&self) -> Result<HookResult> {
        let context = HookContext {
            hook_event_name: HookEvent::SessionStart.as_str().to_string(),
            session_id: self.session_id.clone(),
            tool_name: None,
            tool_input: None,
            tool_response: None,
            prompt: None,
            stop_reason: None,
        };
        self.executor
            .execute(HookEvent::SessionStart, &context)
            .await
    }

    /// Fires the SessionEnd event.
    ///
    /// Called when the application session ends.
    ///
    /// # Arguments
    ///
    /// * `stop_reason` - Optional reason for ending the session (e.g., "user_exit", "error")
    pub async fn fire_session_end(&self, stop_reason: Option<&str>) -> Result<HookResult> {
        let context = HookContext {
            hook_event_name: HookEvent::SessionEnd.as_str().to_string(),
            session_id: self.session_id.clone(),
            tool_name: None,
            tool_input: None,
            tool_response: None,
            prompt: None,
            stop_reason: stop_reason.map(String::from),
        };
        self.executor.execute(HookEvent::SessionEnd, &context).await
    }

    /// Fires the UserPromptSubmit event.
    ///
    /// Called when the user submits a prompt before it's sent to the API.
    ///
    /// # Arguments
    ///
    /// * `prompt` - The user's input text
    pub async fn fire_user_prompt_submit(&self, prompt: &str) -> Result<HookResult> {
        let context = HookContext {
            hook_event_name: HookEvent::UserPromptSubmit.as_str().to_string(),
            session_id: self.session_id.clone(),
            tool_name: None,
            tool_input: None,
            tool_response: None,
            prompt: Some(prompt.to_string()),
            stop_reason: None,
        };
        self.executor
            .execute(HookEvent::UserPromptSubmit, &context)
            .await
    }

    /// Fires the PreToolUse event.
    ///
    /// Called before a tool is executed.
    ///
    /// # Arguments
    ///
    /// * `tool_name` - The name of the tool being called
    /// * `tool_input` - The input parameters for the tool
    pub async fn fire_pre_tool_use(
        &self,
        tool_name: &str,
        tool_input: serde_json::Value,
    ) -> Result<HookResult> {
        let context = HookContext {
            hook_event_name: HookEvent::PreToolUse.as_str().to_string(),
            session_id: self.session_id.clone(),
            tool_name: Some(tool_name.to_string()),
            tool_input: Some(tool_input),
            tool_response: None,
            prompt: None,
            stop_reason: None,
        };
        self.executor.execute(HookEvent::PreToolUse, &context).await
    }

    /// Fires the PostToolUse event.
    ///
    /// Called after a tool executes successfully.
    ///
    /// # Arguments
    ///
    /// * `tool_name` - The name of the tool that was called
    /// * `tool_input` - The input parameters that were passed to the tool
    /// * `tool_response` - The response from the tool execution
    pub async fn fire_post_tool_use(
        &self,
        tool_name: &str,
        tool_input: serde_json::Value,
        tool_response: serde_json::Value,
    ) -> Result<HookResult> {
        let context = HookContext {
            hook_event_name: HookEvent::PostToolUse.as_str().to_string(),
            session_id: self.session_id.clone(),
            tool_name: Some(tool_name.to_string()),
            tool_input: Some(tool_input),
            tool_response: Some(tool_response),
            prompt: None,
            stop_reason: None,
        };
        self.executor
            .execute(HookEvent::PostToolUse, &context)
            .await
    }

    /// Fires the PostToolUseFailure event.
    ///
    /// Called after a tool execution fails.
    ///
    /// # Arguments
    ///
    /// * `tool_name` - The name of the tool that failed
    /// * `tool_input` - The input parameters that were passed to the tool
    /// * `error_response` - Error information from the failed execution
    pub async fn fire_post_tool_use_failure(
        &self,
        tool_name: &str,
        tool_input: serde_json::Value,
        error_response: serde_json::Value,
    ) -> Result<HookResult> {
        let context = HookContext {
            hook_event_name: HookEvent::PostToolUseFailure.as_str().to_string(),
            session_id: self.session_id.clone(),
            tool_name: Some(tool_name.to_string()),
            tool_input: Some(tool_input),
            tool_response: Some(error_response),
            prompt: None,
            stop_reason: None,
        };
        self.executor
            .execute(HookEvent::PostToolUseFailure, &context)
            .await
    }

    /// Fires the PermissionRequest event.
    ///
    /// Called when a permission is requested for a tool operation.
    ///
    /// # Arguments
    ///
    /// * `tool_name` - The tool requesting permission
    /// * `tool_input` - The tool's input parameters
    pub async fn fire_permission_request(
        &self,
        tool_name: &str,
        tool_input: serde_json::Value,
    ) -> Result<HookResult> {
        let context = HookContext {
            hook_event_name: HookEvent::PermissionRequest.as_str().to_string(),
            session_id: self.session_id.clone(),
            tool_name: Some(tool_name.to_string()),
            tool_input: Some(tool_input),
            tool_response: None,
            prompt: None,
            stop_reason: None,
        };
        self.executor
            .execute(HookEvent::PermissionRequest, &context)
            .await
    }

    /// Fires the Stop event.
    ///
    /// Called when stop is requested.
    ///
    /// # Arguments
    ///
    /// * `stop_reason` - The reason for stopping
    pub async fn fire_stop(&self, stop_reason: &str) -> Result<HookResult> {
        let context = HookContext {
            hook_event_name: HookEvent::Stop.as_str().to_string(),
            session_id: self.session_id.clone(),
            tool_name: None,
            tool_input: None,
            tool_response: None,
            prompt: None,
            stop_reason: Some(stop_reason.to_string()),
        };
        self.executor.execute(HookEvent::Stop, &context).await
    }

    /// Fires the SubagentStop event.
    ///
    /// Called when a subagent stops.
    ///
    /// # Arguments
    ///
    /// * `subagent_id` - The ID of the subagent that stopped
    /// * `stop_reason` - The reason for stopping
    pub async fn fire_subagent_stop(
        &self,
        subagent_id: &str,
        stop_reason: &str,
    ) -> Result<HookResult> {
        let context = HookContext {
            hook_event_name: HookEvent::SubagentStop.as_str().to_string(),
            session_id: self.session_id.clone(),
            tool_name: Some(subagent_id.to_string()),
            tool_input: None,
            tool_response: None,
            prompt: None,
            stop_reason: Some(stop_reason.to_string()),
        };
        self.executor
            .execute(HookEvent::SubagentStop, &context)
            .await
    }

    /// Fires the Notification event.
    ///
    /// Called when a notification is sent.
    ///
    /// # Arguments
    ///
    /// * `message` - The notification message
    pub async fn fire_notification(&self, message: &str) -> Result<HookResult> {
        let context = HookContext {
            hook_event_name: HookEvent::Notification.as_str().to_string(),
            session_id: self.session_id.clone(),
            tool_name: None,
            tool_input: None,
            tool_response: None,
            prompt: Some(message.to_string()),
            stop_reason: None,
        };
        self.executor
            .execute(HookEvent::Notification, &context)
            .await
    }

    /// Fires the PreCompact event.
    ///
    /// Called before context compaction occurs.
    pub async fn fire_pre_compact(&self) -> Result<HookResult> {
        let context = HookContext {
            hook_event_name: HookEvent::PreCompact.as_str().to_string(),
            session_id: self.session_id.clone(),
            tool_name: None,
            tool_input: None,
            tool_response: None,
            prompt: None,
            stop_reason: None,
        };
        self.executor.execute(HookEvent::PreCompact, &context).await
    }
}

/// Configuration structure for hooks loaded from TOML.
#[derive(Debug, Deserialize)]
struct HooksConfig {
    #[serde(rename = "PreToolUse")]
    pre_tool_use: Option<Vec<HookDefinition>>,
    #[serde(rename = "PostToolUse")]
    post_tool_use: Option<Vec<HookDefinition>>,
    #[serde(rename = "PostToolUseFailure")]
    post_tool_use_failure: Option<Vec<HookDefinition>>,
    #[serde(rename = "PermissionRequest")]
    permission_request: Option<Vec<HookDefinition>>,
    #[serde(rename = "UserPromptSubmit")]
    user_prompt_submit: Option<Vec<HookDefinition>>,
    #[serde(rename = "SessionStart")]
    session_start: Option<Vec<HookDefinition>>,
    #[serde(rename = "SessionEnd")]
    session_end: Option<Vec<HookDefinition>>,
    #[serde(rename = "Notification")]
    notification: Option<Vec<HookDefinition>>,
    #[serde(rename = "Stop")]
    stop: Option<Vec<HookDefinition>>,
    #[serde(rename = "SubagentStop")]
    subagent_stop: Option<Vec<HookDefinition>>,
    #[serde(rename = "PreCompact")]
    pre_compact: Option<Vec<HookDefinition>>,
}
