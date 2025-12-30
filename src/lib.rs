//! WSI Streamer - A tile server for Whole Slide Images
//!
//! This library provides the core functionality for serving tiles from
//! Whole Slide Images stored in cloud object storage using HTTP range requests.

pub mod error;
pub mod format;
pub mod io;

// Re-export commonly used types
pub use error::{IoError, TiffError};
pub use format::tiff::{
    ByteOrder, Compression, FieldType, Ifd, IfdEntry, TiffHeader, TiffTag, ValueReader,
    BIGTIFF_HEADER_SIZE, TIFF_HEADER_SIZE, parse_u32_array, parse_u64_array,
};
pub use io::{BlockCache, RangeReader, S3RangeReader, create_s3_client};
