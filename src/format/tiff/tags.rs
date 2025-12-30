//! TIFF tag and field type definitions.
//!
//! This module defines the vocabulary for TIFF parsing, including:
//! - Field types that determine how values are encoded
//! - Tag IDs that identify metadata fields
//!
//! The definitions support both classic TIFF and BigTIFF formats.

// =============================================================================
// TIFF Field Types
// =============================================================================

/// TIFF field types that determine how values are encoded.
///
/// Each field type has a specific size in bytes, which is critical for:
/// - Determining if a value fits inline in an IFD entry
/// - Reading arrays of values correctly
///
/// Note: We only define types actually used in WSI files. TIFF supports
/// additional types (RATIONAL, FLOAT, etc.) that are not needed here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum FieldType {
    /// Unsigned 8-bit integer (1 byte)
    Byte = 1,

    /// 8-bit ASCII character (1 byte)
    Ascii = 2,

    /// Unsigned 16-bit integer (2 bytes)
    Short = 3,

    /// Unsigned 32-bit integer (4 bytes)
    Long = 4,

    /// Unsigned 64-bit integer (8 bytes) - BigTIFF only
    Long8 = 16,

    /// Undefined byte data (1 byte per element)
    Undefined = 7,
}

impl FieldType {
    /// Size of a single value of this type in bytes.
    ///
    /// This is essential for:
    /// - Calculating total array sizes
    /// - Determining inline vs offset storage
    #[inline]
    pub const fn size_in_bytes(self) -> usize {
        match self {
            FieldType::Byte => 1,
            FieldType::Ascii => 1,
            FieldType::Short => 2,
            FieldType::Long => 4,
            FieldType::Long8 => 8,
            FieldType::Undefined => 1,
        }
    }

    /// Create a FieldType from its numeric value.
    ///
    /// Returns `None` for unsupported or unknown type values.
    pub fn from_u16(value: u16) -> Option<Self> {
        match value {
            1 => Some(FieldType::Byte),
            2 => Some(FieldType::Ascii),
            3 => Some(FieldType::Short),
            4 => Some(FieldType::Long),
            7 => Some(FieldType::Undefined),
            16 => Some(FieldType::Long8),
            _ => None,
        }
    }

    /// Maximum bytes that can be stored inline in a classic TIFF IFD entry.
    ///
    /// In classic TIFF, the value/offset field is 4 bytes.
    pub const INLINE_THRESHOLD_TIFF: usize = 4;

    /// Maximum bytes that can be stored inline in a BigTIFF IFD entry.
    ///
    /// In BigTIFF, the value/offset field is 8 bytes.
    pub const INLINE_THRESHOLD_BIGTIFF: usize = 8;

    /// Check if a value with this type and count fits inline in a TIFF entry.
    ///
    /// # Arguments
    /// * `count` - Number of values
    /// * `is_bigtiff` - Whether this is a BigTIFF file
    ///
    /// # Returns
    /// `true` if the total value size fits in the inline value field.
    #[inline]
    pub fn fits_inline(self, count: u64, is_bigtiff: bool) -> bool {
        let total_size = self.size_in_bytes() as u64 * count;
        let threshold = if is_bigtiff {
            Self::INLINE_THRESHOLD_BIGTIFF as u64
        } else {
            Self::INLINE_THRESHOLD_TIFF as u64
        };
        total_size <= threshold
    }
}

// =============================================================================
// TIFF Tags
// =============================================================================

/// TIFF tag IDs relevant to WSI parsing.
///
/// Tags are 16-bit identifiers that describe the type of metadata in an IFD entry.
/// We define only the tags needed for:
/// - Basic image structure (dimensions, organization)
/// - Tile access (offsets, byte counts, sizes)
/// - Compression and JPEG handling
/// - Format-specific metadata (SVS ImageDescription)
///
/// Tags not listed here are ignored during parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum TiffTag {
    // -------------------------------------------------------------------------
    // Basic Image Structure
    // -------------------------------------------------------------------------
    /// Image width in pixels
    ImageWidth = 256,

    /// Image height (length) in pixels
    ImageLength = 257,

    /// Bits per sample (typically 8 for JPEG)
    BitsPerSample = 258,

    /// Compression scheme used
    Compression = 259,

    /// Photometric interpretation (RGB, YCbCr, etc.)
    PhotometricInterpretation = 262,

    /// Description string (contains metadata in SVS files)
    ImageDescription = 270,

    /// Number of components per pixel (e.g., 3 for RGB)
    SamplesPerPixel = 277,

    /// How components are organized (chunky vs planar)
    PlanarConfiguration = 284,

    // -------------------------------------------------------------------------
    // Strip Organization (used to detect unsupported files)
    // -------------------------------------------------------------------------
    /// Row count per strip (indicates strip organization)
    RowsPerStrip = 278,

    /// Byte offsets of strips (indicates strip organization)
    StripOffsets = 273,

    /// Byte counts of strips (indicates strip organization)
    StripByteCounts = 279,

    // -------------------------------------------------------------------------
    // Tile Organization (required for WSI support)
    // -------------------------------------------------------------------------
    /// Width of each tile in pixels
    TileWidth = 322,

    /// Height (length) of each tile in pixels
    TileLength = 323,

    /// Byte offsets of each tile in the file
    TileOffsets = 324,

    /// Byte counts of each tile
    TileByteCounts = 325,

    // -------------------------------------------------------------------------
    // JPEG Handling
    // -------------------------------------------------------------------------
    /// JPEG quantization and Huffman tables for abbreviated streams
    ///
    /// This is critical for SVS files which store incomplete JPEG streams.
    /// The tables from this tag must be merged with tile data before decoding.
    JpegTables = 347,

    /// YCbCr subsampling factors
    YCbCrSubSampling = 530,

    // -------------------------------------------------------------------------
    // Resolution (optional metadata)
    // -------------------------------------------------------------------------
    /// Pixels per unit in X direction
    XResolution = 282,

    /// Pixels per unit in Y direction
    YResolution = 283,

    /// Unit of resolution (1=none, 2=inch, 3=centimeter)
    ResolutionUnit = 296,
}

