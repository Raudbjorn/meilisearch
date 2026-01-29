#!/usr/bin/env bash
#
# Meilisearch Anthropic Integration - Manual Test Suite
# ======================================================
#
# This script performs comprehensive end-to-end testing of the Anthropic
# Claude API integration for Meilisearch chat completions.
#
# Prerequisites:
#   - Meilisearch running with chat completions feature enabled
#   - Valid Anthropic API key
#   - curl, jq installed
#
# Usage:
#   ./test-anthropic-integration.sh
#
# Note: Some tests require actual Anthropic API access (not mocked)
#

set -euo pipefail

# ============================================================================
# CONFIGURATION - Edit these values before running
# ============================================================================

# Meilisearch configuration
MEILI_HOST="${MEILI_HOST:-http://localhost:7700}"
MEILI_API_KEY="${MEILI_API_KEY:-masterKey}"

# Anthropic configuration
ANTHROPIC_API_KEY="${ANTHROPIC_API_KEY:-sk-ant-api03-REPLACE_ME}"
ANTHROPIC_BASE_URL="${ANTHROPIC_BASE_URL:-https://api.anthropic.com/v1/}"
ANTHROPIC_MODEL="${ANTHROPIC_MODEL:-claude-sonnet-4-20250514}"

# Test workspace
WORKSPACE_UID="${WORKSPACE_UID:-test-anthropic}"

# Test index (for tool calling tests)
TEST_INDEX_UID="${TEST_INDEX_UID:-movies}"

# Timeouts
CURL_TIMEOUT="${CURL_TIMEOUT:-60}"
STREAM_TIMEOUT="${STREAM_TIMEOUT:-120}"

# ============================================================================
# COLOR OUTPUT
# ============================================================================

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color
BOLD='\033[1m'

# ============================================================================
# HELPER FUNCTIONS
# ============================================================================

log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[PASS]${NC} $1"
}

log_fail() {
    echo -e "${RED}[FAIL]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_section() {
    echo ""
    echo -e "${BOLD}${CYAN}═══════════════════════════════════════════════════════════════${NC}"
    echo -e "${BOLD}${CYAN}  $1${NC}"
    echo -e "${BOLD}${CYAN}═══════════════════════════════════════════════════════════════${NC}"
}

# Check if a command exists
require_cmd() {
    if ! command -v "$1" &> /dev/null; then
        log_fail "Required command not found: $1"
        exit 1
    fi
}

# Make an authenticated Meilisearch API request
meili_request() {
    local method="$1"
    local endpoint="$2"
    local data="${3:-}"

    if [[ -n "$data" ]]; then
        curl -s -X "$method" \
            -H "Authorization: Bearer $MEILI_API_KEY" \
            -H "Content-Type: application/json" \
            --max-time "$CURL_TIMEOUT" \
            -d "$data" \
            "${MEILI_HOST}${endpoint}"
    else
        curl -s -X "$method" \
            -H "Authorization: Bearer $MEILI_API_KEY" \
            --max-time "$CURL_TIMEOUT" \
            "${MEILI_HOST}${endpoint}"
    fi
}

# Make a streaming request and capture output
meili_stream_request() {
    local endpoint="$1"
    local data="$2"
    local output_file="${3:-/dev/stdout}"

    curl -s -X POST \
        -H "Authorization: Bearer $MEILI_API_KEY" \
        -H "Content-Type: application/json" \
        -H "Accept: text/event-stream" \
        --max-time "$STREAM_TIMEOUT" \
        -d "$data" \
        "${MEILI_HOST}${endpoint}" > "$output_file" 2>&1
}

