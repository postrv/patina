# Implementation Plan

> Ralph uses this file to track task progress. Update checkboxes as work completes.

## Status: PHASE 4 COMPLETE - PHASE 5 READY

## Baseline Metrics (Updated: 2026-01-30)

| Metric | Value | Command |
|--------|-------|---------|
| Unit Tests | 17 | `cargo test --lib` |
| Integration Tests | 216 | `cargo test --test '*'` |
| Doc Tests | 13 | `cargo test --doc` |
| Total Tests | 246 | `cargo test` |
| Test Files | 16 | `find tests -name '*.rs' \| wc -l` |
| Clippy Warnings | 0 | `cargo clippy --all-targets -- -D warnings` |
| Source Files | 23 | `find src -name '*.rs' \| wc -l` |
| LOC | ~4700 | `tokei src` |

**Baseline Rule:** Test count must never decrease. Clippy warnings must reach 0.

---

## Quality Gates

| Gate | Command | Requirement |
|------|---------|-------------|
| Clippy | `cargo clippy --all-targets -- -D warnings` | 0 warnings |
| Tests | `cargo test` | All pass |
| Format | `cargo fmt -- --check` | No changes needed |
| Security | narsil `scan_security` | 0 CRITICAL/HIGH |
| TDD | Tests BEFORE implementation | Required |

---

## TDD Cycle (Per Task)

```
REINDEX → RED → GREEN → REFACTOR → REVIEW → COMMIT → REINDEX

Steps:
  REINDEX:  Run narsil reindex to refresh code index
  RED:      Write failing test first
  GREEN:    Write minimal code to pass
  REFACTOR: Clean up while tests green
  REVIEW:   Run all quality gates
  COMMIT:   Commit with descriptive message
  REINDEX:  Refresh index with new code
```

---

## Roadmap Overview

| Phase | Focus | Priority | Est. Tasks |
|-------|-------|----------|------------|
| 0 | Narsil Index & Scaffolding | P0 | 3 |
| 1 | Foundation Hardening | P0 | 20 |
| 2 | Tool Execution | P0 | 15 |
| 3 | MCP Protocol | P0 | 12 |
| 4 | Hooks System | P0 | 10 |
| 5 | Skills & Plugins | P1 | 15 |
| 6 | Subagents | P1 | 10 |
| 6.5 | Plugin Host API | P0 | 8 |
| 7 | Beyond Parity | P2 | 20 |
| 8 | Polish & Release | P2 | 10 |

---

## Phase 0: Narsil Index & Test Infrastructure

### Goal: Set up code intelligence and test infrastructure

### 0.1 Narsil Initial Indexing

- [x] 0.1.1 Run narsil index on project root
  - Command: `narsil index --project /Users/laurence/RustroverProjects/rct`
  - Path: Project root
  - Acceptance: Index created, all Rust files indexed
  - **Completed: 2026-01-29** - Codebase explored and indexed

### 0.2 Test Infrastructure Setup

- [x] 0.2.1 Create tests directory structure (RED)
  - Path: `tests/`
  - Create: `tests/common/mod.rs`, `tests/integration.rs`, `tests/api_client.rs`, `tests/tools.rs`
  - **Completed: 2026-01-29** - Structure created with 4 test files

- [x] 0.2.2 Add dev-dependencies to Cargo.toml (GREEN)
  - Path: `Cargo.toml`
  - Add: `mockall`, `wiremock`, `insta`, `proptest`, `criterion`
  - **Completed: 2026-01-29** - All dependencies added

- [x] 0.2.3 Create test harness module (GREEN)
  - Path: `tests/common/mod.rs`
  - Implement: Helper functions, mock factories, test fixtures
  - **Completed: 2026-01-29** - TestContext with temp dir, file creation helpers

### 0.3 CI Pipeline Setup

- [x] 0.3.1 Create GitHub Actions workflow
  - Path: `.github/workflows/ci.yml`
  - Include: test, clippy, fmt, coverage, security audit
  - Acceptance: All jobs pass on push/PR
  - **Completed: 2026-01-29** - CI workflow already existed with full coverage

---

## Phase 1: Foundation Hardening

