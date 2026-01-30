//! Chat completion types and operations.
//!
//! This module provides the RAG (Retrieval-Augmented Generation) pipeline for
//! chat completions, combining Meilisearch's hybrid search with LLM providers.

use std::pin::Pin;

use futures::Stream;
use serde::{Deserialize, Serialize};

use crate::chat::config::{ChatConfig, ChatIndexConfig, ChatSource};
use crate::error::Error;
use crate::indexes::search::{HybridQuery, SearchQuery};
use crate::MeilisearchLib;

// ============================================================================
// Message Types
// ============================================================================

/// Message role.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// System message.
    System,
    /// User message.
    User,
    /// Assistant message.
    Assistant,
    /// Tool message.
    Tool,
}

/// Chat message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Message role.
    pub role: Role,
    /// Message content.
    pub content: String,
    /// Tool call ID (for tool messages).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    /// Create a user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: Role::User, content: content.into(), tool_call_id: None }
    }

    /// Create an assistant message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: content.into(), tool_call_id: None }
    }

    /// Create a system message.
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: Role::System, content: content.into(), tool_call_id: None }
    }

    /// Create a tool message.
    pub fn tool(content: impl Into<String>, tool_call_id: impl Into<String>) -> Self {
        Self { role: Role::Tool, content: content.into(), tool_call_id: Some(tool_call_id.into()) }
    }
}

// ============================================================================
// Request/Response Types
// ============================================================================

/// Chat completion request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    /// Conversation messages.
    pub messages: Vec<Message>,
    /// Index to search for context.
    pub index_uid: String,
    /// Whether to stream the response.
    #[serde(default)]
    pub stream: bool,
}

/// Chat completion response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    /// Response content.
    pub content: String,
    /// Sources used (document IDs).
    pub sources: Vec<String>,
    /// Token usage.
    pub usage: Option<Usage>,
}

/// Token usage statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    /// Number of tokens in the prompt.
    pub prompt_tokens: u32,
    /// Number of tokens in the completion.
    pub completion_tokens: u32,
    /// Total number of tokens.
    pub total_tokens: u32,
}

/// Streaming chat chunk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChunk {
    /// Incremental content.
    pub delta: String,
    /// Is this the final chunk?
    pub done: bool,
    /// Sources used (only present in first/last chunk).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<String>>,
}

// ============================================================================
// Internal Types
// ============================================================================

/// Internal response from provider calls.
struct ProviderResponse {
    content: String,
    usage: Option<Usage>,
}

// ============================================================================
// MeilisearchLib Chat Implementation
// ============================================================================

impl MeilisearchLib {
    /// Execute a chat completion (non-streaming).
    ///
    /// This implements a RAG (Retrieval-Augmented Generation) pipeline:
    /// 1. Extract query from the last user message
    /// 2. Execute hybrid search for context retrieval
    /// 3. Format context using document templates
    /// 4. Call the LLM provider with system prompt + context
    /// 5. Return the response with source citations
    ///
    /// # Arguments
    ///
    /// * `request` - The chat completion request containing messages and index_uid
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Chat is not configured (`ChatNotConfigured`)
    /// - The index doesn't exist
    /// - The LLM provider returns an error
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use meilisearch_lib::{ChatRequest, Message};
    ///
    /// let response = meili.chat_completion(ChatRequest {
    ///     messages: vec![Message::user("What products do you have?")],
    ///     index_uid: "products".into(),
    ///     stream: false,
    /// }).await?;
    ///
    /// println!("Response: {}", response.content);
    /// println!("Sources: {:?}", response.sources);
    /// ```
    pub async fn chat_completion(&self, request: ChatRequest) -> Result<ChatResponse, Error> {
        let config = self.get_chat_config().ok_or(Error::ChatNotConfigured)?;

        // 1. Extract query from last user message
        let query = request
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(|m| m.content.clone())
            .unwrap_or_default();

        // 2. Get index-specific config
        let index_config = config.index_configs.get(&request.index_uid);

        // 3. Execute hybrid search for context
        let search_params = index_config.and_then(|c| c.search_params.as_ref());

        let search_query = SearchQuery {
            q: Some(query.clone()),
            hybrid: Some(HybridQuery {
                semantic_ratio: search_params.and_then(|p| p.semantic_ratio).unwrap_or(0.5),
                embedder: search_params.and_then(|p| p.embedder.clone()),
            }),
            limit: search_params.and_then(|p| p.limit).unwrap_or(10),
            ..Default::default()
        };

        let search_results = self.search(&request.index_uid, search_query)?;

        // 4. Format context from search results
        let hits: Vec<serde_json::Value> =
            search_results.hits.iter().map(|h| h.document.clone()).collect();

        let context = format_context(&hits, index_config);

        // Extract source IDs
        let sources: Vec<String> = hits
            .iter()
            .filter_map(|h| {
                h.get("id").or_else(|| h.get("_id")).and_then(|v| match v {
                    serde_json::Value::String(s) => Some(s.clone()),
                    serde_json::Value::Number(n) => Some(n.to_string()),
                    _ => None,
                })
            })
            .collect();

        // 5. Build LLM request with system prompt + context
        let system_prompt = config.prompts.system.clone().unwrap_or_else(default_system_prompt);

        let full_prompt = format!("{}\n\nContext:\n{}", system_prompt, context);

        // 6. Call provider
        let response = match config.source {
            ChatSource::OpenAi => call_openai(&config, &full_prompt, &request.messages).await?,
            ChatSource::Anthropic => {
                call_anthropic(&config, &full_prompt, &request.messages).await?
            }
            ChatSource::AzureOpenAi => {
                call_azure_openai(&config, &full_prompt, &request.messages).await?
            }
            ChatSource::Mistral => call_mistral(&config, &full_prompt, &request.messages).await?,
            ChatSource::VLlm => call_vllm(&config, &full_prompt, &request.messages).await?,
        };

        Ok(ChatResponse { content: response.content, sources, usage: response.usage })
    }

