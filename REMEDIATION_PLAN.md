# Patina Remediation Plan v0.7.1 → v0.8.0

**Based on:** Project Review (2026-02-04)
**Author:** Claude Opus 4.5
**Methodology:** TDD with Quality Gates
**Target Completion:** 4-6 weeks

---

## Executive Summary

This plan addresses 12 issues identified in the Patina + Narsil-MCP + CCG project review. Issues are prioritized by impact and grouped into four phases:

1. **Phase R.1: Documentation & Consistency** (Days 1-3) - Fix mismatches
2. **Phase R.2: Architecture Improvements** (Days 4-10) - Trait-based patterns
3. **Phase R.3: CCG Integration Wiring** (Days 11-20) - End-to-end MCP calls
4. **Phase R.4: Auto-Compaction** (Days 21-28) - Main loop integration

Each task follows the TDD cycle from CLAUDE.md:
```
REINDEX → RED → COMMIT → GREEN → COMMIT → REFACTOR → REVIEW → COMMIT → REINDEX
```

---

## Quality Gates (MANDATORY)

Every task MUST pass these gates before being marked complete:

| Gate | Command | Requirement |
|------|---------|-------------|
| Clippy | `cargo clippy --all-targets -- -D warnings` | 0 warnings |
| Tests | `cargo test` | All pass |
| Format | `cargo fmt -- --check` | No changes |
| Security | narsil `scan_security` | 0 CRITICAL/HIGH |

---

## Phase R.1: Documentation & Consistency

**Goal:** Resolve all documentation inconsistencies identified in review sections 2.1-2.5.
**Duration:** 3 days
**Risk:** Low
**Dependencies:** None

### R.1.1 Fix SPARQL Namespace URIs

**Location:** `src/context/compression/ccg_backend.rs:97, 116, 132`

**Current State:**
```rust
PREFIX ccg: <http://example.org/ccg#>
```

**Required State (per CCG Spec v0.2, Section 3):**
```rust
PREFIX ccg: <https://codecontextgraph.com/ontology/v1#>
PREFIX narsil: <https://narsilmcp.com/ontology/v1#>
```

**TDD Tasks:**

- [x] R.1.1.1 Add test `test_sparql_uses_spec_namespace` verifying PREFIX uses spec URIs
  - Path: `src/context/compression/ccg_backend.rs`
  - Test: Assert `build_scope_query` output contains `codecontextgraph.com/ontology/v1#`
  - RED: Test fails with current `example.org` namespace

- [x] R.1.1.2 Update all SPARQL query methods to use spec namespaces
  - Update `build_scope_query()` (line 95-107)
  - Update `build_callers_query()` (line 110-126)
  - Update `build_security_query()` (line 128-145)
  - GREEN: All namespace tests pass

- [x] R.1.1.3 Add constant for namespace URIs to centralize definition
  - Add `pub const CCG_NAMESPACE: &str = "https://codecontextgraph.com/ontology/v1#";`
  - Note: NARSIL_NAMESPACE deferred to Phase R.3 (YAGNI)
  - REFACTOR: Use constants in all query methods

**Acceptance:** All SPARQL queries contain correct namespace URIs.

---

### R.1.2 Update Tool Count in Documentation

**Locations:**
- `README.md` (line 69): "76 analysis tools"
- Strategic roadmap: "76+ code intelligence tools"

**Required State:** "90 tools" (per narsil-mcp README v1.3.x)

**TDD Tasks:**

- [x] R.1.2.1 Update README.md narsil tool count
  - Path: `README.md:69`
  - Change: `76` → `90`

- [x] R.1.2.2 Update any other documentation references
  - Updated: `patina-strategic-roadmap.md` (3 occurrences)

**Acceptance:** All documentation states 90 tools.

---

### R.1.3 Add Automated Metrics Validation

**Goal:** Prevent future version/LOC/test count drift by computing metrics automatically.

**TDD Tasks:**

- [ ] R.1.3.1 Create metrics computation script
  - Path: `scripts/compute_metrics.sh`
  - Outputs: LOC count (`tokei` or `cloc`), test count, version from Cargo.toml

- [ ] R.1.3.2 Add CI step to validate metrics consistency
  - Path: `.github/workflows/ci.yml`
  - Add step that runs `scripts/compute_metrics.sh` and compares to README claims

