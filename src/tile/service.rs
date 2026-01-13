//! Tile Service for orchestrating tile generation.
//!
//! The TileService is the main entry point for tile requests. It orchestrates:
//! - Request validation
//! - Cache lookups
//! - Slide access via registry
//! - JPEG decoding and re-encoding
//! - Result caching
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                         TileService                              │
//! │  ┌─────────────────────────────────────────────────────────┐    │
//! │  │                    get_tile()                           │    │
//! │  │  1. Validate params   4. Read tile from slide           │    │
//! │  │  2. Check cache       5. Encode at quality              │    │
//! │  │  3. Get slide         6. Cache & return                 │    │
//! │  └─────────────────────────────────────────────────────────┘    │
//! │           │                    │                    │            │
//! │           ▼                    ▼                    ▼            │
//! │    ┌───────────┐      ┌──────────────┐    ┌──────────────────┐  │
//! │    │ TileCache │      │ SlideRegistry│    │ JpegTileEncoder  │  │
//! │    └───────────┘      └──────────────┘    └──────────────────┘  │
//! └─────────────────────────────────────────────────────────────────┘
//! ```

use std::sync::Arc;

use bytes::Bytes;

use crate::error::TileError;
use crate::slide::{SlideRegistry, SlideSource};

use super::cache::{TileCache, TileCacheKey};
use super::encoder::{is_valid_quality, JpegTileEncoder, DEFAULT_JPEG_QUALITY};

// =============================================================================
// Tile Request
// =============================================================================

/// A request for a tile.
///
/// This struct contains all parameters needed to identify and render a tile.
#[derive(Debug, Clone)]
pub struct TileRequest {
    /// Slide identifier (e.g., S3 path)
    pub slide_id: String,

    /// Pyramid level (0 = highest resolution)
    pub level: usize,

    /// Tile X coordinate (0-indexed from left)
    pub tile_x: u32,

    /// Tile Y coordinate (0-indexed from top)
    pub tile_y: u32,

    /// JPEG quality (1-100, defaults to 80)
    pub quality: u8,
}

impl TileRequest {
    /// Create a new tile request with default quality.
    pub fn new(slide_id: impl Into<String>, level: usize, tile_x: u32, tile_y: u32) -> Self {
        Self {
            slide_id: slide_id.into(),
            level,
            tile_x,
            tile_y,
            quality: DEFAULT_JPEG_QUALITY,
        }
    }

    /// Create a new tile request with specified quality.
    pub fn with_quality(
        slide_id: impl Into<String>,
        level: usize,
        tile_x: u32,
        tile_y: u32,
        quality: u8,
    ) -> Self {
        Self {
            slide_id: slide_id.into(),
            level,
            tile_x,
            tile_y,
            quality,
        }
    }
}

// =============================================================================
// Tile Response
// =============================================================================

/// Response from the tile service.
#[derive(Debug, Clone)]
pub struct TileResponse {
    /// The encoded JPEG tile data
    pub data: Bytes,

    /// Whether this tile was served from cache
    pub cache_hit: bool,

    /// The JPEG quality used for encoding
    pub quality: u8,
}

// =============================================================================
// Tile Service
// =============================================================================

/// Service for generating and caching tiles.
///
/// The TileService orchestrates the full tile pipeline:
/// 1. Validates request parameters
/// 2. Checks the tile cache for existing results
/// 3. Fetches the slide from the registry
/// 4. Reads raw tile data from the slide
/// 5. Decodes and re-encodes at the requested quality
/// 6. Caches and returns the result
///
/// # Type Parameters
///
/// * `S` - The slide source type (e.g., S3-based source)
///
/// # Example
///
/// ```ignore
/// use wsi_streamer::tile::{TileService, TileRequest};
/// use wsi_streamer::slide::SlideRegistry;
///
/// // Create registry and service
/// let registry = SlideRegistry::new(source);
/// let service = TileService::new(registry);
///
/// // Request a tile
/// let request = TileRequest::new("slides/sample.svs", 0, 1, 2);
/// let response = service.get_tile(request).await?;
///
/// println!("Tile size: {} bytes, cache hit: {}", response.data.len(), response.cache_hit);
/// ```
pub struct TileService<S: SlideSource> {
    /// The slide registry for accessing slides
    registry: Arc<SlideRegistry<S>>,

