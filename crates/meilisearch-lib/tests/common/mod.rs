//! Common test utilities for meilisearch-lib integration tests.
//!
//! This module provides testing utilities similar to the Meilisearch Rust SDK's
//! `#[meilisearch_test]` macro, enabling consistent test setup, index management,
//! and cleanup across all integration tests.

#![allow(dead_code)] // Many utilities are provided for future tests
#![allow(clippy::result_large_err)] // Test utilities can have large error types

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use meilisearch_lib::{Config, Error, MeilisearchLib, TaskStatus, TaskView};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tempfile::TempDir;

// ============================================================================
// Test Counter for Unique Index Names
// ============================================================================

static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Generate a unique index name for test isolation.
///
/// Each call returns a unique name like "test_idx_0", "test_idx_1", etc.
pub fn unique_index_name(prefix: &str) -> String {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("{}_{}", prefix, id)
}

// ============================================================================
// Test Instance Management
// ============================================================================

/// A test context that manages a MeilisearchLib instance and its temporary directory.
///
/// Similar to the SDK's test macro, this struct provides:
/// - Automatic database creation in a temp directory
/// - Helper methods for common operations
/// - Automatic cleanup when dropped
pub struct TestContext {
    /// The MeilisearchLib instance.
    pub client: MeilisearchLib,
    /// The temporary directory (kept alive for the duration of the test).
    _temp_dir: TempDir,
    /// Indexes created during the test (for cleanup).
    created_indexes: Vec<String>,
}

impl TestContext {
    /// Create a new test context with a fresh MeilisearchLib instance.
    pub fn new() -> Self {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let config = Config::builder()
            .db_path(temp_dir.path())
            .build()
            .expect("failed to build config");

        let client =
            MeilisearchLib::new(config).expect("failed to create MeilisearchLib instance");

        Self { client, _temp_dir: temp_dir, created_indexes: Vec::new() }
    }

    /// Create a new index with a unique name and optional primary key.
    ///
    /// Returns the index UID and the creation task.
    pub async fn create_index(
        &mut self,
        prefix: &str,
        primary_key: Option<&str>,
    ) -> (String, TaskView) {
        let uid = unique_index_name(prefix);
        let task = self
            .client
            .create_index(&uid, primary_key.map(String::from))
            .expect("failed to create index");

        let completed = self.wait_for_task(task.uid).await;
        assert_eq!(
            completed.status,
            TaskStatus::Succeeded,
            "Index creation failed: {:?}",
            completed.error
        );

        self.created_indexes.push(uid.clone());
        (uid, completed)
    }

    /// Create an index and return just the UID (convenience method).
    pub async fn create_index_simple(&mut self, prefix: &str) -> String {
        let (uid, _) = self.create_index(prefix, Some("id")).await;
        uid
    }

    /// Wait for a task to complete with a default timeout.
    pub async fn wait_for_task(&self, task_id: u32) -> TaskView {
        self.client
            .wait_for_task_async(task_id, Some(Duration::from_secs(30)))
            .await
            .expect("task wait failed")
    }

    /// Add documents to an index and wait for completion.
    pub async fn add_documents<T: Serialize>(
        &self,
        index_uid: &str,
        documents: Vec<T>,
    ) -> TaskView {
        let docs: Vec<serde_json::Value> =
            documents.into_iter().map(|d| serde_json::to_value(d).unwrap()).collect();

        let task =
            self.client.add_documents(index_uid, docs, None).expect("failed to add documents");

        let completed = self.wait_for_task(task.uid).await;
        assert_eq!(
            completed.status,
            TaskStatus::Succeeded,
            "Document addition failed: {:?}",
            completed.error
        );
        completed
    }

    /// Delete an index and wait for completion.
    pub async fn delete_index(&self, uid: &str) -> TaskView {
        let task = self.client.delete_index(uid).expect("failed to delete index");
        self.wait_for_task(task.uid).await
    }

    /// Shutdown the client (consumes self).
    pub fn shutdown(self) -> Result<(), Error> {
        self.client.shutdown()
    }
}

impl Default for TestContext {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Test Data Structures
// ============================================================================

/// A simple movie document for testing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Movie {
    pub id: String,
    pub title: String,
    pub genres: Vec<String>,
    pub year: u32,
    pub rating: f32,
}

impl Movie {
    pub fn new(id: &str, title: &str, genres: Vec<&str>, year: u32, rating: f32) -> Self {
        Self {
            id: id.to_string(),
            title: title.to_string(),
            genres: genres.into_iter().map(String::from).collect(),
            year,
            rating,
        }
    }
}

/// A simple book document for testing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Book {
    pub id: String,
    pub title: String,
    pub author: String,
    pub year: u32,
    pub pages: u32,
}

impl Book {
    pub fn new(id: &str, title: &str, author: &str, year: u32, pages: u32) -> Self {
        Self {
            id: id.to_string(),
            title: title.to_string(),
            author: author.to_string(),
            year,
            pages,
        }
    }
}

/// A product document for e-commerce testing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Product {
    pub id: String,
    pub name: String,
    pub description: String,
    pub price: f64,
    pub category: String,
    pub in_stock: bool,
}

impl Product {
    pub fn new(
        id: &str,
        name: &str,
        description: &str,
        price: f64,
        category: &str,
        in_stock: bool,
    ) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            price,
            category: category.to_string(),
            in_stock,
        }
    }
}

