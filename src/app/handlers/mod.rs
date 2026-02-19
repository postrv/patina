//! Event handlers for the dispatched event loop architecture.
//!
//! Each handler in this module implements [`EventHandler`](super::dispatch::EventHandler)
//! and is responsible for a single concern. Handlers are registered with
//! [`EventDispatcher`](super::dispatch::EventDispatcher) in priority order.

pub mod keyboard;
pub mod permission;
pub mod session;
pub mod stream;
pub mod tick;
