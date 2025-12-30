//! SlideReader trait for format-agnostic slide access.
//!
//! This module defines the `SlideReader` trait, which provides a unified interface
//! for reading tiles from Whole Slide Images regardless of their underlying format.
//!
//! # Usage
//!
//! The trait is implemented by format-specific readers:
//! - [`crate::format::SvsReader`] for Aperio SVS files
//! - [`crate::format::GenericTiffReader`] for standard pyramidal TIFF files
//!
//! This allows the tile service layer to work with any supported format without
//! format-specific logic.

use async_trait::async_trait;
use bytes::Bytes;

use crate::error::TiffError;
use crate::io::RangeReader;

// =============================================================================
// Level Information
// =============================================================================

/// Information about a single pyramid level.
///
/// This struct provides a snapshot of level metadata that can be queried
/// without async operations.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LevelInfo {
    /// Width of this level in pixels
    pub width: u32,

    /// Height of this level in pixels
    pub height: u32,

    /// Width of each tile in pixels
    pub tile_width: u32,

    /// Height of each tile in pixels
    pub tile_height: u32,

    /// Number of tiles in X direction
    pub tiles_x: u32,

    /// Number of tiles in Y direction
    pub tiles_y: u32,

    /// Downsample factor relative to level 0
    ///
    /// Level 0 has downsample 1.0, level 1 might have 2.0, etc.
    pub downsample: f64,
}

// =============================================================================
// SlideReader Trait
// =============================================================================

/// Format-agnostic interface for reading tiles from Whole Slide Images.
///
/// This trait provides a unified API for accessing slide metadata and reading
/// tiles, abstracting over the underlying file format. Implementations handle
/// format-specific details like JPEGTables merging for SVS files.
///
/// # Type Parameters
///
/// The trait is generic over the `RangeReader` used for I/O, allowing the same
/// slide reader to work with different storage backends (S3, local files, etc.).
///
/// # Example
///
/// ```ignore
/// use wsi_streamer::slide::SlideReader;
///
/// async fn read_tile<R: RangeReader, S: SlideReader>(
///     reader: &R,
///     slide: &S,
///     level: usize,
///     x: u32,
///     y: u32,
/// ) -> Result<Bytes, TiffError> {
///     // Check bounds
///     let info = slide.level_info(level).ok_or_else(|| /* error */)?;
///     if x >= info.tiles_x || y >= info.tiles_y {
///         return Err(/* error */);
///     }
///
///     // Read tile
///     slide.read_tile(reader, level, x, y).await
/// }
/// ```
#[async_trait]
pub trait SlideReader: Send + Sync {
    /// Get the number of pyramid levels.
    ///
    /// Level 0 is always the highest resolution (full size).
    /// Higher levels have progressively lower resolution.
    fn level_count(&self) -> usize;

    /// Get dimensions of the full-resolution (level 0) image.
    ///
    /// Returns `(width, height)` in pixels, or `None` if no levels exist.
    fn dimensions(&self) -> Option<(u32, u32)>;

    /// Get dimensions of a specific level.
    ///
    /// Returns `(width, height)` in pixels, or `None` if level is out of range.
    fn level_dimensions(&self, level: usize) -> Option<(u32, u32)>;

    /// Get the downsample factor for a level.
    ///
    /// Level 0 always has downsample 1.0. Higher levels have larger values
    /// (e.g., 2.0 means half the resolution in each dimension).
    ///
    /// Returns `None` if level is out of range.
    fn level_downsample(&self, level: usize) -> Option<f64>;

    /// Get tile size for a level.
    ///
    /// Returns `(tile_width, tile_height)` in pixels, or `None` if level is out of range.
    ///
    /// Note: Edge tiles may be smaller than this size.
    fn tile_size(&self, level: usize) -> Option<(u32, u32)>;

    /// Get the number of tiles in X and Y directions for a level.
    ///
    /// Returns `(tiles_x, tiles_y)`, or `None` if level is out of range.
    fn tile_count(&self, level: usize) -> Option<(u32, u32)>;

    /// Get complete information about a level.
    ///
    /// Returns `None` if level is out of range.
    fn level_info(&self, level: usize) -> Option<LevelInfo> {
        let (width, height) = self.level_dimensions(level)?;
        let (tile_width, tile_height) = self.tile_size(level)?;
        let (tiles_x, tiles_y) = self.tile_count(level)?;
        let downsample = self.level_downsample(level)?;

        Some(LevelInfo {
            width,
            height,
            tile_width,
            tile_height,
            tiles_x,
            tiles_y,
            downsample,
        })
    }

    /// Find the best level for a given downsample factor.
    ///
    /// Returns the index of the level with the smallest downsample that is
    /// greater than or equal to the requested factor.
    ///
    /// This is useful for selecting an appropriate resolution level when
    /// the viewer requests a specific zoom level.
    ///
    /// Returns `None` if no suitable level exists.
    fn best_level_for_downsample(&self, downsample: f64) -> Option<usize>;

    /// Read a tile and prepare it for JPEG decoding.
    ///
    /// This reads the tile data from storage and performs any necessary
    /// processing (e.g., JPEGTables merging for SVS files) to produce
    /// a complete JPEG stream that can be decoded by standard libraries.
    ///
    /// # Arguments
    ///
    /// * `reader` - The range reader for accessing the file data
    /// * `level` - Pyramid level index (0 = highest resolution)
    /// * `tile_x` - Tile X coordinate (0-indexed from left)
    /// * `tile_y` - Tile Y coordinate (0-indexed from top)
    ///
    /// # Returns
    ///
    /// Complete JPEG data ready for decoding.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Level is out of range
    /// - Tile coordinates are out of range
    /// - I/O error occurs during read
    async fn read_tile<R: RangeReader>(
        &self,
        reader: &R,
        level: usize,
        tile_x: u32,
        tile_y: u32,
    ) -> Result<Bytes, TiffError>;
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_level_info_equality() {
        let info1 = LevelInfo {
            width: 1000,
            height: 800,
            tile_width: 256,
            tile_height: 256,
            tiles_x: 4,
            tiles_y: 4,
            downsample: 1.0,
        };

        let info2 = LevelInfo {
            width: 1000,
            height: 800,
            tile_width: 256,
            tile_height: 256,
            tiles_x: 4,
            tiles_y: 4,
            downsample: 1.0,
        };

        assert_eq!(info1, info2);
    }

    #[test]
    fn test_level_info_clone() {
        let info = LevelInfo {
            width: 1000,
            height: 800,
            tile_width: 256,
            tile_height: 256,
            tiles_x: 4,
            tiles_y: 4,
            downsample: 2.0,
        };

        let cloned = info;
        assert_eq!(info.width, cloned.width);
        assert_eq!(info.downsample, cloned.downsample);
    }
}
