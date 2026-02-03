//! Continuous coding automation module.
//!
//! This module provides types and infrastructure for continuous coding sessions,
//! allowing plugins to hook into the automation loop and react to events.
//!
//! # Library-Only
//!
//! This module provides building blocks for programmatic integration with Patina.
//! The types are tested and stable but not exposed through the CLI. They are intended
//! for:
//!
//! - Building custom automation tools on top of Patina
//! - Implementing CI/CD integrations
//! - Creating IDE extensions that need fine-grained control
//!
//! # Usage Example
//!
//! ```rust,ignore
//! use patina::continuous::{ContinuousCodingPlugin, ContinuousEvent, QualityGate};
//!
//! struct MyPlugin;
//!
//! impl ContinuousCodingPlugin for MyPlugin {
//!     fn name(&self) -> &str { "my-plugin" }
//!
//!     fn on_event(&mut self, event: &ContinuousEvent) {
//!         match event {
//!             ContinuousEvent::IterationComplete { iteration, .. } => {
//!                 println!("Iteration {} complete", iteration);
//!             }
//!             _ => {}
//!         }
//!     }
//! }
//! ```
//!
//! # Submodules
//!
//! - [`events`]: Event types emitted during continuous coding sessions
//! - [`plugin`]: Plugin trait and quality gate definitions

pub mod events;
pub mod plugin;

pub use events::ContinuousEvent;
pub use plugin::{ContinuousCodingPlugin, QualityGate};
