# RCT TDD Production Plan

## Strategic Roadmap: Feature Parity + Modular Plugin Ecosystem

**Document Version:** 2.0
**Date:** January 28, 2026
**Target:** Anthropic Acquisition / Commercial Viability

---

## Executive Summary

This document outlines a Test-Driven Development (TDD) production plan for RCT (Rust Claude Terminal), with a **core-first, plugin-later** architecture:

### Priority 1: Core Product (Must Ship First)
- **Feature Parity** with Claude Code - every feature, no regressions
- **Performance Superiority** - 16x faster, 4-8x less memory
- **Stability** - comprehensive tests, no glitches
- **Plugin Host API** - stable interface for extensions

### Priority 2: First-Party Plugins (Ship Independently)
- **narsil-mcp** - Code intelligence (90 tools)
- **ralph** - Automation suite
- **worktree** - Git branch isolation
- **analytics** - Usage tracking and insights

### Architectural Principle
> "Advanced features are modular plugins that don't disrupt delivery of the main product."

**Value Proposition for Anthropic:**
- Native-speed CLI with no GC pauses or React overhead
- Extensible plugin ecosystem (like VS Code)
- Enterprise-grade first-party plugins available day one
- Clean separation enables independent release cycles

---

## Core vs Plugin Architecture

### The Fundamental Split

| Layer | Ships With | Release Cycle | Breaking Changes |
|-------|-----------|---------------|------------------|
| **RCT Core** | Binary | Major versions | Rare, versioned |
| **Plugin Host API** | Binary | Core releases | Stable, backward compat |
| **First-Party Plugins** | Separate packages | Independent | Plugin-specific |
| **Third-Party Plugins** | Community | Anytime | Community managed |

### What Goes in Core (Non-Negotiable)

These features MUST ship with the core binary for Claude Code parity:

```
RCT Core Binary (~20MB)
├── Terminal UI (ratatui)
├── Event Loop (tokio select!)
├── API Client (streaming)
├── Built-in Tools
│   ├── Bash
│   ├── Read / Write / Edit
│   ├── Glob / Grep
│   ├── WebFetch / WebSearch
│   ├── Git operations
│   └── NotebookEdit
├── MCP Client (protocol only)
├── Hooks System (all 11 events)
├── Skills Engine (matching + activation)
├── Slash Commands
├── Subagent Orchestration
├── Plugin Host API
└── Auto-Update
```

### What Becomes Plugins (Modular)

These ship as separate, optional packages:

```
First-Party Plugins (separate repos/packages)
├── rct-plugin-narsil        # Code intelligence (90 tools)
├── rct-plugin-ralph         # Automation suite
├── rct-plugin-worktree      # Git worktree management
├── rct-plugin-analytics     # Usage tracking
├── rct-plugin-semantic      # Neural code search
└── rct-plugin-enterprise    # SSO, audit, compliance
```

### Plugin Host API Design

```rust
// src/plugins/api.rs - STABLE INTERFACE

/// Plugin capability trait - implement to add features
pub trait RctPlugin: Send + Sync {
    /// Plugin metadata
    fn manifest(&self) -> PluginManifest;

    /// Called when plugin loads
    fn on_load(&mut self, ctx: &PluginContext) -> Result<()>;

    /// Called when plugin unloads
    fn on_unload(&mut self) -> Result<()>;
}

/// Extend with tools
pub trait ToolProvider: RctPlugin {
    fn tools(&self) -> Vec<ToolDefinition>;
    fn execute(&self, tool: &str, input: Value) -> Result<Value>;
}

/// Extend with commands
pub trait CommandProvider: RctPlugin {
    fn commands(&self) -> Vec<SlashCommand>;
    fn execute(&self, cmd: &str, args: &str) -> Result<String>;
}

/// Extend with hooks
pub trait HookProvider: RctPlugin {
    fn hooks(&self) -> Vec<(HookEvent, HookConfig)>;
}

/// Extend with skills
pub trait SkillProvider: RctPlugin {
    fn skills(&self) -> Vec<Skill>;
}

/// Plugin context - what plugins can access
pub struct PluginContext {
    pub working_dir: PathBuf,
    pub api_client: Arc<AnthropicClient>,  // Read-only
    pub tool_executor: Arc<ToolExecutor>,   // For delegation
    pub event_bus: EventSender,             // For notifications
}
```

### Plugin Loading Strategy

```rust
// Plugins loaded from:
// 1. ~/.config/rct/plugins/         (user-installed)
// 2. /usr/share/rct/plugins/        (system-wide)
// 3. .rct/plugins/                  (project-local)

impl PluginRegistry {
    pub fn discover(&mut self) -> Result<Vec<PluginManifest>> {
        // Scan plugin directories
        // Validate manifests
        // Check version compatibility
        // Return available plugins
    }

    pub fn load(&mut self, name: &str) -> Result<()> {
        // Dynamic loading via libloading (Rust dylibs)
        // Or via MCP spawn (external processes)
    }
}
```

### Why This Architecture?

1. **Independent Release Cycles**
   - Core can ship without waiting for plugin features
   - Plugins can iterate faster without core releases
   - Breaking changes in plugins don't affect core

2. **Reduced Core Complexity**
   - Core stays focused on Claude Code parity
   - Fewer dependencies in core binary
   - Easier to audit and maintain

3. **User Choice**
   - Power users install plugins they need
   - Minimal installs stay minimal
   - Enterprise can mandate specific plugins

4. **Acquisition Friendly**
   - Core is clean, well-tested, focused
   - Plugins demonstrate ecosystem potential
   - Multiple value centers, not one monolith

---

## Current State Analysis

### Working Features (Phase 1-2 Complete)
| Component | Status | Test Coverage |
|-----------|--------|---------------|
| TUI + Event Loop | Working | 0% |
| API Streaming | Working | 0% |
| Message History | Working | 0% |
| Basic Input | Working | 0% |
| Throbber Animation | Working | 0% |

### Scaffolded Features (Need Implementation)

**CORE (Ships in binary - blocks release):**
| Component | Status | Priority | Notes |
|-----------|--------|----------|-------|
| Tool Execution | Scaffolded | P0 | All Claude Code tools |
| MCP Client | Scaffolded | P0 | Protocol only, not narsil |
| Hooks System | Scaffolded | P0 | All 11 events |
| Skills Engine | Scaffolded | P0 | Matching + activation |
| Slash Commands | Scaffolded | P0 | /help, /config, etc. |
| Subagents | Scaffolded | P0 | Parallel execution |
| Plugin Host API | Not started | P0 | Stable interface |
| Auto-Update | Scaffolded | P1 | Self-update mechanism |

