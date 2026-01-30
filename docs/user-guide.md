# RCT User Guide

Rust Claude Terminal - A high-performance CLI for the Claude API.

## Installation

### From Source

```bash
# Clone the repository
git clone https://github.com/your-org/rct.git
cd rct

# Build release binary
cargo build --release

# Install to ~/.cargo/bin
cargo install --path .
```

### Requirements

- Rust 1.75 or later
- An Anthropic API key

## Quick Start

1. Set your API key:

```bash
export ANTHROPIC_API_KEY=sk-ant-...
```

2. Run RCT:

```bash
rct
```

3. Start chatting with Claude in your terminal.

## CLI Options

```
rct [OPTIONS]

Options:
  --api-key <API_KEY>       API key (or set ANTHROPIC_API_KEY env var)
  -m, --model <MODEL>       Model to use [default: claude-sonnet-4-20250514]
  -C, --directory <DIR>     Working directory [default: .]
      --debug               Enable debug logging
  -h, --help                Print help
  -V, --version             Print version
```

### Examples

```bash
# Use a specific model
rct --model claude-opus-4-20250514

# Work in a specific directory
rct -C /path/to/project

# Enable debug logging
rct --debug
```

## Features

### Interactive Chat

RCT provides a terminal-based chat interface with:

- Streaming responses with real-time display
- Syntax highlighting for code blocks
- Markdown rendering
- Message history with scrolling
- Multi-line input support

#### Keyboard Shortcuts

| Key | Action |
|-----|--------|
| Enter | Send message |
| Ctrl+C | Cancel/Exit |
| Up/Down | Scroll messages |
| Home/End | Jump to start/end |

### Tools

RCT includes built-in tools that Claude can use to interact with your system:

#### bash

Executes shell commands in your working directory.

```json
{
  "name": "bash",
  "input": {
    "command": "ls -la"
  }
}
```

**Security**: Dangerous commands are blocked by default, including:
- `rm -rf /` and other destructive operations
- `sudo` and privilege escalation
- `chmod 777` and dangerous permissions
- Remote code execution (`curl | bash`)
- System disruption (`shutdown`, `reboot`)

#### read_file

Reads file contents within the working directory.

```json
{
  "name": "read_file",
  "input": {
    "path": "src/main.rs"
  }
}
```

#### write_file

Writes content to a file. Creates backups automatically.

```json
{
  "name": "write_file",
  "input": {
    "path": "output.txt",
    "content": "Hello, world!"
  }
}
```

#### edit

Performs string replacement edits. Requires a unique match.

```json
{
  "name": "edit",
  "input": {
    "path": "src/main.rs",
    "old_string": "fn old_name()",
    "new_string": "fn new_name()"
  }
}
```

#### glob

Searches for files matching a glob pattern.

```json
{
  "name": "glob",
  "input": {
    "pattern": "**/*.rs",
    "respect_gitignore": true
  }
}
```

#### grep

Searches file contents with regex patterns.

```json
{
  "name": "grep",
  "input": {
    "pattern": "TODO|FIXME",
    "case_insensitive": true,
    "file_pattern": "*.rs"
  }
}
```

### Slash Commands

Define custom workflows with markdown-based slash commands.

#### Creating Commands

Create a markdown file in `.rct/commands/`:

```markdown
---
name: commit
description: Create a git commit with a message
args:
  - name: message
    required: true
    arg_type: string
---

Please create a git commit with the following message: {{ message }}

Make sure to:
1. Stage all modified files
2. Run tests before committing
3. Use a conventional commit format
```

#### Using Commands

```
/commit "feat: add new feature"
```

### Skills

Skills provide context-aware assistance based on file patterns and keywords.

#### Creating Skills

Create a markdown file in `.rct/skills/`:

```markdown
---
name: rust-development
description: Rust development assistance
triggers:
  file_patterns:
    - "*.rs"
    - "Cargo.toml"
  keywords:
    - rust
    - cargo
allowed_tools:
  - bash
  - read_file
  - write_file
  - edit
---

You are a Rust development expert. When working with Rust code:

1. Use idiomatic Rust patterns
2. Prefer `Result` over `panic!`
3. Write comprehensive tests
4. Use clippy and rustfmt
```

#### Skill Matching

Skills are automatically activated based on:
- File patterns being worked on
- Keywords in the conversation
- Explicit activation

### Hooks

Hooks allow custom actions at lifecycle events.

#### Configuration

Create `.rct/hooks.toml`:

```toml
[[PreToolUse]]
matcher = "bash"
hooks = [
  { type = "command", command = "echo 'About to run bash command'" }
]

[[PostToolUse]]
hooks = [
  { type = "command", command = "notify-send 'Tool completed'" }
]

[[SessionStart]]
hooks = [
  { type = "command", command = "echo 'Session started' >> ~/.rct/session.log" }
]
```

#### Hook Events

| Event | Description |
|-------|-------------|
| PreToolUse | Before tool execution (can block with exit code 2) |
| PostToolUse | After successful tool execution |
| PostToolUseFailure | After failed tool execution |
| PermissionRequest | When permission is requested |
| UserPromptSubmit | When user submits a prompt |
| SessionStart | When session begins |
| SessionEnd | When session ends |
| Notification | When a notification is sent |
| Stop | When stop is requested |
| SubagentStop | When a subagent stops |
| PreCompact | Before context compaction |

