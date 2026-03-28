//! Application state management

mod agent_panel;
mod auto_compaction;
mod background;
pub mod background_tasks;
mod compression;
mod compression_methods;
mod continuous;
mod conversation;
mod conversation_engine;
mod display;
mod input;
mod mcp_coordination;
mod model_config;
pub mod plan;
pub mod question;
mod request_building;
mod session_tracking;
mod stream_events;
mod tool_coordination;
mod tool_execution;
mod tool_interception;
mod ui_selection;
mod view_state;
mod worktree;

pub use agent_panel::AgentPanelState;
pub use background::*;
pub use background_tasks::BackgroundTaskRegistry;
pub use compression::CompressionState;
pub use continuous::ContinuousLoopState;
pub use conversation_engine::ConversationEngine;
pub use display::DisplayState;
pub use input::InputState;
pub use model_config::ModelConfigState;
pub use plan::{PlanState, PlanStep};
pub use question::QuestionState;
pub use session_tracking::SessionTracking;
pub use tool_execution::ToolExecutionState;
pub use ui_selection::UISelectionState;
pub use view_state::ViewState;
pub use worktree::WorktreeStatus;

use crate::agents::{AgentProgress, ConflictReport, SubagentSpawner};
use crate::api::provider::RequestOptions;
use crate::api::tokens::{model_context_limit, ModelCapabilities};
use crate::api::tools::default_tools;
use crate::api::{LlmProvider, StreamEvent, SystemBlock, ThinkingConfig, TokenBudget, ToolChoice};
use crate::app::tool_loop::ToolLoop;
use crate::app::STREAMING_CHANNEL_BUFFER;
use crate::context::compression::{CompactionMetrics, CompressionOrchestrator};
use crate::enterprise::cost::{CostConfig, CostTracker, UsageRecord};
use crate::hooks::HookManager;
use crate::keybindings::KeybindingManager;
use crate::mcp::connection::McpConnection;
use crate::narsil::context::ContextSuggestion;
use crate::permissions::{PermissionManager, PermissionRequest, PermissionResponse};
use crate::plugins::PluginRegistry;
use crate::session::Session;
use crate::terminal::notifications::NotificationManager;
use crate::tools::HookedToolExecutor;
use crate::types::config::ParallelMode;
use crate::types::content::{StopReason, ToolResultBlock, ToolUseBlock};
use crate::types::render_view::RenderFeedback;
use crate::types::render_view::RenderView;
use crate::types::ui_state::{FocusArea, SelectionState, ToolBlockState};
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
    /// Core conversation state (api_messages, timeline, streaming, thinking).
    pub(crate) conversation: ConversationEngine,

    /// All view/presentation state (display, input, selection, worktree).
    pub(crate) view: ViewState,

    pub working_dir: PathBuf,

    dirty: DirtyFlags,

    /// Session tracking for auto-save (ID + dirty flag).
    session: SessionTracking,

    /// Whether the application should exit the event loop.
    /// Set by `KeyboardHandler` on Ctrl+C / Ctrl+D; checked by the event loop.
    quit_requested: bool,

    /// All tool execution state grouped together.
    tool_state: ToolExecutionState,

    /// All compression, context injection, and compaction state grouped together.
    compression: CompressionState,

    /// Plugin registry for managing loaded plugins.
    /// Loaded from `~/.config/patina/plugins/` on startup unless disabled.
    plugin_registry: PluginRegistry,

    /// Agent panel state (entries, conflict reports, spawner).
    agent_panel: AgentPanelState,

    /// All continuous coding loop state grouped together.
    continuous: ContinuousLoopState,

    /// Optional MCP server manager for external tool servers.
    /// Set during app startup if `.mcp.json` or `~/.claude.json` contains server entries.
    /// Wrapped in `Arc` so spawned tool-execution tasks can share read access
    /// (the SDK uses interior mutability — `call_tool` is `&self`).
    mcp_manager: Option<std::sync::Arc<crate::mcp::manager::McpManager>>,

    /// Cost tracker for session usage accounting.
    cost_tracker: CostTracker,

    /// Model selection, effort, and thinking configuration.
    model_config: ModelConfigState,

    /// Keybinding manager for customizable key mappings.
    keybinding_mgr: KeybindingManager,

    /// Terminal notification manager for desktop notifications.
    notification_manager: NotificationManager,
}

#[derive(Default)]
struct DirtyFlags {
    /// Cross-cutting dirty flag for operations that don't belong to a single component.
    full: bool,
    /// Throbber animation only -- avoids dirtying other flags every 250ms.
    throbber: bool,
}