    /// Cache for encoded tiles
    cache: TileCache,

    /// JPEG encoder
    encoder: JpegTileEncoder,
}

impl<S: SlideSource> TileService<S> {
    /// Create a new tile service with default cache settings.
    ///
    /// Uses default tile cache capacity (100MB).
    pub fn new(registry: SlideRegistry<S>) -> Self {
        Self {
            registry: Arc::new(registry),
            cache: TileCache::new(),
            encoder: JpegTileEncoder::new(),
        }
    }

    /// Create a new tile service with a shared registry.
    ///
    /// This allows multiple services or components to share the same registry.
    pub fn with_shared_registry(registry: Arc<SlideRegistry<S>>) -> Self {
        Self {
            registry,
            cache: TileCache::new(),
            encoder: JpegTileEncoder::new(),
        }
    }

    /// Create a new tile service with custom cache capacity.
    ///
    /// # Arguments
    ///
    /// * `registry` - The slide registry
    /// * `cache_capacity` - Maximum tile cache size in bytes
    pub fn with_cache_capacity(registry: SlideRegistry<S>, cache_capacity: usize) -> Self {
        Self {
            registry: Arc::new(registry),
            cache: TileCache::with_capacity(cache_capacity),
            encoder: JpegTileEncoder::new(),
        }
    }

    /// Get a tile, using cache when available.
    ///
    /// This is the main entry point for tile requests. It:
    /// 1. Validates the request parameters
    /// 2. Checks the cache for an existing tile
    /// 3. If not cached, fetches from the slide and encodes
    /// 4. Caches and returns the result
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The slide cannot be found or opened
    /// - The level is out of range
    /// - The tile coordinates are out of bounds
    /// - The tile data cannot be decoded or encoded
    pub async fn get_tile(&self, request: TileRequest) -> Result<TileResponse, TileError> {
        // Validate quality
        if !is_valid_quality(request.quality) {
            return Err(TileError::InvalidQuality {
                quality: request.quality,
            });
        }
        let quality = request.quality;

        // Create cache key
        let cache_key = TileCacheKey::new(
            request.slide_id.as_str(),
            request.level as u32,
            request.tile_x,
            request.tile_y,
            quality,
        );

        // Check cache first
        if let Some(cached_data) = self.cache.get(&cache_key).await {
            return Ok(TileResponse {
                data: cached_data,
                cache_hit: true,
                quality,
            });
        }

        // Cache miss - need to generate tile
        let tile_data = self.generate_tile(&request, quality).await?;

        // Cache the result
        self.cache.put(cache_key, tile_data.clone()).await;

        Ok(TileResponse {
            data: tile_data,
            cache_hit: false,
            quality,
        })
    }

    /// Generate a tile without caching.
    ///
    /// This is useful for one-off requests or when you want to bypass the cache.
    pub async fn generate_tile(
        &self,
        request: &TileRequest,
        quality: u8,
    ) -> Result<Bytes, TileError> {
        // Get the slide from registry
        let slide = self
            .registry
            .get_slide(&request.slide_id)
            .await
            .map_err(|e| match e {
                crate::error::FormatError::Io(io_err) => {
                    if matches!(io_err, crate::error::IoError::NotFound(_)) {
                        TileError::SlideNotFound {
                            slide_id: request.slide_id.clone(),
                        }
                    } else {
                        TileError::Io(io_err)
                    }
                }
                crate::error::FormatError::Tiff(tiff_err) => TileError::Slide(tiff_err),
                crate::error::FormatError::UnsupportedFormat { reason } => {
                    TileError::Slide(crate::error::TiffError::InvalidTagValue {
                        tag: "Format",
                        message: reason,
                    })
                }
            })?;

        // Validate level
        let level_count = slide.level_count();
        if request.level >= level_count {
            return Err(TileError::InvalidLevel {
                level: request.level,
                max_levels: level_count,
            });
        }

        // Validate tile coordinates
        let (max_x, max_y) = slide
            .tile_count(request.level)
            .ok_or(TileError::InvalidLevel {
                level: request.level,
                max_levels: level_count,
            })?;

        if request.tile_x >= max_x || request.tile_y >= max_y {
            return Err(TileError::TileOutOfBounds {
                level: request.level,
                x: request.tile_x,
                y: request.tile_y,
                max_x,
                max_y,
            });
        }

        // Read the raw tile data from the slide
        let raw_tile = slide
            .read_tile(request.level, request.tile_x, request.tile_y)
            .await?;

        // Decode and re-encode at the requested quality
        let encoded_tile = self.encoder.encode(&raw_tile, quality)?;

        Ok(encoded_tile)
    }

