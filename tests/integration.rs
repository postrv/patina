//! Integration tests for Patina.
//!
//! Tests that verify component interactions and end-to-end behavior.

mod common;

use common::TestContext;

#[test]
fn test_context_setup() {
    let ctx = TestContext::new();
    assert!(
        ctx.path().exists(),
        "test context should create temp directory"
    );
}