**PLUGINS (Ship separately - don't block release):**
| Component | Status | Priority | Notes |
|-----------|--------|----------|-------|
| narsil integration | Scaffolded | P1-Plugin | First-party, code intel |
| ralph integration | Not started | P2-Plugin | First-party, automation |
| Git worktree | Not started | P2-Plugin | Branch isolation |
| IDE Integration | Scaffolded | P2-Plugin | VS Code, JetBrains |
| Analytics | Not started | P3-Plugin | Usage tracking |
| Enterprise features | Not started | P3-Plugin | SSO, audit, compliance |

### Architectural Issues to Address
1. **Circular Dependency:** `app` <-> `api` module coupling
2. **Missing Tests:** 0% test coverage currently
3. **Stub Implementations:** MCP, agents, plugins return empty/placeholder data
4. **Security Hardening:** Tool executor needs sandboxing

---

## TDD Methodology: Integration-First Testing

### The Problem with Traditional TDD

Traditional TDD builds confidence bottom-up:
```
Unit Tests (many) → Integration Tests (some) → E2E Tests (few)
```

This creates a failure mode where **optimized subcomponents don't roll up into a coherent system**. For a terminal application that must work across every weird niche context ever, this is unacceptable.

### Our Approach: Inverted Testing Pyramid

```
┌─────────────────────────────────────────────────────────────────────┐
│                    RCT TESTING PYRAMID (INVERTED)                   │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│                    ┌─────────────────────┐                         │
│                    │    E2E Tests        │  ← PRIMARY (most)       │
│                    │  (Real terminals,   │    Lock in behavior     │
│                    │   full workflows)   │    across ALL contexts  │
│                    └──────────┬──────────┘                         │
│                               │                                     │
│               ┌───────────────┴───────────────┐                    │
│               │    Integration Tests          │  ← HIGH PRIORITY   │
│               │  (Module boundaries,          │    Verify modules  │
│               │   subsystem interaction)      │    compose correctly│
│               └───────────────┬───────────────┘                    │
│                               │                                     │
│       ┌───────────────────────┴───────────────────────┐            │
│       │              Unit Tests                        │ ← SUPPORT │
│       │  (Component isolation, fast feedback)          │   Fast dev │
│       └────────────────────────────────────────────────┘   feedback │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

### Test-First Development Process (Updated)

```
For each feature:
1. Write failing E2E test (what does user see?)           ← START HERE
2. Write failing integration test (how do modules interact?)
3. Write unit tests as needed for complex logic
4. Implement minimum code to pass ALL tests
5. Refactor for quality
6. Verify E2E still passes (regression gate)
7. Add to compatibility matrix
```

### Test Categories (Prioritized)

| Priority | Category | Purpose | Tools | CI Gate? |
|----------|----------|---------|-------|----------|
| **P0** | E2E Tests | Full user workflows across terminals | `expectrl`, `rexpect`, custom harness | **Yes - Blocks merge** |
| **P0** | Terminal Compat | Works in all terminal contexts | Matrix CI, real terminal VMs | **Yes - Blocks release** |
| **P1** | Integration Tests | Module boundaries work together | `tokio_test`, `tempfile` | **Yes - Blocks merge** |
| **P1** | Snapshot Tests | UI doesn't regress | `insta` | **Yes - Blocks merge** |
| **P2** | Unit Tests | Component logic correct | `#[test]`, `mockall` | Yes |
| **P2** | Property Tests | Invariants hold | `proptest` | Yes |
| **P3** | Benchmark Tests | Performance doesn't regress | `criterion` | Warning only |
| **P3** | Fuzzing | Security/robustness | `cargo-fuzz` | Nightly |

---

## E2E Testing Infrastructure

### The Core Insight

> "If an E2E test passes, the feature works. If only unit tests pass, we hope the feature works."

E2E tests for RCT must verify **actual terminal behavior**, not simulated behavior.

### E2E Test Harness

```rust
// tests/e2e/harness.rs

use expectrl::{spawn, Expect};
use std::time::Duration;

/// E2E test harness that spawns real RCT process
pub struct RctTestHarness {
    process: expectrl::Session,
    terminal_type: TerminalType,
    timeout: Duration,
}

#[derive(Debug, Clone)]
pub enum TerminalType {
    Xterm,
    Vt100,
    Screen,
    Tmux,
    Dumb,
}

impl RctTestHarness {
    /// Spawn RCT with specific terminal emulation
    pub fn spawn(terminal: TerminalType) -> Result<Self> {
        std::env::set_var("TERM", terminal.to_term_string());

        let process = spawn("./target/release/rct --api-key test")?;

        Ok(Self {
            process,
            terminal_type: terminal,
            timeout: Duration::from_secs(10),
        })
    }

    /// Send input as if user typed it
    pub fn type_input(&mut self, input: &str) -> Result<()> {
        self.process.send_line(input)?;
        Ok(())
    }

    /// Wait for specific output pattern
    pub fn expect(&mut self, pattern: &str) -> Result<()> {
        self.process.expect(pattern)?;
        Ok(())
    }

    /// Expect output within timeout
    pub fn expect_within(&mut self, pattern: &str, timeout: Duration) -> Result<()> {
        // ...
    }

    /// Send control character (Ctrl+C, Ctrl+D, etc.)
    pub fn send_control(&mut self, c: char) -> Result<()> {
        let ctrl = (c as u8) - 64; // Ctrl+C = 0x03
        self.process.send(&[ctrl])?;
        Ok(())
    }

    /// Capture current screen state for snapshot
    pub fn capture_screen(&self) -> String {
        // Parse ANSI output into screen buffer
    }

    /// Verify no ANSI corruption or escape sequence leaks
    pub fn assert_clean_output(&self) -> Result<()> {
        let screen = self.capture_screen();
        assert!(!screen.contains("\x1b["), "Raw escape sequences in output");
        Ok(())
    }
}
```

### E2E Test Examples

```rust
// tests/e2e/basic_workflow_test.rs

#[test]
fn test_startup_and_quit() {
    for terminal in [TerminalType::Xterm, TerminalType::Vt100, TerminalType::Dumb] {
        let mut rct = RctTestHarness::spawn(terminal).unwrap();

        // Should show input prompt
        rct.expect("Input").unwrap();

        // Quit with Ctrl+C
        rct.send_control('c').unwrap();

        // Should exit cleanly
        assert!(rct.process.wait().unwrap().success());
    }
}

#[test]
fn test_message_send_and_receive() {
    let mut rct = RctTestHarness::spawn(TerminalType::Xterm).unwrap();

    // Type a message
    rct.type_input("Hello, Claude!").unwrap();
    rct.type_input("\n").unwrap(); // Enter

    // Should show "You:" prefix
    rct.expect("You:").unwrap();

    // Should show loading indicator
    rct.expect_any(&["⠋", "⠙", "⠹", "⠸"]).unwrap();

    // Should eventually show response (mock API)
    rct.expect("Claude:").unwrap();

    // Clean quit
    rct.send_control('c').unwrap();
}

#[test]
fn test_streaming_doesnt_flicker() {
    let mut rct = RctTestHarness::spawn(TerminalType::Xterm).unwrap();

    rct.type_input("Write a short poem\n").unwrap();

    // Capture multiple frames during streaming
    let mut frames = Vec::new();
    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(100));
        frames.push(rct.capture_screen());
    }

    // Verify no screen corruption between frames
    for (i, frame) in frames.iter().enumerate() {
        // Each frame should be valid (no partial escape sequences)
        assert!(
            is_valid_screen_state(frame),
            "Frame {} has corruption: {:?}",
            i,
            frame
        );
    }
}

#[test]
fn test_resize_handling() {
    let mut rct = RctTestHarness::spawn(TerminalType::Xterm).unwrap();

    // Add some content
    rct.type_input("Hello\n").unwrap();
    rct.expect("Claude:").unwrap();

    // Simulate resize (SIGWINCH)
    rct.resize(40, 20).unwrap();

    // Should re-render correctly
    rct.expect("Input").unwrap(); // UI still functional

    // Verify no corruption
    rct.assert_clean_output().unwrap();
}

#[test]
fn test_long_message_scrolling() {
    let mut rct = RctTestHarness::spawn(TerminalType::Xterm).unwrap();

    // Request long response
    rct.type_input("Write a 500 word essay\n").unwrap();

    // Wait for response
    rct.expect("Claude:").unwrap();

    // Scroll up
    rct.send_control_key(KeyCode::PageUp).unwrap();

    // Content should still be visible (not lost)
    rct.expect("Claude:").unwrap();

    // Scroll down
    rct.send_control_key(KeyCode::PageDown).unwrap();

    // Should see recent content
    rct.assert_clean_output().unwrap();
}
```

### Terminal Compatibility Matrix

**Every E2E test runs across this matrix:**

| Terminal | Platform | Priority | Notes |
|----------|----------|----------|-------|
| **xterm-256color** | All | P0 | Reference implementation |
| **screen** | All | P0 | tmux/screen sessions |
| **vt100** | All | P0 | Legacy baseline |
| **xterm** | All | P0 | Common default |
| **dumb** | All | P1 | Minimal terminal |
| **iTerm2** | macOS | P1 | Popular macOS |
| **Terminal.app** | macOS | P1 | macOS default |
| **Windows Terminal** | Windows | P1 | Modern Windows |
| **cmd.exe** | Windows | P2 | Legacy Windows |
| **Ghostty** | All | P2 | Modern GPU-accelerated |
| **Kitty** | Linux/macOS | P2 | Advanced features |
| **Alacritty** | All | P2 | Minimal GPU |
| **Konsole** | Linux | P3 | KDE default |
| **GNOME Terminal** | Linux | P3 | GNOME default |

### CI Matrix Configuration

```yaml
# .github/workflows/e2e.yml
name: E2E Tests

on: [push, pull_request]

jobs:
  e2e-linux:
    runs-on: ubuntu-latest
    strategy:
      matrix:
        term: [xterm-256color, xterm, vt100, screen, dumb]
    steps:
      - uses: actions/checkout@v4
      - name: Build release
        run: cargo build --release
      - name: Run E2E tests
        env:
          TERM: ${{ matrix.term }}
          RCT_TEST_API_KEY: ${{ secrets.TEST_API_KEY }}
        run: cargo test --test e2e -- --test-threads=1

  e2e-macos:
    runs-on: macos-latest
    strategy:
      matrix:
        term: [xterm-256color, screen]
    steps:
      - uses: actions/checkout@v4
      - run: cargo build --release
      - run: cargo test --test e2e -- --test-threads=1
        env:
          TERM: ${{ matrix.term }}

  e2e-windows:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo build --release
      - run: cargo test --test e2e -- --test-threads=1

  # Real terminal testing (scheduled, not every PR)
  e2e-real-terminals:
    runs-on: ubuntu-latest
    if: github.event_name == 'schedule' || github.ref == 'refs/heads/main'
    services:
      xvfb:
        image: 'ghcr.io/postrv/xvfb-terminal-runner'
    steps:
      - uses: actions/checkout@v4
      - name: Test in real terminal emulators
        run: |
          # Spawn actual terminal emulators and run tests
          ./scripts/test-real-terminals.sh
```

---

## Integration Testing Strategy

### Integration Test Boundaries

Integration tests verify that **module boundaries work correctly together**.

```
┌─────────────────────────────────────────────────────────────────────┐
│                    INTEGRATION TEST BOUNDARIES                      │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ┌─────────────┐     ┌─────────────┐     ┌─────────────┐          │
│  │    App      │────►│    API      │────►│   TUI       │          │
│  │   State     │     │   Client    │     │  Renderer   │          │
│  └─────────────┘     └─────────────┘     └─────────────┘          │
│        │                   │                   │                    │
│        │ Integration       │ Integration       │ Integration       │
│        │ Boundary 1        │ Boundary 2        │ Boundary 3        │
│        ▼                   ▼                   ▼                    │
│  ┌─────────────┐     ┌─────────────┐     ┌─────────────┐          │
│  │   Tools     │     │    MCP      │     │   Hooks     │          │
│  │  Executor   │     │   Client    │     │  Executor   │          │
│  └─────────────┘     └─────────────┘     └─────────────┘          │
│                                                                     │
│  Each boundary = Integration test suite                            │
│  All boundaries together = E2E test                                │
└─────────────────────────────────────────────────────────────────────┘
```

### Integration Test Examples

```rust
// tests/integration/api_state_integration_test.rs

/// Tests that API responses correctly update app state
#[tokio::test]
async fn test_api_response_updates_state() {
    // Real components, mocked external API
    let mock_api = MockAnthropicServer::start().await;
    let client = AnthropicClient::new_with_url(mock_api.url());
    let mut state = AppState::new(tempdir().unwrap().path());

    // Simulate user sending message
    state.submit_message(&client, "Hello".into()).await.unwrap();

    // API returns streaming response
    mock_api.send_content_delta("Hi ").await;
    mock_api.send_content_delta("there!").await;
    mock_api.send_message_stop().await;

    // Process all events
    while let Some(event) = state.recv_api_chunk().await {
        state.append_chunk(event).unwrap();
    }

    // Verify state is correct
    assert_eq!(state.messages.len(), 2);
    assert_eq!(state.messages[1].content, "Hi there!");
    assert!(!state.is_loading());
}

// tests/integration/tool_state_integration_test.rs

/// Tests that tool execution integrates with state correctly
#[tokio::test]
async fn test_tool_execution_updates_state() {
    let dir = tempdir().unwrap();
    let mut state = AppState::new(dir.path());
    let executor = ToolExecutor::new(dir.path().to_path_buf());

    // Create a file via tool
    let result = executor.execute(ToolCall {
        name: "write_file".into(),
        input: json!({"path": "test.txt", "content": "hello"}),
    }).await.unwrap();

    // Verify file exists
    assert!(dir.path().join("test.txt").exists());

    // Verify tool result can be added to state
    state.add_tool_result("write_file", result);
    assert!(state.last_tool_result().is_some());
}

// tests/integration/hook_tool_integration_test.rs

/// Tests that hooks can block tool execution
#[tokio::test]
async fn test_hook_blocks_tool() {
    let mut hook_executor = HookExecutor::new();
    hook_executor.register(HookEvent::PreToolUse, vec![
        HookDefinition {
            matcher: Some("Bash".into()),
            hooks: vec![HookCommand {
                hook_type: "command".into(),
                command: "exit 2".into(), // Block
                timeout_ms: Some(1000),
            }],
        },
    ]);

    let tool_executor = ToolExecutor::new(tempdir().unwrap().path().to_path_buf());

    // Attempt tool execution through hooks
    let context = HookContext {
        tool_name: Some("Bash".into()),
        tool_input: Some(json!({"command": "ls"})),
        ..Default::default()
    };

    let hook_result = hook_executor.execute(HookEvent::PreToolUse, &context).await.unwrap();

    // Hook should block
    assert!(matches!(hook_result.decision, HookDecision::Block { .. }));

    // Tool should NOT be executed when hook blocks
    // (This is the integration point we're testing)
}

// tests/integration/mcp_tool_integration_test.rs

/// Tests that MCP tools integrate with built-in tools
#[tokio::test]
async fn test_mcp_tools_available_to_executor() {
    let mut mcp_manager = McpManager::new();
    let mock_server = MockMcpServer::start().await;

    mock_server.register_tool("custom_search", json!({
        "description": "Custom search",
        "inputSchema": {"type": "object"}
    })).await;

    mcp_manager.connect("test", mock_server.config()).await.unwrap();

    let executor = ToolExecutor::new(tempdir().unwrap().path().to_path_buf())
        .with_mcp_manager(mcp_manager);

    // MCP tool should be callable through executor
    let result = executor.execute(ToolCall {
        name: "mcp:test:custom_search".into(),
        input: json!({"query": "test"}),
    }).await;

    assert!(result.is_ok());
}
```

### Regression Prevention: The Lock File

Every passing E2E test generates a "behavior signature" that gets committed:

```rust
// tests/e2e/behavior_lock.rs

/// Generate behavior signature for an E2E test
pub fn capture_behavior_signature(test_name: &str, harness: &RctTestHarness) -> BehaviorSignature {
    BehaviorSignature {
        test_name: test_name.to_string(),
        terminal_type: harness.terminal_type.clone(),
        final_screen_hash: hash(harness.capture_screen()),
        event_sequence: harness.event_log.clone(),
        timing_bounds: harness.timing_stats(),
    }
}

/// Lock file: tests/e2e/behavior.lock
/// Committed to repo, reviewed in PRs
#[derive(Serialize, Deserialize)]
pub struct BehaviorLock {
    pub version: String,
    pub signatures: HashMap<String, BehaviorSignature>,
}
```

When behavior changes:
1. Test fails with clear diff
2. Developer must explicitly update lock file
3. Lock file change visible in PR review
4. Intentional changes get approved
5. Accidental regressions get caught

---

## System Coherence Testing

### The Decoherence Problem

> "Optimized subcomponents not rolling up into a coherent top level system"

This happens when:
- Module A is optimized independently
- Module B is optimized independently
- A + B together have emergent bugs neither had alone

### Coherence Test Suite

```rust
// tests/coherence/mod.rs

/// Tests that verify system-level coherence
/// These run AFTER all other tests pass

#[test]
fn test_no_state_leakage_between_messages() {
    // Send 100 messages
    // Verify no cross-contamination
}

#[test]
fn test_concurrent_tool_execution_coherence() {
    // Run multiple tools in parallel
    // Verify results don't interfere
}

#[test]
fn test_rapid_input_during_streaming() {
    // Type while response is streaming
    // Verify no corruption or lost input
}

#[test]
fn test_resize_during_streaming() {
    // Resize terminal during streaming
    // Verify render recovers correctly
}

#[test]
fn test_hook_failure_doesnt_corrupt_state() {
    // Hook crashes
    // Verify main state is still valid
}

#[test]
fn test_mcp_server_crash_recovery() {
    // MCP server dies mid-call
    // Verify graceful degradation
}

#[test]
fn test_memory_stability_over_long_session() {
    // Run 1000 message cycles
    // Verify memory doesn't grow unbounded
}

#[test]
fn test_interrupt_during_file_write() {
    // Ctrl+C during file write
    // Verify file isn't corrupted
}
```

### Chaos Testing (Advanced)

For ultimate confidence, inject failures randomly:

```rust
// tests/chaos/mod.rs

use chaos_monkey::{ChaosConfig, Fault};

#[test]
fn test_survives_chaos() {
    let chaos = ChaosConfig::new()
        .with_fault(Fault::RandomDelay { max_ms: 500 })
        .with_fault(Fault::NetworkDrop { probability: 0.1 })
        .with_fault(Fault::ProcessSignal { signal: "SIGWINCH" })
        .with_fault(Fault::MemoryPressure { probability: 0.05 });

    let mut rct = RctTestHarness::spawn_with_chaos(chaos);

    // Run standard workflow 100 times
    for _ in 0..100 {
        rct.type_input("Test message\n").unwrap();
        rct.expect_any(&["Claude:", "Error:"]).unwrap();
    }

    // Should still be responsive
    rct.send_control('c').unwrap();
    assert!(rct.process.wait().unwrap().success());
}
```

---

## Test Infrastructure Summary

### CI Pipeline Order

```yaml
test:
  stages:
    - stage: unit
      name: "Unit Tests"
      fast: true
      blocking: true

    - stage: integration
      name: "Integration Tests"
      fast: true
      blocking: true

    - stage: snapshot
      name: "Snapshot Tests"
      fast: true
      blocking: true

    - stage: e2e
      name: "E2E Tests (Matrix)"
      slow: true
      blocking: true  # ← BLOCKS MERGE

    - stage: coherence
      name: "Coherence Tests"
      slow: true
      blocking: true  # ← BLOCKS MERGE

    - stage: benchmark
      name: "Benchmarks"
      slow: true
      blocking: false  # Warning only

    - stage: chaos
      name: "Chaos Tests"
      scheduled: nightly
      blocking: false
```

### Test File Structure

```
tests/
├── unit/                    # Traditional unit tests
│   ├── api_client_test.rs
│   ├── state_test.rs
│   └── ...
│
├── integration/             # Module boundary tests
│   ├── api_state_test.rs
│   ├── tool_hook_test.rs
│   └── mcp_executor_test.rs
│
├── e2e/                     # End-to-end (PRIMARY)
│   ├── harness.rs           # Test harness
│   ├── behavior.lock        # Behavior lock file
│   ├── basic_workflow_test.rs
│   ├── streaming_test.rs
│   ├── tool_execution_test.rs
│   └── ...
│
├── coherence/               # System coherence
│   ├── state_isolation_test.rs
│   ├── concurrent_test.rs
│   └── recovery_test.rs
│
├── chaos/                   # Chaos/fault injection
│   └── survival_test.rs
│
├── snapshots/               # Insta snapshots
│   └── *.snap
│
└── fixtures/                # Shared test data
    ├── mock_api_responses/
    └── test_projects/
```

### Definition of "Test Complete"

A feature is NOT complete until:

| Requirement | What It Means |
|-------------|---------------|
| E2E test exists | User can actually do the thing |
| E2E passes all terminals | Works in xterm, vt100, screen, dumb |
| Integration tests exist | Module boundaries verified |
| Behavior locked | Changes require explicit approval |
| No coherence regression | System still works as whole |
| Snapshot approved | UI looks correct |
| **VLM visual check passes** | AI confirms it looks right |

---

## VLM-Powered Visual Testing (SOTA)

### The Insight

Traditional terminal testing has a fundamental limitation:

```
Text-based assertion: "Output contains 'Claude:'"  ✓
Reality: Text is there but UI is corrupted        ✗

VLM assertion: "Does this terminal look correct?"  ✓
Reality: VLM sees the corruption humans would see  ✓
```

**VLMs can answer questions humans would ask:**
- "Does this look like a properly rendered chat interface?"
- "Is there any visual corruption or misalignment?"
- "Does the streaming text appear smooth or glitchy?"
- "Are the colors correct and readable?"

### Architecture: Visual Test Pipeline

```
┌─────────────────────────────────────────────────────────────────────┐
│                    VLM VISUAL TESTING PIPELINE                      │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────────────────┐ │
│  │   RCT       │    │  Terminal   │    │   Screenshot            │ │
│  │  Process    │───►│  Emulator   │───►│   Capture               │ │
│  │             │    │  (headless) │    │   (PNG/WebP)            │ │
│  └─────────────┘    └─────────────┘    └───────────┬─────────────┘ │
│                                                     │               │
│                                        ┌────────────▼────────────┐ │
│                                        │   VLM Analysis          │ │
│                                        │   (Claude Vision /      │ │
│                                        │    GPT-4V / Gemini)     │ │
│                                        └────────────┬────────────┘ │
│                                                     │               │
│  ┌─────────────────────────────────────────────────▼─────────────┐ │
│  │                    Structured Assessment                       │ │
│  │  {                                                             │ │
│  │    "layout_correct": true,                                     │ │
│  │    "text_readable": true,                                      │ │
│  │    "colors_appropriate": true,                                 │ │
│  │    "no_visual_corruption": true,                               │ │
│  │    "streaming_smooth": true,                                   │ │
│  │    "issues": [],                                               │ │
│  │    "confidence": 0.95                                          │ │
│  │  }                                                             │ │
│  └───────────────────────────────────────────────────────────────┘ │
│                                                                     │
│  Future: Video Analysis Pipeline                                    │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────────────────┐ │
│  │  Session    │    │   Video     │    │   Temporal VLM          │ │
│  │  Recording  │───►│  Encoding   │───►│   (Gemini 2.0 /        │ │
│  │  (asciinema)│    │  (MP4/WebM) │    │    Future models)       │ │
│  └─────────────┘    └─────────────┘    └─────────────────────────┘ │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

### Playwright + xterm.js Infrastructure

We use **Playwright** as the backbone - it's battle-tested for visual testing with built-in:
- Screenshot comparison
- Video recording
- Network interception
- Cross-browser testing
- Visual regression APIs

```typescript
// tests/visual/playwright.config.ts
import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './tests/visual',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,

  use: {
    // Capture screenshot on failure
    screenshot: 'only-on-failure',
    // Record video for debugging
    video: 'retain-on-failure',
    // Trace for debugging
    trace: 'retain-on-failure',
  },

  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
    {
      name: 'firefox',
      use: { ...devices['Desktop Firefox'] },
    },
    {
      name: 'webkit',
      use: { ...devices['Desktop Safari'] },
    },
  ],

  // Web server for xterm.js terminal
  webServer: {
    command: 'npm run serve-terminal',
    url: 'http://localhost:3456',
    reuseExistingServer: !process.env.CI,
  },
});
```

```typescript
// tests/visual/terminal-harness.ts
import { Page, expect } from '@playwright/test';

