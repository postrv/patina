//! Narsil integration module for code intelligence.
//!
//! This module provides deep integration with narsil-mcp for:
//! - Code graph analysis
//! - Security scanning
//! - Context suggestion
//! - Call graph navigation
//!
//! # Example
//!
//! ```ignore
//! use patina::narsil::NarsilIntegration;
//!
//! let integration = NarsilIntegration::new("/path/to/project").await?;
//! if integration.has_capability(NarsilCapability::SecurityScan) {
//!     let findings = integration.scan_security().await?;
//! }
//! ```

pub mod context;
pub mod integration;

pub use context::{
    extract_code_references, CodeReference, ContextKind, ContextSuggestion, LineRef,
};
pub use integration::{NarsilCapabilities, NarsilCapability, NarsilIntegration};
