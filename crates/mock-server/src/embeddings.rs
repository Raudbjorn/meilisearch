//! Mock embeddings endpoint - OpenAI-compatible `/v1/embeddings`.
//!
//! Generates deterministic embeddings using SHA-256 hash of input text.
//! This ensures consistent vectors for the same input across test runs.

use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::atomic::Ordering;
use tracing::{debug, info, instrument, warn};

use crate::error::MockServerError;
use crate::server::AppState;

// =============================================================================
// Constants
// =============================================================================

/// Supported embedding dimensions (OpenAI models).
pub const SUPPORTED_DIMENSIONS: &[usize] = &[256, 512, 1024, 1536, 3072];

/// Default embedding dimension (text-embedding-3-small).
pub const DEFAULT_DIMENSIONS: usize = 1536;

// =============================================================================
// Request/Response Types
// =============================================================================

/// OpenAI-compatible embedding request.
#[derive(Debug, Deserialize)]
pub struct EmbeddingRequest {
    /// Input text(s) to embed. Can be a single string or array of strings.
    pub input: EmbeddingInput,
    /// Model identifier (accepted but not used for mock).
    pub model: String,
    /// Optional: Number of dimensions for the embedding.
    #[serde(default)]
    pub dimensions: Option<usize>,
    /// Optional: Encoding format (float or base64).
    #[serde(default = "default_encoding_format")]
    pub encoding_format: String,
}

fn default_encoding_format() -> String {
    "float".to_string()
}

/// Input can be a single string or array of strings.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum EmbeddingInput {
    Single(String),
    Multiple(Vec<String>),
}

impl EmbeddingInput {
    /// Convert to a vector of strings for uniform processing.
    pub fn into_vec(self) -> Vec<String> {
        match self {
            EmbeddingInput::Single(s) => vec![s],
            EmbeddingInput::Multiple(v) => v,
        }
    }
}

/// OpenAI-compatible embedding response.
#[derive(Debug, Serialize)]
pub struct EmbeddingResponse {
    pub object: &'static str,
    pub data: Vec<EmbeddingData>,
    pub model: String,
    pub usage: EmbeddingUsage,
}

#[derive(Debug, Serialize)]
pub struct EmbeddingData {
    pub object: &'static str,
    pub embedding: Vec<f32>,
    pub index: usize,
}

#[derive(Debug, Serialize)]
pub struct EmbeddingUsage {
    pub prompt_tokens: usize,
    pub total_tokens: usize,
}

// =============================================================================
// Handler
// =============================================================================

/// Handle embedding requests.
///
/// # Endpoint
/// `POST /v1/embeddings`
///
/// # Example Request
/// ```json
/// {
///   "input": "movies about criminals",
///   "model": "text-embedding-3-small",
///   "dimensions": 1536
/// }
/// ```
#[instrument(skip(state, request), fields(model = %request.model))]
pub async fn handle_embeddings(
    State(state): State<AppState>,
    Json(request): Json<EmbeddingRequest>,
) -> Result<Json<EmbeddingResponse>, MockServerError> {
    let dimensions = request.dimensions.unwrap_or(DEFAULT_DIMENSIONS);

    // Validate dimensions
    if !SUPPORTED_DIMENSIONS.contains(&dimensions) {
        warn!(
            requested = dimensions,
            supported = ?SUPPORTED_DIMENSIONS,
            "Invalid dimensions requested"
        );
        return Err(MockServerError::InvalidDimensions {
            requested: dimensions,
            supported: SUPPORTED_DIMENSIONS,
        });
    }

    let inputs = request.input.into_vec();
    let input_count = inputs.len();

    // Validate inputs
    if inputs.is_empty() {
        return Err(MockServerError::InvalidRequest {
            field: "input".to_string(),
            reason: "Input cannot be empty".to_string(),
        });
    }

    // Sanitize and validate each input
    for (i, input) in inputs.iter().enumerate() {
        if input.is_empty() {
            return Err(MockServerError::InvalidRequest {
                field: format!("input[{}]", i),
                reason: "Input string cannot be empty".to_string(),
            });
        }
        // Limit input size to prevent abuse (OpenAI limit is ~8191 tokens)
        const MAX_INPUT_CHARS: usize = 32_768;
        if input.len() > MAX_INPUT_CHARS {
            warn!(
                input_index = i,
                input_len = input.len(),
                max = MAX_INPUT_CHARS,
                "Input exceeds maximum length"
            );
            return Err(MockServerError::InvalidRequest {
                field: format!("input[{}]", i),
                reason: format!(
                    "Input exceeds maximum length of {} characters",
                    MAX_INPUT_CHARS
                ),
            });
        }
    }

    debug!(
        input_count = input_count,
        dimensions = dimensions,
        "Generating embeddings"
    );

    // Generate embeddings
    let data: Vec<EmbeddingData> = inputs
        .iter()
        .enumerate()
        .map(|(index, text)| {
            let embedding = generate_deterministic_embedding(text, dimensions);
            EmbeddingData {
                object: "embedding",
                embedding,
                index,
            }
        })
        .collect();

    // Approximate token count (rough estimate: ~4 chars per token)
    let total_chars: usize = inputs.iter().map(|s| s.len()).sum();
    let approx_tokens = total_chars / 4 + 1;

    let response = EmbeddingResponse {
        object: "list",
        data,
        model: request.model,
        usage: EmbeddingUsage {
            prompt_tokens: approx_tokens,
            total_tokens: approx_tokens,
        },
    };

    state
        .metrics
        .embeddings_generated
        .fetch_add(input_count, Ordering::Relaxed);

    info!(
        input_count = input_count,
        dimensions = dimensions,
        tokens = approx_tokens,
        "Successfully generated embeddings"
    );

    Ok(Json(response))
}

