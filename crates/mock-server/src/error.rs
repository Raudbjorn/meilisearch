//! Error types for the mock server.
//!
//! Follows errors-as-values pattern - no panics, explicit error handling.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use std::fmt;

/// Result type alias for mock server operations.
pub type MockServerResult<T> = Result<T, MockServerError>;

/// Errors that can occur in the mock server.
#[derive(Debug, Clone)]
pub enum MockServerError {
    /// Failed to bind to the specified address.
    BindError { address: String, reason: String },
    /// Invalid request format or missing required fields.
    InvalidRequest { field: String, reason: String },
    /// Server is not running.
    NotRunning,
    /// Internal server error.
    Internal { reason: String },
    /// Unsupported model requested.
    UnsupportedModel { model: String },
    /// Invalid embedding dimensions requested.
    InvalidDimensions {
        requested: usize,
        supported: &'static [usize],
    },
}

impl fmt::Display for MockServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BindError { address, reason } => {
                write!(f, "Failed to bind to {}: {}", address, reason)
            }
            Self::InvalidRequest { field, reason } => {
                write!(f, "Invalid request field '{}': {}", field, reason)
            }
            Self::NotRunning => write!(f, "Mock server is not running"),
            Self::Internal { reason } => write!(f, "Internal error: {}", reason),
            Self::UnsupportedModel { model } => {
                write!(f, "Unsupported model: {}", model)
            }
            Self::InvalidDimensions {
                requested,
                supported,
            } => {
                write!(
                    f,
                    "Invalid dimensions {}, supported: {:?}",
                    requested, supported
                )
            }
        }
    }
}

impl std::error::Error for MockServerError {}

/// OpenAI-compatible error response format.
#[derive(Debug, Serialize)]
pub struct ApiErrorResponse {
    pub error: ApiError,
}

#[derive(Debug, Serialize)]
pub struct ApiError {
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: String,
    pub code: Option<String>,
}

impl IntoResponse for MockServerError {
    fn into_response(self) -> Response {
        let (status, error_type, code) = match &self {
            MockServerError::InvalidRequest { .. } => (
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                Some("invalid_request"),
            ),
            MockServerError::UnsupportedModel { .. } => (
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                Some("model_not_found"),
            ),
            MockServerError::InvalidDimensions { .. } => (
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                Some("invalid_dimensions"),
            ),
            MockServerError::BindError { .. } | MockServerError::Internal { .. } => {
                (StatusCode::INTERNAL_SERVER_ERROR, "server_error", None)
            }
            MockServerError::NotRunning => (
                StatusCode::SERVICE_UNAVAILABLE,
                "server_error",
                Some("server_not_running"),
            ),
        };

        let body = ApiErrorResponse {
            error: ApiError {
                message: self.to_string(),
                error_type: error_type.to_string(),
                code: code.map(String::from),
            },
        };

        (status, Json(body)).into_response()
    }
}
