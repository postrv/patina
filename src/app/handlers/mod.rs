//! Event handlers for the dispatched event loop architecture.
//!
//! Each handler in this module implements [`EventHandler`](super::dispatch::EventHandler)
//! and is responsible for a single concern. Handlers are registered with
//! [`EventDispatcher`](super::dispatch::EventDispatcher) in priority order.

pub mod session;
pub mod tick;