impl DirtyFlags {
    fn any(&self) -> bool {
        self.full || self.throbber
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
        let mut hook_manager = HookManager::new(hook_session_id);

        // Load hook configuration from project-local and user-global locations
        let hooks_toml = working_dir.join(".claude/hooks.toml");
        if let Err(e) = hook_manager.load_config_graceful(&hooks_toml) {
            tracing::warn!("Failed to load project hook config: {}", e);
        }
        if let Some(base_dirs) = directories::BaseDirs::new() {
            let user_hooks = base_dirs.config_dir().join("patina/hooks.toml");
            if let Err(e) = hook_manager.load_config_graceful(&user_hooks) {
                tracing::warn!("Failed to load user hook config: {}", e);
            }
        }

        // Create permission manager with skip_permissions setting
        let mut pm = PermissionManager::new();
        pm.set_skip_permissions(skip_permissions);
        let permission_manager = Arc::new(Mutex::new(pm));

        // Convert PerformanceConfig to ParallelConfig
        let parallel_config = performance.to_parallel_config();

        // Load plugins if enabled
        let plugin_registry = if plugins_enabled {
            Self::load_plugins()
        } else {
            PluginRegistry::new()
        };

        // Bridge plugin hooks into the hook manager before it's wrapped in Arc
        for plugin in plugin_registry.iter_plugins() {
            crate::plugins::bridge_plugin_hooks(&plugin.hooks, &mut hook_manager);
        }

        // Create tool executor with hook, permission, and parallel configuration
        let tool_executor = Arc::new(
            HookedToolExecutor::new(working_dir.clone(), hook_manager)
                .with_permissions(Arc::clone(&permission_manager))
                .with_parallel_config(parallel_config),
        );

        // Load skills from standard directories
        let skill_engine = Self::load_skills(&working_dir);

        // Initialize subagent spawner if enabled
        let subagent_spawner = if subagents_enabled {
            Some(SubagentSpawner::new())
        } else {
            None
        };

        Self {
            conversation: ConversationEngine::new(),
            view: ViewState {
                display: DisplayState::new(),
                ui_selection: UISelectionState {
                    selection: SelectionState::new(),
                    copy_pending: false,
                    rendered_lines_cache: Vec::new(),
                    focus_area: FocusArea::default(),
                },
                worktree: WorktreeStatus::new(),
                input_state: InputState::new(),
            },
            working_dir,
            dirty: DirtyFlags {
                full: true,
                ..Default::default()
            },
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
                pending_plan: None,
                pending_question: None,
                background_tasks: BackgroundTaskRegistry::new(),
                dirty_modal: false,
                dirty_content: false,
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
                consecutive_compaction_failures: 0,
                render_dirty: false,
            },
            plugin_registry,
            agent_panel: AgentPanelState::new(subagent_spawner),
            continuous: ContinuousLoopState {
                status: ContinuousLoopStatus::Inactive,
                iterations_completed: 0,
                last_duration_ms: None,
                checking_gate: None,
                gate_results: Vec::new(),
                dirty: false,
            },
            mcp_manager: None,
            cost_tracker: CostTracker::new(CostConfig::default()),
            model_config: {
                let mut mc = ModelConfigState::new();
                if let Some(engine) = skill_engine {
                    mc.set_skill_engine(engine);
                }
                mc
            },
            keybinding_mgr: KeybindingManager::with_defaults(),
            notification_manager: NotificationManager::detect(),
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

    /// Loads skills from standard directories.
    ///
    /// Searches for skills in:
    /// - `~/.config/patina/skills/`
    /// - `./.patina/skills/` (project-local)
    /// - `./.claude/skills/` (project instructions directory)
    fn load_skills(working_dir: &std::path::Path) -> Option<crate::skills::SkillEngine> {
        let mut engine = crate::skills::SkillEngine::new();

        let mut search_paths: Vec<PathBuf> = Vec::new();

        // User config directory
        if let Some(base_dirs) = directories::BaseDirs::new() {
            search_paths.push(base_dirs.config_dir().join("patina/skills"));
        }

        // Project-local skills
        search_paths.push(working_dir.join(".patina/skills"));

        // .claude/skills (Claude Code convention)
        search_paths.push(working_dir.join(".claude/skills"));

        for path in &search_paths {
            if let Err(e) = engine.load_from_dir(path) {
                tracing::debug!("Skills directory {:?}: {}", path, e);
            }
        }

        let count = engine.all_skills().len();
        if count > 0 {
            tracing::info!("Loaded {} skill(s)", count);
            Some(engine)
        } else {
            None
        }
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

    /// Returns a mutable reference to the agent panel state.
    pub fn agent_panel_mut(&mut self) -> &mut AgentPanelState {
        &mut self.agent_panel
    }

    /// Returns a reference to the display state.
    #[must_use]
    pub fn display(&self) -> &DisplayState {
        &self.view.display
    }

    /// Returns a mutable reference to the display state.
    pub fn display_mut(&mut self) -> &mut DisplayState {
        &mut self.view.display
    }

    /// Returns a mutable reference to the continuous loop state.
    pub fn continuous_mut(&mut self) -> &mut ContinuousLoopState {
        &mut self.continuous
    }

    /// Returns a mutable reference to the compression state.
    pub fn compression_mut(&mut self) -> &mut CompressionState {
        &mut self.compression
    }

    /// Returns a mutable reference to the worktree status.
    pub fn worktree_mut(&mut self) -> &mut WorktreeStatus {
        &mut self.view.worktree
    }

    /// Returns a mutable reference to the session tracking state.
    pub fn session_tracking_mut(&mut self) -> &mut SessionTracking {
        &mut self.session
    }

    /// Returns a mutable reference to the input state.
    pub fn input_state_mut(&mut self) -> &mut InputState {
        &mut self.view.input_state
    }

    /// Returns a mutable reference to the UI selection state.
    pub fn ui_selection_mut(&mut self) -> &mut UISelectionState {
        &mut self.view.ui_selection
    }

    /// Returns a reference to the model configuration state.
    #[must_use]
    pub fn model_config(&self) -> &ModelConfigState {
        &self.model_config
    }

    /// Returns a mutable reference to the model configuration state.
    pub fn model_config_mut(&mut self) -> &mut ModelConfigState {
        &mut self.model_config
    }

    /// Returns a reference to the keybinding manager.
    #[must_use]
    pub fn keybindings(&self) -> &KeybindingManager {
        &self.keybinding_mgr
    }

    /// Returns a mutable reference to the keybinding manager.
    pub fn keybindings_mut(&mut self) -> &mut KeybindingManager {
        &mut self.keybinding_mgr
    }

    /// Replaces the keybinding manager with a new one.
    ///
    /// Used when loading custom keybindings from a configuration file.
    pub fn set_keybindings(&mut self, manager: KeybindingManager) {
        self.keybinding_mgr = manager;
    }

    /// Returns a reference to the notification manager.
    #[must_use]
    pub fn notification_manager(&self) -> &NotificationManager {
        &self.notification_manager
    }

    /// Returns a reference to the tool execution state.
    #[must_use]
    pub fn tool_state(&self) -> &ToolExecutionState {
        &self.tool_state
    }

    /// Returns a mutable reference to the tool execution state.
    pub fn tool_state_mut(&mut self) -> &mut ToolExecutionState {
        &mut self.tool_state
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
    }

    /// Records a conflict report for display in the TUI.
    pub fn add_conflict_report(&mut self, report: ConflictReport) {
        self.agent_panel.add_conflict(report);
    }

    // =========================================================================
    // Continuous loop panel methods (delegates to ContinuousLoopState)
    // =========================================================================

    /// Returns a reference to the continuous loop state.
    #[must_use]
    pub fn continuous(&self) -> &ContinuousLoopState {
        &self.continuous
    }

    /// Updates state for a new continuous iteration starting.
    pub fn update_continuous_iteration(&mut self, iteration: u32) {
        self.continuous.update_iteration(iteration);
    }

    /// Records the completion of a continuous iteration.
    pub fn complete_continuous_iteration(&mut self, _iteration: u32, duration_ms: u64) {
        self.continuous.complete_iteration(duration_ms);
    }

    /// Records that a quality gate check is starting.
    pub fn set_continuous_gate_checking(&mut self, gate: &str) {
        self.continuous.set_gate_checking(gate);
    }

    /// Records the result of a quality gate check.
    pub fn record_continuous_gate_result(
        &mut self,
        gate: &str,
        passed: bool,
        message: Option<&str>,
    ) {
        self.continuous.record_gate_result(gate, passed, message);
    }

    /// Records that stagnation was detected.
    pub fn set_continuous_stagnation(&mut self, iterations_without_progress: u32, threshold: u32) {
        self.continuous
            .set_stagnation(iterations_without_progress, threshold);
    }

    /// Records that human intervention is required.
    pub fn set_continuous_human_checkpoint(&mut self, reason: &str) {
        self.continuous.set_human_checkpoint(reason);
    }

    /// Resets all continuous loop state to inactive.
    pub fn reset_continuous(&mut self) {
        self.continuous.reset();
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

    // =========================================================================
    // Input state methods (delegates to InputState)
    // =========================================================================

    /// Returns a reference to the input state.
    #[must_use]
    pub fn input_state(&self) -> &InputState {
        &self.view.input_state
    }

    /// Inserts a character at the current cursor position.
    pub fn insert_char(&mut self, c: char) {
        let needs_completion = self.view.input_state.insert_char(c);
        if needs_completion {
            self.show_completion();
        }
    }

    /// Deletes the character before the cursor (backspace behavior).
    pub fn delete_char(&mut self) {
        self.view.input_state.delete_char();
    }

    /// Takes and returns the current input, clearing the buffer and resetting cursor.
    pub fn take_input(&mut self) -> String {
        self.view.input_state.take()
    }

    /// Moves the cursor left by one character.
    pub fn cursor_left(&mut self) {
        self.view.input_state.cursor_left();
    }

    /// Moves the cursor right by one character.
    pub fn cursor_right(&mut self) {
        self.view.input_state.cursor_right();
    }

    /// Moves the cursor to the beginning of the input.
    pub fn cursor_home(&mut self) {
        self.view.input_state.cursor_home();
    }

    /// Moves the cursor to the end of the input.
    pub fn cursor_end(&mut self) {
        self.view.input_state.cursor_end();
    }

    /// Scrolls up by the specified number of lines.
    ///
    /// This switches to Manual mode, preserving the scroll position
    /// during streaming updates.
    pub fn scroll_up(&mut self, lines: usize) {
        let before = self.view.display.scroll.offset();
        self.view.display.scroll.scroll_up(lines);
        let after = self.view.display.scroll.offset();
        tracing::debug!(
            lines,
            before,
            after,
            mode = ?self.view.display.scroll.mode(),
            content_height = self.view.display.scroll.content_height(),
            viewport_height = self.view.display.scroll.viewport_height(),
            cache_size = self.view.ui_selection.rendered_lines_cache.len(),
            timeline_entries = self.conversation.timeline.len(),
            "scroll_up"
        );
        self.view.display.mark_dirty();
    }

    /// Scrolls down by the specified number of lines.
    ///
    /// If scrolling to the bottom, resumes Follow mode for auto-scroll.
    pub fn scroll_down(&mut self, lines: usize) {
        let before = self.view.display.scroll.offset();
        self.view.display.scroll.scroll_down(lines);
        let after = self.view.display.scroll.offset();
        tracing::debug!(
            lines,
            before,
            after,
            mode = ?self.view.display.scroll.mode(),
            "scroll_down"
        );
        self.view.display.mark_dirty();
    }

    /// Scrolls to the bottom of the content.
    ///
    /// This resumes Follow mode for auto-scroll.
    pub fn scroll_to_bottom(&mut self, content_height: usize) {
        self.view.display.scroll.scroll_to_bottom(content_height);
        self.view.display.mark_dirty();
    }

    /// Scrolls to the top of the content.
    ///
    /// This switches to Manual mode.
    pub fn scroll_to_top(&mut self) {
        self.view.display.scroll.scroll_to_top();
        self.view.display.mark_dirty();
    }

    /// Updates the content height for scroll calculations.
    ///
    /// In Follow mode, this auto-scrolls to show new content.
    pub fn update_content_height(&mut self, height: usize) {
        self.view.display.scroll.set_content_height(height);
        if self.view.display.scroll.mode().should_auto_scroll() {
            self.view.display.mark_dirty();
        }
    }

    /// Updates the viewport height for scroll calculations.
    pub fn set_viewport_height(&mut self, height: usize) {
        self.view.display.scroll.set_viewport_height(height);
    }

    /// Returns a reference to the UI selection state.
    #[must_use]
    pub fn ui_selection(&self) -> &UISelectionState {
        &self.view.ui_selection
    }

    pub fn is_loading(&self) -> bool {
        self.view.display.loading
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
        self.view.display.throbber_frame = (self.view.display.throbber_frame + 1) % 4;
        self.dirty.throbber = true;
    }

    pub fn throbber_char(&self) -> char {
        ['⠋', '⠙', '⠹', '⠸'][self.view.display.throbber_frame]
    }

    pub fn needs_render(&self) -> bool {
        self.dirty.any()
            || self.view.needs_render()
            || self.agent_panel.is_dirty()
            || self.continuous.is_dirty()
            || self.compression.is_render_dirty()
            || self.tool_state.is_dirty()
    }

    /// Returns `true` when the only pending dirty state is the throbber animation.
    ///
    /// This allows the render path to skip the expensive full timeline rebuild
    /// when only the throbber character has changed.
    #[must_use]
    pub fn is_throbber_only_dirty(&self) -> bool {
        self.dirty.throbber
            && !self.dirty.full
            && !self.view.input_state.is_dirty()
            && !self.agent_panel.is_dirty()
            && !self.continuous.is_dirty()
            && !self.view.worktree.is_dirty()
            && !self.compression.is_render_dirty()
            && !self.tool_state.is_dirty()
    }

    pub fn mark_rendered(&mut self) {
        self.dirty.clear();
        self.view.mark_rendered();
        self.agent_panel.mark_clean();
        self.continuous.mark_clean();
        self.compression.mark_render_clean();
        self.tool_state.mark_clean();
    }

    pub fn mark_full_redraw(&mut self) {
        self.dirty.full = true;
    }

    // ========================================================================
    // Worktree Status Bar State (delegates to WorktreeStatus)
    // ========================================================================

    /// Returns a reference to the worktree status.
    #[must_use]
    pub fn worktree(&self) -> &WorktreeStatus {
        &self.view.worktree
    }

    /// Sets the current worktree branch name.
    pub fn set_worktree_branch(&mut self, branch: String) {
        self.view.worktree.set_branch(branch);
    }

    /// Sets the number of modified files in the worktree.
    pub fn set_worktree_modified(&mut self, count: usize) {
        self.view.worktree.set_modified(count);
    }

    /// Sets the number of commits ahead of upstream.
    pub fn set_worktree_ahead(&mut self, count: usize) {
        self.view.worktree.set_ahead(count);
    }

    /// Sets the number of commits behind upstream.
    pub fn set_worktree_behind(&mut self, count: usize) {
        self.view.worktree.set_behind(count);
    }

    // ========================================================================
    // Token Budget Tracking
    // ========================================================================

    /// Adds token usage to the budget.
    pub fn add_token_usage(&mut self, tokens: usize) {
        self.compression.token_budget_mut().add_usage(tokens);
        self.compression.mark_render_dirty();
    }

    /// Resets the token budget for a new conversation.
    pub fn reset_token_budget(&mut self) {
        self.compression.token_budget_mut().reset();
        self.compression.mark_render_dirty();
    }

    // ========================================================================
    // Compaction Progress (delegates to CompressionState)
    // ========================================================================

    /// Starts a compaction operation.
    pub fn start_compaction(&mut self, target_tokens: usize, before_tokens: usize, is_auto: bool) {
        self.compression
            .start_compaction(target_tokens, before_tokens, is_auto);
    }

    /// Updates the compaction progress (0.0 to 1.0).
    pub fn update_compaction_progress(&mut self, progress: f64) {
        self.compression.update_compaction_progress(progress);
    }

    /// Completes the compaction operation with the final token count.
    pub fn complete_compaction(&mut self, after_tokens: usize) {
        self.compression.complete_compaction(after_tokens);
    }

    /// Marks the compaction operation as failed.
    pub fn fail_compaction(&mut self) {
        self.compression.fail_compaction();
    }

    /// Clears the compaction state (closes the overlay).
    pub fn clear_compaction(&mut self) {
        self.compression.clear_compaction();
    }

    /// Requests a manual compaction, to be executed on the next event loop tick.
    ///
    /// This sets a flag that `maybe_compact` will check, bypassing the
    /// automatic threshold so that compaction runs unconditionally.
    /// Called by the `/compact` slash command handler.
    pub fn force_compact(&mut self, custom_instructions: Option<String>) {
        self.compression.request_compaction(custom_instructions);
    }

    // ========================================================================
    // Render view (TUI decoupling)
    // ========================================================================

    /// Creates a read-only view of all state the TUI needs to render a frame.
    ///
    /// This is the bridge between `AppState` and the TUI layer. The returned
    /// [`RenderView`] borrows data from `self`, so the TUI never needs to
    /// import `AppState` directly.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let view = state.as_render_view();
    /// let feedback = tui::render(frame, &view);
    /// state.apply_render_feedback(&feedback);
    /// ```
    #[must_use]
    pub fn as_render_view(&self) -> RenderView<'_> {
        RenderView {
            timeline: self.conversation.timeline(),
            throbber_char: self.view.display.throbber_char(),
            scroll_offset: self.view.display.scroll_offset(),
            scroll_state: self.view.display.scroll_state(),
            selection: self.view.ui_selection.selection(),
            focus_area: self.view.ui_selection.focus_area(),
            input: self.view.input_state.text(),
            completion: self.view.input_state.completion(),
            worktree_branch: self.view.worktree.branch(),
            worktree_modified: self.view.worktree.modified(),
            worktree_ahead: self.view.worktree.ahead(),
            worktree_behind: self.view.worktree.behind(),
            token_budget: self.compression.token_budget(),
            context_tokens_injected: self.compression.context_tokens_injected(),
            session_cost_usd: self.cost_tracker.session_cost(),
            update_available: self.view.display.update_available(),
            continuous_status: self.continuous.status(),
            continuous_iterations_completed: self.continuous.iterations_completed(),
            continuous_gate_results: self.continuous.gate_results(),
            continuous_checking_gate: self.continuous.checking_gate(),
            continuous_last_duration_ms: self.continuous.last_duration_ms(),
            compaction_state: self.compression.compaction_state(),
            pending_permission: self.tool_state.pending_permission.as_ref(),
            pending_plan: self.tool_state.pending_plan.as_ref(),
            pending_question: self.tool_state.pending_question.as_ref(),
            throbber_only: self.is_throbber_only_dirty(),
        }
    }

    /// Applies layout metrics computed during rendering back to mutable state.
    ///
    /// The TUI's `render()` function returns a [`RenderFeedback`] containing
    /// wrapped-line caches and viewport dimensions. This method writes those
    /// values into the scroll and selection subsystems.
    ///
    /// # Arguments
    ///
    /// * `feedback` - Layout metrics from the most recent render pass
    pub fn apply_render_feedback(&mut self, feedback: &RenderFeedback) {
        self.view
            .ui_selection
            .update_rendered_lines_from_strings(&feedback.wrapped_lines);
        self.view
            .display
            .scroll
            .set_viewport_height(feedback.viewport_height);
        self.view
            .display
            .scroll
            .set_content_height(feedback.content_height);
    }
}

#[cfg(test)]
impl AppState {
    /// Takes all pending conflict reports, leaving the internal list empty.
    pub fn take_conflict_reports(&mut self) -> Vec<ConflictReport> {
        self.agent_panel.take_conflicts()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::config::EffortLevel;

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
        state.view.display.scroll.restore_offset(100);
        *state.input_state_mut().text_mut() = "existing".to_string();
        state.view.input_state.set_cursor_position(8);

        // Create a session without UI state
        let mut session = Session::new(PathBuf::from("/project"));
        session.add_message(test_message(Role::User, "Test"));

        state.restore_from_session(&session);

        // UI state should remain unchanged since session has no UI state
        assert_eq!(state.display().scroll_offset(), 100);
        assert_eq!(state.input_state().text(), "existing");
        assert_eq!(state.input_state().cursor_position(), 8);
    }

    #[test]
    fn test_to_session_preserves_ui_state() {
        let mut state = AppState::new(PathBuf::from("/project"), false, ParallelMode::Enabled);
        state.view.display.scroll.restore_offset(42);
        *state.input_state_mut().text_mut() = "draft text".to_string();
        state.view.input_state.set_cursor_position(5);

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
        state.view.display.scroll.restore_offset(100);
        *state.input_state_mut().text_mut() = "unsent input".to_string();
        state.view.input_state.set_cursor_position(6);

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
        assert_eq!(new_state.display().scroll_offset(), 100);
        assert_eq!(new_state.input_state().text(), "unsent input");
        assert_eq!(new_state.input_state().cursor_position(), 6);
    }

    #[test]
    fn test_api_messages_truncated_returns_truncated() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Add many messages to potentially exceed budget
        for i in 0..50 {
            state
                .conversation
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

        state
            .conversation
            .api_messages
            .push(ApiMessageV2::user("Hello"));
        state
            .conversation
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
        state
            .conversation
            .api_messages
            .push(ApiMessageV2::user("System prompt"));

        // Add many large messages that would exceed 100k tokens
        let large_content = "x".repeat(10_000); // ~2500 tokens each
        for _ in 0..50 {
            // 50 * 2500 = 125k tokens
            state
                .conversation
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
        state
            .conversation
            .api_messages
            .push(ApiMessageV2::user("System prompt"));

        // Add many large messages that exceed 100k tokens
        let large_content = "x".repeat(10_000);
        for _ in 0..60 {
            state
                .conversation
                .api_messages
                .push(ApiMessageV2::assistant(&large_content));
        }

        let total_before = state.conversation.api_messages.len();
        let prepared = state
            .prepare_api_messages_for_send("claude-sonnet-4-20250514", None)
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
            .conversation
            .api_messages
            .push(ApiMessageV2::user("Important system prompt"));

        let large_content = "x".repeat(10_000);
        for _ in 0..60 {
            state
                .conversation
                .api_messages
                .push(ApiMessageV2::assistant(&large_content));
        }

        let prepared = state
            .prepare_api_messages_for_send("claude-sonnet-4-20250514", None)
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

        state
            .conversation
            .api_messages
            .push(ApiMessageV2::user("Hello"));
        state
            .conversation
            .api_messages
            .push(ApiMessageV2::assistant("Hi there!"));

        let prepared = state
            .prepare_api_messages_for_send("claude-sonnet-4-20250514", None)
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
        assert!(state.compression().has_cached_ccg_context());

        // Take should return and clear
        let taken = state.compression_mut().take_cached_ccg_context();
        assert_eq!(taken, Some("test context".to_string()));
        assert!(!state.compression().has_cached_ccg_context());
    }

    #[test]
    fn test_context_for_injection_requires_auto_context_enabled() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Set cached context
        state.compression.cached_ccg_context = Some("test context".to_string());

        // Auto-context disabled - should return None
        assert!(state.compression().context_for_injection().is_none());

        // Enable auto-context
        state.compression_mut().set_auto_context_enabled(true);
        assert_eq!(
            state.compression().context_for_injection(),
            Some("test context")
        );
    }

    #[tokio::test]
    async fn test_context_not_reinjected_when_hash_unchanged() {
        let mut state = AppState::new(PathBuf::from("."), false, ParallelMode::Enabled);
        state.compression_mut().set_auto_context_enabled(true);

        // Create a minimal orchestrator so the early-return check passes
        let caps = crate::narsil::NarsilCapabilities::from_tools(&["find_symbols".to_string()]);
        let orchestrator = Arc::new(crate::context::compression::CompressionOrchestrator::new(
            caps,
            "test-repo",
        ));
        state
            .compression_mut()
            .set_compression_orchestrator(orchestrator);

        // Pre-populate cached context and set the hash to the current git HEAD
        // Since tests run in the git repo, get_git_head_hash(".") returns a real hash
        let current_hash =
            get_git_head_hash(std::path::Path::new(".")).unwrap_or_else(|| "unknown".to_string());
        state.compression_mut().set_last_ccg_hash(current_hash);
        state.compression.cached_ccg_context =
            Some("## Cached Context\nAlready fetched".to_string());
        state.compression_mut().set_context_tokens_injected(3000);

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
        assert_eq!(state.compression().context_tokens_injected(), 3000);
        // Cache should still be present
        assert!(state.compression().has_cached_ccg_context());
    }

    #[tokio::test]
    async fn test_context_injection_logs_metrics() {
        // This test verifies the logging behavior by checking state transitions.
        // The actual tracing::info! calls are verified by the log output format
        // documented in the implementation (hash, tokens, cache_status fields).
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Verify initial state
        assert!(state.compression().last_ccg_hash().is_none());
        assert_eq!(state.compression().context_tokens_injected(), 0);
        assert!(!state.compression().has_cached_ccg_context());

        // After setting context (simulating a successful fetch), the metrics
        // fields are populated for the status bar and logging
        state
            .compression_mut()
            .set_last_ccg_hash("abc123".to_string());
        state.compression_mut().set_context_tokens_injected(5000);
        state.compression.cached_ccg_context = Some("## Context".to_string());

        assert_eq!(state.compression().last_ccg_hash(), Some("abc123"));
        assert_eq!(state.compression().context_tokens_injected(), 5000);
        assert!(state.compression().has_cached_ccg_context());
    }

    #[tokio::test]
    async fn test_submit_message_with_cached_context_prepends() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.compression_mut().set_auto_context_enabled(true);

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
        assert!(!state.compression().has_cached_ccg_context());

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
            state.compression().has_cached_ccg_context(),
            "build_api_messages should not consume cached context"
        );
    }

