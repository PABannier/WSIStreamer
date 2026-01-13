//! HTTP request handlers for the WSI Streamer tile API.
//!
//! This module contains the Axum handlers for serving tiles and health checks.
//!
//! # Endpoints
//!
//! - `GET /tiles/{slide_id}/{level}/{x}/{y}.jpg` - Serve a tile
//! - `GET /health` - Health check endpoint

use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::{debug, error, warn};

use crate::error::{FormatError, IoError, TiffError, TileError};
use crate::slide::SlideSource;
use crate::tile::{TileRequest, TileService, DEFAULT_JPEG_QUALITY};

use super::auth::SignedUrlAuth;

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

    /// Authentication configuration for generating signed URLs in the viewer
    pub auth: Option<SignedUrlAuth>,
}

impl<S: SlideSource> AppState<S> {
    /// Create a new application state with the given tile service.
    pub fn new(tile_service: TileService<S>) -> Self {
        Self {
            tile_service: Arc::new(tile_service),
            cache_max_age: 3600, // 1 hour default
            auth: None,
        }
    }

    /// Create a new application state with custom cache max-age.
    pub fn with_cache_max_age(tile_service: TileService<S>, cache_max_age: u32) -> Self {
        Self {
            tile_service: Arc::new(tile_service),
            cache_max_age,
            auth: None,
        }
    }

    /// Set authentication for the viewer to generate signed tile URLs.
    pub fn with_auth(mut self, auth: SignedUrlAuth) -> Self {
        self.auth = Some(auth);
        self
    }
}

impl<S: SlideSource> Clone for AppState<S> {
    fn clone(&self) -> Self {
        Self {
            tile_service: Arc::clone(&self.tile_service),
            cache_max_age: self.cache_max_age,
            auth: self.auth.clone(),
        }
    }
}

// =============================================================================
// Request Parameters
// =============================================================================

/// Path parameters for tile requests.
///
/// Extracted from: `/tiles/{slide_id}/{level}/{x}/{filename}`
/// where filename is `{y}` or `{y}.jpg`
#[derive(Debug, Deserialize)]
pub struct TilePathParams {
    /// Slide identifier (can be a path like "bucket/folder/slide.svs")
    pub slide_id: String,

    /// Pyramid level (0 = highest resolution)
    pub level: usize,

    /// Tile X coordinate (0-indexed from left)
    pub x: u32,

    /// Tile Y coordinate with optional .jpg extension (e.g., "0" or "0.jpg")
    pub filename: String,
}

impl TilePathParams {
    /// Parse the Y coordinate from the filename, stripping any .jpg extension.
    pub fn y(&self) -> Result<u32, std::num::ParseIntError> {
        let y_str = self.filename.strip_suffix(".jpg").unwrap_or(&self.filename);
        y_str.parse()
    }
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

/// Query parameters for the slides list endpoint.
#[derive(Debug, Deserialize)]
pub struct SlidesQueryParams {
    /// Maximum number of slides to return (default: 100, max: 1000)
    #[serde(default = "default_limit")]
    pub limit: u32,

    /// Continuation token for pagination (from previous response)
    #[serde(default)]
    pub cursor: Option<String>,

    /// Filter by path prefix (e.g., "folder/subfolder/")
    #[serde(default)]
    pub prefix: Option<String>,

    /// Search string to filter slide names (case-insensitive substring match)
    #[serde(default)]
    pub search: Option<String>,

    /// Signature for authentication (handled by auth middleware)
    #[serde(default)]
    pub sig: Option<String>,