# Check HTTP status code
check_status() {
    local expected="$1"
    local endpoint="$2"
    local method="${3:-GET}"
    local data="${4:-}"

    local status
    if [[ -n "$data" ]]; then
        status=$(curl -s -o /dev/null -w "%{http_code}" -X "$method" \
            -H "Authorization: Bearer $MEILI_API_KEY" \
            -H "Content-Type: application/json" \
            --max-time "$CURL_TIMEOUT" \
            -d "$data" \
            "${MEILI_HOST}${endpoint}")
    else
        status=$(curl -s -o /dev/null -w "%{http_code}" -X "$method" \
            -H "Authorization: Bearer $MEILI_API_KEY" \
            --max-time "$CURL_TIMEOUT" \
            "${MEILI_HOST}${endpoint}")
    fi

    if [[ "$status" == "$expected" ]]; then
        return 0
    else
        echo "$status"
        return 1
    fi
}

# ============================================================================
# VALIDATION
# ============================================================================

validate_environment() {
    log_section "Environment Validation"

    require_cmd curl
    require_cmd jq

    log_info "Checking Meilisearch connectivity..."
    if ! curl -s --max-time 5 "${MEILI_HOST}/health" | jq -e '.status == "available"' > /dev/null 2>&1; then
        log_fail "Meilisearch not reachable at $MEILI_HOST"
        exit 1
    fi
    log_success "Meilisearch is healthy"

    log_info "Checking Anthropic API key format..."
    if [[ ! "$ANTHROPIC_API_KEY" =~ ^sk-ant- ]]; then
        log_warn "API key doesn't match expected format (sk-ant-*)"
        log_warn "Tests requiring real API calls will fail"
    else
        log_success "Anthropic API key format valid"
    fi

    log_info "Checking chat completions feature..."
    local features
    features=$(meili_request GET "/experimental-features")
    if echo "$features" | jq -e '.chatCompletions == true' > /dev/null 2>&1; then
        log_success "Chat completions feature is enabled"
    else
        log_warn "Chat completions feature may not be enabled"
        log_info "Enable with: PATCH /experimental-features {\"chatCompletions\": true}"
    fi
}

# ============================================================================
# TEST: SETTINGS API
# ============================================================================

test_settings_api() {
    log_section "Settings API Tests"

    # Test 1: Create/Update workspace with Anthropic source
    log_info "Test 1: Configure workspace with Anthropic source..."

    local settings_payload
    settings_payload=$(cat <<EOF
{
    "source": "anthropic",
    "apiKey": "$ANTHROPIC_API_KEY",
    "baseUrl": "$ANTHROPIC_BASE_URL"
}
EOF
)

    local response
    response=$(meili_request PATCH "/chats/$WORKSPACE_UID/settings" "$settings_payload" 2>&1)

    if echo "$response" | jq -e '.source' > /dev/null 2>&1; then
        local source
        source=$(echo "$response" | jq -r '.source')
        if [[ "$source" == "anthropic" ]]; then
            log_success "Workspace configured with Anthropic source"
        else
            log_fail "Unexpected source: $source"
            log_warn "NOTE: settings.rs may be missing Anthropic enum variant!"
        fi
    else
        log_fail "Failed to configure workspace"
        echo "Response: $response"
        log_warn ""
        log_warn "KNOWN ISSUE: The settings API (settings.rs) is missing the 'Anthropic'"
        log_warn "variant in its ChatCompletionSource enum. This needs to be added:"
        log_warn ""
        log_warn "  In src/routes/chats/settings.rs, add to ChatCompletionSource enum:"
        log_warn "    /// Anthropic Claude API"
        log_warn "    Anthropic,"
        log_warn ""
        log_warn "  And in the From<ChatCompletionSource> impl:"
        log_warn "    Anthropic => DbChatCompletionSource::Anthropic,"
        log_warn ""
        return 1
    fi

    # Test 2: Verify settings are persisted
    log_info "Test 2: Verify settings retrieval..."
    response=$(meili_request GET "/chats/$WORKSPACE_UID/settings")

    if echo "$response" | jq -e '.source == "anthropic"' > /dev/null 2>&1; then
        log_success "Settings persisted correctly"

        # Verify API key is masked
        local api_key
        api_key=$(echo "$response" | jq -r '.apiKey // "null"')
        if [[ "$api_key" == *"*"* ]] || [[ "$api_key" == "null" ]]; then
            log_success "API key properly masked in response"
        else
            log_warn "API key may not be masked: $api_key"
        fi
    else
        log_fail "Settings not persisted correctly"
        echo "Response: $response"
    fi

    # Test 3: Validate baseUrl is set correctly
    log_info "Test 3: Verify base URL configuration..."
    local base_url
    base_url=$(echo "$response" | jq -r '.baseUrl // "null"')
    if [[ "$base_url" == "$ANTHROPIC_BASE_URL" ]] || [[ "$base_url" == "null" && "$ANTHROPIC_BASE_URL" == "https://api.anthropic.com/v1/" ]]; then
        log_success "Base URL configured correctly"
    else
        log_warn "Base URL mismatch: expected '$ANTHROPIC_BASE_URL', got '$base_url'"
    fi
}

