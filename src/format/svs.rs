//! Aperio SVS format reader.
//!
//! This module provides support for reading Aperio SVS files, a TIFF-based
//! format commonly used for whole slide imaging.
//!
//! # SVS File Structure
//!
//! SVS files are TIFF files containing:
//! - **Pyramid levels**: Full resolution image and progressively smaller versions
//! - **Label image**: Small image of the slide label
//! - **Macro image**: Overview of the entire slide
//! - **Thumbnail**: Small preview image
//!
//! # JPEGTables Handling
//!
//! SVS files use "abbreviated JPEG streams" to save space. Each tile's JPEG
//! data lacks the quantization and Huffman tables needed for decoding. These
//! tables are stored in the `JPEGTables` TIFF tag and must be merged with
//! each tile's data before decoding.
//!
//! # Metadata
//!
//! SVS files store rich metadata in the ImageDescription tag, including:
//! - Microns per pixel (MPP)
//! - Objective magnification
//! - Scanner information

use async_trait::async_trait;
use bytes::Bytes;
use std::collections::HashMap;

use crate::error::TiffError;
use crate::io::RangeReader;
use crate::slide::SlideReader;

use super::jpeg::prepare_tile_jpeg;
use super::tiff::{PyramidLevel, TiffHeader, TiffPyramid, TiffTag, TileData, ValueReader};

// =============================================================================
// SVS Metadata
// =============================================================================

/// Parsed metadata from an SVS file.
///
/// SVS files store metadata in the ImageDescription tag as a pipe-separated
/// string with key=value pairs.
#[derive(Debug, Clone, Default)]
pub struct SvsMetadata {
    /// Microns per pixel (resolution)
    pub mpp: Option<f64>,

    /// Objective magnification (e.g., 20, 40)
    pub magnification: Option<f64>,

    /// Scanner vendor name
    pub vendor: Option<String>,

    /// Full ImageDescription string
    pub image_description: Option<String>,

    /// Additional key-value pairs from ImageDescription
    pub properties: HashMap<String, String>,
}

impl SvsMetadata {
    /// Parse metadata from an ImageDescription string.
    ///
    /// SVS ImageDescription format:
    /// ```text
    /// Aperio Image Library vXX.X.X
    /// width x height (macro dimensions)|AppMag = 20|MPP = 0.5|...
    /// ```
    ///
    /// The first line identifies the format, subsequent parts are pipe-separated
    /// with key=value pairs.
    pub fn parse(description: &str) -> Self {
        let mut metadata = SvsMetadata {
            image_description: Some(description.to_string()),
            ..Default::default()
        };

        // Check for Aperio marker
        if description.contains("Aperio") {
            metadata.vendor = Some("Aperio".to_string());
        }

        // Parse pipe-separated key=value pairs
        for part in description.split('|') {
            let part = part.trim();

            // Try to parse as key=value
            if let Some(eq_pos) = part.find('=') {
                let key = part[..eq_pos].trim();
                let value = part[eq_pos + 1..].trim();

                // Store in properties
                metadata
                    .properties
                    .insert(key.to_string(), value.to_string());

                // Parse known keys
                match key {
                    "MPP" => {
                        if let Ok(mpp) = value.parse::<f64>() {
                            metadata.mpp = Some(mpp);
                        }
                    }
                    "AppMag" => {
                        if let Ok(mag) = value.parse::<f64>() {
                            metadata.magnification = Some(mag);
                        }
                    }
                    _ => {}
                }
            }
        }

        metadata
    }
}

// =============================================================================
// SVS Level Data
// =============================================================================

/// Data for a single pyramid level in an SVS file.
///
/// This includes the level metadata plus cached tile location data
/// and JPEGTables for merging with abbreviated tile streams.
#[derive(Debug, Clone)]
pub struct SvsLevelData {
    /// The pyramid level metadata
    pub level: PyramidLevel,

    /// Tile offsets and byte counts
    pub tile_data: TileData,
}