- [ ] R.1.3.3 Update README with computed values
  - Add comment: `<!-- Metrics auto-validated by CI -->`
  - Update LOC, test count to match computed values

**Acceptance:** CI fails if README claims differ from computed metrics by >5%.

---

## Phase R.2: Architecture Improvements

**Goal:** Replace anti-patterns with idiomatic Rust patterns.
**Duration:** 7 days
**Risk:** Medium (refactoring existing code)
**Dependencies:** Phase R.1 complete

### R.2.1 Replace ContextCompactor is_mock with Trait

**Location:** `src/api/compaction.rs:185-188, 302-309`

**Current State:**
```rust
pub struct ContextCompactor {
    is_mock: bool,
}

fn generate_summary(&self, messages: &[ApiMessageV2], config: &CompactionConfig) -> String {
    if self.is_mock {
        self.generate_mock_summary(messages, config)
    } else {
        // Real implementation would call Claude API
        // For now, use mock summary
        self.generate_mock_summary(messages, config)
    }
}
```

**Required State:**
```rust
#[async_trait]
pub trait Summarizer: Send + Sync {
    async fn summarize(
        &self,
        messages: &[ApiMessageV2],
        config: &CompactionConfig,
    ) -> Result<String>;
}

pub struct ClaudeSummarizer { client: Arc<AnthropicClient> }
pub struct MockSummarizer;

pub struct ContextCompactor<S: Summarizer> {
    summarizer: S,
}
```

**TDD Tasks:**

- [x] R.2.1.1 Define `Summarizer` trait with sync `summarize` method
  - Path: `src/api/compaction.rs`
  - Test: Trait compiles and can be implemented
  - Note: Implemented as sync; async variant deferred to when needed

- [x] R.2.1.2 Implement `MockSummarizer` that returns canned summaries
  - Path: `src/api/compaction.rs`
  - Test: `test_mock_summarizer_implements_trait`
  - GREEN: MockSummarizer passes existing compaction tests

- [ ] R.2.1.3 Implement `ClaudeSummarizer` that calls API
  - Path: `src/api/compaction.rs`
  - DEFERRED: To be implemented when API summarization is needed

- [x] R.2.1.4 Refactor `ContextCompactor` to be generic over `Summarizer`
  - Change: `ContextCompactor` → `ContextCompactor<S: Summarizer>`
  - Test: All existing tests pass with `MockSummarizer`
  - DONE: `is_mock` field removed

- [x] R.2.1.5 Update all call sites to use type parameter
  - Factory functions: `new_mock()` / `with_summarizer(s)`
  - Exports: `Summarizer`, `MockSummarizer` added to api module

**Acceptance:** ✅ `is_mock` field removed; `mockall` can mock `Summarizer` trait.

---

### R.2.2 Add Typestate to ToolUseAccumulator

**Location:** `src/types/stream.rs:188-242`

**Current State:**
```rust
pub struct ToolUseAccumulator {
    pub id: Option<String>,
    pub name: Option<String>,
    pub input_json: String,
}

impl ToolUseAccumulator {
    pub fn start(&mut self, id: String, name: String) { ... }
    pub fn append_input(&mut self, partial_json: &str) { ... }  // Can call before start!
    pub fn parse_input(&self) -> Result<Value, serde_json::Error> { ... }
}
```

**Required State:**
```rust
pub struct Unstarted;
pub struct Started;
pub struct Complete;

pub struct ToolUseAccumulator<State = Unstarted> {
    id: String,
    name: String,
    input_json: String,
    _state: PhantomData<State>,
}

impl ToolUseAccumulator<Unstarted> {
    pub fn new() -> Self { ... }
    pub fn start(self, id: String, name: String) -> ToolUseAccumulator<Started> { ... }
}

impl ToolUseAccumulator<Started> {
    pub fn append_input(&mut self, partial_json: &str) { ... }
    pub fn complete(self) -> ToolUseAccumulator<Complete> { ... }
}

impl ToolUseAccumulator<Complete> {
    pub fn parse_input(&self) -> Result<Value, serde_json::Error> { ... }
    pub fn id(&self) -> &str { ... }
    pub fn name(&self) -> &str { ... }
}
```

**TDD Tasks:**

