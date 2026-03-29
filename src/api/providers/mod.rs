//! LLM provider implementations.
//!
//! This module contains provider-specific types and implementations for
//! different LLM API formats. Each provider has its own message format
//! that gets translated to/from Patina's internal [`StreamEvent`] protocol.
//!
//! # Providers
//!
//! - [`openai_types`]: OpenAI Chat Completions API message types (used by OpenRouter)
//! - [`openrouter`]: Bidirectional message format translation (Anthropic <-> OpenAI)
//! - [`bedrock`]: AWS Bedrock provider with SigV4 request signing
//! - [`vertex`]: Google Vertex AI provider with OAuth2 authentication

pub mod bedrock;
pub mod openai_types;
pub mod openrouter;
pub mod vertex;
