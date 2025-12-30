//! Format detection for Whole Slide Image files.
//!
//! This module provides automatic detection of WSI file formats by examining
//! magic bytes and vendor-specific markers. Currently supports:
//!
//! - **Aperio SVS**: TIFF-based format identified by "Aperio" string in ImageDescription
//! - **Generic Pyramidal TIFF**: Standard tiled TIFF with multiple resolution levels
//!
//! Unsupported formats return an error that should map to HTTP 415 Unsupported Media Type.

use crate::error::FormatError;
use crate::io::RangeReader;

use super::tiff::{ByteOrder, Ifd, TiffHeader, TiffTag, BIGTIFF_HEADER_SIZE, TIFF_HEADER_SIZE};

// =============================================================================
// SlideFormat
// =============================================================================

/// Detected slide format.
///
/// This enum represents the different WSI formats that can be served.
/// Format detection is based on magic bytes and vendor-specific markers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlideFormat {
    /// Aperio SVS format (TIFF-based with JPEGTables for abbreviated streams)
    AperioSvs,

    /// Generic pyramidal TIFF (standard tiled TIFF with multiple resolutions)
    GenericTiff,
}

impl SlideFormat {
    /// Get a human-readable name for the format.
    pub const fn name(&self) -> &'static str {
        match self {
            SlideFormat::AperioSvs => "Aperio SVS",
            SlideFormat::GenericTiff => "Generic Pyramidal TIFF",
        }
    }
}

// =============================================================================
// Format Detection
// =============================================================================

/// Minimum bytes needed for initial format detection (TIFF/BigTIFF header).
const MIN_HEADER_BYTES: usize = BIGTIFF_HEADER_SIZE;

/// Maximum bytes to read from ImageDescription for format detection.
/// We don't need to read the entire description, just enough to find markers.
const MAX_DESCRIPTION_BYTES: usize = 1024;

/// Marker string for Aperio SVS format.
const APERIO_MARKER: &[u8] = b"Aperio";

/// Detect the format of a slide file.
///
/// This function reads the file header and examines vendor-specific markers
/// to determine the slide format.
///
/// # Arguments
/// * `reader` - Range reader for the file
///
/// # Returns
/// * `Ok(SlideFormat)` - The detected format
/// * `Err(FormatError::UnsupportedFormat)` - File is not a recognized format
/// * `Err(FormatError::Tiff)` - Error parsing TIFF structure
///
/// # Format Detection Logic
///
/// 1. Read initial bytes and verify TIFF/BigTIFF magic
/// 2. Parse the first IFD to access ImageDescription tag
/// 3. If ImageDescription contains "Aperio", classify as SVS
/// 4. Otherwise, classify as generic pyramidal TIFF
pub async fn detect_format<R: RangeReader>(reader: &R) -> Result<SlideFormat, FormatError> {
    // Check file size
    if reader.size() < MIN_HEADER_BYTES as u64 {
        return Err(FormatError::UnsupportedFormat {
            reason: "File too small to be a valid TIFF".to_string(),
        });
    }

    // Read and parse header
    let header_bytes = reader.read_exact_at(0, MIN_HEADER_BYTES).await?;
    let header = TiffHeader::parse(&header_bytes, reader.size())?;

    // Read the first IFD to check for format-specific markers
    let format = detect_format_from_first_ifd(reader, &header).await?;

    Ok(format)
}

/// Detect format by examining the first IFD.
///
/// This reads the first IFD and checks the ImageDescription tag for
/// vendor-specific markers.
async fn detect_format_from_first_ifd<R: RangeReader>(
    reader: &R,
    header: &TiffHeader,
) -> Result<SlideFormat, FormatError> {
    // Read first IFD entry count
    let count_size = header.ifd_count_size();
    let count_bytes = reader
        .read_exact_at(header.first_ifd_offset, count_size)
        .await?;

    let entry_count = if header.is_bigtiff {
        header.byte_order.read_u64(&count_bytes)
    } else {
        header.byte_order.read_u16(&count_bytes) as u64
    };

    // Read the full IFD
    let ifd_size = Ifd::calculate_size(entry_count, header);
    let ifd_bytes = reader
        .read_exact_at(header.first_ifd_offset, ifd_size)
        .await?;
    let ifd = Ifd::parse(&ifd_bytes, header)?;

    // Check for ImageDescription tag
    if let Some(description) = read_image_description(reader, &ifd, header).await? {
        // Check for Aperio marker
        if contains_aperio_marker(&description) {
            return Ok(SlideFormat::AperioSvs);
        }
    }

    // Default to generic TIFF
    Ok(SlideFormat::GenericTiff)
}

/// Read the ImageDescription tag value from an IFD.
///
/// Returns None if the tag is not present.
async fn read_image_description<R: RangeReader>(
    reader: &R,
    ifd: &Ifd,
    header: &TiffHeader,
) -> Result<Option<Vec<u8>>, FormatError> {
    let entry = match ifd.get_entry_by_tag(TiffTag::ImageDescription) {
        Some(e) => e,
        None => return Ok(None),
    };

    // Limit how much we read
    let read_len = (entry.count as usize).min(MAX_DESCRIPTION_BYTES);
    if read_len == 0 {
        return Ok(None);
    }

    // Read the bytes
    let bytes = if entry.is_inline {
        // Inline value - extract from entry
        entry.value_offset_bytes[..read_len.min(entry.value_offset_bytes.len())].to_vec()
    } else {
        // Value at offset
        let offset = entry.value_offset(header.byte_order);
        reader.read_exact_at(offset, read_len).await?.to_vec()
    };

    Ok(Some(bytes))
}

