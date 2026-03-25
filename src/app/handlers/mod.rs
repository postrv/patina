//! Event handlers for the dispatched event loop architecture.
//!
//! Each handler in this module implements [`EventHandler`](super::dispatch::EventHandler)
//! and is responsible for a single concern. Handlers are registered with
//! [`EventDispatcher`](super::dispatch::EventDispatcher) in priority order.

pub mod agent;
pub mod completion;
pub mod continuous;
pub mod keyboard;
pub mod permission;
pub mod plan;
pub mod question;
pub mod session;
pub mod stream;
pub mod tick;
