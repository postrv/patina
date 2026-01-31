# Patina Implementation Plan

> Ralph uses this file to track task progress. Update checkboxes as work completes.

## Status: PHASE 8 ACTIVE

## Current State Summary

**Patina** has achieved feature parity with Claude Code:

| Metric | Value |
|--------|-------|
| Tests | 922 |
| Coverage | 85%+ |
| Clippy Warnings | 0 |
| Security Findings | 0 (8/8 resolved) |
| Unmaintained Deps | 2 (transitive from syntect) |
| Unsafe Blocks | 0 |

**Completed Phases (1-6):**
- Phase 0: Project scaffold, CI/CD
- Phase 1: Core TUI, streaming, Anthropic API
- Phase 2: Configuration, markdown rendering
- Phase 3: Tool execution, security sandboxing
- Phase 4: Session persistence, HMAC integrity
- Phase 5: Skills, hooks, slash commands, MCP
- Phase 6: Cross-platform (Linux, macOS, Windows)

---

## Quality Gates

| Gate | Command | Requirement |
|------|---------|-------------|
| Clippy | `cargo clippy --all-targets -- -D warnings` | 0 warnings |
| Tests | `cargo test` | All pass |
| Format | `cargo fmt -- --check` | No changes |
| TDD | Tests BEFORE implementation | Required |

---

## Phase 7: v0.3.0 Release & Rebrand to Patina (MOSTLY COMPLETE)

**Objective:** Ship public release under new "Patina" branding
**Status:** Rebrand complete, GitHub release published. Distribution & announcements pending.

### 7.1 Rebrand to Patina

- [x] 7.1.1 Update Cargo.toml
  - Path: `Cargo.toml`
  - Change: `name = "rct"` to `name = "patina"`
  - Change: description to reference Patina
  - Acceptance: `cargo build` produces `patina` binary

- [x] 7.1.2 Update README.md branding
  - Path: `README.md`
  - Change: All references from RCT to Patina
  - Acceptance: No "RCT" references in README

- [x] 7.1.3 Update CI workflow binary names
  - Path: `.github/workflows/ci.yml`, `.github/workflows/release.yml`
  - Change: All `rct` references to `patina`
  - Acceptance: CI builds produce `patina` binaries

- [x] 7.1.4 Update Docker configuration
  - Path: `Dockerfile`, `.github/workflows/docker.yml`
  - Change: Image name to `patina`
  - Change: Binary references
  - Acceptance: `docker build` works

- [x] 7.1.5 Update Homebrew formula
  - Path: `Formula/rct.rb`
  - Rename: To `Formula/patina.rb`
  - Change: All references to patina
  - Acceptance: Formula syntax valid

- [x] 7.1.6 Update source code references
  - Paths: `src/main.rs`, `src/lib.rs`, `src/tui/mod.rs`
  - Change: User-visible "RCT" strings to "Patina"
  - Acceptance: No user-visible "RCT" strings

- [x] 7.1.7 Update CLAUDE.md project documentation
  - Path: `.claude/CLAUDE.md`
  - Change: Project name references
  - Acceptance: Documentation accurate

### 7.2 Release Preparation

- [x] 7.2.1 Clean up .gitignore
  - Path: `.gitignore`
  - Add: `.mcp.json`, `.ralph/`, `.cowork/`, `coverage/`
  - Remove: Any personal paths
  - Acceptance: Sensitive files excluded

- [x] 7.2.2 Remove sensitive files from git tracking
  - Command: `git rm --cached .mcp.json` (if tracked)
  - Acceptance: No sensitive paths in repo

- [x] 7.2.3 Tag v0.3.0
  - Command: `git tag -a v0.3.0 -m "Patina v0.3.0 - Public Release"`
  - Acceptance: Tag created

