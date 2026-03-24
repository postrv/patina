//! Application core

use anyhow::Result;
use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, EventStream, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{io, time::Duration};
use tokio::time::interval;
use tracing::{debug, info, warn};

pub mod commands;
pub mod completion;
pub mod context;
pub mod context_analysis;
pub mod dispatch;
pub mod events;
pub mod handlers;
pub mod print;
pub mod session_helpers;
pub mod state;
pub mod tool_loop;

use state::AppState;

use crate::api::LlmProvider;
use crate::ide::controller::IdeController;
use crate::narsil::NarsilIntegration;
use crate::plugins::narsil::{has_supported_code_files, is_narsil_available};
use crate::session::{default_sessions_dir, SessionManager};
use crate::terminal;
use crate::tui;
use crate::types::config::{NarsilMode, ResumeMode};

// Re-export Config for backward compatibility
pub use crate::types::Config;

/// Buffer size for API streaming channels.
///
/// This value needs to be large enough to handle rapid tool execution iterations
/// where Claude generates many events faster than the event loop can process them.
/// A small buffer (e.g., 100) can cause backpressure and block the API streaming task,
/// leading to UI freezing during intensive tool execution.
///
/// Setting to 1000 provides headroom for ~50 tool_use events (each generates ~20 events)
/// without causing backpressure.
pub const STREAMING_CHANNEL_BUFFER: usize = 1000;

/// Initializes the compression orchestrator based on narsil configuration.
///
/// This function checks the narsil mode and creates a `CompressionOrchestrator`
/// if narsil integration should be enabled. The orchestrator is then set on
/// the `AppState` for CCG context management.
///
/// # Arguments
///
/// * `state` - Mutable reference to the application state
/// * `config` - Application configuration containing narsil mode
///
/// # Behavior
///
/// - `NarsilMode::Auto`: Enables if narsil-mcp is in PATH and project has code files
/// - `NarsilMode::Enabled`: Always tries to enable (logs warning if unavailable)
/// - `NarsilMode::Disabled`: Does nothing
#[doc(hidden)]
pub fn initialize_compression_orchestrator(state: &mut AppState, config: &Config) {
    let should_enable = match config.narsil_mode() {
        NarsilMode::Disabled => false,
        NarsilMode::Enabled => {
            if !is_narsil_available() {
                warn!("narsil-mcp not found in PATH, compression orchestrator unavailable");
                false
            } else {
                true
            }
        }
        NarsilMode::Auto => {
            // Auto-detect: enable if narsil is available AND project has supported code files
            is_narsil_available() && has_supported_code_files(&config.working_dir)
        }
    };

    if should_enable {
        // Create NarsilIntegration with detected tools
        // Since we're not connected to MCP yet, create with empty tool list
        // The capabilities will be discovered when MCP tools are registered
        let integration = NarsilIntegration::new(&config.working_dir);
        let orchestrator = integration.create_compression_orchestrator();
        state.set_compression_orchestrator(orchestrator);
        info!(
            "Compression orchestrator initialized for {}",
            config.working_dir.display()
        );
    }

    // Wire auto-context settings from config into state
    state.set_auto_context_enabled(config.auto_context_enabled());
    state.set_context_token_budget(config.compression.max_context_tokens);
}

/// Initializes MCP servers from config files.
///
/// Loads `.mcp.json` (project-local) and `~/.claude.json` (user-global),
/// starts all configured servers in parallel with a 10-second timeout,
/// and returns a manager if any servers connected successfully.
///
/// # Arguments
///
/// * `working_dir` - The project root to search for `.mcp.json`
pub(crate) async fn initialize_mcp_servers(
    working_dir: &std::path::Path,
) -> Option<crate::mcp::manager::McpManager> {
    let configs = match crate::mcp::config::load_mcp_config(working_dir) {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to load MCP config: {}", e);
            return None;
        }
    };

    if configs.is_empty() {
        return None;
    }

    info!(
        server_count = configs.len(),
        "Starting MCP servers from config"
    );

    let manager =
        crate::mcp::manager::McpManager::start_all(configs, Duration::from_secs(10)).await;

    // Log server statuses
    for (name, status) in manager.server_statuses() {
        if status.is_connected() {
            info!(server = %name, "MCP server connected");
        } else {
            warn!(server = %name, status = ?status, "MCP server failed to connect");
        }
    }

    if manager.connected_count() > 0 {
        info!(
            connected = manager.connected_count(),
            tools = manager.tool_count(),
            "MCP servers initialized"
        );
        Some(manager)
    } else {
        info!("No MCP servers connected, continuing without MCP");
        None
    }
}

