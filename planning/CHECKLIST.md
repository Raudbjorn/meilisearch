# Implementation Checklist

## Pre-Implementation

- [ ] Read `planning/Requirements.md` to understand scope
- [ ] Read `planning/Design.md` to understand architecture
- [ ] Read `planning/Tasks.md` to identify current task
- [ ] Load `planning/SKILL.md` for patterns
- [ ] Verify build succeeds: `cargo build -p meilisearch`

## Phase 1: Foundation

### T-001: Finalize AnthropicConfig
- [x] Add `from_settings()` constructor
- [x] Handle default base_url
- [x] Validate API key presence
- [x] Write `test_config_from_settings`
- [x] Verify: `cargo test -p meilisearch config`

### T-002: Complete Request Conversion
- [x] Extract system messages
- [x] Merge developer messages
- [x] Convert user messages
- [x] Convert assistant messages
- [x] Convert tool messages to tool_result
- [x] Convert tool definitions
- [x] Map optional parameters
- [x] Default max_tokens
- [x] Write unit tests
- [x] Verify: `cargo test -p meilisearch convert_request`

### T-003: Implement Response Conversion
- [x] Generate chatcmpl- ID
- [x] Extract text content
- [x] Convert tool_use blocks
- [x] Map stop_reason
- [x] Convert usage tokens
- [x] Write unit tests
- [x] Verify: `cargo test -p meilisearch convert_response`

### T-004: Implement Streaming
- [x] Implement Stream trait
- [x] Handle message_start
- [x] Handle content_block_start
- [x] Handle content_block_delta
- [x] Handle content_block_stop
- [x] Handle message_delta
- [x] Handle message_stop
- [x] Implement ToolStateTracker
- [x] Write unit tests
- [x] Verify: `cargo test -p meilisearch stream`

### T-005: Implement Error Handling
- [x] Define AnthropicClientError
- [x] Implement From<> for ResponseError
- [x] Map authentication_error
- [x] Map rate_limit_error
- [x] Map overloaded_error
- [x] Map other errors
- [x] Write unit tests
- [x] Verify: `cargo test -p meilisearch error`

## Phase 1 Gate
- [x] All Phase 1 tests pass (34 tests)
- [x] No clippy warnings
- [x] Code formatted

## Phase 2: Integration

### T-006: Add Source Routing
- [x] Remove early-return error for Anthropic
- [x] Add match arm in chat() - integrated into streamed_chat()
- [x] Create anthropic_streamed_chat() - merged into streamed_chat() with source routing
- [ ] Create anthropic_non_streamed_chat() - DEFERRED (streaming only for now)
- [x] Extract common setup logic
- [x] Wire up AnthropicClient
- [x] Verify other sources still work
- [x] Verify: `cargo build -p meilisearch`

### T-007: Implement Conversation Loop
- [x] Create run_anthropic_conversation()
- [x] Handle streaming tool calls
- [x] Execute search tool calls (via handle_meili_tools)
- [x] Format tool results
- [x] Append to messages
- [x] Loop until complete
- [x] Emit progress/sources
- [ ] Write integration tests (DEFERRED - requires mock server)
- [x] Verify: `cargo test -p meilisearch anthropic`

### T-008: Update Settings Validation
- [x] Verify Anthropic doesn't require baseUrl (has default)
- [x] Verify API key validation
- [x] Update validate() if needed
- [x] Verify hide_secrets() works
- [x] Verify: `cargo test -p meilisearch-types features`

## Phase 2 Gate
- [x] All Phase 2 tests pass (34 anthropic tests)
- [ ] Integration tests pass (DEFERRED - requires mock server)
- [x] Other sources verified working (OpenAI/Mistral paths unchanged)

## Phase 3: Polish

### T-009: Add Unit Tests
- [x] test_convert_simple_request (existing)
- [x] test_convert_developer_message_merged_with_system
- [x] test_convert_assistant_message_with_tool_calls
- [x] test_convert_tool_message_to_tool_result
- [x] test_convert_tool_definitions
- [x] test_response_conversion
- [x] test_convert_finish_reason
- [x] test_tool_state_tracking
- [x] test_error_* (17 error tests)
- [x] test_config_from_settings

### T-010: Add Integration Tests
- [ ] test_anthropic_basic_chat
- [ ] test_anthropic_streaming
- [ ] test_anthropic_tool_call
- [ ] test_anthropic_multi_tool
- [ ] test_anthropic_multi_turn
- [ ] test_anthropic_error_handling
- [ ] test_anthropic_rate_limit

### T-011: Update Documentation
- [ ] Add to chat completions docs
- [ ] Document configuration
- [ ] Add example config
- [ ] Document supported models
- [ ] Note behavioral differences

### T-012: Add Metrics
- [ ] Increment token counters
- [ ] Track search calls
- [ ] Add source label if needed

## Final Gate

- [ ] All tests pass: `cargo test -p meilisearch`
- [ ] No warnings: `cargo clippy -p meilisearch -- -D warnings`
- [ ] Formatted: `cargo fmt -p meilisearch -- --check`
- [ ] Documentation updated
- [ ] Ready for review

## Post-Implementation

- [ ] Manual testing with real API key
- [ ] Test streaming in browser
- [ ] Test tool calling flow
- [ ] Test error scenarios
- [ ] Performance check
