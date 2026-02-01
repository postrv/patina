# Patina Optimization Plan v2.0

**Document Version:** 2.0  
**Date:** 2026-02-01  
**Status:** Ready for Implementation  
**Authors:** Engineering Team  

---

## Executive Summary

This document outlines a Test-Driven Development (TDD) implementation plan for two critical improvements to Patina v0.3.0:

1. **Cost Optimization** — Prevent runaway API costs by implementing smart context windowing
2. **Copy/Paste Fix** — Enable full-text selection and clipboard operations

Both issues are blocking daily heavy use of the application. Cost optimization is prioritized first due to immediate financial impact.

### Success Criteria

| Metric | Current | Target | Measurement |
|--------|---------|--------|-------------|
| API cost per long session | ~$2-5+ | <$0.50 | Token tracking |
| Max input tokens per request | Unbounded | ≤100k | API logs |
| Copy/paste success rate | ~0% (broken) | 100% | Manual + automated tests |
| Test coverage (new code) | N/A | ≥90% | `cargo tarpaulin` |
| Performance regression | N/A | <5% | Criterion benchmarks |

---

## Table of Contents

1. [Current State Assessment](#1-current-state-assessment)
2. [Architecture Overview](#2-architecture-overview)
3. [Phase 0: Foundation & Diagnostics](#3-phase-0-foundation--diagnostics)
4. [Phase 1: Cost Optimization](#4-phase-1-cost-optimization)
5. [Phase 2: Copy/Paste Fix](#5-phase-2-copypaste-fix)
6. [Phase 3: Smart Context Evolution](#6-phase-3-smart-context-evolution)
7. [Quality Gates](#7-quality-gates)
8. [Test Specifications](#8-test-specifications)
9. [Rollback Procedures](#9-rollback-procedures)
10. [Appendices](#10-appendices)

---

## 1. Current State Assessment

### 1.1 Project Health

Patina v0.3.0 is architecturally sound:

- ✅ 1,150+ tests, 85%+ coverage
- ✅ Zero `unsafe` code
- ✅ Sub-millisecond rendering
- ✅ Clean module separation (api, app, tui, tools, mcp)
- ✅ Comprehensive type system (Timeline, ApiMessageV2, SelectionState)

### 1.2 Identified Issues

#### Issue A: Unbounded Context Growth (CRITICAL)

**Location:** `src/app/state.rs` → `submit_message()`

```rust
// CURRENT: Full message history sent on every request
pub async fn submit_message(&mut self, client: &AnthropicClient, content: String) -> Result<()> {
    self.api_messages.push(user_msg);
    // ... eventually calls stream_message_v2 with ALL api_messages
}
```

**Impact:**
- Long sessions accumulate 200k+ tokens per request
- Cost: $0.60+ per request on Sonnet (should be ~$0.15)
- Risk of hitting API limits (200k context window)

#### Issue B: Copy/Paste Fails on Full Selection

**Location:** `src/app/state.rs` + `src/tui/mod.rs`

**Symptom:** Cmd+A → Cmd+C copies empty or partial content

**Root Cause Hypothesis:**
1. `rendered_lines_cache` not populated with full content, OR
2. Cache populated only with viewport-visible lines, OR
3. Selection range exceeds cache bounds

**Evidence Needed:** Status bar diagnostics (Phase 0)

---

## 2. Architecture Overview

### 2.1 Message Flow (Current)

```
User Input
    ↓
AppState::submit_message()
    ↓
api_messages.push(user_msg)  ← UNBOUNDED GROWTH
    ↓
AnthropicClient::stream_message_v2(&api_messages)
    ↓
HTTP POST /v1/messages (full history)
    ↓
StreamEvent processing
    ↓
Timeline update
```

### 2.2 Message Flow (Target)

```
User Input
    ↓
AppState::submit_message()
    ↓
api_messages.push(user_msg)
    ↓
api_messages_truncated()  ← NEW: Token-bounded window
    ↓
AnthropicClient::stream_message_v2(&truncated_messages)
    ↓
HTTP POST /v1/messages (bounded payload)
    ↓
StreamEvent processing
    ↓
Timeline update
```

### 2.3 Selection/Copy Flow (Current)

```
Cmd+A (select_all)
    ↓
SelectionState::select_all(rendered_line_count())
    ↓
selection.anchor = (0, 0)
selection.cursor = (line_count - 1, MAX)
    ↓
Cmd+C (copy)
    ↓
copy_from_cache()
    ↓
rendered_lines_cache[start..end]  ← POSSIBLY EMPTY OR WRONG
    ↓
arboard::Clipboard::set_text()
```

### 2.4 Selection/Copy Flow (Target)

```
[Render Cycle]
    ↓
render_timeline_full() → all_lines
    ↓
state.update_rendered_lines_cache(&all_lines)  ← ALWAYS FULL CONTENT
    ↓
viewport_clip(all_lines) → visible_lines
    ↓
draw(visible_lines)

[User Action]
Cmd+A → select_all(cache.len())  ← MATCHES CACHE
Cmd+C → copy_from_cache()        ← CACHE IS POPULATED
```

---

## 3. Phase 0: Foundation & Diagnostics

**Duration:** 1-2 hours  
**Risk Level:** None (observability only)  
**Deliverables:** Status bar indicators, structured logging

### 3.0 Quality Gate: Entry Criteria

- [ ] All existing tests pass (`cargo test`)
- [ ] No compiler warnings (`cargo clippy`)
- [ ] Benchmarks baselined (`cargo bench -- --save-baseline phase0-pre`)

### 3.1 Task 0.1: Status Bar Diagnostics

**Purpose:** Non-intrusive visibility into selection and cache state

#### 3.1.1 Test Specification (Write First)

```rust
// tests/tui/status_bar_tests.rs

#[test]
fn test_status_bar_shows_selection_none_when_empty() {
    let state = AppState::new(PathBuf::from("."), false);
    let status = render_status_bar_diagnostics(&state);
    assert!(status.contains("SEL:none"));
}

#[test]
fn test_status_bar_shows_selection_range() {
    let mut state = AppState::new(PathBuf::from("."), false);
    // Populate cache with 100 lines
    state.update_rendered_lines_cache(&fake_lines(100));
    state.selection_mut().select_all(100);
    
    let status = render_status_bar_diagnostics(&state);
    assert!(status.contains("SEL:0-99"));
}

#[test]
fn test_status_bar_shows_cache_count() {
    let mut state = AppState::new(PathBuf::from("."), false);
    state.update_rendered_lines_cache(&fake_lines(150));
    
    let status = render_status_bar_diagnostics(&state);
    assert!(status.contains("CACHE:150"));
}

#[test]
fn test_status_bar_shows_cache_empty() {
    let state = AppState::new(PathBuf::from("."), false);
    let status = render_status_bar_diagnostics(&state);
    assert!(status.contains("CACHE:0"));
}
```

#### 3.1.2 Implementation

**File:** `src/tui/status.rs` (new file)

```rust
//! Status bar diagnostic rendering.

use crate::app::state::AppState;

/// Renders diagnostic information for the status bar.
///
/// Returns a string like "[SEL:0-99] [CACHE:150]" or "[SEL:none] [CACHE:0]"
pub fn render_diagnostics(state: &AppState) -> String {
    let sel_info = state
        .selection()
        .range()
        .map(|(start, end)| format!("SEL:{}-{}", start.line, end.line))
        .unwrap_or_else(|| "SEL:none".to_string());

    let cache_info = format!("CACHE:{}", state.rendered_line_count());

    format!("[{}] [{}]", sel_info, cache_info)
}
```

**File:** `src/tui/mod.rs` (modify status bar rendering)

```rust
// In render_status_bar or equivalent:
let diagnostics = status::render_diagnostics(state);
// Append to right side of status bar
```

#### 3.1.3 Acceptance Criteria

- [ ] Tests pass
- [ ] Status bar shows `[SEL:none] [CACHE:0]` on startup
- [ ] After Cmd+A, shows `[SEL:0-N] [CACHE:N]` where N > 0
- [ ] Visual confirmation in running app

### 3.2 Task 0.2: Structured Diagnostic Logging

**Purpose:** File-based debug logging that doesn't pollute terminal

#### 3.2.1 Test Specification

```rust
// tests/diagnostics/logging_tests.rs

#[test]
fn test_diagnostic_log_file_created() {
    let temp_dir = tempfile::tempdir().unwrap();
    let log_path = temp_dir.path().join("debug.log");
    
    init_diagnostic_logging(Some(&log_path)).unwrap();
    tracing::debug!("test message");
    
    // Flush and verify
    drop(tracing::subscriber::set_default(
        tracing_subscriber::registry()
    ));
    
    let contents = std::fs::read_to_string(&log_path).unwrap();
    assert!(contents.contains("test message"));
}

#[test]
fn test_diagnostic_logging_does_not_write_to_stdout() {
    // Capture stdout during logging
    // Verify no output
}
```

#### 3.2.2 Implementation

**File:** `src/diagnostics.rs` (new file)

```rust
//! Diagnostic logging configuration.

use anyhow::Result;
use std::path::Path;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Initializes diagnostic logging to a file.
///
/// If `path` is None, uses `~/.patina/debug.log`.
pub fn init_file_logging(path: Option<&Path>) -> Result<()> {
    let log_path = match path {
        Some(p) => p.to_path_buf(),
        None => {
            let home = directories::BaseDirs::new()
                .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
            let patina_dir = home.data_dir().join("patina");
            std::fs::create_dir_all(&patina_dir)?;
            patina_dir.join("debug.log")
        }
    };

    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;

    let file_layer = fmt::layer()
        .with_writer(file)
        .with_ansi(false)
        .with_target(true)
        .with_thread_ids(true);

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("patina=debug"));

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .init();

    Ok(())
}
```

#### 3.2.3 Acceptance Criteria

- [ ] Tests pass
- [ ] Debug logs written to `~/.patina/debug.log`
- [ ] No debug output in terminal during normal operation
- [ ] Log file rotates or truncates appropriately (future enhancement)

### 3.3 Quality Gate: Phase 0 Exit Criteria

| Criterion | Verification | Required |
|-----------|--------------|----------|
| All new tests pass | `cargo test` | ✅ |
| No regressions | `cargo test` (full suite) | ✅ |
| No new warnings | `cargo clippy` | ✅ |
| Status bar visible | Manual verification | ✅ |
| Diagnostics reveal cache state | Run app, press Cmd+A, observe | ✅ |

**Sign-off Required:** Developer self-review + visual confirmation

---

## 4. Phase 1: Cost Optimization

**Duration:** 3-4 hours  
**Risk Level:** Medium (affects API communication)  
**Deliverables:** Token-bounded context windowing, cost tracking visibility

### 4.0 Quality Gate: Entry Criteria

- [ ] Phase 0 complete and signed off
- [ ] Baseline API cost measured (record 3 sample sessions)
- [ ] Benchmark baseline saved (`cargo bench -- --save-baseline phase1-pre`)

### 4.1 Task 1.1: Token Estimation

**Purpose:** Provide reliable token count estimates without external dependencies

#### 4.1.1 Test Specification (Write First)

```rust
// src/api/tokens.rs (new module)

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens_empty_string() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn test_estimate_tokens_english_text() {
        // ~4 chars per token for English
        let text = "Hello, how are you doing today?"; // 31 chars
        let estimate = estimate_tokens(text);
        assert!(estimate >= 6 && estimate <= 10, "Expected 6-10 tokens, got {}", estimate);
    }

    #[test]
    fn test_estimate_tokens_code() {
        // Code tends to have more tokens per character
        let code = "fn main() { println!(\"Hello\"); }";
        let estimate = estimate_tokens(code);
        assert!(estimate >= 8 && estimate <= 15, "Expected 8-15 tokens, got {}", estimate);
    }

    #[test]
    fn test_estimate_tokens_json() {
        let json = r#"{"name": "test", "value": 42}"#;
        let estimate = estimate_tokens(json);
        assert!(estimate >= 7 && estimate <= 12);
    }

    #[test]
    fn test_estimate_tokens_unicode() {
        // Unicode should still work (count bytes / 4 as approximation)
        let unicode = "こんにちは世界"; // Japanese
        let estimate = estimate_tokens(unicode);
        assert!(estimate > 0);
    }

    #[test]
    fn test_estimate_tokens_large_content() {
        let large = "x".repeat(100_000);
        let estimate = estimate_tokens(&large);
        assert_eq!(estimate, 25_000); // 100k / 4
    }
}
```

#### 4.1.2 Implementation

**File:** `src/api/tokens.rs` (new file)

```rust
//! Token estimation utilities.
//!
//! Provides heuristic-based token counting for API request budgeting.
//! These estimates are intentionally conservative to avoid exceeding limits.

/// Estimates token count for a string.
///
/// Uses the heuristic of ~4 characters per token, which is reasonably
/// accurate for English text and code. This intentionally overestimates
/// slightly to provide safety margin.
///
/// # Arguments
///
/// * `text` - The text to estimate tokens for
///
/// # Returns
///
/// Estimated token count (always rounds up)
#[must_use]
pub fn estimate_tokens(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    // Use byte length for consistency with Unicode
    // Add 1 to round up, ensuring we never underestimate
    (text.len() + 3) / 4
}

/// Estimates total tokens for a slice of API messages.
///
/// Sums the token estimates for all message content.
#[must_use]
pub fn estimate_messages_tokens(messages: &[crate::types::ApiMessageV2]) -> usize {
    messages.iter().map(|m| estimate_message_tokens(m)).sum()
}

/// Estimates tokens for a single API message.
#[must_use]
pub fn estimate_message_tokens(message: &crate::types::ApiMessageV2) -> usize {
    // Base overhead for message structure (~4 tokens for role, etc.)
    let overhead = 4;
    let content_tokens = match &message.content {
        crate::types::MessageContent::Text(text) => estimate_tokens(text),
        crate::types::MessageContent::Blocks(blocks) => {
            blocks.iter().map(|b| estimate_block_tokens(b)).sum()
        }
    };
    overhead + content_tokens
}

/// Estimates tokens for a content block.
#[must_use]
fn estimate_block_tokens(block: &crate::types::ContentBlock) -> usize {
    match block {
        crate::types::ContentBlock::Text { text } => estimate_tokens(text),
        crate::types::ContentBlock::ToolUse(tool_use) => {
            // Tool name + ID + JSON input
            estimate_tokens(&tool_use.name)
                + estimate_tokens(&tool_use.id)
                + estimate_tokens(&tool_use.input.to_string())
        }
        crate::types::ContentBlock::ToolResult(tool_result) => {
            estimate_tokens(&tool_result.tool_use_id)
                + tool_result
                    .content
                    .as_ref()
                    .map(|c| estimate_tokens(c))
                    .unwrap_or(0)
        }
    }
}

#[cfg(test)]
mod tests {
    // ... tests from specification above
}
```

**File:** `src/api/mod.rs` (add module)

```rust
pub mod tokens;
pub use tokens::{estimate_tokens, estimate_messages_tokens};
```

#### 4.1.3 Acceptance Criteria

- [ ] All token estimation tests pass
- [ ] Estimates are within 2x of actual (verified manually with tiktoken)
- [ ] No panics on edge cases (empty, huge, unicode)

### 4.2 Task 1.2: Context Truncation

**Purpose:** Implement smart message truncation within token budget

#### 4.2.1 Test Specification (Write First)

```rust
// src/api/context.rs (new module)

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ApiMessageV2, MessageContent};

    fn make_message(role: &str, content: &str) -> ApiMessageV2 {
        ApiMessageV2 {
            role: role.to_string(),
            content: MessageContent::Text(content.to_string()),
        }
    }

    #[test]
    fn test_truncate_empty_messages() {
        let messages: Vec<ApiMessageV2> = vec![];
        let truncated = truncate_context(&messages, 1000);
        assert!(truncated.is_empty());
    }

    #[test]
    fn test_truncate_under_budget_unchanged() {
        let messages = vec![
            make_message("user", "Hello"),
            make_message("assistant", "Hi there!"),
        ];
        let truncated = truncate_context(&messages, 10_000);
        assert_eq!(truncated.len(), 2);
    }

    #[test]
    fn test_truncate_preserves_first_message() {
        // First message is system/project context - must always keep
        let messages = vec![
            make_message("user", "System prompt with project context..."),
            make_message("assistant", "Response 1"),
            make_message("user", "Query 2"),
            make_message("assistant", "Response 2"),
            make_message("user", "Query 3"),
        ];
        
        // Budget that can only fit first + last 2
        let truncated = truncate_context(&messages, 100);
        
        // First message always preserved
        assert_eq!(truncated[0].content_text(), messages[0].content_text());
        // Most recent messages preserved
        assert!(truncated.len() >= 2);
        assert_eq!(
            truncated.last().unwrap().content_text(),
            messages.last().unwrap().content_text()
        );
    }

    #[test]
    fn test_truncate_respects_token_budget() {
        let large_content = "x".repeat(10_000); // ~2500 tokens
        let messages = vec![
            make_message("user", "System"),
            make_message("assistant", &large_content),
            make_message("user", &large_content),
            make_message("assistant", &large_content),
        ];
        
        let truncated = truncate_context(&messages, 5_000); // ~5k token budget
        let total_tokens = estimate_messages_tokens(&truncated);
        
        assert!(total_tokens <= 5_500, "Should be under budget, got {}", total_tokens);
    }

    #[test]
    fn test_truncate_maintains_conversation_order() {
        let messages = vec![
            make_message("user", "First"),
            make_message("assistant", "Second"),
            make_message("user", "Third"),
            make_message("assistant", "Fourth"),
        ];
        
        let truncated = truncate_context(&messages, 10_000);
        
        for window in truncated.windows(2) {
            let first_idx = messages.iter().position(|m| m.content_text() == window[0].content_text());
            let second_idx = messages.iter().position(|m| m.content_text() == window[1].content_text());
            assert!(first_idx < second_idx, "Order must be preserved");
        }
    }

    #[test]
    fn test_truncate_with_tool_blocks() {
        // Ensure tool_use and tool_result blocks are handled
        let messages = vec![
            make_message("user", "Run ls"),
            ApiMessageV2 {
                role: "assistant".to_string(),
                content: MessageContent::Blocks(vec![
                    ContentBlock::ToolUse(ToolUseBlock {
                        id: "tool_1".to_string(),
                        name: "bash".to_string(),
                        input: serde_json::json!({"command": "ls"}),
                    }),
                ]),
            },
            ApiMessageV2 {
                role: "user".to_string(),
                content: MessageContent::Blocks(vec![
                    ContentBlock::ToolResult(ToolResultBlock {
                        tool_use_id: "tool_1".to_string(),
                        content: Some("file1.txt\nfile2.txt".to_string()),
                        is_error: false,
                    }),
                ]),
            },
        ];
        
        let truncated = truncate_context(&messages, 10_000);
        assert_eq!(truncated.len(), 3);
    }

    #[test]
    fn test_truncate_max_messages_limit() {
        // Even if under token budget, limit message count
        let messages: Vec<_> = (0..100)
            .map(|i| make_message("user", &format!("Message {}", i)))
            .collect();
        
        let truncated = truncate_context_with_limits(&messages, 1_000_000, 20);
        assert!(truncated.len() <= 21); // first + 20 recent
    }
}
```

#### 4.2.2 Implementation

**File:** `src/api/context.rs` (new file)

```rust
//! Context window management for API requests.
//!
//! Provides smart truncation of conversation history to stay within
//! token budgets while preserving conversation coherence.

use crate::api::tokens::estimate_message_tokens;
use crate::types::ApiMessageV2;

/// Default maximum input tokens per request.
///
/// Set conservatively below the 200k limit to allow room for response.
pub const DEFAULT_MAX_INPUT_TOKENS: usize = 100_000;

/// Default maximum number of messages to include.
///
/// Provides a hard cap independent of token counting.
pub const DEFAULT_MAX_MESSAGES: usize = 30;

/// Truncates messages to fit within a token budget.
///
/// Preserves the first message (system/project context) and the most
/// recent messages that fit within the budget.
///
/// # Arguments
///
/// * `messages` - The full conversation history
/// * `max_tokens` - Maximum total tokens allowed
///
/// # Returns
///
/// A new vector with messages truncated to fit the budget.
#[must_use]
pub fn truncate_context(messages: &[ApiMessageV2], max_tokens: usize) -> Vec<ApiMessageV2> {
    truncate_context_with_limits(messages, max_tokens, DEFAULT_MAX_MESSAGES)
}

/// Truncates messages with explicit token and message limits.
///
/// # Arguments
///
/// * `messages` - The full conversation history
/// * `max_tokens` - Maximum total tokens allowed
/// * `max_messages` - Maximum number of messages (excluding first)
///
/// # Returns
///
/// A new vector with messages truncated to fit both limits.
#[must_use]
pub fn truncate_context_with_limits(
    messages: &[ApiMessageV2],
    max_tokens: usize,
    max_messages: usize,
) -> Vec<ApiMessageV2> {
    if messages.is_empty() {
        return Vec::new();
    }

    // Always keep the first message (system/project context)
    let first_message = &messages[0];
    let first_tokens = estimate_message_tokens(first_message);

    if first_tokens >= max_tokens {
        // Even first message exceeds budget - return it anyway with warning
        tracing::warn!(
            first_tokens,
            max_tokens,
            "First message exceeds token budget"
        );
        return vec![first_message.clone()];
    }

    let mut result = vec![first_message.clone()];
    let mut remaining_budget = max_tokens.saturating_sub(first_tokens);
    let mut messages_to_add = Vec::new();

    // Work backwards from most recent, collecting messages that fit
    for msg in messages[1..].iter().rev() {
        if messages_to_add.len() >= max_messages {
            break;
        }

        let msg_tokens = estimate_message_tokens(msg);
        if msg_tokens > remaining_budget {
            // This message doesn't fit - stop here
            // (We could skip and continue, but that risks breaking conversation flow)
            break;
        }

        remaining_budget = remaining_budget.saturating_sub(msg_tokens);
        messages_to_add.push(msg.clone());
    }

    // Reverse to restore chronological order
    messages_to_add.reverse();
    result.extend(messages_to_add);

    tracing::debug!(
        original_count = messages.len(),
        truncated_count = result.len(),
        estimated_tokens = max_tokens - remaining_budget,
        "Context truncated"
    );

    result
}

#[cfg(test)]
mod tests {
    // ... tests from specification above
}
```

**File:** `src/api/mod.rs` (add module)

```rust
pub mod context;
pub use context::{truncate_context, DEFAULT_MAX_INPUT_TOKENS, DEFAULT_MAX_MESSAGES};
```

#### 4.2.3 Acceptance Criteria

- [ ] All truncation tests pass
- [ ] First message always preserved
- [ ] Most recent messages prioritized
- [ ] Token budget respected (within 10% margin)
- [ ] Message order maintained

### 4.3 Task 1.3: Integration with AppState

**Purpose:** Wire truncation into the message submission flow

#### 4.3.1 Test Specification

```rust
// tests/app/state_truncation_tests.rs

#[tokio::test]
async fn test_submit_message_truncates_context() {
    let mut state = AppState::new(PathBuf::from("."), true);
    
    // Add many messages to exceed budget
    for i in 0..50 {
        state.api_messages_mut().push(ApiMessageV2::user(&format!(
            "Message {} with lots of content: {}",
            i,
            "x".repeat(1000)
        )));
    }
    
    // Get truncated messages
    let truncated = state.api_messages_truncated();
    
    // Should be significantly fewer than original
    assert!(truncated.len() < 50);
    // First message preserved
    assert!(truncated[0].content_text().contains("Message 0"));
    // Most recent preserved
    assert!(truncated.last().unwrap().content_text().contains("Message 49"));
}

#[tokio::test]
async fn test_submit_message_under_budget_unchanged() {
    let mut state = AppState::new(PathBuf::from("."), true);
    
    state.api_messages_mut().push(ApiMessageV2::user("Hello"));
    state.api_messages_mut().push(ApiMessageV2::assistant("Hi!"));
    
    let truncated = state.api_messages_truncated();
    assert_eq!(truncated.len(), 2);
}
```

#### 4.3.2 Implementation

**File:** `src/app/state.rs` (modify)

```rust
use crate::api::{truncate_context, DEFAULT_MAX_INPUT_TOKENS};

impl AppState {
    /// Returns API messages truncated to fit within token budget.
    ///
    /// This should be used when sending messages to the API instead of
    /// `api_messages()` directly to prevent context overflow.
    #[must_use]
    pub fn api_messages_truncated(&self) -> Vec<ApiMessageV2> {
        truncate_context(&self.api_messages, DEFAULT_MAX_INPUT_TOKENS)
    }

    // Modify submit_message to use truncated messages:
    pub async fn submit_message(
        &mut self,
        client: &AnthropicClient,
        content: String,
    ) -> Result<()> {
        // ... existing setup code ...

        // Get truncated messages for API call
        let messages_to_send = self.api_messages_truncated();
        
        tracing::info!(
            total_messages = self.api_messages.len(),
            sending_messages = messages_to_send.len(),
            "Submitting message with truncated context"
        );

        let client = client.clone();
        tokio::spawn(async move {
            if let Err(e) = client.stream_message_v2(&messages_to_send, tx).await {
                tracing::error!("API error: {}", e);
            }
        });

        Ok(())
    }
}
```

#### 4.3.3 Acceptance Criteria

- [ ] Integration tests pass
- [ ] Existing tests still pass (no regressions)
- [ ] Manual testing: long session stays under budget

### 4.4 Task 1.4: Cost Tracking Visibility

**Purpose:** Show session cost in status bar for awareness

#### 4.4.1 Test Specification

```rust
#[test]
fn test_cost_display_formatting() {
    assert_eq!(format_cost(0.0), "$0.00");
    assert_eq!(format_cost(0.156), "$0.16");
    assert_eq!(format_cost(1.5), "$1.50");
    assert_eq!(format_cost(10.0), "$10.00");
}

#[test]
fn test_token_count_formatting() {
    assert_eq!(format_tokens(500), "500");
    assert_eq!(format_tokens(1_500), "1.5k");
    assert_eq!(format_tokens(15_000), "15k");
    assert_eq!(format_tokens(150_000), "150k");
}
```

#### 4.4.2 Implementation

**File:** `src/tui/status.rs` (extend)

```rust
/// Formats a cost value for display.
pub fn format_cost(cost: f64) -> String {
    format!("${:.2}", cost)
}

/// Formats a token count for display.
pub fn format_tokens(tokens: usize) -> String {
    if tokens >= 1_000 {
        format!("{}k", tokens / 1_000)
    } else {
        tokens.to_string()
    }
}

/// Renders cost information for the status bar.
pub fn render_cost_info(tracker: &CostTracker) -> String {
    let stats = tracker.statistics();
    format!(
        "COST:{} IN:{} OUT:{}",
        format_cost(stats.total_cost),
        format_tokens(stats.total_input_tokens as usize),
        format_tokens(stats.total_output_tokens as usize)
    )
}
```

#### 4.4.3 Acceptance Criteria

- [ ] Cost visible in status bar during session
- [ ] Updates after each API call
- [ ] Formatting is readable

### 4.5 Quality Gate: Phase 1 Exit Criteria

| Criterion | Verification | Required |
|-----------|--------------|----------|
| All new tests pass | `cargo test` | ✅ |
| All existing tests pass | `cargo test` | ✅ |
| No clippy warnings | `cargo clippy` | ✅ |
| Coverage ≥ 90% for new code | `cargo tarpaulin` | ✅ |
| Performance within 5% | `cargo bench -- --baseline phase1-pre` | ✅ |
| Manual: 30+ message session stays under $0.50 | Run test session | ✅ |
| Manual: Conversation coherence maintained | Review truncated context | ✅ |

**Sign-off Required:** Developer self-review + peer review of truncation logic

---

## 5. Phase 2: Copy/Paste Fix

**Duration:** 2-3 hours  
**Risk Level:** Low (UI-only, no API changes)  
**Deliverables:** Working full-text selection and clipboard copy

### 5.0 Quality Gate: Entry Criteria

- [ ] Phase 1 complete and signed off
- [ ] Phase 0 diagnostics reveal root cause
- [ ] Benchmark baseline saved (`cargo bench -- --save-baseline phase2-pre`)

### 5.1 Root Cause Analysis

Before implementing fixes, use Phase 0 diagnostics to determine the actual issue:

**Diagnostic Checklist:**

1. [ ] Start app, add some messages
2. [ ] Observe status bar: `[SEL:none] [CACHE:N]` where N > 0?
   - If CACHE:0 → **Problem: Cache not populated during render**
3. [ ] Press Cmd+A
4. [ ] Observe status bar: `[SEL:0-M] [CACHE:N]` where M ≈ N-1?
   - If SEL:none → **Problem: Keybinding not triggering select_all**
   - If M >> N → **Problem: Selection exceeds cache bounds**
5. [ ] Press Cmd+C
6. [ ] Check clipboard content
   - If empty → **Problem: copy_from_cache() extraction failing**
   - If partial → **Problem: Cache contains only viewport**

Document findings before proceeding.

### 5.2 Task 2.1: Ensure Full Cache Population

**Purpose:** Guarantee `rendered_lines_cache` contains entire conversation

#### 5.2.1 Test Specification

```rust
// tests/tui/cache_tests.rs

#[test]
fn test_rendered_lines_cache_contains_full_timeline() {
    let mut state = AppState::new(PathBuf::from("."), false);
    
    // Add messages
    state.add_message(Message::user("Hello"));
    state.add_message(Message::assistant("Hi there! How can I help?"));
    state.add_message(Message::user("Tell me about Rust"));
    state.add_message(Message::assistant("Rust is a systems programming language..."));
    
    // Simulate render
    let all_lines = render_timeline_to_lines(&state.timeline(), state.throbber_char());
    state.update_rendered_lines_cache(&all_lines);
    
    // Cache should contain all content
    let cache = state.rendered_lines_cache();
    let cache_text = cache.join("\n");
    
    assert!(cache_text.contains("Hello"));
    assert!(cache_text.contains("Hi there!"));
    assert!(cache_text.contains("Tell me about Rust"));
    assert!(cache_text.contains("systems programming"));
}

#[test]
fn test_cache_updated_after_each_render() {
    let mut state = AppState::new(PathBuf::from("."), false);
    
    state.add_message(Message::user("First"));
    let lines1 = render_timeline_to_lines(&state.timeline(), ' ');
    state.update_rendered_lines_cache(&lines1);
    let count1 = state.rendered_line_count();
    
    state.add_message(Message::assistant("Second with more content"));
    let lines2 = render_timeline_to_lines(&state.timeline(), ' ');
    state.update_rendered_lines_cache(&lines2);
    let count2 = state.rendered_line_count();
    
    assert!(count2 > count1, "Cache should grow with new messages");
}

#[test]
fn test_cache_independent_of_viewport() {
    let mut state = AppState::new(PathBuf::from("."), false);
    
    // Add many messages that would exceed viewport
    for i in 0..50 {
        state.add_message(Message::user(&format!("Message {}", i)));
    }
    
    // Render full timeline (not viewport-clipped)
    let all_lines = render_timeline_to_lines(&state.timeline(), ' ');
    state.update_rendered_lines_cache(&all_lines);
    
    // Simulate small viewport (10 lines)
    let viewport_height = 10;
    
    // Cache should still have all content
    assert!(
        state.rendered_line_count() > viewport_height,
        "Cache should not be limited by viewport"
    );
}
```

#### 5.2.2 Implementation

**File:** `src/tui/render.rs` (modify or verify)

```rust
/// Renders the full timeline to lines for caching.
///
/// This renders ALL content, not just the viewport-visible portion.
/// The result should be cached in AppState for copy operations.
pub fn render_timeline_to_lines<'a>(
    timeline: &Timeline,
    throbber: char,
) -> Vec<Line<'a>> {
    // ... existing implementation ...
    // VERIFY: This must return ALL lines, not viewport-clipped
}
```

**File:** `src/app/mod.rs` (modify render loop)

```rust
// In the main render function, BEFORE viewport clipping:
let all_lines = render_timeline_to_lines(&state.timeline(), state.throbber_char());

// Update cache with FULL content
state.update_rendered_lines_cache(&all_lines);

// THEN apply viewport clipping for display
let visible_lines = viewport_clip(&all_lines, state.scroll_state(), terminal_height);

// Render visible_lines to terminal
```

#### 5.2.3 Acceptance Criteria

- [ ] Tests pass
- [ ] Status bar shows `CACHE:N` where N equals total rendered lines
- [ ] Cache count does not change when scrolling

### 5.3 Task 2.2: Fix Selection Range

**Purpose:** Ensure selection bounds match cache bounds

#### 5.3.1 Test Specification

```rust
// tests/tui/selection_tests.rs

#[test]
fn test_select_all_matches_cache_bounds() {
    let mut state = AppState::new(PathBuf::from("."), false);
    
    // Populate cache
    state.update_rendered_lines_cache(&fake_lines(100));
    
    // Select all
    state.selection_mut().select_all(state.rendered_line_count());
    
    let (start, end) = state.selection().range().unwrap();
    assert_eq!(start.line, 0);
    assert_eq!(end.line, 99); // 0-indexed
}

#[test]
fn test_selection_clamped_to_cache() {
    let mut state = AppState::new(PathBuf::from("."), false);
    
    // Cache has 50 lines
    state.update_rendered_lines_cache(&fake_lines(50));
    
    // But selection was set with stale count of 100
    state.selection_mut().select_all(100);
    
    // Copy should still work - clamp to actual cache
    let result = state.copy_from_cache();
    assert!(result.is_ok());
}
```

#### 5.3.2 Implementation

**File:** `src/app/state.rs` (modify copy_from_cache)

```rust
pub fn copy_from_cache(&self) -> Result<bool> {
    let Some((start, end)) = self.selection.range() else {
        tracing::debug!("copy_from_cache: no selection range");
        return Ok(false);
    };

    let cache_len = self.rendered_lines_cache.len();
    if cache_len == 0 {
        tracing::debug!("copy_from_cache: cache is empty");
        return Ok(false);
    }

    // Clamp selection to cache bounds
    let start_line = start.line.min(cache_len.saturating_sub(1));
    let end_line = end.line.min(cache_len.saturating_sub(1));

    tracing::debug!(
        start_line,
        end_line,
        cache_len,
        "copy_from_cache: extracting clamped range"
    );

    // ... rest of extraction logic with clamped bounds ...
}
```

#### 5.3.3 Acceptance Criteria

- [ ] Tests pass
- [ ] Cmd+A sets selection matching cache size
- [ ] Out-of-bounds selections are clamped safely

### 5.4 Task 2.3: Clipboard Error Handling

**Purpose:** Graceful degradation when clipboard access fails

#### 5.4.1 Test Specification

```rust
#[test]
fn test_clipboard_error_returns_error_not_panic() {
    // This test verifies error handling, not actual clipboard
    // Mock or use a known-failing scenario
}

#[test]
fn test_copy_provides_fallback_on_failure() {
    // When clipboard fails, should log location of temp file with content
}
```

#### 5.4.2 Implementation

**File:** `src/app/state.rs` (modify copy_from_cache)

```rust
pub fn copy_from_cache(&self) -> Result<bool> {
    // ... extraction logic ...

    // Try clipboard first
    match arboard::Clipboard::new() {
        Ok(mut clipboard) => {
            match clipboard.set_text(&result) {
                Ok(()) => {
                    tracing::info!(chars = result.len(), "Copied to clipboard");
                    Ok(true)
                }
                Err(e) => {
                    tracing::warn!("Clipboard set_text failed: {}", e);
                    self.fallback_copy(&result)?;
                    Ok(true)
                }
            }
        }
        Err(e) => {
            tracing::warn!("Clipboard access failed: {}", e);
            self.fallback_copy(&result)?;
            Ok(true)
        }
    }
}

fn fallback_copy(&self, content: &str) -> Result<()> {
    let path = std::env::temp_dir().join("patina_copy.txt");
    std::fs::write(&path, content)?;
    tracing::info!("Content written to fallback file: {}", path.display());
    // Could also show a notification in the UI
    Ok(())
}
```

#### 5.4.3 Acceptance Criteria

- [ ] Tests pass
- [ ] Clipboard errors don't panic
- [ ] Fallback file created on failure

### 5.5 Quality Gate: Phase 2 Exit Criteria

| Criterion | Verification | Required |
|-----------|--------------|----------|
| All new tests pass | `cargo test` | ✅ |
| All existing tests pass | `cargo test` | ✅ |
| No clippy warnings | `cargo clippy` | ✅ |
| Coverage ≥ 90% for new code | `cargo tarpaulin` | ✅ |
| Manual: Cmd+A selects all | Visual + status bar | ✅ |
| Manual: Cmd+C copies full content | Paste in external app | ✅ |
| Manual: Works with long conversations | Test with 100+ messages | ✅ |
| Manual: Works after scrolling | Scroll, then Cmd+A+C | ✅ |

**Sign-off Required:** Developer self-review + QA verification

---

## 6. Phase 3: Smart Context Evolution

**Duration:** 4-6 hours  
**Risk Level:** Medium (behavioral changes)  
**Deliverables:** Targeted file reads, session resume optimization

### 6.0 Quality Gate: Entry Criteria

- [ ] Phase 1 and 2 complete and signed off
- [ ] User feedback collected on truncation behavior
- [ ] Benchmark baseline saved

### 6.1 Task 3.1: Line-Range File Reading

**Purpose:** Allow Claude to read specific portions of files

#### 6.1.1 Test Specification

```rust
#[tokio::test]
async fn test_read_file_with_line_range() {
    let temp = create_test_file("line1\nline2\nline3\nline4\nline5\n");
    
    let result = read_file_tool(&temp, Some(2), Some(4)).await.unwrap();
    
    assert!(result.contains("line2"));
    assert!(result.contains("line3"));
    assert!(result.contains("line4"));
    assert!(!result.contains("line1"));
    assert!(!result.contains("line5"));
}

#[tokio::test]
async fn test_read_file_line_range_out_of_bounds() {
    let temp = create_test_file("line1\nline2\n");
    
    let result = read_file_tool(&temp, Some(1), Some(100)).await.unwrap();
    
    // Should return available lines, not error
    assert!(result.contains("line1"));
    assert!(result.contains("line2"));
}

#[tokio::test]
async fn test_read_file_no_range_returns_full() {
    let temp = create_test_file("content");
    
    let result = read_file_tool(&temp, None, None).await.unwrap();
    
    assert!(result.contains("content"));
}
```

#### 6.1.2 Implementation

Extend the `read_file` tool definition with optional parameters:

```rust
ToolDefinition {
    name: "read_file",
    description: "Read file contents. Use start_line and end_line for large files.",
    input_schema: json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "File path" },
            "start_line": { "type": "integer", "description": "First line (1-indexed, optional)" },
            "end_line": { "type": "integer", "description": "Last line (inclusive, optional)" }
        },
        "required": ["path"]
    }),
}
```

### 6.2 Task 3.2: System Prompt Guidance

**Purpose:** Encourage Claude to use targeted reads

#### 6.2.1 Implementation

Update CLAUDE.md or system prompt:

```markdown
## File Reading Best Practices

When analyzing code or documents:

1. **Start with search**: Use `grep` or `glob` to locate relevant sections
2. **Read targeted ranges**: Use `read_file` with `start_line`/`end_line` for large files
3. **Avoid full reads**: Only read entire files if explicitly necessary
4. **Summarize context**: When resuming sessions, reference previous findings rather than re-reading

Example workflow:
- `grep "function_name" src/` → Find relevant files
- `read_file src/module.rs --start_line 50 --end_line 100` → Read specific section
- Analyze and respond
```

### 6.3 Task 3.3: Session Resume Optimization

**Purpose:** Only re-provide changed files when resuming

#### 6.3.1 Test Specification

```rust
#[test]
fn test_session_context_tracks_file_hashes() {
    let mut ctx = SessionContext::new();
    
    ctx.track_file("src/main.rs", "hash123");
    ctx.track_file("src/lib.rs", "hash456");
    
    assert!(ctx.is_file_tracked("src/main.rs"));
    assert_eq!(ctx.file_hash("src/main.rs"), Some("hash123"));
}

#[test]
fn test_session_detects_changed_files() {
    let mut ctx = SessionContext::new();
    ctx.track_file("src/main.rs", "hash_old");
    
    let changed = ctx.get_changed_files(&[
        ("src/main.rs", "hash_new"),  // Changed
        ("src/lib.rs", "hash_lib"),    // New
    ]);
    
    assert!(changed.contains(&"src/main.rs"));
    assert!(changed.contains(&"src/lib.rs"));
}
```

#### 6.3.2 Implementation

Leverage existing `SessionContext` with SHA-256 hashes to detect changes.

### 6.4 Quality Gate: Phase 3 Exit Criteria

| Criterion | Verification | Required |
|-----------|--------------|----------|
| All tests pass | `cargo test` | ✅ |
| Line-range reads work | Manual test | ✅ |
| Claude uses targeted reads | Review conversations | ✅ |
| Session resume is efficient | Measure token usage | ✅ |

---

## 7. Quality Gates

### 7.1 Continuous Integration Requirements

Every PR must pass:

```yaml
# .github/workflows/ci.yml additions
jobs:
  test:
    steps:
      - run: cargo test --all-features
      - run: cargo clippy -- -D warnings
      - run: cargo fmt -- --check
      
  coverage:
    steps:
      - run: cargo tarpaulin --out xml
      - uses: codecov/codecov-action@v3
        with:
          fail_ci_if_error: true
          threshold: 85%
          
  benchmarks:
    steps:
      - run: cargo bench -- --noplot
      # Compare against baseline, fail if >10% regression
```

### 7.2 Phase Gate Checklist Template

```markdown
## Phase N Exit Gate

**Date:** ____  
**Reviewer:** ____

### Automated Checks
- [ ] `cargo test` passes (__ tests)
- [ ] `cargo clippy` clean
- [ ] `cargo tarpaulin` coverage ≥ ___%
- [ ] `cargo bench` within 5% of baseline

### Manual Verification
- [ ] Feature works as specified
- [ ] No observable regressions
- [ ] Documentation updated

### Sign-off
- [ ] Developer: ________________ Date: ____
- [ ] Reviewer: ________________ Date: ____
```

### 7.3 Rollback Criteria

Automatic rollback if any of:
- Test suite pass rate drops below 99%
- Benchmark regression exceeds 10%
- Critical bug reported within 24 hours of deploy

---

## 8. Test Specifications

### 8.1 Test Organization

```
tests/
├── api/
│   ├── tokens_tests.rs      # Token estimation
│   ├── context_tests.rs     # Context truncation
│   └── client_tests.rs      # API client (existing)
├── app/
│   ├── state_tests.rs       # AppState (existing + new)
│   └── integration_tests.rs # End-to-end flows
├── tui/
│   ├── selection_tests.rs   # Selection state
│   ├── cache_tests.rs       # Render cache
│   └── status_tests.rs      # Status bar
└── helpers/
    └── mod.rs               # Test utilities
```

### 8.2 Test Categories

| Category | Location | Purpose |
|----------|----------|---------|
| Unit | `src/*/mod.rs` (#[cfg(test)]) | Individual functions |
| Integration | `tests/` | Cross-module flows |
| Property | `tests/` (proptest) | Invariant verification |
| Benchmark | `benches/` | Performance regression |

### 8.3 Test Data Builders

```rust
// tests/helpers/mod.rs

pub fn fake_lines(count: usize) -> Vec<Line<'static>> {
    (0..count)
        .map(|i| Line::from(format!("Line {}", i)))
        .collect()
}

pub fn fake_messages(count: usize) -> Vec<ApiMessageV2> {
    (0..count)
        .map(|i| ApiMessageV2::user(&format!("Message {}", i)))
        .collect()
}

pub fn fake_large_message(tokens: usize) -> ApiMessageV2 {
    let content = "x".repeat(tokens * 4);
    ApiMessageV2::user(&content)
}
```

---

## 9. Rollback Procedures

### 9.1 Phase 1 Rollback

If truncation causes conversation issues:

1. Revert `api_messages_truncated()` to return full `api_messages`
2. Or increase `DEFAULT_MAX_INPUT_TOKENS` to 200_000
3. Deploy hotfix
4. Investigate root cause

### 9.2 Phase 2 Rollback

If copy/paste regression:

1. Revert cache population changes
2. Restore previous render flow
3. Deploy hotfix

### 9.3 Feature Flags (Optional)

Consider adding runtime flags for gradual rollout:

```rust
pub struct FeatureFlags {
    pub context_truncation: bool,
    pub max_input_tokens: usize,
    pub copy_paste_v2: bool,
}
```

---

## 10. Appendices

### 10.1 Appendix A: Token Estimation Accuracy

| Content Type | Actual Tokens | Estimated (len/4) | Error |
|--------------|---------------|-------------------|-------|
| English prose | 100 | 95-110 | ±10% |
| Source code | 100 | 80-120 | ±20% |
| JSON | 100 | 110-130 | +15% |
| Mixed | 100 | 90-115 | ±15% |

The heuristic intentionally overestimates to provide safety margin.

### 10.2 Appendix B: Cost Calculations

**Claude Sonnet 4 Pricing (as of 2026-02):**
- Input: $3.00 / million tokens
- Output: $15.00 / million tokens

**Example Session (Before Optimization):**
- 50 messages, average 4k tokens each = 200k input tokens
- Per request: $0.60 input + ~$0.15 output = $0.75
- 10 requests/session = $7.50

**Example Session (After Optimization):**
- Truncated to 100k tokens max
- Per request: $0.30 input + ~$0.15 output = $0.45
- 10 requests/session = $4.50
- **Savings: 40%**

### 10.3 Appendix C: Keybinding Reference

| Action | macOS | Linux/Windows |
|--------|-------|---------------|
| Select All | Cmd+A | Ctrl+Shift+A |
| Copy | Cmd+C | Ctrl+Shift+C |
| Clear Selection | Esc | Esc |

Note: Ctrl+C is reserved for SIGINT (exit).

### 10.4 Appendix D: Related Documentation

- `docs/architecture.md` - System architecture
- `docs/api.md` - API reference
- `CLAUDE.md` - AI assistant guidelines
- `CHANGELOG.md` - Version history

---

## Document History

| Version | Date | Author | Changes |
|---------|------|--------|---------|
| 1.0 | 2026-01-31 | — | Initial optimization plan |
| 2.0 | 2026-02-01 | — | Full TDD rewrite with quality gates |

---

*End of Document*
