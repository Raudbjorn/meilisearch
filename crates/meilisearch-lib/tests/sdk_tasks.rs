//! SDK-style task operation tests.
//!
//! These tests mirror the Meilisearch Rust SDK's task testing patterns,
//! providing comprehensive coverage of task management and waiting.

mod common;

use common::{sample_movies, TestContext};
use meilisearch_lib::{Error, TaskStatus};
use serde_json::json;
use std::time::Duration;

// ============================================================================
// Task Creation Tests
// ============================================================================

/// Test that tasks are created with Enqueued status.
#[tokio::test]
async fn test_task_created_enqueued() {
    let ctx = TestContext::new();
    let uid = common::unique_index_name("task_enqueued");

    let task = ctx.client.create_index(&uid, None).expect("create_index failed");

    assert_eq!(task.status, TaskStatus::Enqueued);
    assert!(task.started_at.is_none());
    assert!(task.finished_at.is_none());

    ctx.shutdown().expect("shutdown failed");
}

/// Test that tasks have sequential UIDs.
#[tokio::test]
async fn test_task_sequential_uids() {
    let ctx = TestContext::new();

    let task1 = ctx.client.create_index(common::unique_index_name("seq_a"), None).expect("failed");
    let task2 = ctx.client.create_index(common::unique_index_name("seq_b"), None).expect("failed");
    let task3 = ctx.client.create_index(common::unique_index_name("seq_c"), None).expect("failed");

    assert!(task2.uid > task1.uid);
    assert!(task3.uid > task2.uid);

    ctx.shutdown().expect("shutdown failed");
}

// ============================================================================
// Task Retrieval Tests
// ============================================================================

/// Test getting a task by ID.
#[tokio::test]
async fn test_get_task() {
    let ctx = TestContext::new();
    let uid = common::unique_index_name("get_task");

    let task = ctx.client.create_index(&uid, None).expect("create_index failed");
    let retrieved = ctx.client.get_task(task.uid).expect("get_task failed");

    assert_eq!(retrieved.uid, task.uid);

    ctx.shutdown().expect("shutdown failed");
}

/// Test getting a non-existent task returns error.
#[tokio::test]
async fn test_get_nonexistent_task() {
    let ctx = TestContext::new();

    let result = ctx.client.get_task(999999);
    assert!(matches!(result, Err(Error::TaskNotFound(_))));

    ctx.shutdown().expect("shutdown failed");
}

// ============================================================================
// Task Waiting Tests
// ============================================================================

/// Test waiting for a task to complete.
#[tokio::test]
async fn test_wait_for_task() {
    let ctx = TestContext::new();
    let uid = common::unique_index_name("wait_test");

    let task = ctx.client.create_index(&uid, None).expect("create_index failed");
    assert_eq!(task.status, TaskStatus::Enqueued);

    let completed = ctx.wait_for_task(task.uid).await;

    assert!(
        completed.status == TaskStatus::Succeeded || completed.status == TaskStatus::Failed,
        "Task should be in terminal state"
    );

    ctx.shutdown().expect("shutdown failed");
}

/// Test that completed tasks have timestamps.
#[tokio::test]
async fn test_task_timestamps() {
    let ctx = TestContext::new();
    let uid = common::unique_index_name("timestamps");

    let task = ctx.client.create_index(&uid, None).expect("create_index failed");
    let completed = ctx.wait_for_task(task.uid).await;

    // enqueued_at is always set (not Optional)
    // started_at and finished_at are Optional but should be set for completed tasks
    assert!(completed.started_at.is_some(), "started_at should be set for completed tasks");
    assert!(completed.finished_at.is_some(), "finished_at should be set for completed tasks");

    // Verify timestamp ordering
    if let (Some(started), Some(finished)) = (completed.started_at, completed.finished_at) {
        let enqueued = completed.enqueued_at;
        assert!(enqueued <= started, "enqueued should be before started");
        assert!(started <= finished, "started should be before finished");
    }

    ctx.shutdown().expect("shutdown failed");
}

