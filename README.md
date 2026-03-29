<p align="center">
  <img src="patina.jpg" alt="Patina Logo" width="300">
</p>

# Patina

[![CI](https://github.com/postrv/patina/actions/workflows/ci.yml/badge.svg)](https://github.com/postrv/patina/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE)
[![Version](https://img.shields.io/badge/version-1.0.0-green.svg)](Cargo.toml)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org/)

A high-performance terminal client for the Claude API, written in Rust. Designed for developers who want a fast, secure, and extensible AI assistant in their terminal.

## Highlights

- **Sub-millisecond rendering** - Full 100-message redraw in <1ms
- **Multi-provider LLM support** - Claude (Anthropic) and OpenRouter-compatible models with automatic failover
- **Parallel tool execution** - 5x+ speedup on multi-file operations
- **Full MCP support** - Connect any MCP server via stdio, streamable HTTP, or legacy SSE transports
- **Autonomous agent orchestration** - Spawn parallel sub-agents in isolated git worktrees
- **Continuous coding loop** - Run tasks autonomously with stagnation detection and quality gates
- **5,000+ tests** with 85%+ code coverage
- **Zero unsafe code** - Pure safe Rust (~139,000 LOC)
- **Cross-platform** - Linux, macOS, Windows
- **Security-first** - Defense-in-depth with command filtering, path validation, and session integrity

## Features

### Core Capabilities

| Feature | Description |
|---------|-------------|
| **Streaming TUI** | Real-time response streaming with syntax highlighting |
| **Agentic Tool Loop** | Claude can autonomously execute tools and continue conversations |
| **Multi-Provider LLM** | Anthropic, OpenRouter, AWS Bedrock, Google Vertex AI, and any OpenAI-compatible API |
| **Provider Failover** | Automatic fallback between providers on failure |
| **Parallel Execution** | Concurrent tool execution with safety classification (5x+ speedup) |
| **Session Resume** | Save and restore conversations with full context |
| **Context Compaction** | Automatic summarization when context window fills |
| **Context Compression** | Intelligent context building with token budgeting and narsil-mcp integration |
| **MCP Support** | Full Model Context Protocol with stdio, HTTP, and legacy SSE transports |
| **Plan Mode** | Interactive plan review with approve/reject before multi-step execution |
| **Background Execution** | Run long-running bash commands in background with task management |
| **Effort Levels** | Configurable effort (auto/low/medium/high) controlling token budget and thinking depth |
| **Transcript Search** | Regex-powered search across conversation history with Ctrl+F |
| **External Editor** | Edit input in `$VISUAL`/`$EDITOR` with Ctrl+X Ctrl+E |
| **Sandbox Mode** | Configurable filesystem isolation with allow/deny lists |
| **Enterprise Settings** | Managed settings for organization policy enforcement |
| **Diagnostics** | `/doctor` command with 10 environment checks and fix suggestions |
| **Export** | Export conversations as Markdown, JSON, or plain text with filtering options |
| **Slash Command Completion** | Tab-completion popup for 32 slash commands |

### Autonomous Agents

| Feature | Description |
|---------|-------------|
| **Worktree Agents** | Spawn sub-agents in isolated git worktrees for parallel work |
| **Conflict Detection** | Cross-agent file conflict detection before merging |
| **Continuous Loop** | Run tasks autonomously with `/continuous` command |
| **Stagnation Detection** | Multi-factor scoring detects stuck agents and triggers recovery |
| **Quality Gates** | Automated clippy, test, and format checks with timeout enforcement |

### Built-in Tools (25)

| Tool | Description |
|------|-------------|
| `bash` | Execute shell commands with timeout, security filtering, and background execution |
| `read_file` | Read file contents with optional line-range selection (offset/limit) |
| `write_file` | Write files with path validation and sandbox enforcement |
| `edit` | Search-and-replace editing with optional `replace_all` mode |
| `list_files` | List files and directories at a given path |
| `glob` | File discovery with glob pattern matching |
| `grep` | Content search with regex, output modes, context lines, file type filters |
| `web_fetch` | Fetch and convert web pages to markdown |
| `web_search` | Search the web via DuckDuckGo |
| `analyze_image` | Analyze images using Claude's vision (PNG, JPEG, GIF, WebP) |
| `lsp` | Language Server Protocol operations (go-to-definition, references, hover) |
| `todo_write` | Persistent task tracking with add/complete/remove/list operations |
| `plan` | Present a structured plan for user review before execution |
| `ask_user` | Ask the user a question with structured choices or free-text input |
| `send_message` | Send messages between agents via the inter-agent message router |
| `tool_search` | Discover deferred tool schemas by keyword or exact name |
| `task_create` | Create a new task with description, status, and dependency tracking |
| `task_get` | Retrieve a task by ID with full details |
| `task_list` | List all tasks with optional status filtering |
| `task_update` | Update task status, description, or dependencies |
| `cron_create` | Create a scheduled prompt with interval (e.g., "5m", "1h", "daily") |
| `cron_list` | List all cron schedules with status and next run time |
| `cron_delete` | Delete a cron schedule by ID |
| `task_output` | Get output from a background bash task |
| `task_stop` | Stop a running background bash task |

### Extensibility

| Feature | Description |
|---------|-------------|
| **MCP Servers** | Connect any MCP-compatible tool server (narsil, JetBrains, etc.) |
| **Plugin System** | TOML-based plugins with auto-discovery |
| **Skills Engine** | Context-aware suggestions via SKILL.md files |
| **Hooks** | 13 lifecycle events (PreToolUse, PostToolUse, SessionStart, Notification, etc.) |
| **Slash Commands** | 32 built-in commands including `/model`, `/compact`, `/export`, `/doctor`, `/agent`, `/continuous` |

### Developer Experience

| Feature | Description |
|---------|-------------|
| **Project Context** | Automatic CLAUDE.md discovery for project instructions |
| **Git Worktrees** | Parallel AI-assisted development with isolation |
| **IDE Integration** | TCP server for VS Code and JetBrains extensions |
| **narsil-mcp** | Optional code intelligence with 90+ analysis tools |

## Installation

### From Source

```bash
git clone https://github.com/postrv/patina.git
cd patina
cargo install --path .
```

## Quick Start

```bash
# Set your API key
export ANTHROPIC_API_KEY="your-api-key"

# Run patina
patina

# With an initial prompt
patina "Explain this codebase"

# Print mode (non-interactive)
patina -p "What is 2+2?"

# Resume last session
patina -c

# List saved sessions
patina --list-sessions

# Use OpenRouter instead of Anthropic
patina --provider openrouter --model anthropic/claude-sonnet-4

# Use AWS Bedrock
patina --provider bedrock --model claude-sonnet-4-20250514

# Use Google Vertex AI
patina --provider vertex --model claude-sonnet-4-20250514
```

## Command Line Options

| Option | Description | Default |
|--------|-------------|---------|
| `[PROMPT]` | Initial prompt to start with | - |
| `-p, --print` | Print mode (non-interactive) | `false` |
| `--api-key` | API key (or `ANTHROPIC_API_KEY` env) | - |
| `-m, --model` | Model to use | `claude-sonnet-4-20250514` |
| `--provider` | LLM provider (`anthropic`, `openrouter`, `bedrock`, `vertex`) | `anthropic` |
| `--fallback-provider` | Fallback provider on failure | - |
| `--effort` | Effort level (`auto`, `low`, `medium`, `high`) | `auto` |
| `--bare` | Fast startup mode (skip plugins, context loading) | `false` |
| `-C, --directory` | Working directory | `.` |
| `-c, --continue` | Resume most recent session | - |
| `-r, --resume` | Resume specific session by ID | - |
| `--list-sessions` | List available sessions | - |
| `--with-narsil` | Enable narsil-mcp integration | auto |
| `--no-narsil` | Disable narsil-mcp integration | - |
| `--no-parallel` | Disable parallel tool execution | - |
| `--parallel-aggressive` | Parallelize all tools (use with caution) | - |
| `--debug` | Enable debug logging (writes to `/tmp/patina.log`) | `false` |

## Key Bindings

| Key | Action |
|-----|--------|
| `Enter` | Send message |
| `Ctrl+C` / `Ctrl+D` | Quit |
| `Left` / `Right` | Move cursor in input |
| `Home` / `End` | Jump to start/end of input |
| `Delete` | Delete character forward |
| `PageUp` / `PageDown` | Scroll conversation |
| `Ctrl+F` | Search transcript (regex) |
| `Ctrl+X Ctrl+E` | Open input in external editor |
| `Ctrl+A` | Select all (universal) |
| `Ctrl+Y` | Copy selection (universal) |
| `Ctrl+Shift+V` | Paste (universal) |

**Terminal-specific shortcuts:**

| Terminal | Select All | Copy | Paste |
|----------|------------|------|-------|
| **iTerm2** | `Cmd+A` | `Cmd+C` | `Cmd+V` |
| **Kitty/WezTerm** | `Cmd+A` | `Cmd+C` | `Cmd+V` |
| **JetBrains** | `Option+A` | `Option+C` | `Option+V` |
| **Other** | `Ctrl+A` | `Ctrl+Y` | `Ctrl+Shift+V` |

**Permission Prompts:**

| Key | Action |
|-----|--------|
| `y` / `Enter` | Allow once |
| `a` | Allow always (save rule) |
| `n` / `Esc` | Deny |

**Plan Review (when model proposes a plan):**

| Key | Action |
|-----|--------|
| `Enter` | Approve plan |
| `Esc` / `n` | Reject plan |
| `Up` / `Down` | Navigate steps |

**Question Prompt (when model asks a question):**

| Key | Action |
|-----|--------|
| `Enter` | Submit response |
| `Esc` | Cancel |
| `Up` / `Down` | Navigate options |
| `Tab` | Toggle between options and free-text |

## Slash Commands (32)

| Command | Description |
|---------|-------------|
| `/help` | Show available commands and usage |
| `/model <name>` | Switch model mid-conversation (aliases: sonnet, opus, haiku) |
| `/compact [instructions]` | Trigger context compaction with optional custom instructions |
| `/clear` | Clear conversation history, preserving configuration |
| `/context` | Show context window usage with token breakdown and warnings |
| `/cost` | Show session cost and token usage |
| `/effort [level]` | Get or set effort level (auto/low/medium/high) |
| `/export [format] [path]` | Export conversation as Markdown, JSON, or plain text |
| `/doctor` | Run 10 diagnostic checks with fix suggestions |
| `/plan [subcommand]` | Show, accept, or reject pending execution plans |
| `/memory` | Manage persistent memory (list, add, remove, search) |
| `/mcp` | Show MCP server status and connected tools |
| `/continuous` | Start autonomous coding loop with quality gates |
| `/agent <subcommand>` | Manage sub-agents (spawn, list, merge, stop) |
| `/worktree <subcommand>` | Manage git worktrees (new, list, switch, remove, status) |
| `/experiment <subcommand>` | Manage experiments (start, list, accept, reject, pause) |
| `/analyze` | Run narsil-powered code analysis |
| `/audit` | Run narsil-powered security audit |
| `/plugins` | List loaded plugins with their capabilities |
| `/terminal-setup` | Configure terminal key bindings |
| `/fork` / `/branch` | Fork session into a new branch |
| `/rewind` | Rewind to a previous checkpoint |
| `/rename <name>` | Rename current session |
| `/color <color>` | Set prompt color |
| `/btw <question>` | Ask a side question without breaking context |
| `/copy [index]` | Copy message to clipboard |
| `/config` / `/settings` | Show configuration |
| `/permissions` | Show permission rules |
| `/sandbox` | Show sandbox configuration |
| `/status` | Show session status |
| `/bug` / `/feedback` | Report a bug or provide feedback |

All commands support tab completion -- start typing `/` and press Tab to see options.

## MCP Support

Patina implements the [Model Context Protocol](https://spec.modelcontextprotocol.io/) for connecting external tool servers, built on the [rmcp SDK](https://github.com/modelcontextprotocol/rust-sdk) for full spec compliance.

### Transports

| Transport | Use Case | Example |
|-----------|----------|---------|
| **stdio** | Local tool servers, CLI tools | narsil-mcp, filesystem servers |
| **Streamable HTTP** | Remote/cloud MCP servers | POST-based JSON-RPC |
| **Legacy SSE** | JetBrains, older MCP servers | GET `/sse` + POST endpoint |

### Configuration

MCP servers are configured via `.mcp.json` in your project root (compatible with Claude Code format):

```json
{
  "mcpServers": {
    "narsil": {
      "command": "narsil-mcp",
      "args": ["--repo", "."],
      "env": {}
    },
    "jetbrains": {
      "type": "sse",
      "url": "http://localhost:63342/api/mcp/sse"
    },
    "remote-server": {
      "url": "https://example.com/mcp"
    }
  }
}
```

Global servers can be configured in `~/.claude.json` under `"mcpServers"`.

### Transport Detection

- **stdio**: Entries with a `command` field spawn a child process
- **Legacy SSE**: Entries with `"type": "sse"` or a URL ending in `/sse`
- **Streamable HTTP**: Entries with a `url` field (default for HTTP)

### Features

- **Namespaced tools**: Each server's tools are prefixed with `servername__` to avoid collisions
- **Parallel startup**: All servers connect concurrently
- **Auto-discovery**: Tools from connected servers are automatically available to Claude
- **OAuth & Bearer auth**: Bearer token from environment variables, OAuth with token refresh and caching
- **Security**: Command validation and interpreter path requirements for stdio servers
- **Trust management**: Content-hash verification for project-level MCP configs
- **Graceful degradation**: Failed servers don't block startup; other servers continue normally

## Security

Patina implements defense-in-depth security controls:

| Control | Implementation |
|---------|----------------|
| **Command Filtering** | 28+ dangerous patterns blocked (rm -rf, sudo, etc.) |
| **Path Validation** | Canonicalization + symlink protection |
| **Sandbox Policy** | Configurable filesystem allow/deny lists with deny-wins precedence |
| **Permission System** | Explicit approval required for tool execution |
| **API Key Protection** | SecretString with `[REDACTED]` in logs and debug output |
| **OAuth Token Protection** | SecretString for MCP bearer and OAuth tokens |
| **MCP Validation** | Pre-spawn command validation for stdio servers |
| **Session Integrity** | HMAC-SHA256 checksum verification (RFC 2104 compliant) |
| **Seatbelt Sandbox** | macOS Seatbelt + Linux Landlock OS-level sandboxing |
| **Enterprise Policy** | Managed settings for organization-wide policy enforcement |

See [SECURITY.md](SECURITY.md) for security policy and reporting vulnerabilities.

## Configuration

Configuration directories:
- Linux/macOS: `~/.config/patina/`

### Project Context (CLAUDE.md)

Place a `CLAUDE.md` file in your project root to provide project-specific instructions. Patina automatically discovers:
- `CLAUDE.md` (project root)
- `.patina/CLAUDE.md` (framework config)
- `*/CLAUDE.md` (subdirectories)

### Plugins

Plugins extend Patina with custom tools, commands, skills, and hooks.

**Plugin Management CLI:**

```bash
# Install from local directory
patina plugin install ./my-plugin

# List installed plugins
patina plugin list

# Update all plugins
patina plugin update

# Remove a plugin
patina plugin remove my-plugin
```

**Example Plugin Manifest:**

```toml
# rct-plugin.toml
name = "my-plugin"
version = "1.0.0"
description = "My custom plugin"
author = "Your Name"

[capabilities]
commands = true   # Provides slash commands
skills = true     # Provides skills
tools = true      # Provides tools for the agent
hooks = false     # No lifecycle hooks
mcp = false       # No MCP server
```

See `plugins/template/` for a minimal plugin template to get started.

## Performance

Benchmarks (Criterion, 120x40 terminal):

| Benchmark | Target |
|-----------|--------|
| Full redraw (100 messages) | <1ms |
| Streaming token append | <100us |
| Scroll operations | <1us |
| Large message rendering | <5ms |

### Parallel Tool Execution

| Scenario | Speedup |
|----------|---------|
| Multi-file read (10 files) | 5-8x |
| Concurrent grep (5 patterns) | 4-6x |
| Mixed read operations | 3-5x |

Tools are classified by safety:
- **ReadOnly**: `read_file`, `glob`, `grep`, `web_fetch`, `web_search` (parallelized)
- **Mutating**: `write_file`, `edit` (sequential)
- **Unknown**: `bash`, MCP tools (sequential by default)

```bash
cargo bench
# HTML reports in target/criterion/
```

## Architecture

Patina uses an event-driven architecture with a priority-ordered handler dispatch system:

```
src/
├── main.rs           # CLI entry point
├── app/              # Event loop, dispatcher, and handlers
│   ├── state.rs      # Application state
│   ├── context.rs    # Handler context (shared state bundle)
│   ├── dispatch.rs   # Priority-ordered event dispatcher
│   └── handlers/     # Focused event handlers (9 + 1 observer)
│       ├── permission# Tool approval prompts
│       ├── plan      # Interactive plan review
│       ├── question  # User question prompts
│       ├── completion# Slash command tab-completion
│       ├── keyboard  # Input, copy/paste, selection
│       ├── stream    # API streaming events
│       ├── agent     # Sub-agent lifecycle
│       ├── continuous# Autonomous loop control
│       ├── tick      # UI refresh, throbber animation
│       └── session   # Session persistence (observer)
├── api/              # LLM providers (Anthropic, OpenRouter, Bedrock, Vertex, fallback)
├── tui/              # Terminal UI (ratatui), image display, transcript search
├── tools/            # Tool execution, security, parallel execution, sandbox
├── mcp/              # MCP client (rmcp SDK): config, connection, manager
├── hooks/            # Lifecycle events
├── skills/           # Context-aware suggestions
├── commands/         # Slash command parsing
├── agents/           # Worktree-based agent orchestration
├── plugins/          # Plugin system
├── session/          # Session persistence
├── context/          # Context management, compression, token budgeting
├── worktree/         # Git worktree management
├── permissions/      # Permission management
├── auth/             # Authentication (API key, optional OAuth)
├── enterprise/       # Audit logging, cost tracking, managed settings
├── update/           # Auto-update checking
└── types/            # Core types
```

## Development

```bash
# Run tests
cargo test

# Run clippy
cargo clippy --all-targets -- -D warnings

# Check formatting
cargo fmt -- --check

# Run with coverage
cargo tarpaulin --out Html

# View debug logs (TUI mode writes to file)
tail -f /tmp/patina.log
```

## Technical Details

| Metric | Value |
|--------|-------|
| Version | 1.0.0 |
| MSRV | Rust 1.85 |
| Edition | 2021 |
| Tests | 5,200+ |
| Coverage | 85%+ |
| Unsafe | 0 blocks |
| LOC (src) | ~139,000 |
| LOC (total) | ~171,000 |
| Dependencies | 534 |
| Built-in tools | 25 |
| Slash commands | 32 |
| Hook events | 13 |
| Event handlers | 9 + 1 observer |

### Key Dependencies

| Crate | Purpose |
|-------|---------|
| tokio 1.45 | Async runtime |
| ratatui 0.30 | Terminal UI |
| crossterm 0.28 | Terminal events |
| reqwest 0.12 | HTTP client (rustls) |
| rmcp 1.2 | MCP SDK (stdio, HTTP, SSE transports) |
| secrecy 0.10 | Secret storage |
| serde 1.0 | Serialization |
| clap 4.5 | CLI parsing |

## Library API

Patina can be used as a Rust library for building custom AI-powered tools.

### Available Modules

| Module | Description |
|--------|-------------|
| `patina::api` | Multi-provider LLM client (Anthropic, OpenRouter, Bedrock, Vertex) |
| `patina::tools` | Tool execution framework with security policies and sandbox |
| `patina::mcp` | MCP client: config, connection management, OAuth, tool routing |
| `patina::context` | Context management, compression, and token budgeting |
| `patina::continuous` | Continuous autonomous coding infrastructure |
| `patina::agents` | Worktree-based agent orchestration with inter-agent messaging |
| `patina::worktree` | Git worktree management and experiments |
| `patina::enterprise` | Managed settings, audit logging, cost tracking |
| `patina::narsil` | Code intelligence integration |

### Example: Custom Tool Loop

```rust,ignore
use patina::api::AnthropicClient;
use patina::types::{Message, Role, StreamEvent};
use secrecy::SecretString;

#[tokio::main]
async fn main() {
    let client = AnthropicClient::new(
        SecretString::from("your-api-key"),
        "claude-sonnet-4-20250514",
    );

    let (tx, mut rx) = tokio::sync::mpsc::channel(100);
    let messages = vec![Message {
        role: Role::User,
        content: "Hello!".to_string(),
    }];

    client.stream_message(&messages, tx).await.unwrap();

    while let Some(event) = rx.recv().await {
        match event {
            StreamEvent::ContentDelta(text) => print!("{}", text),
            StreamEvent::MessageStop => break,
            _ => {}
        }
    }
}
```

## Documentation

- [Architecture](docs/architecture.md) - System design and data flow
- [API Reference](docs/api.md) - API client documentation
- [Plugin API](docs/plugin-api.md) - Plugin development guide
- [Security Model](docs/security-model.md) - Security architecture
- [User Guide](docs/user-guide.md) - Usage documentation

## Contributing

1. Fork the repository
2. Create a feature branch
3. Ensure all quality gates pass (`cargo test`, `cargo clippy`, `cargo fmt`)
4. Submit a pull request

See [CONTRIBUTING.md](CONTRIBUTING.md) for detailed guidelines.

## License

MIT OR Apache-2.0

Copyright (c) 2026 Laurence Avent

## Author

**Laurence Avent** ([@postrv](https://github.com/postrv))

<!-- metrics: tests 5178, loc 138974, tools 27, commands 32 -->
