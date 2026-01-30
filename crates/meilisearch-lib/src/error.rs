//! Error types for meilisearch-lib.
//!
//! This module provides a unified error type for all meilisearch-lib operations,
//! integrating with the meilisearch_types error code system for consistent
//! HTTP status codes and error responses.

use meilisearch_types::error::{Code, ErrorCode};
use thiserror::Error;

/// Result type alias for meilisearch-lib operations.
#[allow(dead_code)]
pub type Result<T> = std::result::Result<T, Error>;

/// Error type for meilisearch-lib operations.
///
/// Each variant maps to a specific error code and HTTP status code,
/// enabling consistent error responses across the API.
#[derive(Debug, Error)]
pub enum Error {
    /// Database path is required but not provided.
    #[error("database path is required")]
    MissingDbPath,

    /// Index not found.
    #[error("index `{0}` not found")]
    IndexNotFound(String),

    /// Invalid index UID.
    #[error("invalid index uid `{0}`: index uid must be non-empty and contain only alphanumeric characters, hyphens, and underscores")]
    InvalidIndexUid(String),

    /// Invalid settings.
    #[error("invalid settings: {0}")]
    InvalidSettings(String),

    /// Document not found.
    #[error("document `{0}` not found")]
    DocumentNotFound(String),

    /// Task not found.
    #[error("task `{0}` not found")]
    TaskNotFound(u32),

    /// Task timed out while waiting for completion.
    #[error("task {0} timed out after {1:?}")]
    TaskTimeout(u32, std::time::Duration),

    /// Chat configuration not set.
    #[error("chat is not configured; set chat configuration before using chat completions")]
    ChatNotConfigured,

    /// Chat provider error (LLM API error).
    #[error("chat provider error: {0}")]
    ChatProvider(String),

    /// Configuration error.
    #[error("configuration error: {0}")]
    Config(String),

    /// Search error.
    #[error("search error: {0}")]
    Search(String),

