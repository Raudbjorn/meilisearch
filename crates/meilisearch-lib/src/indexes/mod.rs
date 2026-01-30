//! Index operations for meilisearch-lib.
//!
//! This module provides index management functionality including:
//! - Creating and deleting indexes
//! - Retrieving index information and statistics
//! - Listing all indexes with pagination
//!
//! Index operations that modify data (create, delete) return a `TaskView`
//! representing the enqueued task. Use the task API to monitor completion.

pub mod documents;
pub mod search;
pub mod settings;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// View of a Meilisearch index.
///
/// This struct contains metadata about an index without including
/// the actual documents or search data.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexView {
    /// Index unique identifier.
    pub uid: String,
    /// Primary key field name.
    pub primary_key: Option<String>,
    /// When the index was created.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// When the index was last updated.
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

impl IndexView {
    /// Create an `IndexView` from a milli `Index`.
    ///
    /// This reads index metadata from the index's transaction.
    #[allow(clippy::result_large_err)]
    pub(crate) fn from_index(
        uid: String,
        index: &meilisearch_types::milli::Index,
    ) -> Result<Self, meilisearch_types::milli::Error> {
        let rtxn = index.read_txn()?;
        Ok(Self {
            uid,
            primary_key: index.primary_key(&rtxn)?.map(String::from),
            created_at: index.created_at(&rtxn)?,
            updated_at: index.updated_at(&rtxn)?,
        })
    }
}

/// Statistics for a Meilisearch index.
///
/// Provides detailed information about the index's contents and state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStats {
    /// Number of documents in the index.
    pub number_of_documents: u64,
    /// Whether the index is currently being indexed.
    pub is_indexing: bool,
    /// Distribution of fields across documents.
    ///
    /// Maps field names to the number of documents containing that field.
    pub field_distribution: BTreeMap<String, u64>,
}

impl From<index_scheduler::IndexStats> for IndexStats {
    fn from(stats: index_scheduler::IndexStats) -> Self {
        Self {
            number_of_documents: stats
                .inner_stats
                .number_of_documents
                .unwrap_or(stats.inner_stats.documents_database_stats.number_of_entries()),
            is_indexing: stats.is_indexing,
            field_distribution: stats.inner_stats.field_distribution,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    /// Test that IndexView serializes correctly with camelCase field names.
    ///
    /// This test verifies:
    /// 1. All fields serialize to camelCase as per Meilisearch API spec
    /// 2. Optional fields are properly handled
    /// 3. Timestamps are serialized in RFC 3339 format
    /// 4. Round-trip serialization/deserialization preserves data
    #[test]
    fn test_index_view_serialization() {
        let created = datetime!(2024-01-15 10:30:00 UTC);
        let updated = datetime!(2024-01-20 14:45:30 UTC);

        let view = IndexView {
            uid: "test-index".to_string(),
            primary_key: Some("id".to_string()),
            created_at: created,
            updated_at: updated,
        };

        // Serialize to JSON
        let json = serde_json::to_string(&view).expect("serialization failed");

        // Verify camelCase field names
        assert!(json.contains("\"uid\""), "should have uid field");
        assert!(json.contains("\"primaryKey\""), "should use camelCase for primary_key");
        assert!(json.contains("\"createdAt\""), "should use camelCase for created_at");
        assert!(json.contains("\"updatedAt\""), "should use camelCase for updated_at");

        // Verify values
        assert!(json.contains("\"test-index\""), "should contain uid value");
        assert!(json.contains("\"id\""), "should contain primary_key value");

        // Verify timestamps are in RFC 3339 format
        assert!(json.contains("2024-01-15T10:30:00Z"), "created_at should be RFC 3339");
        assert!(json.contains("2024-01-20T14:45:30Z"), "updated_at should be RFC 3339");

        // Test round-trip
        let deserialized: IndexView =
            serde_json::from_str(&json).expect("deserialization failed");
        assert_eq!(deserialized.uid, view.uid);
        assert_eq!(deserialized.primary_key, view.primary_key);
        assert_eq!(deserialized.created_at, view.created_at);
        assert_eq!(deserialized.updated_at, view.updated_at);

        // Test with None primary_key
        let view_no_pk = IndexView {
            uid: "no-pk-index".to_string(),
            primary_key: None,
            created_at: created,
            updated_at: updated,
        };

        let json_no_pk = serde_json::to_string(&view_no_pk).expect("serialization failed");
        assert!(json_no_pk.contains("\"primaryKey\":null"), "null primary_key should serialize");

        let deserialized_no_pk: IndexView =
            serde_json::from_str(&json_no_pk).expect("deserialization failed");
        assert_eq!(deserialized_no_pk.primary_key, None);
    }

    /// Test that IndexStats serializes correctly.
    #[test]
    fn test_index_stats_serialization() {
        let mut field_dist = BTreeMap::new();
        field_dist.insert("title".to_string(), 100);
        field_dist.insert("author".to_string(), 95);

        let stats = IndexStats {
            number_of_documents: 100,
            is_indexing: false,
            field_distribution: field_dist,
        };

        let json = serde_json::to_string(&stats).expect("serialization failed");

        // Verify camelCase
        assert!(json.contains("\"numberOfDocuments\""), "should use camelCase");
        assert!(json.contains("\"isIndexing\""), "should use camelCase");
        assert!(json.contains("\"fieldDistribution\""), "should use camelCase");

        // Round-trip
        let deserialized: IndexStats =
            serde_json::from_str(&json).expect("deserialization failed");
        assert_eq!(deserialized.number_of_documents, 100);
        assert!(!deserialized.is_indexing);
        assert_eq!(deserialized.field_distribution.get("title"), Some(&100));
    }
}
