# Implementation Plan - Security Hardening & Quality Improvements

> Ralph uses this file to track task progress. Update checkboxes as work completes.

## Status: PHASE 0 IN PROGRESS

## Baseline Metrics (Updated: 2026-01-30)

| Metric | Value | Command |
|--------|-------|---------|
| Unit Tests | 182 | `cargo test --lib` |
| Integration Tests | 278 | `cargo test --test '*'` |
| Doc Tests | 19 | `cargo test --doc` |
| Total Tests | 491 | `cargo test` |
| Test Files | 33 | `find tests -name '*.rs' \| wc -l` |
| Clippy Warnings | 0 | `cargo clippy --all-targets -- -D warnings` |
| Source Files | 30 | `find src -name '*.rs' \| wc -l` |
| LOC | ~7900 | `wc -l src/**/*.rs` |
| Coverage | 84.38% | `cargo tarpaulin --out Stdout` |

**Baseline Rule:** Test count must never decrease. Clippy warnings must stay at 0.

---

## Quality Gates

| Gate | Command | Requirement |
|------|---------|-------------|
| Clippy | `cargo clippy --all-targets -- -D warnings` | 0 warnings |
| Tests | `cargo test` | All pass |
| Format | `cargo fmt -- --check` | No changes needed |
| Security | `cargo audit` | 0 CRITICAL/HIGH in direct deps |
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
| 0 | Critical Security Fixes | P0 | 8 |
| 1 | High Security Fixes | P0 | 12 |
| 2 | Medium Security Fixes | P1 | 6 |
| 3 | Test Coverage Expansion | P1 | 15 |
| 4 | Error Handling Hardening | P2 | 10 |
| 5 | Final Security Audit | P0 | 4 |

---

## Phase 0: Critical Security Fixes

### Goal: Fix the most severe security vulnerabilities

### 0.1 Path Traversal in list_files (HIGH H-3)

- [ ] 0.1.1 Write path traversal tests for list_files (RED)
  - Path: `tests/tools.rs`
  - Test: `test_list_files_blocks_path_traversal`
  - Test: `test_list_files_blocks_absolute_path`
  - Test: `test_list_files_blocks_parent_escape`
  - Acceptance: All tests fail initially (no validation exists)

- [ ] 0.1.2 Add validate_path call to list_files (GREEN)
  - Path: `src/tools/mod.rs:431-448`
  - Change: Add `validate_path()` call before `read_dir()`
  - Acceptance: All new tests pass

- [ ] 0.1.3 Commit path traversal fix
  - Message: `fix(tools): Prevent path traversal in list_files`

### 0.2 Plain String API Keys (HIGH H-1)

- [ ] 0.2.1 Write API key secrecy tests (RED)
  - Path: `tests/unit/multi_model_test.rs`
  - Test: `test_api_key_not_in_debug_output`
  - Test: `test_api_key_uses_secret_string`
  - Acceptance: Tests verify SecretString behavior

- [ ] 0.2.2 Change api_key to SecretString (GREEN)
  - Path: `src/api/multi_model.rs:70-96`
  - Change: `api_key: String` → `api_key: secrecy::SecretString`
  - Update: All usages to call `.expose_secret()`
  - Acceptance: Tests pass, no API key in Debug output

- [ ] 0.2.3 Commit SecretString fix
  - Message: `fix(api): Use SecretString for API keys in multi_model`

### 0.3 Unsandboxed Hook Execution (HIGH H-2)

- [ ] 0.3.1 Write hook command validation tests (RED)
  - Path: `tests/integration/hooks_test.rs`
  - Test: `test_hook_blocks_rm_rf`
  - Test: `test_hook_blocks_sudo`
  - Test: `test_hook_blocks_curl_pipe_bash`
  - Test: `test_hook_allows_safe_commands`
  - Acceptance: Tests fail (no validation exists)

- [ ] 0.3.2 Add dangerous command filtering to hooks (GREEN)
  - Path: `src/hooks/mod.rs:199-234`
  - Change: Reuse `ToolExecutionPolicy::dangerous_patterns`
  - Add: Validation before shell execution
  - Acceptance: All hook security tests pass

- [ ] 0.3.3 Commit hook security fix
  - Message: `fix(hooks): Add dangerous command filtering to hook executor`