    /// Expiry timestamp for authentication (handled by auth middleware)
    #[serde(default)]
    pub exp: Option<u64>,
}

fn default_limit() -> u32 {
    100
}

/// Query parameters for thumbnail requests.
#[derive(Debug, Deserialize)]
pub struct ThumbnailQueryParams {
    /// Maximum width or height for the thumbnail (default: 512, max: 2048)
    #[serde(default = "default_thumbnail_size")]
    pub max_size: u32,

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

fn default_thumbnail_size() -> u32 {
    512
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

/// Response from the slides list endpoint.
#[derive(Debug, Serialize)]
pub struct SlidesResponse {
    /// List of slide paths/IDs
    pub slides: Vec<String>,

    /// Continuation token for next page (None if no more pages)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Metadata for a single pyramid level.
#[derive(Debug, Serialize)]
pub struct LevelMetadataResponse {
    /// Pyramid level index (0 = highest resolution)
    pub level: usize,

    /// Width of this level in pixels
    pub width: u32,

    /// Height of this level in pixels
    pub height: u32,

    /// Width of each tile in pixels
    pub tile_width: u32,

    /// Height of each tile in pixels
    pub tile_height: u32,

    /// Number of tiles in X direction
    pub tiles_x: u32,

    /// Number of tiles in Y direction
    pub tiles_y: u32,

    /// Downsample factor relative to level 0
    pub downsample: f64,
}

/// Response from the slide metadata endpoint.
#[derive(Debug, Serialize)]
pub struct SlideMetadataResponse {
    /// Slide identifier
    pub slide_id: String,

    /// Detected slide format (e.g., "aperio_svs", "generic_tiff")
    pub format: String,

    /// Width of the full-resolution image in pixels
    pub width: u32,

    /// Height of the full-resolution image in pixels
    pub height: u32,

    /// Number of pyramid levels
    pub level_count: usize,

    /// Metadata for each pyramid level
    pub levels: Vec<LevelMetadataResponse>,
}

// =============================================================================
// Error Mapping
// =============================================================================

/// Convert TileError to HTTP response.
///
/// This implementation logs errors appropriately based on their severity:
/// - 4xx errors are logged at WARN level (client errors)
/// - 5xx errors are logged at ERROR level (server errors)
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

            // TIFF structure errors map to 415 Unsupported Media Type
            TileError::Slide(TiffError::Io(io_err)) => match io_err {
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
            },
            TileError::Slide(tiff_err) => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_format",
                tiff_err.to_string(),
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
        };

        // Log errors based on severity
        if status.is_server_error() {
            error!(
                error_type = error_type,
                status = status.as_u16(),
                "Server error: {}",
                message
            );
        } else if status.is_client_error() {
            // Log 404s at debug level (common and expected), others at warn
            if status == StatusCode::NOT_FOUND {
                debug!(
                    error_type = error_type,
                    status = status.as_u16(),
                    "Resource not found: {}",
                    message
                );
            } else {
                warn!(
                    error_type = error_type,
                    status = status.as_u16(),
                    "Client error: {}",
                    message
                );
            }
        }

        let error_response = ErrorResponse::with_status(error_type, message, status);

        (status, Json(error_response)).into_response()
    }
}

/// Convert FormatError to HTTP response.
///
/// FormatError typically indicates an unsupported file format (HTTP 415)
/// or an I/O error during format detection.
impl IntoResponse for FormatError {
    fn into_response(self) -> Response {
        let (status, error_type, message) = match &self {
            FormatError::Io(io_err) => match io_err {
                IoError::NotFound(path) => (
                    StatusCode::NOT_FOUND,
                    "not_found",
                    format!("Slide not found: {}", path),
                ),
                IoError::S3(msg) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    format!("Storage error: {}", msg),
                ),
                IoError::Connection(msg) => (
                    StatusCode::BAD_GATEWAY,
                    "connection_error",
                    format!("Connection error: {}", msg),
                ),
                IoError::RangeOutOfBounds { .. } => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "io_error",
                    format!("I/O error: {}", io_err),
                ),
            },

