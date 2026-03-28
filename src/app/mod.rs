//! Application core

use anyhow::Result;
use crossterm::event::EventStream;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{io, time::Duration};
use tokio::time::interval;
use tracing::{info, warn};

pub mod commands;
pub mod completion;
pub mod context;
pub mod context_analysis;
pub mod dispatch;
pub mod events;
pub mod handlers;
pub mod print;
pub mod session_helpers;
pub mod startup;
pub mod state;
pub mod system_prompt;
pub mod tool_loop;

use state::AppState;

use crate::api::LlmProvider;
use crate::narsil::NarsilIntegration;
use crate::plugins::narsil::{has_supported_code_files, is_narsil_available};
use crate::session::{default_sessions_dir, SessionManager};
use crate::tui;
use crate::types::config::NarsilMode;

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
    state
        .compression_mut()
        .set_auto_context_enabled(config.auto_context_enabled());
    state
        .compression_mut()
        .set_context_token_budget(config.compression.max_context_tokens);
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
    let config_result = match crate::mcp::config::load_mcp_config(working_dir) {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to load MCP config: {}", e);
            return None;
        }
    };

    if config_result.servers.is_empty() {
        return None;
    }

    // Try Forge mode if configured
    if let Some(ref forge_settings) = config_result.forge {
        if crate::mcp::forge::is_forge_available(forge_settings) {
            let session_dir = std::env::temp_dir().join("patina");
            match crate::mcp::forge::ForgeContext::prepare(
                forge_settings,
                &config_result.servers,
                &session_dir,
            ) {
                Ok(context) => {
                    info!(
                        server_count = context.server_count(),
                        "Starting MCP servers via Forge gateway"
                    );
                    let timeout = Duration::from_secs(forge_settings.timeout_secs.unwrap_or(30));
                    let manager =
                        crate::mcp::manager::McpManager::start_with_forge(&context, timeout).await;

                    if manager.connected_count() > 0 {
                        info!(
                            tools = manager.tool_count(),
                            managed = context.server_count(),
                            "Forge gateway connected"
                        );
                        return Some(manager);
                    }
                    warn!("Forge gateway failed to connect, falling back to direct connections");
                }
                Err(e) => {
                    warn!("Failed to prepare Forge config: {}, falling back", e);
                }
            }
        }
    }

    // Fallback: direct connections (existing behavior)
    info!(
        server_count = config_result.servers.len(),
        "Starting MCP servers from config"
    );

    let manager =
        crate::mcp::manager::McpManager::start_all(config_result.servers, Duration::from_secs(10))
            .await;

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
    // Print mode early exit
    if config.print_mode {
        if let Some(ref prompt) = config.initial_prompt {
            return print::run_print_mode(&config, prompt).await;
        }
    }

    startup::configure_terminal_environment();

    let session_manager = SessionManager::new(default_sessions_dir()?);
    let mut state = startup::init_app_state(&config).await?;
    let mut guard = startup::TerminalGuard::setup()?;

    let mut client: std::sync::Arc<dyn LlmProvider> =
        std::sync::Arc::from(crate::api::provider_factory::create_provider(&config)?);

    startup::spawn_background_services(&config);
    startup::fire_startup(&mut state, &client, &config).await?;

    let result = event_loop(
        guard.terminal_mut(),
        &mut client,
        &mut state,
        &session_manager,
    )
    .await;

    startup::shutdown(&mut state).await;
    guard.restore()?;

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
        Box::new(handlers::permission::PermissionHandler::new()),
        Box::new(handlers::plan::PlanHandler),
        Box::new(handlers::question::QuestionHandler),
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
    client: &mut std::sync::Arc<dyn LlmProvider>,
    state: &mut AppState,
    session_manager: &SessionManager,
) -> Result<()> {
    let mut events = EventStream::new();
    let mut tick_interval = interval(Duration::from_millis(250));
    let mut dispatcher = create_dispatcher();

    // Cache initial terminal height in state for handler access.
    if let Ok(size) = terminal.size() {
        state.display_mut().set_terminal_height(size.height);
    }

    loop {
        if state.needs_render() {
            let mut feedback = None;
            terminal.draw(|frame| {
                let view = state.as_render_view();
                feedback = Some(tui::render(frame, &view));
            })?;
            if let Some(fb) = feedback {
                state.apply_render_feedback(&fb);
            }
            state.mark_rendered();
        }

        let mut ctx =
            context::AppContext::new(std::sync::Arc::clone(client), state, session_manager);
        let event = ctx.recv_event(&mut events, &mut tick_interval).await;
        let is_quit = event.is_quit();
        let _ = dispatcher.dispatch(&event, &mut ctx).await?;

        // Propagate client changes (e.g., from /model command) back to the loop.
        if !std::sync::Arc::ptr_eq(client, &ctx.client) {
            *client = ctx.client.clone();
        }

        if is_quit || ctx.state.wants_quit() {
            break;
        }
    }

    // Safety save for programmatic quit (request_quit) without Quit event.
    // SessionHandler handles Quit events, but this catches edge cases.
    session_helpers::auto_save_session(state, session_manager).await;

    Ok(())
}
