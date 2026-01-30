//! SDK-style document operation tests.
//!
//! These tests mirror the Meilisearch Rust SDK's document testing patterns,
//! providing comprehensive coverage of document CRUD operations.

mod common;

use common::{sample_books, sample_movies, sample_products, TestContext};
use meilisearch_lib::{Error, TaskStatus};
use serde_json::json;

// ============================================================================
// Document Addition Tests
// ============================================================================

/// Test adding documents with JSON format.
#[tokio::test]
async fn test_add_documents_json() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("add_json").await;

    let movies = sample_movies();
    ctx.add_documents(&uid, movies).await;

    let stats = ctx.client.index_stats(&uid).expect("stats failed");
    assert_eq!(stats.number_of_documents, 10);

    ctx.shutdown().expect("shutdown failed");
}

/// Test adding a single document.
#[tokio::test]
async fn test_add_single_document() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("add_single").await;

    let doc = json!({"id": "1", "name": "Test Document", "value": 42});
    let task = ctx.client.add_documents(&uid, vec![doc], None).expect("add failed");
    ctx.wait_for_task(task.uid).await;

    let stats = ctx.client.index_stats(&uid).expect("stats failed");
    assert_eq!(stats.number_of_documents, 1);

    ctx.shutdown().expect("shutdown failed");
}

/// Test adding documents with custom primary key.
#[tokio::test]
async fn test_add_documents_with_primary_key() {
    let ctx = TestContext::new();

    let uid = common::unique_index_name("custom_pk");
    let task = ctx.client.create_index(&uid, None).expect("create failed");
    ctx.wait_for_task(task.uid).await;

    let docs = vec![
        json!({"custom_id": "a", "value": 1}),
        json!({"custom_id": "b", "value": 2}),
    ];

    let add_task = ctx
        .client
        .add_documents(&uid, docs, Some("custom_id".to_string()))
        .expect("add failed");
    let completed = ctx.wait_for_task(add_task.uid).await;

    assert_eq!(completed.status, TaskStatus::Succeeded);

    let index = ctx.client.get_index(&uid).expect("get_index failed");
    assert_eq!(index.primary_key, Some("custom_id".to_string()));

    ctx.shutdown().expect("shutdown failed");
}

/// Test adding documents incrementally.
#[tokio::test]
async fn test_add_documents_incremental() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("incremental").await;

    // Add first batch
    let batch1 = vec![json!({"id": "1", "name": "First"})];
    ctx.add_documents(&uid, batch1).await;

    let stats = ctx.client.index_stats(&uid).expect("stats failed");
    assert_eq!(stats.number_of_documents, 1);

    // Add second batch
    let batch2 = vec![json!({"id": "2", "name": "Second"}), json!({"id": "3", "name": "Third"})];
    ctx.add_documents(&uid, batch2).await;

    let stats = ctx.client.index_stats(&uid).expect("stats failed");
    assert_eq!(stats.number_of_documents, 3);

    ctx.shutdown().expect("shutdown failed");
}

// ============================================================================
// Document Update Tests
// ============================================================================

/// Test updating existing documents (partial update).
#[tokio::test]
async fn test_update_documents() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("update_test").await;

    // Add initial document
    let initial = vec![json!({"id": "1", "name": "Original", "count": 10, "extra": "preserved"})];
    ctx.add_documents(&uid, initial).await;

    // Update with partial data
    let update = vec![json!({"id": "1", "name": "Updated"})];
    let task = ctx.client.update_documents(&uid, update, None).expect("update failed");
    ctx.wait_for_task(task.uid).await;

    // Verify partial update preserved other fields
    let doc = ctx.client.get_document(&uid, "1").expect("get failed");
    assert_eq!(doc["name"], "Updated");
    assert_eq!(doc["count"], 10); // Should be preserved
    assert_eq!(doc["extra"], "preserved"); // Should be preserved

    ctx.shutdown().expect("shutdown failed");
}

/// Test that add_documents replaces entire documents.
#[tokio::test]
async fn test_add_documents_replaces() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("replace_test").await;

    // Add initial document
    let initial = vec![json!({"id": "1", "name": "Original", "extra_field": "will_be_lost"})];
    ctx.add_documents(&uid, initial).await;

    // Add replacement (full replace)
    let replacement = vec![json!({"id": "1", "name": "Replaced"})];
    ctx.add_documents(&uid, replacement).await;

    // Verify full replacement (extra_field should be gone)
    let doc = ctx.client.get_document(&uid, "1").expect("get failed");
    assert_eq!(doc["name"], "Replaced");
    assert!(doc.get("extra_field").is_none() || doc["extra_field"].is_null());

    ctx.shutdown().expect("shutdown failed");
}