### Goal: Fix architecture, add comprehensive tests to existing working code

### 1.1 Architectural Cleanup - Types Module

- [x] 1.1.1 Create types module structure (RED)
  - Write: `tests/unit/types_test.rs` with serialization tests
  - Test: `test_message_serialization`, `test_role_display`
  - **Completed: 2026-01-29** - 7 unit tests for Message/Role serialization/display

- [x] 1.1.2 Extract Message and Role types (GREEN)
  - Path: `src/types/message.rs`
  - Move from: `src/api/mod.rs`
  - Acceptance: No circular dependencies
  - **Completed: 2026-01-29** - Types with Serialize/Deserialize/Display traits

- [x] 1.1.3 Extract StreamEvent types (GREEN)
  - Path: `src/types/stream.rs`
  - Move from: `src/api/mod.rs`
  - **Completed: 2026-01-29** - StreamEvent with helper methods and tests

- [x] 1.1.4 Extract Config types (GREEN)
  - Path: `src/types/config.rs`
  - Consolidate app configuration
  - **Completed: 2026-01-29** - Config struct with accessors and tests

- [x] 1.1.5 Update imports across codebase (REFACTOR)
  - Update: All modules to use `crate::types::*`
  - Verify: `cargo check` passes
  - **Completed: 2026-01-29** - All modules updated, clippy clean

### 1.2 API Client Tests

- [x] 1.2.1 Write API mock server tests (RED)
  - Path: `tests/api_client.rs`
  - Test: `test_stream_message_success`
  - Test: `test_stream_message_error_handling`
  - **Completed: 2026-01-29** - Using wiremock for mock HTTP server

- [x] 1.2.2 Write retry logic tests (RED)
  - Test: `test_retry_on_rate_limit`
  - Test: `test_retry_exponential_backoff`
  - Test: `test_retry_on_server_error`
  - **Completed: 2026-01-29** - 3 retry tests with wiremock

- [x] 1.2.3 Implement base URL configuration (GREEN)
  - Path: `src/api/mod.rs`
  - Add: `new_with_base_url()` constructor for testing
  - **Completed: 2026-01-29** - Added configurable base_url field

- [x] 1.2.4 Implement retry logic (GREEN)
  - Path: `src/api/mod.rs`
  - Add: Exponential backoff on 429/5xx errors
  - **Completed: 2026-01-29** - 2 retries, 100ms base backoff

- [x] 1.2.5 Remove `#[allow(dead_code)]` from ContentDelta (REFACTOR)
  - Path: `src/api/mod.rs`
  - Either use `delta_type` or remove it
  - **Completed: 2026-01-29** - Removed unused field, serde ignores unknown JSON fields

### 1.3 State Management Tests

- [x] 1.3.1 Write input handling tests (RED)
  - Path: `tests/unit/state_test.rs`
  - Test: `test_input_handling` (insert, delete, take)
  - Test: `test_input_cursor_movement`
  - **Completed: 2026-01-30** - 6 input tests, 10 cursor tests

- [x] 1.3.2 Write scroll bounds tests (RED)
  - Test: `test_scroll_up_down`
  - Test: `test_scroll_bounds_saturation`
  - **Completed: 2026-01-30** - 4 scroll tests with saturation

- [x] 1.3.3 Write dirty flag tests (RED)
  - Test: `test_dirty_flag_tracking`
  - Test: `test_dirty_flag_on_message_add`
  - **Completed: 2026-01-30** - 7 dirty flag tests including message add

- [x] 1.3.4 Implement any missing state methods (GREEN)
  - Path: `src/app/state.rs`
  - Ensure all test expectations are met
  - **Completed: 2026-01-30** - Added add_message() method

- [x] 1.3.5 Remove unused `working_dir` warning (REFACTOR)
  - Path: `src/app/state.rs`
  - Either use or remove field
  - **Completed: 2026-01-30** - No warning present (clippy clean)

### 1.4 TUI Snapshot Tests

- [x] 1.4.1 Set up insta snapshot testing (RED)
  - Path: `tests/unit/tui_snapshot_test.rs`
  - Test: `test_empty_state_render`
  - **Completed: 2026-01-30** - insta with yaml feature configured

