use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::{Bytes, BytesMut};
use lru::LruCache;
use tokio::sync::{Mutex, Notify, RwLock};

use super::RangeReader;
use crate::error::IoError;

/// Default block size: 256KB
/// This is large enough to amortize S3 latency, small enough to not waste bandwidth.
pub const DEFAULT_BLOCK_SIZE: usize = 256 * 1024;

/// Default cache capacity in number of blocks.
/// 100 blocks * 256KB = 25.6MB default cache size.
const DEFAULT_CACHE_CAPACITY: usize = 100;

/// Block-based caching layer that wraps any RangeReader.
///
/// This cache is critical for performance:
/// - TIFF parsing requires many small reads at scattered offsets
/// - Without caching, each read would be an S3 request
/// - Block cache amortizes these into fewer, larger requests
///
/// Features:
/// - Fixed-size block cache (default 256KB blocks)
/// - LRU eviction when cache reaches capacity
/// - Singleflight: concurrent requests for the same block share one fetch
/// - Handles reads spanning multiple blocks
pub struct BlockCache<R> {
    /// The underlying reader
    inner: Arc<R>,
    /// Block size in bytes
    block_size: usize,
    /// Cached blocks indexed by block number
    cache: RwLock<LruCache<u64, Bytes>>,
    /// In-flight block fetches for singleflight pattern
    in_flight: Mutex<HashMap<u64, Arc<Notify>>>,
}

impl<R: RangeReader> BlockCache<R> {
    /// Create a new BlockCache wrapping the given reader.
    ///
    /// Uses default block size (256KB) and cache capacity (100 blocks).
    pub fn new(inner: R) -> Self {
        Self::with_capacity(inner, DEFAULT_BLOCK_SIZE, DEFAULT_CACHE_CAPACITY)
    }

    /// Create a new BlockCache with custom block size and capacity.
    ///
    /// # Arguments
    /// * `inner` - The underlying reader to wrap
    /// * `block_size` - Size of each cached block in bytes
    /// * `capacity` - Maximum number of blocks to cache
    pub fn with_capacity(inner: R, block_size: usize, capacity: usize) -> Self {
        Self {
            inner: Arc::new(inner),
            block_size,
            cache: RwLock::new(LruCache::new(
                std::num::NonZeroUsize::new(capacity).unwrap(),
            )),
            in_flight: Mutex::new(HashMap::new()),
        }
    }

    /// Get a block from cache or fetch it from the underlying reader.
    ///
    /// Implements the singleflight pattern: if multiple tasks request the same
    /// block concurrently, only one fetch is performed and all tasks share the result.
    async fn get_block(&self, block_idx: u64) -> Result<Bytes, IoError> {
        loop {
            // Fast path: check cache
            {
                let cache = self.cache.read().await;
                if let Some(data) = cache.peek(&block_idx) {
                    return Ok(data.clone());
                }
            }

            // Slow path: check in_flight or become leader
            let notify = {
                let mut in_flight = self.in_flight.lock().await;

                if let Some(notify) = in_flight.get(&block_idx) {
                    // Another task is fetching this block, wait for it
                    let notify = notify.clone();
                    drop(in_flight);
                    notify.notified().await;
                    // Loop back to check cache
                    continue;
                }

                // We're the leader for this block
                let notify = Arc::new(Notify::new());
                in_flight.insert(block_idx, notify.clone());
                notify
            };

            // Fetch the block from source
            let result = self.fetch_block_from_source(block_idx).await;

            // Update cache and in_flight atomically, then notify waiters
            {
                let mut cache = self.cache.write().await;
                let mut in_flight = self.in_flight.lock().await;

                if let Ok(ref data) = result {
                    cache.put(block_idx, data.clone());
                }

                in_flight.remove(&block_idx);
            }

            notify.notify_waiters();

            return result;
        }
    }

    /// Fetch a block directly from the underlying reader.
    async fn fetch_block_from_source(&self, block_idx: u64) -> Result<Bytes, IoError> {
        let offset = block_idx * self.block_size as u64;
        let size = self.inner.size();

        // Calculate actual bytes to read (may be less for last block)
        let remaining = size.saturating_sub(offset);
        if remaining == 0 {
            return Err(IoError::RangeOutOfBounds {
                offset,
                requested: self.block_size as u64,
                size,
            });
        }

        let len = std::cmp::min(self.block_size as u64, remaining) as usize;
        self.inner.read_exact_at(offset, len).await
    }

