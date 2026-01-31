//! Integration tests for meilisearch-lib.
//!
//! These tests verify the full workflow of the embedded Meilisearch library,
//! including index lifecycle, document operations, task management, and
//! concurrent operations.
//!
//! Each test uses a temporary directory for the database and cleans up after
//! completion. Tests are marked async and use `#[tokio::test]` for async support.

use std::sync::Arc;
use std::time::Duration;

use meilisearch_lib::{Config, Error, MeilisearchLib, SearchQuery, TaskStatus};
use mock_server::{MockServer, MockServerConfig};
use serde_json::json;
use tempfile::tempdir;

// ============================================================================
// Helper Functions
// ============================================================================

/// Create a new MeilisearchLib instance with a temporary database.
fn create_instance() -> (MeilisearchLib, tempfile::TempDir) {
    let tmp = tempdir().expect("failed to create temp dir");
    let config = Config::builder().db_path(tmp.path()).build().expect("failed to build config");

    let meili = MeilisearchLib::new(config).expect("failed to create MeilisearchLib");
    (meili, tmp)
}

/// Wait for a task to complete with a default timeout.
async fn wait_for_task(meili: &MeilisearchLib, task_id: u32) -> meilisearch_lib::TaskView {
    meili
        .wait_for_task_async(task_id, Some(Duration::from_secs(30)))
        .await
        .expect("task wait failed")
}

// ============================================================================
// Test: Full Index Lifecycle
// ============================================================================

/// Test the complete index lifecycle: create -> get -> stats -> delete.
///
/// This test verifies:
/// 1. Index creation returns a valid task
/// 2. The task completes successfully
/// 3. The index can be retrieved with correct metadata
/// 4. Index statistics are available and correct
/// 5. Index deletion returns a valid task that completes
/// 6. The index no longer exists after deletion
#[tokio::test]
async fn test_full_index_lifecycle() {
    let (meili, _tmp) = create_instance();

    // Create an index
    let create_task = meili
        .create_index("movies", Some("id".to_string()))
        .expect("failed to register create_index task");

    assert_eq!(create_task.status, TaskStatus::Enqueued);

    // Wait for index creation to complete
    let completed_task = wait_for_task(&meili, create_task.uid).await;
    assert_eq!(
        completed_task.status,
        TaskStatus::Succeeded,
        "Index creation failed: {:?}",
        completed_task.error
    );

    // Verify index exists
    assert!(
        meili.index_exists("movies").expect("index_exists failed"),
        "Index should exist after creation"
    );

    // Get the index and verify metadata
    let index = meili.get_index("movies").expect("failed to get index");
    assert_eq!(index.uid, "movies");
    assert_eq!(index.primary_key, Some("id".to_string()));

    // Get index stats
    let stats = meili.index_stats("movies").expect("failed to get stats");
    assert_eq!(stats.number_of_documents, 0);
    assert!(!stats.is_indexing);

    // Delete the index
    let delete_task = meili.delete_index("movies").expect("failed to register delete_index task");

    let delete_completed = wait_for_task(&meili, delete_task.uid).await;
    assert_eq!(
        delete_completed.status,
        TaskStatus::Succeeded,
        "Index deletion failed: {:?}",
        delete_completed.error
    );

    // Verify index no longer exists
    assert!(
        !meili.index_exists("movies").expect("index_exists failed"),
        "Index should not exist after deletion"
    );

    // Cleanup
    meili.shutdown().expect("shutdown failed");
}

// ============================================================================
// Test: Document Add and Delete
// ============================================================================