/**
 * Playwright-based terminal test harness
 * Uses xterm.js in browser connected to real RCT process
 */
export class TerminalHarness {
  constructor(private page: Page) {}

  static async create(page: Page): Promise<TerminalHarness> {
    await page.goto('http://localhost:3456/terminal');

    // Wait for terminal to initialize
    await page.waitForSelector('.xterm-screen');

    // Connect to RCT process via WebSocket
    await page.evaluate(() => {
      return (window as any).connectToRct();
    });

    return new TerminalHarness(page);
  }

  /** Type input as a real user would */
  async type(input: string): Promise<void> {
    await this.page.locator('.xterm-helper-textarea').type(input);
  }

  /** Press Enter */
  async submit(): Promise<void> {
    await this.page.keyboard.press('Enter');
  }

  /** Send control character */
  async sendControl(char: string): Promise<void> {
    await this.page.keyboard.press(`Control+${char}`);
  }

  /** Wait for text to appear in terminal */
  async waitForText(text: string, timeout = 10000): Promise<void> {
    await expect(this.page.locator('.xterm-screen'))
      .toContainText(text, { timeout });
  }

  /** Take screenshot of terminal area */
  async screenshot(name: string): Promise<Buffer> {
    const terminal = this.page.locator('.xterm-screen');
    return terminal.screenshot({ path: `tests/visual/screenshots/${name}.png` });
  }

  /** Compare against baseline */
  async expectVisualMatch(name: string): Promise<void> {
    const terminal = this.page.locator('.xterm-screen');
    await expect(terminal).toHaveScreenshot(`${name}.png`, {
      maxDiffPixels: 100,  // Allow minor differences
      threshold: 0.2,      // Per-pixel threshold
    });
  }

  /** Record video of interaction */
  async startRecording(): Promise<void> {
    // Playwright records automatically with video: 'on'
  }

  /** Get terminal content as text */
  async getContent(): Promise<string> {
    return this.page.evaluate(() => {
      return (window as any).terminal.buffer.active.getLine(0)?.translateToString() || '';
    });
  }
}
```

### xterm.js Test Server

```typescript
// tests/visual/server/index.ts
import express from 'express';
import { WebSocketServer } from 'ws';
import { spawn } from 'node-pty';

const app = express();
app.use(express.static('tests/visual/server/public'));

const server = app.listen(3456);
const wss = new WebSocketServer({ server });

wss.on('connection', (ws) => {
  // Spawn RCT process
  const pty = spawn('./target/release/rct', ['--api-key', process.env.RCT_TEST_API_KEY], {
    name: 'xterm-256color',
    cols: 120,
    rows: 40,
    env: { ...process.env, TERM: 'xterm-256color' },
  });

  // Pipe pty output to WebSocket
  pty.onData((data) => ws.send(data));

  // Pipe WebSocket input to pty
  ws.on('message', (data) => pty.write(data.toString()));

  ws.on('close', () => pty.kill());
});
```

```html
<!-- tests/visual/server/public/terminal.html -->
<!DOCTYPE html>
<html>
<head>
  <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/xterm/css/xterm.css" />
  <script src="https://cdn.jsdelivr.net/npm/xterm/lib/xterm.js"></script>
  <script src="https://cdn.jsdelivr.net/npm/xterm-addon-fit/lib/xterm-addon-fit.js"></script>
  <style>
    body { margin: 0; background: #1e1e1e; }
    #terminal { width: 100vw; height: 100vh; }
  </style>
</head>
<body>
  <div id="terminal"></div>
  <script>
    const terminal = new Terminal({
      cols: 120,
      rows: 40,
      fontFamily: 'Menlo, Monaco, monospace',
      fontSize: 14,
      theme: {
        background: '#1e1e1e',
        foreground: '#d4d4d4',
      }
    });

    const fitAddon = new FitAddon.FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.open(document.getElementById('terminal'));
    fitAddon.fit();

    window.terminal = terminal;

    window.connectToRct = () => {
      return new Promise((resolve) => {
        const ws = new WebSocket('ws://localhost:3456');
        ws.onopen = () => resolve();
        ws.onmessage = (e) => terminal.write(e.data);
        terminal.onData((data) => ws.send(data));
        window.ws = ws;
      });
    };
  </script>
</body>
</html>
```

### Claude Cowork/Computer Use Integration

For the most sophisticated testing, we can use **Claude's Computer Use** or **Cowork** capabilities to act as an intelligent test agent:

```typescript
// tests/visual/claude-tester.ts
import Anthropic from '@anthropic-ai/sdk';
import { Page } from '@playwright/test';

/**
 * Uses Claude's vision + computer use to intelligently test the terminal
 * This is the SOTA approach - AI that can see and interact like a human
 */
export class ClaudeTester {
  private client: Anthropic;

  constructor(private page: Page) {
    this.client = new Anthropic();
  }

  /**
   * Ask Claude to perform a test scenario and assess the result
   */
  async executeTestScenario(scenario: string): Promise<TestResult> {
    // Take initial screenshot
    const beforeScreenshot = await this.captureScreen();

    // Ask Claude to execute the scenario using computer use
    const response = await this.client.messages.create({
      model: 'claude-sonnet-4-20250514',
      max_tokens: 4096,
      tools: [
        {
          type: 'computer_20241022',
          name: 'computer',
          display_width_px: 1200,
          display_height_px: 800,
        }
      ],
      messages: [{
        role: 'user',
        content: [
          {
            type: 'image',
            source: {
              type: 'base64',
              media_type: 'image/png',
              data: beforeScreenshot,
            },
          },
          {
            type: 'text',
            text: `You are testing a terminal application called RCT.

Scenario: ${scenario}

Please:
1. Execute the necessary interactions to complete this scenario
2. Observe the terminal's response
3. Assess whether the behavior is correct

After each action, I'll show you the updated screen.`,
          },
        ],
      }],
    });

    // Process Claude's actions
    const actions = this.extractActions(response);
    for (const action of actions) {
      await this.executeAction(action);
      // Give Claude the updated screenshot
    }

    // Get final assessment
    const finalScreenshot = await this.captureScreen();
    const assessment = await this.getAssessment(finalScreenshot, scenario);

    return assessment;
  }

  /**
   * Have Claude visually assess a screenshot
   */
  async assessVisual(context: string): Promise<VisualAssessment> {
    const screenshot = await this.captureScreen();

    const response = await this.client.messages.create({
      model: 'claude-sonnet-4-20250514',
      max_tokens: 2048,
      messages: [{
        role: 'user',
        content: [
          {
            type: 'image',
            source: {
              type: 'base64',
              media_type: 'image/png',
              data: screenshot,
            },
          },
          {
            type: 'text',
            text: `Analyze this terminal screenshot for a QA test.

Context: ${context}

Assess:
1. Is the layout correct? (message area, input area, borders)
2. Is all text readable? No overlapping or corruption?
3. Are colors appropriate? (user vs assistant messages)
4. Are there any visual artifacts or rendering errors?
5. Is the overall UX professional and polished?

Respond with JSON:
{
  "pass": boolean,
  "confidence": number (0-1),
  "issues": [{"severity": "critical|major|minor", "description": string}],
  "notes": string
}`,
          },
        ],
      }],
    });

    return JSON.parse(response.content[0].text);
  }

  /**
   * Watch a video recording and assess temporal behavior
   * (For when video-capable models are available)
   */
  async assessVideo(videoPath: string, expectations: string): Promise<VideoAssessment> {
    // Read video file
    const videoBase64 = await fs.readFile(videoPath, 'base64');

    // When Gemini 2.0 or similar is available:
    // const response = await gemini.generateContent({
    //   contents: [{ parts: [
    //     { inlineData: { mimeType: 'video/mp4', data: videoBase64 } },
    //     { text: `Analyze this terminal session video...` }
    //   ]}]
    // });

    // For now, extract frames and analyze sequence
    const frames = await this.extractKeyFrames(videoPath);
    return this.analyzeFrameSequence(frames, expectations);
  }

  private async captureScreen(): Promise<string> {
    const buffer = await this.page.screenshot();
    return buffer.toString('base64');
  }
}

interface TestResult {
  passed: boolean;
  scenario: string;
  actions_taken: string[];
  final_assessment: VisualAssessment;
  duration_ms: number;
}

interface VisualAssessment {
  pass: boolean;
  confidence: number;
  issues: Array<{ severity: string; description: string }>;
  notes: string;
}
```

### Playwright Visual Test Examples

```typescript
// tests/visual/rct.spec.ts
import { test, expect } from '@playwright/test';
import { TerminalHarness } from './terminal-harness';
import { ClaudeTester } from './claude-tester';

test.describe('RCT Visual Tests', () => {
  let terminal: TerminalHarness;
  let claudeTester: ClaudeTester;

  test.beforeEach(async ({ page }) => {
    terminal = await TerminalHarness.create(page);
    claudeTester = new ClaudeTester(page);
  });

  test('startup renders correctly', async () => {
    // Playwright visual comparison
    await terminal.expectVisualMatch('startup');

    // Claude assessment for higher-level verification
    const assessment = await claudeTester.assessVisual(
      'Fresh startup state - should show empty message area with input prompt'
    );
    expect(assessment.pass).toBe(true);
    expect(assessment.confidence).toBeGreaterThan(0.8);
  });

  test('message exchange displays correctly', async () => {
    await terminal.type('Hello, Claude!');
    await terminal.submit();

    // Wait for response
    await terminal.waitForText('Claude:', 30000);

    // Visual regression check
    await terminal.expectVisualMatch('message-exchange');

    // AI assessment
    const assessment = await claudeTester.assessVisual(
      'After sending "Hello, Claude!" - should show user message and assistant response'
    );
    expect(assessment.pass).toBe(true);
    expect(assessment.issues.filter(i => i.severity === 'critical')).toHaveLength(0);
  });

  test('streaming appears smooth', async ({ page }) => {
    // Enable video recording for this test
    await page.video()?.saveAs('tests/visual/videos/streaming.webm');

    await terminal.type('Write a haiku about programming');
    await terminal.submit();

    // Capture frames during streaming
    const frames: Buffer[] = [];
    for (let i = 0; i < 20; i++) {
      await page.waitForTimeout(200);
      frames.push(await terminal.screenshot(`streaming-frame-${i}`));
    }

    // Check no frame shows corruption
    for (let i = 0; i < frames.length; i++) {
      const assessment = await claudeTester.assessVisual(
        `Streaming frame ${i} - text should be appearing progressively`
      );
      expect(assessment.issues.filter(i => i.severity === 'critical')).toHaveLength(0);
    }
  });

  test('handles resize gracefully', async ({ page }) => {
    await terminal.type('Some content');
    await terminal.submit();
    await terminal.waitForText('Claude:');

    // Resize
    await page.setViewportSize({ width: 800, height: 400 });
    await page.waitForTimeout(500);

    // Should re-render correctly
    const assessment = await claudeTester.assessVisual(
      'After resize to 800x400 - content should reflow correctly'
    );
    expect(assessment.pass).toBe(true);
  });

  test('Claude executes full workflow', async () => {
    // Let Claude drive the entire test scenario
    const result = await claudeTester.executeTestScenario(`
      1. Type "What is 2+2?" and press Enter
      2. Wait for the response
      3. Verify the response mentions "4"
      4. Type a follow-up question
      5. Verify the conversation continues correctly
    `);

    expect(result.passed).toBe(true);
  });
});

// Cross-terminal compatibility tests
test.describe('Terminal Compatibility', () => {
  const terminals = [
    { name: 'xterm-256color', env: { TERM: 'xterm-256color' } },
    { name: 'xterm', env: { TERM: 'xterm' } },
    { name: 'vt100', env: { TERM: 'vt100' } },
    { name: 'screen', env: { TERM: 'screen' } },
  ];

  for (const { name, env } of terminals) {
    test(`works in ${name}`, async ({ page }) => {
      // Set terminal type
      process.env = { ...process.env, ...env };

      const terminal = await TerminalHarness.create(page);
      await terminal.type('Hello');
      await terminal.submit();
      await terminal.waitForText('Claude:');

      // Should work in this terminal type
      await terminal.expectVisualMatch(`compat-${name}`);
    });
  }
});
```

### CI Configuration for Playwright

```yaml
# .github/workflows/visual-tests.yml
name: Visual Tests

on:
  push:
    branches: [main]
  pull_request:

jobs:
  visual-tests:
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@v4

      - name: Setup Node.js
        uses: actions/setup-node@v4
        with:
          node-version: '20'

      - name: Install Playwright
        run: |
          npm ci
          npx playwright install --with-deps

      - name: Build RCT
        run: cargo build --release

      - name: Run Playwright tests
        env:
          RCT_TEST_API_KEY: ${{ secrets.TEST_API_KEY }}
          ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}
        run: npx playwright test

      - name: Upload test results
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: playwright-report
          path: playwright-report/

      - name: Upload screenshots
        if: failure()
        uses: actions/upload-artifact@v4
        with:
          name: visual-diffs
          path: |
            tests/visual/screenshots/
            test-results/

  # Update baselines (manual trigger)
  update-baselines:
    runs-on: ubuntu-latest
    if: github.event_name == 'workflow_dispatch'

    steps:
      - uses: actions/checkout@v4

      - name: Run tests and update snapshots
        run: npx playwright test --update-snapshots

      - name: Commit updated baselines
        run: |
          git config user.name "github-actions[bot]"
          git config user.email "github-actions[bot]@users.noreply.github.com"
          git add tests/visual/screenshots/
          git commit -m "chore: update visual baselines" || exit 0
          git push