    /// Get tile cache statistics.
    ///
    /// Returns `(current_size, capacity, entry_count)`.
    pub async fn cache_stats(&self) -> (usize, usize, usize) {
        let size = self.cache.size().await;
        let capacity = self.cache.capacity();
        let count = self.cache.len().await;
        (size, capacity, count)
    }

    /// Clear the tile cache.
    pub async fn clear_cache(&self) {
        self.cache.clear().await;
    }

    /// Invalidate cached tiles for a specific slide.
    ///
    /// This removes all cached tiles for the given slide from the tile cache.
    /// Note: This is O(n) where n is the number of cached tiles.
    pub async fn invalidate_slide(&self, _slide_id: &str) {
        // TODO: Implement efficient per-slide invalidation
        // For now, this would require iterating the cache which isn't supported
        // by the LRU cache. A production implementation might use a different
        // data structure or maintain a secondary index.
    }

    /// Get a reference to the underlying registry.
    pub fn registry(&self) -> &Arc<SlideRegistry<S>> {
        &self.registry
    }

    /// Generate a thumbnail for a slide.
    ///
    /// This finds the lowest resolution level that fits within the requested
    /// max dimension and returns a tile or composited image.
    ///
    /// # Arguments
    ///
    /// * `slide_id` - The slide identifier
    /// * `max_dimension` - Maximum width or height for the thumbnail
    /// * `quality` - JPEG quality (1-100)
    ///
    /// # Returns
    ///
    /// A JPEG-encoded thumbnail image.
    pub async fn generate_thumbnail(
        &self,
        slide_id: &str,
        max_dimension: u32,
        quality: u8,
    ) -> Result<TileResponse, TileError> {
        // Validate quality
        if !is_valid_quality(quality) {
            return Err(TileError::InvalidQuality { quality });
        }

        // Get the slide from registry
        let slide = self
            .registry
            .get_slide(slide_id)
            .await
            .map_err(|e| match e {
                crate::error::FormatError::Io(io_err) => {
                    if matches!(io_err, crate::error::IoError::NotFound(_)) {
                        TileError::SlideNotFound {
                            slide_id: slide_id.to_string(),
                        }
                    } else {
                        TileError::Io(io_err)
                    }
                }
                crate::error::FormatError::Tiff(tiff_err) => TileError::Slide(tiff_err),
                crate::error::FormatError::UnsupportedFormat { reason } => {
                    TileError::Slide(crate::error::TiffError::InvalidTagValue {
                        tag: "Format",
                        message: reason,
                    })
                }
            })?;

        let (full_width, full_height) = slide.dimensions().ok_or(TileError::InvalidLevel {
            level: 0,
            max_levels: 0,
        })?;

        // Calculate target downsample
        let max_dim = full_width.max(full_height);
        let downsample = max_dim as f64 / max_dimension as f64;

        // Find best level for this downsample (or use lowest resolution level)
        let level = slide
            .best_level_for_downsample(downsample)
            .unwrap_or(slide.level_count().saturating_sub(1));

        let info = slide.level_info(level).ok_or(TileError::InvalidLevel {
            level,
            max_levels: slide.level_count(),
        })?;

        // If single tile covers the entire level, just return that tile
        if info.tiles_x == 1 && info.tiles_y == 1 {
            let request = TileRequest::with_quality(slide_id, level, 0, 0, quality);
            return self.get_tile(request).await;
        }

        // For multiple tiles, we need to composite them
        // For now, return the first tile from the lowest resolution level
        // as a simple implementation
        let lowest_level = slide.level_count().saturating_sub(1);
        let request = TileRequest::with_quality(slide_id, lowest_level, 0, 0, quality);
        self.get_tile(request).await
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::IoError;
    use crate::io::RangeReader;
    use crate::slide::SlideSource;
    use async_trait::async_trait;
    use image::codecs::jpeg::JpegEncoder;
    use image::{GrayImage, Luma};

    /// Create a test JPEG image
    fn create_test_jpeg() -> Vec<u8> {
        let img = GrayImage::from_fn(256, 256, |x, y| {
            let val = ((x + y) % 256) as u8;
            Luma([val])
        });

        let mut buf = Vec::new();
        let mut encoder = JpegEncoder::new_with_quality(&mut buf, 90);
        encoder.encode_image(&img).unwrap();
        buf
    }

    /// Create a minimal valid TIFF file with actual JPEG tile data
    fn create_tiff_with_jpeg_tile() -> Vec<u8> {
        let jpeg_data = create_test_jpeg();
        let jpeg_len = jpeg_data.len() as u32;

        // We need enough space for the TIFF structure + JPEG data
        let tile_data_offset = 1000u32;
        let total_size = tile_data_offset as usize + jpeg_data.len() + 100;
        let mut data = vec![0u8; total_size];

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
        // Entry count = 8
        data[8] = 0x08;
        data[9] = 0x00;

        let mut offset = 10;

        // Helper to write IFD entry
        let write_entry =
            |data: &mut [u8], offset: &mut usize, tag: u16, typ: u16, count: u32, value: u32| {
                data[*offset..*offset + 2].copy_from_slice(&tag.to_le_bytes());
                data[*offset + 2..*offset + 4].copy_from_slice(&typ.to_le_bytes());
                data[*offset + 4..*offset + 8].copy_from_slice(&count.to_le_bytes());
                data[*offset + 8..*offset + 12].copy_from_slice(&value.to_le_bytes());
                *offset += 12;
            };

        // ImageWidth (2048)
        write_entry(&mut data, &mut offset, 256, 4, 1, 2048);

        // ImageLength (1536)
        write_entry(&mut data, &mut offset, 257, 4, 1, 1536);

        // Compression (7 = JPEG)
        write_entry(&mut data, &mut offset, 259, 3, 1, 7);

        // TileWidth (256)
        write_entry(&mut data, &mut offset, 322, 3, 1, 256);

        // TileLength (256)
        write_entry(&mut data, &mut offset, 323, 3, 1, 256);

        // TileOffsets - 8x6=48 tiles, all pointing to same JPEG data for simplicity
        // Store offsets at position 200
        write_entry(&mut data, &mut offset, 324, 4, 48, 200);

        // TileByteCounts - all tiles have same size
        write_entry(&mut data, &mut offset, 325, 4, 48, 600);

        // BitsPerSample
        write_entry(&mut data, &mut offset, 258, 3, 1, 8);

        // Next IFD offset (0 = no more IFDs)
        data[offset..offset + 4].copy_from_slice(&0u32.to_le_bytes());

        // Write tile offsets array at offset 200 (all point to same tile data)
        for i in 0..48u32 {
            let arr_offset = 200 + (i as usize) * 4;
            data[arr_offset..arr_offset + 4].copy_from_slice(&tile_data_offset.to_le_bytes());
        }

        // Write tile byte counts array at offset 600
        for i in 0..48u32 {
            let arr_offset = 600 + (i as usize) * 4;
            data[arr_offset..arr_offset + 4].copy_from_slice(&jpeg_len.to_le_bytes());
        }

        // Write the actual JPEG tile data
        data[tile_data_offset as usize..tile_data_offset as usize + jpeg_data.len()]
            .copy_from_slice(&jpeg_data);

        data
    }

    /// Mock range reader
    struct MockReader {
        data: Bytes,
        identifier: String,
    }

    #[async_trait]
    impl RangeReader for MockReader {
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
            Ok(self.data.slice(start..end))
        }

        fn size(&self) -> u64 {
            self.data.len() as u64
        }

        fn identifier(&self) -> &str {
            &self.identifier
        }
    }

