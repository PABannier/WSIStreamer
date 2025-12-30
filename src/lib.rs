//! WSI Streamer - A tile server for Whole Slide Images
//!
//! This library provides the core functionality for serving tiles from
//! Whole Slide Images stored in cloud object storage using HTTP range requests.

pub mod error;
pub mod io;

// Re-export commonly used types
pub use error::IoError;
pub use io::{RangeReader, S3RangeReader, create_s3_client};
