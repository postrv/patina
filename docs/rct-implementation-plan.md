# Rust Claude Terminal (RCT) Implementation Plan

## Technical Specification & Development Roadmap

**Document Version:** 2.0  
**Date:** January 28, 2026  
**Classification:** Technical Implementation Plan

---

## Executive Summary

This document outlines a formal implementation plan for **Rust Claude Terminal (RCT)**, a high-performance, event-driven terminal client for Anthropic's Claude API. The project addresses fundamental architectural deficiencies in the existing Claude Code implementation, specifically the inappropriate use of React/Ink for a TUI application that has resulted in:

- 11ms+ React scene graph construction overhead per frame
- Unnecessary 60 FPS game-loop rendering model
- GC pauses and memory bloat from Node.js runtime
- Flickering, input lag, and poor responsiveness

RCT will demonstrate that a "simple" TUI does not require game-engine complexity, delivering sub-millisecond frame times through event-driven architecture and zero-overhead Rust abstractions.

**Full Feature Parity Goals:**

This implementation targets complete feature parity with Claude Code, including:

- **Plugins** — Loadable extensions with commands, agents, skills, and MCP servers
- **Skills** — Auto-invoked contextual capabilities (SKILL.md system)
- **Hooks** — Lifecycle event handlers (PreToolUse, PostToolUse, SessionStart, etc.)
- **MCP Integration** — Model Context Protocol server management
- **Subagents** — Parallel task execution with isolated contexts
- **Live Updates** — Background self-update with release channels
- **IDE Integration** — VS Code/JetBrains extension protocol compatibility

---

## 1. Problem Analysis

### 1.1 Identified Architectural Issues

Based on public commentary from Anthropic engineers and community analysis, Claude Code's current architecture suffers from:

| Issue | Current Implementation | Impact |
|-------|----------------------|--------|
| Rendering Model | Fixed 60 FPS game loop | Unnecessary CPU usage when idle |
| Framework Choice | React via Ink | 11ms layout overhead for ~2400 cells |
| Scene Management | Full scene graph + diffing per frame | O(n) operations when O(1) possible |
| Runtime | Node.js | GC pauses, memory overhead (~200MB+) |
| Input Handling | Frame-coupled | Input lag tied to frame budget |

### 1.2 Actual TUI Requirements

The Claude Code interface requires rendering for exactly three event types:

1. **User Input** — Character echo, cursor movement (~60-120 events/second max during typing)
2. **API Streaming** — Token arrival (~10-50 tokens/second during generation)
3. **Throbber Animation** — Activity indicator (~3-4 Hz)

None of these require 60 FPS. The appropriate model is **event-driven rendering**: redraw only when state changes.

### 1.3 Performance Targets

| Metric | Claude Code (Current) | RCT (Target) |
|--------|----------------------|--------------|
| Frame Time | 16ms budget, 11ms React overhead | <1ms typical |
| Memory (Idle) | ~200-400MB | <50MB |
| Input Latency | Frame-coupled (~16ms) | Direct (<1ms) |
| CPU (Idle) | Constant polling | Zero (epoll/kqueue) |
| Startup Time | ~2-3 seconds | <500ms |

---

## 2. Technical Architecture

### 2.1 Technology Stack

| Component | Choice | Rationale |
|-----------|--------|-----------|
| **Language** | Rust 2024 Edition | Zero-cost abstractions, no GC, memory safety |
| **TUI Framework** | Ratatui | Mature, immediate-mode, widget-rich |
| **Terminal Backend** | Crossterm | Cross-platform, async-friendly, excellent ANSI |
| **Async Runtime** | Tokio | Industry-standard, excellent for I/O-bound work |
| **HTTP Client** | Reqwest | Streaming support, async, battle-tested |
| **Serialization** | Serde + serde_json | Zero-copy where possible |
| **Syntax Highlighting** | Syntect | Sublime Text syntax definitions |
| **CLI Arguments** | Clap | Derive macro, excellent UX |
| **Configuration** | Config + Directories | XDG-compliant, layered config |

### 2.2 High-Level Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                           Event Loop (Tokio)                            │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  ┌──────────────────┐   ┌──────────────────┐   ┌──────────────────┐   │
│  │  Terminal Events │   │   API Streaming   │   │   Timer Events    │   │
│  │   (Crossterm)    │   │    (Reqwest)     │   │    (Throbber)    │   │
│  └────────┬─────────┘   └────────┬─────────┘   └────────┬─────────┘   │
│           │                      │                      │              │
│           └──────────────────────┼──────────────────────┘              │
│                                  ▼                                      │
│                    ┌─────────────────────────┐                         │
│                    │     State Manager       │                         │
│                    │  (dirty flag tracking)  │                         │
│                    └───────────┬─────────────┘                         │
│                                │                                        │
│                    ┌───────────▼─────────────┐                         │
│                    │      Render Layer       │                         │
│                    │       (Ratatui)         │                         │
│                    └───────────┬─────────────┘                         │
│                                │                                        │
│                    ┌───────────▼─────────────┐                         │
│                    │    Terminal Output      │                         │
│                    │      (Crossterm)        │                         │
│                    └─────────────────────────┘                         │
└─────────────────────────────────────────────────────────────────────────┘
```

### 2.3 Core Design Principles

#### 2.3.1 Event-Driven Rendering

```rust
// Pseudo-code: Event-driven vs Game Loop
// ❌ Current (Game Loop)
loop {
    let start = Instant::now();
    process_input();
    update_state();
    render();  // Always renders, even if nothing changed
    sleep_until(start + Duration::from_millis(16));
}

