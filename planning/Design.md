# Anthropic API Integration - Technical Design

## Document Information

| Field | Value |
|-------|-------|
| Feature | Anthropic Claude API Integration |
| Version | 1.0.0 |
| Status | Draft |
| Author | Claude Code |
| Created | 2026-01-29 |
| Implements | Requirements.md v1.0.0 |

## 1. Architecture Overview

### 1.1 High-Level Design

```
┌─────────────────────────────────────────────────────────────────────┐
│                        Chat Completions Endpoint                      │
│                     POST /chats/{workspace}/chat/completions          │
└─────────────────────────────────────┬───────────────────────────────┘
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────┐
│                         Source Router                                 │
│            Determines provider based on workspace settings            │
└──────┬──────────────┬──────────────┬──────────────┬────────────────┘
       │              │              │              │
       ▼              ▼              ▼              ▼
┌──────────┐   ┌──────────┐   ┌──────────┐   ┌──────────────┐
│  OpenAI  │   │  Azure   │   │ Mistral/ │   │  Anthropic   │
│  Client  │   │  Client  │   │  vLLM    │   │   Client     │
│(async_   │   │(async_   │   │(async_   │   │ (custom)     │
│ openai)  │   │ openai)  │   │ openai)  │   │              │
└──────────┘   └──────────┘   └──────────┘   └──────┬───────┘
                                                    │
                                                    ▼
                              ┌─────────────────────────────────────┐
                              │     Request/Response Converter      │
                              │   OpenAI ←→ Anthropic Translation   │
                              └─────────────────────────────────────┘
```

### 1.2 Component Diagram

```
src/routes/chats/
├── mod.rs                    # Module registration, constants
├── chat_completions.rs       # Main handler, conversation loop
├── anthropic.rs              # Anthropic-specific implementation
│   ├── AnthropicConfig       # Configuration struct
│   ├── AnthropicClient       # HTTP client wrapper
│   ├── Request types         # Anthropic API request structures
│   ├── Response types        # Anthropic API response structures
│   ├── Converter             # OpenAI ↔ Anthropic conversion
│   └── AnthropicStream       # SSE stream wrapper
├── config.rs                 # Provider configuration (OpenAI/Azure)
├── errors.rs                 # Error types and mapping
├── settings.rs               # Settings endpoints
└── utils.rs                  # Shared utilities
```

## 2. Component Specifications

### 2.1 AnthropicConfig

**Purpose**: Hold Anthropic API configuration.

```rust
#[derive(Debug, Clone)]
pub struct AnthropicConfig {
    pub api_key: String,
    pub base_url: String,
    pub anthropic_version: String,
}

impl AnthropicConfig {
    pub fn new(api_key: String) -> Self;
    pub fn with_base_url(self, base_url: String) -> Self;
    pub fn with_version(self, version: String) -> Self;

    /// Create from ChatCompletionSettings
    pub fn from_settings(settings: &ChatCompletionSettings) -> Result<Self, ResponseError>;
}
```

**Design Rationale**: Separate from `config.rs::Config` because Anthropic doesn't use `async_openai` and requires different headers (x-api-key, anthropic-version).

### 2.2 AnthropicClient

**Purpose**: HTTP client for Anthropic Messages API.

```rust
pub struct AnthropicClient {
    config: AnthropicConfig,
    http_client: http_client::reqwest::Client,
}

impl AnthropicClient {
    pub fn new(config: AnthropicConfig, ip_policy: IpPolicy) -> Self;

    /// Send non-streaming request
    pub async fn create_message(
        &self,
        request: AnthropicRequest,
    ) -> Result<AnthropicResponse, AnthropicClientError>;

    /// Create streaming request, return EventSource
    pub async fn create_message_stream(
        &self,
        request: AnthropicRequest,
    ) -> Result<EventSource, AnthropicClientError>;
}
```

**Design Rationale**:
- Uses `http_client::reqwest` for IP policy enforcement
- Separate methods for streaming vs non-streaming
- Returns raw types, conversion happens at higher level

### 2.3 Request Conversion

