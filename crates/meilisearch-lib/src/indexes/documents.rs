//! Document operations for meilisearch-lib.
//!
//! This module provides document management functionality including:
//! - Adding and updating documents
//! - Deleting individual documents, batches, or all documents
//!
//! Document operations return a `TaskView` representing the enqueued task.
//! Use the task API to monitor completion.

use std::io::Write;

use meilisearch_types::milli::update::IndexDocumentsMethod;
use meilisearch_types::tasks::KindWithContent;

use crate::error::Error;
use crate::tasks::TaskView;
use crate::MeilisearchLib;

impl MeilisearchLib {
    // =========================================================================
    // Document Operations
    // =========================================================================

    /// Add or replace documents in an index.
    ///
    /// This method adds new documents or replaces existing documents with the same
    /// primary key. Documents are serialized to NDJSON format and processed
    /// asynchronously by the index scheduler.
    ///
    /// # Arguments
    ///
    /// * `uid` - The unique identifier of the index
    /// * `documents` - A vector of JSON documents to add
    /// * `primary_key` - Optional primary key field name. If the index already has
    ///   a primary key, this is ignored.
    ///
    /// # Returns
    ///
    /// A `TaskView` representing the enqueued document addition task.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The index UID is invalid
    /// - Document serialization fails
    /// - The update file cannot be created
    /// - There's a database error registering the task
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use serde_json::json;
    ///
    /// let documents = vec![
    ///     json!({"id": 1, "title": "The Hobbit", "author": "J.R.R. Tolkien"}),
    ///     json!({"id": 2, "title": "1984", "author": "George Orwell"}),
    /// ];
    ///
    /// let task = meili.add_documents("books", documents, Some("id".to_string()))?;
    /// let task = meili.wait_for_task(task.uid, None)?;
    /// ```
    pub fn add_documents(
        &self,
        uid: impl Into<String>,
        documents: Vec<serde_json::Value>,
        primary_key: Option<String>,
    ) -> Result<TaskView, Error> {
        self.add_or_update_documents(
            uid,
            documents,
            primary_key,
            IndexDocumentsMethod::ReplaceDocuments,
        )
    }

    /// Update documents in an index (partial update).
    ///
    /// This method partially updates existing documents or adds new ones.
    /// Unlike `add_documents`, this only updates the fields provided in the
    /// new document, keeping existing fields intact.
    ///
    /// # Arguments
    ///
    /// * `uid` - The unique identifier of the index
    /// * `documents` - A vector of JSON documents to update
    /// * `primary_key` - Optional primary key field name. If the index already has
    ///   a primary key, this is ignored.
    ///
    /// # Returns
    ///
    /// A `TaskView` representing the enqueued document update task.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The index UID is invalid
    /// - Document serialization fails
    /// - The update file cannot be created
    /// - There's a database error registering the task
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use serde_json::json;
    ///
    /// // Only update the title field for document with id 1
    /// let documents = vec![
    ///     json!({"id": 1, "title": "The Hobbit: An Unexpected Journey"}),
    /// ];
    ///
    /// let task = meili.update_documents("books", documents, None)?;
    /// let task = meili.wait_for_task(task.uid, None)?;
    /// ```
    pub fn update_documents(
        &self,
        uid: impl Into<String>,
        documents: Vec<serde_json::Value>,
        primary_key: Option<String>,
    ) -> Result<TaskView, Error> {
        self.add_or_update_documents(
            uid,
            documents,
            primary_key,
            IndexDocumentsMethod::UpdateDocuments,
        )
    }

