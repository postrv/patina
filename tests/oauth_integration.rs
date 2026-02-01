//! OAuth integration test suite for Patina.
//!
//! Tests for OAuth 2.0 authorization flow including:
//! - Callback URL parsing
//! - Token exchange
//! - State parameter CSRF protection
//! - PKCE verification

#[path = "integration/oauth_test.rs"]
mod oauth_test;
