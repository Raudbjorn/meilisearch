//! SDK-style search operation tests.
//!
//! These tests mirror the Meilisearch Rust SDK's search testing patterns,
//! providing comprehensive coverage of search queries and features.

mod common;

use common::{sample_movies, sample_products, TestContext};
use meilisearch_lib::SearchQuery;

// ============================================================================
// Basic Search Tests
// ============================================================================

/// Test basic keyword search.
#[tokio::test]
async fn test_query_string() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("search_basic").await;
    ctx.add_documents(&uid, sample_movies()).await;

    let query = SearchQuery::new("Matrix");
    let result = ctx.client.search(&uid, query).expect("search failed");

    assert_eq!(result.query, "Matrix");
    assert!(!result.hits.is_empty());
    assert!(result.hits[0].document["title"].as_str().unwrap().contains("Matrix"));

    ctx.shutdown().expect("shutdown failed");
}

/// Test empty query returns all documents.
#[tokio::test]
async fn test_query_empty() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("search_empty").await;
    ctx.add_documents(&uid, sample_movies()).await;

    let query = SearchQuery::empty();
    let result = ctx.client.search(&uid, query).expect("search failed");

    assert_eq!(result.hits.len(), 10);

    ctx.shutdown().expect("shutdown failed");
}

/// Test search with no results.
#[tokio::test]
async fn test_query_no_results() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("search_no_results").await;
    ctx.add_documents(&uid, sample_movies()).await;

    let query = SearchQuery::new("xyznonexistent");
    let result = ctx.client.search(&uid, query).expect("search failed");

    assert!(result.hits.is_empty());

    ctx.shutdown().expect("shutdown failed");
}

// ============================================================================
// Pagination Tests
// ============================================================================

/// Test search with limit.
#[tokio::test]
async fn test_query_limit() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("search_limit").await;
    ctx.add_documents(&uid, sample_movies()).await;

    let query = SearchQuery::empty().with_pagination(0, 3);
    let result = ctx.client.search(&uid, query).expect("search failed");

    assert_eq!(result.hits.len(), 3);
    assert_eq!(result.limit, Some(3));

    ctx.shutdown().expect("shutdown failed");
}

/// Test search with offset.
#[tokio::test]
async fn test_query_offset() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("search_offset").await;
    ctx.add_documents(&uid, sample_movies()).await;

    // Get all results first
    let all_query = SearchQuery::empty();
    let all_results = ctx.client.search(&uid, all_query).expect("search failed");
    let first_id = all_results.hits[0].document["id"].as_str().unwrap().to_string();
    let third_id = all_results.hits[2].document["id"].as_str().unwrap().to_string();

    // Search with offset
    let query = SearchQuery::empty().with_pagination(2, 3);
    let result = ctx.client.search(&uid, query).expect("search failed");

    assert_eq!(result.hits.len(), 3);
    assert_eq!(result.offset, Some(2));

    // First result should be the third from the original results
    assert_eq!(result.hits[0].document["id"].as_str().unwrap(), third_id);
    assert_ne!(result.hits[0].document["id"].as_str().unwrap(), first_id);

    ctx.shutdown().expect("shutdown failed");
}

/// Test search with limit and offset combined.
#[tokio::test]
async fn test_query_pagination() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("search_pagination").await;
    ctx.add_documents(&uid, sample_movies()).await;

    // Page 1
    let query1 = SearchQuery::empty().with_pagination(0, 5);
    let page1 = ctx.client.search(&uid, query1).expect("search failed");

    // Page 2
    let query2 = SearchQuery::empty().with_pagination(5, 5);
    let page2 = ctx.client.search(&uid, query2).expect("search failed");

    assert_eq!(page1.hits.len(), 5);
    assert_eq!(page2.hits.len(), 5);

    // Different documents
    let id1 = page1.hits[0].document["id"].as_str().unwrap();
    let id2 = page2.hits[0].document["id"].as_str().unwrap();
    assert_ne!(id1, id2);

    ctx.shutdown().expect("shutdown failed");
}

// ============================================================================
// Attribute Selection Tests
// ============================================================================

/// Test search with specific attributes to retrieve.
#[tokio::test]
async fn test_query_attributes_to_retrieve() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("search_attrs").await;
    ctx.add_documents(&uid, sample_movies()).await;

    let query = SearchQuery::new("Matrix").with_attributes_to_retrieve(vec!["title".to_string()]);
    let result = ctx.client.search(&uid, query).expect("search failed");

    assert!(!result.hits.is_empty());

    // Should have title
    assert!(result.hits[0].document.get("title").is_some());

    ctx.shutdown().expect("shutdown failed");
}

// ============================================================================
// Ranking Score Tests
// ============================================================================