impl SvsLevelData {
    /// Get the offset and size for a specific tile.
    pub fn get_tile_location(&self, tile_x: u32, tile_y: u32) -> Option<(u64, u64)> {
        let tile_index = self.level.tile_index(tile_x, tile_y)?;
        self.tile_data.get_tile_location(tile_index)
    }

    /// Get the JPEGTables for this level.
    pub fn jpeg_tables(&self) -> Option<&Bytes> {
        self.tile_data.jpeg_tables.as_ref()
    }
}

// =============================================================================
// SVS Reader
// =============================================================================

/// Reader for Aperio SVS files.
///
/// This provides access to the image pyramid and handles the JPEGTables
/// merging required for decoding tiles.
#[derive(Debug)]
pub struct SvsReader {
    /// Parsed TIFF pyramid structure
    pyramid: TiffPyramid,

    /// Level data including tile offsets and JPEGTables
    levels: Vec<SvsLevelData>,

    /// Parsed SVS metadata
    metadata: SvsMetadata,
}

impl SvsReader {
    /// Open an SVS file and parse its structure.
    ///
    /// This reads the TIFF structure, identifies pyramid levels,
    /// loads tile offset arrays, and caches JPEGTables for each level.
    pub async fn open<R: RangeReader>(reader: &R) -> Result<Self, TiffError> {
        // Parse the TIFF pyramid structure
        let pyramid = TiffPyramid::parse(reader).await?;

        // Load tile data for each pyramid level
        let mut levels = Vec::with_capacity(pyramid.levels.len());
        for level in &pyramid.levels {
            let tile_data = TileData::load(reader, level, &pyramid.header).await?;
            levels.push(SvsLevelData {
                level: level.clone(),
                tile_data,
            });
        }

        // Parse metadata from first IFD's ImageDescription
        let metadata = Self::parse_metadata(reader, &pyramid).await?;

        Ok(SvsReader {
            pyramid,
            levels,
            metadata,
        })
    }

    /// Parse SVS metadata from the first pyramid level's ImageDescription.
    async fn parse_metadata<R: RangeReader>(
        reader: &R,
        pyramid: &TiffPyramid,
    ) -> Result<SvsMetadata, TiffError> {
        // Get the first pyramid level's IFD
        let first_level = match pyramid.levels.first() {
            Some(level) => level,
            None => return Ok(SvsMetadata::default()),
        };

        // Check for ImageDescription tag
        let entry = match first_level.ifd.get_entry_by_tag(TiffTag::ImageDescription) {
            Some(e) => e,
            None => return Ok(SvsMetadata::default()),
        };

        // Read the ImageDescription
        let value_reader = ValueReader::new(reader, &pyramid.header);
        let description = value_reader.read_string(entry).await?;

        Ok(SvsMetadata::parse(&description))
    }

    /// Get the TIFF header.
    pub fn header(&self) -> &TiffHeader {
        &self.pyramid.header
    }

    /// Get the parsed SVS metadata.
    pub fn metadata(&self) -> &SvsMetadata {
        &self.metadata
    }

    /// Get the number of pyramid levels.
    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    /// Get data for a specific pyramid level.
    pub fn get_level(&self, level: usize) -> Option<&SvsLevelData> {
        self.levels.get(level)
    }

    /// Get dimensions of the full-resolution (level 0) image.
    pub fn dimensions(&self) -> Option<(u32, u32)> {
        self.levels.first().map(|l| (l.level.width, l.level.height))
    }