    /// Execute a streaming chat completion.
    ///
    /// Same RAG pipeline as `chat_completion`, but returns a stream of chunks
    /// instead of waiting for the complete response.
    ///
    /// # Arguments
    ///
    /// * `request` - The chat completion request
    ///
    /// # Returns
    ///
    /// A `Stream` of `ChatChunk` items. The first chunk may contain sources,
    /// and the last chunk will have `done: true`.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use futures::StreamExt;
    /// use meilisearch_lib::{ChatRequest, Message};
    ///
    /// let mut stream = meili.chat_completion_stream(ChatRequest {
    ///     messages: vec![Message::user("What products do you have?")],
    ///     index_uid: "products".into(),
    ///     stream: true,
    /// }).await?;
    ///
    /// while let Some(chunk) = stream.next().await {
    ///     match chunk {
    ///         Ok(chunk) => {
    ///             print!("{}", chunk.delta);
    ///             if chunk.done {
    ///                 println!("\nDone!");
    ///             }
    ///         }
    ///         Err(e) => eprintln!("Error: {}", e),
    ///     }
    /// }
    /// ```
    pub async fn chat_completion_stream(
        &self,
        request: ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<ChatChunk, Error>> + Send>>, Error> {
        let config = self.get_chat_config().ok_or(Error::ChatNotConfigured)?;

        // 1. Extract query from last user message
        let query = request
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(|m| m.content.clone())
            .unwrap_or_default();

        // 2. Get index-specific config and execute search for context
        let index_config = config.index_configs.get(&request.index_uid).cloned();
        let search_params = index_config.as_ref().and_then(|c| c.search_params.as_ref());

        let search_query = SearchQuery {
            q: Some(query.clone()),
            hybrid: Some(HybridQuery {
                semantic_ratio: search_params.and_then(|p| p.semantic_ratio).unwrap_or(0.5),
                embedder: search_params.and_then(|p| p.embedder.clone()),
            }),
            limit: search_params.and_then(|p| p.limit).unwrap_or(10),
            ..Default::default()
        };

        let search_results = self.search(&request.index_uid, search_query)?;

        // 3. Format context from search results
        let hits: Vec<serde_json::Value> =
            search_results.hits.iter().map(|h| h.document.clone()).collect();

        let context = format_context(&hits, index_config.as_ref());

        // Extract source IDs for the first chunk
        let sources: Vec<String> = hits
            .iter()
            .filter_map(|h| {
                h.get("id").or_else(|| h.get("_id")).and_then(|v| match v {
                    serde_json::Value::String(s) => Some(s.clone()),
                    serde_json::Value::Number(n) => Some(n.to_string()),
                    _ => None,
                })
            })
            .collect();

        // 4. Build system prompt with context
        let system_prompt = config.prompts.system.clone().unwrap_or_else(default_system_prompt);
        let full_prompt = format!("{}\n\nContext:\n{}", system_prompt, context);

        // 5. Create provider-specific streaming request
        let http_client = reqwest::Client::new();

        let stream: Pin<Box<dyn Stream<Item = Result<ChatChunk, Error>> + Send>> = match config
            .source
        {
            ChatSource::OpenAi => {
                stream_openai(&http_client, &config, &full_prompt, &request.messages, sources)
                    .await?
            }
            ChatSource::Anthropic => {
                stream_anthropic(&http_client, &config, &full_prompt, &request.messages, sources)
                    .await?
            }
            ChatSource::AzureOpenAi => {
                stream_azure_openai(&http_client, &config, &full_prompt, &request.messages, sources)
                    .await?
            }
            ChatSource::Mistral => {
                stream_mistral(&http_client, &config, &full_prompt, &request.messages, sources)
                    .await?
            }
            ChatSource::VLlm => {
                stream_vllm(&http_client, &config, &full_prompt, &request.messages, sources).await?
            }
        };

        Ok(stream)
    }
}

// ============================================================================
// Context Formatting
// ============================================================================

/// Format search results into context for the LLM.
fn format_context(hits: &[serde_json::Value], config: Option<&ChatIndexConfig>) -> String {
    let max_docs =
        config.and_then(|c| c.search_params.as_ref().and_then(|p| p.limit)).unwrap_or(10);

    let max_bytes = config.and_then(|c| c.max_bytes).unwrap_or(400);

    hits.iter()
        .take(max_docs)
        .enumerate()
        .map(|(idx, doc)| {
            let formatted = if let Some(template) = config.and_then(|c| c.template.as_ref()) {
                apply_template(template, doc)
            } else {
                // Default: compact JSON
                serde_json::to_string(doc).unwrap_or_default()
            };

            // Truncate to max_bytes if needed
            let truncated = if formatted.len() > max_bytes {
                let mut end = max_bytes;
                // Avoid cutting in the middle of a UTF-8 character
                while !formatted.is_char_boundary(end) && end > 0 {
                    end -= 1;
                }
                format!("{}...", &formatted[..end])
            } else {
                formatted
            };

            format!("[{}] {}", idx + 1, truncated)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Apply a Liquid template to a document.
fn apply_template(template: &str, doc: &serde_json::Value) -> String {
    // Parse the Liquid template
    let parser = match liquid::ParserBuilder::with_stdlib().build() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("Failed to build Liquid parser: {}", e);
            return serde_json::to_string(doc).unwrap_or_default();
        }
    };

    let compiled = match parser.parse(template) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("Failed to parse Liquid template: {}", e);
            return serde_json::to_string(doc).unwrap_or_default();
        }
    };

    // Convert JSON Value to Liquid Object
    let liquid_obj = match doc {
        serde_json::Value::Object(map) => {
            let mut globals = liquid::Object::new();
            for (key, value) in map {
                globals.insert(key.clone().into(), json_to_liquid_value(value));
            }
            // Also provide 'doc' as the root document for templates that use {{ doc.field }}
            globals.insert("doc".into(), json_to_liquid_value(doc));

            // Provide 'fields' array for templates that iterate over fields
            let fields: Vec<liquid::model::Value> = map
                .iter()
                .map(|(name, value)| {
                    let mut field_obj = liquid::Object::new();
                    field_obj.insert("name".into(), liquid::model::Value::scalar(name.clone()));
                    field_obj.insert("value".into(), json_to_liquid_value(value));
                    field_obj.insert("is_searchable".into(), liquid::model::Value::scalar(true));
                    liquid::model::Value::Object(field_obj)
                })
                .collect();
            globals.insert("fields".into(), liquid::model::Value::Array(fields));

            globals
        }
        _ => {
            let mut globals = liquid::Object::new();
            globals.insert("doc".into(), json_to_liquid_value(doc));
            globals
        }
    };

