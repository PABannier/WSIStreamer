//! Slide abstraction layer.
//!
//! This module provides a unified interface for working with Whole Slide Images
//! regardless of their underlying format.
//!
//! # Architecture
//!
//! The slide abstraction layer sits between the format-specific parsers and the
//! tile service:
//!
//! ```text
//! ┌─────────────────────────────────────────┐
//! │              Tile Service               │
//! └────────────────────┬────────────────────┘
//!                      │
//!                      ▼
//! ┌─────────────────────────────────────────┐
//! │            SlideRegistry                │
//! │  (caches slides, auto-detects format)   │
//! └────────────────────┬────────────────────┘
//!                      │
//!                      ▼
//! ┌─────────────────────────────────────────┐
//! │           SlideReader Trait             │
//! │  (format-agnostic slide interface)      │
//! └────────────────────┬────────────────────┘
//!                      │
//!          ┌───────────┴───────────┐
//!          ▼                       ▼
//! ┌─────────────────┐    ┌─────────────────────┐
//! │   SvsReader     │    │ GenericTiffReader   │
//! │  (SVS format)   │    │ (standard TIFF)     │
//! └─────────────────┘    └─────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```ignore
//! use wsi_streamer::slide::{SlideRegistry, SlideSource};
//! use wsi_streamer::io::RangeReader;
//!
//! // Create a source that can open slides
//! struct MySlideSource { /* ... */ }
//!
//! impl SlideSource for MySlideSource {
//!     type Reader = MyReader;
//!     async fn create_reader(&self, slide_id: &str) -> Result<Self::Reader, IoError> {
//!         // Create a reader for the given slide ID
//!     }
//! }
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

mod reader;
mod registry;

pub use reader::{LevelInfo, SlideReader};
pub use registry::{CachedSlide, SlideRegistry, SlideSource};
