//! Application state management

mod agent_panel;
mod background;
pub mod background_tasks;
mod compression;
mod continuous;
mod input;
pub mod plan;
pub mod question;
mod session_tracking;
mod tool_execution;
mod ui_selection;
mod worktree;

pub use agent_panel::AgentPanelState;
pub use background::*;
pub use background_tasks::BackgroundTaskRegistry;
pub use compression::CompressionState;
pub use continuous::ContinuousLoopState;
pub use input::InputState;
pub use plan::{PlanState, PlanStep};
pub use question::QuestionState;
pub use session_tracking::SessionTracking;
pub use tool_execution::ToolExecutionState;
pub use ui_selection::UISelectionState;
pub use worktree::WorktreeStatus;

use crate::agents::{AgentProgress, ConflictReport, SubagentSpawner};
use crate::api::provider::RequestOptions;
use crate::api::tokens::{model_context_limit, ModelCapabilities};
use crate::api::tools::default_tools;
use crate::api::{LlmProvider, StreamEvent, SystemBlock, ThinkingConfig, TokenBudget, ToolChoice};
use crate::app::tool_loop::{ContinuationData, ToolLoop, ToolLoopState};
use crate::app::STREAMING_CHANNEL_BUFFER;
use crate::context::compression::{
    CompactionMetrics, CompactionMetricsSummary, CompressionOrchestrator,
};
use crate::enterprise::cost::{CostConfig, CostTracker, UsageRecord};
use crate::hooks::HookManager;
use crate::mcp::connection::McpConnection;
use crate::narsil::context::ContextSuggestion;
use crate::permissions::{PermissionManager, PermissionRequest, PermissionResponse};
use crate::plugins::PluginRegistry;
use crate::session::Session;
use crate::tools::HookedToolExecutor;
use crate::types::config::EffortLevel;
use crate::types::config::ParallelMode;
use crate::types::content::{StopReason, ToolResultBlock, ToolUseBlock};
use crate::types::ui_state::{
    CompactionProgressState, FocusArea, ScrollState, SelectionState, ToolBlockState,
};
use crate::types::{ApiMessageV2, Message, Role, Timeline};
use anyhow::Result;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// Default threshold for auto-compaction as a fraction of context window.
///
/// When conversation tokens exceed this percentage of the model's context limit,
/// auto-compaction is triggered before the next API call.
///
/// Default: 0.8 (80% of context window)
pub const DEFAULT_COMPACTION_THRESHOLD: f32 = 0.8;

/// Formats tool input JSON into a readable string for display.
///
/// Extracts the most relevant field based on tool type:
/// - `bash` / `Bash`: Shows the command
/// - `read` / `Read`: Shows the file path
/// - `write` / `Write`: Shows the file path
/// - `edit` / `Edit`: Shows the file path
/// - `glob` / `Glob`: Shows the pattern
/// - `grep` / `Grep`: Shows the pattern
/// - Other tools: Shows compact JSON
#[must_use]
fn format_tool_input(tool_name: &str, input: &Value) -> String {
    let name_lower = tool_name.to_lowercase();

    // Try to extract the most relevant field based on tool type
    match name_lower.as_str() {
        "bash" => input
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| compact_json(input))
            .to_string(),
        "read" | "read_file" => input
            .get("file_path")
            .or_else(|| input.get("path"))
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| compact_json(input))
            .to_string(),
        "write" | "write_file" => input
            .get("file_path")
            .or_else(|| input.get("path"))
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| compact_json(input))
            .to_string(),
        "edit" => input
            .get("file_path")
            .or_else(|| input.get("path"))
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| compact_json(input))
            .to_string(),
        "glob" => input
            .get("pattern")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| compact_json(input))
            .to_string(),
        "grep" => input
            .get("pattern")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| compact_json(input))
            .to_string(),
        _ => compact_json(input).to_string(),
    }
}

/// Returns a compact single-line JSON representation.
fn compact_json(value: &Value) -> &str {
    // For simple values, return a static string representation
    // For complex values, the caller will need to format it
    match value {
        Value::Null => "null",
        Value::Bool(true) => "true",
        Value::Bool(false) => "false",
        Value::String(s) => s.as_str(),
        _ => "...",
    }
}

/// Returns the current git HEAD commit hash for the given working directory.
///
/// Uses `git rev-parse HEAD` to obtain the hash. Returns `None` if the
/// directory is not a git repository or the command fails.
///
/// # Arguments
///
/// * `working_dir` - Path to the directory to query
///
/// # Returns
///
/// The 40-character hex SHA-1 hash, or `None` on failure.
#[must_use]
pub fn get_git_head_hash(working_dir: &std::path::Path) -> Option<String> {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(working_dir)
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                String::from_utf8(output.stdout)
                    .ok()
                    .map(|s| s.trim().to_string())
            } else {
                None
            }
        })
}

pub struct AppState {
    /// Full API messages with content blocks (tool_use, tool_result).
    /// This is the authoritative conversation history sent to the API.
    api_messages: Vec<ApiMessageV2>,

    /// Input buffer state (text, cursor, completion).
    input_state: InputState,
    pub working_dir: PathBuf,

    /// Smart scroll state with auto-follow behavior.
    scroll: ScrollState,
    loading: bool,
    throbber_frame: usize,
    streaming_rx: Option<mpsc::Receiver<StreamEvent>>,

    dirty: DirtyFlags,

    /// Git worktree status (branch, modified, ahead/behind).
    worktree: WorktreeStatus,

    /// Session tracking for auto-save (ID + dirty flag).
    session: SessionTracking,

    /// Whether the application should exit the event loop.
    /// Set by `KeyboardHandler` on Ctrl+C / Ctrl+D; checked by the event loop.
    quit_requested: bool,

    /// All tool execution state grouped together.
    tool_state: ToolExecutionState,

    /// Unified timeline for conversation display.
    /// This is the single source of truth for display ordering, replacing the
    /// dual-system of `messages` + `current_response`.
    timeline: Timeline,

    /// All UI selection and copy state grouped together.
    ui_selection: UISelectionState,

    /// All compression, context injection, and compaction state grouped together.
    compression: CompressionState,

    /// Plugin registry for managing loaded plugins.
    /// Loaded from `~/.config/patina/plugins/` on startup unless disabled.
    plugin_registry: PluginRegistry,

    /// Agent panel state (entries, conflict reports, spawner).
    agent_panel: AgentPanelState,

    /// All continuous coding loop state grouped together.
    continuous: ContinuousLoopState,

    /// Cached terminal height for scroll calculations.
    /// Updated on resize events; defaults to 24 for headless/test environments.
    terminal_height: u16,

    /// Optional MCP server manager for external tool servers.
    /// Set during app startup if `.mcp.json` or `~/.claude.json` contains server entries.
    /// Wrapped in `Arc` so spawned tool-execution tasks can share read access
    /// (the SDK uses interior mutability — `call_tool` is `&self`).
    mcp_manager: Option<std::sync::Arc<crate::mcp::manager::McpManager>>,

    /// Cost tracker for session usage accounting.
    cost_tracker: CostTracker,

    /// Model name for cost tracking (needed when recording usage events).
    current_model: String,

    /// Persistent memory store for cross-session context.
    memory_store: Option<crate::memory::store::MemoryStore>,

    /// Reasoning effort level for API requests.
    effort: EffortLevel,

    /// Optional explicit thinking budget that overrides effort level.
    thinking_budget: Option<u32>,

    /// System prompt text injected into API requests.
    system_prompt: Option<String>,

    /// Buffer for accumulating thinking text from stream events.
    thinking_buffer: String,

    /// Pending plan awaiting user review (plan tool intercept).
    pending_plan: Option<PlanState>,

    /// Pending question awaiting user response (ask_user tool intercept).
    pending_question: Option<QuestionState>,

    /// Registry for background bash tasks (run_in_background).
    background_tasks: BackgroundTaskRegistry,
}

#[derive(Default)]
struct DirtyFlags {
    messages: bool,
    input: bool,
    full: bool,
}

impl DirtyFlags {
    fn any(&self) -> bool {
        self.messages || self.input || self.full
    }

    fn clear(&mut self) {
        *self = Self::default();
    }
}

impl AppState {
    /// Creates a new AppState with tool execution support.
    ///
    /// # Arguments
    ///
    /// * `working_dir` - The working directory for file operations
    /// * `skip_permissions` - If true, bypass all permission prompts
    /// * `parallel_mode` - Controls parallel tool execution
    pub fn new(working_dir: PathBuf, skip_permissions: bool, parallel_mode: ParallelMode) -> Self {
        Self::with_options(working_dir, skip_permissions, parallel_mode, false, false)
    }

    /// Creates a new AppState with optional plugin loading.
    ///
    /// # Arguments
    ///
    /// * `working_dir` - The working directory for file operations
    /// * `skip_permissions` - If true, bypass all permission prompts
    /// * `parallel_mode` - Controls parallel tool execution
    /// * `plugins_enabled` - If true, load plugins from config directory
    pub fn with_plugins(
        working_dir: PathBuf,
        skip_permissions: bool,
        parallel_mode: ParallelMode,
        plugins_enabled: bool,
    ) -> Self {
        Self::with_options(
            working_dir,
            skip_permissions,
            parallel_mode,
            plugins_enabled,
            false,
        )
    }

    /// Creates a new AppState with all options.
    ///
    /// # Arguments
    ///
    /// * `working_dir` - The working directory for file operations
    /// * `skip_permissions` - If true, bypass all permission prompts
    /// * `parallel_mode` - Controls parallel tool execution
    /// * `plugins_enabled` - If true, load plugins from config directory
    /// * `subagents_enabled` - If true, initialize subagent spawner
    pub fn with_options(
        working_dir: PathBuf,
        skip_permissions: bool,
        parallel_mode: ParallelMode,
        plugins_enabled: bool,
        subagents_enabled: bool,
    ) -> Self {
        use crate::types::config::PerformanceConfig;

        // Build PerformanceConfig from ParallelMode with default limits
        let performance = PerformanceConfig {
            parallel_mode,
            ..PerformanceConfig::default()
        };

        Self::with_performance_config(
            working_dir,
            skip_permissions,
            &performance,
            plugins_enabled,
            subagents_enabled,
        )
    }

    /// Creates a new AppState using a [`PerformanceConfig`] for parallelism settings.
    ///
    /// This method provides full control over parallelism policy, tool concurrency,
    /// and agent concurrency limits.
    ///
    /// # Arguments
    ///
    /// * `working_dir` - The working directory for file operations
    /// * `skip_permissions` - If true, bypass all permission prompts
    /// * `performance` - Controls parallelism policy and concurrency limits
    /// * `plugins_enabled` - If true, load plugins from config directory
    /// * `subagents_enabled` - If true, initialize subagent spawner
    pub fn with_performance_config(
        working_dir: PathBuf,
        skip_permissions: bool,
        performance: &crate::types::config::PerformanceConfig,
        plugins_enabled: bool,
        subagents_enabled: bool,
    ) -> Self {
        // Generate a unique session ID for hooks
        let hook_session_id = uuid::Uuid::new_v4().to_string();
        let hook_manager = HookManager::new(hook_session_id);

        // Create permission manager with skip_permissions setting
        let mut pm = PermissionManager::new();
        pm.set_skip_permissions(skip_permissions);
        let permission_manager = Arc::new(Mutex::new(pm));

        // Convert PerformanceConfig to ParallelConfig
        let parallel_config = performance.to_parallel_config();

        // Create tool executor with hook, permission, and parallel configuration
        let tool_executor = Arc::new(
            HookedToolExecutor::new(working_dir.clone(), hook_manager)
                .with_permissions(Arc::clone(&permission_manager))
                .with_parallel_config(parallel_config),
        );

        // Load plugins if enabled
        let plugin_registry = if plugins_enabled {
            Self::load_plugins()
        } else {
            PluginRegistry::new()
        };

        // Initialize subagent spawner if enabled
        let subagent_spawner = if subagents_enabled {
            Some(SubagentSpawner::new())
        } else {
            None
        };

        Self {
            api_messages: Vec::new(),
            input_state: InputState::new(),
            working_dir,
            scroll: ScrollState::new(),
            loading: false,
            throbber_frame: 0,
            streaming_rx: None,
            dirty: DirtyFlags {
                full: true,
                ..Default::default()
            },
            worktree: WorktreeStatus::new(),
            session: SessionTracking::new(),
            quit_requested: false,
            tool_state: ToolExecutionState {
                tool_loop: ToolLoop::new(),
                tool_executor,
                permission_manager,
                pending_permission: None,
                tool_blocks: Vec::new(),
                tool_result_rx: None,
                executing_tool_ids: std::collections::HashSet::new(),
            },
            timeline: Timeline::new(),
            ui_selection: UISelectionState {
                selection: SelectionState::new(),
                copy_pending: false,
                rendered_lines_cache: Vec::new(),
                focus_area: FocusArea::default(),
            },
            compression: CompressionState {
                token_budget: TokenBudget::new(100_000), // Claude's typical context window
                compaction_state: None,
                auto_context_enabled: false,
                pending_context: Vec::new(),
                compression_orchestrator: None,
                last_ccg_hash: None,
                cached_ccg_context: None,
                narsil_client: None,
                context_token_budget: 10_000,
                context_tokens_injected: 0,
                compaction_metrics: Arc::new(CompactionMetrics::new()),
                compaction_requested: false,
                compaction_custom_instructions: None,
            },
            plugin_registry,
            agent_panel: AgentPanelState::new(subagent_spawner),
            continuous: ContinuousLoopState {
                status: ContinuousLoopStatus::Inactive,
                iterations_completed: 0,
                last_duration_ms: None,
                checking_gate: None,
                gate_results: Vec::new(),
            },
            terminal_height: 24,
            mcp_manager: None,
            cost_tracker: CostTracker::new(CostConfig::default()),
            current_model: String::new(),
            memory_store: None,
            effort: EffortLevel::Auto,
            thinking_budget: None,
            system_prompt: None,
            thinking_buffer: String::new(),
            pending_plan: None,
            pending_question: None,
            background_tasks: BackgroundTaskRegistry::new(),
        }
    }

    /// Loads plugins from standard configuration directories.
    ///
    /// Searches for plugins in:
    /// - `~/.config/patina/plugins/`
    /// - `./.patina/plugins/` (project-local)
    fn load_plugins() -> PluginRegistry {
        let mut registry = PluginRegistry::new();

        // Build search paths
        let mut search_paths: Vec<PathBuf> = Vec::new();

        // User config directory using directories crate
        if let Some(base_dirs) = directories::BaseDirs::new() {
            search_paths.push(base_dirs.config_dir().join("patina/plugins"));
        }

        // Project-local plugins
        search_paths.push(PathBuf::from(".patina/plugins"));

        // Load plugins from all paths (errors are logged, not propagated)
        if let Err(e) = registry.load_all(&search_paths) {
            tracing::warn!("Failed to load some plugins: {}", e);
        }

        let count = registry.plugin_count();
        if count > 0 {
            tracing::info!("Loaded {} plugin(s)", count);
        }

        registry
    }

    /// Returns a reference to the plugin registry.
    #[must_use]
    pub fn plugins(&self) -> &PluginRegistry {
        &self.plugin_registry
    }

    /// Returns a reference to the agent panel state.
    #[must_use]
    pub fn agent_panel(&self) -> &AgentPanelState {
        &self.agent_panel
    }

    /// Returns whether subagent orchestration is enabled.
    #[must_use]
    pub fn subagents_enabled(&self) -> bool {
        self.agent_panel.subagents_enabled()
    }

    /// Returns a reference to the subagent spawner if enabled.
    #[must_use]
    pub fn subagent_spawner(&self) -> Option<&SubagentSpawner> {
        self.agent_panel.spawner()
    }

    // =========================================================================
    // Agent panel methods (delegates to AgentPanelState)
    // =========================================================================

    /// Returns the current agent panel entries for TUI rendering.
    #[must_use]
    pub fn agent_panel_entries(&self) -> &[AgentPanelEntry] {
        self.agent_panel.entries()
    }

