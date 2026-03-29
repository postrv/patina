//! Application startup, initialization, and cleanup.
//!
//! Extracts the initialization sequence from [`run()`](super::run) into focused,
//! independently testable functions. Each function handles one responsibility
//! of the startup process.
//!
//! # Startup Sequence
//!
//! 1. [`configure_terminal_environment`] -- iTerm2 keybinding config, env detection
//! 2. [`init_app_state`] -- session resume, model config, memory, compression, keybindings, MCP
//! 3. [`TerminalGuard::setup`] -- raw mode, keyboard enhancement, alternate screen
//! 4. [`spawn_background_services`] -- IDE server, update checker (fire-and-forget)
//! 5. [`fire_startup`] -- session hooks, initial prompt submission
//! 6. Event loop (in parent module)
//! 7. [`shutdown`] -- session hooks, MCP server teardown
//! 8. [`TerminalGuard::restore`] -- terminal state restoration (RAII)

use anyhow::Result;
use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::sync::Arc;
use tracing::{debug, info, warn};

use super::state::AppState;
use super::{initialize_compression_orchestrator, session_helpers, system_prompt};
use crate::api::LlmProvider;
use crate::ide::controller::IdeController;
use crate::terminal;
use crate::types::config::ResumeMode;
use crate::types::Config;

/// Configures terminal environment for the current session.
///
/// Performs iTerm2-specific keybinding setup (idempotent) and detects
/// terminal capabilities (graphics degradation, remote session).
///
/// # Side Effects
///
/// - May print iTerm2 configuration notice to stderr
/// - Logs terminal environment details
pub(crate) fn configure_terminal_environment() {
    // Configure terminal key bindings for Cmd+A/C/V on macOS iTerm2
    // This is idempotent and only modifies settings once
    match terminal::configure_iterm2_keybindings() {
        Ok(true) => {
            eprintln!("\n\u{2728} Configured iTerm2 for native Cmd+A/C/V support.");
            eprintln!("   Please restart iTerm2 for changes to take effect.\n");
        }
        Ok(false) => {}
        Err(e) => {
            warn!("Failed to configure iTerm2 bindings: {}", e);
        }
    }

    // Detect terminal environment for capability adaptation
    let term_env = terminal::detect_terminal_environment();
    if term_env.graphics_degraded() {
        info!(
            "Running inside {} \u{2014} image rendering degraded to half-blocks",
            term_env
        );
    }
    if term_env.is_remote() {
        info!("Running over SSH \u{2014} clipboard will use OSC 52 fallback");
    }
    debug!("Terminal environment: {}", term_env);
}

/// Initializes all application state from configuration.
///
/// Handles session resume (or fresh creation), then wires up:
/// - Model configuration (effort, thinking budget, system prompt)
/// - Persistent memory store
/// - Compression orchestrator (narsil CCG)
/// - Custom keybindings
/// - MCP servers
///
/// # Arguments
///
/// * `config` - Application configuration
///
/// # Errors
///
/// Returns error if session resume fails or AppState construction fails.
pub(crate) async fn init_app_state(config: &Config) -> Result<AppState> {
    // In bare mode, disable plugins and subagents for faster startup
    let plugins_enabled = config.plugins_enabled && !config.is_bare();
    let subagents_enabled = config.subagents_enabled && !config.is_bare();
    let mut state = match &config.resume_mode {
        ResumeMode::None => AppState::with_performance_config(
            config.working_dir.clone(),
            config.skip_permissions,
            &config.performance,
            plugins_enabled,
            subagents_enabled,
        ),
        ResumeMode::Last | ResumeMode::SessionId(_) => {
            session_helpers::load_session_state(config).await?
        }
    };

    // Wire effort/thinking configuration from CLI into AppState
    state.model_config_mut().set_effort(config.effort);
    state
        .model_config_mut()
        .set_thinking_budget(config.thinking_budget);
    state
        .model_config_mut()
        .set_current_model(config.model.clone());

    // Build and inject system prompt
    let prompt = system_prompt::build_system_prompt(&config.working_dir);
    state.model_config_mut().set_system_prompt(Some(prompt));

    // Load persistent memory store (skipped in bare mode)
    if !config.is_bare() {
        if let Some(project_dirs) = directories::ProjectDirs::from("com", "patina", "patina") {
            let memory_dir = crate::memory::store::default_memory_dir(project_dirs.data_dir());
            let mut store = crate::memory::store::MemoryStore::new(memory_dir);
            if let Err(e) = store.load() {
                warn!("Failed to load memory store: {}", e);
            } else {
                let count = store.list(None).len();
                if count > 0 {
                    info!("Loaded {} memory entries", count);
                }
                state.model_config_mut().set_memory_store(store);
            }
        }
    }

    // Initialize compression orchestrator for CCG context management (skipped in bare mode)
    if !config.is_bare() {
        initialize_compression_orchestrator(&mut state, config);
    }

    // Load custom keybindings from ~/.config/patina/keybindings.json if present
    if let Some(base_dirs) = directories::BaseDirs::new() {
        let keybindings_path = base_dirs.config_dir().join("patina/keybindings.json");
        if keybindings_path.exists() {
            match crate::keybindings::KeybindingManager::load_from_file(&keybindings_path) {
                Ok(mgr) => {
                    info!(
                        "Loaded custom keybindings from {}",
                        keybindings_path.display()
                    );
                    state.set_keybindings(mgr);
                }
                Err(e) => {
                    warn!(
                        "Failed to load keybindings from {}: {}",
                        keybindings_path.display(),
                        e
                    );
                }
            }
        }
    }

    // Initialize MCP servers from .mcp.json / ~/.claude.json (skipped in bare mode)
    if !config.is_bare() {
        if let Some(manager) = super::initialize_mcp_servers(&config.working_dir).await {
            state.set_mcp_manager(manager);
        }
    }

    // Load enterprise managed settings (silent no-op if files don't exist)
    load_managed_settings(&mut state);

    Ok(state)
}

