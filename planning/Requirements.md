# Anthropic API Integration Requirements

## Document Information

| Field | Value |
|-------|-------|
| Feature | Anthropic Claude API Integration for Chat Completions |
| Version | 1.0.0 |
| Status | Draft |
| Author | Claude Code |
| Created | 2026-01-29 |

## 1. Overview

### 1.1 Purpose

Enable Meilisearch's chat completions feature to use Anthropic's Claude models as an alternative to OpenAI, Azure OpenAI, Mistral, and vLLM providers while maintaining full compatibility with the existing OpenAI-style API interface.

### 1.2 Scope

This specification covers:
- Anthropic Messages API client implementation
- Request/response format conversion between OpenAI and Anthropic formats
- Streaming SSE support with tool state tracking
- Integration with existing chat_completions infrastructure
- Error handling and mapping

Out of scope:
- Vision/image content blocks (future enhancement)
- Anthropic-specific extended thinking features
- Direct Anthropic API endpoint exposure

### 1.3 Stakeholders

| Stakeholder | Interest |
|-------------|----------|
| Meilisearch Users | Use Claude models for RAG-powered search chat |
| Meilisearch Team | Maintain provider-agnostic chat infrastructure |
| API Consumers | Consistent OpenAI-compatible interface |

## 2. Functional Requirements

### 2.1 Configuration Requirements

#### FR-CFG-001: Anthropic Source Selection
**WHEN** a user configures chat completion settings with `source: "anthropic"`
**THEN** the system SHALL accept and store the configuration
**AND** the system SHALL validate that an API key is provided

#### FR-CFG-002: Base URL Configuration
**WHEN** `source` is `anthropic` AND no `baseUrl` is provided
**THEN** the system SHALL use `https://api.anthropic.com/v1/` as the default base URL

#### FR-CFG-003: API Key Handling
**WHEN** displaying chat completion settings for Anthropic source
**THEN** the system SHALL mask the API key using the existing `hide_secrets()` mechanism

### 2.2 Request Conversion Requirements

#### FR-REQ-001: OpenAI to Anthropic Message Conversion
**WHEN** a chat completion request is received with Anthropic source
**THEN** the system SHALL convert OpenAI `ChatCompletionRequestMessage` to Anthropic `AnthropicMessage` format:
- `system` messages → extracted to top-level `system` field
- `developer` messages → merged with system prompt
- `user` messages → `role: "user"` with content blocks
- `assistant` messages → `role: "assistant"` with content/tool_use blocks
- `tool` messages → merged into preceding user message as `tool_result` blocks

#### FR-REQ-002: Tool Definition Conversion
**WHEN** OpenAI-format tools are provided in the request
**THEN** the system SHALL convert `ChatCompletionTool` to `AnthropicTool`:
- `function.name` → `name`
- `function.description` → `description`
- `function.parameters` → `input_schema`

#### FR-REQ-003: Model Parameter Mapping
**WHEN** converting request parameters
**THEN** the system SHALL map:
- `model` → `model` (direct passthrough)
- `max_tokens` → `max_tokens` (required, default to 4096 if not provided)
- `temperature` → `temperature`
- `top_p` → `top_p`
- `stop` → `stop_sequences`
- `stream` → `stream`

#### FR-REQ-004: Unsupported Parameter Handling
**WHEN** OpenAI-specific parameters are provided (e.g., `frequency_penalty`, `presence_penalty`, `logit_bias`)
**THEN** the system SHALL silently ignore these parameters
**AND** the system SHALL NOT return an error

### 2.3 Response Conversion Requirements

#### FR-RES-001: Non-Streaming Response Conversion
**WHEN** a non-streaming response is received from Anthropic
**THEN** the system SHALL convert `AnthropicResponse` to `CreateChatCompletionResponse`:
- Generate unique `id` with `chatcmpl-` prefix
- Set `object` to `"chat.completion"`
- Map `content` blocks to `choices[0].message.content`
- Map `tool_use` blocks to `choices[0].message.tool_calls`
- Map `stop_reason` to appropriate `FinishReason`
- Map `usage` tokens to `CompletionUsage`

#### FR-RES-002: Stop Reason Mapping
**WHEN** converting Anthropic stop reasons
**THEN** the system SHALL map:
- `"end_turn"` → `FinishReason::Stop`
- `"stop_sequence"` → `FinishReason::Stop`
- `"tool_use"` → `FinishReason::ToolCalls`
- `"max_tokens"` → `FinishReason::Length`
- `null` or unknown → `FinishReason::Stop`

#### FR-RES-003: Streaming Response Conversion
**WHEN** streaming is enabled
**THEN** the system SHALL convert Anthropic SSE events to OpenAI `CreateChatCompletionStreamResponse`:
- `message_start` → initial chunk with role
- `content_block_start` → tool call index initialization
- `content_block_delta` → incremental content/tool updates
- `content_block_stop` → tool call completion
- `message_delta` → finish reason and usage
- `message_stop` → stream termination

### 2.4 Tool Calling Requirements

#### FR-TOOL-001: Tool State Tracking
**WHEN** processing streaming tool use blocks
**THEN** the system SHALL maintain state for each tool call:
- Track `index` to `tool_id` mapping
- Accumulate partial `input` JSON strings
- Track `name` for each tool call