---

## Phase 1: High Security Fixes

### Goal: Address remaining high-priority security issues

### 1.1 Bash Command Filter Strengthening (CRITICAL C-1 - Mitigation)

- [ ] 1.1.1 Write bypass attempt tests (RED)
  - Path: `tests/tools.rs`
  - Test: `test_bash_blocks_escaped_rm` (`r\m -rf /`)
  - Test: `test_bash_blocks_command_substitution` (`$(which rm) -rf /`)
  - Test: `test_bash_blocks_su_root` (`su root`)
  - Test: `test_bash_blocks_eval_variable` (`eval $dangerous`)
  - Acceptance: Tests demonstrate bypass vulnerabilities

- [ ] 1.1.2 Implement enhanced command validation (GREEN)
  - Path: `src/tools/mod.rs`
  - Add: Normalize command before pattern matching (remove escapes)
  - Add: Block command substitution patterns
  - Add: More comprehensive privilege escalation patterns
  - Acceptance: All bypass tests pass

- [ ] 1.1.3 Add allowlist mode option (GREEN)
  - Path: `src/tools/mod.rs`
  - Add: `ToolExecutionPolicy::allowlist_mode: bool`
  - Add: `ToolExecutionPolicy::allowed_commands: Vec<Regex>`
  - Add: When allowlist_mode=true, only allow matching commands
  - Acceptance: Allowlist tests pass

- [ ] 1.1.4 Document security model (REFACTOR)
  - Path: `docs/security-model.md`
  - Document: Blocklist vs allowlist tradeoffs
  - Document: How to enable strict mode
  - Document: Known limitations

- [ ] 1.1.5 Commit enhanced bash security
  - Message: `feat(tools): Enhance bash command security with allowlist mode`

### 1.2 MCP Command Validation (MEDIUM M-1)

- [ ] 1.2.1 Write MCP command validation tests (RED)
  - Path: `tests/integration/mcp_test.rs`
  - Test: `test_mcp_blocks_dangerous_command`
  - Test: `test_mcp_requires_absolute_path`
  - Test: `test_mcp_warns_on_new_server`
  - Acceptance: Tests fail initially

- [ ] 1.2.2 Add MCP command validation (GREEN)
  - Path: `src/mcp/transport.rs`
  - Add: Validate command path exists
  - Add: Warn on first use of new server
  - Add: Block dangerous patterns
  - Acceptance: All MCP security tests pass

- [ ] 1.2.3 Commit MCP validation
  - Message: `feat(mcp): Add command validation to MCP transport`

### 1.3 TOCTOU Mitigation (MEDIUM M-2)

- [ ] 1.3.1 Write symlink race condition tests (RED)
  - Path: `tests/tools.rs`
  - Test: `test_file_read_rejects_symlinks`
  - Test: `test_file_write_rejects_symlinks`
  - Test: `test_edit_rejects_symlinks`
  - Acceptance: Tests verify symlink handling

- [ ] 1.3.2 Add symlink detection to file operations (GREEN)
  - Path: `src/tools/mod.rs`
  - Add: Check `full_path.is_symlink()` before operations
  - Add: Error if path is symlink pointing outside working dir
  - Acceptance: All symlink tests pass

- [ ] 1.3.3 Commit TOCTOU mitigation
  - Message: `fix(tools): Reject symlinks in file operations`

---

## Phase 2: Medium Security & Code Quality

### Goal: Address medium-priority issues and code quality

### 2.1 Regex Pattern Safety

- [ ] 2.1.1 Use lazy_static for regex patterns (REFACTOR)
  - Path: `src/tools/mod.rs:40-75`
  - Change: Move patterns to `lazy_static!` block
  - Benefit: Compile-time validation, no runtime panics
  - Acceptance: Clippy clean, tests pass

- [ ] 2.1.2 Commit regex refactor
  - Message: `refactor(tools): Use lazy_static for dangerous patterns`

### 2.2 Plugin file_stem Safety

- [ ] 2.2.1 Write plugin path edge case tests (RED)
  - Path: `tests/unit/plugins_test.rs`
  - Test: `test_plugin_handles_no_extension`
  - Test: `test_plugin_handles_dotfile`
  - Acceptance: Tests cover edge cases