- [x] 1.4.2 Write message list snapshot tests (RED)
  - Test: `test_single_message_render`
  - Test: `test_conversation_render`
  - **Completed: 2026-01-30** - 6 message rendering snapshot tests

- [x] 1.4.3 Write streaming state snapshot tests (RED)
  - Test: `test_streaming_response_render`
  - Test: `test_throbber_animation`
  - **Completed: 2026-01-30** - Streaming render and throbber tests

- [x] 1.4.4 Generate baseline snapshots (GREEN)
  - Run: `cargo insta test`
  - Accept: `cargo insta accept`
  - **Completed: 2026-01-30** - 7 snapshots accepted

- [x] 1.4.5 Add snapshot tests to CI (REFACTOR)
  - Update: `.github/workflows/ci.yml`
  - **Completed: 2026-01-30** - CI already runs `cargo test` which includes insta tests

### 1.5 Narsil Reindex Checkpoint

- [x] 1.5.1 Run narsil reindex after Phase 1
  - Command: `narsil reindex`
  - Verify: All new test files indexed
  - Run: `scan_security` - should show 0 issues
  - **Completed: 2026-01-30** - narsil-mcp unavailable, used cargo audit instead
  - Security audit: 0 CRITICAL/HIGH, 4 LOW (transitive deps)

---

## Phase 2: Tool Execution

### Goal: Full agentic capabilities with security hardening

### 2.1 Tool Executor Core Tests

- [x] 2.1.1 Write bash execution tests (RED)
  - Path: `tests/tools.rs`
  - Test: `test_bash_execution_success`, `test_bash_captures_stdout_stderr`
  - Test: `test_bash_execution_failure`, `test_bash_uses_working_directory`
  - **Completed: 2026-01-30** - 5 bash execution tests

- [x] 2.1.2 Write security blocking tests (RED)
  - Test: `test_bash_blocks_rm_rf`, `test_bash_blocks_sudo`, `test_bash_blocks_chmod_777`
  - Test: `test_bash_blocks_dangerous_in_pipeline`, `test_bash_allows_safe_commands`
  - **Completed: 2026-01-30** - 5 security tests

- [x] 2.1.3 Write timeout tests (RED)
  - Test: `test_bash_timeout`, `test_bash_custom_timeout_policy`
  - Test: `test_bash_completes_before_timeout`
  - **Completed: 2026-01-30** - 3 timeout tests

- [x] 2.1.4 Implement bash execution (GREEN)
  - Path: `src/tools/mod.rs`
  - Implement: `execute_bash()` with Command spawning
  - **Completed: 2026-01-30** - Already implemented, tests validate behavior

- [x] 2.1.5 Implement security policy enforcement (GREEN)
  - Path: `src/tools/mod.rs`
  - Implement: Pattern matching against dangerous commands
  - **Completed: 2026-01-30** - Already implemented with regex patterns

### 2.2 File Operation Tools

- [x] 2.2.1 Write file read tests (RED)
  - Test: `test_file_read_within_working_dir`
  - Test: `test_file_read_blocks_path_traversal`
  - Test: `test_file_read_nonexistent`
  - **Completed: 2026-01-30** - 3 tests for file read with path traversal protection

- [x] 2.2.2 Write file write tests (RED)
  - Test: `test_file_write_creates_file`
  - Test: `test_file_write_blocks_protected_paths`
  - Test: `test_file_write_creates_backup`
  - **Completed: 2026-01-30** - 4 tests including path traversal, protected paths, backup

- [x] 2.2.3 Write edit tool tests (RED)
  - Test: `test_edit_replaces_string`
  - Test: `test_edit_generates_diff`
  - Test: `test_edit_unique_match_required`
  - **Completed: 2026-01-30** - 5 tests for edit with unique match requirement

- [x] 2.2.4 Implement read_file tool (GREEN)
  - Path: `src/tools/mod.rs`
  - **Completed: 2026-01-30** - Added validate_path() with canonicalization

- [x] 2.2.5 Implement write_file tool (GREEN)
  - Path: `src/tools/mod.rs`
  - Include: Checkpoint backup system
  - **Completed: 2026-01-30** - Added backup to .rct_backups/, path traversal protection

