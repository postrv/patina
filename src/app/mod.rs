//! Application core

use anyhow::{Context, Result};
use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyModifiers,
        KeyboardEnhancementFlags, MouseButton, MouseEventKind, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};
use futures::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{io, time::Duration};
use tokio::time::interval;
use tracing::{debug, info, warn};

pub mod commands;
pub mod state;
pub mod tool_loop;

use state::AppState;
use tool_loop::ToolLoopState;

use crate::api::AnthropicClient;
use crate::permissions::PermissionResponse;
use crate::session::{default_sessions_dir, SessionManager};
use crate::terminal;
use crate::tui;
use crate::tui::selection::{ContentPosition, FocusArea};
use crate::tui::widgets::handle_permission_key;
use crate::tui::widgets::permission_prompt::PermissionPromptState;
use crate::types::config::ResumeMode;
use crate::types::{ApiMessageV2, Message, Role};

// Re-export Config for backward compatibility
pub use crate::types::Config;

/// Handles copy operation with detailed logging.
///
/// Copies the current selection to clipboard and logs the result.
fn handle_copy(state: &AppState) {
    let selection = state.selection();
    let cache_len = state.rendered_line_count();

    if let Some((start, end)) = selection.range() {
        let selected_lines = end.line.saturating_sub(start.line) + 1;
        debug!(
            start_line = start.line,
            end_line = end.line,
            selected_lines,
            cache_len,
            "copy: attempting to copy {} lines from cache of {} lines",
            selected_lines,
            cache_len
        );

        match state.copy_from_cache() {
            Ok(true) => {
                info!("Copied {} lines to clipboard", selected_lines);
            }
            Ok(false) => {
                warn!(
                    "copy: no text extracted (cache_len={}, selection=L{}-L{})",
                    cache_len, start.line, end.line
                );
            }
            Err(e) => {
                warn!("copy: clipboard error: {}", e);
            }
        }
    } else {
        debug!(
            "copy: no selection (has_selection={})",
            selection.has_selection()
        );
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

    // Initialize session manager for auto-save
    let sessions_dir = default_sessions_dir()?;
    let session_manager = SessionManager::new(sessions_dir);

    // Check for session resume before initializing terminal
    let mut state = match &config.resume_mode {
        ResumeMode::None => AppState::with_plugins(
            config.working_dir.clone(),
            config.skip_permissions,
            config.parallel_mode,
            config.plugins_enabled,
        ),
        ResumeMode::Last | ResumeMode::SessionId(_) => {
            load_session_state(&config, config.skip_permissions, config.parallel_mode).await?
        }
    };

    enable_raw_mode()?;
    let mut stdout = io::stdout();

    // Check if terminal supports enhanced keyboard mode (kitty protocol)
    // This enables proper Cmd+key detection on iTerm2, kitty, WezTerm, etc.
    // Full enhancement flags are required for SUPER (Cmd) modifier detection.
    let keyboard_enhancement_supported = supports_keyboard_enhancement().unwrap_or(false);
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
        info!("Keyboard enhancement enabled (kitty protocol) - Cmd+A/Cmd+C supported");
    } else {
        info!("Keyboard enhancement not supported - use Ctrl+A/Ctrl+Y instead");
    }

    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let client = AnthropicClient::new(config.api_key.clone(), &config.model);

    // If there's an initial prompt, submit it immediately
    if let Some(ref prompt) = config.initial_prompt {
        state.submit_message(&client, prompt.clone()).await?;
    }

    let result = event_loop(&mut terminal, &client, &mut state, &session_manager).await;

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
async fn load_session_state(
    config: &Config,
    skip_permissions: bool,
    parallel_mode: crate::types::config::ParallelMode,
) -> Result<AppState> {
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
    let mut state = AppState::with_plugins(
        session.working_dir().clone(),
        skip_permissions,
        parallel_mode,
        config.plugins_enabled,
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
    use crate::api::tools::default_tools;
    use crate::api::{StreamEvent, ToolChoice};

    let client = AnthropicClient::new(config.api_key.clone(), &config.model);
    let mut state = AppState::with_plugins(
        config.working_dir.clone(),
        config.skip_permissions,
        config.parallel_mode,
        config.plugins_enabled,
    );

    // Add the user's prompt (adds to both display and API messages via submit logic)
    let user_msg = ApiMessageV2::user(prompt);
    state.add_message(Message {
        role: Role::User,
        content: prompt.to_string(),
    });
    state.api_messages_mut().push(user_msg);

    // Set up streaming using API messages
    let tools = default_tools();
    let (tx, mut rx) = tokio::sync::mpsc::channel(100);
    let api_messages = state.api_messages().to_vec();
    let client_clone = client.clone();
    let tools_clone = tools.clone();

    tokio::spawn(async move {
        if let Err(e) = client_clone
            .stream_message_v2_with_tools(
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
    let mut response = String::new();

    while let Some(event) = rx.recv().await {
        match event {
            StreamEvent::ContentDelta(text) => {
                print!("{}", text);
                response.push_str(&text);
            }
            StreamEvent::MessageStop | StreamEvent::MessageComplete { .. } => {
                println!(); // Newline after response
                break;
            }
            StreamEvent::Error(e) => {
                eprintln!("Error: {}", e);
                return Err(anyhow::anyhow!("API error: {}", e));
            }
            // Handle tool use events for the tool loop
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

        // Finish execution and get continuation data
        let continuation = state.finish_tool_execution()?;

        // Build the messages for the conversation
        let (assistant_msg, user_msg) = continuation.build_messages();

        // Add to API message history for conversation continuation
        // Note: The assistant message is NOT added to the timeline here because
        // finalize_streaming_for_tool_use() already converted the streaming entry
        // to an AssistantMessage. Adding it again would cause duplicate messages.
        state.api_messages_mut().push(assistant_msg);

        // Add tool results to both timeline (for display) and API (for continuation)
        let tool_result_summary = format_tool_results_for_display(&user_msg);
        state.add_message(Message {
            role: Role::User,
            content: tool_result_summary,
        });
        state.api_messages_mut().push(user_msg);

        state.tool_loop_mut().start_streaming()?;

        let (tx, mut rx) = tokio::sync::mpsc::channel(100);
        let api_messages = state.api_messages().to_vec();
        let client_clone = client.clone();
        let tools = default_tools();

        tokio::spawn(async move {
            if let Err(e) = client_clone
                .stream_message_v2_with_tools(
                    &api_messages,
                    Some(&tools),
                    Some(&ToolChoice::Auto),
                    tx,
                )
                .await
            {
                tracing::error!("API error during tool continuation: {}", e);
            }
        });

        // Process the continuation
        response.clear();
        while let Some(event) = rx.recv().await {
            match event {
                StreamEvent::ContentDelta(text) => {
                    print!("{}", text);
                    response.push_str(&text);
                }
                StreamEvent::MessageStop | StreamEvent::MessageComplete { .. } => {
                    println!();
                    break;
                }
                StreamEvent::ToolUseStart { id, name, index } => {
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
                StreamEvent::Error(e) => {
                    eprintln!("Error: {}", e);
                    break;
                }
                _ => {}
            }
        }
    }

    Ok(())
}

async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    client: &AnthropicClient,
    state: &mut AppState,
    session_manager: &SessionManager,
) -> Result<()> {
    let mut events = EventStream::new();
    let mut throbber_interval = interval(Duration::from_millis(250));

    loop {
        if state.needs_render() {
            terminal.draw(|frame| tui::render(frame, state))?;
            state.mark_rendered();
        }

        tokio::select! {
            biased;

            Some(Ok(event)) = events.next() => {
                match event {
                    Event::Key(key) => {
                        // Check if we have a pending permission - handle that first
                        if state.has_pending_permission() {
                            if let Some(response) = handle_permission_key_event(state, key) {
                                // Handle the permission response
                                state.handle_permission_response(response).await;

                                // If user allowed, continue with tool execution
                                if matches!(
                                    response,
                                    PermissionResponse::AllowOnce | PermissionResponse::AllowAlways
                                ) {
                                    // Continue with the tool execution
                                    handle_tool_execution(state, client, session_manager).await?;
                                } else {
                                    // User denied - cancel the tool execution
                                    state.deny_all_tools()?;
                                }
                            }
                            continue; // Don't process other keys while permission prompt is active
                        }

                        debug!(?key, "key event received");

                        match (key.code, key.modifiers) {
                            // Exit commands
                            (KeyCode::Char('c'), KeyModifiers::CONTROL) |
                            (KeyCode::Char('d'), KeyModifiers::CONTROL) => break,

                            // Submit input
                            (KeyCode::Enter, KeyModifiers::NONE) if !state.input.is_empty() => {
                                let input = state.take_input();

                                // Check for slash commands before sending to API
                                if input.trim().starts_with('/') {
                                    use crate::app::commands::{CommandResult, SlashCommandHandler};

                                    let plugin_info =
                                        SlashCommandHandler::build_plugin_info(state.plugins());
                                    let handler = SlashCommandHandler::new(state.working_dir.clone())
                                        .with_plugins(plugin_info);
                                    let result = handler.handle(&input);

                                    // Display the user's command in timeline
                                    state.add_message(Message {
                                        role: Role::User,
                                        content: input.clone(),
                                    });

                                    // Display the command result
                                    let response = match result {
                                        CommandResult::Executed(output) => output,
                                        CommandResult::NotACommand => {
                                            // This shouldn't happen since we checked for /
                                            format!("Input doesn't look like a command: {}", input)
                                        }
                                        CommandResult::UnknownCommand(cmd) => {
                                            format!("Unknown command: /{}. Type /help for available commands.", cmd)
                                        }
                                        CommandResult::Error(err) => {
                                            format!("Error: {}", err)
                                        }
                                    };

                                    state.add_message(Message {
                                        role: Role::Assistant,
                                        content: response,
                                    });

                                    state.mark_full_redraw();
                                } else {
                                    state.submit_message(client, input).await?;
                                    // Auto-save after user message
                                    auto_save_session(state, session_manager).await;
                                }
                            }

                            // Delete character
                            (KeyCode::Backspace, _) => {
                                state.delete_char();
                            }

                            // Scroll up: Ctrl+Up, PageUp, Ctrl+k (vim-style)
                            (KeyCode::Up, KeyModifiers::CONTROL) |
                            (KeyCode::PageUp, _) |
                            (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
                                debug!("scroll_up triggered");
                                state.scroll_up(10);
                            }
                            // Scroll down: Ctrl+Down, PageDown, Ctrl+j (vim-style)
                            (KeyCode::Down, KeyModifiers::CONTROL) |
                            (KeyCode::PageDown, _) |
                            (KeyCode::Char('j'), KeyModifiers::CONTROL) => {
                                debug!("scroll_down triggered");
                                state.scroll_down(10);
                            }
                            // Scroll to top: Home, Ctrl+g
                            (KeyCode::Home, _) |
                            (KeyCode::Char('g'), KeyModifiers::CONTROL) => {
                                debug!("scroll_to_top triggered");
                                state.scroll_to_top();
                            }
                            // Scroll to bottom: End
                            (KeyCode::End, _) => {
                                debug!("scroll_to_bottom triggered");
                                let height = state.scroll_state().content_height();
                                state.scroll_to_bottom(height);
                            }

                            // Select all: Cmd+A (macOS), Ctrl+A, or Ctrl+Shift+A
                            // Cmd+A works when kitty protocol is enabled (iTerm2, kitty, WezTerm)
                            // Always selects all content regardless of focus area
                            (KeyCode::Char('a') | KeyCode::Char('A'), modifiers)
                                if modifiers == KeyModifiers::SUPER
                                    || modifiers == KeyModifiers::CONTROL
                                    || modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT) =>
                            {
                                let line_count = state.rendered_line_count();
                                let timeline_len = state.timeline().len();
                                let modifier_name = if modifiers == KeyModifiers::SUPER {
                                    "Cmd+A"
                                } else {
                                    "Ctrl+A"
                                };
                                debug!(
                                    line_count,
                                    timeline_len,
                                    modifier = modifier_name,
                                    focus_area = ?state.focus_area(),
                                    "select_all triggered via {}",
                                    modifier_name
                                );
                                if line_count == 0 {
                                    debug!("select_all: no content to select (cache empty, timeline_len={})", timeline_len);
                                } else {
                                    // Set focus to Content so status bar shows correctly
                                    state.set_focus_area(FocusArea::Content);
                                    state.selection_mut().select_all(line_count);
                                    state.mark_full_redraw();
                                    let copy_hint = if modifiers == KeyModifiers::SUPER {
                                        "Cmd+C"
                                    } else {
                                        "Ctrl+Y"
                                    };
                                    info!(
                                        line_count,
                                        "Selected all {} lines ({} to copy)",
                                        line_count,
                                        copy_hint
                                    );
                                }
                            }

                            // Copy selection: Cmd+C (macOS), Ctrl+Shift+C, or Ctrl+Y (yank)
                            // Cmd+C works when kitty protocol is enabled (iTerm2, kitty, WezTerm)
                            // Note: Ctrl+C alone is reserved for exit
                            (KeyCode::Char('c') | KeyCode::Char('C'), modifiers)
                                if modifiers == KeyModifiers::SUPER
                                    || modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT) =>
                            {
                                debug!(modifier = ?modifiers, "copy triggered");
                                handle_copy(state);
                            }

                            // Alternative copy: Ctrl+Y (yank) - easier to type than Ctrl+Shift+C
                            // This is the RECOMMENDED copy keybinding as it doesn't conflict
                            (KeyCode::Char('y') | KeyCode::Char('Y'), KeyModifiers::CONTROL) =>
                            {
                                handle_copy(state);
                            }

                            // Clear selection: Escape
                            (KeyCode::Esc, KeyModifiers::NONE) if state.selection().has_selection() => {
                                state.selection_mut().clear();
                                state.mark_full_redraw();
                            }

                            // Text input (must come after special char bindings)
                            (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                                state.insert_char(c);
                            }

                            _ => {
                                debug!(?key.code, ?key.modifiers, "unhandled key");
                            }
                        }
                    }
                    Event::Resize(_, _) => {
                        state.mark_full_redraw();
                    }
                    Event::Mouse(mouse) => {
                        // Get terminal height for focus area detection
                        let terminal_height = terminal.size().map(|s| s.height).unwrap_or(24);

                        match mouse.kind {
                            MouseEventKind::Down(MouseButton::Left) => {
                                // Determine which area was clicked and set focus
                                let clicked_area =
                                    AppState::focus_area_for_row(mouse.row, terminal_height);

                                // Update focus (clears selection if focus changes)
                                state.set_focus_area(clicked_area);

                                // Only start text selection if clicking in content area
                                if clicked_area == FocusArea::Content {
                                    // Convert screen coordinates to content position
                                    // mouse.row is terminal row (0 = top of screen)
                                    // first_visible_line() gives the content line at viewport top
                                    // Account for Messages box border (1 row)
                                    let first_visible = state.scroll_state().first_visible_line();
                                    let content_row = mouse.row.saturating_sub(1) as usize;
                                    let pos = ContentPosition::new(
                                        first_visible + content_row,
                                        mouse.column.saturating_sub(1) as usize,
                                    );
                                    state.selection_mut().start(pos);
                                }
                                state.mark_full_redraw();
                            }
                            MouseEventKind::Drag(MouseButton::Left) => {
                                // Only update selection if content area has focus
                                if state.focus_area() == FocusArea::Content {
                                    let first_visible = state.scroll_state().first_visible_line();
                                    let content_row = mouse.row.saturating_sub(1) as usize;
                                    let pos = ContentPosition::new(
                                        first_visible + content_row,
                                        mouse.column.saturating_sub(1) as usize,
                                    );
                                    state.selection_mut().update(pos);
                                    state.mark_full_redraw();
                                }
                            }
                            MouseEventKind::Up(MouseButton::Left) => {
                                // Complete selection if content area has focus
                                if state.focus_area() == FocusArea::Content {
                                    state.selection_mut().end();
                                    state.mark_full_redraw();
                                }
                            }
                            MouseEventKind::ScrollUp => {
                                debug!("mouse scroll up");
                                state.scroll_up(3);
                            }
                            MouseEventKind::ScrollDown => {
                                debug!("mouse scroll down");
                                state.scroll_down(3);
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }

            Some(chunk) = state.recv_api_chunk() => {
                let is_message_complete = matches!(
                    &chunk,
                    crate::api::StreamEvent::MessageStop | crate::api::StreamEvent::MessageComplete { .. }
                );

                // Check if this is a tool_use stop reason BEFORE processing
                let is_tool_use_complete = matches!(
                    &chunk,
                    crate::api::StreamEvent::MessageComplete { stop_reason }
                    if stop_reason.needs_tool_execution()
                );

                state.append_chunk(chunk)?;

                // Auto-save after assistant message completes
                if is_message_complete {
                    auto_save_session(state, session_manager).await;
                }

                // Handle tool execution if this was a tool_use stop
                if is_tool_use_complete {
                    handle_tool_execution(state, client, session_manager).await?;
                }
            }

            _ = throbber_interval.tick(), if state.is_loading() => {
                state.tick_throbber();
            }
        }
    }

    // Save session before exit
    auto_save_session(state, session_manager).await;

    Ok(())
}

/// Handles one iteration of tool execution after receiving tool_use stop reason.
///
/// This function:
/// 1. Auto-approves pending tools
/// 2. Executes the tools
/// 3. Sets up continuation streaming (returns immediately, doesn't block)
///
/// The main event loop continues to receive chunks via `recv_api_chunk()`.
/// This function is called each time a tool_use stop reason is received,
/// so the loop continues naturally through the event loop.
async fn handle_tool_execution(
    state: &mut AppState,
    client: &AnthropicClient,
    session_manager: &SessionManager,
) -> Result<()> {
    use crate::api::tools::default_tools;
    use crate::api::ToolChoice;

    // Only process if we're in PendingApproval state
    if !matches!(state.tool_loop_state(), ToolLoopState::PendingApproval) {
        debug!("Tool loop not in PendingApproval state, skipping");
        return Ok(());
    }

    debug!("Tool loop in PendingApproval state, auto-approving tools");

    // For now, auto-approve all tools
    // TODO: In Phase 10.5.3, show permission prompt and wait for user response
    state.approve_all_tools()?;

    // Execute the tools
    debug!("Executing pending tools");
    let needs_permission = state.execute_pending_tools().await?;

    // Handle tools that need permission
    if !needs_permission.is_empty() {
        warn!("Some tools need permission: {:?}", needs_permission);
        // Cannot proceed - tools haven't been executed yet
        // Return early to prevent "Cannot finish execution with unexecuted tools" error
        return Ok(());
    }

    // Finish execution and get continuation data
    let continuation = state.finish_tool_execution()?;

    // Build the messages for the conversation
    let (assistant_msg, user_msg) = continuation.build_messages();

    // P0-2: Debug logging to verify tool results reach API
    if let Some(blocks) = assistant_msg.content.as_blocks() {
        let tool_use_count = blocks.iter().filter(|b| b.is_tool_use()).count();
        let text_count = blocks.iter().filter(|b| b.is_text()).count();
        debug!(
            text_blocks = text_count,
            tool_use_blocks = tool_use_count,
            "Assistant continuation message built"
        );
    }
    if let Some(blocks) = user_msg.content.as_blocks() {
        let tool_result_count = blocks.iter().filter(|b| b.is_tool_result()).count();
        debug!(
            tool_result_blocks = tool_result_count,
            "User tool_result message built"
        );
    }

    // Add to API message history for conversation continuation
    // Note: The assistant message is NOT added to the timeline here because
    // finalize_streaming_for_tool_use() already converted the streaming entry
    // to an AssistantMessage. Adding it again would cause duplicate messages.
    state.api_messages_mut().push(assistant_msg);

    // Add tool results to both timeline (for display) and API (for continuation)
    let tool_result_summary = format_tool_results_for_display(&user_msg);
    state.add_message(Message {
        role: Role::User,
        content: tool_result_summary,
    });
    state.api_messages_mut().push(user_msg);

    // Auto-save after tool execution
    auto_save_session(state, session_manager).await;

    // Continue the conversation with Claude using the full API messages
    debug!("Continuing conversation with tool results");

    // Start streaming the continuation - this sets state to Streaming
    state.tool_loop_mut().start_streaming()?;

    // Set up the streaming channel for the main event loop to receive
    let (tx, rx) = tokio::sync::mpsc::channel(100);
    state.set_streaming_rx(rx);

    // Mark as loading so throbber animates and current_response accumulates
    state.set_loading(true);
    state.set_current_response(String::new());

    let api_messages = state.api_messages().to_vec();
    let client_clone = client.clone();
    let tools = default_tools();

    tokio::spawn(async move {
        if let Err(e) = client_clone
            .stream_message_v2_with_tools(&api_messages, Some(&tools), Some(&ToolChoice::Auto), tx)
            .await
        {
            tracing::error!("API error during tool continuation: {}", e);
        }
    });

    // Return immediately - the main event loop will receive chunks via recv_api_chunk()
    // When another tool_use stop is received, this function will be called again
    Ok(())
}

/// Formats tool results for display in the conversation history.
///
/// Extracts content from tool_result blocks and creates a human-readable summary.
/// This is used for text-only display contexts.
fn format_tool_results_for_display(user_msg: &ApiMessageV2) -> String {
    match &user_msg.content {
        crate::types::MessageContent::Text(s) => s.clone(),
        crate::types::MessageContent::Blocks(blocks) => {
            let mut parts = Vec::new();
            for block in blocks {
                if let Some(result) = block.as_tool_result() {
                    // Truncate long results for display
                    let content = if result.content.len() > 500 {
                        format!("{}... (truncated)", &result.content[..500])
                    } else {
                        result.content.clone()
                    };
                    let prefix = if result.is_error { "Error: " } else { "" };
                    parts.push(format!("[Tool result: {}{}]", prefix, content));
                }
            }
            if parts.is_empty() {
                "[Tool results received]".to_string()
            } else {
                parts.join("\n")
            }
        }
    }
}

/// Handles a key event for the permission prompt.
///
/// Converts crossterm key events to the format expected by the permission
/// prompt handler and returns the user's response if a decision was made.
///
/// # Arguments
///
/// * `state` - The application state (used to get the pending permission)
/// * `key` - The key event from crossterm
///
/// # Returns
///
/// The permission response if the user made a decision, or `None` if the
/// key was handled but no decision was made (e.g., navigation).
fn handle_permission_key_event(
    state: &mut AppState,
    key: crossterm::event::KeyEvent,
) -> Option<PermissionResponse> {
    // Create a prompt state from the pending permission
    let request = state.pending_permission()?.clone();
    let mut prompt_state = PermissionPromptState::new(request);

    // Convert crossterm key event to char for the handler
    let key_char = match key.code {
        KeyCode::Char(c) => c,
        KeyCode::Enter => '\r',
        KeyCode::Esc => '\x1b',
        KeyCode::Tab => '\t',
        KeyCode::Backspace => '\x08',
        KeyCode::Left => 'h',  // vim-style navigation
        KeyCode::Right => 'l', // vim-style navigation
        _ => return None,
    };

    // Handle the key input
    let response = handle_permission_key(&mut prompt_state, key_char);

    // If we got a response, clear the pending permission
    if response.is_some() {
        state.clear_pending_permission();
    }

    response
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
    use crate::permissions::PermissionRequest;
    use crate::types::config::ParallelMode;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use std::path::PathBuf;

    /// Creates a key event for testing.
    fn make_key_event(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    }

    // =========================================================================
    // Permission key event handling tests
    // =========================================================================

    #[test]
    fn test_y_key_allows_once() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        let request = PermissionRequest::new("Bash", Some("echo hello"), "Print hello");
        state.set_pending_permission(request);

        let key = make_key_event(KeyCode::Char('y'), KeyModifiers::NONE);
        let response = handle_permission_key_event(&mut state, key);

        assert_eq!(response, Some(PermissionResponse::AllowOnce));
        assert!(!state.has_pending_permission()); // Should be cleared
    }

    #[test]
    fn test_y_uppercase_allows_once() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        let request = PermissionRequest::new("Bash", Some("ls"), "List files");
        state.set_pending_permission(request);

        let key = make_key_event(KeyCode::Char('Y'), KeyModifiers::SHIFT);
        let response = handle_permission_key_event(&mut state, key);

        assert_eq!(response, Some(PermissionResponse::AllowOnce));
    }

    #[test]
    fn test_a_key_allows_always() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        let request = PermissionRequest::new("Read", Some("/tmp/test.txt"), "Read file");
        state.set_pending_permission(request);

        let key = make_key_event(KeyCode::Char('a'), KeyModifiers::NONE);
        let response = handle_permission_key_event(&mut state, key);

        assert_eq!(response, Some(PermissionResponse::AllowAlways));
        assert!(!state.has_pending_permission()); // Should be cleared
    }

    #[test]
    fn test_a_uppercase_allows_always() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        let request = PermissionRequest::new("Bash", Some("git status"), "Git status");
        state.set_pending_permission(request);

        let key = make_key_event(KeyCode::Char('A'), KeyModifiers::SHIFT);
        let response = handle_permission_key_event(&mut state, key);

        assert_eq!(response, Some(PermissionResponse::AllowAlways));
    }

    #[test]
    fn test_n_key_denies() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        let request = PermissionRequest::new("Bash", Some("rm -rf /tmp"), "Delete files");
        state.set_pending_permission(request);

        let key = make_key_event(KeyCode::Char('n'), KeyModifiers::NONE);
        let response = handle_permission_key_event(&mut state, key);

        assert_eq!(response, Some(PermissionResponse::Deny));
        assert!(!state.has_pending_permission()); // Should be cleared
    }

    #[test]
    fn test_n_uppercase_denies() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        let request = PermissionRequest::new("Bash", Some("sudo"), "Sudo command");
        state.set_pending_permission(request);

        let key = make_key_event(KeyCode::Char('N'), KeyModifiers::SHIFT);
        let response = handle_permission_key_event(&mut state, key);

        assert_eq!(response, Some(PermissionResponse::Deny));
    }

    #[test]
    fn test_escape_key_denies() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        let request = PermissionRequest::new("Bash", Some("command"), "Test");
        state.set_pending_permission(request);

        let key = make_key_event(KeyCode::Esc, KeyModifiers::NONE);
        let response = handle_permission_key_event(&mut state, key);

        assert_eq!(response, Some(PermissionResponse::Deny));
    }

    #[test]
    fn test_enter_key_confirms_default() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        let request = PermissionRequest::new("Bash", Some("ls"), "List");
        state.set_pending_permission(request);

        // Default selection is AllowOnce
        let key = make_key_event(KeyCode::Enter, KeyModifiers::NONE);
        let response = handle_permission_key_event(&mut state, key);

        assert_eq!(response, Some(PermissionResponse::AllowOnce));
    }

    #[test]
    fn test_navigation_keys_no_response() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        let request = PermissionRequest::new("Bash", Some("ls"), "List");
        state.set_pending_permission(request);

        // Tab should navigate but not confirm
        let key = make_key_event(KeyCode::Tab, KeyModifiers::NONE);
        let response = handle_permission_key_event(&mut state, key);

        assert!(response.is_none());
        assert!(state.has_pending_permission()); // Still pending
    }

    #[test]
    fn test_no_pending_permission_returns_none() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        // No pending permission set

        let key = make_key_event(KeyCode::Char('y'), KeyModifiers::NONE);
        let response = handle_permission_key_event(&mut state, key);

        assert!(response.is_none());
    }

    #[test]
    fn test_unrecognized_key_returns_none() {
        let mut state = AppState::new(PathBuf::from("/test"), false, ParallelMode::Enabled);
        let request = PermissionRequest::new("Bash", Some("ls"), "List");
        state.set_pending_permission(request);

        // F1 key - not handled
        let key = make_key_event(KeyCode::F(1), KeyModifiers::NONE);
        let response = handle_permission_key_event(&mut state, key);

        assert!(response.is_none());
        assert!(state.has_pending_permission()); // Still pending
    }
}