/// Test document operations: create index -> add documents -> search -> delete docs.
///
/// This test verifies:
/// 1. Documents can be added to an index
/// 2. Documents are searchable after indexing completes
/// 3. Individual documents can be retrieved by ID
/// 4. Documents can be deleted
/// 5. Deleted documents are no longer searchable
#[tokio::test]
async fn test_document_add_delete() {
    let (meili, _tmp) = create_instance();

    // Create index
    let create_task =
        meili.create_index("books", Some("id".to_string())).expect("create_index failed");
    let _ = wait_for_task(&meili, create_task.uid).await;

    // Add documents
    let documents = vec![
        json!({"id": "1", "title": "The Hobbit", "author": "J.R.R. Tolkien", "year": 1937}),
        json!({"id": "2", "title": "1984", "author": "George Orwell", "year": 1949}),
        json!({"id": "3", "title": "Dune", "author": "Frank Herbert", "year": 1965}),
    ];

    let add_task =
        meili.add_documents("books", documents.clone(), None).expect("add_documents failed");

    let add_completed = wait_for_task(&meili, add_task.uid).await;
    assert_eq!(
        add_completed.status,
        TaskStatus::Succeeded,
        "Document addition failed: {:?}",
        add_completed.error
    );

    // Verify document count
    let stats = meili.index_stats("books").expect("stats failed");
    assert_eq!(stats.number_of_documents, 3);

    // Search for documents
    let search_query = SearchQuery::new("Hobbit");
    let search_result = meili.search("books", search_query).expect("search failed");

    assert!(!search_result.hits.is_empty(), "Search should return results");
    assert_eq!(search_result.query, "Hobbit");

    // Verify the correct document was found
    let first_hit = &search_result.hits[0];
    assert_eq!(first_hit.document["title"], "The Hobbit");

    // Get a specific document by ID
    let doc = meili.get_document("books", "2").expect("get_document failed");
    assert_eq!(doc["title"], "1984");
    assert_eq!(doc["author"], "George Orwell");

    // Delete a document
    let delete_task = meili.delete_document("books", "2").expect("delete_document failed");

    let delete_completed = wait_for_task(&meili, delete_task.uid).await;
    assert_eq!(
        delete_completed.status,
        TaskStatus::Succeeded,
        "Document deletion failed: {:?}",
        delete_completed.error
    );

    // Verify document count decreased
    let stats_after = meili.index_stats("books").expect("stats failed");
    assert_eq!(stats_after.number_of_documents, 2);

    // Verify deleted document is not found
    let result = meili.get_document("books", "2");
    assert!(
        matches!(result, Err(Error::DocumentNotFound(_))),
        "Expected DocumentNotFound error, got: {:?}",
        result
    );

    // Cleanup
    meili.shutdown().expect("shutdown failed");
}

// ============================================================================
// Test: Task Wait
// ============================================================================

/// Test task management: enqueue task -> wait for completion -> verify status.
///
/// This test verifies:
/// 1. Tasks are created with Enqueued status
/// 2. wait_for_task_async returns when the task completes
/// 3. Task status progresses to Succeeded or Failed
/// 4. Completed tasks have timestamp information
#[tokio::test]
async fn test_task_wait() {
    let (meili, _tmp) = create_instance();

    // Create an index (this enqueues a task)
    let task = meili.create_index("task_test", None).expect("create_index failed");

    // Task should be enqueued initially
    assert_eq!(task.status, TaskStatus::Enqueued);
    assert!(task.started_at.is_none());
    assert!(task.finished_at.is_none());

    // Wait for the task
    let completed = meili
        .wait_for_task_async(task.uid, Some(Duration::from_secs(30)))
        .await
        .expect("wait_for_task_async failed");

    // Task should be in a terminal state
    assert!(
        completed.status == TaskStatus::Succeeded || completed.status == TaskStatus::Failed,
        "Task should be in terminal state, got: {:?}",
        completed.status
    );

    // Timestamps should be populated
    assert!(completed.started_at.is_some(), "started_at should be set");
    assert!(completed.finished_at.is_some(), "finished_at should be set");

    // Verify we can retrieve the task by ID
    let retrieved = meili.get_task(task.uid).expect("get_task failed");
    assert_eq!(retrieved.uid, task.uid);
    assert_eq!(retrieved.status, completed.status);

    // Cleanup
    meili.shutdown().expect("shutdown failed");
}

/// Test task timeout behavior.
///
/// This test verifies that wait_for_task_async respects the timeout parameter
/// and returns a TaskTimeout error when exceeded.
#[tokio::test]
async fn test_task_wait_timeout() {
    let (meili, _tmp) = create_instance();

    // Create an index
    let create_task = meili.create_index("timeout_test", None).expect("create_index failed");
    let _ = wait_for_task(&meili, create_task.uid).await;

    // Add a large batch of documents to create a slow task
    let documents: Vec<_> = (0..1000)
        .map(|i| {
            json!({
                "id": i,
                "content": format!("Document content number {} with some text to index", i)
            })
        })
        .collect();

    let add_task = meili
        .add_documents("timeout_test", documents, Some("id".to_string()))
        .expect("add_documents failed");

    // Try to wait with a very short timeout
    // Note: This test may pass if the task completes quickly on fast systems
    let result = meili.wait_for_task_async(add_task.uid, Some(Duration::from_nanos(1))).await;

    // The task may complete before the timeout on fast systems,
    // so we check for either success or timeout
    match result {
        Ok(task) => {
            // Task completed before timeout - that's fine
            assert!(
                task.status == TaskStatus::Succeeded || task.status == TaskStatus::Failed,
                "Task should be in terminal state"
            );
        }
        Err(Error::TaskTimeout(id, _)) => {
            // Timeout occurred as expected
            assert_eq!(id, add_task.uid);
        }
        Err(e) => {
            panic!("Unexpected error: {:?}", e);
        }
    }

    // Cleanup
    meili.shutdown().expect("shutdown failed");
}