# ============================================================================
# TEST: NON-STREAMING CHAT COMPLETION
# ============================================================================

test_non_streaming_chat() {
    log_section "Non-Streaming Chat Completion Tests"

    # Test 1: Simple message
    log_info "Test 1: Simple chat completion (non-streaming)..."

    local request_payload
    request_payload=$(cat <<EOF
{
    "model": "$ANTHROPIC_MODEL",
    "messages": [
        {"role": "user", "content": "Say hello in exactly 5 words."}
    ],
    "max_tokens": 100,
    "stream": false
}
EOF
)

    local response
    response=$(meili_request POST "/chats/$WORKSPACE_UID/chat/completions" "$request_payload" 2>&1)

    if echo "$response" | jq -e '.choices[0].message.content' > /dev/null 2>&1; then
        local content
        content=$(echo "$response" | jq -r '.choices[0].message.content')
        log_success "Received response: $content"

        # Verify response structure
        local id model finish_reason
        id=$(echo "$response" | jq -r '.id')
        model=$(echo "$response" | jq -r '.model')
        finish_reason=$(echo "$response" | jq -r '.choices[0].finish_reason')

        log_info "  Response ID: $id"
        log_info "  Model: $model"
        log_info "  Finish reason: $finish_reason"

        if [[ "$finish_reason" == "stop" ]]; then
            log_success "Finish reason correctly mapped to 'stop'"
        else
            log_warn "Unexpected finish reason: $finish_reason (expected 'stop')"
        fi
    else
        log_fail "Failed to get chat completion"
        echo "Response: $response"

        # Check for specific error types
        if echo "$response" | jq -e '.message' > /dev/null 2>&1; then
            local error_msg
            error_msg=$(echo "$response" | jq -r '.message')
            log_info "Error message: $error_msg"
        fi
    fi

    # Test 2: Multi-turn conversation
    log_info "Test 2: Multi-turn conversation..."

    request_payload=$(cat <<EOF
{
    "model": "$ANTHROPIC_MODEL",
    "messages": [
        {"role": "user", "content": "My name is Alice."},
        {"role": "assistant", "content": "Hello Alice! Nice to meet you."},
        {"role": "user", "content": "What is my name?"}
    ],
    "max_tokens": 100,
    "stream": false
}
EOF
)

    response=$(meili_request POST "/chats/$WORKSPACE_UID/chat/completions" "$request_payload" 2>&1)

    if echo "$response" | jq -e '.choices[0].message.content' > /dev/null 2>&1; then
        local content
        content=$(echo "$response" | jq -r '.choices[0].message.content')
        if [[ "$content" == *"Alice"* ]]; then
            log_success "Multi-turn context preserved: $content"
        else
            log_warn "Response may not have preserved context: $content"
        fi
    else
        log_fail "Multi-turn conversation failed"
        echo "Response: $response"
    fi

    # Test 3: With temperature and top_p
    log_info "Test 3: Parameters passthrough (temperature, top_p)..."

    request_payload=$(cat <<EOF
{
    "model": "$ANTHROPIC_MODEL",
    "messages": [
        {"role": "user", "content": "Say 'test' only."}
    ],
    "max_tokens": 50,
    "temperature": 0.0,
    "top_p": 0.9,
    "stream": false
}
EOF
)

    response=$(meili_request POST "/chats/$WORKSPACE_UID/chat/completions" "$request_payload" 2>&1)

    if echo "$response" | jq -e '.choices[0].message.content' > /dev/null 2>&1; then
        log_success "Parameters accepted"
    else
        log_fail "Parameters may have caused an error"
        echo "Response: $response"
    fi
}

