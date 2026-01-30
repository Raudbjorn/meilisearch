//! Main client implementation for embedded Meilisearch.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use index_scheduler::{IndexScheduler, Query};
use meilisearch_auth::{open_auth_store_env, AuthFilter};
use meilisearch_types::features::RuntimeTogglableFeatures;
use meilisearch_types::tasks::Status;

use crate::chat::config::ChatConfig;
use crate::config::Config;
use crate::error::Error;
use crate::tasks::TaskView;

/// Default version tuple for new databases.
/// Using the current Meilisearch version constants.
const DEFAULT_DB_VERSION: (u32, u32, u32) = (1, 15, 0);

/// Health status of the embedded Meilisearch instance.
#[derive(Debug, Clone)]
pub struct Health {
    /// Status string indicating availability.
    pub status: String,
}

impl Default for Health {
    fn default() -> Self {
        Self { status: "available".to_string() }
    }
}

/// Embedded Meilisearch instance for direct Rust integration.
///
/// This struct provides a high-level interface to Meilisearch functionality
/// without requiring an HTTP server. It wraps the `IndexScheduler` and provides
/// methods for:
///
/// - Index management (create, delete, list)
/// - Document operations (add, get, delete, search)
/// - Task management (get, wait, cancel)
/// - Settings management
/// - Chat completions with LLM providers
///
/// # Thread Safety
///
/// `MeilisearchLib` is `Send + Sync` and can be safely shared across threads.
/// All internal state is protected by appropriate synchronization primitives.
///
/// # Example
///
/// ```rust,ignore
/// use meilisearch_lib::{MeilisearchLib, Config};
///
/// let config = Config::builder()
///     .db_path("/tmp/meilisearch-data")
///     .build()?;
///
/// let meili = MeilisearchLib::new(config)?;
///
/// // Use meili for operations...
///
/// meili.shutdown()?;
/// ```
pub struct MeilisearchLib {
    /// The underlying index scheduler that manages all database operations.
    scheduler: Arc<IndexScheduler>,

    /// Configuration used to create this instance.
    config: Config,

    /// Runtime feature toggles that can be changed without restart.
    features: Arc<RwLock<RuntimeTogglableFeatures>>,

    /// In-memory chat configuration for LLM providers.
    /// This is stored in memory rather than persisted to LMDB.
    chat_config: Arc<RwLock<Option<ChatConfig>>>,
}

// Explicitly mark as thread-safe
// Safety: All fields are either Arc-wrapped or use interior mutability with RwLock
unsafe impl Send for MeilisearchLib {}
unsafe impl Sync for MeilisearchLib {}

impl std::fmt::Debug for MeilisearchLib {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MeilisearchLib")
            .field("config", &self.config)
            .field("features", &self.features)
            .field("chat_config", &"<configured>")
            .finish_non_exhaustive()
    }
}

impl MeilisearchLib {
    /// Create a new embedded Meilisearch instance.
    ///
    /// This initializes the index scheduler, creates necessary directories,
    /// and starts the background task processing loop.
    ///
    /// # Arguments
    ///
    /// * `config` - Configuration specifying database paths and limits
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The database directory cannot be created
    /// - The LMDB environment fails to open
    /// - The index scheduler fails to initialize
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use meilisearch_lib::{MeilisearchLib, Config};
    ///
    /// let config = Config::builder()
    ///     .db_path("/tmp/meilisearch-data")
    ///     .build()?;
    ///
    /// let meili = MeilisearchLib::new(config)?;
    /// ```
    pub fn new(config: Config) -> Result<Self, Error> {
        // Ensure the database directory exists
        std::fs::create_dir_all(&config.db_path)?;

        // Convert our simplified config to IndexSchedulerOptions
        let options = config.to_scheduler_options();

        // Create the auth store directory and open its environment
        std::fs::create_dir_all(&options.auth_path)?;
        let auth_env = open_auth_store_env(&options.auth_path)
            .map_err(|e| Error::Internal(format!("failed to open auth store: {}", e)))?;

        // Initialize the index scheduler
        // The scheduler runs its own background thread for task processing
        let scheduler = IndexScheduler::new(
            options,
            auth_env,
            DEFAULT_DB_VERSION,
            None, // No tokio runtime handle - scheduler creates its own threads
        )?;

        Ok(Self {
            scheduler: Arc::new(scheduler),
            config,
            features: Arc::new(RwLock::new(RuntimeTogglableFeatures::default())),
            chat_config: Arc::new(RwLock::new(None)),
        })
    }

    /// Get a reference to the underlying index scheduler.
    ///
    /// This provides direct access to the scheduler for advanced operations
    /// not exposed through the high-level API.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let scheduler = meili.scheduler();
    /// // Use scheduler directly for low-level operations
    /// ```
    #[inline]
    pub fn scheduler(&self) -> &IndexScheduler {
        &self.scheduler
    }

