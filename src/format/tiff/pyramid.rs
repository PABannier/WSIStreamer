//! TIFF pyramid level identification and management.
//!
//! WSI files contain multiple IFDs (Image File Directories), but not all are
//! pyramid levels. This module identifies which IFDs belong to the image pyramid
//! and provides structured access to them.
//!
//! # Pyramid Structure
//!
//! A typical WSI file contains:
//! - **Pyramid levels**: Full resolution image and progressively smaller versions
//! - **Label image**: Small image of the slide label (often ~500x500)
//! - **Macro image**: Overview of the entire slide (medium resolution)
//! - **Thumbnail**: Very small preview image
//!
//! # Identification Heuristics
//!
//! Pyramid levels are identified by:
//! 1. Must be tiled (have TileWidth/TileLength tags)
//! 2. Dimensions decrease by consistent ratios (typically 2x or 4x)
//! 3. Largest tiled image is level 0
//!
//! Non-pyramid images are identified by:
//! - Label: Small, often square-ish, may not be tiled
//! - Macro: Medium-sized, different aspect ratio than pyramid
//! - Thumbnail: Very small, may lack tile structure

use bytes::Bytes;

use crate::error::TiffError;
use crate::io::RangeReader;

use super::parser::{ByteOrder, Ifd, IfdEntry, TiffHeader, BIGTIFF_HEADER_SIZE};
use super::tags::TiffTag;
use super::values::ValueReader;

// =============================================================================
// Constants
// =============================================================================

/// Maximum number of IFDs to parse (safety limit)
const MAX_IFDS: usize = 100;

/// Minimum dimension to be considered a pyramid level (pixels)
/// Images smaller than this are likely thumbnails
const MIN_PYRAMID_DIMENSION: u32 = 256;

/// Maximum size for a label image (pixels)
const MAX_LABEL_DIMENSION: u32 = 2000;

// =============================================================================
// PyramidLevel
// =============================================================================

/// A single level in the image pyramid.
///
/// Each level represents the image at a specific resolution. Level 0 is the
/// highest resolution (full size), with higher levels being progressively
/// smaller (lower resolution).
#[derive(Debug, Clone)]
pub struct PyramidLevel {
    /// Index of this level in the pyramid (0 = highest resolution)
    pub level_index: usize,

    /// Index of the IFD in the file's IFD chain
    pub ifd_index: usize,

    /// Image width in pixels
    pub width: u32,

    /// Image height in pixels
    pub height: u32,

    /// Tile width in pixels
    pub tile_width: u32,

    /// Tile height in pixels
    pub tile_height: u32,

    /// Number of tiles in X direction
    pub tiles_x: u32,

    /// Number of tiles in Y direction
    pub tiles_y: u32,

    /// Total number of tiles
    pub tile_count: u32,

    /// Downsample factor relative to level 0 (1.0 for level 0)
    pub downsample: f64,

    /// Compression scheme (7 = JPEG)
    pub compression: u16,

    /// The parsed IFD for this level
    pub ifd: Ifd,

    /// Offset in file where TileOffsets array is stored (if not inline)
    pub tile_offsets_entry: Option<IfdEntry>,

    /// Offset in file where TileByteCounts array is stored (if not inline)
    pub tile_byte_counts_entry: Option<IfdEntry>,

    /// JPEGTables entry for this level (if present)
    pub jpeg_tables_entry: Option<IfdEntry>,
}