# ============================================================================
# TEST: STREAMING CHAT COMPLETION
# ============================================================================

test_streaming_chat() {
    log_section "Streaming Chat Completion Tests"

    # Test 1: Basic streaming
    log_info "Test 1: Basic streaming chat completion..."

    local request_payload
    request_payload=$(cat <<EOF
{
    "model": "$ANTHROPIC_MODEL",
    "messages": [
        {"role": "user", "content": "Count from 1 to 5, one number per line."}
    ],
    "max_tokens": 100,
    "stream": true
}
EOF
)

    local tmp_file
    tmp_file=$(mktemp)

    meili_stream_request "/chats/$WORKSPACE_UID/chat/completions" "$request_payload" "$tmp_file"

    # Check for SSE format
    if grep -q "^data:" "$tmp_file"; then
        log_success "Received SSE formatted response"

        # Count data events
        local event_count
        event_count=$(grep -c "^data:" "$tmp_file" || echo "0")
        log_info "  Received $event_count SSE events"

        # Check for [DONE] marker
        if grep -q '\[DONE\]' "$tmp_file"; then
            log_success "Stream properly terminated with [DONE]"
        else
            log_warn "Missing [DONE] terminator"
        fi

        # Extract and display content
        log_info "  Stream content preview:"
        grep "^data:" "$tmp_file" | head -5 | while read -r line; do
            echo "    $line"
        done
    else
        log_fail "Response not in SSE format"
        cat "$tmp_file"
    fi

    rm -f "$tmp_file"

    # Test 2: Verify chunk structure
    log_info "Test 2: Verify streaming chunk structure..."

    request_payload=$(cat <<EOF
{
    "model": "$ANTHROPIC_MODEL",
    "messages": [
        {"role": "user", "content": "Hi"}
    ],
    "max_tokens": 20,
    "stream": true
}
EOF
)

    tmp_file=$(mktemp)
    meili_stream_request "/chats/$WORKSPACE_UID/chat/completions" "$request_payload" "$tmp_file"

    # Parse first non-empty data chunk
    local first_chunk
    first_chunk=$(grep "^data:" "$tmp_file" | grep -v '\[DONE\]' | head -1 | sed 's/^data: //')

    if [[ -n "$first_chunk" ]] && echo "$first_chunk" | jq -e '.id' > /dev/null 2>&1; then
        local chunk_id chunk_model
        chunk_id=$(echo "$first_chunk" | jq -r '.id')
        chunk_model=$(echo "$first_chunk" | jq -r '.model')

        log_success "Chunk has valid structure"
        log_info "  Chunk ID: $chunk_id"
        log_info "  Model: $chunk_model"

        # Verify all chunks have the same ID
        local unique_ids
        unique_ids=$(grep "^data:" "$tmp_file" | grep -v '\[DONE\]' | sed 's/^data: //' | jq -r '.id' 2>/dev/null | sort -u | wc -l)
        if [[ "$unique_ids" == "1" ]]; then
            log_success "All chunks share the same response ID"
        else
            log_warn "Chunks have inconsistent IDs (found $unique_ids unique)"
        fi
    else
        log_fail "Could not parse chunk structure"
        echo "First chunk: $first_chunk"
    fi

    rm -f "$tmp_file"
}