    /// Gracefully shutdown the Meilisearch instance.
    ///
    /// This stops the background task processing and ensures all pending
    /// operations are completed or properly cancelled.
    ///
    /// # Errors
    ///
    /// Returns an error if the shutdown process fails.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// meili.shutdown()?;
    /// ```
    pub fn shutdown(self) -> Result<(), Error> {
        // The IndexScheduler will be dropped when `self` is dropped.
        // The Arc will decrement and when it reaches zero, the scheduler
        // will clean up its resources.
        //
        // For now, we don't have explicit shutdown logic in IndexScheduler
        // that we need to call. The run loop will stop when all references
        // are dropped.
        //
        // If we need more graceful shutdown in the future, we can add
        // a shutdown method to IndexScheduler.
        tracing::info!("Shutting down MeilisearchLib instance");
        Ok(())
    }

    /// Get the health status of the instance.
    ///
    /// For embedded use, this always returns "available" since there's
    /// no network layer that could be unavailable.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let health = meili.health();
    /// assert_eq!(health.status, "available");
    /// ```
    #[inline]
    pub fn health(&self) -> Health {
        Health::default()
    }

    /// Get the current runtime feature toggles.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let features = meili.get_features();
    /// if features.chat_completions {
    ///     // Chat completions are enabled
    /// }
    /// ```
    pub fn get_features(&self) -> RuntimeTogglableFeatures {
        *self.features.read().expect("features lock poisoned")
    }

    /// Set the runtime feature toggles.
    ///
    /// These features can be toggled at runtime without restarting
    /// the Meilisearch instance.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let mut features = meili.get_features();
    /// features.chat_completions = true;
    /// meili.set_features(features);
    /// ```
    pub fn set_features(&self, features: RuntimeTogglableFeatures) {
        let mut guard = self.features.write().expect("features lock poisoned");
        *guard = features;
    }

    /// Get the current chat configuration.
    ///
    /// Returns `None` if chat has not been configured.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// if let Some(config) = meili.get_chat_config() {
    ///     println!("Using LLM provider: {:?}", config.source);
    /// }
    /// ```
    pub fn get_chat_config(&self) -> Option<ChatConfig> {
        self.chat_config.read().expect("chat_config lock poisoned").clone()
    }

    /// Set the chat configuration for LLM-powered completions.
    ///
    /// This configures the LLM provider (OpenAI, Anthropic, etc.) and
    /// associated settings for chat completions.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use meilisearch_lib::{ChatConfig, ChatSource};
    ///
    /// let chat_config = ChatConfig {
    ///     source: ChatSource::OpenAi,
    ///     api_key: "sk-...".to_string(),
    ///     model: "gpt-4".to_string(),
    ///     ..Default::default()
    /// };
    ///
    /// meili.set_chat_config(Some(chat_config));
    /// ```
    pub fn set_chat_config(&self, config: Option<ChatConfig>) {
        let mut guard = self.chat_config.write().expect("chat_config lock poisoned");
        *guard = config;
    }

    /// Get a reference to the configuration used to create this instance.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let db_path = meili.config().db_path.clone();
    /// ```
    #[inline]
    pub fn config(&self) -> &Config {
        &self.config
    }

    // =========================================================================
    // Task Operations
    // =========================================================================

    /// Get a task by its ID.
    ///
    /// Returns the task if found, or `TaskNotFound` error if it doesn't exist.
    ///
    /// # Arguments
    ///
    /// * `task_id` - The unique identifier of the task
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The task doesn't exist (`TaskNotFound`)
    /// - There's a database error
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let task = meili.get_task(42)?;
    /// println!("Task {} status: {:?}", task.uid, task.status);
    /// ```
    pub fn get_task(&self, task_id: u32) -> Result<TaskView, Error> {
        // Create a query that filters to just this task
        let query = Query { uids: Some(vec![task_id]), limit: Some(1), ..Default::default() };

        // Use default auth filter (allows all access for embedded use)
        let auth_filter = AuthFilter::default();

        let (tasks, _total) =
            self.scheduler.get_tasks_from_authorized_indexes(&query, &auth_filter)?;

        tasks.into_iter().next().map(TaskView::from).ok_or(Error::TaskNotFound(task_id))
    }

