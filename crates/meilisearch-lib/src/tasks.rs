//! Task operations for meilisearch-lib.
//!
//! This module provides task management for the embedded Meilisearch library.
//! Tasks are created for asynchronous operations like document indexing,
//! settings updates, and index management.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use meilisearch_types::tasks::{Kind, Status};

/// View of a Meilisearch task.
///
/// This struct represents the state of an asynchronous task in Meilisearch.
/// Tasks are created when you perform operations like adding documents,
/// updating settings, or creating indexes.
///
/// # Example
///
/// ```rust,ignore
/// let task = meili.get_task(42)?;
/// println!("Task {} status: {:?}", task.uid, task.status);
/// if let Some(err) = task.error {
///     println!("Error: {} ({})", err.message, err.code);
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskView {
    /// Unique task identifier.
    pub uid: u32,

    /// Index UID this task operates on (if applicable).
    ///
    /// This is `None` for global tasks like `dumpCreation` or `taskDeletion`
    /// that don't target a specific index.
    pub index_uid: Option<String>,

    /// Current status of the task.
    ///
    /// Possible values:
    /// - `Enqueued` - Task is waiting to be processed
    /// - `Processing` - Task is currently being processed
    /// - `Succeeded` - Task completed successfully
    /// - `Failed` - Task failed with an error
    /// - `Canceled` - Task was canceled before completion
    pub status: Status,

    /// Type of operation this task performs.
    ///
    /// Examples include `DocumentAdditionOrUpdate`, `DocumentDeletion`,
    /// `SettingsUpdate`, `IndexCreation`, `IndexDeletion`, etc.
    #[serde(rename = "type")]
    pub kind: Kind,

    /// When the task was added to the queue.
    #[serde(with = "time::serde::rfc3339")]
    pub enqueued_at: OffsetDateTime,

    /// When the task started processing.
    ///
    /// `None` if the task hasn't started yet.
    #[serde(with = "time::serde::rfc3339::option")]
    pub started_at: Option<OffsetDateTime>,

    /// When the task finished (successfully or with an error).
    ///
    /// `None` if the task hasn't finished yet.
    #[serde(with = "time::serde::rfc3339::option")]
    pub finished_at: Option<OffsetDateTime>,

    /// Error information if the task failed.
    ///
    /// `None` if the task succeeded or is still processing.
    pub error: Option<TaskError>,
}

/// Error information for a failed task.
///
/// When a task fails, this struct contains details about what went wrong.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskError {
    /// Human-readable error message describing what went wrong.
    pub message: String,

    /// Machine-readable error code for programmatic handling.
    ///
    /// Error codes follow Meilisearch conventions, e.g.:
    /// - `index_not_found`
    /// - `invalid_document_id`
    /// - `primary_key_inference_failed`
    pub code: String,
}

impl From<&meilisearch_types::tasks::Task> for TaskView {
    fn from(task: &meilisearch_types::tasks::Task) -> Self {
        TaskView {
            uid: task.uid,
            index_uid: task.index_uid().map(ToOwned::to_owned),
            status: task.status,
            kind: task.kind.as_kind(),
            enqueued_at: task.enqueued_at,
            started_at: task.started_at,
            finished_at: task.finished_at,
            error: task.error.as_ref().map(TaskError::from),
        }
    }
}

impl From<&meilisearch_types::error::ResponseError> for TaskError {
    fn from(e: &meilisearch_types::error::ResponseError) -> Self {
        // ResponseError serializes with camelCase, so we can extract fields from JSON
        // or access the public message field directly.
        // The error_code field is private, but we can get it from JSON serialization.
        let json = serde_json::to_value(e).unwrap_or_default();
        TaskError {
            message: e.message.clone(),
            code: json.get("code").and_then(|v| v.as_str()).unwrap_or("internal").to_string(),
        }
    }
}

impl From<meilisearch_types::tasks::Task> for TaskView {
    fn from(task: meilisearch_types::tasks::Task) -> Self {
        TaskView::from(&task)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use meilisearch_types::tasks::Status;

    #[test]
    fn test_task_error_serialization() {
        let error = TaskError {
            message: "Index not found".to_string(),
            code: "index_not_found".to_string(),
        };

        let json = serde_json::to_string(&error).unwrap();
        assert!(json.contains("message"));
        assert!(json.contains("code"));
        assert!(json.contains("Index not found"));
        assert!(json.contains("index_not_found"));
    }

    #[test]
    fn test_task_error_deserialization() {
        let json = r#"{"message":"Index not found","code":"index_not_found"}"#;
        let error: TaskError = serde_json::from_str(json).unwrap();
        assert_eq!(error.message, "Index not found");
        assert_eq!(error.code, "index_not_found");
    }

    #[test]
    fn test_task_view_status_variants() {
        // Just verify the Status enum can be used
        let statuses = [
            Status::Enqueued,
            Status::Processing,
            Status::Succeeded,
            Status::Failed,
            Status::Canceled,
        ];
        for status in statuses {
            assert!(format!("{:?}", status).len() > 0);
        }
    }
}