```
```

### VLM Analysis Client

```rust
// tests/visual/vlm.rs

use base64::Engine;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub enum VlmProvider {
    Claude,      // Claude 3.5 Sonnet with vision
    Gpt4V,       // GPT-4 Vision
    Gemini,      // Gemini Pro Vision
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VisualAssessment {
    pub layout_correct: bool,
    pub text_readable: bool,
    pub colors_appropriate: bool,
    pub no_visual_corruption: bool,
    pub alignment_correct: bool,
    pub spacing_consistent: bool,
    pub issues: Vec<VisualIssue>,
    pub confidence: f32,
    pub raw_analysis: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VisualIssue {
    pub severity: IssueSeverity,
    pub description: String,
    pub location: Option<String>,  // e.g., "top-left", "input area"
}

#[derive(Debug, Serialize, Deserialize)]
pub enum IssueSeverity {
    Critical,  // Unusable
    Major,     // Significant UX problem
    Minor,     // Cosmetic
    Info,      // Suggestion
}

pub struct VlmClient {
    provider: VlmProvider,
    api_key: String,
}

impl VlmClient {
    /// Analyze a terminal screenshot
    pub async fn analyze_screenshot(
        &self,
        image: &DynamicImage,
        context: &VisualTestContext,
    ) -> Result<VisualAssessment> {
        let base64_image = self.encode_image(image)?;

        let prompt = format!(r#"
You are a QA engineer testing a terminal application called RCT (Rust Claude Terminal).
Analyze this screenshot and assess whether it displays correctly.

Context:
- Expected state: {}
- Terminal type: {}
- Test scenario: {}

Evaluate the following criteria and respond with JSON:

1. layout_correct: Is the overall layout correct? (messages area, input area, borders)
2. text_readable: Is all text clearly readable? No overlapping or cut-off text?
3. colors_appropriate: Are colors correct? (user messages, assistant messages, UI elements)
4. no_visual_corruption: Are there any visual artifacts, garbled characters, or rendering errors?
5. alignment_correct: Are elements properly aligned? (text, borders, indicators)
6. spacing_consistent: Is spacing between elements consistent?
7. issues: List any specific issues found with severity and description
8. confidence: Your confidence in this assessment (0.0-1.0)

Respond ONLY with valid JSON matching this schema:
{{
  "layout_correct": boolean,
  "text_readable": boolean,
  "colors_appropriate": boolean,
  "no_visual_corruption": boolean,
  "alignment_correct": boolean,
  "spacing_consistent": boolean,
  "issues": [{{ "severity": "Critical|Major|Minor|Info", "description": string, "location": string|null }}],
  "confidence": number,
  "raw_analysis": string  // Brief explanation of your assessment
}}
"#, context.expected_state, context.terminal_type, context.scenario);

        let response = self.call_vlm(&prompt, &base64_image).await?;
        let assessment: VisualAssessment = serde_json::from_str(&response)?;

        Ok(assessment)
    }

    /// Analyze a sequence of screenshots for temporal issues
    pub async fn analyze_sequence(
        &self,
        frames: &[DynamicImage],
        context: &VisualTestContext,
    ) -> Result<TemporalAssessment> {
        // For now, analyze key frames
        // Future: Send as video to temporal VLM

        let prompt = format!(r#"
You are analyzing a sequence of {} terminal screenshots taken during a streaming response.
Evaluate the temporal behavior:

1. smooth_progression: Does text appear smoothly, character by character?
2. no_flickering: Are there any frames that appear corrupted then recover?
3. cursor_behavior: Does the cursor move correctly?
4. scroll_smooth: If scrolling occurred, was it smooth?
5. no_regression: Does any frame show less content than the previous?

Respond with JSON assessment.
"#, frames.len());

        // Encode multiple frames
        let frame_descriptions = self.analyze_key_frames(frames, context).await?;

        // Synthesize temporal assessment
        self.synthesize_temporal_assessment(&frame_descriptions).await
    }

    async fn call_vlm(&self, prompt: &str, base64_image: &str) -> Result<String> {
        match self.provider {
            VlmProvider::Claude => self.call_claude(prompt, base64_image).await,
            VlmProvider::Gpt4V => self.call_gpt4v(prompt, base64_image).await,
            VlmProvider::Gemini => self.call_gemini(prompt, base64_image).await,
        }
    }

    async fn call_claude(&self, prompt: &str, base64_image: &str) -> Result<String> {
        let client = reqwest::Client::new();
        let response = client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-20250514",
                "max_tokens": 2048,
                "messages": [{
                    "role": "user",
                    "content": [
                        {
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": "image/png",
                                "data": base64_image
                            }
                        },
                        {
                            "type": "text",
                            "text": prompt
                        }
                    ]
                }]
            }))
            .send()
            .await?;

        let body: serde_json::Value = response.json().await?;
        Ok(body["content"][0]["text"].as_str().unwrap().to_string())
    }
}

#[derive(Debug)]
pub struct VisualTestContext {
    pub expected_state: String,
    pub terminal_type: String,
    pub scenario: String,
}
```

### Visual E2E Tests

```rust
// tests/visual/visual_e2e_test.rs

use crate::visual::{TerminalCapture, VlmClient, VisualTestContext};

/// Visual E2E test that uses VLM to verify correctness
#[tokio::test]
async fn test_startup_screen_visual() {
    let capture = TerminalCapture::spawn().unwrap();
    let vlm = VlmClient::new(VlmProvider::Claude);

    // Wait for startup
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Capture screenshot
    let screenshot = capture.screenshot_terminal_area().unwrap();

    // Save for debugging
    screenshot.save("tests/visual/artifacts/startup.png").unwrap();

    // VLM analysis
    let assessment = vlm.analyze_screenshot(&screenshot, &VisualTestContext {
        expected_state: "Empty chat interface with input prompt at bottom".into(),
        terminal_type: "xterm-256color".into(),
        scenario: "Fresh startup, no messages yet".into(),
    }).await.unwrap();

    // Assert VLM findings
    assert!(assessment.layout_correct, "Layout issues: {:?}", assessment.issues);
    assert!(assessment.text_readable, "Readability issues: {:?}", assessment.issues);
    assert!(assessment.no_visual_corruption, "Corruption: {:?}", assessment.issues);
    assert!(assessment.confidence > 0.8, "Low confidence assessment");
    assert!(assessment.issues.iter().all(|i| !matches!(i.severity, IssueSeverity::Critical)));
}

#[tokio::test]
async fn test_message_display_visual() {
    let capture = TerminalCapture::spawn().unwrap();
    let vlm = VlmClient::new(VlmProvider::Claude);

    // Send a message
    capture.send_input("Hello, Claude!\r").unwrap();

    // Wait for response (mock API)
    tokio::time::sleep(Duration::from_secs(3)).await;

    let screenshot = capture.screenshot_terminal_area().unwrap();
    screenshot.save("tests/visual/artifacts/message.png").unwrap();

    let assessment = vlm.analyze_screenshot(&screenshot, &VisualTestContext {
        expected_state: "User message 'Hello, Claude!' followed by assistant response".into(),
        terminal_type: "xterm-256color".into(),
        scenario: "Simple message exchange".into(),
    }).await.unwrap();

    assert!(assessment.layout_correct);
    assert!(assessment.colors_appropriate, "Color issues - user/assistant should be different colors");
    assert!(assessment.no_visual_corruption);
}

#[tokio::test]
async fn test_streaming_visual_sequence() {
    let capture = TerminalCapture::spawn().unwrap();
    let vlm = VlmClient::new(VlmProvider::Claude);

    // Send a message that will generate streaming response
    capture.send_input("Write a short poem\r").unwrap();

    // Capture frames during streaming
    let mut frames = Vec::new();
    for i in 0..20 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let frame = capture.screenshot_terminal_area().unwrap();
        frame.save(format!("tests/visual/artifacts/streaming_{:02}.png", i)).unwrap();
        frames.push(frame);
    }

    // Analyze sequence
    let temporal = vlm.analyze_sequence(&frames, &VisualTestContext {
        expected_state: "Streaming response appearing progressively".into(),
        terminal_type: "xterm-256color".into(),
        scenario: "Streaming poem generation".into(),
    }).await.unwrap();

    assert!(temporal.smooth_progression, "Streaming should be smooth");
    assert!(temporal.no_flickering, "Should not flicker during streaming");
    assert!(temporal.no_regression, "Content should only grow, not shrink");
}

#[tokio::test]
async fn test_visual_across_terminal_types() {
    let terminals = vec![
        ("xterm-256color", "Modern terminal with full color"),
        ("xterm", "Basic xterm"),
        ("vt100", "Legacy VT100"),
        ("screen", "GNU Screen/tmux"),
    ];

    let vlm = VlmClient::new(VlmProvider::Claude);

    for (term_type, description) in terminals {
        std::env::set_var("TERM", term_type);
        let capture = TerminalCapture::spawn().unwrap();

        capture.send_input("Hello\r").unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;

        let screenshot = capture.screenshot_terminal_area().unwrap();
        screenshot.save(format!("tests/visual/artifacts/terminal_{}.png", term_type)).unwrap();

        let assessment = vlm.analyze_screenshot(&screenshot, &VisualTestContext {
            expected_state: "Chat interface with message".into(),
            terminal_type: term_type.into(),
            scenario: format!("{} compatibility test", description),
        }).await.unwrap();

        assert!(
            assessment.no_visual_corruption,
            "Terminal {} has corruption: {:?}",
            term_type,
            assessment.issues
        );
    }
}
```

### Video Recording for Future Analysis

```rust
// tests/visual/recording.rs

use std::process::{Command, Stdio};

/// Records terminal session for future video analysis
pub struct SessionRecorder {
    asciinema_process: std::process::Child,
    output_path: PathBuf,
}

impl SessionRecorder {
    /// Start recording with asciinema
    pub fn start(output_path: impl Into<PathBuf>) -> Result<Self> {
        let output_path = output_path.into();

        let process = Command::new("asciinema")
            .args(&["rec", "--stdin", "-q", output_path.to_str().unwrap()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;

        Ok(Self {
            asciinema_process: process,
            output_path,
        })
    }

    /// Stop recording
    pub fn stop(mut self) -> Result<RecordedSession> {
        self.asciinema_process.kill()?;
        self.asciinema_process.wait()?;

        // Convert to video for VLM analysis
        let video_path = self.output_path.with_extension("mp4");
        self.convert_to_video(&video_path)?;

        Ok(RecordedSession {
            asciicast_path: self.output_path,
            video_path,
        })
    }

    fn convert_to_video(&self, output: &Path) -> Result<()> {
        // Use agg (asciinema gif generator) or similar
        Command::new("agg")
            .args(&[
                self.output_path.to_str().unwrap(),
                output.with_extension("gif").to_str().unwrap(),
            ])
            .output()?;

        // Convert gif to mp4 for VLM
        Command::new("ffmpeg")
            .args(&[
                "-i", output.with_extension("gif").to_str().unwrap(),
                "-movflags", "faststart",
                "-pix_fmt", "yuv420p",
                "-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2",
                output.to_str().unwrap(),
            ])
            .output()?;

        Ok(())
    }
}

pub struct RecordedSession {
    pub asciicast_path: PathBuf,
    pub video_path: PathBuf,
}

impl RecordedSession {
    /// Analyze with video-capable VLM (future)
    pub async fn analyze_video(&self, vlm: &VlmClient) -> Result<VideoAssessment> {
        // For now: extract key frames and analyze
        // Future: Send full video to Gemini 2.0 or similar

        let frames = self.extract_key_frames()?;
        vlm.analyze_sequence(&frames, &VisualTestContext::default()).await
    }

    fn extract_key_frames(&self) -> Result<Vec<DynamicImage>> {
        // Use ffmpeg to extract frames
        let output_dir = tempfile::tempdir()?;

        Command::new("ffmpeg")
            .args(&[
                "-i", self.video_path.to_str().unwrap(),
                "-vf", "fps=2",  // 2 frames per second
                &format!("{}/frame_%04d.png", output_dir.path().display()),
            ])
            .output()?;

        // Load frames
        let mut frames = Vec::new();
        for entry in std::fs::read_dir(output_dir.path())? {
            let path = entry?.path();
            if path.extension().map_or(false, |e| e == "png") {
                frames.push(image::open(&path)?);
            }
        }

        frames.sort_by_key(|_| rand::random::<u32>()); // Sort by filename would be better
        Ok(frames)
    }
}

/// Future-proofed interface for video VLMs
#[async_trait::async_trait]
pub trait VideoVlm {
    /// Analyze a video file
    async fn analyze_video(
        &self,
        video_path: &Path,
        prompt: &str,
    ) -> Result<VideoAssessment>;
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VideoAssessment {
    pub smooth_animation: bool,
    pub no_flickering: bool,
    pub responsive_input: bool,
    pub correct_timing: bool,
    pub temporal_issues: Vec<TemporalIssue>,
    pub frame_by_frame_notes: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TemporalIssue {
    pub timestamp_ms: u64,
    pub description: String,
    pub severity: IssueSeverity,
}

/// Placeholder for Gemini 2.0 video analysis (coming soon)
pub struct GeminiVideoVlm {
    api_key: String,
}

#[async_trait::async_trait]
impl VideoVlm for GeminiVideoVlm {
    async fn analyze_video(
        &self,
        video_path: &Path,
        prompt: &str,
    ) -> Result<VideoAssessment> {
        // TODO: Implement when Gemini 2.0 video API is available
        // For now, fall back to frame extraction
        unimplemented!("Gemini 2.0 video analysis not yet available")
    }
}
```

### Visual Regression System

