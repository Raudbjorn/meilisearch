//! Search operations for meilisearch-lib.
//!
//! This module provides hybrid search functionality combining keyword and semantic search.
//! It wraps the milli search engine and provides a simplified API for search operations.

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::Error;
use crate::MeilisearchLib;

// ============================================================================
// Search Query Types
// ============================================================================

/// Configuration for hybrid search combining keyword and semantic search.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridQuery {
    /// Balance between keyword search (0.0) and semantic search (1.0).
    ///
    /// - `0.0` = pure keyword search
    /// - `0.5` = equal weighting (default)
    /// - `1.0` = pure semantic search
    #[serde(default = "default_semantic_ratio")]
    pub semantic_ratio: f32,

    /// Name of the embedder to use for semantic search.
    ///
    /// If not specified, uses the default embedder configured for the index.
    pub embedder: Option<String>,
}

fn default_semantic_ratio() -> f32 {
    0.5
}

impl HybridQuery {
    /// Create a new hybrid query with the specified semantic ratio.
    pub fn new(semantic_ratio: f32) -> Self {
        Self { semantic_ratio: semantic_ratio.clamp(0.0, 1.0), embedder: None }
    }

    /// Create a new hybrid query with the specified embedder.
    pub fn with_embedder(mut self, embedder: impl Into<String>) -> Self {
        self.embedder = Some(embedder.into());
        self
    }
}

/// Search query parameters for meilisearch-lib.
///
/// This is a simplified version of the full Meilisearch search query,
/// containing the most commonly used parameters for hybrid search.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    /// Query string for keyword matching.
    ///
    /// For hybrid search, this is used for keyword matching and may also
    /// be used to generate embeddings if no vector is provided.
    #[serde(default)]
    pub q: Option<String>,

    /// Embedding vector for semantic matching.
    ///
    /// If provided, this vector is used for semantic search. If not provided
    /// and hybrid search is enabled, the query string will be embedded.
    #[serde(default)]
    pub vector: Option<Vec<f32>>,

    /// Hybrid search configuration.
    ///
    /// When set, combines keyword and semantic search results based on
    /// the `semantic_ratio` parameter.
    #[serde(default)]
    pub hybrid: Option<HybridQuery>,

    /// Filter expression to narrow search results.
    ///
    /// Supports the Meilisearch filter syntax for complex queries.
    #[serde(default)]
    pub filter: Option<Value>,

    /// Number of documents to skip (for pagination).
    #[serde(default)]
    pub offset: usize,

    /// Maximum number of documents to return.
    #[serde(default = "default_limit")]
    pub limit: usize,

    /// Specific fields to return in the response.
    ///
    /// If not specified, all displayed fields are returned.
    #[serde(default)]
    pub attributes_to_retrieve: Option<Vec<String>>,

    /// Sort criteria for the results.
    ///
    /// Example: `["price:asc", "rating:desc"]`
    #[serde(default)]
    pub sort: Option<Vec<String>>,

    /// Restrict search to these attributes only.
    ///
    /// If not specified, searches all searchable attributes.
    #[serde(default)]
    pub attributes_to_search_on: Option<Vec<String>>,

    /// Fields to highlight in search results.
    #[serde(default)]
    pub attributes_to_highlight: Option<HashSet<String>>,

    /// Fields to crop in search results.
    #[serde(default)]
    pub attributes_to_crop: Option<Vec<String>>,

    /// Maximum length of cropped fields in words.
    #[serde(default = "default_crop_length")]
    pub crop_length: usize,

    /// Display the ranking score for each document.
    #[serde(default)]
    pub show_ranking_score: bool,

    /// Display detailed ranking score breakdown.
    #[serde(default)]
    pub show_ranking_score_details: bool,

    /// Minimum ranking score threshold (0.0 to 1.0).
    ///
    /// Documents with scores below this threshold are excluded.
    #[serde(default)]
    pub ranking_score_threshold: Option<f64>,
}

fn default_limit() -> usize {
    20
}

fn default_crop_length() -> usize {
    10
}

impl SearchQuery {
    /// Create a new search query with the specified query string.
    pub fn new(q: impl Into<String>) -> Self {
        Self {
            q: Some(q.into()),
            limit: default_limit(),
            crop_length: default_crop_length(),
            ..Default::default()
        }
    }

