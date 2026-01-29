# Anthropic API Integration - Implementation Tasks

## Document Information

| Field | Value |
|-------|-------|
| Feature | Anthropic Claude API Integration |
| Version | 1.0.0 |
| Status | Draft |
| Author | Claude Code |
| Created | 2026-01-29 |
| Implements | Design.md v1.0.0 |
| Traces To | Requirements.md v1.0.0 |

## Overview

This document defines the implementation tasks for integrating Anthropic's Claude API into Meilisearch's chat completions feature. Tasks are organized in phases with explicit dependencies.

## Phase 1: Foundation (Core Types & Client)

### T-001: Finalize AnthropicConfig

**Priority**: P0
**Estimated Effort**: Small
**Traces To**: FR-CFG-001, FR-CFG-002
**File**: `src/routes/chats/anthropic.rs`

**Description**:
- Add `from_settings()` constructor that extracts config from `ChatCompletionSettings`
- Handle base_url default to `https://api.anthropic.com/v1/`
- Validate API key is present for Anthropic source

**Acceptance Criteria**:
- [ ] `AnthropicConfig::from_settings()` returns `Result<Self, ResponseError>`
- [ ] Missing API key returns `Code::BadRequest` error
- [ ] Default base_url is applied when not specified
- [ ] Unit test: `test_config_from_settings`

---

### T-002: Complete Request Conversion

**Priority**: P0
**Estimated Effort**: Medium
**Traces To**: FR-REQ-001, FR-REQ-002, FR-REQ-003, FR-REQ-004
**File**: `src/routes/chats/anthropic.rs`
**Depends On**: None

**Description**:
Implement `convert_request()` function that transforms OpenAI `CreateChatCompletionRequest` to `AnthropicRequest`.

**Subtasks**:
1. Extract system messages to top-level `system` field
2. Merge developer messages with system prompt
3. Convert user/assistant messages to Anthropic format
4. Handle tool messages by merging as `tool_result` blocks into prior user message
5. Convert tool definitions from OpenAI to Anthropic format
6. Map optional parameters (temperature, top_p, stop → stop_sequences)
7. Default max_tokens to 4096 if not specified
8. Silently ignore unsupported parameters

**Acceptance Criteria**:
- [ ] System messages extracted correctly
- [ ] Tool messages merged into user messages as tool_result
- [ ] Assistant tool_calls converted to tool_use blocks
- [ ] Tool definitions correctly formatted with input_schema
- [ ] Unit tests for each message type conversion

---

### T-003: Implement Non-Streaming Response Conversion

**Priority**: P0
**Estimated Effort**: Medium
**Traces To**: FR-RES-001, FR-RES-002
**File**: `src/routes/chats/anthropic.rs`
**Depends On**: None

**Description**:
Implement `convert_response()` function that transforms `AnthropicResponse` to `CreateChatCompletionResponse`.

**Subtasks**:
1. Generate response ID with `chatcmpl-` prefix
2. Extract text content from content blocks
3. Convert tool_use blocks to `tool_calls` array
4. Map stop_reason to FinishReason enum
5. Convert usage tokens
6. Set object type and created timestamp

**Acceptance Criteria**:
- [ ] Response ID format matches `chatcmpl-{uuid}`
- [ ] Text blocks concatenated to content field
- [ ] Tool uses converted to ChatCompletionMessageToolCall format
- [ ] All stop_reason values mapped correctly
- [ ] Unit test: `test_response_conversion`

---

### T-004: Implement Streaming Response Wrapper

**Priority**: P0
**Estimated Effort**: Large
**Traces To**: FR-RES-003, FR-TOOL-001
**File**: `src/routes/chats/anthropic.rs`
**Depends On**: T-003

**Description**:
Complete the `AnthropicStream` implementation that wraps EventSource and produces OpenAI-format stream chunks.

**Subtasks**:
1. Implement `Stream` trait for `AnthropicStream`
2. Handle `message_start` → emit initial chunk with role
3. Handle `content_block_start` for text and tool_use
4. Handle `content_block_delta` → emit content/arguments chunks
5. Handle `content_block_stop` → finalize tool calls
6. Handle `message_delta` → emit finish_reason chunk
7. Handle `message_stop` → close stream
8. Implement `ToolStateTracker` for accumulating tool state

