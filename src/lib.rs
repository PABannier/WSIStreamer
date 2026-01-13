//! # WSI Streamer
//!
//! A tile server for Whole Slide Images (WSI) stored in S3-compatible object storage.
//!
//! This library provides the core functionality for serving tiles from
//! Whole Slide Images stored in cloud object storage using HTTP range requests.
//! It streams tiles directly without downloading entire files, making it ideal
//! for large medical imaging files (1-10GB+).
//!
//! ## Features
//!
//! - **Range-based streaming**: Fetches only the bytes needed for each tile via HTTP range requests
//! - **Format support**: Native parsers for Aperio SVS and pyramidal TIFF formats
//! - **Multi-level caching**: Caches slides, blocks, and encoded tiles for performance
//! - **Built-in web viewer**: Includes OpenSeadragon-based viewer
//! - **Authentication**: Optional HMAC-SHA256 signed URL authentication
//!
//! ## Architecture
//!
//! The library is organized into several modules:
//!
//! - [`io`] - I/O layer with S3 range reader and block caching
//! - [`mod@format`] - TIFF/SVS parsers and JPEG handling
//! - [`slide`] - Slide abstraction and registry
//! - [`tile`] - Tile service and encoding
//! - [`server`] - Axum-based HTTP server and routes
//! - [`config`] - CLI and configuration types
//!
//! ## Example
//!
//! ```rust,no_run
//! use wsi_streamer::Cli;
//! use clap::Parser;
//!
//! #[tokio::main]
//! async fn main() {
//!     // Parse CLI arguments (e.g., `wsi-streamer s3://my-bucket`)
//!     let cli = Cli::parse();
//!
//!     match cli.into_command() {
//!         wsi_streamer::Command::Serve(config) => {
//!             // Validate and start the server
//!             config.validate().expect("Invalid configuration");
//!             println!("Starting server on {}", config.bind_address());
//!         }
//!         wsi_streamer::Command::Sign(config) => {
//!             // Generate signed URL
//!         }
//!         wsi_streamer::Command::Check(config) => {
//!             // Validate S3 connectivity
//!         }
//!     }
//! }
//! ```

pub mod config;
pub mod error;
pub mod format;
pub mod io;
pub mod server;
pub mod slide;
pub mod tile;

// Re-export commonly used types
pub use config::{CheckConfig, Cli, Command, Config, ServeConfig, SignConfig, SignOutputFormat};
pub use error::{FormatError, IoError, TiffError, TileError};
pub use format::tiff::{
    check_compression, check_tile_tags, check_tiled, parse_u32_array, parse_u64_array,
    validate_ifd, validate_ifd_strict, validate_level, validate_pyramid, ByteOrder, Compression,
    FieldType, Ifd, IfdEntry, PyramidLevel, TiffHeader, TiffPyramid, TiffTag, TileData,
    ValidationError, ValidationResult, ValueReader, BIGTIFF_HEADER_SIZE, TIFF_HEADER_SIZE,
};
pub use format::{detect_format, is_tiff_header, SlideFormat};
pub use format::{
    is_abbreviated_stream, is_complete_stream, merge_jpeg_tables, prepare_tile_jpeg,
    GenericTiffLevelData, GenericTiffReader, SvsLevelData, SvsMetadata, SvsReader,
};
pub use io::{create_s3_client, BlockCache, RangeReader, S3RangeReader};
pub use server::{
    auth_middleware, create_dev_router, create_production_router, create_router, health_handler,
    slide_metadata_handler, slides_handler, tile_handler, AppState, AuthError, AuthQueryParams,
    ErrorResponse, HealthResponse, LevelMetadataResponse, OptionalAuth, RouterConfig,
    SignedUrlAuth, SlideMetadataResponse, SlidesQueryParams, SlidesResponse, TilePathParams,
    TileQueryParams,
};
pub use slide::{
    CachedSlide, LevelInfo, S3SlideSource, SlideListResult, SlideReader, SlideRegistry, SlideSource,
};
pub use tile::{
    clamp_quality, is_valid_quality, JpegTileEncoder, TileCache, TileCacheKey, TileRequest,
    TileResponse, TileService, DEFAULT_JPEG_QUALITY, DEFAULT_TILE_CACHE_CAPACITY, MAX_JPEG_QUALITY,
    MIN_JPEG_QUALITY,
};