    // Render the template
    match compiled.render(&liquid_obj) {
        Ok(rendered) => rendered,
        Err(e) => {
            tracing::warn!("Failed to render Liquid template: {}", e);
            serde_json::to_string(doc).unwrap_or_default()
        }
    }
}

/// Convert a JSON Value to a Liquid Value.
fn json_to_liquid_value(value: &serde_json::Value) -> liquid::model::Value {
    match value {
        serde_json::Value::Null => liquid::model::Value::Nil,
        serde_json::Value::Bool(b) => liquid::model::Value::scalar(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                liquid::model::Value::scalar(i)
            } else if let Some(f) = n.as_f64() {
                liquid::model::Value::scalar(f)
            } else {
                liquid::model::Value::Nil
            }
        }
        serde_json::Value::String(s) => liquid::model::Value::scalar(s.clone()),
        serde_json::Value::Array(arr) => {
            liquid::model::Value::Array(arr.iter().map(json_to_liquid_value).collect())
        }
        serde_json::Value::Object(map) => {
            let mut obj = liquid::Object::new();
            for (k, v) in map {
                obj.insert(k.clone().into(), json_to_liquid_value(v));
            }
            liquid::model::Value::Object(obj)
        }
    }
}

fn default_system_prompt() -> String {
    "You are a helpful assistant that answers questions based on the provided context. \
     When answering, cite the relevant document numbers in brackets like [1], [2], etc."
        .to_string()
}

// ============================================================================
// OpenAI Provider
// ============================================================================

