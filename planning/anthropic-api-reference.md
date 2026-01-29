# Anthropic Messages API Reference

## API Endpoint

```
POST https://api.anthropic.com/v1/messages
```

## Required Headers

| Header | Value |
|--------|-------|
| `x-api-key` | `sk-ant-api03-...` |
| `anthropic-version` | `2023-06-01` (stable) |
| `content-type` | `application/json` |

## Request Format

```json
{
  "model": "claude-sonnet-4-20250514",
  "max_tokens": 4096,
  "messages": [
    {"role": "user", "content": "Hello"}
  ],
  "system": "You are a helpful assistant",
  "stream": false,
  "temperature": 0.7,
  "top_p": 0.9,
  "top_k": 40,
  "stop_sequences": ["END"],
  "tools": [
    {
      "name": "get_weather",
      "description": "Get weather for a location",
      "input_schema": {
        "type": "object",
        "properties": {
          "location": {"type": "string"}
        },
        "required": ["location"]
      }
    }
  ],
  "tool_choice": {"type": "auto"}
}
```

## Message Roles

| Role | Usage |
|------|-------|
| `user` | User messages, including tool results |
| `assistant` | Model responses, including tool use |

**Note**: System prompt is a separate top-level field, not a message.

## Content Block Types

### Text Content

```json
{"type": "text", "text": "Hello, world!"}
```

### Tool Use (in assistant message)

```json
{
  "type": "tool_use",
  "id": "toolu_01XFDUDYJgAACzvnptvVoYEL",
  "name": "get_weather",
  "input": {"location": "San Francisco"}
}
```

### Tool Result (in user message)

```json
{
  "type": "tool_result",
  "tool_use_id": "toolu_01XFDUDYJgAACzvnptvVoYEL",
  "content": "72°F, sunny"
}
```

## Response Format (Non-Streaming)

```json
{
  "id": "msg_01XFDUDYJgAACzvnptvVoYEL",
  "type": "message",
  "role": "assistant",
  "model": "claude-sonnet-4-20250514",
  "content": [
    {"type": "text", "text": "Hello!"}
  ],
  "stop_reason": "end_turn",
  "stop_sequence": null,
  "usage": {
    "input_tokens": 10,
    "output_tokens": 5
  }
}
```

## Stop Reasons

| Value | Meaning |
|-------|---------|
| `end_turn` | Model finished naturally |
| `stop_sequence` | Hit a stop sequence |
| `tool_use` | Model wants to use a tool |
| `max_tokens` | Hit token limit |

## Streaming (SSE)

### Enable Streaming

```json
{"stream": true, ...}
```

### Event Types

#### message_start

```json
{
  "type": "message_start",
  "message": {
    "id": "msg_...",
    "type": "message",
    "role": "assistant",
    "model": "claude-sonnet-4-20250514",
    "content": [],
    "stop_reason": null,
    "stop_sequence": null,
    "usage": {"input_tokens": 25, "output_tokens": 1}
  }
}
```

#### content_block_start (text)

```json
{
  "type": "content_block_start",
  "index": 0,
  "content_block": {"type": "text", "text": ""}
}
```

#### content_block_start (tool_use)

```json
{
  "type": "content_block_start",
  "index": 0,
  "content_block": {
    "type": "tool_use",
    "id": "toolu_...",
    "name": "get_weather",
    "input": {}
  }
}
```

#### content_block_delta (text)

```json
{
  "type": "content_block_delta",
  "index": 0,
  "delta": {"type": "text_delta", "text": "Hello"}
}
```

#### content_block_delta (tool input)

```json
{
  "type": "content_block_delta",
  "index": 0,
  "delta": {
    "type": "input_json_delta",
    "partial_json": "{\"location\": \"San"
  }
}
```

#### content_block_stop

```json
{
  "type": "content_block_stop",
  "index": 0
}
```

#### message_delta

```json
{
  "type": "message_delta",
  "delta": {
    "stop_reason": "end_turn",
    "stop_sequence": null
  },
  "usage": {"output_tokens": 15}
}
```

#### message_stop

```json
{"type": "message_stop"}
```

#### error

```json
{
  "type": "error",
  "error": {
    "type": "overloaded_error",
    "message": "Overloaded"
  }
}
```

## Error Responses

### HTTP Error Format

```json
{
  "type": "error",
  "error": {
    "type": "authentication_error",
    "message": "Invalid API key"
  }
}
```

### Error Types

| Type | HTTP Status | Meaning |
|------|-------------|---------|
| `authentication_error` | 401 | Invalid API key |
| `invalid_request_error` | 400 | Malformed request |
| `rate_limit_error` | 429 | Rate limited |
| `overloaded_error` | 529 | Server overloaded |
| `api_error` | 500 | Internal error |

## Tool Choice Options

```json
// Auto (default) - model decides
{"tool_choice": {"type": "auto"}}

// Force specific tool
{"tool_choice": {"type": "tool", "name": "get_weather"}}

// Force any tool
{"tool_choice": {"type": "any"}}

// Disable tools
{"tool_choice": {"type": "none"}}
```

## Rate Limits

| Tier | Requests/min | Tokens/min |
|------|--------------|------------|
| Free | 5 | 20,000 |
| Build | 50 | 80,000 |
| Scale | 500 | 800,000 |

## Models

| Model ID | Context | Notes |
|----------|---------|-------|
| `claude-opus-4-5-20251101` | 200K | Most capable (Opus 4.5) |
| `claude-sonnet-4-5-20250929` | 200K | Latest balanced (Sonnet 4.5) |
| `claude-haiku-4-5-20251001` | 200K | Fastest (Haiku 4.5) |

> Note: Claude 3.5 models (claude-3-5-sonnet-20241022, claude-3-5-haiku-20241022) are deprecated and will be retired.

## Differences from OpenAI

| Feature | OpenAI | Anthropic |
|---------|--------|-----------|
| System prompt | Message with role "system" | Top-level `system` field |
| Tool results | Separate message role "tool" | Content block in "user" message |
| Function calling | `function_call` (deprecated) | N/A (use tools) |
| max_tokens | Optional | Required |
| Streaming chunks | Choice delta objects | Content block deltas |
| Stop reasons | `stop`, `tool_calls`, `length` | `end_turn`, `tool_use`, `max_tokens` |
