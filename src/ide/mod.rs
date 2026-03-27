//! IDE integration - VS Code and JetBrains extension support
//!
//! This module provides TCP server infrastructure for IDE extensions to
//! communicate with Patina. See [`protocol`] for message format documentation.
//!
//! # Modules
//!
//! - [`protocol`] - Message types and serialization
//! - [`handlers`] - Request handlers
//! - [`controller`] - Server lifecycle management

pub mod controller;
pub mod handlers;
pub mod protocol;