// ✅ Proposed (Event-Driven)
loop {
    select! {
        event = terminal_events.recv() => {
            state.handle_terminal_event(event);
            state.mark_dirty();
        }
        chunk = api_stream.recv() => {
            state.append_content(chunk);
            state.mark_dirty();
        }
        _ = throbber_tick.tick() => {
            if state.is_loading {
                state.advance_throbber();
                state.mark_dirty();
            }
        }
    }
    
    if state.is_dirty() {
        render(&state);
        state.clear_dirty();
    }
}
```

#### 2.3.2 Layered Rendering Model

Instead of a scene graph with full-frame diffing, use a simple layered model:

```
Layer 3: Overlay (throbber, status, popups)
Layer 2: Input Area (prompt, cursor)
Layer 1: Scrollback (message history)
Layer 0: Background (optional styling)
```

Each layer maintains its own dirty state. Partial redraws target specific regions.

#### 2.3.3 Zero-Copy Where Possible

```rust
// Avoid allocations in hot paths
struct StreamingMessage<'a> {
    role: Role,
    content: Cow<'a, str>,  // Borrowed from API response
    tool_calls: SmallVec<[ToolCall<'a>; 4]>,
}
```

---

## 3. Module Specification

### 3.1 Core Modules

```
rct/
├── src/
│   ├── main.rs                 # Entry point, CLI parsing
│   ├── lib.rs                  # Library root
│   ├── app/
│   │   ├── mod.rs
│   │   ├── state.rs            # Application state
│   │   ├── event_loop.rs       # Main event loop
│   │   └── config.rs           # Configuration management
│   ├── api/
│   │   ├── mod.rs
│   │   ├── client.rs           # Anthropic API client
│   │   ├── streaming.rs        # SSE streaming handler
│   │   ├── messages.rs         # Message types
│   │   └── tools.rs            # Tool use definitions
│   ├── tui/
│   │   ├── mod.rs
│   │   ├── renderer.rs         # Main render coordinator
│   │   ├── widgets/
│   │   │   ├── message_list.rs # Scrollable message history
│   │   │   ├── input.rs        # Multi-line input editor
│   │   │   ├── throbber.rs     # Activity indicator
│   │   │   ├── code_block.rs   # Syntax-highlighted code
│   │   │   ├── diff_view.rs    # File diff display
│   │   │   └── markdown.rs     # Markdown rendering
│   │   └── theme.rs            # Color schemes
│   ├── tools/
│   │   ├── mod.rs
│   │   ├── executor.rs         # Tool execution coordinator
│   │   ├── bash.rs             # Bash command execution
│   │   ├── file_ops.rs         # File read/write/edit
│   │   └── sandbox.rs          # Security sandbox
│   ├── plugins/
│   │   ├── mod.rs
│   │   ├── loader.rs           # Plugin discovery & loading
│   │   ├── manifest.rs         # plugin.json parsing
│   │   ├── commands.rs         # Slash command system
│   │   ├── marketplace.rs      # Plugin marketplace client
│   │   └── registry.rs         # Installed plugin registry
│   ├── skills/
│   │   ├── mod.rs
│   │   ├── engine.rs           # Skill matching & activation
│   │   ├── parser.rs           # SKILL.md parsing
│   │   └── context.rs          # Context injection
│   ├── hooks/
│   │   ├── mod.rs
│   │   ├── executor.rs         # Hook execution engine
│   │   ├── events.rs           # Lifecycle event definitions
│   │   ├── matcher.rs          # Tool pattern matching
│   │   └── protocol.rs         # JSON I/O protocol
│   ├── mcp/
│   │   ├── mod.rs
│   │   ├── manager.rs          # MCP server lifecycle
│   │   ├── transport.rs        # stdio/SSE transports
│   │   ├── protocol.rs         # JSON-RPC implementation
│   │   └── discovery.rs        # Tool/resource discovery
│   ├── agents/
│   │   ├── mod.rs
│   │   ├── subagent.rs         # Subagent execution
│   │   ├── orchestrator.rs     # Multi-agent coordination
│   │   └── isolation.rs        # Context isolation
│   ├── update/
│   │   ├── mod.rs
│   │   ├── checker.rs          # Version check
│   │   ├── installer.rs        # Binary replacement
│   │   ├── channels.rs         # Release channel logic
│   │   └── signature.rs        # Signature verification
│   ├── ide/
│   │   ├── mod.rs
│   │   ├── server.rs           # TCP/IPC server
│   │   ├── protocol.rs         # IDE message protocol
│   │   └── vscode.rs           # VS Code specifics
│   └── util/
│       ├── mod.rs
│       ├── ansi.rs             # ANSI escape sequences
│       ├── text.rs             # Text processing utilities
│       └── paths.rs            # XDG path resolution
├── Cargo.toml
├── README.md
└── CLAUDE.md                   # Claude Code integration hints
```

### 3.2 State Management

```rust
pub struct AppState {
    // Core state
    messages: Vec<Message>,
    input_buffer: InputBuffer,
    scroll_offset: usize,
    
    // UI state
    throbber_state: ThrobberState,
    overlay: Option<Overlay>,
    
