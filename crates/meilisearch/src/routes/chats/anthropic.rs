//! Anthropic Claude API client for Meilisearch chat completions.
//!
//! This module provides a streaming client for Anthropic's Messages API that outputs
//! OpenAI-compatible types, allowing seamless integration with Meilisearch's existing
//! chat infrastructure.
//!
//! Translated from claude-gate's Go implementation (internal/proxy/openai_converter.go).
//!
//! NOTE: This module provides the Anthropic integration for Meilisearch chat completions.
//!
//! Some types and functions in this module are prepared for future phases (non-streaming support,
//! detailed error handling) and are marked as dead_code until they're wired in.

#![allow(dead_code)] // Some code is for future phases (non-streaming, error handling)
#![allow(deprecated)] // async_openai has deprecated function_call fields

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use async_openai::reqwest_eventsource::{Event, EventSource, RequestBuilderExt};
use async_openai::types::{
    ChatChoice, ChatChoiceStream, ChatCompletionMessageToolCall,
    ChatCompletionMessageToolCallChunk, ChatCompletionRequestAssistantMessage,
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
    ChatCompletionRequestToolMessage, ChatCompletionRequestUserMessage,
    ChatCompletionResponseMessage, ChatCompletionStreamResponseDelta, ChatCompletionTool,
    ChatCompletionToolType, CompletionUsage, CreateChatCompletionRequest,
    CreateChatCompletionResponse, CreateChatCompletionStreamResponse, FinishReason, FunctionCall,
    FunctionCallStream, Role,
};
use futures::Stream;
use http_client::reqwest::header::CONTENT_TYPE;
use meilisearch_types::error::{Code, ErrorCode};
use meilisearch_types::features::{ChatCompletionSettings, ChatCompletionSource};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tokio::sync::Mutex;
use uuid::Uuid;

use super::errors::{StreamError, StreamErrorEvent};

// ============================================================================
// Configuration
// ============================================================================

/// Configuration for the Anthropic API client.
#[derive(Debug, Clone)]
pub struct AnthropicConfig {
    pub api_key: String,
    pub base_url: String,
    pub anthropic_version: String,
}

/// Error type for Anthropic configuration.
#[derive(Debug)]
pub enum AnthropicConfigError {
    /// API key is required for Anthropic source.
    MissingApiKey,
    /// Settings source is not Anthropic.
    WrongSource,
}

impl std::fmt::Display for AnthropicConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingApiKey => write!(f, "API key is required for Anthropic source"),
            Self::WrongSource => write!(f, "Settings source must be Anthropic"),
        }
    }
}

impl std::error::Error for AnthropicConfigError {}

impl AnthropicConfig {
    /// Default Anthropic API base URL.
    pub const DEFAULT_BASE_URL: &'static str = "https://api.anthropic.com/v1/";

    /// Default Anthropic API version.
    pub const DEFAULT_VERSION: &'static str = "2023-06-01";

    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: Self::DEFAULT_BASE_URL.to_string(),
            anthropic_version: Self::DEFAULT_VERSION.to_string(),
        }
    }

    /// Creates an AnthropicConfig from ChatCompletionSettings.
    ///
    /// # Errors
    /// Returns `AnthropicConfigError::MissingApiKey` if the settings don't contain an API key.
    pub fn from_settings(settings: &ChatCompletionSettings) -> Result<Self, AnthropicConfigError> {
        if settings.source != ChatCompletionSource::Anthropic {
            return Err(AnthropicConfigError::WrongSource);
        }

        let api_key = settings.api_key.clone().ok_or(AnthropicConfigError::MissingApiKey)?;

        let base_url = settings
            .base_url
            .clone()
            .or_else(|| settings.source.base_url().map(String::from))
            .unwrap_or_else(|| Self::DEFAULT_BASE_URL.to_string());

        let anthropic_version =
            settings.api_version.clone().unwrap_or_else(|| Self::DEFAULT_VERSION.to_string());

        Ok(Self { api_key, base_url, anthropic_version })
    }

    pub fn with_base_url(mut self, base_url: String) -> Self {
        self.base_url = base_url;
        self
    }

    pub fn with_version(mut self, version: String) -> Self {
        self.anthropic_version = version;
        self
    }
}

// ============================================================================
// Anthropic API Types
// ============================================================================

/// Anthropic Messages API request format.
#[derive(Debug, Serialize)]
pub struct AnthropicRequest {
    pub model: String,
    pub max_tokens: u32,
    pub messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<AnthropicSystem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<AnthropicTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<serde_json::Value>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,
}

/// Anthropic system prompt - can be string or array of content blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AnthropicSystem {
    Text(String),
    Blocks(Vec<SystemBlock>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    pub text: String,
}

/// Anthropic message format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicMessage {
    pub role: String,
    pub content: AnthropicContent,
}

/// Content can be a string or array of content blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AnthropicContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

/// Content block types in Anthropic's format.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse { id: String, name: String, input: serde_json::Value },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
}

/// Anthropic tool definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicTool {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub input_schema: serde_json::Value,
}

/// Anthropic non-streaming response.
#[derive(Debug, Deserialize)]
pub struct AnthropicResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub response_type: String,
    pub model: String,
    pub content: Vec<ContentBlock>,
    pub stop_reason: Option<String>,
    pub usage: Option<AnthropicUsage>,
}

#[derive(Debug, Deserialize)]
pub struct AnthropicUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Anthropic error response.
#[derive(Debug, Clone, Deserialize)]
pub struct AnthropicError {
    #[serde(rename = "type")]
    pub error_type: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct AnthropicErrorResponse {
    pub error: AnthropicError,
}

// ============================================================================
// Streaming Event Types
// ============================================================================

/// Anthropic SSE event types.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum AnthropicStreamEvent {
    #[serde(rename = "message_start")]
    MessageStart { message: MessageStartData },
    #[serde(rename = "content_block_start")]
    ContentBlockStart { index: u32, content_block: ContentBlockStartData },
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta { index: u32, delta: ContentDelta },
    #[serde(rename = "content_block_stop")]
    ContentBlockStop { index: u32 },
    #[serde(rename = "message_delta")]
    MessageDelta { delta: MessageDeltaData, usage: Option<AnthropicUsage> },
    #[serde(rename = "message_stop")]
    MessageStop,
    #[serde(rename = "ping")]
    Ping,
    #[serde(rename = "error")]
    Error { error: AnthropicError },
}

