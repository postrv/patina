# Project Memory - Patina

## Project Type: Rust CLI Application

Patina is a high-performance Rust terminal client for Claude API with a modular, extensible architecture.

**Primary Language:** Rust
**Target:** Feature parity with Claude Code + performance superiority
**Author:** Laurence Avent

---

## PRODUCTION STANDARDS

### Zero-Tolerance Policy

These patterns are **FORBIDDEN** in merged code:

```rust
#[allow(dead_code)]           // Wire in or delete
#[allow(unused_*)]            // Use or remove
#[allow(clippy::*)]           // Fix the issue
todo!()                       // Implement now
unimplemented!()              // Implement or remove
// TODO: ...                  // Implement now or don't merge
// FIXME: ...                 // Fix now or don't merge
panic!("not implemented")     // Implement or remove
```

### Required Patterns

```rust
#[must_use]                   // On functions returning values that should be used
/// # Panics                  // Document panic conditions
/// # Errors                  // Document error conditions
/// # Examples                // Provide usage examples for public APIs
#[cfg(test)]                  // Keep tests in modules
```

---

## Quality Gates

| Gate | Command | Requirement |
|------|---------|-------------|
| Clippy | `cargo clippy --all-targets -- -D warnings` | 0 warnings |
| Tests | `cargo test` | All pass |
| Format | `cargo fmt -- --check` | No changes |
| Security | narsil `scan_security` | 0 CRITICAL/HIGH |
| TDD | Tests BEFORE implementation | Required |

### Pre-Commit Checklist

**MANDATORY:**
- [ ] `cargo clippy --all-targets -- -D warnings` passes
- [ ] `cargo test` passes
- [ ] `cargo fmt -- --check` passes
- [ ] No forbidden patterns introduced
- [ ] New code has test coverage

**IF NARSIL AVAILABLE:**
- [ ] `scan_security` shows no new issues
- [ ] `reindex` run after significant changes

---

## TDD CYCLE

```
REINDEX → RED → COMMIT → GREEN → COMMIT → REFACTOR → REVIEW → COMMIT → REINDEX
```

1. **REINDEX**: Run `narsil reindex` to refresh code intelligence
2. **RED**: Write failing test first - test names describe expected behavior
3. **COMMIT**: Commit failing tests (documents intent, signals progress)
4. **GREEN**: Write minimal code to make test pass
5. **COMMIT**: Commit working implementation (checkpoint before refactor)
6. **REFACTOR**: Clean up while keeping tests green
7. **REVIEW**: Run all quality gates (clippy, tests, fmt)
8. **COMMIT**: Final commit with task marked complete
9. **REINDEX**: Refresh index with new code

**Key insight:** Commits at steps 3, 5, and 8 ensure Ralph always sees progress.

### Test Requirements

- Every public function: at least 1 test
- Every public type: exercised in tests
- Every error path: tested
- Use `#[should_panic]` for expected panics
- Use `#[cfg(test)]` modules for unit tests
- Integration tests go in `tests/` directory

---

## GIT AUTHENTICATION

### Required Setup

**HTTPS only. NEVER use SSH for git operations.**

Ralph requires `gh` CLI for all GitHub operations:

```bash
# Verify gh CLI is authenticated
gh auth status

# If not authenticated:
gh auth login

# Ensure git uses HTTPS (not SSH)
git remote -v  # Should show https:// URLs, NOT git@github.com
```

### Protocol Rules

| Action | Use | NEVER Use |
|--------|-----|-----------|
| Clone | `gh repo clone` or `https://` URL | `git@github.com:` |
| Push/Pull | `git push/pull` via HTTPS | SSH remotes |
| Auth | `gh auth login` | SSH keys |
| API calls | `gh api`, `gh pr`, `gh issue` | Direct curl with tokens |

### Git Safety

- NEVER use `--force` push unless explicitly requested
- NEVER skip hooks (`--no-verify`) unless explicitly requested
- NEVER amend commits unless explicitly requested
- Always create NEW commits after hook failures
- NEVER use SSH URLs or SSH keys for git operations

### Commit Cadence (CRITICAL for Ralph)

**Ralph detects stagnation by commit hash changes. No commits = stagnation detected.**

**MANDATORY commit points:**
1. **After each task completion** - When marking `[x]`, commit immediately
2. **After quality gates pass** - Don't proceed to next task without committing
3. **After RED phase** - Commit failing tests before implementing (documents intent)
4. **After GREEN phase** - Commit working implementation before refactoring

**Commit message format:**
```
feat(module): Brief description (task-number)

Co-Authored-By: Claude <noreply@anthropic.com>
```

**Example cadence for task 2.4.1:**
```bash
# RED: Write failing tests
git commit -m "test(continuous): Add ContinuousEvent tests (2.4.1 RED)"

# GREEN: Make tests pass
git commit -m "feat(continuous): Implement ContinuousEvent enum (2.4.1 GREEN)"

# Update checkbox and commit
git commit -m "docs: Mark 2.4.1 complete"
```

**Anti-patterns to avoid:**
- ❌ Completing multiple tasks before committing
- ❌ Running quality gates without committing passing code
- ❌ Moving to next task without committing current progress
- ❌ Large commits spanning multiple tasks

**Why this matters:**
- Ralph tracks `last_commit_hash` to detect progress
- No new commits after N iterations = stagnation warning
- Small, frequent commits = clear progress signal

---

## NARSIL-MCP INTEGRATION

### When Available

narsil-mcp provides code intelligence. Use gracefully - if unavailable, continue without it.