            FormatError::Tiff(tiff_err) => match tiff_err {
                TiffError::Io(io_err) => match io_err {
                    IoError::NotFound(path) => (
                        StatusCode::NOT_FOUND,
                        "not_found",
                        format!("Slide not found: {}", path),
                    ),
                    IoError::S3(msg) => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "storage_error",
                        format!("Storage error: {}", msg),
                    ),
                    IoError::Connection(msg) => (
                        StatusCode::BAD_GATEWAY,
                        "connection_error",
                        format!("Connection error: {}", msg),
                    ),
                    IoError::RangeOutOfBounds { .. } => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "io_error",
                        format!("I/O error: {}", io_err),
                    ),
                },
                _ => (
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    "unsupported_format",
                    tiff_err.to_string(),
                ),
            },

            FormatError::UnsupportedFormat { reason } => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_format",
                format!("Unsupported format: {}", reason),
            ),
        };

        // Log errors based on severity
        if status.is_server_error() {
            error!(
                error_type = error_type,
                status = status.as_u16(),
                "Server error: {}",
                message
            );
        } else if status == StatusCode::UNSUPPORTED_MEDIA_TYPE {
            warn!(
                error_type = error_type,
                status = status.as_u16(),
                "Unsupported format: {}",
                message
            );
        } else if status == StatusCode::NOT_FOUND {
            debug!(
                error_type = error_type,
                status = status.as_u16(),
                "Resource not found: {}",
                message
            );
        }

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

/// Wrapper for slides listing errors to implement IntoResponse.
pub struct SlidesError(pub IoError);

impl IntoResponse for SlidesError {
    fn into_response(self) -> Response {
        let (status, error_type, message) = match &self.0 {
            IoError::NotFound(path) => (
                StatusCode::NOT_FOUND,
                "not_found",
                format!("Resource not found: {}", path),
            ),
            IoError::S3(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                format!("Storage error: {}", msg),
            ),
            IoError::Connection(msg) => (
                StatusCode::BAD_GATEWAY,
                "connection_error",
                format!("Connection error: {}", msg),
            ),
            IoError::RangeOutOfBounds { .. } => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "io_error",
                format!("I/O error: {}", self.0),
            ),
        };

        // Log based on severity
        if status.is_server_error() {
            error!(
                error_type = error_type,
                status = status.as_u16(),
                "Server error: {}",
                message
            );
        } else {
            debug!(
                error_type = error_type,
                status = status.as_u16(),
                "Client error: {}",
                message
            );
        }

        let error_response = ErrorResponse::with_status(error_type, message, status);
        (status, Json(error_response)).into_response()
    }
}

impl From<IoError> for SlidesError {
    fn from(err: IoError) -> Self {
        SlidesError(err)
    }
}

/// Wrapper for slide metadata errors to implement IntoResponse.
pub struct SlideMetadataError(pub FormatError);

impl IntoResponse for SlideMetadataError {
    fn into_response(self) -> Response {
        self.0.into_response()
    }
}

