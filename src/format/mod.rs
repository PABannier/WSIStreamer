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

pub mod detect;
pub mod tiff;

pub use detect::{detect_format, is_tiff_header, SlideFormat};
