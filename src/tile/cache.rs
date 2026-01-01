//! Tile cache for encoded JPEG tiles.
//!
//! This module provides an LRU cache for encoded tiles, preventing repeated
//! decode/encode cycles for frequently accessed tiles.
//!
//! # Cache Key
//!
//! Tiles are cached by a composite key including:
//! - Slide identifier (path or ID)
//! - Pyramid level
//! - Tile X coordinate
//! - Tile Y coordinate
//! - JPEG quality setting
//!
//! # Size-Based Eviction
//!
//! The cache tracks the total size of cached tiles in bytes and evicts
//! least-recently-used entries when the capacity is exceeded.

use std::sync::Arc;

use bytes::Bytes;
use lru::LruCache;
use tokio::sync::RwLock;

/// Default cache capacity: 100MB
pub const DEFAULT_TILE_CACHE_CAPACITY: usize = 100 * 1024 * 1024;

/// Default maximum number of entries (to bound LRU overhead)
const DEFAULT_MAX_ENTRIES: usize = 10_000;

// =============================================================================
// Cache Key
// =============================================================================

/// Cache key for encoded tiles.
///
/// This key uniquely identifies a tile at a specific quality level.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TileCacheKey {
    /// Slide identifier (typically the S3 path or slide ID)
    pub slide_id: Arc<str>,

    /// Pyramid level (0 = highest resolution)
    pub level: u32,

    /// Tile X coordinate (0-indexed from left)
    pub tile_x: u32,

    /// Tile Y coordinate (0-indexed from top)
    pub tile_y: u32,

    /// JPEG quality (1-100)
    pub quality: u8,
}

impl TileCacheKey {
    /// Create a new cache key.
    pub fn new(
        slide_id: impl Into<Arc<str>>,
        level: u32,
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
// Tile Cache
// =============================================================================

/// LRU cache for encoded JPEG tiles with size-based capacity.
///
/// This cache stores encoded tile data and evicts least-recently-used entries
/// when the total cached size exceeds capacity.
///
/// # Thread Safety
///
/// The cache is thread-safe and can be shared across async tasks via `Arc`.
///
/// # Example
///
/// ```
/// use wsi_streamer::tile::{TileCache, TileCacheKey};
/// use bytes::Bytes;
/// use std::sync::Arc;
///
/// #[tokio::main]
/// async fn main() {
///     let cache = TileCache::new();
///
///     let key = TileCacheKey::new("slides/sample.svs", 0, 1, 2, 80);
///     let tile_data = Bytes::from(vec![0xFF, 0xD8, 0xFF, 0xE0]); // JPEG header
///
///     // Store tile
///     cache.put(key.clone(), tile_data.clone()).await;
///
///     // Retrieve tile
///     let cached = cache.get(&key).await;
///     assert_eq!(cached, Some(tile_data));
/// }
/// ```
pub struct TileCache {
    /// The underlying LRU cache
    cache: RwLock<LruCache<TileCacheKey, Bytes>>,

    /// Maximum total size in bytes
    max_size: usize,

    /// Current total size in bytes
    current_size: RwLock<usize>,
}

impl TileCache {
    /// Create a new tile cache with default capacity (100MB).
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_TILE_CACHE_CAPACITY)
    }

    /// Create a new tile cache with the specified capacity in bytes.
    ///
    /// # Arguments
    ///
    /// * `max_size` - Maximum total size of cached tiles in bytes
    pub fn with_capacity(max_size: usize) -> Self {
        Self {
            cache: RwLock::new(LruCache::new(
                std::num::NonZeroUsize::new(DEFAULT_MAX_ENTRIES).unwrap(),
            )),
            max_size,
            current_size: RwLock::new(0),
        }
    }

    /// Create a new tile cache with specified capacity and maximum entries.
    ///
    /// # Arguments
    ///
    /// * `max_size` - Maximum total size of cached tiles in bytes
    /// * `max_entries` - Maximum number of entries in the cache
    pub fn with_capacity_and_entries(max_size: usize, max_entries: usize) -> Self {
        Self {
            cache: RwLock::new(LruCache::new(
                std::num::NonZeroUsize::new(max_entries).unwrap(),
            )),
            max_size,
            current_size: RwLock::new(0),
        }
    }

    /// Get a tile from the cache.
    ///
    /// Returns `Some(data)` if the tile is cached, `None` otherwise.
    /// This operation marks the entry as recently used.
    pub async fn get(&self, key: &TileCacheKey) -> Option<Bytes> {
        let mut cache = self.cache.write().await;
        cache.get(key).cloned()
    }

    /// Check if a tile is in the cache without updating LRU order.
    ///
    /// Returns `true` if the tile is cached, `false` otherwise.
    pub async fn contains(&self, key: &TileCacheKey) -> bool {
        let cache = self.cache.read().await;
        cache.contains(key)
    }

