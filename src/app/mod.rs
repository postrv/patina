//! Application core

use anyhow::{Context, Result};
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{io, time::Duration};
use tokio::time::interval;
use tracing::{debug, info, warn};

pub mod state;
use state::AppState;

use crate::api::AnthropicClient;
use crate::session::{default_sessions_dir, SessionManager};
use crate::tui;
use crate::types::config::ResumeMode;

// Re-export Config for backward compatibility
pub use crate::types::Config;

pub async fn run(config: Config) -> Result<()> {
    // Initialize session manager for auto-save
    let sessions_dir = default_sessions_dir()?;
    let session_manager = SessionManager::new(sessions_dir);

    // Check for session resume before initializing terminal
    let mut state = match &config.resume_mode {
        ResumeMode::None => AppState::new(config.working_dir.clone()),
        ResumeMode::Last | ResumeMode::SessionId(_) => load_session_state(&config).await?,
    };

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let client = AnthropicClient::new(config.api_key, &config.model);

    let result = event_loop(&mut terminal, &client, &mut state, &session_manager).await;

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
    let mut state = AppState::new(session.working_dir().clone());
    state.restore_from_session(&session);

    Ok(state)
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
                        match (key.code, key.modifiers) {
                            (KeyCode::Char('c'), KeyModifiers::CONTROL) |
                            (KeyCode::Char('d'), KeyModifiers::CONTROL) => break,

                            (KeyCode::Enter, KeyModifiers::NONE) if !state.input.is_empty() => {
                                let input = state.take_input();
                                state.submit_message(client, input).await?;
                                // Auto-save after user message
                                auto_save_session(state, session_manager).await;
                            }

                            (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                                state.insert_char(c);
                            }

                            (KeyCode::Backspace, _) => {
                                state.delete_char();
                            }

                            (KeyCode::Up, KeyModifiers::CONTROL) |
                            (KeyCode::PageUp, _) => {
                                state.scroll_up(10);
                            }
                            (KeyCode::Down, KeyModifiers::CONTROL) |
                            (KeyCode::PageDown, _) => {
                                state.scroll_down(10);
                            }

                            _ => {}
                        }
                    }
                    Event::Resize(_, _) => {
                        state.mark_full_redraw();
                    }
                    _ => {}
                }
            }

            Some(chunk) = state.recv_api_chunk() => {
                let is_message_complete = matches!(&chunk, crate::api::StreamEvent::MessageStop);
                state.append_chunk(chunk)?;
                // Auto-save after assistant message completes
                if is_message_complete {
                    auto_save_session(state, session_manager).await;
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