#[derive(Debug, Deserialize)]
pub struct MessageStartData {
    pub id: String,
    pub model: String,
    #[serde(default)]
    pub usage: Option<AnthropicUsage>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlockStartData {
    #[serde(rename = "text")]
    Text,
    #[serde(rename = "tool_use")]
    ToolUse { id: String, name: String },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ContentDelta {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
}

#[derive(Debug, Deserialize)]
pub struct MessageDeltaData {
    pub stop_reason: Option<String>,
}

// ============================================================================
// Stream State Management
// ============================================================================

/// Tracks tool use information across SSE events.
/// Key is Anthropic content block index, value contains tool info and OpenAI tool index.
#[derive(Debug, Default)]
struct StreamState {
    /// Map from Anthropic content block index to tool info
    tool_state: HashMap<u32, ToolInfo>,
    /// Next available OpenAI tool call index
    tool_call_index: u32,
    /// Message ID from message_start event
    message_id: String,
    /// Model name from message_start event
    model: String,
    /// Unix timestamp for created field
    created: i64,
    /// Accumulated usage for the response
    usage: Option<CompletionUsage>,
}

#[derive(Debug, Clone)]
struct ToolInfo {
    id: String,
    name: String,
    openai_index: u32,
}

impl StreamState {
    fn new() -> Self {
        Self {
            tool_state: HashMap::new(),
            tool_call_index: 0,
            message_id: String::new(),
            model: String::new(),
            created: OffsetDateTime::now_utc().unix_timestamp(),
            usage: None,
        }
    }

    fn reset(&mut self) {
        self.tool_state.clear();
        self.tool_call_index = 0;
    }
}

// ============================================================================
// Request Conversion: OpenAI → Anthropic
// ============================================================================

/// Convert OpenAI CreateChatCompletionRequest to Anthropic format.
pub fn convert_request_to_anthropic(request: &CreateChatCompletionRequest) -> AnthropicRequest {
    let mut system_contents: Vec<String> = Vec::new();
    let mut anthropic_messages: Vec<AnthropicMessage> = Vec::new();

    // Process messages, extracting system messages
    for msg in &request.messages {
        match msg {
            ChatCompletionRequestMessage::System(sys) => {
                let text = extract_system_content(sys);
                if !text.is_empty() {
                    system_contents.push(text);
                }
            }
            ChatCompletionRequestMessage::User(user) => {
                let content = extract_user_content(user);
                anthropic_messages.push(AnthropicMessage { role: "user".to_string(), content });
            }
            ChatCompletionRequestMessage::Assistant(assistant) => {
                let content = extract_assistant_content(assistant);
                anthropic_messages
                    .push(AnthropicMessage { role: "assistant".to_string(), content });
            }
            ChatCompletionRequestMessage::Tool(tool) => {
                // Tool results go in user messages with tool_result content blocks
                let content = extract_tool_result_content(tool);
                anthropic_messages.push(AnthropicMessage { role: "user".to_string(), content });
            }
            ChatCompletionRequestMessage::Developer(dev) => {
                // Developer messages are treated as system in Anthropic
                let text = match &dev.content {
                    async_openai::types::ChatCompletionRequestDeveloperMessageContent::Text(t) => {
                        t.clone()
                    }
                    async_openai::types::ChatCompletionRequestDeveloperMessageContent::Array(
                        arr,
                    ) => arr.iter().map(|p| p.text.clone()).collect::<Vec<_>>().join("\n"),
                };
                if !text.is_empty() {
                    system_contents.push(text);
                }
            }
            _ => {
                // Skip function messages (deprecated) and any unknown types
            }
        }
    }

    // Build system prompt
    let system = if system_contents.is_empty() {
        None
    } else if system_contents.len() == 1 {
        Some(AnthropicSystem::Text(system_contents.remove(0)))
    } else {
        Some(AnthropicSystem::Text(system_contents.join("\n\n")))
    };

    // Convert tools
    let tools =
        request.tools.as_ref().map(|tools| tools.iter().map(convert_tool_to_anthropic).collect());

    // Convert tool_choice if present
    let tool_choice = request.tool_choice.as_ref().map(|tc| match tc {
        async_openai::types::ChatCompletionToolChoiceOption::None => {
            serde_json::json!({"type": "none"})
        }
        async_openai::types::ChatCompletionToolChoiceOption::Auto => {
            serde_json::json!({"type": "auto"})
        }
        async_openai::types::ChatCompletionToolChoiceOption::Required => {
            serde_json::json!({"type": "any"})
        }
        async_openai::types::ChatCompletionToolChoiceOption::Named(named) => {
            serde_json::json!({
                "type": "tool",
                "name": named.function.name
            })
        }
    });

    // Determine max_tokens based on model
    let max_tokens = request.max_tokens.unwrap_or_else(|| get_default_max_tokens(&request.model));

    AnthropicRequest {
        model: request.model.clone(),
        max_tokens,
        messages: anthropic_messages,
        system,
        tools,
        tool_choice,
        stream: request.stream.unwrap_or(false),
        temperature: request.temperature,
        top_p: request.top_p,
        top_k: None, // OpenAI doesn't have top_k
        stop_sequences: request.stop.clone().map(|s| match s {
            async_openai::types::Stop::String(s) => vec![s],
            async_openai::types::Stop::StringArray(arr) => arr,
        }),
    }
}

fn extract_system_content(sys: &ChatCompletionRequestSystemMessage) -> String {
    match &sys.content {
        async_openai::types::ChatCompletionRequestSystemMessageContent::Text(text) => text.clone(),
        async_openai::types::ChatCompletionRequestSystemMessageContent::Array(parts) => parts
            .iter()
            .map(|p| {
                let async_openai::types::ChatCompletionRequestSystemMessageContentPart::Text(
                    text_part,
                ) = p;
                text_part.text.clone()
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

fn extract_user_content(user: &ChatCompletionRequestUserMessage) -> AnthropicContent {
    match &user.content {
        async_openai::types::ChatCompletionRequestUserMessageContent::Text(text) => {
            AnthropicContent::Text(text.clone())
        }
        async_openai::types::ChatCompletionRequestUserMessageContent::Array(parts) => {
            let blocks: Vec<ContentBlock> = parts
                .iter()
                .filter_map(|p| match p {
                    async_openai::types::ChatCompletionRequestUserMessageContentPart::Text(
                        text_part,
                    ) => Some(ContentBlock::Text { text: text_part.text.clone() }),
                    async_openai::types::ChatCompletionRequestUserMessageContentPart::ImageUrl(
                        _img,
                    ) => {
                        // TODO: Handle image content if needed
                        None
                    }
                    _ => None,
                })
                .collect();

            if blocks.len() == 1 {
                if let ContentBlock::Text { text } = &blocks[0] {
                    return AnthropicContent::Text(text.clone());
                }
            }
            AnthropicContent::Blocks(blocks)
        }
    }
}

fn extract_assistant_content(
    assistant: &ChatCompletionRequestAssistantMessage,
) -> AnthropicContent {
    // Check for tool calls first
    if let Some(tool_calls) = &assistant.tool_calls {
        let blocks: Vec<ContentBlock> = tool_calls
            .iter()
            .map(|tc| {
                let input = match serde_json::from_str(&tc.function.arguments) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!(
                            tool_call_id = %tc.id,
                            function_name = %tc.function.name,
                            arguments_snippet = %tc.function.arguments.chars().take(100).collect::<String>(),
                            error = %e,
                            "Failed to parse tool call arguments as JSON, using null"
                        );
                        serde_json::Value::Null
                    }
                };
                ContentBlock::ToolUse {
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    input,
                }
            })
            .collect();

        // If there's also text content, prepend it
        if let Some(content) = &assistant.content {
            let text = match content {
                async_openai::types::ChatCompletionRequestAssistantMessageContent::Text(t) => {
                    t.clone()
                }
                async_openai::types::ChatCompletionRequestAssistantMessageContent::Array(arr) => {
                    arr.iter()
                        .filter_map(|p| {
                            if let async_openai::types::ChatCompletionRequestAssistantMessageContentPart::Text(text_part) = p {
                                Some(text_part.text.clone())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("")
                }
            };
            if !text.is_empty() {
                let mut all_blocks = vec![ContentBlock::Text { text }];
                all_blocks.extend(blocks);
                return AnthropicContent::Blocks(all_blocks);
            }
        }
        return AnthropicContent::Blocks(blocks);
    }

    // No tool calls, just text content
    if let Some(content) = &assistant.content {
        match content {
            async_openai::types::ChatCompletionRequestAssistantMessageContent::Text(text) => {
                AnthropicContent::Text(text.clone())
            }
            async_openai::types::ChatCompletionRequestAssistantMessageContent::Array(arr) => {
                let text: String = arr
                    .iter()
                    .filter_map(|p| {
                        if let async_openai::types::ChatCompletionRequestAssistantMessageContentPart::Text(text_part) = p {
                            Some(text_part.text.clone())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("");
                AnthropicContent::Text(text)
            }
        }
    } else {
        AnthropicContent::Text(String::new())
    }
}

fn extract_tool_result_content(tool: &ChatCompletionRequestToolMessage) -> AnthropicContent {
    let content_str = match &tool.content {
        async_openai::types::ChatCompletionRequestToolMessageContent::Text(text) => text.clone(),
        async_openai::types::ChatCompletionRequestToolMessageContent::Array(arr) => arr
            .iter()
            .map(|p| {
                let async_openai::types::ChatCompletionRequestToolMessageContentPart::Text(
                    text_part,
                ) = p;
                text_part.text.clone()
            })
            .collect::<Vec<_>>()
            .join(""),
    };

    AnthropicContent::Blocks(vec![ContentBlock::ToolResult {
        tool_use_id: tool.tool_call_id.clone(),
        content: content_str,
        is_error: None,
    }])
}

fn convert_tool_to_anthropic(tool: &ChatCompletionTool) -> AnthropicTool {
    AnthropicTool {
        name: tool.function.name.clone(),
        description: tool.function.description.clone(),
        input_schema: tool
            .function
            .parameters
            .clone()
            .unwrap_or(serde_json::json!({"type": "object", "properties": {}})),
    }
}

fn get_default_max_tokens(model: &str) -> u32 {
    // Conservative default max_tokens values for Claude models.
    //
    // These are NOT the absolute maximum values supported by each model, but rather
    // sensible defaults that work well for most use cases while leaving headroom.
    // The actual max varies by model (e.g., Claude 4.5 supports up to 64K output tokens).
    //
    // Model name patterns use substring matching to support versioned model IDs
    // (e.g., "claude-opus-4-5-20251101" matches "claude-opus-4-5").
    if model.contains("claude-opus-4-5")
        || model.contains("claude-sonnet-4-5")
        || model.contains("claude-haiku-4-5")
    {
        // Claude 4.5 series can output up to 64K tokens; 32K is a conservative default
        32000
    } else if model.contains("claude-opus-4") {
        // Claude Opus 4.x can output up to 32K tokens; 16K is a conservative default
        16000
    } else if model.contains("claude-sonnet-4") || model.contains("claude-3-7-sonnet") {
        // Claude Sonnet 4.x and 3.7 can output up to 64K tokens; 32K is a conservative default
        32000
    } else if model.contains("claude-3-5-sonnet") || model.contains("claude-3-5-haiku") {
        // Claude 3.5 series can output up to 8K tokens
        8192
    } else if model.contains("claude-3-opus")
        || model.contains("claude-3-sonnet")
        || model.contains("claude-3-haiku")
    {
        // Claude 3 series can output up to 4K tokens
        4096
    } else {
        // Unknown models get a moderate default
        8192
    }
}

// ============================================================================
// Response Conversion: Anthropic → OpenAI (Non-streaming)
// ============================================================================

/// Convert Anthropic response to OpenAI format.
pub fn convert_response_to_openai(response: AnthropicResponse) -> CreateChatCompletionResponse {
    // Extract text content and tool calls
    let mut text_content = String::new();
    let mut tool_calls: Vec<ChatCompletionMessageToolCall> = Vec::new();

    for block in &response.content {
        match block {
            ContentBlock::Text { text } => {
                text_content.push_str(text);
            }
            ContentBlock::ToolUse { id, name, input } => {
                tool_calls.push(ChatCompletionMessageToolCall {
                    id: id.clone(),
                    r#type: Some(ChatCompletionToolType::Function),
                    function: FunctionCall {
                        name: name.clone(),
                        arguments: serde_json::to_string(input).unwrap_or_default(),
                    },
                });
            }
            _ => {}
        }
    }

    // Map stop_reason to finish_reason
    let finish_reason = response.stop_reason.as_deref().map(|sr| match sr {
        "end_turn" => FinishReason::Stop,
        "max_tokens" => FinishReason::Length,
        "stop_sequence" => FinishReason::Stop,
        "tool_use" => FinishReason::ToolCalls,
        _ => FinishReason::Stop,
    });

    // Build the response message
    let message = ChatCompletionResponseMessage {
        role: Role::Assistant,
        content: if text_content.is_empty() { None } else { Some(text_content) },
        tool_calls: if tool_calls.is_empty() { None } else { Some(tool_calls) },
        function_call: None,
        refusal: None,
        audio: None,
    };

    // Build usage
    let usage = response.usage.map(|u| CompletionUsage {
        prompt_tokens: u.input_tokens,
        completion_tokens: u.output_tokens,
        total_tokens: u.input_tokens + u.output_tokens,
        prompt_tokens_details: None,
        completion_tokens_details: None,
    });

    CreateChatCompletionResponse {
        id: format!("chatcmpl-{}", Uuid::new_v4()),
        object: "chat.completion".to_string(),
        created: OffsetDateTime::now_utc().unix_timestamp() as u32,
        model: response.model,
        choices: vec![ChatChoice { index: 0, message, finish_reason, logprobs: None }],
        usage,
        system_fingerprint: None,
        service_tier: None,
    }
}

// ============================================================================
// SSE Conversion: Anthropic → OpenAI (Streaming)
// ============================================================================

/// Convert a single Anthropic SSE event to OpenAI streaming format.
///
/// Returns Ok(None) if the event should be skipped (e.g., ping events).
/// Returns Err if the event represents an error that should terminate the stream.
fn convert_sse_event_to_openai(
    event: &AnthropicStreamEvent,
    state: &mut StreamState,
) -> Result<Option<CreateChatCompletionStreamResponse>, AnthropicClientError> {
    match event {
        AnthropicStreamEvent::MessageStart { message } => {
            // Reset tool call index for new message
            state.reset();
            state.message_id = message.id.clone();
            state.model = message.model.clone();
            state.created = OffsetDateTime::now_utc().unix_timestamp();

            // Send initial chunk with role
            Ok(Some(create_stream_chunk(
                &state.message_id,
                &state.model,
                state.created,
                ChatCompletionStreamResponseDelta {
                    role: Some(Role::Assistant),
                    content: None,
                    tool_calls: None,
                    function_call: None,
                    refusal: None,
                },
                None,
                None,
            )))
        }

        AnthropicStreamEvent::ContentBlockStart { index, content_block } => {
            match content_block {
                ContentBlockStartData::ToolUse { id, name } => {
                    // Track this tool use block
                    let openai_index = state.tool_call_index;
                    state.tool_call_index += 1;
                    state.tool_state.insert(
                        *index,
                        ToolInfo { id: id.clone(), name: name.clone(), openai_index },
                    );

                    // Send initial tool call chunk
                    Ok(Some(create_stream_chunk(
                        &state.message_id,
                        &state.model,
                        state.created,
                        ChatCompletionStreamResponseDelta {
                            role: None,
                            content: None,
                            tool_calls: Some(vec![ChatCompletionMessageToolCallChunk {
                                index: openai_index,
                                id: Some(id.clone()),
                                r#type: Some(ChatCompletionToolType::Function),
                                function: Some(FunctionCallStream {
                                    name: Some(name.clone()),
                                    arguments: Some(String::new()),
                                }),
                            }]),
                            function_call: None,
                            refusal: None,
                        },
                        None,
                        None,
                    )))
                }
                ContentBlockStartData::Text => {
                    // Text blocks don't need a start event in OpenAI format
                    Ok(None)
                }
            }
        }

        AnthropicStreamEvent::ContentBlockDelta { index, delta } => {
            match delta {
                ContentDelta::TextDelta { text } => Ok(Some(create_stream_chunk(
                    &state.message_id,
                    &state.model,
                    state.created,
                    ChatCompletionStreamResponseDelta {
                        role: None,
                        content: Some(text.clone()),
                        tool_calls: None,
                        function_call: None,
                        refusal: None,
                    },
                    None,
                    None,
                ))),
                ContentDelta::InputJsonDelta { partial_json } => {
                    // Look up the tool info for this content block
                    if let Some(tool_info) = state.tool_state.get(index) {
                        Ok(Some(create_stream_chunk(
                            &state.message_id,
                            &state.model,
                            state.created,
                            ChatCompletionStreamResponseDelta {
                                role: None,
                                content: None,
                                tool_calls: Some(vec![ChatCompletionMessageToolCallChunk {
                                    index: tool_info.openai_index,
                                    id: Some(tool_info.id.clone()),
                                    r#type: None,
                                    function: Some(FunctionCallStream {
                                        name: None,
                                        arguments: Some(partial_json.clone()),
                                    }),
                                }]),
                                function_call: None,
                                refusal: None,
                            },
                            None,
                            None,
                        )))
                    } else {
                        Ok(None)
                    }
                }
            }
        }

        AnthropicStreamEvent::ContentBlockStop { index } => {
            // Clear tool state for the completed block
            state.tool_state.remove(index);
            Ok(None)
        }

        AnthropicStreamEvent::MessageDelta { delta, usage } => {
            // Update usage if provided
            if let Some(u) = usage {
                state.usage = Some(CompletionUsage {
                    prompt_tokens: u.input_tokens,
                    completion_tokens: u.output_tokens,
                    total_tokens: u.input_tokens + u.output_tokens,
                    prompt_tokens_details: None,
                    completion_tokens_details: None,
                });
            }

            // Map stop_reason to finish_reason
            let finish_reason = delta.stop_reason.as_deref().map(|sr| match sr {
                "end_turn" => FinishReason::Stop,
                "max_tokens" => FinishReason::Length,
                "stop_sequence" => FinishReason::Stop,
                "tool_use" => FinishReason::ToolCalls,
                _ => FinishReason::Stop,
            });

            if finish_reason.is_some() {
                Ok(Some(create_stream_chunk(
                    &state.message_id,
                    &state.model,
                    state.created,
                    ChatCompletionStreamResponseDelta {
                        role: None,
                        content: None,
                        tool_calls: None,
                        function_call: None,
                        refusal: None,
                    },
                    finish_reason,
                    state.usage.clone(),
                )))
            } else {
                Ok(None)
            }
        }

        AnthropicStreamEvent::MessageStop => {
            // Final chunk - OpenAI expects [DONE] which is handled by the stream
            Ok(None)
        }

        AnthropicStreamEvent::Ping => {
            // Skip ping events
            Ok(None)
        }

        AnthropicStreamEvent::Error { error } => {
            // Propagate error to stream consumer
            Err(AnthropicClientError::from_anthropic_error(error.clone()))
        }
    }
}

fn create_stream_chunk(
    id: &str,
    model: &str,
    created: i64,
    delta: ChatCompletionStreamResponseDelta,
    finish_reason: Option<FinishReason>,
    usage: Option<CompletionUsage>,
) -> CreateChatCompletionStreamResponse {
    CreateChatCompletionStreamResponse {
        id: id.to_string(),
        object: "chat.completion.chunk".to_string(),
        created: created as u32,
        model: model.to_string(),
        choices: vec![ChatChoiceStream { index: 0, delta, finish_reason, logprobs: None }],
        usage,
        system_fingerprint: None,
        service_tier: None,
    }
}

// ============================================================================
// Anthropic Client Error Handling
// ============================================================================

/// Known Anthropic API error types.
///
/// Reference: https://docs.anthropic.com/en/api/errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnthropicErrorType {
    /// Invalid request (400) - malformed request, missing parameters, etc.
    InvalidRequest,
    /// Authentication error (401) - invalid or missing API key
    Authentication,
    /// Permission error (403) - API key lacks required permissions
    Permission,
    /// Not found (404) - requested resource not found
    NotFound,
    /// Request too large (413) - request exceeds size limits
    RequestTooLarge,
    /// Rate limit exceeded (429) - too many requests
    RateLimit,
    /// Internal server error (500) - Anthropic service issue
    ApiError,
    /// Overloaded (529) - API is temporarily overloaded
    Overloaded,
    /// Unknown error type
    Unknown,
}

impl AnthropicErrorType {
    /// Parse error type from Anthropic's error type string.
    pub fn from_error_type(error_type: &str) -> Self {
        match error_type {
            "invalid_request_error" => Self::InvalidRequest,
            "authentication_error" => Self::Authentication,
            "permission_error" => Self::Permission,
            "not_found_error" => Self::NotFound,
            "request_too_large" => Self::RequestTooLarge,
            "rate_limit_error" => Self::RateLimit,
            "api_error" => Self::ApiError,
            "overloaded_error" => Self::Overloaded,
            _ => Self::Unknown,
        }
    }

    /// Get the canonical error type string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request_error",
            Self::Authentication => "authentication_error",
            Self::Permission => "permission_error",
            Self::NotFound => "not_found_error",
            Self::RequestTooLarge => "request_too_large",
            Self::RateLimit => "rate_limit_error",
            Self::ApiError => "api_error",
            Self::Overloaded => "overloaded_error",
            Self::Unknown => "unknown_error",
        }
    }
}

impl std::fmt::Display for AnthropicErrorType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Error type for Anthropic client operations.
///
/// Maps Anthropic API errors to appropriate Meilisearch error codes:
/// - `authentication_error` -> `Code::InvalidChatCompletionApiKey` (401 Unauthorized)
/// - `permission_error` -> `Code::InvalidChatCompletionApiKey` (403 Forbidden)
/// - `rate_limit_error` -> `Code::TooManySearchRequests` (503 Service Unavailable)
/// - `overloaded_error` -> `Code::TooManySearchRequests` (503 Service Unavailable)
/// - Other errors -> `Code::Internal` (500 Internal Server Error)
#[derive(Debug, thiserror::Error)]
pub enum AnthropicClientError {
    /// HTTP request failed (connection error, timeout, etc.)
    #[error("Anthropic API request failed: {0}")]
    Request(#[source] http_client::reqwest::Error),

    /// Failed to parse API response
    #[error("Failed to parse Anthropic API response: {0}")]
    Parse(#[source] serde_json::Error),

    /// Anthropic API returned an error response
    #[error("Anthropic API error ({error_type}): {message}")]
    Api {
        /// The parsed error type
        error_type: AnthropicErrorType,
        /// The error message from Anthropic (sanitized)
        message: String,
    },

    /// Error during SSE stream processing
    #[error("Anthropic stream error: {0}")]
    Stream(String),
}

impl AnthropicClientError {
    /// Create an API error from an AnthropicError response.
    ///
    /// This sanitizes the error message to avoid leaking sensitive information.
    pub fn from_anthropic_error(error: AnthropicError) -> Self {
        let error_type = AnthropicErrorType::from_error_type(&error.error_type);

        // Sanitize error message to avoid leaking API keys or sensitive data
        let message = Self::sanitize_error_message(&error.message, error_type);

        Self::Api { error_type, message }
    }

    /// Sanitize error messages to prevent leaking sensitive information.
    fn sanitize_error_message(message: &str, error_type: AnthropicErrorType) -> String {
        match error_type {
            AnthropicErrorType::Authentication | AnthropicErrorType::Permission => {
                // Don't include the original message for auth errors as it might contain API key hints
                "Invalid or missing Anthropic API key. Please check your chat completion settings."
                    .to_string()
            }
            AnthropicErrorType::RateLimit => {
                "Anthropic API rate limit exceeded. Please retry after a short delay.".to_string()
            }
            AnthropicErrorType::Overloaded => {
                "Anthropic API is temporarily overloaded. Please retry after a short delay."
                    .to_string()
            }
            _ => {
                // For other errors, include the message but ensure no API key patterns are present
                if message.to_lowercase().contains("api") && message.to_lowercase().contains("key")
                {
                    "An error occurred with the Anthropic API. Please check your configuration."
                        .to_string()
                } else {
                    message.to_string()
                }
            }
        }
    }

    /// Get the error type if this is an API error.
    pub fn error_type(&self) -> Option<AnthropicErrorType> {
        match self {
            Self::Api { error_type, .. } => Some(*error_type),
            _ => None,
        }
    }

    /// Check if this error indicates the request should be retried.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Api { error_type, .. } => matches!(
                error_type,
                AnthropicErrorType::RateLimit
                    | AnthropicErrorType::Overloaded
                    | AnthropicErrorType::ApiError
            ),
            Self::Request(e) => e.is_timeout(),
            Self::Stream(_) => true,
            Self::Parse(_) => false,
        }
    }

    /// Convert to a StreamErrorEvent for streaming error responses.
    pub fn into_stream_error_event(self) -> StreamErrorEvent {
        let event_id = Uuid::new_v4().to_string();
        let (error_type_str, code, message) = match &self {
            Self::Api { error_type, message } => {
                let code = match error_type {
                    AnthropicErrorType::Authentication | AnthropicErrorType::Permission => {
                        Some("authentication_error".to_string())
                    }
                    AnthropicErrorType::RateLimit => Some("rate_limit_error".to_string()),
                    AnthropicErrorType::Overloaded => Some("overloaded_error".to_string()),
                    _ => Some("api_error".to_string()),
                };
                (error_type.as_str().to_string(), code, message.clone())
            }
            Self::Request(e) => {
                let msg = if e.is_timeout() {
                    "Request to Anthropic API timed out".to_string()
                } else {
                    "Network error communicating with Anthropic API".to_string()
                };
                ("request_error".to_string(), Some("internal".to_string()), msg)
            }
            Self::Parse(_) => (
                "parse_error".to_string(),
                Some("internal".to_string()),
                "Failed to parse response from Anthropic API".to_string(),
            ),
            Self::Stream(msg) => {
                ("stream_error".to_string(), Some("internal".to_string()), msg.clone())
            }
        };

        StreamErrorEvent {
            event_id,
            r#type: "error".to_string(),
            error: StreamError {
                r#type: error_type_str,
                code,
                message,
                param: None,
                event_id: None,
            },
        }
    }
}

impl ErrorCode for AnthropicClientError {
    fn error_code(&self) -> Code {
        match self {
            // Authentication errors map to API key error
            Self::Api { error_type: AnthropicErrorType::Authentication, .. }
            | Self::Api { error_type: AnthropicErrorType::Permission, .. } => {
                Code::InvalidChatCompletionApiKey
            }

            // Rate limiting and overload map to TooManySearchRequests (503)
            Self::Api { error_type: AnthropicErrorType::RateLimit, .. }
            | Self::Api { error_type: AnthropicErrorType::Overloaded, .. } => {
                Code::TooManySearchRequests
            }

            // Invalid request errors
            Self::Api { error_type: AnthropicErrorType::InvalidRequest, .. }
            | Self::Api { error_type: AnthropicErrorType::RequestTooLarge, .. } => Code::BadRequest,

            // All other errors map to internal
            Self::Api { .. } | Self::Request(_) | Self::Parse(_) | Self::Stream(_) => {
                Code::Internal
            }
        }
    }
}

// ============================================================================
// Anthropic Client
// ============================================================================

/// Anthropic API client that outputs OpenAI-compatible types.
pub struct AnthropicClient {
    config: AnthropicConfig,
    http_client: http_client::reqwest::Client,
}

impl AnthropicClient {
    pub fn new(config: AnthropicConfig, ip_policy: http_client::policy::IpPolicy) -> Self {
        let http_client = http_client::reqwest::Client::builder()
            .build_with_policies(ip_policy, http_client::reqwest::redirect::Policy::default())
            .expect("Failed to build HTTP client");
        Self { config, http_client }
    }

    /// Create a streaming chat completion, returning OpenAI-compatible stream events.
    pub async fn create_stream(
        &self,
        request: CreateChatCompletionRequest,
    ) -> Result<
        impl Stream<Item = Result<CreateChatCompletionStreamResponse, AnthropicClientError>>,
        AnthropicClientError,
    > {
        let mut anthropic_request = convert_request_to_anthropic(&request);
        anthropic_request.stream = true;

        let url = format!("{}messages", self.config.base_url);

        let api_key = self.config.api_key.clone();
        let anthropic_version = self.config.anthropic_version.clone();

        let request_builder = self.http_client.post(&url).prepare(move |rb| {
            rb.header("x-api-key", api_key)
                .header("anthropic-version", anthropic_version)
                .header(CONTENT_TYPE, "application/json")
                .json(&anthropic_request)
        });

        let event_source = request_builder
            .eventsource()
            .map_err(|e| AnthropicClientError::Stream(e.to_string()))?;

        let state = Arc::new(Mutex::new(StreamState::new()));

        Ok(AnthropicStream { event_source, state, pending_event: None })
    }

    /// Create a non-streaming chat completion.
    pub async fn create(
        &self,
        request: CreateChatCompletionRequest,
    ) -> Result<CreateChatCompletionResponse, AnthropicClientError> {
        let mut anthropic_request = convert_request_to_anthropic(&request);
        anthropic_request.stream = false;

        let url = format!("{}messages", self.config.base_url);

        let api_key = self.config.api_key.clone();
        let anthropic_version = self.config.anthropic_version.clone();

        let response = self
            .http_client
            .post(&url)
            .prepare(move |rb| {
                rb.header("x-api-key", api_key)
                    .header("anthropic-version", anthropic_version)
                    .header(CONTENT_TYPE, "application/json")
                    .json(&anthropic_request)
            })
            .send()
            .await
            .map_err(AnthropicClientError::Request)?;

        let body = response.bytes().await.map_err(|e| AnthropicClientError::Request(e.into()))?;

        // Check for error response
        if let Ok(error_response) = serde_json::from_slice::<AnthropicErrorResponse>(&body) {
            return Err(AnthropicClientError::from_anthropic_error(error_response.error));
        }

        let anthropic_response: AnthropicResponse =
            serde_json::from_slice(&body).map_err(AnthropicClientError::Parse)?;

        Ok(convert_response_to_openai(anthropic_response))
    }
}

/// Stream wrapper that converts Anthropic SSE events to OpenAI format.
struct AnthropicStream {
    event_source: EventSource,
    state: Arc<Mutex<StreamState>>,
    /// Buffer for events that couldn't be processed due to lock contention
    pending_event: Option<AnthropicStreamEvent>,
}

impl Stream for AnthropicStream {
    type Item = Result<CreateChatCompletionStreamResponse, AnthropicClientError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Clone the state Arc to avoid borrowing self through the lock
        let state_arc = self.state.clone();

        loop {
            // First, try to process any pending event from a previous poll
            if let Some(pending) = self.pending_event.take() {
                match state_arc.try_lock() {
                    Ok(mut state) => {
                        match convert_sse_event_to_openai(&pending, &mut state) {
                            Ok(Some(chunk)) => return Poll::Ready(Some(Ok(chunk))),
                            Ok(None) => { /* Event processed but no output, continue */ }
                            Err(e) => return Poll::Ready(Some(Err(e))),
                        }
                    }
                    Err(_) => {
                        // Lock still contended, put the event back and return Pending
                        tracing::warn!("Stream state lock contended, buffering event for retry");
                        self.pending_event = Some(pending);
                        cx.waker().wake_by_ref();
                        return Poll::Pending;
                    }
                }
            }

            match Pin::new(&mut self.event_source).poll_next(cx) {
                Poll::Ready(Some(Ok(event))) => {
                    match event {
                        Event::Open => continue,
                        Event::Message(msg) => {
                            // Parse the Anthropic event
                            let anthropic_event: AnthropicStreamEvent =
                                match serde_json::from_str(&msg.data) {
                                    Ok(e) => e,
                                    Err(e) => {
                                        tracing::warn!(
                                            "Failed to parse Anthropic event: {} - data: {}",
                                            e,
                                            msg.data
                                        );
                                        continue;
                                    }
                                };

                            // Convert to OpenAI format
                            match state_arc.try_lock() {
                                Ok(mut state) => {
                                    match convert_sse_event_to_openai(&anthropic_event, &mut state)
                                    {
                                        Ok(Some(chunk)) => {
                                            return Poll::Ready(Some(Ok(chunk)));
                                        }
                                        Ok(None) => { /* Event processed but no output, continue */ }
                                        Err(e) => return Poll::Ready(Some(Err(e))),
                                    }
                                }
                                Err(_) => {
                                    // Lock contended, buffer the event for next poll
                                    tracing::warn!(
                                        "Stream state lock contended, buffering event for retry"
                                    );
                                    self.pending_event = Some(anthropic_event);
                                    cx.waker().wake_by_ref();
                                    return Poll::Pending;
                                }
                            }
                        }
                    }
                }
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Some(Err(AnthropicClientError::Stream(e.to_string()))));
                }
                Poll::Ready(None) => {
                    return Poll::Ready(None);
                }
                Poll::Pending => {
                    return Poll::Pending;
                }
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use meilisearch_types::error::ResponseError;

    #[test]
    fn test_convert_simple_request() {
        let request = CreateChatCompletionRequest {
            model: "claude-sonnet-4-20250514".to_string(),
            messages: vec![
                ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage {
                    content: async_openai::types::ChatCompletionRequestSystemMessageContent::Text(
                        "You are a helpful assistant.".to_string(),
                    ),
                    name: None,
                }),
                ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                    content: async_openai::types::ChatCompletionRequestUserMessageContent::Text(
                        "Hello!".to_string(),
                    ),
                    name: None,
                }),
            ],
            ..Default::default()
        };

        let anthropic_req = convert_request_to_anthropic(&request);

        assert_eq!(anthropic_req.model, "claude-sonnet-4-20250514");
        assert!(matches!(
            anthropic_req.system,
            Some(AnthropicSystem::Text(ref s)) if s == "You are a helpful assistant."
        ));
        assert_eq!(anthropic_req.messages.len(), 1);
        assert_eq!(anthropic_req.messages[0].role, "user");
    }

    #[test]
    fn test_get_default_max_tokens() {
        assert_eq!(get_default_max_tokens("claude-opus-4-5-20251101"), 32000);
        assert_eq!(get_default_max_tokens("claude-sonnet-4-20250514"), 32000);
        assert_eq!(get_default_max_tokens("claude-3-5-sonnet-20241022"), 8192);
        assert_eq!(get_default_max_tokens("claude-3-opus-20240229"), 4096);
        assert_eq!(get_default_max_tokens("unknown-model"), 8192);
    }

    #[test]
    fn test_convert_finish_reason() {
        let mut state = StreamState::new();
        state.message_id = "msg_123".to_string();
        state.model = "claude-sonnet-4".to_string();

        let event = AnthropicStreamEvent::MessageDelta {
            delta: MessageDeltaData { stop_reason: Some("tool_use".to_string()) },
            usage: None,
        };

        let chunk = convert_sse_event_to_openai(&event, &mut state).unwrap();
        assert!(chunk.is_some());

        let chunk = chunk.unwrap();
        assert_eq!(chunk.choices[0].finish_reason, Some(FinishReason::ToolCalls));
    }

    #[test]
    fn test_tool_state_tracking() {
        let mut state = StreamState::new();
        state.message_id = "msg_123".to_string();
        state.model = "claude-sonnet-4".to_string();

        // Simulate tool_use content block start
        let event = AnthropicStreamEvent::ContentBlockStart {
            index: 0,
            content_block: ContentBlockStartData::ToolUse {
                id: "toolu_123".to_string(),
                name: "_meiliSearchInIndex".to_string(),
            },
        };

        let chunk = convert_sse_event_to_openai(&event, &mut state).unwrap();
        assert!(chunk.is_some());

        // Verify state was updated
        assert_eq!(state.tool_call_index, 1);
        assert!(state.tool_state.contains_key(&0));

        let tool_info = state.tool_state.get(&0).unwrap();
        assert_eq!(tool_info.id, "toolu_123");
        assert_eq!(tool_info.name, "_meiliSearchInIndex");
        assert_eq!(tool_info.openai_index, 0);
    }

    #[test]
    fn test_response_conversion() {
        // Test basic text response conversion
        let anthropic_response = AnthropicResponse {
            id: "msg_01XFDUDYJgAACzvnptvVoYEL".to_string(),
            response_type: "message".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            content: vec![ContentBlock::Text {
                text: "Hello! How can I help you today?".to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(AnthropicUsage { input_tokens: 10, output_tokens: 25 }),
        };

        let openai_response = convert_response_to_openai(anthropic_response);

        // Verify ID format: chatcmpl-{uuid}
        assert!(openai_response.id.starts_with("chatcmpl-"));
        assert!(openai_response.id.len() > "chatcmpl-".len());

        // Verify object field
        assert_eq!(openai_response.object, "chat.completion");

        // Verify model passthrough
        assert_eq!(openai_response.model, "claude-sonnet-4-20250514");

        // Verify choices
        assert_eq!(openai_response.choices.len(), 1);
        let choice = &openai_response.choices[0];
        assert_eq!(choice.index, 0);
        assert_eq!(choice.message.content.as_deref(), Some("Hello! How can I help you today?"));
        assert_eq!(choice.message.role, Role::Assistant);
        assert!(choice.message.tool_calls.is_none());
        assert_eq!(choice.finish_reason, Some(FinishReason::Stop));

        // Verify usage conversion
        let usage = openai_response.usage.as_ref().unwrap();
        assert_eq!(usage.prompt_tokens, 10);
        assert_eq!(usage.completion_tokens, 25);
        assert_eq!(usage.total_tokens, 35);

        // Test tool_use response conversion
        let anthropic_response_with_tools = AnthropicResponse {
            id: "msg_02ABC".to_string(),
            response_type: "message".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            content: vec![ContentBlock::ToolUse {
                id: "toolu_01A".to_string(),
                name: "_meiliSearchInIndex".to_string(),
                input: serde_json::json!({"q": "search query", "index_uid": "movies"}),
            }],
            stop_reason: Some("tool_use".to_string()),
            usage: Some(AnthropicUsage { input_tokens: 50, output_tokens: 100 }),
        };

        let openai_response_tools = convert_response_to_openai(anthropic_response_with_tools);

        // Verify tool_use stop_reason maps to ToolCalls
        assert_eq!(openai_response_tools.choices[0].finish_reason, Some(FinishReason::ToolCalls));

        // Verify tool calls
        let tool_calls = openai_response_tools.choices[0].message.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "toolu_01A");
        assert_eq!(tool_calls[0].function.name, "_meiliSearchInIndex");
        assert!(tool_calls[0].function.arguments.contains("search query"));

        // Test max_tokens stop_reason
        let anthropic_response_max_tokens = AnthropicResponse {
            id: "msg_03DEF".to_string(),
            response_type: "message".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            content: vec![ContentBlock::Text { text: "Truncated response...".to_string() }],
            stop_reason: Some("max_tokens".to_string()),
            usage: None,
        };

        let openai_response_length = convert_response_to_openai(anthropic_response_max_tokens);
        assert_eq!(openai_response_length.choices[0].finish_reason, Some(FinishReason::Length));

        // Test stop_sequence stop_reason
        let anthropic_response_stop_seq = AnthropicResponse {
            id: "msg_04GHI".to_string(),
            response_type: "message".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            content: vec![ContentBlock::Text { text: "Stopped at sequence".to_string() }],
            stop_reason: Some("stop_sequence".to_string()),
            usage: None,
        };

        let openai_response_stop_seq = convert_response_to_openai(anthropic_response_stop_seq);
        assert_eq!(openai_response_stop_seq.choices[0].finish_reason, Some(FinishReason::Stop));
    }

    #[test]
    fn test_config_from_settings() {
        // Test with all fields provided
        let settings = ChatCompletionSettings {
            source: ChatCompletionSource::Anthropic,
            api_key: Some("test-api-key".to_string()),
            base_url: Some("https://custom.anthropic.com/v1/".to_string()),
            api_version: Some("2024-01-01".to_string()),
            ..Default::default()
        };

        let config = AnthropicConfig::from_settings(&settings).unwrap();
        assert_eq!(config.api_key, "test-api-key");
        assert_eq!(config.base_url, "https://custom.anthropic.com/v1/");
        assert_eq!(config.anthropic_version, "2024-01-01");

        // Test with defaults (no base_url or api_version)
        let settings_defaults = ChatCompletionSettings {
            source: ChatCompletionSource::Anthropic,
            api_key: Some("test-api-key".to_string()),
            ..Default::default()
        };

        let config_defaults = AnthropicConfig::from_settings(&settings_defaults).unwrap();
        assert_eq!(config_defaults.api_key, "test-api-key");
        assert_eq!(config_defaults.base_url, AnthropicConfig::DEFAULT_BASE_URL);
        assert_eq!(config_defaults.anthropic_version, AnthropicConfig::DEFAULT_VERSION);

        // Test missing API key error
        let settings_no_key = ChatCompletionSettings {
            source: ChatCompletionSource::Anthropic,
            api_key: None,
            ..Default::default()
        };

        let result = AnthropicConfig::from_settings(&settings_no_key);
        assert!(matches!(result, Err(AnthropicConfigError::MissingApiKey)));
    }

    // ========================================================================
    // Error Handling Tests
    // ========================================================================

    #[test]
    fn test_anthropic_error_type_parsing() {
        assert_eq!(
            AnthropicErrorType::from_error_type("invalid_request_error"),
            AnthropicErrorType::InvalidRequest
        );
        assert_eq!(
            AnthropicErrorType::from_error_type("authentication_error"),
            AnthropicErrorType::Authentication
        );
        assert_eq!(
            AnthropicErrorType::from_error_type("permission_error"),
            AnthropicErrorType::Permission
        );
        assert_eq!(
            AnthropicErrorType::from_error_type("not_found_error"),
            AnthropicErrorType::NotFound
        );
        assert_eq!(
            AnthropicErrorType::from_error_type("request_too_large"),
            AnthropicErrorType::RequestTooLarge
        );
        assert_eq!(
            AnthropicErrorType::from_error_type("rate_limit_error"),
            AnthropicErrorType::RateLimit
        );
        assert_eq!(AnthropicErrorType::from_error_type("api_error"), AnthropicErrorType::ApiError);
        assert_eq!(
            AnthropicErrorType::from_error_type("overloaded_error"),
            AnthropicErrorType::Overloaded
        );
        assert_eq!(
            AnthropicErrorType::from_error_type("unknown_type"),
            AnthropicErrorType::Unknown
        );
    }

    #[test]
    fn test_anthropic_error_type_as_str() {
        assert_eq!(AnthropicErrorType::InvalidRequest.as_str(), "invalid_request_error");
        assert_eq!(AnthropicErrorType::Authentication.as_str(), "authentication_error");
        assert_eq!(AnthropicErrorType::Permission.as_str(), "permission_error");
        assert_eq!(AnthropicErrorType::NotFound.as_str(), "not_found_error");
        assert_eq!(AnthropicErrorType::RequestTooLarge.as_str(), "request_too_large");
        assert_eq!(AnthropicErrorType::RateLimit.as_str(), "rate_limit_error");
        assert_eq!(AnthropicErrorType::ApiError.as_str(), "api_error");
        assert_eq!(AnthropicErrorType::Overloaded.as_str(), "overloaded_error");
        assert_eq!(AnthropicErrorType::Unknown.as_str(), "unknown_error");
    }

    #[test]
    fn test_error_code_mapping_authentication() {
        let error = AnthropicClientError::from_anthropic_error(AnthropicError {
            error_type: "authentication_error".to_string(),
            message: "Invalid API key provided".to_string(),
        });

        assert_eq!(error.error_code(), Code::InvalidChatCompletionApiKey);
        assert!(matches!(
            error,
            AnthropicClientError::Api { error_type: AnthropicErrorType::Authentication, .. }
        ));
    }

    #[test]
    fn test_error_code_mapping_permission() {
        let error = AnthropicClientError::from_anthropic_error(AnthropicError {
            error_type: "permission_error".to_string(),
            message: "Your API key does not have permission".to_string(),
        });

        assert_eq!(error.error_code(), Code::InvalidChatCompletionApiKey);
    }

    #[test]
    fn test_error_code_mapping_rate_limit() {
        let error = AnthropicClientError::from_anthropic_error(AnthropicError {
            error_type: "rate_limit_error".to_string(),
            message: "Rate limit exceeded".to_string(),
        });

        assert_eq!(error.error_code(), Code::TooManySearchRequests);
        assert!(matches!(
            error,
            AnthropicClientError::Api { error_type: AnthropicErrorType::RateLimit, .. }
        ));
    }

    #[test]
    fn test_error_code_mapping_overloaded() {
        let error = AnthropicClientError::from_anthropic_error(AnthropicError {
            error_type: "overloaded_error".to_string(),
            message: "Overloaded".to_string(),
        });

        assert_eq!(error.error_code(), Code::TooManySearchRequests);
        assert!(matches!(
            error,
            AnthropicClientError::Api { error_type: AnthropicErrorType::Overloaded, .. }
        ));
    }

    #[test]
    fn test_error_code_mapping_invalid_request() {
        let error = AnthropicClientError::from_anthropic_error(AnthropicError {
            error_type: "invalid_request_error".to_string(),
            message: "Invalid model specified".to_string(),
        });

        assert_eq!(error.error_code(), Code::BadRequest);
    }

    #[test]
    fn test_error_code_mapping_request_too_large() {
        let error = AnthropicClientError::from_anthropic_error(AnthropicError {
            error_type: "request_too_large".to_string(),
            message: "Request exceeds maximum allowed size".to_string(),
        });

        assert_eq!(error.error_code(), Code::BadRequest);
    }

    #[test]
    fn test_error_code_mapping_internal_errors() {
        // API error
        let api_error = AnthropicClientError::from_anthropic_error(AnthropicError {
            error_type: "api_error".to_string(),
            message: "Internal server error".to_string(),
        });
        assert_eq!(api_error.error_code(), Code::Internal);

        // Not found error
        let not_found = AnthropicClientError::from_anthropic_error(AnthropicError {
            error_type: "not_found_error".to_string(),
            message: "Resource not found".to_string(),
        });
        assert_eq!(not_found.error_code(), Code::Internal);

        // Unknown error
        let unknown = AnthropicClientError::from_anthropic_error(AnthropicError {
            error_type: "some_new_error_type".to_string(),
            message: "Unknown error".to_string(),
        });
        assert_eq!(unknown.error_code(), Code::Internal);

        // Parse error
        let parse_error =
            AnthropicClientError::Parse(serde_json::from_str::<String>("invalid").unwrap_err());
        assert_eq!(parse_error.error_code(), Code::Internal);

        // Stream error
        let stream_error = AnthropicClientError::Stream("Connection reset".to_string());
        assert_eq!(stream_error.error_code(), Code::Internal);
    }

    #[test]
    fn test_error_message_sanitization_auth() {
        // Authentication errors should have sanitized messages
        let error = AnthropicClientError::from_anthropic_error(AnthropicError {
            error_type: "authentication_error".to_string(),
            message: "Invalid API key: sk-ant-api03-xxxxx".to_string(),
        });

        if let AnthropicClientError::Api { message, .. } = &error {
            // Should not contain the original message with API key
            assert!(!message.contains("sk-ant"));
            assert!(!message.contains("xxxxx"));
            // Should have generic message
            assert!(message.contains("Invalid or missing Anthropic API key"));
        } else {
            panic!("Expected Api error variant");
        }
    }

    #[test]
    fn test_error_message_sanitization_rate_limit() {
        let error = AnthropicClientError::from_anthropic_error(AnthropicError {
            error_type: "rate_limit_error".to_string(),
            message: "Rate limit exceeded, retry after 30 seconds".to_string(),
        });

        if let AnthropicClientError::Api { message, .. } = &error {
            // Should have generic rate limit message
            assert!(message.contains("rate limit"));
            assert!(message.contains("retry"));
        } else {
            panic!("Expected Api error variant");
        }
    }

    #[test]
    fn test_error_message_sanitization_overloaded() {
        let error = AnthropicClientError::from_anthropic_error(AnthropicError {
            error_type: "overloaded_error".to_string(),
            message: "Overloaded".to_string(),
        });

        if let AnthropicClientError::Api { message, .. } = &error {
            assert!(message.contains("overloaded"));
            assert!(message.contains("retry"));
        } else {
            panic!("Expected Api error variant");
        }
    }

    #[test]
    fn test_error_message_sanitization_other_with_api_key_mention() {
        // Messages that mention "api" and "key" should be sanitized even for non-auth errors
        let error = AnthropicClientError::from_anthropic_error(AnthropicError {
            error_type: "invalid_request_error".to_string(),
            message: "The API key format is invalid".to_string(),
        });

        if let AnthropicClientError::Api { message, .. } = &error {
            // Should be sanitized
            assert!(!message.contains("API key format"));
            assert!(message.contains("check your configuration"));
        } else {
            panic!("Expected Api error variant");
        }
    }

    #[test]
    fn test_error_message_passthrough_for_safe_messages() {
        // Safe messages should pass through unchanged
        let error = AnthropicClientError::from_anthropic_error(AnthropicError {
            error_type: "invalid_request_error".to_string(),
            message: "max_tokens must be a positive integer".to_string(),
        });

        if let AnthropicClientError::Api { message, .. } = &error {
            assert_eq!(message, "max_tokens must be a positive integer");
        } else {
            panic!("Expected Api error variant");
        }
    }

    #[test]
    fn test_error_is_retryable() {
        // Rate limit is retryable
        let rate_limit = AnthropicClientError::from_anthropic_error(AnthropicError {
            error_type: "rate_limit_error".to_string(),
            message: "Rate limit exceeded".to_string(),
        });
        assert!(rate_limit.is_retryable());

        // Overloaded is retryable
        let overloaded = AnthropicClientError::from_anthropic_error(AnthropicError {
            error_type: "overloaded_error".to_string(),
            message: "Overloaded".to_string(),
        });
        assert!(overloaded.is_retryable());

        // API error is retryable
        let api_error = AnthropicClientError::from_anthropic_error(AnthropicError {
            error_type: "api_error".to_string(),
            message: "Internal error".to_string(),
        });
        assert!(api_error.is_retryable());

        // Authentication errors are not retryable
        let auth_error = AnthropicClientError::from_anthropic_error(AnthropicError {
            error_type: "authentication_error".to_string(),
            message: "Invalid API key".to_string(),
        });
        assert!(!auth_error.is_retryable());

        // Invalid request is not retryable
        let invalid_request = AnthropicClientError::from_anthropic_error(AnthropicError {
            error_type: "invalid_request_error".to_string(),
            message: "Bad request".to_string(),
        });
        assert!(!invalid_request.is_retryable());

        // Parse errors are not retryable
        let parse_error =
            AnthropicClientError::Parse(serde_json::from_str::<String>("invalid").unwrap_err());
        assert!(!parse_error.is_retryable());

        // Stream errors are retryable
        let stream_error = AnthropicClientError::Stream("Connection reset".to_string());
        assert!(stream_error.is_retryable());
    }

    #[test]
    fn test_error_to_response_error() {
        let error = AnthropicClientError::from_anthropic_error(AnthropicError {
            error_type: "rate_limit_error".to_string(),
            message: "Rate limit exceeded".to_string(),
        });

        let response_error: ResponseError = error.into();

        // Check that the error message is present
        assert!(response_error.to_string().contains("rate_limit_error"));
    }

    #[test]
    fn test_error_into_stream_error_event() {
        // Test authentication error
        let auth_error = AnthropicClientError::from_anthropic_error(AnthropicError {
            error_type: "authentication_error".to_string(),
            message: "Invalid API key".to_string(),
        });
        let event = auth_error.into_stream_error_event();
        assert_eq!(event.r#type, "error");
        assert_eq!(event.error.r#type, "authentication_error");
        assert_eq!(event.error.code, Some("authentication_error".to_string()));

        // Test rate limit error
        let rate_limit_error = AnthropicClientError::from_anthropic_error(AnthropicError {
            error_type: "rate_limit_error".to_string(),
            message: "Too many requests".to_string(),
        });
        let event = rate_limit_error.into_stream_error_event();
        assert_eq!(event.error.r#type, "rate_limit_error");
        assert_eq!(event.error.code, Some("rate_limit_error".to_string()));

        // Test stream error
        let stream_error = AnthropicClientError::Stream("Connection lost".to_string());
        let event = stream_error.into_stream_error_event();
        assert_eq!(event.error.r#type, "stream_error");
        assert_eq!(event.error.code, Some("internal".to_string()));
        assert_eq!(event.error.message, "Connection lost");

        // Test parse error
        let parse_error =
            AnthropicClientError::Parse(serde_json::from_str::<String>("invalid").unwrap_err());
        let event = parse_error.into_stream_error_event();
        assert_eq!(event.error.r#type, "parse_error");
        assert_eq!(event.error.code, Some("internal".to_string()));
    }

    #[test]
    fn test_error_display() {
        let api_error = AnthropicClientError::Api {
            error_type: AnthropicErrorType::RateLimit,
            message: "Rate limit exceeded".to_string(),
        };
        let display = format!("{}", api_error);
        assert!(display.contains("rate_limit_error"));
        assert!(display.contains("Rate limit exceeded"));

        let stream_error = AnthropicClientError::Stream("Connection reset".to_string());
        let display = format!("{}", stream_error);
        assert!(display.contains("Connection reset"));
    }

    #[test]
    fn test_error_type_getter() {
        let api_error = AnthropicClientError::Api {
            error_type: AnthropicErrorType::RateLimit,
            message: "Rate limit exceeded".to_string(),
        };
        assert_eq!(api_error.error_type(), Some(AnthropicErrorType::RateLimit));

        let stream_error = AnthropicClientError::Stream("Connection reset".to_string());
        assert_eq!(stream_error.error_type(), None);
    }

    // ============================================================================
    // Message Type Conversion Tests
    // ============================================================================

    #[test]
    fn test_convert_developer_message_merged_with_system() {
        use async_openai::types::{
            ChatCompletionRequestDeveloperMessage, ChatCompletionRequestDeveloperMessageContent,
        };

        let request = CreateChatCompletionRequest {
            model: "claude-sonnet-4-20250514".to_string(),
            messages: vec![
                ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage {
                    content: async_openai::types::ChatCompletionRequestSystemMessageContent::Text(
                        "You are a helpful assistant.".to_string(),
                    ),
                    name: None,
                }),
                ChatCompletionRequestMessage::Developer(ChatCompletionRequestDeveloperMessage {
                    content: ChatCompletionRequestDeveloperMessageContent::Text(
                        "Additional developer instructions.".to_string(),
                    ),
                    name: None,
                }),
                ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                    content: async_openai::types::ChatCompletionRequestUserMessageContent::Text(
                        "Hello!".to_string(),
                    ),
                    name: None,
                }),
            ],
            ..Default::default()
        };

        let anthropic_req = convert_request_to_anthropic(&request);

        // System and developer messages should be merged
        assert!(matches!(
            anthropic_req.system,
            Some(AnthropicSystem::Text(ref s)) if s.contains("You are a helpful assistant.") && s.contains("Additional developer instructions.")
        ));
        // Only user message should remain
        assert_eq!(anthropic_req.messages.len(), 1);
        assert_eq!(anthropic_req.messages[0].role, "user");
    }

    #[test]
    fn test_convert_assistant_message_with_tool_calls() {
        let request = CreateChatCompletionRequest {
            model: "claude-sonnet-4-20250514".to_string(),
            messages: vec![
                ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                    content: async_openai::types::ChatCompletionRequestUserMessageContent::Text(
                        "Search for movies".to_string(),
                    ),
                    name: None,
                }),
                ChatCompletionRequestMessage::Assistant(ChatCompletionRequestAssistantMessage {
                    content: Some(
                        async_openai::types::ChatCompletionRequestAssistantMessageContent::Text(
                            "I'll search for movies.".to_string(),
                        ),
                    ),
                    tool_calls: Some(vec![ChatCompletionMessageToolCall {
                        id: "call_123".to_string(),
                        r#type: Some(ChatCompletionToolType::Function),
                        function: FunctionCall {
                            name: "_meiliSearchInIndex".to_string(),
                            arguments: r#"{"q": "movies", "index_uid": "films"}"#.to_string(),
                        },
                    }]),
                    name: None,
                    refusal: None,
                    audio: None,
                    function_call: None,
                }),
            ],
            ..Default::default()
        };

        let anthropic_req = convert_request_to_anthropic(&request);

        assert_eq!(anthropic_req.messages.len(), 2);

        // Check assistant message has both text and tool_use blocks
        let assistant_msg = &anthropic_req.messages[1];
        assert_eq!(assistant_msg.role, "assistant");

        match &assistant_msg.content {
            AnthropicContent::Blocks(blocks) => {
                assert_eq!(blocks.len(), 2);
                // First block should be text
                assert!(
                    matches!(&blocks[0], ContentBlock::Text { text } if text == "I'll search for movies.")
                );
                // Second block should be tool_use
                assert!(matches!(&blocks[1], ContentBlock::ToolUse { id, name, .. }
                    if id == "call_123" && name == "_meiliSearchInIndex"));
            }
            _ => panic!("Expected Blocks content"),
        }
    }

    #[test]
    fn test_convert_tool_message_to_tool_result() {
        let request = CreateChatCompletionRequest {
            model: "claude-sonnet-4-20250514".to_string(),
            messages: vec![
                ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                    content: async_openai::types::ChatCompletionRequestUserMessageContent::Text(
                        "Search for movies".to_string(),
                    ),
                    name: None,
                }),
                ChatCompletionRequestMessage::Assistant(ChatCompletionRequestAssistantMessage {
                    content: None,
                    tool_calls: Some(vec![ChatCompletionMessageToolCall {
                        id: "call_123".to_string(),
                        r#type: Some(ChatCompletionToolType::Function),
                        function: FunctionCall {
                            name: "_meiliSearchInIndex".to_string(),
                            arguments: r#"{"q": "movies"}"#.to_string(),
                        },
                    }]),
                    name: None,
                    refusal: None,
                    audio: None,
                    function_call: None,
                }),
                ChatCompletionRequestMessage::Tool(ChatCompletionRequestToolMessage {
                    content: async_openai::types::ChatCompletionRequestToolMessageContent::Text(
                        r#"{"hits": [{"title": "The Matrix"}]}"#.to_string(),
                    ),
                    tool_call_id: "call_123".to_string(),
                }),
            ],
            ..Default::default()
        };

        let anthropic_req = convert_request_to_anthropic(&request);

        // Should have 3 messages: user, assistant, user (with tool_result)
        assert_eq!(anthropic_req.messages.len(), 3);

        // Check the tool result message
        let tool_result_msg = &anthropic_req.messages[2];
        assert_eq!(tool_result_msg.role, "user");

        match &tool_result_msg.content {
            AnthropicContent::Blocks(blocks) => {
                assert_eq!(blocks.len(), 1);
                match &blocks[0] {
                    ContentBlock::ToolResult { tool_use_id, content, is_error } => {
                        assert_eq!(tool_use_id, "call_123");
                        assert!(content.contains("The Matrix"));
                        assert!(is_error.is_none());
                    }
                    _ => panic!("Expected ToolResult block"),
                }
            }
            _ => panic!("Expected Blocks content"),
        }
    }

    #[test]
    fn test_convert_tool_definitions() {
        use async_openai::types::FunctionObject;

        let request = CreateChatCompletionRequest {
            model: "claude-sonnet-4-20250514".to_string(),
            messages: vec![ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                content: async_openai::types::ChatCompletionRequestUserMessageContent::Text(
                    "Hello".to_string(),
                ),
                name: None,
            })],
            tools: Some(vec![ChatCompletionTool {
                r#type: ChatCompletionToolType::Function,
                function: FunctionObject {
                    name: "_meiliSearchInIndex".to_string(),
                    description: Some("Search in an index".to_string()),
                    parameters: Some(serde_json::json!({
                        "type": "object",
                        "properties": {
                            "q": {"type": "string", "description": "Search query"},
                            "index_uid": {"type": "string", "description": "Index to search"}
                        },
                        "required": ["q", "index_uid"]
                    })),
                    strict: None,
                },
            }]),
            ..Default::default()
        };

        let anthropic_req = convert_request_to_anthropic(&request);

        assert!(anthropic_req.tools.is_some());
        let tools = anthropic_req.tools.unwrap();
        assert_eq!(tools.len(), 1);

        let tool = &tools[0];
        assert_eq!(tool.name, "_meiliSearchInIndex");
        assert_eq!(tool.description, Some("Search in an index".to_string()));

        // Verify input_schema has correct structure
        assert!(tool.input_schema.get("type").is_some());
        assert_eq!(tool.input_schema["type"], "object");
        assert!(tool.input_schema.get("properties").is_some());
    }