    // API state
    current_request: Option<RequestId>,
    streaming_content: Option<StreamingContent>,
    
    // Dirty tracking
    dirty_regions: DirtyRegions,
}

#[derive(Default)]
pub struct DirtyRegions {
    messages: bool,
    input: bool,
    overlay: bool,
    full: bool,  // Force full redraw (e.g., resize)
}
```

### 3.3 API Client Design

```rust
pub struct AnthropicClient {
    http: reqwest::Client,
    api_key: SecretString,
    model: String,
    base_url: Url,
}

impl AnthropicClient {
    /// Stream a message completion
    pub fn stream_message(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> impl Stream<Item = Result<StreamEvent, ApiError>> {
        // Returns async stream of SSE events
    }
}

pub enum StreamEvent {
    MessageStart { id: String },
    ContentBlockStart { index: usize, block_type: BlockType },
    ContentBlockDelta { index: usize, delta: ContentDelta },
    ContentBlockStop { index: usize },
    MessageDelta { stop_reason: Option<StopReason> },
    MessageStop,
    Error { error: ApiError },
}
```

---

## 4. Implementation Phases

### Phase 1: Foundation (Weeks 1-3)

**Objective:** Establish project structure, basic event loop, and API connectivity.

| Deliverable | Description | Acceptance Criteria |
|-------------|-------------|---------------------|
| Project scaffold | Cargo workspace, CI/CD | `cargo build` succeeds, tests pass |
| Event loop | Tokio-based select loop | Handles terminal + timer events |
| API client | Streaming message completion | Can stream a response, parse SSE |
| Basic TUI | Single-frame render | Displays static content correctly |

**Key Implementation Details:**

```rust
// Phase 1 event loop skeleton
async fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    api: &AnthropicClient,
) -> Result<()> {
    let mut state = AppState::new();
    let mut events = EventStream::new();
    
    loop {
        // Render if dirty
        if state.dirty_regions.any() {
            terminal.draw(|f| render(f, &state))?;
            state.dirty_regions.clear();
        }
        
        // Wait for next event
        tokio::select! {
            Some(Ok(event)) = events.next() => {
                match state.handle_event(event).await {
                    Action::Continue => {},
                    Action::Quit => break,
                }
            }
        }
    }
    Ok(())
}
```

### Phase 2: Core TUI (Weeks 4-6)

**Objective:** Full TUI implementation with scrolling, input, and streaming display.

| Deliverable | Description | Acceptance Criteria |
|-------------|-------------|---------------------|
| Input editor | Multi-line, history, shortcuts | No input lag, proper cursor handling |
| Message list | Scrollable, markdown rendering | Smooth scroll, correct wrapping |
| Streaming display | Real-time token rendering | No flicker, proper line handling |
| Throbber | Animated activity indicator | 3-4 Hz, no CPU when idle |

**Performance Benchmarks (Phase 2):**

```
Target: 2400-cell terminal (80x30)
- Full redraw: <1ms
- Partial redraw (input only): <100μs
- Streaming token append: <50μs
- Memory (10k message history): <20MB
```

### Phase 3: Tool Execution (Weeks 7-10)

**Objective:** Implement safe tool calling for agentic capabilities.

| Deliverable | Description | Acceptance Criteria |
|-------------|-------------|---------------------|
| Tool protocol | Handle tool_use responses | Parse and route correctly |
| Bash executor | Shell command execution | Proper streaming, exit codes |
| File operations | Read, write, edit files | Atomic writes, diff display |
| Safety sandbox | Confirmation prompts, limits | Blocks dangerous commands |
| Git integration | Common git workflows | Status, diff, commit display |

**Security Considerations:**

```rust
pub struct ToolExecutionPolicy {
    /// Commands that always require confirmation
    dangerous_patterns: Vec<Regex>,
    
    /// Paths that cannot be modified
    protected_paths: Vec<PathBuf>,
    
    /// Maximum file size for operations
    max_file_size: usize,
    
    /// Timeout for command execution
    command_timeout: Duration,
}

