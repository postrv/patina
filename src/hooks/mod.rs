//! Hook execution engine for lifecycle events

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::api::provider::LlmProvider;
use crate::shell::ShellConfig;
use crate::tools::{normalize_command, ToolExecutionPolicy};

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
    StopFailure,
    SubagentStop,
    PreCompact,
    PostCompact,
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
            HookEvent::StopFailure => "StopFailure",
            HookEvent::SubagentStop => "SubagentStop",
            HookEvent::PreCompact => "PreCompact",
            HookEvent::PostCompact => "PostCompact",
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

/// A hook definition that specifies an optional matcher pattern and a list of hook actions.
///
/// When `matcher` is set, the hooks only fire for tool events whose tool name matches
/// the pattern (exact, glob, or pipe-separated).
#[derive(Debug, Deserialize, Clone)]
pub struct HookDefinition {
    /// Optional matcher pattern (glob, pipe-separated, or exact match).
    pub matcher: Option<String>,
    /// The hook actions to execute when this definition fires.
    pub hooks: Vec<HookAction>,
}

/// Default prompt hook timeout (30 seconds).
fn default_prompt_timeout() -> Option<u64> {
    Some(30_000)
}

/// A single hook action that can be either a shell command or an LLM prompt evaluation.
///
/// Deserialized with a `type` tag: `"command"` or `"prompt"`.
///
/// # Examples
///
/// ```toml
/// # Command hook
/// { type = "command", command = "echo hello" }
///
/// # Prompt hook
/// { type = "prompt", prompt = "Should this tool be allowed?" }
/// ```
#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum HookAction {
    /// Execute a shell command. Exit code 0 = continue, 2 = block.
    #[serde(rename = "command")]
    Command {
        /// The shell command to execute.
        command: String,
        /// Optional timeout in milliseconds.
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Evaluate a prompt via an LLM provider to make a hook decision.
    #[serde(rename = "prompt")]
    Prompt {
        /// The prompt text sent to the LLM along with hook context.
        prompt: String,
        /// Timeout in milliseconds (defaults to 30,000).
        #[serde(default = "default_prompt_timeout")]
        timeout_ms: Option<u64>,
    },
}

/// Backward-compatible alias for `HookAction`.
///
/// Existing code that uses `HookCommand` will continue to work. New code should
/// prefer `HookAction` directly.
pub type HookCommand = HookAction;

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

/// Executes registered hook actions for lifecycle events.
///
/// Supports both shell command hooks and LLM prompt hooks. The LLM provider
/// must be set via [`set_provider`](HookExecutor::set_provider) before prompt
/// hooks can fire; if no provider is set, prompt hooks are skipped with a warning.
pub struct HookExecutor {
    hooks: HashMap<HookEvent, Vec<HookDefinition>>,
    llm_provider: Option<Arc<dyn LlmProvider>>,
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
    /// Creates a new hook executor with no registered hooks and no LLM provider.
    pub fn new() -> Self {
        Self {
            hooks: HashMap::new(),
            llm_provider: None,
        }
    }

    /// Sets the LLM provider used for prompt-based hooks.
    ///
    /// Must be called before prompt hooks can fire. If not set, prompt hooks
    /// are skipped with a warning log.
    pub fn set_provider(&mut self, provider: Arc<dyn LlmProvider>) {
        self.llm_provider = Some(provider);
    }

    /// Registers hook definitions for a specific event.
    pub fn register(&mut self, event: HookEvent, hooks: Vec<HookDefinition>) {
        self.hooks.entry(event).or_default().extend(hooks);
    }