impl TiffTag {
    /// Create a TiffTag from its numeric value.
    ///
    /// Returns `None` for unrecognized tags. Unknown tags are not an error;
    /// they are simply ignored during parsing.
    pub fn from_u16(value: u16) -> Option<Self> {
        match value {
            256 => Some(TiffTag::ImageWidth),
            257 => Some(TiffTag::ImageLength),
            258 => Some(TiffTag::BitsPerSample),
            259 => Some(TiffTag::Compression),
            262 => Some(TiffTag::PhotometricInterpretation),
            270 => Some(TiffTag::ImageDescription),
            273 => Some(TiffTag::StripOffsets),
            277 => Some(TiffTag::SamplesPerPixel),
            278 => Some(TiffTag::RowsPerStrip),
            279 => Some(TiffTag::StripByteCounts),
            282 => Some(TiffTag::XResolution),
            283 => Some(TiffTag::YResolution),
            284 => Some(TiffTag::PlanarConfiguration),
            296 => Some(TiffTag::ResolutionUnit),
            322 => Some(TiffTag::TileWidth),
            323 => Some(TiffTag::TileLength),
            324 => Some(TiffTag::TileOffsets),
            325 => Some(TiffTag::TileByteCounts),
            347 => Some(TiffTag::JpegTables),
            530 => Some(TiffTag::YCbCrSubSampling),
            _ => None,
        }
    }

    /// Get the numeric tag ID.
    #[inline]
    pub const fn as_u16(self) -> u16 {
        self as u16
    }
}

// =============================================================================
// Compression Values
// =============================================================================

/// TIFF compression scheme identifiers.
///
/// We only support JPEG compression (value 7). Other compression schemes
/// will result in HTTP 415 Unsupported Media Type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum Compression {
    /// No compression
    None = 1,

    /// LZW compression (not supported)
    Lzw = 5,

    /// "Old-style" JPEG (not supported, rarely used)
    OldJpeg = 6,

    /// JPEG compression (supported)
    Jpeg = 7,

    /// Deflate/zlib compression (not supported)
    Deflate = 8,

    /// Adobe Deflate (not supported)
    AdobeDeflate = 32946,

    /// JPEG 2000 (not supported)
    Jpeg2000 = 33003,
}

impl Compression {
    /// Create a Compression from its numeric value.
    ///
    /// Returns `None` for unrecognized compression values.
    pub fn from_u16(value: u16) -> Option<Self> {
        match value {
            1 => Some(Compression::None),
            5 => Some(Compression::Lzw),
            6 => Some(Compression::OldJpeg),
            7 => Some(Compression::Jpeg),
            8 => Some(Compression::Deflate),
            32946 => Some(Compression::AdobeDeflate),
            33003 => Some(Compression::Jpeg2000),
            _ => None,
        }
    }

    /// Check if this compression scheme is supported.
    #[inline]
    pub const fn is_supported(self) -> bool {
        matches!(self, Compression::Jpeg)
    }

    /// Get a human-readable name for the compression scheme.
    pub const fn name(self) -> &'static str {
        match self {
            Compression::None => "None",
            Compression::Lzw => "LZW",
            Compression::OldJpeg => "Old JPEG",
            Compression::Jpeg => "JPEG",
            Compression::Deflate => "Deflate",
            Compression::AdobeDeflate => "Adobe Deflate",
            Compression::Jpeg2000 => "JPEG 2000",
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // FieldType Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_field_type_sizes() {
        assert_eq!(FieldType::Byte.size_in_bytes(), 1);
        assert_eq!(FieldType::Ascii.size_in_bytes(), 1);
        assert_eq!(FieldType::Short.size_in_bytes(), 2);
        assert_eq!(FieldType::Long.size_in_bytes(), 4);
        assert_eq!(FieldType::Long8.size_in_bytes(), 8);
        assert_eq!(FieldType::Undefined.size_in_bytes(), 1);
    }