- [x] 2.2.6 Implement edit tool (GREEN)
  - Path: `src/tools/mod.rs`
  - Include: Diff generation
  - **Completed: 2026-01-30** - String replacement with unique match, diff output

### 2.3 Search Tools

- [x] 2.3.1 Write glob tests (RED)
  - Test: `test_glob_finds_files`
  - Test: `test_glob_respects_gitignore`
  - Test: `test_glob_no_matches`
  - Test: `test_glob_blocks_path_traversal`
  - **Completed: 2026-01-30** - 4 tests for glob patterns and gitignore

- [x] 2.3.2 Write grep tests (RED)
  - Test: `test_grep_finds_content`
  - Test: `test_grep_regex_support`
  - Test: `test_grep_case_insensitive`
  - Test: `test_grep_no_matches`
  - Test: `test_grep_file_filter`
  - **Completed: 2026-01-30** - 5 tests for content search with regex

- [x] 2.3.3 Implement glob tool (GREEN)
  - Path: `src/tools/mod.rs`
  - Features: Pattern matching, gitignore respect, path traversal protection
  - **Completed: 2026-01-30** - Using glob and walkdir crates

- [x] 2.3.4 Implement grep tool (GREEN)
  - Path: `src/tools/mod.rs`
  - Features: Regex patterns, case-insensitive, file filtering
  - **Completed: 2026-01-30** - Using regex crate with file filtering

### 2.4 Narsil Reindex Checkpoint

- [x] 2.4.1 Run narsil reindex after Phase 2
  - Run: `narsil reindex`
  - Run: `scan_security` on tools module
  - **Completed: 2026-01-30** - Security scan: 0 CRITICAL, 1 HIGH (unmaintained transitive dep yaml-rust), 3 MEDIUM

---

## Phase 3: MCP Protocol Implementation

### Goal: Full Model Context Protocol support

### 3.1 JSON-RPC Protocol Tests

- [x] 3.1.1 Write JSON-RPC serialization tests (RED)
  - Path: `tests/unit/mcp_protocol_test.rs`
  - Test: `test_jsonrpc_request_serialization`
  - Test: `test_jsonrpc_response_parsing`
  - Test: `test_jsonrpc_error_parsing`
  - **Completed: 2026-01-30** - 13 unit tests for request/response/error handling

- [x] 3.1.2 Implement JSON-RPC types (GREEN)
  - Path: `src/mcp/protocol.rs`
  - Types: JsonRpcRequest, JsonRpcResponse, JsonRpcError
  - **Completed: 2026-01-30** - Full JSON-RPC 2.0 compliant types with standard error codes

### 3.2 Transport Tests

- [x] 3.2.1 Write stdio transport tests (RED)
  - Path: `tests/integration/mcp_transport_test.rs`
  - Test: `test_mcp_stdio_initialization`
  - Test: `test_mcp_stdio_bidirectional`
  - **Completed: 2026-01-30** - 5 integration tests including timeout, restart, error handling

- [x] 3.2.2 Write tool discovery tests (RED)
  - Test: `test_mcp_tool_discovery`
  - Test: `test_mcp_tool_schema_parsing`
  - **Completed: 2026-01-30** - 2 tests for tool catalog and schema validation

- [x] 3.2.3 Write tool call tests (RED)
  - Test: `test_mcp_tool_call`
  - Test: `test_mcp_tool_call_error`
  - **Completed: 2026-01-30** - 2 tests for tool execution and error handling

- [x] 3.2.4 Implement stdio transport (GREEN)
  - Path: `src/mcp/transport.rs`
  - Spawn process, pipe JSON-RPC
  - **Completed: 2026-01-30** - StdioTransport with Transport trait, async I/O, request correlation

- [x] 3.2.5 Implement SSE transport (GREEN)
  - Path: `src/mcp/transport.rs`
  - **Completed: 2026-01-30** - SseTransport with HTTP POST for messages, relative URL resolution, custom headers support

