//! LLM provider implementations.
//!
//! This module contains provider-specific types and implementations for
//! different LLM API formats. Each provider has its own message format
//! that gets translated to/from Patina's internal [`StreamEvent`] protocol.
//!
//! # Providers
//!
//! - [`openai_types`]: OpenAI Chat Completions API message types (used by OpenRouter)

pub mod openai_types;
