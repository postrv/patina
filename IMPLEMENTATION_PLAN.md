# Implementation Plan - Security Hardening & Quality Improvements

> Ralph uses this file to track task progress. Update checkboxes as work completes.

## Status: PHASE 3 IN PROGRESS

## Baseline Metrics (Updated: 2026-01-30)

| Metric | Value | Command |
|--------|-------|---------|
| Unit Tests | 193 | `cargo test --lib` |
| Integration Tests | 318 | `cargo test --test '*'` |
| Doc Tests | 20 | `cargo test --doc` |
| Total Tests | 598 | `cargo test` |
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

- [x] 0.1.1 Write path traversal tests for list_files (RED)
  - Path: `tests/tools.rs`
  - Test: `test_list_files_blocks_path_traversal`
  - Test: `test_list_files_blocks_absolute_path`
  - Test: `test_list_files_blocks_parent_escape`
  - Acceptance: All tests fail initially (no validation exists)

- [x] 0.1.2 Add validate_path call to list_files (GREEN)
  - Path: `src/tools/mod.rs:431-448`
  - Change: Add `validate_path()` call before `read_dir()`
  - Acceptance: All new tests pass

- [x] 0.1.3 Commit path traversal fix
  - Message: `fix(tools): Prevent path traversal in list_files`

### 0.2 Plain String API Keys (HIGH H-1)

- [x] 0.2.1 Write API key secrecy tests (RED)
  - Path: `tests/unit/multi_model_test.rs`
  - Test: `test_api_key_not_in_debug_output`
  - Test: `test_api_key_uses_secret_string`
  - Acceptance: Tests verify SecretString behavior

- [x] 0.2.2 Change api_key to SecretString (GREEN)
  - Path: `src/api/multi_model.rs:70-96`
  - Change: `api_key: String` → `api_key: secrecy::SecretString`
  - Update: All usages to call `.expose_secret()`
  - Acceptance: Tests pass, no API key in Debug output

- [x] 0.2.3 Commit SecretString fix
  - Message: `fix(api): Use SecretString for API keys in multi_model`

### 0.3 Unsandboxed Hook Execution (HIGH H-2)

- [x] 0.3.1 Write hook command validation tests (RED)
  - Path: `tests/integration/hooks_test.rs`
  - Test: `test_hook_blocks_rm_rf`
  - Test: `test_hook_blocks_sudo`
  - Test: `test_hook_blocks_curl_pipe_bash`
  - Test: `test_hook_allows_safe_commands`
  - Acceptance: Tests fail (no validation exists)

- [x] 0.3.2 Add dangerous command filtering to hooks (GREEN)
  - Path: `src/hooks/mod.rs:199-234`
  - Change: Reuse `ToolExecutionPolicy::dangerous_patterns`
  - Add: Validation before shell execution
  - Acceptance: All hook security tests pass

- [x] 0.3.3 Commit hook security fix
  - Message: `fix(hooks): Add dangerous command filtering to hook executor`

---

## Phase 1: High Security Fixes

### Goal: Address remaining high-priority security issues

### 1.1 Bash Command Filter Strengthening (CRITICAL C-1 - Mitigation)

- [x] 1.1.1 Write bypass attempt tests (RED)
  - Path: `tests/tools.rs`
  - Test: `test_bash_blocks_escaped_rm` (`r\m -rf /`)
  - Test: `test_bash_blocks_command_substitution` (`$(which rm) -rf /`)
  - Test: `test_bash_blocks_su_root` (`su root`)
  - Test: `test_bash_blocks_eval_variable` (`eval $dangerous`)
  - Acceptance: Tests demonstrate bypass vulnerabilities

- [x] 1.1.2 Implement enhanced command validation (GREEN)
  - Path: `src/tools/mod.rs`
  - Add: Normalize command before pattern matching (remove escapes)
  - Add: Block command substitution patterns
  - Add: More comprehensive privilege escalation patterns
  - Acceptance: All bypass tests pass

- [x] 1.1.3 Add allowlist mode option (GREEN)
  - Path: `src/tools/mod.rs`
  - Add: `ToolExecutionPolicy::allowlist_mode: bool`
  - Add: `ToolExecutionPolicy::allowed_commands: Vec<Regex>`
  - Add: When allowlist_mode=true, only allow matching commands
  - Acceptance: Allowlist tests pass