/// Loads enterprise managed settings from system-wide configuration files.
///
/// Attempts to load:
/// 1. `/managed-settings.json` (base settings)
/// 2. `/managed-settings.d/*.json` (overlay files, merged alphabetically)
///
/// If settings are loaded, enforces:
/// - Default model override (if specified)
/// - Enforced permission mode (if specified)
///
/// This is a silent no-op if no managed settings files exist.
fn load_managed_settings(state: &mut super::state::AppState) {
    use crate::enterprise::managed_settings;

    // Load base settings file
    let base = match managed_settings::load_managed_settings() {
        Ok(Some(settings)) => {
            info!("Loaded enterprise managed settings");
            settings
        }
        Ok(None) => {
            debug!("No managed settings file found (this is normal)");
            // Try directory-only settings
            match managed_settings::load_managed_settings_dir() {
                Ok(overlays) if !overlays.is_empty() => {
                    let merged = managed_settings::merge_all_managed_settings(
                        managed_settings::ManagedSettings::default(),
                        overlays,
                    );
                    info!("Loaded managed settings from drop-in directory");
                    merged
                }
                Ok(_) => return, // No settings at all, silent no-op
                Err(e) => {
                    warn!("Failed to load managed settings directory: {}", e);
                    return;
                }
            }
        }
        Err(e) => {
            warn!("Failed to load managed settings: {}", e);
            return;
        }
    };

    // Merge with directory overlays if base was loaded from file
    let final_settings = if base != managed_settings::ManagedSettings::default() {
        match managed_settings::load_managed_settings_dir() {
            Ok(overlays) if !overlays.is_empty() => {
                info!("Merging {} managed settings overlay(s)", overlays.len());
                managed_settings::merge_all_managed_settings(base, overlays)
            }
            Ok(_) => base,
            Err(e) => {
                warn!("Failed to load managed settings directory: {}", e);
                base
            }
        }
    } else {
        base
    };

    // Enforce default model if specified
    if let Some(ref model) = final_settings.default_model {
        info!(model = %model, "Enforcing managed default model");
        state.model_config_mut().set_current_model(model.clone());
    }

    // Enforce permission mode if specified
    if let Some(mode) = managed_settings::get_enforced_permission_mode(&final_settings) {
        info!(mode = %mode, "Enforcing managed permission mode");
        // The permission mode is stored in managed settings for other systems to query
    }

    state.set_managed_settings(final_settings);
}

/// RAII guard for terminal state.
///
/// Manages raw mode, keyboard enhancement protocol, alternate screen, and
/// mouse capture. Call [`restore`](Self::restore) for explicit cleanup, or
/// rely on [`Drop`] for best-effort restoration on panic/early return.
///
/// # Examples
///
/// ```no_run
/// # use patina::app::startup::TerminalGuard;
/// let mut guard = TerminalGuard::setup()?;
/// // ... use guard.terminal_mut() for rendering ...
/// guard.restore()?; // explicit cleanup
/// # Ok::<(), anyhow::Error>(())
/// ```
pub struct TerminalGuard {
    terminal: Option<Terminal<CrosstermBackend<io::Stdout>>>,
    keyboard_enhancement_enabled: bool,
}

impl TerminalGuard {
    /// Sets up the terminal for TUI rendering.
    ///
    /// Enables raw mode, detects and activates keyboard enhancement protocol
    /// (kitty), enters alternate screen, and enables mouse capture.
    ///
    /// # Errors
    ///
    /// Returns error if terminal setup fails (raw mode, alternate screen, etc.).
    pub fn setup() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();

        // Check if terminal supports enhanced keyboard mode (kitty protocol)
        // This enables proper Cmd+key detection on iTerm2, kitty, WezTerm, etc.
        let is_iterm2 = terminal::is_iterm2();
        let is_jetbrains = terminal::is_jetbrains_terminal();
        let query_supported = supports_keyboard_enhancement().unwrap_or(false);
        let keyboard_enhancement_enabled = query_supported || is_iterm2;

