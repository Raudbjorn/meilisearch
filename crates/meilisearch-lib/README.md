# meilisearch-lib

Embedded Meilisearch library for direct Rust integration.

This crate provides a Rust API for interacting with Meilisearch without going through the HTTP server. It wraps the `index-scheduler` crate and provides an ergonomic interface for building search-powered applications entirely in Rust.

## Features

- **Index Management** - Create, delete, list, and manage indexes
- **Document Operations** - Add, update, delete, and retrieve documents
- **Hybrid Search** - Combine keyword and semantic (vector) search
- **Task Management** - Monitor asynchronous operations with task polling
- **Settings Management** - Configure searchable attributes, embedders, ranking rules
- **Chat Completions** - RAG pipeline with OpenAI, Anthropic, and other LLM providers
- **Thread-Safe** - `Send + Sync` implementation for concurrent access

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
meilisearch-lib = { path = "crates/meilisearch-lib" }
tokio = { version = "1", features = ["full"] }
serde_json = "1"
```

## Quick Start

```rust
use meilisearch_lib::{MeilisearchLib, Config, SearchQuery};
use serde_json::json;

fn main() -> Result<(), meilisearch_lib::Error> {
    // 1. Create an embedded instance
    let meili = MeilisearchLib::new(
        Config::builder()
            .db_path("/tmp/meilisearch-data")
            .build()?
    )?;

    // 2. Create an index with a primary key
    let task = meili.create_index("movies", Some("id".to_string()))?;
    meili.wait_for_task(task.uid, None)?;

    // 3. Add documents
    let documents = vec![
        json!({"id": 1, "title": "The Matrix", "year": 1999, "genre": "sci-fi"}),
        json!({"id": 2, "title": "Inception", "year": 2010, "genre": "sci-fi"}),
        json!({"id": 3, "title": "The Dark Knight", "year": 2008, "genre": "action"}),
    ];
    let task = meili.add_documents("movies", documents, None)?;
    meili.wait_for_task(task.uid, None)?;

    // 4. Search
    let results = meili.search("movies", SearchQuery::new("matrix"))?;
    println!("Found {} results in {}ms", results.hits.len(), results.processing_time_ms);

    for hit in &results.hits {
        println!("  - {}", hit.document["title"]);
    }

    // 5. Cleanup
    meili.shutdown()?;
    Ok(())
}
```

## API Overview

### Creating an Instance

```rust
use meilisearch_lib::{MeilisearchLib, Config};

let meili = MeilisearchLib::new(
    Config::builder()
        .db_path("/var/lib/meilisearch")
        .max_index_size(100 * 1024 * 1024 * 1024)  // 100 GiB
        .max_task_db_size(10 * 1024 * 1024 * 1024) // 10 GiB
        .build()?
)?;
```

### Index Operations

```rust
// Create an index
let task = meili.create_index("products", Some("product_id".to_string()))?;
meili.wait_for_task(task.uid, None)?;

// Get index info
let index = meili.get_index("products")?;
println!("Primary key: {:?}", index.primary_key);
println!("Created: {}", index.created_at);

// Check if index exists
if meili.index_exists("products")? {
    println!("Products index exists");
}

// Get index statistics
let stats = meili.index_stats("products")?;
println!("Documents: {}", stats.number_of_documents);
println!("Is indexing: {}", stats.is_indexing);

// List all indexes
let (total, indexes) = meili.list_indexes(0, 100)?;
println!("Total indexes: {}", total);

// Delete an index
let task = meili.delete_index("products")?;
meili.wait_for_task(task.uid, None)?;
```

### Document Operations

```rust
use serde_json::json;

// Add documents (replaces existing)
let docs = vec![
    json!({"id": 1, "name": "iPhone", "price": 999}),
    json!({"id": 2, "name": "iPad", "price": 799}),
];
let task = meili.add_documents("products", docs, Some("id".to_string()))?;
meili.wait_for_task(task.uid, None)?;

// Update documents (partial update)
let updates = vec![
    json!({"id": 1, "price": 899}), // Only updates price
];
let task = meili.update_documents("products", updates, None)?;
meili.wait_for_task(task.uid, None)?;

// Get a single document
let doc = meili.get_document("products", "1")?;
println!("Product: {}", doc);

// Get multiple documents with pagination
let (total, docs) = meili.get_documents("products", 0, 10)?;
println!("Total documents: {}", total);

// Delete a single document
let task = meili.delete_document("products", "1")?;
meili.wait_for_task(task.uid, None)?;

// Delete multiple documents
let task = meili.delete_documents_batch("products", vec!["2".into(), "3".into()])?;
meili.wait_for_task(task.uid, None)?;

// Delete all documents (keeps index structure)
let task = meili.delete_all_documents("products")?;
meili.wait_for_task(task.uid, None)?;
```

### Search

```rust
use meilisearch_lib::{SearchQuery, HybridQuery};
use serde_json::json;