```rust
// tests/visual/regression.rs

use image_compare::{Algorithm, Similarity};

/// Visual baseline for regression detection
#[derive(Debug, Serialize, Deserialize)]
pub struct VisualBaseline {
    pub test_name: String,
    pub terminal_type: String,
    pub screenshot_hash: String,
    pub vlm_assessment: VisualAssessment,
    pub created_at: DateTime<Utc>,
}

pub struct VisualRegressionTracker {
    baselines_dir: PathBuf,
    tolerance: f64,  // Image similarity threshold (0.95 = 95% similar)
}

impl VisualRegressionTracker {
    /// Compare current screenshot against baseline
    pub fn check_regression(
        &self,
        test_name: &str,
        current: &DynamicImage,
    ) -> Result<RegressionResult> {
        let baseline_path = self.baselines_dir.join(format!("{}.png", test_name));

        if !baseline_path.exists() {
            return Ok(RegressionResult::NewBaseline);
        }

        let baseline = image::open(&baseline_path)?;

        // Structural similarity comparison
        let similarity = image_compare::rgba_hybrid_compare(
            &baseline.to_rgba8(),
            &current.to_rgba8(),
        )?.score;

        if similarity >= self.tolerance {
            Ok(RegressionResult::Pass { similarity })
        } else {
            // Generate diff image
            let diff = self.generate_diff(&baseline, current)?;
            diff.save(self.baselines_dir.join(format!("{}_diff.png", test_name)))?;

            Ok(RegressionResult::Regression {
                similarity,
                diff_path: self.baselines_dir.join(format!("{}_diff.png", test_name)),
            })
        }
    }

    /// Update baseline with new screenshot
    pub fn update_baseline(
        &self,
        test_name: &str,
        screenshot: &DynamicImage,
        assessment: &VisualAssessment,
    ) -> Result<()> {
        screenshot.save(self.baselines_dir.join(format!("{}.png", test_name)))?;

        let baseline = VisualBaseline {
            test_name: test_name.to_string(),
            terminal_type: "xterm-256color".to_string(), // from context
            screenshot_hash: self.hash_image(screenshot),
            vlm_assessment: assessment.clone(),
            created_at: Utc::now(),
        };

        let json = serde_json::to_string_pretty(&baseline)?;
        std::fs::write(
            self.baselines_dir.join(format!("{}.json", test_name)),
            json,
        )?;

        Ok(())
    }

    fn generate_diff(&self, a: &DynamicImage, b: &DynamicImage) -> Result<DynamicImage> {
        // Highlight differences in red
        let a_rgba = a.to_rgba8();
        let b_rgba = b.to_rgba8();
        let mut diff = image::RgbaImage::new(a_rgba.width(), a_rgba.height());

        for (x, y, pixel) in diff.enumerate_pixels_mut() {
            let a_pixel = a_rgba.get_pixel(x, y);
            let b_pixel = b_rgba.get_pixel(x, y);

            if a_pixel != b_pixel {
                *pixel = image::Rgba([255, 0, 0, 255]); // Red for differences
            } else {
                *pixel = *a_pixel;
            }
        }

        Ok(DynamicImage::ImageRgba8(diff))
    }
}

#[derive(Debug)]
pub enum RegressionResult {
    Pass { similarity: f64 },
    Regression { similarity: f64, diff_path: PathBuf },
    NewBaseline,
}
```

### CI Integration for Visual Tests

```yaml
# .github/workflows/visual-tests.yml
name: Visual Tests

on:
  push:
    branches: [main]
  pull_request:

jobs:
  visual-tests:
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@v4

      - name: Install dependencies
        run: |
          sudo apt-get update
          sudo apt-get install -y \
            chromium-browser \
            xvfb \
            ffmpeg \
            asciinema
          pip install agg

      - name: Build RCT
        run: cargo build --release

      - name: Run visual tests
        env:
          ANTHROPIC_API_KEY: ${{ secrets.VISUAL_TEST_API_KEY }}
          VLM_PROVIDER: claude
        run: |
          xvfb-run cargo test --test visual -- --test-threads=1

      - name: Upload visual artifacts
        if: failure()
        uses: actions/upload-artifact@v4
        with:
          name: visual-test-artifacts
          path: tests/visual/artifacts/

      - name: Check for regressions
        run: |
          if [ -n "$(git diff tests/visual/baselines/)" ]; then
            echo "Visual regression detected!"
            git diff tests/visual/baselines/
            exit 1
          fi

  # Nightly: Full video analysis (when available)
  video-analysis:
    runs-on: ubuntu-latest
    if: github.event_name == 'schedule'

    steps:
      - uses: actions/checkout@v4

      - name: Record test sessions
        run: |
          cargo build --release
          ./scripts/record-test-sessions.sh

      - name: Analyze with video VLM
        env:
          GEMINI_API_KEY: ${{ secrets.GEMINI_API_KEY }}
        run: |
          cargo test --test video_analysis -- --test-threads=1

      - name: Upload recordings
        uses: actions/upload-artifact@v4
        with:
          name: session-recordings
          path: tests/visual/recordings/
```

### Cost Management for VLM Tests

```rust
// tests/visual/cost.rs

/// Track and limit VLM API costs
pub struct VlmCostTracker {
    daily_budget_usd: f64,
    spent_today: f64,
    last_reset: DateTime<Utc>,
}

impl VlmCostTracker {
    const CLAUDE_VISION_COST_PER_IMAGE: f64 = 0.003;  // Approximate
    const GPT4V_COST_PER_IMAGE: f64 = 0.005;

    pub fn can_afford(&self, provider: VlmProvider) -> bool {
        let cost = match provider {
            VlmProvider::Claude => Self::CLAUDE_VISION_COST_PER_IMAGE,
            VlmProvider::Gpt4V => Self::GPT4V_COST_PER_IMAGE,
            VlmProvider::Gemini => 0.001, // Usually cheaper
        };

        self.spent_today + cost <= self.daily_budget_usd
    }

    pub fn record_usage(&mut self, provider: VlmProvider) {
        let cost = match provider {
            VlmProvider::Claude => Self::CLAUDE_VISION_COST_PER_IMAGE,
            VlmProvider::Gpt4V => Self::GPT4V_COST_PER_IMAGE,
            VlmProvider::Gemini => 0.001,
        };

        self.spent_today += cost;
    }
}

/// Only run expensive VLM tests in appropriate contexts
pub fn should_run_vlm_tests() -> bool {
    // Always run in CI
    if std::env::var("CI").is_ok() {
        return true;
    }

    // Run locally if explicitly requested
    if std::env::var("RCT_VLM_TESTS").is_ok() {
        return true;
    }

    // Skip expensive tests in regular local development
    false
}
```

### Summary: Visual Testing Stack (SOTA)

```
┌─────────────────────────────────────────────────────────────────────┐
│               VISUAL TESTING HIERARCHY (Playwright + VLM)           │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  Level 5: Autonomous AI Tester (Now - Claude Computer Use/Cowork)  │
│  ┌───────────────────────────────────────────────────────────────┐ │
│  │  Claude acts as intelligent test agent                         │ │
│  │  - Executes scenarios like a human would                       │ │
│  │  - Identifies issues humans would notice                       │ │
│  │  - Adapts to unexpected UI changes                             │ │
│  │  - Cowork for complex multi-step test workflows                │ │
│  └───────────────────────────────────────────────────────────────┘ │
│                              ▲                                      │
│                              │                                      │
│  Level 4: Video Analysis (Future - Gemini 2.0+)                    │
│  ┌───────────────────────────────────────────────────────────────┐ │
│  │  Full session videos analyzed for temporal correctness         │ │
│  │  - Streaming smoothness, animation quality                     │ │
│  │  - Playwright records automatically                            │ │
│  └───────────────────────────────────────────────────────────────┘ │
│                              ▲                                      │
│                              │                                      │
│  Level 3: VLM Screenshot Analysis (Now - Claude Vision)            │
│  ┌───────────────────────────────────────────────────────────────┐ │
│  │  AI-powered visual correctness checking                        │ │
│  │  - "Does this look right to a human?"                          │ │
│  │  - Catches issues regex can't                                  │ │
│  └───────────────────────────────────────────────────────────────┘ │
│                              ▲                                      │
│                              │                                      │
│  Level 2: Playwright Visual Regression (Foundation)                │
│  ┌───────────────────────────────────────────────────────────────┐ │
│  │  expect(terminal).toHaveScreenshot('baseline.png')             │ │
│  │  - Pixel-level comparison                                      │ │
│  │  - Cross-browser testing (Chromium, Firefox, WebKit)          │ │
│  │  - Built-in video recording                                    │ │
│  │  - CI integration                                              │ │
│  └───────────────────────────────────────────────────────────────┘ │
│                              ▲                                      │
│                              │                                      │
│  Level 1: Playwright + xterm.js (Infrastructure)                   │
│  ┌───────────────────────────────────────────────────────────────┐ │
│  │  Real terminal emulation in browser                            │ │
│  │  - xterm.js connected to RCT via PTY                          │ │
│  │  - Full keyboard/mouse interaction                             │ │
│  │  - Real ANSI rendering                                         │ │
│  └───────────────────────────────────────────────────────────────┘ │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

### Why Playwright + Claude is SOTA

| Capability | Traditional Testing | Playwright + Claude |
|------------|--------------------|--------------------|
| Visual correctness | Pixel diff (brittle) | "Does it look right?" (intelligent) |
| Dynamic content | Hard to test | Claude understands streaming |
| Edge cases | Must enumerate all | Claude notices unexpected issues |
| Test maintenance | High (selectors break) | Low (Claude adapts) |
| Cross-terminal | Manual setup | Automated matrix |
| Temporal behavior | Not tested | Video analysis (coming) |

### Future: Claude Cowork as Test Driver

When Claude Cowork matures, it can drive entire test suites:

```
User: "Test the RCT terminal application thoroughly"

Cowork:
1. Spawns test environment
2. Executes comprehensive test scenarios
3. Identifies and documents issues
4. Generates test report with screenshots
5. Suggests fixes for failures
```

This is the future of QA - AI that tests like a human but at machine scale.

---

## Phase 1: Foundation Hardening (Weeks 1-4)

### Goal: Fix architecture, add comprehensive tests to existing code

### 1.1 Architectural Cleanup

**Task:** Extract shared types to break circular dependency

```
src/
├── types/           # NEW: Shared types
│   ├── mod.rs
│   ├── message.rs   # Message, Role
│   ├── stream.rs    # StreamEvent
│   └── config.rs    # Config types
├── app/             # Uses types::*
├── api/             # Uses types::*
```

**Tests First:**
```rust
// tests/types_test.rs
#[test]
fn test_message_serialization() {
    let msg = Message { role: Role::User, content: "hello".into() };
    let json = serde_json::to_string(&msg).unwrap();
    let parsed: Message = serde_json::from_str(&json).unwrap();
    assert_eq!(msg.content, parsed.content);
}

#[test]
fn test_role_display() {
    assert_eq!(Role::User.to_string(), "user");
    assert_eq!(Role::Assistant.to_string(), "assistant");
}
```

### 1.2 API Client Tests

**Tests First:**
```rust
// tests/api_client_test.rs
#[tokio::test]
async fn test_stream_message_success() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_string("data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"Hello\"}}\n\n"))
        .mount(&mock_server)
        .await;

    let client = AnthropicClient::new_with_base_url(
        "test-key".into(),
        "claude-sonnet-4-20250514",
        mock_server.uri(),
    );

    let (tx, mut rx) = mpsc::channel(10);
    let messages = vec![Message { role: Role::User, content: "Hi".into() }];

    client.stream_message(&messages, tx).await.unwrap();

    let event = rx.recv().await.unwrap();
    assert!(matches!(event, StreamEvent::ContentDelta(s) if s == "Hello"));
}

#[tokio::test]
async fn test_stream_message_error_handling() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401)
            .set_body_json(json!({"error": {"message": "Invalid API key"}})))
        .mount(&mock_server)
        .await;

    // Test error propagation...
}

#[tokio::test]
async fn test_retry_on_rate_limit() {
    // Test 429 handling with exponential backoff
}
```

### 1.3 State Management Tests

**Tests First:**
```rust
// tests/app_state_test.rs
#[test]
fn test_input_handling() {
    let mut state = AppState::new(PathBuf::from("."));

    state.insert_char('h');
    state.insert_char('i');
    assert_eq!(state.input, "hi");

    state.delete_char();
    assert_eq!(state.input, "h");

    let taken = state.take_input();
    assert_eq!(taken, "h");
    assert!(state.input.is_empty());
}

#[test]
fn test_scroll_bounds() {
    let mut state = AppState::new(PathBuf::from("."));

    state.scroll_up(100);
    assert_eq!(state.scroll_offset, 100);

    state.scroll_down(50);
    assert_eq!(state.scroll_offset, 50);

    state.scroll_down(1000); // Should saturate at 0
    assert_eq!(state.scroll_offset, 0);
}

#[test]
fn test_dirty_flag_tracking() {
    let mut state = AppState::new(PathBuf::from("."));
    state.mark_rendered();

    assert!(!state.needs_render());

    state.insert_char('x');
    assert!(state.needs_render());

    state.mark_rendered();
    assert!(!state.needs_render());
}
```

### 1.4 TUI Snapshot Tests

**Setup:** Add `insta` for snapshot testing

```rust
// tests/tui_snapshot_test.rs
use insta::assert_snapshot;
use ratatui::backend::TestBackend;

#[test]
fn test_empty_state_render() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let state = AppState::new(PathBuf::from("."));

    terminal.draw(|f| tui::render(f, &state)).unwrap();

    assert_snapshot!(terminal.backend().to_string());
}

#[test]
fn test_message_list_render() {
    let mut state = AppState::new(PathBuf::from("."));
    state.messages.push(Message {
        role: Role::User,
        content: "Hello, Claude!".into(),
    });
    state.messages.push(Message {
        role: Role::Assistant,
        content: "Hello! How can I help you today?".into(),
    });

    // Render and snapshot...
}

#[test]
fn test_streaming_response_render() {
    let mut state = AppState::new(PathBuf::from("."));
    state.current_response = Some("Thinking...".into());
    state.loading = true;

    // Verify throbber appears...
}
```

### Phase 1 Deliverables

| Deliverable | Acceptance Criteria |
|-------------|---------------------|
| Types module | No circular dependencies, all tests pass |
| API client tests | 90%+ coverage, mock server tests |
| State tests | 95%+ coverage, property tests for bounds |
| TUI snapshots | Baseline snapshots for regression |
| CI pipeline | Tests run on every PR |

---

## Phase 2: Tool Execution (Weeks 5-8)

### Goal: Full agentic capabilities with security hardening

### 2.1 Tool Executor Tests

**Tests First:**
```rust
// tests/tool_executor_test.rs
#[tokio::test]
async fn test_bash_execution_success() {
    let executor = ToolExecutor::new(tempdir().unwrap().path().to_path_buf());

    let result = executor.execute(ToolCall {
        name: "bash".into(),
        input: json!({"command": "echo 'hello'"}),
    }).await.unwrap();

    assert!(matches!(result, ToolResult::Success(s) if s.contains("hello")));
}

#[tokio::test]
async fn test_bash_blocks_dangerous_commands() {
    let executor = ToolExecutor::new(tempdir().unwrap().path().to_path_buf());

    let result = executor.execute(ToolCall {
        name: "bash".into(),
        input: json!({"command": "rm -rf /"}),
    }).await.unwrap();

    assert!(matches!(result, ToolResult::Error(s) if s.contains("blocked")));
}

#[tokio::test]
async fn test_bash_timeout() {
    let executor = ToolExecutor::new(tempdir().unwrap().path().to_path_buf())
        .with_policy(ToolExecutionPolicy {
            command_timeout: Duration::from_millis(100),
            ..Default::default()
        });

    let result = executor.execute(ToolCall {
        name: "bash".into(),
        input: json!({"command": "sleep 10"}),
    }).await;

    // Should timeout, not hang forever
    assert!(result.is_err() || matches!(result.unwrap(), ToolResult::Error(_)));
}

