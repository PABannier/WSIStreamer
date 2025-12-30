//! Generic pyramidal TIFF reader.
//!
//! This module provides support for reading standard pyramidal TIFF files
//! that use tiled organization with JPEG compression.
//!
//! # Supported Files
//!
//! This reader supports TIFF files that:
//! - Use tiled organization (not strips)
//! - Use JPEG compression (compression tag = 7)
//! - Have multiple resolution levels (pyramid structure)
//!
//! # Unsupported Files
//!
//! Files that don't meet these requirements return an error that can be
//! mapped to HTTP 415 Unsupported Media Type:
//! - Strip-based TIFFs
//! - Non-JPEG compression (LZW, Deflate, JPEG2000, etc.)
//! - Single-level TIFFs without pyramid structure

use bytes::Bytes;

use crate::error::TiffError;
use crate::io::RangeReader;

use super::jpeg::prepare_tile_jpeg;
use super::tiff::{
    validate_pyramid, PyramidLevel, TiffHeader, TiffPyramid, TileData, ValidationResult,
};

// =============================================================================
// Generic TIFF Level Data
// =============================================================================

/// Data for a single pyramid level in a generic TIFF file.
#[derive(Debug, Clone)]
pub struct GenericTiffLevelData {
    /// The pyramid level metadata
    pub level: PyramidLevel,

    /// Tile offsets and byte counts
    pub tile_data: TileData,
}

impl GenericTiffLevelData {
    /// Get the offset and size for a specific tile.
    pub fn get_tile_location(&self, tile_x: u32, tile_y: u32) -> Option<(u64, u64)> {
        let tile_index = self.level.tile_index(tile_x, tile_y)?;
        self.tile_data.get_tile_location(tile_index)
    }

    /// Get the JPEGTables for this level (if present).
    pub fn jpeg_tables(&self) -> Option<&Bytes> {
        self.tile_data.jpeg_tables.as_ref()
    }
}

// =============================================================================
// Generic TIFF Reader
// =============================================================================

/// Reader for generic pyramidal TIFF files.
///
/// This reader handles standard tiled TIFF files with JPEG compression.
/// It validates the file structure on open and rejects unsupported configurations.
#[derive(Debug)]
pub struct GenericTiffReader {
    /// Parsed TIFF pyramid structure
    pyramid: TiffPyramid,

    /// Level data including tile offsets and optional JPEGTables
    levels: Vec<GenericTiffLevelData>,

    /// Validation warnings (non-fatal issues)
    warnings: Vec<String>,
}

impl GenericTiffReader {
    /// Open a generic pyramidal TIFF file.
    ///
    /// This reads the TIFF structure, validates it meets requirements,
    /// and loads tile offset arrays.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The file is not a valid TIFF
    /// - The file uses strip organization (not tiles)
    /// - The file uses unsupported compression (not JPEG)
    /// - No pyramid levels are found
    pub async fn open<R: RangeReader>(reader: &R) -> Result<Self, TiffError> {
        // Parse the TIFF pyramid structure
        let pyramid = TiffPyramid::parse(reader).await?;

        // Validate the pyramid meets our requirements
        let validation = validate_pyramid(&pyramid);
        if !validation.is_valid {
            return Err(validation.into_result().unwrap_err());
        }

        // Store warnings for later inspection
        let warnings = validation.warnings;

        // Load tile data for each pyramid level
        let mut levels = Vec::with_capacity(pyramid.levels.len());
        for level in &pyramid.levels {
            let tile_data = TileData::load(reader, level, &pyramid.header).await?;
            levels.push(GenericTiffLevelData {
                level: level.clone(),
                tile_data,
            });
        }

        Ok(GenericTiffReader {
            pyramid,
            levels,
            warnings,
        })
    }

    /// Open a generic pyramidal TIFF with detailed validation result.
    ///
    /// This is like `open()` but returns the validation result separately,
    /// allowing access to warnings even on success.
    pub async fn open_with_validation<R: RangeReader>(
        reader: &R,
    ) -> Result<(Self, ValidationResult), TiffError> {
        // Parse the TIFF pyramid structure
        let pyramid = TiffPyramid::parse(reader).await?;

        // Validate the pyramid
        let validation = validate_pyramid(&pyramid);
        if !validation.is_valid {
            return Err(validation.clone().into_result().unwrap_err());
        }

        // Load tile data for each pyramid level
        let mut levels = Vec::with_capacity(pyramid.levels.len());
        for level in &pyramid.levels {
            let tile_data = TileData::load(reader, level, &pyramid.header).await?;
            levels.push(GenericTiffLevelData {
                level: level.clone(),
                tile_data,
            });
        }

        let reader = GenericTiffReader {
            pyramid,
            levels,
            warnings: validation.warnings.clone(),
        };

        Ok((reader, validation))
    }

