# Implementation Plan

> Ralph uses this file to track task progress. Update checkboxes as work completes.

## Status: READY

## Global Quality Gates

Before EVERY commit:
```bash
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Before marking ANY task `[x]`:
- All quality gates pass
- No forbidden patterns (`todo!()`, `unimplemented!()`, `#[allow(dead_code)]`)
- New code has test coverage
- Public functions have doc comments with `# Errors` / `# Panics`

## Baseline Metrics

- Tests: 1186+ (must never decrease)
- Clippy warnings: 0 (must stay 0)
- Unsafe blocks: 0 (must stay 0)

---

## Sprint 1: Event Loop Refactoring (Priority 0)

**Goal**: Decompose the monolithic `event_loop()` (CC=25) into a message-passing architecture with isolated, testable event handlers. Use the strangler fig pattern: new handlers run alongside existing code, verified with tests, then old code is removed.

**Success Criteria**:
- `event_loop()` reduced to <30 lines (recv + dispatch)
- Each handler is independently unit-testable
- All 1186+ existing tests still pass
- No behavioral changes — pure refactor

**Risk Mitigation**:
- Never delete old code until new handler is proven equivalent
- Each phase is a self-contained commit with all tests green
- Extract simplest concerns first (throbber → session → background → keyboard)
- Run full test suite after every extraction

---

### 1. Phase 1.1: Define AppEvent Enum

**Description**: Create the unified `AppEvent` enum that all event sources will produce. This is a pure addition — no existing code changes.

**TDD Cycle**:
- [x] RED: Write tests for `AppEvent` variants — construction, matching, `Display` impl, `From` conversions for `CrosstermEvent`, `StreamEvent`, `BackgroundEvent`
- [x] GREEN: Implement `AppEvent` enum in `src/app/events.rs` with all variants: `Key`, `Mouse`, `Resize`, `ApiChunk`, `ToolResult`, `Tick`, `Quit`, `PermissionResponse`
- [x] REFACTOR: Add `#[must_use]` annotations, doc comments with `# Examples`, ensure `Send + Sync`
- Files: `src/app/events.rs` (new), `src/app/mod.rs` (add `mod events`)
- Acceptance: `cargo test` passes, new file compiles, `AppEvent` is public, all variants exercised in tests

---

### 2. Phase 1.2: Define AppContext Struct

**Description**: Extract shared context from `event_loop()` parameters into an `AppContext` struct. This bundles `terminal`, `client`, `state`, `session_manager` into a single borrow-friendly structure. Pure addition.

**TDD Cycle**:
- [x] RED: Write tests for `AppContext` construction, accessor methods, `needs_render()` delegation, `mark_rendered()` delegation
- [x] GREEN: Implement `AppContext` struct in `src/app/context.rs` wrapping terminal, client, state, session_manager references
- [x] REFACTOR: Add convenience methods that delegate to `state` — `needs_render()`, `is_loading()`, `has_background_work()`, `has_pending_permission()`
- Files: `src/app/context.rs` (new), `src/app/mod.rs` (add `mod context`)
- Acceptance: `cargo test` passes, `AppContext` is constructible from existing `event_loop` parameters

---

### 3. Phase 1.3: Define EventHandler Trait and EventDispatcher

**Description**: Create the `EventHandler` trait and `EventDispatcher` that will orchestrate handler execution. Handlers return `Handled(bool)` to indicate consumption. Pure addition.

**TDD Cycle**:
- [x] RED: Write tests for `EventDispatcher` — dispatching to multiple handlers, first-handler-wins semantics, empty dispatcher, handler ordering
- [x] GREEN: Implement `EventHandler` trait with `async fn handle(&mut self, event: &AppEvent, ctx: &mut AppContext) -> Result<Handled>` and `EventDispatcher` struct with `dispatch()` method
- [x] REFACTOR: Add `EventDispatcher::new(handlers)` builder, ensure handler vec is ordered, add `#[must_use]` on `Handled`
- Files: `src/app/dispatch.rs` (new), `src/app/mod.rs` (add `mod dispatch`)
- Acceptance: `EventDispatcher` can be constructed with mock handlers, dispatch calls handlers in order, stops on first `Handled(true)`

---

### 4. Phase 1.4: Create Event Unification Layer

**Description**: Create `recv_event()` on `AppContext` that merges all event sources (crossterm, background channels, tick timer) into a single `AppEvent` stream. This replaces the `tokio::select!` branching with a single receive point. Pure addition — the old `tokio::select!` remains untouched.