impl From<FormatError> for SlideMetadataError {
    fn from(err: FormatError) -> Self {
        SlideMetadataError(err)
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
    // Parse Y coordinate from filename (handles both "0" and "0.jpg")
    let y = params.y().map_err(|_| {
        HandlerError(TileError::TileOutOfBounds {
            level: params.level,
            x: params.x,
            y: 0,
            max_x: 0,
            max_y: 0,
        })
    })?;

    // Build tile request
    let request =
        TileRequest::with_quality(&params.slide_id, params.level, params.x, y, query.quality);

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

/// Handle slides list requests.
///
/// # Endpoint
///
/// `GET /slides`
///
/// # Query Parameters
///
/// - `limit`: Maximum number of slides to return (default: 100, max: 1000)
/// - `cursor`: Continuation token for pagination (from previous response)
/// - `sig`: Authentication signature (for signed URLs)
/// - `exp`: Signature expiry timestamp (for signed URLs)
///
/// # Response
///
/// `200 OK` with JSON body:
/// ```json
/// {
///   "slides": ["path/to/slide1.svs", "path/to/slide2.tif"],
///   "next_cursor": "continuation_token_or_null"
/// }
/// ```
///
/// # Errors
///
/// - `401 Unauthorized`: Invalid or missing signature
/// - `500 Internal Server Error`: Storage error
pub async fn slides_handler<S: SlideSource>(
    State(state): State<AppState<S>>,
    Query(query): Query<SlidesQueryParams>,
) -> Result<Json<SlidesResponse>, SlidesError> {
    // Clamp limit to valid range (1-1000)
    let limit = query.limit.clamp(1, 1000);

    // List slides from the source with optional prefix filter
    let result = state
        .tile_service
        .registry()
        .source()
        .list_slides(limit, query.cursor.as_deref(), query.prefix.as_deref())
        .await?;

    // Apply search filter if provided (case-insensitive substring match)
    let slides = if let Some(ref search) = query.search {
        let search_lower = search.to_lowercase();
        result
            .slides
            .into_iter()
            .filter(|s| s.to_lowercase().contains(&search_lower))
            .collect()
    } else {
        result.slides
    };

    Ok(Json(SlidesResponse {
        slides,
        next_cursor: result.next_cursor,
    }))
}

/// Handle slide metadata requests.
///
/// # Endpoint
///
/// `GET /slides/{slide_id}`
///
/// # Path Parameters
///
/// - `slide_id`: Slide identifier (URL-encoded if contains special characters)
///
/// # Response
///
/// `200 OK` with JSON body containing slide metadata:
/// ```json
/// {
///   "slide_id": "path/to/slide.svs",
///   "format": "aperio_svs",
///   "width": 46920,
///   "height": 33600,
///   "level_count": 4,
///   "levels": [
///     {
///       "level": 0,
///       "width": 46920,
///       "height": 33600,
///       "tile_width": 256,
///       "tile_height": 256,
///       "tiles_x": 184,
///       "tiles_y": 132,
///       "downsample": 1.0
///     }
///   ]
/// }
/// ```
///
/// # Errors
///
/// - `401 Unauthorized`: Invalid or missing signature (when auth enabled)
/// - `404 Not Found`: Slide not found
/// - `415 Unsupported Media Type`: Slide format not supported
/// - `500 Internal Server Error`: Storage or processing error
pub async fn slide_metadata_handler<S: SlideSource>(
    State(state): State<AppState<S>>,
    Path(slide_id): Path<String>,
) -> Result<Json<SlideMetadataResponse>, SlideMetadataError> {
    // Get slide from registry (opens and caches if needed)
    let slide = state.tile_service.registry().get_slide(&slide_id).await?;

    // Get dimensions (should always be available for valid slides)
    let (width, height) = slide.dimensions().unwrap_or((0, 0));

    // Build level metadata for each pyramid level
    let level_count = slide.level_count();
    let levels: Vec<LevelMetadataResponse> = (0..level_count)
        .filter_map(|level| {
            slide.level_info(level).map(|info| LevelMetadataResponse {
                level,
                width: info.width,
                height: info.height,
                tile_width: info.tile_width,
                tile_height: info.tile_height,
                tiles_x: info.tiles_x,
                tiles_y: info.tiles_y,
                downsample: info.downsample,
            })
        })
        .collect();

    Ok(Json(SlideMetadataResponse {
        slide_id,
        format: slide.format().name().to_string(),
        width,
        height,
        level_count,
        levels,
    }))
}

/// Handle viewer requests - serves an HTML page with OpenSeadragon viewer.
///
/// # Endpoint
///
/// `GET /view/{slide_id}`
///
/// # Path Parameters
///
/// - `slide_id`: Slide identifier (URL-encoded if contains special characters)
///
/// # Response
///
/// `200 OK` with HTML page containing an embedded OpenSeadragon viewer.
///
/// # Errors
///
/// - `404 Not Found`: Slide not found
/// - `415 Unsupported Media Type`: Slide format not supported
/// - `500 Internal Server Error`: Storage or processing error
pub async fn viewer_handler<S: SlideSource>(
    State(state): State<AppState<S>>,
    Path(slide_id): Path<String>,
    headers: HeaderMap,
) -> Result<Html<String>, SlideMetadataError> {
    // Get slide from registry to retrieve metadata
    let slide = state.tile_service.registry().get_slide(&slide_id).await?;

    // Get dimensions
    let (width, height) = slide.dimensions().unwrap_or((0, 0));

    // Build level metadata
    let level_count = slide.level_count();
    let levels: Vec<LevelMetadataResponse> = (0..level_count)
        .filter_map(|level| {
            slide.level_info(level).map(|info| LevelMetadataResponse {
                level,
                width: info.width,
                height: info.height,
                tile_width: info.tile_width,
                tile_height: info.tile_height,
                tiles_x: info.tiles_x,
                tiles_y: info.tiles_y,
                downsample: info.downsample,
            })
        })
        .collect();

    let metadata = SlideMetadataResponse {
        slide_id: slide_id.clone(),
        format: slide.format().name().to_string(),
        width,
        height,
        level_count,
        levels,
    };

    // Extract host from headers, defaulting to localhost:3000
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost:3000");

    // Detect protocol from X-Forwarded-Proto header (for reverse proxy support)
    // or default to http for local development
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("http");

    // Generate the base URL from the host and protocol
    let base_url = format!("{}://{}", proto, host);

    // Generate viewer token if auth is enabled
    // This token authorizes access to all tiles for this specific slide
    let auth_query = state
        .auth
        .as_ref()
        .map(|auth| {
            // Generate viewer token valid for 1 hour
            let ttl = Duration::from_secs(3600);
            let (token, expiry) = auth.generate_viewer_token(&slide_id, ttl);
            format!("?vt={}&exp={}", token, expiry)
        })
        .unwrap_or_default();

    // Generate the viewer HTML with auth info
    let html = super::viewer::generate_viewer_html(&slide_id, &metadata, &base_url, &auth_query);

    Ok(Html(html))
}

/// Handle DZI descriptor requests - returns XML descriptor for Deep Zoom viewers.
///
/// # Endpoint
///
/// `GET /slides/{slide_id}/dzi`
///
/// # Path Parameters
///
/// - `slide_id`: Slide identifier
///
/// # Response
///
/// `200 OK` with XML body containing DZI descriptor.
///
/// # Example Response
///
/// ```xml
/// <?xml version="1.0" encoding="UTF-8"?>
/// <Image xmlns="http://schemas.microsoft.com/deepzoom/2008"
///        TileSize="256"
///        Overlap="0"
///        Format="jpg">
///   <Size Width="46920" Height="33600" />
/// </Image>
/// ```
///
/// # Errors
///
/// - `404 Not Found`: Slide not found
/// - `415 Unsupported Media Type`: Slide format not supported
/// - `500 Internal Server Error`: Storage or processing error
pub async fn dzi_descriptor_handler<S: SlideSource>(
    State(state): State<AppState<S>>,
    Path(slide_id): Path<String>,
) -> Result<Response, SlideMetadataError> {
    // Get slide from registry
    let slide = state.tile_service.registry().get_slide(&slide_id).await?;

    // Get dimensions
    let (width, height) = slide.dimensions().unwrap_or((0, 0));

    // Get tile size from level 0 (or default)
    let tile_size = slide.tile_size(0).map(|(w, _)| w).unwrap_or(256);

    // Generate DZI XML
    let xml = super::dzi::generate_dzi_xml(width, height, tile_size);

    // Build response with XML content type
    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/xml")
        .header(
            header::CACHE_CONTROL,
            format!("public, max-age={}", state.cache_max_age),
        )
        .body(axum::body::Body::from(xml))
        .unwrap();

    Ok(response)
}

/// Handle thumbnail requests - returns a low-resolution preview image.
///
/// # Endpoint
///
/// `GET /slides/{slide_id}/thumbnail`
///
/// # Path Parameters
///
/// - `slide_id`: Slide identifier (URL-encoded if contains special characters)
///
/// # Query Parameters
///
/// - `max_size`: Maximum width or height for the thumbnail (default: 512, max: 2048)
/// - `quality`: JPEG quality 1-100 (default: 80)
/// - `sig`: Authentication signature (for signed URLs)
/// - `exp`: Signature expiry timestamp (for signed URLs)
///
/// # Response
///
/// `200 OK` with JPEG thumbnail image.
///
/// # Errors
///
/// - `400 Bad Request`: Invalid quality or max_size parameter
/// - `404 Not Found`: Slide not found
/// - `415 Unsupported Media Type`: Slide format not supported
/// - `500 Internal Server Error`: Storage or processing error
pub async fn thumbnail_handler<S: SlideSource>(
    State(state): State<AppState<S>>,
    Path(slide_id): Path<String>,
    Query(query): Query<ThumbnailQueryParams>,
) -> Result<Response, HandlerError> {
    // Clamp max_size to reasonable bounds (64 to 2048)
    let requested_size = query.max_size;
    let max_size = requested_size.clamp(64, 2048);
    let was_clamped = max_size != requested_size;

    // Generate thumbnail
    let response = state
        .tile_service
        .generate_thumbnail(&slide_id, max_size, query.quality)
        .await?;

    // Build HTTP response with appropriate headers
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/jpeg")
        .header(
            header::CACHE_CONTROL,
            format!("public, max-age={}", state.cache_max_age),
        )
        .header("X-Tile-Cache-Hit", response.cache_hit.to_string())
        .header("X-Tile-Quality", response.quality.to_string());

    // Add header indicating if max_size was clamped
    if was_clamped {
        builder = builder
            .header("X-Thumbnail-Size-Clamped", "true")
            .header("X-Thumbnail-Requested-Size", requested_size.to_string())
            .header("X-Thumbnail-Actual-Size", max_size.to_string());
    }

    let http_response = builder.body(axum::body::Body::from(response.data)).unwrap();

    Ok(http_response)
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

    #[test]
    fn test_format_error_to_status_code() {
        // Test IoError::NotFound -> 404
        let err = FormatError::Io(IoError::NotFound("test.svs".to_string()));
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        // Test IoError::S3 -> 500
        let err = FormatError::Io(IoError::S3("connection refused".to_string()));
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        // Test IoError::Connection -> 502
        let err = FormatError::Io(IoError::Connection("timeout".to_string()));
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);

        // Test UnsupportedCompression -> 415
        let err = FormatError::Tiff(TiffError::UnsupportedCompression("LZW".to_string()));
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

        // Test StripOrganization -> 415
        let err = FormatError::Tiff(TiffError::StripOrganization);
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

        // Test InvalidMagic -> 415
        let err = FormatError::Tiff(TiffError::InvalidMagic(0x1234));
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

        // Test InvalidVersion -> 415
        let err = FormatError::Tiff(TiffError::InvalidVersion(99));
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

        // Test UnsupportedFormat -> 415
        let err = FormatError::UnsupportedFormat {
            reason: "not a TIFF file".to_string(),
        };
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

        // Test other TiffError -> 415
        let err = FormatError::Tiff(TiffError::MissingTag("TileOffsets"));
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[test]
    fn test_io_error_in_tile_error() {
        // Test NotFound via I/O -> 404
        let err = TileError::Io(IoError::NotFound("s3://bucket/slide.svs".to_string()));
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        // Test S3 error -> 500
        let err = TileError::Io(IoError::S3("access denied".to_string()));
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        // Test Connection error -> 500
        let err = TileError::Io(IoError::Connection("reset by peer".to_string()));
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_slides_query_params_defaults() {
        let params: SlidesQueryParams = serde_json::from_str("{}").unwrap();
        assert_eq!(params.limit, 100);
        assert!(params.cursor.is_none());
        assert!(params.sig.is_none());
        assert!(params.exp.is_none());
    }

    #[test]
    fn test_slides_query_params_with_values() {
        let params: SlidesQueryParams = serde_json::from_str(
            r#"{"limit": 50, "cursor": "token123", "sig": "abc", "exp": 1234567890}"#,
        )
        .unwrap();
        assert_eq!(params.limit, 50);
        assert_eq!(params.cursor, Some("token123".to_string()));
        assert_eq!(params.sig, Some("abc".to_string()));
        assert_eq!(params.exp, Some(1234567890));
    }

    #[test]
    fn test_slides_response_serialization() {
        let response = SlidesResponse {
            slides: vec!["slide1.svs".to_string(), "folder/slide2.tif".to_string()],
            next_cursor: Some("token123".to_string()),
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("slide1.svs"));
        assert!(json.contains("folder/slide2.tif"));
        assert!(json.contains("token123"));
    }

    #[test]
    fn test_slides_response_no_cursor() {
        let response = SlidesResponse {
            slides: vec!["slide.svs".to_string()],
            next_cursor: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(!json.contains("next_cursor"));
    }

    #[test]
    fn test_slides_error_to_status_code() {
        // Test NotFound -> 404
        let err = SlidesError(IoError::NotFound("bucket/slide.svs".to_string()));
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        // Test S3 -> 500
        let err = SlidesError(IoError::S3("access denied".to_string()));
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        // Test Connection -> 502
        let err = SlidesError(IoError::Connection("timeout".to_string()));
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn test_level_metadata_response_serialization() {
        let response = LevelMetadataResponse {
            level: 0,
            width: 46920,
            height: 33600,
            tile_width: 256,
            tile_height: 256,
            tiles_x: 184,
            tiles_y: 132,
            downsample: 1.0,
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"level\":0"));
        assert!(json.contains("\"width\":46920"));
        assert!(json.contains("\"height\":33600"));
        assert!(json.contains("\"tile_width\":256"));
        assert!(json.contains("\"tile_height\":256"));
        assert!(json.contains("\"tiles_x\":184"));
        assert!(json.contains("\"tiles_y\":132"));
        assert!(json.contains("\"downsample\":1.0"));
    }

    #[test]
    fn test_slide_metadata_response_serialization() {
        let response = SlideMetadataResponse {
            slide_id: "path/to/slide.svs".to_string(),
            format: "aperio_svs".to_string(),
            width: 46920,
            height: 33600,
            level_count: 2,
            levels: vec![
                LevelMetadataResponse {
                    level: 0,
                    width: 46920,
                    height: 33600,
                    tile_width: 256,
                    tile_height: 256,
                    tiles_x: 184,
                    tiles_y: 132,
                    downsample: 1.0,
                },
                LevelMetadataResponse {
                    level: 1,
                    width: 23460,
                    height: 16800,
                    tile_width: 256,
                    tile_height: 256,
                    tiles_x: 92,
                    tiles_y: 66,
                    downsample: 2.0,
                },
            ],
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"slide_id\":\"path/to/slide.svs\""));
        assert!(json.contains("\"format\":\"aperio_svs\""));
        assert!(json.contains("\"width\":46920"));
        assert!(json.contains("\"height\":33600"));
        assert!(json.contains("\"level_count\":2"));
        assert!(json.contains("\"levels\":["));
    }

    #[test]
    fn test_slide_metadata_response_empty_levels() {
        let response = SlideMetadataResponse {
            slide_id: "empty.tif".to_string(),
            format: "generic_tiff".to_string(),
            width: 0,
            height: 0,
            level_count: 0,
            levels: vec![],
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"levels\":[]"));
        assert!(json.contains("\"level_count\":0"));
    }

    #[test]
    fn test_slide_metadata_error_to_status_code() {
        // Test NotFound -> 404
        let err = SlideMetadataError(FormatError::Io(IoError::NotFound(
            "bucket/slide.svs".to_string(),
        )));
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        // Test UnsupportedFormat -> 415
        let err = SlideMetadataError(FormatError::UnsupportedFormat {
            reason: "not a TIFF file".to_string(),
        });
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

        // Test S3 error -> 500
        let err = SlideMetadataError(FormatError::Io(IoError::S3("access denied".to_string())));
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
