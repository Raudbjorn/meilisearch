//! Mock chat completions endpoint - OpenAI-compatible `/v1/chat/completions`.
//!
//! Supports:
//! - Non-streaming responses
//! - SSE streaming responses
//! - Tool/function calling
//! - Rule-based response generation

use axum::{
    extract::State,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use futures::stream::{self, Stream};
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, pin::Pin, sync::atomic::Ordering, time::Duration};
use tracing::{debug, info, instrument, warn};
use uuid::Uuid;

use crate::error::MockServerError;
use crate::server::AppState;

// =============================================================================
// Request Types
// =============================================================================

/// OpenAI-compatible chat completion request.
#[derive(Debug, Deserialize)]
pub struct ChatCompletionRequest {
    /// Model identifier.
    pub model: String,
    /// Conversation messages.
    pub messages: Vec<ChatMessage>,
    /// Whether to stream the response.
    #[serde(default)]
    pub stream: bool,
    /// Available tools/functions.
    #[serde(default)]
    pub tools: Vec<Tool>,
    /// Tool choice configuration.
    #[serde(default)]
    pub tool_choice: Option<ToolChoice>,
    /// Maximum tokens to generate.
    #[serde(default)]
    pub max_tokens: Option<usize>,
    /// Sampling temperature.
    #[serde(default)]
    pub temperature: Option<f32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default)]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Tool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: FunctionDefinition,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FunctionDefinition {
    pub name: String,
    pub description: Option<String>,
    pub parameters: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ToolChoice {
    Auto(String),
    Specific {
        #[serde(rename = "type")]
        choice_type: String,
        function: ToolChoiceFunction,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ToolChoiceFunction {
    pub name: String,
}

// =============================================================================
// Response Types
// =============================================================================

/// OpenAI-compatible chat completion response.
#[derive(Debug, Serialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub object: &'static str,
    pub created: u64,
    pub model: String,
    pub choices: Vec<Choice>,
    pub usage: Usage,
}

#[derive(Debug, Clone, Serialize)]
pub struct Choice {
    pub index: usize,
    pub message: ResponseMessage,
    pub finish_reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResponseMessage {
    pub role: String,
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Serialize)]
pub struct Usage {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub total_tokens: usize,
}

// =============================================================================
// Streaming Response Types
// =============================================================================

#[derive(Debug, Serialize)]
pub struct ChatCompletionChunk {
    pub id: String,
    pub object: &'static str,
    pub created: u64,
    pub model: String,
    pub choices: Vec<StreamChoice>,
}

#[derive(Debug, Serialize)]
pub struct StreamChoice {
    pub index: usize,
    pub delta: Delta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Delta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<StreamToolCall>>,
}

#[derive(Debug, Serialize)]
pub struct StreamToolCall {
    pub index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "type")]
    pub call_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<StreamFunctionCall>,
}

