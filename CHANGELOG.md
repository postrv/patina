# Changelog

All notable changes to Patina are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.1.0] - 2026-03-30

### Added

#### Multi-Provider API Support
- AWS Bedrock provider with region-based model routing
- Google Vertex AI provider with project/region configuration
- Provider fallback chains — try multiple providers in order

#### New Tools
- Task management tools: `task_create`, `task_get`, `task_list`, `task_update`
- `cron_create`, `cron_list`, `cron_delete` for scheduled recurring jobs
- `ask_user` for structured user interaction during tool loops
- `tool_search` for deferred tool loading and MCP tool discovery
- `send_message` for inter-agent communication
- `multi_edit` for batch file edits across multiple files
- `notebook_edit` for Jupyter notebook editing
- External editor support via `$EDITOR` (Ctrl+X Ctrl+E)

#### New Commands
- `/plan` — plan mode workflow with step-by-step review
- `/effort` — set reasoning effort level (low/medium/high/auto)
- `/doctor` — diagnostic checks for system health
- `/export` — conversation export (JSON, Markdown, text)
- `/context` — context window capacity analysis and breakdown
- `/analyze` and `/audit` — narsil code intelligence and security scanning
- `/rewind` — session checkpoint picker for conversation branching
- `/sandbox` — sandbox configuration management
- `/settings` — view and edit configuration
- `/status` — session status overview
- `/color`, `/copy`, `/btw`, `/bug`, `/rename`, `/config`

#### Forge Integration
- Forge V8 sandbox MCP gateway support
- Automatic Forge config generation from `.mcp.json`
- Process context injection for sandboxed MCP servers

#### MCP Enhancements
- Migrated to rmcp SDK (0.16 → 1.2) for full MCP spec coverage
- OAuth and bearer token authentication for MCP servers
- Legacy SSE transport support
- MCP config trust store for project `.mcp.json` validation

#### Session Enhancements
- Checkpoint system with named save points
- Session branching and rewind
- Session forking with parent tracking
- Context restoration on resume (files, skills)
- UI state persistence (scroll, cursor, input buffer)

#### TUI Improvements
- Transcript search (Ctrl+F) with regex and match navigation
- Slash command completion popup with fuzzy matching
- @-mention file autocomplete
- Real-time session cost display in status bar
- Desktop notifications via OSC escape sequences (iTerm2, Kitty, Ghostty)
- Viewport-only rendering with `VirtualizedViewport` for performance
- Keybinding customization system

#### Infrastructure
- Enterprise managed settings with `managed-settings.d/` drop-in fragments
- OS-level sandboxing: macOS Seatbelt and Linux Landlock
- Path-scoped rules engine for contextual `.claude` rules
- Prompt-based hooks with LLM-driven decisions
- `PostCompact` and `StopFailure` hook events
- `--bare` flag for fast startup (skips hooks/plugins/skills)
- stdin pipe support for scripting integration
- Project structure and git status injected into system prompt
- Accurate BPE token counting with tiktoken
- Prompt caching with system blocks and cache cost accounting
- Extended thinking with SSE parsing and per-turn usage tracking
- Narsil system prompt integration for code-aware context

### Changed
- AppState decomposed from 4,467 to 2,531 lines across focused sub-states (`ConversationEngine`, `ViewState`, `ToolExecutionState`, etc.)
- `KeyboardHandler` split into sub-modules (submit, clipboard, mouse)
- Dirty flag tracking pushed into individual sub-states for granular re-rendering
- Compaction migrated to V2 API with async `Summarizer` trait
- `process_stream()` nesting flattened from depth 6 to 3
- `main()` decomposed from cyclomatic complexity 44 into 7 focused functions
- Provider factory extracted to break circular import
- Retry logic extracted to dedicated `api/retry.rs` module
- `RenderView` introduced to fully decouple TUI rendering from `AppState`
- Dead code removed: unused multi_model stubs, audit_logger scaffolding, command_executor placeholders

