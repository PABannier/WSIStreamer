//! Slide Registry for managing slide lifecycle and caching.
//!
//! The registry provides:
//! - LRU caching of opened slide readers to avoid re-parsing metadata
//! - Singleflight pattern to prevent duplicate opens for the same slide
//! - Format auto-detection when opening slides
//! - Block caching for efficient I/O
//!
//! # Example
//!
//! ```ignore
//! use wsi_streamer::slide::{SlideRegistry, S3SlideSource};
//! use wsi_streamer::io::create_s3_client;
//!
//! // Create S3 source
//! let client = create_s3_client(None).await;
//! let source = S3SlideSource::new(client, "my-bucket".to_string());
//!
//! // Create registry
//! let registry = SlideRegistry::new(source);
//!
//! // Get a slide (opens and caches on first access)
//! let slide = registry.get_slide("path/to/slide.svs").await?;
//!
//! // Read a tile
//! let tile = slide.read_tile(0, 0, 0).await?;
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use lru::LruCache;
use tokio::sync::{Mutex, Notify, RwLock};

use crate::error::{FormatError, IoError, TiffError};
use crate::format::{detect_format, GenericTiffReader, SlideFormat, SvsReader};
use crate::io::{BlockCache, RangeReader, DEFAULT_BLOCK_SIZE};

use super::reader::{LevelInfo, SlideReader};

// =============================================================================
// Configuration
// =============================================================================

/// Default capacity for slide cache (number of slides).
const DEFAULT_SLIDE_CACHE_CAPACITY: usize = 100;

/// Default capacity for block cache per slide (number of blocks).
const DEFAULT_BLOCK_CACHE_CAPACITY: usize = 100;

// =============================================================================
// SlideSource Trait
// =============================================================================

/// Trait for creating range readers from slide identifiers.
///
/// This abstraction allows the registry to work with different storage backends
/// (S3, local files, etc.) without being tied to a specific implementation.
#[async_trait]
pub trait SlideSource: Send + Sync {
    /// The type of range reader this source creates.
    type Reader: RangeReader + 'static;

    /// Create a range reader for the given slide identifier.
    ///
    /// # Arguments
    /// * `slide_id` - Unique identifier for the slide (e.g., S3 key)
    ///
    /// # Returns
    /// A range reader for accessing the slide's bytes.
    async fn create_reader(&self, slide_id: &str) -> Result<Self::Reader, IoError>;
}

// =============================================================================
// CachedSlide
// =============================================================================

/// A slide that has been opened and cached.
///
/// This holds both the parsed slide structure and the underlying reader
/// (wrapped in a BlockCache for efficient I/O).
pub struct CachedSlide<R: RangeReader + 'static> {
    /// The detected format of this slide
    format: SlideFormat,

    /// The underlying reader with block caching
    reader: Arc<BlockCache<R>>,

    /// The slide reader (either SVS or generic TIFF)
    inner: SlideReaderInner,
}

/// Internal enum to hold format-specific readers.
///
/// We use an enum instead of trait objects because `SlideReader::read_tile`
/// is generic over the reader type, making the trait not object-safe.
enum SlideReaderInner {
    Svs(SvsReader),
    GenericTiff(GenericTiffReader),
}

