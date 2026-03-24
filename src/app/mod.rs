//! Application core

use anyhow::{Context, Result};
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
pub mod dispatch;
pub mod events;
pub mod handlers;
pub mod state;
pub mod tool_loop;

use state::AppState;
use tool_loop::ToolLoopState;

use crate::api::LlmProvider;
use crate::ide::controller::IdeController;
use crate::narsil::NarsilIntegration;
use crate::plugins::narsil::{has_supported_code_files, is_narsil_available};
use crate::session::{default_sessions_dir, SessionManager};
use crate::terminal;
use crate::tui;
use crate::types::config::{NarsilMode, ResumeMode};
use crate::types::{ApiMessageV2, Message, Role};

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

/// Result of processing a print mode stream.
enum PrintStreamResult {
    /// Stream completed successfully (MessageStop or MessageComplete).
    Completed(String),
    /// Stream ended with an error.
    Error(String),
}

/// Processes a print mode stream, printing content and handling tool use events.
///
/// Returns the accumulated response text and the final result.
async fn process_print_stream(
    rx: &mut tokio::sync::mpsc::Receiver<crate::api::StreamEvent>,
    state: &mut AppState,
) -> Result<PrintStreamResult> {
    use crate::api::StreamEvent;

    let mut response = String::new();

    while let Some(event) = rx.recv().await {
        match event {
            StreamEvent::ContentDelta(text) => {
                print!("{}", text);
                response.push_str(&text);
            }
            StreamEvent::MessageStop | StreamEvent::MessageComplete { .. } => {
                println!(); // Newline after response
                return Ok(PrintStreamResult::Completed(response));
            }
            StreamEvent::Error(e) => {
                eprintln!("Error: {}", e);
                return Ok(PrintStreamResult::Error(e));
            }
            StreamEvent::ToolUseStart { id, name, index } => {
                state.tool_loop_mut().start_streaming().ok();
                state.handle_tool_use_start(id, name, index);
            }
            StreamEvent::ToolUseInputDelta {
                index,
                partial_json,
            } => {
                state.handle_tool_use_input_delta(index, &partial_json);
            }
            StreamEvent::ToolUseComplete { index } => {
                state.handle_tool_use_complete(index)?;
            }
            _ => {}
        }
    }

    // Channel closed without explicit completion
    Ok(PrintStreamResult::Completed(response))
}

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
fn initialize_compression_orchestrator(state: &mut AppState, config: &Config) {
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
async fn initialize_mcp_servers(
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
            return run_print_mode(&config, prompt).await;
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
        ResumeMode::Last | ResumeMode::SessionId(_) => load_session_state(&config).await?,
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

/// Loads session state based on the resume mode.
async fn load_session_state(config: &Config) -> Result<AppState> {
    let sessions_dir = default_sessions_dir()?;
    let manager = SessionManager::new(sessions_dir);

    let session_id = match &config.resume_mode {
        ResumeMode::None => unreachable!("load_session_state called with ResumeMode::None"),
        ResumeMode::Last => {
            let (id, metadata) = manager
                .find_latest()
                .await?
                .context("No sessions found to resume")?;
            info!(
                session_id = %id,
                message_count = metadata.message_count,
                "Resuming most recent session"
            );
            id
        }
        ResumeMode::SessionId(id) => {
            info!(session_id = %id, "Resuming session by ID");
            id.clone()
        }
    };

    let session = manager
        .load(&session_id)
        .await
        .context(format!("Failed to load session '{}'", session_id))?;

    // Create AppState from the loaded session
    let mut state = AppState::with_performance_config(
        session.working_dir().clone(),
        config.skip_permissions,
        &config.performance,
        config.plugins_enabled,
        config.subagents_enabled,
    );
    state.restore_from_session(&session);

    Ok(state)
}

/// Runs in print mode (non-interactive).
///
/// This function:
/// 1. Sends the prompt to Claude
/// 2. Streams and prints the response to stdout
/// 3. Executes any tools Claude requests
/// 4. Continues the conversation until Claude is done
/// 5. Exits
///
/// This matches Claude Code's `-p` / `--print` flag behavior.
async fn run_print_mode(config: &Config, prompt: &str) -> Result<()> {
    use crate::api::ToolChoice;

    let client: std::sync::Arc<dyn LlmProvider> =
        std::sync::Arc::from(crate::api::provider::create_provider(config));
    let mut state = AppState::with_performance_config(
        config.working_dir.clone(),
        config.skip_permissions,
        &config.performance,
        config.plugins_enabled,
        config.subagents_enabled,
    );

    // Initialize compression orchestrator for CCG context management
    // This enables auto-context injection in print mode when narsil is available
    initialize_compression_orchestrator(&mut state, config);

    // Initialize MCP servers from .mcp.json / ~/.claude.json
    if let Some(manager) = initialize_mcp_servers(&config.working_dir).await {
        state.set_mcp_manager(manager);
    }

    // Refresh CCG context before the first API call (async, requires narsil MCP)
    state.refresh_build_context().await;

    // Add the user's prompt (adds to both display and API messages via submit logic)
    let user_msg = ApiMessageV2::user(prompt);
    state.add_message(Message {
        role: Role::User,
        content: prompt.to_string(),
    });
    state.api_messages_mut().push(user_msg);

    // Set up streaming using build_api_messages which includes context injection
    let tools = state.all_tool_definitions();
    let (tx, mut rx) = tokio::sync::mpsc::channel(STREAMING_CHANNEL_BUFFER);
    let api_messages = state.build_api_messages();
    let client_clone = std::sync::Arc::clone(&client);
    let tools_clone = tools.clone();

    tokio::spawn(async move {
        if let Err(e) = client_clone
            .stream_message(
                &api_messages,
                Some(&tools_clone),
                Some(&ToolChoice::Auto),
                tx,
            )
            .await
        {
            tracing::error!("API error: {}", e);
        }
    });

    // Collect and print the response
    let response = match process_print_stream(&mut rx, &mut state).await? {
        PrintStreamResult::Completed(text) => text,
        PrintStreamResult::Error(e) => return Err(anyhow::anyhow!("API error: {}", e)),
    };

    // If there are no tool uses, add the assistant message to both display and API
    if !response.is_empty() && !matches!(state.tool_loop_state(), ToolLoopState::PendingApproval) {
        state.add_message(Message {
            role: Role::Assistant,
            content: response.clone(),
        });
        state
            .api_messages_mut()
            .push(ApiMessageV2::assistant(&response));
    }

    // Handle any tool execution if needed
    while matches!(state.tool_loop_state(), ToolLoopState::PendingApproval) {
        // Auto-approve all tools in non-interactive mode
        state.approve_all_tools()?;

        // Execute the tools
        let needs_permission = state.execute_pending_tools().await?;

        // Check if any tools still need permission
        if !needs_permission.is_empty() {
            warn!(
                "Tools need permission in print mode (skipping): {:?}",
                needs_permission
            );
            break;
        }

        // Complete tool execution: build messages, add to history, truncate
        tool_loop::complete_tool_cycle(&mut state)?;

        let (tx, mut rx) = tokio::sync::mpsc::channel(STREAMING_CHANNEL_BUFFER);
        let api_messages = state.prepare_api_messages_for_send(client.model()).await;
        let client_clone = std::sync::Arc::clone(&client);
        let tools = state.all_tool_definitions();

        tokio::spawn(async move {
            if let Err(e) = client_clone
                .stream_message(&api_messages, Some(&tools), Some(&ToolChoice::Auto), tx)
                .await
            {
                tracing::error!("API error during tool continuation: {}", e);
            }
        });

        // Process the continuation using the same helper
        match process_print_stream(&mut rx, &mut state).await? {
            PrintStreamResult::Completed(_) => {} // Continue loop if more tools
            PrintStreamResult::Error(e) => {
                warn!("Error during tool continuation: {}", e);
                break;
            }
        }
    }

    // Shut down MCP servers
    if let Some(manager) = state.mcp_manager_mut() {
        manager.shutdown_all().await;
    }

    Ok(())
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
#[must_use]
fn create_dispatcher() -> dispatch::EventDispatcher {
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
    auto_save_session(state, session_manager).await;

    Ok(())
}

/// Auto-saves the current session.
///
/// Creates a new session or updates an existing one. Errors are logged
/// but do not interrupt the application flow.
async fn auto_save_session(state: &mut AppState, session_manager: &SessionManager) {
    let session = state.to_session();

    let result = if let Some(existing_id) = state.session_id() {
        // Update existing session
        session_manager
            .update(existing_id, &session)
            .await
            .map(|()| existing_id.to_string())
    } else {
        // Create new session
        session_manager.save(&session).await
    };

    match result {
        Ok(id) => {
            if state.session_id().is_none() {
                debug!(session_id = %id, "Created new session");
                state.set_session_id(id);
            } else {
                debug!(session_id = %id, "Updated session");
            }
        }
        Err(e) => {
            warn!(error = %e, "Failed to auto-save session");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{AnthropicClient, LlmProvider};
    use crate::app::context::AppContext;
    use crate::app::dispatch::Handled;
    use crate::app::events::AppEvent;
    use crate::permissions::PermissionRequest;
    use crate::session::SessionManager;
    use crate::types::config::ParallelMode;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
    use secrecy::SecretString;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::TempDir;

    // =========================================================================
    // Test helpers
    // =========================================================================

    fn test_client() -> Arc<dyn LlmProvider> {
        Arc::new(AnthropicClient::new(
            SecretString::from("test-key"),
            "claude-test",
        ))
    }

    fn test_state() -> AppState {
        AppState::new(PathBuf::from("/tmp/test"), true, ParallelMode::Disabled)
    }

    fn test_session_manager() -> (SessionManager, TempDir) {
        let dir = TempDir::new().expect("failed to create temp dir");
        let mgr = SessionManager::new(dir.path().to_path_buf());
        (mgr, dir)
    }

    // =========================================================================
    // Compression orchestrator initialization tests
    // =========================================================================

    #[test]
    fn test_initialize_compression_orchestrator_disabled_mode() {
        use secrecy::SecretString;

        let mut state = AppState::new(PathBuf::from("/tmp"), false, ParallelMode::Enabled);
        let config = Config::new(
            SecretString::new("test".into()),
            "model",
            PathBuf::from("/tmp"),
        )
        .with_narsil_mode(NarsilMode::Disabled);

        // Disabled mode should never set orchestrator
        initialize_compression_orchestrator(&mut state, &config);
        assert!(state.compression_orchestrator().is_none());
    }

    #[test]
    fn test_initialize_compression_orchestrator_auto_mode_no_code_files() {
        use secrecy::SecretString;
        use tempfile::tempdir;

        // Create a temp dir with no code files
        let temp = tempdir().unwrap();
        let mut state = AppState::new(temp.path().to_path_buf(), false, ParallelMode::Enabled);
        let config = Config::new(
            SecretString::new("test".into()),
            "model",
            temp.path().to_path_buf(),
        )
        .with_narsil_mode(NarsilMode::Auto);

        // Auto mode with no code files should not set orchestrator
        initialize_compression_orchestrator(&mut state, &config);
        // Result depends on is_narsil_available() AND has_supported_code_files()
        // Since temp dir has no code files, orchestrator should be None
        assert!(state.compression_orchestrator().is_none());
    }

    // =========================================================================
    // create_dispatcher() tests — Phase 1.10 integration
    // =========================================================================

    #[test]
    fn create_dispatcher_returns_seven_handlers_and_one_observer() {
        let dispatcher = create_dispatcher();
        assert_eq!(
            dispatcher.handler_count(),
            7,
            "Dispatcher must have 7 handlers: Permission, Completion, Keyboard, Stream, Agent, Continuous, Tick"
        );
        assert_eq!(
            dispatcher.observer_count(),
            1,
            "Dispatcher must have 1 observer: Session"
        );
    }

    // =========================================================================
    // Full dispatcher integration tests — all handlers working together
    // =========================================================================

    #[tokio::test]
    async fn dispatcher_quit_event_triggers_session_save() {
        let mut dispatcher = create_dispatcher();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        // Quit event should be dispatched to all handlers (none consume it
        // except SessionHandler which observes and returns IGNORED).
        let event = AppEvent::Quit;
        let result = dispatcher.dispatch(&event, &mut ctx).await.unwrap();

        // SessionHandler always returns IGNORED, so overall result is IGNORED.
        assert_eq!(
            result,
            Handled::IGNORED,
            "Quit event should not be consumed (SessionHandler returns IGNORED)"
        );

        // SessionHandler must have saved the session on Quit.
        assert!(
            ctx.state.session_id().is_some(),
            "SessionHandler must auto-save session on Quit event"
        );
    }

    #[tokio::test]
    async fn dispatcher_char_input_inserts_into_state() {
        let mut dispatcher = create_dispatcher();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        let result = dispatcher.dispatch(&event, &mut ctx).await.unwrap();

        assert_eq!(result, Handled::CONSUMED);
        assert_eq!(
            ctx.state.input(),
            "x",
            "Character input must flow through to KeyboardHandler and insert into input"
        );
    }

    #[tokio::test]
    async fn dispatcher_tick_advances_throbber() {
        let mut dispatcher = create_dispatcher();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        let before = state.throbber_char();

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Tick;
        let result = dispatcher.dispatch(&event, &mut ctx).await.unwrap();

        assert_eq!(result, Handled::CONSUMED);
        assert_ne!(
            ctx.state.throbber_char(),
            before,
            "Tick event must advance throbber via TickHandler"
        );
    }

    #[tokio::test]
    async fn dispatcher_api_chunk_content_delta_consumed() {
        let mut dispatcher = create_dispatcher();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::ApiChunk(crate::api::StreamEvent::ContentDelta("hello".to_string()));
        let result = dispatcher.dispatch(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "ApiChunk must be consumed by StreamHandler"
        );
    }

    #[tokio::test]
    async fn dispatcher_message_complete_marks_dirty_and_saves() {
        let mut dispatcher = create_dispatcher();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        // MessageComplete should: StreamHandler marks dirty → SessionHandler saves.
        let event = AppEvent::ApiChunk(crate::api::StreamEvent::MessageComplete {
            stop_reason: crate::types::StopReason::EndTurn,
        });
        let result = dispatcher.dispatch(&event, &mut ctx).await.unwrap();

        assert_eq!(result, Handled::CONSUMED);

        // SessionHandler should have observed the dirty flag and saved.
        assert!(
            ctx.state.session_id().is_some(),
            "MessageComplete → StreamHandler marks dirty → SessionHandler saves"
        );
    }

    #[tokio::test]
    async fn dispatcher_permission_key_consumed_before_keyboard() {
        let mut dispatcher = create_dispatcher();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // Set up a pending permission.
        state.set_pending_permission(PermissionRequest::new("Bash", Some("ls"), "List"));

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        // Send 'z' key — should be consumed by PermissionHandler, NOT
        // reach KeyboardHandler (which would insert 'z' into input).
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        let result = dispatcher.dispatch(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "Key must be consumed by PermissionHandler when permission is pending"
        );
        assert_eq!(
            ctx.state.input(),
            "",
            "Key must NOT reach KeyboardHandler when permission is pending"
        );
    }

    #[tokio::test]
    async fn dispatcher_resize_marks_redraw() {
        let mut dispatcher = create_dispatcher();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // Clear initial render flags.
        state.mark_rendered();

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Resize {
            width: 120,
            height: 40,
        };
        let result = dispatcher.dispatch(&event, &mut ctx).await.unwrap();

        assert_eq!(result, Handled::CONSUMED);
        assert!(
            ctx.state.needs_render(),
            "Resize must mark UI for redraw via KeyboardHandler"
        );
    }

    #[tokio::test]
    async fn dispatcher_mouse_scroll_consumed() {
        let mut dispatcher = create_dispatcher();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        let event = AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        });
        let result = dispatcher.dispatch(&event, &mut ctx).await.unwrap();

        assert_eq!(
            result,
            Handled::CONSUMED,
            "Mouse events must be consumed by KeyboardHandler"
        );
    }

    #[tokio::test]
    async fn dispatcher_ctrl_c_as_quit_saves_and_is_detectable() {
        let mut dispatcher = create_dispatcher();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        // In the real event loop, Ctrl+C is mapped to AppEvent::Quit by recv_event().
        // Dispatching Quit should save the session (via SessionHandler).
        let event = AppEvent::Quit;
        let is_quit = event.is_quit();
        let _result = dispatcher.dispatch(&event, &mut ctx).await.unwrap();

        assert!(is_quit, "AppEvent::Quit must report is_quit() == true");
        assert!(
            ctx.state.session_id().is_some(),
            "Quit dispatch must trigger session save"
        );
    }

    #[tokio::test]
    async fn dispatcher_keyboard_quit_sets_wants_quit() {
        let mut dispatcher = create_dispatcher();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);

        // If a Key event for Ctrl+C reaches KeyboardHandler (bypassing recv_event's
        // Quit mapping), it should set wants_quit on state.
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        let result = dispatcher.dispatch(&event, &mut ctx).await.unwrap();

        assert_eq!(result, Handled::CONSUMED);
        assert!(
            ctx.state.wants_quit(),
            "Ctrl+C Key event must set wants_quit via KeyboardHandler"
        );
    }

    #[tokio::test]
    async fn dispatcher_multiple_events_sequence() {
        let mut dispatcher = create_dispatcher();

        let client = test_client();
        let mut state = test_state();
        let (session_mgr, _dir) = test_session_manager();

        // Simulate a sequence: type "hi", then get an API chunk, then tick.
        let events = vec![
            AppEvent::Key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE)),
            AppEvent::Key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE)),
            AppEvent::ApiChunk(crate::api::StreamEvent::ContentDelta(
                "response".to_string(),
            )),
            AppEvent::Tick,
        ];

        for event in &events {
            let mut ctx = AppContext::new(Arc::clone(&client), &mut state, &session_mgr);
            let result = dispatcher.dispatch(event, &mut ctx).await.unwrap();
            assert_eq!(result, Handled::CONSUMED, "Event {event} must be consumed");
        }

        assert_eq!(state.input(), "hi", "Both characters must be inserted");
    }
}