- [x] 1.1.4 Document security model (REFACTOR)
  - Path: `docs/security-model.md`
  - Document: Blocklist vs allowlist tradeoffs
  - Document: How to enable strict mode
  - Document: Known limitations

- [x] 1.1.5 Commit enhanced bash security
  - Message: `feat(tools): Enhance bash command security with allowlist mode`

### 1.2 MCP Command Validation (MEDIUM M-1)

- [x] 1.2.1 Write MCP command validation tests (RED)
  - Path: `tests/integration/mcp_test.rs`
  - Test: `test_mcp_blocks_dangerous_command`
  - Test: `test_mcp_blocks_sudo_command`
  - Test: `test_mcp_requires_absolute_path`
  - Test: `test_mcp_blocks_path_traversal`
  - Test: `test_mcp_warns_on_new_server`
  - Test: `test_mcp_allows_valid_absolute_path`
  - Test: `test_mcp_blocks_shell_injection_in_args`
  - Acceptance: 4 security tests fail (no validation exists yet)

- [x] 1.2.2 Add MCP command validation (GREEN)
  - Path: `src/mcp/client.rs`
  - Add: `validate_mcp_command()` function with security checks
  - Add: Block dangerous commands (rm, sudo, dd, etc.) even with absolute paths
  - Add: Require absolute paths for interpreters (bash, python, etc.)
  - Add: Block path traversal and relative paths
  - Add: Block shell injection patterns in non-interpreter arguments
  - Acceptance: All 9 MCP security tests pass

- [x] 1.2.3 Commit MCP validation
  - Message: `feat(mcp): Add command validation to MCP transport`
  - Commit: 63aced4

### 1.3 TOCTOU Mitigation (MEDIUM M-2)

- [x] 1.3.1 Write symlink race condition tests (RED)
  - Path: `tests/tools.rs`
  - Test: `test_file_read_rejects_symlinks`
  - Test: `test_file_write_rejects_symlinks`
  - Test: `test_edit_rejects_symlinks`
  - Test: `test_file_read_rejects_internal_symlinks` (added for defense in depth)
  - Acceptance: Tests verify symlink handling

- [x] 1.3.2 Add symlink detection to file operations (GREEN)
  - Path: `src/tools/mod.rs`
  - Add: `check_symlink()` helper using `symlink_metadata()`
  - Add: Reject ALL symlinks uniformly for defense in depth
  - Acceptance: All symlink tests pass

- [x] 1.3.3 Commit TOCTOU mitigation
  - Message: `fix(tools): Reject symlinks in file operations`
  - Commit: d54046f

---

## Phase 2: Medium Security & Code Quality

### Goal: Address medium-priority issues and code quality

### 2.1 Regex Pattern Safety

- [x] 2.1.1 Use lazy_static for regex patterns (REFACTOR)
  - Path: `src/tools/mod.rs:40-75`
  - Change: Move patterns to `once_cell::sync::Lazy` static
  - Benefit: Compile-time validation, no runtime panics
  - Acceptance: Clippy clean, tests pass

- [x] 2.1.2 Commit regex refactor
  - Message: `refactor(tools): Use once_cell Lazy for dangerous patterns`
  - Commit: b5576ae

### 2.2 Plugin file_stem Safety

- [x] 2.2.1 Write plugin path edge case tests (RED)
  - Path: `tests/unit/plugins_test.rs`
  - Test: `test_plugin_handles_no_extension`
  - Test: `test_plugin_handles_dotfile`
  - Acceptance: Tests cover edge cases

- [x] 2.2.2 Fix unsafe file_stem unwrap (GREEN)
  - Path: `src/plugins/mod.rs:160`
  - Change: `unwrap()` → `unwrap_or_else()` with default
  - Acceptance: Tests pass, no panic possible

- [x] 2.2.3 Commit plugin safety fix
  - Message: `fix(plugins): Handle edge cases in plugin path parsing`
  - Commit: bd3b51f

### 2.3 Session Integrity