### Fixed
- Arrow keys, autocomplete, and several panic-on-edge-case bugs in TUI
- Keybinding issues: stale prompt state, chord clearing, shift+letter parsing, plus key, chord timeout
- Lock poisoning panics replaced with graceful recovery across all Mutex usage
- All unbounded channels replaced with bounded to prevent memory exhaustion
- Hook normalization bypass that could skip security checks
- OpenRouter unwrap panics on malformed responses
- HTTP client panic on connection failure
- Sandbox made opt-in via policy to prevent test breakage
- TUI logging redirected to file to prevent display corruption
- Production panics replaced with proper error handling throughout
- HTML-escape on OAuth error responses to prevent XSS
- Background task command validation
- Doctest failures and formatting issues

### Security
- Post-connect DNS rebinding SSRF protection
- Session HMAC hardening with per-user keys
- OAuth token storage hardening
- Seatbelt sandbox path validation
- Symlink traversal checks
- MCP command validation blocklists
- Token-based IDE controller authentication
- quinn-proto and rustls-webpki updated for security patches
- Environment variable validation and credential scrubbing

## [1.0.0] - 2026-02-19

### Added
- Terminal environment detection for tmux, GNU screen, SSH, and JetBrains terminals
- Environment-aware clipboard strategy (arboard, OSC 52, wl-copy fallback)
- Platform-specific smoke tests for macOS, Windows, and Linux
- Image rendering adaptation for multiplexers (Sixel/Kitty → Unicode fallback)
- Comprehensive manual test script (`scripts/manual_test_checklist.sh`)
- CI version consistency and naming checks in `validate_metrics.sh`

### Changed
- AppState decomposed into focused sub-structs: `ToolExecutionState`, `CompressionState`, `ContinuousLoopState`, `UISelectionState` (50+ fields → 26 direct fields)
- Compression orchestrator wired into API message flow with cache-aware refresh
- IDE `ApplyEdit` handler fully implemented (was stub returning NOT_IMPLEMENTED)
- OAuth dependencies gated behind `oauth` feature flag
- Dangerous command detection consolidated into single canonical source (`tools/security.rs`)
- All documentation aligned to project name "Patina" (removed "rct" remnants)

### Fixed
- Pre-release security hardening: `.unwrap()` calls replaced with `.expect()` in security-critical paths
- `cargo audit` vulnerabilities resolved (bytes 1.11.0→1.11.1, time 0.3.46→0.3.47)
- Race condition in platform integration tests (env var mutations serialized via Mutex)

### Security
- Full security audit: `scan_security`, `check_owasp_top10`, `check_cwe_top25` — all findings classified
- All 16 narsil CRITICAL findings confirmed as false positives and documented
- `cargo audit` clean (0 vulnerable dependencies)

## [0.9.0] - 2026-02-19

### Added
- Event dispatcher architecture for decoupled component communication
- Continuous coding loop with stagnation detection and quality gates
- `StagnationDetector` with multi-factor scoring
- `ContinuousRunner` state machine with recovery loop
- Narsil-powered root-cause analysis for continuous coding failures
- `/continuous` slash command with TUI widget
- `QualityGateRunner` with timeout enforcement and output truncation
- Parallel context building via `tokio::join!`
- Parallel MCP capability detection
- `PerformanceConfig` for parallelism policy
- Agent execution loop with cross-agent conflict detection
- Agent slash commands wired into event dispatcher

### Changed
- Multi-model provider support: OpenRouter and fallback providers
- `ProviderConfig` factory with CLI flags for provider selection

### Fixed
- Parallel test race conditions causing SIGSEGV
- Terminal removed from AppContext to fix 180 CI test failures

## [0.8.0] - 2026-02-04

### Added
- Auto-compaction with configurable threshold and model-specific token limits
- Compaction metrics tracking and TUI progress widget
- `ClaudeSummarizer` for conversation compaction via API calls
- CCG context injection into message-sending path
- CI metrics validation script (`scripts/validate_metrics.sh`)
- Bash alias resolution for security classification

### Changed
- `ToolUseBuilder` migrated to typestate API
- Compression orchestrator initialized from `NarsilIntegration`

## [0.7.0] - 2026-02-04

### Added
- Smart context compression with CCG and fallback backends
- `CompressionOrchestrator` with result caching and hash invalidation
- `CompressionMetrics` with atomic counters
- Token-budget-aware context building
- JSON-LD parsing and Markdown rendering for compressed context
- Graceful degradation when narsil is unavailable
- End-to-end compression integration tests and benchmarks