**Acceptance Criteria**:
- [ ] Stream produces valid OpenAI-format chunks
- [ ] Tool calls accumulated correctly across deltas
- [ ] Finish reason emitted with final chunk
- [ ] Stream terminates cleanly
- [ ] Integration test with mock SSE server

---

### T-005: Implement Error Handling

**Priority**: P0
**Estimated Effort**: Small
**Traces To**: FR-ERR-001, FR-ERR-002, FR-ERR-003
**File**: `src/routes/chats/anthropic.rs`
**Depends On**: None

**Description**:
Implement error type mappings from Anthropic errors to Meilisearch ResponseError.

**Subtasks**:
1. Define `AnthropicClientError` enum with thiserror
2. Implement `From<AnthropicClientError> for ResponseError`
3. Map authentication_error → Unauthorized
4. Map rate_limit_error → TooManyRequests
5. Map overloaded_error → ServiceUnavailable
6. Map other errors → InternalError
7. Handle streaming errors gracefully

**Acceptance Criteria**:
- [ ] All error types have appropriate mappings
- [ ] Error messages are descriptive but don't leak internals
- [ ] Unit test for each error mapping

---

## Phase 2: Integration

### T-006: Add Source Routing in chat_completions.rs

**Priority**: P0
**Estimated Effort**: Medium
**Traces To**: FR-INT-001
**File**: `src/routes/chats/chat_completions.rs`
**Depends On**: T-001, T-002, T-003, T-004, T-005

**Description**:
Modify the `chat()` handler to route Anthropic requests to the new client.

**Subtasks**:
1. Remove early-return error for Anthropic source
2. Add match arm in `chat()` to detect Anthropic source
3. Create `anthropic_streamed_chat()` function
4. Create `anthropic_non_streamed_chat()` function
5. Extract common setup logic (tool definition, system prompt injection)
6. Wire up AnthropicClient with IP policy from index_scheduler

**Acceptance Criteria**:
- [ ] Anthropic requests no longer return "not yet integrated" error
- [ ] Requests properly route based on source
- [ ] Common setup logic shared between providers
- [ ] IP policy enforced on Anthropic requests

---

### T-007: Implement Anthropic Conversation Loop

**Priority**: P0
**Estimated Effort**: Large
**Traces To**: FR-TOOL-002, FR-TOOL-003, FR-INT-003
**File**: `src/routes/chats/chat_completions.rs`
**Depends On**: T-006

**Description**:
Implement the tool-calling conversation loop for Anthropic, similar to existing `run_conversation()`.

**Subtasks**:
1. Create `run_anthropic_conversation()` function
2. Handle streaming tool calls with state accumulation
3. Execute `_meiliSearchInIndex` tool calls
4. Format tool results as `tool_result` content blocks
5. Append assistant response and tool results to messages
6. Loop until no more tool calls
7. Emit progress/sources via internal functions if requested

**Acceptance Criteria**:
- [ ] Search tool is called when LLM requests it
- [ ] Search results formatted and returned to LLM
- [ ] Multi-turn tool calling works correctly
- [ ] Progress and sources reported to frontend
- [ ] Integration test with mock LLM

---

### T-008: Update Settings Validation

**Priority**: P1
**Estimated Effort**: Small
**Traces To**: FR-CFG-001, FR-CFG-003
**File**: `src/features.rs` (meilisearch-types)
**Depends On**: None

**Description**:
Ensure settings validation handles Anthropic correctly.

**Subtasks**:
1. Verify Anthropic doesn't require baseUrl (has default)
2. Verify API key validation for Anthropic source
3. Update `validate()` method if needed
4. Ensure `hide_secrets()` masks Anthropic API keys

**Acceptance Criteria**:
- [ ] Settings validation passes with valid Anthropic config
- [ ] Settings validation fails without API key
- [ ] API keys are masked in responses

---

## Phase 3: Polish & Testing

### T-009: Add Comprehensive Unit Tests

**Priority**: P1
**Estimated Effort**: Medium
**File**: `src/routes/chats/anthropic.rs` (inline tests)
**Depends On**: T-001 through T-005