impl ToolExecutor {
    pub async fn execute(&self, call: ToolCall) -> Result<ToolResult> {
        // Check policy
        self.policy.validate(&call)?;
        
        // Prompt for confirmation if dangerous
        if self.policy.requires_confirmation(&call) {
            if !self.prompt_confirmation(&call).await? {
                return Ok(ToolResult::Cancelled);
            }
        }
        
        // Execute with timeout and resource limits
        timeout(self.policy.command_timeout, self.run_tool(call)).await?
    }
}
```

### Phase 4: Polish & Parity (Weeks 11-14)

**Objective:** Feature parity with Claude Code, production polish.

| Deliverable | Description | Acceptance Criteria |
|-------------|-------------|---------------------|
| Syntax highlighting | Code block coloring | Fast, accurate, themeable |
| Configuration | TOML config, XDG paths | Layered config, env overrides |
| Themes | Light/dark, custom | Consistent, accessible |
| Error handling | Graceful degradation | No crashes, helpful messages |
| Checkpoints | Undo/redo for file changes | Reliable restore |

### Phase 5: Plugin & Extension System (Weeks 15-20)

**Objective:** Full plugin ecosystem with skills, hooks, MCP, and marketplace support.

#### 5.1 Plugin Architecture

```
~/.config/rct/
├── plugins/                      # Installed plugins
│   └── my-plugin/
│       ├── .claude-plugin/
│       │   └── plugin.json       # Plugin manifest
│       ├── commands/             # Slash commands
│       │   └── hello.md
│       ├── agents/               # Subagent definitions
│       │   └── code-reviewer/
│       │       └── AGENT.md
│       ├── skills/               # Auto-invoked skills
│       │   └── code-review/
│       │       └── SKILL.md
│       ├── hooks/                # Lifecycle hooks
│       │   └── hooks.json
│       └── .mcp.json             # MCP server config
├── marketplaces.json             # Registered marketplaces
└── settings.json                 # Global settings
```

| Deliverable | Description | Acceptance Criteria |
|-------------|-------------|---------------------|
| Plugin loader | Discovery, validation, loading | Hot-reload, conflict detection |
| Command system | `/command` slash commands | Arguments, namespacing |
| Skill engine | Auto-invocation by context | SKILL.md parsing, matching |
| Hook executor | 8 lifecycle events | Exit code handling, JSON I/O |
| MCP client | Model Context Protocol | stdio/SSE transports |
| Marketplace | Plugin discovery & install | `/plugin` command interface |

#### 5.2 Plugin Manifest Schema

```rust
#[derive(Debug, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub homepage: Option<String>,
    
    /// Minimum RCT version required
    pub min_rct_version: Option<String>,
    
    /// Plugin capabilities
    pub capabilities: PluginCapabilities,
}

#[derive(Debug, Deserialize, Default)]
pub struct PluginCapabilities {
    pub commands: bool,
    pub agents: bool,
    pub skills: bool,
    pub hooks: bool,
    pub mcp: bool,
}
```

#### 5.3 Skill System

Skills are context-aware capabilities that Claude automatically invokes based on task matching.

```rust
#[derive(Debug, Deserialize)]
pub struct SkillDefinition {
    /// Unique skill identifier
    pub name: String,
    
    /// Description used for context matching
    /// Should start with "This skill should be used when..."
    pub description: String,
    
    /// Full instructions loaded when skill activates
    pub instructions: String,
    
    /// Optional reference files
    pub references: Vec<PathBuf>,
}

pub struct SkillEngine {
    skills: Vec<SkillDefinition>,
    embeddings: Option<EmbeddingIndex>,  // For semantic matching
}

impl SkillEngine {
    /// Find skills relevant to the current task
    pub fn match_skills(&self, task_description: &str) -> Vec<&SkillDefinition> {
        // 1. Keyword matching on description
        // 2. Optional: semantic similarity via embeddings
        // 3. Return top N matches
    }
    
    /// Load skill instructions into context
    pub fn activate_skill(&self, skill: &SkillDefinition) -> String {
        format!(
            "# Skill: {}\n\n{}\n",
            skill.name,
            skill.instructions
        )
    }
}
```

**SKILL.md Format:**

```markdown
---
name: code-review
description: This skill should be used when the user asks to review code, analyze code quality, or check for best practices.
---

## Code Review Guidelines

When reviewing code, follow these steps:

1. Check for security vulnerabilities
2. Verify error handling
3. Assess performance implications
4. Review naming conventions
5. Check test coverage

[Full instructions...]
```

#### 5.4 Hook System

Hooks provide deterministic control at 8 lifecycle events:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    /// Before tool execution (can block)
    PreToolUse,
    /// After successful tool execution
    PostToolUse,
    /// After failed tool execution
    PostToolUseFailure,
    /// When permission dialog would show (can auto-allow/deny)
    PermissionRequest,
    /// When user submits a prompt
    UserPromptSubmit,
    /// When session starts
    SessionStart,
    /// When session ends
    SessionEnd,
    /// When Claude sends a notification
    Notification,
    /// When Claude finishes responding
    Stop,
    /// When a subagent stops
    SubagentStop,
    /// Before context compaction
    PreCompact,
}

#[derive(Debug, Deserialize)]
pub struct HookConfig {
    /// Tool name matcher (glob patterns, e.g., "Bash", "Edit|Write", "*")
    pub matcher: Option<String>,
    pub hooks: Vec<HookDefinition>,
}

#[derive(Debug, Deserialize)]
pub struct HookDefinition {
    #[serde(rename = "type")]
    pub hook_type: HookType,
    pub command: String,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub enum HookType {
    #[serde(rename = "command")]
    Command,
}

/// Hook execution result
pub struct HookResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub decision: Option<HookDecision>,
}

/// Exit code 2 = block/deny with feedback
pub enum HookDecision {
    Continue,
    Block { reason: String },
    Allow,   // For PermissionRequest
    Deny,    // For PermissionRequest
}
```

**Hook Executor:**

```rust
pub struct HookExecutor {
    hooks: HashMap<HookEvent, Vec<HookConfig>>,
}

impl HookExecutor {
    /// Execute hooks for an event, passing context as JSON to stdin
    pub async fn execute(
        &self,
        event: HookEvent,
        context: &HookContext,
    ) -> Result<HookResult> {
        let configs = self.hooks.get(&event).unwrap_or(&vec![]);
        
        for config in configs {
            // Check matcher for tool-based hooks
            if let Some(matcher) = &config.matcher {
                if !self.matches_tool(matcher, &context.tool_name) {
                    continue;
                }
            }
            
            for hook in &config.hooks {
                let input_json = serde_json::to_string(context)?;
                
                let output = tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg(&hook.command)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()?
                    .wait_with_output()
                    .await?;
                
                // Exit code 2 = block with stderr as reason
                if output.status.code() == Some(2) {
                    return Ok(HookResult {
                        exit_code: 2,
                        stdout: String::from_utf8_lossy(&output.stdout).into(),
                        stderr: String::from_utf8_lossy(&output.stderr).into(),
                        decision: Some(HookDecision::Block {
                            reason: String::from_utf8_lossy(&output.stderr).into(),
                        }),
                    });
                }
            }
        }
        
        Ok(HookResult::default())
    }
}
```