    /// Internal method for adding or updating documents.
    ///
    /// Shared implementation for both `add_documents` and `update_documents`.
    fn add_or_update_documents(
        &self,
        uid: impl Into<String>,
        documents: Vec<serde_json::Value>,
        primary_key: Option<String>,
        method: IndexDocumentsMethod,
    ) -> Result<TaskView, Error> {
        let uid = uid.into();

        // Validate the index UID
        if uid.is_empty() {
            return Err(Error::InvalidIndexUid(uid));
        }

        let documents_count = documents.len() as u64;

        // Create an update file (not a dry run)
        let (uuid, mut update_file) = self.scheduler().queue.create_update_file(false)?;

        // Serialize documents to NDJSON format and write to the update file
        {
            let mut writer = std::io::BufWriter::new(&mut update_file);
            for doc in &documents {
                serde_json::to_writer(&mut writer, doc)
                    .map_err(|e| Error::Internal(format!("failed to serialize document: {}", e)))?;
                writer
                    .write_all(b"\n")
                    .map_err(|e| Error::Internal(format!("failed to write newline: {}", e)))?;
            }
            writer
                .flush()
                .map_err(|e| Error::Internal(format!("failed to flush writer: {}", e)))?;
        }

        // Persist the update file
        update_file
            .persist()
            .map_err(|e| Error::Internal(format!("failed to persist update file: {}", e)))?;

        // Register the document addition task
        let kind = KindWithContent::DocumentAdditionOrUpdate {
            index_uid: uid,
            primary_key,
            method,
            content_file: uuid,
            documents_count,
            allow_index_creation: true,
            on_missing_document: meilisearch_types::milli::update::MissingDocumentPolicy::Create,
        };

        let task = self.scheduler().register(kind, None, false)?;
        Ok(TaskView::from(task))
    }

    /// Delete a single document by its ID.
    ///
    /// This registers a `DocumentDeletion` task with the scheduler.
    ///
    /// # Arguments
    ///
    /// * `uid` - The unique identifier of the index
    /// * `document_id` - The primary key value of the document to delete
    ///
    /// # Returns
    ///
    /// A `TaskView` representing the enqueued document deletion task.
    ///
    /// # Errors
    ///
    /// Returns an error if there's a database error registering the task.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let task = meili.delete_document("books", "1")?;
    /// let task = meili.wait_for_task(task.uid, None)?;
    ///
    /// match task.status {
    ///     Status::Succeeded => println!("Document deleted"),
    ///     Status::Failed => println!("Failed: {:?}", task.error),
    ///     _ => {}
    /// }
    /// ```
    pub fn delete_document(
        &self,
        uid: impl Into<String>,
        document_id: impl Into<String>,
    ) -> Result<TaskView, Error> {
        let uid = uid.into();

        // Validate the index UID
        if uid.is_empty() {
            return Err(Error::InvalidIndexUid(uid));
        }

        let kind = KindWithContent::DocumentDeletion {
            index_uid: uid,
            documents_ids: vec![document_id.into()],
        };

        let task = self.scheduler().register(kind, None, false)?;
        Ok(TaskView::from(task))
    }

    /// Delete multiple documents by their IDs.
    ///
    /// This registers a `DocumentDeletion` task with the scheduler for
    /// multiple documents at once.
    ///
    /// # Arguments
    ///
    /// * `uid` - The unique identifier of the index
    /// * `document_ids` - A vector of primary key values for documents to delete
    ///
    /// # Returns
    ///
    /// A `TaskView` representing the enqueued document deletion task.
    ///
    /// # Errors
    ///
    /// Returns an error if there's a database error registering the task.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let ids = vec!["1".to_string(), "2".to_string(), "3".to_string()];
    /// let task = meili.delete_documents_batch("books", ids)?;
    /// let task = meili.wait_for_task(task.uid, None)?;
    /// ```
    pub fn delete_documents_batch(
        &self,
        uid: impl Into<String>,
        document_ids: Vec<String>,
    ) -> Result<TaskView, Error> {
        let uid = uid.into();

        // Validate the index UID
        if uid.is_empty() {
            return Err(Error::InvalidIndexUid(uid));
        }

        let kind =
            KindWithContent::DocumentDeletion { index_uid: uid, documents_ids: document_ids };

        let task = self.scheduler().register(kind, None, false)?;
        Ok(TaskView::from(task))
    }