#[tokio::test]
async fn test_file_read_within_working_dir() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("test.txt"), "content").unwrap();

    let executor = ToolExecutor::new(dir.path().to_path_buf());

    let result = executor.execute(ToolCall {
        name: "read_file".into(),
        input: json!({"path": "test.txt"}),
    }).await.unwrap();

    assert!(matches!(result, ToolResult::Success(s) if s == "content"));
}

#[tokio::test]
async fn test_file_read_blocks_path_traversal() {
    let dir = tempdir().unwrap();
    let executor = ToolExecutor::new(dir.path().to_path_buf());

    let result = executor.execute(ToolCall {
        name: "read_file".into(),
        input: json!({"path": "../../../etc/passwd"}),
    }).await.unwrap();

    assert!(matches!(result, ToolResult::Error(_)));
}
```

### 2.2 Security Hardening

**New Tools to Implement:**

```rust
// src/tools/mod.rs - Extended tool set
pub enum ToolType {
    // File Operations
    Read,           // Read file contents
    Write,          // Write file (with backup)
    Edit,           // Surgical edit (old_string -> new_string)

    // Search
    Glob,           // File pattern matching
    Grep,           // Content search

    // Execution
    Bash,           // Shell commands

    // Git
    GitStatus,      // Status without execution risk
    GitDiff,        // Safe diff viewing
    GitLog,         // History viewing

    // Web
    WebFetch,       // URL content retrieval
    WebSearch,      // Web search (if API available)

    // Notebook
    NotebookEdit,   // Jupyter notebook editing
}
```

**Sandbox Tests:**
```rust
#[tokio::test]
async fn test_sandbox_filesystem_isolation() {
    let sandbox = Sandbox::new(SandboxPolicy::Strict);

    // Should only access working directory
    assert!(sandbox.can_read("/project/src/main.rs"));
    assert!(!sandbox.can_read("/etc/passwd"));
    assert!(!sandbox.can_read("/home/user/.ssh/id_rsa"));
}

#[tokio::test]
async fn test_sandbox_network_restrictions() {
    let sandbox = Sandbox::new(SandboxPolicy::Strict);

    // Should block outbound except allowed hosts
    assert!(!sandbox.can_connect("http://malicious.com"));
    assert!(sandbox.can_connect("https://api.anthropic.com"));
}
```

### 2.3 Edit Tool with Diff Preview

**Tests First:**
```rust
#[tokio::test]
async fn test_edit_generates_diff() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("test.rs"), "fn foo() { }\n").unwrap();

    let executor = ToolExecutor::new(dir.path().to_path_buf());

    let result = executor.execute(ToolCall {
        name: "edit".into(),
        input: json!({
            "file_path": "test.rs",
            "old_string": "fn foo() { }",
            "new_string": "fn foo() {\n    println!(\"hello\");\n}"
        }),
    }).await.unwrap();

    // Verify diff is included in response
    if let ToolResult::Success(output) = result {
        assert!(output.contains("-fn foo() { }"));
        assert!(output.contains("+fn foo() {"));
    }
}

#[tokio::test]
async fn test_edit_creates_backup() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("test.rs");
    std::fs::write(&file_path, "original").unwrap();

    let executor = ToolExecutor::new(dir.path().to_path_buf());

    executor.execute(ToolCall {
        name: "edit".into(),
        input: json!({
            "file_path": "test.rs",
            "old_string": "original",
            "new_string": "modified"
        }),
    }).await.unwrap();

    // Backup should exist in .rct/checkpoints/
    let backups = std::fs::read_dir(dir.path().join(".rct/checkpoints")).unwrap();
    assert!(backups.count() > 0);
}
```

### Phase 2 Deliverables

| Deliverable | Acceptance Criteria |
|-------------|---------------------|
| Full tool set | All Claude Code tools implemented |
| Security sandbox | Path traversal, command injection blocked |
| Checkpoints | Undo/redo for file operations |
| Tool tests | 95%+ coverage, fuzz testing |
| Integration | Tools work in conversation flow |

---

## Phase 3: MCP Protocol Implementation (Weeks 9-12)

### Goal: Full Model Context Protocol support (PROTOCOL ONLY - not specific servers)

**Critical Distinction:** Core implements MCP *protocol*. narsil-mcp is a *plugin* that uses MCP.

```
┌─────────────────────────────────────────────────────────────────────┐
│                         CORE (Phase 3)                              │
│  ┌───────────────────────────────────────────────────────────────┐ │
│  │                    MCP Protocol Client                         │ │
│  │  • JSON-RPC 2.0 implementation                                │ │
│  │  • Stdio transport (spawn processes)                          │ │
│  │  • SSE transport (connect to servers)                         │ │
│  │  • HTTP transport (REST endpoints)                            │ │
│  │  • Tool discovery and invocation                              │ │
│  │  • Resource management                                         │ │
│  │  • Server lifecycle (start/stop/restart)                      │ │
│  └───────────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────────┘
                              ▲
                              │ Uses MCP protocol
                              │
┌─────────────────────────────┴───────────────────────────────────────┐
│                    PLUGINS (Phase 7+)                               │
│  ┌────────────┐  ┌────────────┐  ┌────────────┐  ┌────────────┐   │
│  │ narsil-mcp │  │ filesystem │  │ github     │  │ postgres   │   │
│  │ (90 tools) │  │ (official) │  │ (official) │  │ (official) │   │
│  └────────────┘  └────────────┘  └────────────┘  └────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
```

**What Core Phase 3 Delivers:**
- Any MCP server works out of the box
- Configuration via `.mcp.json`
- Server health monitoring
- Graceful degradation on server failure

### 3.1 MCP Transport Tests

**Tests First:**
```rust
// tests/mcp_test.rs
#[tokio::test]
async fn test_mcp_stdio_initialization() {
    // Use a mock MCP server
    let config = McpServerConfig {
        transport: McpTransport::Stdio {
            command: "echo".into(),
            args: vec![r#"{"jsonrpc":"2.0","result":{"capabilities":{}}}"#.into()],
            env: HashMap::new(),
        },
        enabled: true,
    };

    let mut manager = McpManager::new();
    manager.initialize(hashmap!["test".into() => config]).await.unwrap();

    assert!(manager.servers.contains_key("test"));
}

#[tokio::test]
async fn test_mcp_tool_discovery() {
    let manager = create_mock_mcp_manager_with_tools(vec![
        McpTool {
            name: "read_file".into(),
            description: "Read a file".into(),
            input_schema: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
        },
    ]).await;

    let tools = manager.get_tools();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "read_file");
}

#[tokio::test]
async fn test_mcp_tool_call() {
    let manager = create_mock_mcp_manager().await;

    let result = manager.call_tool("test_tool", json!({"arg": "value"})).await.unwrap();

    assert!(result.is_object());
}

#[tokio::test]
async fn test_mcp_server_crash_recovery() {
    let manager = create_mcp_manager_with_crashing_server().await;

    // Should handle server crash gracefully
    let result = manager.call_tool("test_tool", json!({})).await;
    assert!(result.is_err());

    // Should be able to restart
    manager.restart_server("crashed_server").await.unwrap();
}
```

### 3.2 MCP JSON-RPC Implementation

```rust
// src/mcp/protocol.rs
#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    jsonrpc: String,
    id: String,
    method: String,
    params: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    jsonrpc: String,
    id: String,
    result: Option<serde_json::Value>,
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcError {
    code: i32,
    message: String,
    data: Option<serde_json::Value>,
}
```

**Protocol Tests:**
```rust
#[test]
fn test_jsonrpc_request_serialization() {
    let req = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: "1".into(),
        method: "tools/call".into(),
        params: Some(json!({"name": "test"})),
    };

    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("\"jsonrpc\":\"2.0\""));
}

#[test]
fn test_jsonrpc_response_parsing() {
    let json = r#"{"jsonrpc":"2.0","id":"1","result":{"content":"hello"}}"#;
    let resp: JsonRpcResponse = serde_json::from_str(json).unwrap();

    assert!(resp.error.is_none());
    assert_eq!(resp.result.unwrap()["content"], "hello");
}

#[test]
fn test_jsonrpc_error_parsing() {
    let json = r#"{"jsonrpc":"2.0","id":"1","error":{"code":-32600,"message":"Invalid Request"}}"#;
    let resp: JsonRpcResponse = serde_json::from_str(json).unwrap();

    assert!(resp.result.is_none());
    assert_eq!(resp.error.unwrap().code, -32600);
}
```

### Phase 3 Deliverables

| Deliverable | Acceptance Criteria |
|-------------|---------------------|
| Stdio transport | Process spawning, bidirectional JSON-RPC |
| SSE transport | EventSource connection, reconnection |
| HTTP transport | REST-based MCP calls |
| Tool discovery | Dynamic tool loading from servers |
| Error handling | Graceful degradation, restart capability |

---

## Phase 4: Hooks System (Weeks 13-16)

### Goal: Full lifecycle event hooks with all 11 events

### 4.1 Hook Event Tests

**Tests First:**
```rust
// tests/hooks_test.rs
#[tokio::test]
async fn test_pre_tool_use_hook_blocks() {
    let mut executor = HookExecutor::new();
    executor.register(HookEvent::PreToolUse, vec![
        HookDefinition {
            matcher: Some("Bash".into()),
            hooks: vec![HookCommand {
                hook_type: "command".into(),
                command: "exit 2".into(), // Exit code 2 = block
                timeout_ms: Some(1000),
            }],
        },
    ]);

    let context = HookContext {
        hook_event_name: "PreToolUse".into(),
        session_id: "test".into(),
        tool_name: Some("Bash".into()),
        tool_input: Some(json!({"command": "npm test"})),
        ..Default::default()
    };

    let result = executor.execute(HookEvent::PreToolUse, &context).await.unwrap();

    assert!(matches!(result.decision, HookDecision::Block { .. }));
}

#[tokio::test]
async fn test_post_tool_use_hook_receives_response() {
    let mut executor = HookExecutor::new();
    executor.register(HookEvent::PostToolUse, vec![
        HookDefinition {
            matcher: Some("Edit".into()),
            hooks: vec![HookCommand {
                hook_type: "command".into(),
                command: "jq '.tool_response' > /tmp/last_edit.json".into(),
                timeout_ms: Some(1000),
            }],
        },
    ]);

    let context = HookContext {
        tool_name: Some("Edit".into()),
        tool_response: Some(json!({"success": true, "lines_changed": 5})),
        ..Default::default()
    };

    executor.execute(HookEvent::PostToolUse, &context).await.unwrap();

    // Verify hook received the response
    let saved = std::fs::read_to_string("/tmp/last_edit.json").unwrap();
    assert!(saved.contains("lines_changed"));
}

#[tokio::test]
async fn test_hook_matcher_patterns() {
    let executor = HookExecutor::new();

    // Simple match
    assert!(executor.matches("Bash", "Bash"));

    // Pipe-separated
    assert!(executor.matches("Edit|Write", "Edit"));
    assert!(executor.matches("Edit|Write", "Write"));
    assert!(!executor.matches("Edit|Write", "Bash"));

    // Wildcard
    assert!(executor.matches("*", "AnyTool"));

    // Argument pattern
    assert!(executor.matches("Bash(npm *)", "Bash")); // Tool matches
}

#[tokio::test]
async fn test_hook_timeout() {
    let mut executor = HookExecutor::new();
    executor.register(HookEvent::PreToolUse, vec![
        HookDefinition {
            matcher: None,
            hooks: vec![HookCommand {
                hook_type: "command".into(),
                command: "sleep 10".into(),
                timeout_ms: Some(100),
            }],
        },
    ]);

    let start = Instant::now();
    let result = executor.execute(HookEvent::PreToolUse, &HookContext::default()).await;

    // Should timeout quickly, not wait 10 seconds
    assert!(start.elapsed() < Duration::from_secs(1));
}
```

### 4.2 All 11 Hook Events

```rust
pub enum HookEvent {
    PreToolUse,          // Before tool execution (can block)
    PostToolUse,         // After successful tool execution
    PostToolUseFailure,  // After failed tool execution
    PermissionRequest,   // When permission dialog would show (can auto-allow/deny)
    UserPromptSubmit,    // When user submits a prompt
    SessionStart,        // When session starts
    SessionEnd,          // When session ends
    Notification,        // When Claude sends a notification
    Stop,                // When Claude finishes responding
    SubagentStop,        // When a subagent stops
    PreCompact,          // Before context compaction
}
```

### Phase 4 Deliverables

| Deliverable | Acceptance Criteria |
|-------------|---------------------|
| All 11 events | Each event fires at correct lifecycle point |
| Matcher patterns | Glob, regex, argument matching |
| JSON I/O | Context passed to stdin, stdout parsed |
| Exit codes | 0=continue, 2=block, others=log |
| Timeout handling | No hung hooks |

---

## Phase 5: Skills & Plugins (Weeks 17-20)

### Goal: Full extensibility ecosystem

### 5.1 Skill Engine Tests

**Tests First:**
```rust
// tests/skills_test.rs
#[test]
fn test_skill_md_parsing() {
    let content = r#"---
name: code-review
description: This skill should be used when the user asks to review code or analyze code quality.
---

## Code Review Guidelines

When reviewing code, follow these steps:
1. Check for security vulnerabilities
2. Verify error handling
"#;

    let skill = parse_skill_md(content).unwrap();

    assert_eq!(skill.name, "code-review");
    assert!(skill.description.contains("review code"));
    assert!(skill.instructions.contains("security vulnerabilities"));
}

#[test]
fn test_skill_matching() {
    let mut engine = SkillEngine::new();
    engine.load_from_plugins(vec![
        Skill {
            name: "code-review".into(),
            description: "This skill should be used when the user asks to review code".into(),
            instructions: "Review guidelines...".into(),
        },
        Skill {
            name: "testing".into(),
            description: "This skill should be used when writing or running tests".into(),
            instructions: "Testing guidelines...".into(),
        },
    ]);

    let matches = engine.match_skills("Please review my code for bugs");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].name, "code-review");

    let matches = engine.match_skills("Write some unit tests");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].name, "testing");
}

#[test]
fn test_skill_context_injection() {
    let engine = SkillEngine::new();
    let skills = vec![
        Skill {
            name: "security".into(),
            description: "Security review".into(),
            instructions: "Check for OWASP Top 10...".into(),
        },
    ];

    let context = engine.activate(&skills.iter().collect::<Vec<_>>());

    assert!(context.contains("## Skill: security"));
    assert!(context.contains("OWASP Top 10"));
}
```

### 5.2 Plugin System Tests

**Tests First:**
```rust
// tests/plugins_test.rs
#[test]
fn test_plugin_discovery() {
    let dir = tempdir().unwrap();
    create_mock_plugin(&dir, "my-plugin", r#"{
        "name": "my-plugin",
        "version": "1.0.0",
        "description": "Test plugin"
    }"#);

    let mut registry = PluginRegistry::new();
    registry.load_all(&[dir.path().to_path_buf()]).unwrap();

    assert!(registry.plugins.contains_key("my-plugin"));
}