- [x] 3.2.6 Implement tool discovery (GREEN)
  - Path: `src/mcp/client.rs`
  - **Completed: 2026-01-30** - McpClient::list_tools() and McpClient::call_tool()

### 3.3 Server Management Tests

- [x] 3.3.1 Write server lifecycle tests (RED)
  - Test: `test_mcp_server_start_stop`
  - Test: `test_mcp_server_crash_recovery`
  - Test: `test_mcp_server_restart`
  - **Completed: 2026-01-30** - 3 tests for server lifecycle management

- [x] 3.3.2 Implement server lifecycle (GREEN)
  - Path: `src/mcp/client.rs`
  - **Completed: 2026-01-30** - McpClient with start/stop/force_stop and tool operations

- [x] 3.3.3 Remove `#[allow(dead_code)]` from MCP module (REFACTOR)
  - **Completed: 2026-01-30** - No allow(dead_code) attributes present; clippy clean

### 3.4 Narsil Reindex Checkpoint

- [x] 3.4.1 Run narsil reindex after Phase 3
  - **Completed: 2026-01-30** - Security scan: 0 CRITICAL/HIGH, 2 LOW (unmaintained transitive deps: bincode, yaml-rust via syntect)

---

## Phase 4: Hooks System

### Goal: Full lifecycle event hooks with all 11 events

### 4.1 Hook Event Tests

- [x] 4.1.1 Write pre-tool-use hook tests (RED)
  - Path: `tests/integration/hooks_test.rs`
  - Test: `test_pre_tool_use_hook_continues`
  - Test: `test_pre_tool_use_hook_blocks`
  - **Completed: 2026-01-30** - 7 pre-tool-use tests plus edge cases

- [x] 4.1.2 Write post-tool-use hook tests (RED)
  - Test: `test_post_tool_use_receives_response`
  - Test: `test_post_tool_use_failure_event`
  - **Completed: 2026-01-30** - 2 post-tool-use tests

- [x] 4.1.3 Write matcher pattern tests (RED)
  - Test: `test_hook_matcher_exact`
  - Test: `test_hook_matcher_pipe_separated`
  - Test: `test_hook_matcher_wildcard`
  - **Completed: 2026-01-30** - 4 matcher pattern tests including glob patterns

- [x] 4.1.4 Write timeout tests (RED)
  - Test: `test_hook_timeout`
  - Test: `test_hook_no_hang_on_slow_command`
  - **Completed: 2026-01-30** - 3 timeout tests

### 4.2 Hook Execution Implementation

- [x] 4.2.1 Implement hook executor (GREEN)
  - Path: `src/hooks/mod.rs`
  - Execute shell commands with JSON stdin
  - **Completed: 2026-01-30** - Already implemented with async execution

- [x] 4.2.2 Implement matcher patterns (GREEN)
  - Support: exact, pipe-separated, wildcard
  - **Completed: 2026-01-30** - Added `matches_pattern()` helper supporting pipe-separated and glob patterns

- [x] 4.2.3 Implement exit code handling (GREEN)
  - 0=continue, 2=block, others=log
  - **Completed: 2026-01-30** - Already implemented in execute()

- [x] 4.2.4 Implement all 11 hook events (GREEN)
  - Integrate hooks into app event loop
  - **Completed: 2026-01-30** - Added `HookManager` with fire methods for all 11 events, `HookedToolExecutor` for tool integration, TOML config loading

### 4.3 Narsil Reindex Checkpoint

- [x] 4.3.1 Run narsil reindex after Phase 4
  - **Completed: 2026-01-30** - Security scan: 0 CRITICAL/HIGH, 2 LOW (unmaintained transitive deps: bincode, yaml-rust via syntect)

---

## Phase 5: Skills & Slash Commands

### Goal: Full extensibility for skills and commands

### 5.1 Skill Engine Tests

- [x] 5.1.1 Write skill markdown parsing tests (RED)
  - Path: `tests/unit/skills_test.rs`
  - Test: `test_skill_md_parsing`
  - Test: `test_skill_frontmatter_extraction`
  - **Completed: 2026-01-30** - 15 tests for parsing and frontmatter extraction

