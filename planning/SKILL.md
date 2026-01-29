---
name: meilisearch-anthropic-integrator
description: Procedural skill for completing Anthropic Claude API integration in Meilisearch's Rust chat infrastructure
version: 1.0.0
author: claude-code
created: 2026-01-29
tags:
  - rust
  - api-integration
  - anthropic
  - meilisearch
  - streaming
---

# Meilisearch Anthropic Integrator

> "The bridge between two worlds: OpenAI's ubiquity and Anthropic's capability."

A procedural skill encoding HOW to complete Anthropic integration—not just reference material, but a step-by-step process Claude can follow reliably.

---

## When to Use This Skill

Use this skill when:
- Implementing OpenAI → Anthropic request conversion
- Implementing Anthropic → OpenAI response conversion
- Building SSE streaming with tool state tracking
- Wiring `anthropic.rs` into `chat_completions.rs`
- Completing tasks from `./Tasks.md`
- Debugging Anthropic-specific conversion issues

## When NOT to Use This Skill

Do not use this skill for:
- OpenAI/Azure/Mistral provider work (use existing patterns)
- General Meilisearch development (wrong domain)
- Prompt engineering or model selection (not in scope)
- Frontend/UI changes (wrong layer)
- Database schema changes (wrong module)

---

## Core Procedure

### Phase 1: Preparation

1. **Load context**
   ```
   Read: ./Tasks.md (current work items)
   Read: ./constitution.md (inviolable rules)
   Read: ./CHECKLIST.md (progress tracking)
   ```

2. **Verify environment**
   ```bash
   cargo build -p meilisearch  # Confirms baseline builds
   ```

3. **Identify current task** from Tasks.md

### Phase 2: Implement

1. **Read before writing**
   - Open the target file(s)
   - Find similar patterns in codebase
   - Understand existing structure

2. **Implement minimal change**
   - One function at a time
   - Match existing style exactly
   - No extras, no ceremony

3. **Add test immediately**
   - Write test before moving on
   - Test the specific conversion/behavior

### Phase 3: Verify (VIGIL)

This phase is **constitutional**—NEVER skip.

```bash
cargo build -p meilisearch           # Must pass
cargo clippy -p meilisearch -- -D warnings  # Must pass (warnings are failures)
cargo test -p meilisearch anthropic  # Must pass
```

**Task is NOT complete until all three pass.**

### Phase 4: Update Tracking

1. Mark task complete in `./Tasks.md`
2. Update `./CHECKLIST.md`
3. Document any patterns learned

---

## Key Patterns (Load on Demand)

### HTTP Client (Constitutional: IR-003)

```rust
// CORRECT: Use http_client wrapper with policies
let http_client = http_client::reqwest::Client::builder()
    .build_with_policies(ip_policy, http_client::reqwest::redirect::Policy::default())
    .expect("Failed to build HTTP client");

// CORRECT: Use .prepare() for request building
let response = http_client
    .post(&url)
    .prepare(|rb| {
        rb.header("x-api-key", &self.config.api_key)
          .header("anthropic-version", &self.config.anthropic_version)
          .header(CONTENT_TYPE, "application/json")
          .json(&request)
    })
    .send()
    .await?;

// NEVER: Raw reqwest bypasses IP policy (IR-003 violation)
```

### async_openai Types

```rust
use async_openai::types::{
    ChatChoice, ChatCompletionMessageToolCall,
    CreateChatCompletionRequest, CreateChatCompletionResponse,
    FinishReason, Role,
};
use async_openai::reqwest_eventsource::{Event, EventSource, RequestBuilderExt};
```

### Tool Result Placement (Critical)

```rust
// Tool results MUST be in user message, not standalone
AnthropicMessage {
    role: "user".to_string(),
    content: AnthropicContent::Blocks(vec![
        ContentBlock::ToolResult {
            tool_use_id: id.clone(),
            content: result_json,
            is_error: None,
        }
    ]),
}
```

---

## Conversion Reference

**Full reference:** `./Design.md` §Conversion Tables

### Quick Lookup: Messages

| OpenAI Type | Anthropic Handling |
|-------------|-------------------|
| `System(content)` | Extract to top-level `system` field |
| `Developer(content)` | Merge with system prompt |
| `User(content)` | `{role: "user", content: "..."}` |
| `Assistant(content)` | `{role: "assistant", content: "..."}` |
| `Assistant(tool_calls)` | `{role: "assistant", content: [{type: "tool_use", ...}]}` |
| `Tool(id, content)` | Merge as `tool_result` block into user message |

### Quick Lookup: Stop Reasons

| Anthropic | OpenAI |
|-----------|--------|
| `end_turn` | `stop` |
| `stop_sequence` | `stop` |
| `tool_use` | `tool_calls` |
| `max_tokens` | `length` |

### Quick Lookup: SSE Events

| Anthropic Event | Action |
|-----------------|--------|
| `message_start` | Emit chunk with `role: "assistant"` |
| `content_block_start(tool_use)` | Emit chunk with `tool_calls[idx].id`, `name` |
| `content_block_delta(text)` | Emit chunk with `delta.content` |
| `content_block_delta(input_json)` | Emit chunk with `tool_calls[idx].function.arguments` |
| `message_delta` | Emit chunk with `finish_reason`, `usage` |
| `message_stop` | Close stream |

---

## Key Files

| File | Purpose |
|------|---------|
| `crates/meilisearch/src/routes/chats/anthropic.rs` | Anthropic client, types, conversion |
| `crates/meilisearch/src/routes/chats/chat_completions.rs` | Main handler, conversation loop |
| `crates/meilisearch/src/routes/chats/config.rs` | Provider config |
| `crates/meilisearch-types/src/features.rs` | ChatCompletionSource enum |

---

## Common Pitfalls

| Pitfall | Symptom | Fix |
|---------|---------|-----|
| Missing `max_tokens` | Anthropic rejects request | Default to 4096 |
| Tool result in wrong message | Anthropic rejects | Put in user message, not standalone |
| Stop reason mapping | Wrong finish_reason | Both `end_turn` and `stop_sequence` → `stop` |
| Inconsistent chunk IDs | Client confusion | Same ID for ALL chunks in stream |
| Raw reqwest usage | IP policy bypassed | Use `http_client` wrapper |

---

## Integration Points

This skill integrates with:
- **constitution.md** — Inviolable rules checked before every action
- **Tasks.md** — Work items and dependency graph
- **CHECKLIST.md** — Progress tracking
- **Design.md** — Full technical architecture

---

## Error Handling

| Situation | Response |
|-----------|----------|
| VIGIL fails (build/clippy/test) | Fix before proceeding—task is NOT complete |
| Type mismatch | Read actual type definition, don't guess |
| Unknown pattern | Search codebase for examples |
| Blocked by dependency | Surface blocker clearly, don't proceed |

**Exit codes for vigil.sh:**
```
0 = All checks pass (proceed)
1 = Build failed
2 = Clippy failed (warnings are failures)
3 = Tests failed
```

---

## Resources

| Resource | Content |
|----------|---------|
| `./Design.md` | Full technical architecture, conversion tables |
| `./Tasks.md` | Implementation work items |
| `./Requirements.md` | EARS-format requirements |
| `./constitution.md` | Inviolable rules |
| [Anthropic Messages API](https://docs.anthropic.com/en/api/messages) | Official API docs |
| [Anthropic Streaming](https://docs.anthropic.com/en/api/messages-streaming) | SSE event reference |