        if keyboard_enhancement_enabled {
            execute!(
                stdout,
                PushKeyboardEnhancementFlags(
                    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                        | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
                        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
                        | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                )
            )?;
            if query_supported {
                info!("Keyboard enhancement enabled (kitty protocol) - Cmd+A/Cmd+C supported");
            } else {
                info!("Keyboard enhancement force-enabled for iTerm2 - Cmd+A/Cmd+C may work if keys configured");
            }
        } else if is_jetbrains {
            info!("JetBrains terminal detected - use Option+A/C/V or Ctrl+A/Y for clipboard");
        } else {
            info!("Keyboard enhancement not supported - use Ctrl+A/Ctrl+Y instead");
        }

        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;

        Ok(Self {
            terminal: Some(terminal),
            keyboard_enhancement_enabled,
        })
    }

    /// Returns a mutable reference to the underlying terminal.
    ///
    /// # Panics
    ///
    /// Panics if called after [`restore`](Self::restore) has consumed the terminal.
    #[must_use]
    pub fn terminal_mut(&mut self) -> &mut Terminal<CrosstermBackend<io::Stdout>> {
        self.terminal
            .as_mut()
            .expect("terminal accessed after restore()")
    }

    /// Restores the terminal to its original state.
    ///
    /// Pops keyboard enhancement flags (if enabled), disables raw mode,
    /// leaves alternate screen, disables mouse capture, and shows the cursor.
    ///
    /// # Errors
    ///
    /// Returns error if any terminal restoration step fails.
    pub fn restore(mut self) -> Result<()> {
        self.do_restore()
    }

    fn do_restore(&mut self) -> Result<()> {
        if let Some(ref mut terminal) = self.terminal {
            if self.keyboard_enhancement_enabled {
                execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags)?;
            }
            disable_raw_mode()?;
            execute!(
                terminal.backend_mut(),
                LeaveAlternateScreen,
                DisableMouseCapture
            )?;
            terminal.show_cursor()?;
        }
        self.terminal = None;
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.terminal.is_some() {
            // Best-effort cleanup on panic/early return
            let _ = self.do_restore();
        }
    }
}

/// Spawns fire-and-forget background services.
///
/// Starts the IDE integration server (if configured) and background update
/// checker (if enabled). Both run as detached tokio tasks. Skipped in bare mode.
///
/// # Arguments
///
/// * `config` - Application configuration
pub(crate) fn spawn_background_services(config: &Config) {
    // Start IDE server if port is specified (skipped in bare mode)
    if !config.is_bare() {
        if let Some(port) = config.ide_port {
            let controller = IdeController::new(port);
            tokio::spawn(async move {
                if let Err(e) = controller.run().await {
                    warn!("IDE server error: {}", e);
                }
            });
            info!("IDE server started on port {}", port);
        }
    }

    // Start background update check if enabled (skipped in bare mode)
    if config.update_check_enabled && !config.is_bare() {
        let version = env!("CARGO_PKG_VERSION").to_string();
        tokio::spawn(async move {
            match crate::update::UpdateChecker::new(&version, crate::update::ReleaseChannel::Stable)
            {
                Ok(checker) => match checker.check_for_updates().await {
                    Ok(Some(manifest)) => {
                        info!("Update available: v{}", manifest.version);
                    }
                    Ok(None) => {
                        debug!("Patina is up to date (v{})", version);
                    }
                    Err(e) => {
                        debug!("Update check failed: {}", e);
                    }
                },
                Err(e) => {
                    debug!("Could not create update checker: {}", e);
                }
            }
        });
    }
}

/// Fires startup lifecycle hooks and submits the initial prompt.
///
/// Fires the `SessionStart` hook (skipped in bare mode) and, if an initial
/// prompt was provided via CLI, submits it to begin streaming immediately.
///
/// # Arguments
///
/// * `state` - Mutable application state
/// * `client` - LLM provider for prompt submission
/// * `config` - Application configuration
///
/// # Errors
///
/// Returns error if initial prompt submission fails.
pub(crate) async fn fire_startup(
    state: &mut AppState,
    client: &Arc<dyn LlmProvider>,
    config: &Config,
) -> Result<()> {
    // Fire SessionStart hook (skipped in bare mode)
    if !config.is_bare() {
        if let Err(e) = state
            .tool_state()
            .tool_executor
            .hooks()
            .fire_session_start()
            .await
        {
            debug!("Hook fire failed: {e}");
        }
    }

    // If there's an initial prompt, submit it immediately
    if let Some(ref prompt) = config.initial_prompt {
        state.submit_message(client, prompt.clone()).await?;
    }

    Ok(())
}

/// Performs async shutdown: lifecycle hooks and MCP server teardown.
///
/// Must be called before [`TerminalGuard::restore`] since hooks may
/// produce terminal output.
///
/// # Arguments
///
/// * `state` - Mutable application state
pub(crate) async fn shutdown(state: &mut AppState) {
    // Fire SessionEnd hook
    if let Err(e) = state
        .tool_state()
        .tool_executor
        .hooks()
        .fire_session_end(None)
        .await
    {
        debug!("Hook fire failed: {e}");
    }

    // Shut down MCP servers before terminal cleanup
    if let Some(manager) = state.mcp_manager_mut() {
        manager.shutdown_all().await;
    }
}