    /// Get the TIFF header.
    pub fn header(&self) -> &TiffHeader {
        &self.pyramid.header
    }

    /// Get validation warnings from file open.
    ///
    /// Warnings indicate non-fatal issues like unusual tile dimensions.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Get the number of pyramid levels.
    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    /// Get data for a specific pyramid level.
    pub fn get_level(&self, level: usize) -> Option<&GenericTiffLevelData> {
        self.levels.get(level)
    }

    /// Get dimensions of the full-resolution (level 0) image.
    pub fn dimensions(&self) -> Option<(u32, u32)> {
        self.levels.first().map(|l| (l.level.width, l.level.height))
    }

    /// Get dimensions of a specific level.
    pub fn level_dimensions(&self, level: usize) -> Option<(u32, u32)> {
        self.levels
            .get(level)
            .map(|l| (l.level.width, l.level.height))
    }

    /// Get the downsample factor for a level.
    pub fn level_downsample(&self, level: usize) -> Option<f64> {
        self.levels.get(level).map(|l| l.level.downsample)
    }

    /// Get tile size for a level.
    pub fn tile_size(&self, level: usize) -> Option<(u32, u32)> {
        self.levels
            .get(level)
            .map(|l| (l.level.tile_width, l.level.tile_height))
    }

    /// Get the number of tiles in X and Y directions for a level.
    pub fn tile_count(&self, level: usize) -> Option<(u32, u32)> {
        self.levels
            .get(level)
            .map(|l| (l.level.tiles_x, l.level.tiles_y))
    }

    /// Read raw tile data from the file.
    ///
    /// This reads the raw bytes from the file without any processing.
    pub async fn read_raw_tile<R: RangeReader>(
        &self,
        reader: &R,
        level: usize,
        tile_x: u32,
        tile_y: u32,
    ) -> Result<Bytes, TiffError> {
        let level_data = self.levels.get(level).ok_or(TiffError::InvalidTagValue {
            tag: "level",
            message: format!("level {} out of range (max {})", level, self.levels.len()),
        })?;

        let (offset, size) =
            level_data
                .get_tile_location(tile_x, tile_y)
                .ok_or(TiffError::InvalidTagValue {
                    tag: "tile",
                    message: format!(
                        "tile ({}, {}) out of range for level {}",
                        tile_x, tile_y, level
                    ),
                })?;

        let data = reader.read_exact_at(offset, size as usize).await?;
        Ok(data)
    }

    /// Read a tile and prepare it for JPEG decoding.
    ///
    /// This reads the tile data and merges it with JPEGTables if the tile
    /// contains an abbreviated JPEG stream (rare for generic TIFF but handled).
    ///
    /// # Arguments
    /// * `reader` - Range reader for the file
    /// * `level` - Pyramid level index
    /// * `tile_x` - Tile X coordinate
    /// * `tile_y` - Tile Y coordinate
    ///
    /// # Returns
    /// Complete JPEG data ready for decoding.
    pub async fn read_tile<R: RangeReader>(
        &self,
        reader: &R,
        level: usize,
        tile_x: u32,
        tile_y: u32,
    ) -> Result<Bytes, TiffError> {
        // Read raw tile data
        let raw_data = self.read_raw_tile(reader, level, tile_x, tile_y).await?;

        // Get JPEGTables for this level (may not be present in generic TIFF)
        let level_data = self.levels.get(level).ok_or(TiffError::InvalidTagValue {
            tag: "level",
            message: format!("level {} out of range", level),
        })?;

        let tables = level_data.jpeg_tables();

        // Prepare the JPEG data (merge tables if needed)
        let jpeg_data = prepare_tile_jpeg(tables.map(|t| t.as_ref()), &raw_data);

        Ok(jpeg_data)
    }