**Purpose**: Convert OpenAI request format to Anthropic format.

```rust
pub fn convert_request(
    openai_request: &CreateChatCompletionRequest,
) -> Result<AnthropicRequest, ConversionError>;
```

**Conversion Logic**:

| OpenAI Field | Anthropic Field | Notes |
|--------------|-----------------|-------|
| `model` | `model` | Direct passthrough |
| `messages` | `messages` + `system` | System extracted |
| `max_tokens` | `max_tokens` | Required, default 4096 |
| `temperature` | `temperature` | Optional |
| `top_p` | `top_p` | Optional |
| `stop` | `stop_sequences` | Array conversion |
| `tools` | `tools` | Format conversion |
| `stream` | `stream` | Direct passthrough |

**Message Conversion Rules**:

```
┌─────────────────────────────────────────────────────────────────┐
│ OpenAI Messages                  │ Anthropic Format             │
├─────────────────────────────────────────────────────────────────┤
│ System("prompt")                 │ system: "prompt"             │
│ Developer("prompt")              │ system: "prompt" (merged)    │
│ User("content")                  │ {role:"user", content:"..."}│
│ Assistant("content")             │ {role:"assistant",...}       │
│ Assistant(tool_calls:[...])      │ {role:"assistant", content:  │
│                                  │   [{type:"tool_use",...}]}   │
│ Tool(id, content)                │ Merged into prior user msg   │
│                                  │ as tool_result block         │
└─────────────────────────────────────────────────────────────────┘
```

### 2.4 Response Conversion

**Purpose**: Convert Anthropic responses to OpenAI format.

#### 2.4.1 Non-Streaming Converter

```rust
pub fn convert_response(
    anthropic_response: AnthropicResponse,
    model: &str,
) -> CreateChatCompletionResponse;
```

**Field Mapping**:

| Anthropic | OpenAI | Transformation |
|-----------|--------|----------------|
| `id` | `id` | Prefix with `chatcmpl-` |
| `model` | `model` | Direct |
| `content[text]` | `choices[0].message.content` | Extract text blocks |
| `content[tool_use]` | `choices[0].message.tool_calls` | Convert format |
| `stop_reason` | `choices[0].finish_reason` | Map enum |
| `usage` | `usage` | Map tokens |

#### 2.4.2 Streaming Converter (AnthropicStream)

```rust
pub struct AnthropicStream {
    inner: EventSource,
    state: Arc<Mutex<StreamState>>,
    model: String,
    response_id: String,
}

struct StreamState {
    tool_calls: HashMap<u32, ToolCallState>,
    current_block_index: Option<u32>,
    first_chunk_sent: bool,
}

struct ToolCallState {
    id: String,
    name: String,
    arguments: String,
}

impl Stream for AnthropicStream {
    type Item = Result<CreateChatCompletionStreamResponse, StreamError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>>;
}
```

**SSE Event Mapping**:

```
┌────────────────────────────────────────────────────────────────────┐
│ Anthropic SSE Event          │ OpenAI Stream Chunk                 │
├────────────────────────────────────────────────────────────────────┤
│ message_start                │ Initial chunk with role: "assistant"│
│ content_block_start(text)    │ (internal state update)             │
│ content_block_start(tool)    │ Chunk with tool_call[idx].id, name  │
│ content_block_delta(text)    │ Chunk with delta.content            │
│ content_block_delta(tool)    │ Chunk with tool_call[idx].arguments │
│ content_block_stop           │ (finalize tool call state)          │
│ message_delta                │ Chunk with finish_reason, usage     │
│ message_stop                 │ [DONE] marker (stream end)          │
│ error                        │ Error event                         │
└────────────────────────────────────────────────────────────────────┘
```

### 2.5 Tool State Tracker

**Purpose**: Track partial tool call state during streaming.