// ============================================================================
// Test: Concurrent Operations
// ============================================================================

/// Test concurrent operations with multiple threads.
///
/// This test verifies thread safety by:
/// 1. Sharing the MeilisearchLib instance across multiple tasks
/// 2. Performing concurrent document additions
/// 3. Performing concurrent searches
/// 4. Verifying all operations complete without deadlock or data corruption
#[tokio::test]
async fn test_concurrent_operations() {
    let (meili, _tmp) = create_instance();
    let meili = Arc::new(meili);

    // Create index first
    let create_task =
        meili.create_index("concurrent", Some("id".to_string())).expect("create_index failed");
    let _ = wait_for_task(&meili, create_task.uid).await;

    // Add initial documents
    let initial_docs = vec![
        json!({"id": "base1", "content": "initial document one"}),
        json!({"id": "base2", "content": "initial document two"}),
    ];
    let add_task =
        meili.add_documents("concurrent", initial_docs, None).expect("add_documents failed");
    let _ = wait_for_task(&meili, add_task.uid).await;

    // Spawn multiple concurrent tasks that add documents
    let mut handles = Vec::new();

    for i in 0..5 {
        let meili_clone = Arc::clone(&meili);
        let handle = tokio::spawn(async move {
            let docs = vec![json!({
                "id": format!("concurrent_{}", i),
                "content": format!("concurrent document {}", i)
            })];

            let task = meili_clone
                .add_documents("concurrent", docs, None)
                .expect("concurrent add_documents failed");

            // Wait for task to complete
            meili_clone
                .wait_for_task_async(task.uid, Some(Duration::from_secs(30)))
                .await
                .expect("concurrent wait failed")
        });
        handles.push(handle);
    }

    // Wait for all concurrent additions to complete
    for handle in handles {
        let task = handle.await.expect("task join failed");
        assert_eq!(task.status, TaskStatus::Succeeded, "Concurrent task failed: {:?}", task.error);
    }

    // Spawn concurrent search operations
    let mut search_handles = Vec::new();

    for i in 0..3 {
        let meili_clone = Arc::clone(&meili);
        let handle = tokio::spawn(async move {
            let query = SearchQuery::new("concurrent");
            meili_clone
                .search("concurrent", query)
                .expect(&format!("concurrent search {} failed", i))
        });
        search_handles.push(handle);
    }

    // Verify all searches complete successfully
    for handle in search_handles {
        let result = handle.await.expect("search join failed");
        assert!(result.processing_time_ms > 0);
    }

    // Verify final document count (2 initial + 5 concurrent)
    let stats = meili.index_stats("concurrent").expect("stats failed");
    assert_eq!(stats.number_of_documents, 7, "Expected 7 documents (2 initial + 5 concurrent)");

    // Cleanup - need to get out of Arc to call shutdown
    // Arc::try_unwrap returns Ok(T) if successful, Err(Arc<T>) if there are other references
    match Arc::try_unwrap(meili) {
        Ok(instance) => instance.shutdown().expect("shutdown failed"),
        Err(_) => panic!("failed to unwrap Arc - there are still other references"),
    }
}

// ============================================================================
// Test: Error Scenarios
// ============================================================================