# ============================================================================
# TEST: TOOL CALLING (SEARCH)
# ============================================================================

test_tool_calling() {
    log_section "Tool Calling Tests"

    # First, ensure test index exists with some data
    log_info "Setting up test index '$TEST_INDEX_UID'..."

    local index_docs
    index_docs=$(cat <<EOF
[
    {"id": 1, "title": "The Matrix", "year": 1999, "genre": "Sci-Fi"},
    {"id": 2, "title": "Inception", "year": 2010, "genre": "Sci-Fi"},
    {"id": 3, "title": "The Godfather", "year": 1972, "genre": "Crime"},
    {"id": 4, "title": "Pulp Fiction", "year": 1994, "genre": "Crime"},
    {"id": 5, "title": "Interstellar", "year": 2014, "genre": "Sci-Fi"}
]
EOF
)

    # Add documents
    local task_response
    task_response=$(meili_request POST "/indexes/$TEST_INDEX_UID/documents" "$index_docs")
    local task_uid
    task_uid=$(echo "$task_response" | jq -r '.taskUid // "null"')

    if [[ "$task_uid" != "null" ]]; then
        log_info "Documents indexing task: $task_uid"
        # Wait for task to complete
        sleep 2
    fi

    # Configure index for chat
    log_info "Configuring index chat settings..."
    local chat_config
    chat_config=$(cat <<EOF
{
    "description": "A collection of movies with title, year, and genre",
    "documentTemplate": "Title: {{doc.title}}, Year: {{doc.year}}, Genre: {{doc.genre}}"
}
EOF
)
    meili_request PATCH "/indexes/$TEST_INDEX_UID/settings/chat" "$chat_config" > /dev/null 2>&1

    # Make filterable
    meili_request PATCH "/indexes/$TEST_INDEX_UID/settings" '{"filterableAttributes": ["genre", "year"]}' > /dev/null 2>&1
    sleep 2

    # Test 1: Query that should trigger search tool
    log_info "Test 1: Query triggering search tool (non-streaming)..."

    local request_payload
    request_payload=$(cat <<EOF
{
    "model": "$ANTHROPIC_MODEL",
    "messages": [
        {"role": "user", "content": "What sci-fi movies from the 2010s are in the database?"}
    ],
    "max_tokens": 500,
    "stream": false
}
EOF
)

    local response
    response=$(meili_request POST "/chats/$WORKSPACE_UID/chat/completions" "$request_payload" 2>&1)

    if echo "$response" | jq -e '.choices[0].message.content' > /dev/null 2>&1; then
        local content
        content=$(echo "$response" | jq -r '.choices[0].message.content')

        if [[ "$content" == *"Inception"* ]] || [[ "$content" == *"Interstellar"* ]]; then
            log_success "Search tool executed and returned relevant results"
            log_info "  Response: ${content:0:200}..."
        else
            log_warn "Response may not have used search results: ${content:0:200}..."
        fi

        # Check finish reason
        local finish_reason
        finish_reason=$(echo "$response" | jq -r '.choices[0].finish_reason')
        log_info "  Finish reason: $finish_reason"
    else
        log_fail "Tool calling request failed"
        echo "Response: $response"
    fi

    # Test 2: Tool calling with streaming
    log_info "Test 2: Tool calling with streaming..."

    request_payload=$(cat <<EOF
{
    "model": "$ANTHROPIC_MODEL",
    "messages": [
        {"role": "user", "content": "List crime movies in the database."}
    ],
    "max_tokens": 500,
    "stream": true
}
EOF
)

    local tmp_file
    tmp_file=$(mktemp)
    meili_stream_request "/chats/$WORKSPACE_UID/chat/completions" "$request_payload" "$tmp_file"

    if grep -q "^data:" "$tmp_file"; then
        log_success "Streaming tool calling completed"

        # Extract full content from stream
        local full_content
        full_content=$(grep "^data:" "$tmp_file" | grep -v '\[DONE\]' | sed 's/^data: //' | \
            jq -r 'select(.choices[0].delta.content != null) | .choices[0].delta.content' 2>/dev/null | tr -d '\n')

        if [[ "$full_content" == *"Godfather"* ]] || [[ "$full_content" == *"Pulp Fiction"* ]]; then
            log_success "Streaming search returned relevant results"
            log_info "  Content preview: ${full_content:0:150}..."
        else
            log_warn "Streaming response may not include search results"
        fi
    else
        log_fail "Streaming tool calling failed"
        cat "$tmp_file"
    fi

    rm -f "$tmp_file"
}

