# Project Memory - RCT (Rust Claude Terminal)

## Project Type: Rust CLI Application

RCT is a high-performance Rust CLI for Claude API with a modular, extensible architecture.

**Primary Language:** Rust
**Target:** Feature parity with Claude Code + performance superiority

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
REINDEX → RED → GREEN → REFACTOR → REVIEW → COMMIT → REINDEX
```

1. **REINDEX**: Run `narsil reindex` to refresh code intelligence
2. **RED**: Write failing test first - test names describe expected behavior
3. **GREEN**: Write minimal code to make test pass
4. **REFACTOR**: Clean up while keeping tests green
5. **REVIEW**: Run all quality gates
6. **COMMIT**: Descriptive commit message
7. **REINDEX**: Refresh index with new code

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

Ralph requires `gh` CLI for all GitHub operations. **No SSH keys.**

```bash
# Verify gh CLI is authenticated
gh auth status

# If not authenticated:
gh auth login
```

### Git Safety

- NEVER use `--force` push unless explicitly requested
- NEVER skip hooks (`--no-verify`) unless explicitly requested
- NEVER amend commits unless explicitly requested
- Always create NEW commits after hook failures

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

### Detection Levels

| Level | Condition | Action |
|-------|-----------|--------|
| Warning | 3 iterations, no checkbox progress | Review blockers in IMPLEMENTATION_PLAN.md |
| Elevated | 5 iterations, no progress | Re-read task requirements, consider alternative approach |
| Critical | 8 iterations, no progress | Stop and document blocker, request human review |

### Recovery Steps

1. Check `IMPLEMENTATION_PLAN.md` for blocked tasks
2. Run tests to identify specific failures
3. Run linters to find warnings
4. Use narsil `get_call_graph` to understand dependencies
5. Check if task requires clarification
6. Document blocker and move task to Blocked section

---

## PROJECT STRUCTURE

```
rct/
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
├── docs/
│   └── TDD-PRODUCTION-PLAN.md  # Master development plan
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

### RCT-Specific Notes

**Current State (2026-01-29):**
- TUI, API streaming, basic input: WORKING
- Tools, MCP, hooks, skills, commands, agents, plugins: SCAFFOLDED
- Test coverage: 0%
- Primary goal: Reach feature parity with Claude Code

**Reference Documents:**
- `docs/TDD-PRODUCTION-PLAN.md` - Master 8-phase development plan
- `docs/architecture.md` - System architecture
- `docs/api.md` - API documentation

<!-- USER_CUSTOM_END -->