    /// Get dimensions of a specific level.
    pub fn level_dimensions(&self, level: usize) -> Option<(u32, u32)> {
        self.levels.get(level).map(|l| (l.level.width, l.level.height))
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
    /// For SVS files, this is typically an abbreviated JPEG stream.
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
    /// contains an abbreviated JPEG stream (common in SVS files).
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

        // Get JPEGTables for this level
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
// SlideReader Implementation
// =============================================================================

#[async_trait]
impl SlideReader for SvsReader {
    fn level_count(&self) -> usize {
        self.levels.len()
    }

    fn dimensions(&self) -> Option<(u32, u32)> {
        self.levels.first().map(|l| (l.level.width, l.level.height))
    }

    fn level_dimensions(&self, level: usize) -> Option<(u32, u32)> {
        self.levels.get(level).map(|l| (l.level.width, l.level.height))
    }

    fn level_downsample(&self, level: usize) -> Option<f64> {
        self.levels.get(level).map(|l| l.level.downsample)
    }

    fn tile_size(&self, level: usize) -> Option<(u32, u32)> {
        self.levels
            .get(level)
            .map(|l| (l.level.tile_width, l.level.tile_height))
    }

    fn tile_count(&self, level: usize) -> Option<(u32, u32)> {
        self.levels
            .get(level)
            .map(|l| (l.level.tiles_x, l.level.tiles_y))
    }

    fn best_level_for_downsample(&self, downsample: f64) -> Option<usize> {
        SvsReader::best_level_for_downsample(self, downsample)
    }

    async fn read_tile<R: RangeReader>(
        &self,
        reader: &R,
        level: usize,
        tile_x: u32,
        tile_y: u32,
    ) -> Result<Bytes, TiffError> {
        SvsReader::read_tile(self, reader, level, tile_x, tile_y).await
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // SvsMetadata parsing tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_parse_metadata_basic() {
        let description = "Aperio Image Library v12.0.15\n46920x33600 (256x256) JPEG/RGB Q=70|AppMag = 20|MPP = 0.499";

        let metadata = SvsMetadata::parse(description);

        assert_eq!(metadata.vendor, Some("Aperio".to_string()));
        assert!((metadata.mpp.unwrap() - 0.499).abs() < 0.001);
        assert!((metadata.magnification.unwrap() - 20.0).abs() < 0.1);
    }

    #[test]
    fn test_parse_metadata_with_many_fields() {
        let description = "Aperio Image Library v12.0.15\n\
            46920x33600 (256x256) JPEG/RGB Q=70|\
            AppMag = 40|\
            StripeWidth = 2040|\
            ScanScope ID = SS1234|\
            Filename = test.svs|\
            MPP = 0.25|\
            Left = 25.5|\
            Top = 18.2";

        let metadata = SvsMetadata::parse(description);

        assert_eq!(metadata.vendor, Some("Aperio".to_string()));
        assert!((metadata.mpp.unwrap() - 0.25).abs() < 0.001);
        assert!((metadata.magnification.unwrap() - 40.0).abs() < 0.1);
        assert_eq!(metadata.properties.get("Filename"), Some(&"test.svs".to_string()));
        assert_eq!(metadata.properties.get("StripeWidth"), Some(&"2040".to_string()));
    }

    #[test]
    fn test_parse_metadata_no_mpp() {
        let description = "Aperio Image Library v12.0.15\n46920x33600|AppMag = 20";

        let metadata = SvsMetadata::parse(description);

        assert_eq!(metadata.vendor, Some("Aperio".to_string()));
        assert!(metadata.mpp.is_none());
        assert!((metadata.magnification.unwrap() - 20.0).abs() < 0.1);
    }

    #[test]
    fn test_parse_metadata_empty() {
        let metadata = SvsMetadata::parse("");

        assert!(metadata.vendor.is_none());
        assert!(metadata.mpp.is_none());
        assert!(metadata.magnification.is_none());
    }

    #[test]
    fn test_parse_metadata_non_aperio() {
        let description = "Generic TIFF image\nSome other format";

        let metadata = SvsMetadata::parse(description);

        assert!(metadata.vendor.is_none());
    }

    #[test]
    fn test_parse_metadata_invalid_mpp() {
        let description = "Aperio Image Library|MPP = invalid|AppMag = 20";

        let metadata = SvsMetadata::parse(description);

        assert!(metadata.mpp.is_none()); // Invalid value should be None
        assert!((metadata.magnification.unwrap() - 20.0).abs() < 0.1);
    }

    #[test]
    fn test_parse_metadata_whitespace() {
        let description = "Aperio Image Library | MPP = 0.5 | AppMag = 40 ";

        let metadata = SvsMetadata::parse(description);

        assert!((metadata.mpp.unwrap() - 0.5).abs() < 0.001);
        assert!((metadata.magnification.unwrap() - 40.0).abs() < 0.1);
    }
}