    /// Mock slide source
    struct MockSlideSource {
        data: Bytes,
    }

    impl MockSlideSource {
        fn new(data: Vec<u8>) -> Self {
            Self {
                data: Bytes::from(data),
            }
        }
    }

    #[async_trait]
    impl SlideSource for MockSlideSource {
        type Reader = MockReader;

        async fn create_reader(&self, slide_id: &str) -> Result<Self::Reader, IoError> {
            if slide_id.contains("notfound") {
                return Err(IoError::NotFound(slide_id.to_string()));
            }
            Ok(MockReader {
                data: self.data.clone(),
                identifier: format!("mock://{}", slide_id),
            })
        }
    }

    #[tokio::test]
    async fn test_tile_request_creation() {
        let request = TileRequest::new("test.svs", 0, 1, 2);
        assert_eq!(request.slide_id, "test.svs");
        assert_eq!(request.level, 0);
        assert_eq!(request.tile_x, 1);
        assert_eq!(request.tile_y, 2);
        assert_eq!(request.quality, DEFAULT_JPEG_QUALITY);

        let request_q = TileRequest::with_quality("test.svs", 1, 3, 4, 95);
        assert_eq!(request_q.quality, 95);
    }

    #[tokio::test]
    async fn test_get_tile_success() {
        let tiff_data = create_tiff_with_jpeg_tile();
        let source = MockSlideSource::new(tiff_data);
        let registry = SlideRegistry::new(source);
        let service = TileService::new(registry);

        let request = TileRequest::new("test.tif", 0, 0, 0);
        let response = service.get_tile(request).await;

        assert!(response.is_ok());
        let response = response.unwrap();

        // Should be a cache miss on first request
        assert!(!response.cache_hit);
        assert_eq!(response.quality, DEFAULT_JPEG_QUALITY);

        // Verify it's valid JPEG
        assert!(response.data.len() > 2);
        assert_eq!(response.data[0], 0xFF);
        assert_eq!(response.data[1], 0xD8);
    }

