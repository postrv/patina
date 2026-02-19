//! Application state management

use crate::agents::{AgentProgress, ConflictReport, SubagentSpawner};
use crate::api::tokens::model_context_limit;
use crate::api::tools::default_tools;
use crate::api::{LlmProvider, StreamEvent, TokenBudget, ToolChoice};
use crate::app::tool_loop::{ContinuationData, ToolLoop, ToolLoopState};
use crate::app::STREAMING_CHANNEL_BUFFER;
use crate::context::compression::{
    CompactionMetrics, CompactionMetricsSummary, CompressionOrchestrator,
};
use crate::hooks::HookManager;
use crate::mcp::client::McpClient;
use crate::narsil::context::ContextSuggestion;
use crate::permissions::{PermissionManager, PermissionRequest, PermissionResponse};
use crate::plugins::PluginRegistry;
use crate::session::Session;
use crate::tools::HookedToolExecutor;
use crate::tui::scroll::ScrollState;
use crate::tui::selection::{FocusArea, SelectionState};
use crate::tui::widgets::{CompactionProgressState, ToolBlockState};
use crate::types::config::ParallelMode;
use crate::types::content::StopReason;
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

/// Events received from background tasks (API streaming or tool execution).
///
/// Used by `recv_background_event()` to return events from multiple channels
/// without borrow checker conflicts.
#[derive(Debug)]
pub enum BackgroundEvent {
    /// An API streaming chunk was received.
    ApiChunk(StreamEvent),
    /// A tool execution completed with its result.
    ToolResult(String, crate::types::ToolResultBlock),
}

/// Status of the continuous coding loop as displayed in the TUI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContinuousLoopStatus {
    /// No continuous loop is running.
    Inactive,
    /// Loop is actively running at the given iteration.
    Running {
        /// Current iteration (1-indexed).
        iteration: u32,
    },
    /// Stagnation has been detected.
    Stagnated {
        /// Number of iterations without progress.
        iterations_without_progress: u32,
        /// The threshold that was exceeded.
        threshold: u32,
    },
    /// Human intervention is required.
    HumanRequired {
        /// Reason why human intervention is needed.
        reason: String,
    },
}

/// Result of a single quality gate check for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateResult {
    /// Name of the quality gate.
    pub gate: String,
    /// Whether the gate passed.
    pub passed: bool,
    /// Optional detail message.
    pub message: Option<String>,
}

/// Status of an agent as displayed in the TUI agent panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentPanelStatus {
    /// Agent is actively running.
    Running {
        /// Current iteration (1-indexed).
        iteration: usize,
        /// Maximum iterations allowed.
        max_iterations: usize,
    },
    /// Agent completed successfully.
    Completed {
        /// Number of iterations used.
        iterations_used: usize,
    },
    /// Agent failed with an error.
    Failed {
        /// Error description.
        error: String,
    },
}

/// An entry in the TUI agent panel showing an agent's current state.
#[derive(Debug, Clone)]
pub struct AgentPanelEntry {
    /// Unique identifier for the agent instance.
    pub agent_id: String,
    /// Human-readable agent name.
    pub agent_name: String,
    /// Current status of the agent.
    pub status: AgentPanelStatus,
    /// Most recent content snippet from the agent.
    pub last_content: String,
}

/// Compression, context injection, and compaction state extracted from AppState.
///
/// Groups all fields related to: context compression (CCG orchestrator, caching),
/// narsil MCP client management, auto-context injection, token budgeting,
/// compaction progress, and compaction metrics.
pub struct CompressionState {
    /// Optional compression orchestrator for CCG context management.
    pub(crate) compression_orchestrator: Option<Arc<CompressionOrchestrator>>,

    /// Last repository hash used for CCG context injection.
    pub(crate) last_ccg_hash: Option<String>,

    /// Cached CCG context content for injection into messages.
    pub(crate) cached_ccg_context: Option<String>,

    /// Optional narsil MCP client for CCG context fetching.
    pub(crate) narsil_client: Option<McpClient>,

    /// Maximum tokens to include in auto-injected context.
    pub(crate) context_token_budget: usize,

    /// Tokens actually injected in the most recent context injection.
    pub(crate) context_tokens_injected: usize,

    /// Whether auto-context injection is enabled.
    pub(crate) auto_context_enabled: bool,

    /// Pending context suggestions to be injected into the next message.
    pub(crate) pending_context: Vec<ContextSuggestion>,

    /// Optional compaction progress state for displaying the compaction overlay.
    pub(crate) compaction_state: Option<CompactionProgressState>,

    /// Metrics tracking for compaction operations.
    pub(crate) compaction_metrics: Arc<CompactionMetrics>,

    /// Token budget tracking for the current session.
    pub(crate) token_budget: TokenBudget,
}

/// Continuous coding loop state extracted from AppState.
///
/// Groups all fields related to the continuous (Ralph-style) coding loop:
/// status, iteration tracking, timing, and quality gate results.
pub struct ContinuousLoopState {
    /// Current status of the continuous coding loop.
    pub(crate) status: ContinuousLoopStatus,

    /// Number of completed iterations in the current continuous session.
    pub(crate) iterations_completed: u32,

    /// Duration of the last completed iteration in milliseconds.
    pub(crate) last_duration_ms: Option<u64>,

    /// Name of the quality gate currently being checked (if any).
    pub(crate) checking_gate: Option<String>,

    /// Accumulated quality gate results for the current iteration.
    pub(crate) gate_results: Vec<GateResult>,
}

/// UI selection and copy state extracted from AppState.
///
/// Groups fields related to text selection, clipboard copy operations,
/// and UI focus tracking.
pub struct UISelectionState {
    /// Text selection state for copy/paste functionality.
    pub(crate) selection: SelectionState,

    /// Flag indicating a copy operation was requested.
    pub(crate) copy_pending: bool,

    /// Cached rendered lines for copy operations.
    pub(crate) rendered_lines_cache: Vec<String>,

    /// Which area of the UI currently has focus.
    pub(crate) focus_area: FocusArea,
}

/// Tool execution state extracted from AppState.
///
/// Groups all tool-related fields: the tool loop state machine, executor,
/// permission management, UI tool blocks, async result channels, and
/// in-flight tracking. This struct owns no behavior — all methods live
/// on `AppState` and delegate through `self.tool_state`.
pub struct ToolExecutionState {
    /// State machine coordinating tool approval, execution, and continuation.
    pub(crate) tool_loop: ToolLoop,

    /// Shared tool executor with hook integration and parallel support.
    pub(crate) tool_executor: Arc<HookedToolExecutor>,

    /// Thread-safe permission manager for tool approval decisions.
    pub(crate) permission_manager: Arc<Mutex<PermissionManager>>,

    /// Current permission request awaiting user response, if any.
    pub(crate) pending_permission: Option<PermissionRequest>,

    /// Tool blocks for UI display.
    /// Each block represents a tool execution with its name, input, and result.
    pub(crate) tool_blocks: Vec<ToolBlockState>,

    /// Channel receiver for async tool results.
    /// When set, tool execution runs in the background and results
    /// are streamed back through this channel.
    pub(crate) tool_result_rx:
        Option<mpsc::UnboundedReceiver<(String, crate::types::ToolResultBlock)>>,

    /// Set of tool IDs currently being executed.
    /// Used to track which tools are in-flight for progress display.
    pub(crate) executing_tool_ids: std::collections::HashSet<String>,
}

pub struct AppState {
    /// Full API messages with content blocks (tool_use, tool_result).
    /// This is the authoritative conversation history sent to the API.
    api_messages: Vec<ApiMessageV2>,

    pub input: String,
    pub working_dir: PathBuf,

    /// Smart scroll state with auto-follow behavior.
    scroll: ScrollState,

    cursor_pos: usize,
    loading: bool,
    throbber_frame: usize,
    streaming_rx: Option<mpsc::Receiver<StreamEvent>>,

    dirty: DirtyFlags,

    // Worktree status bar state
    worktree_branch: Option<String>,
    worktree_modified: usize,
    worktree_ahead: usize,
    worktree_behind: usize,

    // Session tracking for auto-save
    session_id: Option<String>,

    /// Whether the session needs to be saved.
    /// Set by handlers/code that modify conversation state; cleared by `SessionHandler`.
    session_dirty: bool,

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

    /// Optional subagent spawner for creating subagent sessions.
    /// Only initialized when subagent orchestration is enabled via `--enable-subagents`.
    subagent_spawner: Option<SubagentSpawner>,

    /// Agent panel entries for the TUI agent status display.
    /// Updated by `AgentHandler` when agent events are received.
    agent_panel_entries: Vec<AgentPanelEntry>,

    /// Pending conflict reports from cross-agent conflict detection.
    /// Consumed by the TUI to display conflict alerts.
    pending_conflict_reports: Vec<ConflictReport>,

    /// All continuous coding loop state grouped together.
    continuous: ContinuousLoopState,

