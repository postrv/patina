<p align="center">
  <img src="patina.jpg" alt="Patina Logo" width="300">
</p>

# Patina

[![CI](https://github.com/postrv/patina/actions/workflows/ci.yml/badge.svg)](https://github.com/postrv/patina/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)

A high-performance terminal client for the Claude API, written in Rust. Designed for developers who want a fast, secure, and extensible AI assistant in their terminal.

## Highlights

- **Sub-millisecond rendering** - Full 100-message redraw in <1ms
- **Multi-provider LLM support** - Claude (Anthropic) and OpenRouter-compatible models with automatic failover
- **Parallel tool execution** - 5x+ speedup on multi-file operations
- **Autonomous agent orchestration** - Spawn parallel sub-agents in isolated git worktrees
- **Continuous coding loop** - Run tasks autonomously with stagnation detection and quality gates
- **3,500+ tests** with 85%+ code coverage
- **Zero unsafe code** - Pure safe Rust (~87,000 LOC)
- **Cross-platform** - Linux, macOS, Windows
- **Security-first** - Defense-in-depth with command filtering, path validation, and session integrity

## Features

### Core Capabilities

| Feature | Description |
|---------|-------------|
| **Streaming TUI** | Real-time response streaming with syntax highlighting |
| **Agentic Tool Loop** | Claude can autonomously execute tools and continue conversations |
| **Multi-Provider LLM** | Anthropic, OpenRouter, and any OpenAI-compatible API |
| **Provider Failover** | Automatic fallback between providers on failure |
| **Parallel Execution** | Concurrent tool execution with safety classification (5x+ speedup) |
| **Session Resume** | Save and restore conversations with full context |
| **Context Compaction** | Automatic summarization when context window fills |
| **Context Compression** | Intelligent context building with token budgeting and narsil-mcp integration |
| **MCP Support** | Model Context Protocol for tool server integration |

### Autonomous Agents

| Feature | Description |
|---------|-------------|
| **Worktree Agents** | Spawn sub-agents in isolated git worktrees for parallel work |
| **Conflict Detection** | Cross-agent file conflict detection before merging |
| **Continuous Loop** | Run tasks autonomously with `/continuous` command |
| **Stagnation Detection** | Multi-factor scoring detects stuck agents and triggers recovery |
| **Quality Gates** | Automated clippy, test, and format checks with timeout enforcement |

### Built-in Tools

| Tool | Description |
|------|-------------|
| `bash` | Execute shell commands with security filtering |
| `read_file` | Read file contents with path traversal protection |
| `write_file` | Write files with validation |
| `edit` | Edit files with diff-based changes |
| `glob` | File discovery with pattern matching |
| `grep` | Content search with regex support |
| `web_fetch` | Fetch and convert web pages to markdown |
| `web_search` | Search the web via DuckDuckGo |
| `vision` | Analyze images (PNG, JPEG, GIF, WebP) |

### Extensibility

| Feature | Description |
|---------|-------------|
| **Plugin System** | TOML-based plugins with auto-discovery |
| **Skills Engine** | Context-aware suggestions via SKILL.md files |
| **Hooks** | 11 lifecycle events (PreToolUse, PostToolUse, SessionStart, etc.) |
| **Slash Commands** | `/worktree`, `/agent`, `/continuous`, `/help`, and user-defined workflows |

### Developer Experience

| Feature | Description |
|---------|-------------|
| **Project Context** | Automatic CLAUDE.md discovery for project instructions |
| **Git Worktrees** | Parallel AI-assisted development with isolation |
| **IDE Integration** | TCP server for VS Code and JetBrains extensions |
| **narsil-mcp** | Optional code intelligence with 90 analysis tools |

## Installation

### Pre-built Binaries

Download from [GitHub Releases](https://github.com/postrv/patina/releases):

| Platform | Download |
|----------|----------|
| Linux x86_64 | `patina-linux-x86_64.tar.gz` |
| Linux x86_64 (static) | `patina-linux-x86_64-musl.tar.gz` |
| macOS x86_64 | `patina-macos-x86_64.tar.gz` |
| macOS Apple Silicon | `patina-macos-aarch64.tar.gz` |
| Windows x86_64 | `patina-windows-x86_64.zip` |

### From Source

```bash
git clone https://github.com/postrv/patina.git
cd patina
cargo build --release
# Binary at target/release/patina
```

### Cargo

```bash
cargo install patina
```

### Docker

```bash
docker pull ghcr.io/postrv/patina:latest
docker run -it -e ANTHROPIC_API_KEY="$ANTHROPIC_API_KEY" ghcr.io/postrv/patina
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
```

## Command Line Options

| Option | Description | Default |
|--------|-------------|---------|
| `[PROMPT]` | Initial prompt to start with | - |
| `-p, --print` | Print mode (non-interactive) | `false` |
| `--api-key` | API key (or `ANTHROPIC_API_KEY` env) | - |
| `-m, --model` | Model to use | `claude-sonnet-4-20250514` |
| `--provider` | LLM provider (`anthropic`, `openrouter`) | `anthropic` |
| `--fallback-provider` | Fallback provider on failure | - |
| `-C, --directory` | Working directory | `.` |
| `-c, --continue` | Resume most recent session | - |
| `-r, --resume` | Resume specific session by ID | - |
| `--list-sessions` | List available sessions | - |
| `--with-narsil` | Enable narsil-mcp integration | auto |
| `--no-narsil` | Disable narsil-mcp integration | - |
| `--no-parallel` | Disable parallel tool execution | - |
| `--parallel-aggressive` | Parallelize all tools (use with caution) | - |
| `--debug` | Enable debug logging | `false` |

## Key Bindings

| Key | Action |
|-----|--------|
| `Enter` | Send message |
| `Ctrl+C` / `Ctrl+D` | Quit |
| `PageUp` / `PageDown` | Scroll conversation |
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

## Slash Commands

| Command | Description |
|---------|-------------|
| `/worktree new <name>` | Create new git worktree |
| `/worktree list` | List all worktrees |
| `/worktree switch <name>` | Switch to worktree |
| `/worktree remove <name>` | Remove worktree |
| `/worktree status` | Show worktree status |
| `/agent spawn <task>` | Spawn a sub-agent in an isolated worktree |
| `/agent list` | List running agents |
| `/agent merge <name>` | Merge agent changes back |
| `/continuous` | Start autonomous coding loop with quality gates |
| `/help` | Show available commands |

## Security

Patina implements defense-in-depth security controls:

| Control | Implementation |
|---------|----------------|
| **Command Filtering** | 28+ dangerous patterns blocked (rm -rf, sudo, etc.) |
| **Path Validation** | Canonicalization + symlink protection |
| **Permission System** | Explicit approval required for tool execution |
| **API Key Protection** | SecretString with `[REDACTED]` in logs |
| **MCP Validation** | Pre-spawn command validation |
| **Session Integrity** | HMAC-SHA256 checksum verification |

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

# Install from GitHub (coming soon)
patina plugin install gh:user/repo

# List installed plugins
patina plugin list

# Update all plugins
patina plugin update

# Remove a plugin
patina plugin remove my-plugin
```

**Example Plugin Manifest:**

```toml
# patina-plugin.toml
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

## MCP Support

Patina implements the [Model Context Protocol](https://spec.modelcontextprotocol.io/) for tool server integration:

- **Protocol:** JSON-RPC 2.0
- **Transports:** stdio (default), HTTP SSE
- **Security:** Command validation, interpreter path requirements
- **Parallel detection:** Concurrent capability probing across MCP servers

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
│   └── handlers/     # Focused event handlers
│       ├── keyboard  # Input, copy/paste, selection
│       ├── stream    # API streaming events
│       ├── session   # Session persistence
│       ├── permission# Tool approval prompts
│       ├── tick      # UI refresh, throbber animation
│       ├── agent     # Sub-agent lifecycle
│       └── continuous# Autonomous loop control
├── api/              # LLM providers (Anthropic, OpenRouter, fallback)
├── tui/              # Terminal UI (ratatui), image display
├── tools/            # Tool execution, security, parallel execution
├── mcp/              # Model Context Protocol client
├── hooks/            # Lifecycle events
├── skills/           # Context-aware suggestions
├── commands/         # Slash command parsing
├── agents/           # Worktree-based agent orchestration
├── plugins/          # Plugin system
├── session/          # Session persistence
├── context/          # Context management, compression, token budgeting
├── worktree/         # Git worktree management
├── permissions/      # Permission management
├── auth/             # Authentication (API key, OAuth scaffolding)
├── enterprise/       # Audit logging, cost tracking
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
```

## Technical Details

| Metric | Value |
|--------|-------|
| Version | 1.0.0 |
| MSRV | Rust 1.75 |
| Edition | 2021 |
| Tests | 3,500+ |
| Coverage | 85%+ |
| Unsafe | 0 blocks |
| LOC | ~87,000 |

### Key Dependencies

| Crate | Purpose |
|-------|---------|
| tokio 1.45 | Async runtime |
| ratatui 0.30 | Terminal UI |
| crossterm 0.28 | Terminal events |
| reqwest 0.12 | HTTP client (rustls) |
| secrecy 0.10 | Secret storage |
| serde 1.0 | Serialization |
| clap 4.5 | CLI parsing |

## Library API

Patina can be used as a Rust library for building custom AI-powered tools:

```toml
[dependencies]
patina = "0.9"
```

### Available Modules

| Module | Description |
|--------|-------------|
| `patina::api` | Multi-provider LLM client with streaming support |
| `patina::tools` | Tool execution framework with security policies |
| `patina::mcp` | Model Context Protocol client |
| `patina::context` | Context management, compression, and token budgeting |
| `patina::continuous` | Continuous autonomous coding infrastructure |
| `patina::agents` | Worktree-based agent orchestration |
| `patina::worktree` | Git worktree management and experiments |
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

<!-- METRICS:tests=3500,loc=87000 -->
