//! Configuration for embedded Meilisearch instance.

use std::path::PathBuf;
use std::sync::Arc;

use byte_unit::Byte;
use index_scheduler::IndexSchedulerOptions;
use meilisearch_types::features::InstanceTogglableFeatures;
use meilisearch_types::milli::update::IndexerConfig;

/// Default maximum index size: 100 GiB
const DEFAULT_MAX_INDEX_SIZE: usize = 100 * 1024 * 1024 * 1024;

/// Default maximum task database size: 10 GiB
const DEFAULT_MAX_TASK_DB_SIZE: usize = 10 * 1024 * 1024 * 1024;

/// Default index growth amount: 10 GiB
const DEFAULT_INDEX_GROWTH_AMOUNT: usize = 10 * 1024 * 1024 * 1024;

/// Default maximum number of concurrent indexes in memory
const DEFAULT_INDEX_COUNT: usize = 20;

/// Default maximum number of tasks
const DEFAULT_MAX_NUMBER_OF_TASKS: usize = 1_000_000;

/// Default maximum number of batched tasks
const DEFAULT_MAX_NUMBER_OF_BATCHED_TASKS: usize = usize::MAX;

/// Default batched tasks size limit: 10 GiB
const DEFAULT_BATCHED_TASKS_SIZE_LIMIT: u64 = 10 * 1024 * 1024 * 1024;

/// Default export payload size: 20 MiB
const DEFAULT_EXPORT_PAYLOAD_SIZE_BYTES: u64 = 20 * 1024 * 1024;

/// Default embedding cache capacity
const DEFAULT_EMBEDDING_CACHE_CAP: usize = 1000;

/// Version file name in the database directory
const VERSION_FILE_NAME: &str = "VERSION";

/// Configuration for embedded Meilisearch instance.
#[derive(Debug, Clone)]
pub struct Config {
    /// Path to the database directory
    pub db_path: PathBuf,
    /// Maximum size of index databases (default: 100 GiB)
    pub max_index_size: usize,
    /// Maximum size of task database (default: 10 GiB)
    pub max_task_db_size: usize,
}

impl Config {
    /// Create a new configuration builder.
    pub fn builder() -> ConfigBuilder {
        ConfigBuilder::default()
    }

    /// Convert this configuration to `IndexSchedulerOptions`.
    ///
    /// This method generates all the necessary paths and reasonable defaults
    /// required by the index scheduler based on the simplified library configuration.
    pub fn to_scheduler_options(&self) -> IndexSchedulerOptions {
        IndexSchedulerOptions {
            // Path configuration - all paths derive from db_path
            version_file_path: self.db_path.join(VERSION_FILE_NAME),
            auth_path: self.db_path.join("auth"),
            tasks_path: self.db_path.join("tasks"),
            update_file_path: self.db_path.join("update_files"),
            indexes_path: self.db_path.join("indexes"),
            snapshots_path: self.db_path.join("snapshots"),
            dumps_path: self.db_path.join("dumps"),

            // Webhook configuration - disabled by default for embedded use
            cli_webhook_url: None,
            cli_webhook_authorization: None,

            // Database size configuration
            task_db_size: self.max_task_db_size,
            index_base_map_size: self.max_index_size,

            // LMDB configuration
            enable_mdb_writemap: false,
            index_growth_amount: DEFAULT_INDEX_GROWTH_AMOUNT,
            index_count: DEFAULT_INDEX_COUNT,

            // Indexer configuration - use defaults
            indexer_config: Arc::new(IndexerConfig::default()),

            // Batching configuration - enable autobatching and cleanup
            autobatching_enabled: true,
            cleanup_enabled: true,
            max_number_of_tasks: DEFAULT_MAX_NUMBER_OF_TASKS,
            max_number_of_batched_tasks: DEFAULT_MAX_NUMBER_OF_BATCHED_TASKS,
            batched_tasks_size_limit: DEFAULT_BATCHED_TASKS_SIZE_LIMIT,

            // Export configuration
            export_default_payload_size_bytes: Byte::from_u64(DEFAULT_EXPORT_PAYLOAD_SIZE_BYTES),

            // Feature flags - all disabled by default for embedded use
            instance_features: InstanceTogglableFeatures::default(),

            // Auto-upgrade - disabled for embedded use
            auto_upgrade: false,

            // Embedding cache
            embedding_cache_cap: DEFAULT_EMBEDDING_CACHE_CAP,

            // IP policy - allow all for embedded use (no SSRF risk since requests come from host)
            ip_policy: http_client::policy::IpPolicy::danger_always_allow(),

            // Snapshot compaction - enabled by default
            experimental_no_snapshot_compaction: false,
        }
    }
}

