//! TIFF validation for WSI support.
//!
//! This module validates that TIFF files meet the requirements for serving
//! as Whole Slide Images. Unsupported files are rejected early with clear
//! error messages.
//!
//! # Supported Subset
//!
//! The following constraints define what slides are supported:
//! - **Organization**: Tiled only (no strips)
//! - **Compression**: JPEG only (no LZW, Deflate, JPEG2000)
//! - **Format**: Standard TIFF or BigTIFF
//! - **Structure**: Must have tile offsets and byte counts tags
//!
//! Files outside this subset return appropriate errors that can be mapped
//! to HTTP 415 Unsupported Media Type.

use crate::error::TiffError;

use super::parser::{ByteOrder, Ifd};
use super::pyramid::{PyramidLevel, TiffPyramid};
use super::tags::{Compression, TiffTag};

// =============================================================================
// Validation Result
// =============================================================================

/// Result of validating a TIFF file for WSI support.
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// Whether the file is valid for WSI serving
    pub is_valid: bool,

    /// List of validation errors (empty if valid)
    pub errors: Vec<ValidationError>,

    /// List of validation warnings (non-fatal issues)
    pub warnings: Vec<String>,
}

impl ValidationResult {
    /// Create a successful validation result.
    pub fn ok() -> Self {
        ValidationResult {
            is_valid: true,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// Create a failed validation result with a single error.
    pub fn error(error: ValidationError) -> Self {
        ValidationResult {
            is_valid: false,
            errors: vec![error],
            warnings: Vec::new(),
        }
    }

    /// Add an error to the result.
    pub fn add_error(&mut self, error: ValidationError) {
        self.is_valid = false;
        self.errors.push(error);
    }

    /// Add a warning to the result.
    pub fn add_warning(&mut self, warning: String) {
        self.warnings.push(warning);
    }

    /// Convert to a TiffError if invalid.
    ///
    /// Returns the first error as a TiffError, or Ok(()) if valid.
    pub fn into_result(self) -> Result<(), TiffError> {
        if self.is_valid {
            Ok(())
        } else {
            Err(self.errors.into_iter().next().unwrap().into())
        }
    }
}

/// A specific validation error.
#[derive(Debug, Clone)]
pub enum ValidationError {
    /// File uses strip organization instead of tiles
    StripOrganization {
        /// Index of the IFD with strip organization
        ifd_index: usize,
    },

    /// Unsupported compression scheme
    UnsupportedCompression {
        /// Index of the IFD with unsupported compression
        ifd_index: usize,
        /// The compression value found
        compression: u16,
        /// Human-readable compression name
        compression_name: String,
    },

    /// Missing required tile tags
    MissingTileTags {
        /// Index of the IFD missing tile tags
        ifd_index: usize,
        /// Which tags are missing
        missing_tags: Vec<&'static str>,
    },

    /// No pyramid levels found
    NoPyramidLevels,

    /// Invalid tile dimensions
    InvalidTileDimensions {
        /// Index of the IFD with invalid dimensions
        ifd_index: usize,
        /// The tile width found
        tile_width: u32,
        /// The tile height found
        tile_height: u32,
        /// Description of the problem
        message: String,
    },
}

impl From<ValidationError> for TiffError {
    fn from(error: ValidationError) -> Self {
        match error {
            ValidationError::StripOrganization { .. } => TiffError::StripOrganization,
            ValidationError::UnsupportedCompression {
                compression_name, ..
            } => TiffError::UnsupportedCompression(compression_name),
            ValidationError::MissingTileTags { missing_tags, .. } => {
                TiffError::MissingTag(missing_tags.first().copied().unwrap_or("TileOffsets"))
            }
            ValidationError::NoPyramidLevels => {
                TiffError::MissingTag("No valid pyramid levels found")
            }
            ValidationError::InvalidTileDimensions { message, .. } => TiffError::InvalidTagValue {
                tag: "TileWidth/TileLength",
                message,
            },
        }
    }
}

// =============================================================================
// IFD Validation
// =============================================================================

/// Validate a single IFD for WSI support.
///
/// This checks that the IFD:
/// - Uses tiled organization (not strips)
/// - Uses JPEG compression
/// - Has required tile tags
///
/// Returns a ValidationResult that may contain errors or warnings.
pub fn validate_ifd(ifd: &Ifd, ifd_index: usize, byte_order: ByteOrder) -> ValidationResult {
    let mut result = ValidationResult::ok();

    // Check for strip organization (unsupported)
    if ifd.is_stripped() && !ifd.is_tiled() {
        result.add_error(ValidationError::StripOrganization { ifd_index });
        return result; // No point checking further
    }

    // If not tiled, skip validation (might be label/macro)
    if !ifd.is_tiled() {
        return result;
    }

    // Check compression
    if let Some(compression_value) = ifd.compression(byte_order) {
        if let Some(compression) = Compression::from_u16(compression_value) {
            if !compression.is_supported() {
                result.add_error(ValidationError::UnsupportedCompression {
                    ifd_index,
                    compression: compression_value,
                    compression_name: compression.name().to_string(),
                });
            }
        } else {
            // Unknown compression value
            result.add_error(ValidationError::UnsupportedCompression {
                ifd_index,
                compression: compression_value,
                compression_name: format!("Unknown ({})", compression_value),
            });
        }
    }
    // If no compression tag, assume JPEG (common default)

    // Check for required tile tags
    let mut missing_tags = Vec::new();

    if ifd.get_entry_by_tag(TiffTag::TileWidth).is_none() {
        missing_tags.push("TileWidth");
    }
    if ifd.get_entry_by_tag(TiffTag::TileLength).is_none() {
        missing_tags.push("TileLength");
    }
    if ifd.get_entry_by_tag(TiffTag::TileOffsets).is_none() {
        missing_tags.push("TileOffsets");
    }
    if ifd.get_entry_by_tag(TiffTag::TileByteCounts).is_none() {
        missing_tags.push("TileByteCounts");
    }

    if !missing_tags.is_empty() {
        result.add_error(ValidationError::MissingTileTags {
            ifd_index,
            missing_tags,
        });
    }

    // Validate tile dimensions
    if let (Some(tile_width), Some(tile_height)) =
        (ifd.tile_width(byte_order), ifd.tile_height(byte_order))
    {
        // Tile dimensions should be reasonable
        if tile_width == 0 || tile_height == 0 {
            result.add_error(ValidationError::InvalidTileDimensions {
                ifd_index,
                tile_width,
                tile_height,
                message: "Tile dimensions cannot be zero".to_string(),
            });
        } else if tile_width > 4096 || tile_height > 4096 {
            // Very large tiles are unusual and may cause memory issues
            result.add_warning(format!(
                "IFD {}: Large tile dimensions ({}x{}) may cause memory issues",
                ifd_index, tile_width, tile_height
            ));
        }

        // Tiles are typically powers of 2 or multiples of 16
        if tile_width % 16 != 0 || tile_height % 16 != 0 {
            result.add_warning(format!(
                "IFD {}: Tile dimensions ({}x{}) are not multiples of 16",
                ifd_index, tile_width, tile_height
            ));
        }
    }

    result
}

/// Validate a pyramid level for WSI support.
pub fn validate_level(level: &PyramidLevel, _byte_order: ByteOrder) -> ValidationResult {
    let mut result = ValidationResult::ok();

    // Check compression
    if let Some(compression) = Compression::from_u16(level.compression) {
        if !compression.is_supported() {
            result.add_error(ValidationError::UnsupportedCompression {
                ifd_index: level.ifd_index,
                compression: level.compression,
                compression_name: compression.name().to_string(),
            });
        }
    } else {
        result.add_error(ValidationError::UnsupportedCompression {
            ifd_index: level.ifd_index,
            compression: level.compression,
            compression_name: format!("Unknown ({})", level.compression),
        });
    }

    // Check tile data entries
    if !level.has_tile_data() {
        let mut missing = Vec::new();
        if level.tile_offsets_entry.is_none() {
            missing.push("TileOffsets");
        }
        if level.tile_byte_counts_entry.is_none() {
            missing.push("TileByteCounts");
        }
        result.add_error(ValidationError::MissingTileTags {
            ifd_index: level.ifd_index,
            missing_tags: missing,
        });
    }

    // Validate tile dimensions
    if level.tile_width == 0 || level.tile_height == 0 {
        result.add_error(ValidationError::InvalidTileDimensions {
            ifd_index: level.ifd_index,
            tile_width: level.tile_width,
            tile_height: level.tile_height,
            message: "Tile dimensions cannot be zero".to_string(),
        });
    }

    // Check for JPEGTables on JPEG-compressed levels
    if level.compression == 7 && level.jpeg_tables_entry.is_none() {
        // This is a warning, not an error - some files have inline tables
        result.add_warning(format!(
            "Level {}: No JPEGTables tag found (tiles may have inline tables)",
            level.level_index
        ));
    }

    result
}

/// Validate a complete pyramid for WSI support.
///
/// This validates that:
/// - At least one pyramid level exists
/// - All levels use supported compression
/// - All levels have required tile data
pub fn validate_pyramid(pyramid: &TiffPyramid) -> ValidationResult {
    let mut result = ValidationResult::ok();
    let byte_order = pyramid.header.byte_order;

    // Must have at least one level
    if pyramid.levels.is_empty() {
        result.add_error(ValidationError::NoPyramidLevels);
        return result;
    }

    // Validate each level
    for level in &pyramid.levels {
        let level_result = validate_level(level, byte_order);
        for error in level_result.errors {
            result.add_error(error);
        }
        for warning in level_result.warnings {
            result.add_warning(warning);
        }
    }

    result
}

// =============================================================================
// Quick validation functions
// =============================================================================

/// Check if an IFD uses supported compression.
///
/// Returns Ok(()) if compression is JPEG, or an error otherwise.
pub fn check_compression(ifd: &Ifd, byte_order: ByteOrder) -> Result<(), TiffError> {
    if let Some(compression_value) = ifd.compression(byte_order) {
        if let Some(compression) = Compression::from_u16(compression_value) {
            if compression.is_supported() {
                return Ok(());
            }
            return Err(TiffError::UnsupportedCompression(
                compression.name().to_string(),
            ));
        }
        return Err(TiffError::UnsupportedCompression(format!(
            "Unknown ({})",
            compression_value
        )));
    }
    // No compression tag - assume JPEG (common default)
    Ok(())
}

/// Check if an IFD uses tiled organization.
///
/// Returns Ok(()) if tiled, or an error if stripped.
pub fn check_tiled(ifd: &Ifd) -> Result<(), TiffError> {
    if ifd.is_stripped() && !ifd.is_tiled() {
        return Err(TiffError::StripOrganization);
    }
    Ok(())
}

/// Check if an IFD has all required tile tags.
///
/// Returns Ok(()) if all tags present, or an error with the first missing tag.
pub fn check_tile_tags(ifd: &Ifd) -> Result<(), TiffError> {
    if ifd.get_entry_by_tag(TiffTag::TileWidth).is_none() {
        return Err(TiffError::MissingTag("TileWidth"));
    }
    if ifd.get_entry_by_tag(TiffTag::TileLength).is_none() {
        return Err(TiffError::MissingTag("TileLength"));
    }
    if ifd.get_entry_by_tag(TiffTag::TileOffsets).is_none() {
        return Err(TiffError::MissingTag("TileOffsets"));
    }
    if ifd.get_entry_by_tag(TiffTag::TileByteCounts).is_none() {
        return Err(TiffError::MissingTag("TileByteCounts"));
    }
    Ok(())
}

/// Perform full validation on an IFD.
///
/// This is a convenience function that checks all requirements.
/// Returns Ok(()) if the IFD is valid for WSI serving.
pub fn validate_ifd_strict(
    ifd: &Ifd,
    ifd_index: usize,
    byte_order: ByteOrder,
) -> Result<(), TiffError> {
    let result = validate_ifd(ifd, ifd_index, byte_order);
    result.into_result()
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::tiff::parser::TiffHeader;
    use crate::format::tiff::tags::FieldType;
    use crate::format::tiff::IfdEntry;
    use std::collections::HashMap;

    fn make_header() -> TiffHeader {
        TiffHeader {
            byte_order: ByteOrder::LittleEndian,
            is_bigtiff: false,
            first_ifd_offset: 8,
        }
    }

    fn make_entry(tag: TiffTag, value: u32) -> IfdEntry {
        IfdEntry {
            tag_id: tag.as_u16(),
            field_type: Some(FieldType::Long),
            field_type_raw: 4,
            count: 1,
            value_offset_bytes: value.to_le_bytes().to_vec(),
            is_inline: true,
        }
    }

    fn make_tiled_ifd() -> Ifd {
        // Create a valid tiled IFD with JPEG compression
        let entries = vec![
            make_entry(TiffTag::ImageWidth, 10000),
            make_entry(TiffTag::ImageLength, 8000),
            make_entry(TiffTag::TileWidth, 256),
            make_entry(TiffTag::TileLength, 256),
            IfdEntry {
                tag_id: TiffTag::TileOffsets.as_u16(),
                field_type: Some(FieldType::Long),
                field_type_raw: 4,
                count: 100,
                value_offset_bytes: vec![0, 0, 0, 0],
                is_inline: false,
            },
            IfdEntry {
                tag_id: TiffTag::TileByteCounts.as_u16(),
                field_type: Some(FieldType::Long),
                field_type_raw: 4,
                count: 100,
                value_offset_bytes: vec![0, 0, 0, 0],
                is_inline: false,
            },
            IfdEntry {
                tag_id: TiffTag::Compression.as_u16(),
                field_type: Some(FieldType::Short),
                field_type_raw: 3,
                count: 1,
                value_offset_bytes: vec![7, 0, 0, 0], // JPEG = 7
                is_inline: true,
            },
        ];

        let mut entries_by_tag = HashMap::new();
        for (i, entry) in entries.iter().enumerate() {
            entries_by_tag.insert(entry.tag_id, i);
        }

        Ifd {
            entries,
            entries_by_tag,
            next_ifd_offset: 0,
        }
    }

    fn make_stripped_ifd() -> Ifd {
        // Create a strip-based IFD (unsupported)
        let entries = vec![
            make_entry(TiffTag::ImageWidth, 1000),
            make_entry(TiffTag::ImageLength, 800),
            IfdEntry {
                tag_id: TiffTag::StripOffsets.as_u16(),
                field_type: Some(FieldType::Long),
                field_type_raw: 4,
                count: 10,
                value_offset_bytes: vec![0, 0, 0, 0],
                is_inline: false,
            },
            IfdEntry {
                tag_id: TiffTag::StripByteCounts.as_u16(),
                field_type: Some(FieldType::Long),
                field_type_raw: 4,
                count: 10,
                value_offset_bytes: vec![0, 0, 0, 0],
                is_inline: false,
            },
        ];

        let mut entries_by_tag = HashMap::new();
        for (i, entry) in entries.iter().enumerate() {
            entries_by_tag.insert(entry.tag_id, i);
        }

        Ifd {
            entries,
            entries_by_tag,
            next_ifd_offset: 0,
        }
    }

    fn make_lzw_ifd() -> Ifd {
        // Create a tiled IFD with LZW compression (unsupported)
        let entries = vec![
            make_entry(TiffTag::ImageWidth, 10000),
            make_entry(TiffTag::ImageLength, 8000),
            make_entry(TiffTag::TileWidth, 256),
            make_entry(TiffTag::TileLength, 256),
            IfdEntry {
                tag_id: TiffTag::TileOffsets.as_u16(),
                field_type: Some(FieldType::Long),
                field_type_raw: 4,
                count: 100,
                value_offset_bytes: vec![0, 0, 0, 0],
                is_inline: false,
            },
            IfdEntry {
                tag_id: TiffTag::TileByteCounts.as_u16(),
                field_type: Some(FieldType::Long),
                field_type_raw: 4,
                count: 100,
                value_offset_bytes: vec![0, 0, 0, 0],
                is_inline: false,
            },
            IfdEntry {
                tag_id: TiffTag::Compression.as_u16(),
                field_type: Some(FieldType::Short),
                field_type_raw: 3,
                count: 1,
                value_offset_bytes: vec![5, 0, 0, 0], // LZW = 5
                is_inline: true,
            },
        ];

        let mut entries_by_tag = HashMap::new();
        for (i, entry) in entries.iter().enumerate() {
            entries_by_tag.insert(entry.tag_id, i);
        }

        Ifd {
            entries,
            entries_by_tag,
            next_ifd_offset: 0,
        }
    }

    // -------------------------------------------------------------------------
    // validate_ifd tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_validate_tiled_jpeg_ifd() {
        let ifd = make_tiled_ifd();
        let header = make_header();
        let result = validate_ifd(&ifd, 0, header.byte_order);

        assert!(result.is_valid);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_validate_stripped_ifd() {
        let ifd = make_stripped_ifd();
        let header = make_header();
        let result = validate_ifd(&ifd, 0, header.byte_order);

        assert!(!result.is_valid);
        assert_eq!(result.errors.len(), 1);
        assert!(matches!(
            result.errors[0],
            ValidationError::StripOrganization { ifd_index: 0 }
        ));
    }

    #[test]
    fn test_validate_lzw_ifd() {
        let ifd = make_lzw_ifd();
        let header = make_header();
        let result = validate_ifd(&ifd, 0, header.byte_order);

        assert!(!result.is_valid);
        assert!(matches!(
            result.errors[0],
            ValidationError::UnsupportedCompression { compression: 5, .. }
        ));
    }

    #[test]
    fn test_validate_missing_tile_tags() {
        // IFD with TileWidth/TileLength but missing TileOffsets/TileByteCounts
        let entries = vec![
            make_entry(TiffTag::ImageWidth, 10000),
            make_entry(TiffTag::ImageLength, 8000),
            make_entry(TiffTag::TileWidth, 256),
            make_entry(TiffTag::TileLength, 256),
            IfdEntry {
                tag_id: TiffTag::Compression.as_u16(),
                field_type: Some(FieldType::Short),
                field_type_raw: 3,
                count: 1,
                value_offset_bytes: vec![7, 0, 0, 0],
                is_inline: true,
            },
        ];

        let mut entries_by_tag = HashMap::new();
        for (i, entry) in entries.iter().enumerate() {
            entries_by_tag.insert(entry.tag_id, i);
        }

        let ifd = Ifd {
            entries,
            entries_by_tag,
            next_ifd_offset: 0,
        };

        let header = make_header();
        let result = validate_ifd(&ifd, 0, header.byte_order);

        assert!(!result.is_valid);
        assert!(matches!(
            result.errors[0],
            ValidationError::MissingTileTags { .. }
        ));
    }

    // -------------------------------------------------------------------------
    // check_* function tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_check_compression_jpeg() {
        let ifd = make_tiled_ifd();
        let header = make_header();
        assert!(check_compression(&ifd, header.byte_order).is_ok());
    }

    #[test]
    fn test_check_compression_lzw() {
        let ifd = make_lzw_ifd();
        let header = make_header();
        let result = check_compression(&ifd, header.byte_order);
        assert!(matches!(result, Err(TiffError::UnsupportedCompression(_))));
    }

    #[test]
    fn test_check_tiled_with_tiles() {
        let ifd = make_tiled_ifd();
        assert!(check_tiled(&ifd).is_ok());
    }

    #[test]
    fn test_check_tiled_with_strips() {
        let ifd = make_stripped_ifd();
        let result = check_tiled(&ifd);
        assert!(matches!(result, Err(TiffError::StripOrganization)));
    }

    #[test]
    fn test_check_tile_tags_present() {
        let ifd = make_tiled_ifd();
        assert!(check_tile_tags(&ifd).is_ok());
    }

    #[test]
    fn test_check_tile_tags_missing() {
        let ifd = make_stripped_ifd();
        let result = check_tile_tags(&ifd);
        assert!(matches!(result, Err(TiffError::MissingTag(_))));
    }

    // -------------------------------------------------------------------------
    // ValidationResult tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_validation_result_ok() {
        let result = ValidationResult::ok();
        assert!(result.is_valid);
        assert!(result.errors.is_empty());
        assert!(result.into_result().is_ok());
    }

    #[test]
    fn test_validation_result_error() {
        let result = ValidationResult::error(ValidationError::NoPyramidLevels);
        assert!(!result.is_valid);
        assert!(result.into_result().is_err());
    }

    #[test]
    fn test_validation_error_to_tiff_error() {
        let strip_error = ValidationError::StripOrganization { ifd_index: 0 };
        let tiff_error: TiffError = strip_error.into();
        assert!(matches!(tiff_error, TiffError::StripOrganization));

        let compression_error = ValidationError::UnsupportedCompression {
            ifd_index: 0,
            compression: 5,
            compression_name: "LZW".to_string(),
        };
        let tiff_error: TiffError = compression_error.into();
        assert!(matches!(tiff_error, TiffError::UnsupportedCompression(_)));
    }
}