#[test]
fn test_plugin_command_namespacing() {
    let mut registry = create_registry_with_plugins(vec![
        ("plugin-a", vec!["hello", "goodbye"]),
        ("plugin-b", vec!["hello"]), // Duplicate name
    ]);

    // Should be accessible with namespace
    assert!(registry.get_command("plugin-a:hello").is_some());
    assert!(registry.get_command("plugin-b:hello").is_some());

    // Ambiguous without namespace - should prefer first loaded or error
    let result = registry.get_command("hello");
    // Implementation decision: first match or error?
}

#[test]
fn test_plugin_version_compatibility() {
    let plugin = PluginManifest {
        name: "test".into(),
        version: "1.0.0".into(),
        min_rct_version: Some("2.0.0".into()), // Requires future version
        ..Default::default()
    };

    let result = validate_plugin_compatibility(&plugin, "1.0.0");
    assert!(result.is_err());
}
```

### 5.3 Slash Command Tests

```rust
#[test]
fn test_command_argument_parsing() {
    let cmd = SlashCommand {
        name: "review".into(),
        description: "Review code".into(),
        args: vec![
            CommandArg {
                name: "file".into(),
                arg_type: "path".into(),
                required: true,
                ..Default::default()
            },
            CommandArg {
                name: "strict".into(),
                arg_type: "bool".into(),
                required: false,
                default: Some("false".into()),
                ..Default::default()
            },
        ],
        content: "Review {{ file }} with strict={{ strict }}".into(),
        source_path: PathBuf::new(),
    };

    let result = execute_command(&cmd, &hashmap!["file".into() => "main.rs".into()]).unwrap();

    assert!(result.contains("Review main.rs"));
    assert!(result.contains("strict=false")); // Default applied
}
```

### Phase 5 Deliverables

| Deliverable | Acceptance Criteria |
|-------------|---------------------|
| Skill engine | Context-aware matching, activation |
| Plugin loader | Discovery, validation, loading |
| Command system | Arguments, namespacing, execution |
| SKILL.md parser | Frontmatter + content extraction |
| Plugin registry | Conflict detection, versioning |

---

## Phase 6: Subagents (Weeks 21-24)

### Goal: Parallel task execution with isolated contexts

### 6.1 Subagent Tests

**Tests First:**
```rust
// tests/subagent_test.rs
#[tokio::test]
async fn test_subagent_spawn() {
    let mut orchestrator = SubagentOrchestrator::new();

    let config = SubagentConfig {
        name: "test-agent".into(),
        description: "Test agent".into(),
        system_prompt: "You are a test agent.".into(),
        allowed_tools: vec!["read_file".into()],
        max_turns: 5,
    };

    let id = orchestrator.spawn(config);

    assert!(orchestrator.get_status(id).is_some());
    assert_eq!(orchestrator.get_status(id).unwrap(), "pending");
}

#[tokio::test]
async fn test_subagent_isolation() {
    let mut orchestrator = SubagentOrchestrator::new();

    // Spawn two agents - they should not share context
    let agent1 = orchestrator.spawn(SubagentConfig {
        name: "agent1".into(),
        system_prompt: "You know secret A.".into(),
        ..Default::default()
    });

    let agent2 = orchestrator.spawn(SubagentConfig {
        name: "agent2".into(),
        system_prompt: "You know secret B.".into(),
        ..Default::default()
    });

    // Each agent should only see its own context
    // (Implementation would verify message isolation)
}

#[tokio::test]
async fn test_subagent_tool_restrictions() {
    let mut orchestrator = SubagentOrchestrator::new();

    let id = orchestrator.spawn(SubagentConfig {
        name: "restricted-agent".into(),
        allowed_tools: vec!["read_file".into()], // Only read
        ..Default::default()
    });

    // Agent should not be able to use write_file
    // (Test through actual agent execution)
}

#[tokio::test]
async fn test_subagent_max_turns() {
    let mut orchestrator = SubagentOrchestrator::new();

    let id = orchestrator.spawn(SubagentConfig {
        name: "bounded-agent".into(),
        max_turns: 3,
        ..Default::default()
    });

    let result = orchestrator.run(id).await.unwrap();

    // Agent should stop after max_turns
    assert!(result.turns <= 3);
}

#[tokio::test]
async fn test_parallel_subagent_execution() {
    let mut orchestrator = SubagentOrchestrator::new().with_max_concurrent(4);

    let ids: Vec<_> = (0..4).map(|i| {
        orchestrator.spawn(SubagentConfig {
            name: format!("agent-{}", i),
            ..Default::default()
        })
    }).collect();

    // All should run in parallel
    let start = Instant::now();
    let results = futures::future::join_all(
        ids.iter().map(|id| orchestrator.run(*id))
    ).await;

    // Should complete faster than sequential
    // (Each agent takes ~1s, parallel should be ~1s total, not 4s)
}
```

### Phase 6 Deliverables

| Deliverable | Acceptance Criteria |
|-------------|---------------------|
| Agent spawning | Config-based agent creation |
| Context isolation | Separate message histories |
| Tool restrictions | Per-agent tool allow-lists |
| Concurrency control | Max concurrent limit |
| Result aggregation | Combine agent outputs |

---

## Phase 6.5: Plugin Host API (Weeks 25-26)

### Goal: Stable, documented API for plugins

This is the **CRITICAL GATE** that enables the plugin ecosystem. Must be stable before first-party plugins ship.

### 6.5.1 Plugin Host API Tests

**Tests First:**
```rust
// tests/plugin_api_test.rs

#[test]
fn test_plugin_discovery() {
    let dir = tempdir().unwrap();
    create_mock_plugin(&dir, "test-plugin", r#"{
        "name": "test-plugin",
        "version": "1.0.0",
        "capabilities": ["tools"]
    }"#);

    let registry = PluginRegistry::new(&[dir.path().to_path_buf()]);
    let plugins = registry.discover().unwrap();

    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].name, "test-plugin");
}

#[test]
fn test_plugin_version_compatibility() {
    let manifest = PluginManifest {
        name: "test".into(),
        min_rct_version: Some("99.0.0".into()),  // Future version
        ..Default::default()
    };

    let result = check_compatibility(&manifest, "1.0.0");

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("requires RCT 99.0.0"));
}

#[tokio::test]
async fn test_plugin_lifecycle() {
    let plugin = MockPlugin::new();
    let ctx = create_test_context();

    // Load
    plugin.on_load(&ctx).await.unwrap();
    assert!(plugin.is_loaded());

    // Use
    let tools = plugin.tools();
    assert!(!tools.is_empty());

    // Unload
    plugin.on_unload().await.unwrap();
    assert!(!plugin.is_loaded());
}

#[tokio::test]
async fn test_plugin_isolation() {
    // Plugins should not be able to access other plugins' state
    let plugin_a = MockPlugin::new();
    let plugin_b = MockPlugin::new();
    let ctx = create_test_context();

    plugin_a.on_load(&ctx).await.unwrap();
    plugin_b.on_load(&ctx).await.unwrap();

    // Plugin A's data should not be visible to Plugin B
    plugin_a.set_data("secret", "value");
    assert!(plugin_b.get_data("secret").is_none());
}
```

### 6.5.2 Plugin Host Implementation

```rust
// src/plugins/host.rs

pub struct PluginHost {
    registry: PluginRegistry,
    loaded: HashMap<String, Box<dyn RctPlugin>>,
    tool_index: HashMap<String, String>,  // tool_name -> plugin_name
    command_index: HashMap<String, String>,
}

impl PluginHost {
    /// Load all enabled plugins
    pub async fn load_all(&mut self) -> Result<()> {
        for manifest in self.registry.discover()? {
            if self.should_load(&manifest) {
                self.load_plugin(&manifest).await?;
            }
        }
        Ok(())
    }

    /// Route tool call to appropriate plugin
    pub async fn execute_tool(&self, name: &str, input: Value) -> Result<Value> {
        let plugin_name = self.tool_index.get(name)
            .ok_or_else(|| anyhow!("Unknown tool: {}", name))?;

        let plugin = self.loaded.get(plugin_name)
            .ok_or_else(|| anyhow!("Plugin not loaded: {}", plugin_name))?;

        plugin.as_tool_provider()?.execute(name, input).await
    }
}
```

### 6.5.3 Plugin Documentation Requirements

| Document | Purpose |
|----------|---------|
| `docs/plugin-api.md` | API reference for plugin developers |
| `docs/plugin-tutorial.md` | Step-by-step plugin creation guide |
| `examples/minimal-plugin/` | Minimal working plugin template |

### Phase 6.5 Deliverables

| Deliverable | Acceptance Criteria |
|-------------|---------------------|
| Plugin Host API v1.0 | Stable traits, documented |
| Plugin Discovery | Find plugins in standard locations |
| Plugin Loading | Dynamic load/unload lifecycle |
| Tool Routing | Route tool calls to correct plugin |
| Command Routing | Route slash commands to correct plugin |
| API Documentation | Complete reference + examples |

---

# PLUGIN DEVELOPMENT (Parallel Track)

The following phases can proceed **independently** of core development once the Plugin Host API (Phase 6.5) is stable. These don't block core release.

---

## Plugin Phase A: narsil-mcp Integration Plugin

**Repository:** `rct-plugin-narsil` (separate from core)

### Goal: Deep code intelligence via narsil-mcp

**Plugin Manifest:**
```json
{
  "name": "narsil",
  "version": "1.0.0",
  "description": "Code intelligence powered by narsil-mcp",
  "author": "postrv",
  "capabilities": ["tools", "skills"],
  "dependencies": {
    "narsil-mcp": ">=1.3.0"
  }
}
```

**What the Plugin Provides:**

| Capability | How It Works |
|------------|--------------|
| Auto-spawn narsil-mcp | Plugin starts server on session init |
| 90 tools | Proxied through MCP protocol |
| Skills | Code review, security scan, architecture |
| Commands | `/narsil:explore`, `/narsil:security-scan` |

**Tests First:**
```rust
#[tokio::test]
async fn test_narsil_plugin_loads() {
    let ctx = create_test_plugin_context();
    let plugin = NarsilPlugin::new();

    plugin.on_load(&ctx).await.unwrap();

    assert!(plugin.tools().len() >= 90);
}

#[tokio::test]
async fn test_narsil_graceful_when_unavailable() {
    let ctx = create_test_plugin_context();
    let plugin = NarsilPlugin::new();

    // narsil-mcp not installed
    std::env::set_var("PATH", "/nonexistent");

    let result = plugin.on_load(&ctx).await;

    // Should warn, not fail
    assert!(result.is_ok());
    assert!(plugin.tools().is_empty());
}
```

---

## Plugin Phase B: Ralph Automation Plugin

**Repository:** `rct-plugin-ralph` (separate from core)

### Goal: Autonomous coding with quality gates

**What the Plugin Provides:**

| Capability | Description |
|------------|-------------|
| `/ralph:loop` | Start autonomous execution |
| `/ralph:checkpoint` | Create quality checkpoint |
| `/ralph:rollback` | Rollback to checkpoint |
| Quality Gates | Polyglot gates (Rust, Python, TS, Go, etc.) |
| Supervisor | Chief Wiggum health monitoring |
| Session Persistence | Resume after crash |

**Integration Approach:**

```rust
// The plugin wraps ralph's Rust library
use ralph_core::{LoopManager, QualityGateEnforcer, CheckpointManager};

pub struct RalphPlugin {
    loop_manager: Option<LoopManager>,
    working_dir: PathBuf,
}

impl CommandProvider for RalphPlugin {
    fn commands(&self) -> Vec<SlashCommand> {
        vec![
            SlashCommand::new("ralph:loop", "Start autonomous execution loop"),
            SlashCommand::new("ralph:checkpoint", "Create quality checkpoint"),
            SlashCommand::new("ralph:rollback", "Rollback to last checkpoint"),
            SlashCommand::new("ralph:gates", "Run quality gates"),
        ]
    }

    fn execute(&self, cmd: &str, args: &str) -> Result<String> {
        match cmd {
            "ralph:loop" => self.start_loop(args),
            "ralph:checkpoint" => self.create_checkpoint(args),
            // ...
        }
    }
}
```

**Tests First:**
```rust
#[tokio::test]
async fn test_ralph_quality_gates() {
    let plugin = RalphPlugin::new(tempdir().unwrap().path());

    let result = plugin.execute("ralph:gates", "").await.unwrap();

    // Should return gate results
    assert!(result.contains("ClippyGate") || result.contains("No gates configured"));
}

#[tokio::test]
async fn test_ralph_checkpoint_rollback() {
    let dir = tempdir().unwrap();
    let plugin = RalphPlugin::new(dir.path());

    // Create checkpoint
    plugin.execute("ralph:checkpoint", "test-checkpoint").await.unwrap();

    // Make changes
    std::fs::write(dir.path().join("test.txt"), "changed").unwrap();

    // Rollback
    plugin.execute("ralph:rollback", "test-checkpoint").await.unwrap();

    // File should be gone
    assert!(!dir.path().join("test.txt").exists());
}
```

---

## Plugin Phase C: Git Worktree Plugin

**Repository:** `rct-plugin-worktree`

### Goal: Branch isolation for experiments

**What the Plugin Provides:**

| Command | Description |
|---------|-------------|
| `/worktree:create <branch>` | Create isolated worktree for branch |
| `/worktree:switch <branch>` | Switch context to worktree |
| `/worktree:merge` | Merge worktree back to main |
| `/worktree:list` | List active worktrees |
| `/worktree:cleanup` | Remove stale worktrees |

**Use Case:**
```
User: "Try implementing auth with JWT"

Claude: I'll create a worktree to experiment safely.

/worktree:create feature/jwt-auth

Now working in isolated branch. If this doesn't work out,
we can discard without affecting main.

[... implementation ...]

/worktree:merge
# or
/worktree:cleanup  # Discard experiment
```

---

## Plugin Phase D: Analytics Plugin

**Repository:** `rct-plugin-analytics`

### Goal: Usage insights and optimization

**What the Plugin Provides:**

| Feature | Description |
|---------|-------------|
| Session tracking | Token usage, API calls, tool invocations |
| Performance metrics | Response times, cache hit rates |
| Cost estimation | Estimated API costs per session |
| Reports | Daily/weekly summaries |

**Privacy-First:**
- All data stored locally
- Opt-in for anonymous aggregate sharing
- No PII or conversation content

---

## Plugin Phase E: Enterprise Plugin

**Repository:** `rct-plugin-enterprise` (separate licensing)

### Goal: Enterprise compliance and security

| Feature | Description |
|---------|-------------|
| SSO/SAML | Enterprise authentication |
| Audit logging | Tamper-evident logs |
| Policy enforcement | IT-managed settings |
| Cost controls | Budget limits per team |
| Compliance reports | SOC2, HIPAA exports |

---

# CORE DEVELOPMENT (Continues)

## Phase 7: Beyond Parity - Competitive Differentiation (Weeks 25-32)

### Goal: Features Claude Code doesn't have (but in CORE, not plugins)

### 7.1 Performance Benchmarks (Marketing Differentiator)

```rust
// benches/rendering.rs
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn benchmark_full_redraw(c: &mut Criterion) {
    let state = create_state_with_messages(100);
    let mut buffer = Buffer::empty(Rect::new(0, 0, 120, 40));

    c.bench_function("full_redraw_100_messages", |b| {
        b.iter(|| {
            render_message_list(black_box(&mut buffer), black_box(&state.messages));
        })
    });
}

