//! Chat module for LLM-powered completions.
//!
//! This module provides RAG (Retrieval-Augmented Generation) chat completions
//! by combining Meilisearch's hybrid search with LLM providers (OpenAI, Anthropic,
//! Azure OpenAI, Mistral, vLLM).

pub mod completions;
pub mod config;
