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
    ByteOrder, Compression, FieldType, Ifd, IfdEntry, PyramidLevel, TiffHeader, TiffPyramid,
    TiffTag, TileData, ValidationError, ValidationResult, ValueReader, BIGTIFF_HEADER_SIZE,
    TIFF_HEADER_SIZE, check_compression, check_tile_tags, check_tiled, parse_u32_array,
    parse_u64_array, validate_ifd, validate_ifd_strict, validate_level, validate_pyramid,
};
pub use format::{SlideFormat, detect_format, is_tiff_header};
pub use format::{
    GenericTiffLevelData, GenericTiffReader,
    SvsLevelData, SvsMetadata, SvsReader,
    is_abbreviated_stream, is_complete_stream, merge_jpeg_tables, prepare_tile_jpeg,
};
pub use io::{BlockCache, RangeReader, S3RangeReader, create_s3_client};
pub use slide::{
    CachedSlide, LevelInfo, S3SlideSource, SlideListResult, SlideReader, SlideRegistry, SlideSource,
};
pub use tile::{
    JpegTileEncoder, TileCache, TileCacheKey, TileRequest, TileResponse, TileService,
    DEFAULT_JPEG_QUALITY, DEFAULT_TILE_CACHE_CAPACITY, MAX_JPEG_QUALITY, MIN_JPEG_QUALITY,
    clamp_quality, is_valid_quality,
};
pub use server::{
    AppState, ErrorResponse, HealthResponse, SlidesQueryParams, SlidesResponse, TilePathParams,
    TileQueryParams, health_handler, slides_handler, tile_handler,
    AuthError, AuthQueryParams, OptionalAuth, SignedUrlAuth, auth_middleware,
    RouterConfig, create_router, create_dev_router, create_production_router,
};