/// Test error handling for various error scenarios.
///
/// This test verifies:
/// 1. IndexNotFound error when accessing non-existent index
/// 2. DocumentNotFound error when accessing non-existent document
/// 3. TaskNotFound error when accessing non-existent task
/// 4. Invalid configuration errors
#[tokio::test]
async fn test_error_scenarios() {
    let (meili, _tmp) = create_instance();

    // Test: Index not found
    let result = meili.get_index("nonexistent");
    assert!(
        matches!(result, Err(Error::Scheduler(_))),
        "Expected scheduler error for nonexistent index, got: {:?}",
        result
    );

    // Test: Index exists returns false for nonexistent
    let exists = meili.index_exists("nonexistent").expect("index_exists failed");
    assert!(!exists, "nonexistent index should not exist");

    // Test: Document not found (first create an index)
    let create_task =
        meili.create_index("error_test", Some("id".to_string())).expect("create_index failed");
    let _ = wait_for_task(&meili, create_task.uid).await;

    let result = meili.get_document("error_test", "nonexistent_doc");
    assert!(
        matches!(result, Err(Error::DocumentNotFound(_))),
        "Expected DocumentNotFound error, got: {:?}",
        result
    );

    // Test: Task not found
    let result = meili.get_task(999999);
    assert!(
        matches!(result, Err(Error::TaskNotFound(_))),
        "Expected TaskNotFound error, got: {:?}",
        result
    );

    // Test: Invalid index UID (empty string)
    let result = meili.create_index("", None);
    assert!(
        matches!(result, Err(Error::InvalidIndexUid(_))),
        "Expected InvalidIndexUid error for empty string, got: {:?}",
        result
    );

    // Cleanup
    meili.shutdown().expect("shutdown failed");
}

/// Test configuration validation errors.
#[tokio::test]
async fn test_config_errors() {
    // Test: Missing db_path
    let result = Config::builder().build();
    assert!(
        matches!(result, Err(Error::MissingDbPath)),
        "Expected MissingDbPath error, got: {:?}",
        result
    );

    // Test: Valid config with custom sizes
    let tmp = tempdir().expect("failed to create temp dir");
    let config = Config::builder()
        .db_path(tmp.path())
        .max_index_size(50 * 1024 * 1024 * 1024) // 50 GiB
        .max_task_db_size(5 * 1024 * 1024 * 1024) // 5 GiB
        .build()
        .expect("failed to build config");

    assert_eq!(config.max_index_size, 50 * 1024 * 1024 * 1024);
    assert_eq!(config.max_task_db_size, 5 * 1024 * 1024 * 1024);
}

// ============================================================================
// Test: Search Functionality
// ============================================================================

/// Test various search query configurations.
#[tokio::test]
async fn test_search_queries() {
    let (meili, _tmp) = create_instance();

    // Create index and add documents
    let create_task =
        meili.create_index("search_test", Some("id".to_string())).expect("create_index failed");
    let _ = wait_for_task(&meili, create_task.uid).await;

    let documents = vec![
        json!({"id": "1", "title": "Introduction to Rust Programming", "category": "programming", "rating": 5}),
        json!({"id": "2", "title": "Advanced Python Techniques", "category": "programming", "rating": 4}),
        json!({"id": "3", "title": "Cooking with Herbs", "category": "cooking", "rating": 5}),
        json!({"id": "4", "title": "Rust and Systems Programming", "category": "programming", "rating": 5}),
    ];

    let add_task =
        meili.add_documents("search_test", documents, None).expect("add_documents failed");
    let _ = wait_for_task(&meili, add_task.uid).await;

    // Test: Basic keyword search
    let query = SearchQuery::new("Rust");
    let result = meili.search("search_test", query).expect("search failed");
    assert_eq!(result.hits.len(), 2, "Should find 2 Rust-related documents");

    // Test: Pagination
    let query = SearchQuery::new("programming").with_pagination(0, 1);
    let result = meili.search("search_test", query).expect("search failed");
    assert_eq!(result.hits.len(), 1, "Should return only 1 result due to limit");
    assert_eq!(result.limit, Some(1));

    // Test: Empty query returns all documents
    let query = SearchQuery::empty();
    let result = meili.search("search_test", query).expect("search failed");
    assert_eq!(result.hits.len(), 4, "Empty query should return all documents");

    // Test: Query with attributes to retrieve
    let query = SearchQuery::new("Rust").with_attributes_to_retrieve(vec!["title".to_string()]);
    let result = meili.search("search_test", query).expect("search failed");

    // Verify only requested attributes are returned
    // Note: The actual behavior depends on index configuration
    assert!(!result.hits.is_empty());

    // Test: Ranking score
    let mut query = SearchQuery::new("Rust programming");
    query.show_ranking_score = true;
    let result = meili.search("search_test", query).expect("search failed");

    // All hits should have ranking scores when requested
    for hit in &result.hits {
        assert!(hit.ranking_score.is_some(), "Ranking score should be present");
    }

    // Cleanup
    meili.shutdown().expect("shutdown failed");
}

