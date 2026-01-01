//! JPEG stream handling utilities.
//!
//! This module provides utilities for working with JPEG streams, specifically
//! for handling abbreviated JPEG streams used in Aperio SVS files.
//!
//! # Abbreviated JPEG Streams
//!
//! SVS files use "abbreviated JPEG streams" to save space. Each tile's JPEG
//! data is incomplete - it lacks the quantization (DQT) and Huffman (DHT)
//! tables needed for decoding. These tables are stored once in the TIFF's
//! `JPEGTables` tag and must be merged with each tile's data before decoding.
//!
//! # Merging Process
//!
//! 1. JPEGTables starts with SOI (FFD8) and ends with EOI (FFD9)
//! 2. Tile data also starts with SOI and ends with EOI
//! 3. To merge: strip EOI from tables, strip SOI from tile, concatenate
//!
//! Result: SOI + tables_content + tile_content + EOI

use bytes::{Bytes, BytesMut};

// =============================================================================
// JPEG Markers
// =============================================================================

/// Start Of Image marker
pub const SOI: [u8; 2] = [0xFF, 0xD8];

/// End Of Image marker
pub const EOI: [u8; 2] = [0xFF, 0xD9];

/// Start Of Frame (baseline DCT) marker
pub const SOF0: [u8; 2] = [0xFF, 0xC0];

/// Start Of Frame (progressive DCT) marker
pub const SOF2: [u8; 2] = [0xFF, 0xC2];

/// Define Huffman Table marker
pub const DHT: [u8; 2] = [0xFF, 0xC4];

/// Define Quantization Table marker
pub const DQT: [u8; 2] = [0xFF, 0xDB];

/// Define Restart Interval marker
pub const DRI: [u8; 2] = [0xFF, 0xDD];

/// Start Of Scan marker
pub const SOS: [u8; 2] = [0xFF, 0xDA];

/// Application segment 0 (JFIF) marker
pub const APP0: [u8; 2] = [0xFF, 0xE0];

/// Application segment 14 (Adobe) marker
pub const APP14: [u8; 2] = [0xFF, 0xEE];

// =============================================================================
// JPEG Stream Analysis
// =============================================================================

/// Check if JPEG data is an abbreviated stream (missing tables).
///
/// An abbreviated stream starts with SOI (FFD8) but is immediately followed
/// by SOS (FFDA) without any DQT (FFDB) or DHT (FFC4) markers in between.
///
/// # Arguments
/// * `data` - JPEG data to check
///
/// # Returns
/// `true` if the data appears to be an abbreviated stream that needs tables.
pub fn is_abbreviated_stream(data: &[u8]) -> bool {
    if data.len() < 4 {
        return false;
    }

    // Must start with SOI
    if data[0..2] != SOI {
        return false;
    }

    // Scan for markers after SOI
    let mut pos = 2;
    while pos + 1 < data.len() {
        if data[pos] != 0xFF {
            pos += 1;
            continue;
        }

        let marker = [data[pos], data[pos + 1]];

        // If we find DQT or DHT, it's a full stream
        if marker == DQT || marker == DHT {
            return false;
        }

        // If we find SOS first (without DQT/DHT), it's abbreviated
        if marker == SOS {
            return true;
        }

        // Skip marker segment (marker + 2-byte length + data)
        if pos + 3 < data.len() && marker[1] != 0x00 && marker[1] != 0xD8 && marker[1] != 0xD9 {
            let length = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
            pos += 2 + length;
        } else {
            pos += 2;
        }
    }

    // Didn't find SOS - inconclusive, treat as not abbreviated
    false
}

/// Check if JPEG data is a complete stream (has required tables).
///
/// A complete stream contains at least one DQT marker.
///
/// # Arguments
/// * `data` - JPEG data to check
///
/// # Returns
/// `true` if the data appears to be a complete JPEG stream.
pub fn is_complete_stream(data: &[u8]) -> bool {
    if data.len() < 4 {
        return false;
    }

    // Must start with SOI
    if data[0..2] != SOI {
        return false;
    }

    // Look for DQT marker
    for i in 2..data.len().saturating_sub(1) {
        if data[i] == 0xFF && data[i + 1] == 0xDB {
            return true;
        }
    }

    false
}

// =============================================================================
// JPEG Tables Merging
// =============================================================================