    /// Delete all documents in an index.
    ///
    /// This registers a `DocumentClear` task that removes all documents
    /// from the index while keeping the index structure and settings intact.
    ///
    /// # Arguments
    ///
    /// * `uid` - The unique identifier of the index
    ///
    /// # Returns
    ///
    /// A `TaskView` representing the enqueued document clear task.
    ///
    /// # Errors
    ///
    /// Returns an error if there's a database error registering the task.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let task = meili.delete_all_documents("books")?;
    /// let task = meili.wait_for_task(task.uid, None)?;
    ///
    /// // The index still exists but is now empty
    /// let stats = meili.index_stats("books")?;
    /// assert_eq!(stats.number_of_documents, 0);
    /// ```
    pub fn delete_all_documents(&self, uid: impl Into<String>) -> Result<TaskView, Error> {
        let uid = uid.into();

        // Validate the index UID
        if uid.is_empty() {
            return Err(Error::InvalidIndexUid(uid));
        }

        let kind = KindWithContent::DocumentClear { index_uid: uid };

        let task = self.scheduler().register(kind, None, false)?;
        Ok(TaskView::from(task))
    }

    /// Get a document by its ID.
    ///
    /// Retrieves a single document from the index by its primary key.
    ///
    /// # Arguments
    ///
    /// * `uid` - The unique identifier of the index
    /// * `document_id` - The primary key value of the document to retrieve
    ///
    /// # Returns
    ///
    /// The document as a JSON value, or an error if not found.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The index doesn't exist
    /// - The document doesn't exist
    /// - There's a database error
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let doc = meili.get_document("books", "1")?;
    /// println!("Title: {}", doc["title"]);
    /// ```
    pub fn get_document(
        &self,
        uid: impl AsRef<str>,
        document_id: impl AsRef<str>,
    ) -> Result<serde_json::Value, Error> {
        let uid = uid.as_ref();
        let document_id = document_id.as_ref();

        let index = self.scheduler().index(uid)?;
        let rtxn = index.read_txn()?;

        // Get the internal document ID
        let external_ids = index.external_documents_ids();
        let internal_id = external_ids
            .get(&rtxn, document_id)?
            .ok_or_else(|| Error::DocumentNotFound(document_id.to_string()))?;

        // Get the fields IDs map
        let fields_ids_map = index.fields_ids_map(&rtxn)?;
        let all_fields: Vec<_> = fields_ids_map.iter().map(|(id, _)| id).collect();

        // Get the document
        let document = index.document(&rtxn, internal_id)?;
        let json_doc =
            meilisearch_types::milli::obkv_to_json(&all_fields, &fields_ids_map, document)?;

        Ok(serde_json::Value::Object(json_doc))
    }

    /// Get multiple documents from an index with pagination.
    ///
    /// Retrieves documents from the index with optional pagination.
    ///
    /// # Arguments
    ///
    /// * `uid` - The unique identifier of the index
    /// * `offset` - Number of documents to skip (for pagination)
    /// * `limit` - Maximum number of documents to return
    ///
    /// # Returns
    ///
    /// A tuple of `(total_count, documents)` where `total_count` is the total
    /// number of documents in the index and `documents` is the paginated list.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The index doesn't exist
    /// - There's a database error
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let (total, docs) = meili.get_documents("books", 0, 10)?;
    /// println!("Found {} documents total", total);
    /// for doc in docs {
    ///     println!("  {}", doc["title"]);
    /// }
    /// ```
    pub fn get_documents(
        &self,
        uid: impl AsRef<str>,
        offset: usize,
        limit: usize,
    ) -> Result<(u64, Vec<serde_json::Value>), Error> {
        let uid = uid.as_ref();

        let index = self.scheduler().index(uid)?;
        let rtxn = index.read_txn()?;

        // Get total document count
        let total = index.number_of_documents(&rtxn)?;

        // Get the fields IDs map
        let fields_ids_map = index.fields_ids_map(&rtxn)?;
        let all_fields: Vec<_> = fields_ids_map.iter().map(|(id, _)| id).collect();

        // Get document IDs with pagination
        let documents_ids = index.documents_ids(&rtxn)?;

        // Iterate and collect documents
        let mut documents = Vec::with_capacity(limit);
        for (_idx, doc_id) in documents_ids.iter().enumerate().skip(offset).take(limit) {
            let document = index.document(&rtxn, doc_id)?;
            let json_doc =
                meilisearch_types::milli::obkv_to_json(&all_fields, &fields_ids_map, document)?;
            documents.push(serde_json::Value::Object(json_doc));
        }

        Ok((total, documents))
    }
}

#[cfg(test)]
mod tests {
    // Integration tests for document operations require a real filesystem
    // and are in the integration tests directory.
}