// Basic keyword search
let results = meili.search("products", SearchQuery::new("phone"))?;

// Search with pagination
let query = SearchQuery::new("phone")
    .with_pagination(0, 20);
let results = meili.search("products", query)?;

// Search with filter
let query = SearchQuery::new("phone")
    .with_filter(json!(["price < 1000", "brand = Apple"]));
let results = meili.search("products", query)?;

// Search with sort
let query = SearchQuery::new("phone")
    .with_sort(vec!["price:asc".to_string()]);
let results = meili.search("products", query)?;

// Select specific fields
let query = SearchQuery::new("phone")
    .with_attributes_to_retrieve(vec!["id".into(), "name".into()]);
let results = meili.search("products", query)?;

// Get ranking scores
let mut query = SearchQuery::new("phone");
query.show_ranking_score = true;
query.show_ranking_score_details = true;
let results = meili.search("products", query)?;

for hit in &results.hits {
    if let Some(score) = hit.ranking_score {
        println!("{}: score={}", hit.document["name"], score);
    }
}
```

### Hybrid Search

Combine keyword and semantic search for better results:

```rust
use meilisearch_lib::{SearchQuery, HybridQuery};
use serde_json::json;

// First, configure an embedder
let embedders = json!({
    "default": {
        "source": "openAi",
        "apiKey": std::env::var("OPENAI_API_KEY").unwrap(),
        "model": "text-embedding-3-small",
        "documentTemplate": "A product named '{{doc.name}}' priced at ${{doc.price}}"
    }
});
let task = meili.update_embedders("products", embedders)?;
meili.wait_for_task(task.uid, None)?;

// Hybrid search (keyword + semantic)
let query = SearchQuery::new("affordable smartphone")
    .with_hybrid(HybridQuery::new(0.7));  // 70% semantic, 30% keyword
let results = meili.search("products", query)?;

println!("Semantic hit count: {:?}", results.semantic_hit_count);

// Pure semantic search with pre-computed vector
let query = SearchQuery::empty()
    .with_vector(vec![0.1, 0.2, 0.3, /* ... */]);
let results = meili.search("products", query)?;

// Hybrid with specific embedder
let query = SearchQuery::new("query")
    .with_hybrid(HybridQuery::new(0.5).with_embedder("my-embedder"));
let results = meili.search("products", query)?;
```

### Settings

```rust
use meilisearch_lib::{Settings, Setting};
use std::collections::BTreeSet;

// Update settings
let mut settings = Settings::default();
settings.searchable_attributes = Setting::Set(vec![
    "title".to_string(),
    "description".to_string(),
]).into();
settings.filterable_attributes = Setting::Set(
    ["genre", "year", "price"].iter().map(|s| s.to_string()).collect()
);
settings.sortable_attributes = Setting::Set(
    ["year", "price"].iter().map(|s| s.to_string()).collect()
);

let task = meili.update_settings("movies", settings)?;
meili.wait_for_task(task.uid, None)?;

// Get current settings
let settings = meili.get_settings("movies")?;
println!("Searchable: {:?}", settings.searchable_attributes);

// Get embedder configuration
if let Some(embedders) = meili.get_embedders("movies")? {
    println!("Embedders: {}", serde_json::to_string_pretty(&embedders)?);
}

// Reset settings to defaults
let task = meili.reset_settings("movies")?;
meili.wait_for_task(task.uid, None)?;

// Reset only embedders
let task = meili.reset_embedders("movies")?;
meili.wait_for_task(task.uid, None)?;
```

### Task Management

```rust
use std::time::Duration;
use meilisearch_lib::TaskStatus;

// Get a task by ID
let task = meili.get_task(42)?;
println!("Task {} status: {:?}", task.uid, task.status);

// Wait for task with timeout
let task = meili.wait_for_task(task_id, Some(Duration::from_secs(30)))?;

// Async version
let task = meili.wait_for_task_async(task_id, Some(Duration::from_secs(30))).await?;

// Check task status
match task.status {
    TaskStatus::Succeeded => println!("Task completed successfully"),
    TaskStatus::Failed => {
        if let Some(error) = task.error {
            println!("Task failed: {} ({})", error.message, error.code);
        }
    }
    TaskStatus::Canceled => println!("Task was canceled"),
    TaskStatus::Enqueued => println!("Task is waiting"),
    TaskStatus::Processing => println!("Task is running"),
}
```

### Chat Completions

Use Meilisearch as the retrieval backend for RAG (Retrieval-Augmented Generation):

```rust
use meilisearch_lib::{
    ChatConfig, ChatSource, ChatRequest, ChatResponse, Message,
    ChatPrompts, ChatIndexConfig, ChatSearchParams,
};
use std::collections::HashMap;