// =============================================================================
// Embedding Generation
// =============================================================================

/// Generate a deterministic embedding vector from input text using SHA-256.
///
/// The algorithm:
/// 1. Hash the input text with SHA-256 (32 bytes)
/// 2. Use the hash as a seed to generate `dimensions` floats
/// 3. Normalize the vector to unit length (L2 norm = 1)
///
/// This ensures:
/// - Same input always produces same output (deterministic)
/// - Different inputs produce different outputs (high probability)
/// - Vectors are normalized for cosine similarity
pub fn generate_deterministic_embedding(text: &str, dimensions: usize) -> Vec<f32> {
    // Initial hash of the input text
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let initial_hash = hasher.finalize();

    let mut embedding = Vec::with_capacity(dimensions);
    let mut current_hash = initial_hash.to_vec();
    let mut hash_index = 0;

    // Generate enough floats to fill the embedding
    while embedding.len() < dimensions {
        if hash_index + 4 > current_hash.len() {
            // Need more hash bytes - hash the current hash
            let mut hasher = Sha256::new();
            hasher.update(&current_hash);
            current_hash = hasher.finalize().to_vec();
            hash_index = 0;
        }

        // Convert 4 bytes to a float in range [-1, 1]
        let bytes: [u8; 4] = current_hash[hash_index..hash_index + 4]
            .try_into()
            .expect("slice with incorrect length");
        let uint_val = u32::from_le_bytes(bytes);
        // Map to [-1, 1] range
        let float_val = (uint_val as f64 / u32::MAX as f64) * 2.0 - 1.0;
        embedding.push(float_val as f32);

        hash_index += 4;
    }

    // Normalize to unit vector (L2 norm = 1)
    normalize_vector(&mut embedding);

    embedding
}

/// Normalize a vector to unit length (L2 normalization).
fn normalize_vector(v: &mut [f32]) {
    let magnitude: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();

    if magnitude > f32::EPSILON {
        for x in v.iter_mut() {
            *x /= magnitude;
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deterministic_embedding_same_input() {
        let text = "movies about criminals";
        let emb1 = generate_deterministic_embedding(text, 1536);
        let emb2 = generate_deterministic_embedding(text, 1536);

        assert_eq!(emb1, emb2, "Same input should produce same embedding");
    }

    #[test]
    fn test_deterministic_embedding_different_inputs() {
        let emb1 = generate_deterministic_embedding("movies about criminals", 1536);
        let emb2 = generate_deterministic_embedding("romantic comedies", 1536);

        assert_ne!(
            emb1, emb2,
            "Different inputs should produce different embeddings"
        );
    }

    #[test]
    fn test_embedding_is_normalized() {
        let emb = generate_deterministic_embedding("test text", 1536);
        let magnitude: f32 = emb.iter().map(|x| x * x).sum::<f32>().sqrt();

        assert!(
            (magnitude - 1.0).abs() < 0.001,
            "Embedding should be normalized (L2 norm = 1), got {}",
            magnitude
        );
    }

    #[test]
    fn test_embedding_dimensions() {
        for &dim in SUPPORTED_DIMENSIONS {
            let emb = generate_deterministic_embedding("test", dim);
            assert_eq!(emb.len(), dim, "Embedding should have {} dimensions", dim);
        }
    }

    #[test]
    fn test_embedding_values_in_range() {
        let emb = generate_deterministic_embedding("test text", 1536);

        for (i, &val) in emb.iter().enumerate() {
            assert!(
                val >= -1.0 && val <= 1.0,
                "Value at index {} should be in [-1, 1], got {}",
                i,
                val
            );
        }
    }
}