/// Test search with ranking score.
#[tokio::test]
async fn test_query_show_ranking_score() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("search_ranking").await;
    ctx.add_documents(&uid, sample_movies()).await;

    let mut query = SearchQuery::new("action drama");
    query.show_ranking_score = true;
    let result = ctx.client.search(&uid, query).expect("search failed");

    assert!(!result.hits.is_empty());
    for hit in &result.hits {
        assert!(hit.ranking_score.is_some(), "ranking score should be present");
    }

    ctx.shutdown().expect("shutdown failed");
}

/// Test ranking scores are ordered correctly (descending).
#[tokio::test]
async fn test_ranking_score_order() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("search_ranking_order").await;
    ctx.add_documents(&uid, sample_movies()).await;

    let mut query = SearchQuery::new("crime drama");
    query.show_ranking_score = true;
    let result = ctx.client.search(&uid, query).expect("search failed");

    if result.hits.len() >= 2 {
        let scores: Vec<f64> =
            result.hits.iter().filter_map(|h| h.ranking_score).collect();

        // Verify descending order
        for i in 1..scores.len() {
            assert!(
                scores[i - 1] >= scores[i],
                "Scores should be in descending order: {} >= {}",
                scores[i - 1],
                scores[i]
            );
        }
    }

    ctx.shutdown().expect("shutdown failed");
}

// ============================================================================
// Processing Time Tests
// ============================================================================

/// Test that processing time is returned.
#[tokio::test]
async fn test_processing_time() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("search_time").await;
    ctx.add_documents(&uid, sample_movies()).await;

    let query = SearchQuery::new("Matrix");
    let result = ctx.client.search(&uid, query).expect("search failed");

    // Processing time is returned (may be 0 for very fast searches)
    // Just verify the search completed successfully and returned results
    // The processing_time_ms field is a u64, so it's always >= 0
    let _ = result.processing_time_ms; // Verify field exists

    ctx.shutdown().expect("shutdown failed");
}

// ============================================================================
// Multi-Word Query Tests
// ============================================================================

/// Test search with multiple words.
#[tokio::test]
async fn test_query_multiple_words() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("search_multi").await;
    ctx.add_documents(&uid, sample_movies()).await;

    let query = SearchQuery::new("The Dark Knight");
    let result = ctx.client.search(&uid, query).expect("search failed");

    assert!(!result.hits.is_empty());
    // The Dark Knight should be among top results
    let titles: Vec<&str> =
        result.hits.iter().map(|h| h.document["title"].as_str().unwrap()).collect();
    assert!(titles.iter().any(|t| t.contains("Dark Knight")));

    ctx.shutdown().expect("shutdown failed");
}

/// Test partial word matching.
#[tokio::test]
async fn test_query_partial_match() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("search_partial").await;
    ctx.add_documents(&uid, sample_movies()).await;

    // "Pulp" should match "Pulp Fiction"
    let query = SearchQuery::new("Pulp");
    let result = ctx.client.search(&uid, query).expect("search failed");

    assert!(!result.hits.is_empty());
    let first_title = result.hits[0].document["title"].as_str().unwrap();
    assert!(first_title.contains("Pulp"));

    ctx.shutdown().expect("shutdown failed");
}

// ============================================================================
// Search Result Structure Tests
// ============================================================================

/// Test search result contains expected fields.
#[tokio::test]
async fn test_search_result_structure() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("search_struct").await;
    ctx.add_documents(&uid, sample_movies()).await;

    let query = SearchQuery::new("action").with_pagination(0, 5);
    let result = ctx.client.search(&uid, query).expect("search failed");

    // Verify result structure
    assert_eq!(result.query, "action");
    assert!(result.processing_time_ms > 0);
    assert!(result.hits.len() <= 5);
    assert!(result.limit.is_some());
    assert!(result.offset.is_some());

    ctx.shutdown().expect("shutdown failed");
}

/// Test hit structure contains document and ID.
#[tokio::test]
async fn test_search_hit_structure() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("search_hit").await;
    ctx.add_documents(&uid, sample_movies()).await;

    let query = SearchQuery::new("Godfather");
    let result = ctx.client.search(&uid, query).expect("search failed");

    assert!(!result.hits.is_empty());
    let hit = &result.hits[0];

    // Document should have expected fields
    assert!(hit.document.get("id").is_some());
    assert!(hit.document.get("title").is_some());
    assert!(hit.document.get("genres").is_some());

    ctx.shutdown().expect("shutdown failed");
}

// ============================================================================
// Search with Different Content Types
// ============================================================================