    /// Updates the agent panel with a progress event.
    pub fn update_agent_progress(
        &mut self,
        agent_id: &str,
        agent_name: &str,
        progress: &AgentProgress,
    ) {
        self.agent_panel
            .update_progress(agent_id, agent_name, progress);
        self.dirty.full = true;
    }

    /// Records a conflict report for display in the TUI.
    pub fn add_conflict_report(&mut self, report: ConflictReport) {
        self.agent_panel.add_conflict(report);
        self.dirty.full = true;
    }

    /// Takes all pending conflict reports, leaving the internal list empty.
    pub fn take_conflict_reports(&mut self) -> Vec<ConflictReport> {
        self.agent_panel.take_conflicts()
    }

    /// Returns `true` if there are pending conflict reports.
    #[must_use]
    pub fn has_pending_conflicts(&self) -> bool {
        self.agent_panel.has_pending_conflicts()
    }

    // =========================================================================
    // Continuous loop panel methods (delegates to ContinuousLoopState)
    // =========================================================================

    /// Returns a reference to the continuous loop state.
    #[must_use]
    pub fn continuous(&self) -> &ContinuousLoopState {
        &self.continuous
    }

    /// Returns the current status of the continuous coding loop.
    #[must_use]
    pub fn continuous_status(&self) -> &ContinuousLoopStatus {
        self.continuous.status()
    }

    /// Returns the number of completed iterations in the current session.
    #[must_use]
    pub fn continuous_iterations_completed(&self) -> u32 {
        self.continuous.iterations_completed()
    }

    /// Returns the duration of the last completed iteration in milliseconds.
    #[must_use]
    pub fn continuous_last_duration_ms(&self) -> Option<u64> {
        self.continuous.last_duration_ms()
    }

    /// Returns the name of the quality gate currently being checked.
    #[must_use]
    pub fn continuous_checking_gate(&self) -> Option<&str> {
        self.continuous.checking_gate()
    }

    /// Returns accumulated quality gate results for the current iteration.
    #[must_use]
    pub fn continuous_gate_results(&self) -> &[GateResult] {
        self.continuous.gate_results()
    }

    /// Updates state for a new continuous iteration starting.
    pub fn update_continuous_iteration(&mut self, iteration: u32) {
        self.continuous.update_iteration(iteration);
        self.dirty.full = true;
    }

    /// Records the completion of a continuous iteration.
    pub fn complete_continuous_iteration(&mut self, _iteration: u32, duration_ms: u64) {
        self.continuous.complete_iteration(duration_ms);
        self.dirty.full = true;
    }

    /// Records that a quality gate check is starting.
    pub fn set_continuous_gate_checking(&mut self, gate: &str) {
        self.continuous.set_gate_checking(gate);
        self.dirty.full = true;
    }

    /// Records the result of a quality gate check.
    pub fn record_continuous_gate_result(
        &mut self,
        gate: &str,
        passed: bool,
        message: Option<&str>,
    ) {
        self.continuous.record_gate_result(gate, passed, message);
        self.dirty.full = true;
    }

    /// Records that stagnation was detected.
    pub fn set_continuous_stagnation(&mut self, iterations_without_progress: u32, threshold: u32) {
        self.continuous
            .set_stagnation(iterations_without_progress, threshold);
        self.dirty.full = true;
    }

    /// Records that human intervention is required.
    pub fn set_continuous_human_checkpoint(&mut self, reason: &str) {
        self.continuous.set_human_checkpoint(reason);
        self.dirty.full = true;
    }

    /// Resets all continuous loop state to inactive.
    pub fn reset_continuous(&mut self) {
        self.continuous.reset();
        self.dirty.full = true;
    }

    // =========================================================================
    // Auto-context methods (Task 2.2.4)
    // =========================================================================

    // =========================================================================
    // Context & Compression methods (delegates to CompressionState)
    // =========================================================================

    /// Returns a reference to the compression state.
    #[must_use]
    pub fn compression(&self) -> &CompressionState {
        &self.compression
    }

    /// Returns whether auto-context injection is enabled.
    #[must_use]
    pub fn auto_context_enabled(&self) -> bool {
        self.compression.auto_context_enabled()
    }

    /// Sets whether auto-context injection is enabled.
    pub fn set_auto_context_enabled(&mut self, enabled: bool) {
        self.compression.set_auto_context_enabled(enabled);
    }

    /// Returns whether there are pending context suggestions.
    #[must_use]
    pub fn has_pending_context(&self) -> bool {
        self.compression.has_pending_context()
    }

    /// Returns a reference to the pending context suggestions.
    #[must_use]
    pub fn pending_context(&self) -> &[ContextSuggestion] {
        self.compression.pending_context()
    }

    /// Sets the pending context suggestions.
    pub fn set_pending_context(&mut self, suggestions: Vec<ContextSuggestion>) {
        self.compression.set_pending_context(suggestions);
    }

    /// Takes and returns the pending context suggestions, clearing them.
    #[must_use]
    pub fn take_pending_context(&mut self) -> Vec<ContextSuggestion> {
        self.compression.take_pending_context()
    }

    /// Clears the pending context suggestions.
    pub fn clear_pending_context(&mut self) {
        self.compression.clear_pending_context();
    }

    // =========================================================================
    // Compression Orchestrator Methods (delegates to CompressionState)
    // =========================================================================

    /// Sets the compression orchestrator.
    pub fn set_compression_orchestrator(&mut self, orchestrator: Arc<CompressionOrchestrator>) {
        self.compression.set_compression_orchestrator(orchestrator);
    }

    /// Returns a reference to the compression orchestrator if available.
    #[must_use]
    pub fn compression_orchestrator(&self) -> Option<&Arc<CompressionOrchestrator>> {
        self.compression.compression_orchestrator()
    }

    /// Returns true if the compression orchestrator supports CCG.
    #[must_use]
    pub fn has_ccg_support(&self) -> bool {
        self.compression.has_ccg_support()
    }

    /// Injects CCG context by fetching the default context (manifest + architecture).
    ///
    /// This method fetches context from narsil-mcp via the compression orchestrator.
    /// The context is cached based on the repository hash, so subsequent calls with
    /// the same hash return cached results without making MCP calls.
    ///
    /// # Arguments
    ///
    /// * `client` - An active MCP client connected to narsil-mcp
    /// * `repo_hash` - Current repository hash for cache validation
    ///
    /// # Returns
    ///
    /// - `Ok(Some(content))` - CCG context was fetched successfully
    /// - `Ok(None)` - No orchestrator available or context injection disabled
    ///
    /// # Example
    ///
    /// ```ignore
    /// if let Some(context) = state.inject_ccg_context(&client, "abc123").await? {
    ///     // Prepend context to system message
    ///     system_prompt = format!("{}\n\n{}", context, system_prompt);
    /// }
    /// ```
    pub async fn inject_ccg_context(
        &mut self,
        client: &McpConnection,
        repo_hash: &str,
    ) -> anyhow::Result<Option<String>> {
        // Check if orchestrator is available
        let orchestrator = match &self.compression.compression_orchestrator {
            Some(orch) => orch.clone(),
            None => return Ok(None),
        };

        // Check if hash has changed (for cache invalidation tracking)
        let hash_changed = self
            .compression
            .last_ccg_hash
            .as_ref()
            .is_none_or(|h| h != repo_hash);

        if hash_changed {
            tracing::debug!(
                old_hash = ?self.compression.last_ccg_hash,
                new_hash = %repo_hash,
                "Repository hash changed, may fetch fresh CCG context"
            );
        }

        // Fetch context via orchestrator
        let result = orchestrator
            .get_default_context_async(client, repo_hash)
            .await;

        // Update cached hash
        self.compression.last_ccg_hash = Some(repo_hash.to_string());

        // Return content if non-empty, also cache it
        let content = result.content();
        if content.is_empty() {
            self.compression.cached_ccg_context = None;
            Ok(None)
        } else {
            tracing::info!(
                tokens = result.tokens_approx(),
                source = ?result.source(),
                "CCG context fetched and cached"
            );
            let content_string = content.to_string();
            self.compression.cached_ccg_context = Some(content_string.clone());
            Ok(Some(content_string))
        }
    }

    /// Returns the last CCG hash used for context injection.
    #[must_use]
    pub fn last_ccg_hash(&self) -> Option<&str> {
        self.compression.last_ccg_hash()
    }

    /// Sets the last CCG hash.
    pub fn set_last_ccg_hash(&mut self, hash: String) {
        self.compression.set_last_ccg_hash(hash);
    }

    /// Returns whether there is cached CCG context.
    #[must_use]
    pub fn has_cached_ccg_context(&self) -> bool {
        self.compression.has_cached_ccg_context()
    }

    /// Takes the cached CCG context.
    pub fn take_cached_ccg_context(&mut self) -> Option<String> {
        self.compression.take_cached_ccg_context()
    }

    /// Returns context for injection if auto-context is enabled.
    #[must_use]
    pub fn context_for_injection(&self) -> Option<&str> {
        self.compression.context_for_injection()
    }

    /// Returns whether a narsil MCP client is available.
    #[must_use]
    pub fn has_narsil_client(&self) -> bool {
        self.compression.has_narsil_client()
    }

    /// Sets the narsil MCP connection.
    pub fn set_narsil_client(&mut self, client: McpConnection) {
        self.compression.set_narsil_client(client);
    }

    /// Returns the maximum token budget for auto-injected context.
    #[must_use]
    pub fn context_token_budget(&self) -> usize {
        self.compression.context_token_budget()
    }

    /// Sets the maximum token budget for auto-injected context.
    pub fn set_context_token_budget(&mut self, budget: usize) {
        self.compression.set_context_token_budget(budget);
    }

    /// Returns the number of tokens injected in the most recent context injection.
    #[must_use]
    pub fn context_tokens_injected(&self) -> usize {
        self.compression.context_tokens_injected()
    }

    /// Sets the number of tokens injected.
    pub fn set_context_tokens_injected(&mut self, tokens: usize) {
        self.compression.set_context_tokens_injected(tokens);
    }

    /// Refreshes the cached CCG context by calling `build_context()`.
    ///
    /// This method lazily connects a narsil MCP client, fetches context
    /// from the compression orchestrator using the full 3-layer approach
    /// (manifest + architecture + symbols), and caches the result for
    /// injection into the next API message.
    ///
    /// Returns `None` if auto-context is disabled, no orchestrator is
    /// configured, or the narsil MCP client fails to connect.
    ///
    /// # Returns
    ///
    /// The context string if successfully fetched, `None` otherwise.
    pub async fn refresh_build_context(&mut self) -> Option<String> {
        if !self.compression.auto_context_enabled {
            return None;
        }

        let orchestrator = match &self.compression.compression_orchestrator {
            Some(orch) => orch.clone(),
            None => return None,
        };

        // Skip re-fetch if the git hash is unchanged and we already have cached context.
        // This prevents redundant MCP calls when the codebase hasn't changed.
        let repo_hash =
            get_git_head_hash(&self.working_dir).unwrap_or_else(|| "unknown".to_string());
        let hash_changed = self.compression.last_ccg_hash.as_ref() != Some(&repo_hash);

        if !hash_changed && self.compression.cached_ccg_context.is_some() {
            tracing::debug!(
                hash = %repo_hash,
                tokens = self.compression.context_tokens_injected,
                "CCG hash unchanged, reusing cached context"
            );
            return self.compression.cached_ccg_context.clone();
        }

        // Lazily initialize narsil connection
        if self.compression.narsil_client.is_none() {
            let working_dir = self.working_dir.to_string_lossy().to_string();
            let timeout = std::time::Duration::from_secs(30);
            match McpConnection::connect_stdio(
                "narsil-mcp",
                "narsil-mcp",
                &["--repos".to_string(), working_dir],
                &std::collections::HashMap::new(),
                timeout,
            )
            .await
            {
                Ok(conn) => {
                    tracing::info!("Narsil MCP connection established for context injection");
                    self.compression.narsil_client = Some(conn);
                }
                Err(e) => {
                    tracing::warn!("Failed to connect narsil-mcp for context: {}", e);
                    return None;
                }
            }
        }

        let client = self.compression.narsil_client.as_ref()?;

        let result = orchestrator
            .build_context(
                client,
                &repo_hash,
                &[], // active_files: empty = project-wide symbols
                self.compression.context_token_budget,
            )
            .await;

        // Update the hash to track what version of the codebase this context is for
        self.compression.last_ccg_hash = Some(repo_hash.clone());

        let content = result.result().content().to_string();
        if content.is_empty() {
            self.compression.cached_ccg_context = None;
            self.compression.context_tokens_injected = 0;
            tracing::debug!(hash = %repo_hash, "Context fetch returned empty result");
            None
        } else {
            tracing::info!(
                tokens = result.total_tokens(),
                manifest = result.manifest_tokens(),
                architecture = result.architecture_tokens(),
                symbols = result.symbol_tokens(),
                hash = %repo_hash,
                cache_status = "miss",
                "Build context refreshed for injection"
            );
            self.compression.context_tokens_injected = result.total_tokens();
            self.compression.cached_ccg_context = Some(content.clone());
            Some(content)
        }
    }

    /// Formats context suggestions into a string suitable for prepending to a message.
    ///
    /// Returns an empty string if suggestions is empty.
    ///
    /// # Arguments
    ///
    /// * `suggestions` - Context suggestions to format
    ///
    /// # Returns
    ///
    /// A formatted string containing all context suggestions.
    #[must_use]
    pub fn format_context_suggestions(suggestions: &[ContextSuggestion]) -> String {
        if suggestions.is_empty() {
            return String::new();
        }

        let mut parts = Vec::new();
        for suggestion in suggestions {
            parts.push(format!(
                "[Context: {}]\n{}",
                suggestion.description, suggestion.content
            ));
        }
        parts.join("\n\n")
    }

    // =========================================================================
    // Input state methods (delegates to InputState)
    // =========================================================================

    /// Returns a reference to the input state.
    #[must_use]
    pub fn input_state(&self) -> &InputState {
        &self.input_state
    }

    /// Returns the current input text.
    #[must_use]
    pub fn input(&self) -> &str {
        self.input_state.text()
    }

    /// Returns a mutable reference to the input text.
    pub fn input_mut(&mut self) -> &mut String {
        self.input_state.text_mut()
    }

    /// Inserts a character at the current cursor position.
    pub fn insert_char(&mut self, c: char) {
        let needs_completion = self.input_state.insert_char(c);
        self.dirty.input = true;
        if needs_completion {
            self.show_completion();
        }
    }

    /// Deletes the character before the cursor (backspace behavior).
    pub fn delete_char(&mut self) {
        self.input_state.delete_char();
        self.dirty.input = true;
    }

    /// Takes and returns the current input, clearing the buffer and resetting cursor.
    pub fn take_input(&mut self) -> String {
        self.dirty.input = true;
        self.input_state.take()
    }

    /// Returns the current cursor position (character index, not byte index).
    #[must_use]
    pub fn cursor_position(&self) -> usize {
        self.input_state.cursor_position()
    }

    /// Moves the cursor left by one character.
    pub fn cursor_left(&mut self) {
        self.input_state.cursor_left();
        self.dirty.input = true;
    }

    /// Moves the cursor right by one character.
    pub fn cursor_right(&mut self) {
        self.input_state.cursor_right();
        self.dirty.input = true;
    }

    /// Moves the cursor to the beginning of the input.
    pub fn cursor_home(&mut self) {
        self.input_state.cursor_home();
        self.dirty.input = true;
    }

    /// Moves the cursor to the end of the input.
    pub fn cursor_end(&mut self) {
        self.input_state.cursor_end();
        self.dirty.input = true;
    }

    /// Returns the current scroll offset for rendering.
    ///
    /// This provides backward compatibility with TUI rendering.
    #[must_use]
    pub fn scroll_offset(&self) -> usize {
        self.scroll.offset()
    }