- [x] R.2.2.1 Add typestate types (alternative approach)
  - Path: `src/types/stream.rs`
  - Add: `ToolUseBuilder`, `StartedToolUse`, `CompletedToolUse`
  - Note: Used separate types instead of PhantomData for cleaner API

- [x] R.2.2.2 Implement ToolUseBuilder as unstarted state
  - `ToolUseBuilder::new()` creates unstarted builder
  - `start(self, ...) -> StartedToolUse` transitions state

- [x] R.2.2.3 Implement state transitions as consuming methods
  - `ToolUseBuilder::start() -> StartedToolUse`
  - `StartedToolUse::complete() -> Result<CompletedToolUse>`
  - Test: State transitions work correctly

- [x] R.2.2.4 Methods only available in appropriate states
  - `append_input` only on `StartedToolUse`
  - `complete()` only on `StartedToolUse`
  - `id`, `name`, `input` fields only on `CompletedToolUse`
  - Test: Compile-time enforcement works

- [ ] R.2.2.5 Migrate tool_loop.rs to use ToolUseBuilder
  - DEFERRED: Existing ToolUseAccumulator preserved for backward compat
  - New code can use typestate API for stronger guarantees

**Acceptance:** ✅ Calling `append_input()` before `start()` fails at compile time (on new ToolUseBuilder API).

---

### R.2.3 Add Bash Alias Resolution (Optional Enhancement)

**Location:** `src/tools/bash.rs` (security classification)

**Current State:** Bash command classification doesn't account for aliases.

**Required State:** Add optional runtime alias resolution for non-aggressive mode.

**TDD Tasks:**

- [ ] R.2.3.1 Add `resolve_alias` function that runs `type -a <cmd>`
  - Path: `src/tools/bash.rs`
  - Test: `test_resolve_alias_finds_real_command`
  - Note: This is optional/enhancement, not blocking

- [ ] R.2.3.2 Integrate alias resolution into classification pipeline
  - Only in default parallel mode (not aggressive)
  - Test: Aliased dangerous commands are still blocked

**Acceptance:** `alias rm='rm -rf /'; rm` is correctly classified as dangerous.

---

## Phase R.3: CCG Integration Wiring

**Goal:** Connect Patina's compression layer to narsil-mcp's CCG tools end-to-end.
**Duration:** 10 days
**Risk:** High (critical path for differentiation)
**Dependencies:** Phase R.2 complete

### R.3.1 Add Async MCP Client to Orchestrator

**Location:** `src/context/compression/orchestrator.rs`

**Current State:**
- `get_context()` accepts synchronous `content_fn: F where F: FnOnce() -> String`
- No actual MCP calls; just constructs results from callback

**Required State:**
- Async methods that call narsil-mcp tools via MCP client
- `get_manifest()`, `get_architecture()`, `query_symbols()` methods

**TDD Tasks:**

- [ ] R.3.1.1 Add MCP client dependency to CompressionOrchestrator
  - Path: `src/context/compression/orchestrator.rs`
  - Add: `mcp_client: Option<Arc<McpClient>>` field
  - Test: Orchestrator constructs with client

- [ ] R.3.1.2 Implement `get_manifest_async()` that calls `get_ccg_manifest`
  - Path: `src/context/compression/orchestrator.rs`
  - Test: `test_get_manifest_calls_mcp_tool` (mocked MCP)
  - RED: Method doesn't exist

- [ ] R.3.1.3 Implement `get_architecture_async()` that calls `export_ccg_architecture`
  - Test: `test_get_architecture_calls_mcp_tool`
  - GREEN: Method calls correct tool with correct args

- [ ] R.3.1.4 Implement `query_async()` that calls `query_ccg`
  - Test: `test_query_calls_mcp_with_sparql`
  - Verify: LIMIT clause enforced

- [ ] R.3.1.5 Add graceful fallback when MCP unavailable
  - Test: `test_orchestrator_degrades_gracefully_without_mcp`
  - GREEN: Returns Constructed source when client is None

**Acceptance:** Orchestrator can call narsil-mcp CCG tools and handle failures.

---

### R.3.2 Wire CCG Backend to MCP Tool Calls

**Location:** `src/context/compression/ccg_backend.rs`

**Current State:**
- `manifest_args()`, `architecture_args()`, `query_args()` build JSON but don't execute
- No async methods

