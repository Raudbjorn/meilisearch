//! SDK-style index operation tests.
//!
//! These tests mirror the Meilisearch Rust SDK's index testing patterns,
//! providing comprehensive coverage of index CRUD operations.

mod common;

use common::{sample_movies, TestContext};
use meilisearch_lib::{Error, SearchQuery, TaskStatus};
use serde_json::json;

// ============================================================================
// Index Creation Tests
// ============================================================================

/// Test creating an index with a primary key.
#[tokio::test]
async fn test_create_index_with_primary_key() {
    let mut ctx = TestContext::new();
    let (uid, task) = ctx.create_index("movies", Some("id")).await;

    assert_eq!(task.status, TaskStatus::Succeeded);

    let index = ctx.client.get_index(&uid).expect("get_index failed");
    assert_eq!(index.uid, uid);
    assert_eq!(index.primary_key, Some("id".to_string()));

    ctx.shutdown().expect("shutdown failed");
}

/// Test creating an index without a primary key (auto-infer).
#[tokio::test]
async fn test_create_index_without_primary_key() {
    let ctx = TestContext::new();

    let uid = common::unique_index_name("no_pk");
    let task = ctx.client.create_index(&uid, None).expect("create_index failed");
    let completed = ctx.wait_for_task(task.uid).await;

    assert_eq!(completed.status, TaskStatus::Succeeded);

    let index = ctx.client.get_index(&uid).expect("get_index failed");
    assert_eq!(index.primary_key, None);

    ctx.shutdown().expect("shutdown failed");
}

/// Test that creating an index with empty UID fails.
#[tokio::test]
async fn test_create_index_empty_uid_fails() {
    let ctx = TestContext::new();

    let result = ctx.client.create_index("", None);
    assert!(matches!(result, Err(Error::InvalidIndexUid(_))));

    ctx.shutdown().expect("shutdown failed");
}

// ============================================================================
// Index Retrieval Tests
// ============================================================================

/// Test getting an existing index.
#[tokio::test]
async fn test_get_index() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("get_test").await;

    let index = ctx.client.get_index(&uid).expect("get_index failed");

    assert_eq!(index.uid, uid);
    assert!(index.created_at <= index.updated_at);

    ctx.shutdown().expect("shutdown failed");
}

/// Test getting a non-existent index returns an error.
#[tokio::test]
async fn test_get_nonexistent_index() {
    let ctx = TestContext::new();

    let result = ctx.client.get_index("nonexistent_index_xyz");
    assert!(result.is_err());

    ctx.shutdown().expect("shutdown failed");
}

/// Test checking if an index exists.
#[tokio::test]
async fn test_index_exists() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("exists_test").await;

    assert!(ctx.client.index_exists(&uid).expect("index_exists failed"));
    assert!(!ctx.client.index_exists("nonexistent").expect("index_exists failed"));

    ctx.shutdown().expect("shutdown failed");
}

// ============================================================================
// Index Listing Tests
// ============================================================================

/// Test listing all indexes.
#[tokio::test]
async fn test_list_indexes() {
    let mut ctx = TestContext::new();

    // Create multiple indexes
    let uid1 = ctx.create_index_simple("list_a").await;
    let uid2 = ctx.create_index_simple("list_b").await;
    let uid3 = ctx.create_index_simple("list_c").await;

    let (total, indexes) = ctx.client.list_indexes(0, 10).expect("list_indexes failed");

    assert!(total >= 3);
    let uids: Vec<_> = indexes.iter().map(|i| i.uid.as_str()).collect();
    assert!(uids.contains(&uid1.as_str()));
    assert!(uids.contains(&uid2.as_str()));
    assert!(uids.contains(&uid3.as_str()));

    ctx.shutdown().expect("shutdown failed");
}

/// Test listing indexes with pagination.
#[tokio::test]
async fn test_list_indexes_pagination() {
    let mut ctx = TestContext::new();

    // Create 5 indexes
    for i in 0..5 {
        ctx.create_index_simple(&format!("page_{}", i)).await;
    }

    // Get first page
    let (total, page1) = ctx.client.list_indexes(0, 2).expect("list_indexes failed");
    assert!(total >= 5);
    assert_eq!(page1.len(), 2);

    // Get second page
    let (_, page2) = ctx.client.list_indexes(2, 2).expect("list_indexes failed");
    assert_eq!(page2.len(), 2);

    // Ensure different indexes
    assert_ne!(page1[0].uid, page2[0].uid);

    ctx.shutdown().expect("shutdown failed");
}

// ============================================================================
// Index Deletion Tests
// ============================================================================

