//! Tile service layer.
//!
//! This module provides tile generation and caching functionality for serving
//! Whole Slide Image tiles over HTTP.
//!
//! # Architecture
//!
//! The tile service sits between the HTTP layer and the slide abstraction:
//!
//! ```text
//! ┌─────────────────────────────────────────┐
//! │              HTTP Handlers              │
//! └────────────────────┬────────────────────┘
//!                      │
//!                      ▼
//! ┌─────────────────────────────────────────┐
//! │              Tile Service               │
//! │  ┌──────────────┐  ┌─────────────────┐  │
//! │  │  TileCache   │  │  JPEG Encoder   │  │
//! │  │  (encoded    │  │  (decode →      │  │
//! │  │   JPEGs)     │  │   encode)       │  │
//! │  └──────────────┘  └─────────────────┘  │
//! └────────────────────┬────────────────────┘
//!                      │
//!                      ▼
//! ┌─────────────────────────────────────────┐
//! │            SlideRegistry                │
//! └─────────────────────────────────────────┘
//! ```
//!
//! # Components
//!
//! - [`TileService`]: Main entry point for tile requests, orchestrates the full pipeline
//! - [`TileCache`]: LRU cache for encoded JPEG tiles with size-based eviction
//! - [`TileCacheKey`]: Composite key for tile identification (slide, level, coords, quality)
//! - [`JpegTileEncoder`]: Decodes source JPEG and re-encodes at requested quality
//! - [`TileRequest`]: Parameters for a tile request
//! - [`TileResponse`]: Response containing tile data and metadata
//!
//! # Example
//!
//! ```
//! use wsi_streamer::tile::{TileCache, TileCacheKey};
//! use bytes::Bytes;
//!
//! #[tokio::main]
//! async fn main() {
//!     // Create a cache with 50MB capacity
//!     let cache = TileCache::with_capacity(50 * 1024 * 1024);
//!
//!     // Create a cache key
//!     let key = TileCacheKey::new("slides/sample.svs", 0, 1, 2, 80);
//!
//!     // Check cache before generating tile
//!     if let Some(cached_tile) = cache.get(&key).await {
//!         // Use cached tile
//!         println!("Cache hit: {} bytes", cached_tile.len());
//!     } else {
//!         // Generate tile and cache it
//!         let tile_data = Bytes::from(vec![/* JPEG data */]);
//!         cache.put(key, tile_data).await;
//!     }
//! }
//! ```

mod cache;
mod encoder;
mod service;

pub use cache::{TileCache, TileCacheKey, DEFAULT_TILE_CACHE_CAPACITY};
pub use encoder::{
    clamp_quality, is_valid_quality, JpegTileEncoder, DEFAULT_JPEG_QUALITY, MAX_JPEG_QUALITY,
    MIN_JPEG_QUALITY,
};
pub use service::{TileRequest, TileResponse, TileService};