**Reindex Triggers:**
- After creating new files
- After significant refactors
- At start of each phase
- Before security scans

**Code Intelligence:**
```bash
reindex                          # Refresh code index
get_call_graph <function>        # Function relationships
find_references <symbol>         # Impact analysis
get_dependencies                 # Module dependencies
get_type_hierarchy <type>        # Type inheritance
find_dead_code                   # Unused code detection
get_complexity_report            # Cyclomatic complexity
```

**Security Analysis:**
```bash
scan_security                    # Full security audit
find_injection_vulnerabilities   # SQL/XSS/command injection
check_cwe_top25                  # CWE vulnerability check
```

### Graceful Degradation

If narsil-mcp is unavailable:
- Continue with standard tooling
- Use `cargo clippy` for lint analysis
- Use `cargo test` for verification
- Log warning but don't fail

---

## STAGNATION HANDLING

### Prevention (Most Important)

**Commit frequently to signal progress:**
- Commit after writing tests (RED phase)
- Commit after tests pass (GREEN phase)
- Commit after marking task complete
- If stuck for more than 10 minutes, commit partial progress with `WIP:` prefix

### Detection Levels

| Level | Condition | Action |
|-------|-----------|--------|
| Warning | 3 iterations, no commit | **Commit immediately** - even WIP progress |
| Elevated | 5 iterations, no progress | Re-read task requirements, consider alternative approach |
| Critical | 8 iterations, no progress | Stop and document blocker, request human review |

### Recovery Steps

1. **First: Commit any uncommitted work** (prevents false stagnation)
2. Check `IMPLEMENTATION_PLAN.md` for blocked tasks
3. Run tests to identify specific failures
4. Run linters to find warnings
5. Use narsil `get_call_graph` to understand dependencies
6. Check if task requires clarification
7. Document blocker and move task to Blocked section

---

## PROJECT STRUCTURE

```
patina/
├── src/
│   ├── main.rs          # Entry point
│   ├── app/             # Application state and event loop
│   ├── api/             # Anthropic API client
│   ├── tui/             # Terminal UI (ratatui)
│   ├── tools/           # Tool execution (bash, file ops)
│   ├── mcp/             # MCP protocol client
│   ├── hooks/           # Lifecycle hooks
│   ├── skills/          # Skill engine
│   ├── commands/        # Slash commands
│   ├── agents/          # Subagent orchestration
│   ├── plugins/         # Plugin system
│   ├── context/         # Project context loading
│   ├── update/          # Auto-update
│   ├── ide/             # IDE integration
│   └── util/            # Utilities
├── tests/
│   ├── unit/            # Unit tests
│   ├── integration/     # Integration tests
│   └── e2e/             # End-to-end tests
└── IMPLEMENTATION_PLAN.md      # Active task tracking
```

---

## IMPLEMENTATION PLAN TRACKING

Ralph reads `IMPLEMENTATION_PLAN.md` each iteration to select the next task.

### Task Format

```markdown
- [ ] N.M.X Task description (RED/GREEN/REFACTOR)
  - Path: `src/module/file.rs`
  - Test: `test_function_name`
  - Acceptance: What defines completion
```

### Progress Signals

- `[ ]` - Task pending
- `[x]` - Task completed
- Checkbox completion signals progress to loop
- Tasks are prioritized top-to-bottom within each phase

### Baseline Rules

- Test count must never decrease
- Clippy warnings must reach and stay at 0
- Update baseline metrics after each phase

---

## ARCHIVE POLICY

### What to Archive

- Completed implementation plans → `.archive/implementation-plans/`
- Outdated documentation → `.archive/docs/`

### Never Delete

- Test files (may contain important edge cases)
- Documentation (archive instead)
- Configuration files

---

## USER CUSTOMIZATIONS

Add project-specific notes below. This section is preserved during regeneration.

<!-- USER_CUSTOM_START -->

### Patina Project Notes

**Current State (2026-01-31):**
- **All Phases Complete:** TUI, API streaming, tools, MCP, hooks, skills, commands, agents, plugins
- **Test Coverage:** 85.84% (911 tests)
- **Cross-Platform:** Linux, macOS, Windows
- **Version:** 0.3.0

**Security Audit (2026-01-30) - ALL RESOLVED:**
| ID | Severity | Status | Issue | Resolution |
|----|----------|--------|-------|------------|
| C-1 | CRITICAL | ✅ FIXED | Bypassable bash command filter | Normalization + allowlist mode |
| H-1 | HIGH | ✅ FIXED | Plain string API key in multi_model | SecretString |
| H-2 | HIGH | ✅ FIXED | Unsandboxed hook execution | Dangerous command filtering |
| H-3 | HIGH | ✅ FIXED | list_files path traversal | validate_path() |
| M-1 | MEDIUM | ✅ FIXED | Unvalidated MCP commands | validate_mcp_command() |
| M-2 | MEDIUM | ✅ FIXED | TOCTOU race in path validation | Symlink rejection |
| L-1 | LOW | ✅ FIXED | Runtime regex compilation | once_cell::sync::Lazy |
| L-2 | LOW | ✅ FIXED | Session deserialization trust | HMAC-SHA256 integrity |

**Archived Plans:**
- `.archive/implementation-plans/` - Historical implementation plans

**Reference Documents:**
- `IMPLEMENTATION_PLAN.md` - Current development plan
- `docs/architecture.md` - System architecture
- `docs/api.md` - API documentation

<!-- USER_CUSTOM_END -->