- [x] 2.3.1 Write session integrity tests (RED)
  - Path: `tests/integration/session_test.rs`
  - Test: `test_session_detects_tampering`
  - Test: `test_session_validates_schema`
  - Acceptance: Tests verify integrity checking

- [x] 2.3.2 Add session checksum validation (GREEN)
  - Path: `src/session/mod.rs`
  - Add: HMAC-SHA256 checksum on session files
  - Add: Validation on load
  - Acceptance: Tampered sessions rejected

- [x] 2.3.3 Commit session integrity
  - Message: `feat(session): Add integrity checking to session files`
  - Commit: 7607275

---

## Phase 3: Test Coverage Expansion

### Goal: Increase coverage to 90%+ with focus on error paths

### 3.1 Error Path Tests for Tools

- [x] 3.1.1 Write file operation error tests (RED)
  - Path: `tests/tools.rs`
  - Test: `test_read_file_permission_denied`
  - Test: `test_write_file_to_readonly_directory`
  - Test: `test_edit_file_no_read_permission`
  - Test: `test_edit_file_no_write_permission`
  - Test: `test_bash_timeout_kills_process`
  - Test: `test_read_file_large_file`
  - Test: `test_write_file_exceeds_size_limit`
  - Test: `test_list_files_nonexistent_directory`
  - Acceptance: Error paths properly tested

- [x] 3.1.2 Implement any missing error handling (GREEN)
  - Path: `src/tools/mod.rs`
  - Fixed: bash timeout now properly kills child process (kill_on_drop)
  - Fixed: list_files returns ToolResult::Error for nonexistent directories
  - Acceptance: All error tests pass

### 3.2 Error Path Tests for API

- [x] 3.2.1 Write API error tests (RED)
  - Path: `tests/api_client.rs`
  - Test: `test_api_network_timeout`
  - Test: `test_api_invalid_json_response`
  - Test: `test_api_rate_limit_retry`
  - Test: `test_api_server_error_retry`
  - Acceptance: Network errors properly handled

- [x] 3.2.2 Verify retry logic (GREEN)
  - Path: `src/api/mod.rs`
  - Verify: Retry on 429, 5xx errors
  - Verify: Exponential backoff implemented
  - Acceptance: Retry tests pass

### 3.3 Error Path Tests for MCP

- [x] 3.3.1 Write MCP transport error tests (RED)
  - Path: `tests/integration/mcp_transport_test.rs`
  - Test: `test_stdio_process_crash`
  - Test: `test_stdio_invalid_json`
  - Test: `test_sse_connection_lost`
  - Test: `test_http_timeout`
  - Acceptance: Transport errors handled

- [x] 3.3.2 Implement error recovery (GREEN)
  - Path: `src/mcp/transport.rs`
  - Verified: Graceful handling of process crashes (test_stdio_process_crash passes)
  - Verified: Invalid JSON handling in stdio transport (test_stdio_invalid_json passes)
  - Verified: SSE connection loss handling (test_sse_connection_lost passes)
  - Verified: HTTP timeout handling (test_http_timeout passes)
  - Note: Automatic reconnection not implemented (would require additional feature)
  - Acceptance: Error tests pass

### 3.4 Error Path Tests for Session

- [x] 3.4.1 Write session error tests (RED)
  - Path: `tests/integration/session_test.rs`
  - Test: `test_session_load_corrupted_json`
  - Test: `test_session_save_permission_denied`
  - Test: `test_session_concurrent_access`
  - Acceptance: Session errors handled

- [x] 3.4.2 Implement error recovery (GREEN)
  - Path: `src/session/mod.rs`
  - Verified: Schema validation on load (serde deserialization)
  - Verified: Integrity checking with HMAC-SHA256 (SessionFile wrapper)
  - Verified: Concurrent access handled gracefully (atomic file writes)
  - Note: File locking not implemented (tokio file ops are already atomic)
  - Acceptance: Error tests pass

### 3.5 TUI Functional Tests

- [x] 3.5.1 Write TUI rendering tests (RED)
  - Path: `tests/unit/tui_snapshot_test.rs`
  - Test: `test_tui_handles_unicode`
  - Test: `test_tui_unicode_input`
  - Test: `test_tui_scrolls_long_content`
  - Test: `test_tui_input_cursor_visible`
  - Test: `test_tui_cursor_movement`
  - Acceptance: TUI logic verified