// ============================================================================
// Document Retrieval Tests
// ============================================================================

/// Test getting a single document by ID.
#[tokio::test]
async fn test_get_document() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("get_doc").await;

    ctx.add_documents(&uid, sample_movies()).await;

    let doc = ctx.client.get_document(&uid, "3").expect("get_document failed");

    assert_eq!(doc["id"], "3");
    assert_eq!(doc["title"], "The Dark Knight");

    ctx.shutdown().expect("shutdown failed");
}

/// Test getting a non-existent document.
#[tokio::test]
async fn test_get_nonexistent_document() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("get_nonexistent").await;

    ctx.add_documents(&uid, sample_movies()).await;

    let result = ctx.client.get_document(&uid, "999");
    assert!(matches!(result, Err(Error::DocumentNotFound(_))));

    ctx.shutdown().expect("shutdown failed");
}

/// Test getting documents with pagination.
#[tokio::test]
async fn test_get_documents_with_pagination() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("get_docs_page").await;

    ctx.add_documents(&uid, sample_movies()).await;

    // Get first page
    let (total, docs) = ctx.client.get_documents(&uid, 0, 3).expect("get_documents failed");
    assert_eq!(total, 10);
    assert_eq!(docs.len(), 3);

    // Get second page
    let (total2, docs2) = ctx.client.get_documents(&uid, 3, 3).expect("get_documents failed");
    assert_eq!(total2, 10);
    assert_eq!(docs2.len(), 3);

    // Ensure different documents
    assert_ne!(docs[0]["id"], docs2[0]["id"]);

    ctx.shutdown().expect("shutdown failed");
}

/// Test getting documents with offset beyond count.
#[tokio::test]
async fn test_get_documents_offset_beyond() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("offset_beyond").await;

    ctx.add_documents(&uid, sample_movies()).await;

    let (total, docs) = ctx.client.get_documents(&uid, 100, 10).expect("get_documents failed");
    assert_eq!(total, 10);
    assert!(docs.is_empty());

    ctx.shutdown().expect("shutdown failed");
}

// ============================================================================
// Document Deletion Tests
// ============================================================================

/// Test deleting a single document.
#[tokio::test]
async fn test_delete_document() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("delete_single").await;

    ctx.add_documents(&uid, sample_movies()).await;

    // Verify document exists
    let doc = ctx.client.get_document(&uid, "5").expect("get failed");
    assert_eq!(doc["title"], "Forrest Gump");

    // Delete document
    let task = ctx.client.delete_document(&uid, "5").expect("delete failed");
    ctx.wait_for_task(task.uid).await;

    // Verify deleted
    let result = ctx.client.get_document(&uid, "5");
    assert!(matches!(result, Err(Error::DocumentNotFound(_))));

    // Verify count decreased
    let stats = ctx.client.index_stats(&uid).expect("stats failed");
    assert_eq!(stats.number_of_documents, 9);

    ctx.shutdown().expect("shutdown failed");
}

/// Test batch deleting documents by IDs.
#[tokio::test]
async fn test_delete_documents_batch() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("delete_batch").await;

    ctx.add_documents(&uid, sample_movies()).await;

    // Delete multiple documents
    let ids = vec!["1".to_string(), "3".to_string(), "5".to_string()];
    let task = ctx.client.delete_documents_batch(&uid, ids).expect("batch delete failed");
    ctx.wait_for_task(task.uid).await;

    // Verify deletions
    let stats = ctx.client.index_stats(&uid).expect("stats failed");
    assert_eq!(stats.number_of_documents, 7);

    assert!(matches!(ctx.client.get_document(&uid, "1"), Err(Error::DocumentNotFound(_))));
    assert!(matches!(ctx.client.get_document(&uid, "3"), Err(Error::DocumentNotFound(_))));
    assert!(matches!(ctx.client.get_document(&uid, "5"), Err(Error::DocumentNotFound(_))));

    // Verify others remain
    assert!(ctx.client.get_document(&uid, "2").is_ok());
    assert!(ctx.client.get_document(&uid, "4").is_ok());

    ctx.shutdown().expect("shutdown failed");
}