    #[tokio::test]
    async fn test_get_tile_cache_hit() {
        let tiff_data = create_tiff_with_jpeg_tile();
        let source = MockSlideSource::new(tiff_data);
        let registry = SlideRegistry::new(source);
        let service = TileService::new(registry);

        let request = TileRequest::new("test.tif", 0, 0, 0);

        // First request - cache miss
        let response1 = service.get_tile(request.clone()).await.unwrap();
        assert!(!response1.cache_hit);

        // Second request - cache hit
        let response2 = service.get_tile(request).await.unwrap();
        assert!(response2.cache_hit);
        assert_eq!(response1.data, response2.data);
    }

    #[tokio::test]
    async fn test_different_quality_different_cache() {
        let tiff_data = create_tiff_with_jpeg_tile();
        let source = MockSlideSource::new(tiff_data);
        let registry = SlideRegistry::new(source);
        let service = TileService::new(registry);

        let request_q80 = TileRequest::with_quality("test.tif", 0, 0, 0, 80);
        let request_q95 = TileRequest::with_quality("test.tif", 0, 0, 0, 95);

        // Request at quality 80
        let response1 = service.get_tile(request_q80.clone()).await.unwrap();
        assert!(!response1.cache_hit);

        // Request at quality 95 - should be cache miss (different quality)
        let response2 = service.get_tile(request_q95).await.unwrap();
        assert!(!response2.cache_hit);

        // Request at quality 80 again - should be cache hit
        let response3 = service.get_tile(request_q80).await.unwrap();
        assert!(response3.cache_hit);
    }