**TDD Cycle**:
- [x] RED: Write tests for `recv_event()` — returns `Key` events from crossterm, `ApiChunk` from streaming channel, `Tick` when loading, `Quit` on Ctrl+C
- [x] GREEN: Implement `recv_event()` using internal `tokio::select!` that maps each source to `AppEvent` variants. Keep `biased` for keyboard priority
- [x] REFACTOR: Ensure guard conditions match existing ones (`if state.has_background_work()`, `if state.is_loading()`)
- Files: `src/app/context.rs`
- Acceptance: `recv_event()` produces correct `AppEvent` for each source, tests cover all branches

---

### 5. Phase 1.5: Extract TickHandler (Simplest Handler)

**Description**: Extract throbber animation tick handling into a standalone `TickHandler`. This is the simplest concern (1 line of logic) — ideal first extraction to validate the pattern.

**TDD Cycle**:
- [x] RED: Write tests for `TickHandler` — `handle(Tick)` calls `state.tick_throbber()`, ignores non-Tick events, returns `Handled(true)` only for `Tick`
- [x] GREEN: Implement `TickHandler` struct implementing `EventHandler`
- [x] REFACTOR: Wire `TickHandler` into a test `EventDispatcher`, verify it works in isolation
- Files: `src/app/handlers/tick.rs` (new), `src/app/handlers/mod.rs` (new)
- Acceptance: `TickHandler` passes all tests, throbber animation behavior identical to current code

---

### 6. Phase 1.6: Extract SessionHandler

**Description**: Extract session auto-save logic into `SessionHandler`. Currently auto-save is called inline after message submission, message completion, and before exit.

