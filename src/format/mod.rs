//! Format parsers for Whole Slide Image files.
//!
//! This module provides parsers for WSI formats, starting with TIFF-based formats
//! which are the foundation for SVS and generic pyramidal TIFF files.
//!
//! # Format Detection
//!
//! Use [`detect::detect_format`] to automatically identify the format of a slide file.
//! Currently supported formats:
//!
//! - **Aperio SVS**: Identified by "Aperio" marker in ImageDescription
//! - **Generic Pyramidal TIFF**: Standard tiled TIFF with pyramid structure
//!
//! # Reading Slides
//!
//! - Use [`svs::SvsReader`] for Aperio SVS files
//! - Use [`generic_tiff::GenericTiffReader`] for standard pyramidal TIFF files
//! - Both readers handle JPEGTables merging automatically when needed

pub mod detect;
pub mod generic_tiff;
pub mod jpeg;
pub mod svs;
pub mod tiff;

pub use detect::{detect_format, is_tiff_header, SlideFormat};
pub use generic_tiff::{GenericTiffLevelData, GenericTiffReader};
pub use jpeg::{is_abbreviated_stream, is_complete_stream, merge_jpeg_tables, prepare_tile_jpeg};
pub use svs::{SvsLevelData, SvsMetadata, SvsReader};