**hooks.json Format:**

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "python3 validate_command.py"
          }
        ]
      }
    ],
    "PostToolUse": [
      {
        "matcher": "Edit|Write",
        "hooks": [
          {
            "type": "command",
            "command": "prettier --write \"$(jq -r '.tool_input.file_path')\""
          }
        ]
      }
    ],
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "cat ~/.claude/context.md"
          }
        ]
      }
    ]
  }
}
```

#### 5.5 MCP (Model Context Protocol) Integration

```rust
pub struct McpManager {
    servers: HashMap<String, McpServer>,
}

#[derive(Debug)]
pub struct McpServer {
    pub name: String,
    pub transport: McpTransport,
    pub tools: Vec<McpTool>,
    pub resources: Vec<McpResource>,
    pub status: McpStatus,
}

#[derive(Debug)]
pub enum McpTransport {
    Stdio {
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
    },
    Sse {
        url: String,
        headers: HashMap<String, String>,
    },
}

impl McpManager {
    /// Start an MCP server
    pub async fn start_server(&mut self, config: &McpServerConfig) -> Result<()> {
        match &config.transport {
            McpTransport::Stdio { command, args, env } => {
                let child = tokio::process::Command::new(command)
                    .args(args)
                    .envs(env)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .spawn()?;
                
                // Initialize JSON-RPC communication
                self.initialize_stdio_server(config.name.clone(), child).await?;
            }
            McpTransport::Sse { url, headers } => {
                self.connect_sse_server(config.name.clone(), url, headers).await?;
            }
        }
        Ok(())
    }
    
    /// Call an MCP tool
    pub async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let server = self.servers.get(server)
            .ok_or_else(|| anyhow!("MCP server not found: {}", server))?;
        
        // Send JSON-RPC request
        let request = json!({
            "jsonrpc": "2.0",
            "id": uuid::Uuid::new_v4().to_string(),
            "method": "tools/call",
            "params": {
                "name": tool,
                "arguments": arguments
            }
        });
        
        server.send_request(request).await
    }
}
```

**.mcp.json Format:**

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@anthropic-ai/mcp-server-filesystem"],
      "env": {
        "ALLOWED_PATHS": "/home/user/projects"
      }
    },
    "github": {
      "command": "uvx",
      "args": ["mcp-server-github"],
      "env": {
        "GITHUB_TOKEN": "${GITHUB_TOKEN}"
      }
    },
    "remote-api": {
      "url": "https://api.example.com/mcp/sse",
      "transport": "sse"
    }
  }
}
```

#### 5.6 Subagent System

```rust
pub struct SubagentManager {
    agents: HashMap<String, SubagentDefinition>,
    running: HashMap<Uuid, RunningSubagent>,
}

#[derive(Debug)]
pub struct SubagentDefinition {
    pub name: String,
    pub description: String,
    pub system_prompt: String,
    pub tools: Vec<String>,  // Allowed tools
    pub max_tokens: Option<u32>,
}

#[derive(Debug)]
pub struct RunningSubagent {
    pub id: Uuid,
    pub definition: SubagentDefinition,
    pub messages: Vec<Message>,
    pub status: SubagentStatus,
}

impl SubagentManager {
    /// Spawn a subagent for parallel task execution
    pub async fn spawn(
        &mut self,
        agent_name: &str,
        task: &str,
        parent_context: &Context,
    ) -> Result<Uuid> {
        let definition = self.agents.get(agent_name)
            .ok_or_else(|| anyhow!("Agent not found: {}", agent_name))?;
        
        let id = Uuid::new_v4();
        let subagent = RunningSubagent {
            id,
            definition: definition.clone(),
            messages: vec![Message {
                role: Role::User,
                content: task.to_string(),
            }],
            status: SubagentStatus::Running,
        };
        
        self.running.insert(id, subagent);
        
        // Execute in background task
        let agent = self.running.get(&id).unwrap().clone();
        tokio::spawn(async move {
            Self::run_agent_loop(agent).await
        });
        
        Ok(id)
    }
}
```

### Phase 6: Live Updates & Distribution (Weeks 21-24)

**Objective:** Self-updating binary with release channels and cross-platform distribution.

#### 6.1 Auto-Update System