    /// Create an empty search query (returns all documents).
    pub fn empty() -> Self {
        Self { limit: default_limit(), crop_length: default_crop_length(), ..Default::default() }
    }

    /// Set the hybrid search configuration.
    pub fn with_hybrid(mut self, hybrid: HybridQuery) -> Self {
        self.hybrid = Some(hybrid);
        self
    }

    /// Set the filter expression.
    pub fn with_filter(mut self, filter: Value) -> Self {
        self.filter = Some(filter);
        self
    }

    /// Set pagination parameters.
    pub fn with_pagination(mut self, offset: usize, limit: usize) -> Self {
        self.offset = offset;
        self.limit = limit;
        self
    }

    /// Set the attributes to retrieve.
    pub fn with_attributes_to_retrieve(mut self, attributes: Vec<String>) -> Self {
        self.attributes_to_retrieve = Some(attributes);
        self
    }

    /// Set the vector for semantic search.
    pub fn with_vector(mut self, vector: Vec<f32>) -> Self {
        self.vector = Some(vector);
        self
    }

    /// Set the sort criteria.
    pub fn with_sort(mut self, sort: Vec<String>) -> Self {
        self.sort = Some(sort);
        self
    }

    /// Determine if this is a hybrid search query.
    pub fn is_hybrid(&self) -> bool {
        self.hybrid.is_some()
    }

    /// Determine if this is a pure semantic search (vector-only).
    pub fn is_semantic_only(&self) -> bool {
        self.vector.is_some() && self.q.is_none() && self.hybrid.is_none()
    }

    /// Determine if this is a pure keyword search.
    pub fn is_keyword_only(&self) -> bool {
        self.q.is_some() && self.vector.is_none() && self.hybrid.is_none()
    }
}

// ============================================================================
// Search Result Types
// ============================================================================

/// A single search result hit.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    /// The document data.
    #[serde(flatten)]
    pub document: Value,

    /// Global ranking score (0.0 to 1.0) if `show_ranking_score` was enabled.
    #[serde(rename = "_rankingScore", skip_serializing_if = "Option::is_none")]
    pub ranking_score: Option<f64>,

    /// Detailed ranking score breakdown if `show_ranking_score_details` was enabled.
    #[serde(rename = "_rankingScoreDetails", skip_serializing_if = "Option::is_none")]
    pub ranking_score_details: Option<Value>,
}

impl SearchHit {
    /// Create a new search hit from a document.
    pub fn from_document(document: Value) -> Self {
        Self { document, ranking_score: None, ranking_score_details: None }
    }

    /// Set the ranking score.
    pub fn with_ranking_score(mut self, score: f64) -> Self {
        self.ranking_score = Some(score);
        self
    }
}

/// Pagination information for search results.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HitsInfo {
    /// Offset-based pagination with estimated counts.
    #[serde(rename_all = "camelCase")]
    OffsetLimit {
        /// Maximum number of documents returned.
        limit: usize,
        /// Number of documents skipped.
        offset: usize,
        /// Estimated total number of matching documents.
        estimated_total_hits: usize,
    },
    /// Page-based pagination with exact counts.
    #[serde(rename_all = "camelCase")]
    Pagination {
        /// Results per page.
        hits_per_page: usize,
        /// Current page number.
        page: usize,
        /// Total number of pages.
        total_pages: usize,
        /// Total number of matching documents.
        total_hits: usize,
    },
}

/// Search response containing matching documents and metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    /// The matching documents.
    pub hits: Vec<SearchHit>,

    /// Estimated total number of matching documents.
    ///
    /// This is an estimate because Meilisearch may not count all candidates
    /// for performance reasons.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_total_hits: Option<u64>,

    /// Pagination offset used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,

    /// Maximum results limit used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,

    /// Time taken to process the search in milliseconds.
    pub processing_time_ms: u64,

    /// The original query string.
    pub query: String,

    /// Number of semantic search hits (for hybrid/semantic searches).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_hit_count: Option<u32>,
}

impl SearchResult {
    /// Create an empty search result.
    pub fn empty(query: impl Into<String>, processing_time_ms: u64) -> Self {
        Self {
            hits: Vec::new(),
            estimated_total_hits: Some(0),
            offset: Some(0),
            limit: Some(20),
            processing_time_ms,
            query: query.into(),
            semantic_hit_count: None,
        }
    }
}