fn benchmark_streaming_append(c: &mut Criterion) {
    let mut state = create_state_with_messages(10);

    c.bench_function("streaming_token_append", |b| {
        b.iter(|| {
            state.append_chunk(black_box(StreamEvent::ContentDelta("token ".into()))).unwrap();
        })
    });
}

fn benchmark_input_latency(c: &mut Criterion) {
    let mut state = AppState::new(PathBuf::from("."));

    c.bench_function("input_character_echo", |b| {
        b.iter(|| {
            state.insert_char(black_box('x'));
        })
    });
}

// Target benchmarks:
// - full_redraw: < 1ms
// - streaming_append: < 100μs
// - input_echo: < 10μs
```

### 7.2 Unique Features

#### 7.2.1 Native Git Worktree Support

```rust
// Feature: Branch isolation with git worktrees
pub struct WorktreeManager {
    pub fn create_for_task(&self, task_name: &str) -> Result<Worktree>;
    pub fn switch_to(&self, worktree: &Worktree) -> Result<()>;
    pub fn merge_back(&self, worktree: &Worktree) -> Result<()>;
}
```

#### 7.2.2 Session Persistence & Resume

```rust
// Feature: Resume sessions across restarts
pub struct SessionManager {
    pub fn save(&self, session: &Session) -> Result<SessionId>;
    pub fn load(&self, id: SessionId) -> Result<Session>;
    pub fn list_recent(&self) -> Vec<SessionSummary>;
}

#[tokio::test]
async fn test_session_resume() {
    let manager = SessionManager::new();

    let session = Session::new();
    session.add_message(Message { role: Role::User, content: "Hello".into() });

    let id = manager.save(&session).await.unwrap();

    // Simulate restart
    let restored = manager.load(id).await.unwrap();
    assert_eq!(restored.messages.len(), 1);
}
```

#### 7.2.3 Semantic Code Search (Embeddings)

```rust
// Feature: Find code by meaning, not just text
pub struct CodeIndexer {
    pub async fn index_codebase(&self, root: &Path) -> Result<CodeIndex>;
    pub async fn search(&self, query: &str, limit: usize) -> Vec<CodeMatch>;
}

#[tokio::test]
async fn test_semantic_search() {
    let indexer = CodeIndexer::new();
    indexer.index_codebase(Path::new("./src")).await.unwrap();

    let results = indexer.search("function that validates user input", 5).await;

    // Should find validation functions even without exact keyword match
    assert!(results.iter().any(|r| r.file.contains("validation")));
}
```

#### 7.2.4 Multi-Model Support

```rust
// Feature: Switch between Claude models and providers
pub enum ModelProvider {
    Anthropic { api_key: SecretString },
    Bedrock { region: String, profile: Option<String> },
    VertexAI { project: String, location: String },
    Ollama { base_url: String },  // Local models!
}

#[tokio::test]
async fn test_model_switching() {
    let mut client = MultiModelClient::new();

    client.set_provider(ModelProvider::Anthropic {
        api_key: "<test-placeholder>".into()
    });

    // Switch mid-session
    client.set_provider(ModelProvider::Ollama {
        base_url: "http://localhost:11434".into()
    });

    // Should work with local models for offline use
}
```

#### 7.2.5 Collaborative Sessions

```rust
// Feature: Multiple users in same session
pub struct CollaborativeSession {
    pub fn share(&self) -> ShareLink;
    pub fn join(&self, link: ShareLink) -> Result<Session>;
    pub fn broadcast(&self, message: Message);
}
```

### 7.3 Enterprise Features

#### 7.3.1 Audit Logging

```rust
pub struct AuditLogger {
    pub fn log_tool_execution(&self, tool: &str, input: &Value, output: &Value);
    pub fn log_file_access(&self, path: &Path, operation: FileOp);
    pub fn export(&self, format: ExportFormat) -> Vec<u8>;
}

#[test]
fn test_audit_log_compliance() {
    let logger = AuditLogger::new();

    logger.log_tool_execution("Bash", &json!({"command": "ls"}), &json!({"output": "..."}));

    let log = logger.export(ExportFormat::Json);
    let entries: Vec<AuditEntry> = serde_json::from_slice(&log).unwrap();

    assert!(entries[0].timestamp.is_some());
    assert!(entries[0].user.is_some());
    assert!(entries[0].action.is_some());
}
```

#### 7.3.2 SSO/SAML Integration

```rust
pub struct EnterpriseAuth {
    pub async fn authenticate_saml(&self, assertion: &str) -> Result<User>;
    pub async fn authenticate_oidc(&self, token: &str) -> Result<User>;
}
```

#### 7.3.3 Rate Limiting & Cost Controls

```rust
pub struct CostController {
    pub fn set_budget(&mut self, daily_limit: Decimal);
    pub fn check_budget(&self) -> BudgetStatus;
    pub fn estimate_cost(&self, tokens: usize) -> Decimal;
}
```

### Phase 7 Deliverables

| Deliverable | Differentiation Value |
|-------------|----------------------|
| Git worktrees | Branch isolation for experiments |
| Session persistence | Resume work across restarts |
| Semantic search | Find code by meaning |
| Multi-model | Local models, provider switching |
| Collaborative sessions | Team pair programming |
| Audit logging | Enterprise compliance |
| SSO integration | Enterprise security |
| Cost controls | Enterprise budget management |

---

## Phase 8: Polish & Release (Weeks 33-36)

### 8.1 Documentation

- API documentation with examples
- User guide with tutorials
- Contributing guide
- Security policy

### 8.2 Distribution

| Platform | Method | Priority |
|----------|--------|----------|
| macOS | Homebrew, direct download | P0 |
| Linux | apt, dnf, direct download | P0 |
| Windows | WinGet, Scoop, direct download | P1 |
| Docker | Official image | P2 |

### 8.3 Auto-Update System

```rust
#[tokio::test]
async fn test_auto_update_check() {
    let updater = UpdateManager::new();

    let update = updater.check_for_updates().await.unwrap();

    if let Some(release) = update {
        assert!(release.version > current_version());
        assert!(release.platforms.contains_key("darwin-aarch64"));
    }
}

#[tokio::test]
async fn test_auto_update_verify_signature() {
    let updater = UpdateManager::new();

    let release = create_mock_release_with_invalid_signature();

    let result = updater.install_update(&release).await;
    assert!(result.is_err()); // Should reject invalid signature
}
```

---

## Success Metrics

### Technical Metrics

| Metric | Target | Measurement |
|--------|--------|-------------|
| Test coverage | >90% | `cargo tarpaulin` |
| Frame time | <1ms | Criterion benchmarks |
| Memory (idle) | <50MB | `heaptrack` |
| Startup time | <500ms | Wall clock |
| Binary size | <20MB | `cargo build --release` |

### Product Metrics

| Metric | Target | Measurement |
|--------|--------|-------------|
| Feature parity | 100% | Feature matrix comparison |
| GitHub stars | 10,000+ | First 6 months |
| Active users | 50,000+ | Telemetry (opt-in) |
| Enterprise customers | 10+ | Sales |

### Acquisition Readiness

| Criterion | Status |
|-----------|--------|
| Clean IP (no copyleft dependencies) | TBD |
| Comprehensive test suite | TBD |
| Security audit | TBD |
| Performance benchmarks vs Claude Code | TBD |
| Enterprise features | TBD |
| Documentation | TBD |

---

## Strategic Synergy: RCT + Plugin Ecosystem

### The Acquisition Package

Anthropic would acquire a modular, production-ready AI coding platform:

```
┌─────────────────────────────────────────────────────────────────────┐
│                    postrv AI Coding Ecosystem                       │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ┌───────────────────────────────────────────────────────────────┐ │
│  │                    RCT CORE (Priority 1)                       │ │
│  │  Full Claude Code feature parity + 16x performance             │ │
│  │  Single binary, no Node.js, production-ready                   │ │
│  │  ┌─────────────┐ ┌─────────────┐ ┌─────────────────────────┐  │ │
│  │  │ TUI/Events  │ │ API Client  │ │ Plugin Host API         │  │ │
│  │  └─────────────┘ └─────────────┘ └─────────────────────────┘  │ │
│  └───────────────────────────────────────────────────────────────┘ │
│                              │                                      │
│                     Plugin Interface (Stable)                       │
│                              │                                      │
│  ┌───────────────────────────┴───────────────────────────────────┐ │
│  │                 FIRST-PARTY PLUGINS (Priority 2)               │ │
│  │  Ship independently, don't block core release                  │ │
│  │  ┌────────────┐ ┌────────────┐ ┌────────────┐ ┌────────────┐  │ │
│  │  │  narsil    │ │   ralph    │ │  worktree  │ │ enterprise │  │ │
│  │  │ 90 tools   │ │ automation │ │  git iso   │ │ SSO/audit  │  │ │
│  │  │ code intel │ │ quality    │ │  branches  │ │ compliance │  │ │
│  │  └────────────┘ └────────────┘ └────────────┘ └────────────┘  │ │
│  └───────────────────────────────────────────────────────────────┘ │
│                                                                     │
│  ┌───────────────────────────────────────────────────────────────┐ │
│  │              COMMUNITY PLUGINS (Priority 3)                    │ │
│  │  Ecosystem growth, community contributions                     │ │
│  └───────────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────────┘
```

### Why This Architecture Wins

| Aspect | Monolithic Approach | Our Plugin Architecture |
|--------|--------------------|-----------------------|
| **Time to Market** | Blocked by all features | Core ships fast |
| **Stability** | One bug breaks everything | Plugins isolated |
| **Maintenance** | One huge codebase | Focused repos |
| **Community** | Hard to contribute | Easy plugin contributions |
| **Enterprise** | All or nothing | Pick plugins you need |
| **Acquisition** | Complex dependencies | Clean boundaries |

### Combined Value Proposition

| Component | Standalone Value | Combined Value |
|-----------|------------------|----------------|
| **RCT** | Fast Claude CLI | Native AI coding assistant |
| **narsil-mcp** | Code intel server | Built-in code understanding |
| **Ralph** | Automation suite | Autonomous development agent |

### Competitive Moat

| Capability | Claude Code | RCT + narsil-mcp |
|------------|-------------|------------------|
| Runtime | Node.js (GC pauses) | Native Rust (zero overhead) |
| Code Intelligence | External MCP servers | Built-in (narsil-core library) |
| Security Scanning | Manual tools | 111 built-in rules (OWASP, CWE) |
| Neural Code Search | Not built-in | Native embeddings support |
| Type Inference | Requires LSP | Built-in for Python/JS/TS |
| Call Graph Analysis | Not available | Native call/control/data flow |
| SBOM Generation | Manual | One command (CycloneDX/SPDX) |
| Languages | LSP-dependent | 32 languages tree-sitter |
| Binary Size | ~200MB+ (Node) | ~50MB (single binary) |
| Startup Time | 2-3s | <500ms |

### Enterprise Differentiators

The combined package offers enterprise features Claude Code lacks:

1. **Compliance-Ready Security**
   - Built-in OWASP Top 10 scanning
   - CWE Top 25 vulnerability detection
   - SBOM generation for audits
   - License compliance checking

2. **Supply Chain Security**
   - Dependency vulnerability scanning (OSV database)
   - Transitive dependency analysis
   - Safe upgrade path recommendations

3. **Audit Logging**
   - Every tool execution logged
   - Every file access tracked
   - Export to SIEM systems

4. **Air-Gapped Deployment**
   - No Node.js/npm required
   - Single binary distribution
   - Offline ONNX model support

### Acquisition Valuation Factors

| Factor | Impact | Evidence |
|--------|--------|----------|
| **Technical Moat** | High | Rust performance, 90 tools, 32 languages |
| **IP Portfolio** | High | MIT/Apache-2.0 clean, no copyleft |
| **Community** | Medium | GitHub stars, active contributors |
| **Enterprise Traction** | High | Security/compliance features |
| **Strategic Fit** | Very High | Direct replacement for Claude Code |
| **Team Capability** | High | Shipped 3 production Rust projects |

### Integration Path for Anthropic

**Phase 1: Technology Validation**
- Benchmark RCT vs Claude Code
- Security audit of narsil-mcp
- Code review by Anthropic engineers

**Phase 2: Parallel Operation**
- Ship RCT as "Claude Code Rust Edition"
- Gradual migration of power users
- Collect telemetry on performance gains

**Phase 3: Full Transition**
- Deprecate Node.js Claude Code
- RCT becomes primary CLI
- narsil-mcp tools integrated into Claude's native capabilities

### Suggested Deal Structure

| Component | Valuation Basis |
|-----------|-----------------|
| RCT codebase | Engineering cost to rebuild + opportunity cost |
| narsil-mcp | 90 tools × eng-months + security rules IP |
| Team | Retention packages for key engineers |
| Community | GitHub stars, Discord members, enterprise customers |
| IP + Documentation | Clean licensing, comprehensive docs |

**Target: "A few milli" = $X-XX million range**

Comparable acquisitions:
- Vercel acquired Turbo (Rust toolchain) - undisclosed but significant
- Figma acquired Diagram (AI design) - team + tech acquisition
- GitHub acquired Semmle (code analysis) - $XX million range

---

## Appendix: Test Infrastructure

### CI/CD Pipeline

```yaml
# .github/workflows/ci.yml
name: CI

on: [push, pull_request]

jobs:
  test:
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test --all-features
      - run: cargo clippy -- -D warnings
      - run: cargo fmt -- --check

  coverage:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo install cargo-tarpaulin
      - run: cargo tarpaulin --out Xml
      - uses: codecov/codecov-action@v3

  bench:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo bench --no-run

  security:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo install cargo-audit
      - run: cargo audit
```

### Required Dev Dependencies

```toml
[dev-dependencies]
# Testing
tokio-test = "0.4"
pretty_assertions = "1.4"
tempfile = "3.14"
mockall = "0.12"
wiremock = "0.5"  # HTTP mocking

# Snapshot testing
insta = { version = "1.34", features = ["yaml"] }

# Property testing
proptest = "1.4"

# Benchmarking
criterion = { version = "0.5", features = ["async_tokio"] }

# Fuzzing (optional, separate crate)
# cargo-fuzz integration
```

---

*This TDD Production Plan provides a structured path from current state to feature parity and beyond. Each phase builds on the previous, with comprehensive tests ensuring quality and preventing regression.*

*Total estimated timeline: 36 weeks (9 months) for solo developer, faster with team.*
