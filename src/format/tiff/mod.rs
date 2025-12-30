//! TIFF parser for Whole Slide Images.
//!
//! This module handles parsing of TIFF and BigTIFF files, which are the foundation
//! for most WSI formats including Aperio SVS and generic pyramidal TIFF.
//!
//! # Key Concepts
//!
//! - **Byte order**: TIFF files declare their endianness (II = little-endian, MM = big-endian)
//!   in the header. All multi-byte values must be read respecting this order.
//!
//! - **Classic TIFF vs BigTIFF**: Classic TIFF uses 32-bit offsets (max 4GB files),
//!   while BigTIFF uses 64-bit offsets. The parser handles both transparently.
//!
//! - **IFD (Image File Directory)**: Contains metadata and pointers to image data.
//!   WSI files typically have multiple IFDs for pyramid levels, labels, and macros.
//!
//! - **Inline vs offset values**: Small values are stored inline in the IFD entry,
//!   larger values are stored at an offset pointed to by the entry.

mod parser;
mod pyramid;
mod tags;
mod values;

pub use parser::{ByteOrder, Ifd, IfdEntry, TiffHeader, BIGTIFF_HEADER_SIZE, TIFF_HEADER_SIZE};
pub use pyramid::{PyramidLevel, TiffPyramid, TileData};
pub use tags::{Compression, FieldType, TiffTag};
pub use values::{ValueReader, parse_u32_array, parse_u64_array};
