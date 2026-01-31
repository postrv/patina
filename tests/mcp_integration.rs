//! MCP integration test suite for RCT.
//!
//! Tests for MCP transport, server communication, and security validation.

#[path = "integration/mcp_transport_test.rs"]
mod mcp_transport_test;

#[path = "integration/mcp_sse_transport_test.rs"]
mod mcp_sse_transport_test;

#[path = "integration/mcp_test.rs"]
mod mcp_test;