- [x] 3.5.2 Write TUI event tests (RED)
  - Path: `tests/unit/tui_snapshot_test.rs`
  - Test: `test_tui_key_events`
  - Test: `test_tui_resize_event`
  - Test: `test_tui_paste_event`
  - Test: `test_dirty_flags`
  - Acceptance: Event handling verified

### 3.6 Concurrency Tests

- [x] 3.6.1 Write concurrent tool execution tests (RED)
  - Path: `tests/tools.rs`
  - Test: `test_parallel_file_operations`
  - Test: `test_parallel_bash_commands`
  - Test: `test_parallel_tool_calls`
  - Acceptance: No race conditions

- [x] 3.6.2 Write concurrent session tests (RED)
  - Path: `tests/integration/session_test.rs` (completed in 3.4.1)
  - Test: `test_session_concurrent_access` (writes)
  - Test: `test_session_concurrent_reads` (reads)
  - Acceptance: Session thread-safe

---

## Phase 4: Error Handling Hardening

### Goal: Ensure robust error handling throughout

### 4.1 Consistent Error Types

- [x] 4.1.1 Create error types module (REFACTOR)
  - Path: `src/error.rs`
  - Add: `RctError` enum with variants for each module
  - Add: Proper `Display` and `Error` implementations
  - Add: Conversion from anyhow errors
  - Commit: 6c69842

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

### 2026-01-30
- [x] 0.1.1-0.1.3 Path Traversal fix for list_files (H-3)
  - Added validate_path() to list_files
  - 3 security tests added
  - Commit: 953e40d

- [x] 0.2.1-0.2.3 SecretString for API keys (H-1)
  - Changed api_key: String to SecretString in multi_model
  - Custom Debug impl shows [REDACTED]
  - 4 security tests added
  - Commit: b9d8301

- [x] 0.3.1-0.3.3 Hook command security filtering (H-2)
  - Added dangerous command validation to run_hook_command
  - Reuses ToolExecutionPolicy::dangerous_patterns
  - 4 security tests added (blocks rm -rf, sudo, curl|bash, allows safe)
  - Commit: cd53187

- [x] 1.1.1-1.1.5 Bash Command Filter Strengthening (C-1 Mitigation)
  - Added command normalization for escape bypass detection (r\m -> rm)
  - Expanded privilege escalation patterns (su root, pkexec, runuser)
  - Added command substitution detection ($(...), backticks)
  - Added encoded command detection (base64, hex via printf)
  - Implemented allowlist mode for strict security environments
  - Created docs/security-model.md documentation
  - 13 security tests added
  - Commit: b430478

- [x] 1.2.1-1.2.3 MCP Command Validation (M-1)
  - Added validate_mcp_command() to src/mcp/client.rs
  - Blocks dangerous commands (rm, sudo, dd, etc.) even with absolute paths
  - Requires absolute paths for interpreters (bash, python, etc.)
  - Blocks path traversal and relative paths
  - Validates arguments for shell injection on non-interpreters
  - 9 security tests added
  - Commit: 63aced4

- [x] 1.3.1-1.3.3 TOCTOU Mitigation - Symlink Rejection (M-2)
  - Added check_symlink() helper to src/tools/mod.rs
  - Uses symlink_metadata() to check paths without following symlinks
  - Rejects ALL symlinks uniformly for defense in depth (both internal and external)
  - Added symlink checks to read_file, write_file, and edit_file operations
  - 4 security tests added
  - Commit: d54046f

- [x] 2.1.1-2.1.2 Regex Pattern Safety (L-1)
  - Refactored dangerous_patterns to use once_cell::sync::Lazy
  - Patterns compile once on first access, not on every Default impl call
  - Eliminates runtime panics from invalid regex after initialization
  - Commit: b5576ae

- [x] 2.2.1-2.2.3 Plugin file_stem Safety
  - Replaced unwrap() with unwrap_or_else() on file_stem()
  - Added tests for files without extensions, dotfiles, and edge cases
  - 2 new tests added
  - Commit: bd3b51f