**TDD Cycle**:
- [x] RED: Write tests for `SessionHandler` — auto-saves on `MessageComplete` event, creates session if none exists, updates existing session, handles save errors gracefully (log, don't crash)
- [x] GREEN: Implement `SessionHandler` that listens for relevant events and calls session save
- [x] REFACTOR: Consolidate the 3 separate `auto_save_session()` call sites into handler logic
- Files: `src/app/handlers/session.rs` (new)
- Acceptance: Session persistence behavior unchanged, auto-save fires at same trigger points

---

### 7. Phase 1.7: Extract StreamHandler (Background API Events)

**Description**: Extract API chunk processing and tool result handling from Branch 2 of the `tokio::select!` into `StreamHandler`. This handles `AppEvent::ApiChunk` and `AppEvent::ToolResult`.

**TDD Cycle**:
- [x] RED: Write tests for `StreamHandler` — processes `ApiChunk` by calling `state.append_chunk()`, detects message completion, detects tool_use stop reason and triggers tool execution, processes `ToolResult` by calling `state.record_tool_result()`, detects all-tools-complete and triggers continuation
- [x] GREEN: Implement `StreamHandler` with the logic currently in `event_loop` lines 818-861 plus `start_tool_execution()` and `finish_tool_execution_and_continue()`
- [x] REFACTOR: Move `start_tool_execution()` and `finish_tool_execution_and_continue()` helper functions to be methods on `StreamHandler`
- Files: `src/app/handlers/stream.rs` (new)
- Acceptance: All streaming and tool continuation behavior identical, existing integration tests pass

---

### 8. Phase 1.8: Extract PermissionHandler

**Description**: Extract permission prompt handling from the keyboard input branch into `PermissionHandler`. Currently permission key events are processed inline when `state.has_pending_permission()` is true.

**TDD Cycle**:
- [ ] RED: Write tests for `PermissionHandler` — intercepts `Key` events when permission is pending, converts key to response (y/n/a), calls `state.handle_permission_response()`, triggers tool execution on approval, calls `state.deny_all_tools()` on denial, ignores events when no permission pending
- [ ] GREEN: Implement `PermissionHandler` using logic from `event_loop` lines 529-548
- [ ] REFACTOR: Ensure `PermissionHandler` runs before `KeyboardHandler` in dispatch order (consumes events when permission modal is active)
- Files: `src/app/handlers/permission.rs` (new)
- Acceptance: Permission prompt UX unchanged, approve/deny behavior identical

---

### 9. Phase 1.9: Extract KeyboardHandler (Largest Handler)

**Description**: Extract all keyboard and mouse input handling into `KeyboardHandler`. This is the largest extraction (lines 520-813) and covers: text input, submit, scroll, copy/paste, selection, slash commands.

**TDD Cycle**:
- [ ] RED: Write tests for `KeyboardHandler` — Ctrl+C/D quits, Enter submits message, character input calls `insert_char`, scroll keys delegate to scroll methods, Cmd+C copies, Cmd+V pastes, mouse click/drag/scroll handled, Escape clears selection
- [ ] GREEN: Implement `KeyboardHandler` by moving the keyboard/mouse match arms from `event_loop`
- [ ] REFACTOR: Group related key handlers into private helper methods: `handle_submit()`, `handle_scroll()`, `handle_copy()`, `handle_paste()`, `handle_selection()`, `handle_mouse()`
- Files: `src/app/handlers/keyboard.rs` (new)
- Acceptance: All keyboard/mouse interaction identical, all existing tests pass including TUI snapshot tests

---

### 10. Phase 1.10: Wire Dispatcher into Event Loop (Switchover)

**Description**: Replace the monolithic `tokio::select!` with the unified `recv_event()` + `EventDispatcher`. This is the critical integration step. Use a feature flag or runtime toggle so the old path can be restored if anything breaks.

**TDD Cycle**:
- [ ] RED: Write integration test that exercises the full dispatched event loop — sends a user message, receives streaming response, handles tool execution, auto-saves session
- [ ] GREEN: Rewrite `event_loop()` to use `AppContext::recv_event()` + `EventDispatcher::dispatch()` with all handlers registered in correct order: `PermissionHandler` > `KeyboardHandler` > `StreamHandler` > `TickHandler` > `SessionHandler`
- [ ] REFACTOR: Remove the old `tokio::select!` code, remove helper functions that are now handler methods, clean up dead code. Run `cargo clippy`, `cargo test`, verify zero warnings
- Files: `src/app/mod.rs`
- Acceptance: `event_loop()` is <30 lines, all 1186+ tests pass, `cargo clippy` clean, no behavioral changes

---

### 11. Phase 1.11: Remove Dead Code and Finalize

**Description**: Clean up any remaining dead code from the extraction. Remove unused helper functions, consolidate imports, run narsil `find_dead_code` and `find_unused_exports`.

**TDD Cycle**:
- [ ] RED: Run `find_dead_code` and `find_unused_exports` on `src/app/` — document any findings as test assertions
- [ ] GREEN: Remove all dead code identified, fix any broken references
- [ ] REFACTOR: Final pass — ensure all new modules have doc comments, all public types have `#[must_use]` where appropriate, run full quality gates
- Files: `src/app/mod.rs`, `src/app/state.rs`, all handler files
- Acceptance: Zero dead code in `src/app/`, all quality gates pass, narsil security scan clean

---

## Sprint 2: Provider Abstraction & OpenRouter (Priority 1)

**Goal**: Create an `LlmProvider` trait abstracting over API providers, wrap existing `AnthropicClient` as a provider, then implement `OpenRouterProvider` with OpenAI message format translation.

**Success Criteria**:
- `AnthropicClient` usage fully behind `LlmProvider` trait
- OpenRouter provider with message + streaming format translation
- Fallback chain for automatic provider failover
- Config supports provider selection

**Dependency**: Sprint 1 complete (StreamHandler is provider-agnostic)

---

### 12. Phase 2.1: Define LlmProvider Trait

**Description**: Define the provider abstraction trait. Supports streaming messages with tools, model info, and provider identity.

**TDD Cycle**:
- [ ] RED: Write tests for trait contract — mock provider implements `stream_message()`, returns expected `StreamEvent`s via channel, `name()` and `model()` return correct values
- [ ] GREEN: Define `LlmProvider` trait in `src/api/provider.rs` with `stream_message()`, `name()`, `model()` methods
- [ ] REFACTOR: Add associated types or generics if needed for tool choice, ensure `Send + Sync + 'static` bounds, doc comments with usage examples
- Files: `src/api/provider.rs` (new), `src/api/mod.rs` (add `mod provider`)
- Acceptance: Trait compiles, mock provider passes contract tests

---

### 13. Phase 2.2: Wrap AnthropicClient as AnthropicProvider

**Description**: Implement `LlmProvider` for `AnthropicClient` (or a thin wrapper). This is a non-breaking change — all existing call sites continue to work via the trait.

**TDD Cycle**:
- [ ] RED: Write tests verifying `AnthropicProvider` implements `LlmProvider`, `stream_message()` delegates to existing `stream_message_v2_with_tools()`, produces same `StreamEvent` sequence
- [ ] GREEN: Implement `AnthropicProvider` wrapping `AnthropicClient`, delegate `stream_message()` to existing streaming method
- [ ] REFACTOR: Update `AppContext` / `AppState` to hold `Box<dyn LlmProvider>` instead of direct `AnthropicClient` reference. Update all call sites
- Files: `src/api/provider.rs`, `src/api/mod.rs`, `src/app/mod.rs`, `src/app/state.rs`
- Acceptance: All existing tests pass with zero behavioral changes, client accessed only via trait

---

### 14. Phase 2.3: Define OpenAI Message Types

**Description**: Define the OpenAI/OpenRouter message format types for serialization/deserialization. These are distinct from Anthropic types.

**TDD Cycle**:
- [ ] RED: Write serde round-trip tests for `OpenAiMessage`, `OpenAiFunctionCall`, `OpenAiStreamChunk`, `OpenAiDelta` — verify serialization matches OpenAI API spec
- [ ] GREEN: Implement types in `src/api/providers/openai_types.rs` with serde derives
- [ ] REFACTOR: Add `From` conversions where appropriate, doc comments referencing OpenAI API docs
- Files: `src/api/providers/openai_types.rs` (new), `src/api/providers/mod.rs` (new)
- Acceptance: Types serialize/deserialize correctly, round-trip tests pass

---

### 15. Phase 2.4: Implement Message Format Translation

**Description**: Implement bidirectional translation between Anthropic message format and OpenAI message format. This is the core complexity of OpenRouter support.

**TDD Cycle**:
- [ ] RED: Write tests for `translate_to_openai()` — user messages, assistant messages, multi-turn conversations, tool_use blocks become function_call, tool_result blocks become function responses, system prompt handling, image content handling
- [ ] GREEN: Implement `translate_to_openai()` and `translate_tool_use()` in `src/api/providers/openrouter.rs`
- [ ] REFACTOR: Write tests for `translate_stream_event()` — OpenAI SSE chunks mapped to `StreamEvent` variants (text delta, tool call delta, stop reason). Implement and verify
- Files: `src/api/providers/openrouter.rs` (new)
- Acceptance: Bidirectional translation produces correct format, all edge cases tested (empty content, multiple tool calls, streaming partial JSON)

---

### 16. Phase 2.5: Implement OpenRouterProvider

**Description**: Full `LlmProvider` implementation for OpenRouter. Handles HTTPS request building, SSE streaming with OpenAI format, and translation to Patina's `StreamEvent`.

**TDD Cycle**:
- [ ] RED: Write integration tests with mock HTTP server — sends OpenAI-format request, receives OpenAI-format SSE stream, produces correct `StreamEvent` sequence via channel
- [ ] GREEN: Implement `OpenRouterProvider` with `stream_message()` that translates messages, makes HTTP request, parses OpenAI SSE format, translates back to `StreamEvent`
- [ ] REFACTOR: Add OpenRouter-specific headers (`HTTP-Referer`, `X-Title`), error handling for rate limits, model validation
- Files: `src/api/providers/openrouter.rs`
- Acceptance: Mock integration tests pass, provider produces identical `StreamEvent` sequence as `AnthropicProvider` for equivalent responses

---

### 17. Phase 2.6: Provider Config and Selection

**Description**: Extend `Config` to support provider selection. Add `[provider]` section to config with per-provider settings.

**TDD Cycle**:
- [ ] RED: Write tests for config parsing — `default` provider, per-provider API keys (as `SecretString`), model selection, OpenRouter-specific fields (`site_url`, `app_name`)
- [ ] GREEN: Add `ProviderConfig` to `src/types/config.rs`, parse from config file, construct appropriate `Box<dyn LlmProvider>` based on selection
- [ ] REFACTOR: Add config validation — warn on missing API keys, validate model names, default to Anthropic if no provider section
- Files: `src/types/config.rs`, `src/main.rs` (provider construction)
- Acceptance: Config file with `[provider.openrouter]` section correctly creates `OpenRouterProvider`

---

### 18. Phase 2.7: Implement FallbackProvider

**Description**: Provider that tries multiple providers in order, falling back on failure. Wraps a `Vec<Box<dyn LlmProvider>>`.

**TDD Cycle**:
- [ ] RED: Write tests for fallback behavior — first provider succeeds (no fallback), first fails then second succeeds, all fail returns error, logging on fallback
- [ ] GREEN: Implement `FallbackProvider` implementing `LlmProvider`, iterates providers, catches errors, tries next
- [ ] REFACTOR: Add config support for fallback chains (`fallback = ["anthropic", "openrouter"]`), emit warnings on fallback
- Files: `src/api/provider.rs`
- Acceptance: Fallback chain works correctly, tests cover all permutations

---

## Sprint 3: Context Compression Wiring (Priority 2)

**Goal**: Wire the existing `CompressionOrchestrator` scaffolding to live MCP calls. Replace closure-based `get_context()` with actual `McpClient::call_tool()` invocations.

**Success Criteria**:
- Live MCP calls to narsil for manifest, architecture, symbols
- Git commit hash-based cache invalidation
- Token-budgeted context building
- Injected into message-sending path before API call

**Dependency**: Sprint 1 complete (handler architecture)

---

### 19. Phase 3.1: Wire Manifest Fetch to Live MCP

**Description**: Replace the closure-based manifest fetch with actual `McpClient::call_tool("get_project_structure", ...)` call. Add git hash cache invalidation.

**TDD Cycle**:
- [ ] RED: Write tests for `fetch_manifest()` — calls MCP with correct tool name and args, parses response, caches result keyed by commit hash, returns cached on hash match, re-fetches on hash change
- [ ] GREEN: Implement `fetch_manifest()` in `CompressionOrchestrator` using `McpClient::call_tool()`, parse response into `CcgManifest`, cache with git commit hash
- [ ] REFACTOR: Add error handling for MCP connection failure (graceful degradation — return empty context, log warning)
- Files: `src/context/compression/orchestrator.rs`
- Acceptance: Live MCP call works with running narsil instance, cache invalidation triggers on new commits

---

### 20. Phase 3.2: Wire Architecture Fetch to Live MCP

**Description**: Wire architecture/import graph fetch to `McpClient::call_tool("get_import_graph", ...)` and `get_dependencies`.

**TDD Cycle**:
- [ ] RED: Write tests for `fetch_architecture()` — calls correct MCP tools, parses module dependency info, caches by commit hash, renders to markdown within token budget
- [ ] GREEN: Implement `fetch_architecture()` using MCP calls, parse into `CcgArchitecture`, render as markdown
- [ ] REFACTOR: Add token estimation for architecture output, truncate if exceeds budget allocation
- Files: `src/context/compression/orchestrator.rs`, `src/context/compression/render.rs`
- Acceptance: Architecture context produced from live narsil data, token-limited

---

### 21. Phase 3.3: Wire Symbol Queries to Live MCP

**Description**: Implement symbol-level context using narsil's `find_symbols`, `search_code`, and `get_export_map` tools for active files.

**TDD Cycle**:
- [ ] RED: Write tests for `query_symbols()` — builds correct query for active files, parses symbol results, respects token budget, returns truncated results if over budget
- [ ] GREEN: Implement `query_symbols()` using `McpClient::call_tool("find_symbols", ...)` and `search_code`, parse results into `SymbolResults`
- [ ] REFACTOR: Add smart symbol prioritization — public API surface first, then internal, then tests
- Files: `src/context/compression/orchestrator.rs`
- Acceptance: Symbol context for active files produced from live narsil, budget-respecting

---

### 22. Phase 3.4: Implement build_context with Token Budgeting

**Description**: Implement the top-level `build_context()` method that assembles manifest + architecture + symbols within a total token budget.

**TDD Cycle**:
- [ ] RED: Write tests for `build_context()` — allocates budget across layers (manifest ~500, architecture ~5-12K, symbols remainder), parallel fetching with `tokio::try_join!`, graceful degradation when narsil unavailable
- [ ] GREEN: Implement `build_context()` with layered budget allocation, parallel fetching, markdown assembly
- [ ] REFACTOR: Add metrics tracking — cache hit rate, fetch latency, token usage per layer
- Files: `src/context/compression/orchestrator.rs`
- Acceptance: Full context built within budget, parallel fetching measurably faster than sequential

---

### 23. Phase 3.5: Inject Context into Message-Sending Path

**Description**: Wire `build_context()` into the API message preparation path so compressed context is automatically included before each API call.

**TDD Cycle**:
- [ ] RED: Write tests verifying context injection — system message includes compressed context, context updates on file changes, context omitted when narsil unavailable
- [ ] GREEN: Call `build_context()` in `submit_message()` / continuation path, prepend to system message or first user message
- [ ] REFACTOR: Add config toggle for context injection (`auto_context_enabled`), add context size to token budget display in status bar
- Files: `src/app/state.rs`, `src/app/handlers/stream.rs`
- Acceptance: API calls include fresh compressed context, visible in token budget display

---

## Sprint 4: Agent Orchestration (Priority 3)

**Goal**: Implement agent spawning via git worktrees with cross-agent conflict detection and slash commands.

**Success Criteria**:
- Agents run in separate git worktrees
- Session persistence per worktree
- Conflict detection via narsil
- `/agent` slash commands functional

**Dependency**: Sprint 1 complete (AgentHandler slot in dispatcher)

---

### 24. Phase 4.1: WorktreeAgentManager Core

**Description**: Create `WorktreeAgentManager` that manages agent lifecycle — create worktree, track status, clean up.

**TDD Cycle**:
- [ ] RED: Write tests for agent lifecycle — `spawn()` creates git worktree, `list()` returns active agents, `status()` returns per-agent info, `cleanup()` removes worktree
- [ ] GREEN: Implement `WorktreeAgentManager` with `HashMap<String, AgentHandle>`, `AgentStatus` enum, git worktree commands
- [ ] REFACTOR: Add worktree path validation, branch name sanitization, concurrent spawn protection
- Files: `src/agents/worktree_agent.rs` (new)
- Acceptance: Agents create real git worktrees in temp dirs, lifecycle tracked correctly

---

### 25. Phase 4.2: Agent Execution Loop

**Description**: Implement `run_agent_loop()` — a simplified event loop that runs inside a worktree with its own session.

**TDD Cycle**:
- [ ] RED: Write tests for agent loop — creates session in worktree, runs LLM turns, records results, reports completion/failure, respects max iterations
- [ ] GREEN: Implement `run_agent_loop()` using the new `EventHandler` architecture with a subset of handlers (Stream + Tool only, no TUI)
- [ ] REFACTOR: Add iteration tracking, progress reporting via channel back to main loop
- Files: `src/agents/worktree_agent.rs`
- Acceptance: Agent loop runs independently, produces results, respects iteration limit

---

### 26. Phase 4.3: Cross-Agent Conflict Detection

**Description**: Use narsil to detect when multiple agents modify the same symbols or files.

**TDD Cycle**:
- [ ] RED: Write tests for conflict detection — two agents modifying same file detected, same symbol detected, non-overlapping changes no conflict, stale worktree handled
- [ ] GREEN: Implement `check_conflicts()` using `McpClient::call_tool("get_modified_files", ...)` and `find_references` to identify overlapping changes
- [ ] REFACTOR: Add conflict severity levels (warning for same file, error for same function), merge preview
- Files: `src/agents/worktree_agent.rs`
- Acceptance: Conflicts detected accurately for test scenarios with overlapping edits

---

### 27. Phase 4.4: Agent Slash Commands

**Description**: Implement `/agent new`, `/agent list`, `/agent status`, `/agent merge` slash commands.

**TDD Cycle**:
- [ ] RED: Write tests for each command — `new` spawns agent, `list` shows table, `status` shows detail, `merge` performs conflict check + git merge
- [ ] GREEN: Register agent commands in `SlashCommandHandler`, wire to `WorktreeAgentManager`
- [ ] REFACTOR: Add tab completion for agent names, status display in TUI, merge confirmation prompt
- Files: `src/app/commands.rs`, `src/agents/worktree_agent.rs`
- Acceptance: All agent commands functional from TUI input

---

### 28. Phase 4.5: Wire AgentHandler into Event Dispatcher

**Description**: Create `AgentHandler` that processes `AppEvent::AgentEvent` for agent progress updates, completion notifications, and conflict alerts in the main event loop.

**TDD Cycle**:
- [ ] RED: Write tests for `AgentHandler` — processes agent progress events, updates TUI state for agent panel, handles agent completion, handles agent failure
- [ ] GREEN: Implement `AgentHandler`, add `AgentEvent` variant to `AppEvent`, wire into `EventDispatcher`
- [ ] REFACTOR: Add agent panel to TUI showing active agents and their status
- Files: `src/app/handlers/agent.rs` (new), `src/app/events.rs`, `src/app/mod.rs`
- Acceptance: Agent events flow through dispatcher, TUI shows agent status

---

## Sprint 5: Parallelism & Performance (Priority 4)

**Goal**: Add parallel context building, parallel MCP capability detection, and configurable parallelism policies.

**Success Criteria**:
- Context layers fetched in parallel via `tokio::try_join!`
- MCP capabilities detected in parallel on startup
- Configurable parallelism policy
- Measurable latency improvement in benchmarks

**Dependency**: Sprint 3 complete (context compression wired)

---

### 29. Phase 5.1: Parallel Context Building

**Description**: Fetch manifest, architecture, and symbols concurrently using `tokio::try_join!`.

**TDD Cycle**:
- [ ] RED: Write benchmark test comparing sequential vs parallel context building — parallel should be measurably faster
- [ ] GREEN: Refactor `build_context()` to use `tokio::try_join!` for independent fetches
- [ ] REFACTOR: Add timing metrics, ensure cache hits don't block parallel fetches
- Files: `src/context/compression/orchestrator.rs`
- Acceptance: Parallel fetch measurably faster in benchmarks, behavior identical

---

### 30. Phase 5.2: Parallel MCP Capability Detection

**Description**: Detect capabilities of all configured MCP servers in parallel on startup.

**TDD Cycle**:
- [ ] RED: Write tests for parallel detection — multiple MCP clients queried concurrently, results collected, timeout handling for slow servers
- [ ] GREEN: Implement `detect_all_capabilities()` using `futures::future::join_all()` on `McpClient::list_tools()`
- [ ] REFACTOR: Add timeout per server, graceful handling of unreachable servers
- Files: `src/narsil/integration.rs`, `src/mcp/client.rs`
- Acceptance: Multiple MCP servers detected concurrently, no blocking

---

### 31. Phase 5.3: Parallelism Config and Policy

**Description**: Add configurable parallelism policy to `Config` with `parallel_policy`, `max_parallel_agents`, `max_parallel_tools`.

**TDD Cycle**:
- [ ] RED: Write tests for config parsing — default policy, aggressive, sequential, max values validated
- [ ] GREEN: Add `PerformanceConfig` to `src/types/config.rs`, wire into parallel executor and agent manager
- [ ] REFACTOR: Add runtime policy switching via `/config` command, document in user guide
- Files: `src/types/config.rs`, `src/tools/parallel/mod.rs`, `src/agents/worktree_agent.rs`
- Acceptance: Parallelism policy respected by all parallel execution paths

---

## Sprint 6: Continuous Self-Healing Loop (Priority 5)

**Goal**: Implement the continuous coding loop with quality gates, stagnation detection, and narsil-powered root-cause analysis for self-healing.

**Success Criteria**:
- Continuous loop runs with configurable quality gates
- Stagnation detection with multi-factor risk scoring
- Narsil root-cause analysis on failures
- Recovery loop with automatic retry
- TUI progress display via ContinuousHandler

**Dependency**: Sprints 1, 2, 3 complete

---

### 32. Phase 6.1: Quality Gate Execution

**Description**: Implement `QualityGateRunner` that executes configured gates (tests, clippy, security scan) and reports pass/fail.

**TDD Cycle**:
- [ ] RED: Write tests for gate execution — `TestsPass` gate runs `cargo test`, `ClippyClean` runs `cargo clippy`, `Custom` runs arbitrary command, results collected, partial failures reported
- [ ] GREEN: Implement `QualityGateRunner` in `src/continuous/gates.rs` using `ToolExecutor::execute_bash()`
- [ ] REFACTOR: Add gate timeout, output capture for failure diagnostics, parallel gate execution where independent
- Files: `src/continuous/gates.rs` (new)
- Acceptance: All quality gates execute correctly, failures include diagnostic output

---

### 33. Phase 6.2: Stagnation Detector

**Description**: Implement multi-factor stagnation detection — commit gaps, repeated file edits, recurring errors, test count plateau.

**TDD Cycle**:
- [ ] RED: Write tests for stagnation scoring — commit gap increases score, same files edited repeatedly increases score, recurring errors increase score, score resets on meaningful progress, risk levels (Low/Medium/High/Critical) at correct thresholds
- [ ] GREEN: Implement `StagnationDetector` in `src/continuous/stagnation.rs` with weighted risk factors
- [ ] REFACTOR: Add configurable weights and thresholds, history window (last N iterations)
- Files: `src/continuous/stagnation.rs` (new)
- Acceptance: Stagnation detected accurately for synthetic scenarios, risk levels trigger at correct thresholds

---

### 34. Phase 6.3: Narsil Root-Cause Analysis

**Description**: Use narsil call graph and symbol analysis to identify root causes of failures — which functions are involved, what changed recently, what files are relevant.

**TDD Cycle**:
- [ ] RED: Write tests for root-cause analysis — extracts file/function from error message, queries call graph via MCP, identifies relevant files, returns structured `RootCauseAnalysis`
- [ ] GREEN: Implement `narsil_root_cause()` using `McpClient::call_tool("get_callers", ...)`, `get_file_history`, `get_blame`
- [ ] REFACTOR: Add error location parsing for common Rust error formats (cargo test, clippy), fallback when narsil unavailable
- Files: `src/continuous/recovery.rs` (new)
- Acceptance: Root cause analysis produces actionable file list for test failures

---

### 35. Phase 6.4: Recovery Loop

**Description**: Implement the self-healing recovery loop — on failure, use root-cause analysis to build recovery prompt, ask LLM to fix, re-run quality gates.

**TDD Cycle**:
- [ ] RED: Write tests for recovery — single recovery attempt succeeds, multiple attempts with diminishing returns, max attempts respected, `NeedsHuman` returned when recovery fails, `Fixed` returned on success
- [ ] GREEN: Implement `attempt_recovery()` with recovery prompt building, LLM turn execution, gate re-check
- [ ] REFACTOR: Add recovery attempt logging, metrics (success rate, avg attempts to fix)
- Files: `src/continuous/recovery.rs`
- Acceptance: Recovery loop can fix simple test failures automatically, gives up gracefully on complex issues

---

### 36. Phase 6.5: ContinuousRunner State Machine

**Description**: Implement the top-level `ContinuousRunner` that orchestrates iteration → quality gates → stagnation check → recovery → result.

**TDD Cycle**:
- [ ] RED: Write tests for full loop — all gates pass on first try returns `AllGatesPassed`, gate failure triggers recovery, stagnation detected returns `HumanCheckpointRequired`, max iterations returns `MaxIterationsReached`, fatal error propagated
- [ ] GREEN: Implement `ContinuousRunner::run()` with the full state machine: iterate → run gates → check stagnation → attempt recovery → continue or stop
- [ ] REFACTOR: Add event emission (`ContinuousEvent`) for TUI progress display, configurable max iterations and recovery attempts
- Files: `src/continuous/runner.rs` (new), `src/continuous/mod.rs`
- Acceptance: Full continuous loop works end-to-end with mock provider and tools

---

### 37. Phase 6.6: Wire ContinuousHandler into Event Dispatcher

**Description**: Create `ContinuousHandler` that processes `AppEvent::ContinuousEvent` for loop progress, gate results, stagnation alerts, and recovery status in the TUI.

**TDD Cycle**:
- [ ] RED: Write tests for `ContinuousHandler` — processes iteration progress, gate pass/fail display, stagnation warning display, recovery status
- [ ] GREEN: Implement `ContinuousHandler`, add `ContinuousEvent` variant to `AppEvent`, wire into dispatcher
- [ ] REFACTOR: Add TUI widget for continuous loop progress (iteration count, gate status, recovery attempts), `/continuous start` and `/continuous stop` commands
- Files: `src/app/handlers/continuous.rs` (new), `src/app/events.rs`, `src/continuous/mod.rs`
- Acceptance: Continuous loop progress visible in TUI, start/stop commands functional

---

## Completed
<!-- Move completed tasks here -->

---

## Blocked
<!-- Document blockers with suggested actions -->

---

## Notes

- Ralph reads this file each iteration to select the next task
- Checkbox completion (`[x]`) signals progress to the loop
- Tasks are prioritized top-to-bottom within each section
- Blocked tasks should document why and suggest resolution
- Sprint 1 is the foundation — all other sprints depend on it
- Sprints 2 and 3 can run in parallel after Sprint 1
- Sprint 4 depends on Sprint 3
- Sprint 5 depends on Sprint 1 (event dispatcher)
- Sprint 6 depends on Sprints 1, 2, and 3

### Commit Cadence

For each phase (N.M):
```bash
# RED: Write failing tests
git commit -m "test(module): Add Phase N.M tests (N.M RED)"

# GREEN: Make tests pass
git commit -m "feat(module): Implement Phase N.M (N.M GREEN)"

# REFACTOR: Clean up
git commit -m "refactor(module): Clean up Phase N.M (N.M REFACTOR)"

# Mark checkbox complete
git commit -m "docs: Mark Phase N.M complete"
```

### Sprint Dependency Graph

```
Sprint 1 (Event Loop) ──┬──> Sprint 2 (OpenRouter)
                         ├──> Sprint 3 (Compression) ──> Sprint 5 (Parallelism)
                         ├──> Sprint 4 (Agents)
                         └──> Sprint 6 (Continuous Loop)
                                   ↑
                              Sprint 2 + Sprint 3
```

<!-- Ralph: work continues across sprints. Never signal plan completion. -->