/// Test deleting all documents.
#[tokio::test]
async fn test_delete_all_documents() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("delete_all").await;

    ctx.add_documents(&uid, sample_movies()).await;

    // Verify documents exist
    let stats = ctx.client.index_stats(&uid).expect("stats failed");
    assert_eq!(stats.number_of_documents, 10);

    // Delete all
    let task = ctx.client.delete_all_documents(&uid).expect("delete_all failed");
    ctx.wait_for_task(task.uid).await;

    // Verify empty but index still exists
    let stats = ctx.client.index_stats(&uid).expect("stats failed");
    assert_eq!(stats.number_of_documents, 0);
    assert!(ctx.client.index_exists(&uid).expect("exists failed"));

    ctx.shutdown().expect("shutdown failed");
}

// ============================================================================
// Large Batch Tests
// ============================================================================

/// Test adding a large batch of documents.
#[tokio::test]
async fn test_add_large_batch() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("large_batch").await;

    let docs = common::generate_large_batch(500);
    let task = ctx.client.add_documents(&uid, docs, None).expect("add failed");
    ctx.wait_for_task(task.uid).await;

    let stats = ctx.client.index_stats(&uid).expect("stats failed");
    assert_eq!(stats.number_of_documents, 500);

    ctx.shutdown().expect("shutdown failed");
}

// ============================================================================
// Document Type Tests
// ============================================================================

/// Test with different document types (typed structs).
#[tokio::test]
async fn test_various_document_types() {
    let mut ctx = TestContext::new();

    // Movies
    let movies_uid = ctx.create_index_simple("typed_movies").await;
    ctx.add_documents(&movies_uid, sample_movies()).await;
    let stats = ctx.client.index_stats(&movies_uid).expect("stats failed");
    assert_eq!(stats.number_of_documents, 10);

    // Books
    let books_uid = ctx.create_index_simple("typed_books").await;
    ctx.add_documents(&books_uid, sample_books()).await;
    let stats = ctx.client.index_stats(&books_uid).expect("stats failed");
    assert_eq!(stats.number_of_documents, 5);

    // Products
    let products_uid = ctx.create_index_simple("typed_products").await;
    ctx.add_documents(&products_uid, sample_products()).await;
    let stats = ctx.client.index_stats(&products_uid).expect("stats failed");
    assert_eq!(stats.number_of_documents, 5);

    ctx.shutdown().expect("shutdown failed");
}

// ============================================================================
// Document Lifecycle Integration Test
// ============================================================================

/// Comprehensive document lifecycle test.
#[tokio::test]
async fn test_document_lifecycle_sdk_style() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("doc_lifecycle").await;

    // 1. Add initial documents
    let initial = vec![
        json!({"id": "1", "name": "Alice", "score": 100}),
        json!({"id": "2", "name": "Bob", "score": 85}),
        json!({"id": "3", "name": "Charlie", "score": 90}),
    ];
    ctx.add_documents(&uid, initial).await;

    // 2. Verify count
    let stats = ctx.client.index_stats(&uid).expect("stats failed");
    assert_eq!(stats.number_of_documents, 3);

    // 3. Get specific document
    let alice = ctx.client.get_document(&uid, "1").expect("get failed");
    assert_eq!(alice["name"], "Alice");

    // 4. Update a document (partial)
    let update = vec![json!({"id": "2", "score": 95})];
    let task = ctx.client.update_documents(&uid, update, None).expect("update failed");
    ctx.wait_for_task(task.uid).await;

    let bob = ctx.client.get_document(&uid, "2").expect("get failed");
    assert_eq!(bob["name"], "Bob"); // Preserved
    assert_eq!(bob["score"], 95); // Updated

    // 5. Add more documents
    let more = vec![json!({"id": "4", "name": "Diana", "score": 92})];
    ctx.add_documents(&uid, more).await;

    let stats = ctx.client.index_stats(&uid).expect("stats failed");
    assert_eq!(stats.number_of_documents, 4);

    // 6. Delete one document
    let task = ctx.client.delete_document(&uid, "3").expect("delete failed");
    ctx.wait_for_task(task.uid).await;

    let stats = ctx.client.index_stats(&uid).expect("stats failed");
    assert_eq!(stats.number_of_documents, 3);

    // 7. Get all documents
    let (total, docs) = ctx.client.get_documents(&uid, 0, 10).expect("get_documents failed");
    assert_eq!(total, 3);
    assert_eq!(docs.len(), 3);

    // 8. Delete all
    let task = ctx.client.delete_all_documents(&uid).expect("delete_all failed");
    ctx.wait_for_task(task.uid).await;

    let stats = ctx.client.index_stats(&uid).expect("stats failed");
    assert_eq!(stats.number_of_documents, 0);

    ctx.shutdown().expect("shutdown failed");
}