- [x] 2.3.1-2.3.3 Session Integrity (L-2)
  - Added HMAC-SHA256 checksum to session files via SessionFile wrapper
  - Checksum computed on save, verified on load
  - Tampered sessions are rejected with integrity check failure
  - 2 new tests added
  - Commit: 7607275

- [x] 3.1.1-3.1.2 Error Path Tests for Tools
  - Added 8 error path tests: permission denied, read-only dir, no permissions, timeout
  - Fixed bash timeout to properly kill child process (kill_on_drop)
  - Fixed list_files to return ToolResult::Error for nonexistent directories
  - Tests: permission handling, size limits, boundary conditions
  - 8 new tests added
  - Commit: 34bd99d

- [x] 3.2.1-3.2.2 Error Path Tests for API
  - Added test_api_network_timeout: verifies connection errors are properly propagated
  - Added test_api_invalid_json_response: verifies invalid JSON is gracefully skipped
  - Renamed test_retry_on_rate_limit → test_api_rate_limit_retry
  - Renamed test_retry_on_server_error → test_api_server_error_retry
  - Verified retry logic: MAX_RETRIES=2, exponential backoff (100ms, 200ms, 400ms)
  - Verified retryable statuses: 429 (rate limit), 5xx (server errors)
  - 2 new tests added

- [x] 3.3.1-3.3.2 Error Path Tests for MCP Transport
  - Added test_stdio_invalid_json: verifies transport handles invalid JSON gracefully
  - Added test_stdio_process_crash: verifies transport handles process crashes
  - Added test_sse_connection_lost: verifies SSE handles endpoint errors
  - Added test_http_timeout: verifies SSE handles slow POST responses
  - Added test_sse_invalid_json_response: verifies SSE handles invalid JSON
  - Verified error recovery: timeouts, crashes, invalid responses handled gracefully
  - 5 new tests added

- [x] 3.4.1-3.4.2 Error Path Tests for Session
  - Added test_session_load_corrupted_json: verifies corrupted JSON is rejected
  - Added test_session_save_permission_denied: verifies permission errors handled
  - Added test_session_concurrent_access: verifies concurrent writes don't corrupt
  - Added test_session_concurrent_reads: verifies concurrent reads work
  - Verified error recovery: integrity checking, schema validation, atomic writes
  - 4 new tests added

- [x] 3.5.1-3.5.2 TUI Functional Tests
  - Added test_tui_handles_unicode: verifies unicode messages render correctly
  - Added test_tui_unicode_input: verifies unicode input handling
  - Added test_tui_scrolls_long_content: verifies scroll behavior
  - Added test_tui_input_cursor_visible: verifies cursor tracking
  - Added test_tui_cursor_movement: verifies cursor navigation
  - Added test_tui_key_events: verifies key event processing
  - Added test_tui_resize_event: verifies resize handling
  - Added test_tui_paste_event: verifies paste behavior
  - Added test_dirty_flags: verifies dirty flag system
  - 9 new tests added

- [x] 3.6.1-3.6.2 Concurrency Tests
  - Added test_parallel_file_operations: verifies parallel file reads/writes
  - Added test_parallel_bash_commands: verifies parallel bash execution
  - Added test_parallel_tool_calls: verifies parallel tool executor calls
  - Session concurrent tests already covered in 3.4.1
  - 3 new tests added (+ 2 from 3.4.1)

- [x] 4.1.1 Create error types module (REFACTOR)
  - Created `src/error.rs` with centralized `RctError` enum
  - Added variants for all module error categories:
    - Tool: PathTraversal, PermissionDenied, Timeout, SecurityViolation
    - API: Network, RateLimited, Authentication, InvalidResponse
    - MCP: Transport, Validation, Protocol
    - Session: Integrity, Io, Validation
    - Hook: Validation, Execution
    - Plugin: Load, Execution
    - Context: Io
  - Implemented Display, Error traits with consistent formatting
  - Added category methods: is_retryable(), is_security_related(), module()
  - Added From<anyhow::Error> conversion for gradual migration
  - 34 new tests added (29 in error_test.rs + 5 in module)
  - Commit: 6c69842

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