- [x] 7.2.4 Create GitHub Release
  - Platform: GitHub Releases via `gh release create`
  - Include: Changelog, platform binaries
  - Acceptance: Release visible on GitHub (https://github.com/postrv/patina/releases/tag/v0.3.0)

### 7.3 Distribution

- [ ] 7.3.1 Publish to crates.io
  - Command: `cargo publish`
  - Acceptance: `cargo install patina` works

- [ ] 7.3.2 Submit Homebrew formula
  - Create: homebrew-patina tap or PR to homebrew-core
  - Acceptance: `brew install patina` works

- [ ] 7.3.3 Update Docker Hub
  - Push: `ghcr.io/postrv/patina:0.3.0`
  - Acceptance: `docker pull` works

### 7.4 Announcement

- [ ] 7.4.1 Write r/rust post
  - Focus: Performance benchmarks, Rust implementation
  - Acceptance: Post submitted

- [ ] 7.4.2 Write r/ClaudeAI post
  - Focus: Feature comparison with Claude Code
  - Acceptance: Post submitted

- [ ] 7.4.3 Write Hacker News submission
  - Title: "Show HN: Patina - Claude terminal client (16x faster than Claude Code)"
  - Acceptance: Post submitted

---

## Phase 8: Git Worktree Integration

**Objective:** Native support for parallel AI-assisted development

### 8.1 Core Worktree Module

- [x] 8.1.1 Create worktree module structure (RED)
  - Path: `src/worktree/mod.rs` (new)
  - Test: `tests/unit/worktree_test.rs` (new)
  - Test: `test_worktree_manager_detects_git_repo`
  - Test: `test_worktree_config_defaults`
  - Acceptance: Tests fail (module doesn't exist)

- [x] 8.1.2 Implement WorktreeConfig and WorktreeManager (GREEN)
  - Path: `src/worktree/mod.rs`
  - Types: `WorktreeConfig`, `WorktreeManager`, `WorktreeInfo`, `WorktreeError`
  - Acceptance: Basic structure tests pass

- [x] 8.1.3 Implement create/list/remove operations (GREEN)
  - Path: `src/worktree/mod.rs`
  - Methods: `create()`, `list()`, `remove()`, `status()`
  - Test: `test_create_worktree`
  - Test: `test_list_worktrees`
  - Test: `test_remove_worktree`
  - Acceptance: CRUD operations work

- [x] 8.1.4 Implement worktree status (GREEN)
  - Path: `src/worktree/mod.rs`
  - Method: `status()` returns modified/staged/ahead/behind counts
  - Test: `test_worktree_status_dirty`
  - Test: `test_worktree_status_clean`
  - Acceptance: Status accurately reflects git state

### 8.2 Slash Commands

- [x] 8.2.1 Add /worktree command parser (RED)
  - Path: `src/commands/mod.rs`
  - Test: `test_parse_worktree_new`
  - Test: `test_parse_worktree_list`
  - Test: `test_parse_worktree_switch`
  - Acceptance: Tests document expected parsing

- [x] 8.2.2 Implement /worktree commands (GREEN)
  - Path: `src/commands/worktree.rs` (new)
  - Commands: `new <name>`, `list`, `switch <name>`, `remove <name>`, `clean`, `status`
  - Acceptance: Commands execute correctly

### 8.3 Experiment Mode

- [x] 8.3.1 Design Experiment struct (RED)
  - Path: `src/worktree/experiment.rs` (new)
  - Test: `test_experiment_start`
  - Test: `test_experiment_accept`
  - Test: `test_experiment_reject`
  - Acceptance: Tests document experiment workflow

- [x] 8.3.2 Implement Experiment workflow (GREEN)
  - Methods: `start()`, `accept()`, `reject()`, `pause()`
  - Creates isolated worktree for risky changes
  - Acceptance: Full experiment lifecycle works

### 8.4 TUI Integration

- [x] 8.4.1 Add worktree picker widget (GREEN)
  - Path: `src/tui/widgets/worktree_picker.rs` (new)
  - Display: List worktrees with status indicators
  - Keybindings: n=new, s=switch, d=delete, c=clean
  - Acceptance: Widget renders correctly

- [x] 8.4.2 Add status bar worktree indicator (GREEN)
  - Path: `src/tui/mod.rs`
  - Display: Current branch, ahead/behind, modified count
  - Acceptance: Status bar shows worktree info

### 8.5 Session-Worktree Linking

- [x] 8.5.1 Extend session metadata (GREEN)
  - Path: `src/session/mod.rs`
  - Add: `WorktreeSession` struct with worktree_name, original_branch, commits
  - Acceptance: Sessions can be linked to worktrees

- [x] 8.5.2 Implement session restore per worktree (GREEN)
  - Path: `src/session/mod.rs`
  - Feature: Resume session in correct worktree context
  - Test: `test_session_restore_in_worktree`
  - Acceptance: Session resume respects worktree

---

## Phase 9: Plugin Ecosystem & narsil-mcp Integration

**Objective:** First-party plugin support with narsil-mcp as flagship

### 9.1 Plugin Manifest Format

- [x] 9.1.1 Define plugin manifest schema (RED)
  - Path: `src/plugins/manifest.rs` (new)
  - Test: `test_parse_plugin_manifest`
  - Test: `test_validate_plugin_capabilities`
  - Format: `rct-plugin.toml`
  - Acceptance: Tests document manifest structure

- [x] 9.1.2 Implement manifest parsing (GREEN)
  - Path: `src/plugins/manifest.rs`
  - Parse: name, version, description, capabilities, config
  - Acceptance: Manifests parse correctly

### 9.2 Plugin Registry

- [x] 9.2.1 Implement plugin discovery (GREEN)
  - Path: `src/plugins/registry.rs`
  - Scan: `~/.config/patina/plugins/` for manifests
  - Test: `test_discover_plugins`
  - Acceptance: Plugins discovered from filesystem

- [x] 9.2.2 Implement plugin lifecycle (GREEN)
  - Methods: `load()`, `unload()`, `list_enabled()`
  - Test: `test_plugin_load_unload`
  - Acceptance: Plugins can be loaded/unloaded

### 9.3 narsil-mcp Integration

- [x] 9.3.1 Create narsil plugin manifest
  - Path: `plugins/narsil/rct-plugin.toml`
  - Config: Auto-start MCP server, code intelligence tools
  - Acceptance: Manifest valid
  - **Completed: 2026-01-31** - Created TOML manifest with MCP, tools, and skills capabilities

- [x] 9.3.2 Implement auto-detection (GREEN)
  - Path: `src/plugins/narsil.rs` (new)
  - Detect: `which narsil-mcp` availability
  - Detect: Supported code files in project
  - Acceptance: narsil auto-enables when available
  - **Completed: 2026-01-31** - Added is_narsil_available(), has_supported_code_files(), should_enable_narsil()

- [x] 9.3.3 Add --with-narsil / --no-narsil flags (GREEN)
  - Path: `src/main.rs`
  - CLI: Override auto-detection
  - Acceptance: Flags control narsil loading
  - **Completed: 2026-01-31** - Added NarsilMode enum and CLI flags

---

## Phase 10: Session Resume & Context Persistence

**Objective:** Full session resume across reboots

### 10.1 Enhanced Session State

- [x] 10.1.1 Extend session with UI state (RED)
  - Path: `src/session/mod.rs`
  - Test: `test_ui_state_new`, `test_ui_state_with_values`, `test_ui_state_serialization`
  - Test: `test_session_with_ui_state`, `test_session_ui_state_serialization`
  - Fields: scroll_offset, input_buffer, cursor_position
  - Acceptance: Tests document serialization
  - **Completed: 2026-01-31** - Added UiState struct with full test coverage

- [x] 10.1.2 Implement UI state persistence (GREEN)
  - Path: `src/session/mod.rs`
  - Serialize: All restorable UI state via UiState struct
  - Test: `test_session_ui_state_persistence`
  - Acceptance: UI state survives save/load
  - **Completed: 2026-01-31** - UiState persists across session save/load

### 10.2 Context State

- [x] 10.2.1 Track context files (GREEN)
  - Path: `src/session/mod.rs`
  - Track: Files that were read during session
  - Track: Active skills
  - Acceptance: Context files recorded
  - **Completed: 2026-01-31** - Added ContextFile and SessionContext structs with full test coverage

- [x] 10.2.2 Restore context on resume (GREEN)
  - Path: `src/session/mod.rs`
  - Restore: Re-read context files if unchanged
  - Acceptance: Context restored on --resume
  - **Completed: 2026-01-31** - Added ContextFile::compute_hash(), ContextFile::is_unchanged(), SessionContext::restore(), and ContextRestoreResult with full test coverage

### 10.3 Resume Commands

- [x] 10.3.1 Implement --resume flag (GREEN)
  - Path: `src/main.rs`
  - Syntax: `--resume last` or `--resume <session-id>`
  - Acceptance: Sessions can be resumed
  - **Completed: 2026-01-31** - Added ResumeMode enum, --resume CLI flag, session loading logic, AppState restoration

- [x] 10.3.2 Implement --list-sessions flag (GREEN)
  - Path: `src/main.rs`
  - Display: Available sessions with timestamps
  - Acceptance: Sessions can be listed
  - **Completed: 2026-01-31** - Added --list-sessions CLI flag, format_session_list(), SessionManager::list_sorted(), full test coverage

### 10.4 Auto-Save

- [x] 10.4.1 Implement auto-save on message (GREEN)
  - Path: `src/app/mod.rs`
  - Trigger: After each message sent/received
  - Acceptance: Sessions auto-save
  - **Completed: 2026-01-31** - Added session_id tracking to AppState, to_session() method, auto_save_session() in event loop triggered after user message sent, assistant message completed, and on exit

---

## Phase 11: Visual Testing (Future)

**Objective:** VLM-based TUI verification

### 11.1 Test Harness

- [ ] 11.1.1 Set up Playwright + xterm.js
- [ ] 11.1.2 Create screenshot capture framework
- [ ] 11.1.3 Implement baseline comparison

### 11.2 VLM Integration

- [ ] 11.2.1 Create visual assertions with Claude Vision
- [ ] 11.2.2 Natural language test descriptions
- [ ] 11.2.3 CI integration for visual regression

---

## Phase 12: Beyond Parity Features (Future)

### 12.1 Semantic Code Search
- [ ] Local embeddings integration
- [ ] narsil-mcp fallback

### 12.2 Cost Tracking Dashboard
- [ ] Token usage display
- [ ] Budget limits
- [ ] Historical tracking

---

## Notes

### Rebrand Checklist

Files requiring "RCT" → "Patina" changes:
- `Cargo.toml` - package name
- `README.md` - all references
- `src/main.rs` - version display
- `src/tui/mod.rs` - title bar
- `.github/workflows/*.yml` - binary names
- `Dockerfile` - binary name
- `Formula/rct.rb` - formula name
- `.claude/CLAUDE.md` - project docs
- `CONTRIBUTING.md` - project name
- `SECURITY.md` - project name

### Git Worktree Commands Reference

```bash
git worktree add <path> -b <branch>
git worktree list
git worktree remove <path>
git worktree prune
```

### Previous Implementation Plans

Archived in `.archive/implementation-plans/`:
- `IMPLEMENTATION_PLAN_crossplatform_2026-01-30.md` - Cross-platform support (COMPLETE)
- Earlier plans in `.archive/docs/`