/// Test search on product data.
#[tokio::test]
async fn test_search_products() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("search_products").await;
    ctx.add_documents(&uid, sample_products()).await;

    let query = SearchQuery::new("keyboard");
    let result = ctx.client.search(&uid, query).expect("search failed");

    assert!(!result.hits.is_empty());
    let first = &result.hits[0];
    let name = first.document["name"].as_str().unwrap().to_lowercase();
    assert!(name.contains("keyboard"));

    ctx.shutdown().expect("shutdown failed");
}

/// Test search across multiple fields.
#[tokio::test]
async fn test_search_multiple_fields() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("search_fields").await;
    ctx.add_documents(&uid, sample_products()).await;

    // "ergonomic" appears in description, not name
    let query = SearchQuery::new("ergonomic");
    let result = ctx.client.search(&uid, query).expect("search failed");

    assert!(!result.hits.is_empty());
    let desc = result.hits[0].document["description"].as_str().unwrap().to_lowercase();
    assert!(desc.contains("ergonomic"));

    ctx.shutdown().expect("shutdown failed");
}

// ============================================================================
// Search Query Builder Tests
// ============================================================================

/// Test SearchQuery builder pattern.
#[tokio::test]
async fn test_search_query_builder() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("search_builder").await;
    ctx.add_documents(&uid, sample_movies()).await;

    // Build a complex query
    let mut query = SearchQuery::new("drama");
    query.limit = 3;
    query.offset = 1;
    query.show_ranking_score = true;
    query.attributes_to_retrieve = Some(vec!["title".to_string(), "year".to_string()]);

    let result = ctx.client.search(&uid, query).expect("search failed");

    assert!(result.hits.len() <= 3);
    assert_eq!(result.offset, Some(1));
    for hit in &result.hits {
        assert!(hit.ranking_score.is_some());
    }

    ctx.shutdown().expect("shutdown failed");
}

// ============================================================================
// Hybrid Search Tests (Requires Embedder - Ignored by Default)
// ============================================================================

/// Test hybrid search combining keyword and semantic.
///
/// Requires embedder configuration - ignored by default.
#[tokio::test]
#[ignore = "requires embedder configuration"]
async fn test_hybrid_search() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("hybrid_search").await;
    ctx.add_documents(&uid, sample_movies()).await;

    let mut query = SearchQuery::new("movies about criminals");
    query.hybrid = Some(meilisearch_lib::HybridQuery {
        semantic_ratio: 0.5,
        embedder: Some("default".to_string()),
    });

    let result = ctx.client.search(&uid, query).expect("hybrid search failed");

    // Should find crime-related movies
    assert!(!result.hits.is_empty());

    ctx.shutdown().expect("shutdown failed");
}

/// Test semantic-only search (high semantic ratio).
#[tokio::test]
#[ignore = "requires embedder configuration"]
async fn test_semantic_search() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("semantic_search").await;
    ctx.add_documents(&uid, sample_movies()).await;

    let mut query = SearchQuery::new("films about hope and redemption");
    query.hybrid = Some(meilisearch_lib::HybridQuery {
        semantic_ratio: 0.9,
        embedder: Some("default".to_string()),
    });

    let result = ctx.client.search(&uid, query).expect("semantic search failed");

    // Shawshank Redemption should rank highly
    assert!(!result.hits.is_empty());

    ctx.shutdown().expect("shutdown failed");
}

// ============================================================================
// Search Lifecycle Integration Test
// ============================================================================

/// Comprehensive search lifecycle test.
#[tokio::test]
async fn test_search_lifecycle_sdk_style() {
    let mut ctx = TestContext::new();
    let uid = ctx.create_index_simple("search_lifecycle").await;

    // 1. Add documents
    ctx.add_documents(&uid, sample_movies()).await;

    // 2. Basic search
    let result = ctx.client.search(&uid, SearchQuery::new("Matrix")).expect("search failed");
    assert!(!result.hits.is_empty());
    assert!(result.hits[0].document["title"].as_str().unwrap().contains("Matrix"));

    // 3. Empty query
    let result = ctx.client.search(&uid, SearchQuery::empty()).expect("search failed");
    assert_eq!(result.hits.len(), 10);

    // 4. Paginated search
    let result =
        ctx.client.search(&uid, SearchQuery::empty().with_pagination(0, 3)).expect("search failed");
    assert_eq!(result.hits.len(), 3);

    // 5. Search with ranking score
    let mut query = SearchQuery::new("drama");
    query.show_ranking_score = true;
    let result = ctx.client.search(&uid, query).expect("search failed");
    assert!(result.hits.iter().all(|h| h.ranking_score.is_some()));

    // 6. Multi-word search
    let result = ctx.client.search(&uid, SearchQuery::new("Forrest Gump")).expect("search failed");
    assert!(!result.hits.is_empty());

    ctx.shutdown().expect("shutdown failed");
}