// Configure the LLM provider
let mut index_configs = HashMap::new();
index_configs.insert("products".to_string(), ChatIndexConfig {
    description: "Product catalog with electronics, phones, and accessories".to_string(),
    template: Some("Product: {{doc.name}}, Price: ${{doc.price}}".to_string()),
    max_bytes: Some(500),
    search_params: Some(ChatSearchParams {
        limit: Some(5),
        semantic_ratio: Some(0.7),
        embedder: Some("default".to_string()),
        ..Default::default()
    }),
});

let chat_config = ChatConfig {
    source: ChatSource::OpenAi,
    api_key: std::env::var("OPENAI_API_KEY").unwrap(),
    base_url: None,
    model: "gpt-4".to_string(),
    org_id: None,
    project_id: None,
    api_version: None,
    deployment_id: None,
    prompts: ChatPrompts {
        system: Some("You are a helpful shopping assistant.".to_string()),
        ..Default::default()
    },
    index_configs,
};

meili.set_chat_config(Some(chat_config));

// Send a chat request
let response = meili.chat_completion(ChatRequest {
    messages: vec![
        Message::user("What phones do you have under $500?"),
    ],
    index_uid: "products".to_string(),
    stream: false,
}).await?;

println!("Assistant: {}", response.content);
println!("Sources: {:?}", response.sources);
if let Some(usage) = response.usage {
    println!("Tokens: {} prompt, {} completion",
        usage.prompt_tokens, usage.completion_tokens);
}

// Streaming response
let request = ChatRequest {
    messages: vec![Message::user("Tell me about your best products")],
    index_uid: "products".to_string(),
    stream: true,
};

let mut stream = meili.chat_completion_stream(request).await?;
while let Some(chunk) = stream.next().await {
    let chunk = chunk?;
    print!("{}", chunk.delta);
    if chunk.done {
        println!("\nSources: {:?}", chunk.sources);
    }
}
```

#### Supported LLM Providers

| Provider | `ChatSource` | Required Fields |
|----------|--------------|-----------------|
| OpenAI | `OpenAi` | `api_key`, `model` |
| Anthropic | `Anthropic` | `api_key`, `model` |
| Azure OpenAI | `AzureOpenAi` | `api_key`, `base_url`, `deployment_id`, `api_version` |
| Mistral AI | `Mistral` | `api_key`, `model` |
| vLLM | `VLlm` | `base_url`, `model` |

## Configuration Options

| Option | Default | Description |
|--------|---------|-------------|
| `db_path` | Required | Path to the database directory |
| `max_index_size` | 100 GiB | Maximum size per index |
| `max_task_db_size` | 10 GiB | Maximum size of task history database |

```rust
let config = Config::builder()
    .db_path("/var/lib/meilisearch")
    .max_index_size(200 * 1024 * 1024 * 1024)  // 200 GiB
    .max_task_db_size(20 * 1024 * 1024 * 1024) // 20 GiB
    .build()?;
```

## Error Handling

All operations return `Result<T, meilisearch_lib::Error>`. The error type provides HTTP-compatible status codes:

```rust
use meilisearch_lib::Error;

match meili.get_index("movies") {
    Ok(index) => println!("Found: {}", index.uid),
    Err(Error::IndexNotFound(uid)) => {
        // 404 Not Found
        println!("Index '{}' not found", uid);
    }
    Err(Error::InvalidIndexUid(uid)) => {
        // 400 Bad Request
        println!("Invalid index UID: {}", uid);
    }
    Err(Error::ChatNotConfigured) => {
        // 404 Not Found
        println!("Chat not configured");
    }
    Err(e) => {
        // Get HTTP status code
        println!("Error: {} (HTTP {})", e, e.status_code());
    }
}
```

### Error Types

| Error | Status | Description |
|-------|--------|-------------|
| `IndexNotFound` | 404 | Index does not exist |
| `DocumentNotFound` | 404 | Document does not exist |
| `TaskNotFound` | 404 | Task does not exist |
| `ChatNotConfigured` | 404 | Chat configuration not set |
| `InvalidIndexUid` | 400 | Index UID is invalid |
| `InvalidSettings` | 400 | Settings validation failed |
| `MissingDbPath` | 400 | Database path not provided |
| `TaskTimeout` | 400 | Task did not complete in time |
| `ChatProvider` | 500 | LLM provider returned an error |
| `Search` | 500 | Search execution failed |
| `Internal` | 500 | Unexpected internal error |

## Thread Safety

`MeilisearchLib` is `Send + Sync` and can be safely shared across threads:

```rust
use std::sync::Arc;
use std::thread;

let meili = Arc::new(MeilisearchLib::new(config)?);

let handles: Vec<_> = (0..4).map(|i| {
    let meili = Arc::clone(&meili);
    thread::spawn(move || {
        let results = meili.search("products", SearchQuery::new("phone"))?;
        println!("Thread {}: {} results", i, results.hits.len());
        Ok::<_, meilisearch_lib::Error>(())
    })
}).collect();

for handle in handles {
    handle.join().unwrap()?;
}
```

## License

MIT License. See [LICENSE](../../LICENSE) for details.
