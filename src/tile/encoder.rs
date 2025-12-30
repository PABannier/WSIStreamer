//! JPEG tile encoder.
//!
//! This module handles decoding source JPEG tiles and re-encoding them
//! at a specified quality level.
//!
//! # Design Decisions
//!
//! - **Always decode/encode**: For simplicity and correctness, tiles are always
//!   decoded from source format and re-encoded as JPEG. No passthrough optimization.
//!
//! - **No resizing**: Tiles are served at their native size. The tile coordinates
//!   specify tile indices, not pixel coordinates.
//!
//! - **Quality control**: JPEG quality is configurable per request, allowing
//!   clients to trade off file size vs image quality.

use bytes::Bytes;
use image::codecs::jpeg::JpegEncoder;
use image::ImageReader;
use std::io::Cursor;

use crate::error::TileError;

/// Default JPEG quality (1-100).
pub const DEFAULT_JPEG_QUALITY: u8 = 80;

/// Minimum allowed JPEG quality.
pub const MIN_JPEG_QUALITY: u8 = 1;

/// Maximum allowed JPEG quality.
pub const MAX_JPEG_QUALITY: u8 = 100;

// =============================================================================
// JPEG Encoder
// =============================================================================

/// JPEG tile encoder for decoding and re-encoding tiles.
///
/// This encoder takes raw JPEG data from slide tiles, decodes it to pixels,
/// and re-encodes it at the requested quality level.
///
/// # Example
///
/// ```ignore
/// use wsi_streamer::tile::JpegTileEncoder;
/// use bytes::Bytes;
///
/// let encoder = JpegTileEncoder::new();
///
/// // Source JPEG data from slide
/// let source_jpeg: Bytes = /* ... */;
///
/// // Re-encode at quality 85
/// let output = encoder.encode(&source_jpeg, 85)?;
/// ```
#[derive(Debug, Clone, Default)]
pub struct JpegTileEncoder {
    // Currently stateless, but struct allows future extension
    // (e.g., shared thread pool, encoder settings)
}

impl JpegTileEncoder {
    /// Create a new JPEG tile encoder.
    pub fn new() -> Self {
        Self {}
    }

    /// Decode source JPEG and re-encode at the specified quality.
    ///
    /// # Arguments
    ///
    /// * `source` - Raw JPEG data from the slide tile
    /// * `quality` - Output JPEG quality (1-100)
    ///
    /// # Returns
    ///
    /// Encoded JPEG data at the requested quality.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The source data is not valid JPEG
    /// - Decoding fails
    /// - Encoding fails
    pub fn encode(&self, source: &[u8], quality: u8) -> Result<Bytes, TileError> {
        // Clamp quality to valid range
        let quality = quality.clamp(MIN_JPEG_QUALITY, MAX_JPEG_QUALITY);

        // Decode source JPEG
        let cursor = Cursor::new(source);
        let reader = ImageReader::with_format(cursor, image::ImageFormat::Jpeg);

        let img = reader.decode().map_err(|e| TileError::DecodeError {
            message: e.to_string(),
        })?;

        // Encode to JPEG at requested quality
        let mut output = Vec::new();
        let mut encoder = JpegEncoder::new_with_quality(&mut output, quality);

        encoder
            .encode_image(&img)
            .map_err(|e| TileError::EncodeError {
                message: e.to_string(),
            })?;

        Ok(Bytes::from(output))
    }

    /// Decode source JPEG and re-encode at the default quality.
    ///
    /// This is a convenience method equivalent to `encode(source, DEFAULT_JPEG_QUALITY)`.
    pub fn encode_default(&self, source: &[u8]) -> Result<Bytes, TileError> {
        self.encode(source, DEFAULT_JPEG_QUALITY)
    }

    /// Get image dimensions without fully decoding.
    ///
    /// This is useful for validation or metadata queries.
    ///
    /// # Returns
    ///
    /// `(width, height)` in pixels.
    pub fn dimensions(&self, source: &[u8]) -> Result<(u32, u32), TileError> {
        let cursor = Cursor::new(source);
        let reader = ImageReader::with_format(cursor, image::ImageFormat::Jpeg);

        let (width, height) = reader.into_dimensions().map_err(|e| TileError::DecodeError {
            message: e.to_string(),
        })?;

        Ok((width, height))
    }
}

// =============================================================================
// Utility Functions
// =============================================================================

/// Validate JPEG quality parameter.
///
/// Returns `true` if quality is in the valid range (1-100).
#[inline]
pub fn is_valid_quality(quality: u8) -> bool {
    quality >= MIN_JPEG_QUALITY && quality <= MAX_JPEG_QUALITY
}

