//! Router configuration for WSI Streamer.
//!
//! This module defines the HTTP routes and applies middleware for authentication
//! and CORS.
//!
//! # Route Structure
//!
//! ```text
//! /health                                    - Health check (public)
//! /tiles/{slide_id}/{level}/{x}/{y}.jpg      - Tile endpoint (protected)
//! /slides                                    - List slides (protected)
//! ```
//!
//! # Example
//!
//! ```ignore
//! use wsi_streamer::server::routes::{create_router, RouterConfig};
//! use wsi_streamer::tile::TileService;
//! use wsi_streamer::slide::SlideRegistry;
//!
//! // Create the tile service
//! let registry = SlideRegistry::new(source);
//! let tile_service = TileService::new(registry);
//!
//! // Configure and create router
//! let config = RouterConfig::new("my-secret-key")
//!     .with_cors_origins(vec!["https://example.com".to_string()]);
//!
//! let router = create_router(tile_service, config);
//!
//! // Run the server
//! let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
//! axum::serve(listener, router).await?;
//! ```

use std::time::Duration;

use axum::{middleware, routing::get, Router};
use http::header::{AUTHORIZATION, CONTENT_TYPE};
use http::Method;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use super::auth::SignedUrlAuth;
use super::handlers::{health_handler, slides_handler, tile_handler, AppState};
use crate::slide::SlideSource;
use crate::tile::TileService;

// =============================================================================
// Router Configuration
// =============================================================================

/// Configuration for the HTTP router.
#[derive(Clone)]
pub struct RouterConfig {
    /// Secret key for signed URL authentication
    pub auth_secret: String,

    /// Whether authentication is enabled for tile requests
    pub auth_enabled: bool,

    /// Allowed CORS origins (None = allow any origin)
    pub cors_origins: Option<Vec<String>>,

    /// Cache-Control max-age in seconds
    pub cache_max_age: u32,

    /// Whether to enable request tracing
    pub enable_tracing: bool,
}

impl RouterConfig {
    /// Create a new router configuration with the given auth secret.
    ///
    /// By default:
    /// - Authentication is enabled
    /// - CORS allows any origin
    /// - Cache max-age is 1 hour (3600 seconds)
    /// - Tracing is enabled
    pub fn new(auth_secret: impl Into<String>) -> Self {
        Self {
            auth_secret: auth_secret.into(),
            auth_enabled: true,
            cors_origins: None, // Allow any origin by default
            cache_max_age: 3600,
            enable_tracing: true,
        }
    }

    /// Create a configuration with authentication disabled.
    ///
    /// **Warning**: This should only be used for development/testing.
    pub fn without_auth() -> Self {
        Self {
            auth_secret: String::new(),
            auth_enabled: false,
            cors_origins: None,
            cache_max_age: 3600,
            enable_tracing: true,
        }
    }

    /// Set specific allowed CORS origins.
    ///
    /// Pass an empty vec to disallow all cross-origin requests.
    /// Pass None (or don't call this method) to allow any origin.
    pub fn with_cors_origins(mut self, origins: Vec<String>) -> Self {
        self.cors_origins = Some(origins);
        self
    }

    /// Allow any CORS origin.
    pub fn with_cors_any_origin(mut self) -> Self {
        self.cors_origins = None;
        self
    }

    /// Set the Cache-Control max-age in seconds.
    pub fn with_cache_max_age(mut self, seconds: u32) -> Self {
        self.cache_max_age = seconds;
        self
    }

    /// Enable or disable authentication.
    pub fn with_auth_enabled(mut self, enabled: bool) -> Self {
        self.auth_enabled = enabled;
        self
    }

    /// Enable or disable request tracing.
    pub fn with_tracing(mut self, enabled: bool) -> Self {
        self.enable_tracing = enabled;
        self
    }
}

// =============================================================================
// Router Builder
// =============================================================================

/// Create the main application router.
///
/// This function builds the complete Axum router with:
/// - Public routes (health check)
/// - Protected routes (tile API with optional auth)
/// - CORS configuration
/// - Request tracing (optional)
///
/// # Arguments
///
/// * `tile_service` - The tile service for handling tile requests
/// * `config` - Router configuration
///
/// # Returns
///
/// A configured Axum router ready to be served.
pub fn create_router<S>(tile_service: TileService<S>, config: RouterConfig) -> Router
where
    S: SlideSource + 'static,
{
    // Create application state
    let app_state = AppState::with_cache_max_age(tile_service, config.cache_max_age);

    // Create the auth layer if enabled
    let auth = SignedUrlAuth::new(&config.auth_secret);

    // Build CORS layer
    let cors = build_cors_layer(&config);

    // Build the router
    let router = if config.auth_enabled {
        build_protected_router(app_state, auth, cors)
    } else {
        build_public_router(app_state, cors)
    };

    // Add tracing if enabled
    if config.enable_tracing {
        router.layer(TraceLayer::new_for_http())
    } else {
        router
    }
}