/// Check if bytes contain the Aperio marker.
fn contains_aperio_marker(data: &[u8]) -> bool {
    // Simple substring search
    data.windows(APERIO_MARKER.len())
        .any(|window| window == APERIO_MARKER)
}

/// Check if bytes represent a valid TIFF header.
///
/// This is a quick check that can be used before attempting full parsing.
pub fn is_tiff_header(bytes: &[u8]) -> bool {
    if bytes.len() < TIFF_HEADER_SIZE {
        return false;
    }

    // Check magic bytes
    let magic = u16::from_le_bytes([bytes[0], bytes[1]]);
    if magic != 0x4949 && magic != 0x4D4D {
        return false;
    }

    // Check version
    let byte_order = if magic == 0x4949 {
        ByteOrder::LittleEndian
    } else {
        ByteOrder::BigEndian
    };

    let version = byte_order.read_u16(&bytes[2..4]);
    version == 42 || version == 43
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // is_tiff_header tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_is_tiff_header_little_endian_classic() {
        let header = [
            0x49, 0x49, // II (little-endian)
            0x2A, 0x00, // Version 42 (TIFF)
            0x08, 0x00, 0x00, 0x00, // IFD offset
        ];
        assert!(is_tiff_header(&header));
    }

    #[test]
    fn test_is_tiff_header_big_endian_classic() {
        let header = [
            0x4D, 0x4D, // MM (big-endian)
            0x00, 0x2A, // Version 42 (TIFF)
            0x00, 0x00, 0x00, 0x08, // IFD offset
        ];
        assert!(is_tiff_header(&header));
    }

    #[test]
    fn test_is_tiff_header_little_endian_bigtiff() {
        let header = [
            0x49, 0x49, // II (little-endian)
            0x2B, 0x00, // Version 43 (BigTIFF)
            0x08, 0x00, // Offset size
            0x00, 0x00, // Reserved
            0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // IFD offset
        ];
        assert!(is_tiff_header(&header));
    }

    #[test]
    fn test_is_tiff_header_big_endian_bigtiff() {
        let header = [
            0x4D, 0x4D, // MM (big-endian)
            0x00, 0x2B, // Version 43 (BigTIFF)
            0x00, 0x08, // Offset size
            0x00, 0x00, // Reserved
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, // IFD offset
        ];
        assert!(is_tiff_header(&header));
    }

    #[test]
    fn test_is_tiff_header_invalid_magic() {
        let header = [
            0x00, 0x00, // Invalid magic
            0x2A, 0x00, 0x08, 0x00, 0x00, 0x00,
        ];
        assert!(!is_tiff_header(&header));
    }

    #[test]
    fn test_is_tiff_header_invalid_version() {
        let header = [
            0x49, 0x49, // II
            0x00, 0x00, // Invalid version
            0x08, 0x00, 0x00, 0x00,
        ];
        assert!(!is_tiff_header(&header));
    }

    #[test]
    fn test_is_tiff_header_too_small() {
        let header = [0x49, 0x49, 0x2A, 0x00]; // Only 4 bytes
        assert!(!is_tiff_header(&header));
    }

    #[test]
    fn test_is_tiff_header_jpeg() {
        // JPEG magic bytes
        let header = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46];
        assert!(!is_tiff_header(&header));
    }

    #[test]
    fn test_is_tiff_header_png() {
        // PNG magic bytes
        let header = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        assert!(!is_tiff_header(&header));
    }

    // -------------------------------------------------------------------------
    // contains_aperio_marker tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_contains_aperio_marker_present() {
        let data = b"Aperio Image Library v12.0.0";
        assert!(contains_aperio_marker(data));
    }

    #[test]
    fn test_contains_aperio_marker_in_description() {
        let data = b"Some prefix|Aperio Image Library|Some suffix";
        assert!(contains_aperio_marker(data));
    }

    #[test]
    fn test_contains_aperio_marker_not_present() {
        let data = b"Generic TIFF image description";
        assert!(!contains_aperio_marker(data));
    }

    #[test]
    fn test_contains_aperio_marker_empty() {
        let data = b"";
        assert!(!contains_aperio_marker(data));
    }

    #[test]
    fn test_contains_aperio_marker_partial() {
        let data = b"Aperi"; // Partial match
        assert!(!contains_aperio_marker(data));
    }

    #[test]
    fn test_contains_aperio_marker_case_sensitive() {
        let data = b"aperio"; // Lowercase
        assert!(!contains_aperio_marker(data));
    }

    // -------------------------------------------------------------------------
    // SlideFormat tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_slide_format_name() {
        assert_eq!(SlideFormat::AperioSvs.name(), "Aperio SVS");
        assert_eq!(SlideFormat::GenericTiff.name(), "Generic Pyramidal TIFF");
    }
}