impl<R: RangeReader + 'static> CachedSlide<R> {
    /// Get the detected format of this slide.
    pub fn format(&self) -> SlideFormat {
        self.format
    }

    /// Get the number of pyramid levels.
    pub fn level_count(&self) -> usize {
        match &self.inner {
            SlideReaderInner::Svs(r) => r.level_count(),
            SlideReaderInner::GenericTiff(r) => r.level_count(),
        }
    }

    /// Get dimensions of the full-resolution (level 0) image.
    pub fn dimensions(&self) -> Option<(u32, u32)> {
        match &self.inner {
            SlideReaderInner::Svs(r) => r.dimensions(),
            SlideReaderInner::GenericTiff(r) => r.dimensions(),
        }
    }

    /// Get dimensions of a specific level.
    pub fn level_dimensions(&self, level: usize) -> Option<(u32, u32)> {
        match &self.inner {
            SlideReaderInner::Svs(r) => r.level_dimensions(level),
            SlideReaderInner::GenericTiff(r) => r.level_dimensions(level),
        }
    }

    /// Get the downsample factor for a level.
    pub fn level_downsample(&self, level: usize) -> Option<f64> {
        match &self.inner {
            SlideReaderInner::Svs(r) => r.level_downsample(level),
            SlideReaderInner::GenericTiff(r) => r.level_downsample(level),
        }
    }

    /// Get tile size for a level.
    pub fn tile_size(&self, level: usize) -> Option<(u32, u32)> {
        match &self.inner {
            SlideReaderInner::Svs(r) => r.tile_size(level),
            SlideReaderInner::GenericTiff(r) => r.tile_size(level),
        }
    }

    /// Get the number of tiles in X and Y directions for a level.
    pub fn tile_count(&self, level: usize) -> Option<(u32, u32)> {
        match &self.inner {
            SlideReaderInner::Svs(r) => r.tile_count(level),
            SlideReaderInner::GenericTiff(r) => r.tile_count(level),
        }
    }

    /// Get complete information about a level.
    pub fn level_info(&self, level: usize) -> Option<LevelInfo> {
        match &self.inner {
            SlideReaderInner::Svs(r) => r.level_info(level),
            SlideReaderInner::GenericTiff(r) => r.level_info(level),
        }
    }

    /// Find the best level for a given downsample factor.
    pub fn best_level_for_downsample(&self, downsample: f64) -> Option<usize> {
        match &self.inner {
            SlideReaderInner::Svs(r) => SlideReader::best_level_for_downsample(r, downsample),
            SlideReaderInner::GenericTiff(r) => {
                SlideReader::best_level_for_downsample(r, downsample)
            }
        }
    }

    /// Read a tile and prepare it for JPEG decoding.
    ///
    /// # Arguments
    /// * `level` - Pyramid level index (0 = highest resolution)
    /// * `tile_x` - Tile X coordinate (0-indexed from left)
    /// * `tile_y` - Tile Y coordinate (0-indexed from top)
    ///
    /// # Returns
    /// Complete JPEG data ready for decoding.
    pub async fn read_tile(
        &self,
        level: usize,
        tile_x: u32,
        tile_y: u32,
    ) -> Result<Bytes, TiffError> {
        match &self.inner {
            SlideReaderInner::Svs(r) => {
                r.read_tile(self.reader.as_ref(), level, tile_x, tile_y)
                    .await
            }
            SlideReaderInner::GenericTiff(r) => {
                r.read_tile(self.reader.as_ref(), level, tile_x, tile_y)
                    .await
            }
        }
    }
}

// =============================================================================
// SlideRegistry
// =============================================================================

/// Registry for managing slide lifecycle and caching.
///
/// The registry:
/// - Caches opened slide readers with LRU eviction
/// - Creates readers on-demand with format auto-detection
/// - Wraps readers in BlockCache for efficient I/O
/// - Uses singleflight to prevent duplicate opens for the same slide
pub struct SlideRegistry<S: SlideSource> {
    /// The source for creating range readers
    source: S,

    /// Cached slides indexed by slide ID
    cache: RwLock<LruCache<String, Arc<CachedSlide<S::Reader>>>>,

    /// In-flight opens for singleflight pattern
    in_flight: Mutex<HashMap<String, Arc<InFlightState<S::Reader>>>>,

    /// Block size for BlockCache
    block_size: usize,

    /// Block cache capacity per slide
    block_cache_capacity: usize,
}

/// State for an in-flight slide open operation.
struct InFlightState<R: RangeReader + 'static> {
    /// Notification for waiters
    notify: Notify,
    /// Result of the open operation (set when complete)
    result: Mutex<Option<Result<Arc<CachedSlide<R>>, FormatError>>>,
}

impl<S: SlideSource> SlideRegistry<S> {
    /// Create a new SlideRegistry with default settings.
    ///
    /// Uses default cache capacities:
    /// - Slide cache: 100 slides
    /// - Block cache per slide: 100 blocks (25.6 MB per slide)
    pub fn new(source: S) -> Self {
        Self::with_capacity(
            source,
            DEFAULT_SLIDE_CACHE_CAPACITY,
            DEFAULT_BLOCK_SIZE,
            DEFAULT_BLOCK_CACHE_CAPACITY,
        )
    }

    /// Create a new SlideRegistry with custom capacity settings.
    ///
    /// # Arguments
    /// * `source` - The slide source for creating readers
    /// * `slide_cache_capacity` - Maximum number of slides to cache
    /// * `block_size` - Block size for the block cache (bytes)
    /// * `block_cache_capacity` - Number of blocks to cache per slide
    pub fn with_capacity(
        source: S,
        slide_cache_capacity: usize,
        block_size: usize,
        block_cache_capacity: usize,
    ) -> Self {
        Self {
            source,
            cache: RwLock::new(LruCache::new(
                std::num::NonZeroUsize::new(slide_cache_capacity).unwrap(),
            )),
            in_flight: Mutex::new(HashMap::new()),
            block_size,
            block_cache_capacity,
        }
    }