async fn call_openai(
    config: &ChatConfig,
    system: &str,
    messages: &[Message],
) -> Result<ProviderResponse, Error> {
    let client = reqwest::Client::new();

    let base_url =
        config.base_url.clone().unwrap_or_else(|| "https://api.openai.com/v1".to_string());

    // Build messages array with system prompt first
    let mut openai_messages: Vec<serde_json::Value> =
        vec![serde_json::json!({"role": "system", "content": system})];

    for msg in messages {
        let role = match msg.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::System => "system",
            Role::Tool => "tool",
        };
        let mut msg_obj = serde_json::json!({
            "role": role,
            "content": msg.content
        });
        if let Some(ref tool_call_id) = msg.tool_call_id {
            msg_obj["tool_call_id"] = serde_json::Value::String(tool_call_id.clone());
        }
        openai_messages.push(msg_obj);
    }

    let request_body = serde_json::json!({
        "model": config.model,
        "messages": openai_messages
    });

    let mut request_builder = client
        .post(format!("{}/chat/completions", base_url))
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("Content-Type", "application/json");

    // Add optional headers
    if let Some(ref org_id) = config.org_id {
        request_builder = request_builder.header("OpenAI-Organization", org_id);
    }
    if let Some(ref project_id) = config.project_id {
        request_builder = request_builder.header("OpenAI-Project", project_id);
    }

    let response = request_builder
        .json(&request_body)
        .send()
        .await
        .map_err(|e| Error::chat_provider(format!("OpenAI request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(Error::chat_provider(format!("OpenAI API error ({}): {}", status, body)));
    }

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| Error::chat_provider(format!("Failed to parse OpenAI response: {}", e)))?;

    // Extract content from response
    let content = json["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string();

    // Extract usage if present
    let usage = json.get("usage").map(|usage_obj| Usage {
        prompt_tokens: usage_obj["prompt_tokens"].as_u64().unwrap_or(0) as u32,
        completion_tokens: usage_obj["completion_tokens"].as_u64().unwrap_or(0) as u32,
        total_tokens: usage_obj["total_tokens"].as_u64().unwrap_or(0) as u32,
    });

    Ok(ProviderResponse { content, usage })
}

async fn stream_openai(
    client: &reqwest::Client,
    config: &ChatConfig,
    system: &str,
    messages: &[Message],
    sources: Vec<String>,
) -> Result<Pin<Box<dyn Stream<Item = Result<ChatChunk, Error>> + Send>>, Error> {
    let base_url =
        config.base_url.clone().unwrap_or_else(|| "https://api.openai.com/v1".to_string());

    // Build messages array with system prompt
    let mut openai_messages: Vec<serde_json::Value> =
        vec![serde_json::json!({"role": "system", "content": system})];

    for msg in messages {
        openai_messages.push(serde_json::json!({
            "role": match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System => "system",
                Role::Tool => "tool",
            },
            "content": msg.content
        }));
    }

    let request_body = serde_json::json!({
        "model": config.model,
        "messages": openai_messages,
        "stream": true
    });

    let mut request_builder = client
        .post(format!("{}/chat/completions", base_url))
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("Content-Type", "application/json");

    if let Some(ref org_id) = config.org_id {
        request_builder = request_builder.header("OpenAI-Organization", org_id);
    }

    let response = request_builder
        .json(&request_body)
        .send()
        .await
        .map_err(|e| Error::chat_provider(format!("OpenAI request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(Error::chat_provider(format!("OpenAI API error ({}): {}", status, body)));
    }

    // Use bytes_stream for SSE parsing
    let byte_stream = response.bytes_stream();

    // Track state for SSE parsing and first chunk
    let sources_for_first = sources;
    let first_chunk = true;

    let stream = futures::stream::unfold(
        (byte_stream, String::new(), first_chunk, sources_for_first),
        |(mut byte_stream, mut buffer, mut is_first, sources)| async move {
            use futures::TryStreamExt;

            loop {
                // Try to extract a complete SSE event from buffer
                if let Some(event_end) = buffer.find("\n\n") {
                    let event_data = buffer[..event_end].to_string();
                    buffer = buffer[event_end + 2..].to_string();

                    // Parse SSE event
                    for line in event_data.lines() {
                        if let Some(data) = line.strip_prefix("data: ") {
                            if data == "[DONE]" {
                                return Some((
                                    Ok(ChatChunk {
                                        delta: String::new(),
                                        done: true,
                                        sources: None,
                                    }),
                                    (byte_stream, buffer, is_first, sources),
                                ));
                            }

                            match serde_json::from_str::<serde_json::Value>(data) {
                                Ok(json) => {
                                    let delta = json["choices"][0]["delta"]["content"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string();
                                    let finish_reason =
                                        json["choices"][0]["finish_reason"].as_str();

                                    let chunk_sources = if is_first {
                                        is_first = false;
                                        Some(sources.clone())
                                    } else {
                                        None
                                    };

                                    if !delta.is_empty() || finish_reason.is_some() {
                                        return Some((
                                            Ok(ChatChunk {
                                                delta,
                                                done: finish_reason.is_some(),
                                                sources: chunk_sources,
                                            }),
                                            (byte_stream, buffer, is_first, sources),
                                        ));
                                    }
                                }
                                Err(e) => {
                                    return Some((
                                        Err(Error::chat_provider(format!(
                                            "Failed to parse SSE: {}",
                                            e
                                        ))),
                                        (byte_stream, buffer, is_first, sources),
                                    ));
                                }
                            }
                        }
                    }
                    continue;
                }

                // Need more data
                match byte_stream.try_next().await {
                    Ok(Some(bytes)) => {
                        buffer.push_str(&String::from_utf8_lossy(&bytes));
                    }
                    Ok(None) => return None, // Stream ended
                    Err(e) => {
                        return Some((
                            Err(Error::chat_provider(format!("Stream error: {}", e))),
                            (byte_stream, buffer, is_first, sources),
                        ));
                    }
                }
            }
        },
    );

    Ok(Box::pin(stream))
}

// ============================================================================
// Anthropic Provider
// ============================================================================

async fn call_anthropic(
    config: &ChatConfig,
    system: &str,
    messages: &[Message],
) -> Result<ProviderResponse, Error> {
    let client = reqwest::Client::new();

    let base_url =
        config.base_url.clone().unwrap_or_else(|| "https://api.anthropic.com/v1".to_string());

    let api_version = config.api_version.clone().unwrap_or_else(|| "2023-06-01".to_string());

    // Build messages array in Anthropic format
    // Note: Anthropic handles system prompt separately
    let anthropic_messages: Vec<serde_json::Value> = messages
        .iter()
        .filter(|m| m.role != Role::System)
        .map(|msg| {
            let role = match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "user", // Tool responses go as user messages with tool_result
                Role::System => unreachable!(),
            };

            // Handle tool messages specially
            if msg.role == Role::Tool {
                if let Some(ref tool_call_id) = msg.tool_call_id {
                    return serde_json::json!({
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": tool_call_id,
                            "content": msg.content
                        }]
                    });
                }
            }

            serde_json::json!({
                "role": role,
                "content": msg.content
            })
        })
        .collect();

    // Determine max_tokens based on model
    let max_tokens = determine_anthropic_max_tokens(&config.model);

    let request_body = serde_json::json!({
        "model": config.model,
        "max_tokens": max_tokens,
        "system": system,
        "messages": anthropic_messages
    });

    let response = client
        .post(format!("{}/messages", base_url))
        .header("x-api-key", &config.api_key)
        .header("anthropic-version", &api_version)
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
        .map_err(|e| Error::chat_provider(format!("Anthropic request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();

        // Check for common error types and provide helpful messages
        if status.as_u16() == 401 {
            return Err(Error::chat_provider(
                "Invalid or missing Anthropic API key. Please check your chat configuration."
                    .to_string(),
            ));
        }

        return Err(Error::chat_provider(format!("Anthropic API error ({}): {}", status, body)));
    }

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| Error::chat_provider(format!("Failed to parse Anthropic response: {}", e)))?;

    // Extract content from response (Anthropic uses content array)
    let content = json["content"]
        .as_array()
        .and_then(|arr| {
            arr.iter()
                .filter_map(|block| {
                    if block["type"].as_str() == Some("text") {
                        block["text"].as_str().map(String::from)
                    } else {
                        None
                    }
                })
                .next()
        })
        .unwrap_or_default();

    // Extract usage
    let usage = json.get("usage").map(|usage_obj| Usage {
        prompt_tokens: usage_obj["input_tokens"].as_u64().unwrap_or(0) as u32,
        completion_tokens: usage_obj["output_tokens"].as_u64().unwrap_or(0) as u32,
        total_tokens: (usage_obj["input_tokens"].as_u64().unwrap_or(0)
            + usage_obj["output_tokens"].as_u64().unwrap_or(0)) as u32,
    });

    Ok(ProviderResponse { content, usage })
}

async fn stream_anthropic(
    client: &reqwest::Client,
    config: &ChatConfig,
    system: &str,
    messages: &[Message],
    sources: Vec<String>,
) -> Result<Pin<Box<dyn Stream<Item = Result<ChatChunk, Error>> + Send>>, Error> {
    let base_url =
        config.base_url.clone().unwrap_or_else(|| "https://api.anthropic.com/v1".to_string());

    let api_version = config.api_version.clone().unwrap_or_else(|| "2023-06-01".to_string());

    // Build messages array (Anthropic format)
    let anthropic_messages: Vec<serde_json::Value> = messages
        .iter()
        .filter(|m| m.role != Role::System)
        .map(|msg| {
            serde_json::json!({
                "role": match msg.role {
                    Role::User | Role::Tool => "user",
                    Role::Assistant => "assistant",
                    Role::System => unreachable!(),
                },
                "content": msg.content
            })
        })
        .collect();

    let max_tokens = determine_anthropic_max_tokens(&config.model);

    let request_body = serde_json::json!({
        "model": config.model,
        "max_tokens": max_tokens,
        "system": system,
        "messages": anthropic_messages,
        "stream": true
    });

    let response = client
        .post(format!("{}/messages", base_url))
        .header("x-api-key", &config.api_key)
        .header("anthropic-version", &api_version)
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
        .map_err(|e| Error::chat_provider(format!("Anthropic request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(Error::chat_provider(format!("Anthropic API error ({}): {}", status, body)));
    }

    let byte_stream = response.bytes_stream();
    let sources_for_first = sources;
    let first_chunk = true;

    let stream = futures::stream::unfold(
        (byte_stream, String::new(), first_chunk, sources_for_first),
        |(mut byte_stream, mut buffer, mut is_first, sources)| async move {
            use futures::TryStreamExt;

            loop {
                // Try to extract a complete SSE event from buffer
                if let Some(event_end) = buffer.find("\n\n") {
                    let event_data = buffer[..event_end].to_string();
                    buffer = buffer[event_end + 2..].to_string();

                    // Parse SSE event
                    for line in event_data.lines() {
                        if let Some(data) = line.strip_prefix("data: ") {
                            match serde_json::from_str::<serde_json::Value>(data) {
                                Ok(json) => {
                                    let event_type = json["type"].as_str().unwrap_or("");

                                    match event_type {
                                        "content_block_delta" => {
                                            let delta = json["delta"]["text"]
                                                .as_str()
                                                .unwrap_or("")
                                                .to_string();

                                            let chunk_sources = if is_first {
                                                is_first = false;
                                                Some(sources.clone())
                                            } else {
                                                None
                                            };

                                            if !delta.is_empty() {
                                                return Some((
                                                    Ok(ChatChunk {
                                                        delta,
                                                        done: false,
                                                        sources: chunk_sources,
                                                    }),
                                                    (byte_stream, buffer, is_first, sources),
                                                ));
                                            }
                                        }
                                        "message_stop" => {
                                            return Some((
                                                Ok(ChatChunk {
                                                    delta: String::new(),
                                                    done: true,
                                                    sources: None,
                                                }),
                                                (byte_stream, buffer, is_first, sources),
                                            ));
                                        }
                                        "message_delta" => {
                                            let stop_reason = json["delta"]["stop_reason"].as_str();
                                            if stop_reason.is_some() {
                                                return Some((
                                                    Ok(ChatChunk {
                                                        delta: String::new(),
                                                        done: true,
                                                        sources: None,
                                                    }),
                                                    (byte_stream, buffer, is_first, sources),
                                                ));
                                            }
                                        }
                                        "error" => {
                                            let msg = json["error"]["message"]
                                                .as_str()
                                                .unwrap_or("Unknown error")
                                                .to_string();
                                            return Some((
                                                Err(Error::chat_provider(msg)),
                                                (byte_stream, buffer, is_first, sources),
                                            ));
                                        }
                                        _ => {} // Ignore other events
                                    }
                                }
                                Err(e) => {
                                    return Some((
                                        Err(Error::chat_provider(format!(
                                            "Failed to parse SSE: {}",
                                            e
                                        ))),
                                        (byte_stream, buffer, is_first, sources),
                                    ));
                                }
                            }
                        }
                    }
                    continue;
                }

                // Need more data
                match byte_stream.try_next().await {
                    Ok(Some(bytes)) => {
                        buffer.push_str(&String::from_utf8_lossy(&bytes));
                    }
                    Ok(None) => return None,
                    Err(e) => {
                        return Some((
                            Err(Error::chat_provider(format!("Stream error: {}", e))),
                            (byte_stream, buffer, is_first, sources),
                        ));
                    }
                }
            }
        },
    );

    Ok(Box::pin(stream))
}

/// Determine max tokens for Anthropic models.
fn determine_anthropic_max_tokens(model: &str) -> u32 {
    if model.contains("claude-opus-4-5")
        || model.contains("claude-sonnet-4-5")
        || model.contains("claude-haiku-4-5")
    {
        32000
    } else if model.contains("claude-opus-4") {
        16000
    } else if model.contains("claude-sonnet-4") || model.contains("claude-3-7-sonnet") {
        32000
    } else if model.contains("claude-3-5") {
        8192
    } else {
        4096
    }
}

// ============================================================================
// Azure OpenAI Provider
// ============================================================================

async fn call_azure_openai(
    config: &ChatConfig,
    system: &str,
    messages: &[Message],
) -> Result<ProviderResponse, Error> {
    let client = reqwest::Client::new();

    let base_url = config
        .base_url
        .as_ref()
        .ok_or_else(|| Error::chat_provider("Azure OpenAI requires base_url"))?;

    let deployment_id = config
        .deployment_id
        .as_ref()
        .ok_or_else(|| Error::chat_provider("Azure OpenAI requires deployment_id"))?;

    let api_version = config
        .api_version
        .as_ref()
        .ok_or_else(|| Error::chat_provider("Azure OpenAI requires api_version"))?;

    // Build messages array with system prompt first
    let mut azure_messages: Vec<serde_json::Value> =
        vec![serde_json::json!({"role": "system", "content": system})];

    for msg in messages {
        azure_messages.push(serde_json::json!({
            "role": match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System => "system",
                Role::Tool => "tool",
            },
            "content": msg.content
        }));
    }

    let request_body = serde_json::json!({
        "messages": azure_messages
    });

    let url = format!(
        "{}/openai/deployments/{}/chat/completions?api-version={}",
        base_url, deployment_id, api_version
    );

    let response = client
        .post(&url)
        .header("api-key", &config.api_key)
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
        .map_err(|e| Error::chat_provider(format!("Azure OpenAI request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(Error::chat_provider(format!("Azure OpenAI API error ({}): {}", status, body)));
    }

    let json: serde_json::Value = response.json().await.map_err(|e| {
        Error::chat_provider(format!("Failed to parse Azure OpenAI response: {}", e))
    })?;

    let content = json["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string();

    let usage = json.get("usage").map(|usage_obj| Usage {
        prompt_tokens: usage_obj["prompt_tokens"].as_u64().unwrap_or(0) as u32,
        completion_tokens: usage_obj["completion_tokens"].as_u64().unwrap_or(0) as u32,
        total_tokens: usage_obj["total_tokens"].as_u64().unwrap_or(0) as u32,
    });

    Ok(ProviderResponse { content, usage })
}

async fn stream_azure_openai(
    client: &reqwest::Client,
    config: &ChatConfig,
    system: &str,
    messages: &[Message],
    sources: Vec<String>,
) -> Result<Pin<Box<dyn Stream<Item = Result<ChatChunk, Error>> + Send>>, Error> {
    let base_url = config
        .base_url
        .as_ref()
        .ok_or_else(|| Error::chat_provider("Azure OpenAI requires base_url"))?;

    let deployment_id = config
        .deployment_id
        .as_ref()
        .ok_or_else(|| Error::chat_provider("Azure OpenAI requires deployment_id"))?;

    let api_version = config
        .api_version
        .as_ref()
        .ok_or_else(|| Error::chat_provider("Azure OpenAI requires api_version"))?;

    let mut azure_messages: Vec<serde_json::Value> =
        vec![serde_json::json!({"role": "system", "content": system})];

    for msg in messages {
        azure_messages.push(serde_json::json!({
            "role": match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System => "system",
                Role::Tool => "tool",
            },
            "content": msg.content
        }));
    }

    let request_body = serde_json::json!({
        "messages": azure_messages,
        "stream": true
    });

    let url = format!(
        "{}/openai/deployments/{}/chat/completions?api-version={}",
        base_url, deployment_id, api_version
    );

    let response = client
        .post(&url)
        .header("api-key", &config.api_key)
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
        .map_err(|e| Error::chat_provider(format!("Azure OpenAI request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(Error::chat_provider(format!("Azure OpenAI API error ({}): {}", status, body)));
    }

    // Reuse OpenAI SSE parsing logic since Azure uses the same format
    let byte_stream = response.bytes_stream();
    let first_chunk = true;

    let stream = futures::stream::unfold(
        (byte_stream, String::new(), first_chunk, sources),
        |(mut byte_stream, mut buffer, mut is_first, sources)| async move {
            use futures::TryStreamExt;

            loop {
                if let Some(event_end) = buffer.find("\n\n") {
                    let event_data = buffer[..event_end].to_string();
                    buffer = buffer[event_end + 2..].to_string();

                    for line in event_data.lines() {
                        if let Some(data) = line.strip_prefix("data: ") {
                            if data == "[DONE]" {
                                return Some((
                                    Ok(ChatChunk {
                                        delta: String::new(),
                                        done: true,
                                        sources: None,
                                    }),
                                    (byte_stream, buffer, is_first, sources),
                                ));
                            }

                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                                let delta = json["choices"][0]["delta"]["content"]
                                    .as_str()
                                    .unwrap_or("")
                                    .to_string();
                                let finish_reason = json["choices"][0]["finish_reason"].as_str();

                                let chunk_sources = if is_first {
                                    is_first = false;
                                    Some(sources.clone())
                                } else {
                                    None
                                };

                                if !delta.is_empty() || finish_reason.is_some() {
                                    return Some((
                                        Ok(ChatChunk {
                                            delta,
                                            done: finish_reason.is_some(),
                                            sources: chunk_sources,
                                        }),
                                        (byte_stream, buffer, is_first, sources),
                                    ));
                                }
                            }
                        }
                    }
                    continue;
                }

                match byte_stream.try_next().await {
                    Ok(Some(bytes)) => buffer.push_str(&String::from_utf8_lossy(&bytes)),
                    Ok(None) => return None,
                    Err(e) => {
                        return Some((
                            Err(Error::chat_provider(format!("Stream error: {}", e))),
                            (byte_stream, buffer, is_first, sources),
                        ))
                    }
                }
            }
        },
    );

    Ok(Box::pin(stream))
}

// ============================================================================
// Mistral Provider
// ============================================================================

async fn call_mistral(
    config: &ChatConfig,
    system: &str,
    messages: &[Message],
) -> Result<ProviderResponse, Error> {
    let client = reqwest::Client::new();

    let base_url =
        config.base_url.clone().unwrap_or_else(|| "https://api.mistral.ai/v1".to_string());

    let mut mistral_messages: Vec<serde_json::Value> =
        vec![serde_json::json!({"role": "system", "content": system})];

    for msg in messages {
        mistral_messages.push(serde_json::json!({
            "role": match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System => "system",
                Role::Tool => "tool",
            },
            "content": msg.content
        }));
    }

    let request_body = serde_json::json!({
        "model": config.model,
        "messages": mistral_messages
    });

    let response = client
        .post(format!("{}/chat/completions", base_url))
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
        .map_err(|e| Error::chat_provider(format!("Mistral request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(Error::chat_provider(format!("Mistral API error ({}): {}", status, body)));
    }

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| Error::chat_provider(format!("Failed to parse Mistral response: {}", e)))?;

    let content = json["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string();

    let usage = json.get("usage").map(|usage_obj| Usage {
        prompt_tokens: usage_obj["prompt_tokens"].as_u64().unwrap_or(0) as u32,
        completion_tokens: usage_obj["completion_tokens"].as_u64().unwrap_or(0) as u32,
        total_tokens: usage_obj["total_tokens"].as_u64().unwrap_or(0) as u32,
    });

    Ok(ProviderResponse { content, usage })
}

async fn stream_mistral(
    client: &reqwest::Client,
    config: &ChatConfig,
    system: &str,
    messages: &[Message],
    sources: Vec<String>,
) -> Result<Pin<Box<dyn Stream<Item = Result<ChatChunk, Error>> + Send>>, Error> {
    let base_url =
        config.base_url.clone().unwrap_or_else(|| "https://api.mistral.ai/v1".to_string());

    let mut mistral_messages: Vec<serde_json::Value> =
        vec![serde_json::json!({"role": "system", "content": system})];

    for msg in messages {
        mistral_messages.push(serde_json::json!({
            "role": match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System => "system",
                Role::Tool => "tool",
            },
            "content": msg.content
        }));
    }

    let request_body = serde_json::json!({
        "model": config.model,
        "messages": mistral_messages,
        "stream": true
    });

    let response = client
        .post(format!("{}/chat/completions", base_url))
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
        .map_err(|e| Error::chat_provider(format!("Mistral request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(Error::chat_provider(format!("Mistral API error ({}): {}", status, body)));
    }

    // Mistral uses OpenAI-compatible SSE format
    let byte_stream = response.bytes_stream();
    let first_chunk = true;

    let stream = futures::stream::unfold(
        (byte_stream, String::new(), first_chunk, sources),
        |(mut byte_stream, mut buffer, mut is_first, sources)| async move {
            use futures::TryStreamExt;

            loop {
                if let Some(event_end) = buffer.find("\n\n") {
                    let event_data = buffer[..event_end].to_string();
                    buffer = buffer[event_end + 2..].to_string();

                    for line in event_data.lines() {
                        if let Some(data) = line.strip_prefix("data: ") {
                            if data == "[DONE]" {
                                return Some((
                                    Ok(ChatChunk {
                                        delta: String::new(),
                                        done: true,
                                        sources: None,
                                    }),
                                    (byte_stream, buffer, is_first, sources),
                                ));
                            }

                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                                let delta = json["choices"][0]["delta"]["content"]
                                    .as_str()
                                    .unwrap_or("")
                                    .to_string();
                                let finish_reason = json["choices"][0]["finish_reason"].as_str();

                                let chunk_sources = if is_first {
                                    is_first = false;
                                    Some(sources.clone())
                                } else {
                                    None
                                };

                                if !delta.is_empty() || finish_reason.is_some() {
                                    return Some((
                                        Ok(ChatChunk {
                                            delta,
                                            done: finish_reason.is_some(),
                                            sources: chunk_sources,
                                        }),
                                        (byte_stream, buffer, is_first, sources),
                                    ));
                                }
                            }
                        }
                    }
                    continue;
                }

                match byte_stream.try_next().await {
                    Ok(Some(bytes)) => buffer.push_str(&String::from_utf8_lossy(&bytes)),
                    Ok(None) => return None,
                    Err(e) => {
                        return Some((
                            Err(Error::chat_provider(format!("Stream error: {}", e))),
                            (byte_stream, buffer, is_first, sources),
                        ))
                    }
                }
            }
        },
    );

    Ok(Box::pin(stream))
}

// ============================================================================
// vLLM Provider
// ============================================================================

async fn call_vllm(
    config: &ChatConfig,
    system: &str,
    messages: &[Message],
) -> Result<ProviderResponse, Error> {
    let client = reqwest::Client::new();

    let base_url =
        config.base_url.as_ref().ok_or_else(|| Error::chat_provider("vLLM requires base_url"))?;

    let mut vllm_messages: Vec<serde_json::Value> =
        vec![serde_json::json!({"role": "system", "content": system})];

    for msg in messages {
        vllm_messages.push(serde_json::json!({
            "role": match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System => "system",
                Role::Tool => "tool",
            },
            "content": msg.content
        }));
    }

    let request_body = serde_json::json!({
        "model": config.model,
        "messages": vllm_messages
    });

    let response = client
        .post(format!("{}/chat/completions", base_url))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
        .map_err(|e| Error::chat_provider(format!("vLLM request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(Error::chat_provider(format!("vLLM API error ({}): {}", status, body)));
    }

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| Error::chat_provider(format!("Failed to parse vLLM response: {}", e)))?;

    let content = json["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string();

    let usage = json.get("usage").map(|usage_obj| Usage {
        prompt_tokens: usage_obj["prompt_tokens"].as_u64().unwrap_or(0) as u32,
        completion_tokens: usage_obj["completion_tokens"].as_u64().unwrap_or(0) as u32,
        total_tokens: usage_obj["total_tokens"].as_u64().unwrap_or(0) as u32,
    });

    Ok(ProviderResponse { content, usage })
}

async fn stream_vllm(
    client: &reqwest::Client,
    config: &ChatConfig,
    system: &str,
    messages: &[Message],
    sources: Vec<String>,
) -> Result<Pin<Box<dyn Stream<Item = Result<ChatChunk, Error>> + Send>>, Error> {
    let base_url =
        config.base_url.as_ref().ok_or_else(|| Error::chat_provider("vLLM requires base_url"))?;

    let mut vllm_messages: Vec<serde_json::Value> =
        vec![serde_json::json!({"role": "system", "content": system})];

    for msg in messages {
        vllm_messages.push(serde_json::json!({
            "role": match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System => "system",
                Role::Tool => "tool",
            },
            "content": msg.content
        }));
    }

    let request_body = serde_json::json!({
        "model": config.model,
        "messages": vllm_messages,
        "stream": true
    });

    let response = client
        .post(format!("{}/chat/completions", base_url))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
        .map_err(|e| Error::chat_provider(format!("vLLM request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(Error::chat_provider(format!("vLLM API error ({}): {}", status, body)));
    }

    // vLLM uses OpenAI-compatible SSE format
    let byte_stream = response.bytes_stream();
    let first_chunk = true;

    let stream = futures::stream::unfold(
        (byte_stream, String::new(), first_chunk, sources),
        |(mut byte_stream, mut buffer, mut is_first, sources)| async move {
            use futures::TryStreamExt;

            loop {
                if let Some(event_end) = buffer.find("\n\n") {
                    let event_data = buffer[..event_end].to_string();
                    buffer = buffer[event_end + 2..].to_string();

                    for line in event_data.lines() {
                        if let Some(data) = line.strip_prefix("data: ") {
                            if data == "[DONE]" {
                                return Some((
                                    Ok(ChatChunk {
                                        delta: String::new(),
                                        done: true,
                                        sources: None,
                                    }),
                                    (byte_stream, buffer, is_first, sources),
                                ));
                            }

                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                                let delta = json["choices"][0]["delta"]["content"]
                                    .as_str()
                                    .unwrap_or("")
                                    .to_string();
                                let finish_reason = json["choices"][0]["finish_reason"].as_str();

                                let chunk_sources = if is_first {
                                    is_first = false;
                                    Some(sources.clone())
                                } else {
                                    None
                                };

                                if !delta.is_empty() || finish_reason.is_some() {
                                    return Some((
                                        Ok(ChatChunk {
                                            delta,
                                            done: finish_reason.is_some(),
                                            sources: chunk_sources,
                                        }),
                                        (byte_stream, buffer, is_first, sources),
                                    ));
                                }
                            }
                        }
                    }
                    continue;
                }

                match byte_stream.try_next().await {
                    Ok(Some(bytes)) => buffer.push_str(&String::from_utf8_lossy(&bytes)),
                    Ok(None) => return None,
                    Err(e) => {
                        return Some((
                            Err(Error::chat_provider(format!("Stream error: {}", e))),
                            (byte_stream, buffer, is_first, sources),
                        ))
                    }
                }
            }
        },
    );

    Ok(Box::pin(stream))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_user() {
        let msg = Message::user("Hello");
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content, "Hello");
        assert!(msg.tool_call_id.is_none());
    }

    #[test]
    fn test_message_assistant() {
        let msg = Message::assistant("Hi there");
        assert_eq!(msg.role, Role::Assistant);
        assert_eq!(msg.content, "Hi there");
    }

    #[test]
    fn test_message_system() {
        let msg = Message::system("You are helpful");
        assert_eq!(msg.role, Role::System);
        assert_eq!(msg.content, "You are helpful");
    }

    #[test]
    fn test_message_tool() {
        let msg = Message::tool("Result", "call_123");
        assert_eq!(msg.role, Role::Tool);
        assert_eq!(msg.content, "Result");
        assert_eq!(msg.tool_call_id, Some("call_123".to_string()));
    }

    #[test]
    fn test_chat_request_serialization() {
        let request = ChatRequest {
            messages: vec![Message::user("test")],
            index_uid: "products".to_string(),
            stream: false,
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"index_uid\":\"products\""));

        let parsed: ChatRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.index_uid, "products");
        assert!(!parsed.stream);
    }

    #[test]
    fn test_chat_response_serialization() {
        let response = ChatResponse {
            content: "Hello!".to_string(),
            sources: vec!["doc1".to_string(), "doc2".to_string()],
            usage: Some(Usage { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 }),
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"content\":\"Hello!\""));
        assert!(json.contains("\"prompt_tokens\":10"));

        let parsed: ChatResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.content, "Hello!");
        assert_eq!(parsed.sources.len(), 2);
    }

    #[test]
    fn test_chat_chunk_serialization() {
        let chunk = ChatChunk {
            delta: "Hello".to_string(),
            done: false,
            sources: Some(vec!["doc1".to_string()]),
        };

        let json = serde_json::to_string(&chunk).unwrap();
        assert!(json.contains("\"delta\":\"Hello\""));
        assert!(json.contains("\"done\":false"));
        assert!(json.contains("\"sources\""));

        // Test without sources
        let chunk_no_sources = ChatChunk { delta: "World".to_string(), done: true, sources: None };

        let json2 = serde_json::to_string(&chunk_no_sources).unwrap();
        assert!(!json2.contains("sources")); // Should be skipped
    }

    #[test]
    fn test_format_context_default() {
        let hits = vec![
            serde_json::json!({"id": "1", "title": "Product A"}),
            serde_json::json!({"id": "2", "title": "Product B"}),
        ];

        let context = format_context(&hits, None);

        assert!(context.contains("[1]"));
        assert!(context.contains("[2]"));
        assert!(context.contains("Product A"));
        assert!(context.contains("Product B"));
    }

    #[test]
    fn test_format_context_with_template() {
        let hits = vec![serde_json::json!({"id": "1", "name": "Widget", "price": 99})];

        let config = ChatIndexConfig {
            description: "Products".to_string(),
            template: Some("{{ name }} costs ${{ price }}".to_string()),
            max_bytes: None,
            search_params: None,
        };

        let context = format_context(&hits, Some(&config));

        assert!(context.contains("Widget costs $99"));
    }

    #[test]
    fn test_format_context_truncation() {
        let hits = vec![serde_json::json!({
            "id": "1",
            "description": "This is a very long description that should be truncated to fit within the max bytes limit"
        })];

        let config = ChatIndexConfig {
            description: "Test".to_string(),
            template: None,
            max_bytes: Some(50),
            search_params: None,
        };

        let context = format_context(&hits, Some(&config));

        // Should be truncated and end with "..."
        assert!(context.contains("..."));
        assert!(context.len() < 100);
    }

    #[test]
    fn test_json_to_liquid_value() {
        // Test null
        assert!(matches!(
            json_to_liquid_value(&serde_json::Value::Null),
            liquid::model::Value::Nil
        ));

        // Test bool
        let bool_val = json_to_liquid_value(&serde_json::json!(true));
        assert!(matches!(bool_val, liquid::model::Value::Scalar(_)));

        // Test number
        let num_val = json_to_liquid_value(&serde_json::json!(42));
        assert!(matches!(num_val, liquid::model::Value::Scalar(_)));

        // Test string
        let str_val = json_to_liquid_value(&serde_json::json!("hello"));
        assert!(matches!(str_val, liquid::model::Value::Scalar(_)));

        // Test array
        let arr_val = json_to_liquid_value(&serde_json::json!([1, 2, 3]));
        assert!(matches!(arr_val, liquid::model::Value::Array(_)));

        // Test object
        let obj_val = json_to_liquid_value(&serde_json::json!({"a": 1}));
        assert!(matches!(obj_val, liquid::model::Value::Object(_)));
    }

    #[test]
    fn test_default_system_prompt() {
        let prompt = default_system_prompt();
        assert!(prompt.contains("helpful assistant"));
        assert!(prompt.contains("context"));
    }

    #[test]
    fn test_determine_anthropic_max_tokens() {
        assert_eq!(determine_anthropic_max_tokens("claude-opus-4-5-20250101"), 32000);
        assert_eq!(determine_anthropic_max_tokens("claude-sonnet-4-20250101"), 32000);
        assert_eq!(determine_anthropic_max_tokens("claude-3-5-sonnet-20240620"), 8192);
        assert_eq!(determine_anthropic_max_tokens("claude-3-haiku"), 4096);
    }

    #[test]
    fn test_role_serialization() {
        let user_json = serde_json::to_string(&Role::User).unwrap();
        assert_eq!(user_json, "\"user\"");

        let assistant_json = serde_json::to_string(&Role::Assistant).unwrap();
        assert_eq!(assistant_json, "\"assistant\"");

        let system_json = serde_json::to_string(&Role::System).unwrap();
        assert_eq!(system_json, "\"system\"");

        let tool_json = serde_json::to_string(&Role::Tool).unwrap();
        assert_eq!(tool_json, "\"tool\"");
    }

    #[test]
    fn test_role_deserialization() {
        let user: Role = serde_json::from_str("\"user\"").unwrap();
        assert_eq!(user, Role::User);

        let assistant: Role = serde_json::from_str("\"assistant\"").unwrap();
        assert_eq!(assistant, Role::Assistant);
    }
}