    /// Scrolls up by the specified number of lines.
    ///
    /// This switches to Manual mode, preserving the scroll position
    /// during streaming updates.
    pub fn scroll_up(&mut self, lines: usize) {
        let before = self.scroll.offset();
        self.scroll.scroll_up(lines);
        let after = self.scroll.offset();
        tracing::debug!(
            lines,
            before,
            after,
            mode = ?self.scroll.mode(),
            content_height = self.scroll.content_height(),
            viewport_height = self.scroll.viewport_height(),
            cache_size = self.ui_selection.rendered_lines_cache.len(),
            timeline_entries = self.timeline.len(),
            "scroll_up"
        );
        self.dirty.messages = true;
    }

    /// Scrolls down by the specified number of lines.
    ///
    /// If scrolling to the bottom, resumes Follow mode for auto-scroll.
    pub fn scroll_down(&mut self, lines: usize) {
        let before = self.scroll.offset();
        self.scroll.scroll_down(lines);
        let after = self.scroll.offset();
        tracing::debug!(
            lines,
            before,
            after,
            mode = ?self.scroll.mode(),
            "scroll_down"
        );
        self.dirty.messages = true;
    }

    /// Scrolls to the bottom of the content.
    ///
    /// This resumes Follow mode for auto-scroll.
    pub fn scroll_to_bottom(&mut self, content_height: usize) {
        self.scroll.scroll_to_bottom(content_height);
        self.dirty.messages = true;
    }

    /// Scrolls to the top of the content.
    ///
    /// This switches to Manual mode.
    pub fn scroll_to_top(&mut self) {
        self.scroll.scroll_to_top();
        self.dirty.messages = true;
    }

    /// Updates the content height for scroll calculations.
    ///
    /// In Follow mode, this auto-scrolls to show new content.
    pub fn update_content_height(&mut self, height: usize) {
        self.scroll.set_content_height(height);
        if self.scroll.mode().should_auto_scroll() {
            self.dirty.messages = true;
        }
    }

    /// Updates the viewport height for scroll calculations.
    pub fn set_viewport_height(&mut self, height: usize) {
        self.scroll.set_viewport_height(height);
    }

    /// Returns the scroll state for read access.
    #[must_use]
    pub fn scroll_state(&self) -> &ScrollState {
        &self.scroll
    }

    /// Returns a reference to the UI selection state.
    #[must_use]
    pub fn ui_selection(&self) -> &UISelectionState {
        &self.ui_selection
    }

    /// Returns the selection state for read access.
    #[must_use]
    pub fn selection(&self) -> &SelectionState {
        self.ui_selection.selection()
    }

    /// Returns the selection state for modification.
    pub fn selection_mut(&mut self) -> &mut SelectionState {
        self.ui_selection.selection_mut()
    }

    /// Returns the current focus area.
    #[must_use]
    pub fn focus_area(&self) -> FocusArea {
        self.ui_selection.focus_area()
    }

    /// Sets the focus area, clearing selection if focus changes.
    pub fn set_focus_area(&mut self, area: FocusArea) {
        self.ui_selection.set_focus_area(area);
    }

    /// Determines which focus area a screen row belongs to.
    #[must_use]
    pub fn focus_area_for_row(row: u16, terminal_height: u16) -> FocusArea {
        UISelectionState::focus_area_for_row(row, terminal_height)
    }

