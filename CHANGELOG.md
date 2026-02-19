# Changelog

All notable changes to Patina are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
