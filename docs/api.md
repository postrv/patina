# Patina API Reference

This document provides the library API reference for Patina (Rust Claude Terminal).

## Overview

Patina can be used both as a CLI application and as a library. The library exposes core types and functionality for building custom Claude integrations.

## Modules

| Module | Description |
|--------|-------------|
| `api` | Anthropic API client with streaming support |
| `api::multi_model` | Multi-model and multi-provider support |
| `agents` | Subagent orchestration |
| `commands` | Slash command parsing and execution |
| `enterprise::audit` | Audit logging for compliance |
| `enterprise::cost` | Cost tracking and budget controls |
| `hooks` | Lifecycle event hooks |
| `mcp` | Model Context Protocol client |
| `plugins` | Plugin system |
| `session` | Session persistence |
| `skills` | Context-aware skill matching |
| `tools` | Tool execution with security policy |
| `types` | Core types (Message, Role, StreamEvent, Config) |

## Core Types

### Message

Represents a conversation message.

```rust
use patina::types::{Message, Role};

let message = Message {
    role: Role::User,
    content: "Hello, Claude!".to_string(),
};
```

**Fields:**
- `role: Role` - The message sender (User or Assistant)
- `content: String` - The message content

### Role

The sender of a message.

```rust
use patina::types::Role;

let user = Role::User;
let assistant = Role::Assistant;

// Display trait
assert_eq!(format!("{}", Role::User), "user");
assert_eq!(format!("{}", Role::Assistant), "assistant");
```

### StreamEvent

Events received during streaming API responses.

```rust
use patina::types::StreamEvent;

match event {
    StreamEvent::ContentDelta(text) => {
        // Handle streamed text chunk
        print!("{}", text);
    }
    StreamEvent::MessageStop => {
        // Message complete
        println!();
    }
    StreamEvent::Error(err) => {
        // Handle error
        eprintln!("Error: {}", err);
    }
}
```

**Variants:**
- `ContentDelta(String)` - A chunk of content from the response
- `MessageStop` - The message is complete
- `Error(String)` - An error occurred

### Config

Application configuration.

```rust
use patina::types::Config;
use std::path::PathBuf;
use secrecy::SecretString;

let config = Config::new(
    SecretString::new("sk-ant-...".into()),
    "claude-sonnet-4-20250514".to_string(),
    PathBuf::from("."),
);

// Accessors
let model = config.model();
let working_dir = config.working_dir();
```

## API Client

### AnthropicClient

The main API client for communicating with Anthropic's Claude API.

```rust
use patina::api::AnthropicClient;
use patina::types::{Message, Role, StreamEvent};
use secrecy::SecretString;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = AnthropicClient::new(
        SecretString::new("sk-ant-...".into()),
        "claude-sonnet-4-20250514",
    );

    let messages = vec![
        Message {
            role: Role::User,
            content: "Hello!".to_string(),
        },
    ];

    let (tx, mut rx) = mpsc::channel::<StreamEvent>(100);

    // Spawn stream processing
    tokio::spawn(async move {
        client.stream_message(&messages, tx).await.unwrap();
    });

    // Receive events
    while let Some(event) = rx.recv().await {
        match event {
            StreamEvent::ContentDelta(text) => print!("{}", text),
            StreamEvent::MessageStop => break,
            StreamEvent::Error(e) => eprintln!("Error: {}", e),
        }
    }

    Ok(())
}
```

**Methods:**

#### `new(api_key: SecretString, model: &str) -> Self`

Creates a new client with the default Anthropic API endpoint.

#### `new_with_base_url(api_key: SecretString, model: &str, base_url: &str) -> Self`

Creates a new client with a custom base URL (useful for testing).

#### `stream_message(&self, messages: &[Message], tx: Sender<StreamEvent>) -> Result<()>`

Sends a streaming message request. Events are sent through the provided channel.

**Retry Behavior:**
- Automatically retries on 429 (rate limit) and 5xx errors
- Uses exponential backoff starting at 100ms
- Maximum 2 retry attempts

## Multi-Model Support

### MultiModelClient

Supports switching between models and providers.

```rust
use patina::api::multi_model::{MultiModelClient, ModelConfig, ModelProvider};
use secrecy::SecretString;

let mut client = MultiModelClient::new();

// Add Anthropic models
client.add_model(ModelConfig {
    name: "claude-sonnet".to_string(),
    model_id: "claude-sonnet-4-20250514".to_string(),
    provider: ModelProvider::Anthropic,
    max_tokens: 8192,
});

// Set current model
client.set_current("claude-sonnet")?;

// Get current model
let current = client.current();
```

### ModelProvider

```rust
pub enum ModelProvider {
    Anthropic,
    Bedrock,
}
```

### BedrockConfig

Configuration for AWS Bedrock provider.