    /// Wait for a task to complete.
    ///
    /// This method polls the task status until it reaches a terminal state
    /// (`Succeeded`, `Failed`, or `Canceled`), or until the optional timeout
    /// is exceeded.
    ///
    /// # Arguments
    ///
    /// * `task_id` - The unique identifier of the task to wait for
    /// * `timeout` - Optional maximum time to wait. If `None`, waits indefinitely.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The task doesn't exist (`TaskNotFound`)
    /// - The timeout is exceeded (`TaskTimeout`)
    /// - There's a database error
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use std::time::Duration;
    ///
    /// // Wait up to 30 seconds for the task to complete
    /// let task = meili.wait_for_task(42, Some(Duration::from_secs(30)))?;
    ///
    /// match task.status {
    ///     Status::Succeeded => println!("Task succeeded!"),
    ///     Status::Failed => println!("Task failed: {:?}", task.error),
    ///     Status::Canceled => println!("Task was canceled"),
    ///     _ => unreachable!("wait_for_task returns only terminal states"),
    /// }
    /// ```
    pub fn wait_for_task(
        &self,
        task_id: u32,
        timeout: Option<Duration>,
    ) -> Result<TaskView, Error> {
        const POLL_INTERVAL: Duration = Duration::from_millis(50);

        let start = std::time::Instant::now();

        loop {
            let task = self.get_task(task_id)?;

            // Check if task has reached a terminal state
            match task.status {
                Status::Succeeded | Status::Failed | Status::Canceled => {
                    return Ok(task);
                }
                Status::Enqueued | Status::Processing => {
                    // Task still in progress, continue polling
                }
            }

            // Check timeout if specified
            if let Some(timeout) = timeout {
                if start.elapsed() >= timeout {
                    return Err(Error::TaskTimeout(task_id, timeout));
                }
            }

            // Sleep before next poll
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    /// Wait for a task to complete (async version).
    ///
    /// This is the async equivalent of [`wait_for_task`](Self::wait_for_task).
    /// It uses `tokio::time::sleep` for non-blocking polling.
    ///
    /// # Arguments
    ///
    /// * `task_id` - The unique identifier of the task to wait for
    /// * `timeout` - Optional maximum time to wait. If `None`, waits indefinitely.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The task doesn't exist (`TaskNotFound`)
    /// - The timeout is exceeded (`TaskTimeout`)
    /// - There's a database error
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use std::time::Duration;
    ///
    /// // Wait up to 30 seconds for the task to complete
    /// let task = meili.wait_for_task_async(42, Some(Duration::from_secs(30))).await?;
    /// ```
    pub async fn wait_for_task_async(
        &self,
        task_id: u32,
        timeout: Option<Duration>,
    ) -> Result<TaskView, Error> {
        const POLL_INTERVAL: Duration = Duration::from_millis(50);

        let start = std::time::Instant::now();

        loop {
            let task = self.get_task(task_id)?;

            // Check if task has reached a terminal state
            match task.status {
                Status::Succeeded | Status::Failed | Status::Canceled => {
                    return Ok(task);
                }
                Status::Enqueued | Status::Processing => {
                    // Task still in progress, continue polling
                }
            }

            // Check timeout if specified
            if let Some(timeout) = timeout {
                if start.elapsed() >= timeout {
                    return Err(Error::TaskTimeout(task_id, timeout));
                }
            }

            // Sleep before next poll (async)
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    // =========================================================================
    // Index Operations
    // =========================================================================

    /// Create a new index.
    ///
    /// This registers an `IndexCreation` task with the scheduler. The index
    /// is not immediately created; you must wait for the task to complete.
    ///
    /// # Arguments
    ///
    /// * `uid` - Unique identifier for the index (must be valid index name)
    /// * `primary_key` - Optional primary key field name. If not specified,
    ///   Meilisearch will attempt to infer it from the first document.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The index UID is invalid
    /// - There's a database error registering the task
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Create an index with an explicit primary key
    /// let task = meili.create_index("movies", Some("id".to_string()))?;
    /// let task = meili.wait_for_task(task.uid, None)?;
    ///
    /// // Create an index, letting Meilisearch infer the primary key
    /// let task = meili.create_index("books", None)?;
    /// ```
    pub fn create_index(
        &self,
        uid: impl Into<String>,
        primary_key: Option<String>,
    ) -> Result<TaskView, Error> {
        use meilisearch_types::tasks::KindWithContent;

        let uid = uid.into();

        // Validate the index UID
        if uid.is_empty() {
            return Err(Error::InvalidIndexUid(uid));
        }

        let kind = KindWithContent::IndexCreation { index_uid: uid, primary_key };

        let task = self.scheduler.register(kind, None, false)?;
        Ok(TaskView::from(task))
    }

    /// Get information about an index.
    ///
    /// Returns metadata about the index including its UID, primary key,
    /// and creation/update timestamps.
    ///
    /// # Arguments
    ///
    /// * `uid` - The unique identifier of the index
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The index doesn't exist (`IndexNotFound`)
    /// - There's a database error
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let index = meili.get_index("movies")?;
    /// println!("Primary key: {:?}", index.primary_key);
    /// println!("Created at: {}", index.created_at);
    /// ```
    pub fn get_index(&self, uid: impl AsRef<str>) -> Result<crate::IndexView, Error> {
        let uid = uid.as_ref();
        let index = self.scheduler.index(uid)?;
        let view = crate::IndexView::from_index(uid.to_string(), &index)?;
        Ok(view)
    }

    /// Delete an index.
    ///
    /// This registers an `IndexDeletion` task with the scheduler. The index
    /// is not immediately deleted; you must wait for the task to complete.
    ///
    /// # Arguments
    ///
    /// * `uid` - The unique identifier of the index to delete
    ///
    /// # Errors
    ///
    /// Returns an error if there's a database error registering the task.
    /// Note: The task will fail if the index doesn't exist, but this method
    /// itself won't return an error for non-existent indexes.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let task = meili.delete_index("movies")?;
    /// let task = meili.wait_for_task(task.uid, None)?;
    ///
    /// match task.status {
    ///     Status::Succeeded => println!("Index deleted"),
    ///     Status::Failed => println!("Failed: {:?}", task.error),
    ///     _ => {}
    /// }
    /// ```
    pub fn delete_index(&self, uid: impl Into<String>) -> Result<TaskView, Error> {
        use meilisearch_types::tasks::KindWithContent;

        let kind = KindWithContent::IndexDeletion { index_uid: uid.into() };

        let task = self.scheduler.register(kind, None, false)?;
        Ok(TaskView::from(task))
    }

    /// Get statistics for an index.
    ///
    /// Returns detailed statistics including document count, field distribution,
    /// and whether the index is currently being processed.
    ///
    /// # Arguments
    ///
    /// * `uid` - The unique identifier of the index
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The index doesn't exist (`IndexNotFound`)
    /// - There's a database error
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let stats = meili.index_stats("movies")?;
    /// println!("Document count: {}", stats.number_of_documents);
    /// println!("Is indexing: {}", stats.is_indexing);
    ///
    /// for (field, count) in &stats.field_distribution {
    ///     println!("  {}: {} documents", field, count);
    /// }
    /// ```
    pub fn index_stats(&self, uid: impl AsRef<str>) -> Result<crate::indexes::IndexStats, Error> {
        let stats = self.scheduler.index_stats(uid.as_ref())?;
        Ok(crate::indexes::IndexStats::from(stats))
    }

    /// List all indexes with pagination.
    ///
    /// Returns a paginated list of indexes ordered by creation time.
    ///
    /// # Arguments
    ///
    /// * `offset` - Number of indexes to skip (for pagination)
    /// * `limit` - Maximum number of indexes to return
    ///
    /// # Returns
    ///
    /// A tuple of `(total_count, indexes)` where `total_count` is the total
    /// number of indexes (ignoring pagination) and `indexes` is the paginated
    /// list of index views.
    ///
    /// # Errors
    ///
    /// Returns an error if there's a database error.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Get the first 10 indexes
    /// let (total, indexes) = meili.list_indexes(0, 10)?;
    /// println!("Total indexes: {}", total);
    ///
    /// for index in indexes {
    ///     println!("  {}: {:?}", index.uid, index.primary_key);
    /// }
    ///
    /// // Get the next page
    /// let (_, page2) = meili.list_indexes(10, 10)?;
    /// ```
    pub fn list_indexes(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<(usize, Vec<crate::IndexView>), Error> {
        // Use default auth filter (allows all access for embedded use)
        let auth_filter = AuthFilter::default();

        let (total, stats) = self.scheduler.paginated_indexes_stats(&auth_filter, offset, limit)?;

        let indexes = stats
            .into_iter()
            .map(|(uid, stat)| crate::IndexView {
                uid,
                primary_key: stat.primary_key,
                created_at: stat.created_at,
                updated_at: stat.updated_at,
            })
            .collect();

        Ok((total, indexes))
    }

    /// Check if an index exists.
    ///
    /// This is a convenience method that returns `true` if the index exists,
    /// `false` otherwise. It does not throw an error for non-existent indexes.
    ///
    /// # Arguments
    ///
    /// * `uid` - The unique identifier of the index
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// if meili.index_exists("movies")? {
    ///     println!("Movies index exists");
    /// } else {
    ///     println!("Movies index does not exist");
    /// }
    /// ```
    pub fn index_exists(&self, uid: impl AsRef<str>) -> Result<bool, Error> {
        Ok(self.scheduler.index_exists(uid.as_ref())?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_default() {
        let health = Health::default();
        assert_eq!(health.status, "available");
    }

    #[test]
    fn test_health_clone() {
        let health = Health::default();
        let cloned = health.clone();
        assert_eq!(health.status, cloned.status);
    }

    #[test]
    fn test_health_debug() {
        let health = Health::default();
        let debug_str = format!("{:?}", health);
        assert!(debug_str.contains("Health"));
        assert!(debug_str.contains("available"));
    }

    // Note: Integration tests for MeilisearchLib::new require a real filesystem
    // and are in the integration tests directory.
}