impl PyramidLevel {
    /// Create a PyramidLevel from a parsed IFD.
    ///
    /// Returns None if the IFD doesn't have the required tile tags.
    fn from_ifd(ifd: Ifd, ifd_index: usize, byte_order: ByteOrder) -> Option<Self> {
        // Must have tile dimensions
        let tile_width = ifd.tile_width(byte_order)?;
        let tile_height = ifd.tile_height(byte_order)?;

        // Must have image dimensions
        let width = ifd.image_width(byte_order)?;
        let height = ifd.image_height(byte_order)?;

        // Get compression (default to JPEG if not specified)
        let compression = ifd.compression(byte_order).unwrap_or(7);

        // Calculate tile counts
        let tiles_x = (width + tile_width - 1) / tile_width;
        let tiles_y = (height + tile_height - 1) / tile_height;
        let tile_count = tiles_x * tiles_y;

        // Get entries for tile offsets and byte counts
        let tile_offsets_entry = ifd.get_entry_by_tag(TiffTag::TileOffsets).cloned();
        let tile_byte_counts_entry = ifd.get_entry_by_tag(TiffTag::TileByteCounts).cloned();

        // Get JPEGTables entry if present
        let jpeg_tables_entry = ifd.get_entry_by_tag(TiffTag::JpegTables).cloned();

        Some(PyramidLevel {
            level_index: 0, // Will be set later when sorting
            ifd_index,
            width,
            height,
            tile_width,
            tile_height,
            tiles_x,
            tiles_y,
            tile_count,
            downsample: 1.0, // Will be calculated later
            compression,
            ifd,
            tile_offsets_entry,
            tile_byte_counts_entry,
            jpeg_tables_entry,
        })
    }

    /// Check if this level has valid tile offset and byte count entries.
    pub fn has_tile_data(&self) -> bool {
        self.tile_offsets_entry.is_some() && self.tile_byte_counts_entry.is_some()
    }

    /// Get the tile index for a given tile coordinate.
    ///
    /// Returns None if the coordinates are out of bounds.
    pub fn tile_index(&self, tile_x: u32, tile_y: u32) -> Option<u32> {
        if tile_x >= self.tiles_x || tile_y >= self.tiles_y {
            return None;
        }
        Some(tile_y * self.tiles_x + tile_x)
    }

    /// Calculate pixel dimensions of a specific tile.
    ///
    /// Edge tiles may be smaller than tile_width/tile_height.
    pub fn tile_dimensions(&self, tile_x: u32, tile_y: u32) -> Option<(u32, u32)> {
        if tile_x >= self.tiles_x || tile_y >= self.tiles_y {
            return None;
        }

        let w = if tile_x == self.tiles_x - 1 {
            // Last column - may be partial
            let remainder = self.width % self.tile_width;
            if remainder == 0 {
                self.tile_width
            } else {
                remainder
            }
        } else {
            self.tile_width
        };

        let h = if tile_y == self.tiles_y - 1 {
            // Last row - may be partial
            let remainder = self.height % self.tile_height;
            if remainder == 0 {
                self.tile_height
            } else {
                remainder
            }
        } else {
            self.tile_height
        };

        Some((w, h))
    }
}

// =============================================================================
// TiffPyramid
// =============================================================================

/// A parsed TIFF image pyramid.
///
/// Contains all pyramid levels identified from the TIFF file's IFDs,
/// sorted by resolution (level 0 = highest resolution).
#[derive(Debug, Clone)]
pub struct TiffPyramid {
    /// The TIFF header
    pub header: TiffHeader,

    /// Pyramid levels, sorted by resolution (0 = highest)
    pub levels: Vec<PyramidLevel>,

    /// IFDs that were identified as non-pyramid images (label, macro, etc.)
    pub other_ifds: Vec<(usize, Ifd)>,
}

impl TiffPyramid {
    /// Parse a TIFF file and identify pyramid levels.
    ///
    /// This reads all IFDs from the file, identifies which ones belong to the
    /// image pyramid, and sorts them by resolution.
    pub async fn parse<R: RangeReader>(reader: &R) -> Result<Self, TiffError> {
        // Read and parse header
        let header_bytes = reader.read_exact_at(0, BIGTIFF_HEADER_SIZE).await?;
        let header = TiffHeader::parse(&header_bytes, reader.size())?;

        // Parse all IFDs
        let ifds = Self::parse_all_ifds(reader, &header).await?;

        // Identify pyramid levels
        Self::build_pyramid(header, ifds)
    }