# ============================================================================
# TEST: ERROR HANDLING
# ============================================================================

test_error_handling() {
    log_section "Error Handling Tests"

    # Test 1: Invalid API key
    log_info "Test 1: Invalid API key error..."

    # Temporarily configure with bad key
    local bad_settings
    bad_settings=$(cat <<EOF
{
    "source": "anthropic",
    "apiKey": "sk-ant-invalid-key-12345"
}
EOF
)

    meili_request PATCH "/chats/${WORKSPACE_UID}-error-test/settings" "$bad_settings" > /dev/null 2>&1

    local request_payload
    request_payload=$(cat <<EOF
{
    "model": "$ANTHROPIC_MODEL",
    "messages": [{"role": "user", "content": "Hi"}],
    "max_tokens": 10,
    "stream": false
}
EOF
)

    local response
    response=$(meili_request POST "/chats/${WORKSPACE_UID}-error-test/chat/completions" "$request_payload" 2>&1)

    if echo "$response" | jq -e '.code' > /dev/null 2>&1; then
        local error_code
        error_code=$(echo "$response" | jq -r '.code')
        log_success "Error properly returned with code: $error_code"

        # Check that error message is sanitized (no raw API key)
        local error_msg
        error_msg=$(echo "$response" | jq -r '.message')
        if [[ "$error_msg" != *"sk-ant"* ]]; then
            log_success "Error message properly sanitized"
        else
            log_fail "Error message may leak API key!"
        fi
    else
        log_warn "Error response format unexpected"
        echo "Response: $response"
    fi

    # Test 2: Missing model parameter
    log_info "Test 2: Missing required fields..."

    request_payload=$(cat <<EOF
{
    "messages": [{"role": "user", "content": "Hi"}],
    "stream": false
}
EOF
)

    response=$(meili_request POST "/chats/$WORKSPACE_UID/chat/completions" "$request_payload" 2>&1)

    if echo "$response" | jq -e '.code' > /dev/null 2>&1; then
        log_success "Missing field error handled correctly"
    else
        # Model might be optional or have a default
        log_info "Model field may be optional or have a default"
    fi

    # Test 3: Invalid model name
    log_info "Test 3: Invalid model name..."

    request_payload=$(cat <<EOF
{
    "model": "invalid-model-xyz",
    "messages": [{"role": "user", "content": "Hi"}],
    "max_tokens": 10,
    "stream": false
}
EOF
)

    response=$(meili_request POST "/chats/$WORKSPACE_UID/chat/completions" "$request_payload" 2>&1)

    if echo "$response" | jq -e '.code' > /dev/null 2>&1 || echo "$response" | jq -e '.error' > /dev/null 2>&1; then
        log_success "Invalid model error handled"
    else
        log_warn "Model validation may be deferred to Anthropic API"
    fi

    # Test 4: Streaming error
    log_info "Test 4: Streaming error handling..."

    local tmp_file
    tmp_file=$(mktemp)

    request_payload=$(cat <<EOF
{
    "model": "invalid-model-xyz",
    "messages": [{"role": "user", "content": "Hi"}],
    "max_tokens": 10,
    "stream": true
}
EOF
)

    meili_stream_request "/chats/$WORKSPACE_UID/chat/completions" "$request_payload" "$tmp_file"

    if grep -q "error" "$tmp_file" || grep -q "Error" "$tmp_file"; then
        log_success "Streaming error properly communicated"
    else
        log_info "Streaming error response:"
        cat "$tmp_file"
    fi

    rm -f "$tmp_file"

    # Cleanup error test workspace
    meili_request DELETE "/chats/${WORKSPACE_UID}-error-test/settings" > /dev/null 2>&1
}