/// Builder for creating Config instances.
#[derive(Debug, Clone, Default)]
pub struct ConfigBuilder {
    db_path: Option<PathBuf>,
    max_index_size: Option<usize>,
    max_task_db_size: Option<usize>,
}

impl ConfigBuilder {
    /// Set the database path.
    ///
    /// This is a required field. The builder will fail to build without it.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let config = Config::builder()
    ///     .db_path("/var/lib/meilisearch")
    ///     .build()?;
    /// ```
    pub fn db_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.db_path = Some(path.into());
        self
    }

    /// Set the maximum index size in bytes.
    ///
    /// Default: 100 GiB (107,374,182,400 bytes)
    ///
    /// This is the maximum size that any single index database can grow to.
    /// If you're indexing large datasets, you may need to increase this value.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Set to 200 GiB
    /// let config = Config::builder()
    ///     .db_path("/var/lib/meilisearch")
    ///     .max_index_size(200 * 1024 * 1024 * 1024)
    ///     .build()?;
    /// ```
    pub fn max_index_size(mut self, size: usize) -> Self {
        self.max_index_size = Some(size);
        self
    }

    /// Set the maximum task database size in bytes.
    ///
    /// Default: 10 GiB (10,737,418,240 bytes)
    ///
    /// This is the maximum size of the database that stores task history.
    /// Tasks are automatically cleaned up when the limit is approached,
    /// but you may want to increase this if you need longer task history.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Set to 20 GiB
    /// let config = Config::builder()
    ///     .db_path("/var/lib/meilisearch")
    ///     .max_task_db_size(20 * 1024 * 1024 * 1024)
    ///     .build()?;
    /// ```
    pub fn max_task_db_size(mut self, size: usize) -> Self {
        self.max_task_db_size = Some(size);
        self
    }

    /// Build the configuration.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::MissingDbPath`] if `db_path` was not set.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let config = Config::builder()
    ///     .db_path("/var/lib/meilisearch")
    ///     .build()?;
    /// ```
    pub fn build(self) -> Result<Config, crate::Error> {
        let db_path = self.db_path.ok_or(crate::Error::MissingDbPath)?;

        Ok(Config {
            db_path,
            max_index_size: self.max_index_size.unwrap_or(DEFAULT_MAX_INDEX_SIZE),
            max_task_db_size: self.max_task_db_size.unwrap_or(DEFAULT_MAX_TASK_DB_SIZE),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_with_all_fields() {
        let config = Config::builder()
            .db_path("/tmp/test-db")
            .max_index_size(50 * 1024 * 1024 * 1024)
            .max_task_db_size(5 * 1024 * 1024 * 1024)
            .build()
            .expect("should build successfully");

        assert_eq!(config.db_path, PathBuf::from("/tmp/test-db"));
        assert_eq!(config.max_index_size, 50 * 1024 * 1024 * 1024);
        assert_eq!(config.max_task_db_size, 5 * 1024 * 1024 * 1024);
    }

    #[test]
    fn test_builder_with_defaults() {
        let config =
            Config::builder().db_path("/tmp/test-db").build().expect("should build successfully");

        assert_eq!(config.db_path, PathBuf::from("/tmp/test-db"));
        assert_eq!(config.max_index_size, DEFAULT_MAX_INDEX_SIZE);
        assert_eq!(config.max_task_db_size, DEFAULT_MAX_TASK_DB_SIZE);
    }

    #[test]
    fn test_builder_missing_db_path() {
        let result = Config::builder().build();

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, crate::Error::MissingDbPath));
    }

    #[test]
    fn test_builder_db_path_from_string() {
        let config = Config::builder()
            .db_path(String::from("/tmp/test-db"))
            .build()
            .expect("should build successfully");

        assert_eq!(config.db_path, PathBuf::from("/tmp/test-db"));
    }

    #[test]
    fn test_builder_db_path_from_pathbuf() {
        let path = PathBuf::from("/tmp/test-db");
        let config =
            Config::builder().db_path(path.clone()).build().expect("should build successfully");

        assert_eq!(config.db_path, path);
    }

    #[test]
    fn test_to_scheduler_options_paths() {
        let config =
            Config::builder().db_path("/tmp/test-db").build().expect("should build successfully");

        let opts = config.to_scheduler_options();

        assert_eq!(opts.version_file_path, PathBuf::from("/tmp/test-db/VERSION"));
        assert_eq!(opts.auth_path, PathBuf::from("/tmp/test-db/auth"));
        assert_eq!(opts.tasks_path, PathBuf::from("/tmp/test-db/tasks"));
        assert_eq!(opts.update_file_path, PathBuf::from("/tmp/test-db/update_files"));
        assert_eq!(opts.indexes_path, PathBuf::from("/tmp/test-db/indexes"));
        assert_eq!(opts.snapshots_path, PathBuf::from("/tmp/test-db/snapshots"));
        assert_eq!(opts.dumps_path, PathBuf::from("/tmp/test-db/dumps"));
    }

    #[test]
    fn test_to_scheduler_options_sizes() {
        let config = Config::builder()
            .db_path("/tmp/test-db")
            .max_index_size(50 * 1024 * 1024 * 1024)
            .max_task_db_size(5 * 1024 * 1024 * 1024)
            .build()
            .expect("should build successfully");

        let opts = config.to_scheduler_options();

        assert_eq!(opts.index_base_map_size, 50 * 1024 * 1024 * 1024);
        assert_eq!(opts.task_db_size, 5 * 1024 * 1024 * 1024);
    }

    #[test]
    fn test_to_scheduler_options_defaults() {
        let config =
            Config::builder().db_path("/tmp/test-db").build().expect("should build successfully");

        let opts = config.to_scheduler_options();

        // Verify reasonable defaults
        assert!(opts.autobatching_enabled);
        assert!(opts.cleanup_enabled);
        assert!(!opts.auto_upgrade);
        assert!(!opts.enable_mdb_writemap);
        assert!(!opts.experimental_no_snapshot_compaction);
        assert!(opts.cli_webhook_url.is_none());
        assert!(opts.cli_webhook_authorization.is_none());
        assert_eq!(opts.max_number_of_tasks, DEFAULT_MAX_NUMBER_OF_TASKS);
        assert_eq!(opts.index_growth_amount, DEFAULT_INDEX_GROWTH_AMOUNT);
        assert_eq!(opts.index_count, DEFAULT_INDEX_COUNT);
        assert_eq!(opts.embedding_cache_cap, DEFAULT_EMBEDDING_CACHE_CAP);
    }

    #[test]
    fn test_config_clone() {
        let config = Config::builder()
            .db_path("/tmp/test-db")
            .max_index_size(50 * 1024 * 1024 * 1024)
            .build()
            .expect("should build successfully");

        let cloned = config.clone();

        assert_eq!(config.db_path, cloned.db_path);
        assert_eq!(config.max_index_size, cloned.max_index_size);
        assert_eq!(config.max_task_db_size, cloned.max_task_db_size);
    }

    #[test]
    fn test_config_builder_clone() {
        let builder =
            Config::builder().db_path("/tmp/test-db").max_index_size(50 * 1024 * 1024 * 1024);

        let cloned = builder.clone();
        let config = cloned.build().expect("should build successfully");

        assert_eq!(config.db_path, PathBuf::from("/tmp/test-db"));
        assert_eq!(config.max_index_size, 50 * 1024 * 1024 * 1024);
    }

    #[test]
    fn test_config_debug() {
        let config =
            Config::builder().db_path("/tmp/test-db").build().expect("should build successfully");

        let debug_str = format!("{:?}", config);

        assert!(debug_str.contains("Config"));
        assert!(debug_str.contains("db_path"));
        assert!(debug_str.contains("/tmp/test-db"));
    }

    #[test]
    fn test_config_builder_debug() {
        let builder = Config::builder().db_path("/tmp/test-db");

        let debug_str = format!("{:?}", builder);

        assert!(debug_str.contains("ConfigBuilder"));
        assert!(debug_str.contains("db_path"));
    }
}
