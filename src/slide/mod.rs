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
//! use wsi_streamer::slide::SlideReader;
//! use wsi_streamer::format::{SvsReader, GenericTiffReader, detect_format, SlideFormat};
//!
//! async fn open_slide<R: RangeReader>(reader: &R) -> Result<Box<dyn SlideReader>, Error> {
//!     match detect_format(reader).await? {
//!         SlideFormat::AperioSvs => {
//!             let svs = SvsReader::open(reader).await?;
//!             Ok(Box::new(svs))
//!         }
//!         SlideFormat::GenericTiff => {
//!             let tiff = GenericTiffReader::open(reader).await?;
//!             Ok(Box::new(tiff))
//!         }
//!         SlideFormat::Unknown => {
//!             Err(Error::UnsupportedFormat)
//!         }
//!     }
//! }
//! ```

mod reader;

pub use reader::{LevelInfo, SlideReader};