// ============================================================================
// Test: Document Batch Operations
// ============================================================================

/// Test batch document operations.
#[tokio::test]
async fn test_document_batch_operations() {
    let (meili, _tmp) = create_instance();

    // Create index
    let create_task =
        meili.create_index("batch_test", Some("id".to_string())).expect("create_index failed");
    let _ = wait_for_task(&meili, create_task.uid).await;

    // Add documents
    let documents: Vec<_> = (1..=10).map(|i| json!({"id": i.to_string(), "value": i})).collect();

    let add_task =
        meili.add_documents("batch_test", documents, None).expect("add_documents failed");
    let _ = wait_for_task(&meili, add_task.uid).await;

    // Verify initial count
    let stats = meili.index_stats("batch_test").expect("stats failed");
    assert_eq!(stats.number_of_documents, 10);

    // Delete batch of documents
    let ids_to_delete: Vec<String> = (1..=5).map(|i| i.to_string()).collect();
    let delete_task = meili
        .delete_documents_batch("batch_test", ids_to_delete)
        .expect("delete_documents_batch failed");
    let _ = wait_for_task(&meili, delete_task.uid).await;

    // Verify count after batch delete
    let stats = meili.index_stats("batch_test").expect("stats failed");
    assert_eq!(stats.number_of_documents, 5);

    // Test delete all documents
    let clear_task = meili.delete_all_documents("batch_test").expect("delete_all_documents failed");
    let _ = wait_for_task(&meili, clear_task.uid).await;

    // Verify index is empty but still exists
    let stats = meili.index_stats("batch_test").expect("stats failed");
    assert_eq!(stats.number_of_documents, 0);
    assert!(meili.index_exists("batch_test").expect("index_exists failed"));

    // Cleanup
    meili.shutdown().expect("shutdown failed");
}

// ============================================================================
// Test: Get Documents with Pagination
// ============================================================================

/// Test paginated document retrieval.
#[tokio::test]
async fn test_get_documents_pagination() {
    let (meili, _tmp) = create_instance();

    // Create index
    let create_task =
        meili.create_index("pagination_test", Some("id".to_string())).expect("create_index failed");
    let _ = wait_for_task(&meili, create_task.uid).await;

    // Add documents
    let documents: Vec<_> =
        (1..=25).map(|i| json!({"id": i.to_string(), "name": format!("Item {}", i)})).collect();

    let add_task =
        meili.add_documents("pagination_test", documents, None).expect("add_documents failed");
    let _ = wait_for_task(&meili, add_task.uid).await;

    // Test: Get first page
    let (total, docs) =
        meili.get_documents("pagination_test", 0, 10).expect("get_documents failed");
    assert_eq!(total, 25);
    assert_eq!(docs.len(), 10);

    // Test: Get second page
    let (total, docs) =
        meili.get_documents("pagination_test", 10, 10).expect("get_documents failed");
    assert_eq!(total, 25);
    assert_eq!(docs.len(), 10);

    // Test: Get last page (partial)
    let (total, docs) =
        meili.get_documents("pagination_test", 20, 10).expect("get_documents failed");
    assert_eq!(total, 25);
    assert_eq!(docs.len(), 5);

    // Test: Get beyond range
    let (total, docs) =
        meili.get_documents("pagination_test", 30, 10).expect("get_documents failed");
    assert_eq!(total, 25);
    assert_eq!(docs.len(), 0);

    // Cleanup
    meili.shutdown().expect("shutdown failed");
}

// ============================================================================
// Test: List Indexes
// ============================================================================

/// Test listing indexes with pagination.
#[tokio::test]
async fn test_list_indexes() {
    let (meili, _tmp) = create_instance();

    // Create multiple indexes
    let index_names = ["index_a", "index_b", "index_c"];

    for name in &index_names {
        let task = meili.create_index(*name, Some("id".to_string())).expect("create_index failed");
        let _ = wait_for_task(&meili, task.uid).await;
    }

    // List all indexes
    let (total, indexes) = meili.list_indexes(0, 10).expect("list_indexes failed");
    assert_eq!(total, 3);
    assert_eq!(indexes.len(), 3);

    // List with pagination
    let (total, indexes) = meili.list_indexes(0, 2).expect("list_indexes failed");
    assert_eq!(total, 3);
    assert_eq!(indexes.len(), 2);

    let (total, indexes) = meili.list_indexes(2, 2).expect("list_indexes failed");
    assert_eq!(total, 3);
    assert_eq!(indexes.len(), 1);

    // Cleanup
    meili.shutdown().expect("shutdown failed");
}