#### Matcher Patterns

Hooks can filter by tool name using patterns:

- Exact match: `"bash"`
- Multiple tools: `"bash|read_file|write_file"`
- Glob patterns: `"mcp__*"`

### MCP Integration

RCT supports the Model Context Protocol (MCP) for external tool servers.

#### Configuration

Create `.mcp.json`:

```json
{
  "servers": {
    "filesystem": {
      "command": "mcp-server-filesystem",
      "args": ["/allowed/path"],
      "env": {}
    },
    "remote": {
      "url": "https://mcp.example.com/sse",
      "headers": {
        "Authorization": "Bearer token"
      }
    }
  }
}
```

#### Transport Types

- **stdio**: Local process with JSON-RPC over stdin/stdout
- **SSE**: Remote server with Server-Sent Events

### Plugins

Extend RCT with custom plugins.

#### Plugin Structure

```
my-plugin/
├── manifest.yaml
├── commands/
│   └── hello.md
└── skills/
    └── greeting.md
```

#### manifest.yaml

```yaml
name: my-plugin
version: "1.0.0"
description: My custom plugin
author: Your Name
min_rct_version: "0.1.0"
```

#### Loading Plugins

Place plugins in:
- User: `~/.rct/plugins/`
- Project: `.rct/plugins/`

### Multi-Model Support

Switch between Claude models and providers.

#### Available Models

- `claude-opus-4-20250514` - Most capable
- `claude-sonnet-4-20250514` - Balanced (default)
- `claude-haiku-3-20250307` - Fast and efficient

#### Model Aliases

- `opus` → `claude-opus-4-20250514`
- `sonnet` → `claude-sonnet-4-20250514`
- `haiku` → `claude-haiku-3-20250307`

#### AWS Bedrock Support

```bash
# Configure Bedrock provider
export AWS_REGION=us-east-1
rct --provider bedrock --model anthropic.claude-v2
```

### Session Persistence

Save and resume conversation sessions.

#### Session Storage

Sessions are stored in `~/.rct/sessions/` as JSON files containing:
- Message history
- Working directory
- Timestamps
- Session metadata

### Enterprise Features

#### Audit Logging

Track all operations for compliance and debugging.

Configuration in `.rct/config.toml`:

```toml
[audit]
enabled = true
level = "all"  # all, api_only, tools_only, session_only
path = "~/.rct/audit/"
```

Audit entries include:
- Tool invocations
- API calls
- Session lifecycle events

#### Cost Controls

Monitor and limit API usage costs.

```toml
[cost]
enabled = true
warn_threshold = 10.0  # USD
session_limit = 50.0   # USD
daily_limit = 100.0    # USD
monthly_limit = 500.0  # USD
```

Model pricing (per million tokens):

| Model | Input | Output |
|-------|-------|--------|
| Opus | $15.00 | $75.00 |
| Sonnet | $3.00 | $15.00 |
| Haiku | $0.25 | $1.25 |

## Configuration

### Configuration Files

RCT looks for configuration in these locations (in order):

1. `.rct/` in the current directory (project-specific)
2. `~/.rct/` (user-specific)
3. Environment variables

### Directory Structure

```
~/.rct/
├── config.toml       # Main configuration
├── hooks.toml        # Hook definitions
├── commands/         # Slash commands
├── skills/           # Context skills
├── plugins/          # Installed plugins
├── sessions/         # Saved sessions
├── audit/            # Audit logs
└── backups/          # File backups
```

### Environment Variables

| Variable | Description |
|----------|-------------|
| ANTHROPIC_API_KEY | Your Anthropic API key |
| RCT_MODEL | Default model to use |
| RCT_DEBUG | Enable debug logging (1/true) |

## Security

### Security Model

RCT implements multiple layers of protection:

1. **Command Blocking**: Dangerous bash commands are blocked
2. **Path Validation**: File operations are restricted to the working directory
3. **Protected Paths**: System directories (/etc, /usr, /bin) are write-protected
4. **Backup System**: Files are backed up before modification
5. **Timeout Enforcement**: Commands have configurable timeouts

### Path Traversal Protection

All file operations validate paths to prevent escaping the working directory:

- Absolute paths are rejected
- `..` traversal is blocked
- Symlinks are not followed

### Protected Directories

Write operations are blocked in:
- `/etc`
- `/usr`
- `/bin`
- Other system directories

### Audit Trail

Enable audit logging to track all operations:

```toml
[audit]
enabled = true
```

## Troubleshooting

### Common Issues

#### "API key required"

Set your API key:
```bash
export ANTHROPIC_API_KEY=sk-ant-...
```

#### "Command blocked by security policy"

The command matches a dangerous pattern. Review the command or use a safer alternative.

#### "Path traversal outside working directory"

File operations must stay within the working directory. Use relative paths.

### Debug Mode

Enable debug logging for troubleshooting:

```bash
rct --debug
```

Or set the environment variable:

```bash
export RCT_DEBUG=1
rct
```

### Getting Help

- Check the [API documentation](api.md)
- File issues on GitHub
- Join the community discussions

---

*RCT v0.1.0*