/// Merge JPEGTables with abbreviated tile data.
///
/// This function combines the tables (containing DQT/DHT markers) with
/// the tile's compressed data to create a complete, decodable JPEG.
///
/// # Arguments
/// * `tables` - JPEGTables data (starts with SOI, ends with EOI)
/// * `tile_data` - Abbreviated tile JPEG data (starts with SOI, ends with EOI)
///
/// # Returns
/// Complete JPEG data that can be decoded by standard JPEG libraries.
///
/// # Merge Algorithm
///
/// 1. Validate both inputs start with SOI
/// 2. Strip EOI from tables (if present)
/// 3. Strip SOI from tile data
/// 4. Concatenate: tables_content + tile_content
///
/// The result maintains proper JPEG structure: SOI + tables + scan data + EOI
pub fn merge_jpeg_tables(tables: &[u8], tile_data: &[u8]) -> Bytes {
    // Handle edge cases
    if tables.is_empty() {
        return Bytes::copy_from_slice(tile_data);
    }
    if tile_data.is_empty() {
        return Bytes::new();
    }

    // Find where tables content ends (strip trailing EOI if present)
    let tables_end = if tables.len() >= 2 && tables[tables.len() - 2..] == EOI {
        tables.len() - 2
    } else {
        tables.len()
    };

    // Find where tile content starts (skip leading SOI if present)
    let tile_start = if tile_data.len() >= 2 && tile_data[0..2] == SOI {
        2
    } else {
        0
    };

    // Calculate total size and allocate buffer
    let total_size = tables_end + (tile_data.len() - tile_start);
    let mut result = BytesMut::with_capacity(total_size);

    // Copy tables (including SOI at start, but not EOI at end)
    result.extend_from_slice(&tables[..tables_end]);

    // Copy tile data (excluding SOI at start, keeping EOI at end)
    result.extend_from_slice(&tile_data[tile_start..]);

    result.freeze()
}