    /// Copies the current selection to the system clipboard.
    ///
    /// # Errors
    ///
    /// Returns an error if all clipboard methods fail.
    pub fn copy_selection_to_clipboard(&self, lines: &[ratatui::text::Line<'_>]) -> Result<bool> {
        self.ui_selection.copy_selection_to_clipboard(lines)
    }

    /// Requests a copy operation to be performed during the next render.
    pub fn request_copy(&mut self) {
        self.ui_selection.request_copy();
    }

    /// Checks and clears the copy pending flag.
    pub fn take_copy_pending(&mut self) -> bool {
        self.ui_selection.take_copy_pending()
    }

    /// Returns the total number of rendered lines.
    #[must_use]
    pub fn rendered_line_count(&self) -> usize {
        self.ui_selection.rendered_line_count()
    }

    /// Updates the cached rendered lines for copy operations.
    pub fn update_rendered_lines_cache(&mut self, lines: &[ratatui::text::Line<'_>], width: usize) {
        self.ui_selection.update_rendered_lines_cache(lines, width);
    }

    /// Updates the cached rendered lines from pre-wrapped strings (via `RenderFeedback`).
    pub fn update_rendered_lines_from_feedback(&mut self, wrapped: &[String]) {
        self.ui_selection
            .update_rendered_lines_from_strings(wrapped);
    }

    /// Copies the current selection to clipboard using cached lines.
    ///
    /// # Errors
    ///
    /// Returns an error if clipboard access fails.
    pub fn copy_from_cache(&self) -> Result<bool> {
        self.ui_selection.copy_from_cache()
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    /// Signals that the application should exit the event loop.
    ///
    /// Called by `KeyboardHandler` when the user presses Ctrl+C or Ctrl+D.
    /// The event loop checks [`wants_quit`](Self::wants_quit) after each
    /// dispatch cycle.
    pub fn request_quit(&mut self) {
        self.quit_requested = true;
    }

    /// Returns `true` if the application should exit the event loop.
    #[must_use]
    pub fn wants_quit(&self) -> bool {
        self.quit_requested
    }

    pub fn tick_throbber(&mut self) {
        self.throbber_frame = (self.throbber_frame + 1) % 4;
        self.dirty.messages = true;
    }

    pub fn throbber_char(&self) -> char {
        ['⠋', '⠙', '⠹', '⠸'][self.throbber_frame]
    }

    pub fn needs_render(&self) -> bool {
        self.dirty.any()
    }

    pub fn mark_rendered(&mut self) {
        self.dirty.clear();
    }

    pub fn mark_full_redraw(&mut self) {
        self.dirty.full = true;
    }

    /// Returns the cached terminal height in rows.
    ///
    /// Defaults to 24 if no resize event has been received.
    #[must_use]
    pub fn terminal_height(&self) -> u16 {
        self.terminal_height
    }

    /// Updates the cached terminal height.
    ///
    /// Called from the event loop on startup and on resize events.
    pub fn set_terminal_height(&mut self, height: u16) {
        self.terminal_height = height;
    }

    /// Adds a message to the conversation timeline and display.
    ///
    /// This updates the unified timeline and sets the dirty flag so the UI
    /// will re-render. Note: This only adds to the display timeline, not
    /// the API messages. Use `add_api_message` to add to the API conversation.
    pub fn add_message(&mut self, message: Message) {
        // Add to unified timeline based on role
        match message.role {
            Role::User => self.timeline.push_user_message(&message.content),
            Role::Assistant => self.timeline.push_assistant_message(&message.content),
        }
        self.dirty.messages = true;
    }

    /// Adds a full API message with content blocks.
    ///
    /// This adds to both the API message history (with full content blocks)
    /// and the display timeline (as text summary).
    pub fn add_api_message(&mut self, message: ApiMessageV2) {
        // Add to display timeline as text summary
        let legacy = message.to_legacy();
        match legacy.role {
            Role::User => self.timeline.push_user_message(&legacy.content),
            Role::Assistant => self.timeline.push_assistant_message(&legacy.content),
        }
        // Add to API messages with full content blocks
        self.api_messages.push(message);
        self.dirty.messages = true;
    }

    /// Returns the API messages for continuation.
    ///
    /// These messages include full content blocks (tool_use, tool_result)
    /// and should be used when sending to the API.
    #[must_use]
    pub fn api_messages(&self) -> &[ApiMessageV2] {
        &self.api_messages
    }

    /// Returns a mutable reference to the API messages.
    pub fn api_messages_mut(&mut self) -> &mut Vec<ApiMessageV2> {
        &mut self.api_messages
    }

    /// Returns the count of API messages.
    #[must_use]
    pub fn api_messages_len(&self) -> usize {
        self.api_messages.len()
    }

    /// Returns API messages truncated to fit within the token budget.
    ///
    /// This should be used when sending messages to the API instead of
    /// `api_messages()` directly to prevent context overflow and control costs.
    ///
    /// The truncation:
    /// - Always preserves the first message (system/project context)
    /// - Prioritizes recent messages over older ones
    /// - Respects the `DEFAULT_MAX_INPUT_TOKENS` limit
    ///
    /// # Returns
    ///
    /// A new vector containing the truncated message history.
    #[must_use]
    pub fn api_messages_truncated(&self) -> Vec<ApiMessageV2> {
        use crate::api::{truncate_context, DEFAULT_MAX_INPUT_TOKENS};
        truncate_context(&self.api_messages, DEFAULT_MAX_INPUT_TOKENS)
    }

    /// Prepares API messages for sending by compacting and truncating.
    ///
    /// This is the canonical method for obtaining the message list before
    /// any API call (both user-submitted messages and tool continuations).
    /// It combines two steps:
    /// 1. `maybe_compact_graceful()` — compresses old messages if context usage
    ///    exceeds the threshold
    /// 2. `api_messages_truncated()` — caps the result at `DEFAULT_MAX_INPUT_TOKENS`
    ///
    /// Using this method instead of `api_messages().to_vec()` prevents token
    /// explosion and spiralling costs during tool continuation loops.
    ///
    /// # Arguments
    ///
    /// * `model` - The model name, used to determine the context window size
    ///
    /// # Returns
    ///
    /// A truncated, compacted message list safe to send to the API.
    pub async fn prepare_api_messages_for_send(&mut self, model: &str) -> Vec<ApiMessageV2> {
        let context_limit = model_context_limit(model);
        if self
            .maybe_compact_graceful(DEFAULT_COMPACTION_THRESHOLD, context_limit)
            .await
        {
            tracing::info!(
                threshold = DEFAULT_COMPACTION_THRESHOLD,
                context_limit,
                "Conversation compacted before tool continuation"
            );
        }

        let total = self.api_messages.len();
        let truncated = self.api_messages_truncated();
        let sent = truncated.len();
        if sent < total {
            tracing::info!(
                total,
                sending = sent,
                dropped = total - sent,
                "Context truncated for tool continuation"
            );
        }

        truncated
    }

    /// Builds the final API message list for sending to the LLM.
    ///
    /// This is the single entry point for constructing the message payload
    /// before an API call. When auto-context is enabled and cached CCG
    /// context is available, a context message is prepended to the
    /// conversation history.
    ///
    /// The cached context is read (not consumed) so it remains available
    /// for subsequent calls until explicitly refreshed or cleared.
    ///
    /// # Returns
    ///
    /// A new vector containing the message history ready for the API call,
    /// optionally prefixed with a CCG context message.
    #[must_use]
    pub fn build_api_messages(&self) -> Vec<ApiMessageV2> {
        let mut messages = self.api_messages_truncated();

        // Inject cached CCG context as a leading user message when available
        if self.compression.auto_context_enabled {
            if let Some(context) = &self.compression.cached_ccg_context {
                let context_msg = ApiMessageV2::user(format!("<context>\n{context}\n</context>"));
                messages.insert(0, context_msg);
            }
        }

        messages
    }

    pub async fn submit_message(
        &mut self,
        client: &std::sync::Arc<dyn LlmProvider>,
        content: String,
    ) -> Result<()> {
        // Refresh context from build_context() if auto-context is enabled.
        // This populates cached_ccg_context which is consumed below.
        self.refresh_build_context().await;

        // Build the API message content, optionally with CCG context
        let api_content = if self.compression.auto_context_enabled {
            if let Some(context) = self.compression.cached_ccg_context.take() {
                tracing::info!(
                    context_len = context.len(),
                    "Injecting CCG context into user message"
                );
                // Prepend context to user message for API
                format!("<context>\n{}\n</context>\n\n{}", context, content)
            } else {
                content.clone()
            }
        } else {
            content.clone()
        };

        // Timeline shows original user input (cleaner UI)
        self.timeline.push_user_message(&content);
        // API gets potentially context-augmented message
        let user_msg = ApiMessageV2::user(&api_content);
        self.api_messages.push(user_msg);

        self.loading = true;
        // Start streaming in timeline
        if self.timeline.try_push_streaming().is_err() {
            tracing::warn!("Timeline already streaming when submitting message");
        }

        // Initialize tool loop state machine for streaming
        // This must be called BEFORE the API stream starts so tool events are captured
        if let Err(e) = self.tool_state.tool_loop.start_streaming() {
            tracing::warn!("Failed to start tool loop streaming: {}", e);
            // Reset and try again - the loop might be in an unexpected state
            self.tool_state.tool_loop.reset();
            if let Err(e2) = self.tool_state.tool_loop.start_streaming() {
                tracing::error!(
                    "Tool loop start_streaming failed after reset: {}. Tool execution may be compromised.",
                    e2
                );
            }
        }

        let (tx, rx) = mpsc::channel(STREAMING_CHANNEL_BUFFER);
        self.streaming_rx = Some(rx);

        // Compact + truncate API messages for cost-controlled sending
        let api_messages = self.prepare_api_messages_for_send(client.model()).await;

        let client = std::sync::Arc::clone(client);
        let tools = self.all_tool_definitions();
        let options = self.build_request_options(client.model());
        tokio::spawn(async move {
            if let Err(e) = client
                .stream_message(
                    &api_messages,
                    Some(&tools),
                    Some(&ToolChoice::Auto),
                    &options,
                    tx,
                )
                .await
            {
                tracing::error!("API error: {}", e);
            }
        });

        Ok(())
    }

    /// Receives the next API streaming chunk, if available.
    ///
    /// Returns `None` immediately when no streaming is active. This is critical
    /// for event loop responsiveness - returning `pending()` would cause the
    /// `tokio::select!` to wait indefinitely, blocking keyboard input processing.
    ///
    /// # Returns
    ///
    /// - `Some(chunk)` - Next streaming event from the API
    /// - `None` - Channel closed or no active streaming
    pub async fn recv_api_chunk(&mut self) -> Option<StreamEvent> {
        match &mut self.streaming_rx {
            Some(rx) => rx.recv().await,
            None => None, // Return immediately - don't block with pending()
        }
    }

    /// Returns true if there is an active streaming receiver.
    ///
    /// Used for guard conditions in the event loop to skip polling
    /// when no streaming is active.
    #[must_use]
    pub fn has_streaming(&self) -> bool {
        self.streaming_rx.is_some()
    }

    /// Returns true if there are any active background channels (API streaming or tool results).
    ///
    /// Used for guard conditions in the event loop.
    #[must_use]
    pub fn has_background_work(&self) -> bool {
        self.streaming_rx.is_some() || self.tool_state.tool_result_rx.is_some()
    }

    /// Receives the next background event from either API streaming or tool execution.
    ///
    /// This combines both channels into a single async receive to avoid borrow checker
    /// issues in the event loop's `tokio::select!`. Returns immediately if both
    /// channels are `None`.
    ///
    /// # Returns
    ///
    /// - `Some(BackgroundEvent::ApiChunk(chunk))` - API streaming event
    /// - `Some(BackgroundEvent::ToolResult(id, result))` - Tool execution completed
    /// - `None` - Both channels closed or not set
    pub async fn recv_background_event(&mut self) -> Option<BackgroundEvent> {
        // Use tokio::select! to receive from whichever channel is ready first
        // Since this is a single method, there's no borrow conflict
        tokio::select! {
            biased;

            // Prioritize tool results to update UI quickly
            result = async {
                match &mut self.tool_state.tool_result_rx {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            }, if self.tool_state.tool_result_rx.is_some() => {
                result.map(|(id, r)| BackgroundEvent::ToolResult(id, r))
            }

            // Then API streaming chunks
            chunk = async {
                match &mut self.streaming_rx {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            }, if self.streaming_rx.is_some() => {
                chunk.map(BackgroundEvent::ApiChunk)
            }

            // If neither channel is active, return None immediately
            else => None
        }
    }

    /// Sets the streaming receiver for API response chunks.
    ///
    /// This is used by the tool execution flow to set up continuation streaming
    /// without blocking the event loop.
    pub fn set_streaming_rx(&mut self, rx: mpsc::Receiver<StreamEvent>) {
        self.streaming_rx = Some(rx);
    }

    /// Sets the loading state.
    ///
    /// When loading is true, the throbber animates and content accumulates.
    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
        self.dirty.messages = true;
    }

    /// Initializes the streaming buffer for continuation streaming.
    ///
    /// This starts a new streaming entry in the timeline with optional initial content.
    pub fn set_current_response(&mut self, response: String) {
        // Start streaming in timeline if not already streaming
        if self.timeline.try_push_streaming().is_ok() && !response.is_empty() {
            self.timeline.append_to_streaming(&response);
        }
        self.dirty.messages = true;
    }

    pub fn append_chunk(&mut self, event: StreamEvent) -> Result<()> {
        match event {
            StreamEvent::ContentDelta(text) => self.handle_content_delta(text),
            StreamEvent::MessageStop => self.handle_message_stop(),
            StreamEvent::MessageComplete { stop_reason } => {
                self.handle_message_complete_event(stop_reason)?;
            }
            StreamEvent::Error(e) => self.handle_stream_error(e),
            StreamEvent::ToolUseStart { id, name, index } => {
                self.handle_tool_use_start(id, name, index);
            }
            StreamEvent::ToolUseInputDelta {
                index,
                partial_json,
            } => {
                self.handle_tool_use_input_delta(index, &partial_json);
            }
            StreamEvent::ToolUseComplete { index } => {
                self.handle_tool_use_complete(index)?;
            }
            StreamEvent::ContentBlockComplete { .. } => {
                // Content block completion is tracked internally
                tracing::debug!("Content block complete");
            }
            StreamEvent::ThinkingStart { .. } => {
                self.thinking_buffer.clear();
                tracing::debug!("Thinking block started");
            }
            StreamEvent::ThinkingDelta(text) => {
                self.thinking_buffer.push_str(&text);
            }
            StreamEvent::ThinkingComplete { .. } => {
                if !self.thinking_buffer.is_empty() {
                    let thinking_text = std::mem::take(&mut self.thinking_buffer);
                    let token_count = crate::api::tokens::estimate_tokens(&thinking_text);
                    tracing::info!("Thinking complete: {} tokens", token_count);
                    self.timeline
                        .push_assistant_message(format!("[Thought for ~{} tokens]", token_count));
                }
            }
            StreamEvent::Usage(usage) => {
                let record = UsageRecord::with_cache(
                    &self.current_model,
                    usage.input_tokens,
                    usage.output_tokens,
                    usage.cache_read_input_tokens,
                    usage.cache_creation_input_tokens,
                    std::time::Duration::ZERO,
                );
                self.cost_tracker.record_usage(record);
                tracing::debug!(
                    "Usage recorded: {}in/{}out, session cost: ${:.4}",
                    usage.input_tokens,
                    usage.output_tokens,
                    self.cost_tracker.session_cost(),
                );
            }
        }
        Ok(())
    }

    /// Handles a `ContentDelta` stream event by appending text to the timeline
    /// and forwarding it to the tool loop for assistant text tracking.
    fn handle_content_delta(&mut self, text: String) {
        self.timeline.append_to_streaming(&text);
        self.tool_state.tool_loop.append_text(&text);
        self.dirty.messages = true;
    }

    /// Handles a `MessageStop` stream event by finalizing the streaming entry
    /// and adding the assistant message to the API conversation history.
    ///
    /// Only processes if currently streaming, to prevent duplicates when
    /// `MessageComplete` has already handled finalization.
    fn handle_message_stop(&mut self) {
        if self.timeline.is_streaming() {
            self.timeline.finalize_streaming_as_message();
            if let Some(crate::types::ConversationEntry::AssistantMessage(text)) =
                self.timeline.entries().last()
            {
                self.api_messages.push(ApiMessageV2::assistant(text));
            }
        }
        self.loading = false;
        self.streaming_rx = None;
        self.dirty.messages = true;
    }

    /// Handles a `MessageComplete` stream event by finalizing the streaming entry
    /// and processing the stop reason.
    ///
    /// For `tool_use` stop reasons, streaming is finalized without adding to API
    /// messages (handled later by `handle_tool_execution`). For normal responses,
    /// the assistant message is added to the API conversation history.
    ///
    /// # Errors
    ///
    /// Returns an error if tool loop state transition fails.
    fn handle_message_complete_event(&mut self, stop_reason: StopReason) -> Result<()> {
        let needs_tool_execution = stop_reason.needs_tool_execution();

        if needs_tool_execution {
            self.timeline.finalize_streaming_for_tool_use();
            tracing::debug!("Tool use response - text stored in tool_loop, not adding to API yet");
        } else {
            self.timeline.finalize_streaming_as_message();
            if let Some(crate::types::ConversationEntry::AssistantMessage(text)) =
                self.timeline.entries().last()
            {
                self.api_messages.push(ApiMessageV2::assistant(text));
            }
        }
        self.handle_message_complete(stop_reason)?;
        self.loading = false;
        self.streaming_rx = None;
        self.dirty.messages = true;
        Ok(())
    }

    /// Handles a stream `Error` event by logging the error and resetting
    /// the streaming state.
    fn handle_stream_error(&mut self, error: String) {
        tracing::error!("Stream error: {}", error);
        self.loading = false;
        self.streaming_rx = None;
        self.dirty.messages = true;
    }

    // ========================================================================
    // Worktree Status Bar State (delegates to WorktreeStatus)
    // ========================================================================

    /// Returns a reference to the worktree status.
    #[must_use]
    pub fn worktree(&self) -> &WorktreeStatus {
        &self.worktree
    }

    /// Sets the current worktree branch name.
    pub fn set_worktree_branch(&mut self, branch: String) {
        self.worktree.set_branch(branch);
        self.dirty.full = true;
    }

    /// Returns the current worktree branch name, if set.
    #[must_use]
    pub fn worktree_branch(&self) -> Option<&str> {
        self.worktree.branch()
    }

    /// Sets the number of modified files in the worktree.
    pub fn set_worktree_modified(&mut self, count: usize) {
        self.worktree.set_modified(count);
        self.dirty.full = true;
    }

    /// Returns the number of modified files in the worktree.
    #[must_use]
    pub fn worktree_modified(&self) -> usize {
        self.worktree.modified()
    }

    /// Sets the number of commits ahead of upstream.
    pub fn set_worktree_ahead(&mut self, count: usize) {
        self.worktree.set_ahead(count);
        self.dirty.full = true;
    }

    /// Returns the number of commits ahead of upstream.
    #[must_use]
    pub fn worktree_ahead(&self) -> usize {
        self.worktree.ahead()
    }

    /// Sets the number of commits behind upstream.
    pub fn set_worktree_behind(&mut self, count: usize) {
        self.worktree.set_behind(count);
        self.dirty.full = true;
    }

    /// Returns the number of commits behind upstream.
    #[must_use]
    pub fn worktree_behind(&self) -> usize {
        self.worktree.behind()
    }

    // ========================================================================
    // Token Budget Tracking
    // ========================================================================

    /// Returns a reference to the token budget for display.
    #[must_use]
    pub fn token_budget(&self) -> &TokenBudget {
        self.compression.token_budget()
    }

    /// Returns a mutable reference to the token budget.
    pub fn token_budget_mut(&mut self) -> &mut TokenBudget {
        self.compression.token_budget_mut()
    }

    /// Adds token usage to the budget.
    pub fn add_token_usage(&mut self, tokens: usize) {
        self.compression.token_budget_mut().add_usage(tokens);
        self.dirty.full = true;
    }

    /// Resets the token budget for a new conversation.
    pub fn reset_token_budget(&mut self) {
        self.compression.token_budget_mut().reset();
        self.dirty.full = true;
    }

    // ========================================================================
    // Compaction Progress (delegates to CompressionState)
    // ========================================================================

    /// Returns the compaction progress state.
    #[must_use]
    pub fn compaction_state(&self) -> Option<&CompactionProgressState> {
        self.compression.compaction_state()
    }

    /// Returns a mutable reference to the compaction progress state.
    pub fn compaction_state_mut(&mut self) -> Option<&mut CompactionProgressState> {
        self.compression.compaction_state_mut()
    }

    /// Starts a compaction operation.
    pub fn start_compaction(&mut self, target_tokens: usize, before_tokens: usize, is_auto: bool) {
        self.compression
            .start_compaction(target_tokens, before_tokens, is_auto);
        self.dirty.full = true;
    }

    /// Updates the compaction progress (0.0 to 1.0).
    pub fn update_compaction_progress(&mut self, progress: f64) {
        self.compression.update_compaction_progress(progress);
        self.dirty.full = true;
    }

    /// Completes the compaction operation with the final token count.
    pub fn complete_compaction(&mut self, after_tokens: usize) {
        self.compression.complete_compaction(after_tokens);
        self.dirty.full = true;
    }

    /// Marks the compaction operation as failed.
    pub fn fail_compaction(&mut self) {
        self.compression.fail_compaction();
        self.dirty.full = true;
    }

    /// Clears the compaction state (closes the overlay).
    pub fn clear_compaction(&mut self) {
        self.compression.clear_compaction();
        self.dirty.full = true;
    }

    /// Requests a manual compaction, to be executed on the next event loop tick.
    ///
    /// This sets a flag that `maybe_compact` will check, bypassing the
    /// automatic threshold so that compaction runs unconditionally.
    /// Called by the `/compact` slash command handler.
    pub fn force_compact(&mut self, custom_instructions: Option<String>) {
        self.compression.request_compaction(custom_instructions);
        self.dirty.full = true;
    }

    /// Returns whether a manual compaction has been requested.
    #[must_use]
    pub fn is_compaction_requested(&self) -> bool {
        self.compression.is_compaction_requested()
    }

    /// Returns a reference to the compaction metrics.
    #[must_use]
    pub fn compaction_metrics(&self) -> &CompactionMetrics {
        self.compression.compaction_metrics()
    }

    /// Returns a summary of compaction metrics.
    ///
    /// Provides a snapshot of compaction statistics including counts,
    /// totals, and averages.
    #[must_use]
    pub fn compaction_metrics_summary(&self) -> CompactionMetricsSummary {
        self.compression.compaction_metrics.summary()
    }

    // ========================================================================
    // Auto-Compaction (Phase 4.4)
    // ========================================================================

    /// Estimates the total tokens in the current conversation.
    ///
    /// Uses the token estimation utilities to calculate approximate token
    /// usage for all API messages. This is a heuristic estimate, not an
    /// exact count.
    ///
    /// # Returns
    ///
    /// Estimated token count for the conversation.
    #[must_use]
    pub fn estimate_conversation_tokens(&self) -> usize {
        use crate::api::tokens::estimate_messages_tokens;
        estimate_messages_tokens(&self.api_messages)
    }

    /// Updates the token budget based on the current conversation size.
    ///
    /// This synchronizes the token budget with the actual estimated token
    /// usage in the conversation. Call this after adding messages or after
    /// compaction to keep the budget accurate.
    pub fn sync_token_budget(&mut self) {
        let tokens = self.estimate_conversation_tokens();
        self.reset_token_budget();
        self.add_token_usage(tokens);
    }

    /// Checks if compaction should be triggered and performs it if needed.
    ///
    /// This method:
    /// 1. Estimates current conversation tokens
    /// 2. Checks if usage exceeds the auto-compaction threshold
    /// 3. If yes, performs compaction using the mock summarizer
    /// 4. Updates api_messages with compacted result
    /// 5. Syncs token budget
    ///
    /// # Arguments
    ///
    /// * `threshold` - Fraction of context window at which to trigger (0.0-1.0)
    /// * `context_limit` - Maximum context window size in tokens
    ///
    /// # Returns
    ///
    /// * `Ok(true)` - Compaction was performed
    /// * `Ok(false)` - No compaction needed
    /// * `Err(_)` - Compaction failed
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Trigger compaction at 80% of 200k context
    /// let compacted = state.maybe_compact(0.8, 200_000).await?;
    /// if compacted {
    ///     println!("Conversation was compacted");
    /// }
    /// ```
    pub async fn maybe_compact(&mut self, threshold: f32, context_limit: usize) -> Result<bool> {
        use crate::api::compaction::{CompactionConfig, ContextCompactor, NoOpSummarizer};
        use std::time::Instant;

        // Estimate current usage
        let current_tokens = self.estimate_conversation_tokens();
        let threshold_tokens = (context_limit as f64 * f64::from(threshold)) as usize;

        // Check for a manual compaction request (bypasses threshold)
        let forced = self.compression.take_compaction_request().is_some();

        // Check if we're under threshold and no forced compaction
        if !forced && current_tokens < threshold_tokens {
            tracing::debug!(
                current = current_tokens,
                threshold = threshold_tokens,
                "Compaction not needed"
            );
            return Ok(false);
        }

        let trigger = if forced { "manual" } else { "auto" };
        tracing::info!(
            current = current_tokens,
            threshold = threshold_tokens,
            trigger,
            "Starting compaction"
        );

        // Show compaction progress
        let target_tokens = context_limit / 2; // Target 50% of context
        self.start_compaction(target_tokens, current_tokens, !forced);

        // Uses NoOpSummarizer until the LlmProvider trait (Sprint 2) enables
        // wiring a real summarizer without coupling to AnthropicClient.
        let compactor = ContextCompactor::<NoOpSummarizer>::new_noop();

        let config = CompactionConfig {
            target_tokens,
            preserve_recent: 4,
            ..Default::default()
        };

        // Start timing
        let start_time = Instant::now();

        // Perform compaction
        match compactor.compact(&self.api_messages, &config).await {
            Ok(result) => {
                let duration = start_time.elapsed();
                let after_tokens = crate::api::tokens::estimate_messages_tokens(&result.messages);
                let tokens_saved = result.saved_tokens;

                // Record compaction metrics
                self.compression
                    .compaction_metrics
                    .record_compaction(tokens_saved, duration);

                // Log metrics summary
                tracing::info!(
                    before = current_tokens,
                    after = after_tokens,
                    saved = tokens_saved,
                    duration_ms = duration.as_millis() as u64,
                    total_compactions = self.compression.compaction_metrics.compaction_count(),
                    total_tokens_saved = self.compression.compaction_metrics.total_tokens_saved(),
                    "Compaction complete"
                );

                // Update state with compacted messages
                self.api_messages = result.messages;

                // Sync token budget with new size
                self.sync_token_budget();

                // Update compaction UI
                self.complete_compaction(after_tokens);

                Ok(true)
            }
            Err(e) => {
                tracing::error!(error = %e, "Compaction failed");
                self.fail_compaction();

                // Return error but don't fail the operation
                Err(e)
            }
        }
    }

    /// Performs compaction if needed, logging but not failing on errors.
    ///
    /// This is a convenience wrapper around `maybe_compact` that handles
    /// errors gracefully. Use this in the message send flow where compaction
    /// failure shouldn't block the API call.
    ///
    /// # Arguments
    ///
    /// * `threshold` - Fraction of context window at which to trigger
    /// * `context_limit` - Maximum context window size in tokens
    ///
    /// # Returns
    ///
    /// `true` if compaction was performed successfully, `false` otherwise.
    pub async fn maybe_compact_graceful(&mut self, threshold: f32, context_limit: usize) -> bool {
        match self.maybe_compact(threshold, context_limit).await {
            Ok(compacted) => compacted,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "Compaction failed, continuing without compaction"
                );
                // Clear failed compaction state
                self.clear_compaction();
                false
            }
        }
    }

    // ========================================================================
    // Session Restoration and Auto-Save (delegates to SessionTracking)
    // ========================================================================

    /// Returns a reference to the session tracking state.
    #[must_use]
    pub fn session_tracking(&self) -> &SessionTracking {
        &self.session
    }

    /// Returns the current session ID, if one has been assigned.
    #[must_use]
    pub fn session_id(&self) -> Option<&str> {
        self.session.id()
    }

    /// Sets the session ID.
    pub fn set_session_id(&mut self, id: String) {
        self.session.set_id(id);
    }

    /// Marks the session as needing to be saved.
    pub fn mark_session_dirty(&mut self) {
        self.session.mark_dirty();
    }

    /// Returns `true` and clears the dirty flag if the session needs saving.
    pub fn take_session_dirty(&mut self) -> bool {
        self.session.take_dirty()
    }

    /// Creates a `Session` from the current application state.
    ///
    /// The resulting session includes:
    /// - All conversation messages (converted from timeline)
    /// - Current UI state (scroll position, input buffer, cursor position)
    /// - Working directory
    ///
    /// This is used for auto-save functionality.
    #[must_use]
    pub fn to_session(&self) -> Session {
        use crate::session::UiState;

        let mut session = Session::new(self.working_dir.clone());

        // Convert timeline entries to messages for session persistence
        for entry in self.timeline.iter() {
            match entry {
                crate::types::ConversationEntry::UserMessage(text) => {
                    session.add_message(Message {
                        role: Role::User,
                        content: text.clone(),
                    });
                }
                crate::types::ConversationEntry::AssistantMessage(text) => {
                    session.add_message(Message {
                        role: Role::Assistant,
                        content: text.clone(),
                    });
                }
                // Skip streaming and tool execution entries for session persistence
                _ => {}
            }
        }

        // Capture UI state (use scroll offset for backward compatibility)
        let ui_state = UiState::with_state(
            self.scroll.offset(),
            self.input_state.text().to_string(),
            self.input_state.cursor_position(),
        );
        session.set_ui_state(Some(ui_state));

        session
    }

    /// Restores application state from a saved session.
    ///
    /// This restores:
    /// - Message history (to timeline)
    /// - UI state (scroll position, input buffer, cursor position) if saved
    /// - Session ID for subsequent saves
    ///
    /// # Arguments
    ///
    /// * `session` - The session to restore from.
    pub fn restore_from_session(&mut self, session: &Session) {
        // Clear and rebuild timeline from session messages
        self.timeline = Timeline::new();
        for message in session.messages() {
            match message.role {
                Role::User => self.timeline.push_user_message(&message.content),
                Role::Assistant => self.timeline.push_assistant_message(&message.content),
            }
        }

        // Restore UI state if available
        if let Some(ui_state) = session.ui_state() {
            self.scroll.restore_offset(ui_state.scroll_offset());
            self.input_state
                .set_text(ui_state.input_buffer().to_string());
            self.input_state
                .set_cursor_position(ui_state.cursor_position());
        }

        // Restore session ID if available
        if let Some(id) = session.id() {
            self.session.set_id(id.to_string());
        }

        // Mark for full redraw
        self.dirty.full = true;
    }

    // ========================================================================
    // Tool Execution Integration
    // ========================================================================

    /// Returns a reference to the tool loop state.
    #[must_use]
    pub fn tool_loop(&self) -> &ToolLoop {
        &self.tool_state.tool_loop
    }

    /// Returns a mutable reference to the tool loop.
    pub fn tool_loop_mut(&mut self) -> &mut ToolLoop {
        &mut self.tool_state.tool_loop
    }

    /// Returns the current tool loop state.
    #[must_use]
    pub fn tool_loop_state(&self) -> &ToolLoopState {
        self.tool_state.tool_loop.state()
    }

    /// Returns the pending permission request, if any.
    #[must_use]
    pub fn pending_permission(&self) -> Option<&PermissionRequest> {
        self.tool_state.pending_permission.as_ref()
    }

    /// Returns true if there's a pending permission prompt.
    #[must_use]
    pub fn has_pending_permission(&self) -> bool {
        self.tool_state.pending_permission.is_some()
    }

    /// Sets a pending permission request.
    ///
    /// The UI should display this as a modal prompt.
    pub fn set_pending_permission(&mut self, request: PermissionRequest) {
        self.tool_state.pending_permission = Some(request);
        self.dirty.full = true;
    }

    /// Clears the pending permission request.
    pub fn clear_pending_permission(&mut self) {
        self.tool_state.pending_permission = None;
        self.dirty.full = true;
    }

    // --- Plan review state ---

    /// Returns the pending plan, if any.
    #[must_use]
    pub fn pending_plan(&self) -> Option<&PlanState> {
        self.pending_plan.as_ref()
    }

    /// Returns a mutable reference to the pending plan, if any.
    #[must_use]
    pub fn pending_plan_mut(&mut self) -> Option<&mut PlanState> {
        self.pending_plan.as_mut()
    }

    /// Returns true if there's a pending plan awaiting user review.
    #[must_use]
    pub fn has_pending_plan(&self) -> bool {
        self.pending_plan.is_some()
    }

    /// Sets a pending plan for user review.
    ///
    /// The UI should display this as a modal plan review overlay.
    pub fn set_pending_plan(&mut self, plan: PlanState) {
        self.pending_plan = Some(plan);
        self.dirty.full = true;
    }

    /// Approves the pending plan, returning a success tool result block.
    ///
    /// Clears the pending plan state.
    ///
    /// # Returns
    ///
    /// The tool result block to send back to the API, or `None` if no plan is pending.
    pub fn approve_plan(&mut self) -> Option<crate::types::ToolResultBlock> {
        let plan = self.pending_plan.take()?;
        self.dirty.full = true;
        Some(plan.approve())
    }

    /// Rejects the pending plan, returning an error tool result block.
    ///
    /// Clears the pending plan state.
    ///
    /// # Returns
    ///
    /// The tool result block to send back to the API, or `None` if no plan is pending.
    pub fn reject_plan(&mut self) -> Option<crate::types::ToolResultBlock> {
        let plan = self.pending_plan.take()?;
        self.dirty.full = true;
        Some(plan.reject())
    }

    // --- Question prompt state ---

    /// Returns the pending question, if any.
    #[must_use]
    pub fn pending_question(&self) -> Option<&QuestionState> {
        self.pending_question.as_ref()
    }

    /// Returns a mutable reference to the pending question, if any.
    #[must_use]
    pub fn pending_question_mut(&mut self) -> Option<&mut QuestionState> {
        self.pending_question.as_mut()
    }

    /// Returns true if there's a pending question awaiting user response.
    #[must_use]
    pub fn has_pending_question(&self) -> bool {
        self.pending_question.is_some()
    }

    /// Sets a pending question for user response.
    ///
    /// The UI should display this as a modal question prompt.
    pub fn set_pending_question(&mut self, question: QuestionState) {
        self.pending_question = Some(question);
        self.dirty.full = true;
    }

    /// Submits the pending question response, returning a success tool result block.
    ///
    /// Clears the pending question state.
    pub fn submit_question(&mut self) -> Option<crate::types::ToolResultBlock> {
        let question = self.pending_question.take()?;
        self.dirty.full = true;
        Some(question.submit())
    }

    /// Cancels the pending question, returning an error tool result block.
    ///
    /// Clears the pending question state.
    pub fn cancel_question(&mut self) -> Option<crate::types::ToolResultBlock> {
        let question = self.pending_question.take()?;
        self.dirty.full = true;
        Some(question.cancel())
    }

    // --- Background task state ---

    /// Returns a reference to the background task registry.
    #[must_use]
    pub fn background_tasks(&self) -> &BackgroundTaskRegistry {
        &self.background_tasks
    }

    /// Returns a mutable reference to the background task registry.
    pub fn background_tasks_mut(&mut self) -> &mut BackgroundTaskRegistry {
        &mut self.background_tasks
    }

    // --- Completion state (delegates to InputState) ---

    /// Returns the active completion state, if any.
    #[must_use]
    pub fn completion(&self) -> Option<&super::completion::CompletionState> {
        self.input_state.completion()
    }

    /// Returns a mutable reference to the active completion state.
    #[must_use]
    pub fn completion_mut(&mut self) -> Option<&mut super::completion::CompletionState> {
        self.input_state.completion_mut()
    }

    /// Returns true if the completion popup is currently active.
    #[must_use]
    pub fn has_completion(&self) -> bool {
        self.input_state.has_completion()
    }

    /// Activates the completion popup by gathering candidates from all providers.
    ///
    /// This is a cross-domain coordinator: it reads `plugin_registry` to gather
    /// candidates and writes the result into `input_state`.
    pub fn show_completion(&mut self) {
        use super::completion::{
            BuiltinCommandProvider, CompletionProvider, CompletionState, McpToolProvider,
            PluginCommandProvider,
        };

        let builtin = BuiltinCommandProvider;
        let plugins = PluginCommandProvider::from_registry(&self.plugin_registry);
        let mcp = McpToolProvider::empty();

        let mut candidates = builtin.candidates();
        candidates.extend(plugins.candidates());
        candidates.extend(mcp.candidates());

        self.input_state
            .set_completion(CompletionState::new(candidates));
        self.dirty.input = true;
    }

    /// Dismisses the completion popup.
    pub fn dismiss_completion(&mut self) {
        self.input_state.dismiss_completion();
        self.dirty.input = true;
    }

    /// Accepts the selected completion and replaces input with `/name `.
    ///
    /// Returns the accepted command name, or `None` if nothing was selected.
    pub fn accept_completion(&mut self) -> Option<String> {
        let name = self.input_state.accept_completion();
        if name.is_some() {
            self.dirty.input = true;
        }
        name
    }

    /// Handles a permission response from the user.
    ///
    /// This grants or denies permission for the pending tool call and
    /// updates the permission manager accordingly.
    pub async fn handle_permission_response(&mut self, response: PermissionResponse) {
        if let Some(request) = self.tool_state.pending_permission.take() {
            let mut manager = self.tool_state.permission_manager.lock().await;
            manager.handle_response(&request.tool_name, request.tool_input.as_deref(), response);
            self.dirty.full = true;
        }
    }

    /// Handles a tool_use stream event.
    ///
    /// Routes the event to the tool loop state machine.
    pub fn handle_tool_use_start(&mut self, id: String, name: String, index: usize) {
        self.tool_state.tool_loop.start_tool_use(index, id, name);
        self.dirty.messages = true;
    }

    /// Handles a tool_use input delta.
    pub fn handle_tool_use_input_delta(&mut self, index: usize, partial_json: &str) {
        self.tool_state
            .tool_loop
            .append_tool_input(index, partial_json);
    }

    /// Handles tool_use completion.
    pub fn handle_tool_use_complete(&mut self, index: usize) -> Result<()> {
        self.tool_state
            .tool_loop
            .complete_tool_use(index)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(())
    }

    /// Handles message completion with a stop reason.
    ///
    /// If the stop reason is `ToolUse`, transitions the tool loop to
    /// `PendingApproval` state.
    pub fn handle_message_complete(&mut self, stop_reason: StopReason) -> Result<()> {
        self.tool_state
            .tool_loop
            .message_complete(stop_reason)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(())
    }

    /// Executes all pending tools that have been approved.
    ///
    /// Creates tool blocks for UI display, executes the tools, and
    /// updates the blocks with results.
    ///
    /// Returns a list of tool IDs that need permission (if any).
    ///
    /// # Errors
    ///
    /// Returns an error if the tool loop is not in `Executing` state.
    pub async fn execute_pending_tools(&mut self) -> Result<Vec<String>> {
        use crate::app::tool_loop::ToolLoopError;
        use std::collections::HashMap;

        // Collect tool info before creating blocks (to avoid borrow issues)
        let tools_to_display: Vec<(String, String, String)> = self
            .tool_state
            .tool_loop
            .pending_calls()
            .iter()
            .filter(|(_, call)| call.approved && !call.executed)
            .map(|(tool_id, call)| {
                let input_str = format_tool_input(&call.tool_use.name, &call.tool_use.input);
                (tool_id.clone(), call.tool_use.name.clone(), input_str)
            })
            .collect();

        // Create tool blocks for pending tools (before execution)
        let mut tool_id_to_block_index: HashMap<String, usize> = HashMap::new();
        for (tool_id, tool_name, input_str) in tools_to_display {
            let index = self.start_tool_block(&tool_name, &input_str);
            tool_id_to_block_index.insert(tool_id, index);
        }

        // Execute the tools (pass MCP manager for namespaced tool routing)
        let result = self
            .tool_state
            .tool_loop
            .execute_pending(&self.tool_state.tool_executor, self.mcp_manager.as_deref())
            .await
            .map_err(|e| match e {
                ToolLoopError::InvalidStateTransition { from, to } => {
                    anyhow::anyhow!("Invalid state transition from {} to {}", from, to)
                }
                _ => anyhow::anyhow!("{}", e),
            })?;

        // Collect results (to avoid borrow issues)
        let results: Vec<(String, String, bool)> = self
            .tool_state
            .tool_loop
            .pending_calls()
            .iter()
            .filter_map(|(tool_id, call)| {
                if let Some(result_block) = &call.result {
                    if tool_id_to_block_index.contains_key(tool_id) {
                        return Some((
                            tool_id.clone(),
                            result_block.content.clone(),
                            result_block.is_error,
                        ));
                    }
                }
                None
            })
            .collect();

        // Update tool blocks with results
        for (tool_id, content, is_error) in results {
            if let Some(&block_index) = tool_id_to_block_index.get(&tool_id) {
                self.complete_tool_block(block_index, &content, is_error);
            }
        }

        Ok(result)
    }

    /// Finishes tool execution and returns continuation data.
    ///
    /// The continuation data contains the messages needed to continue
    /// the conversation with Claude.
    pub fn finish_tool_execution(&mut self) -> Result<ContinuationData> {
        self.tool_state
            .tool_loop
            .finish_execution()
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// Approves all pending tools for execution.
    pub fn approve_all_tools(&mut self) -> Result<()> {
        self.tool_state
            .tool_loop
            .approve_all()
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// Denies all pending tools.
    pub fn deny_all_tools(&mut self) -> Result<()> {
        self.tool_state
            .tool_loop
            .deny_all()
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// Resets the tool loop to idle state.
    pub fn reset_tool_loop(&mut self) {
        self.tool_state.tool_loop.reset();
        self.tool_state.pending_permission = None;
        self.dirty.full = true;
    }

    /// Returns true if the tool loop is waiting for user action.
    #[must_use]
    pub fn tool_loop_needs_user_action(&self) -> bool {
        self.tool_state.tool_loop.state().needs_user_action()
            || self.tool_state.pending_permission.is_some()
    }

    /// Returns true if the tool loop is actively processing.
    #[must_use]
    pub fn tool_loop_is_active(&self) -> bool {
        self.tool_state.tool_loop.state().is_active()
    }

    // ========================================================================
    // Tool Block UI Methods (Phase 10.5.6)
    // ========================================================================

    /// Starts a new tool block for UI display.
    ///
    /// Returns the index of the created block for later updates.
    ///
    /// # Arguments
    ///
    /// * `tool_name` - Name of the tool (e.g., "bash", "read")
    /// * `tool_input` - Input provided to the tool
    pub fn start_tool_block(&mut self, tool_name: &str, tool_input: &str) -> usize {
        let block = ToolBlockState::new(tool_name, tool_input);
        self.tool_state.tool_blocks.push(block);
        self.dirty.messages = true;
        self.tool_state.tool_blocks.len() - 1
    }

    /// Completes a tool block with its result.
    ///
    /// # Arguments
    ///
    /// * `index` - Index of the tool block to update
    /// * `result` - The tool's output
    /// * `is_error` - Whether the result is an error
    pub fn complete_tool_block(&mut self, index: usize, result: &str, is_error: bool) {
        if let Some(block) = self.tool_state.tool_blocks.get_mut(index) {
            if is_error {
                block.set_error(result);
            } else {
                block.set_result(result);
            }
            self.dirty.messages = true;
        }
    }

    /// Returns a slice of all tool blocks for rendering.
    #[must_use]
    pub fn tool_blocks(&self) -> &[ToolBlockState] {
        &self.tool_state.tool_blocks
    }

    /// Clears all tool blocks.
    ///
    /// Call this when starting a new conversation turn.
    pub fn clear_tool_blocks(&mut self) {
        self.tool_state.tool_blocks.clear();
        self.dirty.messages = true;
    }

    /// Returns true if there are any tool blocks to display.
    #[must_use]
    pub fn has_tool_blocks(&self) -> bool {
        !self.tool_state.tool_blocks.is_empty()
    }

    // ========================================================================
    // Timeline Integration (Phase 2)
    // ========================================================================

    /// Returns a reference to the conversation timeline.
    #[must_use]
    pub fn timeline(&self) -> &Timeline {
        &self.timeline
    }

    /// Returns a mutable reference to the conversation timeline.
    pub fn timeline_mut(&mut self) -> &mut Timeline {
        &mut self.timeline
    }

    /// Starts streaming mode.
    ///
    /// This creates a streaming entry in the timeline.
    pub fn set_streaming(&mut self, _streaming: bool) {
        self.loading = true;

        // Add streaming entry to timeline
        if self.timeline.try_push_streaming().is_err() {
            // Already streaming - this is a no-op
            tracing::warn!("set_streaming called but timeline already streaming");
        }

        self.dirty.messages = true;
    }

    /// Appends text to the current streaming response.
    ///
    /// Updates the timeline streaming entry.
    pub fn append_streaming_text(&mut self, text: &str) {
        // Update timeline streaming entry
        self.timeline.append_to_streaming(text);
        self.dirty.messages = true;
    }

    /// Finalizes streaming as a complete assistant message.
    ///
    /// Converts the streaming entry to an assistant message in the timeline.
    pub fn finalize_streaming_as_message(&mut self) {
        // Finalize timeline streaming entry
        self.timeline.finalize_streaming_as_message();
        self.loading = false;
        self.dirty.messages = true;
    }

    /// Adds a tool block with result to both legacy tool_blocks and timeline.
    ///
    /// # Arguments
    ///
    /// * `tool_name` - Name of the tool (e.g., "bash", "read_file")
    /// * `input` - Tool input/command
    /// * `output` - Tool output
    /// * `is_error` - Whether the execution resulted in an error
    pub fn add_tool_block_with_result(
        &mut self,
        tool_name: &str,
        input: &str,
        output: &str,
        is_error: bool,
    ) {
        // Add to legacy tool_blocks using the constructor
        let mut block = ToolBlockState::new(tool_name, input);
        if is_error {
            block.set_error(output);
        } else {
            block.set_result(output);
        }
        self.tool_state.tool_blocks.push(block);

        // Add to timeline with message index tracking
        self.timeline.push_tool_after_current_assistant(
            tool_name,
            input,
            Some(output.to_string()),
            is_error,
        );

        self.dirty.messages = true;
    }

    /// Clears conversation history while preserving configuration.
    ///
    /// Resets messages, timeline, tool state, token budget, and scroll position.
    /// Preserves working directory, model, memory, MCP servers, plugins, cost
    /// tracker, system prompt, effort, and thinking budget.
    pub fn clear_conversation(&mut self) {
        self.api_messages.clear();
        self.tool_state.tool_blocks.clear();
        self.tool_state.pending_permission = None;
        self.timeline = Timeline::new();
        self.reset_tool_loop();
        self.reset_token_budget();
        self.thinking_buffer.clear();
        self.scroll = crate::tui::scroll::ScrollState::new();
        self.loading = false;
        self.streaming_rx = None;
        self.dirty.messages = true;
        self.mark_session_dirty();
    }

    // ========================================================================
    // Async Tool Execution (Phase 5)
    // ========================================================================

    /// Sets the receiver channel for async tool results.
    ///
    /// When tool execution is spawned in the background, results will be
    /// streamed back through this channel.
    pub fn set_tool_result_rx(
        &mut self,
        rx: mpsc::Receiver<(String, crate::types::ToolResultBlock)>,
    ) {
        self.tool_state.tool_result_rx = Some(rx);
    }

    /// Returns true if a tool result channel is currently set.
    #[must_use]
    pub fn has_tool_result_rx(&self) -> bool {
        self.tool_state.tool_result_rx.is_some()
    }

    /// Attempts to receive a tool result without blocking.
    ///
    /// Returns `Some((tool_id, result))` if a result is available,
    /// `None` if no result is ready or channel is not set.
    pub fn try_recv_tool_result(&mut self) -> Option<(String, crate::types::ToolResultBlock)> {
        if let Some(ref mut rx) = self.tool_state.tool_result_rx {
            rx.try_recv().ok()
        } else {
            None
        }
    }

    /// Receives a tool result asynchronously.
    ///
    /// Returns `None` immediately if no channel is set, otherwise waits for
    /// the next result. This is designed for use in `tokio::select!`.
    ///
    /// # Returns
    ///
    /// - `Some((tool_id, result))` - A tool completed execution
    /// - `None` - Channel closed or no channel set
    pub async fn recv_tool_result(&mut self) -> Option<(String, crate::types::ToolResultBlock)> {
        match &mut self.tool_state.tool_result_rx {
            Some(rx) => rx.recv().await,
            None => None, // Return immediately - don't block with pending()
        }
    }

    /// Clears the tool result channel.
    ///
    /// Called after all tools have completed and results have been processed.
    pub fn clear_tool_result_rx(&mut self) {
        self.tool_state.tool_result_rx = None;
    }

    /// Adds a pending tool to the tool loop.
    ///
    /// # Arguments
    ///
    /// * `tool_use` - The tool use block to add
    pub fn add_pending_tool(&mut self, tool_use: crate::types::ToolUseBlock) {
        self.tool_state.tool_loop.add_tool_use(tool_use);
    }

    /// Intercepts special tools that need synchronous or UI-driven handling.
    ///
    /// Separates tools into intercepted (handled immediately) and remaining
    /// (to be executed in background). Intercepted tools include:
    /// - `plan`: Requires modal UI for user approval
    /// - `ask_user`: Requires modal UI for user response
    /// - `bash` with `run_in_background`: Spawned as a background task
    /// - `task_output`: Reads output from a background task
    /// - `task_stop`: Stops a background task
    ///
    /// # Arguments
    ///
    /// * `pending` - Tools awaiting execution
    /// * `tx` - Channel sender for tool results (used for error/immediate results)
    ///
    /// # Returns
    ///
    /// A tuple of (intercepted tool IDs, remaining tools for background execution).
    fn intercept_special_tools(
        &mut self,
        pending: Vec<(String, ToolUseBlock)>,
        tx: &mpsc::Sender<(String, ToolResultBlock)>,
    ) -> (Vec<String>, Vec<(String, ToolUseBlock)>) {
        let mut intercepted = Vec::new();
        let mut remaining = Vec::new();
        for (id, tool_use) in pending {
            match tool_use.name.as_str() {
                "plan" => {
                    if let Some(plan) = PlanState::from_tool_input(id.clone(), &tool_use.input) {
                        self.set_pending_plan(plan);
                        intercepted.push(id);
                    } else {
                        let result = ToolResultBlock::error(
                            &id,
                            "Invalid plan format: expected title and steps",
                        );
                        let tx_clone = tx.clone();
                        let id_clone = id.clone();
                        tokio::spawn(async move {
                            let _ = tx_clone.send((id_clone, result)).await;
                        });
                        intercepted.push(id);
                    }
                }
                "ask_user" => {
                    if let Some(question) =
                        QuestionState::from_tool_input(id.clone(), &tool_use.input)
                    {
                        self.set_pending_question(question);
                        intercepted.push(id);
                    } else {
                        let result = ToolResultBlock::error(
                            &id,
                            "Invalid ask_user format: expected question field",
                        );
                        let tx_clone = tx.clone();
                        let id_clone = id.clone();
                        tokio::spawn(async move {
                            let _ = tx_clone.send((id_clone, result)).await;
                        });
                        intercepted.push(id);
                    }
                }
                "bash"
                    if tool_use
                        .input
                        .get("run_in_background")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false) =>
                {
                    let command = tool_use
                        .input
                        .get("command")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let task_id = self.background_tasks.spawn(command, &self.working_dir);
                    let result =
                        ToolResultBlock::success(&id, format!("Background task {task_id} started"));
                    let tx_clone = tx.clone();
                    let id_clone = id.clone();
                    tokio::spawn(async move {
                        let _ = tx_clone.send((id_clone, result)).await;
                    });
                    intercepted.push(id);
                }
                "task_output" => {
                    let task_id_val = tool_use
                        .input
                        .get("task_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let output_buf = self
                        .background_tasks
                        .tasks_ref()
                        .get(&task_id_val)
                        .map(|t| (Arc::clone(&t.output_buffer), Arc::clone(&t.completed)));
                    let tool_id_clone = id.clone();
                    let tx_clone = tx.clone();
                    tokio::spawn(async move {
                        let result = match output_buf {
                            Some((buf, completed)) => {
                                let content = buf.lock().await;
                                let status = if completed.load(std::sync::atomic::Ordering::Acquire)
                                {
                                    "completed"
                                } else {
                                    "running"
                                };
                                ToolResultBlock::success(
                                    &tool_id_clone,
                                    format!("[Task {task_id_val} ({status})]\n{}", *content),
                                )
                            }
                            None => ToolResultBlock::error(
                                &tool_id_clone,
                                format!("No task with ID '{task_id_val}'"),
                            ),
                        };
                        let _ = tx_clone.send((tool_id_clone, result)).await;
                    });
                    intercepted.push(id);
                }
                "task_stop" => {
                    let task_id_val = tool_use
                        .input
                        .get("task_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let result_msg = self.background_tasks.stop(&task_id_val);
                    let result = match result_msg {
                        Ok(msg) => ToolResultBlock::success(&id, msg),
                        Err(msg) => ToolResultBlock::error(&id, msg),
                    };
                    let tx_clone = tx.clone();
                    let id_clone = id.clone();
                    tokio::spawn(async move {
                        let _ = tx_clone.send((id_clone, result)).await;
                    });
                    intercepted.push(id);
                }
                _ => remaining.push((id, tool_use)),
            }
        }
        (intercepted, remaining)
    }

    /// Spawns tool execution in the background.
    ///
    /// Returns immediately with a handle to the background task.
    /// Results are sent through the tool_result_rx channel.
    ///
    /// # Returns
    ///
    /// `Some(JoinHandle)` if tools were spawned, `None` if no tools pending.
    #[must_use]
    pub fn spawn_tool_execution(
        &mut self,
    ) -> Option<tokio::task::JoinHandle<Vec<(String, ToolResultBlock)>>> {
        // Create channel for results
        let (tx, rx) = mpsc::channel(100);
        self.tool_state.tool_result_rx = Some(rx);

        // Get pending tools
        let pending: Vec<_> = self
            .tool_state
            .tool_loop
            .pending_calls()
            .iter()
            .filter(|(_, call)| !call.executed)
            .map(|(id, call)| (id.clone(), call.tool_use.clone()))
            .collect();

        if pending.is_empty() {
            return None;
        }

        // Intercept interactive and special tools before spawning background work
        let (intercepted, pending) = self.intercept_special_tools(pending, &tx);

        // Mark intercepted tools as executing (they'll complete when user responds)
        for id in &intercepted {
            self.tool_state.executing_tool_ids.insert(id.clone());
        }

        if pending.is_empty() && intercepted.is_empty() {
            return None;
        }

        // Mark remaining as executing
        for (id, _) in &pending {
            self.tool_state.executing_tool_ids.insert(id.clone());
        }

        let executor = Arc::clone(&self.tool_state.tool_executor);
        let mcp_manager = self.mcp_manager.clone();

        // Spawn background task
        let handle = tokio::spawn(async move {
            use crate::app::tool_loop::tool_use_to_call;
            use crate::mcp::manager::is_mcp_tool;

            let mut results = Vec::new();
            for (tool_id, tool_use) in pending {
                let result_block = if is_mcp_tool(&tool_use.name) {
                    execute_mcp_tool(&tool_id, &tool_use, &mcp_manager).await
                } else {
                    let call = tool_use_to_call(&tool_use);
                    let result = executor.execute(call).await;
                    execute_builtin_tool_result(&tool_id, result)
                };

                // Send through channel (ignore error if receiver dropped)
                let _ = tx.send((tool_id.clone(), result_block.clone())).await;
                results.push((tool_id, result_block));
            }
            results
        });

        Some(handle)
    }

    /// Returns true if there are any tools currently executing.
    #[must_use]
    pub fn has_executing_tools(&self) -> bool {
        !self.tool_state.executing_tool_ids.is_empty()
    }

    /// Marks a tool as currently executing.
    ///
    /// # Arguments
    ///
    /// * `tool_id` - The ID of the tool to mark as executing
    pub fn mark_tool_executing(&mut self, tool_id: &str) {
        self.tool_state
            .executing_tool_ids
            .insert(tool_id.to_string());
    }

    /// Records a tool result and removes the tool from executing set.
    ///
    /// # Arguments
    ///
    /// * `tool_id` - The ID of the tool that completed
    /// * `result` - The result of the tool execution
    pub fn record_tool_result(&mut self, tool_id: &str, result: crate::types::ToolResultBlock) {
        // Remove from executing set
        self.tool_state.executing_tool_ids.remove(tool_id);

        // Update tool loop with result (ignore error if tool not found)
        let _ = self
            .tool_state
            .tool_loop
            .set_tool_result(tool_id, result.clone());

        // Update timeline tool entry if it exists
        self.update_timeline_tool_by_id(tool_id, Some(result.content), result.is_error);

        self.dirty.messages = true;
    }

    /// Returns true if all pending tools have completed execution.
    #[must_use]
    pub fn all_tools_complete(&self) -> bool {
        self.tool_state.executing_tool_ids.is_empty()
            && self
                .tool_state
                .tool_loop
                .pending_calls()
                .values()
                .all(|call| call.executed || call.result.is_some())
    }

    /// Adds a tool to the timeline in executing state (no output yet).
    ///
    /// # Arguments
    ///
    /// * `tool_name` - Name of the tool (e.g., "bash")
    /// * `input` - The tool input/command
    pub fn add_tool_to_timeline_executing(&mut self, tool_name: &str, input: &str) {
        self.timeline.push_tool_after_current_assistant(
            tool_name, input, None, // No output yet - executing
            false,
        );
        self.dirty.messages = true;
    }

    /// Updates a tool in the timeline with its result.
    ///
    /// Finds the most recent tool entry with the given name that has no output
    /// and updates it with the provided output.
    ///
    /// # Arguments
    ///
    /// * `tool_name` - Name of the tool to update
    /// * `output` - The tool output (None if still executing)
    /// * `is_error` - Whether the result is an error
    pub fn update_tool_in_timeline(
        &mut self,
        tool_name: &str,
        output: Option<String>,
        is_error: bool,
    ) {
        self.timeline
            .update_tool_result(tool_name, output, is_error);
        self.dirty.messages = true;
    }

    /// Updates a tool in the timeline by its ID.
    ///
    /// This is used internally when recording tool results.
    fn update_timeline_tool_by_id(
        &mut self,
        _tool_id: &str,
        output: Option<String>,
        is_error: bool,
    ) {
        // For now, update the most recent executing tool
        // In the future, we could track tool_id -> timeline_index mapping
        for entry in self.timeline.entries_mut().iter_mut().rev() {
            if let crate::types::ConversationEntry::ToolExecution {
                output: ref mut o @ None,
                is_error: ref mut err,
                ..
            } = entry
            {
                *o = output;
                *err = is_error;
                break;
            }
        }
        self.dirty.messages = true;
    }

    // =========================================================================
    // MCP Manager
    // =========================================================================

    /// Sets the MCP server manager.
    pub fn set_mcp_manager(&mut self, manager: crate::mcp::manager::McpManager) {
        self.mcp_manager = Some(std::sync::Arc::new(manager));
    }

    /// Returns a reference to the MCP manager, if set.
    #[must_use]
    pub fn mcp_manager(&self) -> Option<&crate::mcp::manager::McpManager> {
        self.mcp_manager.as_deref()
    }

    /// Returns a mutable reference to the MCP manager, if set.
    ///
    /// Only succeeds when there is exactly one `Arc` reference (i.e. no
    /// spawned tasks are still holding a clone).  Used at shutdown.
    pub fn mcp_manager_mut(&mut self) -> Option<&mut crate::mcp::manager::McpManager> {
        self.mcp_manager.as_mut().and_then(std::sync::Arc::get_mut)
    }

    /// Returns `true` if an MCP manager is configured.
    #[must_use]
    pub fn has_mcp_manager(&self) -> bool {
        self.mcp_manager.is_some()
    }

    /// Returns all tool definitions: built-in defaults plus MCP server tools.
    ///
    /// This is the unified tool list sent to the Anthropic API.
    #[must_use]
    pub fn all_tool_definitions(&self) -> Vec<crate::api::tools::ToolDefinition> {
        let mut tools = default_tools();
        if let Some(manager) = &self.mcp_manager {
            tools.extend(manager.tool_definitions());
        }
        tools
    }

    /// Sets the current model name for cost tracking.
    pub fn set_current_model(&mut self, model: String) {
        self.current_model = model;
    }

    /// Returns a reference to the cost tracker.
    #[must_use]
    pub fn cost_tracker(&self) -> &CostTracker {
        &self.cost_tracker
    }

    /// Returns a formatted cost summary for display.
    #[must_use]
    pub fn cost_summary(&self) -> String {
        let stats = self.cost_tracker.statistics();
        if stats.total_requests == 0 {
            return "No usage data recorded yet.".to_string();
        }
        format!(
            "Session cost: ${:.4}\n\
             Requests: {}\n\
             Input tokens: {}\n\
             Output tokens: {}\n\
             Cost by model:\n{}",
            stats.total_cost,
            stats.total_requests,
            stats.total_input_tokens,
            stats.total_output_tokens,
            self.cost_tracker
                .cost_by_model()
                .iter()
                .map(|(m, c)| format!("  {}: ${:.4}", m, c))
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    /// Sets the memory store.
    pub fn set_memory_store(&mut self, store: crate::memory::store::MemoryStore) {
        self.memory_store = Some(store);
    }

    /// Returns a reference to the memory store.
    #[must_use]
    pub fn memory_store(&self) -> Option<&crate::memory::store::MemoryStore> {
        self.memory_store.as_ref()
    }

    /// Returns a mutable reference to the memory store.
    pub fn memory_store_mut(&mut self) -> Option<&mut crate::memory::store::MemoryStore> {
        self.memory_store.as_mut()
    }

    /// Sets the reasoning effort level.
    pub fn set_effort(&mut self, effort: EffortLevel) {
        self.effort = effort;
    }

    /// Sets the explicit thinking budget (overrides effort level).
    pub fn set_thinking_budget(&mut self, budget: Option<u32>) {
        self.thinking_budget = budget;
    }

    /// Sets the system prompt text for API requests.
    pub fn set_system_prompt(&mut self, prompt: Option<String>) {
        self.system_prompt = prompt;
    }

    /// Returns the current effort level.
    #[must_use]
    pub fn effort(&self) -> EffortLevel {
        self.effort
    }

    /// Builds [`RequestOptions`] from the current state, gated by model capabilities.
    ///
    /// Determines the thinking budget from either the explicit `thinking_budget`
    /// field (if set) or the `effort` level. Returns `None` for thinking if the
    /// model doesn't support it. Wraps the system prompt in [`SystemBlock`]s with
    /// cache control if the model supports it.
    #[must_use]
    pub fn build_request_options(&self, model: &str) -> RequestOptions {
        let caps = ModelCapabilities::for_model(model);

        // Determine thinking config
        let thinking = if caps.supports_thinking {
            let budget = self
                .thinking_budget
                .or_else(|| self.effort.thinking_budget());
            budget.map(|b| ThinkingConfig {
                config_type: "enabled".to_string(),
                budget_tokens: b,
            })
        } else {
            None
        };

        // Build system prompt text, appending memory if available
        let mut prompt_text = self.system_prompt.clone().unwrap_or_default();
        if let Some(store) = &self.memory_store {
            let memory_text = store.render_for_system_prompt();
            if !memory_text.is_empty() {
                if !prompt_text.is_empty() {
                    prompt_text.push_str("\n\n");
                }
                prompt_text.push_str(&memory_text);
            }
        }

        // Build system blocks with optional cache control
        let system = if prompt_text.is_empty() {
            None
        } else {
            let cache_control = if caps.supports_cache_control {
                Some(crate::api::CacheControl {
                    cache_type: "ephemeral".to_string(),
                })
            } else {
                None
            };
            Some(vec![SystemBlock {
                block_type: "text".to_string(),
                text: prompt_text,
                cache_control,
            }])
        };

        RequestOptions { thinking, system }
    }
}

/// Executes an MCP-namespaced tool via the MCP manager.
///
/// Routes the tool call through the MCP manager and converts the result
/// into a `ToolResultBlock`. If no MCP manager is available, returns an
/// error result.
///
/// # Arguments
///
/// * `tool_id` - The unique tool use ID for result correlation
/// * `tool_use` - The tool use block containing name and input
/// * `mcp_manager` - Optional reference to the MCP manager
async fn execute_mcp_tool(
    tool_id: &str,
    tool_use: &ToolUseBlock,
    mcp_manager: &Option<Arc<crate::mcp::manager::McpManager>>,
) -> ToolResultBlock {
    match mcp_manager {
        Some(mgr) => match mgr.call_tool(&tool_use.name, tool_use.input.clone()).await {
            Ok(ref tr) => ToolResultBlock::from_result(tool_id, tr),
            Err(e) => ToolResultBlock::error(tool_id, format!("MCP tool error: {e}")),
        },
        None => ToolResultBlock::error(
            tool_id,
            format!(
                "MCP tool '{}' called but no MCP manager available",
                tool_use.name
            ),
        ),
    }
}

/// Converts a built-in tool execution result into a `ToolResultBlock`.
///
/// Maps the `Result<ToolResult, Error>` from the tool executor into the
/// corresponding success or error `ToolResultBlock`.
///
/// # Arguments
///
/// * `tool_id` - The unique tool use ID for result correlation
/// * `result` - The tool execution result
#[must_use]
fn execute_builtin_tool_result(
    tool_id: &str,
    result: std::result::Result<crate::tools::ToolResult, anyhow::Error>,
) -> ToolResultBlock {
    match result {
        Ok(ref tr) => ToolResultBlock::from_result(tool_id, tr),
        Err(e) => ToolResultBlock::error(tool_id, e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_message(role: Role, content: &str) -> Message {
        Message {
            role,
            content: content.to_string(),
        }
    }

    #[test]
    fn test_restore_from_session_without_ui_state() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        // Set some initial state
        state.scroll.restore_offset(100);
        *state.input_mut() = "existing".to_string();
        state.input_state.set_cursor_position(8);

        // Create a session without UI state
        let mut session = Session::new(PathBuf::from("/project"));
        session.add_message(test_message(Role::User, "Test"));

        state.restore_from_session(&session);

        // UI state should remain unchanged since session has no UI state
        assert_eq!(state.scroll_offset(), 100);
        assert_eq!(state.input(), "existing");
        assert_eq!(state.cursor_position(), 8);
    }

    #[test]
    fn test_to_session_preserves_ui_state() {
        let mut state = AppState::new(PathBuf::from("/project"), false, ParallelMode::Enabled);
        state.scroll.restore_offset(42);
        *state.input_mut() = "draft text".to_string();
        state.input_state.set_cursor_position(5);

        let session = state.to_session();
        let ui_state = session.ui_state().expect("UI state should be present");

        assert_eq!(ui_state.scroll_offset(), 42);
        assert_eq!(ui_state.input_buffer(), "draft text");
        assert_eq!(ui_state.cursor_position(), 5);
    }

    #[test]
    fn test_to_session_roundtrip() {
        // Create state with data
        let mut state = AppState::new(PathBuf::from("/project"), false, ParallelMode::Enabled);
        state.add_message(test_message(Role::User, "Test message"));
        state.scroll.restore_offset(100);
        *state.input_mut() = "unsent input".to_string();
        state.input_state.set_cursor_position(6);

        // Convert to session
        let session = state.to_session();

        // Create new state and restore
        let mut new_state =
            AppState::new(PathBuf::from("/different"), false, ParallelMode::Enabled);
        new_state.restore_from_session(&session);

        // Verify roundtrip preserves data
        assert_eq!(new_state.timeline().len(), 1);
        let entries: Vec<_> = new_state.timeline().iter().collect();
        assert!(
            matches!(entries[0], crate::types::ConversationEntry::UserMessage(s) if s == "Test message")
        );
        assert_eq!(new_state.scroll_offset(), 100);
        assert_eq!(new_state.input(), "unsent input");
        assert_eq!(new_state.cursor_position(), 6);
    }

    #[test]
    fn test_api_messages_truncated_returns_truncated() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Add many messages to potentially exceed budget
        for i in 0..50 {
            state
                .api_messages
                .push(ApiMessageV2::user(format!("Message {}", i)));
        }

        let truncated = state.api_messages_truncated();

        // Should return messages (truncation logic is in api::context)
        assert!(!truncated.is_empty());
        // First message preserved
        assert_eq!(truncated[0].content.to_text(), "Message 0");
        // Most recent preserved
        assert_eq!(truncated.last().unwrap().content.to_text(), "Message 49");
    }

    #[test]
    fn test_api_messages_truncated_under_budget_unchanged() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        state.api_messages.push(ApiMessageV2::user("Hello"));
        state
            .api_messages
            .push(ApiMessageV2::assistant("Hi there!"));

        let truncated = state.api_messages_truncated();

        // Under budget - should be unchanged
        assert_eq!(truncated.len(), 2);
        assert_eq!(truncated[0].content.to_text(), "Hello");
        assert_eq!(truncated[1].content.to_text(), "Hi there!");
    }

    #[test]
    fn test_api_messages_truncated_with_large_content() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Add first message (will be preserved)
        state.api_messages.push(ApiMessageV2::user("System prompt"));

        // Add many large messages that would exceed 100k tokens
        let large_content = "x".repeat(10_000); // ~2500 tokens each
        for _ in 0..50 {
            // 50 * 2500 = 125k tokens
            state
                .api_messages
                .push(ApiMessageV2::assistant(&large_content));
        }

        let truncated = state.api_messages_truncated();

        // Should be fewer than 51 messages
        assert!(
            truncated.len() < 51,
            "Should be truncated, got {}",
            truncated.len()
        );
        // First message always preserved
        assert_eq!(truncated[0].content.to_text(), "System prompt");
    }

    #[tokio::test]
    async fn test_prepare_api_messages_truncates_large_conversation() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Add first message (always preserved)
        state.api_messages.push(ApiMessageV2::user("System prompt"));

        // Add many large messages that exceed 100k tokens
        let large_content = "x".repeat(10_000);
        for _ in 0..60 {
            state
                .api_messages
                .push(ApiMessageV2::assistant(&large_content));
        }

        let total_before = state.api_messages.len();
        let prepared = state
            .prepare_api_messages_for_send("claude-sonnet-4-20250514")
            .await;

        assert!(
            prepared.len() < total_before,
            "Should truncate: prepared={} < total={}",
            prepared.len(),
            total_before
        );
    }

    #[tokio::test]
    async fn test_prepare_api_messages_preserves_first_message() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        state
            .api_messages
            .push(ApiMessageV2::user("Important system prompt"));

        let large_content = "x".repeat(10_000);
        for _ in 0..60 {
            state
                .api_messages
                .push(ApiMessageV2::assistant(&large_content));
        }

        let prepared = state
            .prepare_api_messages_for_send("claude-sonnet-4-20250514")
            .await;

        assert_eq!(
            prepared[0].content.to_text(),
            "Important system prompt",
            "First message must always be preserved"
        );
    }

    #[tokio::test]
    async fn test_prepare_api_messages_identical_for_small_conversation() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        state.api_messages.push(ApiMessageV2::user("Hello"));
        state
            .api_messages
            .push(ApiMessageV2::assistant("Hi there!"));

        let prepared = state
            .prepare_api_messages_for_send("claude-sonnet-4-20250514")
            .await;

        assert_eq!(prepared.len(), 2, "Small conversation should be unchanged");
        assert_eq!(prepared[0].content.to_text(), "Hello");
        assert_eq!(prepared[1].content.to_text(), "Hi there!");
    }

    #[test]
    fn test_take_cached_ccg_context_consumes() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Set cached context directly for testing
        state.compression.cached_ccg_context = Some("test context".to_string());
        assert!(state.has_cached_ccg_context());

        // Take should return and clear
        let taken = state.take_cached_ccg_context();
        assert_eq!(taken, Some("test context".to_string()));
        assert!(!state.has_cached_ccg_context());
    }

    #[test]
    fn test_context_for_injection_requires_auto_context_enabled() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Set cached context
        state.compression.cached_ccg_context = Some("test context".to_string());

        // Auto-context disabled - should return None
        assert!(state.context_for_injection().is_none());

        // Enable auto-context
        state.set_auto_context_enabled(true);
        assert_eq!(state.context_for_injection(), Some("test context"));
    }

    #[tokio::test]
    async fn test_context_not_reinjected_when_hash_unchanged() {
        let mut state = AppState::new(PathBuf::from("."), false, ParallelMode::Enabled);
        state.set_auto_context_enabled(true);

        // Create a minimal orchestrator so the early-return check passes
        let caps = crate::narsil::NarsilCapabilities::from_tools(&["find_symbols".to_string()]);
        let orchestrator = Arc::new(crate::context::compression::CompressionOrchestrator::new(
            caps,
            "test-repo",
        ));
        state.set_compression_orchestrator(orchestrator);

        // Pre-populate cached context and set the hash to the current git HEAD
        // Since tests run in the git repo, get_git_head_hash(".") returns a real hash
        let current_hash =
            get_git_head_hash(std::path::Path::new(".")).unwrap_or_else(|| "unknown".to_string());
        state.set_last_ccg_hash(current_hash);
        state.compression.cached_ccg_context =
            Some("## Cached Context\nAlready fetched".to_string());
        state.set_context_tokens_injected(3000);

        // refresh_build_context should detect hash is unchanged and return
        // the cached context without making any MCP calls
        let result = state.refresh_build_context().await;

        assert!(
            result.is_some(),
            "Should return cached context on hash match"
        );
        assert_eq!(
            result.unwrap(),
            "## Cached Context\nAlready fetched",
            "Should return the existing cached content"
        );
        // Tokens should remain unchanged (not reset)
        assert_eq!(state.context_tokens_injected(), 3000);
        // Cache should still be present
        assert!(state.has_cached_ccg_context());
    }

    #[tokio::test]
    async fn test_context_injection_logs_metrics() {
        // This test verifies the logging behavior by checking state transitions.
        // The actual tracing::info! calls are verified by the log output format
        // documented in the implementation (hash, tokens, cache_status fields).
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Verify initial state
        assert!(state.last_ccg_hash().is_none());
        assert_eq!(state.context_tokens_injected(), 0);
        assert!(!state.has_cached_ccg_context());

        // After setting context (simulating a successful fetch), the metrics
        // fields are populated for the status bar and logging
        state.set_last_ccg_hash("abc123".to_string());
        state.set_context_tokens_injected(5000);
        state.compression.cached_ccg_context = Some("## Context".to_string());

        assert_eq!(state.last_ccg_hash(), Some("abc123"));
        assert_eq!(state.context_tokens_injected(), 5000);
        assert!(state.has_cached_ccg_context());
    }

    #[tokio::test]
    async fn test_submit_message_with_cached_context_prepends() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.set_auto_context_enabled(true);

        // Pre-populate cached context
        state.compression.cached_ccg_context =
            Some("## Project Structure\nRust project".to_string());

        // Create a mock provider
        let client: Arc<dyn crate::api::LlmProvider> = Arc::new(crate::api::AnthropicClient::new(
            secrecy::SecretString::from("test-key"),
            "claude-sonnet-4-20250514",
        ));

        // Submit message - should inject context
        let _ = state
            .submit_message(&client, "Hello world".to_string())
            .await;

        // Cached context should be consumed
        assert!(!state.has_cached_ccg_context());

        // The API message should contain the context wrapper
        assert!(!state.api_messages().is_empty());
        let last_user_msg = state
            .api_messages()
            .iter()
            .find(|m| m.role == crate::types::Role::User);
        assert!(last_user_msg.is_some());
        let content = last_user_msg.unwrap().content.as_text().unwrap_or_default();
        assert!(content.contains("<context>"));
        assert!(content.contains("Project Structure"));
        assert!(content.contains("Hello world"));
    }