    /// Parse all IFDs in the file following the next-IFD chain.
    async fn parse_all_ifds<R: RangeReader>(
        reader: &R,
        header: &TiffHeader,
    ) -> Result<Vec<Ifd>, TiffError> {
        let mut ifds = Vec::new();
        let mut offset = header.first_ifd_offset;

        while offset != 0 && ifds.len() < MAX_IFDS {
            // First, read just enough to get the entry count
            let count_size = header.ifd_count_size();
            let count_bytes = reader.read_exact_at(offset, count_size).await?;

            let entry_count = if header.is_bigtiff {
                header.byte_order.read_u64(&count_bytes)
            } else {
                header.byte_order.read_u16(&count_bytes) as u64
            };

            // Now read the full IFD
            let ifd_size = Ifd::calculate_size(entry_count, header);
            let ifd_bytes = reader.read_exact_at(offset, ifd_size).await?;
            let ifd = Ifd::parse(&ifd_bytes, header)?;

            let next_offset = ifd.next_ifd_offset;
            ifds.push(ifd);

            offset = next_offset;
        }

        Ok(ifds)
    }

    /// Build the pyramid structure from parsed IFDs.
    fn build_pyramid(header: TiffHeader, ifds: Vec<Ifd>) -> Result<Self, TiffError> {
        let byte_order = header.byte_order;

        let mut pyramid_candidates: Vec<PyramidLevel> = Vec::new();
        let mut other_ifds: Vec<(usize, Ifd)> = Vec::new();

        for (ifd_index, ifd) in ifds.into_iter().enumerate() {
            // Try to create a pyramid level from this IFD
            if let Some(level) = PyramidLevel::from_ifd(ifd.clone(), ifd_index, byte_order) {
                // Check if this looks like a pyramid level
                if Self::is_pyramid_candidate(&level) {
                    pyramid_candidates.push(level);
                } else {
                    other_ifds.push((ifd_index, ifd));
                }
            } else {
                // IFD doesn't have tile structure
                other_ifds.push((ifd_index, ifd));
            }
        }

        // Sort candidates by area (largest first = level 0)
        pyramid_candidates.sort_by(|a, b| {
            let area_a = (a.width as u64) * (a.height as u64);
            let area_b = (b.width as u64) * (b.height as u64);
            area_b.cmp(&area_a)
        });

        // Filter to keep only levels that form a consistent pyramid
        let levels = Self::filter_pyramid_levels(pyramid_candidates);

        Ok(TiffPyramid {
            header,
            levels,
            other_ifds,
        })
    }

    /// Check if a level looks like a pyramid candidate (vs label/macro).
    fn is_pyramid_candidate(level: &PyramidLevel) -> bool {
        // Must have minimum dimensions
        if level.width < MIN_PYRAMID_DIMENSION || level.height < MIN_PYRAMID_DIMENSION {
            return false;
        }

        // Must have tile data
        if !level.has_tile_data() {
            return false;
        }

        // Exclude likely label images (small and square-ish)
        if level.width <= MAX_LABEL_DIMENSION && level.height <= MAX_LABEL_DIMENSION {
            let aspect_ratio = level.width as f64 / level.height as f64;
            // Labels are often square or nearly square
            if aspect_ratio > 0.5 && aspect_ratio < 2.0 {
                // This might be a label, but only exclude if it's small
                if level.width <= 1000 && level.height <= 1000 {
                    return false;
                }
            }
        }

        true
    }