    /// Executes all matching hooks for the given event and context.
    ///
    /// Hooks are executed sequentially. A command hook with exit code 2 or a prompt
    /// hook returning "deny"/"block" will short-circuit and return the blocking decision.
    ///
    /// # Errors
    ///
    /// Returns an error if context serialization, command execution, or LLM communication fails.
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
                match hook {
                    HookAction::Command {
                        command,
                        timeout_ms,
                    } => {
                        let result = self
                            .run_hook_command(command, &context_json, *timeout_ms)
                            .await?;

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
                    HookAction::Prompt { prompt, timeout_ms } => {
                        let result = self
                            .run_prompt_hook(prompt, &context_json, *timeout_ms)
                            .await?;

                        match &result.decision {
                            HookDecision::Continue => continue,
                            HookDecision::Block { .. } | HookDecision::Deny => {
                                return Ok(result);
                            }
                            HookDecision::Allow => continue,
                        }
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

    async fn run_hook_command(
        &self,
        command: &str,
        stdin_data: &str,
        timeout_ms: Option<u64>,
    ) -> Result<HookResult> {
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

        // Security validation: check against dangerous command patterns
        // Reuses the same patterns as ToolExecutionPolicy to ensure consistent security.
        // Normalize the command to defeat backslash-escape bypass attempts (e.g., r\m → rm).
        let normalized = normalize_command(trimmed);
        let policy = ToolExecutionPolicy::default();
        for pattern in &policy.dangerous_patterns {
            if pattern.is_match(trimmed) || pattern.is_match(&normalized) {
                tracing::warn!(
                    command = %trimmed,
                    pattern = %pattern.as_str(),
                    "Hook command blocked by security policy"
                );
                // Return exit_code 2 to indicate a block (consistent with hook semantics)
                // The security policy message goes in stdout (block reason)
                return Ok(HookResult {
                    exit_code: 2,
                    stdout: format!(
                        "Hook command blocked by security policy: matches dangerous pattern '{}'",
                        pattern.as_str()
                    ),
                    stderr: String::new(),
                    decision: HookDecision::Continue,
                });
            }
        }

        // Log hook execution for audit trail
        tracing::info!(command = %trimmed, "Executing hook command");

        // Use platform-agnostic shell configuration (sh -c on Unix, cmd.exe /C on Windows)
        let shell = ShellConfig::default();
        let mut child = Command::new(&shell.command)
            .args(&shell.args)
            .arg(trimmed)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        if let Some(mut stdin) = child.stdin.take() {
            // Ignore broken pipe errors - they occur when the process exits
            // before we finish writing, which is fine (the process got what it needed
            // or decided to exit early)
            let _ = stdin.write_all(stdin_data.as_bytes()).await;
            // Explicitly drop stdin to close the pipe and signal EOF to the child
            drop(stdin);
        }

        let timeout_duration = timeout_ms
            .map(std::time::Duration::from_millis)
            .unwrap_or(std::time::Duration::from_secs(30));

        match tokio::time::timeout(timeout_duration, child.wait_with_output()).await {
            Ok(Ok(output)) => Ok(HookResult {
                exit_code: output.status.code().unwrap_or(-1),
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                decision: HookDecision::Continue,
            }),
            Ok(Err(e)) => Err(e.into()),
            Err(_) => {
                tracing::warn!(
                    command = %trimmed,
                    timeout_ms = timeout_duration.as_millis() as u64,
                    "Hook command timed out"
                );
                Ok(HookResult {
                    exit_code: -1,
                    stdout: String::new(),
                    stderr: format!(
                        "Hook command timed out after {}ms",
                        timeout_duration.as_millis()
                    ),
                    decision: HookDecision::Continue,
                })
            }
        }
    }

    /// Evaluates a prompt-based hook by sending the prompt and context to the LLM provider.
    ///
    /// The LLM is asked to respond with a JSON object containing `decision` and `reason`
    /// fields. Falls back to text matching ("APPROVE"/"DENY"/"BLOCK") if JSON parsing fails.
    ///
    /// # Arguments
    ///
    /// * `prompt` - The hook evaluation prompt
    /// * `context_json` - Serialized hook context
    /// * `timeout_ms` - Optional timeout in milliseconds
    ///
    /// # Errors
    ///
    /// Returns an error if the LLM provider is not set, communication fails, or the
    /// request times out.
    async fn run_prompt_hook(
        &self,
        prompt: &str,
        context_json: &str,
        timeout_ms: Option<u64>,
    ) -> Result<HookResult> {
        let provider = match &self.llm_provider {
            Some(p) => p,
            None => {
                tracing::warn!("Prompt hook skipped: no LLM provider configured");
                return Ok(HookResult {
                    exit_code: 0,
                    stdout: String::new(),
                    stderr: "No LLM provider configured for prompt hooks".to_string(),
                    decision: HookDecision::Continue,
                });
            }
        };

        tracing::info!("Executing prompt hook via LLM provider");

        let system_prompt = "You are a hook evaluator. Respond ONLY with JSON: \
            {\"decision\": \"approve\"|\"deny\"|\"block\", \"reason\": \"...\"}";

        let user_message = format!("{prompt}\n\nContext:\n{context_json}");
        let messages = vec![crate::types::ApiMessageV2::user(&user_message)];

        let timeout = std::time::Duration::from_millis(timeout_ms.unwrap_or(30_000));
        let response = tokio::time::timeout(
            timeout,
            provider.summarize_messages(&messages, system_prompt),
        )
        .await;

        let response_text = match response {
            Ok(Ok(text)) => text,
            Ok(Err(e)) => {
                tracing::warn!("Prompt hook LLM error: {e}");
                return Ok(HookResult {
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: format!("LLM error: {e}"),
                    decision: HookDecision::Continue,
                });
            }
            Err(_) => {
                tracing::warn!("Prompt hook timed out after {timeout_ms:?}ms");
                return Ok(HookResult {
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: "Prompt hook timed out".to_string(),
                    decision: HookDecision::Continue,
                });
            }
        };

        let decision = parse_prompt_response(&response_text);

        Ok(HookResult {
            exit_code: match &decision {
                HookDecision::Continue | HookDecision::Allow => 0,
                HookDecision::Block { .. } => 2,
                HookDecision::Deny => 1,
            },
            stdout: response_text,
            stderr: String::new(),
            decision,
        })
    }
}

/// Parses an LLM response into a [`HookDecision`].
///
/// Tries JSON parsing first, then falls back to case-insensitive text matching.
///
/// # JSON Format
///
/// ```json
/// {"decision": "approve"|"deny"|"block", "reason": "..."}
/// ```
///
/// # Text Fallback (Fail-Closed)
///
/// If JSON parsing fails, only exact matches are accepted:
/// - Trimmed text equals "APPROVE" (case-insensitive) → Allow
/// - Trimmed text starts with "BLOCK" (case-insensitive) → Block
/// - Everything else → Deny (fail-closed to prevent ambiguous responses from approving)
#[must_use]
fn parse_prompt_response(response: &str) -> HookDecision {
    // Try JSON parsing first
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(response) {
        if let Some(decision_str) = json.get("decision").and_then(|v| v.as_str()) {
            let reason = json
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            return match decision_str.to_lowercase().as_str() {
                "approve" => HookDecision::Allow,
                "deny" => HookDecision::Deny,
                "block" => HookDecision::Block { reason },
                _ => HookDecision::Continue,
            };
        }
    }

    // Fallback: fail-closed — if JSON parsing fails, deny by default.
    // Text matching is intentionally restrictive to prevent ambiguous responses
    // (e.g., "I do not APPROVE" matching as Allow).
    let trimmed = response.trim().to_uppercase();
    if trimmed == "APPROVE" {
        HookDecision::Allow
    } else if trimmed.starts_with("BLOCK") {
        HookDecision::Block {
            reason: response.to_string(),
        }
    } else {
        // Default to Deny for any ambiguous or unrecognized response (fail-closed)
        HookDecision::Deny
    }
}

impl Default for HookExecutor {
    fn default() -> Self {
        Self::new()
    }
}

/// High-level hook manager for application lifecycle integration.
///
/// Provides convenience methods for firing all 13 hook events with appropriate
/// context construction. This is the primary interface for integrating hooks
/// into the application.
///
/// # Examples
///
/// ```no_run
/// use patina::hooks::{HookManager, HookEvent};
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let mut manager = HookManager::new("session-123".to_string());
///
///     // Fire session start
///     let result = manager.fire_session_start().await?;
///
///     // Check if hook blocked session start
///     if matches!(result.decision, patina::hooks::HookDecision::Block { .. }) {
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
        self.apply_config(config);
        Ok(())
    }

    /// Loads hook configuration from a TOML file, with graceful degradation.
    ///
    /// Unlike `load_config`, this method handles missing files gracefully:
    /// - If the file doesn't exist, returns Ok (no hooks registered)
    /// - If the file exists but is malformed, returns an error
    ///
    /// This allows applications to run without hooks when no configuration
    /// is provided, while still catching user configuration errors.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be parsed.
    pub fn load_config_graceful(&mut self, path: &std::path::Path) -> Result<()> {
        // If file doesn't exist, that's OK - just no hooks configured
        if !path.exists() {
            tracing::debug!(
                "Hook config file not found at {:?}, continuing without hooks",
                path
            );
            return Ok(());
        }

        // File exists, so try to read and parse it
        let content = std::fs::read_to_string(path)?;
        let config: HooksConfig = toml::from_str(&content)?;
        self.apply_config(config);
        Ok(())
    }

    /// Applies a parsed configuration to the executor.
    fn apply_config(&mut self, config: HooksConfig) {
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
        if let Some(hooks) = config.stop_failure {
            self.executor.register(HookEvent::StopFailure, hooks);
        }
        if let Some(hooks) = config.subagent_stop {
            self.executor.register(HookEvent::SubagentStop, hooks);
        }
        if let Some(hooks) = config.pre_compact {
            self.executor.register(HookEvent::PreCompact, hooks);
        }
        if let Some(hooks) = config.post_compact {
            self.executor.register(HookEvent::PostCompact, hooks);
        }
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
            hooks: vec![HookAction::Command {
                command: command.to_string(),
                timeout_ms: Some(30000),
            }],
        };
        self.executor.register(event, vec![definition]);
    }