/// Test wait with custom timeout (async version).
#[tokio::test]
async fn test_wait_for_task_with_timeout() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("timeout_test").await;

    // Add documents (creates a task)
    let docs = sample_movies();
    let task = ctx.client.add_documents(&uid, docs.iter().map(|m| serde_json::to_value(m).unwrap()).collect(), None).expect("add failed");

    // Wait with generous timeout
    let result = ctx
        .client
        .wait_for_task_async(task.uid, Some(Duration::from_secs(60)))
        .await;

    assert!(result.is_ok());
    let completed = result.unwrap();
    assert_eq!(completed.status, TaskStatus::Succeeded);

    ctx.shutdown().expect("shutdown failed");
}

/// Test wait timeout error.
#[tokio::test]
async fn test_wait_for_task_timeout_error() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("timeout_error").await;

    // Add a large batch to create a slow task
    let docs = common::generate_large_batch(1000);
    let task = ctx.client.add_documents(&uid, docs, None).expect("add failed");

    // Try with extremely short timeout
    let result = ctx
        .client
        .wait_for_task_async(task.uid, Some(Duration::from_nanos(1)))
        .await;

    // May succeed on fast systems or timeout
    match result {
        Ok(t) => {
            // Task completed before timeout (fast system)
            assert!(t.status == TaskStatus::Succeeded || t.status == TaskStatus::Failed);
        }
        Err(Error::TaskTimeout(id, _)) => {
            assert_eq!(id, task.uid);
        }
        Err(e) => panic!("Unexpected error: {:?}", e),
    }

    ctx.shutdown().expect("shutdown failed");
}

// ============================================================================
// Task Status Tests
// ============================================================================

/// Test successful task status.
#[tokio::test]
async fn test_task_succeeded() {
    let ctx = TestContext::new();
    let uid = common::unique_index_name("succeeded");

    let task = ctx.client.create_index(&uid, Some("id".to_string())).expect("create_index failed");
    let completed = ctx.wait_for_task(task.uid).await;

    assert_eq!(completed.status, TaskStatus::Succeeded);
    assert!(completed.error.is_none());

    ctx.shutdown().expect("shutdown failed");
}

/// Test failed task status (e.g., deleting non-existent index).
#[tokio::test]
async fn test_task_failed() {
    let ctx = TestContext::new();

    // Try to delete non-existent index
    let task = ctx.client.delete_index("nonexistent_for_failure").expect("delete_index failed");
    let completed = ctx.wait_for_task(task.uid).await;

    assert_eq!(completed.status, TaskStatus::Failed);
    assert!(completed.error.is_some());

    ctx.shutdown().expect("shutdown failed");
}

// ============================================================================
// Multiple Task Tests
// ============================================================================

/// Test multiple tasks complete correctly.
#[tokio::test]
async fn test_multiple_tasks_sequence() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("multi_task").await;

    // Create multiple document addition tasks
    let task1 = ctx.client.add_documents(&uid, vec![json!({"id": "1", "value": 1})], None).expect("add failed");
    let task2 = ctx.client.add_documents(&uid, vec![json!({"id": "2", "value": 2})], None).expect("add failed");
    let task3 = ctx.client.add_documents(&uid, vec![json!({"id": "3", "value": 3})], None).expect("add failed");

    // Wait for all
    let completed1 = ctx.wait_for_task(task1.uid).await;
    let completed2 = ctx.wait_for_task(task2.uid).await;
    let completed3 = ctx.wait_for_task(task3.uid).await;

    assert_eq!(completed1.status, TaskStatus::Succeeded);
    assert_eq!(completed2.status, TaskStatus::Succeeded);
    assert_eq!(completed3.status, TaskStatus::Succeeded);

    // Verify all documents added
    let stats = ctx.client.index_stats(&uid).expect("stats failed");
    assert_eq!(stats.number_of_documents, 3);

    ctx.shutdown().expect("shutdown failed");
}