```rust
pub struct UpdateManager {
    current_version: Version,
    channel: ReleaseChannel,
    manifest_url: String,
    install_path: PathBuf,
}

#[derive(Debug, Clone, Copy)]
pub enum ReleaseChannel {
    Latest,  // Immediate releases
    Stable,  // ~1 week delay, skip regressions
}

#[derive(Debug, Deserialize)]
pub struct ReleaseManifest {
    pub version: String,
    pub channel: String,
    pub platforms: HashMap<String, PlatformRelease>,
    pub release_notes: String,
    pub min_version: Option<String>,  // Force update below this
}

#[derive(Debug, Deserialize)]
pub struct PlatformRelease {
    pub url: String,
    pub sha256: String,
    pub size: u64,
    pub signature: Option<String>,
}

impl UpdateManager {
    /// Check for updates (called on startup and periodically)
    pub async fn check_for_updates(&self) -> Result<Option<ReleaseManifest>> {
        let manifest: ReleaseManifest = reqwest::get(&self.manifest_url)
            .await?
            .json()
            .await?;
        
        let remote_version = Version::parse(&manifest.version)?;
        
        if remote_version > self.current_version {
            Ok(Some(manifest))
        } else {
            Ok(None)
        }
    }
    
    /// Download and install update in background
    pub async fn install_update(&self, manifest: &ReleaseManifest) -> Result<()> {
        let platform = self.current_platform();
        let release = manifest.platforms.get(&platform)
            .ok_or_else(|| anyhow!("No release for platform: {}", platform))?;
        
        // Download to temp file
        let temp_path = self.install_path.with_extension("new");
        let response = reqwest::get(&release.url).await?;
        let bytes = response.bytes().await?;
        
        // Verify checksum
        let checksum = sha256::digest(&bytes);
        if checksum != release.sha256 {
            return Err(anyhow!("Checksum mismatch"));
        }
        
        // Verify signature if present
        if let Some(sig) = &release.signature {
            self.verify_signature(&bytes, sig)?;
        }
        
        // Atomic replace
        tokio::fs::write(&temp_path, &bytes).await?;
        
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&temp_path, std::fs::Permissions::from_mode(0o755)).await?;
        }
        
        tokio::fs::rename(&temp_path, &self.install_path).await?;
        
        Ok(())
    }
}
```

#### 6.2 Distribution Strategy

| Platform | Primary Method | Package Manager | Auto-Update |
|----------|---------------|-----------------|-------------|
| macOS | Signed binary | Homebrew cask | ✓ |
| Linux | Native binary | apt, dnf, pacman | ✓ |
| Windows | Signed .exe | WinGet, Scoop | ✓ |

**Installation Script (install.sh):**

```bash
#!/bin/bash
set -euo pipefail

INSTALL_DIR="${HOME}/.local/bin"
MANIFEST_URL="https://releases.rct.dev/manifest.json"

# Detect platform
case "$(uname -s)-$(uname -m)" in
    Linux-x86_64)  PLATFORM="linux-x86_64" ;;
    Linux-aarch64) PLATFORM="linux-aarch64" ;;
    Darwin-x86_64) PLATFORM="darwin-x86_64" ;;
    Darwin-arm64)  PLATFORM="darwin-aarch64" ;;
    *) echo "Unsupported platform"; exit 1 ;;
esac

# Fetch manifest
MANIFEST=$(curl -fsSL "$MANIFEST_URL")
VERSION=$(echo "$MANIFEST" | jq -r '.version')
URL=$(echo "$MANIFEST" | jq -r ".platforms[\"$PLATFORM\"].url")
SHA256=$(echo "$MANIFEST" | jq -r ".platforms[\"$PLATFORM\"].sha256")

# Download
echo "Installing RCT v${VERSION}..."
curl -fsSL "$URL" -o /tmp/rct

# Verify
echo "$SHA256  /tmp/rct" | sha256sum -c -

# Install
mkdir -p "$INSTALL_DIR"
mv /tmp/rct "$INSTALL_DIR/rct"
chmod +x "$INSTALL_DIR/rct"

echo "Installed to $INSTALL_DIR/rct"
echo "Add to PATH: export PATH=\"\$PATH:$INSTALL_DIR\""
```

### Phase 7: IDE Integration (Weeks 25-28)

**Objective:** VS Code and JetBrains extension compatibility.

```rust
pub struct IdeServer {
    listener: TcpListener,
    sessions: HashMap<Uuid, IdeSession>,
}

/// Protocol for IDE ↔ RCT communication
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum IdeMessage {
    // From IDE
    #[serde(rename = "init")]
    Init { workspace: PathBuf, capabilities: Vec<String> },
    
    #[serde(rename = "prompt")]
    Prompt { text: String, selection: Option<Selection> },
    
    #[serde(rename = "apply_edit")]
    ApplyEdit { file: PathBuf, diff: String },
    
    // From RCT
    #[serde(rename = "streaming_content")]
    StreamingContent { delta: String },
    
    #[serde(rename = "edit_proposal")]
    EditProposal { file: PathBuf, diff: String, description: String },
    
    #[serde(rename = "tool_use")]
    ToolUse { tool: String, input: serde_json::Value },
}
```

---

## 5. Feature Parity Matrix

**Total Timeline:** 7-8 months for full parity (solo/small team), faster with contributors.