    /// Find the best level for a given downsample factor.
    ///
    /// Returns the level with the smallest downsample that is >= the requested factor.
    pub fn best_level_for_downsample(&self, downsample: f64) -> Option<usize> {
        self.pyramid
            .best_level_for_downsample(downsample)
            .map(|l| l.level_index)
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::IoError;
    use crate::format::tiff::{FieldType, Ifd, IfdEntry, TiffTag};
    use crate::io::RangeReader;
    use async_trait::async_trait;
    use std::collections::HashMap;

    // Mock reader for testing
    struct MockTiffReader {
        // Simulates a minimal tiled TIFF with JPEG compression
        data: Vec<u8>,
    }

    impl MockTiffReader {
        fn new_valid_tiff() -> Self {
            // Create a minimal valid TIFF header and structure
            // This is a simplified mock - real tests would use actual TIFF files
            let mut data = vec![0u8; 1024];

            // Little-endian TIFF header
            data[0] = 0x49; // 'I'
            data[1] = 0x49; // 'I'
            data[2] = 0x2A; // Version 42
            data[3] = 0x00;
            data[4] = 0x08; // First IFD at offset 8
            data[5] = 0x00;
            data[6] = 0x00;
            data[7] = 0x00;

            // IFD at offset 8
            // Entry count = 7
            data[8] = 0x07;
            data[9] = 0x00;

            // This is a simplified structure - actual IFD parsing is complex
            // The real tests would use integration tests with actual files

            MockTiffReader { data }
        }
    }

    #[async_trait]
    impl RangeReader for MockTiffReader {
        async fn read_exact_at(&self, offset: u64, len: usize) -> Result<Bytes, IoError> {
            let start = offset as usize;
            let end = start + len;
            if end > self.data.len() {
                return Err(IoError::RangeOutOfBounds {
                    offset,
                    requested: len as u64,
                    size: self.data.len() as u64,
                });
            }
            Ok(Bytes::copy_from_slice(&self.data[start..end]))
        }

        fn size(&self) -> u64 {
            self.data.len() as u64
        }

        fn identifier(&self) -> &str {
            "mock://test.tif"
        }
    }

    // -------------------------------------------------------------------------
    // GenericTiffLevelData tests
    // -------------------------------------------------------------------------

    fn make_mock_level() -> GenericTiffLevelData {
        let ifd = Ifd {
            entries: vec![],
            entries_by_tag: HashMap::new(),
            next_ifd_offset: 0,
        };

        let level = PyramidLevel {
            level_index: 0,
            ifd_index: 0,
            width: 1000,
            height: 800,
            tile_width: 256,
            tile_height: 256,
            tiles_x: 4,
            tiles_y: 4,
            tile_count: 16,
            downsample: 1.0,
            compression: 7,
            ifd,
            tile_offsets_entry: Some(IfdEntry {
                tag_id: TiffTag::TileOffsets.as_u16(),
                field_type: Some(FieldType::Long),
                field_type_raw: 4,
                count: 16,
                value_offset_bytes: vec![0, 0, 0, 0],
                is_inline: false,
            }),
            tile_byte_counts_entry: Some(IfdEntry {
                tag_id: TiffTag::TileByteCounts.as_u16(),
                field_type: Some(FieldType::Long),
                field_type_raw: 4,
                count: 16,
                value_offset_bytes: vec![0, 0, 0, 0],
                is_inline: false,
            }),
            jpeg_tables_entry: None,
        };

        let tile_data = TileData {
            offsets: vec![1000, 2000, 3000, 4000, 5000, 6000, 7000, 8000, 9000, 10000, 11000, 12000, 13000, 14000, 15000, 16000],
            byte_counts: vec![500; 16],
            jpeg_tables: None,
        };

        GenericTiffLevelData { level, tile_data }
    }

    #[test]
    fn test_get_tile_location() {
        let level_data = make_mock_level();

        // First tile
        assert_eq!(level_data.get_tile_location(0, 0), Some((1000, 500)));

        // Second tile in first row
        assert_eq!(level_data.get_tile_location(1, 0), Some((2000, 500)));

        // First tile in second row
        assert_eq!(level_data.get_tile_location(0, 1), Some((5000, 500)));

        // Out of bounds
        assert_eq!(level_data.get_tile_location(10, 0), None);
        assert_eq!(level_data.get_tile_location(0, 10), None);
    }

    #[test]
    fn test_jpeg_tables_none() {
        let level_data = make_mock_level();
        assert!(level_data.jpeg_tables().is_none());
    }

    #[test]
    fn test_jpeg_tables_present() {
        let mut level_data = make_mock_level();
        level_data.tile_data.jpeg_tables = Some(Bytes::from_static(&[0xFF, 0xD8, 0xFF, 0xD9]));

        let tables = level_data.jpeg_tables();
        assert!(tables.is_some());
        assert_eq!(tables.unwrap().len(), 4);
    }
}
