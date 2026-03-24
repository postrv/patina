//! Tool execution integration tests.
//!
//! Tests for tool execution including:
//! - Bash command execution
//! - Security policy enforcement
//! - File operations
//! - Search tools (glob, grep)
//! - Hook integration
//! - Parallel execution

#[path = "../common/mod.rs"]
mod common;

mod async_test;
mod bash_test;
mod file_test;
mod hooks_test;
mod permission_test;
mod search_test;
mod security_test;
