//! Continuous coding automation module.
//!
//! This module provides types and infrastructure for continuous coding sessions,
//! allowing plugins to hook into the automation loop and react to events.
//!
//! # Submodules
//!
//! - [`events`]: Event types emitted during continuous coding sessions
//! - [`plugin`]: Plugin trait and quality gate definitions

pub mod events;
pub mod plugin;

pub use events::ContinuousEvent;
pub use plugin::{ContinuousCodingPlugin, QualityGate};