#[derive(Debug, Serialize)]
pub struct StreamFunctionCall {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

// =============================================================================
// Handler
// =============================================================================

/// Handle chat completion requests.
///
/// # Endpoint
/// `POST /v1/chat/completions`
///
/// Supports both streaming (`stream: true`) and non-streaming responses.
#[instrument(skip(state, request), fields(model = %request.model, stream = request.stream))]
pub async fn handle_chat_completions(
    State(state): State<AppState>,
    Json(request): Json<ChatCompletionRequest>,
) -> Result<Response, MockServerError> {
    // Validate request
    validate_request(&request)?;

    info!(
        model = %request.model,
        message_count = request.messages.len(),
        has_tools = !request.tools.is_empty(),
        stream = request.stream,
        "Processing chat completion request"
    );

    // Decide on response based on rules
    let mock_response = generate_mock_response(&request, &state);

    if request.stream {
        debug!("Returning streaming response");
        Ok(create_streaming_response(mock_response, request.model).into_response())
    } else {
        debug!("Returning non-streaming response");
        state
            .metrics
            .chat_completions
            .fetch_add(1, Ordering::Relaxed);
        Ok(Json(create_non_streaming_response(mock_response, request.model)).into_response())
    }
}

/// Validate the chat completion request.
fn validate_request(request: &ChatCompletionRequest) -> Result<(), MockServerError> {
    if request.messages.is_empty() {
        return Err(MockServerError::InvalidRequest {
            field: "messages".to_string(),
            reason: "Messages array cannot be empty".to_string(),
        });
    }

    // Validate message roles
    for (i, msg) in request.messages.iter().enumerate() {
        let valid_roles = ["system", "user", "assistant", "tool"];
        if !valid_roles.contains(&msg.role.as_str()) {
            return Err(MockServerError::InvalidRequest {
                field: format!("messages[{}].role", i),
                reason: format!(
                    "Invalid role '{}', must be one of: {:?}",
                    msg.role, valid_roles
                ),
            });
        }

        // Tool messages must have tool_call_id
        if msg.role == "tool" && msg.tool_call_id.is_none() {
            return Err(MockServerError::InvalidRequest {
                field: format!("messages[{}].tool_call_id", i),
                reason: "Tool messages must include tool_call_id".to_string(),
            });
        }
    }

    // Validate tools
    for (i, tool) in request.tools.iter().enumerate() {
        if tool.tool_type != "function" {
            warn!(
                tool_index = i,
                tool_type = %tool.tool_type,
                "Unsupported tool type"
            );
        }
        if tool.function.name.is_empty() {
            return Err(MockServerError::InvalidRequest {
                field: format!("tools[{}].function.name", i),
                reason: "Function name cannot be empty".to_string(),
            });
        }
    }

    Ok(())
}

// =============================================================================
// Mock Response Generation
// =============================================================================

/// Represents the mock response to generate.
#[derive(Debug)]
struct MockResponse {
    content: Option<String>,
    tool_calls: Option<Vec<ToolCall>>,
    finish_reason: String,
}

/// Generate a mock response based on the request content.
///
/// Rules:
/// 1. If tools include a search function and user message contains search-related terms,
///    return a tool call.
/// 2. If this is a tool result message, synthesize a response.
/// 3. Otherwise, return a generic helpful response.
fn generate_mock_response(request: &ChatCompletionRequest, _state: &AppState) -> MockResponse {
    let last_user_message = request
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .and_then(|m| m.content.as_ref())
        .map(|s| s.to_lowercase())
        .unwrap_or_default();

    let last_message = request.messages.last();

    // Check if this is following up on a tool call result
    let is_tool_result = last_message.map(|m| m.role == "tool").unwrap_or(false);

    // Check if we have search-related tools
    let has_search_tool = request.tools.iter().any(|t| {
        let name = t.function.name.to_lowercase();
        name.contains("search") || name.contains("meilisearch") || name.contains("query")
    });

    // Check if user message suggests they want to search
    let wants_search = last_user_message.contains("search")
        || last_user_message.contains("find")
        || last_user_message.contains("look for")
        || last_user_message.contains("query")
        || last_user_message.contains("looking for");

    // Rule 1: Tool call if appropriate
    if has_search_tool && wants_search && !is_tool_result {
        if let Some(search_tool) = request.tools.iter().find(|t| {
            let name = t.function.name.to_lowercase();
            name.contains("search") || name.contains("meilisearch")
        }) {
            debug!(
                tool_name = %search_tool.function.name,
                "Generating search tool call"
            );

            // Extract search query from user message
            let search_query = extract_search_query(&last_user_message);

            let tool_call_id = format!(
                "call_{}",
                Uuid::new_v4().to_string().replace('-', "")[..24].to_string()
            );

            return MockResponse {
                content: None,
                tool_calls: Some(vec![ToolCall {
                    id: tool_call_id,
                    call_type: "function".to_string(),
                    function: FunctionCall {
                        name: search_tool.function.name.clone(),
                        arguments: serde_json::json!({
                            "q": search_query,
                            "limit": 10
                        })
                        .to_string(),
                    },
                }]),
                finish_reason: "tool_calls".to_string(),
            };
        }
    }

    // Rule 2: Synthesize response after tool result
    if is_tool_result {
        let tool_content = last_message
            .and_then(|m| m.content.as_ref())
            .unwrap_or(&String::new())
            .clone();

        let response = if tool_content.contains("error") || tool_content.contains("Error") {
            "I encountered an issue while searching. Could you please try rephrasing your query?"
                .to_string()
        } else if tool_content.is_empty() || tool_content == "[]" {
            "I couldn't find any results matching your query. Would you like to try different search terms?".to_string()
        } else {
            format!(
                "Based on the search results, here's what I found:\n\n{}",
                summarize_tool_result(&tool_content)
            )
        };

        return MockResponse {
            content: Some(response),
            tool_calls: None,
            finish_reason: "stop".to_string(),
        };
    }

    // Rule 3: Generic response
    let response = generate_generic_response(&last_user_message);

    MockResponse {
        content: Some(response),
        tool_calls: None,
        finish_reason: "stop".to_string(),
    }
}

/// Extract a search query from user message.
fn extract_search_query(message: &str) -> String {
    // Simple extraction - remove common prefixes
    let prefixes = [
        "search for ",
        "find ",
        "look for ",
        "query ",
        "search ",
        "looking for ",
        "can you find ",
        "please search for ",
        "i want to find ",
        "i'm looking for ",
    ];

    let mut query = message.to_string();
    for prefix in prefixes {
        if let Some(rest) = query.strip_prefix(prefix) {
            query = rest.to_string();
            break;
        }
    }

    // Clean up the query
    query = query.trim().to_string();

    // Remove trailing punctuation
    query = query
        .trim_end_matches(|c| c == '?' || c == '!' || c == '.')
        .to_string();

    if query.is_empty() {
        "movies".to_string() // Default fallback
    } else {
        query
    }
}

/// Summarize tool results for the response.
fn summarize_tool_result(content: &str) -> String {
    // Try to parse as JSON and extract useful info
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(content) {
        if let Some(hits) = value.get("hits").and_then(|h| h.as_array()) {
            let count = hits.len();
            return format!(
                "Found {} result(s). The search completed successfully.",
                count
            );
        }
        if let Some(arr) = value.as_array() {
            return format!("Found {} result(s).", arr.len());
        }
    }

    // Truncate if too long
    if content.len() > 500 {
        format!("{}...", &content[..500])
    } else {
        content.to_string()
    }
}

/// Generate a generic helpful response.
fn generate_generic_response(message: &str) -> String {
    if message.contains("hello") || message.contains("hi ") || message.starts_with("hi") {
        "Hello! I'm a mock assistant for testing. How can I help you today?".to_string()
    } else if message.contains("help") {
        "I'm here to help! I can assist with searching and retrieving information. What would you like to find?".to_string()
    } else if message.contains("thank") {
        "You're welcome! Let me know if you need anything else.".to_string()
    } else {
        "I understand your request. In a production environment, I would process this with a real LLM. For testing purposes, this is a mock response.".to_string()
    }
}

// =============================================================================
// Response Creation
// =============================================================================

/// Create a non-streaming chat completion response.
fn create_non_streaming_response(mock: MockResponse, model: String) -> ChatCompletionResponse {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let completion_tokens = mock.content.as_ref().map(|c| c.len() / 4).unwrap_or(10);

    ChatCompletionResponse {
        id: format!("chatcmpl-{}", Uuid::new_v4()),
        object: "chat.completion",
        created: now,
        model,
        choices: vec![Choice {
            index: 0,
            message: ResponseMessage {
                role: "assistant".to_string(),
                content: mock.content,
                tool_calls: mock.tool_calls,
            },
            finish_reason: mock.finish_reason,
        }],
        usage: Usage {
            prompt_tokens: 50, // Mock value
            completion_tokens,
            total_tokens: 50 + completion_tokens,
        },
    }
}

/// Create a streaming SSE response.
fn create_streaming_response(
    mock: MockResponse,
    model: String,
) -> Sse<Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>>> {
    let id = format!("chatcmpl-{}", Uuid::new_v4());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let chunks = generate_stream_chunks(mock, id, model, now);

    let stream = stream::iter(chunks.into_iter().map(Ok));

    Sse::new(Box::pin(stream) as Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>>)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

/// Generate SSE chunks for streaming response.
fn generate_stream_chunks(
    mock: MockResponse,
    id: String,
    model: String,
    created: u64,
) -> Vec<Event> {
    let mut events = Vec::new();

    // First chunk: role
    events.push(create_chunk_event(
        &id,
        &model,
        created,
        Delta {
            role: Some("assistant".to_string()),
            content: None,
            tool_calls: None,
        },
        None,
    ));

    // Content chunks (if any)
    if let Some(content) = mock.content {
        // Split content into smaller chunks for realistic streaming
        let chunk_size = 10; // characters per chunk
        for chunk in content.chars().collect::<Vec<_>>().chunks(chunk_size) {
            let text: String = chunk.iter().collect();
            events.push(create_chunk_event(
                &id,
                &model,
                created,
                Delta {
                    role: None,
                    content: Some(text),
                    tool_calls: None,
                },
                None,
            ));
        }
    }

    // Tool call chunks (if any)
    if let Some(tool_calls) = mock.tool_calls {
        for (i, tc) in tool_calls.into_iter().enumerate() {
            // First: id and type
            events.push(create_chunk_event(
                &id,
                &model,
                created,
                Delta {
                    role: None,
                    content: None,
                    tool_calls: Some(vec![StreamToolCall {
                        index: i,
                        id: Some(tc.id),
                        call_type: Some(tc.call_type),
                        function: Some(StreamFunctionCall {
                            name: Some(tc.function.name),
                            arguments: None,
                        }),
                    }]),
                },
                None,
            ));

            // Then: arguments in chunks
            let args = tc.function.arguments;
            let chunk_size = 20;
            for chunk in args.chars().collect::<Vec<_>>().chunks(chunk_size) {
                let text: String = chunk.iter().collect();
                events.push(create_chunk_event(
                    &id,
                    &model,
                    created,
                    Delta {
                        role: None,
                        content: None,
                        tool_calls: Some(vec![StreamToolCall {
                            index: i,
                            id: None,
                            call_type: None,
                            function: Some(StreamFunctionCall {
                                name: None,
                                arguments: Some(text),
                            }),
                        }]),
                    },
                    None,
                ));
            }
        }
    }

    // Final chunk with finish_reason
    events.push(create_chunk_event(
        &id,
        &model,
        created,
        Delta {
            role: None,
            content: None,
            tool_calls: None,
        },
        Some(mock.finish_reason),
    ));

    // [DONE] marker
    events.push(Event::default().data("[DONE]"));

    events
}

/// Create a single SSE chunk event.
fn create_chunk_event(
    id: &str,
    model: &str,
    created: u64,
    delta: Delta,
    finish_reason: Option<String>,
) -> Event {
    let chunk = ChatCompletionChunk {
        id: id.to_string(),
        object: "chat.completion.chunk",
        created,
        model: model.to_string(),
        choices: vec![StreamChoice {
            index: 0,
            delta,
            finish_reason,
        }],
    };

    Event::default().data(serde_json::to_string(&chunk).unwrap_or_default())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_search_query() {
        assert_eq!(extract_search_query("search for movies"), "movies");
        assert_eq!(extract_search_query("find action films"), "action films");
        assert_eq!(
            extract_search_query("looking for documentaries?"),
            "documentaries"
        );
        assert_eq!(extract_search_query("random text"), "random text");
    }

    #[test]
    fn test_generate_generic_response() {
        assert!(generate_generic_response("hello there").contains("Hello"));
        assert!(generate_generic_response("can you help me").contains("help"));
        assert!(generate_generic_response("thank you").contains("welcome"));
    }
}
