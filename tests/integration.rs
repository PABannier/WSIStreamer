//! Integration tests for WSI Streamer.
//!
//! These tests verify end-to-end functionality including:
//! - Tile retrieval for SVS and generic pyramidal TIFF formats
//! - Slides listing with pagination and extension filtering
//! - Authentication (valid, expired, invalid signatures)
//! - Error handling (missing slide, invalid coordinates, unsupported format)
//! - TIFF parser edge cases (endianness, BigTIFF)
//! - SVS JPEGTables handling
//! - Block cache effectiveness

mod integration {
    pub mod test_utils;

    pub mod api_tests;
    pub mod auth_tests;
    pub mod cache_tests;
    pub mod format_tests;
    pub mod slides_tests;
}