## [0.6.1] - 2026-02-03

### Changed
- Major module extraction refactoring: tools (security, executor, parallel), session (context, format, persistence, manager, worktree, ui_state)
- Codebase reorganized for maintainability

### Fixed
- UI freeze during tool execution

## [0.6.0] - 2026-02-03

### Added
- Narsil code intelligence integration (`NarsilIntegration` for call graphs, references, dependencies)
- Context suggestions with caller/dependency parsing
- Security pre-flight checks in tool loop with `SecurityVerdict` enum
- Continuous coding plugin foundation (`ContinuousEvent`, `QualityGate`)
- Parallel agent orchestration with division strategies
- Plugin registry with installer, source parsing, and CLI management (`/plugin` subcommand)
- Example plugins (formatter, linter, notifier)
- Auto-context injection support in AppState

## [0.5.5] - 2026-02-02

### Added
- Feature completion sprint: web tools, vision infrastructure, OAuth dependency scaffolding, keyring integration
- Parallel tool execution with safety classification (ReadOnly vs Mutating)

## [0.3.0] - 2026-01-30

### Added
- Cross-platform support: Windows, macOS, Linux
- Cross-platform shell abstraction layer with command translation
- Windows-specific security validation for MCP and tools
- Cross-platform MCP test infrastructure
- Cross-platform hook execution

### Fixed
- Platform-specific test isolation with `#[cfg(target_os)]` attributes

## [0.2.0] - 2026-01-30

### Added
- Security hardening sprint (8 vulnerabilities resolved)
- Bypassable bash command filter replaced with normalization + allowlist mode
- `SecretString` for API keys in multi-model provider
- Dangerous command filtering in hook executor
- Path traversal prevention in `list_files` and session ID handling
- MCP command validation
- TOCTOU race prevention via symlink rejection
- Runtime regex compilation replaced with `once_cell::sync::Lazy`
- Session deserialization integrity via HMAC-SHA256

### Security
- C-1 CRITICAL: Bash command filter bypass — fixed with normalization + allowlist
- H-1 HIGH: Plain string API key exposure — fixed with `SecretString`
- H-2 HIGH: Unsandboxed hook execution — fixed with dangerous command filtering
- H-3 HIGH: `list_files` path traversal — fixed with `validate_path()`
- M-1 MEDIUM: Unvalidated MCP commands — fixed with `validate_mcp_command()`
- M-2 MEDIUM: TOCTOU race in path validation — fixed with symlink rejection
- L-1 LOW: Runtime regex compilation — fixed with `once_cell::sync::Lazy`
- L-2 LOW: Session deserialization trust — fixed with HMAC-SHA256 integrity

## [0.1.0] - 2026-01-29

### Added
- Initial release
- Interactive TUI chat interface with ratatui
- Anthropic Claude API streaming client
- Tool execution: bash, file read/write/edit, glob, grep
- MCP protocol client (stdio and SSE transports)
- Lifecycle hooks system
- Skill engine with file pattern matching
- Slash command framework
- Subagent orchestrator
- Plugin system with host API
- Session persistence with save/load/resume
- Enterprise features: cost controls, audit logging, multi-model support
- Performance benchmarks with criterion

[1.1.0]: https://github.com/postrv/patina/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/postrv/patina/compare/v0.9.0...v1.0.0
[0.9.0]: https://github.com/postrv/patina/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/postrv/patina/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/postrv/patina/compare/v0.6.1...v0.7.0
[0.6.1]: https://github.com/postrv/patina/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/postrv/patina/compare/v0.3.0-crossplatform...v0.6.0
[0.5.5]: https://github.com/postrv/patina/compare/v0.3.0-crossplatform...v0.6.0
[0.3.0]: https://github.com/postrv/patina/compare/v0.2.0-security...v0.3.0-crossplatform
[0.2.0]: https://github.com/postrv/patina/compare/v0.1.0...v0.2.0-security
[0.1.0]: https://github.com/postrv/patina/releases/tag/v0.1.0