    /// Calculate which block contains the given offset.
    #[inline]
    fn block_for_offset(&self, offset: u64) -> u64 {
        offset / self.block_size as u64
    }

    /// Calculate the offset within a block.
    #[inline]
    fn offset_within_block(&self, offset: u64) -> usize {
        (offset % self.block_size as u64) as usize
    }
}

#[async_trait]
impl<R: RangeReader + 'static> RangeReader for BlockCache<R> {
    async fn read_exact_at(&self, offset: u64, len: usize) -> Result<Bytes, IoError> {
        // Validate range
        let size = self.inner.size();
        if offset + len as u64 > size {
            return Err(IoError::RangeOutOfBounds {
                offset,
                requested: len as u64,
                size,
            });
        }

        // Handle zero-length reads
        if len == 0 {
            return Ok(Bytes::new());
        }

        // Calculate which blocks we need
        let start_block = self.block_for_offset(offset);
        let end_block = self.block_for_offset(offset + len as u64 - 1);

        if start_block == end_block {
            // Single block read (common case)
            let block = self.get_block(start_block).await?;
            let block_offset = self.offset_within_block(offset);
            Ok(block.slice(block_offset..block_offset + len))
        } else {
            // Multi-block read: fetch all required blocks and combine
            let mut result = BytesMut::with_capacity(len);
            let mut remaining = len;
            let mut current_offset = offset;

            for block_idx in start_block..=end_block {
                let block = self.get_block(block_idx).await?;
                let block_offset = self.offset_within_block(current_offset);
                let bytes_in_block = std::cmp::min(block.len() - block_offset, remaining);

                result.extend_from_slice(&block[block_offset..block_offset + bytes_in_block]);

                remaining -= bytes_in_block;
                current_offset += bytes_in_block as u64;
            }

            Ok(result.freeze())
        }
    }

    fn size(&self) -> u64 {
        self.inner.size()
    }

    fn identifier(&self) -> &str {
        self.inner.identifier()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Mock reader for testing that tracks read calls
    struct MockReader {
        data: Bytes,
        identifier: String,
        read_count: AtomicUsize,
    }

    impl MockReader {
        fn new(data: Vec<u8>) -> Self {
            Self {
                data: Bytes::from(data),
                identifier: "mock://test".to_string(),
                read_count: AtomicUsize::new(0),
            }
        }

        fn read_count(&self) -> usize {
            self.read_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl RangeReader for MockReader {
        async fn read_exact_at(&self, offset: u64, len: usize) -> Result<Bytes, IoError> {
            self.read_count.fetch_add(1, Ordering::SeqCst);

            if offset + len as u64 > self.data.len() as u64 {
                return Err(IoError::RangeOutOfBounds {
                    offset,
                    requested: len as u64,
                    size: self.data.len() as u64,
                });
            }

            Ok(self.data.slice(offset as usize..offset as usize + len))
        }

        fn size(&self) -> u64 {
            self.data.len() as u64
        }

        fn identifier(&self) -> &str {
            &self.identifier
        }
    }

    #[tokio::test]
    async fn test_single_block_read() {
        // Create mock with 1KB of data
        let data: Vec<u8> = (0..1024).map(|i| (i % 256) as u8).collect();
        let mock = MockReader::new(data.clone());

        // Use small 256-byte blocks for testing
        let cache = BlockCache::with_capacity(mock, 256, 10);

        // Read 100 bytes from offset 50
        let result = cache.read_exact_at(50, 100).await.unwrap();
        assert_eq!(result.len(), 100);
        assert_eq!(&result[..], &data[50..150]);

        // Should have made 1 read (fetched block 0)
        assert_eq!(cache.inner.read_count(), 1);

        // Read again from same block - should hit cache
        let result2 = cache.read_exact_at(10, 50).await.unwrap();
        assert_eq!(&result2[..], &data[10..60]);

        // Still just 1 read (cache hit)
        assert_eq!(cache.inner.read_count(), 1);
    }

    #[tokio::test]
    async fn test_multi_block_read() {
        // Create mock with 1KB of data
        let data: Vec<u8> = (0..1024).map(|i| (i % 256) as u8).collect();
        let mock = MockReader::new(data.clone());

        // Use small 256-byte blocks
        let cache = BlockCache::with_capacity(mock, 256, 10);

        // Read 300 bytes starting at offset 100
        // This spans blocks 0 (bytes 0-255) and 1 (bytes 256-511)
        let result = cache.read_exact_at(100, 300).await.unwrap();
        assert_eq!(result.len(), 300);
        assert_eq!(&result[..], &data[100..400]);

        // Should have made 2 reads (blocks 0 and 1)
        assert_eq!(cache.inner.read_count(), 2);
    }

    #[tokio::test]
    async fn test_cache_eviction() {
        let data: Vec<u8> = (0..2048).map(|i| (i % 256) as u8).collect();
        let mock = MockReader::new(data);

        // Small cache that can only hold 2 blocks
        let cache = BlockCache::with_capacity(mock, 256, 2);

        // Read from blocks 0, 1, 2 (will evict block 0)
        cache.read_exact_at(0, 10).await.unwrap(); // Block 0
        cache.read_exact_at(256, 10).await.unwrap(); // Block 1
        cache.read_exact_at(512, 10).await.unwrap(); // Block 2, evicts block 0

        assert_eq!(cache.inner.read_count(), 3);

        // Read block 1 again - should hit cache
        cache.read_exact_at(300, 10).await.unwrap();
        assert_eq!(cache.inner.read_count(), 3);

        // Read block 0 again - cache miss (was evicted)
        cache.read_exact_at(0, 10).await.unwrap();
        assert_eq!(cache.inner.read_count(), 4);
    }

    #[tokio::test]
    async fn test_concurrent_reads_singleflight() {
        use std::sync::atomic::AtomicBool;
        use tokio::time::{sleep, Duration};

        /// Slow mock reader that takes 50ms per read
        struct SlowMockReader {
            data: Bytes,
            read_count: AtomicUsize,
            is_reading: AtomicBool,
        }

        impl SlowMockReader {
            fn new(data: Vec<u8>) -> Self {
                Self {
                    data: Bytes::from(data),
                    read_count: AtomicUsize::new(0),
                    is_reading: AtomicBool::new(false),
                }
            }
        }

        #[async_trait]
        impl RangeReader for SlowMockReader {
            async fn read_exact_at(&self, offset: u64, len: usize) -> Result<Bytes, IoError> {
                // Check if another read is in progress (would indicate singleflight failure)
                let was_reading = self.is_reading.swap(true, Ordering::SeqCst);
                assert!(!was_reading, "Concurrent reads detected - singleflight failed!");

                self.read_count.fetch_add(1, Ordering::SeqCst);
                sleep(Duration::from_millis(50)).await;

                self.is_reading.store(false, Ordering::SeqCst);

                Ok(self.data.slice(offset as usize..offset as usize + len))
            }

            fn size(&self) -> u64 {
                self.data.len() as u64
            }

            fn identifier(&self) -> &str {
                "slow://test"
            }
        }

        let data: Vec<u8> = (0..1024).map(|i| (i % 256) as u8).collect();
        let mock = SlowMockReader::new(data);
        let cache = Arc::new(BlockCache::with_capacity(mock, 256, 10));

        // Spawn 10 concurrent reads for the same block
        let mut handles = Vec::new();
        for _ in 0..10 {
            let cache = cache.clone();
            handles.push(tokio::spawn(async move {
                cache.read_exact_at(50, 100).await.unwrap()
            }));
        }

        // Wait for all reads
        for handle in handles {
            handle.await.unwrap();
        }

        // Should have made only 1 read due to singleflight
        assert_eq!(cache.inner.read_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_out_of_bounds() {
        let data: Vec<u8> = vec![1, 2, 3, 4, 5];
        let mock = MockReader::new(data);
        let cache = BlockCache::with_capacity(mock, 256, 10);

        // Read past end of file
        let result = cache.read_exact_at(3, 10).await;
        assert!(matches!(result, Err(IoError::RangeOutOfBounds { .. })));
    }

    #[tokio::test]
    async fn test_zero_length_read() {
        let data: Vec<u8> = vec![1, 2, 3, 4, 5];
        let mock = MockReader::new(data);
        let cache = BlockCache::with_capacity(mock, 256, 10);

        let result = cache.read_exact_at(0, 0).await.unwrap();
        assert!(result.is_empty());

        // No reads should have been made
        assert_eq!(cache.inner.read_count(), 0);
    }

    #[tokio::test]
    async fn test_last_partial_block() {
        // Data that doesn't fill the last block completely
        let data: Vec<u8> = (0..300).map(|i| (i % 256) as u8).collect();
        let mock = MockReader::new(data.clone());
        let cache = BlockCache::with_capacity(mock, 256, 10);

        // Read from second block (which is partial: only 44 bytes)
        let result = cache.read_exact_at(260, 30).await.unwrap();
        assert_eq!(result.len(), 30);
        assert_eq!(&result[..], &data[260..290]);
    }
}