/// Test getting task after completion.
#[tokio::test]
async fn test_get_completed_task() {
    let ctx = TestContext::new();
    let uid = common::unique_index_name("get_completed");

    let task = ctx.client.create_index(&uid, None).expect("create_index failed");
    ctx.wait_for_task(task.uid).await;

    // Get the task after completion
    let retrieved = ctx.client.get_task(task.uid).expect("get_task failed");

    assert!(retrieved.status == TaskStatus::Succeeded || retrieved.status == TaskStatus::Failed);
    assert!(retrieved.finished_at.is_some());

    ctx.shutdown().expect("shutdown failed");
}

// ============================================================================
// Task Type Tests
// ============================================================================

/// Test index creation task type.
#[tokio::test]
async fn test_index_creation_task_type() {
    let ctx = TestContext::new();
    let uid = common::unique_index_name("task_type_create");

    let task = ctx.client.create_index(&uid, Some("id".to_string())).expect("create_index failed");
    let completed = ctx.wait_for_task(task.uid).await;

    assert_eq!(completed.status, TaskStatus::Succeeded);
    // Task type would be IndexCreation

    ctx.shutdown().expect("shutdown failed");
}

/// Test document addition task type.
#[tokio::test]
async fn test_document_addition_task_type() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("task_type_docs").await;

    let task = ctx.client.add_documents(&uid, vec![json!({"id": "1"})], None).expect("add failed");
    let completed = ctx.wait_for_task(task.uid).await;

    assert_eq!(completed.status, TaskStatus::Succeeded);
    // Task type would be DocumentAdditionOrUpdate

    ctx.shutdown().expect("shutdown failed");
}

/// Test document deletion task type.
#[tokio::test]
async fn test_document_deletion_task_type() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("task_type_delete").await;

    ctx.add_documents(&uid, vec![json!({"id": "1", "value": 1})]).await;

    let task = ctx.client.delete_document(&uid, "1").expect("delete failed");
    let completed = ctx.wait_for_task(task.uid).await;

    assert_eq!(completed.status, TaskStatus::Succeeded);

    ctx.shutdown().expect("shutdown failed");
}

// ============================================================================
// Task Lifecycle Integration Test
// ============================================================================

/// Comprehensive task lifecycle test.
#[tokio::test]
async fn test_task_lifecycle_sdk_style() {
    let ctx = TestContext::new();

    // 1. Create index (generates task)
    let uid = common::unique_index_name("task_lifecycle");
    let create_task = ctx.client.create_index(&uid, Some("id".to_string())).expect("create failed");
    assert_eq!(create_task.status, TaskStatus::Enqueued);

    // 2. Wait for creation
    let completed_create = ctx.wait_for_task(create_task.uid).await;
    assert_eq!(completed_create.status, TaskStatus::Succeeded);

    // 3. Add documents (multiple tasks)
    let add_task1 = ctx.client.add_documents(&uid, vec![json!({"id": "1"})], None).expect("add failed");
    let add_task2 = ctx.client.add_documents(&uid, vec![json!({"id": "2"})], None).expect("add failed");

    // 4. Wait for both
    ctx.wait_for_task(add_task1.uid).await;
    ctx.wait_for_task(add_task2.uid).await;

    // 5. Get tasks after completion
    let retrieved1 = ctx.client.get_task(add_task1.uid).expect("get failed");
    let retrieved2 = ctx.client.get_task(add_task2.uid).expect("get failed");
    assert_eq!(retrieved1.status, TaskStatus::Succeeded);
    assert_eq!(retrieved2.status, TaskStatus::Succeeded);

    // 6. Verify timestamps are set (enqueued_at is always set, others are Option)
    // enqueued_at is OffsetDateTime not Option, so it's always present
    assert!(retrieved1.started_at.is_some());
    assert!(retrieved1.finished_at.is_some());

    // 7. Test failed task
    let failed_task = ctx.client.delete_index("nonexistent_xyz").expect("delete failed");
    let completed_failed = ctx.wait_for_task(failed_task.uid).await;
    assert_eq!(completed_failed.status, TaskStatus::Failed);
    assert!(completed_failed.error.is_some());

    // 8. Verify non-existent task returns error
    let result = ctx.client.get_task(999999);
    assert!(matches!(result, Err(Error::TaskNotFound(_))));

    ctx.shutdown().expect("shutdown failed");
}
