# RCT - Rust Claude Terminal

A high-performance terminal client for the Claude API, written in Rust.

## Features

- **Streaming TUI** - Real-time response streaming with a modern terminal UI built on ratatui
- **Anthropic API Integration** - Direct integration with Claude's Messages API
- **Skills System** - Auto-invoked context providers via SKILL.md files
- **Hooks** - Lifecycle event hooks for customizing behavior (PreToolUse, PostToolUse, etc.)
- **MCP Support** - Model Context Protocol client for tool servers (stdio, SSE, HTTP)
- **Plugin Architecture** - Extensible plugin system for commands, skills, and agents
- **Slash Commands** - User-triggered workflows via `/command` syntax
- **Project Context** - Automatic CLAUDE.md file discovery for project-specific instructions
- **IDE Integration** - TCP server for VS Code and JetBrains extension support
- **Self-Update** - Built-in update system with release channels (stable, latest, nightly)

## Installation

### From Source

```bash
git clone https://github.com/postrv/rct.git
cd rct
cargo build --release
```

The binary will be available at `target/release/rct`.

## Usage

```bash
# Set your API key
export ANTHROPIC_API_KEY="your-api-key"

# Run rct
rct

# Or specify options
rct --model claude-sonnet-4-20250514 --directory /path/to/project
```

### Command Line Options

| Option | Description | Default |
|--------|-------------|---------|
| `--api-key` | Anthropic API key (or set `ANTHROPIC_API_KEY`) | - |
| `-m, --model` | Model to use | `claude-sonnet-4-20250514` |
| `-C, --directory` | Working directory | `.` |
| `--debug` | Enable debug logging | `false` |

## Key Bindings

| Key | Action |
|-----|--------|
| `Enter` | Send message |
| `Ctrl+C` / `Ctrl+D` | Quit |
| `Ctrl+Up` / `PageUp` | Scroll up |
| `Ctrl+Down` / `PageDown` | Scroll down |

## Project Structure

```
src/
├── main.rs          # Entry point, CLI parsing
├── app/             # Application core, event loop
├── api/             # Anthropic API client
├── tui/             # Terminal UI rendering
├── skills/          # Skills system
├── hooks/           # Hook execution engine
├── mcp/             # MCP client
├── plugins/         # Plugin management
├── commands/        # Slash commands
├── context/         # CLAUDE.md support
├── tools/           # Tool execution
├── agents/          # Subagent orchestration
├── update/          # Self-update system
├── ide/             # IDE integration
└── util/            # Utility functions
```

## Configuration

RCT looks for configuration in:
- `~/.config/rct/` (Linux/macOS)
- `%APPDATA%\rct\` (Windows)

### Project Context (CLAUDE.md)

Place a `CLAUDE.md` file in your project root to provide project-specific context to Claude. RCT automatically discovers and includes these files in the conversation.

## Development

```bash
# Run in debug mode
cargo run

# Run tests
cargo test

# Check for issues
cargo clippy
```

## Documentation

See the `docs/` directory for detailed documentation:
- [Bootstrap Guide](docs/rct-bootstrap-guide.md)
- [Implementation Plan](docs/rct-implementation-plan.md)

## License

MIT OR Apache-2.0
