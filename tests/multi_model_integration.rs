//! Multi-model integration test suite for Patina.
//!
//! Tests for multi-model provider support.
//! Gated behind the `multi-model` feature flag.

#![cfg(feature = "multi-model")]

#[path = "integration/multi_model_test.rs"]
mod multi_model_test;
