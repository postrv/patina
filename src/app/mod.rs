//! Application core

use anyhow::Result;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use secrecy::SecretString;
use std::{io, path::PathBuf, time::Duration};
use tokio::time::interval;

pub mod state;
use state::AppState;

use crate::api::AnthropicClient;
use crate::tui;

pub struct Config {
    pub api_key: SecretString,
    pub model: String,
    pub working_dir: PathBuf,
}

pub async fn run(config: Config) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let client = AnthropicClient::new(config.api_key, &config.model);
    let mut state = AppState::new(config.working_dir);

    let result = event_loop(&mut terminal, &client, &mut state).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    client: &AnthropicClient,
    state: &mut AppState,
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
                state.append_chunk(chunk)?;
            }

            _ = throbber_interval.tick(), if state.is_loading() => {
                state.tick_throbber();
            }
        }
    }

    Ok(())
}