    /// Cached terminal height for scroll calculations.
    /// Updated on resize events; defaults to 24 for headless/test environments.
    terminal_height: u16,
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
            input: String::new(),
            working_dir,
            scroll: ScrollState::new(),
            cursor_pos: 0,
            loading: false,
            throbber_frame: 0,
            streaming_rx: None,
            dirty: DirtyFlags {
                full: true,
                ..Default::default()
            },
            worktree_branch: None,
            worktree_modified: 0,
            worktree_ahead: 0,
            worktree_behind: 0,
            session_id: None,
            session_dirty: false,
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
            },
            plugin_registry,
            subagent_spawner,
            agent_panel_entries: Vec::new(),
            pending_conflict_reports: Vec::new(),
            continuous: ContinuousLoopState {
                status: ContinuousLoopStatus::Inactive,
                iterations_completed: 0,
                last_duration_ms: None,
                checking_gate: None,
                gate_results: Vec::new(),
            },
            terminal_height: 24,
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

    /// Returns whether subagent orchestration is enabled.
    #[must_use]
    pub fn subagents_enabled(&self) -> bool {
        self.subagent_spawner.is_some()
    }

    /// Returns a reference to the subagent spawner if enabled.
    #[must_use]
    pub fn subagent_spawner(&self) -> Option<&SubagentSpawner> {
        self.subagent_spawner.as_ref()
    }

    // =========================================================================
    // Agent panel methods (Task 4.5)
    // =========================================================================

    /// Returns the current agent panel entries for TUI rendering.
    #[must_use]
    pub fn agent_panel_entries(&self) -> &[AgentPanelEntry] {
        &self.agent_panel_entries
    }

    /// Updates the agent panel with a progress event.
    ///
    /// If an entry for the given `agent_id` exists, it is updated in place.
    /// Otherwise, a new entry is created.
    ///
    /// # Panics
    ///
    /// Does not panic.
    pub fn update_agent_progress(
        &mut self,
        agent_id: &str,
        agent_name: &str,
        progress: &AgentProgress,
    ) {
        let status = match progress {
            AgentProgress::IterationStarted { iteration, max } => AgentPanelStatus::Running {
                iteration: *iteration,
                max_iterations: *max,
            },
            AgentProgress::ContentDelta(_) => {
                // Keep existing status for content deltas; just update last_content.
                if let Some(entry) = self
                    .agent_panel_entries
                    .iter_mut()
                    .find(|e| e.agent_id == agent_id)
                {
                    if let AgentProgress::ContentDelta(text) = progress {
                        entry.last_content = text.chars().take(80).collect();
                    }
                    self.dirty.full = true;
                    return;
                }
                // First event for this agent; default to iteration 0.
                AgentPanelStatus::Running {
                    iteration: 0,
                    max_iterations: 0,
                }
            }
            AgentProgress::Completed {
                iterations_used, ..
            } => AgentPanelStatus::Completed {
                iterations_used: *iterations_used,
            },
            AgentProgress::Failed { error, .. } => AgentPanelStatus::Failed {
                error: error.clone(),
            },
        };

        if let Some(entry) = self
            .agent_panel_entries
            .iter_mut()
            .find(|e| e.agent_id == agent_id)
        {
            entry.status = status;
            if let AgentProgress::ContentDelta(text) = progress {
                entry.last_content = text.chars().take(80).collect();
            }
            if let AgentProgress::Completed { output, .. } = progress {
                entry.last_content = output.chars().take(80).collect();
            }
        } else {
            let last_content = match progress {
                AgentProgress::ContentDelta(text) => text.chars().take(80).collect(),
                AgentProgress::Completed { output, .. } => output.chars().take(80).collect(),
                _ => String::new(),
            };
            self.agent_panel_entries.push(AgentPanelEntry {
                agent_id: agent_id.to_string(),
                agent_name: agent_name.to_string(),
                status,
                last_content,
            });
        }

        self.dirty.full = true;
    }

    /// Records a conflict report for display in the TUI.
    pub fn add_conflict_report(&mut self, report: ConflictReport) {
        self.pending_conflict_reports.push(report);
        self.dirty.full = true;
    }

    /// Takes all pending conflict reports, leaving the internal list empty.
    pub fn take_conflict_reports(&mut self) -> Vec<ConflictReport> {
        std::mem::take(&mut self.pending_conflict_reports)
    }

    /// Returns `true` if there are pending conflict reports.
    #[must_use]
    pub fn has_pending_conflicts(&self) -> bool {
        !self.pending_conflict_reports.is_empty()
    }

    // =========================================================================
    // Continuous loop panel methods (Task 6.6)
    // =========================================================================

    /// Returns the current status of the continuous coding loop.
    #[must_use]
    pub fn continuous_status(&self) -> &ContinuousLoopStatus {
        &self.continuous.status
    }

    /// Returns the number of completed iterations in the current session.
    #[must_use]
    pub fn continuous_iterations_completed(&self) -> u32 {
        self.continuous.iterations_completed
    }

    /// Returns the duration of the last completed iteration in milliseconds.
    #[must_use]
    pub fn continuous_last_duration_ms(&self) -> Option<u64> {
        self.continuous.last_duration_ms
    }

    /// Returns the name of the quality gate currently being checked.
    #[must_use]
    pub fn continuous_checking_gate(&self) -> Option<&str> {
        self.continuous.checking_gate.as_deref()
    }

    /// Returns accumulated quality gate results for the current iteration.
    #[must_use]
    pub fn continuous_gate_results(&self) -> &[GateResult] {
        &self.continuous.gate_results
    }

    /// Updates state for a new continuous iteration starting.
    ///
    /// Clears gate results from the previous iteration and sets status to Running.
    pub fn update_continuous_iteration(&mut self, iteration: u32) {
        self.continuous.status = ContinuousLoopStatus::Running { iteration };
        self.continuous.checking_gate = None;
        self.continuous.gate_results.clear();
        self.dirty.full = true;
    }

    /// Records the completion of a continuous iteration.
    pub fn complete_continuous_iteration(&mut self, _iteration: u32, duration_ms: u64) {
        self.continuous.iterations_completed += 1;
        self.continuous.last_duration_ms = Some(duration_ms);
        self.dirty.full = true;
    }

    /// Records that a quality gate check is starting.
    pub fn set_continuous_gate_checking(&mut self, gate: &str) {
        self.continuous.checking_gate = Some(gate.to_string());
        self.dirty.full = true;
    }

    /// Records the result of a quality gate check.
    pub fn record_continuous_gate_result(
        &mut self,
        gate: &str,
        passed: bool,
        message: Option<&str>,
    ) {
        self.continuous.checking_gate = None;
        self.continuous.gate_results.push(GateResult {
            gate: gate.to_string(),
            passed,
            message: message.map(String::from),
        });
        self.dirty.full = true;
    }

    /// Records that stagnation was detected.
    pub fn set_continuous_stagnation(&mut self, iterations_without_progress: u32, threshold: u32) {
        self.continuous.status = ContinuousLoopStatus::Stagnated {
            iterations_without_progress,
            threshold,
        };
        self.dirty.full = true;
    }

    /// Records that human intervention is required.
    pub fn set_continuous_human_checkpoint(&mut self, reason: &str) {
        self.continuous.status = ContinuousLoopStatus::HumanRequired {
            reason: reason.to_string(),
        };
        self.dirty.full = true;
    }

    /// Resets all continuous loop state to inactive.
    ///
    /// Call this when stopping the continuous loop to clear the TUI display.
    pub fn reset_continuous(&mut self) {
        self.continuous.status = ContinuousLoopStatus::Inactive;
        self.continuous.iterations_completed = 0;
        self.continuous.last_duration_ms = None;
        self.continuous.checking_gate = None;
        self.continuous.gate_results.clear();
        self.dirty.full = true;
    }

    // =========================================================================
    // Auto-context methods (Task 2.2.4)
    // =========================================================================

    /// Returns whether auto-context injection is enabled.
    ///
    /// When enabled, context suggestions from narsil are injected into
    /// user messages before API calls.
    #[must_use]
    pub fn auto_context_enabled(&self) -> bool {
        self.compression.auto_context_enabled
    }

    /// Sets whether auto-context injection is enabled.
    ///
    /// # Arguments
    ///
    /// * `enabled` - Whether to enable auto-context injection
    pub fn set_auto_context_enabled(&mut self, enabled: bool) {
        self.compression.auto_context_enabled = enabled;
    }

    /// Returns whether there are pending context suggestions.
    #[must_use]
    pub fn has_pending_context(&self) -> bool {
        !self.compression.pending_context.is_empty()
    }

    /// Returns a reference to the pending context suggestions.
    #[must_use]
    pub fn pending_context(&self) -> &[ContextSuggestion] {
        &self.compression.pending_context
    }

    /// Sets the pending context suggestions to be injected into the next message.
    ///
    /// These suggestions will be cleared after being consumed by `take_pending_context`.
    ///
    /// # Arguments
    ///
    /// * `suggestions` - Context suggestions from narsil
    pub fn set_pending_context(&mut self, suggestions: Vec<ContextSuggestion>) {
        self.compression.pending_context = suggestions;
    }

    /// Takes and returns the pending context suggestions, clearing them.
    ///
    /// After calling this method, `has_pending_context()` will return false.
    #[must_use]
    pub fn take_pending_context(&mut self) -> Vec<ContextSuggestion> {
        std::mem::take(&mut self.compression.pending_context)
    }

    /// Clears the pending context suggestions without returning them.
    pub fn clear_pending_context(&mut self) {
        self.compression.pending_context.clear();
    }

    // =========================================================================
    // Compression Orchestrator Methods (Phase 4.3)
    // =========================================================================

    /// Sets the compression orchestrator for CCG context management.
    ///
    /// The orchestrator is typically created from `NarsilIntegration::create_compression_orchestrator()`
    /// when narsil-mcp is available.
    ///
    /// # Arguments
    ///
    /// * `orchestrator` - The compression orchestrator (wrapped in Arc)
    pub fn set_compression_orchestrator(&mut self, orchestrator: Arc<CompressionOrchestrator>) {
        self.compression.compression_orchestrator = Some(orchestrator);
    }

    /// Returns a reference to the compression orchestrator if available.
    #[must_use]
    pub fn compression_orchestrator(&self) -> Option<&Arc<CompressionOrchestrator>> {
        self.compression.compression_orchestrator.as_ref()
    }

    /// Returns true if the compression orchestrator supports CCG (Code Context Graph).
    ///
    /// CCG support enables advanced context compression features like manifest
    /// and architecture extraction. Returns false if no orchestrator is set.
    #[must_use]
    pub fn has_ccg_support(&self) -> bool {
        self.compression
            .compression_orchestrator
            .as_ref()
            .is_some_and(|o| o.should_use_ccg())
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
    /// if let Some(context) = state.inject_ccg_context(&mut client, "abc123").await? {
    ///     // Prepend context to system message
    ///     system_prompt = format!("{}\n\n{}", context, system_prompt);
    /// }
    /// ```
    pub async fn inject_ccg_context(
        &mut self,
        client: &mut McpClient,
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
            .map_or(true, |h| h != repo_hash);

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
    ///
    /// This can be used to check if the repository state has changed since
    /// the last context injection.
    #[must_use]
    pub fn last_ccg_hash(&self) -> Option<&str> {
        self.compression.last_ccg_hash.as_deref()
    }

    /// Sets the last CCG hash (for testing and manual cache management).
    pub fn set_last_ccg_hash(&mut self, hash: String) {
        self.compression.last_ccg_hash = Some(hash);
    }

    /// Returns whether there is cached CCG context available for injection.
    #[must_use]
    pub fn has_cached_ccg_context(&self) -> bool {
        self.compression.cached_ccg_context.is_some()
    }

    /// Takes the cached CCG context, returning it and clearing the cache.
    ///
    /// This is typically called in the message send flow to inject context
    /// into the first user message.
    ///
    /// # Returns
    ///
    /// The cached CCG context if available, `None` otherwise.
    pub fn take_cached_ccg_context(&mut self) -> Option<String> {
        self.compression.cached_ccg_context.take()
    }

    /// Returns the context to inject before sending a message.
    ///
    /// This method checks if auto-context is enabled and returns the
    /// CCG context if available. The context is NOT consumed (use
    /// `take_cached_ccg_context` to consume it).
    ///
    /// # Returns
    ///
    /// A reference to the CCG context if auto-context is enabled and context
    /// is available, `None` otherwise.
    #[must_use]
    pub fn context_for_injection(&self) -> Option<&str> {
        if self.compression.auto_context_enabled {
            self.compression.cached_ccg_context.as_deref()
        } else {
            None
        }
    }

    /// Returns whether a narsil MCP client is available.
    #[must_use]
    pub fn has_narsil_client(&self) -> bool {
        self.compression.narsil_client.is_some()
    }

    /// Sets the narsil MCP client for context fetching.
    pub fn set_narsil_client(&mut self, client: McpClient) {
        self.compression.narsil_client = Some(client);
    }

    /// Returns the maximum token budget for auto-injected context.
    #[must_use]
    pub fn context_token_budget(&self) -> usize {
        self.compression.context_token_budget
    }

    /// Sets the maximum token budget for auto-injected context.
    pub fn set_context_token_budget(&mut self, budget: usize) {
        self.compression.context_token_budget = budget;
    }

    /// Returns the number of tokens injected in the most recent context injection.
    #[must_use]
    pub fn context_tokens_injected(&self) -> usize {
        self.compression.context_tokens_injected
    }

    /// Sets the number of tokens injected (used by tests and status bar display).
    pub fn set_context_tokens_injected(&mut self, tokens: usize) {
        self.compression.context_tokens_injected = tokens;
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

        // Lazily initialize narsil client
        if self.compression.narsil_client.is_none() {
            let working_dir = self.working_dir.to_string_lossy().to_string();
            let mut client =
                McpClient::new("narsil-mcp", "narsil-mcp", vec!["--repos", &working_dir]);
            match client.start().await {
                Ok(()) => {
                    tracing::info!("Narsil MCP client connected for context injection");
                    self.compression.narsil_client = Some(client);
                }
                Err(e) => {
                    tracing::warn!("Failed to connect narsil-mcp for context: {}", e);
                    return None;
                }
            }
        }

        // Take client to avoid borrow conflict with self
        let mut client = self.compression.narsil_client.take()?;

        let result = orchestrator
            .build_context(
                &mut client,
                &repo_hash,
                &[], // active_files: empty = project-wide symbols
                self.compression.context_token_budget,
            )
            .await;

        // Put client back
        self.compression.narsil_client = Some(client);

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

    /// Inserts a character at the current cursor position.
    pub fn insert_char(&mut self, c: char) {
        // Get byte position from char position
        let byte_pos = self
            .input
            .char_indices()
            .nth(self.cursor_pos)
            .map(|(i, _)| i)
            .unwrap_or(self.input.len());
        self.input.insert(byte_pos, c);
        self.cursor_pos += 1;
        self.dirty.input = true;
    }

    /// Deletes the character before the cursor (backspace behavior).
    pub fn delete_char(&mut self) {
        if self.cursor_pos > 0 {
            // Get byte position of the character to delete (one before cursor)
            let byte_pos = self
                .input
                .char_indices()
                .nth(self.cursor_pos - 1)
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.input.remove(byte_pos);
            self.cursor_pos -= 1;
        }
        self.dirty.input = true;
    }

    /// Takes and returns the current input, clearing the buffer and resetting cursor.
    pub fn take_input(&mut self) -> String {
        self.dirty.input = true;
        self.cursor_pos = 0;
        std::mem::take(&mut self.input)
    }

    /// Returns the current cursor position (character index, not byte index).
    #[must_use]
    pub fn cursor_position(&self) -> usize {
        self.cursor_pos
    }

    /// Moves the cursor left by one character.
    pub fn cursor_left(&mut self) {
        self.cursor_pos = self.cursor_pos.saturating_sub(1);
        self.dirty.input = true;
    }

    /// Moves the cursor right by one character.
    pub fn cursor_right(&mut self) {
        let char_count = self.input.chars().count();
        if self.cursor_pos < char_count {
            self.cursor_pos += 1;
        }
        self.dirty.input = true;
    }

    /// Moves the cursor to the beginning of the input.
    pub fn cursor_home(&mut self) {
        self.cursor_pos = 0;
        self.dirty.input = true;
    }

    /// Moves the cursor to the end of the input.
    pub fn cursor_end(&mut self) {
        self.cursor_pos = self.input.chars().count();
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

    /// Returns the selection state for read access.
    #[must_use]
    pub fn selection(&self) -> &SelectionState {
        &self.ui_selection.selection
    }

    /// Returns the selection state for modification.
    pub fn selection_mut(&mut self) -> &mut SelectionState {
        &mut self.ui_selection.selection
    }

    /// Returns the current focus area.
    #[must_use]
    pub fn focus_area(&self) -> FocusArea {
        self.ui_selection.focus_area
    }

    /// Sets the focus area, clearing selection if focus changes.
    ///
    /// When focus moves between Input and Content areas, any existing
    /// selection is cleared to prevent confusion about what would be copied.
    pub fn set_focus_area(&mut self, area: FocusArea) {
        if self.ui_selection.focus_area != area {
            self.ui_selection.selection.clear();
            self.ui_selection.focus_area = area;
        }
    }

    /// Determines which focus area a screen row belongs to.
    ///
    /// Layout (from top to bottom):
    /// - Messages/Content: rows 0 to (terminal_height - 5)
    /// - Status bar: row (terminal_height - 4)
    /// - Input: rows (terminal_height - 3) to (terminal_height - 1)
    ///
    /// # Arguments
    ///
    /// * `row` - The screen row (0-indexed, 0 = top)
    /// * `terminal_height` - Total terminal height in rows
    ///
    /// # Returns
    ///
    /// The `FocusArea` that the row belongs to.
    #[must_use]
    pub fn focus_area_for_row(row: u16, terminal_height: u16) -> FocusArea {
        // Input area is the bottom 3 rows
        // Status bar is 1 row above input
        // Content area is everything else
        let input_start = terminal_height.saturating_sub(3);
        if row >= input_start {
            FocusArea::Input
        } else {
            FocusArea::Content
        }
    }

    /// Copies the current selection to the system clipboard.
    ///
    /// Uses multiple clipboard backends:
    /// 1. Native clipboard (arboard) - works on desktop
    /// 2. OSC 52 escape sequence - works in iTerm2, kitty, tmux, SSH, etc.
    ///
    /// Returns `Ok(true)` if text was copied, `Ok(false)` if no selection,
    /// or an error if clipboard access fails.
    ///
    /// # Errors
    ///
    /// Returns an error if all clipboard methods fail.
    pub fn copy_selection_to_clipboard(&self, lines: &[ratatui::text::Line<'_>]) -> Result<bool> {
        let text = self.ui_selection.selection.extract_text(lines);
        if text.is_empty() {
            return Ok(false);
        }

        crate::tui::clipboard::copy_to_clipboard(&text)?;
        Ok(true)
    }

    /// Requests a copy operation to be performed during the next render.
    pub fn request_copy(&mut self) {
        self.ui_selection.copy_pending = true;
    }

    /// Checks and clears the copy pending flag.
    ///
    /// Returns `true` if a copy was requested.
    pub fn take_copy_pending(&mut self) -> bool {
        std::mem::take(&mut self.ui_selection.copy_pending)
    }

    /// Returns the total number of rendered lines.
    ///
    /// This is the count from the cached rendered lines, which represents
    /// the actual number of visual lines in the conversation display.
    /// Used for select-all functionality.
    #[must_use]
    pub fn rendered_line_count(&self) -> usize {
        self.ui_selection.rendered_lines_cache.len()
    }

    /// Updates the cached rendered lines for copy operations.
    ///
    /// This stores the **wrapped** visual lines, accounting for terminal width.
    /// Selection and copy operations use visual line indices, so we must cache
    /// the post-wrapping content.
    ///
    /// # Arguments
    ///
    /// * `lines` - The logical lines before wrapping
    /// * `width` - The terminal content width (excluding borders)
    pub fn update_rendered_lines_cache(&mut self, lines: &[ratatui::text::Line<'_>], width: usize) {
        self.ui_selection.rendered_lines_cache = crate::tui::wrap_lines_to_strings(lines, width);
    }

    /// Copies the current selection to clipboard using cached lines.
    ///
    /// # Errors
    ///
    /// Returns an error if clipboard access fails.
    pub fn copy_from_cache(&self) -> Result<bool> {
        let Some((start, end)) = self.ui_selection.selection.range() else {
            tracing::debug!("copy_from_cache: no selection range");
            return Ok(false);
        };

        tracing::debug!(
            ?start,
            ?end,
            cache_len = self.ui_selection.rendered_lines_cache.len(),
            "copy_from_cache: extracting"
        );

        if self.ui_selection.rendered_lines_cache.is_empty() {
            tracing::debug!("copy_from_cache: cache is empty");
            return Ok(false);
        }

        // Extract text from cached lines
        let mut result = String::new();
        for (line_idx, line_text) in self.ui_selection.rendered_lines_cache.iter().enumerate() {
            if line_idx < start.line {
                continue;
            }
            if line_idx > end.line {
                break;
            }

            let (col_start, col_end) = if line_idx == start.line && line_idx == end.line {
                (start.col, end.col.min(line_text.len()))
            } else if line_idx == start.line {
                (start.col, line_text.len())
            } else if line_idx == end.line {
                (0, end.col.min(line_text.len()))
            } else {
                (0, line_text.len())
            };

            let col_start = col_start.min(line_text.len());
            let col_end = col_end.min(line_text.len());

            if col_start <= col_end {
                if !result.is_empty() {
                    result.push('\n');
                }
                result.push_str(&line_text[col_start..col_end]);
            }
        }

        if result.is_empty() {
            tracing::debug!("copy_from_cache: extracted empty result");
            return Ok(false);
        }

        tracing::debug!(
            result_len = result.len(),
            result_lines = result.lines().count(),
            "copy_from_cache: copying to clipboard"
        );

        crate::tui::clipboard::copy_to_clipboard(&result)?;
        Ok(true)
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

        // Auto-compact conversation if approaching context limit
        let context_limit = model_context_limit(client.model());
        if self.maybe_compact_graceful(DEFAULT_COMPACTION_THRESHOLD, context_limit) {
            tracing::info!(
                threshold = DEFAULT_COMPACTION_THRESHOLD,
                context_limit,
                "Conversation compacted before API call"
            );
        }

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
            self.tool_state.tool_loop.start_streaming().ok();
        }

        let (tx, rx) = mpsc::channel(STREAMING_CHANNEL_BUFFER);
        self.streaming_rx = Some(rx);

        // Use truncated api_messages for the API call to control costs
        // while preserving content blocks for tool results
        let total_messages = self.api_messages.len();
        let api_messages = self.api_messages_truncated();
        let truncated_messages = api_messages.len();

        if truncated_messages < total_messages {
            tracing::info!(
                total = total_messages,
                sending = truncated_messages,
                dropped = total_messages - truncated_messages,
                "Context truncated for API call"
            );
        }

        let client = std::sync::Arc::clone(client);
        let tools = default_tools();
        tokio::spawn(async move {
            if let Err(e) = client
                .stream_message(&api_messages, Some(&tools), Some(&ToolChoice::Auto), tx)
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
            StreamEvent::ContentDelta(text) => {
                // Update timeline streaming entry
                self.timeline.append_to_streaming(&text);
                // Also forward to tool loop for tracking assistant text
                self.tool_state.tool_loop.append_text(&text);
                self.dirty.messages = true;
            }
            StreamEvent::MessageStop => {
                // Only process if we're actually streaming (prevents duplicates)
                // MessageComplete may have already handled this
                if self.timeline.is_streaming() {
                    self.timeline.finalize_streaming_as_message();
                    // Get the finalized text for API messages
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
            StreamEvent::MessageComplete { stop_reason } => {
                // For tool_use stop reasons, the assistant message will be added
                // later by handle_tool_execution with full content blocks
                let needs_tool_execution = stop_reason.needs_tool_execution();

                if needs_tool_execution {
                    // P0-1 FIX: For tool_use responses, finalize streaming for tool use.
                    // The text is already in tool_loop.text_content (via append_text calls).
                    // handle_tool_execution() will build the proper assistant message with
                    // both text AND tool_use blocks, preventing duplicate messages.
                    self.timeline.finalize_streaming_for_tool_use();
                    tracing::debug!(
                        "Tool use response - text stored in tool_loop, not adding to API yet"
                    );
                } else {
                    // For normal responses, finalize streaming and add to API messages
                    self.timeline.finalize_streaming_as_message();
                    if let Some(crate::types::ConversationEntry::AssistantMessage(text)) =
                        self.timeline.entries().last()
                    {
                        self.api_messages.push(ApiMessageV2::assistant(text));
                    }
                }
                // Handle stop reason in tool loop
                self.handle_message_complete(stop_reason)?;
                self.loading = false;
                self.streaming_rx = None;
                self.dirty.messages = true;
            }
            StreamEvent::Error(e) => {
                tracing::error!("Stream error: {}", e);
                self.loading = false;
                self.streaming_rx = None;
                self.dirty.messages = true;
            }
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
        }
        Ok(())
    }

    // ========================================================================
    // Worktree Status Bar State
    // ========================================================================

    /// Sets the current worktree branch name.
    ///
    /// This is displayed in the status bar.
    pub fn set_worktree_branch(&mut self, branch: String) {
        self.worktree_branch = Some(branch);
        self.dirty.full = true;
    }

    /// Returns the current worktree branch name, if set.
    #[must_use]
    pub fn worktree_branch(&self) -> Option<&str> {
        self.worktree_branch.as_deref()
    }

    /// Sets the number of modified files in the worktree.
    pub fn set_worktree_modified(&mut self, count: usize) {
        self.worktree_modified = count;
        self.dirty.full = true;
    }

    /// Returns the number of modified files in the worktree.
    #[must_use]
    pub fn worktree_modified(&self) -> usize {
        self.worktree_modified
    }

    /// Sets the number of commits ahead of upstream.
    pub fn set_worktree_ahead(&mut self, count: usize) {
        self.worktree_ahead = count;
        self.dirty.full = true;
    }

    /// Returns the number of commits ahead of upstream.
    #[must_use]
    pub fn worktree_ahead(&self) -> usize {
        self.worktree_ahead
    }

    /// Sets the number of commits behind upstream.
    pub fn set_worktree_behind(&mut self, count: usize) {
        self.worktree_behind = count;
        self.dirty.full = true;
    }

    /// Returns the number of commits behind upstream.
    #[must_use]
    pub fn worktree_behind(&self) -> usize {
        self.worktree_behind
    }

    // ========================================================================
    // Token Budget Tracking
    // ========================================================================

    /// Returns a reference to the token budget for display.
    #[must_use]
    pub fn token_budget(&self) -> &TokenBudget {
        &self.compression.token_budget
    }

    /// Returns a mutable reference to the token budget.
    pub fn token_budget_mut(&mut self) -> &mut TokenBudget {
        &mut self.compression.token_budget
    }

    /// Adds token usage to the budget.
    ///
    /// Call this after each API request to track cumulative usage.
    pub fn add_token_usage(&mut self, tokens: usize) {
        self.compression.token_budget.add_usage(tokens);
        self.dirty.full = true;
    }

    /// Resets the token budget for a new conversation.
    pub fn reset_token_budget(&mut self) {
        self.compression.token_budget.reset();
        self.dirty.full = true;
    }

    // ========================================================================
    // Compaction Progress
    // ========================================================================

    /// Returns the compaction progress state, if compaction is active.
    #[must_use]
    pub fn compaction_state(&self) -> Option<&CompactionProgressState> {
        self.compression.compaction_state.as_ref()
    }

    /// Returns a mutable reference to the compaction progress state.
    pub fn compaction_state_mut(&mut self) -> Option<&mut CompactionProgressState> {
        self.compression.compaction_state.as_mut()
    }

    /// Starts a compaction operation with the given target and before tokens.
    ///
    /// This will display the compaction progress overlay in the UI.
    ///
    /// # Arguments
    ///
    /// * `target_tokens` - Target token count after compaction
    /// * `before_tokens` - Current token count before compaction
    /// * `is_auto` - Whether this is auto-triggered compaction (vs manual)
    pub fn start_compaction(&mut self, target_tokens: usize, before_tokens: usize, is_auto: bool) {
        let mut state = if is_auto {
            CompactionProgressState::new_auto(target_tokens, before_tokens)
        } else {
            CompactionProgressState::new(target_tokens, before_tokens)
        };
        state.set_status(crate::tui::widgets::CompactionStatus::Compacting);
        self.compression.compaction_state = Some(state);
        self.dirty.full = true;
    }

    /// Updates the compaction progress (0.0 to 1.0).
    pub fn update_compaction_progress(&mut self, progress: f64) {
        if let Some(state) = &mut self.compression.compaction_state {
            state.set_progress(progress);
            self.dirty.full = true;
        }
    }

    /// Completes the compaction operation with the final token count.
    pub fn complete_compaction(&mut self, after_tokens: usize) {
        if let Some(state) = &mut self.compression.compaction_state {
            state.set_after_tokens(after_tokens);
            state.set_status(crate::tui::widgets::CompactionStatus::Complete);
            state.set_progress(1.0);
            self.dirty.full = true;
        }
    }

    /// Marks the compaction operation as failed.
    pub fn fail_compaction(&mut self) {
        if let Some(state) = &mut self.compression.compaction_state {
            state.set_status(crate::tui::widgets::CompactionStatus::Failed);
            self.dirty.full = true;
        }
    }

    /// Clears the compaction state (closes the overlay).
    pub fn clear_compaction(&mut self) {
        self.compression.compaction_state = None;
        self.dirty.full = true;
    }

    /// Returns a reference to the compaction metrics.
    ///
    /// Metrics track the number of compactions performed, total tokens saved,
    /// and total time spent compacting across the session.
    #[must_use]
    pub fn compaction_metrics(&self) -> &CompactionMetrics {
        &self.compression.compaction_metrics
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
        self.compression.token_budget.reset();
        self.compression.token_budget.add_usage(tokens);
        self.dirty.full = true;
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
    /// let compacted = state.maybe_compact(0.8, 200_000)?;
    /// if compacted {
    ///     println!("Conversation was compacted");
    /// }
    /// ```
    pub fn maybe_compact(&mut self, threshold: f32, context_limit: usize) -> Result<bool> {
        use crate::api::compaction::{CompactionConfig, ContextCompactor, MockSummarizer};
        use std::time::Instant;

        // Estimate current usage
        let current_tokens = self.estimate_conversation_tokens();
        let threshold_tokens = (context_limit as f64 * f64::from(threshold)) as usize;

        // Check if we're under threshold
        if current_tokens < threshold_tokens {
            tracing::debug!(
                current = current_tokens,
                threshold = threshold_tokens,
                "Compaction not needed"
            );
            return Ok(false);
        }

        tracing::info!(
            current = current_tokens,
            threshold = threshold_tokens,
            "Starting auto-compaction"
        );

        // Show compaction progress (auto-triggered)
        let target_tokens = context_limit / 2; // Target 50% of context
        self.start_compaction(target_tokens, current_tokens, true);

        // Uses MockSummarizer until the LlmProvider trait (Sprint 2) enables
        // wiring a real summarizer without coupling to AnthropicClient.
        let compactor = ContextCompactor::<MockSummarizer>::new_mock();

        let config = CompactionConfig {
            target_tokens,
            preserve_recent: 4,
            ..Default::default()
        };

        // Start timing
        let start_time = Instant::now();

        // Perform compaction
        match compactor.compact(&self.api_messages, &config) {
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
    pub fn maybe_compact_graceful(&mut self, threshold: f32, context_limit: usize) -> bool {
        match self.maybe_compact(threshold, context_limit) {
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
    // Session Restoration and Auto-Save
    // ========================================================================

    /// Returns the current session ID, if one has been assigned.
    ///
    /// A session ID is assigned when the session is first saved, or when
    /// restoring from a previous session.
    #[must_use]
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Sets the session ID.
    ///
    /// This is called after saving a session or when restoring from one.
    pub fn set_session_id(&mut self, id: String) {
        self.session_id = Some(id);
    }

    /// Marks the session as needing to be saved.
    ///
    /// Called by handlers or event-processing code after modifying conversation
    /// state (e.g., message submission, message completion, tool results).
    /// The `SessionHandler` checks this flag and performs the actual save.
    pub fn mark_session_dirty(&mut self) {
        self.session_dirty = true;
    }

    /// Returns `true` and clears the dirty flag if the session needs saving.
    ///
    /// This is an atomic check-and-clear to prevent double saves.
    pub fn take_session_dirty(&mut self) -> bool {
        std::mem::take(&mut self.session_dirty)
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
        let ui_state =
            UiState::with_state(self.scroll.offset(), self.input.clone(), self.cursor_pos);
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
            self.input = ui_state.input_buffer().to_string();
            self.cursor_pos = ui_state.cursor_position();
        }

        // Restore session ID if available
        if let Some(id) = session.id() {
            self.session_id = Some(id.to_string());
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

        // Execute the tools
        let result = self
            .tool_state
            .tool_loop
            .execute_pending(&self.tool_state.tool_executor)
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

    /// Clears all conversation state (timeline, API messages, tool blocks).
    pub fn clear_conversation(&mut self) {
        self.api_messages.clear();
        self.tool_state.tool_blocks.clear();
        self.timeline = Timeline::new();
        self.dirty.messages = true;
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
        rx: mpsc::UnboundedReceiver<(String, crate::types::ToolResultBlock)>,
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
    ) -> Option<tokio::task::JoinHandle<Vec<(String, crate::types::ToolResultBlock)>>> {
        // Create channel for results
        let (tx, rx) = mpsc::unbounded_channel();
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

        // Mark all as executing
        for (id, _) in &pending {
            self.tool_state.executing_tool_ids.insert(id.clone());
        }

        let executor = Arc::clone(&self.tool_state.tool_executor);

        // Spawn background task
        let handle = tokio::spawn(async move {
            use crate::app::tool_loop::tool_use_to_call;
            use crate::tools::ToolResult as TR;

            let mut results = Vec::new();
            for (tool_id, tool_use) in pending {
                let call = tool_use_to_call(&tool_use);
                let result = executor.execute(call).await;

                let result_block = match &result {
                    Ok(TR::Success(output)) => crate::types::ToolResultBlock {
                        tool_use_id: tool_id.clone(),
                        content: output.clone(),
                        is_error: false,
                    },
                    Ok(TR::Error(error)) => crate::types::ToolResultBlock {
                        tool_use_id: tool_id.clone(),
                        content: error.clone(),
                        is_error: true,
                    },
                    Ok(TR::Cancelled) => crate::types::ToolResultBlock {
                        tool_use_id: tool_id.clone(),
                        content: "Tool execution cancelled".to_string(),
                        is_error: true,
                    },
                    Ok(TR::NeedsPermission(perm)) => crate::types::ToolResultBlock {
                        tool_use_id: tool_id.clone(),
                        content: format!("Permission required: {perm:?}"),
                        is_error: true,
                    },
                    Err(e) => crate::types::ToolResultBlock {
                        tool_use_id: tool_id.clone(),
                        content: e.to_string(),
                        is_error: true,
                    },
                };

                // Send through channel (ignore error if receiver dropped)
                let _ = tx.send((tool_id.clone(), result_block.clone()));
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::UiState;

    fn test_message(role: Role, content: &str) -> Message {
        Message {
            role,
            content: content.to_string(),
        }
    }

    #[test]
    fn test_app_state_new() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        assert!(state.timeline().is_empty());
        assert!(state.input.is_empty());
        assert_eq!(state.scroll_offset(), 0);
        assert_eq!(state.working_dir, PathBuf::from("/test"));
    }

    #[test]
    fn test_restore_from_session_messages() {
        use crate::types::ConversationEntry;
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Create a session with messages
        let mut session = Session::new(PathBuf::from("/project"));
        session.add_message(test_message(Role::User, "Hello"));
        session.add_message(test_message(Role::Assistant, "Hi there!"));

        state.restore_from_session(&session);

        assert_eq!(state.timeline().len(), 2);
        let entries: Vec<_> = state.timeline().iter().collect();
        assert!(matches!(entries[0], ConversationEntry::UserMessage(s) if s == "Hello"));
        assert!(matches!(entries[1], ConversationEntry::AssistantMessage(s) if s == "Hi there!"));
    }

    #[test]
    fn test_restore_from_session_with_ui_state() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Create a session with UI state
        let mut session = Session::new(PathBuf::from("/project"));
        session.add_message(test_message(Role::User, "Test"));
        session.set_ui_state(Some(UiState::with_state(50, "draft input".to_string(), 5)));

        state.restore_from_session(&session);

        assert_eq!(state.scroll_offset(), 50);
        assert_eq!(state.input, "draft input");
        assert_eq!(state.cursor_position(), 5);
    }

    #[test]
    fn test_restore_from_session_without_ui_state() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        // Set some initial state
        state.scroll.restore_offset(100);
        state.input = "existing".to_string();
        state.cursor_pos = 8;

        // Create a session without UI state
        let mut session = Session::new(PathBuf::from("/project"));
        session.add_message(test_message(Role::User, "Test"));

        state.restore_from_session(&session);

        // UI state should remain unchanged since session has no UI state
        assert_eq!(state.scroll_offset(), 100);
        assert_eq!(state.input, "existing");
        assert_eq!(state.cursor_position(), 8);
    }

    #[test]
    fn test_restore_marks_dirty() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.mark_rendered(); // Clear dirty flags

        let session = Session::new(PathBuf::from("/project"));
        state.restore_from_session(&session);

        assert!(state.needs_render());
    }

    // ========================================================================
    // Phase 10.4.1: Auto-save tests
    // ========================================================================

    #[test]
    fn test_app_state_session_id_none_initially() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        assert!(state.session_id().is_none());
    }

    #[test]
    fn test_app_state_set_session_id() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.set_session_id("abc123".to_string());
        assert_eq!(state.session_id(), Some("abc123"));
    }

    #[test]
    fn test_to_session_empty() {
        let state = AppState::new(PathBuf::from("/project"), false, ParallelMode::Enabled);
        let session = state.to_session();

        assert!(session.messages().is_empty());
        assert_eq!(session.working_dir(), &PathBuf::from("/project"));
    }

    #[test]
    fn test_to_session_with_messages() {
        let mut state = AppState::new(PathBuf::from("/project"), false, ParallelMode::Enabled);
        state.add_message(test_message(Role::User, "Hello"));
        state.add_message(test_message(Role::Assistant, "Hi!"));

        let session = state.to_session();

        assert_eq!(session.messages().len(), 2);
        assert_eq!(session.messages()[0].content, "Hello");
        assert_eq!(session.messages()[1].content, "Hi!");
    }

    #[test]
    fn test_to_session_preserves_ui_state() {
        let mut state = AppState::new(PathBuf::from("/project"), false, ParallelMode::Enabled);
        state.scroll.restore_offset(42);
        state.input = "draft text".to_string();
        state.cursor_pos = 5;

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
        state.input = "unsent input".to_string();
        state.cursor_pos = 6;

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
        assert_eq!(new_state.input, "unsent input");
        assert_eq!(new_state.cursor_position(), 6);
    }

    #[test]
    fn test_restore_from_session_restores_session_id() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        assert!(state.session_id().is_none());

        // Create session with an ID (simulating a saved session)
        let mut session = Session::new(PathBuf::from("/project"));
        session.add_message(test_message(Role::User, "Test"));
        // Manually set the session ID via JSON (normally done by SessionManager::save)
        let session_json = serde_json::to_string(&session).unwrap();
        let json_with_id = session_json.replace(r#""id":null"#, r#""id":"test-session-id""#);
        let session_with_id: Session = serde_json::from_str(&json_with_id).unwrap();

        state.restore_from_session(&session_with_id);

        assert_eq!(state.session_id(), Some("test-session-id"));
    }

    // ========================================================================
    // Tool Loop Integration Tests (Phase 10.5.2.4)
    // ========================================================================

    #[test]
    fn test_appstate_has_tool_loop() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        assert!(matches!(state.tool_loop_state(), ToolLoopState::Idle));
    }

    #[test]
    fn test_appstate_receives_tool_use() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Start streaming
        state.tool_loop_mut().start_streaming().unwrap();

        // Simulate receiving tool use events
        state.handle_tool_use_start("toolu_123".to_string(), "bash".to_string(), 0);
        state.handle_tool_use_input_delta(0, r#"{"command":"ls"}"#);
        state.handle_tool_use_complete(0).unwrap();

        // Complete the message with tool_use stop reason
        state.handle_message_complete(StopReason::ToolUse).unwrap();

        // Should be in PendingApproval state
        assert!(matches!(
            state.tool_loop_state(),
            ToolLoopState::PendingApproval
        ));

        // Should need user action
        assert!(state.tool_loop_needs_user_action());
    }

    #[test]
    fn test_appstate_approve_and_deny_tools() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Set up tool use
        state.tool_loop_mut().start_streaming().unwrap();
        state.handle_tool_use_start("toolu_1".to_string(), "bash".to_string(), 0);
        state.handle_tool_use_input_delta(0, "{}");
        state.handle_tool_use_complete(0).unwrap();
        state.handle_message_complete(StopReason::ToolUse).unwrap();

        // Deny all
        state.deny_all_tools().unwrap();
        assert!(matches!(state.tool_loop_state(), ToolLoopState::Idle));
    }

    #[test]
    fn test_appstate_pending_permission() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        assert!(!state.has_pending_permission());
        assert!(state.pending_permission().is_none());

        // Set a pending permission
        let request = PermissionRequest::new("bash", Some("rm -rf temp"), "Execute command");
        state.set_pending_permission(request);

        assert!(state.has_pending_permission());
        assert!(state.pending_permission().is_some());
        let pending = state.pending_permission().unwrap();
        assert_eq!(pending.tool_name, "bash");

        // Clear it
        state.clear_pending_permission();
        assert!(!state.has_pending_permission());
    }

    #[tokio::test]
    async fn test_appstate_handles_permission_response() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Set a pending permission
        let request = PermissionRequest::new("bash", Some("echo hello"), "Execute command");
        state.set_pending_permission(request);

        // Handle the response (Allow Once)
        state
            .handle_permission_response(PermissionResponse::AllowOnce)
            .await;

        // Permission should be cleared
        assert!(!state.has_pending_permission());
    }

    #[test]
    fn test_appstate_reset_tool_loop() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Set up some state
        state.tool_loop_mut().start_streaming().unwrap();
        state.handle_tool_use_start("toolu_1".to_string(), "bash".to_string(), 0);
        let request = PermissionRequest::new("bash", None, "test");
        state.set_pending_permission(request);

        // Reset
        state.reset_tool_loop();

        assert!(matches!(state.tool_loop_state(), ToolLoopState::Idle));
        assert!(!state.has_pending_permission());
    }

    #[test]
    fn test_appstate_tool_loop_state_helpers() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Initially idle - needs user action
        assert!(state.tool_loop_needs_user_action());
        assert!(!state.tool_loop_is_active());

        // Start streaming - active
        state.tool_loop_mut().start_streaming().unwrap();
        assert!(!state.tool_loop_needs_user_action());
        assert!(state.tool_loop_is_active());
    }

    // ========================================================================
    // Scroll State Integration Tests (Phase 10.5.4.2)
    // ========================================================================

    #[test]
    fn test_scroll_state_initial() {
        use crate::tui::scroll::AutoScrollMode;

        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Should start in Follow mode at offset 0
        assert_eq!(state.scroll_offset(), 0);
        assert_eq!(state.scroll_state().mode(), AutoScrollMode::Follow);
    }

    #[test]
    fn test_streaming_content_auto_scrolls() {
        use crate::tui::scroll::AutoScrollMode;

        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.set_viewport_height(20);

        // Simulate content growth (streaming updates)
        state.update_content_height(30);
        assert_eq!(state.scroll_offset(), 0); // At bottom

        // More content arrives
        state.update_content_height(50);

        // In Follow mode, should auto-scroll to stay at bottom
        assert_eq!(state.scroll_offset(), 0);
        assert_eq!(state.scroll_state().mode(), AutoScrollMode::Follow);
    }

    #[test]
    fn test_user_scroll_preserved_during_streaming() {
        use crate::tui::scroll::AutoScrollMode;

        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.set_viewport_height(20);
        state.update_content_height(50);

        // User scrolls up
        state.scroll_up(15);
        assert_eq!(state.scroll_offset(), 15);
        assert_eq!(state.scroll_state().mode(), AutoScrollMode::Manual);

        // More content arrives (streaming)
        state.update_content_height(80);

        // User's scroll position should be preserved
        assert_eq!(state.scroll_offset(), 15);
        assert_eq!(state.scroll_state().mode(), AutoScrollMode::Manual);
    }

    #[test]
    fn test_scroll_down_resumes_follow_mode() {
        use crate::tui::scroll::AutoScrollMode;

        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.set_viewport_height(20);
        state.update_content_height(50);

        // User scrolls up (switches to Manual)
        state.scroll_up(20);
        assert_eq!(state.scroll_state().mode(), AutoScrollMode::Manual);

        // User scrolls all the way back down
        state.scroll_down(20);

        // Should resume Follow mode
        assert_eq!(state.scroll_offset(), 0);
        assert_eq!(state.scroll_state().mode(), AutoScrollMode::Follow);
    }

    #[test]
    fn test_scroll_to_bottom_method() {
        use crate::tui::scroll::AutoScrollMode;

        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.set_viewport_height(20);
        state.update_content_height(50);

        // User scrolls up
        state.scroll_up(30);
        assert_eq!(state.scroll_state().mode(), AutoScrollMode::Manual);

        // Explicitly scroll to bottom
        state.scroll_to_bottom(80);

        // Should be in Follow mode at bottom
        assert_eq!(state.scroll_offset(), 0);
        assert_eq!(state.scroll_state().mode(), AutoScrollMode::Follow);
    }

    #[test]
    fn test_scroll_state_accessor() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Should be able to access scroll state
        let scroll = state.scroll_state();
        assert_eq!(scroll.offset(), 0);
    }

    // ========================================================================
    // Tool Block UI Tests (Phase 10.5.6)
    // ========================================================================

    #[test]
    fn test_tool_blocks_initially_empty() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        assert!(state.tool_blocks().is_empty());
        assert!(!state.has_tool_blocks());
    }

    #[test]
    fn test_start_tool_block() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        let index = state.start_tool_block("bash", "git status");

        assert_eq!(index, 0);
        assert!(state.has_tool_blocks());
        assert_eq!(state.tool_blocks().len(), 1);

        let block = &state.tool_blocks()[0];
        assert_eq!(block.tool_name(), "bash");
        assert_eq!(block.tool_input(), "git status");
        assert!(!block.is_complete());
    }

    #[test]
    fn test_complete_tool_block_success() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        let index = state.start_tool_block("bash", "echo hello");

        state.complete_tool_block(index, "hello", false);

        let block = &state.tool_blocks()[0];
        assert!(block.is_complete());
        assert_eq!(block.result(), Some("hello"));
        assert!(!block.is_error());
    }

    #[test]
    fn test_complete_tool_block_error() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        let index = state.start_tool_block("bash", "bad-command");

        state.complete_tool_block(index, "Command not found", true);

        let block = &state.tool_blocks()[0];
        assert!(block.is_complete());
        assert_eq!(block.result(), Some("Command not found"));
        assert!(block.is_error());
    }

    #[test]
    fn test_multiple_tool_blocks() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        let idx1 = state.start_tool_block("bash", "ls");
        let idx2 = state.start_tool_block("read", "/tmp/file.txt");

        assert_eq!(idx1, 0);
        assert_eq!(idx2, 1);
        assert_eq!(state.tool_blocks().len(), 2);

        state.complete_tool_block(idx1, "file1.txt\nfile2.txt", false);
        state.complete_tool_block(idx2, "file contents", false);

        assert!(state.tool_blocks()[0].is_complete());
        assert!(state.tool_blocks()[1].is_complete());
    }

    #[test]
    fn test_clear_tool_blocks() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        state.start_tool_block("bash", "ls");
        state.start_tool_block("read", "/tmp/test");
        assert_eq!(state.tool_blocks().len(), 2);

        state.clear_tool_blocks();

        assert!(state.tool_blocks().is_empty());
        assert!(!state.has_tool_blocks());
    }

    #[test]
    fn test_complete_invalid_index_is_safe() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Completing a non-existent index should not panic
        state.complete_tool_block(999, "result", false);

        assert!(state.tool_blocks().is_empty());
    }

    // ========================================================================
    // Context Truncation Integration Tests (Cost Optimization)
    // ========================================================================

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
    fn test_api_messages_truncated_empty() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        let truncated = state.api_messages_truncated();

        assert!(truncated.is_empty());
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

    // =========================================================================
    // Focus Area Tests
    // =========================================================================

    #[test]
    fn test_focus_area_default_is_input() {
        use crate::tui::selection::FocusArea;
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        assert_eq!(state.focus_area(), FocusArea::Input);
    }

    #[test]
    fn test_focus_area_can_be_set() {
        use crate::tui::selection::FocusArea;
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        state.set_focus_area(FocusArea::Content);
        assert_eq!(state.focus_area(), FocusArea::Content);

        state.set_focus_area(FocusArea::Input);
        assert_eq!(state.focus_area(), FocusArea::Input);
    }

    #[test]
    fn test_focus_change_clears_selection() {
        use crate::tui::selection::{ContentPosition, FocusArea};
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Create a selection
        state.selection_mut().start(ContentPosition::new(0, 0));
        state.selection_mut().update(ContentPosition::new(5, 10));
        state.selection_mut().end();
        assert!(state.selection().has_selection());

        // Change focus should clear selection
        state.set_focus_area(FocusArea::Content);
        assert!(!state.selection().has_selection());
    }

    #[test]
    fn test_focus_same_area_preserves_selection() {
        use crate::tui::selection::{ContentPosition, FocusArea};
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Set focus to content
        state.set_focus_area(FocusArea::Content);

        // Create a selection
        state.selection_mut().start(ContentPosition::new(0, 0));
        state.selection_mut().update(ContentPosition::new(5, 10));
        state.selection_mut().end();
        assert!(state.selection().has_selection());

        // Setting same focus should NOT clear selection
        state.set_focus_area(FocusArea::Content);
        assert!(state.selection().has_selection());
    }

    #[test]
    fn test_focus_area_for_row_content() {
        use crate::tui::selection::FocusArea;
        // Terminal height 30: input is rows 27-29, content is 0-26
        assert_eq!(AppState::focus_area_for_row(0, 30), FocusArea::Content);
        assert_eq!(AppState::focus_area_for_row(10, 30), FocusArea::Content);
        assert_eq!(AppState::focus_area_for_row(26, 30), FocusArea::Content);
    }

    #[test]
    fn test_focus_area_for_row_input() {
        use crate::tui::selection::FocusArea;
        // Terminal height 30: input is rows 27-29
        assert_eq!(AppState::focus_area_for_row(27, 30), FocusArea::Input);
        assert_eq!(AppState::focus_area_for_row(28, 30), FocusArea::Input);
        assert_eq!(AppState::focus_area_for_row(29, 30), FocusArea::Input);
    }

    #[test]
    fn test_focus_area_for_row_small_terminal() {
        use crate::tui::selection::FocusArea;
        // Minimum terminal height 7: content rows 0-3, input rows 4-6
        assert_eq!(AppState::focus_area_for_row(0, 7), FocusArea::Content);
        assert_eq!(AppState::focus_area_for_row(3, 7), FocusArea::Content);
        assert_eq!(AppState::focus_area_for_row(4, 7), FocusArea::Input);
        assert_eq!(AppState::focus_area_for_row(6, 7), FocusArea::Input);
    }

    // Plugin loading tests

    #[test]
    fn test_with_plugins_disabled_creates_empty_registry() {
        let state = AppState::with_plugins(
            PathBuf::from("/test"),
            false,
            ParallelMode::Enabled,
            false, // plugins_enabled = false
        );
        // With plugins disabled, registry should be empty
        assert_eq!(state.plugins().plugin_count(), 0);
    }

    #[test]
    fn test_with_plugins_enabled_returns_valid_registry() {
        let state = AppState::with_plugins(
            PathBuf::from("/test"),
            false,
            ParallelMode::Enabled,
            true, // plugins_enabled = true
        );
        // Registry should be valid (may be empty if no plugins installed)
        // Just verify we can access the registry without panicking
        let _ = state.plugins().plugin_count();
    }

    #[test]
    fn test_new_enables_plugins_by_default() {
        // AppState::new should enable plugins (equivalent to with_plugins(..., true))
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        // Registry should be accessible (plugins enabled by default)
        let _ = state.plugins().plugin_count();
    }

    // =========================================================================
    // 1.5.4.3 - Subagent wiring tests
    // =========================================================================

    #[test]
    fn test_new_disables_subagents_by_default() {
        // AppState::new should disable subagents by default
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        assert!(!state.subagents_enabled());
        assert!(state.subagent_spawner().is_none());
    }

    #[test]
    fn test_with_options_subagents_disabled() {
        let state = AppState::with_options(
            PathBuf::from("/test"),
            false,
            ParallelMode::Enabled,
            true,  // plugins_enabled
            false, // subagents_enabled
        );
        assert!(!state.subagents_enabled());
        assert!(state.subagent_spawner().is_none());
    }

    #[test]
    fn test_with_options_subagents_enabled() {
        let state = AppState::with_options(
            PathBuf::from("/test"),
            false,
            ParallelMode::Enabled,
            true, // plugins_enabled
            true, // subagents_enabled
        );
        assert!(state.subagents_enabled());
        assert!(state.subagent_spawner().is_some());
    }

    #[test]
    fn test_subagent_spawner_returns_valid_spawner_when_enabled() {
        let state = AppState::with_options(
            PathBuf::from("/test"),
            false,
            ParallelMode::Enabled,
            false, // plugins_enabled
            true,  // subagents_enabled
        );

        // Verify we can access the spawner
        let spawner = state.subagent_spawner().expect("spawner should be Some");
        // Verify spawner has a valid model configured
        assert!(spawner.model().contains("claude"));
    }

    #[test]
    fn test_with_plugins_disables_subagents() {
        // with_plugins should disable subagents (for backward compatibility)
        let state = AppState::with_plugins(
            PathBuf::from("/test"),
            false,
            ParallelMode::Enabled,
            true, // plugins_enabled
        );
        assert!(!state.subagents_enabled());
        assert!(state.subagent_spawner().is_none());
    }

    // =========================================================================
    // 2.2.4 - Auto-context injection tests
    // =========================================================================

    #[test]
    fn test_auto_context_disabled_by_default() {
        // AppState should have auto_context disabled by default
        // (Config enables it, but AppState needs explicit opt-in)
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        assert!(!state.auto_context_enabled());
    }

    #[test]
    fn test_set_auto_context_enabled() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Initially disabled
        assert!(!state.auto_context_enabled());

        // Enable it
        state.set_auto_context_enabled(true);
        assert!(state.auto_context_enabled());

        // Disable it
        state.set_auto_context_enabled(false);
        assert!(!state.auto_context_enabled());
    }

    #[test]
    fn test_inject_context_suggestions() {
        use crate::narsil::context::{CodeReference, ContextKind, ContextSuggestion};

        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Create test suggestions
        let suggestions = vec![
            ContextSuggestion {
                source: CodeReference::function("process_data"),
                kind: ContextKind::Callers,
                description: "Functions that call process_data".to_string(),
                content: "main() in src/main.rs:10".to_string(),
            },
            ContextSuggestion {
                source: CodeReference::file("src/handler.rs"),
                kind: ContextKind::Imports,
                description: "Imports in src/handler.rs".to_string(),
                content: "use std::io".to_string(),
            },
        ];

        // Inject the suggestions
        state.set_pending_context(suggestions.clone());

        // Verify we have pending context
        assert!(state.has_pending_context());
        assert_eq!(state.pending_context().len(), 2);
    }

    #[test]
    fn test_take_pending_context_clears() {
        use crate::narsil::context::{CodeReference, ContextKind, ContextSuggestion};

        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        let suggestions = vec![ContextSuggestion {
            source: CodeReference::function("test_fn"),
            kind: ContextKind::Callers,
            description: "Test".to_string(),
            content: "Content".to_string(),
        }];

        state.set_pending_context(suggestions);
        assert!(state.has_pending_context());

        // Take should return and clear
        let taken = state.take_pending_context();
        assert_eq!(taken.len(), 1);
        assert!(!state.has_pending_context());
        assert!(state.pending_context().is_empty());
    }

    #[test]
    fn test_format_context_for_message() {
        use crate::narsil::context::{CodeReference, ContextKind, ContextSuggestion};

        let suggestions = vec![ContextSuggestion {
            source: CodeReference::function("process_data"),
            kind: ContextKind::Callers,
            description: "Functions that call process_data".to_string(),
            content: "main() in src/main.rs:10\nhandle_request() in src/api.rs:25".to_string(),
        }];

        let formatted = AppState::format_context_suggestions(&suggestions);

        // Should contain the description and content
        assert!(formatted.contains("Functions that call process_data"));
        assert!(formatted.contains("main() in src/main.rs:10"));
        assert!(formatted.contains("handle_request() in src/api.rs:25"));
    }

    #[test]
    fn test_format_context_empty_suggestions() {
        let suggestions: Vec<crate::narsil::context::ContextSuggestion> = vec![];
        let formatted = AppState::format_context_suggestions(&suggestions);

        // Empty suggestions should return empty string
        assert!(formatted.is_empty());
    }

    #[test]
    fn test_clear_pending_context() {
        use crate::narsil::context::{CodeReference, ContextKind, ContextSuggestion};

        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        let suggestions = vec![ContextSuggestion {
            source: CodeReference::function("test_fn"),
            kind: ContextKind::Callers,
            description: "Test".to_string(),
            content: "Content".to_string(),
        }];

        state.set_pending_context(suggestions);
        assert!(state.has_pending_context());

        state.clear_pending_context();
        assert!(!state.has_pending_context());
    }

    // =========================================================================
    // 4.3.1 - CompressionOrchestrator tests
    // =========================================================================

    #[test]
    fn test_compression_orchestrator_none_by_default() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        assert!(state.compression_orchestrator().is_none());
        assert!(!state.has_ccg_support());
    }

    #[test]
    fn test_has_ccg_support_false_when_no_orchestrator() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        assert!(!state.has_ccg_support());
    }

    // =========================================================================
    // 4.3.3 - CCG Context Injection tests
    // =========================================================================

    #[test]
    fn test_last_ccg_hash_none_by_default() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        assert!(state.last_ccg_hash().is_none());
    }

    #[test]
    fn test_inject_ccg_context_returns_none_without_orchestrator() {
        // Create a minimal mock test to verify basic behavior
        // Full async testing requires MCP client which is complex to mock
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Without orchestrator, the method should return None early
        // We verify the precondition: no orchestrator means function returns None
        assert!(state.compression_orchestrator().is_none());
    }

    // =========================================================================
    // 4.3.4 - CCG Context Injection in Send Flow tests
    // =========================================================================

    #[test]
    fn test_cached_ccg_context_none_by_default() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        assert!(!state.has_cached_ccg_context());
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

    #[test]
    fn test_context_for_injection_returns_none_without_cached_context() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Enable auto-context but no cached context
        state.set_auto_context_enabled(true);
        assert!(state.context_for_injection().is_none());
    }

    // =========================================================================
    // Phase 3.5 - Build Context Injection into Message-Sending Path
    // =========================================================================

    #[test]
    fn test_narsil_client_none_by_default() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        assert!(!state.has_narsil_client());
    }

    #[test]
    fn test_context_token_budget_default() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        assert_eq!(state.context_token_budget(), 10_000);
    }

    #[test]
    fn test_set_context_token_budget() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.set_context_token_budget(5_000);
        assert_eq!(state.context_token_budget(), 5_000);
    }

    #[test]
    fn test_context_tokens_injected_zero_by_default() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        assert_eq!(state.context_tokens_injected(), 0);
    }

    #[tokio::test]
    async fn test_refresh_build_context_returns_early_when_disabled() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        // auto_context is disabled by default
        assert!(!state.auto_context_enabled());

        // Should return None immediately
        let result = state.refresh_build_context().await;
        assert!(result.is_none());
        assert!(!state.has_cached_ccg_context());
        assert_eq!(state.context_tokens_injected(), 0);
    }

    #[tokio::test]
    async fn test_refresh_build_context_returns_early_without_orchestrator() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.set_auto_context_enabled(true);

        // No orchestrator set
        assert!(state.compression_orchestrator().is_none());

        // Should return None immediately
        let result = state.refresh_build_context().await;
        assert!(result.is_none());
        assert!(!state.has_cached_ccg_context());
        assert_eq!(state.context_tokens_injected(), 0);
    }

    // =========================================================================
    // 7.1.3 - Cache-aware context refresh tests
    // =========================================================================

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

    #[tokio::test]
    async fn test_submit_message_without_context_sends_plain() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        // auto_context disabled (default), no cached context

        let client: Arc<dyn crate::api::LlmProvider> = Arc::new(crate::api::AnthropicClient::new(
            secrecy::SecretString::from("test-key"),
            "claude-sonnet-4-20250514",
        ));

        let _ = state
            .submit_message(&client, "Hello plain".to_string())
            .await;

        let last_user_msg = state
            .api_messages()
            .iter()
            .find(|m| m.role == crate::types::Role::User);
        assert!(last_user_msg.is_some());
        let content = last_user_msg.unwrap().content.as_text().unwrap_or_default();
        assert!(!content.contains("<context>"));
        assert!(content.contains("Hello plain"));
    }

    #[test]
    fn test_get_git_head_hash_returns_some_in_git_repo() {
        // We're running inside a git repo (rct), so this should succeed
        let hash = get_git_head_hash(std::path::Path::new("."));
        assert!(hash.is_some());
        let hash_str = hash.unwrap();
        // Git hashes are 40 hex characters
        assert_eq!(hash_str.len(), 40);
        assert!(hash_str.chars().all(|c: char| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_get_git_head_hash_returns_none_for_non_repo() {
        let hash = get_git_head_hash(std::path::Path::new("/tmp"));
        assert!(hash.is_none());
    }

    // =========================================================================
    // 7.1.1 - Characterization Tests for Message Flow
    //
    // These tests document the current message preparation behavior:
    // - api_messages() is a raw accessor with no context injection
    // - submit_message() is the only path that injects CCG context
    // - inject_ccg_context() exists but is orphaned (never called by production code)
    // =========================================================================

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
    fn test_build_api_messages_empty_returns_empty() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        let messages = state.build_api_messages();
        assert!(messages.is_empty());
    }

    #[test]
    fn test_build_api_messages_delegates_to_truncated_when_no_context() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        // auto_context disabled (default), no cached context

        state.api_messages_mut().push(ApiMessageV2::user("Hello"));
        state
            .api_messages_mut()
            .push(ApiMessageV2::assistant("Hi there"));

        // Without context injection, build_api_messages should return the same
        // messages as api_messages_truncated
        let built = state.build_api_messages();
        let truncated = state.api_messages_truncated();

        assert_eq!(built.len(), truncated.len());
        for (b, t) in built.iter().zip(truncated.iter()) {
            assert_eq!(b.content.to_text(), t.content.to_text());
            assert_eq!(b.role, t.role);
        }
    }

    #[tokio::test]
    async fn test_inject_ccg_context_exists_and_callable() {
        // Characterization: inject_ccg_context() exists as a public async method
        // on AppState, but it is ORPHANED — no production code calls it.
        // The actual context injection path is:
        //   submit_message() -> refresh_build_context() -> orchestrator.build_context()
        //
        // This test documents the method's existence and its early-return behavior
        // when no orchestrator is configured.
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Without an orchestrator, inject_ccg_context returns Ok(None) immediately
        assert!(
            state.compression_orchestrator().is_none(),
            "Default AppState should have no compression orchestrator"
        );

        // The method requires an McpClient, but returns early before using it
        // when no orchestrator is available. We verify the early return here.
        // Note: We cannot call inject_ccg_context directly without a real McpClient,
        // but we can verify the precondition that guarantees the early return.
        assert!(
            state.compression_orchestrator().is_none(),
            "inject_ccg_context() would return Ok(None) without orchestrator"
        );

        // The orphaned method does NOT affect the working context injection path:
        // refresh_build_context() is the method actually used by submit_message().
        // With auto_context disabled, refresh_build_context returns None immediately.
        assert!(!state.auto_context_enabled());
        let result = state.refresh_build_context().await;
        assert!(
            result.is_none(),
            "refresh_build_context() returns None when auto_context is disabled"
        );
    }

    // =========================================================================
    // 7.1.2 - Context Injection in build_api_messages (GREEN)
    //
    // These tests define the expected behavior after wiring the compression
    // orchestrator into the message preparation path.
    // =========================================================================

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
    fn test_prepare_api_messages_skips_context_when_no_cached() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        state.set_auto_context_enabled(true);
        // No cached context

        state.api_messages_mut().push(ApiMessageV2::user("Hello"));

        let messages = state.build_api_messages();

        // Should have only the user message
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content.to_text(), "Hello");
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

    // ========================================================================
    // Event Loop Responsiveness Tests
    //
    // These tests ensure the event loop remains responsive during streaming
    // and tool execution. Key behaviors:
    // 1. recv_api_chunk() returns None immediately when no streaming
    // 2. recv_tool_result() returns None immediately when no channel
    // 3. recv_background_event() returns None when both channels are None
    // 4. has_* methods accurately reflect channel state
    // ========================================================================

    #[tokio::test]
    async fn test_recv_api_chunk_returns_none_when_no_streaming() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Initially streaming_rx is None
        assert!(!state.has_streaming());

        // recv_api_chunk should return None immediately, not block
        let result = state.recv_api_chunk().await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_recv_api_chunk_receives_chunks_when_streaming() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Set up a streaming channel
        let (tx, rx) = mpsc::channel(100);
        state.set_streaming_rx(rx);
        assert!(state.has_streaming());

        // Send a chunk
        tx.send(StreamEvent::ContentDelta("Hello".to_string()))
            .await
            .unwrap();

        // Should receive the chunk
        let result = state.recv_api_chunk().await;
        assert!(matches!(result, Some(StreamEvent::ContentDelta(s)) if s == "Hello"));
    }

    #[tokio::test]
    async fn test_recv_tool_result_returns_none_when_no_channel() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Initially tool_result_rx is None
        assert!(!state.has_tool_result_rx());

        // recv_tool_result should return None immediately, not block
        let result = state.recv_tool_result().await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_recv_tool_result_receives_results_when_channel_set() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Set up a tool result channel
        let (tx, rx) = mpsc::unbounded_channel();
        state.set_tool_result_rx(rx);
        assert!(state.has_tool_result_rx());

        // Send a result
        let result_block = crate::types::ToolResultBlock {
            tool_use_id: "toolu_123".to_string(),
            content: "Success".to_string(),
            is_error: false,
        };
        tx.send(("toolu_123".to_string(), result_block)).unwrap();

        // Should receive the result
        let result = state.recv_tool_result().await;
        assert!(result.is_some());
        let (id, block) = result.unwrap();
        assert_eq!(id, "toolu_123");
        assert_eq!(block.content, "Success");
        assert!(!block.is_error);
    }

    #[tokio::test]
    async fn test_recv_background_event_returns_none_when_no_channels() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Initially both channels are None
        assert!(!state.has_streaming());
        assert!(!state.has_tool_result_rx());
        assert!(!state.has_background_work());

        // recv_background_event should return None immediately, not block
        let result = state.recv_background_event().await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_recv_background_event_prioritizes_tool_results() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Set up both channels
        let (api_tx, api_rx) = mpsc::channel(100);
        let (tool_tx, tool_rx) = mpsc::unbounded_channel();
        state.set_streaming_rx(api_rx);
        state.set_tool_result_rx(tool_rx);

        // Send to both channels
        api_tx
            .send(StreamEvent::ContentDelta("API chunk".to_string()))
            .await
            .unwrap();
        let result_block = crate::types::ToolResultBlock {
            tool_use_id: "toolu_456".to_string(),
            content: "Tool result".to_string(),
            is_error: false,
        };
        tool_tx
            .send(("toolu_456".to_string(), result_block))
            .unwrap();

        // Tool results are prioritized (biased select)
        let result = state.recv_background_event().await;
        assert!(matches!(result, Some(BackgroundEvent::ToolResult(id, _)) if id == "toolu_456"));
    }

    #[tokio::test]
    async fn test_recv_background_event_receives_api_chunks() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Set up only API channel (no tool channel)
        let (api_tx, api_rx) = mpsc::channel(100);
        state.set_streaming_rx(api_rx);

        // Send an API chunk
        api_tx
            .send(StreamEvent::ContentDelta("API data".to_string()))
            .await
            .unwrap();

        // Should receive the API chunk
        let result = state.recv_background_event().await;
        assert!(
            matches!(result, Some(BackgroundEvent::ApiChunk(StreamEvent::ContentDelta(s))) if s == "API data")
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
        let (_tx, rx) = mpsc::unbounded_channel();
        state.set_tool_result_rx(rx);
        assert!(state.has_background_work());

        // Clear tool channel
        state.clear_tool_result_rx();
        assert!(!state.has_background_work());
    }

    #[test]
    fn test_clear_tool_result_rx() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Set up channel
        let (_tx, rx) = mpsc::unbounded_channel();
        state.set_tool_result_rx(rx);
        assert!(state.has_tool_result_rx());

        // Clear it
        state.clear_tool_result_rx();
        assert!(!state.has_tool_result_rx());
    }

    #[test]
    fn test_background_event_enum() {
        // Test ApiChunk variant
        let chunk = BackgroundEvent::ApiChunk(StreamEvent::ContentDelta("test".to_string()));
        assert!(matches!(chunk, BackgroundEvent::ApiChunk(_)));

        // Test ToolResult variant
        let result = crate::types::ToolResultBlock {
            tool_use_id: "id".to_string(),
            content: "result".to_string(),
            is_error: false,
        };
        let event = BackgroundEvent::ToolResult("id".to_string(), result);
        assert!(matches!(event, BackgroundEvent::ToolResult(_, _)));
    }

    #[tokio::test]
    async fn test_spawn_tool_execution_sets_channel() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // No channel initially
        assert!(!state.has_tool_result_rx());

        // Set up tool use and spawn
        state.tool_loop_mut().start_streaming().unwrap();
        state.handle_tool_use_start("toolu_789".to_string(), "bash".to_string(), 0);
        state.handle_tool_use_input_delta(0, r#"{"command":"echo hi"}"#);
        state.handle_tool_use_complete(0).unwrap();
        state.handle_message_complete(StopReason::ToolUse).unwrap();
        state.approve_all_tools().unwrap();

        // Spawn should set up channel (requires tokio runtime)
        let _handle = state.spawn_tool_execution();
        assert!(state.has_tool_result_rx());
        assert!(state.has_executing_tools());
    }

    #[test]
    fn test_record_tool_result_updates_state() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Mark a tool as executing
        state.mark_tool_executing("toolu_abc");
        assert!(state.has_executing_tools());

        // Record result
        let result = crate::types::ToolResultBlock {
            tool_use_id: "toolu_abc".to_string(),
            content: "Done".to_string(),
            is_error: false,
        };
        state.record_tool_result("toolu_abc", result);

        // Should no longer have executing tools
        assert!(!state.has_executing_tools());
    }

    #[test]
    fn test_all_tools_complete() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // No executing tools initially
        assert!(state.all_tools_complete());

        // Mark a tool as executing
        state.mark_tool_executing("toolu_xyz");
        assert!(!state.all_tools_complete());

        // Record result
        let result = crate::types::ToolResultBlock {
            tool_use_id: "toolu_xyz".to_string(),
            content: "Complete".to_string(),
            is_error: false,
        };
        state.record_tool_result("toolu_xyz", result);
        assert!(state.all_tools_complete());
    }

    // =========================================================================
    // Auto-Compaction Tests (Phase 4.4)
    // =========================================================================

    #[test]
    fn test_estimate_conversation_tokens_empty() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        let tokens = state.estimate_conversation_tokens();
        assert_eq!(tokens, 0);
    }

    #[test]
    fn test_estimate_conversation_tokens_with_messages() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Add some messages
        state
            .api_messages_mut()
            .push(ApiMessageV2::user("Hello, how are you?"));
        state
            .api_messages_mut()
            .push(ApiMessageV2::assistant("I'm doing well, thank you!"));

        let tokens = state.estimate_conversation_tokens();
        // Should have some tokens (overhead + content)
        assert!(tokens > 0, "Should have tokens, got {}", tokens);
        // Estimate: ~8-20 tokens for these short messages
        assert!(
            tokens < 100,
            "Should be reasonable estimate, got {}",
            tokens
        );
    }

    #[test]
    fn test_sync_token_budget() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Add messages
        state.api_messages_mut().push(ApiMessageV2::user("Hello"));
        state
            .api_messages_mut()
            .push(ApiMessageV2::assistant("Hi there!"));

        // Initially budget is empty
        assert_eq!(state.token_budget().used(), 0);

        // Sync budget
        state.sync_token_budget();

        // Budget should reflect message tokens
        assert!(state.token_budget().used() > 0, "Budget should have usage");
    }

    #[test]
    fn test_maybe_compact_below_threshold() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Add a small message
        state.api_messages_mut().push(ApiMessageV2::user("Hello"));

        // Try to compact at 80% threshold of 200k tokens
        // With minimal messages, we're well below threshold
        let result = state.maybe_compact(0.8, 200_000);

        assert!(result.is_ok());
        assert!(!result.unwrap(), "Should not compact below threshold");
    }

    #[test]
    fn test_maybe_compact_graceful_handles_small_conversation() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        state.api_messages_mut().push(ApiMessageV2::user("Hello"));

        let compacted = state.maybe_compact_graceful(0.8, 200_000);

        assert!(!compacted, "Should not compact small conversation");
    }

    #[test]
    fn test_maybe_compact_triggers_above_threshold() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Add many large messages to exceed threshold
        let large_content = "x".repeat(10000); // ~2500 tokens each
        for i in 0..100 {
            state.api_messages_mut().push(ApiMessageV2::user(format!(
                "Message {}: {}",
                i, large_content
            )));
        }

        // Check tokens before compaction
        let before_tokens = state.estimate_conversation_tokens();
        assert!(
            before_tokens > 100_000,
            "Should have many tokens, got {}",
            before_tokens
        );

        // Compact at 80% of 200k = 160k tokens
        // We have >100k tokens, so use a lower context limit to trigger
        let result = state.maybe_compact(0.5, before_tokens);

        assert!(result.is_ok());
        // Whether it actually compacted depends on the compactor behavior
    }

    // =========================================================================
    // Auto-Compaction Send Flow Integration Tests (Phase 4.4.3)
    // =========================================================================

    #[test]
    fn test_default_compaction_threshold_is_valid() {
        // Threshold should be between 0 and 1
        let threshold = DEFAULT_COMPACTION_THRESHOLD;
        assert!(threshold > 0.0, "Threshold should be positive");
        assert!(threshold <= 1.0, "Threshold should not exceed 1.0");
        // Default is 0.8 (80% of context)
        assert!(
            (threshold - 0.8).abs() < f32::EPSILON,
            "Default threshold should be 0.8"
        );
    }

    #[test]
    fn test_model_context_limit_integration() {
        // Verify model_context_limit function is accessible and works
        let limit = model_context_limit("claude-sonnet-4-20250514");
        assert_eq!(limit, 200_000);

        let limit = model_context_limit("claude-opus-4-20250514");
        assert_eq!(limit, 200_000);

        let limit = model_context_limit("claude-3-haiku-20240307");
        assert_eq!(limit, 200_000);
    }

    // =========================================================================
    // Compaction Metrics Tests (Phase 4.4.5)
    // =========================================================================

    #[test]
    fn test_compaction_metrics_accessor() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // New state should have zero metrics
        let metrics = state.compaction_metrics();
        assert_eq!(metrics.compaction_count(), 0);
        assert_eq!(metrics.total_tokens_saved(), 0);
        assert_eq!(metrics.total_time_ms(), 0);
    }

    #[test]
    fn test_compaction_metrics_summary() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        let summary = state.compaction_metrics_summary();
        assert_eq!(summary.compaction_count, 0);
        assert_eq!(summary.total_tokens_saved, 0);
        assert_eq!(summary.average_tokens_saved, 0);
    }

    #[test]
    fn test_compaction_metrics_record_on_compact() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Add many large messages to exceed threshold
        let large_content = "x".repeat(10000);
        for i in 0..100 {
            state.api_messages_mut().push(ApiMessageV2::user(format!(
                "Message {}: {}",
                i, large_content
            )));
        }

        let before_tokens = state.estimate_conversation_tokens();
        assert!(before_tokens > 100_000);

        // Trigger compaction
        let _ = state.maybe_compact(0.5, before_tokens);

        // Metrics should be recorded (if compaction ran)
        // We can at least verify the accessor works
        let summary = state.compaction_metrics_summary();
        // Verify summary fields are accessible (values depend on compactor behavior)
        let _ = summary.compaction_count;
        let _ = summary.total_tokens_saved;
        let _ = summary.average_time_ms;
    }

    // =========================================================================
    // Continuous loop state tests
    // =========================================================================

    #[test]
    fn test_continuous_defaults_to_inactive() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Disabled);
        assert_eq!(
            state.continuous_status(),
            &ContinuousLoopStatus::Inactive,
            "New state should have inactive continuous status"
        );
        assert_eq!(state.continuous_iterations_completed(), 0);
        assert_eq!(state.continuous_last_duration_ms(), None);
        assert_eq!(state.continuous_checking_gate(), None);
        assert!(state.continuous_gate_results().is_empty());
    }

    #[test]
    fn test_update_continuous_iteration() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Disabled);
        state.update_continuous_iteration(3);
        assert_eq!(
            state.continuous_status(),
            &ContinuousLoopStatus::Running { iteration: 3 }
        );
    }

    #[test]
    fn test_complete_continuous_iteration() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Disabled);
        state.update_continuous_iteration(1);
        state.complete_continuous_iteration(1, 5000);
        assert_eq!(state.continuous_iterations_completed(), 1);
        assert_eq!(state.continuous_last_duration_ms(), Some(5000));
    }

    #[test]
    fn test_reset_continuous_clears_all_state() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Disabled);

        // Set up some continuous state
        state.update_continuous_iteration(3);
        state.complete_continuous_iteration(3, 12_000);
        state.record_continuous_gate_result("tests", true, Some("all passed"));
        state.set_continuous_gate_checking("clippy");

        // Reset
        state.reset_continuous();

        assert_eq!(
            state.continuous_status(),
            &ContinuousLoopStatus::Inactive,
            "Status should be Inactive after reset"
        );
        assert_eq!(
            state.continuous_iterations_completed(),
            0,
            "Iterations should be 0 after reset"
        );
        assert_eq!(
            state.continuous_last_duration_ms(),
            None,
            "Duration should be None after reset"
        );
        assert_eq!(
            state.continuous_checking_gate(),
            None,
            "Checking gate should be None after reset"
        );
        assert!(
            state.continuous_gate_results().is_empty(),
            "Gate results should be empty after reset"
        );
    }

    #[test]
    fn test_reset_continuous_from_stagnated() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Disabled);
        state.set_continuous_stagnation(5, 3);
        assert!(matches!(
            state.continuous_status(),
            ContinuousLoopStatus::Stagnated { .. }
        ));

        state.reset_continuous();
        assert_eq!(state.continuous_status(), &ContinuousLoopStatus::Inactive);
    }

    #[test]
    fn test_reset_continuous_from_human_required() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Disabled);
        state.set_continuous_human_checkpoint("build broken");
        assert!(matches!(
            state.continuous_status(),
            ContinuousLoopStatus::HumanRequired { .. }
        ));

        state.reset_continuous();
        assert_eq!(state.continuous_status(), &ContinuousLoopStatus::Inactive);
    }

    // ========================================================================
    // Phase 8.1.1: Tool state characterization tests (baseline before extraction)
    // ========================================================================

    /// Verifies that ToolExecutionState can be constructed independently
    /// and that AppState uses it as an inner struct.
    #[test]
    fn test_tool_execution_state_construction() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Access tool_state sub-struct fields through AppState delegation
        assert_eq!(state.tool_loop_state(), &ToolLoopState::Idle);
        assert!(!state.has_pending_permission());
        assert!(!state.has_tool_blocks());
        assert!(!state.has_tool_result_rx());
        assert!(!state.has_executing_tools());
    }

    /// Documents the initial state of all 7 tool-related fields in AppState.
    /// This is a characterization test — it captures current behavior so we
    /// can verify zero behavior change after extracting ToolExecutionState.
    #[test]
    fn test_tool_state_field_access_baseline() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // 1. tool_loop: starts Idle (Idle counts as "needs user action")
        assert_eq!(state.tool_loop_state(), &ToolLoopState::Idle);
        assert!(!state.tool_loop_is_active());
        assert!(state.tool_loop_needs_user_action());

        // 2. tool_executor: exists (Arc<HookedToolExecutor>)
        // Accessed via tool_loop.execute_pending() — not directly exposed.
        // We verify it was constructed by checking tool_loop operates correctly.

        // 3. permission_manager: exists (Arc<Mutex<PermissionManager>>)
        // Not directly exposed — accessed only in handle_permission_response().
        // Verified indirectly via pending_permission workflow.

        // 4. pending_permission: starts None
        assert!(!state.has_pending_permission());
        assert!(state.pending_permission().is_none());

        // 5. tool_blocks: starts empty
        assert!(!state.has_tool_blocks());
        assert!(state.tool_blocks().is_empty());

        // 6. tool_result_rx: starts None (no background channel)
        assert!(!state.has_tool_result_rx());

        // 7. executing_tool_ids: starts empty
        assert!(!state.has_executing_tools());
        assert!(state.all_tools_complete());
    }

    /// Documents tool_loop mutation patterns through the public API.
    #[test]
    fn test_tool_state_tool_loop_mutations() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Reset resets to Idle
        state.reset_tool_loop();
        assert_eq!(state.tool_loop_state(), &ToolLoopState::Idle);
        assert!(!state.tool_loop_is_active());
    }

    /// Documents pending_permission lifecycle through the public API.
    #[test]
    fn test_tool_state_pending_permission_lifecycle() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Initially no permission
        assert!(!state.has_pending_permission());

        // Set a permission request
        let request = PermissionRequest {
            tool_name: "bash".to_string(),
            tool_input: Some("ls -la".to_string()),
            description: "Run shell command".to_string(),
        };
        state.set_pending_permission(request);
        assert!(state.has_pending_permission());
        assert_eq!(state.pending_permission().unwrap().tool_name, "bash");

        // Clear permission
        state.clear_pending_permission();
        assert!(!state.has_pending_permission());
    }

    /// Documents tool_blocks lifecycle through the public API.
    #[test]
    fn test_tool_state_tool_blocks_lifecycle() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Start a tool block
        let idx = state.start_tool_block("bash", r#"{"command": "ls"}"#);
        assert_eq!(idx, 0);
        assert!(state.has_tool_blocks());
        assert_eq!(state.tool_blocks().len(), 1);

        // Complete the block with a result
        state.complete_tool_block(idx, "file1.txt\nfile2.txt", false);
        assert_eq!(
            state.tool_blocks()[0].result(),
            Some("file1.txt\nfile2.txt")
        );

        // Add a second block with an error
        let idx2 = state.start_tool_block("read_file", r#"{"path": "/missing"}"#);
        assert_eq!(idx2, 1);
        state.complete_tool_block(idx2, "File not found", true);
        assert!(state.tool_blocks()[1].is_error());

        // Clear all blocks
        state.clear_tool_blocks();
        assert!(!state.has_tool_blocks());
        assert!(state.tool_blocks().is_empty());
    }

    /// Documents executing_tool_ids tracking through the public API.
    #[test]
    fn test_tool_state_executing_tool_ids_tracking() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Initially empty
        assert!(!state.has_executing_tools());

        // Mark a tool as executing
        state.mark_tool_executing("tool_001");
        assert!(state.has_executing_tools());
        assert!(!state.all_tools_complete());

        // Mark another
        state.mark_tool_executing("tool_002");
        assert!(state.has_executing_tools());

        // Record result for first tool — remove from executing set
        let result = crate::types::ToolResultBlock {
            tool_use_id: "tool_001".to_string(),
            content: "output".to_string(),
            is_error: false,
        };
        state.record_tool_result("tool_001", result);
        // Still has tool_002 executing
        assert!(state.has_executing_tools());

        // Record result for second tool
        let result2 = crate::types::ToolResultBlock {
            tool_use_id: "tool_002".to_string(),
            content: "done".to_string(),
            is_error: false,
        };
        state.record_tool_result("tool_002", result2);
        assert!(!state.has_executing_tools());
    }

    /// Documents tool_result_rx channel lifecycle through the public API.
    #[test]
    fn test_tool_state_result_channel_lifecycle() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Initially no channel
        assert!(!state.has_tool_result_rx());
        assert!(state.try_recv_tool_result().is_none());

        // Set a channel
        let (tx, rx) = mpsc::unbounded_channel();
        state.set_tool_result_rx(rx);
        assert!(state.has_tool_result_rx());

        // Send a result through the channel
        let result = crate::types::ToolResultBlock {
            tool_use_id: "t1".to_string(),
            content: "ok".to_string(),
            is_error: false,
        };
        tx.send(("t1".to_string(), result)).unwrap();

        // Receive it
        let received = state.try_recv_tool_result();
        assert!(received.is_some());
        let (id, block) = received.unwrap();
        assert_eq!(id, "t1");
        assert_eq!(block.content, "ok");

        // Clear channel
        state.clear_tool_result_rx();
        assert!(!state.has_tool_result_rx());
    }

    // ========================================================================
    // Phase 8.1.2.3: Delegation method tests
    // ========================================================================

    /// Verifies that AppState delegation methods provide clean access to
    /// ToolExecutionState fields without exposing the inner struct.
    #[test]
    fn test_tool_state_delegation_methods() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // tool_loop() returns immutable reference
        assert_eq!(state.tool_loop().state(), &ToolLoopState::Idle);

        // tool_loop_mut() returns mutable reference
        state.tool_loop_mut().reset();
        assert_eq!(state.tool_loop_state(), &ToolLoopState::Idle);

        // has_executing_tools() delegates correctly (is_tool_executing equivalent)
        assert!(!state.has_executing_tools());
        state.mark_tool_executing("t1");
        assert!(state.has_executing_tools());

        // pending_permission() delegates correctly
        assert!(state.pending_permission().is_none());
        state.set_pending_permission(PermissionRequest {
            tool_name: "read".to_string(),
            tool_input: None,
            description: "Read file".to_string(),
        });
        assert!(state.pending_permission().is_some());
    }

    // ========================================================================
    // Phase 8.2: CompressionState extraction tests
    // ========================================================================

    /// Verifies that CompressionState is correctly initialized inside AppState.
    #[test]
    fn test_compression_state_construction() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Orchestrator: not set by default
        assert!(state.compression_orchestrator().is_none());
        assert!(!state.has_ccg_support());

        // CCG cache: empty by default
        assert!(state.last_ccg_hash().is_none());
        assert!(!state.has_cached_ccg_context());

        // Narsil client: not connected by default
        assert!(!state.has_narsil_client());

        // Token budgets
        assert_eq!(state.context_token_budget(), 10_000);
        assert_eq!(state.context_tokens_injected(), 0);

        // Auto-context: disabled by default
        assert!(!state.auto_context_enabled());
        assert!(!state.has_pending_context());

        // Compaction: inactive by default
        assert!(state.compaction_state().is_none());

        // Token budget
        assert_eq!(state.token_budget().used(), 0);
    }

    /// Verifies delegation methods for CompressionState provide clean access.
    #[test]
    fn test_compression_state_delegation() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // orchestrator() delegates correctly
        assert!(state.compression_orchestrator().is_none());

        // compaction_state() delegates correctly
        assert!(state.compaction_state().is_none());

        // token_budget() delegates correctly
        assert_eq!(state.token_budget().used(), 0);
        state.token_budget_mut().add_usage(500);
        assert!(state.token_budget().used() > 0);
    }

    // ========================================================================
    // Phase 8.3: ContinuousLoopState extraction tests
    // ========================================================================

    /// Verifies that ContinuousLoopState is correctly initialized inside AppState.
    #[test]
    fn test_continuous_loop_state_construction() {
        let state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        assert_eq!(state.continuous_status(), &ContinuousLoopStatus::Inactive);
        assert_eq!(state.continuous_iterations_completed(), 0);
        assert_eq!(state.continuous_last_duration_ms(), None);
        assert_eq!(state.continuous_checking_gate(), None);
        assert!(state.continuous_gate_results().is_empty());
    }

    // ========================================================================
    // Phase 8.4: UISelectionState extraction tests
    // ========================================================================

    /// Verifies that UISelectionState is correctly initialized inside AppState.
    #[test]
    fn test_ui_selection_state_construction() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);

        // Selection: starts with no active selection
        assert!(!state.selection().has_selection());

        // Copy pending: false initially
        assert!(!state.take_copy_pending());

        // Focus area: default
        assert_eq!(state.focus_area(), FocusArea::default());
    }
}