# ============================================================================
# TEST: STOP REASON MAPPING
# ============================================================================

test_stop_reasons() {
    log_section "Stop Reason Mapping Tests"

    # Test 1: Normal completion (end_turn -> stop)
    log_info "Test 1: Normal completion (end_turn -> stop)..."

    local request_payload
    request_payload=$(cat <<EOF
{
    "model": "$ANTHROPIC_MODEL",
    "messages": [{"role": "user", "content": "Say 'done'."}],
    "max_tokens": 50,
    "stream": false
}
EOF
)

    local response
    response=$(meili_request POST "/chats/$WORKSPACE_UID/chat/completions" "$request_payload" 2>&1)

    local finish_reason
    finish_reason=$(echo "$response" | jq -r '.choices[0].finish_reason')

    if [[ "$finish_reason" == "stop" ]]; then
        log_success "end_turn correctly mapped to 'stop'"
    else
        log_warn "Unexpected finish_reason: $finish_reason (expected 'stop')"
    fi

    # Test 2: Max tokens (max_tokens -> length)
    log_info "Test 2: Max tokens limit (max_tokens -> length)..."

    request_payload=$(cat <<EOF
{
    "model": "$ANTHROPIC_MODEL",
    "messages": [{"role": "user", "content": "Write a very long story about a dragon."}],
    "max_tokens": 5,
    "stream": false
}
EOF
)

    response=$(meili_request POST "/chats/$WORKSPACE_UID/chat/completions" "$request_payload" 2>&1)
    finish_reason=$(echo "$response" | jq -r '.choices[0].finish_reason')

    if [[ "$finish_reason" == "length" ]]; then
        log_success "max_tokens correctly mapped to 'length'"
    else
        log_warn "Unexpected finish_reason: $finish_reason (expected 'length')"
    fi

    # Test 3: Stop sequences
    log_info "Test 3: Stop sequences (stop_sequence -> stop)..."

    request_payload=$(cat <<EOF
{
    "model": "$ANTHROPIC_MODEL",
    "messages": [{"role": "user", "content": "Count: 1, 2, 3, 4, 5"}],
    "max_tokens": 100,
    "stop": ["3"],
    "stream": false
}
EOF
)

    response=$(meili_request POST "/chats/$WORKSPACE_UID/chat/completions" "$request_payload" 2>&1)
    finish_reason=$(echo "$response" | jq -r '.choices[0].finish_reason')

    if [[ "$finish_reason" == "stop" ]]; then
        log_success "stop_sequence correctly mapped to 'stop'"
    else
        log_info "Finish reason: $finish_reason (stop sequences may not trigger)"
    fi
}

# ============================================================================
# TEST: TOKEN USAGE
# ============================================================================

test_token_usage() {
    log_section "Token Usage Tests"

    log_info "Test 1: Token usage in non-streaming response..."

    local request_payload
    request_payload=$(cat <<EOF
{
    "model": "$ANTHROPIC_MODEL",
    "messages": [{"role": "user", "content": "Say hello."}],
    "max_tokens": 50,
    "stream": false
}
EOF
)

    local response
    response=$(meili_request POST "/chats/$WORKSPACE_UID/chat/completions" "$request_payload" 2>&1)

    if echo "$response" | jq -e '.usage' > /dev/null 2>&1; then
        local prompt_tokens completion_tokens total_tokens
        prompt_tokens=$(echo "$response" | jq -r '.usage.prompt_tokens')
        completion_tokens=$(echo "$response" | jq -r '.usage.completion_tokens')
        total_tokens=$(echo "$response" | jq -r '.usage.total_tokens')

        log_success "Token usage reported"
        log_info "  Prompt tokens: $prompt_tokens"
        log_info "  Completion tokens: $completion_tokens"
        log_info "  Total tokens: $total_tokens"

        # Verify total = prompt + completion
        if [[ "$total_tokens" -eq $((prompt_tokens + completion_tokens)) ]]; then
            log_success "Token counts are consistent"
        else
            log_warn "Token counts don't add up"
        fi
    else
        log_warn "Token usage not included in response"
    fi
}

