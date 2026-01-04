//! WSI Streamer - A tile server for Whole Slide Images
//!
//! This library provides the core functionality for serving tiles from
//! Whole Slide Images stored in cloud object storage using HTTP range requests.

pub mod config;
pub mod error;
pub mod format;
pub mod io;
pub mod server;
pub mod slide;
pub mod tile;

// Re-export commonly used types
pub use config::Config;
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
    ErrorResponse, HealthResponse, LevelMetadataResponse, OptionalAuth, RouterConfig, SignedUrlAuth,
    SlideMetadataResponse, SlidesQueryParams, SlidesResponse, TilePathParams, TileQueryParams,
};
pub use slide::{
    CachedSlide, LevelInfo, S3SlideSource, SlideListResult, SlideReader, SlideRegistry, SlideSource,
};
pub use tile::{
    clamp_quality, is_valid_quality, JpegTileEncoder, TileCache, TileCacheKey, TileRequest,
    TileResponse, TileService, DEFAULT_JPEG_QUALITY, DEFAULT_TILE_CACHE_CAPACITY, MAX_JPEG_QUALITY,
    MIN_JPEG_QUALITY,
};
