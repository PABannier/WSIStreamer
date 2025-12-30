//! TIFF header and structure parsing.
//!
//! This module handles parsing of TIFF and BigTIFF file headers,
//! which is the foundation for all subsequent parsing operations.
//!
//! # TIFF Header Structure
//!
//! ## Classic TIFF (8 bytes)
//! ```text
//! Bytes 0-1: Byte order (0x4949 = little-endian "II", 0x4D4D = big-endian "MM")
//! Bytes 2-3: Version (42 = 0x002A)
//! Bytes 4-7: Offset to first IFD (4 bytes)
//! ```
//!
//! ## BigTIFF (16 bytes)
//! ```text
//! Bytes 0-1: Byte order (0x4949 = little-endian "II", 0x4D4D = big-endian "MM")
//! Bytes 2-3: Version (43 = 0x002B)
//! Bytes 4-5: Offset byte size (must be 8)
//! Bytes 6-7: Reserved (must be 0)
//! Bytes 8-15: Offset to first IFD (8 bytes)
//! ```

use crate::error::TiffError;
use crate::io::{read_u16_be, read_u16_le, read_u32_be, read_u32_le, read_u64_be, read_u64_le};

// =============================================================================
// Constants
// =============================================================================

/// Magic bytes indicating little-endian byte order ("II" for Intel)
const BYTE_ORDER_LITTLE_ENDIAN: u16 = 0x4949;

/// Magic bytes indicating big-endian byte order ("MM" for Motorola)
const BYTE_ORDER_BIG_ENDIAN: u16 = 0x4D4D;

/// Version number for classic TIFF
const VERSION_TIFF: u16 = 42;

/// Version number for BigTIFF
const VERSION_BIGTIFF: u16 = 43;

/// Size of classic TIFF header in bytes
pub const TIFF_HEADER_SIZE: usize = 8;

/// Size of BigTIFF header in bytes
pub const BIGTIFF_HEADER_SIZE: usize = 16;

// =============================================================================
// ByteOrder
// =============================================================================

/// Byte order (endianness) of a TIFF file.
///
/// TIFF files declare their byte order in the first two bytes of the header.
/// All multi-byte values in the file must be read respecting this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteOrder {
    /// Little-endian ("II" = Intel)
    LittleEndian,
    /// Big-endian ("MM" = Motorola)
    BigEndian,
}

impl ByteOrder {
    /// Read a u16 from a byte slice using this byte order.
    #[inline]
    pub fn read_u16(self, bytes: &[u8]) -> u16 {
        match self {
            ByteOrder::LittleEndian => read_u16_le(bytes),
            ByteOrder::BigEndian => read_u16_be(bytes),
        }
    }

    /// Read a u32 from a byte slice using this byte order.
    #[inline]
    pub fn read_u32(self, bytes: &[u8]) -> u32 {
        match self {
            ByteOrder::LittleEndian => read_u32_le(bytes),
            ByteOrder::BigEndian => read_u32_be(bytes),
        }
    }

    /// Read a u64 from a byte slice using this byte order.
    #[inline]
    pub fn read_u64(self, bytes: &[u8]) -> u64 {
        match self {
            ByteOrder::LittleEndian => read_u64_le(bytes),
            ByteOrder::BigEndian => read_u64_be(bytes),
        }
    }
}

// =============================================================================
// TiffHeader
// =============================================================================

/// Parsed TIFF file header.
///
/// Contains the essential information needed to begin parsing IFDs:
/// - Byte order for reading all subsequent values
/// - Whether this is classic TIFF or BigTIFF (affects entry sizes and offset widths)
/// - Location of the first IFD
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TiffHeader {
    /// Byte order for all multi-byte values in the file
    pub byte_order: ByteOrder,

    /// Whether this is a BigTIFF file (64-bit offsets)
    pub is_bigtiff: bool,

    /// Offset to the first IFD in the file
    pub first_ifd_offset: u64,
}

