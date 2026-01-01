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
//! use wsi_streamer::slide::{SlideRegistry, S3SlideSource};
//! use wsi_streamer::io::create_s3_client;
//!
//! // Create S3 client and source
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

mod reader;
mod registry;
mod s3_source;

pub use reader::{LevelInfo, SlideReader};
pub use registry::{CachedSlide, SlideListResult, SlideRegistry, SlideSource};
pub use s3_source::S3SlideSource;