```rust
pub struct ToolStateTracker {
    /// Map of content block index to tool call state
    tool_calls: HashMap<u32, ToolCallState>,
    /// Current tool call index for OpenAI format
    current_index: usize,
}

impl ToolStateTracker {
    pub fn new() -> Self;

    /// Handle content_block_start for tool_use
    pub fn start_tool(&mut self, index: u32, id: String, name: String);

    /// Handle content_block_delta for input_json
    pub fn append_arguments(&mut self, index: u32, json_delta: &str);

    /// Finalize tool call and return OpenAI format
    pub fn finish_tool(&mut self, index: u32) -> Option<ChatCompletionMessageToolCallChunk>;
}
```

### 2.6 Error Types

```rust
#[derive(Debug, thiserror::Error)]
pub enum AnthropicClientError {
    #[error("HTTP request failed: {0}")]
    Request(#[from] http_client::reqwest::Error),

    #[error("Failed to parse response: {0}")]
    Parse(#[from] serde_json::Error),

    #[error("Anthropic API error: {error_type} - {message}")]
    Api { error_type: String, message: String },

    #[error("Stream error: {0}")]
    Stream(String),
}

impl From<AnthropicClientError> for ResponseError {
    fn from(err: AnthropicClientError) -> Self {
        match err {
            AnthropicClientError::Api { error_type, .. } => {
                match error_type.as_str() {
                    "authentication_error" => ResponseError::from_msg(..., Code::Unauthorized),
                    "rate_limit_error" => ResponseError::from_msg(..., Code::TooManyRequests),
                    // ... other mappings
                }
            }
            _ => ResponseError::from_msg(..., Code::InternalError),
        }
    }
}
```

## 3. Integration Points

### 3.1 chat_completions.rs Modifications

```rust
// In chat() handler - add source routing
async fn chat(...) -> impl Responder {
    let chat_settings = index_scheduler.chat_settings(&workspace_uid)?;

    match chat_settings.source {
        ChatCompletionSource::Anthropic => {
            // Route to Anthropic-specific handler
            if chat_completion.stream.unwrap_or(false) {
                anthropic_streamed_chat(...).await
            } else {
                anthropic_non_streamed_chat(...).await
            }
        }
        _ => {
            // Existing OpenAI-compatible flow
            if chat_completion.stream.unwrap_or(false) {
                streamed_chat(...).await
            } else {
                non_streamed_chat(...).await
            }
        }
    }
}
```

### 3.2 Conversation Loop Integration

The existing `run_conversation` function handles the tool-calling loop. For Anthropic:

```rust
async fn run_anthropic_conversation(
    client: &AnthropicClient,
    mut request: AnthropicRequest,
    search_handler: impl Fn(SearchParams) -> SearchResult,
    event_sender: SseEventSender,
) -> Result<(), ResponseError> {
    loop {
        let response = if streaming {
            process_anthropic_stream(client.create_message_stream(request).await?, ...)
        } else {
            client.create_message(request).await?
        };

        // Check for tool use
        let tool_uses: Vec<_> = response.content
            .iter()
            .filter_map(|c| match c {
                ContentBlock::ToolUse { id, name, input } => Some((id, name, input)),
                _ => None
            })
            .collect();

        if tool_uses.is_empty() {
            break; // No more tool calls, we're done
        }

        // Execute tool calls and prepare next request
        let mut tool_results = Vec::new();
        for (id, name, input) in tool_uses {
            if name == MEILI_SEARCH_IN_INDEX_FUNCTION_NAME {
                let result = search_handler(parse_search_params(input)?);
                tool_results.push(ContentBlock::ToolResult {
                    tool_use_id: id.clone(),
                    content: serde_json::to_string(&result)?,
                    is_error: None,
                });
            }
        }

        // Append assistant response and tool results
        request.messages.push(AnthropicMessage {
            role: "assistant".to_string(),
            content: AnthropicContent::Blocks(response.content),
        });
        request.messages.push(AnthropicMessage {
            role: "user".to_string(),
            content: AnthropicContent::Blocks(tool_results),
        });
    }

    Ok(())
}
```

## 4. Data Flow

### 4.1 Non-Streaming Flow

