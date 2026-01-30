// Allow large error types since we wrap milli::Error and index_scheduler::Error
// which have large variants that we can't control
#![allow(clippy::result_large_err)]

//! # meilisearch-lib
//!
//! Embedded Meilisearch library for direct Rust integration.
//!
//! This crate provides a Rust API for interacting with Meilisearch without going
//! through the HTTP server. It wraps the `index-scheduler` crate and provides
//! an ergonomic interface for building search-powered applications entirely in Rust.
//!
//! ## Features
//!
//! - **Index Management**: Create, delete, list, and manage indexes
//! - **Document Operations**: Add, update, delete, and retrieve documents
//! - **Hybrid Search**: Combine keyword and semantic (vector) search
//! - **Task Management**: Monitor asynchronous operations with task polling
//! - **Settings Management**: Configure searchable attributes, embedders, and more
//! - **Chat Completions**: RAG pipeline with OpenAI, Anthropic, and other LLM providers
//! - **Thread-Safe**: `Send + Sync` implementation for concurrent access
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use meilisearch_lib::{MeilisearchLib, Config};
//! use serde_json::json;
//!
//! fn main() -> Result<(), meilisearch_lib::Error> {
//!     // 1. Create an embedded instance
//!     let meili = MeilisearchLib::new(
//!         Config::builder()
//!             .db_path("/tmp/meilisearch-data")
//!             .build()?
//!     )?;
//!
//!     // 2. Create an index
//!     let task = meili.create_index("movies", Some("id".to_string()))?;
//!     meili.wait_for_task(task.uid, None)?;
//!
//!     // 3. Add documents
//!     let docs = vec![
//!         json!({"id": 1, "title": "The Matrix", "genre": "sci-fi"}),
//!         json!({"id": 2, "title": "Inception", "genre": "sci-fi"}),
//!     ];
//!     let task = meili.add_documents("movies", docs, None)?;
//!     meili.wait_for_task(task.uid, None)?;
//!
//!     // 4. Search
//!     let results = meili.search("movies", meilisearch_lib::SearchQuery::new("matrix"))?;
//!     for hit in results.hits {
//!         println!("Found: {}", hit.document);
//!     }
//!
//!     // 5. Cleanup
//!     meili.shutdown()?;
//!     Ok(())
//! }
//! ```
//!
//! ## Hybrid Search
//!
//! Combine keyword and semantic search for better results:
//!
//! ```rust,ignore
//! use meilisearch_lib::{SearchQuery, HybridQuery};
//! use serde_json::json;
//!
//! // First, configure an embedder for semantic search
//! let embedders = json!({
//!     "default": {
//!         "source": "openAi",
//!         "apiKey": "sk-...",
//!         "model": "text-embedding-3-small",
//!         "documentTemplate": "A movie titled '{{doc.title}}'"
//!     }
//! });
//! meili.update_embedders("movies", embedders)?;
//!
//! // Then use hybrid search
//! let query = SearchQuery::new("futuristic action")
//!     .with_hybrid(HybridQuery::new(0.7)); // 70% semantic, 30% keyword
//!
//! let results = meili.search("movies", query)?;
//! ```
//!
//! ## Chat Completions (RAG)
//!
//! Use Meilisearch as the retrieval backend for LLM-powered chat:
//!
//! ```rust,ignore
//! use meilisearch_lib::{ChatConfig, ChatSource, ChatRequest, Message, ChatPrompts};
//! use std::collections::HashMap;
//!
//! // Configure the LLM provider
//! let chat_config = ChatConfig {
//!     source: ChatSource::OpenAi,
//!     api_key: "sk-...".to_string(),
//!     base_url: None,
//!     model: "gpt-4".to_string(),
//!     org_id: None,
//!     project_id: None,
//!     api_version: None,
//!     deployment_id: None,
//!     prompts: ChatPrompts::default(),
//!     index_configs: HashMap::new(),
//! };
//! meili.set_chat_config(Some(chat_config));
//!
//! // Send a chat request
//! let response = meili.chat_completion(ChatRequest {
//!     messages: vec![Message::user("What sci-fi movies do you have?")],
//!     index_uid: "movies".to_string(),
//!     stream: false,
//! }).await?;
//!
//! println!("Response: {}", response.content);
//! println!("Sources: {:?}", response.sources);
//! ```
//!
//! ## Error Handling
//!
//! All operations return `Result<T, meilisearch_lib::Error>`. The error type
//! provides detailed information including HTTP-compatible error codes:
//!
//! ```rust,ignore
//! match meili.get_index("nonexistent") {
//!     Ok(index) => println!("Found: {}", index.uid),
//!     Err(meilisearch_lib::Error::IndexNotFound(uid)) => {
//!         println!("Index '{}' not found (404)", uid);
//!     }
//!     Err(e) => {
//!         println!("Error: {} (status {})", e, e.status_code());
//!     }
//! }
//! ```
//!
//! ## Module Overview
//!
//! - [`MeilisearchLib`]: Main client for all operations
//! - [`Config`] / [`ConfigBuilder`]: Configuration for the embedded instance
//! - [`Error`]: Unified error type with error codes
//! - [`SearchQuery`] / [`SearchResult`]: Search request and response types
//! - [`HybridQuery`]: Configuration for hybrid (keyword + semantic) search
//! - [`TaskView`] / [`TaskStatus`]: Task monitoring types
//! - [`Settings`]: Index settings configuration
//! - [`ChatConfig`] / [`ChatRequest`] / [`ChatResponse`]: Chat completion types

mod chat;
mod client;
mod config;
mod error;
mod indexes;
mod tasks;

// Re-exports
pub use client::{Health, MeilisearchLib};
pub use config::{Config, ConfigBuilder};
pub use error::Error;

// Index-related exports
pub use indexes::{IndexStats, IndexView};

// Search-related exports
pub use indexes::search::{HitsInfo, HybridQuery, SearchHit, SearchQuery, SearchResult};

// Task-related exports
pub use tasks::{TaskError, TaskView};

// Chat-related exports
pub use chat::completions::{ChatChunk, ChatRequest, ChatResponse, Message, Role, Usage};
pub use chat::config::{ChatConfig, ChatIndexConfig, ChatPrompts, ChatSearchParams, ChatSource};

// Re-export useful types from meilisearch_types
pub use meilisearch_types::tasks::{Kind as TaskKind, Status as TaskStatus};

// Settings types
pub use indexes::settings::{Checked, SecretPolicy, SettingEmbeddingSettings, Unchecked};
pub use meilisearch_types::milli::update::Setting;
pub use meilisearch_types::settings::Settings;

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