    /// Filter candidates to keep only levels that form a consistent pyramid.
    fn filter_pyramid_levels(candidates: Vec<PyramidLevel>) -> Vec<PyramidLevel> {
        if candidates.is_empty() {
            return candidates;
        }

        // The largest image is always level 0
        let base_width = candidates[0].width as f64;
        let base_height = candidates[0].height as f64;

        let mut levels = Vec::new();

        for (idx, mut level) in candidates.into_iter().enumerate() {
            // Calculate downsample factor
            let downsample_x = base_width / level.width as f64;
            let downsample_y = base_height / level.height as f64;

            // Use average downsample (they should be close)
            let downsample = (downsample_x + downsample_y) / 2.0;

            // Check if this level has a reasonable downsample factor
            // Pyramid levels typically have power-of-2 or power-of-4 downsamples
            if Self::is_valid_downsample(downsample, idx) {
                level.level_index = levels.len();
                level.downsample = downsample;
                levels.push(level);
            }
        }

        levels
    }

    /// Check if a downsample factor is valid for pyramid level.
    fn is_valid_downsample(downsample: f64, level_idx: usize) -> bool {
        if level_idx == 0 {
            // First level should have downsample ~1.0
            return (downsample - 1.0).abs() < 0.1;
        }

        // For other levels, check if it's close to a power of 2
        // Allow some tolerance for rounding
        let log2 = downsample.log2();
        let rounded = log2.round();

        // Must be at least 2x downsample for level 1+
        if rounded < 1.0 {
            return false;
        }

        // Check if close to a power of 2
        let expected = 2.0_f64.powf(rounded);
        let ratio = downsample / expected;

        // Allow 20% tolerance
        ratio > 0.8 && ratio < 1.2
    }

    /// Get the number of pyramid levels.
    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    /// Get a pyramid level by index.
    pub fn get_level(&self, level: usize) -> Option<&PyramidLevel> {
        self.levels.get(level)
    }

    /// Get the base (highest resolution) level.
    pub fn base_level(&self) -> Option<&PyramidLevel> {
        self.levels.first()
    }

    /// Get dimensions of the base level.
    pub fn dimensions(&self) -> Option<(u32, u32)> {
        self.base_level().map(|l| (l.width, l.height))
    }

    /// Find the best level for a given downsample factor.
    ///
    /// Returns the level with the smallest downsample that is >= the requested factor.
    pub fn best_level_for_downsample(&self, downsample: f64) -> Option<&PyramidLevel> {
        // Find the level with smallest downsample >= requested
        self.levels
            .iter()
            .filter(|l| l.downsample >= downsample * 0.99) // Small tolerance
            .min_by(|a, b| a.downsample.partial_cmp(&b.downsample).unwrap())
            .or_else(|| self.levels.last()) // Fall back to lowest resolution
    }
}

// =============================================================================
// Tile Data Loading
// =============================================================================

/// Loaded tile data for a pyramid level.
#[derive(Debug, Clone)]
pub struct TileData {
    /// Byte offset of each tile in the file
    pub offsets: Vec<u64>,

    /// Byte count (size) of each tile
    pub byte_counts: Vec<u64>,

    /// JPEGTables data (if present)
    pub jpeg_tables: Option<Bytes>,
}

impl TileData {
    /// Load tile data for a pyramid level.
    pub async fn load<R: RangeReader>(
        reader: &R,
        level: &PyramidLevel,
        header: &TiffHeader,
    ) -> Result<Self, TiffError> {
        let value_reader = ValueReader::new(reader, header);

        // Load tile offsets
        let offsets = if let Some(ref entry) = level.tile_offsets_entry {
            value_reader.read_u64_array(entry).await?
        } else {
            return Err(TiffError::MissingTag("TileOffsets"));
        };

        // Load tile byte counts
        let byte_counts = if let Some(ref entry) = level.tile_byte_counts_entry {
            value_reader.read_u64_array(entry).await?
        } else {
            return Err(TiffError::MissingTag("TileByteCounts"));
        };

        // Load JPEGTables if present
        let jpeg_tables = if let Some(ref entry) = level.jpeg_tables_entry {
            Some(value_reader.read_raw_bytes(entry).await?)
        } else {
            None
        };

        Ok(TileData {
            offsets,
            byte_counts,
            jpeg_tables,
        })
    }