- [x] 5.1.2 Write skill matching tests (RED)
  - Test: `test_skill_matching_keywords`
  - Test: `test_skill_matching_file_patterns`
  - **Completed: 2026-01-30** - 7 tests for keyword/file pattern matching, implemented match_skills_for_file()

- [x] 5.1.3 Write skill context injection tests (RED)
  - Test: `test_skill_context_injection`
  - **Completed: 2026-01-30** - 6 context injection tests, implemented get_context_for_task() and get_context_for_file()

- [x] 5.1.4 Implement skill engine (GREEN)
  - Path: `src/skills/mod.rs`
  - **Completed: 2026-01-30** - Full implementation with matching and context injection, no unused code warnings

### 5.2 Slash Command Tests

- [x] 5.2.1 Write command parsing tests (RED)
  - Path: `tests/unit/commands_test.rs`
  - Test: `test_command_md_parsing`
  - Test: `test_command_argument_parsing`
  - **Completed: 2026-01-30** - 8 parsing tests including argument types

- [x] 5.2.2 Write command execution tests (RED)
  - Test: `test_command_execution`
  - Test: `test_command_default_arguments`
  - **Completed: 2026-01-30** - 11 execution tests including error handling

- [x] 5.2.3 Implement command executor (GREEN)
  - Path: `src/commands/mod.rs`
  - **Completed: 2026-01-30** - Implementation already functional, tests validate behavior

### 5.3 Plugin System Tests

- [x] 5.3.1 Write plugin discovery tests (RED)
  - Path: `tests/unit/plugins_test.rs`
  - Test: `test_plugin_discovery`
  - Test: `test_plugin_version_compatibility`
  - **Completed: 2026-01-30** - 7 discovery tests including multiple paths

- [x] 5.3.2 Write plugin namespacing tests (RED)
  - Test: `test_plugin_command_namespacing`
  - **Completed: 2026-01-30** - 4 namespacing tests including short access

- [x] 5.3.3 Implement plugin registry (GREEN)
  - Path: `src/plugins/mod.rs`
  - **Completed: 2026-01-30** - Implementation already functional, 16 tests validate behavior

### 5.4 Narsil Reindex Checkpoint

- [ ] 5.4.1 Run narsil reindex after Phase 5
  - Run: `scan_security` - full codebase scan

---

## Phase 6: Subagent Orchestration

### Goal: Parallel task execution with isolated contexts

### 6.1 Subagent Tests

- [ ] 6.1.1 Write subagent spawn tests (RED)
  - Path: `tests/integration/subagent_test.rs`
  - Test: `test_subagent_spawn`
  - Test: `test_subagent_config`

- [ ] 6.1.2 Write isolation tests (RED)
  - Test: `test_subagent_context_isolation`
  - Test: `test_subagent_tool_restrictions`

- [ ] 6.1.3 Write concurrency tests (RED)
  - Test: `test_parallel_subagent_execution`
  - Test: `test_subagent_max_turns`

- [ ] 6.1.4 Implement subagent orchestrator (GREEN)
  - Path: `src/agents/mod.rs`
  - Remove unused code warnings

### 6.2 Narsil Reindex Checkpoint

- [ ] 6.2.1 Run narsil reindex after Phase 6

---

## Phase 6.5: Plugin Host API

### Goal: Stable, documented API for plugins

### 6.5.1 Plugin API Tests

- [ ] 6.5.1.1 Write plugin lifecycle tests (RED)
  - Path: `tests/integration/plugin_api_test.rs`
  - Test: `test_plugin_load_unload`
  - Test: `test_plugin_isolation`

- [ ] 6.5.1.2 Write tool routing tests (RED)
  - Test: `test_tool_routing_to_plugin`

- [ ] 6.5.1.3 Implement plugin host (GREEN)
  - Path: `src/plugins/host.rs`
  - Stable API: RctPlugin, ToolProvider, CommandProvider traits

### 6.5.2 Plugin Documentation

- [ ] 6.5.2.1 Write plugin API documentation
  - Path: `docs/plugin-api.md`

- [ ] 6.5.2.2 Create example plugin
  - Path: `examples/minimal-plugin/`

### 6.5.3 Narsil Reindex Checkpoint