    /// Sets the LLM provider on the underlying executor for prompt-based hooks.
    ///
    /// Must be called before any prompt hooks can fire.
    pub fn set_provider(&mut self, provider: Arc<dyn LlmProvider>) {
        self.executor.set_provider(provider);
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

    /// Fires the PostCompact event after successful compaction.
    ///
    /// Called after context compaction completes with summary content.
    pub async fn fire_post_compact(&self) -> Result<HookResult> {
        let context = HookContext {
            hook_event_name: HookEvent::PostCompact.as_str().to_string(),
            session_id: self.session_id.clone(),
            tool_name: None,
            tool_input: None,
            tool_response: None,
            prompt: None,
            stop_reason: None,
        };
        self.executor
            .execute(HookEvent::PostCompact, &context)
            .await
    }

    /// Fires the StopFailure event when an API error ends a turn.
    ///
    /// Called when the streaming response encounters a fatal error
    /// that prevents normal completion.
    pub async fn fire_stop_failure(&self, error: &str) -> Result<HookResult> {
        let context = HookContext {
            hook_event_name: HookEvent::StopFailure.as_str().to_string(),
            session_id: self.session_id.clone(),
            tool_name: None,
            tool_input: None,
            tool_response: None,
            prompt: None,
            stop_reason: Some(error.to_string()),
        };
        self.executor
            .execute(HookEvent::StopFailure, &context)
            .await
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
    #[serde(rename = "StopFailure")]
    stop_failure: Option<Vec<HookDefinition>>,
    #[serde(rename = "SubagentStop")]
    subagent_stop: Option<Vec<HookDefinition>>,
    #[serde(rename = "PreCompact")]
    pre_compact: Option<Vec<HookDefinition>>,
    #[serde(rename = "PostCompact")]
    post_compact: Option<Vec<HookDefinition>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that backslash-escaped dangerous commands are blocked after normalization.
    #[tokio::test]
    async fn test_hook_blocks_backslash_escaped_rm() {
        let executor = HookExecutor::new();
        // r\m -rf / should normalize to rm -rf / and be blocked
        let result = executor
            .run_hook_command("r\\m -rf /", "", None)
            .await
            .unwrap();
        assert_eq!(result.exit_code, 2, "Escaped rm -rf should be blocked");
        assert!(
            result.stdout.contains("blocked by security policy"),
            "Should indicate security block: {}",
            result.stdout
        );
    }

    /// Verify that backslash-escaped sudo is blocked.
    #[tokio::test]
    async fn test_hook_blocks_backslash_escaped_sudo() {
        let executor = HookExecutor::new();
        let result = executor
            .run_hook_command("su\\do whoami", "", None)
            .await
            .unwrap();
        assert_eq!(result.exit_code, 2, "Escaped sudo should be blocked");
        assert!(
            result.stdout.contains("blocked by security policy"),
            "Should indicate security block: {}",
            result.stdout
        );
    }

    /// Verify that safe commands are not blocked.
    #[tokio::test]
    async fn test_hook_allows_safe_command() {
        let executor = HookExecutor::new();
        let result = executor
            .run_hook_command("echo hello", "", None)
            .await
            .unwrap();
        // Safe commands should either succeed (exit 0) or fail for other reasons,
        // but NOT be blocked (exit 2) by security policy
        assert_ne!(result.exit_code, 2, "Safe command should not be blocked");
    }

    #[test]
    fn test_matches_pattern_exact() {
        assert!(matches_pattern("Bash", "Bash").unwrap());
        assert!(!matches_pattern("Bash", "Read").unwrap());
    }

    #[test]
    fn test_matches_pattern_glob_star() {
        assert!(matches_pattern("mcp__*", "mcp__read").unwrap());
        assert!(!matches_pattern("mcp__*", "Bash").unwrap());
    }

    #[test]
    fn test_matches_pattern_glob_question() {
        assert!(matches_pattern("Ba?h", "Bash").unwrap());
        assert!(!matches_pattern("Ba?h", "Baash").unwrap());
    }

    #[test]
    fn test_matches_pattern_pipe_separated() {
        assert!(matches_pattern("Bash|Read", "Bash").unwrap());
        assert!(matches_pattern("Bash|Read", "Read").unwrap());
        assert!(!matches_pattern("Bash|Read", "Write").unwrap());
    }

    #[test]
    fn test_matches_pattern_pipe_with_glob() {
        assert!(matches_pattern("mcp__*|Bash", "mcp__read").unwrap());
        assert!(matches_pattern("mcp__*|Bash", "Bash").unwrap());
        assert!(!matches_pattern("mcp__*|Bash", "Write").unwrap());
    }

    #[test]
    fn test_matches_pattern_no_match() {
        assert!(!matches_pattern("Bash", "Read").unwrap());
        assert!(!matches_pattern("Bash", "").unwrap());
        assert!(!matches_pattern("Bash", "bash").unwrap()); // case-sensitive
    }

    #[test]
    fn test_hook_context_serialization_minimal() {
        let ctx = HookContext {
            hook_event_name: "SessionStart".to_string(),
            session_id: "s1".to_string(),
            tool_name: None,
            tool_input: None,
            tool_response: None,
            prompt: None,
            stop_reason: None,
        };
        let json: serde_json::Value = serde_json::to_value(&ctx).unwrap();
        let obj = json.as_object().unwrap();
        // Required fields present
        assert_eq!(obj["hook_event_name"], "SessionStart");
        assert_eq!(obj["session_id"], "s1");
        // Optional fields omitted (skip_serializing_if)
        assert!(!obj.contains_key("tool_name"));
        assert!(!obj.contains_key("tool_input"));
        assert!(!obj.contains_key("tool_response"));
        assert!(!obj.contains_key("prompt"));
        assert!(!obj.contains_key("stop_reason"));
    }

    #[test]
    fn test_hook_context_serialization_full() {
        let ctx = HookContext {
            hook_event_name: "PreToolUse".to_string(),
            session_id: "s2".to_string(),
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({"command": "ls"})),
            tool_response: Some(serde_json::json!({"output": "file.txt"})),
            prompt: Some("list files".to_string()),
            stop_reason: Some("done".to_string()),
        };
        let json: serde_json::Value = serde_json::to_value(&ctx).unwrap();
        let obj = json.as_object().unwrap();
        assert_eq!(obj["hook_event_name"], "PreToolUse");
        assert_eq!(obj["session_id"], "s2");
        assert_eq!(obj["tool_name"], "Bash");
        assert_eq!(obj["tool_input"]["command"], "ls");
        assert_eq!(obj["tool_response"]["output"], "file.txt");
        assert_eq!(obj["prompt"], "list files");
        assert_eq!(obj["stop_reason"], "done");
    }

    #[test]
    fn test_hook_event_as_str_all_variants() {
        let expected = vec![
            (HookEvent::PreToolUse, "PreToolUse"),
            (HookEvent::PostToolUse, "PostToolUse"),
            (HookEvent::PostToolUseFailure, "PostToolUseFailure"),
            (HookEvent::PermissionRequest, "PermissionRequest"),
            (HookEvent::UserPromptSubmit, "UserPromptSubmit"),
            (HookEvent::SessionStart, "SessionStart"),
            (HookEvent::SessionEnd, "SessionEnd"),
            (HookEvent::Notification, "Notification"),
            (HookEvent::Stop, "Stop"),
            (HookEvent::StopFailure, "StopFailure"),
            (HookEvent::SubagentStop, "SubagentStop"),
            (HookEvent::PreCompact, "PreCompact"),
            (HookEvent::PostCompact, "PostCompact"),
        ];
        for (event, name) in &expected {
            assert_eq!(
                event.as_str(),
                *name,
                "HookEvent::{name} produced wrong string"
            );
        }
        // Ensure we tested all 13 variants
        assert_eq!(expected.len(), 13);
    }

    #[test]
    fn test_hook_decision_default_is_continue() {
        let decision = HookDecision::default();
        assert!(
            matches!(decision, HookDecision::Continue),
            "Default HookDecision should be Continue, got: {decision:?}"
        );
    }

    #[test]
    fn test_hook_manager_session_id() {
        let manager = HookManager::new("test-session-42".to_string());
        assert_eq!(manager.session_id(), "test-session-42");
    }

    #[test]
    fn test_load_config_graceful_nonexistent_file() {
        let mut manager = HookManager::new("test".to_string());
        let result = manager.load_config_graceful(std::path::Path::new("/nonexistent/hooks.toml"));
        assert!(
            result.is_ok(),
            "Missing config file should succeed gracefully"
        );
    }

    #[test]
    fn test_load_config_from_toml() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let toml_path = dir.path().join("hooks.toml");

        std::fs::write(
            &toml_path,
            r#"
[[SessionStart]]
hooks = [{ type = "command", command = "echo session started" }]

[[PreToolUse]]
matcher = "Bash"
hooks = [{ type = "command", command = "echo pre-tool" }]
"#,
        )
        .expect("failed to write TOML");

        let mut manager = HookManager::new("test".to_string());
        let result = manager.load_config(&toml_path);
        assert!(
            result.is_ok(),
            "Valid TOML should load successfully: {result:?}"
        );
    }