    #[test]
    fn test_convert_stop_sequences() {
        let request = CreateChatCompletionRequest {
            model: "claude-sonnet-4-20250514".to_string(),
            messages: vec![ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                content: async_openai::types::ChatCompletionRequestUserMessageContent::Text(
                    "Hello".to_string(),
                ),
                name: None,
            })],
            stop: Some(async_openai::types::Stop::StringArray(vec![
                "STOP".to_string(),
                "END".to_string(),
            ])),
            ..Default::default()
        };

        let anthropic_req = convert_request_to_anthropic(&request);

        assert!(anthropic_req.stop_sequences.is_some());
        let stop_seqs = anthropic_req.stop_sequences.unwrap();
        assert_eq!(stop_seqs.len(), 2);
        assert!(stop_seqs.contains(&"STOP".to_string()));
        assert!(stop_seqs.contains(&"END".to_string()));
    }

    #[test]
    fn test_convert_temperature_and_top_p() {
        let request = CreateChatCompletionRequest {
            model: "claude-sonnet-4-20250514".to_string(),
            messages: vec![ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                content: async_openai::types::ChatCompletionRequestUserMessageContent::Text(
                    "Hello".to_string(),
                ),
                name: None,
            })],
            temperature: Some(0.7),
            top_p: Some(0.9),
            ..Default::default()
        };

        let anthropic_req = convert_request_to_anthropic(&request);

        assert_eq!(anthropic_req.temperature, Some(0.7));
        assert_eq!(anthropic_req.top_p, Some(0.9));
    }

    #[test]
    fn test_max_tokens_default_when_not_specified() {
        let request = CreateChatCompletionRequest {
            model: "claude-3-opus-20240229".to_string(),
            messages: vec![ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                content: async_openai::types::ChatCompletionRequestUserMessageContent::Text(
                    "Hello".to_string(),
                ),
                name: None,
            })],
            max_tokens: None, // Not specified
            ..Default::default()
        };

        let anthropic_req = convert_request_to_anthropic(&request);

        // Claude 3 Opus should default to 4096
        assert_eq!(anthropic_req.max_tokens, 4096);
    }

    #[test]
    fn test_max_tokens_preserved_when_specified() {
        let request = CreateChatCompletionRequest {
            model: "claude-3-opus-20240229".to_string(),
            messages: vec![ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                content: async_openai::types::ChatCompletionRequestUserMessageContent::Text(
                    "Hello".to_string(),
                ),
                name: None,
            })],
            max_tokens: Some(1000),
            ..Default::default()
        };

        let anthropic_req = convert_request_to_anthropic(&request);

        assert_eq!(anthropic_req.max_tokens, 1000);
    }

    #[test]
    fn test_convert_user_message_with_array_content() {
        use async_openai::types::{
            ChatCompletionRequestMessageContentPartText,
            ChatCompletionRequestUserMessageContentPart,
        };

        let request = CreateChatCompletionRequest {
            model: "claude-sonnet-4-20250514".to_string(),
            messages: vec![ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                content: async_openai::types::ChatCompletionRequestUserMessageContent::Array(vec![
                    ChatCompletionRequestUserMessageContentPart::Text(
                        ChatCompletionRequestMessageContentPartText {
                            text: "First part. ".to_string(),
                        },
                    ),
                    ChatCompletionRequestUserMessageContentPart::Text(
                        ChatCompletionRequestMessageContentPartText {
                            text: "Second part.".to_string(),
                        },
                    ),
                ]),
                name: None,
            })],
            ..Default::default()
        };

        let anthropic_req = convert_request_to_anthropic(&request);

        assert_eq!(anthropic_req.messages.len(), 1);
        // With multiple text parts, should become blocks
        match &anthropic_req.messages[0].content {
            AnthropicContent::Blocks(blocks) => {
                assert_eq!(blocks.len(), 2);
            }
            AnthropicContent::Text(_) => {
                // Single text is also acceptable if implementation simplifies
            }
        }
    }

    // ============================================================================
    // Integration Tests: Full Request/Response Pipeline
    // ============================================================================
    //
    // These tests verify the complete conversion pipeline from OpenAI request
    // format through Anthropic request/response format and back to OpenAI format.

    /// Test a complete basic chat flow: OpenAI request -> Anthropic request -> Anthropic response -> OpenAI response
    #[test]
    fn test_anthropic_basic_chat_pipeline() {
        // 1. Create an OpenAI-style request
        let openai_request = CreateChatCompletionRequest {
            model: "claude-sonnet-4-20250514".to_string(),
            messages: vec![
                ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage {
                    content: async_openai::types::ChatCompletionRequestSystemMessageContent::Text(
                        "You are a helpful assistant.".to_string(),
                    ),
                    name: None,
                }),
                ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                    content: async_openai::types::ChatCompletionRequestUserMessageContent::Text(
                        "What is the capital of France?".to_string(),
                    ),
                    name: None,
                }),
            ],
            temperature: Some(0.7),
            max_tokens: Some(100),
            ..Default::default()
        };

        // 2. Convert to Anthropic format
        let anthropic_request = convert_request_to_anthropic(&openai_request);

        // 3. Verify Anthropic request structure
        assert_eq!(anthropic_request.model, "claude-sonnet-4-20250514");
        assert_eq!(anthropic_request.max_tokens, 100);
        assert_eq!(anthropic_request.temperature, Some(0.7));
        assert!(matches!(
            anthropic_request.system,
            Some(AnthropicSystem::Text(ref s)) if s == "You are a helpful assistant."
        ));
        assert_eq!(anthropic_request.messages.len(), 1);
        assert_eq!(anthropic_request.messages[0].role, "user");

        // 4. Simulate Anthropic response
        let anthropic_response = AnthropicResponse {
            id: "msg_01XYZ123".to_string(),
            response_type: "message".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            content: vec![ContentBlock::Text {
                text: "The capital of France is Paris.".to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(AnthropicUsage { input_tokens: 25, output_tokens: 10 }),
        };

        // 5. Convert back to OpenAI format
        let openai_response = convert_response_to_openai(anthropic_response);

        // 6. Verify OpenAI response structure
        assert!(openai_response.id.starts_with("chatcmpl-"));
        assert_eq!(openai_response.object, "chat.completion");
        assert_eq!(openai_response.model, "claude-sonnet-4-20250514");
        assert_eq!(openai_response.choices.len(), 1);

        let choice = &openai_response.choices[0];
        assert_eq!(choice.index, 0);
        assert_eq!(choice.message.role, Role::Assistant);
        assert_eq!(choice.message.content.as_deref(), Some("The capital of France is Paris."));
        assert_eq!(choice.finish_reason, Some(FinishReason::Stop));

        let usage = openai_response.usage.as_ref().unwrap();
        assert_eq!(usage.prompt_tokens, 25);
        assert_eq!(usage.completion_tokens, 10);
        assert_eq!(usage.total_tokens, 35);
    }

    /// Test streaming event sequence: message_start -> content_block_start -> content_block_delta -> message_delta -> message_stop
    #[test]
    fn test_anthropic_streaming_event_sequence() {
        let mut state = StreamState::new();

        // 1. message_start - should emit chunk with role
        let event1 = AnthropicStreamEvent::MessageStart {
            message: MessageStartData {
                id: "msg_stream_123".to_string(),
                model: "claude-sonnet-4-20250514".to_string(),
                usage: None,
            },
        };
        let chunk1 = convert_sse_event_to_openai(&event1, &mut state).unwrap();
        assert!(chunk1.is_some());
        let chunk1 = chunk1.unwrap();
        assert_eq!(chunk1.id, "msg_stream_123");
        assert_eq!(chunk1.model, "claude-sonnet-4-20250514");
        assert_eq!(chunk1.choices[0].delta.role, Some(Role::Assistant));
        assert!(chunk1.choices[0].delta.content.is_none());
        assert!(chunk1.choices[0].finish_reason.is_none());

        // 2. content_block_start (text) - should be skipped
        let event2 = AnthropicStreamEvent::ContentBlockStart {
            index: 0,
            content_block: ContentBlockStartData::Text,
        };
        let chunk2 = convert_sse_event_to_openai(&event2, &mut state).unwrap();
        assert!(chunk2.is_none());

        // 3. content_block_delta (text) - should emit content
        let event3 = AnthropicStreamEvent::ContentBlockDelta {
            index: 0,
            delta: ContentDelta::TextDelta { text: "Hello, ".to_string() },
        };
        let chunk3 = convert_sse_event_to_openai(&event3, &mut state).unwrap();
        assert!(chunk3.is_some());
        let chunk3 = chunk3.unwrap();
        assert_eq!(chunk3.choices[0].delta.content, Some("Hello, ".to_string()));
        assert!(chunk3.choices[0].finish_reason.is_none());

        // 4. Another text delta
        let event4 = AnthropicStreamEvent::ContentBlockDelta {
            index: 0,
            delta: ContentDelta::TextDelta { text: "world!".to_string() },
        };
        let chunk4 = convert_sse_event_to_openai(&event4, &mut state).unwrap();
        assert!(chunk4.is_some());
        let chunk4 = chunk4.unwrap();
        assert_eq!(chunk4.choices[0].delta.content, Some("world!".to_string()));

        // 5. content_block_stop - should be skipped
        let event5 = AnthropicStreamEvent::ContentBlockStop { index: 0 };
        let chunk5 = convert_sse_event_to_openai(&event5, &mut state).unwrap();
        assert!(chunk5.is_none());

        // 6. message_delta with stop_reason - should emit finish_reason
        let event6 = AnthropicStreamEvent::MessageDelta {
            delta: MessageDeltaData { stop_reason: Some("end_turn".to_string()) },
            usage: Some(AnthropicUsage { input_tokens: 10, output_tokens: 5 }),
        };
        let chunk6 = convert_sse_event_to_openai(&event6, &mut state).unwrap();
        assert!(chunk6.is_some());
        let chunk6 = chunk6.unwrap();
        assert_eq!(chunk6.choices[0].finish_reason, Some(FinishReason::Stop));

        // 7. message_stop - should be skipped
        let event7 = AnthropicStreamEvent::MessageStop;
        let chunk7 = convert_sse_event_to_openai(&event7, &mut state).unwrap();
        assert!(chunk7.is_none());

        // 8. ping - should be skipped
        let event8 = AnthropicStreamEvent::Ping;
        let chunk8 = convert_sse_event_to_openai(&event8, &mut state).unwrap();
        assert!(chunk8.is_none());
    }

    /// Test single tool call streaming sequence
    #[test]
    fn test_anthropic_tool_call_streaming() {
        let mut state = StreamState::new();

        // 1. message_start
        let event1 = AnthropicStreamEvent::MessageStart {
            message: MessageStartData {
                id: "msg_tool_123".to_string(),
                model: "claude-sonnet-4-20250514".to_string(),
                usage: None,
            },
        };
        convert_sse_event_to_openai(&event1, &mut state).unwrap();

        // 2. content_block_start (tool_use) - should emit tool call id and name
        let event2 = AnthropicStreamEvent::ContentBlockStart {
            index: 0,
            content_block: ContentBlockStartData::ToolUse {
                id: "toolu_abc123".to_string(),
                name: "_meiliSearchInIndex".to_string(),
            },
        };
        let chunk2 = convert_sse_event_to_openai(&event2, &mut state).unwrap();
        assert!(chunk2.is_some());
        let chunk2 = chunk2.unwrap();
        let tool_calls = chunk2.choices[0].delta.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].index, 0);
        assert_eq!(tool_calls[0].id, Some("toolu_abc123".to_string()));
        assert_eq!(tool_calls[0].function.as_ref().unwrap().name, Some("_meiliSearchInIndex".to_string()));

        // 3. content_block_delta (input_json) - first part
        let event3 = AnthropicStreamEvent::ContentBlockDelta {
            index: 0,
            delta: ContentDelta::InputJsonDelta { partial_json: r#"{"q": "se"#.to_string() },
        };
        let chunk3 = convert_sse_event_to_openai(&event3, &mut state).unwrap();
        assert!(chunk3.is_some());
        let chunk3 = chunk3.unwrap();
        let tool_calls = chunk3.choices[0].delta.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls[0].function.as_ref().unwrap().arguments, Some(r#"{"q": "se"#.to_string()));

        // 4. content_block_delta (input_json) - second part
        let event4 = AnthropicStreamEvent::ContentBlockDelta {
            index: 0,
            delta: ContentDelta::InputJsonDelta { partial_json: r#"arch"}"#.to_string() },
        };
        let chunk4 = convert_sse_event_to_openai(&event4, &mut state).unwrap();
        assert!(chunk4.is_some());
        let chunk4 = chunk4.unwrap();
        let tool_calls = chunk4.choices[0].delta.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls[0].function.as_ref().unwrap().arguments, Some(r#"arch"}"#.to_string()));

        // 5. content_block_stop
        let event5 = AnthropicStreamEvent::ContentBlockStop { index: 0 };
        convert_sse_event_to_openai(&event5, &mut state).unwrap();

        // Verify tool state was cleared
        assert!(state.tool_state.is_empty());

        // 6. message_delta with tool_use stop_reason
        let event6 = AnthropicStreamEvent::MessageDelta {
            delta: MessageDeltaData { stop_reason: Some("tool_use".to_string()) },
            usage: Some(AnthropicUsage { input_tokens: 50, output_tokens: 25 }),
        };
        let chunk6 = convert_sse_event_to_openai(&event6, &mut state).unwrap();
        assert!(chunk6.is_some());
        let chunk6 = chunk6.unwrap();
        assert_eq!(chunk6.choices[0].finish_reason, Some(FinishReason::ToolCalls));
    }

    /// Test multiple tool calls in a single response
    #[test]
    fn test_anthropic_multi_tool_streaming() {
        let mut state = StreamState::new();

        // Setup message_start
        let event1 = AnthropicStreamEvent::MessageStart {
            message: MessageStartData {
                id: "msg_multi_tool".to_string(),
                model: "claude-sonnet-4-20250514".to_string(),
                usage: None,
            },
        };
        convert_sse_event_to_openai(&event1, &mut state).unwrap();

        // First tool: content_block_start
        let event2 = AnthropicStreamEvent::ContentBlockStart {
            index: 0,
            content_block: ContentBlockStartData::ToolUse {
                id: "toolu_first".to_string(),
                name: "_meiliSearchInIndex".to_string(),
            },
        };
        let chunk2 = convert_sse_event_to_openai(&event2, &mut state).unwrap().unwrap();
        let tool_calls2 = chunk2.choices[0].delta.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls2[0].index, 0);
        assert_eq!(tool_calls2[0].id, Some("toolu_first".to_string()));

        // First tool: arguments
        let event3 = AnthropicStreamEvent::ContentBlockDelta {
            index: 0,
            delta: ContentDelta::InputJsonDelta {
                partial_json: r#"{"index_uid": "movies", "q": "action"}"#.to_string(),
            },
        };
        let chunk3 = convert_sse_event_to_openai(&event3, &mut state).unwrap().unwrap();
        let tool_calls3 = chunk3.choices[0].delta.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls3[0].index, 0);

        // First tool: stop
        convert_sse_event_to_openai(
            &AnthropicStreamEvent::ContentBlockStop { index: 0 },
            &mut state,
        )
        .unwrap();

        // Second tool: content_block_start
        let event4 = AnthropicStreamEvent::ContentBlockStart {
            index: 1,
            content_block: ContentBlockStartData::ToolUse {
                id: "toolu_second".to_string(),
                name: "_meiliSearchInIndex".to_string(),
            },
        };
        let chunk4 = convert_sse_event_to_openai(&event4, &mut state).unwrap().unwrap();
        let tool_calls4 = chunk4.choices[0].delta.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls4[0].index, 1); // OpenAI index should be 1
        assert_eq!(tool_calls4[0].id, Some("toolu_second".to_string()));

        // Verify internal state tracking
        assert_eq!(state.tool_call_index, 2);
        assert!(state.tool_state.contains_key(&1));
    }

    /// Test multi-turn conversation with tool results
    #[test]
    fn test_anthropic_multi_turn_with_tool_results() {
        // Create a conversation with: user -> assistant (tool call) -> tool result -> user follow-up
        let request = CreateChatCompletionRequest {
            model: "claude-sonnet-4-20250514".to_string(),
            messages: vec![
                // Initial user message
                ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                    content: async_openai::types::ChatCompletionRequestUserMessageContent::Text(
                        "Search for sci-fi movies".to_string(),
                    ),
                    name: None,
                }),
                // Assistant's tool call response
                ChatCompletionRequestMessage::Assistant(ChatCompletionRequestAssistantMessage {
                    content: Some(
                        async_openai::types::ChatCompletionRequestAssistantMessageContent::Text(
                            "I'll search for sci-fi movies.".to_string(),
                        ),
                    ),
                    tool_calls: Some(vec![ChatCompletionMessageToolCall {
                        id: "call_scifi".to_string(),
                        r#type: Some(ChatCompletionToolType::Function),
                        function: FunctionCall {
                            name: "_meiliSearchInIndex".to_string(),
                            arguments: r#"{"index_uid": "movies", "q": "sci-fi"}"#.to_string(),
                        },
                    }]),
                    name: None,
                    refusal: None,
                    audio: None,
                    function_call: None,
                }),
                // Tool result
                ChatCompletionRequestMessage::Tool(ChatCompletionRequestToolMessage {
                    content: async_openai::types::ChatCompletionRequestToolMessageContent::Text(
                        r#"{"hits": [{"title": "Blade Runner"}, {"title": "The Matrix"}]}"#
                            .to_string(),
                    ),
                    tool_call_id: "call_scifi".to_string(),
                }),
                // Follow-up user message
                ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                    content: async_openai::types::ChatCompletionRequestUserMessageContent::Text(
                        "Tell me more about the first one".to_string(),
                    ),
                    name: None,
                }),
            ],
            ..Default::default()
        };

        let anthropic_req = convert_request_to_anthropic(&request);

        // Should have 4 messages: user, assistant, user (tool_result), user
        assert_eq!(anthropic_req.messages.len(), 4);

        // First: user message
        assert_eq!(anthropic_req.messages[0].role, "user");
        match &anthropic_req.messages[0].content {
            AnthropicContent::Text(t) => assert!(t.contains("sci-fi")),
            AnthropicContent::Blocks(b) => {
                assert!(matches!(&b[0], ContentBlock::Text { text } if text.contains("sci-fi")))
            }
        }

        // Second: assistant with tool_use
        assert_eq!(anthropic_req.messages[1].role, "assistant");
        match &anthropic_req.messages[1].content {
            AnthropicContent::Blocks(blocks) => {
                assert_eq!(blocks.len(), 2);
                assert!(
                    matches!(&blocks[1], ContentBlock::ToolUse { id, name, .. } if id == "call_scifi" && name == "_meiliSearchInIndex")
                );
            }
            _ => panic!("Expected assistant with blocks"),
        }

        // Third: user with tool_result
        assert_eq!(anthropic_req.messages[2].role, "user");
        match &anthropic_req.messages[2].content {
            AnthropicContent::Blocks(blocks) => {
                assert_eq!(blocks.len(), 1);
                match &blocks[0] {
                    ContentBlock::ToolResult { tool_use_id, content, is_error } => {
                        assert_eq!(tool_use_id, "call_scifi");
                        assert!(content.contains("Blade Runner"));
                        assert!(is_error.is_none());
                    }
                    _ => panic!("Expected ToolResult block"),
                }
            }
            _ => panic!("Expected blocks content"),
        }

        // Fourth: user follow-up
        assert_eq!(anthropic_req.messages[3].role, "user");
    }

    /// Test error event handling in streams
    #[test]
    fn test_anthropic_streaming_error_event() {
        let mut state = StreamState::new();
        state.message_id = "msg_error".to_string();
        state.model = "claude-sonnet-4-20250514".to_string();

        // Error event should return an error
        let error_event = AnthropicStreamEvent::Error {
            error: AnthropicError {
                error_type: "overloaded_error".to_string(),
                message: "The API is temporarily overloaded".to_string(),
            },
        };

        let result = convert_sse_event_to_openai(&error_event, &mut state);
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(matches!(
            err,
            AnthropicClientError::Api { error_type: AnthropicErrorType::Overloaded, .. }
        ));
        assert!(err.is_retryable());
    }

    /// Test all stop_reason to finish_reason mappings
    #[test]
    fn test_anthropic_all_stop_reason_mappings() {
        let test_cases = vec![
            ("end_turn", FinishReason::Stop),
            ("max_tokens", FinishReason::Length),
            ("stop_sequence", FinishReason::Stop),
            ("tool_use", FinishReason::ToolCalls),
            ("unknown_reason", FinishReason::Stop), // Unknown maps to Stop
        ];

        for (anthropic_reason, expected_finish) in test_cases {
            let mut state = StreamState::new();
            state.message_id = "msg_test".to_string();
            state.model = "claude-sonnet-4".to_string();

            let event = AnthropicStreamEvent::MessageDelta {
                delta: MessageDeltaData { stop_reason: Some(anthropic_reason.to_string()) },
                usage: None,
            };

            let chunk = convert_sse_event_to_openai(&event, &mut state).unwrap();
            assert!(chunk.is_some(), "Expected chunk for stop_reason: {}", anthropic_reason);
            let chunk = chunk.unwrap();
            assert_eq!(
                chunk.choices[0].finish_reason,
                Some(expected_finish),
                "Mismatch for stop_reason: {}",
                anthropic_reason
            );
        }
    }

    /// Test rate limit error handling and response generation
    #[test]
    fn test_anthropic_rate_limit_error() {
        let error = AnthropicClientError::from_anthropic_error(AnthropicError {
            error_type: "rate_limit_error".to_string(),
            message: "Number of request tokens has exceeded your daily rate limit".to_string(),
        });

        // Verify error properties
        assert_eq!(error.error_code(), Code::TooManySearchRequests);
        assert!(error.is_retryable());

        // Verify stream error event generation
        let event = error.into_stream_error_event();
        assert_eq!(event.r#type, "error");
        assert_eq!(event.error.r#type, "rate_limit_error");
        assert_eq!(event.error.code, Some("rate_limit_error".to_string()));
    }

    /// Test authentication error handling
    #[test]
    fn test_anthropic_auth_error() {
        let error = AnthropicClientError::from_anthropic_error(AnthropicError {
            error_type: "authentication_error".to_string(),
            message: "Invalid API Key".to_string(),
        });

        // Verify error properties
        assert_eq!(error.error_code(), Code::InvalidChatCompletionApiKey);
        assert!(!error.is_retryable());

        // Verify message sanitization
        if let AnthropicClientError::Api { message, .. } = &error {
            assert!(!message.contains("API Key"));
            assert!(message.contains("Invalid or missing"));
        }
    }

    /// Test invalid request error handling
    #[test]
    fn test_anthropic_invalid_request_error() {
        let error = AnthropicClientError::from_anthropic_error(AnthropicError {
            error_type: "invalid_request_error".to_string(),
            message: "max_tokens: value must be a positive integer".to_string(),
        });

        assert_eq!(error.error_code(), Code::BadRequest);
        assert!(!error.is_retryable());

        // Safe messages should pass through
        if let AnthropicClientError::Api { message, .. } = &error {
            assert_eq!(message, "max_tokens: value must be a positive integer");
        }
    }

    /// Test response conversion with multiple content blocks (text + tool_use)
    #[test]
    fn test_anthropic_response_mixed_content() {
        let response = AnthropicResponse {
            id: "msg_mixed".to_string(),
            response_type: "message".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            content: vec![
                ContentBlock::Text { text: "I found some results. Let me search again.".to_string() },
                ContentBlock::ToolUse {
                    id: "toolu_search1".to_string(),
                    name: "_meiliSearchInIndex".to_string(),
                    input: serde_json::json!({"index_uid": "products", "q": "laptop"}),
                },
                ContentBlock::ToolUse {
                    id: "toolu_search2".to_string(),
                    name: "_meiliSearchInIndex".to_string(),
                    input: serde_json::json!({"index_uid": "reviews", "q": "laptop review"}),
                },
            ],
            stop_reason: Some("tool_use".to_string()),
            usage: Some(AnthropicUsage { input_tokens: 100, output_tokens: 75 }),
        };

        let openai_response = convert_response_to_openai(response);

        // Verify text content
        assert_eq!(
            openai_response.choices[0].message.content.as_deref(),
            Some("I found some results. Let me search again.")
        );

        // Verify tool calls
        let tool_calls = openai_response.choices[0].message.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 2);

        assert_eq!(tool_calls[0].id, "toolu_search1");
        assert_eq!(tool_calls[0].function.name, "_meiliSearchInIndex");
        assert!(tool_calls[0].function.arguments.contains("laptop"));

        assert_eq!(tool_calls[1].id, "toolu_search2");
        assert!(tool_calls[1].function.arguments.contains("review"));

        // Verify finish reason
        assert_eq!(openai_response.choices[0].finish_reason, Some(FinishReason::ToolCalls));
    }

    /// Test that stream state resets correctly between messages
    #[test]
    fn test_anthropic_stream_state_reset() {
        let mut state = StreamState::new();

        // Simulate first message with tool use
        let event1 = AnthropicStreamEvent::MessageStart {
            message: MessageStartData {
                id: "msg_1".to_string(),
                model: "claude-sonnet-4".to_string(),
                usage: None,
            },
        };
        convert_sse_event_to_openai(&event1, &mut state).unwrap();

        // Add tool state
        let event2 = AnthropicStreamEvent::ContentBlockStart {
            index: 0,
            content_block: ContentBlockStartData::ToolUse {
                id: "tool_1".to_string(),
                name: "test_tool".to_string(),
            },
        };
        convert_sse_event_to_openai(&event2, &mut state).unwrap();

        assert_eq!(state.tool_call_index, 1);
        assert!(!state.tool_state.is_empty());
        assert_eq!(state.message_id, "msg_1");

        // Simulate new message (should reset state)
        let event3 = AnthropicStreamEvent::MessageStart {
            message: MessageStartData {
                id: "msg_2".to_string(),
                model: "claude-sonnet-4".to_string(),
                usage: None,
            },
        };
        convert_sse_event_to_openai(&event3, &mut state).unwrap();

        // State should be reset
        assert_eq!(state.tool_call_index, 0);
        assert!(state.tool_state.is_empty());
        assert_eq!(state.message_id, "msg_2");
    }

    /// Test usage tracking in streaming responses
    #[test]
    fn test_anthropic_streaming_usage_tracking() {
        let mut state = StreamState::new();
        state.message_id = "msg_usage".to_string();
        state.model = "claude-sonnet-4".to_string();

        // message_delta with usage
        let event = AnthropicStreamEvent::MessageDelta {
            delta: MessageDeltaData { stop_reason: Some("end_turn".to_string()) },
            usage: Some(AnthropicUsage { input_tokens: 150, output_tokens: 50 }),
        };

        let chunk = convert_sse_event_to_openai(&event, &mut state).unwrap().unwrap();

        // Verify usage is included in the chunk
        let usage = chunk.usage.as_ref().unwrap();
        assert_eq!(usage.prompt_tokens, 150);
        assert_eq!(usage.completion_tokens, 50);
        assert_eq!(usage.total_tokens, 200);

        // Verify state also tracks usage
        let state_usage = state.usage.as_ref().unwrap();
        assert_eq!(state_usage.prompt_tokens, 150);
        assert_eq!(state_usage.completion_tokens, 50);
    }
}
