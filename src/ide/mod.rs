//! IDE integration - VS Code and JetBrains extension support

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum IdeMessage {
    #[serde(rename = "init")]
    Init {
        workspace: PathBuf,
        capabilities: Vec<String>,
    },

    #[serde(rename = "prompt")]
    Prompt {
        text: String,
        selection: Option<Selection>,
    },

    #[serde(rename = "apply_edit")]
    ApplyEdit {
        file: PathBuf,
        diff: String,
    },

    #[serde(rename = "streaming_content")]
    StreamingContent {
        delta: String,
    },

    #[serde(rename = "edit_proposal")]
    EditProposal {
        file: PathBuf,
        diff: String,
        description: String,
    },

    #[serde(rename = "tool_use")]
    ToolUse {
        tool: String,
        input: serde_json::Value,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Selection {
    pub file: PathBuf,
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
    pub text: String,
}

pub struct IdeSession {
    id: Uuid,
    workspace: PathBuf,
    capabilities: Vec<String>,
}

pub struct IdeServer {
    listener: Option<TcpListener>,
    sessions: HashMap<Uuid, IdeSession>,
    port: u16,
}

impl IdeServer {
    pub fn new(port: u16) -> Self {
        Self {
            listener: None,
            sessions: HashMap::new(),
            port,
        }
    }

    pub async fn start(&mut self) -> Result<()> {
        let addr = format!("127.0.0.1:{}", self.port);
        self.listener = Some(TcpListener::bind(&addr).await?);
        tracing::info!("IDE server listening on {}", addr);
        Ok(())
    }

    pub async fn accept(&mut self) -> Result<Option<(Uuid, TcpStream)>> {
        if let Some(ref listener) = self.listener {
            let (stream, addr) = listener.accept().await?;
            let session_id = Uuid::new_v4();
            tracing::info!("IDE connection from {} (session {})", addr, session_id);
            Ok(Some((session_id, stream)))
        } else {
            Ok(None)
        }
    }

    pub fn register_session(&mut self, id: Uuid, workspace: PathBuf, capabilities: Vec<String>) {
        self.sessions.insert(id, IdeSession {
            id,
            workspace,
            capabilities,
        });
    }

    pub fn get_session(&self, id: Uuid) -> Option<&IdeSession> {
        self.sessions.get(&id)
    }

    pub fn remove_session(&mut self, id: Uuid) {
        self.sessions.remove(&id);
    }
}

pub async fn handle_connection(mut stream: TcpStream) -> Result<()> {
    let mut buffer = vec![0u8; 4096];

    loop {
        let n = stream.read(&mut buffer).await?;
        if n == 0 {
            break;
        }

        let message: IdeMessage = serde_json::from_slice(&buffer[..n])?;

        let response = match message {
            IdeMessage::Init { workspace, capabilities } => {
                tracing::info!("IDE init: {:?} with {:?}", workspace, capabilities);
                serde_json::json!({
                    "type": "init_ack",
                    "status": "ok"
                })
            }
            IdeMessage::Prompt { text, selection } => {
                tracing::info!("IDE prompt: {} (selection: {:?})", text, selection);
                serde_json::json!({
                    "type": "prompt_ack",
                    "status": "processing"
                })
            }
            _ => {
                serde_json::json!({
                    "type": "error",
                    "message": "Unhandled message type"
                })
            }
        };

        let response_bytes = serde_json::to_vec(&response)?;
        stream.write_all(&response_bytes).await?;
    }

    Ok(())
}