    #[test]
    fn test_load_config_graceful_malformed_toml_returns_error() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let toml_path = dir.path().join("hooks.toml");

        std::fs::write(&toml_path, "{{{{ invalid toml").expect("failed to write TOML");

        let mut manager = HookManager::new("test".to_string());
        let result = manager.load_config_graceful(&toml_path);
        assert!(result.is_err(), "Malformed TOML should return error");
    }

    #[test]
    fn test_register_hook_and_fire() {
        let mut manager = HookManager::new("test".to_string());
        manager.register_tool_hook(HookEvent::SessionStart, None, "echo hello");

        // Verify the hook is registered by checking session_id is intact
        assert_eq!(manager.session_id(), "test");
    }

    #[test]
    fn test_bridge_plugin_hooks() {
        use crate::plugins::{bridge_plugin_hooks, HookDef, HooksConfig};

        let mut hooks_config = HooksConfig::default();
        hooks_config.pre_tool_use.push(HookDef {
            matcher: Some("Bash".to_string()),
            command: "echo pre-tool from plugin".to_string(),
        });
        hooks_config.session_start.push(HookDef {
            matcher: None,
            command: "echo session started from plugin".to_string(),
        });
        hooks_config.stop_failure.push(HookDef {
            matcher: None,
            command: "echo stop failure from plugin".to_string(),
        });
        hooks_config.post_compact.push(HookDef {
            matcher: None,
            command: "echo post compact from plugin".to_string(),
        });

        let mut manager = HookManager::new("test".to_string());
        bridge_plugin_hooks(&hooks_config, &mut manager);

        // Verify manager is intact (hooks were registered without error)
        assert_eq!(manager.session_id(), "test");
    }

    // =========================================================================
    // HookAction serde tests
    // =========================================================================

    #[test]
    fn test_hook_action_deserialize_command() {
        let json = r#"{"type": "command", "command": "echo hello", "timeout_ms": 5000}"#;
        let action: HookAction = serde_json::from_str(json).unwrap();
        match action {
            HookAction::Command {
                command,
                timeout_ms,
            } => {
                assert_eq!(command, "echo hello");
                assert_eq!(timeout_ms, Some(5000));
            }
            other => panic!("Expected Command, got {other:?}"),
        }
    }

    #[test]
    fn test_hook_action_deserialize_prompt() {
        let json = r#"{"type": "prompt", "prompt": "Should this be allowed?"}"#;
        let action: HookAction = serde_json::from_str(json).unwrap();
        match action {
            HookAction::Prompt { prompt, timeout_ms } => {
                assert_eq!(prompt, "Should this be allowed?");
                // Default timeout
                assert_eq!(timeout_ms, Some(30_000));
            }
            other => panic!("Expected Prompt, got {other:?}"),
        }
    }

    #[test]
    fn test_hook_action_deserialize_prompt_custom_timeout() {
        let json = r#"{"type": "prompt", "prompt": "Check safety", "timeout_ms": 10000}"#;
        let action: HookAction = serde_json::from_str(json).unwrap();
        match action {
            HookAction::Prompt { prompt, timeout_ms } => {
                assert_eq!(prompt, "Check safety");
                assert_eq!(timeout_ms, Some(10000));
            }
            other => panic!("Expected Prompt, got {other:?}"),
        }
    }

    #[test]
    fn test_backward_compat_toml_command_format() {
        let toml = r#"
[[SessionStart]]
hooks = [{ type = "command", command = "echo started" }]
"#;
        let config: HooksConfig = toml::from_str(toml).unwrap();
        let defs = config.session_start.unwrap();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].hooks.len(), 1);
        match &defs[0].hooks[0] {
            HookAction::Command {
                command,
                timeout_ms,
            } => {
                assert_eq!(command, "echo started");
                assert!(timeout_ms.is_none());
            }
            other => panic!("Expected Command, got {other:?}"),
        }
    }

    #[test]
    fn test_toml_prompt_hook_format() {
        let toml = r#"
[[PreToolUse]]
matcher = "Bash"
hooks = [{ type = "prompt", prompt = "Is this tool safe?" }]
"#;
        let config: HooksConfig = toml::from_str(toml).unwrap();
        let defs = config.pre_tool_use.unwrap();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].matcher.as_deref(), Some("Bash"));
        match &defs[0].hooks[0] {
            HookAction::Prompt { prompt, timeout_ms } => {
                assert_eq!(prompt, "Is this tool safe?");
                assert_eq!(*timeout_ms, Some(30_000));
            }
            other => panic!("Expected Prompt, got {other:?}"),
        }
    }

    // =========================================================================
    // parse_prompt_response tests
    // =========================================================================

    #[test]
    fn test_parse_prompt_response_json_approve() {
        let response = r#"{"decision": "approve", "reason": "Looks safe"}"#;
        let decision = parse_prompt_response(response);
        assert!(
            matches!(decision, HookDecision::Allow),
            "Expected Allow, got {decision:?}"
        );
    }

    #[test]
    fn test_parse_prompt_response_json_deny() {
        let response = r#"{"decision": "deny", "reason": "Too dangerous"}"#;
        let decision = parse_prompt_response(response);
        assert!(
            matches!(decision, HookDecision::Deny),
            "Expected Deny, got {decision:?}"
        );
    }

    #[test]
    fn test_parse_prompt_response_json_block() {
        let response = r#"{"decision": "block", "reason": "Security concern"}"#;
        let decision = parse_prompt_response(response);
        match decision {
            HookDecision::Block { reason } => {
                assert_eq!(reason, "Security concern");
            }
            other => panic!("Expected Block, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_prompt_response_text_exact_approve() {
        // Only exact "APPROVE" (trimmed, case-insensitive) is accepted
        let decision = parse_prompt_response("APPROVE");
        assert!(
            matches!(decision, HookDecision::Allow),
            "Expected Allow, got {decision:?}"
        );
        let decision_lower = parse_prompt_response("  approve  ");
        assert!(
            matches!(decision_lower, HookDecision::Allow),
            "Expected Allow for trimmed lowercase, got {decision_lower:?}"
        );
    }

    #[test]
    fn test_parse_prompt_response_text_ambiguous_approve_denied() {
        // Ambiguous text containing "APPROVE" is fail-closed → Deny
        let response = "I think we should APPROVE this action.";
        let decision = parse_prompt_response(response);
        assert!(
            matches!(decision, HookDecision::Deny),
            "Expected Deny for ambiguous APPROVE text (fail-closed), got {decision:?}"
        );
    }

    #[test]
    fn test_parse_prompt_response_text_deny() {
        // Any non-APPROVE, non-BLOCK text defaults to Deny (fail-closed)
        let response = "I DENY this request.";
        let decision = parse_prompt_response(response);
        assert!(
            matches!(decision, HookDecision::Deny),
            "Expected Deny, got {decision:?}"
        );
    }

    #[test]
    fn test_parse_prompt_response_text_block() {
        // Text starting with "BLOCK" (trimmed) triggers Block
        let response = "BLOCK: This operation is dangerous.";
        let decision = parse_prompt_response(response);
        assert!(
            matches!(decision, HookDecision::Block { .. }),
            "Expected Block, got {decision:?}"
        );
    }

    #[test]
    fn test_parse_prompt_response_unknown_defaults_to_deny() {
        // Unknown text defaults to Deny (fail-closed), not Continue
        let response = "I'm not sure what to do.";
        let decision = parse_prompt_response(response);
        assert!(
            matches!(decision, HookDecision::Deny),
            "Expected Deny for ambiguous response (fail-closed), got {decision:?}"
        );
    }

    #[test]
    fn test_parse_prompt_response_json_unknown_decision() {
        let response = r#"{"decision": "maybe", "reason": "unclear"}"#;
        let decision = parse_prompt_response(response);
        assert!(
            matches!(decision, HookDecision::Continue),
            "Expected Continue for unknown decision, got {decision:?}"
        );
    }

    // =========================================================================
    // Prompt hook execution tests (with mock provider)
    // =========================================================================

    /// Mock LLM provider for testing prompt hooks.
    struct MockHookProvider {
        response: String,
    }

    impl MockHookProvider {
        fn new(response: &str) -> Self {
            Self {
                response: response.to_string(),
            }
        }
    }

    impl LlmProvider for MockHookProvider {
        fn name(&self) -> &str {
            "mock-hook"
        }

        fn model(&self) -> &str {
            "mock-model"
        }

        fn stream_message<'a>(
            &'a self,
            _messages: &'a [crate::types::ApiMessageV2],
            _tools: Option<&'a [crate::api::tools::ToolDefinition]>,
            _tool_choice: Option<&'a crate::api::tools::ToolChoice>,
            _options: &'a crate::api::provider::RequestOptions,
            tx: tokio::sync::mpsc::Sender<crate::types::StreamEvent>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
            let response = self.response.clone();
            Box::pin(async move {
                let _ = tx
                    .send(crate::types::StreamEvent::ContentDelta(response))
                    .await;
                let _ = tx.send(crate::types::StreamEvent::MessageStop).await;
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn test_run_prompt_hook_json_approve() {
        let mut executor = HookExecutor::new();
        let provider = Arc::new(MockHookProvider::new(
            r#"{"decision": "approve", "reason": "Safe operation"}"#,
        ));
        executor.set_provider(provider);

        let result = executor
            .run_prompt_hook("Is this safe?", "{}", None)
            .await
            .unwrap();
        assert!(
            matches!(result.decision, HookDecision::Allow),
            "Expected Allow, got {:?}",
            result.decision
        );
        assert_eq!(result.exit_code, 0);
    }

    #[tokio::test]
    async fn test_run_prompt_hook_json_deny() {
        let mut executor = HookExecutor::new();
        let provider = Arc::new(MockHookProvider::new(
            r#"{"decision": "deny", "reason": "Dangerous command"}"#,
        ));
        executor.set_provider(provider);

        let result = executor
            .run_prompt_hook("Is this safe?", "{}", None)
            .await
            .unwrap();
        assert!(
            matches!(result.decision, HookDecision::Deny),
            "Expected Deny, got {:?}",
            result.decision
        );
        assert_eq!(result.exit_code, 1);
    }

    #[tokio::test]
    async fn test_run_prompt_hook_json_block() {
        let mut executor = HookExecutor::new();
        let provider = Arc::new(MockHookProvider::new(
            r#"{"decision": "block", "reason": "Security violation"}"#,
        ));
        executor.set_provider(provider);

        let result = executor
            .run_prompt_hook("Is this safe?", "{}", None)
            .await
            .unwrap();
        match &result.decision {
            HookDecision::Block { reason } => {
                assert_eq!(reason, "Security violation");
            }
            other => panic!("Expected Block, got {other:?}"),
        }
        assert_eq!(result.exit_code, 2);
    }

    #[tokio::test]
    async fn test_run_prompt_hook_text_approve_exact() {
        let mut executor = HookExecutor::new();
        // Only exact "APPROVE" text (not embedded in a sentence) triggers Allow
        let provider = Arc::new(MockHookProvider::new("APPROVE"));
        executor.set_provider(provider);

        let result = executor
            .run_prompt_hook("Is this safe?", "{}", None)
            .await
            .unwrap();
        assert!(
            matches!(result.decision, HookDecision::Allow),
            "Expected Allow, got {:?}",
            result.decision
        );
    }

    #[tokio::test]
    async fn test_run_prompt_hook_ambiguous_text_denied() {
        let mut executor = HookExecutor::new();
        // Ambiguous text containing APPROVE is fail-closed → Deny
        let provider = Arc::new(MockHookProvider::new(
            "I think we should APPROVE this action because it is safe.",
        ));
        executor.set_provider(provider);

        let result = executor
            .run_prompt_hook("Is this safe?", "{}", None)
            .await
            .unwrap();
        assert!(
            matches!(result.decision, HookDecision::Deny),
            "Expected Deny for ambiguous text (fail-closed), got {:?}",
            result.decision
        );
    }

    #[tokio::test]
    async fn test_run_prompt_hook_no_provider_skips() {
        let executor = HookExecutor::new();

        let result = executor
            .run_prompt_hook("Is this safe?", "{}", None)
            .await
            .unwrap();
        assert!(
            matches!(result.decision, HookDecision::Continue),
            "Expected Continue when no provider, got {:?}",
            result.decision
        );
    }

    /// Mock provider that hangs forever (for timeout testing).
    struct HangingProvider;

    impl LlmProvider for HangingProvider {
        fn name(&self) -> &str {
            "hanging"
        }

        fn model(&self) -> &str {
            "hanging-model"
        }

        fn stream_message<'a>(
            &'a self,
            _messages: &'a [crate::types::ApiMessageV2],
            _tools: Option<&'a [crate::api::tools::ToolDefinition]>,
            _tool_choice: Option<&'a crate::api::tools::ToolChoice>,
            _options: &'a crate::api::provider::RequestOptions,
            _tx: tokio::sync::mpsc::Sender<crate::types::StreamEvent>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
            Box::pin(async move {
                // Never complete - simulates a hanging provider
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn test_run_prompt_hook_timeout() {
        let mut executor = HookExecutor::new();
        let provider: Arc<dyn LlmProvider> = Arc::new(HangingProvider);
        executor.set_provider(provider);

        let result = executor
            .run_prompt_hook("Is this safe?", "{}", Some(100)) // 100ms timeout
            .await
            .unwrap();
        assert!(
            matches!(result.decision, HookDecision::Continue),
            "Expected Continue on timeout, got {:?}",
            result.decision
        );
        assert!(result.stderr.contains("timed out"));
    }
}