```
1. Request arrives (OpenAI format)
   │
2. Source routing → Anthropic
   │
3. convert_request() → AnthropicRequest
   │
4. AnthropicClient.create_message()
   │                                    ┌──────────────────┐
   ├────────────────────────────────────│ Anthropic API    │
   │                                    └────────┬─────────┘
5. AnthropicResponse                             │
   │◄────────────────────────────────────────────┘
6. Check for tool_use blocks
   │
   ├─── If tool_use: Execute search, loop back to step 3
   │
7. convert_response() → CreateChatCompletionResponse
   │
8. Return JSON response
```

### 4.2 Streaming Flow

```
1. Request arrives (OpenAI format, stream=true)
   │
2. Source routing → Anthropic
   │
3. convert_request() → AnthropicRequest (stream=true)
   │
4. AnthropicClient.create_message_stream()
   │                                    ┌──────────────────┐
   ├────────────────────────────────────│ Anthropic API    │
   │                                    │   (SSE stream)   │
   │                                    └────────┬─────────┘
5. EventSource                                   │
   │◄───── SSE events ───────────────────────────┘
   │
6. AnthropicStream wrapper
   │
   ├─── message_start ────────────► Emit initial chunk
   ├─── content_block_delta ──────► Emit content/tool chunks
   ├─── message_delta ────────────► Emit finish_reason chunk
   │
7. If stop_reason == "tool_use":
   │   └─── Execute search
   │   └─── Add results to messages
   │   └─── Start new stream (loop)
   │
8. message_stop → Close stream
```

## 5. Configuration Schema

### 5.1 Settings API

```json
{
  "source": "anthropic",
  "apiKey": "sk-ant-api03-...",
  "baseUrl": "https://api.anthropic.com/v1/",  // optional
  "prompts": {
    "system": "...",
    "searchDescription": "...",
    "searchQParam": "...",
    "searchFilterParam": "...",
    "searchIndexUidParam": "..."
  }
}
```

### 5.2 Validation Rules

| Field | Required | Validation |
|-------|----------|------------|
| `source` | Yes | Must be valid enum value |
| `apiKey` | Yes (Anthropic) | Non-empty string |
| `baseUrl` | No | Valid URL if provided |
| `prompts.*` | No | Strings, use defaults |

## 6. Testing Strategy

### 6.1 Unit Tests

- `test_convert_simple_message` - Basic message conversion
- `test_convert_system_extraction` - System prompt handling
- `test_convert_tool_definitions` - Tool format conversion
- `test_convert_tool_results` - Tool result merging
- `test_stop_reason_mapping` - All stop reason cases
- `test_stream_state_tracking` - Tool state accumulation

### 6.2 Integration Tests

- `test_anthropic_non_streaming` - Full request/response cycle
- `test_anthropic_streaming` - SSE stream processing
- `test_anthropic_tool_calling` - Search tool invocation
- `test_anthropic_multi_turn` - Conversation continuation
- `test_anthropic_error_handling` - API error scenarios

### 6.3 Mock Strategy

```rust
// Mock Anthropic API for testing
struct MockAnthropicServer {
    responses: Vec<AnthropicResponse>,
    stream_events: Vec<AnthropicStreamEvent>,
}

impl MockAnthropicServer {
    fn start() -> (Self, String); // Returns mock server URL
}
```

## 7. Performance Considerations

### 7.1 Memory Management

- Stream processing uses incremental parsing
- Tool state uses bounded HashMap (max 50 concurrent tools)
- Response IDs generated lazily

### 7.2 Connection Management

- Reuse HTTP client across requests
- Connection pooling via `http_client::reqwest`
- Timeouts configured per-request

## 8. Security Considerations

### 8.1 API Key Handling

- Never log API keys
- Mask in settings responses
- Pass via headers, not URL

### 8.2 Network Security

- IP policy enforced via `http_client`
- HTTPS required for production
- No following redirects to untrusted hosts

## 9. Future Enhancements

1. **Vision Support**: Handle image content blocks
2. **Extended Thinking**: Support Claude's thinking blocks
3. **Caching**: Prompt caching for repeated system prompts
4. **Batch API**: Non-real-time batch processing
5. **Computer Use**: Tool support for computer control
