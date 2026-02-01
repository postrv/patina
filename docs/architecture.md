# Patina Architecture

## Overview

Patina is a high-performance Rust terminal client for the Claude API. It provides an interactive chat interface with tool execution capabilities, session persistence, and a modular plugin system.

## Module Structure

```
patina/
├── api/            # Anthropic API client with streaming support
├── app/            # Application state and event loop
│   ├── state.rs    # AppState - unified state management
│   ├── tool_loop.rs # Tool execution state machine
│   └── mod.rs      # Main event loop
├── auth/           # OAuth authentication flow
├── commands/       # Slash commands (/help, /worktree, etc.)
├── context/        # Project context loading (CLAUDE.md)
├── hooks/          # Lifecycle hooks (pre/post tool execution)
├── mcp/            # Model Context Protocol client
├── permissions/    # Tool permission management
├── plugins/        # Plugin system with manifest-based discovery
├── session/        # Session persistence with HMAC integrity
├── skills/         # Skill engine for context-aware suggestions
├── tools/          # Tool execution (bash, file ops, etc.)
├── tui/            # Terminal UI (ratatui)
│   ├── mod.rs      # Main rendering
│   ├── scroll.rs   # Smart auto-scroll
│   ├── theme.rs    # Patina color theme
│   └── widgets/    # Reusable widgets
├── types/          # Core types
│   ├── conversation.rs # Timeline - unified display model
│   ├── content.rs  # ContentBlock, StopReason
│   ├── message.rs  # Message, ApiMessageV2
│   └── stream.rs   # StreamEvent types
├── update/         # Auto-update system
└── worktree/       # Git worktree integration
```

## Key Dependencies

**Internal:**
- `app` depends on `api`, `tools`, `tui`, `types`
- `tui` depends on `types`
- `tools` depends on `permissions`, `hooks`

**External:**
- `tokio` - Async runtime
- `ratatui` - Terminal UI framework
- `reqwest` - HTTP client
- `serde` - Serialization
- `anyhow` - Error handling

## Data Flow

### Conversation Timeline Model

The `Timeline` struct is the **single source of truth** for conversation display:

```
┌─────────────────────────────────────────────────────────┐
│                    Timeline                              │
├─────────────────────────────────────────────────────────┤
│  Entry 0: UserMessage("Hello!")                         │
│  Entry 1: AssistantMessage("Hi there!")                 │
│  Entry 2: ToolExecution { name: "bash", output: "..." } │
│  Entry 3: Streaming { text: "Let me help...", ... }     │
└─────────────────────────────────────────────────────────┘
```

The timeline replaces the previous dual-system of `messages` + `current_response`:

1. **UserMessage** - Complete user messages
2. **AssistantMessage** - Complete assistant responses
3. **ToolExecution** - Tool calls with inputs and outputs
4. **Streaming** - Currently streaming response (only one at a time)

### Message Flow

```
User Input
    │
    ▼
┌────────────┐
│  AppState  │──▶ Timeline.push_user_message()
│            │──▶ api_messages.push(ApiMessageV2::user())
└────────────┘
    │
    ▼
┌────────────┐
│ API Client │──▶ Stream ContentDelta events
└────────────┘
    │
    ▼
┌────────────┐
│ Timeline   │──▶ append_to_streaming()
│            │──▶ finalize_streaming_as_message()
└────────────┘
    │
    ▼
┌────────────┐
│ TUI Render │──▶ render_timeline_with_throbber()
└────────────┘
```

### Tool Execution Flow

```
StreamEvent::ToolUseStart
    │
    ▼
┌────────────┐
│ ToolLoop   │──▶ Add pending tool
└────────────┘
    │
    ▼
StreamEvent::MessageComplete (stop_reason=ToolUse)
    │
    ▼
┌────────────┐
│ Permission │──▶ Check/prompt for approval
│  Manager   │
└────────────┘
    │
    ▼
┌────────────┐
│ Hooked     │──▶ Execute tool with hooks
│ Executor   │
└────────────┘
    │
    ▼
┌────────────┐
│ Timeline   │──▶ push_tool_after_current_assistant()
└────────────┘
    │
    ▼
Continue conversation with tool results
```

## State Management

### AppState Structure

```rust
pub struct AppState {
    // API conversation history (with content blocks)
    api_messages: Vec<ApiMessageV2>,

    // Unified display timeline
    timeline: Timeline,

    // Tool execution state
    tool_loop: ToolLoop,
    tool_executor: Arc<HookedToolExecutor>,
    permission_manager: Arc<Mutex<PermissionManager>>,

    // Smart scroll state
    scroll: ScrollState,

    // UI state
    input: String,
    cursor_pos: usize,
    loading: bool,

    // Session tracking
    session_id: Option<String>,
}
```

### Timeline Streaming Lifecycle

1. `set_streaming(true)` - Creates `Streaming` entry
2. `append_streaming_text(chunk)` - Accumulates text
3. `finalize_streaming_as_message()` - Converts to `AssistantMessage`

### Session Persistence

Sessions are persisted as JSON with HMAC-SHA256 integrity:

```
Timeline ──▶ Convert to Messages ──▶ Session ──▶ JSON + HMAC ──▶ File
```

On restore:
```
File ──▶ Verify HMAC ──▶ Session ──▶ Messages ──▶ Timeline
```

## Security Model

### Permission System

Tools require explicit permission before execution:

- **Allow Once** - Grant for this invocation only
- **Allow Always** - Remember for similar commands
- **Deny** - Block execution

### Dangerous Command Detection

Commands are classified as dangerous if they contain:
- Destructive patterns (`rm -rf`, `--force`, etc.)
- Privilege escalation (`sudo`, `chmod 777`)
- Network operations (`curl | sh`, etc.)

### Session Integrity

Sessions use HMAC-SHA256 to prevent tampering:
- Key derived from session file path
- Verification on load
- Re-sign on save

## TUI Rendering

The TUI uses `ratatui` with timeline-based rendering:

```rust
pub fn render_timeline_with_throbber(
    timeline: &Timeline,
    throbber: char,
) -> Vec<Line<'static>>
```

Key features:
- **Smart Auto-Scroll** - Follows new content in Follow mode
- **Patina Theme** - Bronze/verdigris color palette
- **Tool Block Widgets** - Distinct styling for tool executions

---
*Last updated: 2026-01-31*