#### FR-TOOL-002: Meilisearch Search Tool Integration
**WHEN** the LLM calls `_meiliSearchInIndex`
**THEN** the system SHALL:
- Execute the search against the specified index
- Return results as `tool_result` content block
- Support the existing progress/sources reporting functions

#### FR-TOOL-003: Tool Result Formatting
**WHEN** returning tool results to Anthropic
**THEN** the system SHALL format as:
```json
{
  "type": "tool_result",
  "tool_use_id": "<original_tool_use_id>",
  "content": "<serialized_result>"
}
```

### 2.5 Error Handling Requirements

#### FR-ERR-001: API Error Mapping
**WHEN** Anthropic returns an error response
**THEN** the system SHALL map to appropriate `ResponseError`:
- `authentication_error` → `Code::Unauthorized`
- `invalid_request_error` → `Code::BadRequest`
- `rate_limit_error` → `Code::TooManyRequests`
- `overloaded_error` → `Code::ServiceUnavailable`
- Other errors → `Code::InternalError`

#### FR-ERR-002: Streaming Error Handling
**WHEN** an error occurs during SSE streaming
**THEN** the system SHALL emit a `StreamErrorEvent` compatible with the existing error handling
**AND** the system SHALL close the stream gracefully

#### FR-ERR-003: Connection Error Handling
**WHEN** a network error occurs communicating with Anthropic
**THEN** the system SHALL return a descriptive error message
**AND** the system SHALL NOT expose internal connection details

### 2.6 Integration Requirements

#### FR-INT-001: Source Routing
**WHEN** a chat completion request arrives
**AND** the workspace is configured with `source: "anthropic"`
**THEN** the system SHALL route the request to the Anthropic client
**AND** the system SHALL NOT use the `async_openai` Client

#### FR-INT-002: IP Policy Enforcement
**WHEN** making requests to Anthropic API
**THEN** the system SHALL use the `http_client::reqwest` wrapper
**AND** the system SHALL apply the configured IP policy

#### FR-INT-003: Analytics Integration
**WHEN** a chat completion request completes
**THEN** the system SHALL report metrics via `ChatCompletionAggregator`
**AND** the system SHALL increment token counters

## 3. Non-Functional Requirements

### 3.1 Performance Requirements

#### NFR-PERF-001: Streaming Latency
**WHERE** streaming is enabled
**THE SYSTEM** SHALL emit the first token within 100ms of receiving it from Anthropic

#### NFR-PERF-002: Memory Efficiency
**WHERE** processing streaming responses
**THE SYSTEM** SHALL NOT buffer the entire response in memory
**AND** SHALL process events incrementally

### 3.2 Security Requirements

#### NFR-SEC-001: API Key Protection
**THE SYSTEM** SHALL never log API keys in plain text
**AND** SHALL mask API keys in all user-visible outputs

#### NFR-SEC-002: Network Isolation
**THE SYSTEM** SHALL respect the configured IP policy
**AND** SHALL prevent requests to disallowed network ranges

### 3.3 Reliability Requirements

#### NFR-REL-001: Graceful Degradation
**IF** Anthropic API is unavailable
**THEN** the system SHALL return a clear error message
**AND** SHALL NOT affect other chat sources

### 3.4 Compatibility Requirements

#### NFR-COMPAT-001: API Compatibility
**THE SYSTEM** SHALL accept requests in OpenAI chat completion format
**AND** SHALL return responses in OpenAI chat completion format
**AND** SHALL maintain compatibility with existing SDK clients

## 4. Acceptance Criteria

### AC-001: Basic Chat Completion
- [ ] Configure workspace with `source: "anthropic"` and valid API key
- [ ] Send non-streaming chat request
- [ ] Receive valid OpenAI-format response with content

### AC-002: Streaming Chat Completion
- [ ] Configure workspace with Anthropic source
- [ ] Send streaming chat request
- [ ] Receive SSE stream with OpenAI-format chunks
- [ ] Stream terminates with finish reason

### AC-003: Tool Calling
- [ ] LLM receives Meilisearch search tool definition
- [ ] LLM can call `_meiliSearchInIndex`
- [ ] Search results are returned to LLM
- [ ] LLM incorporates results in response

### AC-004: Error Handling
- [ ] Invalid API key returns authentication error
- [ ] Rate limit returns appropriate error
- [ ] Network error returns descriptive message

### AC-005: Multi-turn Conversation
- [ ] Tool calls and results are properly formatted
- [ ] Conversation history is maintained
- [ ] Follow-up questions work correctly

## 5. Dependencies

| Dependency | Version | Purpose |
|------------|---------|---------|
| `http_client` | workspace | HTTP requests with IP policy |
| `async_openai` | 0.x | OpenAI types and SSE utilities |
| `serde` | 1.x | JSON serialization |
| `futures` | 0.3.x | Stream processing |
| `tokio` | 1.x | Async runtime |
| `time` | 0.3.x | Timestamp generation |

## 6. Traceability Matrix

| Requirement | Design Section | Task |
|-------------|----------------|------|
| FR-CFG-001 | AnthropicConfig | T-001 |
| FR-REQ-001 | convert_request() | T-002 |
| FR-RES-001 | AnthropicResponseConverter | T-003 |
| FR-RES-003 | AnthropicStream | T-004 |
| FR-TOOL-001 | ToolStateTracker | T-005 |
| FR-INT-001 | run_conversation() | T-006 |
| FR-ERR-001 | Error mapping | T-007 |