pub async fn run(config: Config) -> Result<()> {
    // If print mode is enabled with an initial prompt, run non-interactively
    if config.print_mode {
        if let Some(ref prompt) = config.initial_prompt {
            return print::run_print_mode(&config, prompt).await;
        }
    }

    // Configure terminal key bindings for Cmd+A/C/V on macOS iTerm2
    // This is idempotent and only modifies settings once
    match terminal::configure_iterm2_keybindings() {
        Ok(true) => {
            // Changes were made - tell user to restart iTerm2
            eprintln!("\n✨ Configured iTerm2 for native Cmd+A/C/V support.");
            eprintln!("   Please restart iTerm2 for changes to take effect.\n");
        }
        Ok(false) => {
            // No changes needed (already configured or not iTerm2)
        }
        Err(e) => {
            warn!("Failed to configure iTerm2 bindings: {}", e);
        }
    }

    // Detect terminal environment for capability adaptation
    let term_env = terminal::detect_terminal_environment();
    if term_env.graphics_degraded() {
        info!(
            "Running inside {} — image rendering degraded to half-blocks",
            term_env
        );
    }
    if term_env.is_remote() {
        info!("Running over SSH — clipboard will use OSC 52 fallback");
    }
    debug!("Terminal environment: {}", term_env);

    // Initialize session manager for auto-save
    let sessions_dir = default_sessions_dir()?;
    let session_manager = SessionManager::new(sessions_dir);

    // Check for session resume before initializing terminal
    let mut state = match &config.resume_mode {
        ResumeMode::None => AppState::with_performance_config(
            config.working_dir.clone(),
            config.skip_permissions,
            &config.performance,
            config.plugins_enabled,
            config.subagents_enabled,
        ),
        ResumeMode::Last | ResumeMode::SessionId(_) => {
            session_helpers::load_session_state(&config).await?
        }
    };

    // Initialize compression orchestrator for CCG context management
    initialize_compression_orchestrator(&mut state, &config);

    // Initialize MCP servers from .mcp.json / ~/.claude.json
    if let Some(manager) = initialize_mcp_servers(&config.working_dir).await {
        state.set_mcp_manager(manager);
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();

    // Check if terminal supports enhanced keyboard mode (kitty protocol)
    // This enables proper Cmd+key detection on iTerm2, kitty, WezTerm, etc.
    // Full enhancement flags are required for SUPER (Cmd) modifier detection.
    //
    // Note: supports_keyboard_enhancement() may return false on some terminals
    // (like iTerm2) even when they actually support the protocol with proper
    // configuration. We force-enable on known-good terminals.
    let is_iterm2 = terminal::is_iterm2();
    let is_jetbrains = terminal::is_jetbrains_terminal();
    let query_supported = supports_keyboard_enhancement().unwrap_or(false);
    let keyboard_enhancement_supported = query_supported || is_iterm2;

    if keyboard_enhancement_supported {
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
        // JetBrains terminals (JediTerm) don't support Kitty protocol
        // Cmd+keys are intercepted by the IDE before reaching the terminal
        info!("JetBrains terminal detected - use Option+A/C/V or Ctrl+A/Y for clipboard");
    } else {
        info!("Keyboard enhancement not supported - use Ctrl+A/Ctrl+Y instead");
    }

    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let client: std::sync::Arc<dyn LlmProvider> =
        std::sync::Arc::from(crate::api::provider::create_provider(&config));

    // Start IDE server if port is specified
    if let Some(port) = config.ide_port {
        let controller = IdeController::new(port);
        tokio::spawn(async move {
            if let Err(e) = controller.run().await {
                warn!("IDE server error: {}", e);
            }
        });
        info!("IDE server started on port {}", port);
    }

    // If there's an initial prompt, submit it immediately
    if let Some(ref prompt) = config.initial_prompt {
        state.submit_message(&client, prompt.clone()).await?;
    }

    let result = event_loop(&mut terminal, &client, &mut state, &session_manager).await;

    // Shut down MCP servers before terminal cleanup
    if let Some(manager) = state.mcp_manager_mut() {
        manager.shutdown_all().await;
    }

    // Clean up terminal state
    if keyboard_enhancement_supported {
        execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags)?;
    }
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

/// Creates the event dispatcher with handlers in priority order.
///
/// Handler priority (highest first):
/// 1. [`PermissionHandler`](handlers::permission::PermissionHandler) — intercepts keys during permission prompts
/// 2. [`CompletionHandler`](handlers::completion::CompletionHandler) — slash command auto-completion navigation
/// 3. [`KeyboardHandler`](handlers::keyboard::KeyboardHandler) — user input (keys, mouse, resize)
/// 4. [`StreamHandler`](handlers::stream::StreamHandler) — API chunks and tool results
/// 5. [`AgentHandler`](handlers::agent::AgentHandler) — background agent events
/// 6. [`ContinuousHandler`](handlers::continuous::ContinuousHandler) — continuous loop progress
/// 7. [`TickHandler`](handlers::tick::TickHandler) — throbber animation
/// 8. [`SessionHandler`](handlers::session::SessionHandler) — auto-save observer (always last, never consumes)
#[doc(hidden)]
#[must_use]
pub fn create_dispatcher() -> dispatch::EventDispatcher {
    dispatch::EventDispatcher::new(vec![
        Box::new(handlers::permission::PermissionHandler),
        Box::new(handlers::completion::CompletionHandler),
        Box::new(handlers::keyboard::KeyboardHandler),
        Box::new(handlers::stream::StreamHandler),
        Box::new(handlers::agent::AgentHandler),
        Box::new(handlers::continuous::ContinuousHandler),
        Box::new(handlers::tick::TickHandler),
    ])
    .with_observers(vec![Box::new(handlers::session::SessionHandler)])
}

/// The main event loop using the dispatched handler architecture.
///
/// Replaces the monolithic `tokio::select!` with a unified
/// `recv_event()` + `EventDispatcher::dispatch()` loop. Events are
/// received from all sources (crossterm, background channels, tick timer)
/// via [`AppContext::recv_event`] and dispatched to handlers in priority
/// order via [`EventDispatcher::dispatch`].
///
/// # Exit conditions
///
/// The loop exits when:
/// - [`AppEvent::Quit`] is received (Ctrl+C or Ctrl+D via `recv_event`)
/// - [`AppState::wants_quit`] returns `true` (programmatic quit request)
///
/// # Errors
///
/// Propagates errors from rendering, event handling, or session saving.
async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    client: &std::sync::Arc<dyn LlmProvider>,
    state: &mut AppState,
    session_manager: &SessionManager,
) -> Result<()> {
    let mut events = EventStream::new();
    let mut tick_interval = interval(Duration::from_millis(250));
    let mut dispatcher = create_dispatcher();

    // Cache initial terminal height in state for handler access.
    if let Ok(size) = terminal.size() {
        state.set_terminal_height(size.height);
    }

    loop {
        if state.needs_render() {
            terminal.draw(|frame| tui::render(frame, state))?;
            state.mark_rendered();
        }

        let mut ctx =
            context::AppContext::new(std::sync::Arc::clone(client), state, session_manager);
        let event = ctx.recv_event(&mut events, &mut tick_interval).await;
        let is_quit = event.is_quit();
        let _ = dispatcher.dispatch(&event, &mut ctx).await?;

        if is_quit || ctx.state.wants_quit() {
            break;
        }
    }

    // Safety save for programmatic quit (request_quit) without Quit event.
    // SessionHandler handles Quit events, but this catches edge cases.
    session_helpers::auto_save_session(state, session_manager).await;

    Ok(())
}