    #[tokio::test]
    async fn test_invalid_level() {
        let tiff_data = create_tiff_with_jpeg_tile();
        let source = MockSlideSource::new(tiff_data);
        let registry = SlideRegistry::new(source);
        let service = TileService::new(registry);

        // Request level 5 when only level 0 exists
        let request = TileRequest::new("test.tif", 5, 0, 0);
        let result = service.get_tile(request).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            TileError::InvalidLevel { level, max_levels } => {
                assert_eq!(level, 5);
                assert_eq!(max_levels, 1);
            }
            e => panic!("Expected InvalidLevel error, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn test_tile_out_of_bounds() {
        let tiff_data = create_tiff_with_jpeg_tile();
        let source = MockSlideSource::new(tiff_data);
        let registry = SlideRegistry::new(source);
        let service = TileService::new(registry);

        // Request tile (100, 100) when max is (8, 6)
        let request = TileRequest::new("test.tif", 0, 100, 100);
        let result = service.get_tile(request).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            TileError::TileOutOfBounds {
                level,
                x,
                y,
                max_x,
                max_y,
            } => {
                assert_eq!(level, 0);
                assert_eq!(x, 100);
                assert_eq!(y, 100);
                assert_eq!(max_x, 8);
                assert_eq!(max_y, 6);
            }
            e => panic!("Expected TileOutOfBounds error, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn test_slide_not_found() {
        let tiff_data = create_tiff_with_jpeg_tile();
        let source = MockSlideSource::new(tiff_data);
        let registry = SlideRegistry::new(source);
        let service = TileService::new(registry);

        let request = TileRequest::new("notfound.tif", 0, 0, 0);
        let result = service.get_tile(request).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            TileError::SlideNotFound { slide_id } => {
                assert_eq!(slide_id, "notfound.tif");
            }
            e => panic!("Expected SlideNotFound error, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn test_cache_stats() {
        let tiff_data = create_tiff_with_jpeg_tile();
        let source = MockSlideSource::new(tiff_data);
        let registry = SlideRegistry::new(source);
        let service = TileService::with_cache_capacity(registry, 10 * 1024 * 1024); // 10MB

        let (size, capacity, count) = service.cache_stats().await;
        assert_eq!(size, 0);
        assert_eq!(capacity, 10 * 1024 * 1024);
        assert_eq!(count, 0);

        // Add a tile
        let request = TileRequest::new("test.tif", 0, 0, 0);
        service.get_tile(request).await.unwrap();

        let (size, _, count) = service.cache_stats().await;
        assert!(size > 0);
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_clear_cache() {
        let tiff_data = create_tiff_with_jpeg_tile();
        let source = MockSlideSource::new(tiff_data);
        let registry = SlideRegistry::new(source);
        let service = TileService::new(registry);

        // Add some tiles
        service
            .get_tile(TileRequest::new("test.tif", 0, 0, 0))
            .await
            .unwrap();
        service
            .get_tile(TileRequest::new("test.tif", 0, 1, 0))
            .await
            .unwrap();

        let (_, _, count) = service.cache_stats().await;
        assert_eq!(count, 2);

        // Clear cache
        service.clear_cache().await;

        let (size, _, count) = service.cache_stats().await;
        assert_eq!(size, 0);
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_quality_validation() {
        let tiff_data = create_tiff_with_jpeg_tile();
        let source = MockSlideSource::new(tiff_data);
        let registry = SlideRegistry::new(source);
        let service = TileService::new(registry);

        // Quality 0 should be rejected
        let request = TileRequest::with_quality("test.tif", 0, 0, 0, 0);
        let result = service.get_tile(request).await;
        assert!(matches!(
            result,
            Err(TileError::InvalidQuality { quality: 0 })
        ));

        // Quality 255 should be rejected
        let request = TileRequest::with_quality("test.tif", 0, 1, 0, 255);
        let result = service.get_tile(request).await;
        assert!(matches!(
            result,
            Err(TileError::InvalidQuality { quality: 255 })
        ));
    }
}