/// Clamp quality to valid range.
///
/// Values below 1 become 1, values above 100 become 100.
#[inline]
pub fn clamp_quality(quality: u8) -> u8 {
    quality.clamp(MIN_JPEG_QUALITY, MAX_JPEG_QUALITY)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_jpeg() -> Vec<u8> {
        // Create a simple 8x8 gray image and encode it
        use image::{GrayImage, Luma};

        let img = GrayImage::from_fn(8, 8, |x, y| {
            let val = ((x + y) * 16) as u8;
            Luma([val])
        });

        let mut buf = Vec::new();
        let mut encoder = JpegEncoder::new_with_quality(&mut buf, 90);
        encoder.encode_image(&img).unwrap();
        buf
    }

    #[test]
    fn test_encoder_creation() {
        let encoder = JpegTileEncoder::new();
        // Just verify the encoder can be created without panicking
        let _ = &encoder;
    }

    #[test]
    fn test_encode_valid_jpeg() {
        let encoder = JpegTileEncoder::new();
        let source = create_test_jpeg();

        let result = encoder.encode(&source, 80);
        assert!(result.is_ok());

        let output = result.unwrap();
        // Output should be valid JPEG (starts with FFD8)
        assert!(output.len() >= 2);
        assert_eq!(output[0], 0xFF);
        assert_eq!(output[1], 0xD8);
    }

    #[test]
    fn test_encode_different_qualities() {
        let encoder = JpegTileEncoder::new();
        let source = create_test_jpeg();

        let low_quality = encoder.encode(&source, 10).unwrap();
        let high_quality = encoder.encode(&source, 95).unwrap();

        // Higher quality should generally produce larger files
        // (though not guaranteed for all images)
        assert!(low_quality.len() > 0);
        assert!(high_quality.len() > 0);
    }

    #[test]
    fn test_encode_default() {
        let encoder = JpegTileEncoder::new();
        let source = create_test_jpeg();

        let result = encoder.encode_default(&source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_encode_invalid_data() {
        let encoder = JpegTileEncoder::new();
        let invalid = vec![0x00, 0x01, 0x02, 0x03];

        let result = encoder.encode(&invalid, 80);
        assert!(result.is_err());

        match result {
            Err(TileError::DecodeError { .. }) => {}
            _ => panic!("Expected DecodeError"),
        }
    }

    #[test]
    fn test_encode_empty_data() {
        let encoder = JpegTileEncoder::new();

        let result = encoder.encode(&[], 80);
        assert!(result.is_err());
    }

    #[test]
    fn test_quality_clamping() {
        let encoder = JpegTileEncoder::new();
        let source = create_test_jpeg();

        // Quality 0 should be clamped to 1
        let result = encoder.encode(&source, 0);
        assert!(result.is_ok());

        // Quality 255 should be clamped to 100
        let result = encoder.encode(&source, 255);
        assert!(result.is_ok());
    }

    #[test]
    fn test_dimensions() {
        let encoder = JpegTileEncoder::new();
        let source = create_test_jpeg();

        let (width, height) = encoder.dimensions(&source).unwrap();
        assert_eq!(width, 8);
        assert_eq!(height, 8);
    }

    #[test]
    fn test_dimensions_invalid() {
        let encoder = JpegTileEncoder::new();
        let invalid = vec![0x00, 0x01, 0x02];

        let result = encoder.dimensions(&invalid);
        assert!(result.is_err());
    }

    #[test]
    fn test_is_valid_quality() {
        assert!(!is_valid_quality(0));
        assert!(is_valid_quality(1));
        assert!(is_valid_quality(50));
        assert!(is_valid_quality(100));
        assert!(!is_valid_quality(101));
    }

    #[test]
    fn test_clamp_quality() {
        assert_eq!(clamp_quality(0), 1);
        assert_eq!(clamp_quality(1), 1);
        assert_eq!(clamp_quality(50), 50);
        assert_eq!(clamp_quality(100), 100);
        assert_eq!(clamp_quality(150), 100);
        assert_eq!(clamp_quality(255), 100);
    }

    #[test]
    fn test_output_is_valid_jpeg() {
        let encoder = JpegTileEncoder::new();
        let source = create_test_jpeg();

        let output = encoder.encode(&source, 80).unwrap();

        // Verify JPEG markers
        assert_eq!(output[0], 0xFF); // SOI marker
        assert_eq!(output[1], 0xD8);
        assert_eq!(output[output.len() - 2], 0xFF); // EOI marker
        assert_eq!(output[output.len() - 1], 0xD9);

        // Verify we can decode the output
        let result = encoder.dimensions(&output);
        assert!(result.is_ok());
    }
}