**Required State:**
- Async methods that execute MCP calls and parse responses
- JSON-LD parsing for manifest/architecture responses

**TDD Tasks:**

- [ ] R.3.2.1 Add async `fetch_manifest()` method
  - Path: `src/context/compression/ccg_backend.rs`
  - Signature: `async fn fetch_manifest(&self, client: &McpClient) -> Result<CompressionResult>`
  - Test: `test_fetch_manifest_parses_jsonld`

- [ ] R.3.2.2 Add async `fetch_architecture()` method
  - Test: `test_fetch_architecture_parses_jsonld`
  - Verify: Modules and publicAPI extracted correctly

- [ ] R.3.2.3 Add async `execute_query()` method
  - Signature: `async fn execute_query(&self, client: &McpClient, sparql: &str) -> Result<CompressionResult>`
  - Test: `test_execute_query_respects_limit`

- [ ] R.3.2.4 Add JSON-LD response parser
  - Path: `src/context/compression/render.rs`
  - Test: `test_parse_manifest_jsonld` with sample response
  - GREEN: Parser extracts repository, languages, symbols fields

**Acceptance:** CcgBackend executes MCP calls and parses CCG responses.

---

### R.3.3 Integrate Orchestrator into App State

**Location:** `src/app/state.rs`

**Current State:**
- No `CompressionOrchestrator` in AppState
- No automatic CCG context injection

**Required State:**
- AppState holds `compression_orchestrator: Option<CompressionOrchestrator>`
- Context injected before API calls when CCG available

**TDD Tasks:**

- [ ] R.3.3.1 Add CompressionOrchestrator field to AppState
  - Path: `src/app/state.rs`
  - Test: AppState constructs with orchestrator

- [ ] R.3.3.2 Initialize orchestrator from NarsilIntegration
  - Path: `src/app/mod.rs`
  - Test: `test_app_creates_orchestrator_when_narsil_available`

- [ ] R.3.3.3 Add `inject_ccg_context()` method to AppState
  - Signature: `async fn inject_ccg_context(&mut self) -> Result<Option<String>>`
  - Test: `test_inject_ccg_context_returns_manifest`
  - Verify: Returns None if orchestrator unavailable

- [ ] R.3.3.4 Call `inject_ccg_context()` before API streaming
  - Path: `src/app/state.rs` in `send_message()` or equivalent
  - Test: `test_api_call_includes_ccg_context`

**Acceptance:** CCG manifest/architecture automatically injected into context when available.

---

## Phase R.4: Auto-Compaction

**Goal:** Trigger compaction automatically when context window approaches limit.
**Duration:** 7 days
**Risk:** Medium
**Dependencies:** Phase R.3 complete (or can run in parallel with R.3.3+)

### R.4.1 Add Token Threshold Configuration

**Location:** `src/types/config.rs`

**Current State:**
- `CompressionConfig` has `target_tokens` but no threshold trigger
- No auto-compaction config in app Config

**Required State:**
```rust
pub struct CompressionConfig {
    pub target_tokens: usize,
    pub preserve_recent: usize,
    pub summary_style: SummaryStyle,
    pub auto_compact_threshold: f32,  // NEW: e.g., 0.8 = compact at 80% of limit
}
```

**TDD Tasks:**

- [ ] R.4.1.1 Add `auto_compact_threshold` to CompressionConfig
  - Path: `src/types/config.rs` or `src/api/compaction.rs`
  - Default: 0.8 (80% of context window)
  - Test: Config deserializes with new field

- [ ] R.4.1.2 Add model context limits lookup
  - Path: `src/api/tokens.rs`
  - Function: `fn model_context_limit(model: &str) -> usize`
  - Test: Returns 200_000 for claude-sonnet-4, etc.

**Acceptance:** Config has threshold; model limits are known.

---

### R.4.2 Implement Token Budget Checker

**Location:** `src/app/state.rs`

**Current State:**
- `token_budget: TokenBudget` exists (line 172)
- Not used to trigger compaction

**Required State:**
- Check token budget before each API call
- Return signal when threshold exceeded

**TDD Tasks:**

- [ ] R.4.2.1 Add `needs_compaction()` method to TokenBudget
  - Path: `src/api/tokens.rs` or `src/app/state.rs`
  - Signature: `fn needs_compaction(&self, threshold: f32, limit: usize) -> bool`
  - Test: `test_needs_compaction_at_threshold`