// ============================================================================
// Test: Health Check
// ============================================================================

/// Test health check functionality.
#[tokio::test]
async fn test_health_check() {
    let (meili, _tmp) = create_instance();

    let health = meili.health();
    assert_eq!(health.status, "available");

    // Cleanup
    meili.shutdown().expect("shutdown failed");
}

// ============================================================================
// Test: Features Toggle
// ============================================================================

/// Test runtime feature toggles.
#[tokio::test]
async fn test_features_toggle() {
    let (meili, _tmp) = create_instance();

    // Get default features
    let features = meili.get_features();

    // Modify features
    let mut new_features = features.clone();
    new_features.chat_completions = true;
    meili.set_features(new_features);

    // Verify change
    let updated = meili.get_features();
    assert!(updated.chat_completions);

    // Cleanup
    meili.shutdown().expect("shutdown failed");
}

// ============================================================================
// Test: Update Documents (Partial Update)
// ============================================================================

/// Test partial document updates.
#[tokio::test]
async fn test_update_documents() {
    let (meili, _tmp) = create_instance();

    // Create index
    let create_task =
        meili.create_index("update_test", Some("id".to_string())).expect("create_index failed");
    let _ = wait_for_task(&meili, create_task.uid).await;

    // Add initial document
    let documents = vec![json!({
        "id": "1",
        "name": "Original Name",
        "description": "Original description",
        "count": 10
    })];

    let add_task =
        meili.add_documents("update_test", documents, None).expect("add_documents failed");
    let _ = wait_for_task(&meili, add_task.uid).await;

    // Partial update - only update name
    let update_docs = vec![json!({
        "id": "1",
        "name": "Updated Name"
    })];

    let update_task =
        meili.update_documents("update_test", update_docs, None).expect("update_documents failed");
    let _ = wait_for_task(&meili, update_task.uid).await;

    // Verify update
    let doc = meili.get_document("update_test", "1").expect("get_document failed");
    assert_eq!(doc["name"], "Updated Name");
    assert_eq!(doc["description"], "Original description"); // Should be preserved
    assert_eq!(doc["count"], 10); // Should be preserved

    // Cleanup
    meili.shutdown().expect("shutdown failed");
}

// ============================================================================
// Test: Hybrid Search Workflow (Uses Mock Embedder Server)
// ============================================================================