    /// Get offset and size for a specific tile.
    pub fn get_tile_location(&self, tile_index: u32) -> Option<(u64, u64)> {
        let idx = tile_index as usize;
        if idx >= self.offsets.len() || idx >= self.byte_counts.len() {
            return None;
        }
        Some((self.offsets[idx], self.byte_counts[idx]))
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tiff_header() -> TiffHeader {
        TiffHeader {
            byte_order: ByteOrder::LittleEndian,
            is_bigtiff: false,
            first_ifd_offset: 8,
        }
    }

    // -------------------------------------------------------------------------
    // PyramidLevel tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_tile_index() {
        let level = PyramidLevel {
            level_index: 0,
            ifd_index: 0,
            width: 1024,
            height: 768,
            tile_width: 256,
            tile_height: 256,
            tiles_x: 4,
            tiles_y: 3,
            tile_count: 12,
            downsample: 1.0,
            compression: 7,
            ifd: create_mock_ifd(),
            tile_offsets_entry: None,
            tile_byte_counts_entry: None,
            jpeg_tables_entry: None,
        };

        // Valid indices
        assert_eq!(level.tile_index(0, 0), Some(0));
        assert_eq!(level.tile_index(1, 0), Some(1));
        assert_eq!(level.tile_index(0, 1), Some(4));
        assert_eq!(level.tile_index(3, 2), Some(11));

        // Out of bounds
        assert_eq!(level.tile_index(4, 0), None);
        assert_eq!(level.tile_index(0, 3), None);
    }

    #[test]
    fn test_tile_dimensions() {
        let level = PyramidLevel {
            level_index: 0,
            ifd_index: 0,
            width: 1000, // Not evenly divisible by 256
            height: 700,
            tile_width: 256,
            tile_height: 256,
            tiles_x: 4, // ceil(1000/256)
            tiles_y: 3, // ceil(700/256)
            tile_count: 12,
            downsample: 1.0,
            compression: 7,
            ifd: create_mock_ifd(),
            tile_offsets_entry: None,
            tile_byte_counts_entry: None,
            jpeg_tables_entry: None,
        };

        // Full tiles
        assert_eq!(level.tile_dimensions(0, 0), Some((256, 256)));
        assert_eq!(level.tile_dimensions(1, 1), Some((256, 256)));

        // Partial tile on right edge (1000 % 256 = 232)
        assert_eq!(level.tile_dimensions(3, 0), Some((232, 256)));

        // Partial tile on bottom edge (700 % 256 = 188)
        assert_eq!(level.tile_dimensions(0, 2), Some((256, 188)));

        // Corner partial tile
        assert_eq!(level.tile_dimensions(3, 2), Some((232, 188)));

        // Out of bounds
        assert_eq!(level.tile_dimensions(4, 0), None);
    }

    #[test]
    fn test_is_valid_downsample() {
        // Level 0 should be ~1.0
        assert!(TiffPyramid::is_valid_downsample(1.0, 0));
        assert!(TiffPyramid::is_valid_downsample(1.05, 0));
        assert!(!TiffPyramid::is_valid_downsample(2.0, 0));

        // Level 1+ should be powers of 2
        assert!(TiffPyramid::is_valid_downsample(2.0, 1));
        assert!(TiffPyramid::is_valid_downsample(4.0, 2));
        assert!(TiffPyramid::is_valid_downsample(8.0, 3));
        assert!(TiffPyramid::is_valid_downsample(16.0, 4));

        // Allow some tolerance
        assert!(TiffPyramid::is_valid_downsample(2.1, 1));
        assert!(TiffPyramid::is_valid_downsample(3.9, 2));

        // Reject values too far off
        assert!(!TiffPyramid::is_valid_downsample(1.5, 1)); // Not close to 2
        assert!(!TiffPyramid::is_valid_downsample(3.0, 2)); // Not close to 4
    }