    #[test]
    fn test_field_type_from_u16() {
        assert_eq!(FieldType::from_u16(1), Some(FieldType::Byte));
        assert_eq!(FieldType::from_u16(2), Some(FieldType::Ascii));
        assert_eq!(FieldType::from_u16(3), Some(FieldType::Short));
        assert_eq!(FieldType::from_u16(4), Some(FieldType::Long));
        assert_eq!(FieldType::from_u16(7), Some(FieldType::Undefined));
        assert_eq!(FieldType::from_u16(16), Some(FieldType::Long8));
        // Unknown types
        assert_eq!(FieldType::from_u16(0), None);
        assert_eq!(FieldType::from_u16(99), None);
    }

    #[test]
    fn test_fits_inline_tiff() {
        // Classic TIFF: 4 bytes inline
        // 4 bytes fit
        assert!(FieldType::Byte.fits_inline(4, false));
        assert!(FieldType::Short.fits_inline(2, false));
        assert!(FieldType::Long.fits_inline(1, false));

        // 5+ bytes don't fit
        assert!(!FieldType::Byte.fits_inline(5, false));
        assert!(!FieldType::Short.fits_inline(3, false));
        assert!(!FieldType::Long.fits_inline(2, false));

        // Long8 never fits in classic TIFF
        assert!(!FieldType::Long8.fits_inline(1, false));
    }

    #[test]
    fn test_fits_inline_bigtiff() {
        // BigTIFF: 8 bytes inline
        // 8 bytes fit
        assert!(FieldType::Byte.fits_inline(8, true));
        assert!(FieldType::Short.fits_inline(4, true));
        assert!(FieldType::Long.fits_inline(2, true));
        assert!(FieldType::Long8.fits_inline(1, true));

        // 9+ bytes don't fit
        assert!(!FieldType::Byte.fits_inline(9, true));
        assert!(!FieldType::Short.fits_inline(5, true));
        assert!(!FieldType::Long.fits_inline(3, true));
        assert!(!FieldType::Long8.fits_inline(2, true));
    }

    // -------------------------------------------------------------------------
    // TiffTag Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_tiff_tag_from_u16() {
        // Basic structure tags
        assert_eq!(TiffTag::from_u16(256), Some(TiffTag::ImageWidth));
        assert_eq!(TiffTag::from_u16(257), Some(TiffTag::ImageLength));
        assert_eq!(TiffTag::from_u16(259), Some(TiffTag::Compression));

        // Tile tags
        assert_eq!(TiffTag::from_u16(322), Some(TiffTag::TileWidth));
        assert_eq!(TiffTag::from_u16(323), Some(TiffTag::TileLength));
        assert_eq!(TiffTag::from_u16(324), Some(TiffTag::TileOffsets));
        assert_eq!(TiffTag::from_u16(325), Some(TiffTag::TileByteCounts));

        // JPEG tables
        assert_eq!(TiffTag::from_u16(347), Some(TiffTag::JpegTables));

        // Strip tags (for detection)
        assert_eq!(TiffTag::from_u16(273), Some(TiffTag::StripOffsets));
        assert_eq!(TiffTag::from_u16(279), Some(TiffTag::StripByteCounts));

        // Unknown tags
        assert_eq!(TiffTag::from_u16(0), None);
        assert_eq!(TiffTag::from_u16(9999), None);
    }

    #[test]
    fn test_tiff_tag_as_u16() {
        assert_eq!(TiffTag::ImageWidth.as_u16(), 256);
        assert_eq!(TiffTag::TileOffsets.as_u16(), 324);
        assert_eq!(TiffTag::JpegTables.as_u16(), 347);
    }

    // -------------------------------------------------------------------------
    // Compression Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_compression_from_u16() {
        assert_eq!(Compression::from_u16(1), Some(Compression::None));
        assert_eq!(Compression::from_u16(5), Some(Compression::Lzw));
        assert_eq!(Compression::from_u16(7), Some(Compression::Jpeg));
        assert_eq!(Compression::from_u16(8), Some(Compression::Deflate));
        assert_eq!(Compression::from_u16(0), None);
    }

    #[test]
    fn test_compression_is_supported() {
        assert!(Compression::Jpeg.is_supported());
        assert!(!Compression::None.is_supported());
        assert!(!Compression::Lzw.is_supported());
        assert!(!Compression::Deflate.is_supported());
        assert!(!Compression::Jpeg2000.is_supported());
    }

    #[test]
    fn test_compression_name() {
        assert_eq!(Compression::Jpeg.name(), "JPEG");
        assert_eq!(Compression::Lzw.name(), "LZW");
        assert_eq!(Compression::Deflate.name(), "Deflate");
    }
}