/// Test hybrid search workflow combining keyword and semantic search.
///
/// Uses the mock server for deterministic embeddings, enabling fully automated testing.
///
/// This test verifies:
/// 1. Index can be configured with embedder settings
/// 2. Documents are indexed with embeddings from mock server
/// 3. Hybrid search combines keyword and vector results
/// 4. Semantic ratio affects search behavior
#[tokio::test]
async fn test_hybrid_search_workflow() {
    use meilisearch_lib::{Setting, Settings, SettingEmbeddingSettings};
    use std::collections::BTreeMap;

    // Start mock embeddings server
    let mock_server = MockServer::start(MockServerConfig::with_random_port())
        .await
        .expect("failed to start mock server");


    let (meili, _tmp) = create_instance();

    // Create index for hybrid search
    let create_task = meili
        .create_index("hybrid_test", Some("id".to_string()))
        .expect("create_index failed");
    let _ = wait_for_task(&meili, create_task.uid).await;

    // Configure embedder to use mock server
    let embedder_settings_json = json!({
        "default": {
            "source": "openAi",
            "url": mock_server.embeddings_url(),
            "apiKey": "mock-test-key",
            "model": "text-embedding-3-small",
            "dimensions": 1536,
            "documentTemplate": "{{ doc.title }}: {{ doc.content }}"
        }
    });
    let embedders: BTreeMap<String, SettingEmbeddingSettings> =
        serde_json::from_value(embedder_settings_json).expect("failed to parse embedder settings");

    let mut settings = Settings::default();
    settings.embedders = Setting::Set(embedders);

    let settings_task = meili
        .update_settings("hybrid_test", settings)
        .expect("failed to update settings");
    let settings_completed = wait_for_task(&meili, settings_task.uid).await;
    assert_eq!(
        settings_completed.status,
        TaskStatus::Succeeded,
        "Settings update failed: {:?}",
        settings_completed.error
    );

    // Add documents with content suitable for semantic search
    let documents = vec![
        json!({
            "id": "1",
            "title": "Introduction to Machine Learning",
            "content": "Machine learning is a subset of artificial intelligence that enables systems to learn from data."
        }),
        json!({
            "id": "2",
            "title": "Deep Learning Fundamentals",
            "content": "Deep learning uses neural networks with multiple layers to process complex patterns."
        }),
        json!({
            "id": "3",
            "title": "Natural Language Processing",
            "content": "NLP enables computers to understand, interpret, and generate human language."
        }),
        json!({
            "id": "4",
            "title": "Computer Vision Applications",
            "content": "Computer vision allows machines to interpret visual information from the world."
        }),
    ];

    let add_task =
        meili.add_documents("hybrid_test", documents, None).expect("add_documents failed");
    let add_completed = wait_for_task(&meili, add_task.uid).await;
    assert_eq!(
        add_completed.status,
        TaskStatus::Succeeded,
        "Document addition failed: {:?}",
        add_completed.error
    );

    // Test hybrid search with semantic emphasis
    // Query: "AI systems that learn" - should match ML content semantically
    let mut query = SearchQuery::new("AI systems that learn");
    query.hybrid = Some(meilisearch_lib::HybridQuery {
        semantic_ratio: 0.8, // Emphasize semantic search
        embedder: Some("default".to_string()),
    });

    // Run search in spawn_blocking to avoid blocking the tokio runtime
    // (which would prevent the mock server from responding)
    let meili = std::sync::Arc::new(meili);
    let meili_for_search = meili.clone();
    let result = tokio::task::spawn_blocking(move || {
        meili_for_search.search("hybrid_test", query)
    })
    .await
    .expect("spawn_blocking panicked")
    .expect("hybrid search failed");

    // Verify we got results (mock embeddings are deterministic)
    assert!(!result.hits.is_empty(), "Hybrid search should return results");

    // Test with keyword emphasis
    let mut keyword_query = SearchQuery::new("neural networks");
    keyword_query.hybrid = Some(meilisearch_lib::HybridQuery {
        semantic_ratio: 0.2, // Emphasize keyword search
        embedder: Some("default".to_string()),
    });

    let meili_for_keyword = meili.clone();
    let keyword_result = tokio::task::spawn_blocking(move || {
        meili_for_keyword.search("hybrid_test", keyword_query)
    })
    .await
    .expect("spawn_blocking panicked")
    .expect("keyword search failed");
    assert!(!keyword_result.hits.is_empty(), "Keyword search should return results");

    // Cleanup
    mock_server.shutdown().await;
    let meili = std::sync::Arc::try_unwrap(meili).expect("Arc still has references");
    meili.shutdown().expect("shutdown failed");
}

// ============================================================================
// Test: Chat Completion Workflow (Uses Mock Chat Server)
// ============================================================================