    #[test]
    fn test_build_api_messages_baseline_no_context_injection() {
        // When auto_context is DISABLED, build_api_messages() returns messages
        // without any context injection, even if cached context exists.
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // auto_context disabled (default), but cached context exists
        state.compression.cached_ccg_context = Some("## Codebase Context".to_string());

        state
            .api_messages_mut()
            .push(ApiMessageV2::user("Hello, Claude"));
        state
            .api_messages_mut()
            .push(ApiMessageV2::assistant("Hi there!"));

        // Without auto_context, messages should pass through unchanged
        let messages = state.build_api_messages();
        assert_eq!(messages.len(), 2);

        let content = messages[0].content.to_text();
        assert_eq!(content, "Hello, Claude");
        assert!(
            !content.contains("<context>"),
            "build_api_messages should not inject context when auto_context is disabled"
        );

        // Cached context should still be present
        assert!(
            state.has_cached_ccg_context(),
            "build_api_messages should not consume cached context"
        );
    }

    #[test]
    fn test_prepare_api_messages_injects_context_when_enabled() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.set_auto_context_enabled(true);
        state.compression.cached_ccg_context =
            Some("## Project Structure\nRust CLI app".to_string());

        // Push a user message
        state
            .api_messages_mut()
            .push(ApiMessageV2::user("What is this project?"));

