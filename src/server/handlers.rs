//! HTTP request handlers for the WSI Streamer tile API.
//!
//! This module contains the Axum handlers for serving tiles and health checks.
//!
//! # Endpoints
//!
//! - `GET /tiles/{slide_id}/{level}/{x}/{y}.jpg` - Serve a tile
//! - `GET /health` - Health check endpoint

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::error::{IoError, TiffError, TileError};
use crate::slide::SlideSource;
use crate::tile::{TileRequest, TileService, DEFAULT_JPEG_QUALITY};

// =============================================================================
// Application State
// =============================================================================

/// Shared application state containing the tile service.
///
/// This is passed to all handlers via Axum's State extractor.
pub struct AppState<S: SlideSource> {
    /// The tile service for processing tile requests
    pub tile_service: Arc<TileService<S>>,

    /// Default cache control max-age in seconds (defaults to 1 hour)
    pub cache_max_age: u32,
}

impl<S: SlideSource> AppState<S> {
    /// Create a new application state with the given tile service.
    pub fn new(tile_service: TileService<S>) -> Self {
        Self {
            tile_service: Arc::new(tile_service),
            cache_max_age: 3600, // 1 hour default
        }
    }

    /// Create a new application state with custom cache max-age.
    pub fn with_cache_max_age(tile_service: TileService<S>, cache_max_age: u32) -> Self {
        Self {
            tile_service: Arc::new(tile_service),
            cache_max_age,
        }
    }
}

impl<S: SlideSource> Clone for AppState<S> {
    fn clone(&self) -> Self {
        Self {
            tile_service: Arc::clone(&self.tile_service),
            cache_max_age: self.cache_max_age,
        }
    }
}

// =============================================================================
// Request Parameters
// =============================================================================

/// Path parameters for tile requests.
///
/// Extracted from: `/tiles/{slide_id}/{level}/{x}/{y}.jpg`
#[derive(Debug, Deserialize)]
pub struct TilePathParams {
    /// Slide identifier (can be a path like "bucket/folder/slide.svs")
    pub slide_id: String,

    /// Pyramid level (0 = highest resolution)
    pub level: usize,

    /// Tile X coordinate (0-indexed from left)
    pub x: u32,

    /// Tile Y coordinate (0-indexed from top)
    pub y: u32,
}

/// Query parameters for tile requests.
#[derive(Debug, Deserialize)]
pub struct TileQueryParams {
    /// JPEG quality (1-100, defaults to 80)
    #[serde(default = "default_quality")]
    pub quality: u8,

    /// Signature for authentication (handled by auth middleware)
    #[serde(default)]
    pub sig: Option<String>,

    /// Expiry timestamp for authentication (handled by auth middleware)
    #[serde(default)]
    pub exp: Option<u64>,
}

fn default_quality() -> u8 {
    DEFAULT_JPEG_QUALITY
}

// =============================================================================
// Response Types
// =============================================================================

/// JSON error response returned for all error conditions.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    /// Error type identifier (e.g., "not_found", "invalid_request")
    pub error: String,

    /// Human-readable error message
    pub message: String,

    /// HTTP status code (included for convenience)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
}

impl ErrorResponse {
    /// Create a new error response.
    pub fn new(error: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            message: message.into(),
            status: None,
        }
    }

    /// Create a new error response with status code.
    pub fn with_status(
        error: impl Into<String>,
        message: impl Into<String>,
        status: StatusCode,
    ) -> Self {
        Self {
            error: error.into(),
            message: message.into(),
            status: Some(status.as_u16()),
        }
    }
}

/// Health check response.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    /// Service status
    pub status: String,

    /// Service version
    pub version: String,
}

// =============================================================================
// Error Mapping
// =============================================================================

/// Convert TileError to HTTP response.
impl IntoResponse for TileError {
    fn into_response(self) -> Response {
        let (status, error_type, message) = match &self {
            // 404 Not Found
            TileError::SlideNotFound { slide_id } => (
                StatusCode::NOT_FOUND,
                "not_found",
                format!("Slide not found: {}", slide_id),
            ),

            // 400 Bad Request - Invalid parameters
            TileError::InvalidLevel { level, max_levels } => (
                StatusCode::BAD_REQUEST,
                "invalid_level",
                format!(
                    "Invalid level: {} (slide has {} levels, valid range: 0-{})",
                    level,
                    max_levels,
                    max_levels.saturating_sub(1)
                ),
            ),

            TileError::TileOutOfBounds {
                level,
                x,
                y,
                max_x,
                max_y,
            } => (
                StatusCode::BAD_REQUEST,
                "tile_out_of_bounds",
                format!(
                    "Tile coordinates ({}, {}) at level {} are out of bounds (max: {}, {})",
                    x,
                    y,
                    level,
                    max_x.saturating_sub(1),
                    max_y.saturating_sub(1)
                ),
            ),

            TileError::InvalidQuality { quality } => (
                StatusCode::BAD_REQUEST,
                "invalid_quality",
                format!("Invalid quality: {} (must be 1-100)", quality),
            ),

            // 415 Unsupported Media Type - Format not supported
            TileError::Slide(TiffError::UnsupportedCompression(compression)) => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_format",
                format!("Unsupported compression: {} (only JPEG is supported)", compression),
            ),

            TileError::Slide(TiffError::StripOrganization) => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_format",
                "Unsupported organization: file uses strips instead of tiles".to_string(),
            ),