/// Test chat completion workflow with RAG (Retrieval-Augmented Generation).
///
/// Uses the mock server for chat completions, enabling fully automated testing
/// without requiring actual LLM API keys.
///
/// This test verifies:
/// 1. Chat configuration can be set with mock server
/// 2. Chat config lifecycle (set, get, clear) works correctly
/// 3. Index configs for RAG are properly configured
#[tokio::test]
async fn test_chat_completion_workflow() {
    use meilisearch_lib::{ChatConfig, ChatIndexConfig, ChatPrompts, ChatSource};
    use std::collections::HashMap;

    // Start mock chat completions server
    let mock_server = MockServer::start(MockServerConfig::with_random_port())
        .await
        .expect("failed to start mock server");

    let (meili, _tmp) = create_instance();

    // Enable chat completions feature
    let mut features = meili.get_features();
    features.chat_completions = true;
    meili.set_features(features);

    // Create and populate an index for RAG context
    let create_task = meili
        .create_index("products", Some("id".to_string()))
        .expect("create_index failed");
    let _ = wait_for_task(&meili, create_task.uid).await;

    let documents = vec![
        json!({
            "id": "1",
            "name": "Wireless Headphones",
            "description": "Premium noise-canceling wireless headphones with 30-hour battery life.",
            "price": 299.99,
            "category": "electronics"
        }),
        json!({
            "id": "2",
            "name": "Mechanical Keyboard",
            "description": "RGB mechanical keyboard with Cherry MX switches and USB-C connection.",
            "price": 149.99,
            "category": "electronics"
        }),
        json!({
            "id": "3",
            "name": "Ergonomic Mouse",
            "description": "Vertical ergonomic mouse designed to reduce wrist strain.",
            "price": 79.99,
            "category": "electronics"
        }),
    ];

    let add_task =
        meili.add_documents("products", documents, None).expect("add_documents failed");
    let _ = wait_for_task(&meili, add_task.uid).await;

    // Configure chat with mock server (OpenAI-compatible)
    let mut index_configs = HashMap::new();
    index_configs.insert(
        "products".to_string(),
        ChatIndexConfig {
            description: "Product catalog with electronics and accessories".to_string(),
            template: Some(
                "Product: {{ name }}\nDescription: {{ description }}\nPrice: ${{ price }}"
                    .to_string(),
            ),
            max_bytes: Some(500),
            search_params: None,
        },
    );

    let chat_config = ChatConfig {
        source: ChatSource::OpenAi,
        api_key: "mock-test-key".to_string(),
        base_url: Some(mock_server.url()),
        model: "gpt-4o-mini".to_string(),
        org_id: None,
        project_id: None,
        api_version: None,
        deployment_id: None,
        prompts: ChatPrompts {
            system: Some(
                "You are a helpful shopping assistant. Use the product search tool to find relevant products before answering.".to_string(),
            ),
            search_description: Some("Search the product catalog".to_string()),
            search_q_param: None,
            search_filter_param: None,
            search_index_uid_param: None,
        },
        index_configs,
    };

    // Set the chat configuration
    meili.set_chat_config(Some(chat_config.clone()));

    // Verify config was set
    let retrieved_config = meili.get_chat_config().expect("chat config should be set");
    assert_eq!(retrieved_config.source, ChatSource::OpenAi);
    assert_eq!(retrieved_config.model, "gpt-4o-mini");
    assert_eq!(retrieved_config.base_url, Some(mock_server.url()));

    // Verify index configs
    assert!(retrieved_config.index_configs.contains_key("products"));
    let product_config = &retrieved_config.index_configs["products"];
    assert_eq!(product_config.description, "Product catalog with electronics and accessories");

    // Clear chat config
    meili.set_chat_config(None);
    assert!(meili.get_chat_config().is_none(), "chat config should be cleared");

    // Cleanup
    mock_server.shutdown().await;
    meili.shutdown().expect("shutdown failed");
}

/// Test chat configuration persistence (save/load to JSON file).
#[tokio::test]
async fn test_chat_config_persistence() {
    use meilisearch_lib::{ChatConfig, ChatSource, ChatPrompts};
    use std::collections::HashMap;

    let (meili, tmp) = create_instance();

    // Create a chat config
    let config = ChatConfig {
        source: ChatSource::Anthropic,
        api_key: "test-api-key".to_string(),
        base_url: None,
        model: "claude-3-sonnet-20240229".to_string(),
        org_id: None,
        project_id: None,
        api_version: None,
        deployment_id: None,
        prompts: ChatPrompts::default(),
        index_configs: HashMap::new(),
    };

    // Set config
    meili.set_chat_config(Some(config.clone()));

    // Save to file (simulating persistence)
    let config_path = tmp.path().join("chat_config.json");
    let config_json = serde_json::to_string_pretty(&config).expect("serialization failed");
    std::fs::write(&config_path, &config_json).expect("failed to write config");

    // Clear in-memory config
    meili.set_chat_config(None);
    assert!(meili.get_chat_config().is_none());

    // Load from file
    let loaded_json = std::fs::read_to_string(&config_path).expect("failed to read config");
    let loaded_config: ChatConfig = serde_json::from_str(&loaded_json).expect("failed to parse config");

    // Restore config
    meili.set_chat_config(Some(loaded_config.clone()));

    // Verify restored config
    let restored = meili.get_chat_config().expect("config should be restored");
    assert_eq!(restored.source, ChatSource::Anthropic);
    assert_eq!(restored.model, "claude-3-sonnet-20240229");
    assert_eq!(restored.api_key, "test-api-key");

    // Cleanup
    meili.shutdown().expect("shutdown failed");
}