- [ ] 2.2.2 Fix unsafe file_stem unwrap (GREEN)
  - Path: `src/plugins/mod.rs:160`
  - Change: `unwrap()` → `unwrap_or_else()` with default
  - Acceptance: Tests pass, no panic possible

- [ ] 2.2.3 Commit plugin safety fix
  - Message: `fix(plugins): Handle edge cases in plugin path parsing`

### 2.3 Session Integrity

- [ ] 2.3.1 Write session integrity tests (RED)
  - Path: `tests/integration/session_test.rs`
  - Test: `test_session_detects_tampering`
  - Test: `test_session_validates_schema`
  - Acceptance: Tests verify integrity checking

- [ ] 2.3.2 Add session checksum validation (GREEN)
  - Path: `src/session/mod.rs`
  - Add: HMAC signature on session files
  - Add: Validation on load
  - Acceptance: Tampered sessions rejected

- [ ] 2.3.3 Commit session integrity
  - Message: `feat(session): Add integrity checking to session files`

---

## Phase 3: Test Coverage Expansion

### Goal: Increase coverage to 90%+ with focus on error paths

### 3.1 Error Path Tests for Tools

- [ ] 3.1.1 Write file operation error tests (RED)
  - Path: `tests/tools.rs`
  - Test: `test_read_file_permission_denied`
  - Test: `test_write_file_disk_full` (mock)
  - Test: `test_edit_file_locked`
  - Test: `test_bash_timeout_kills_process`
  - Acceptance: Error paths properly tested

- [ ] 3.1.2 Implement any missing error handling (GREEN)
  - Path: `src/tools/mod.rs`
  - Verify: All error paths return proper ToolResult::Error
  - Acceptance: All error tests pass

### 3.2 Error Path Tests for API

- [ ] 3.2.1 Write API error tests (RED)
  - Path: `tests/api_client.rs`
  - Test: `test_api_network_timeout`
  - Test: `test_api_invalid_json_response`
  - Test: `test_api_rate_limit_retry`
  - Test: `test_api_server_error_retry`
  - Acceptance: Network errors properly handled

- [ ] 3.2.2 Verify retry logic (GREEN)
  - Path: `src/api/mod.rs`
  - Verify: Retry on 429, 5xx errors
  - Verify: Exponential backoff implemented
  - Acceptance: Retry tests pass

### 3.3 Error Path Tests for MCP

- [ ] 3.3.1 Write MCP transport error tests (RED)
  - Path: `tests/integration/mcp_transport_test.rs`
  - Test: `test_stdio_process_crash`
  - Test: `test_stdio_invalid_json`
  - Test: `test_sse_connection_lost`
  - Test: `test_http_timeout`
  - Acceptance: Transport errors handled

- [ ] 3.3.2 Implement error recovery (GREEN)
  - Path: `src/mcp/transport.rs`
  - Add: Automatic reconnection for SSE
  - Add: Graceful handling of process crashes
  - Acceptance: Error tests pass

### 3.4 Error Path Tests for Session

- [ ] 3.4.1 Write session error tests (RED)
  - Path: `tests/integration/session_test.rs`
  - Test: `test_session_load_corrupted_json`
  - Test: `test_session_save_permission_denied`
  - Test: `test_session_concurrent_access`
  - Acceptance: Session errors handled

- [ ] 3.4.2 Implement error recovery (GREEN)
  - Path: `src/session/mod.rs`
  - Add: Schema validation on load
  - Add: File locking for concurrent access
  - Acceptance: Error tests pass

### 3.5 TUI Functional Tests

- [ ] 3.5.1 Write TUI rendering tests (RED)
  - Path: `tests/unit/tui_test.rs`
  - Test: `test_tui_renders_messages_correctly`
  - Test: `test_tui_handles_unicode`
  - Test: `test_tui_scrolls_long_content`
  - Test: `test_tui_input_cursor_visible`
  - Acceptance: TUI logic verified

- [ ] 3.5.2 Write TUI event tests (RED)
  - Path: `tests/unit/tui_test.rs`
  - Test: `test_tui_key_events`
  - Test: `test_tui_resize_event`
  - Test: `test_tui_paste_event`
  - Acceptance: Event handling verified

### 3.6 Concurrency Tests

