//! Tile encoder with JPEG and JPEG 2000 support.
//!
//! This module handles decoding source tiles (JPEG or JPEG 2000) and
//! re-encoding them as JPEG at a specified quality level.
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
//!
//! - **Format detection**: Source format is auto-detected from magic bytes,
//!   supporting both JPEG (FFD8) and JPEG 2000 (FF4F or JP2 container).

use bytes::Bytes;
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageReader};
use jpeg2k::Image as J2kImage;
use std::io::Cursor;

use crate::error::TileError;

// =============================================================================
// Format Detection
// =============================================================================

/// Detected tile format based on magic bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TileFormat {
    /// JPEG format (FFD8 magic)
    Jpeg,
    /// JPEG 2000 codestream or JP2 container
    Jpeg2000,
    /// Unknown format
    Unknown,
}

/// Detect the format of tile data from its magic bytes.
///
/// # Arguments
/// * `data` - Raw tile bytes
///
/// # Returns
/// The detected format, or `Unknown` if not recognized.
fn detect_tile_format(data: &[u8]) -> TileFormat {
    if data.len() < 2 {
        return TileFormat::Unknown;
    }

    // JPEG: SOI marker (FF D8)
    if data[0] == 0xFF && data[1] == 0xD8 {
        return TileFormat::Jpeg;
    }

    // JPEG 2000 codestream: SOC + SIZ markers (FF 4F FF 51)
    if data.len() >= 4 && data[0..4] == [0xFF, 0x4F, 0xFF, 0x51] {
        return TileFormat::Jpeg2000;
    }

    // JPEG 2000 JP2 container: signature box
    // Box length (4 bytes) + "jP  " signature
    if data.len() >= 12 && &data[4..8] == b"jP  " {
        return TileFormat::Jpeg2000;
    }

    TileFormat::Unknown
}

/// Decode JPEG 2000 data to a DynamicImage.
///
/// # Arguments
/// * `data` - Raw JPEG 2000 bytes (codestream or JP2 container)
///
/// # Returns
/// Decoded image, or error if decoding fails.
fn decode_jpeg2000(data: &[u8]) -> Result<DynamicImage, TileError> {
    let j2k_image = J2kImage::from_bytes(data).map_err(|e| TileError::DecodeError {
        message: format!("JPEG 2000 decode error: {}", e),
    })?;

    // First, try the standard TryFrom conversion which handles color space
    // conversion properly for most images. Wrap in catch_unwind to handle
    // the rare panic case in the jpeg2k crate.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        DynamicImage::try_from(&j2k_image)
    }));

    match result {
        Ok(Ok(img)) => Ok(img),
        Ok(Err(e)) => Err(TileError::DecodeError {
            message: format!("JPEG 2000 to DynamicImage conversion error: {}", e),
        }),
        Err(_) => {
            // TryFrom panicked - fall back to manual conversion using components
            decode_jpeg2000_manual(&j2k_image)
        }
    }
}