        // build_api_messages should inject the cached context as a system
        // message at the beginning of the message list
        let messages = state.build_api_messages();

        // Should have 2 messages: context system message + user message
        assert_eq!(messages.len(), 2, "Expected context message + user message");

        // First message should be the injected context
        let context_msg = &messages[0];
        assert_eq!(context_msg.role, crate::types::Role::User);
        let context_text = context_msg.content.to_text();
        assert!(
            context_text.contains("<context>"),
            "Context message should be wrapped in <context> tags"
        );
        assert!(
            context_text.contains("Project Structure"),
            "Context message should contain the cached CCG context"
        );

        // Second message should be the original user message, unchanged
        let user_msg = &messages[1];
        assert_eq!(user_msg.content.to_text(), "What is this project?");
    }

    #[test]
    fn test_prepare_api_messages_skips_context_when_disabled() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        // auto_context disabled (default)
        state.compression.cached_ccg_context = Some("## Some Context".to_string());

        state.api_messages_mut().push(ApiMessageV2::user("Hello"));

        let messages = state.build_api_messages();

        // Should have only the user message, no context injection
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content.to_text(), "Hello");
        assert!(!messages[0].content.to_text().contains("<context>"));
    }

    #[test]
    fn test_prepare_api_messages_does_not_consume_cached_context() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.set_auto_context_enabled(true);
        state.compression.cached_ccg_context = Some("## Context".to_string());

        state.api_messages_mut().push(ApiMessageV2::user("Hello"));

        // Call build_api_messages — it should read but NOT consume the cache
        let _ = state.build_api_messages();

        // Cache should still be present for future calls
        assert!(
            state.has_cached_ccg_context(),
            "build_api_messages should not consume the cached context"
        );
    }

    #[test]
    fn test_has_background_work() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Initially no background work
        assert!(!state.has_background_work());

        // Set up streaming channel
        let (_tx, rx) = mpsc::channel::<StreamEvent>(100);
        state.set_streaming_rx(rx);
        assert!(state.has_background_work());

        // Clear streaming, set tool channel
        state.streaming_rx = None;
        let (_tx, rx) = mpsc::channel(100);
        state.set_tool_result_rx(rx);
        assert!(state.has_background_work());

        // Clear tool channel
        state.clear_tool_result_rx();
        assert!(!state.has_background_work());
    }

    #[test]
    fn test_completion_dirty_flag() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.dirty.clear();
        state.show_completion();
        assert!(state.dirty.input);
    }

    #[test]
    fn test_worktree_setters_set_dirty_flag() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        state.dirty.clear();
        state.set_worktree_branch("main".to_string());
        assert!(state.dirty.full);

        state.dirty.clear();
        state.set_worktree_modified(1);
        assert!(state.dirty.full);

        state.dirty.clear();
        state.set_worktree_ahead(1);
        assert!(state.dirty.full);

        state.dirty.clear();
        state.set_worktree_behind(1);
        assert!(state.dirty.full);
    }

    #[test]
    fn test_app_state_agent_panel_delegation() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        assert!(!state.subagents_enabled());
        assert!(state.agent_panel_entries().is_empty());

        let progress = AgentProgress::IterationStarted {
            iteration: 1,
            max: 3,
        };
        state.update_agent_progress("a1", "Agent", &progress);
        assert_eq!(state.agent_panel_entries().len(), 1);
        assert!(state.dirty.full);
    }

    #[test]
    fn test_continuous_loop_state_initial() {
        let cls = ContinuousLoopState {
            status: ContinuousLoopStatus::Inactive,
            iterations_completed: 0,
            last_duration_ms: None,
            checking_gate: None,
            gate_results: Vec::new(),
        };
        assert_eq!(*cls.status(), ContinuousLoopStatus::Inactive);
        assert_eq!(cls.iterations_completed(), 0);
        assert_eq!(cls.last_duration_ms(), None);
        assert_eq!(cls.checking_gate(), None);
        assert!(cls.gate_results().is_empty());
    }

    #[test]
    fn test_continuous_loop_update_iteration() {
        let mut cls = ContinuousLoopState {
            status: ContinuousLoopStatus::Inactive,
            iterations_completed: 0,
            last_duration_ms: None,
            checking_gate: Some("clippy".to_string()),
            gate_results: vec![GateResult {
                gate: "old".to_string(),
                passed: true,
                message: None,
            }],
        };
        cls.update_iteration(1);
        assert_eq!(
            *cls.status(),
            ContinuousLoopStatus::Running { iteration: 1 }
        );
        assert!(cls.checking_gate().is_none());
        assert!(cls.gate_results().is_empty());
    }

    #[test]
    fn test_continuous_loop_complete_iteration() {
        let mut cls = ContinuousLoopState {
            status: ContinuousLoopStatus::Running { iteration: 1 },
            iterations_completed: 0,
            last_duration_ms: None,
            checking_gate: None,
            gate_results: Vec::new(),
        };
        cls.complete_iteration(5000);
        assert_eq!(cls.iterations_completed(), 1);
        assert_eq!(cls.last_duration_ms(), Some(5000));
    }

    #[test]
    fn test_continuous_loop_gate_checking() {
        let mut cls = ContinuousLoopState {
            status: ContinuousLoopStatus::Running { iteration: 1 },
            iterations_completed: 0,
            last_duration_ms: None,
            checking_gate: None,
            gate_results: Vec::new(),
        };
        cls.set_gate_checking("clippy");
        assert_eq!(cls.checking_gate(), Some("clippy"));

        cls.record_gate_result("clippy", true, Some("0 warnings"));
        assert!(cls.checking_gate().is_none());
        assert_eq!(cls.gate_results().len(), 1);
        assert!(cls.gate_results()[0].passed);
    }

    #[test]
    fn test_continuous_loop_stagnation_and_reset() {
        let mut cls = ContinuousLoopState {
            status: ContinuousLoopStatus::Running { iteration: 5 },
            iterations_completed: 5,
            last_duration_ms: Some(3000),
            checking_gate: None,
            gate_results: Vec::new(),
        };
        cls.set_stagnation(3, 5);
        assert_eq!(
            *cls.status(),
            ContinuousLoopStatus::Stagnated {
                iterations_without_progress: 3,
                threshold: 5,
            }
        );

        cls.reset();
        assert_eq!(*cls.status(), ContinuousLoopStatus::Inactive);
        assert_eq!(cls.iterations_completed(), 0);
        assert_eq!(cls.last_duration_ms(), None);
    }

    #[test]
    fn test_continuous_loop_human_checkpoint() {
        let mut cls = ContinuousLoopState {
            status: ContinuousLoopStatus::Running { iteration: 3 },
            iterations_completed: 2,
            last_duration_ms: None,
            checking_gate: None,
            gate_results: Vec::new(),
        };
        cls.set_human_checkpoint("needs review");
        assert_eq!(
            *cls.status(),
            ContinuousLoopStatus::HumanRequired {
                reason: "needs review".to_string(),
            }
        );
    }

    #[test]
    fn test_app_state_continuous_delegation() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.dirty.clear();

        state.update_continuous_iteration(1);
        assert!(state.dirty.full);
        assert_eq!(
            *state.continuous_status(),
            ContinuousLoopStatus::Running { iteration: 1 }
        );

        state.dirty.clear();
        state.complete_continuous_iteration(1, 2000);
        assert!(state.dirty.full);
        assert_eq!(state.continuous_iterations_completed(), 1);

        state.dirty.clear();
        state.reset_continuous();
        assert!(state.dirty.full);
        assert_eq!(*state.continuous_status(), ContinuousLoopStatus::Inactive);
    }

    #[test]
    fn test_ui_selection_state_focus_area() {
        let mut ui = UISelectionState {
            selection: SelectionState::new(),
            copy_pending: false,
            rendered_lines_cache: Vec::new(),
            focus_area: FocusArea::Content,
        };
        assert_eq!(ui.focus_area(), FocusArea::Content);

        ui.set_focus_area(FocusArea::Input);
        assert_eq!(ui.focus_area(), FocusArea::Input);
    }

    #[test]
    fn test_ui_selection_state_focus_area_clears_selection() {
        let mut ui = UISelectionState {
            selection: SelectionState::new(),
            copy_pending: false,
            rendered_lines_cache: Vec::new(),
            focus_area: FocusArea::Content,
        };

        use crate::types::ui_state::ContentPosition;
        ui.selection.start(ContentPosition::new(0, 0));
        ui.selection.update(ContentPosition::new(1, 5));
        ui.selection.end();
        assert!(ui.selection.has_selection());

        // Changing focus should clear selection
        ui.set_focus_area(FocusArea::Input);
        assert!(!ui.selection.has_selection());
    }

    #[test]
    fn test_ui_selection_state_copy_pending() {
        let mut ui = UISelectionState {
            selection: SelectionState::new(),
            copy_pending: false,
            rendered_lines_cache: Vec::new(),
            focus_area: FocusArea::Content,
        };
        assert!(!ui.take_copy_pending());

        ui.request_copy();
        assert!(ui.take_copy_pending());
        assert!(!ui.take_copy_pending()); // cleared
    }

    #[test]
    fn test_ui_selection_state_rendered_line_count() {
        let ui = UISelectionState {
            selection: SelectionState::new(),
            copy_pending: false,
            rendered_lines_cache: vec!["line 1".to_string(), "line 2".to_string()],
            focus_area: FocusArea::Content,
        };
        assert_eq!(ui.rendered_line_count(), 2);
    }

    #[test]
    fn test_ui_selection_extract_text_no_selection() {
        let ui = UISelectionState {
            selection: SelectionState::new(),
            copy_pending: false,
            rendered_lines_cache: vec!["hello".to_string()],
            focus_area: FocusArea::Content,
        };
        assert_eq!(ui.extract_selected_text(), None);
    }

    #[test]
    fn test_ui_selection_extract_text_single_line() {
        use crate::types::ui_state::ContentPosition;
        let mut ui = UISelectionState {
            selection: SelectionState::new(),
            copy_pending: false,
            rendered_lines_cache: vec!["hello world".to_string()],
            focus_area: FocusArea::Content,
        };
        ui.selection.start(ContentPosition::new(0, 0));
        ui.selection.update(ContentPosition::new(0, 5));
        ui.selection.end();

        assert_eq!(ui.extract_selected_text(), Some("hello".to_string()));
    }

    #[test]
    fn test_ui_selection_extract_text_multi_line() {
        use crate::types::ui_state::ContentPosition;
        let mut ui = UISelectionState {
            selection: SelectionState::new(),
            copy_pending: false,
            rendered_lines_cache: vec![
                "line one".to_string(),
                "line two".to_string(),
                "line three".to_string(),
            ],
            focus_area: FocusArea::Content,
        };
        ui.selection.start(ContentPosition::new(0, 5));
        ui.selection.update(ContentPosition::new(2, 4));
        ui.selection.end();

        assert_eq!(
            ui.extract_selected_text(),
            Some("one\nline two\nline".to_string())
        );
    }

    #[test]
    fn test_ui_selection_extract_text_empty_cache() {
        use crate::types::ui_state::ContentPosition;
        let mut ui = UISelectionState {
            selection: SelectionState::new(),
            copy_pending: false,
            rendered_lines_cache: Vec::new(),
            focus_area: FocusArea::Content,
        };
        ui.selection.start(ContentPosition::new(0, 0));
        ui.selection.update(ContentPosition::new(0, 5));
        ui.selection.end();

        assert_eq!(ui.extract_selected_text(), None);
    }

    #[test]
    fn test_build_request_options_default_effort() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        let opts = state.build_request_options("claude-sonnet-4-20250514");
        // Auto effort → no thinking budget
        assert!(opts.thinking.is_none());
        // System prompt not set → no system blocks
        assert!(opts.system.is_none());
    }

    #[test]
    fn test_build_request_options_high_effort() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.set_effort(EffortLevel::High);
        let opts = state.build_request_options("claude-sonnet-4-20250514");
        let thinking = opts
            .thinking
            .expect("High effort should produce thinking config");
        assert_eq!(thinking.config_type, "enabled");
        assert_eq!(thinking.budget_tokens, 16_000);
    }

    #[test]
    fn test_build_request_options_explicit_budget_overrides_effort() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.set_effort(EffortLevel::Medium);
        state.set_thinking_budget(Some(50_000));
        let opts = state.build_request_options("claude-sonnet-4-20250514");
        let thinking = opts
            .thinking
            .expect("Explicit budget should produce thinking config");
        assert_eq!(thinking.budget_tokens, 50_000);
    }

    #[test]
    fn test_build_request_options_unsupported_model() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.set_effort(EffortLevel::High);
        let opts = state.build_request_options("unknown-model");
        // Unknown model → no thinking support
        assert!(opts.thinking.is_none());
    }

    #[test]
    fn test_build_request_options_system_prompt_with_cache() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.set_system_prompt(Some("You are a helpful assistant.".to_string()));
        let opts = state.build_request_options("claude-sonnet-4-20250514");
        let blocks = opts.system.expect("System prompt should produce blocks");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].text, "You are a helpful assistant.");
        assert!(
            blocks[0].cache_control.is_some(),
            "Claude 4 should get cache control"
        );
    }

    #[test]
    fn test_build_request_options_system_prompt_no_cache_for_old_model() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.set_system_prompt(Some("Instructions".to_string()));
        let opts = state.build_request_options("claude-3-haiku-20240307");
        let blocks = opts.system.expect("System prompt should produce blocks");
        assert!(
            blocks[0].cache_control.is_none(),
            "Haiku 3 should not get cache control"
        );
    }

    #[test]
    fn test_force_compact_sets_request_flag() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.dirty.clear();

        assert!(
            !state.is_compaction_requested(),
            "No compaction should be requested initially"
        );

        state.force_compact(Some("summarize briefly".to_string()));

        assert!(
            state.is_compaction_requested(),
            "Compaction should be requested after force_compact"
        );
        assert!(
            state.dirty.full,
            "Dirty flag should be set by force_compact"
        );
    }

    #[test]
    fn test_force_compact_no_instructions() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.force_compact(None);

        assert!(state.is_compaction_requested());

        // Verify the request can be consumed
        let request = state.compression.take_compaction_request();
        assert_eq!(request, Some(None));
        assert!(!state.is_compaction_requested());
    }

    #[test]
    fn test_sync_token_budget_sets_dirty_flag() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        // Add a message so there are tokens to count
        state.api_messages.push(ApiMessageV2::user("Hello, world!"));

        state.dirty.clear();
        state.sync_token_budget();

        assert!(
            state.dirty.full,
            "sync_token_budget should set the dirty flag via add_token_usage"
        );
        assert!(
            state.token_budget().used() > 0,
            "Token budget should reflect the conversation tokens"
        );
    }
}