- [ ] 3.6.1 Write concurrent tool execution tests (RED)
  - Path: `tests/integration/concurrency_test.rs`
  - Test: `test_parallel_file_operations`
  - Test: `test_parallel_bash_commands`
  - Test: `test_parallel_mcp_calls`
  - Acceptance: No race conditions

- [ ] 3.6.2 Write concurrent session tests (RED)
  - Test: `test_concurrent_session_writes`
  - Test: `test_concurrent_session_reads`
  - Acceptance: Session thread-safe

---

## Phase 4: Error Handling Hardening

### Goal: Ensure robust error handling throughout

### 4.1 Consistent Error Types

- [ ] 4.1.1 Create error types module (REFACTOR)
  - Path: `src/error.rs`
  - Add: `RctError` enum with variants for each module
  - Add: Proper `Display` and `Error` implementations
  - Add: Conversion from anyhow errors

- [ ] 4.1.2 Update modules to use error types
  - Update: `src/tools/mod.rs`
  - Update: `src/mcp/mod.rs`
  - Update: `src/session/mod.rs`
  - Acceptance: Consistent error handling

### 4.2 Error Recovery

- [ ] 4.2.1 Add graceful degradation for non-critical failures
  - Path: Various modules
  - Add: Fallback behavior when optional features fail
  - Add: Clear error messages for users
  - Acceptance: App doesn't crash on recoverable errors

### 4.3 Error Logging

- [ ] 4.3.1 Ensure all errors are logged appropriately
  - Add: `tracing::error!` for critical failures
  - Add: `tracing::warn!` for recoverable issues
  - Verify: No silent failures
  - Acceptance: Errors traceable in logs

---

## Phase 5: Final Security Audit

### Goal: Verify all security issues are resolved

### 5.1 Security Verification

- [ ] 5.1.1 Run comprehensive security scan
  - Command: `cargo audit`
  - Verify: 0 CRITICAL/HIGH in direct dependencies
  - Document: Any remaining transitive dependency issues

- [ ] 5.1.2 Run all security-focused tests
  - Command: `cargo test security`
  - Verify: All security tests pass
  - Note: Tag security tests with `#[test]` naming convention

- [ ] 5.1.3 Manual penetration testing
  - Test: All path traversal vectors
  - Test: All command injection vectors
  - Test: All privilege escalation vectors
  - Document: Any remaining issues

- [ ] 5.1.4 Generate final security report
  - Path: `docs/SECURITY_AUDIT.md`
  - Include: All findings and resolutions
  - Include: Known limitations
  - Include: Security recommendations

### 5.2 Final Quality Gate

- [ ] 5.2.1 Run full test suite
  - Command: `cargo test`
  - Target: >550 tests
  - Verify: All pass

- [ ] 5.2.2 Run coverage report
  - Command: `cargo tarpaulin`
  - Target: >90% coverage
  - Document: Any intentionally untested code

- [ ] 5.2.3 Final code review
  - Verify: No forbidden patterns
  - Verify: All public APIs documented
  - Verify: No security regressions

- [ ] 5.2.4 Tag release
  - Tag: `v0.2.0-security`
  - Message: Security hardening release

---

## Completed

<!-- Move completed tasks here with completion date -->

---

## Blocked

<!-- Document blockers with suggested actions -->

---

## Notes

### Security Issue Reference

| ID | Severity | Module | Issue | Phase |
|----|----------|--------|-------|-------|
| H-3 | HIGH | tools | list_files path traversal | 0.1 |
| H-1 | HIGH | api | Plain string API key | 0.2 |
| H-2 | HIGH | hooks | Unsandboxed execution | 0.3 |
| C-1 | CRITICAL | tools | Bypassable bash filter | 1.1 |
| M-1 | MEDIUM | mcp | Unvalidated commands | 1.2 |
| M-2 | MEDIUM | tools | TOCTOU race | 1.3 |
| L-1 | LOW | tools | Runtime regex compilation | 2.1 |
| L-2 | LOW | session | Deserialization trust | 2.3 |

### Testing Commands

```bash
# Run all tests
cargo test

# Run only security tests
cargo test --test '*' -- security

# Run with coverage
cargo tarpaulin --out Html

# Run security audit
cargo audit

# Run clippy
cargo clippy --all-targets -- -D warnings
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
- [ ] Security tests added for security-sensitive changes

---