    #[test]
    fn test_is_pyramid_candidate() {
        // Large enough, has tile data
        let good_level = PyramidLevel {
            level_index: 0,
            ifd_index: 0,
            width: 10000,
            height: 8000,
            tile_width: 256,
            tile_height: 256,
            tiles_x: 40,
            tiles_y: 32,
            tile_count: 1280,
            downsample: 1.0,
            compression: 7,
            ifd: create_mock_ifd(),
            tile_offsets_entry: Some(create_mock_entry()),
            tile_byte_counts_entry: Some(create_mock_entry()),
            jpeg_tables_entry: None,
        };
        assert!(TiffPyramid::is_pyramid_candidate(&good_level));

        // Too small
        let small_level = PyramidLevel {
            width: 100,
            height: 100,
            ..good_level.clone()
        };
        assert!(!TiffPyramid::is_pyramid_candidate(&small_level));

        // No tile data
        let no_tiles = PyramidLevel {
            tile_offsets_entry: None,
            ..good_level.clone()
        };
        assert!(!TiffPyramid::is_pyramid_candidate(&no_tiles));

        // Label-like (small and square)
        let label_like = PyramidLevel {
            width: 500,
            height: 500,
            tiles_x: 2,
            tiles_y: 2,
            tile_count: 4,
            ..good_level.clone()
        };
        assert!(!TiffPyramid::is_pyramid_candidate(&label_like));
    }

    #[test]
    fn test_best_level_for_downsample() {
        let header = make_tiff_header();
        let pyramid = TiffPyramid {
            header,
            levels: vec![
                create_level_with_downsample(0, 1.0, 10000, 8000),
                create_level_with_downsample(1, 4.0, 2500, 2000),
                create_level_with_downsample(2, 16.0, 625, 500),
            ],
            other_ifds: vec![],
        };

        // Exact matches
        assert_eq!(
            pyramid.best_level_for_downsample(1.0).unwrap().level_index,
            0
        );
        assert_eq!(
            pyramid.best_level_for_downsample(4.0).unwrap().level_index,
            1
        );
        assert_eq!(
            pyramid.best_level_for_downsample(16.0).unwrap().level_index,
            2
        );

        // In between - should use next higher resolution
        assert_eq!(
            pyramid.best_level_for_downsample(2.0).unwrap().level_index,
            1
        );
        assert_eq!(
            pyramid.best_level_for_downsample(8.0).unwrap().level_index,
            2
        );

        // Below minimum - use highest resolution
        assert_eq!(
            pyramid.best_level_for_downsample(0.5).unwrap().level_index,
            0
        );

        // Above maximum - use lowest resolution
        assert_eq!(
            pyramid.best_level_for_downsample(32.0).unwrap().level_index,
            2
        );
    }

    // -------------------------------------------------------------------------
    // Helper functions for tests
    // -------------------------------------------------------------------------

    fn create_mock_ifd() -> Ifd {
        Ifd::empty()
    }

    fn create_mock_entry() -> IfdEntry {
        IfdEntry {
            tag_id: 324,
            field_type: Some(super::super::tags::FieldType::Long),
            field_type_raw: 4,
            count: 1,
            value_offset_bytes: vec![0, 0, 0, 0],
            is_inline: true,
        }
    }

    fn create_level_with_downsample(
        level_index: usize,
        downsample: f64,
        width: u32,
        height: u32,
    ) -> PyramidLevel {
        PyramidLevel {
            level_index,
            ifd_index: level_index,
            width,
            height,
            tile_width: 256,
            tile_height: 256,
            tiles_x: (width + 255) / 256,
            tiles_y: (height + 255) / 256,
            tile_count: ((width + 255) / 256) * ((height + 255) / 256),
            downsample,
            compression: 7,
            ifd: create_mock_ifd(),
            tile_offsets_entry: Some(create_mock_entry()),
            tile_byte_counts_entry: Some(create_mock_entry()),
            jpeg_tables_entry: None,
        }
    }
}