            // 500 Internal Server Error - I/O and processing errors
            TileError::Io(io_err) => {
                // Map specific I/O errors
                match io_err {
                    IoError::NotFound(path) => (
                        StatusCode::NOT_FOUND,
                        "not_found",
                        format!("Resource not found: {}", path),
                    ),
                    _ => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "io_error",
                        format!("I/O error: {}", io_err),
                    ),
                }
            }

            TileError::DecodeError { message } => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "decode_error",
                format!("Failed to decode tile: {}", message),
            ),

            TileError::EncodeError { message } => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "encode_error",
                format!("Failed to encode tile: {}", message),
            ),

            // Other slide/TIFF errors
            TileError::Slide(tiff_err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "slide_error",
                format!("Slide processing error: {}", tiff_err),
            ),
        };

        let error_response = ErrorResponse::with_status(error_type, message, status);

        (status, Json(error_response)).into_response()
    }
}

/// Wrapper for handler errors to implement IntoResponse.
pub struct HandlerError(pub TileError);

impl IntoResponse for HandlerError {
    fn into_response(self) -> Response {
        self.0.into_response()
    }
}

impl From<TileError> for HandlerError {
    fn from(err: TileError) -> Self {
        HandlerError(err)
    }
}

// =============================================================================
// Handlers
// =============================================================================

/// Handle tile requests.
///
/// # Endpoint
///
/// `GET /tiles/{slide_id}/{level}/{x}/{y}.jpg`
///
/// # Path Parameters
///
/// - `slide_id`: Slide identifier (URL-encoded if contains special characters)
/// - `level`: Pyramid level (0 = highest resolution)
/// - `x`: Tile X coordinate
/// - `y`: Tile Y coordinate
///
/// # Query Parameters
///
/// - `quality`: JPEG quality 1-100 (default: 80)
/// - `sig`: Authentication signature (optional, for signed URLs)
/// - `exp`: Signature expiry timestamp (optional, for signed URLs)
///
/// # Response
///
/// - `200 OK`: JPEG tile image with `Content-Type: image/jpeg`
/// - `400 Bad Request`: Invalid level or tile coordinates
/// - `404 Not Found`: Slide not found
/// - `415 Unsupported Media Type`: Slide format not supported
/// - `500 Internal Server Error`: Processing error
///
/// # Headers
///
/// - `Content-Type: image/jpeg`
/// - `Cache-Control: public, max-age={cache_max_age}`
/// - `X-Tile-Cache-Hit: true|false`
pub async fn tile_handler<S: SlideSource>(
    State(state): State<AppState<S>>,
    Path(params): Path<TilePathParams>,
    Query(query): Query<TileQueryParams>,
) -> Result<Response, HandlerError> {
    // Build tile request
    let request = TileRequest::with_quality(
        &params.slide_id,
        params.level,
        params.x,
        params.y,
        query.quality,
    );

    // Get tile from service
    let response = state.tile_service.get_tile(request).await?;

    // Build HTTP response with appropriate headers
    let http_response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/jpeg")
        .header(
            header::CACHE_CONTROL,
            format!("public, max-age={}", state.cache_max_age),
        )
        .header("X-Tile-Cache-Hit", response.cache_hit.to_string())
        .header("X-Tile-Quality", response.quality.to_string())
        .body(axum::body::Body::from(response.data))
        .unwrap();

    Ok(http_response)
}

/// Handle health check requests.
///
/// # Endpoint
///
/// `GET /health`
///
/// # Response
///
/// `200 OK` with JSON body:
/// ```json
/// {
///   "status": "healthy",
///   "version": "0.1.0"
/// }
/// ```
pub async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[test]
    fn test_error_response_serialization() {
        let response = ErrorResponse::new("test_error", "Test message");
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("test_error"));
        assert!(json.contains("Test message"));
        assert!(!json.contains("status")); // status is None, should be skipped
    }

    #[test]
    fn test_error_response_with_status() {
        let response =
            ErrorResponse::with_status("not_found", "Slide not found", StatusCode::NOT_FOUND);
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("404"));
    }

    #[test]
    fn test_tile_error_to_status_code() {
        // Test SlideNotFound -> 404
        let err = TileError::SlideNotFound {
            slide_id: "test.svs".to_string(),
        };
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        // Test InvalidLevel -> 400
        let err = TileError::InvalidLevel {
            level: 5,
            max_levels: 3,
        };
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        // Test TileOutOfBounds -> 400
        let err = TileError::TileOutOfBounds {
            level: 0,
            x: 100,
            y: 100,
            max_x: 10,
            max_y: 10,
        };
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        // Test UnsupportedCompression -> 415
        let err = TileError::Slide(TiffError::UnsupportedCompression("LZW".to_string()));
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

        // Test StripOrganization -> 415
        let err = TileError::Slide(TiffError::StripOrganization);
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

        // Test DecodeError -> 500
        let err = TileError::DecodeError {
            message: "test".to_string(),
        };
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_health_response_serialization() {
        let response = HealthResponse {
            status: "healthy".to_string(),
            version: "0.1.0".to_string(),
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("healthy"));
        assert!(json.contains("0.1.0"));
    }

    #[test]
    fn test_tile_query_params_defaults() {
        // Test that default quality is applied
        let params: TileQueryParams = serde_json::from_str("{}").unwrap();
        assert_eq!(params.quality, DEFAULT_JPEG_QUALITY);
        assert!(params.sig.is_none());
        assert!(params.exp.is_none());
    }

    #[test]
    fn test_tile_query_params_with_values() {
        let params: TileQueryParams =
            serde_json::from_str(r#"{"quality": 95, "sig": "abc123", "exp": 1234567890}"#).unwrap();
        assert_eq!(params.quality, 95);
        assert_eq!(params.sig, Some("abc123".to_string()));
        assert_eq!(params.exp, Some(1234567890));
    }
}
