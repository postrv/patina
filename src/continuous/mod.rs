//! Continuous coding automation module.
//!
//! This module provides types and infrastructure for continuous coding sessions,
//! allowing plugins to hook into the automation loop and react to events.
//!
//! # Submodules
//!
//! - [`events`]: Event types emitted during continuous coding sessions

pub mod events;

pub use events::ContinuousEvent;