/// Build router with authentication on tile and slides routes.
fn build_protected_router<S>(app_state: AppState<S>, auth: SignedUrlAuth, cors: CorsLayer) -> Router
where
    S: SlideSource + 'static,
{
    // Protected tile routes (require authentication)
    // Uses {filename} to capture both "{y}" and "{y}.jpg" formats
    // Auth middleware is applied to the nested router AFTER nesting so it sees the full /tiles/... path
    let tile_routes = Router::new()
        .route("/{slide_id}/{level}/{x}/{filename}", get(tile_handler::<S>))
        .with_state(app_state.clone());

    // Protected slides list route (require authentication)
    let slides_routes = Router::new()
        .route("/", get(slides_handler::<S>))
        .with_state(app_state.clone());

    // Create nested routes with auth applied AFTER nesting
    let protected_routes = Router::new()
        .nest("/tiles", tile_routes)
        .nest("/slides", slides_routes)
        .layer(middleware::from_fn_with_state(
            auth,
            super::auth::auth_middleware,
        ));

    // Public routes (no auth required)
    let public_routes = Router::new().route("/health", get(health_handler));

    // Combine routes
    Router::new()
        .merge(protected_routes)
        .merge(public_routes)
        .layer(cors)
}

/// Build router without authentication (for development/testing).
fn build_public_router<S>(app_state: AppState<S>, cors: CorsLayer) -> Router
where
    S: SlideSource + 'static,
{
    // All routes are public
    // Uses {filename} to capture both "{y}" and "{y}.jpg" formats
    Router::new()
        .route("/health", get(health_handler))
        .route(
            "/tiles/{slide_id}/{level}/{x}/{filename}",
            get(tile_handler::<S>),
        )
        .route("/slides", get(slides_handler::<S>))
        .with_state(app_state)
        .layer(cors)
}

/// Build the CORS layer based on configuration.
fn build_cors_layer(config: &RouterConfig) -> CorsLayer {
    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::HEAD, Method::OPTIONS])
        .allow_headers([AUTHORIZATION, CONTENT_TYPE])
        .max_age(Duration::from_secs(86400)); // 24 hours

    match &config.cors_origins {
        None => cors.allow_origin(Any),
        Some(origins) if origins.is_empty() => {
            // No origins allowed - this effectively disables CORS
            cors
        }
        Some(origins) => {
            // Parse origins into HeaderValues
            let parsed_origins: Vec<_> = origins.iter().filter_map(|o| o.parse().ok()).collect();
            cors.allow_origin(parsed_origins)
        }
    }
}

// =============================================================================
// Convenience Functions
// =============================================================================

/// Create a development router with authentication disabled.
///
/// **Warning**: This should only be used for local development and testing.
/// Never use this in production.
pub fn create_dev_router<S>(tile_service: TileService<S>) -> Router
where
    S: SlideSource + 'static,
{
    create_router(tile_service, RouterConfig::without_auth())
}

/// Create a production router with the given secret key.
///
/// Uses secure defaults:
/// - Authentication enabled
/// - 1 hour cache max-age
/// - Tracing enabled
/// - CORS allows any origin (configure as needed)
pub fn create_production_router<S>(
    tile_service: TileService<S>,
    auth_secret: impl Into<String>,
) -> Router
where
    S: SlideSource + 'static,
{
    create_router(tile_service, RouterConfig::new(auth_secret))
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_router_config_defaults() {
        let config = RouterConfig::new("secret");
        assert_eq!(config.auth_secret, "secret");
        assert!(config.auth_enabled);
        assert!(config.cors_origins.is_none());
        assert_eq!(config.cache_max_age, 3600);
        assert!(config.enable_tracing);
    }

    #[test]
    fn test_router_config_without_auth() {
        let config = RouterConfig::without_auth();
        assert!(!config.auth_enabled);
        assert!(config.auth_secret.is_empty());
    }

    #[test]
    fn test_router_config_builder() {
        let config = RouterConfig::new("secret")
            .with_cors_origins(vec!["https://example.com".to_string()])
            .with_cache_max_age(7200)
            .with_auth_enabled(false)
            .with_tracing(false);

        assert_eq!(config.auth_secret, "secret");
        assert!(!config.auth_enabled);
        assert_eq!(
            config.cors_origins,
            Some(vec!["https://example.com".to_string()])
        );
        assert_eq!(config.cache_max_age, 7200);
        assert!(!config.enable_tracing);
    }

    #[test]
    fn test_router_config_cors_any() {
        let config = RouterConfig::new("secret")
            .with_cors_origins(vec!["https://example.com".to_string()])
            .with_cors_any_origin();

        assert!(config.cors_origins.is_none());
    }

    #[test]
    fn test_build_cors_layer_any_origin() {
        let config = RouterConfig::new("secret");
        let _cors = build_cors_layer(&config);
        // Just verify it doesn't panic
    }

    #[test]
    fn test_build_cors_layer_specific_origins() {
        let config = RouterConfig::new("secret").with_cors_origins(vec![
            "https://example.com".to_string(),
            "https://other.com".to_string(),
        ]);
        let _cors = build_cors_layer(&config);
        // Just verify it doesn't panic
    }

    #[test]
    fn test_build_cors_layer_empty_origins() {
        let config = RouterConfig::new("secret").with_cors_origins(vec![]);
        let _cors = build_cors_layer(&config);
        // Just verify it doesn't panic
    }
}