- [ ] R.4.2.2 Add `estimate_conversation_tokens()` to AppState
  - Uses `estimate_messages_tokens(&self.api_messages)`
  - Test: `test_estimate_tokens_for_conversation`

**Acceptance:** Can detect when conversation exceeds threshold.

---

### R.4.3 Add Auto-Compaction Trigger in Event Loop

**Location:** `src/app/mod.rs` (main event loop)

**Current State:**
- No compaction trigger before API calls
- `compaction_state: Option<CompactionProgressState>` exists but unused for auto

**Required State:**
- Check token budget before `stream_message()` call
- If over threshold, trigger compaction workflow
- Show compaction progress overlay during compaction

**TDD Tasks:**

- [ ] R.4.3.1 Add `maybe_compact()` async method to AppState
  - Path: `src/app/state.rs`
  - Signature: `async fn maybe_compact(&mut self) -> Result<bool>`
  - Returns: true if compaction occurred
  - Test: `test_maybe_compact_triggers_at_threshold`

- [ ] R.4.3.2 Integrate `maybe_compact()` into send flow
  - Path: `src/app/mod.rs`
  - Call before API streaming starts
  - Test: `test_api_call_triggers_compaction_when_needed`

- [ ] R.4.3.3 Update compaction_state during auto-compaction
  - Show progress overlay with "Compacting context..." message
  - Test: `test_compaction_shows_progress_overlay`

- [ ] R.4.3.4 Add metrics for auto-compaction events
  - Track: compaction_count, tokens_saved, time_taken
  - Test: `test_compaction_metrics_recorded`

**Acceptance:** Long conversations automatically compact before hitting context limit.

---

## Test Count Tracking

**Baseline (v0.7.0):** 1,330+ tests

| Phase | New Tests | Cumulative |
|-------|-----------|------------|
| R.1 | +5 | 1,335 |
| R.2 | +15 | 1,350 |
| R.3 | +20 | 1,370 |
| R.4 | +12 | 1,382 |

**Target:** 1,380+ tests at v0.8.0

---

## Risk Register

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| MCP protocol changes | Low | High | Pin narsil-mcp version, add protocol tests |
| Typestate refactor breaks call sites | Medium | Medium | Incremental migration, alias old API |
| Auto-compaction causes UX lag | Medium | Low | Run compaction in background, show progress |
| CCG data larger than expected | Low | Medium | Enforce LIMIT clauses, add size checks |

---

## Definition of Done (v0.8.0)

1. All 4 phases complete with checkboxes marked
2. Test count ≥ 1,380
3. All quality gates pass
4. `cargo doc --no-deps` builds without warnings
5. README updated with:
   - Correct tool count (90)
   - Auto-compaction documented
   - CCG integration documented
6. `CHANGELOG.md` updated with v0.8.0 section
7. Git tag `v0.8.0` created

---

## Appendix A: Narsil CCG Tools Reference

From `narsil-mcp/src/tool_handlers/ccg.rs`:

| Tool | Layer | Purpose |
|------|-------|---------|
| `get_ccg_manifest` | L0 | Retrieve manifest JSON-LD |
| `export_ccg_manifest` | L0 | Export to file |
| `export_ccg_architecture` | L1 | Export architecture JSON-LD |
| `export_ccg_index` | L2 | Export symbol index N-Quads |
| `export_ccg_full` | L3 | Export full detail N-Quads |
| `export_ccg` | All | Export all layers bundled |
| `query_ccg` | L3 | SPARQL query against graph |
| `get_ccg_acl` | - | Generate WebACL document |
| `get_ccg_access_info` | - | Access tier information |
| `import_ccg` | All | Import from URL/file |
| `import_ccg_from_registry` | All | Import from CCG registry |

---

## Appendix B: CCG Namespace Reference

From `narsil-mcp/docs/ccg-spec.md` Section 3:

```turtle
@prefix ccg: <https://codecontextgraph.com/ontology/v1#> .
@prefix narsil: <https://narsilmcp.com/ontology/v1#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
@prefix acl: <http://www.w3.org/ns/auth/acl#> .
```

---

*Generated: 2026-02-04*
*Reviewer: Claude Opus 4.5*
*Methodology: TDD with Quality Gates per CLAUDE.md*
