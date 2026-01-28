//! Application state management

use crate::api::{AnthropicClient, StreamEvent};
use anyhow::Result;
use std::path::PathBuf;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

pub struct AppState {
    pub messages: Vec<Message>,
    pub input: String,
    pub scroll_offset: usize,
    pub working_dir: PathBuf,
    pub current_response: Option<String>,

    loading: bool,
    throbber_frame: usize,
    streaming_rx: Option<mpsc::Receiver<StreamEvent>>,

    dirty: DirtyFlags,
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
    pub fn new(working_dir: PathBuf) -> Self {
        Self {
            messages: Vec::new(),
            input: String::new(),
            scroll_offset: 0,
            working_dir,
            loading: false,
            throbber_frame: 0,
            streaming_rx: None,
            current_response: None,
            dirty: DirtyFlags { full: true, ..Default::default() },
        }
    }

    pub fn insert_char(&mut self, c: char) {
        self.input.push(c);
        self.dirty.input = true;
    }

    pub fn delete_char(&mut self) {
        self.input.pop();
        self.dirty.input = true;
    }

    pub fn take_input(&mut self) -> String {
        self.dirty.input = true;
        std::mem::take(&mut self.input)
    }

    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(lines);
        self.dirty.messages = true;
    }

    pub fn scroll_down(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
        self.dirty.messages = true;
    }

    pub fn is_loading(&self) -> bool {
        self.loading
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

    pub async fn submit_message(&mut self, client: &AnthropicClient, content: String) -> Result<()> {
        self.messages.push(Message {
            role: Role::User,
            content,
        });
        self.dirty.messages = true;

        self.loading = true;
        self.current_response = Some(String::new());

        let (tx, rx) = mpsc::channel(100);
        self.streaming_rx = Some(rx);

        let messages = self.messages.clone();
        let client = client.clone();
        tokio::spawn(async move {
            if let Err(e) = client.stream_message(&messages, tx).await {
                tracing::error!("API error: {}", e);
            }
        });

        Ok(())
    }

    pub async fn recv_api_chunk(&mut self) -> Option<StreamEvent> {
        if let Some(rx) = &mut self.streaming_rx {
            rx.recv().await
        } else {
            std::future::pending::<Option<StreamEvent>>().await
        }
    }

    pub fn append_chunk(&mut self, event: StreamEvent) -> Result<()> {
        match event {
            StreamEvent::ContentDelta(text) => {
                if let Some(ref mut response) = self.current_response {
                    response.push_str(&text);
                    self.dirty.messages = true;
                }
            }
            StreamEvent::MessageStop => {
                if let Some(response) = self.current_response.take() {
                    self.messages.push(Message {
                        role: Role::Assistant,
                        content: response,
                    });
                }
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
        }
        Ok(())
    }
}
