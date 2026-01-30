# RCT - Rust Claude Terminal

[![CI](https://github.com/postrv/rct/actions/workflows/ci.yml/badge.svg)](https://github.com/postrv/rct/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)

A high-performance terminal client for the Claude API, written in Rust. Feature parity with Claude Code plus performance superiority.

## Highlights

- **Sub-millisecond rendering** - Full 100-message redraw in <1ms
- **624 tests** with 85%+ code coverage
- **Zero unsafe code** - Pure safe Rust
- **Cross-platform** - Linux, macOS, Windows
- **Security-first** - 8/8 security audit findings resolved

## Features

### Core

| Feature | Description |
|---------|-------------|
| **Streaming TUI** | Real-time response streaming with ratatui (120x40 terminal) |
| **Anthropic API** | Direct integration with Messages API, exponential backoff retry |
| **Multi-Model** | Anthropic direct + AWS Bedrock provider support |
| **MCP Client** | Model Context Protocol with stdio/SSE transports |
| **Session Persistence** | Save/load conversations with HMAC-SHA256 integrity |

### Extensibility

| Feature | Description |
|---------|-------------|
| **Skills System** | Auto-invoked context providers via SKILL.md files |
| **Hooks** | 10 lifecycle events (PreToolUse, PostToolUse, SessionStart, etc.) |
| **Slash Commands** | User-triggered workflows via `/command` syntax |
| **Plugin Architecture** | Extensible commands, skills, and agents |
| **Subagent Orchestration** | Multi-agent coordination (4 concurrent by default) |

### Developer Experience

| Feature | Description |
|---------|-------------|
| **Project Context** | Automatic CLAUDE.md discovery for project instructions |
| **IDE Integration** | TCP server for VS Code and JetBrains extensions |
| **Self-Update** | Built-in updates with release channels (stable, latest, nightly) |

### Enterprise

| Feature | Description |
|---------|-------------|
| **Audit Logging** | Configurable levels (All, ApiOnly, ToolsOnly, SessionOnly) |
| **Cost Tracking** | Budget limits per session/daily/monthly with warnings |

## Installation

### Pre-built Binaries

Download from [GitHub Releases](https://github.com/postrv/rct/releases):

| Platform | Download |
|----------|----------|
| Linux x86_64 | `rct-linux-x86_64.tar.gz` |
| Linux x86_64 (static) | `rct-linux-x86_64-musl.tar.gz` |
| macOS x86_64 | `rct-macos-x86_64.tar.gz` |
| macOS Apple Silicon | `rct-macos-aarch64.tar.gz` |
| Windows x86_64 | `rct-windows-x86_64.zip` |

All releases include SHA256 checksums.

### From Source

```bash
git clone https://github.com/postrv/rct.git
cd rct
cargo build --release
# Binary at target/release/rct
```

### Docker

```bash
docker pull ghcr.io/postrv/rct:latest
docker run -it -e ANTHROPIC_API_KEY="$ANTHROPIC_API_KEY" ghcr.io/postrv/rct
```

## Usage

```bash
# Set your API key
export ANTHROPIC_API_KEY="your-api-key"

# Run rct
rct

# With options
rct --model claude-sonnet-4-20250514 --directory /path/to/project
```

### Command Line Options

| Option | Description | Default |
|--------|-------------|---------|
| `--api-key` | Anthropic API key (or `ANTHROPIC_API_KEY` env) | - |
| `-m, --model` | Model to use | `claude-sonnet-4-20250514` |
| `-C, --directory` | Working directory | `.` |
| `--debug` | Enable debug logging | `false` |

### Key Bindings

| Key | Action |
|-----|--------|
| `Enter` | Send message |
| `Ctrl+C` / `Ctrl+D` | Quit |
| `Ctrl+Up` / `PageUp` | Scroll up |
| `Ctrl+Down` / `PageDown` | Scroll down |
| `Backspace` | Delete character |
| `Home` / `End` | Move cursor |

## Built-in Tools

RCT provides 6 integrated tools with security policy enforcement:

| Tool | Description | Security |
|------|-------------|----------|
| `bash` | Execute shell commands | Dangerous pattern blocking (28+ patterns) |
| `read` | Read file contents | Path traversal + symlink protection |
| `write` | Write files | Path validation |
| `edit` | Edit files with diff | Path validation |
| `glob` | File discovery | Pattern validation |
| `grep` | Content search | Regex compilation, path validation |

## MCP Support

RCT implements the [Model Context Protocol](https://spec.modelcontextprotocol.io/) for tool server integration.

**Protocol:** JSON-RPC 2.0

**Transports:**
- `StdioTransport` - Process stdin/stdout (default)
- `SseTransport` - HTTP Server-Sent Events for remote servers

**Security:**
- Always-blocked commands validation
- Interpreter absolute path requirements
- Shell injection detection in arguments

## Configuration

Configuration directories:
- Linux/macOS: `~/.config/rct/`
- Windows: `%APPDATA%\rct\`

### Project Context (CLAUDE.md)

Place a `CLAUDE.md` file in your project root to provide project-specific context. RCT automatically discovers:
- Root: `CLAUDE.md`
- Framework: `.rct/CLAUDE.md`
- Subdirectories: `*/CLAUDE.md`

## Security

RCT implements defense-in-depth security:

| Control | Implementation |
|---------|----------------|
| Command Filtering | 28+ dangerous patterns blocklist + allowlist mode |
| Path Traversal | Canonicalization + `..` rejection |
| Symlink Protection | TOCTOU mitigation via `symlink_metadata()` |
| API Key Protection | SecretString with `[REDACTED]` debug output |
| MCP Validation | Pre-spawn command validation |
| Session Integrity | HMAC-SHA256 checksum verification |

**Blocked Commands (Unix):** rm, sudo, su, chmod 777, mkfs, dd, shutdown, curl\|bash, eval, fork bombs, etc.

**Blocked Commands (Windows):** reg.exe, shutdown.exe, format, del.exe, powershell -enc, iex(), certutil, etc.

See [Security Model](docs/security-model.md) and [Security Audit](docs/SECURITY_AUDIT.md) for details.

## Performance

Benchmarks (Criterion, 120x40 terminal):

| Benchmark | Target | Description |
|-----------|--------|-------------|
| `full_redraw_100_messages` | <1ms | Complete redraw with 100 messages |
| `streaming_token_append` | <100μs | Single token append during streaming |
| `streaming_cycle` | <500μs | Append + render cycle |
| `input_character_echo` | <10μs | Keypress handling |
| `cursor_movement` | <10μs | Cursor navigation |
| `scroll_operations` | <1μs | Scroll up/down |
| `large_message_rendering` | <5ms | 20 messages, ~5000 chars each |

Run benchmarks:
```bash
cargo bench
# HTML reports in target/criterion/
```

## Project Structure

```
src/
├── main.rs           # CLI entry point
├── lib.rs            # Library exports
├── app/              # Event loop, application state
├── api/              # Anthropic API client, multi-model
├── tui/              # Terminal UI (ratatui)
├── tools/            # Tool execution, security policy
├── mcp/              # MCP client (protocol, transports)
├── hooks/            # Lifecycle event execution
├── skills/           # Context-aware skill matching
├── commands/         # Slash command parsing
├── agents/           # Subagent orchestration
├── plugins/          # Plugin system
├── session/          # Session persistence
├── enterprise/       # Audit logging, cost tracking
├── context/          # Project context (CLAUDE.md)
├── update/           # Self-update system
├── ide/              # IDE integration (TCP)
├── shell/            # Cross-platform shell abstraction
├── types/            # Core types (Message, Role)
└── error.rs          # Error types
```

## Development

```bash
# Run in debug mode
cargo run

# Run tests
cargo test

# Run with coverage
cargo tarpaulin --out Html

# Check for issues
cargo clippy --all-targets -- -D warnings

# Format code
cargo fmt
```

### Quality Gates

| Gate | Command | Requirement |
|------|---------|-------------|
| Clippy | `cargo clippy --all-targets -- -D warnings` | 0 warnings |
| Tests | `cargo test` | All pass |
| Format | `cargo fmt -- --check` | No changes |

## Documentation

| Document | Description |
|----------|-------------|
| [Architecture](docs/architecture.md) | System design and module overview |
| [API Reference](docs/api.md) | API client documentation |
| [Plugin API](docs/plugin-api.md) | Plugin development guide |
| [User Guide](docs/user-guide.md) | End-user documentation |
| [Security Model](docs/security-model.md) | Security architecture |
| [Security Audit](docs/SECURITY_AUDIT.md) | Audit findings and resolutions |

## Technical Details

| Metric | Value |
|--------|-------|
| Version | 0.1.0 |
| MSRV | Rust 1.75 |
| Edition | 2021 |
| Tests | 624 |
| Coverage | 85%+ |
| Unsafe | 0 blocks |

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

## Contributing

1. Fork the repository
2. Create a feature branch
3. Ensure all quality gates pass
4. Submit a pull request

See [CLAUDE.md](.claude/CLAUDE.md) for development standards.

## License

MIT OR Apache-2.0