```rust
use patina::api::multi_model::BedrockConfig;

let config = BedrockConfig {
    region: "us-east-1".to_string(),
    role_arn: Some("arn:aws:iam::...".to_string()),
};
```

## Session Management

### Session

Represents a conversation session.

```rust
use patina::session::Session;
use patina::types::{Message, Role};
use std::path::PathBuf;

let mut session = Session::new(PathBuf::from("/my/project"));

session.add_message(Message {
    role: Role::User,
    content: "Hello!".to_string(),
});

// Accessors
let messages = session.messages();
let working_dir = session.working_dir();
let created_at = session.created_at();
let updated_at = session.updated_at();
```

### SessionManager

Handles session persistence.

```rust
use patina::session::SessionManager;
use std::path::PathBuf;

let manager = SessionManager::new(PathBuf::from("~/.config/patina/sessions"));

// Save session
let session_id = manager.save(&session).await?;

// Load session
let loaded = manager.load(&session_id).await?;

// List all sessions
let sessions = manager.list().await?;

// List with metadata
let sessions_with_meta = manager.list_with_metadata().await?;

// Delete session
manager.delete(&session_id).await?;
```

### SessionMetadata

Lightweight session info without full message content.

```rust
pub struct SessionMetadata {
    pub id: String,
    pub working_dir: PathBuf,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
    pub message_count: usize,
}
```

## Tool Execution

### ToolExecutor

Executes tools with security policy enforcement.

```rust
use patina::tools::{ToolExecutor, ToolCall, ToolResult};
use std::path::PathBuf;
use serde_json::json;

let executor = ToolExecutor::new(PathBuf::from("/my/project"));

let call = ToolCall {
    name: "bash".to_string(),
    input: json!({ "command": "ls -la" }),
};

let result = executor.execute(call).await?;

match result {
    ToolResult::Success(output) => println!("{}", output),
    ToolResult::Error(err) => eprintln!("Error: {}", err),
    ToolResult::Cancelled => println!("Cancelled by hook"),
}
```

### Available Tools

| Tool | Description |
|------|-------------|
| `bash` | Execute shell commands |
| `read_file` | Read file contents |
| `write_file` | Write content to file |
| `edit` | String replacement edit |
| `list_files` | List directory contents |
| `glob` | Find files by pattern |
| `grep` | Search file contents |

### ToolExecutionPolicy

Configure security policy.

```rust
use patina::tools::ToolExecutionPolicy;
use std::time::Duration;
use std::path::PathBuf;
use regex::Regex;

let policy = ToolExecutionPolicy {
    dangerous_patterns: vec![
        Regex::new(r"rm\s+-rf\s+/").unwrap(),
    ],
    protected_paths: vec![
        PathBuf::from("/etc"),
    ],
    max_file_size: 10 * 1024 * 1024,
    command_timeout: Duration::from_secs(300),
};

let executor = ToolExecutor::new(PathBuf::from("."))
    .with_policy(policy);
```

### HookedToolExecutor

Tool executor with lifecycle hook integration.

```rust
use patina::tools::{HookedToolExecutor, ToolCall};
use patina::hooks::HookManager;
use std::path::PathBuf;
use serde_json::json;

let manager = HookManager::new("session-123".to_string());
let executor = HookedToolExecutor::new(PathBuf::from("."), manager);

let call = ToolCall {
    name: "bash".to_string(),
    input: json!({ "command": "echo hello" }),
};

let result = executor.execute(call).await?;
```

## Hooks System

### HookManager

High-level hook management.

```rust
use patina::hooks::{HookManager, HookEvent, HookDecision};
use std::path::Path;

let mut manager = HookManager::new("session-123".to_string());

// Load from config file
manager.load_config(Path::new(".rct/hooks.toml"))?;

// Register a tool hook
manager.register_tool_hook(
    HookEvent::PreToolUse,
    Some("bash"),
    "echo 'Running bash command'",
);

// Fire events
let result = manager.fire_pre_tool_use("bash", json!({"command": "ls"})).await?;

match result.decision {
    HookDecision::Continue => { /* proceed */ }
    HookDecision::Block { reason } => { /* blocked */ }
    _ => {}
}
```

### Hook Events

```rust
pub enum HookEvent {
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
    PermissionRequest,
    UserPromptSubmit,
    SessionStart,
    SessionEnd,
    Notification,
    Stop,
    SubagentStop,
    PreCompact,
}
```

### HookDecision

```rust
pub enum HookDecision {
    Continue,
    Block { reason: String },
    Allow,
    Deny,
}
```

## MCP Protocol

### McpClient

Client for MCP servers.

```rust
use patina::mcp::client::McpClient;

let mut client = McpClient::new(
    "my-server",
    "mcp-server-filesystem",
    vec!["/allowed/path".to_string()],
)?;

// Start the server
client.start().await?;

// List available tools
let tools = client.list_tools().await?;

// Call a tool
let result = client.call_tool("read_file", json!({"path": "test.txt"})).await?;

// Stop the server
client.stop().await?;
```