// ============================================================================
// Sample Data Generators
// ============================================================================

/// Generate sample movie data for testing.
pub fn sample_movies() -> Vec<Movie> {
    vec![
        Movie::new("1", "The Shawshank Redemption", vec!["Drama"], 1994, 9.3),
        Movie::new("2", "The Godfather", vec!["Crime", "Drama"], 1972, 9.2),
        Movie::new("3", "The Dark Knight", vec!["Action", "Crime", "Drama"], 2008, 9.0),
        Movie::new("4", "Pulp Fiction", vec!["Crime", "Drama"], 1994, 8.9),
        Movie::new("5", "Forrest Gump", vec!["Drama", "Romance"], 1994, 8.8),
        Movie::new("6", "Inception", vec!["Action", "Sci-Fi", "Thriller"], 2010, 8.8),
        Movie::new("7", "The Matrix", vec!["Action", "Sci-Fi"], 1999, 8.7),
        Movie::new("8", "Goodfellas", vec!["Biography", "Crime", "Drama"], 1990, 8.7),
        Movie::new("9", "Se7en", vec!["Crime", "Drama", "Mystery"], 1995, 8.6),
        Movie::new("10", "Fight Club", vec!["Drama"], 1999, 8.8),
    ]
}

/// Generate sample book data for testing.
pub fn sample_books() -> Vec<Book> {
    vec![
        Book::new("1", "The Hobbit", "J.R.R. Tolkien", 1937, 310),
        Book::new("2", "1984", "George Orwell", 1949, 328),
        Book::new("3", "Dune", "Frank Herbert", 1965, 688),
        Book::new("4", "Neuromancer", "William Gibson", 1984, 271),
        Book::new("5", "Foundation", "Isaac Asimov", 1951, 255),
    ]
}

/// Generate sample product data for testing.
pub fn sample_products() -> Vec<Product> {
    vec![
        Product::new(
            "1",
            "Wireless Headphones",
            "Premium noise-canceling wireless headphones with 30-hour battery",
            299.99,
            "electronics",
            true,
        ),
        Product::new(
            "2",
            "Mechanical Keyboard",
            "RGB mechanical keyboard with Cherry MX switches",
            149.99,
            "electronics",
            true,
        ),
        Product::new(
            "3",
            "Ergonomic Mouse",
            "Vertical ergonomic mouse for wrist comfort",
            79.99,
            "electronics",
            false,
        ),
        Product::new(
            "4",
            "USB-C Hub",
            "7-in-1 USB-C hub with HDMI and SD card reader",
            49.99,
            "accessories",
            true,
        ),
        Product::new(
            "5",
            "Monitor Stand",
            "Adjustable monitor stand with cable management",
            89.99,
            "accessories",
            true,
        ),
    ]
}

/// Generate a large batch of documents for performance testing.
pub fn generate_large_batch(count: usize) -> Vec<serde_json::Value> {
    (0..count)
        .map(|i| {
            json!({
                "id": i.to_string(),
                "title": format!("Document {}", i),
                "content": format!("This is the content of document number {}. It contains some text for indexing and searching.", i),
                "category": format!("category_{}", i % 10),
                "priority": i % 5,
                "timestamp": 1700000000 + i as i64
            })
        })
        .collect()
}

// ============================================================================
// Assertion Helpers
// ============================================================================

/// Assert that a task succeeded.
pub fn assert_task_succeeded(task: &TaskView) {
    assert_eq!(
        task.status,
        TaskStatus::Succeeded,
        "Expected task to succeed, but got {:?}. Error: {:?}",
        task.status,
        task.error
    );
}

/// Assert that a task failed.
pub fn assert_task_failed(task: &TaskView) {
    assert_eq!(
        task.status,
        TaskStatus::Failed,
        "Expected task to fail, but got {:?}",
        task.status
    );
}

/// Assert that a result is an error of the expected type.
#[macro_export]
macro_rules! assert_error {
    ($result:expr, $pattern:pat) => {
        match $result {
            Err($pattern) => {}
            Err(e) => panic!("Expected error matching {}, got: {:?}", stringify!($pattern), e),
            Ok(v) => panic!("Expected error matching {}, got Ok: {:?}", stringify!($pattern), v),
        }
    };
}

// ============================================================================
// Concurrent Test Helpers
// ============================================================================

/// Run multiple async operations concurrently and collect results.
pub async fn run_concurrent<F, T>(count: usize, mut factory: F) -> Vec<T>
where
    F: FnMut(usize) -> tokio::task::JoinHandle<T>,
    T: Send + 'static,
{
    let handles: Vec<_> = (0..count).map(&mut factory).collect();

    let mut results = Vec::with_capacity(count);
    for handle in handles {
        results.push(handle.await.expect("task panicked"));
    }
    results
}

/// Share a client across concurrent operations.
#[allow(unused_variables)]
pub fn shared_client(ctx: &TestContext) -> Arc<MeilisearchLib> {
    // Note: This is a workaround since we can't clone MeilisearchLib.
    // In real usage, you'd share via Arc from the start.
    // For tests, we recommend using separate TestContext instances
    // or restructuring tests to avoid sharing.
    unimplemented!(
        "Use Arc<MeilisearchLib> directly in concurrent tests. \
        See test_concurrent_operations for the pattern."
    )
}