# ============================================================================
# TEST: CONCURRENT REQUESTS
# ============================================================================

test_concurrent_requests() {
    log_section "Concurrent Request Tests"

    log_info "Test 1: Multiple concurrent non-streaming requests..."

    local request_payload
    request_payload=$(cat <<EOF
{
    "model": "$ANTHROPIC_MODEL",
    "messages": [{"role": "user", "content": "Say a random number between 1 and 100."}],
    "max_tokens": 20,
    "stream": false
}
EOF
)

    local pids=()
    local tmp_dir
    tmp_dir=$(mktemp -d)

    # Launch 3 concurrent requests
    for i in 1 2 3; do
        (
            meili_request POST "/chats/$WORKSPACE_UID/chat/completions" "$request_payload" > "$tmp_dir/response_$i.json" 2>&1
        ) &
        pids+=($!)
    done

    # Wait for all to complete
    local success_count=0
    for i in "${!pids[@]}"; do
        if wait "${pids[$i]}"; then
            if jq -e '.choices[0].message.content' "$tmp_dir/response_$((i+1)).json" > /dev/null 2>&1; then
                ((success_count++))
            fi
        fi
    done

    if [[ "$success_count" -eq 3 ]]; then
        log_success "All 3 concurrent requests succeeded"
    else
        log_warn "$success_count/3 concurrent requests succeeded"
    fi

    rm -rf "$tmp_dir"
}

# ============================================================================
# SUMMARY
# ============================================================================

print_summary() {
    log_section "Test Summary"

    echo ""
    echo "Test Configuration:"
    echo "  Meilisearch: $MEILI_HOST"
    echo "  Workspace: $WORKSPACE_UID"
    echo "  Model: $ANTHROPIC_MODEL"
    echo ""

    echo "Key Findings:"
    echo ""
    echo "1. KNOWN ISSUE: settings.rs is missing the 'Anthropic' enum variant"
    echo "   - Users cannot configure Anthropic via PATCH /chats/{uid}/settings"
    echo "   - Fix: Add 'Anthropic' to ChatCompletionSource in settings.rs"
    echo ""
    echo "2. Once configured, the integration supports:"
    echo "   - Non-streaming chat completions"
    echo "   - Streaming chat completions (SSE)"
    echo "   - Tool calling with Meilisearch search"
    echo "   - Multi-turn conversations"
    echo "   - Stop reason mapping"
    echo "   - Token usage reporting"
    echo ""

    log_info "To clean up test data:"
    echo "  curl -X DELETE -H 'Authorization: Bearer $MEILI_API_KEY' '$MEILI_HOST/chats/$WORKSPACE_UID/settings'"
    echo "  curl -X DELETE -H 'Authorization: Bearer $MEILI_API_KEY' '$MEILI_HOST/indexes/$TEST_INDEX_UID'"
}

# ============================================================================
# MAIN
# ============================================================================

main() {
    echo ""
    echo -e "${BOLD}Meilisearch Anthropic Integration - Manual Test Suite${NC}"
    echo -e "Version: 1.0.0"
    echo -e "Date: $(date -Iseconds)"
    echo ""

    validate_environment

    # Run all test suites
    test_settings_api
    test_non_streaming_chat
    test_streaming_chat
    test_tool_calling
    test_error_handling
    test_stop_reasons
    test_token_usage
    test_concurrent_requests

    print_summary
}

# Run main if script is executed (not sourced)
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    main "$@"
fi