    /// Index scheduler error.
    #[error(transparent)]
    Scheduler(#[from] index_scheduler::Error),

    /// Milli search engine error.
    #[error(transparent)]
    Milli(#[from] meilisearch_types::milli::Error),

    /// Heed database error.
    #[error("database error: {0}")]
    Heed(#[from] meilisearch_types::heed::Error),

    /// IO error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization/deserialization error.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// File store error.
    #[error("file store error: {0}")]
    FileStore(#[from] file_store::Error),

    /// Internal error for unexpected conditions.
    #[error("internal error: {0}")]
    Internal(String),
}

impl Error {
    /// Creates a new configuration error.
    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config(msg.into())
    }

    /// Creates a new internal error.
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }

    /// Creates a new search error.
    pub fn search(msg: impl Into<String>) -> Self {
        Self::Search(msg.into())
    }

    /// Creates a new chat provider error.
    pub fn chat_provider(msg: impl Into<String>) -> Self {
        Self::ChatProvider(msg.into())
    }

    /// Returns the error code for this error.
    pub fn code(&self) -> Code {
        self.error_code()
    }

    /// Returns the HTTP status code (as u16) for this error.
    pub fn status_code(&self) -> u16 {
        self.error_code().http().as_u16()
    }
}

impl ErrorCode for Error {
    fn error_code(&self) -> Code {
        match self {
            // Not found errors
            Error::IndexNotFound(_) => Code::IndexNotFound,
            Error::DocumentNotFound(_) => Code::DocumentNotFound,
            Error::TaskNotFound(_) => Code::TaskNotFound,

            // Invalid request errors
            Error::InvalidIndexUid(_) => Code::InvalidIndexUid,
            Error::InvalidSettings(_) => Code::BadRequest,

            // Chat errors - use ChatNotFound if available, else BadRequest
            Error::ChatNotConfigured => Code::ChatNotFound,

            // Bad request / configuration errors
            Error::MissingDbPath => Code::BadRequest,
            Error::Config(_) => Code::BadRequest,
            Error::TaskTimeout(_, _) => Code::BadRequest,

            // Internal / provider errors
            Error::ChatProvider(_) => Code::Internal,
            Error::Search(_) => Code::Internal,
            Error::Internal(_) => Code::Internal,

            // Delegated errors - use their error_code() implementation
            Error::Scheduler(e) => e.error_code(),
            Error::Milli(e) => e.error_code(),
            Error::Io(e) => e.error_code(),

            // Database errors
            Error::Heed(_) => Code::Internal,

            // File store errors
            Error::FileStore(_) => Code::Internal,

            // JSON errors are typically bad requests
            Error::Json(_) => Code::BadRequest,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_index_not_found_error_code() {
        let error = Error::IndexNotFound("test_index".to_string());
        assert_eq!(error.code(), Code::IndexNotFound);
        assert_eq!(error.status_code(), 404);
    }

    #[test]
    fn test_document_not_found_error_code() {
        let error = Error::DocumentNotFound("doc123".to_string());
        assert_eq!(error.code(), Code::DocumentNotFound);
        assert_eq!(error.status_code(), 404);
    }

    #[test]
    fn test_task_not_found_error_code() {
        let error = Error::TaskNotFound(42);
        assert_eq!(error.code(), Code::TaskNotFound);
        assert_eq!(error.status_code(), 404);
    }

    #[test]
    fn test_chat_not_configured_error_code() {
        let error = Error::ChatNotConfigured;
        assert_eq!(error.code(), Code::ChatNotFound);
        assert_eq!(error.status_code(), 404);
    }

    #[test]
    fn test_missing_db_path_error_code() {
        let error = Error::MissingDbPath;
        assert_eq!(error.code(), Code::BadRequest);
        assert_eq!(error.status_code(), 400);
    }

    #[test]
    fn test_config_error_code() {
        let error = Error::config("invalid setting");
        assert_eq!(error.code(), Code::BadRequest);
        assert_eq!(error.status_code(), 400);
    }

    #[test]
    fn test_task_timeout_error_code() {
        let error = Error::TaskTimeout(1, std::time::Duration::from_secs(30));
        assert_eq!(error.code(), Code::BadRequest);
        assert_eq!(error.status_code(), 400);
    }

    #[test]
    fn test_chat_provider_error_code() {
        let error = Error::chat_provider("API rate limit exceeded");
        assert_eq!(error.code(), Code::Internal);
        assert_eq!(error.status_code(), 500);
    }

    #[test]
    fn test_search_error_code() {
        let error = Error::search("invalid query");
        assert_eq!(error.code(), Code::Internal);
        assert_eq!(error.status_code(), 500);
    }

    #[test]
    fn test_internal_error_code() {
        let error = Error::internal("unexpected condition");
        assert_eq!(error.code(), Code::Internal);
        assert_eq!(error.status_code(), 500);
    }

    #[test]
    fn test_json_error_code() {
        let json_err = serde_json::from_str::<serde_json::Value>("not valid json").unwrap_err();
        let error = Error::Json(json_err);
        assert_eq!(error.code(), Code::BadRequest);
        assert_eq!(error.status_code(), 400);
    }

    #[test]
    fn test_io_error_code() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let error = Error::Io(io_err);
        // IO errors map to Code::Internal by default (unless specific OS error codes)
        assert_eq!(error.code(), Code::Internal);
        assert_eq!(error.status_code(), 500);
    }

    #[test]
    fn test_error_display() {
        assert_eq!(
            Error::IndexNotFound("my_index".to_string()).to_string(),
            "index `my_index` not found"
        );
        assert_eq!(
            Error::DocumentNotFound("doc_id".to_string()).to_string(),
            "document `doc_id` not found"
        );
        assert_eq!(Error::TaskNotFound(123).to_string(), "task `123` not found");
        assert_eq!(
            Error::TaskTimeout(5, std::time::Duration::from_secs(60)).to_string(),
            "task 5 timed out after 60s"
        );
        assert_eq!(Error::MissingDbPath.to_string(), "database path is required");
        assert_eq!(
            Error::ChatNotConfigured.to_string(),
            "chat is not configured; set chat configuration before using chat completions"
        );
        assert_eq!(Error::config("bad value").to_string(), "configuration error: bad value");
        assert_eq!(Error::chat_provider("timeout").to_string(), "chat provider error: timeout");
        assert_eq!(Error::search("parse error").to_string(), "search error: parse error");
        assert_eq!(Error::internal("oops").to_string(), "internal error: oops");
    }

    #[test]
    fn test_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "access denied");
        let error: Error = io_err.into();
        assert!(matches!(error, Error::Io(_)));
    }

    #[test]
    fn test_error_from_json() {
        let json_err = serde_json::from_str::<()>("invalid").unwrap_err();
        let error: Error = json_err.into();
        assert!(matches!(error, Error::Json(_)));
    }

    #[test]
    fn test_result_type_alias() {
        fn returns_result() -> Result<u32> {
            Ok(42)
        }

        fn returns_error() -> Result<u32> {
            Err(Error::MissingDbPath)
        }

        assert_eq!(returns_result().unwrap(), 42);
        assert!(returns_error().is_err());
    }
}
