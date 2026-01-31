//! Mock server lifecycle and configuration.
//!
//! Provides a simple API to start/stop the mock server for testing.

use axum::{
    http::{header, Method},
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
use std::{
    net::SocketAddr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tokio::{net::TcpListener, sync::oneshot};
use tower_http::cors::{Any, CorsLayer};
use tracing::{debug, error, info, instrument};

use crate::chat::handle_chat_completions;
use crate::embeddings::handle_embeddings;
use crate::error::{MockServerError, MockServerResult};

// =============================================================================
// Configuration
// =============================================================================

/// Configuration for the mock server.
#[derive(Debug, Clone)]
pub struct MockServerConfig {
    /// Address to bind to. Use "127.0.0.1:0" for random port.
    pub bind_address: String,
    /// Enable CORS for all origins (useful for browser-based tests).
    pub enable_cors: bool,
    /// Server name/identifier for logging.
    pub name: String,
}

impl Default for MockServerConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1:0".to_string(),
            enable_cors: true,
            name: "mock-openai".to_string(),
        }
    }
}

impl MockServerConfig {
    /// Create a new config with a specific port.
    pub fn with_port(port: u16) -> Self {
        Self {
            bind_address: format!("127.0.0.1:{}", port),
            ..Default::default()
        }
    }

    /// Create a new config for random port allocation.
    pub fn with_random_port() -> Self {
        Self::default()
    }
}

// =============================================================================
// Metrics
// =============================================================================

/// Server metrics for monitoring and testing.
#[derive(Debug, Default)]
pub struct ServerMetrics {
    /// Number of embedding requests processed.
    pub embeddings_generated: AtomicUsize,
    /// Number of chat completion requests processed.
    pub chat_completions: AtomicUsize,
    /// Total requests received.
    pub total_requests: AtomicUsize,
}

impl ServerMetrics {
    /// Reset all metrics to zero.
    pub fn reset(&self) {
        self.embeddings_generated.store(0, Ordering::Relaxed);
        self.chat_completions.store(0, Ordering::Relaxed);
        self.total_requests.store(0, Ordering::Relaxed);
    }

    /// Get current metrics as a snapshot.
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            embeddings_generated: self.embeddings_generated.load(Ordering::Relaxed),
            chat_completions: self.chat_completions.load(Ordering::Relaxed),
            total_requests: self.total_requests.load(Ordering::Relaxed),
        }
    }
}

/// Immutable snapshot of server metrics.
#[derive(Debug, Clone, Serialize)]
pub struct MetricsSnapshot {
    pub embeddings_generated: usize,
    pub chat_completions: usize,
    pub total_requests: usize,
}

// =============================================================================
// Application State
// =============================================================================

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    /// Server configuration.
    pub config: Arc<MockServerConfig>,
    /// Server metrics.
    pub metrics: Arc<ServerMetrics>,
}

// =============================================================================
// Mock Server
// =============================================================================

/// A mock OpenAI-compatible server for testing.
///
/// # Example
///
/// ```rust,no_run
/// use mock_server::{MockServer, MockServerConfig};
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     // Start with random port
///     let server = MockServer::start(MockServerConfig::default()).await?;
///
///     println!("Mock server running at: {}", server.url());
///     println!("Embeddings: {}/v1/embeddings", server.url());
///     println!("Chat: {}/v1/chat/completions", server.url());
///
///     // Use in tests...
///
///     // Graceful shutdown
///     server.shutdown().await;
///     Ok(())
/// }
/// ```
pub struct MockServer {
    /// The bound address.
    addr: SocketAddr,
    /// Shutdown signal sender.
    shutdown_tx: Option<oneshot::Sender<()>>,
    /// Server configuration.
    #[allow(dead_code)]
    config: Arc<MockServerConfig>,
    /// Server metrics.
    metrics: Arc<ServerMetrics>,
    /// Handle to the server task.
    _handle: tokio::task::JoinHandle<()>,
}