| Feature | Claude Code | RCT | Phase |
|---------|-------------|-----|-------|
| **Core Chat** ||||
| Streaming responses | ✓ | ✓ | 1 |
| Multi-turn conversation | ✓ | ✓ | 1 |
| Message history/scrollback | ✓ | ✓ | 2 |
| Session persistence | ✓ | ✓ | 4 |
| Markdown rendering | ✓ | ✓ | 2 |
| Syntax highlighting | ✓ | ✓ | 4 |
| **Tool Use** ||||
| Bash execution | ✓ | ✓ | 3 |
| File read/write/edit | ✓ | ✓ | 3 |
| Git integration | ✓ | ✓ | 3 |
| Checkpoints/undo | ✓ | ✓ | 4 |
| Web fetch | ✓ | ✓ | 3 |
| Grep/search | ✓ | ✓ | 3 |
| **Extension System** ||||
| Slash commands (`/cmd`) | ✓ | ✓ | 5 |
| Skills (SKILL.md) | ✓ | ✓ | 5 |
| Agents (AGENT.md) | ✓ | ✓ | 5 |
| Subagents (parallel execution) | ✓ | ✓ | 5 |
| MCP servers (stdio) | ✓ | ✓ | 5 |
| MCP servers (SSE) | ✓ | ✓ | 5 |
| Plugins | ✓ | ✓ | 5 |
| Plugin marketplaces | ✓ | ✓ | 5 |
| **Hooks (Lifecycle Events)** ||||
| PreToolUse | ✓ | ✓ | 5 |
| PostToolUse | ✓ | ✓ | 5 |
| PostToolUseFailure | ✓ | ✓ | 5 |
| PermissionRequest | ✓ | ✓ | 5 |
| UserPromptSubmit | ✓ | ✓ | 5 |
| SessionStart | ✓ | ✓ | 5 |
| SessionEnd | ✓ | ✓ | 5 |
| Notification | ✓ | ✓ | 5 |
| Stop | ✓ | ✓ | 5 |
| SubagentStop | ✓ | ✓ | 5 |
| PreCompact | ✓ | ✓ | 5 |
| **Platform Support** ||||
| macOS (Intel) | ✓ | ✓ | 1 |
| macOS (Apple Silicon) | ✓ | ✓ | 1 |
| Linux (x86_64) | ✓ | ✓ | 1 |
| Linux (ARM64) | ✓ | ✓ | 4 |
| Windows (WSL) | ✓ | ✓ | 4 |
| Windows (native) | ✓ | ✓ | 6 |
| **Updates & Distribution** ||||
| Auto-update (background) | ✓ | ✓ | 6 |
| Release channels (latest/stable) | ✓ | ✓ | 6 |
| Signed binaries | ✓ | ✓ | 6 |
| Homebrew | ✓ | ✓ | 6 |
| WinGet | ✓ | ✓ | 6 |
| Native installer | ✓ | ✓ | 6 |
| **IDE Integration** ||||
| VS Code extension | ✓ | ✓ | 7 |
| JetBrains plugin | ✓ | ✓ | 7 |
| Interactive diffs | ✓ | ✓ | 7 |
| **Configuration** ||||
| settings.json | ✓ | ✓ | 2 |
| settings.local.json | ✓ | ✓ | 2 |
| CLAUDE.md (project) | ✓ | ✓ | 2 |
| Enterprise managed settings | ✓ | ✓ | 6 |
| `/config` command | ✓ | ✓ | 2 |
| `/doctor` diagnostics | ✓ | ✓ | 4 |
| **Cloud Providers** ||||
| Anthropic API (direct) | ✓ | ✓ | 1 |
| Amazon Bedrock | ✓ | ✓ | 4 |
| Google Vertex AI | ✓ | ✓ | 4 |

---

## 6. Development Methodology

### 5.1 Quality Standards

- **Test Coverage:** Minimum 80% line coverage for core modules
- **Documentation:** All public APIs documented with examples
- **Benchmarks:** Criterion benchmarks for rendering paths
- **Fuzzing:** cargo-fuzz for input handling and API parsing
- **Linting:** `clippy::pedantic`, custom lint rules

### 5.2 CI/CD Pipeline

```yaml
# .github/workflows/ci.yml
jobs:
  test:
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    steps:
      - cargo test --all-features
      - cargo clippy -- -D warnings
      - cargo fmt -- --check
      
  bench:
    runs-on: ubuntu-latest
    steps:
      - cargo bench --no-run  # Compile check
      
  release:
    needs: [test]
    if: startsWith(github.ref, 'refs/tags/')
    steps:
      - cargo build --release
      - # Upload artifacts
```

### 5.3 Performance Validation

Each PR must include:
1. No regression in existing benchmarks
2. New benchmarks for new rendering paths
3. Manual testing on legacy terminals (xterm, Terminal.app)

```rust
// Example benchmark
#[bench]
fn bench_full_redraw(b: &mut Bencher) {
    let mut buffer = Buffer::empty(Rect::new(0, 0, 80, 30));
    let state = create_test_state_with_messages(100);
    
    b.iter(|| {
        render_message_list(&mut buffer, &state.messages);
    });
}
```

---

## 6. Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| API changes | Medium | High | Abstract API layer, version pinning |
| Terminal compatibility | Medium | Medium | Extensive terminal matrix testing |
| Performance regression | Low | High | Automated benchmark CI gates |
| Security vulnerabilities | Low | Critical | Audit tool execution paths, sandboxing |
| Scope creep | High | Medium | Strict phase gates, MVP focus |

---

## 7. Success Metrics

### 7.1 Performance (Quantitative)

| Metric | Target | Measurement |
|--------|--------|-------------|
| Typical frame time | <1ms | Criterion benchmarks |
| Input latency | <5ms p99 | User-facing measurement |
| Memory usage (idle) | <50MB | `heaptrack` profiling |
| Memory usage (10k history) | <100MB | Load testing |
| Startup time | <500ms | Wall-clock measurement |
| Binary size (release) | <20MB | `cargo build --release` |