    /// Get a slide, opening it if not already cached.
    ///
    /// This method:
    /// 1. Checks the cache for an existing slide
    /// 2. If not cached, opens the slide with format auto-detection
    /// 3. Uses singleflight to prevent duplicate opens for concurrent requests
    ///
    /// # Arguments
    /// * `slide_id` - Unique identifier for the slide
    ///
    /// # Returns
    /// An Arc-wrapped CachedSlide that can be used to read tiles.
    pub async fn get_slide(
        &self,
        slide_id: &str,
    ) -> Result<Arc<CachedSlide<S::Reader>>, FormatError> {
        // Fast path: check cache
        {
            let mut cache = self.cache.write().await;
            if let Some(slide) = cache.get(slide_id) {
                return Ok(slide.clone());
            }
        }

        // Slow path: check in_flight or become leader
        loop {
            let state = {
                let mut in_flight = self.in_flight.lock().await;

                if let Some(state) = in_flight.get(slide_id) {
                    // Another task is opening this slide
                    state.clone()
                } else {
                    // We're the leader for opening this slide
                    let state = Arc::new(InFlightState {
                        notify: Notify::new(),
                        result: Mutex::new(None),
                    });
                    in_flight.insert(slide_id.to_string(), state.clone());
                    drop(in_flight);

                    // Perform the open
                    let result = self.open_slide_internal(slide_id).await;

                    // Store result and update cache
                    {
                        let mut result_guard = state.result.lock().await;
                        *result_guard = Some(result.clone());
                    }

                    if let Ok(ref slide) = result {
                        let mut cache = self.cache.write().await;
                        cache.put(slide_id.to_string(), slide.clone());
                    }

                    // Clean up in_flight and notify waiters
                    {
                        let mut in_flight = self.in_flight.lock().await;
                        in_flight.remove(slide_id);
                    }
                    state.notify.notify_waiters();

                    return result;
                }
            };

            // Wait for the leader to finish
            state.notify.notified().await;

            // Check if result is available
            let result_guard = state.result.lock().await;
            if let Some(ref result) = *result_guard {
                return result.clone();
            }

            // Result not yet available, loop back (shouldn't normally happen)
        }
    }

    /// Open a slide without caching (internal implementation).
    async fn open_slide_internal(
        &self,
        slide_id: &str,
    ) -> Result<Arc<CachedSlide<S::Reader>>, FormatError> {
        // Create the underlying reader
        let reader = self.source.create_reader(slide_id).await?;

        // Wrap in block cache
        let cached_reader = Arc::new(BlockCache::with_capacity(
            reader,
            self.block_size,
            self.block_cache_capacity,
        ));

        // Detect format
        let format = detect_format(cached_reader.as_ref()).await?;

        // Open the appropriate reader
        let inner = match format {
            SlideFormat::AperioSvs => {
                let svs = SvsReader::open(cached_reader.as_ref()).await?;
                SlideReaderInner::Svs(svs)
            }
            SlideFormat::GenericTiff => {
                let tiff = GenericTiffReader::open(cached_reader.as_ref()).await?;
                SlideReaderInner::GenericTiff(tiff)
            }
        };

        Ok(Arc::new(CachedSlide {
            format,
            reader: cached_reader,
            inner,
        }))
    }

    /// Remove a slide from the cache.
    ///
    /// This can be useful for forcing a reload of a slide's metadata.
    pub async fn invalidate(&self, slide_id: &str) {
        let mut cache = self.cache.write().await;
        cache.pop(slide_id);
    }

    /// Clear all cached slides.
    pub async fn clear(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
    }