impl TiffHeader {
    /// Parse a TIFF header from raw bytes.
    ///
    /// The input must contain at least 8 bytes for classic TIFF or 16 bytes for BigTIFF.
    /// The function first reads enough to determine the format, then validates the rest.
    ///
    /// # Arguments
    /// * `bytes` - Raw header bytes (at least 8 bytes, preferably 16 for BigTIFF support)
    /// * `file_size` - Total file size (used to validate IFD offset)
    ///
    /// # Errors
    /// - `InvalidMagic` if byte order bytes are not II or MM
    /// - `InvalidVersion` if version is not 42 or 43
    /// - `InvalidBigTiffOffsetSize` if BigTIFF offset size is not 8
    /// - `FileTooSmall` if there aren't enough bytes for the header
    /// - `InvalidIfdOffset` if the first IFD offset is outside the file
    pub fn parse(bytes: &[u8], file_size: u64) -> Result<Self, TiffError> {
        // Need at least 8 bytes to read the basic header
        if bytes.len() < TIFF_HEADER_SIZE {
            return Err(TiffError::FileTooSmall {
                required: TIFF_HEADER_SIZE as u64,
                actual: bytes.len() as u64,
            });
        }

        // Read byte order (bytes 0-1)
        // We read this as little-endian because we're checking for specific byte patterns
        let magic = u16::from_le_bytes([bytes[0], bytes[1]]);
        let byte_order = match magic {
            BYTE_ORDER_LITTLE_ENDIAN => ByteOrder::LittleEndian,
            BYTE_ORDER_BIG_ENDIAN => ByteOrder::BigEndian,
            _ => return Err(TiffError::InvalidMagic(magic)),
        };

        // Read version (bytes 2-3) using the detected byte order
        let version = byte_order.read_u16(&bytes[2..4]);

        match version {
            VERSION_TIFF => {
                // Classic TIFF: 4-byte offset at bytes 4-7
                let first_ifd_offset = byte_order.read_u32(&bytes[4..8]) as u64;

                // Validate offset
                if first_ifd_offset >= file_size {
                    return Err(TiffError::InvalidIfdOffset(first_ifd_offset));
                }

                Ok(TiffHeader {
                    byte_order,
                    is_bigtiff: false,
                    first_ifd_offset,
                })
            }
            VERSION_BIGTIFF => {
                // BigTIFF: need 16 bytes total
                if bytes.len() < BIGTIFF_HEADER_SIZE {
                    return Err(TiffError::FileTooSmall {
                        required: BIGTIFF_HEADER_SIZE as u64,
                        actual: bytes.len() as u64,
                    });
                }

                // Bytes 4-5: offset byte size (must be 8)
                let offset_size = byte_order.read_u16(&bytes[4..6]);
                if offset_size != 8 {
                    return Err(TiffError::InvalidBigTiffOffsetSize(offset_size));
                }

                // Bytes 6-7: reserved (should be 0, but we don't strictly require it)

                // Bytes 8-15: first IFD offset (8 bytes)
                let first_ifd_offset = byte_order.read_u64(&bytes[8..16]);

                // Validate offset
                if first_ifd_offset >= file_size {
                    return Err(TiffError::InvalidIfdOffset(first_ifd_offset));
                }

                Ok(TiffHeader {
                    byte_order,
                    is_bigtiff: true,
                    first_ifd_offset,
                })
            }
            _ => Err(TiffError::InvalidVersion(version)),
        }
    }

    /// Size of an IFD entry in bytes.
    ///
    /// Classic TIFF: 12 bytes (2 tag + 2 type + 4 count + 4 value/offset)
    /// BigTIFF: 20 bytes (2 tag + 2 type + 8 count + 8 value/offset)
    #[inline]
    pub const fn ifd_entry_size(&self) -> usize {
        if self.is_bigtiff {
            20
        } else {
            12
        }
    }

    /// Size of the entry count field at the start of an IFD.
    ///
    /// Classic TIFF: 2 bytes (u16)
    /// BigTIFF: 8 bytes (u64)
    #[inline]
    pub const fn ifd_count_size(&self) -> usize {
        if self.is_bigtiff {
            8
        } else {
            2
        }
    }

    /// Size of the next IFD offset field at the end of an IFD.
    ///
    /// Classic TIFF: 4 bytes (u32)
    /// BigTIFF: 8 bytes (u64)
    #[inline]
    pub const fn ifd_next_offset_size(&self) -> usize {
        if self.is_bigtiff {
            8
        } else {
            4
        }
    }