    /// Store a tile in the cache.
    ///
    /// If the cache is over capacity after insertion, least-recently-used
    /// entries are evicted until the cache is within capacity.
    ///
    /// If the tile already exists, it is updated and marked as recently used.
    pub async fn put(&self, key: TileCacheKey, data: Bytes) {
        let data_size = data.len();
        let mut cache = self.cache.write().await;
        let mut current_size = self.current_size.write().await;

        // If key exists, subtract old size first
        if let Some(old_data) = cache.peek(&key) {
            *current_size = current_size.saturating_sub(old_data.len());
        }

        // Insert the new data
        cache.put(key, data);
        *current_size += data_size;

        // Evict entries until we're under capacity
        while *current_size > self.max_size {
            if let Some((_, evicted_data)) = cache.pop_lru() {
                *current_size = current_size.saturating_sub(evicted_data.len());
            } else {
                // Cache is empty, nothing more to evict
                break;
            }
        }
    }

    /// Remove a tile from the cache.
    ///
    /// Returns the cached data if it existed, `None` otherwise.
    pub async fn remove(&self, key: &TileCacheKey) -> Option<Bytes> {
        let mut cache = self.cache.write().await;
        let mut current_size = self.current_size.write().await;

        if let Some(data) = cache.pop(key) {
            *current_size = current_size.saturating_sub(data.len());
            Some(data)
        } else {
            None
        }
    }

    /// Clear all entries from the cache.
    pub async fn clear(&self) {
        let mut cache = self.cache.write().await;
        let mut current_size = self.current_size.write().await;
        cache.clear();
        *current_size = 0;
    }

    /// Get the current number of cached tiles.
    pub async fn len(&self) -> usize {
        let cache = self.cache.read().await;
        cache.len()
    }

    /// Check if the cache is empty.
    pub async fn is_empty(&self) -> bool {
        let cache = self.cache.read().await;
        cache.is_empty()
    }

    /// Get the current total size of cached tiles in bytes.
    pub async fn size(&self) -> usize {
        let current_size = self.current_size.read().await;
        *current_size
    }

    /// Get the maximum capacity in bytes.
    pub fn capacity(&self) -> usize {
        self.max_size
    }
}

impl Default for TileCache {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_key(slide: &str, level: u32, x: u32, y: u32, quality: u8) -> TileCacheKey {
        TileCacheKey::new(slide, level, x, y, quality)
    }

    fn make_tile(size: usize) -> Bytes {
        Bytes::from(vec![0u8; size])
    }

    #[tokio::test]
    async fn test_basic_get_put() {
        let cache = TileCache::new();

        let key = make_key("slide.svs", 0, 1, 2, 80);
        let data = make_tile(1000);

        assert!(cache.get(&key).await.is_none());

        cache.put(key.clone(), data.clone()).await;

        let retrieved = cache.get(&key).await;
        assert_eq!(retrieved, Some(data));
    }

    #[tokio::test]
    async fn test_contains() {
        let cache = TileCache::new();

        let key = make_key("slide.svs", 0, 0, 0, 80);
        assert!(!cache.contains(&key).await);

        cache.put(key.clone(), make_tile(100)).await;
        assert!(cache.contains(&key).await);
    }

    #[tokio::test]
    async fn test_different_quality_different_key() {
        let cache = TileCache::new();

        let key_q80 = make_key("slide.svs", 0, 0, 0, 80);
        let key_q90 = make_key("slide.svs", 0, 0, 0, 90);

        let data_q80 = Bytes::from(vec![80u8; 100]);
        let data_q90 = Bytes::from(vec![90u8; 100]);

        cache.put(key_q80.clone(), data_q80.clone()).await;
        cache.put(key_q90.clone(), data_q90.clone()).await;

        assert_eq!(cache.get(&key_q80).await, Some(data_q80));
        assert_eq!(cache.get(&key_q90).await, Some(data_q90));
    }

    #[tokio::test]
    async fn test_size_tracking() {
        let cache = TileCache::with_capacity(10_000);

        assert_eq!(cache.size().await, 0);

        cache.put(make_key("a", 0, 0, 0, 80), make_tile(1000)).await;
        assert_eq!(cache.size().await, 1000);

        cache.put(make_key("b", 0, 0, 0, 80), make_tile(2000)).await;
        assert_eq!(cache.size().await, 3000);
    }