### 7.2 Quality (Qualitative)

- No user-visible flickering during normal operation
- Responsive during API streaming (no input blocking)
- Graceful handling of terminal resize
- Cross-platform consistency (Linux, macOS, Windows)
- Accessible color schemes (WCAG AA contrast)

---

## 8. Resource Requirements

### 8.1 Development Environment

- Rust 1.75+ (2024 edition)
- Linux/macOS for primary development
- Windows VM for compatibility testing
- Multiple terminal emulators (Ghostty, iTerm2, Windows Terminal, xterm)

### 8.2 External Dependencies (Audited)

| Crate | Version | Audit Status |
|-------|---------|--------------|
| tokio | 1.x | RustSec audited |
| ratatui | 0.28+ | Community reviewed |
| crossterm | 0.28+ | Community reviewed |
| reqwest | 0.12+ | RustSec audited |
| serde | 1.x | RustSec audited |
| syntect | 5.x | Community reviewed |

---

## 9. Appendices

### A. Reference Implementation: Event Loop

```rust
use crossterm::event::{Event, EventStream, KeyCode, KeyModifiers};
use futures::{FutureExt, StreamExt};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::time::Duration;
use tokio::time::interval;

pub async fn run(terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>) -> anyhow::Result<()> {
    let mut state = AppState::new();
    let mut events = EventStream::new();
    let mut throbber_interval = interval(Duration::from_millis(250));
    
    loop {
        // Render if needed
        if state.needs_render() {
            terminal.draw(|frame| ui::render(frame, &state))?;
            state.mark_rendered();
        }
        
        // Select on all event sources
        tokio::select! {
            biased;  // Prioritize user input
            
            // Terminal events (input, resize)
            Some(Ok(event)) = events.next() => {
                match event {
                    Event::Key(key) => {
                        match (key.code, key.modifiers) {
                            (KeyCode::Char('c'), KeyModifiers::CONTROL) => break,
                            (KeyCode::Enter, KeyModifiers::NONE) => {
                                if let Some(input) = state.take_input() {
                                    state.start_request(&input);
                                }
                            }
                            (KeyCode::Char(c), _) => state.insert_char(c),
                            _ => {}
                        }
                    }
                    Event::Resize(w, h) => state.handle_resize(w, h),
                    _ => {}
                }
            }
            
            // API streaming chunks
            Some(chunk) = state.recv_api_chunk() => {
                state.append_chunk(chunk);
            }
            
            // Throbber tick (only if loading)
            _ = throbber_interval.tick(), if state.is_loading() => {
                state.tick_throbber();
            }
        }
    }
    
    Ok(())
}
```

### B. Benchmark Comparison Data

Expected performance comparison based on architectural analysis:

```
Operation                  | Claude Code (React) | RCT (Rust)    | Improvement
---------------------------|---------------------|---------------|------------
Full frame render          | ~16ms               | <1ms          | 16x+
Scene graph construction   | ~11ms               | N/A (none)    | ∞
Input echo latency         | ~16ms               | <1ms          | 16x+
Memory (idle)              | ~200-400MB          | <50MB         | 4-8x
Memory (large session)     | ~1GB+               | <100MB        | 10x+
CPU (idle waiting)         | ~2-5%               | ~0%           | ∞
Startup time               | ~2-3s               | <500ms        | 4-6x
```

### C. Terminal Compatibility Matrix

| Terminal | Platform | Tested | Notes |
|----------|----------|--------|-------|
| Ghostty | macOS, Linux | Target | GPU-accelerated, modern |
| iTerm2 | macOS | Required | Most common macOS terminal |
| Kitty | Linux, macOS | Required | GPU-accelerated |
| Alacritty | Cross-platform | Required | Widely used |
| Windows Terminal | Windows | Required | Modern Windows default |
| xterm | Unix | Required | Legacy baseline |
| Terminal.app | macOS | Required | macOS default |
| GNOME Terminal | Linux | Required | Common Linux default |

---

## 10. Conclusion

RCT addresses a clear technical failure in Claude Code's architecture: using a web UI framework (React) for a terminal application that needs only event-driven text rendering. By applying first-principles engineering with Rust and appropriate TUI libraries, we can deliver:

1. **16x+ performance improvement** in rendering
2. **4-8x memory reduction** through zero-GC architecture
3. **Near-instant input responsiveness** via event-driven model
4. **Robust security** through Rust's memory safety guarantees
5. **Full feature parity** including plugins, skills, hooks, MCP, and subagents
6. **Open-source foundation** for community contribution

The project achieves complete feature parity with Claude Code while dramatically improving performance. The architecture supports the full ecosystem:

- **Plugins** for shareable extensions across teams
- **Skills** for context-aware automatic behaviors
- **Hooks** for deterministic lifecycle control (all 8+ events)
- **MCP** for Model Context Protocol integration
- **Subagents** for parallel task execution
- **Live updates** with release channels and signed binaries

This demonstrates that the "best tool for the job" is not always the most comfortable one — and that terminal UIs don't need game-engine complexity. A Rust implementation with event-driven rendering delivers superior performance while maintaining 100% feature compatibility.

---

*Document prepared for implementation planning. Technical specifications subject to refinement during development.*