**Description**:
Add unit tests for all conversion and error handling logic.

**Test Cases**:
- [ ] `test_convert_simple_user_message`
- [ ] `test_convert_system_message_extraction`
- [ ] `test_convert_developer_message_merge`
- [ ] `test_convert_assistant_with_tool_calls`
- [ ] `test_convert_tool_result_message`
- [ ] `test_convert_tool_definitions`
- [ ] `test_convert_response_with_text`
- [ ] `test_convert_response_with_tool_use`
- [ ] `test_stop_reason_mapping_all_cases`
- [ ] `test_error_mapping`

---

### T-010: Add Integration Tests

**Priority**: P1
**Estimated Effort**: Large
**File**: `tests/` or integration test module
**Depends On**: T-007

**Description**:
Add integration tests using mock Anthropic server.

**Test Cases**:
- [ ] `test_anthropic_basic_chat` - Simple request/response
- [ ] `test_anthropic_streaming` - SSE stream handling
- [ ] `test_anthropic_tool_call` - Single tool call cycle
- [ ] `test_anthropic_multi_tool` - Multiple tool calls
- [ ] `test_anthropic_multi_turn` - Conversation with history
- [ ] `test_anthropic_error_handling` - API error scenarios
- [ ] `test_anthropic_rate_limit` - Rate limit response

---

### T-011: Update Documentation

**Priority**: P2
**Estimated Effort**: Small
**Depends On**: T-007

**Description**:
Update user-facing documentation for Anthropic support.

**Subtasks**:
- [ ] Add Anthropic to chat completions settings docs
- [ ] Document required configuration fields
- [ ] Add example configuration
- [ ] Document supported models
- [ ] Note any differences from OpenAI behavior

---

### T-012: Add Metrics Integration

**Priority**: P2
**Estimated Effort**: Small
**Traces To**: FR-INT-003
**File**: `src/routes/chats/chat_completions.rs`
**Depends On**: T-007

**Description**:
Ensure Anthropic requests are tracked in metrics.

**Subtasks**:
- [ ] Increment token counters from Anthropic usage
- [ ] Track search calls via existing metrics
- [ ] Add source label to metrics if not present

**Acceptance Criteria**:
- [ ] `MEILISEARCH_CHAT_TOKENS_TOTAL` incremented
- [ ] `MEILISEARCH_CHAT_SEARCHES_TOTAL` incremented for tool calls

---

## Phase 4: Future Enhancements (Backlog)

### T-100: Vision/Image Support

**Priority**: P3
**Status**: Backlog

Support image content blocks in messages for multimodal Claude models.

---

### T-101: Extended Thinking

**Priority**: P3
**Status**: Backlog

Support Claude's extended thinking feature with thinking blocks.

---

### T-102: Prompt Caching

**Priority**: P3
**Status**: Backlog

Implement Anthropic's prompt caching for repeated system prompts.

---

## Task Dependencies Graph

```
T-001 ──┐
        │
T-002 ──┼──► T-006 ──► T-007 ──► T-010
        │              │
T-003 ──┤              └──► T-012
        │
T-004 ──┤
        │
T-005 ──┘

T-008 ─────────────────────────► (independent)

T-009 ◄── T-001..T-005

T-011 ◄── T-007
```

## Execution Order

1. **Parallel**: T-001, T-002, T-003, T-005, T-008 (no dependencies)
2. **Sequential**: T-004 (depends on T-003)
3. **Sequential**: T-006 (depends on T-001 through T-005)
4. **Sequential**: T-007 (depends on T-006)
5. **Parallel**: T-009, T-010, T-011, T-012 (after T-007)

## Estimated Total Effort

| Phase | Tasks | Effort |
|-------|-------|--------|
| Phase 1 | T-001 to T-005 | ~3-4 days |
| Phase 2 | T-006 to T-008 | ~2-3 days |
| Phase 3 | T-009 to T-012 | ~2-3 days |
| **Total** | | **~7-10 days** |

## Risk Factors

1. **Streaming complexity**: Tool state tracking during SSE may have edge cases
2. **API version changes**: Anthropic API may evolve
3. **Test infrastructure**: May need mock server setup
4. **Edge cases**: Multi-tool, error-during-stream scenarios