/// Manual JPEG 2000 decoding fallback for when TryFrom panics.
///
/// This handles YCbCr 4:2:0 subsampled images by manually upsampling
/// and converting to RGB.
fn decode_jpeg2000_manual(j2k_image: &J2kImage) -> Result<DynamicImage, TileError> {
    let num_components = j2k_image.num_components();
    let components = j2k_image.components();

    if components.is_empty() {
        return Err(TileError::DecodeError {
            message: "JPEG 2000 image has no components".to_string(),
        });
    }

    match num_components {
        1 => {
            // Grayscale - use first component directly
            let comp = &components[0];
            let comp_data = comp.data();
            let comp_width = comp.width();
            let comp_height = comp.height();

            // Convert i32 to u8
            let pixels: Vec<u8> = comp_data.iter().map(|&v| v.clamp(0, 255) as u8).collect();

            image::GrayImage::from_raw(comp_width, comp_height, pixels)
                .map(DynamicImage::ImageLuma8)
                .ok_or_else(|| TileError::DecodeError {
                    message: format!(
                        "Failed to create grayscale image from components: {}x{}",
                        comp_width, comp_height
                    ),
                })
        }
        3 => {
            // RGB or YCbCr - check for subsampling and handle accordingly
            let y_comp = &components[0];
            let cb_comp = &components[1];
            let cr_comp = &components[2];

            // Check if chroma is subsampled
            let y_width = y_comp.width();
            let y_height = y_comp.height();
            let cb_width = cb_comp.width();
            let cb_height = cb_comp.height();

            if cb_width == y_width && cb_height == y_height {
                // No subsampling - direct RGB conversion
                let y_data = y_comp.data();
                let cb_data = cb_comp.data();
                let cr_data = cr_comp.data();

                let mut pixels = Vec::with_capacity((y_width * y_height * 3) as usize);
                for i in 0..(y_width * y_height) as usize {
                    pixels.push(y_data[i].clamp(0, 255) as u8);
                    pixels.push(cb_data[i].clamp(0, 255) as u8);
                    pixels.push(cr_data[i].clamp(0, 255) as u8);
                }

                image::RgbImage::from_raw(y_width, y_height, pixels)
                    .map(DynamicImage::ImageRgb8)
                    .ok_or_else(|| TileError::DecodeError {
                        message: format!(
                            "Failed to create RGB image from components: {}x{}",
                            y_width, y_height
                        ),
                    })
            } else {
                // Chroma subsampling detected - need to upsample and convert YCbCr to RGB
                let y_data = y_comp.data();
                let cb_data = cb_comp.data();
                let cr_data = cr_comp.data();

                let mut pixels = Vec::with_capacity((y_width * y_height * 3) as usize);

                for y_row in 0..y_height {
                    for y_col in 0..y_width {
                        let y_idx = (y_row * y_width + y_col) as usize;

                        // Map Y coordinate to subsampled Cb/Cr coordinate
                        let cb_col = (y_col * cb_width) / y_width;
                        let cb_row = (y_row * cb_height) / y_height;
                        let cb_idx = (cb_row * cb_width + cb_col) as usize;

                        let y_val = y_data[y_idx] as f32;
                        let cb_val = cb_data.get(cb_idx).copied().unwrap_or(128) as f32 - 128.0;
                        let cr_val = cr_data.get(cb_idx).copied().unwrap_or(128) as f32 - 128.0;

                        // YCbCr to RGB conversion (ITU-R BT.601)
                        let r = (y_val + 1.402 * cr_val).clamp(0.0, 255.0) as u8;
                        let g =
                            (y_val - 0.344136 * cb_val - 0.714136 * cr_val).clamp(0.0, 255.0) as u8;
                        let b = (y_val + 1.772 * cb_val).clamp(0.0, 255.0) as u8;

                        pixels.push(r);
                        pixels.push(g);
                        pixels.push(b);
                    }
                }

                image::RgbImage::from_raw(y_width, y_height, pixels)
                    .map(DynamicImage::ImageRgb8)
                    .ok_or_else(|| TileError::DecodeError {
                        message: format!(
                            "Failed to create RGB image from YCbCr components: {}x{}",
                            y_width, y_height
                        ),
                    })
            }
        }
        _ => Err(TileError::DecodeError {
            message: format!(
                "Unsupported JPEG 2000 component count: {} (expected 1 or 3)",
                num_components
            ),
        }),
    }
}

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

    /// Decode source tile and re-encode at the specified quality.
    ///
    /// This method auto-detects the source format (JPEG or JPEG 2000) and
    /// decodes accordingly. Output is always JPEG.
    ///
    /// # Arguments
    ///
    /// * `source` - Raw tile data (JPEG or JPEG 2000)
    /// * `quality` - Output JPEG quality (1-100)
    ///
    /// # Returns
    ///
    /// Encoded JPEG data at the requested quality.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The source data format is not recognized
    /// - Decoding fails
    /// - Encoding fails
    pub fn encode(&self, source: &[u8], quality: u8) -> Result<Bytes, TileError> {
        // Clamp quality to valid range
        let quality = quality.clamp(MIN_JPEG_QUALITY, MAX_JPEG_QUALITY);

        // Detect source format and decode
        let format = detect_tile_format(source);

        let img = match format {
            TileFormat::Jpeg => {
                let cursor = Cursor::new(source);
                let reader = ImageReader::with_format(cursor, image::ImageFormat::Jpeg);
                reader.decode().map_err(|e| TileError::DecodeError {
                    message: format!("JPEG decode error: {}", e),
                })?
            }
            TileFormat::Jpeg2000 => decode_jpeg2000(source)?,
            TileFormat::Unknown => {
                return Err(TileError::DecodeError {
                    message: "Unknown tile format: expected JPEG or JPEG 2000".to_string(),
                });
            }
        };

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

    /// Get image dimensions without fully decoding (when possible).
    ///
    /// This is useful for validation or metadata queries.
    ///
    /// Note: For JPEG 2000, this currently requires a full decode.
    ///
    /// # Returns
    ///
    /// `(width, height)` in pixels.
    pub fn dimensions(&self, source: &[u8]) -> Result<(u32, u32), TileError> {
        let format = detect_tile_format(source);

        match format {
            TileFormat::Jpeg => {
                let cursor = Cursor::new(source);
                let reader = ImageReader::with_format(cursor, image::ImageFormat::Jpeg);
                reader
                    .into_dimensions()
                    .map_err(|e| TileError::DecodeError {
                        message: format!("JPEG dimensions error: {}", e),
                    })
            }
            TileFormat::Jpeg2000 => {
                // jpeg2k requires full decode to get dimensions
                let j2k = J2kImage::from_bytes(source).map_err(|e| TileError::DecodeError {
                    message: format!("JPEG 2000 decode error: {}", e),
                })?;
                Ok((j2k.width(), j2k.height()))
            }
            TileFormat::Unknown => Err(TileError::DecodeError {
                message: "Unknown tile format: expected JPEG or JPEG 2000".to_string(),
            }),
        }
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

    // -------------------------------------------------------------------------
    // Format Detection Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_detect_jpeg_format() {
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        assert_eq!(detect_tile_format(&jpeg), TileFormat::Jpeg);
    }

    #[test]
    fn test_detect_j2k_codestream_format() {
        // JPEG 2000 codestream: SOC + SIZ markers
        let j2k = [0xFF, 0x4F, 0xFF, 0x51, 0x00, 0x00];
        assert_eq!(detect_tile_format(&j2k), TileFormat::Jpeg2000);
    }

    #[test]
    fn test_detect_jp2_container_format() {
        // JP2 container with signature box
        let jp2 = [
            0x00, 0x00, 0x00, 0x0C, // Box length
            0x6A, 0x50, 0x20, 0x20, // "jP  " signature
            0x0D, 0x0A, 0x87, 0x0A, // Additional signature bytes
        ];
        assert_eq!(detect_tile_format(&jp2), TileFormat::Jpeg2000);
    }

    #[test]
    fn test_detect_unknown_format() {
        let unknown = [0x00, 0x00, 0x00, 0x00];
        assert_eq!(detect_tile_format(&unknown), TileFormat::Unknown);
    }

    #[test]
    fn test_detect_empty_data() {
        assert_eq!(detect_tile_format(&[]), TileFormat::Unknown);
    }

    #[test]
    fn test_detect_short_data() {
        assert_eq!(detect_tile_format(&[0xFF]), TileFormat::Unknown);
    }

    #[test]
    fn test_encode_unknown_format_returns_error() {
        let encoder = JpegTileEncoder::new();
        let unknown = vec![0x00, 0x01, 0x02, 0x03];

        let result = encoder.encode(&unknown, 80);
        assert!(result.is_err());

        match result {
            Err(TileError::DecodeError { message }) => {
                assert!(message.contains("Unknown tile format"));
            }
            _ => panic!("Expected DecodeError with format message"),
        }
    }
}