    /// Size of the value/offset field in an IFD entry.
    ///
    /// This determines the inline value threshold:
    /// Classic TIFF: 4 bytes
    /// BigTIFF: 8 bytes
    #[inline]
    pub const fn value_offset_size(&self) -> usize {
        if self.is_bigtiff {
            8
        } else {
            4
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // ByteOrder Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_byte_order_read_u16() {
        let bytes = [0x01, 0x02];
        assert_eq!(ByteOrder::LittleEndian.read_u16(&bytes), 0x0201);
        assert_eq!(ByteOrder::BigEndian.read_u16(&bytes), 0x0102);
    }

    #[test]
    fn test_byte_order_read_u32() {
        let bytes = [0x01, 0x02, 0x03, 0x04];
        assert_eq!(ByteOrder::LittleEndian.read_u32(&bytes), 0x04030201);
        assert_eq!(ByteOrder::BigEndian.read_u32(&bytes), 0x01020304);
    }

    #[test]
    fn test_byte_order_read_u64() {
        let bytes = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        assert_eq!(
            ByteOrder::LittleEndian.read_u64(&bytes),
            0x0807060504030201
        );
        assert_eq!(ByteOrder::BigEndian.read_u64(&bytes), 0x0102030405060708);
    }

    // -------------------------------------------------------------------------
    // TiffHeader Parsing Tests - Classic TIFF
    // -------------------------------------------------------------------------

    #[test]
    fn test_parse_tiff_little_endian() {
        // Little-endian TIFF with first IFD at offset 8
        let header = [
            0x49, 0x49, // II (little-endian)
            0x2A, 0x00, // Version 42 (little-endian)
            0x08, 0x00, 0x00, 0x00, // First IFD offset = 8 (little-endian)
        ];

        let result = TiffHeader::parse(&header, 1000).unwrap();
        assert_eq!(result.byte_order, ByteOrder::LittleEndian);
        assert!(!result.is_bigtiff);
        assert_eq!(result.first_ifd_offset, 8);
    }

    #[test]
    fn test_parse_tiff_big_endian() {
        // Big-endian TIFF with first IFD at offset 8
        let header = [
            0x4D, 0x4D, // MM (big-endian)
            0x00, 0x2A, // Version 42 (big-endian)
            0x00, 0x00, 0x00, 0x08, // First IFD offset = 8 (big-endian)
        ];

        let result = TiffHeader::parse(&header, 1000).unwrap();
        assert_eq!(result.byte_order, ByteOrder::BigEndian);
        assert!(!result.is_bigtiff);
        assert_eq!(result.first_ifd_offset, 8);
    }

    #[test]
    fn test_parse_tiff_larger_offset() {
        // Little-endian TIFF with first IFD at offset 1000
        let header = [
            0x49, 0x49, // II (little-endian)
            0x2A, 0x00, // Version 42
            0xE8, 0x03, 0x00, 0x00, // First IFD offset = 1000 (little-endian)
        ];

        let result = TiffHeader::parse(&header, 2000).unwrap();
        assert_eq!(result.first_ifd_offset, 1000);
    }

    // -------------------------------------------------------------------------
    // TiffHeader Parsing Tests - BigTIFF
    // -------------------------------------------------------------------------

    #[test]
    fn test_parse_bigtiff_little_endian() {
        // Little-endian BigTIFF with first IFD at offset 16
        let header = [
            0x49, 0x49, // II (little-endian)
            0x2B, 0x00, // Version 43 (BigTIFF)
            0x08, 0x00, // Offset size = 8
            0x00, 0x00, // Reserved
            0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // First IFD offset = 16
        ];

        let result = TiffHeader::parse(&header, 1000).unwrap();
        assert_eq!(result.byte_order, ByteOrder::LittleEndian);
        assert!(result.is_bigtiff);
        assert_eq!(result.first_ifd_offset, 16);
    }

    #[test]
    fn test_parse_bigtiff_big_endian() {
        // Big-endian BigTIFF with first IFD at offset 16
        let header = [
            0x4D, 0x4D, // MM (big-endian)
            0x00, 0x2B, // Version 43 (BigTIFF)
            0x00, 0x08, // Offset size = 8
            0x00, 0x00, // Reserved
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, // First IFD offset = 16
        ];

        let result = TiffHeader::parse(&header, 1000).unwrap();
        assert_eq!(result.byte_order, ByteOrder::BigEndian);
        assert!(result.is_bigtiff);
        assert_eq!(result.first_ifd_offset, 16);
    }

    #[test]
    fn test_parse_bigtiff_large_offset() {
        // BigTIFF with 64-bit offset beyond 4GB
        let header = [
            0x49, 0x49, // II (little-endian)
            0x2B, 0x00, // Version 43 (BigTIFF)
            0x08, 0x00, // Offset size = 8
            0x00, 0x00, // Reserved
            0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, // First IFD offset = 4GB
        ];

        let result = TiffHeader::parse(&header, 10_000_000_000).unwrap();
        assert!(result.is_bigtiff);
        assert_eq!(result.first_ifd_offset, 0x0000_0001_0000_0000); // 4GB
    }

    // -------------------------------------------------------------------------
    // TiffHeader Parsing Tests - Error Cases
    // -------------------------------------------------------------------------

    #[test]
    fn test_parse_invalid_magic() {
        let header = [
            0x00, 0x00, // Invalid magic
            0x2A, 0x00, 0x08, 0x00, 0x00, 0x00,
        ];

        let result = TiffHeader::parse(&header, 1000);
        assert!(matches!(result, Err(TiffError::InvalidMagic(0x0000))));
    }

    #[test]
    fn test_parse_invalid_version() {
        let header = [
            0x49, 0x49, // II
            0x00, 0x00, // Invalid version 0
            0x08, 0x00, 0x00, 0x00,
        ];

        let result = TiffHeader::parse(&header, 1000);
        assert!(matches!(result, Err(TiffError::InvalidVersion(0))));
    }

    #[test]
    fn test_parse_bigtiff_invalid_offset_size() {
        let header = [
            0x49, 0x49, // II
            0x2B, 0x00, // Version 43 (BigTIFF)
            0x04, 0x00, // Invalid offset size = 4 (should be 8)
            0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];

        let result = TiffHeader::parse(&header, 1000);
        assert!(matches!(
            result,
            Err(TiffError::InvalidBigTiffOffsetSize(4))
        ));
    }

    #[test]
    fn test_parse_file_too_small_tiff() {
        let header = [0x49, 0x49, 0x2A, 0x00]; // Only 4 bytes

        let result = TiffHeader::parse(&header, 1000);
        assert!(matches!(
            result,
            Err(TiffError::FileTooSmall {
                required: 8,
                actual: 4
            })
        ));
    }

    #[test]
    fn test_parse_file_too_small_bigtiff() {
        // Valid TIFF header but BigTIFF needs 16 bytes
        let header = [
            0x49, 0x49, // II
            0x2B, 0x00, // Version 43 (BigTIFF)
            0x08, 0x00, // Offset size = 8
            0x00, 0x00, // Only 8 bytes total
        ];

        let result = TiffHeader::parse(&header, 1000);
        assert!(matches!(
            result,
            Err(TiffError::FileTooSmall {
                required: 16,
                actual: 8
            })
        ));
    }

    #[test]
    fn test_parse_invalid_ifd_offset() {
        // IFD offset beyond file size
        let header = [
            0x49, 0x49, // II
            0x2A, 0x00, // Version 42
            0xE8, 0x03, 0x00, 0x00, // First IFD offset = 1000
        ];

        let result = TiffHeader::parse(&header, 500); // File is only 500 bytes
        assert!(matches!(result, Err(TiffError::InvalidIfdOffset(1000))));
    }

    // -------------------------------------------------------------------------
    // TiffHeader Helper Methods Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_ifd_entry_size() {
        let tiff = TiffHeader {
            byte_order: ByteOrder::LittleEndian,
            is_bigtiff: false,
            first_ifd_offset: 8,
        };
        assert_eq!(tiff.ifd_entry_size(), 12);

        let bigtiff = TiffHeader {
            byte_order: ByteOrder::LittleEndian,
            is_bigtiff: true,
            first_ifd_offset: 16,
        };
        assert_eq!(bigtiff.ifd_entry_size(), 20);
    }

    #[test]
    fn test_ifd_count_size() {
        let tiff = TiffHeader {
            byte_order: ByteOrder::LittleEndian,
            is_bigtiff: false,
            first_ifd_offset: 8,
        };
        assert_eq!(tiff.ifd_count_size(), 2);

        let bigtiff = TiffHeader {
            byte_order: ByteOrder::LittleEndian,
            is_bigtiff: true,
            first_ifd_offset: 16,
        };
        assert_eq!(bigtiff.ifd_count_size(), 8);
    }

    #[test]
    fn test_value_offset_size() {
        let tiff = TiffHeader {
            byte_order: ByteOrder::LittleEndian,
            is_bigtiff: false,
            first_ifd_offset: 8,
        };
        assert_eq!(tiff.value_offset_size(), 4);

        let bigtiff = TiffHeader {
            byte_order: ByteOrder::LittleEndian,
            is_bigtiff: true,
            first_ifd_offset: 16,
        };
        assert_eq!(bigtiff.value_offset_size(), 8);
    }
}