/// Prepare tile data for decoding, merging tables if needed.
///
/// This is the main entry point for tile data preparation. It automatically
/// detects whether the tile data is abbreviated and merges tables if needed.
///
/// # Arguments
/// * `tables` - Optional JPEGTables data (may be None for generic TIFF)
/// * `tile_data` - Raw tile JPEG data
///
/// # Returns
/// Complete JPEG data ready for decoding.
pub fn prepare_tile_jpeg(tables: Option<&[u8]>, tile_data: &[u8]) -> Bytes {
    // If tile is already complete, return as-is
    if is_complete_stream(tile_data) {
        return Bytes::copy_from_slice(tile_data);
    }

    // If we have tables and tile is abbreviated, merge them
    if let Some(tables) = tables {
        if is_abbreviated_stream(tile_data) {
            return merge_jpeg_tables(tables, tile_data);
        }
    }

    // Fallback: return tile data as-is
    Bytes::copy_from_slice(tile_data)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // is_abbreviated_stream tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_abbreviated_stream_detection() {
        // Abbreviated: SOI followed directly by SOS (no tables)
        let abbreviated = [
            0xFF, 0xD8, // SOI
            0xFF, 0xDA, // SOS
            0x00, 0x08, // Length
            0x01, 0x01, 0x00, 0x00, 0x3F, 0x00, // SOS parameters
        ];
        assert!(is_abbreviated_stream(&abbreviated));
    }

    #[test]
    fn test_complete_stream_with_dqt() {
        // Complete: SOI + DQT + other data
        let complete = [
            0xFF, 0xD8, // SOI
            0xFF, 0xDB, // DQT
            0x00, 0x43, // Length (67 bytes)
            0x00, // Table ID
                  // ... quantization table data would follow
        ];
        assert!(!is_abbreviated_stream(&complete));
    }

    #[test]
    fn test_complete_stream_with_dht() {
        // Complete: SOI + DHT
        let complete = [
            0xFF, 0xD8, // SOI
            0xFF, 0xC4, // DHT
            0x00, 0x1F, // Length
        ];
        assert!(!is_abbreviated_stream(&complete));
    }

    #[test]
    fn test_abbreviated_empty() {
        assert!(!is_abbreviated_stream(&[]));
    }

    #[test]
    fn test_abbreviated_too_short() {
        assert!(!is_abbreviated_stream(&[0xFF, 0xD8]));
    }

    #[test]
    fn test_abbreviated_no_soi() {
        let no_soi = [0x00, 0x00, 0xFF, 0xDA];
        assert!(!is_abbreviated_stream(&no_soi));
    }

    // -------------------------------------------------------------------------
    // is_complete_stream tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_is_complete_with_dqt() {
        let complete = [
            0xFF, 0xD8, // SOI
            0xFF, 0xDB, // DQT
            0x00, 0x43, // Length
        ];
        assert!(is_complete_stream(&complete));
    }

    #[test]
    fn test_is_complete_without_dqt() {
        let incomplete = [
            0xFF, 0xD8, // SOI
            0xFF, 0xDA, // SOS (no DQT)
            0x00, 0x08,
        ];
        assert!(!is_complete_stream(&incomplete));
    }

    #[test]
    fn test_is_complete_empty() {
        assert!(!is_complete_stream(&[]));
    }

    #[test]
    fn test_is_complete_no_soi() {
        let no_soi = [0xFF, 0xDB, 0x00, 0x43];
        assert!(!is_complete_stream(&no_soi));
    }

    // -------------------------------------------------------------------------
    // merge_jpeg_tables tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_merge_basic() {
        // Tables: SOI + content + EOI
        let tables = [
            0xFF, 0xD8, // SOI
            0xFF, 0xDB, // DQT marker
            0x00, 0x05, 0x00, 0x10, 0x20, // DQT content
            0xFF, 0xD9, // EOI
        ];

        // Tile: SOI + SOS + data + EOI
        let tile = [
            0xFF, 0xD8, // SOI
            0xFF, 0xDA, // SOS
            0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00, // SOS content
            0x12, 0x34, 0x56, // Compressed data
            0xFF, 0xD9, // EOI
        ];

        let result = merge_jpeg_tables(&tables, &tile);

        // Expected: SOI + DQT content + SOS + data + EOI
        assert_eq!(&result[0..2], &SOI); // Starts with SOI
        assert_eq!(&result[2..4], &DQT); // Has DQT from tables
        assert_eq!(&result[result.len() - 2..], &EOI); // Ends with EOI

        // Should not have double SOI
        let soi_count = result.windows(2).filter(|w| *w == SOI).count();
        assert_eq!(soi_count, 1);
    }

    #[test]
    fn test_merge_empty_tables() {
        let tile = [0xFF, 0xD8, 0xFF, 0xDA, 0xFF, 0xD9];
        let result = merge_jpeg_tables(&[], &tile);
        assert_eq!(&result[..], &tile);
    }

    #[test]
    fn test_merge_empty_tile() {
        let tables = [0xFF, 0xD8, 0xFF, 0xDB, 0xFF, 0xD9];
        let result = merge_jpeg_tables(&tables, &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_merge_tables_without_eoi() {
        // Tables without trailing EOI
        let tables = [
            0xFF, 0xD8, // SOI
            0xFF, 0xDB, 0x00, 0x05, 0x00, 0x10, 0x20, // DQT
        ];

        let tile = [
            0xFF, 0xD8, // SOI
            0xFF, 0xDA, 0x00, 0x08, // SOS
            0xFF, 0xD9, // EOI
        ];

        let result = merge_jpeg_tables(&tables, &tile);

        // Should still work - tables content + tile content
        assert_eq!(&result[0..2], &SOI);
        assert_eq!(&result[result.len() - 2..], &EOI);
    }

    #[test]
    fn test_merge_tile_without_soi() {
        let tables = [
            0xFF, 0xD8, // SOI
            0xFF, 0xDB, 0x00, 0x05, 0x00, 0x10, 0x20, // DQT
            0xFF, 0xD9, // EOI
        ];

        // Tile without leading SOI (unusual but handled)
        let tile = [
            0xFF, 0xDA, 0x00, 0x08, // SOS
            0xFF, 0xD9, // EOI
        ];

        let result = merge_jpeg_tables(&tables, &tile);

        // Should work - SOI from tables + tile content
        assert_eq!(&result[0..2], &SOI);
        assert_eq!(&result[result.len() - 2..], &EOI);
    }

    // -------------------------------------------------------------------------
    // prepare_tile_jpeg tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_prepare_complete_tile() {
        // Complete tile - should be returned as-is
        let tile = [
            0xFF, 0xD8, // SOI
            0xFF, 0xDB, 0x00, 0x05, 0x00, 0x10, 0x20, // DQT
            0xFF, 0xC4, 0x00, 0x05, 0x00, 0x10, 0x20, // DHT
            0xFF, 0xDA, 0x00, 0x08, // SOS
            0xFF, 0xD9, // EOI
        ];

        let tables = [0xFF, 0xD8, 0xFF, 0xDB, 0x00, 0x05, 0xFF, 0xD9];

        let result = prepare_tile_jpeg(Some(&tables), &tile);
        assert_eq!(&result[..], &tile);
    }

    #[test]
    fn test_prepare_abbreviated_tile() {
        // Abbreviated tile - should be merged with tables
        let tile = [
            0xFF, 0xD8, // SOI
            0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00, // SOS
            0xFF, 0xD9, // EOI
        ];

        let tables = [
            0xFF, 0xD8, // SOI
            0xFF, 0xDB, 0x00, 0x05, 0x00, 0x10, 0x20, // DQT
            0xFF, 0xD9, // EOI
        ];

        let result = prepare_tile_jpeg(Some(&tables), &tile);

        // Result should have DQT from tables
        assert!(result.windows(2).any(|w| w == DQT));
        // And SOS from tile
        assert!(result.windows(2).any(|w| w == SOS));
    }

    #[test]
    fn test_prepare_no_tables() {
        let tile = [0xFF, 0xD8, 0xFF, 0xDA, 0xFF, 0xD9];

        let result = prepare_tile_jpeg(None, &tile);
        assert_eq!(&result[..], &tile);
    }
}