// ============================================================================
// Search Kind (Internal)
// ============================================================================

/// Internal enum to determine the type of search to execute.
#[derive(Debug, Clone)]
pub(crate) enum SearchKind {
    /// Pure keyword search.
    KeywordOnly,
    /// Pure semantic/vector search.
    SemanticOnly {
        embedder_name: String,
        embedder: Arc<meilisearch_types::milli::vector::Embedder>,
        quantized: bool,
    },
    /// Hybrid search combining keyword and semantic.
    Hybrid {
        embedder_name: String,
        embedder: Arc<meilisearch_types::milli::vector::Embedder>,
        quantized: bool,
        semantic_ratio: f32,
    },
}

// ============================================================================
// MeilisearchLib Search Implementation
// ============================================================================

impl MeilisearchLib {
    /// Execute a search query on an index.
    ///
    /// This method supports:
    /// - **Keyword search**: When only `q` is provided
    /// - **Semantic search**: When only `vector` is provided
    /// - **Hybrid search**: When `hybrid` configuration is set
    ///
    /// # Arguments
    ///
    /// * `uid` - The unique identifier of the index to search
    /// * `query` - The search query parameters
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The index doesn't exist
    /// - The search query is invalid
    /// - The embedder is not configured (for hybrid/semantic search)
    /// - There's a database error
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use meilisearch_lib::{MeilisearchLib, SearchQuery, HybridQuery};
    ///
    /// // Simple keyword search
    /// let query = SearchQuery::new("hello world");
    /// let results = meili.search("my_index", query)?;
    ///
    /// // Hybrid search
    /// let query = SearchQuery::new("hello world")
    ///     .with_hybrid(HybridQuery::new(0.7)); // 70% semantic
    /// let results = meili.search("my_index", query)?;
    ///
    /// for hit in results.hits {
    ///     println!("{}", hit.document);
    /// }
    /// ```
    pub fn search(&self, uid: impl AsRef<str>, query: SearchQuery) -> Result<SearchResult, Error> {
        let start = Instant::now();
        let uid = uid.as_ref();

        // Get the index
        let index = self.scheduler().index(uid)?;
        let rtxn = index.read_txn()?;

        // Determine search kind based on query parameters
        let search_kind = self.determine_search_kind(uid, &index, &rtxn, &query)?;

        // Create milli search
        let progress = meilisearch_types::milli::progress::Progress::default();
        let mut search = index.search(&rtxn, &progress);

        // Set basic search parameters
        search.offset(query.offset);
        search.limit(query.limit);

        if let Some(threshold) = query.ranking_score_threshold {
            search.ranking_score_threshold(threshold);
        }

        // Configure search based on kind
        match &search_kind {
            SearchKind::KeywordOnly => {
                if let Some(ref q) = query.q {
                    search.query(q);
                }
            }
            SearchKind::SemanticOnly { embedder_name, embedder, quantized } => {
                let vector = if let Some(v) = query.vector.clone() {
                    v
                } else if let Some(ref q) = query.q {
                    // Embed the query string
                    embed_query(embedder, q)?
                } else {
                    return Err(Error::search(
                        "semantic search requires either a query string or vector",
                    ));
                };

                search.semantic(
                    embedder_name.clone(),
                    embedder.clone(),
                    *quantized,
                    Some(vector),
                    None, // No media support in simplified API
                );
            }
            SearchKind::Hybrid { embedder_name, embedder, quantized, semantic_ratio: _ } => {
                if let Some(ref q) = query.q {
                    search.query(q);
                }
                search.semantic(
                    embedder_name.clone(),
                    embedder.clone(),
                    *quantized,
                    query.vector.clone(),
                    None,
                );
            }
        }

        // Apply filter if provided
        if let Some(ref filter_value) = query.filter {
            if let Some(filter) = parse_filter(filter_value)? {
                search.filter(filter);
            }
        }

        // Apply sort if provided
        if let Some(ref sort) = query.sort {
            let sort_criteria: Vec<meilisearch_types::milli::AscDesc> =
                sort.iter().filter_map(|s| s.parse().ok()).collect();
            if !sort_criteria.is_empty() {
                search.sort_criteria(sort_criteria);
            }
        }

        // Apply attributes to search on
        if let Some(ref attrs) = query.attributes_to_search_on {
            search.searchable_attributes(attrs);
        }

        // Enable scoring if requested
        if query.show_ranking_score
            || query.show_ranking_score_details
            || query.ranking_score_threshold.is_some()
        {
            search.scoring_strategy(
                meilisearch_types::milli::score_details::ScoringStrategy::Detailed,
            );
        }

        // Determine max total hits from index settings
        let max_total_hits = index
            .pagination_max_total_hits(&rtxn)
            .map_err(meilisearch_types::milli::Error::from)?
            .map(|x| x as usize)
            .unwrap_or(1000);

        search.max_total_hits(Some(max_total_hits));

        // Execute search based on kind
        let (milli_result, semantic_hit_count) = match &search_kind {
            SearchKind::KeywordOnly => {
                let result = search.execute().map_err(|e| Error::search(e.to_string()))?;
                (result, None)
            }
            SearchKind::SemanticOnly { .. } => {
                let result = search.execute().map_err(|e| Error::search(e.to_string()))?;
                let count = result.document_scores.len() as u32;
                (result, Some(count))
            }
            SearchKind::Hybrid { semantic_ratio, .. } => {
                let (result, count) = search
                    .execute_hybrid(*semantic_ratio)
                    .map_err(|e| Error::search(e.to_string()))?;
                (result, count)
            }
        };

        // Convert results to SearchHits
        let hits = self.format_hits(&index, &rtxn, &milli_result, &query)?;

        let processing_time_ms = start.elapsed().as_millis() as u64;
        let estimated_total_hits =
            std::cmp::min(milli_result.candidates.len(), max_total_hits as u64);

        Ok(SearchResult {
            hits,
            estimated_total_hits: Some(estimated_total_hits),
            offset: Some(query.offset),
            limit: Some(query.limit),
            processing_time_ms,
            query: query.q.clone().unwrap_or_default(),
            semantic_hit_count,
        })
    }