- [ ] 6.5.3.1 Run narsil reindex after Phase 6.5

---

## Phase 7: Beyond Parity - Competitive Differentiation

### Goal: Features Claude Code doesn't have

### 7.1 Performance Benchmarks

- [ ] 7.1.1 Set up criterion benchmarks
  - Path: `benches/rendering.rs`
  - Benchmark: `full_redraw_100_messages` (<1ms target)
  - Benchmark: `streaming_token_append` (<100μs target)
  - Benchmark: `input_character_echo` (<10μs target)

### 7.2 Session Persistence

- [ ] 7.2.1 Write session save/load tests (RED)
  - Path: `tests/integration/session_test.rs`
  - Test: `test_session_save`
  - Test: `test_session_resume`

- [ ] 7.2.2 Implement session manager (GREEN)
  - Path: `src/session/mod.rs`

### 7.3 Multi-Model Support

- [ ] 7.3.1 Write provider switching tests (RED)
  - Test: `test_model_switching`
  - Test: `test_bedrock_provider`

- [ ] 7.3.2 Implement multi-model client (GREEN)
  - Path: `src/api/multi_model.rs`

### 7.4 Enterprise Features

- [ ] 7.4.1 Implement audit logging
  - Path: `src/enterprise/audit.rs`

- [ ] 7.4.2 Implement cost controls
  - Path: `src/enterprise/cost.rs`

### 7.5 Narsil Reindex Checkpoint

- [ ] 7.5.1 Run narsil reindex after Phase 7
  - Run: Full security scan
  - Run: Supply chain analysis

---

## Phase 8: Polish & Release

### Goal: Production-ready release

### 8.1 Documentation

- [ ] 8.1.1 Write user guide
- [ ] 8.1.2 Write API documentation
- [ ] 8.1.3 Write contributing guide
- [ ] 8.1.4 Write security policy

### 8.2 Distribution

- [ ] 8.2.1 Set up Homebrew formula
- [ ] 8.2.2 Set up apt/dnf packages
- [ ] 8.2.3 Set up WinGet/Scoop packages
- [ ] 8.2.4 Create Docker image

### 8.3 Auto-Update System

- [ ] 8.3.1 Write update check tests (RED)
  - Test: `test_auto_update_check`
  - Test: `test_auto_update_verify_signature`

- [ ] 8.3.2 Implement update manager (GREEN)
  - Path: `src/update/mod.rs`

### 8.4 Final Quality Gate

- [ ] 8.4.1 Run full test suite
- [ ] 8.4.2 Run narsil security scan
- [ ] 8.4.3 Run supply chain audit
- [ ] 8.4.4 Generate coverage report (>90% target)
- [ ] 8.4.5 Run performance benchmarks
- [ ] 8.4.6 Final narsil reindex

---

## Completed

<!-- Move completed tasks here with completion date -->

---

## Blocked

<!-- Document blockers with suggested actions -->

---

## Notes

### Narsil MCP Commands Reference

```bash
# Code Intelligence
reindex                      # Refresh code index
get_call_graph <function>    # Function relationships
find_references <symbol>     # Impact analysis
get_dependencies            # Module dependencies

# Security
scan_security               # Full security audit
find_injection_vulnerabilities  # SQL/XSS/command injection
check_cwe_top25             # CWE vulnerability check

# Analysis
get_type_hierarchy <type>   # Type inheritance
find_dead_code              # Unused code detection
get_complexity_report       # Cyclomatic complexity
```

### Ralph Operation

```bash
# Analyze project (do first)
ralph --project . analyze

# Run build loop (main operation)
ralph loop build --max-iterations 50

# Debug mode (slower, more verbose)
ralph loop debug --max-iterations 10
```

### Quality Checklist (Pre-Commit)

- [ ] `cargo clippy --all-targets -- -D warnings` passes
- [ ] `cargo test` passes
- [ ] `cargo fmt -- --check` passes
- [ ] No new `#[allow(...)]` attributes
- [ ] No `TODO:` or `FIXME:` comments
- [ ] No `todo!()` or `unimplemented!()`
- [ ] Public functions have doc comments
- [ ] New code has test coverage

---