/// Test deleting an existing index.
#[tokio::test]
async fn test_delete_index() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("delete_test").await;

    // Verify exists
    assert!(ctx.client.index_exists(&uid).expect("index_exists failed"));

    // Delete
    let task = ctx.client.delete_index(&uid).expect("delete_index failed");
    let completed = ctx.wait_for_task(task.uid).await;
    assert_eq!(completed.status, TaskStatus::Succeeded);

    // Verify gone
    assert!(!ctx.client.index_exists(&uid).expect("index_exists failed"));

    ctx.shutdown().expect("shutdown failed");
}

/// Test deleting a non-existent index (task fails gracefully).
#[tokio::test]
async fn test_delete_nonexistent_index() {
    let ctx = TestContext::new();

    let task = ctx.client.delete_index("nonexistent_xyz").expect("delete_index failed");
    let completed = ctx.wait_for_task(task.uid).await;

    // The task should fail because the index doesn't exist
    assert_eq!(completed.status, TaskStatus::Failed);

    ctx.shutdown().expect("shutdown failed");
}

// ============================================================================
// Index Statistics Tests
// ============================================================================

/// Test getting index statistics.
#[tokio::test]
async fn test_index_stats() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("stats_test").await;

    // Empty index stats
    let stats = ctx.client.index_stats(&uid).expect("index_stats failed");
    assert_eq!(stats.number_of_documents, 0);
    assert!(!stats.is_indexing);
    assert!(stats.field_distribution.is_empty());

    // Add documents
    ctx.add_documents(&uid, sample_movies()).await;

    // Stats should reflect documents
    let stats_after = ctx.client.index_stats(&uid).expect("index_stats failed");
    assert_eq!(stats_after.number_of_documents, 10);
    assert!(stats_after.field_distribution.contains_key("title"));
    assert!(stats_after.field_distribution.contains_key("genres"));

    ctx.shutdown().expect("shutdown failed");
}

/// Test stats for non-existent index returns error.
#[tokio::test]
async fn test_index_stats_nonexistent() {
    let ctx = TestContext::new();

    let result = ctx.client.index_stats("nonexistent_stats");
    assert!(result.is_err());

    ctx.shutdown().expect("shutdown failed");
}

// ============================================================================
// Primary Key Tests
// ============================================================================

/// Test that primary key is inferred from first document.
#[tokio::test]
async fn test_primary_key_inference() {
    let ctx = TestContext::new();

    let uid = common::unique_index_name("pk_infer");
    let task = ctx.client.create_index(&uid, None).expect("create_index failed");
    ctx.wait_for_task(task.uid).await;

    // Primary key should be None initially
    let index = ctx.client.get_index(&uid).expect("get_index failed");
    assert_eq!(index.primary_key, None);

    // Add document with "movie_id" field
    let docs = vec![json!({"movie_id": "1", "title": "Test Movie"})];
    let add_task =
        ctx.client.add_documents(&uid, docs, Some("movie_id".to_string())).expect("add failed");
    ctx.wait_for_task(add_task.uid).await;

    // Primary key should be set
    let index_after = ctx.client.get_index(&uid).expect("get_index failed");
    assert_eq!(index_after.primary_key, Some("movie_id".to_string()));

    ctx.shutdown().expect("shutdown failed");
}

// ============================================================================
// Index Lifecycle Integration Test
// ============================================================================

/// Comprehensive test of the full index lifecycle.
#[tokio::test]
async fn test_full_index_lifecycle_sdk_style() {
    let mut ctx = TestContext::new();

    // 1. Create index
    let (uid, create_task) = ctx.create_index("lifecycle", Some("id")).await;
    assert_eq!(create_task.status, TaskStatus::Succeeded);

    // 2. Verify creation
    let index = ctx.client.get_index(&uid).expect("get_index failed");
    assert_eq!(index.uid, uid);
    assert_eq!(index.primary_key, Some("id".to_string()));

    // 3. Check stats (empty)
    let stats = ctx.client.index_stats(&uid).expect("index_stats failed");
    assert_eq!(stats.number_of_documents, 0);

    // 4. Add documents
    ctx.add_documents(&uid, sample_movies()).await;

    // 5. Verify document count
    let stats = ctx.client.index_stats(&uid).expect("index_stats failed");
    assert_eq!(stats.number_of_documents, 10);

    // 6. Search
    let query = SearchQuery::new("Matrix");
    let results = ctx.client.search(&uid, query).expect("search failed");
    assert!(!results.hits.is_empty());
    assert!(results.hits[0].document["title"].as_str().unwrap().contains("Matrix"));

    // 7. List indexes (should include ours)
    let (_, indexes) = ctx.client.list_indexes(0, 100).expect("list_indexes failed");
    assert!(indexes.iter().any(|i| i.uid == uid));

    // 8. Delete index
    let delete_task = ctx.delete_index(&uid).await;
    assert_eq!(delete_task.status, TaskStatus::Succeeded);

    // 9. Verify deletion
    assert!(!ctx.client.index_exists(&uid).expect("index_exists failed"));

    ctx.shutdown().expect("shutdown failed");
}