    /// Determine the search kind based on query parameters.
    fn determine_search_kind(
        &self,
        uid: &str,
        index: &meilisearch_types::milli::Index,
        rtxn: &meilisearch_types::heed::RoTxn<'_>,
        query: &SearchQuery,
    ) -> Result<SearchKind, Error> {
        // If hybrid is specified, use hybrid search
        if let Some(ref hybrid) = query.hybrid {
            let embedder_name = hybrid.embedder.clone().unwrap_or_else(|| "default".to_string());
            let (embedder, quantized) = self.get_embedder(uid, index, rtxn, &embedder_name)?;
            return Ok(SearchKind::Hybrid {
                embedder_name,
                embedder,
                quantized,
                semantic_ratio: hybrid.semantic_ratio,
            });
        }

        // If only vector is provided, use semantic search
        if query.vector.is_some() && query.q.is_none() {
            let embedder_name = "default".to_string();
            let (embedder, quantized) = self.get_embedder(uid, index, rtxn, &embedder_name)?;
            return Ok(SearchKind::SemanticOnly { embedder_name, embedder, quantized });
        }

        // Default to keyword search
        Ok(SearchKind::KeywordOnly)
    }

    /// Get the embedder for the specified name.
    fn get_embedder(
        &self,
        uid: &str,
        index: &meilisearch_types::milli::Index,
        rtxn: &meilisearch_types::heed::RoTxn<'_>,
        embedder_name: &str,
    ) -> Result<(Arc<meilisearch_types::milli::vector::Embedder>, bool), Error> {
        let embedder_configs = index.embedding_configs().embedding_configs(rtxn)?;
        let embedders = self.scheduler().embedders(uid.to_string(), embedder_configs)?;

        let runtime = embedders
            .get(embedder_name)
            .ok_or_else(|| Error::search(format!("embedder `{}` not found", embedder_name)))?;

        Ok((runtime.embedder.clone(), runtime.is_quantized))
    }