    /// Get the number of cached slides.
    pub async fn cached_count(&self) -> usize {
        let cache = self.cache.read().await;
        cache.len()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Mock slide source for testing
    struct MockSlideSource {
        /// Number of times create_reader was called
        create_count: AtomicUsize,
        /// Data to return
        data: Bytes,
    }

    impl MockSlideSource {
        fn new(data: Vec<u8>) -> Self {
            Self {
                create_count: AtomicUsize::new(0),
                data: Bytes::from(data),
            }
        }

        fn create_count(&self) -> usize {
            self.create_count.load(Ordering::SeqCst)
        }
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

    #[async_trait]
    impl SlideSource for MockSlideSource {
        type Reader = MockReader;

        async fn create_reader(&self, slide_id: &str) -> Result<Self::Reader, IoError> {
            self.create_count.fetch_add(1, Ordering::SeqCst);
            Ok(MockReader {
                data: self.data.clone(),
                identifier: format!("mock://{}", slide_id),
            })
        }
    }

    /// Create a minimal valid TIFF file for testing
    ///
    /// The TIFF must have dimensions > 1000x1000 to avoid being classified
    /// as a label image by the pyramid detection logic.
    fn create_minimal_tiff() -> Vec<u8> {
        let mut data = vec![0u8; 8192];

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

        // ImageWidth (2048) - tag 256, type LONG (4), count 1, value 2048
        // Using LONG type to accommodate larger values
        write_entry(&mut data, &mut offset, 256, 4, 1, 2048);

        // ImageLength (1536) - tag 257, type LONG (4), count 1, value 1536
        write_entry(&mut data, &mut offset, 257, 4, 1, 1536);

        // Compression (7 = JPEG) - tag 259, type SHORT (3), count 1, value 7
        write_entry(&mut data, &mut offset, 259, 3, 1, 7);

        // TileWidth (256) - tag 322, type SHORT (3), count 1, value 256
        write_entry(&mut data, &mut offset, 322, 3, 1, 256);

        // TileLength (256) - tag 323, type SHORT (3), count 1, value 256
        write_entry(&mut data, &mut offset, 323, 3, 1, 256);

        // TileOffsets - tag 324, type LONG (4), count 48 (8x6 tiles), value at offset 200
        // 2048/256 = 8 tiles in X, 1536/256 = 6 tiles in Y = 48 tiles
        write_entry(&mut data, &mut offset, 324, 4, 48, 200);

        // TileByteCounts - tag 325, type LONG (4), count 48, value at offset 400
        write_entry(&mut data, &mut offset, 325, 4, 48, 400);

        // BitsPerSample - tag 258, type SHORT (3), count 1, value 8
        write_entry(&mut data, &mut offset, 258, 3, 1, 8);

        // Next IFD offset (0 = no more IFDs)
        data[offset..offset + 4].copy_from_slice(&0u32.to_le_bytes());

        // Write tile offsets array at offset 200 (48 LONG values)
        // Each tile starts at offset 1000 + (index * 100)
        for i in 0..48u32 {
            let tile_offset = 1000 + i * 100;
            let arr_offset = 200 + (i as usize) * 4;
            data[arr_offset..arr_offset + 4].copy_from_slice(&tile_offset.to_le_bytes());
        }

        // Write tile byte counts array at offset 400 (48 LONG values)
        // Each tile is 90 bytes
        for i in 0..48u32 {
            let arr_offset = 400 + (i as usize) * 4;
            data[arr_offset..arr_offset + 4].copy_from_slice(&90u32.to_le_bytes());
        }

        // Put some JPEG-like data for each tile (starting at offset 1000)
        for i in 0..48 {
            let tile_start = 1000 + i * 100;
            data[tile_start] = 0xFF;
            data[tile_start + 1] = 0xD8; // SOI
            data[tile_start + 2] = 0xFF;
            data[tile_start + 3] = 0xDB; // DQT marker (indicates complete JPEG)
            data[tile_start + 88] = 0xFF;
            data[tile_start + 89] = 0xD9; // EOI
        }

        data
    }

    #[tokio::test]
    async fn test_registry_caches_slides() {
        let tiff_data = create_minimal_tiff();
        let source = MockSlideSource::new(tiff_data);
        let registry = SlideRegistry::with_capacity(source, 10, 256, 10);

        // First access should open the slide
        let result = registry.get_slide("test.tif").await;
        assert!(result.is_ok());
        assert_eq!(registry.source.create_count(), 1);

        // Second access should hit cache
        let result2 = registry.get_slide("test.tif").await;
        assert!(result2.is_ok());
        assert_eq!(registry.source.create_count(), 1); // Still 1

        // Different slide should create new reader
        let result3 = registry.get_slide("test2.tif").await;
        assert!(result3.is_ok());
        assert_eq!(registry.source.create_count(), 2);
    }

    #[tokio::test]
    async fn test_registry_cache_eviction() {
        let tiff_data = create_minimal_tiff();
        let source = MockSlideSource::new(tiff_data);
        // Cache capacity of 2
        let registry = SlideRegistry::with_capacity(source, 2, 256, 10);

        // Open 3 slides (cache can only hold 2)
        registry.get_slide("slide1.tif").await.unwrap();
        registry.get_slide("slide2.tif").await.unwrap();
        registry.get_slide("slide3.tif").await.unwrap();

        assert_eq!(registry.source.create_count(), 3);
        assert_eq!(registry.cached_count().await, 2);

        // Access slide1 again - should be evicted, need to reopen
        registry.get_slide("slide1.tif").await.unwrap();
        assert_eq!(registry.source.create_count(), 4);
    }

    #[tokio::test]
    async fn test_registry_invalidate() {
        let tiff_data = create_minimal_tiff();
        let source = MockSlideSource::new(tiff_data);
        let registry = SlideRegistry::new(source);

        // Open slide
        registry.get_slide("test.tif").await.unwrap();
        assert_eq!(registry.source.create_count(), 1);

        // Invalidate
        registry.invalidate("test.tif").await;
        assert_eq!(registry.cached_count().await, 0);

        // Reopen should create new reader
        registry.get_slide("test.tif").await.unwrap();
        assert_eq!(registry.source.create_count(), 2);
    }

    #[tokio::test]
    async fn test_registry_clear() {
        let tiff_data = create_minimal_tiff();
        let source = MockSlideSource::new(tiff_data);
        let registry = SlideRegistry::new(source);

        // Open multiple slides
        registry.get_slide("slide1.tif").await.unwrap();
        registry.get_slide("slide2.tif").await.unwrap();
        assert_eq!(registry.cached_count().await, 2);

        // Clear cache
        registry.clear().await;
        assert_eq!(registry.cached_count().await, 0);
    }

    #[tokio::test]
    async fn test_cached_slide_metadata() {
        let tiff_data = create_minimal_tiff();
        let source = MockSlideSource::new(tiff_data);
        let registry = SlideRegistry::new(source);

        let slide = registry.get_slide("test.tif").await.unwrap();

        // Check metadata access
        assert_eq!(slide.format(), SlideFormat::GenericTiff);
        assert_eq!(slide.level_count(), 1);
        assert_eq!(slide.dimensions(), Some((2048, 1536)));
        assert_eq!(slide.tile_size(0), Some((256, 256)));
        assert_eq!(slide.tile_count(0), Some((8, 6)));
    }

    #[tokio::test]
    async fn test_concurrent_opens_singleflight() {
        use std::sync::atomic::AtomicBool;
        use tokio::time::{sleep, Duration};

        /// Slow source that takes time to create readers
        struct SlowMockSource {
            data: Bytes,
            create_count: AtomicUsize,
            is_creating: AtomicBool,
        }

        impl SlowMockSource {
            fn new(data: Vec<u8>) -> Self {
                Self {
                    data: Bytes::from(data),
                    create_count: AtomicUsize::new(0),
                    is_creating: AtomicBool::new(false),
                }
            }
        }

        #[async_trait]
        impl SlideSource for SlowMockSource {
            type Reader = MockReader;

            async fn create_reader(&self, slide_id: &str) -> Result<Self::Reader, IoError> {
                // Check if another create is in progress
                let was_creating = self.is_creating.swap(true, Ordering::SeqCst);
                assert!(
                    !was_creating,
                    "Concurrent creates detected - singleflight failed!"
                );

                self.create_count.fetch_add(1, Ordering::SeqCst);
                sleep(Duration::from_millis(50)).await;

                self.is_creating.store(false, Ordering::SeqCst);

                Ok(MockReader {
                    data: self.data.clone(),
                    identifier: format!("mock://{}", slide_id),
                })
            }
        }

        let tiff_data = create_minimal_tiff();
        let source = SlowMockSource::new(tiff_data);
        let registry = Arc::new(SlideRegistry::new(source));

        // Spawn multiple concurrent requests for the same slide
        let mut handles = Vec::new();
        for _ in 0..5 {
            let registry = registry.clone();
            handles.push(tokio::spawn(
                async move { registry.get_slide("test.tif").await },
            ));
        }

        // Wait for all to complete
        for handle in handles {
            let result = handle.await.unwrap();
            assert!(result.is_ok());
        }

        // Should have only created one reader due to singleflight
        assert_eq!(registry.source.create_count.load(Ordering::SeqCst), 1);
    }
}
