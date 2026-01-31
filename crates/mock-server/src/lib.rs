//! Mock OpenAI-compatible server for testing Meilisearch vector search and LLM integrations.
//!
//! Provides two endpoints:
//! - `/v1/embeddings` - Returns deterministic hash-based embeddings
//! - `/v1/chat/completions` - Returns rule-based chat responses with optional SSE streaming

pub mod chat;
pub mod embeddings;
pub mod error;
pub mod server;

pub use error::{MockServerError, MockServerResult};
pub use server::{MockServer, MockServerConfig};