    #[tokio::test]
    async fn test_size_based_eviction() {
        // Cache with 1000 byte capacity
        let cache = TileCache::with_capacity_and_entries(1000, 100);

        // Add tiles totaling 800 bytes
        cache.put(make_key("a", 0, 0, 0, 80), make_tile(400)).await;
        cache.put(make_key("b", 0, 0, 0, 80), make_tile(400)).await;

        assert_eq!(cache.len().await, 2);
        assert_eq!(cache.size().await, 800);

        // Add another tile that pushes us over capacity
        cache.put(make_key("c", 0, 0, 0, 80), make_tile(400)).await;

        // LRU entry ("a") should be evicted
        assert!(cache.size().await <= 1000);
        assert!(!cache.contains(&make_key("a", 0, 0, 0, 80)).await);
        assert!(cache.contains(&make_key("b", 0, 0, 0, 80)).await);
        assert!(cache.contains(&make_key("c", 0, 0, 0, 80)).await);
    }

    #[tokio::test]
    async fn test_update_existing_entry() {
        let cache = TileCache::with_capacity(10_000);

        let key = make_key("slide.svs", 0, 0, 0, 80);

        cache.put(key.clone(), make_tile(1000)).await;
        assert_eq!(cache.size().await, 1000);

        // Update with different size
        cache.put(key.clone(), make_tile(500)).await;
        assert_eq!(cache.size().await, 500);
        assert_eq!(cache.len().await, 1);
    }

    #[tokio::test]
    async fn test_remove() {
        let cache = TileCache::with_capacity(10_000);

        let key = make_key("slide.svs", 0, 0, 0, 80);
        let data = make_tile(1000);

        cache.put(key.clone(), data.clone()).await;
        assert_eq!(cache.size().await, 1000);

        let removed = cache.remove(&key).await;
        assert_eq!(removed, Some(data));
        assert_eq!(cache.size().await, 0);
        assert!(cache.is_empty().await);
    }

    #[tokio::test]
    async fn test_clear() {
        let cache = TileCache::with_capacity(10_000);

        cache.put(make_key("a", 0, 0, 0, 80), make_tile(1000)).await;
        cache.put(make_key("b", 0, 0, 0, 80), make_tile(2000)).await;

        assert_eq!(cache.len().await, 2);
        assert_eq!(cache.size().await, 3000);

        cache.clear().await;

        assert!(cache.is_empty().await);
        assert_eq!(cache.size().await, 0);
    }

    #[tokio::test]
    async fn test_lru_order() {
        // Small cache: 1500 bytes capacity
        let cache = TileCache::with_capacity_and_entries(1500, 100);

        // Add three tiles of 500 bytes each (total 1500)
        cache.put(make_key("a", 0, 0, 0, 80), make_tile(500)).await;
        cache.put(make_key("b", 0, 0, 0, 80), make_tile(500)).await;
        cache.put(make_key("c", 0, 0, 0, 80), make_tile(500)).await;

        // Access "a" to make it recently used
        cache.get(&make_key("a", 0, 0, 0, 80)).await;

        // Add new tile, should evict "b" (LRU)
        cache.put(make_key("d", 0, 0, 0, 80), make_tile(500)).await;

        assert!(cache.contains(&make_key("a", 0, 0, 0, 80)).await); // Recently accessed
        assert!(!cache.contains(&make_key("b", 0, 0, 0, 80)).await); // Evicted (LRU)
        assert!(cache.contains(&make_key("c", 0, 0, 0, 80)).await);
        assert!(cache.contains(&make_key("d", 0, 0, 0, 80)).await);
    }

    #[tokio::test]
    async fn test_different_slides_same_coords() {
        let cache = TileCache::new();

        let key1 = make_key("slide1.svs", 0, 0, 0, 80);
        let key2 = make_key("slide2.svs", 0, 0, 0, 80);

        let data1 = Bytes::from(vec![1u8; 100]);
        let data2 = Bytes::from(vec![2u8; 100]);

        cache.put(key1.clone(), data1.clone()).await;
        cache.put(key2.clone(), data2.clone()).await;

        assert_eq!(cache.get(&key1).await, Some(data1));
        assert_eq!(cache.get(&key2).await, Some(data2));
        assert_eq!(cache.len().await, 2);
    }

    #[tokio::test]
    async fn test_capacity() {
        let cache = TileCache::with_capacity(50_000);
        assert_eq!(cache.capacity(), 50_000);
    }

    #[test]
    fn test_cache_key_equality() {
        let key1 = make_key("slide.svs", 0, 1, 2, 80);
        let key2 = make_key("slide.svs", 0, 1, 2, 80);
        let key3 = make_key("slide.svs", 0, 1, 2, 90);

        assert_eq!(key1, key2);
        assert_ne!(key1, key3);
    }

    #[test]
    fn test_cache_key_hash() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        fn hash<T: Hash>(t: &T) -> u64 {
            let mut s = DefaultHasher::new();
            t.hash(&mut s);
            s.finish()
        }

        let key1 = make_key("slide.svs", 0, 1, 2, 80);
        let key2 = make_key("slide.svs", 0, 1, 2, 80);

        assert_eq!(hash(&key1), hash(&key2));
    }
}