### Transport Types

#### StdioTransport

For local process communication.

```rust
use patina::mcp::transport::StdioTransport;

let transport = StdioTransport::new(
    "mcp-server-filesystem",
    vec!["/path".to_string()],
)?;
```

#### SseTransport

For remote HTTP/SSE communication.

```rust
use patina::mcp::transport::SseTransport;
use std::collections::HashMap;

let mut headers = HashMap::new();
headers.insert("Authorization".to_string(), "Bearer token".to_string());

let transport = SseTransport::new(
    "https://mcp.example.com/sse",
    headers,
)?;
```

## Skills Engine

### SkillEngine

Matches and retrieves context-aware skills.

```rust
use patina::skills::SkillEngine;
use std::path::PathBuf;

let mut engine = SkillEngine::new();

// Load skills from directory
engine.load_from_dir(&PathBuf::from(".rct/skills"))?;

// Match skills for a file
let matches = engine.match_skills_for_file("src/main.rs");

// Get context for a task
let context = engine.get_context_for_task("implement authentication");

// Get context for a file
let context = engine.get_context_for_file("Cargo.toml");
```

## Command Executor

### CommandExecutor

Parses and executes slash commands.

```rust
use patina::commands::CommandExecutor;
use std::path::PathBuf;
use std::collections::HashMap;

let mut executor = CommandExecutor::new();

// Load commands from directory
executor.load_from_dir(&PathBuf::from(".rct/commands"))?;

// List available commands
let commands = executor.list();

// Execute a command
let mut args = HashMap::new();
args.insert("message".to_string(), "feat: new feature".to_string());
let result = executor.execute("commit", args)?;
```

## Plugin System

### PluginRegistry

Discovers and manages plugins.

```rust
use patina::plugins::PluginRegistry;
use std::path::PathBuf;

let mut registry = PluginRegistry::new();

// Discover plugins from paths
registry.discover(&[
    PathBuf::from("~/.config/patina/plugins"),
    PathBuf::from(".rct/plugins"),
])?;

// List plugins
let plugins = registry.list_plugins();

// Get a command
let command = registry.get_command("my-plugin:hello");
```

### Plugin Host

For plugin development, see `src/plugins/host.rs` and the [Plugin API documentation](plugin-api.md).

## Enterprise Features

### AuditLogger

```rust
use patina::enterprise::audit::{AuditLogger, AuditConfig, AuditLevel};
use std::path::PathBuf;

let config = AuditConfig {
    enabled: true,
    level: AuditLevel::All,
    path: PathBuf::from("~/.config/patina/audit"),
};

let logger = AuditLogger::new(config)?;

// Log tool use
logger.log_tool_use("session-123", "bash", &json!({"command": "ls"})).await?;

// Log API call
logger.log_api_call("session-123", "claude-sonnet", 100, 500).await?;

// Query logs
let entries = logger.query(AuditQuery {
    session_id: Some("session-123".to_string()),
    ..Default::default()
}).await?;
```

### CostTracker

```rust
use patina::enterprise::cost::{CostTracker, CostConfig, BudgetLimit};

let config = CostConfig {
    enabled: true,
    session_limit: Some(BudgetLimit::new(50.0)),
    daily_limit: Some(BudgetLimit::new(100.0)),
    monthly_limit: Some(BudgetLimit::new(500.0)),
    ..Default::default()
};

let mut tracker = CostTracker::new(config);

// Record usage
tracker.record_usage("claude-sonnet", 1000, 500);

// Check budget
let alerts = tracker.check_budget();
for alert in alerts {
    println!("Alert: {:?}", alert);
}

// Get statistics
let stats = tracker.statistics();
println!("Total cost: ${:.2}", stats.total_cost);
```

## Subagent Orchestration

### SubagentOrchestrator

Manages parallel task execution with isolated contexts.

```rust
use patina::agents::SubagentOrchestrator;

let mut orchestrator = SubagentOrchestrator::new();

// Configure concurrency
orchestrator.set_max_concurrent(5);

// Spawn a subagent
let id = orchestrator.spawn(SubagentConfig {
    task: "Analyze code".to_string(),
    max_turns: Some(10),
    allowed_tools: Some(vec!["read_file".to_string(), "grep".to_string()]),
});

// Check status
let status = orchestrator.get_status(&id);

// Run the subagent
let result = orchestrator.run(&id, context).await?;

// List all agents
let agents = orchestrator.list_agents();
```

## Error Handling

All fallible operations return `anyhow::Result<T>`. Use the `?` operator for propagation:

```rust
use anyhow::Result;

async fn example() -> Result<()> {
    let client = AnthropicClient::new(api_key, model);
    client.stream_message(&messages, tx).await?;
    Ok(())
}
```

For specific error types, modules may define their own error enums that implement `std::error::Error`.

---

*Patina v0.3.0*