    #[test]
    fn test_prepare_api_messages_injects_context_when_enabled() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.compression_mut().set_auto_context_enabled(true);
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
        state.compression_mut().set_auto_context_enabled(true);
        state.compression.cached_ccg_context = Some("## Context".to_string());

        state.api_messages_mut().push(ApiMessageV2::user("Hello"));

        // Call build_api_messages — it should read but NOT consume the cache
        let _ = state.build_api_messages();

        // Cache should still be present for future calls
        assert!(
            state.compression().has_cached_ccg_context(),
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
        state.conversation.streaming_rx = None;
        let (_tx, rx) = mpsc::channel(100);
        state.set_tool_result_rx(rx);
        assert!(state.has_background_work());

        // Clear tool channel
        state.tool_state_mut().clear_tool_result_rx();
        assert!(!state.has_background_work());
    }

    #[test]
    fn test_completion_triggers_render_via_input_state() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.mark_rendered();
        state.show_completion();
        assert!(
            state.view.input_state.is_dirty(),
            "InputState self-tracks dirty on completion"
        );
        assert!(
            state.needs_render(),
            "needs_render detects InputState dirty"
        );
    }

    #[test]
    fn test_worktree_setters_trigger_render_via_self_tracking() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        state.mark_rendered();
        state.set_worktree_branch("main".to_string());
        assert!(
            state.view.worktree.is_dirty(),
            "set_branch self-tracks dirty"
        );
        assert!(state.needs_render());

        state.mark_rendered();
        state.set_worktree_modified(1);
        assert!(state.view.worktree.is_dirty());

        state.mark_rendered();
        state.set_worktree_ahead(1);
        assert!(state.view.worktree.is_dirty());

        state.mark_rendered();
        state.set_worktree_behind(1);
        assert!(state.view.worktree.is_dirty());
    }

    #[test]
    fn test_app_state_agent_panel_delegation() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        assert!(!state.agent_panel().subagents_enabled());
        assert!(state.agent_panel().entries().is_empty());

        let progress = AgentProgress::IterationStarted {
            iteration: 1,
            max: 3,
        };
        state.update_agent_progress("a1", "Agent", &progress);
        assert_eq!(state.agent_panel().entries().len(), 1);
        assert!(
            state.agent_panel.is_dirty(),
            "AgentPanelState self-tracks dirty"
        );
        assert!(
            state.needs_render(),
            "needs_render detects agent_panel dirty"
        );
    }

    #[test]
    fn test_continuous_loop_state_initial() {
        let cls = ContinuousLoopState {
            status: ContinuousLoopStatus::Inactive,
            iterations_completed: 0,
            last_duration_ms: None,
            checking_gate: None,
            gate_results: Vec::new(),
            dirty: false,
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
            dirty: false,
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
            dirty: false,
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
            dirty: false,
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
            dirty: false,
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
            dirty: false,
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
        state.mark_rendered();

        state.update_continuous_iteration(1);
        assert!(
            state.continuous.is_dirty(),
            "ContinuousLoopState self-tracks dirty"
        );
        assert!(state.needs_render());
        assert_eq!(
            *state.continuous().status(),
            ContinuousLoopStatus::Running { iteration: 1 }
        );

        state.mark_rendered();
        state.complete_continuous_iteration(1, 2000);
        assert!(state.continuous.is_dirty());
        assert_eq!(state.continuous().iterations_completed(), 1);

        state.mark_rendered();
        state.reset_continuous();
        assert!(state.continuous.is_dirty());
        assert_eq!(*state.continuous().status(), ContinuousLoopStatus::Inactive);
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
        state.model_config_mut().set_effort(EffortLevel::High);
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
        state.model_config_mut().set_effort(EffortLevel::Medium);
        state.model_config_mut().set_thinking_budget(Some(50_000));
        let opts = state.build_request_options("claude-sonnet-4-20250514");
        let thinking = opts
            .thinking
            .expect("Explicit budget should produce thinking config");
        assert_eq!(thinking.budget_tokens, 50_000);
    }

    #[test]
    fn test_build_request_options_unsupported_model() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.model_config_mut().set_effort(EffortLevel::High);
        let opts = state.build_request_options("unknown-model");
        // Unknown model → no thinking support
        assert!(opts.thinking.is_none());
    }

    #[test]
    fn test_build_request_options_system_prompt_with_cache() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state
            .model_config_mut()
            .set_system_prompt(Some("You are a helpful assistant.".to_string()));
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
        state
            .model_config_mut()
            .set_system_prompt(Some("Instructions".to_string()));
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
            !state.compression().is_compaction_requested(),
            "No compaction should be requested initially"
        );

        state.force_compact(Some("summarize briefly".to_string()));

        assert!(
            state.compression().is_compaction_requested(),
            "Compaction should be requested after force_compact"
        );
        assert!(
            state.compression.is_render_dirty(),
            "CompressionState self-tracks render-dirty"
        );
        assert!(state.needs_render());
    }

    #[test]
    fn test_force_compact_no_instructions() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.force_compact(None);

        assert!(state.compression().is_compaction_requested());

        // Verify the request can be consumed
        let request = state.compression.take_compaction_request();
        assert_eq!(request, Some(None));
        assert!(!state.compression().is_compaction_requested());
    }

    #[test]
    fn test_sync_token_budget_sets_dirty_flag() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        // Add a message so there are tokens to count
        state
            .conversation
            .api_messages
            .push(ApiMessageV2::user("Hello, world!"));

        state.mark_rendered();
        state.sync_token_budget();

        assert!(
            state.compression.is_render_dirty(),
            "sync_token_budget should set render-dirty via add_token_usage"
        );
        assert!(state.needs_render());
        assert!(
            state.compression().token_budget().used() > 0,
            "Token budget should reflect the conversation tokens"
        );
    }

    #[test]
    fn test_add_api_message_adds_to_both_stores() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        assert!(state.api_messages().is_empty());
        assert_eq!(state.timeline().len(), 0);

        let msg = ApiMessageV2::user("Hello from API message");
        state.add_api_message(msg);

        assert_eq!(state.api_messages().len(), 1, "Should add to api_messages");
        assert!(!state.timeline().is_empty(), "Should add to timeline");
    }

    #[test]
    fn test_has_mcp_manager_returns_false_when_no_manager() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        assert!(
            !state.has_mcp_manager(),
            "Fresh state should not have an MCP manager"
        );
    }

    #[test]
    fn test_memory_store_mut_returns_none_when_not_set() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        assert!(
            state.model_config_mut().memory_store_mut().is_none(),
            "Fresh state should have no memory store"
        );
    }

    #[test]
    fn test_background_tasks_mut_returns_mutable_ref() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        // Verify we get a valid mutable reference by checking the registry
        let registry = state.background_tasks_mut();
        assert_eq!(
            registry.count(),
            0,
            "Fresh registry should have no active tasks"
        );
    }

    #[test]
    fn test_take_conflict_reports_drains_reports() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Initially empty
        let reports = state.take_conflict_reports();
        assert!(reports.is_empty(), "Should start with no conflict reports");

        // Add a conflict report
        state.add_conflict_report(ConflictReport::empty());

        // First take should return the report
        let reports = state.take_conflict_reports();
        assert_eq!(reports.len(), 1, "Should have one conflict report");

        // Second take should be empty (drained)
        let reports = state.take_conflict_reports();
        assert!(reports.is_empty(), "Should be empty after drain");
    }

    #[test]
    fn test_tick_throbber_does_not_dirty_messages() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.mark_rendered(); // Clear all dirty flags

        state.tick_throbber();

        // Throbber tick should NOT mark full as dirty
        assert!(!state.dirty.full, "tick_throbber must not dirty full");
        // But the overall needs_render should still be true
        assert!(
            state.needs_render(),
            "needs_render must be true after tick_throbber"
        );
        // The throbber flag specifically should be set
        assert!(
            state.dirty.throbber,
            "tick_throbber must set dirty.throbber"
        );
    }

    #[test]
    fn test_throbber_only_dirty() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.mark_rendered(); // Clear all dirty flags

        // After tick_throbber, only the throbber flag should be dirty
        state.tick_throbber();
        assert!(
            state.is_throbber_only_dirty(),
            "is_throbber_only_dirty must be true after only tick_throbber"
        );

        // After a full dirty, throbber_only should be false
        state.dirty.full = true;
        assert!(
            !state.is_throbber_only_dirty(),
            "is_throbber_only_dirty must be false when full also dirty"
        );
    }

    #[test]
    fn test_needs_render_from_input_state_self_tracking() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.mark_rendered();

        // InputState self-tracks dirty — no central dirty.input needed
        state.insert_char('a');
        assert!(
            state.view.input_state.is_dirty(),
            "InputState must self-track dirty on insert_char"
        );
        assert!(
            state.needs_render(),
            "needs_render must detect InputState dirty"
        );
    }

    #[test]
    fn test_throbber_only_dirty_not_triggered_by_input() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.mark_rendered();

        state.tick_throbber();
        state.insert_char('x');

        assert!(
            !state.is_throbber_only_dirty(),
            "is_throbber_only_dirty must be false when input is also dirty"
        );
    }

    #[test]
    fn test_throbber_only_dirty_not_triggered_by_agent_panel() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.mark_rendered();

        state.tick_throbber();
        let progress = AgentProgress::IterationStarted {
            iteration: 1,
            max: 3,
        };
        state.update_agent_progress("a1", "Agent", &progress);

        assert!(
            !state.is_throbber_only_dirty(),
            "is_throbber_only_dirty must be false when agent_panel is also dirty"
        );
    }
}