impl MockServer {
    /// Start a new mock server with the given configuration.
    #[instrument(skip(config), fields(name = %config.name, bind = %config.bind_address))]
    pub async fn start(config: MockServerConfig) -> MockServerResult<Self> {
        let config = Arc::new(config);
        let metrics = Arc::new(ServerMetrics::default());

        let state = AppState {
            config: config.clone(),
            metrics: metrics.clone(),
        };

        // Build router
        let app = build_router(state.clone());

        // Bind to address
        let listener = TcpListener::bind(&config.bind_address)
            .await
            .map_err(|e| MockServerError::BindError {
                address: config.bind_address.clone(),
                reason: e.to_string(),
            })?;

        let addr = listener
            .local_addr()
            .map_err(|e| MockServerError::Internal {
                reason: format!("Failed to get local address: {}", e),
            })?;

        info!(
            address = %addr,
            name = %config.name,
            "Mock server started"
        );

        // Setup graceful shutdown
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        // Spawn server task
        let handle = tokio::spawn(async move {
            let server = axum::serve(listener, app);

            tokio::select! {
                result = server => {
                    if let Err(e) = result {
                        error!(error = %e, "Server error");
                    }
                }
                _ = async {
                    let _ = shutdown_rx.await;
                } => {
                    debug!("Shutdown signal received");
                }
            }
        });

        Ok(Self {
            addr,
            shutdown_tx: Some(shutdown_tx),
            config,
            metrics,
            _handle: handle,
        })
    }

    /// Get the base URL of the server.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Get the embeddings endpoint URL.
    pub fn embeddings_url(&self) -> String {
        format!("{}/v1/embeddings", self.url())
    }

    /// Get the chat completions endpoint URL.
    pub fn chat_completions_url(&self) -> String {
        format!("{}/v1/chat/completions", self.url())
    }

    /// Get the bound address.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Get the bound port.
    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// Get server metrics.
    pub fn metrics(&self) -> &ServerMetrics {
        &self.metrics
    }

    /// Get a snapshot of current metrics.
    pub fn metrics_snapshot(&self) -> MetricsSnapshot {
        self.metrics.snapshot()
    }

    /// Reset all metrics.
    pub fn reset_metrics(&self) {
        self.metrics.reset();
    }

    /// Shutdown the server gracefully.
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
            info!(address = %self.addr, "Mock server shutdown initiated");
        }
    }
}

// =============================================================================
// Router
// =============================================================================

/// Build the application router.
fn build_router(state: AppState) -> Router {
    let mut router = Router::new()
        // OpenAI-compatible endpoints
        .route("/v1/embeddings", post(handle_embeddings))
        .route("/v1/chat/completions", post(handle_chat_completions))
        // Also support without /v1 prefix for flexibility
        .route("/embeddings", post(handle_embeddings))
        .route("/chat/completions", post(handle_chat_completions))
        // Health/metrics endpoints
        .route("/health", get(health_check))
        .route("/metrics", get(get_metrics))
        .route("/", get(root_handler));

    // Add CORS if enabled
    if state.config.enable_cors {
        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
            .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]);
        router = router.layer(cors);
    }

    router.with_state(state)
}

// =============================================================================
// Utility Handlers
// =============================================================================

/// Health check endpoint.
async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "mock-openai-server",
    })
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
}

/// Metrics endpoint.
async fn get_metrics(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Json<MetricsSnapshot> {
    Json(state.metrics.snapshot())
}

/// Root handler with API info.
async fn root_handler() -> Json<ApiInfo> {
    Json(ApiInfo {
        name: "Mock OpenAI Server",
        version: env!("CARGO_PKG_VERSION"),
        endpoints: vec![
            "/v1/embeddings".to_string(),
            "/v1/chat/completions".to_string(),
            "/health".to_string(),
            "/metrics".to_string(),
        ],
    })
}

#[derive(Serialize)]
struct ApiInfo {
    name: &'static str,
    version: &'static str,
    endpoints: Vec<String>,
}

// =============================================================================
// Test Utilities
// =============================================================================

/// Start a mock server for testing with a random port.
///
/// This is a convenience function for tests.
///
/// # Example
///
/// ```rust,no_run
/// #[tokio::test]
/// async fn test_embeddings() {
///     let server = mock_server::server::start_test_server().await.unwrap();
///
///     // Use server.embeddings_url() for requests
///
///     server.shutdown().await;
/// }
/// ```
pub async fn start_test_server() -> MockServerResult<MockServer> {
    MockServer::start(MockServerConfig::with_random_port()).await
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_server_starts_and_stops() {
        let server = start_test_server().await.expect("Server should start");
        assert!(server.port() > 0);

        let url = server.url();
        assert!(url.starts_with("http://"));

        server.shutdown().await;
    }

    #[tokio::test]
    async fn test_health_check() {
        let server = start_test_server().await.expect("Server should start");

        let client = reqwest::Client::new();
        let resp = client
            .get(format!("{}/health", server.url()))
            .send()
            .await
            .expect("Request should succeed");

        assert!(resp.status().is_success());

        server.shutdown().await;
    }
}