    /// Format milli search results into SearchHits.
    fn format_hits(
        &self,
        index: &meilisearch_types::milli::Index,
        rtxn: &meilisearch_types::heed::RoTxn<'_>,
        milli_result: &meilisearch_types::milli::SearchResult,
        query: &SearchQuery,
    ) -> Result<Vec<SearchHit>, Error> {
        let fields_ids_map = index.fields_ids_map(rtxn)?;

        // Determine which fields to retrieve
        let displayed_fields = index.displayed_fields_ids(rtxn)?;
        let to_retrieve: BTreeSet<_> = if let Some(ref attrs) = query.attributes_to_retrieve {
            attrs
                .iter()
                .filter_map(|name| {
                    if name == "*" {
                        None // Wildcard handled separately
                    } else {
                        fields_ids_map.id(name)
                    }
                })
                .collect()
        } else if let Some(ref displayed) = displayed_fields {
            displayed.iter().copied().collect()
        } else {
            fields_ids_map.iter().map(|(id, _)| id).collect()
        };

        // Check if wildcard was used
        let use_all_fields = query
            .attributes_to_retrieve
            .as_ref()
            .is_some_and(|attrs| attrs.iter().any(|a| a == "*"))
            || query.attributes_to_retrieve.is_none();

        let final_fields: BTreeSet<_> = if use_all_fields {
            if let Some(ref displayed) = displayed_fields {
                displayed.iter().copied().collect()
            } else {
                fields_ids_map.iter().map(|(id, _)| id).collect()
            }
        } else {
            to_retrieve
        };

        let mut hits = Vec::with_capacity(milli_result.documents_ids.len());

        for (idx, &doc_id) in milli_result.documents_ids.iter().enumerate() {
            // Get document from index
            let document =
                index.document(rtxn, doc_id).map_err(|e| Error::search(e.to_string()))?;

            // Convert to JSON, filtering fields
            let mut doc_map = serde_json::Map::new();
            for (fid, value) in document.iter() {
                if final_fields.contains(&fid) {
                    if let Some(name) = fields_ids_map.name(fid) {
                        // Deserialize the obkv value
                        if let Ok(json_value) = serde_json::from_slice::<Value>(value) {
                            doc_map.insert(name.to_string(), json_value);
                        }
                    }
                }
            }

            let mut hit = SearchHit::from_document(Value::Object(doc_map));

            // Add ranking score if requested
            if query.show_ranking_score || query.show_ranking_score_details {
                if let Some(scores) = milli_result.document_scores.get(idx) {
                    use meilisearch_types::milli::score_details::ScoreDetails;

                    if query.show_ranking_score {
                        let score = ScoreDetails::global_score(scores.iter());
                        hit.ranking_score = Some(score);
                    }

                    if query.show_ranking_score_details {
                        // Use the built-in to_json_map method
                        let details = ScoreDetails::to_json_map(scores.iter());
                        hit.ranking_score_details = Some(Value::Object(details));
                    }
                }
            }

            hits.push(hit);
        }

        Ok(hits)
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Parse a filter value into a milli Filter.
fn parse_filter(
    filter_value: &Value,
) -> Result<Option<meilisearch_types::milli::Filter<'_>>, Error> {
    match filter_value {
        Value::String(s) if s.is_empty() => Ok(None),
        Value::Array(arr) if arr.is_empty() => Ok(None),
        _ => {
            let filter = meilisearch_types::milli::Filter::from_json(filter_value)
                .map_err(|e| Error::search(format!("invalid filter: {}", e)))?;
            Ok(filter)
        }
    }
}

/// Embed a query string using the given embedder.
fn embed_query(
    embedder: &meilisearch_types::milli::vector::Embedder,
    query: &str,
) -> Result<Vec<f32>, Error> {
    use meilisearch_types::milli::vector::SearchQuery;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let search_query = SearchQuery::Text(query);

    embedder
        .embed_search(search_query, Some(deadline))
        .map_err(|e| Error::search(format!("embedding error: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_query_new() {
        let query = SearchQuery::new("hello");
        assert_eq!(query.q, Some("hello".to_string()));
        assert_eq!(query.limit, 20);
        assert_eq!(query.offset, 0);
    }

    #[test]
    fn test_search_query_empty() {
        let query = SearchQuery::empty();
        assert!(query.q.is_none());
        assert_eq!(query.limit, 20);
    }

    #[test]
    fn test_search_query_with_hybrid() {
        let query = SearchQuery::new("test").with_hybrid(HybridQuery::new(0.7));

        assert!(query.hybrid.is_some());
        assert_eq!(query.hybrid.unwrap().semantic_ratio, 0.7);
    }

    #[test]
    fn test_search_query_with_pagination() {
        let query = SearchQuery::new("test").with_pagination(10, 50);

        assert_eq!(query.offset, 10);
        assert_eq!(query.limit, 50);
    }

    #[test]
    fn test_search_query_is_hybrid() {
        let keyword_only = SearchQuery::new("test");
        assert!(!keyword_only.is_hybrid());

        let hybrid = SearchQuery::new("test").with_hybrid(HybridQuery::new(0.5));
        assert!(hybrid.is_hybrid());
    }

    #[test]
    fn test_search_query_is_semantic_only() {
        let query = SearchQuery::empty().with_vector(vec![0.1, 0.2, 0.3]);

        assert!(query.is_semantic_only());
        assert!(!query.is_keyword_only());
        assert!(!query.is_hybrid());
    }

    #[test]
    fn test_search_query_is_keyword_only() {
        let query = SearchQuery::new("hello");

        assert!(query.is_keyword_only());
        assert!(!query.is_semantic_only());
        assert!(!query.is_hybrid());
    }

    #[test]
    fn test_hybrid_query_new() {
        let hybrid = HybridQuery::new(0.8);
        assert_eq!(hybrid.semantic_ratio, 0.8);
        assert!(hybrid.embedder.is_none());
    }

    #[test]
    fn test_hybrid_query_clamping() {
        // Values should be clamped to 0.0-1.0
        let too_high = HybridQuery::new(1.5);
        assert_eq!(too_high.semantic_ratio, 1.0);

        let too_low = HybridQuery::new(-0.5);
        assert_eq!(too_low.semantic_ratio, 0.0);
    }

    #[test]
    fn test_hybrid_query_with_embedder() {
        let hybrid = HybridQuery::new(0.5).with_embedder("my-embedder");

        assert_eq!(hybrid.embedder, Some("my-embedder".to_string()));
    }

    #[test]
    fn test_search_hit_from_document() {
        let doc = serde_json::json!({"id": 1, "title": "Test"});
        let hit = SearchHit::from_document(doc.clone());

        assert_eq!(hit.document, doc);
        assert!(hit.ranking_score.is_none());
    }

    #[test]
    fn test_search_hit_with_ranking_score() {
        let doc = serde_json::json!({"id": 1});
        let hit = SearchHit::from_document(doc).with_ranking_score(0.95);

        assert_eq!(hit.ranking_score, Some(0.95));
    }

    #[test]
    fn test_search_result_empty() {
        let result = SearchResult::empty("test query", 5);

        assert!(result.hits.is_empty());
        assert_eq!(result.estimated_total_hits, Some(0));
        assert_eq!(result.processing_time_ms, 5);
        assert_eq!(result.query, "test query");
    }

    #[test]
    fn test_default_semantic_ratio() {
        assert_eq!(default_semantic_ratio(), 0.5);
    }

    #[test]
    fn test_default_limit() {
        assert_eq!(default_limit(), 20);
    }

    #[test]
    fn test_default_crop_length() {
        assert_eq!(default_crop_length(), 10);
    }

    #[test]
    fn test_search_query_serialization() {
        let query =
            SearchQuery::new("hello").with_hybrid(HybridQuery::new(0.7)).with_pagination(5, 10);

        let json = serde_json::to_string(&query).unwrap();
        let parsed: SearchQuery = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.q, query.q);
        assert_eq!(parsed.offset, query.offset);
        assert_eq!(parsed.limit, query.limit);
        assert!(parsed.hybrid.is_some());
    }

    #[test]
    fn test_search_result_serialization() {
        let result = SearchResult {
            hits: vec![SearchHit::from_document(serde_json::json!({"id": 1}))],
            estimated_total_hits: Some(100),
            offset: Some(0),
            limit: Some(20),
            processing_time_ms: 15,
            query: "test".to_string(),
            semantic_hit_count: Some(50),
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"estimatedTotalHits\":100"));
        assert!(json.contains("\"processingTimeMs\":15"));
        assert!(json.contains("\"semanticHitCount\":50"));
    }
}
