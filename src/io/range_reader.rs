use async_trait::async_trait;
use bytes::Bytes;

use crate::error::IoError;

/// Trait for reading byte ranges from a remote resource.
///
/// This abstraction allows the TIFF parser and rest of the system to work
/// with files without downloading them entirely. Implementations must be
/// thread-safe and cloneable.
#[async_trait]
pub trait RangeReader: Send + Sync {
    /// Read exactly `len` bytes starting at `offset`.
    ///
    /// Returns an error if the range is out of bounds or if the read fails.
    async fn read_exact_at(&self, offset: u64, len: usize) -> Result<Bytes, IoError>;

    /// Get the total size of the resource in bytes.
    fn size(&self) -> u64;

    /// Get a unique identifier for this resource (for logging and cache keys).
    ///
    /// For S3, this would typically be `s3://bucket/key`.
    fn identifier(&self) -> &str;
}

// =============================================================================
// Endian Helper Functions
// =============================================================================
//
// TIFF files can be either little-endian or big-endian, determined by the
// magic bytes at the start of the file. These helpers are used extensively
// by the TIFF parser.

/// Read a little-endian u16 from a byte slice.
///
/// # Panics
/// Panics if the slice has fewer than 2 bytes.
#[inline]
pub fn read_u16_le(bytes: &[u8]) -> u16 {
    u16::from_le_bytes([bytes[0], bytes[1]])
}

/// Read a big-endian u16 from a byte slice.
///
/// # Panics
/// Panics if the slice has fewer than 2 bytes.
#[inline]
pub fn read_u16_be(bytes: &[u8]) -> u16 {
    u16::from_be_bytes([bytes[0], bytes[1]])
}

/// Read a little-endian u32 from a byte slice.
///
/// # Panics
/// Panics if the slice has fewer than 4 bytes.
#[inline]
pub fn read_u32_le(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// Read a big-endian u32 from a byte slice.
///
/// # Panics
/// Panics if the slice has fewer than 4 bytes.
#[inline]
pub fn read_u32_be(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// Read a little-endian u64 from a byte slice.
///
/// # Panics
/// Panics if the slice has fewer than 8 bytes.
#[inline]
pub fn read_u64_le(bytes: &[u8]) -> u64 {
    u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}

/// Read a big-endian u64 from a byte slice.
///
/// # Panics
/// Panics if the slice has fewer than 8 bytes.
#[inline]
pub fn read_u64_be(bytes: &[u8]) -> u64 {
    u64::from_be_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_u16_le() {
        // 0x0102 in little-endian is stored as [0x02, 0x01]
        assert_eq!(read_u16_le(&[0x02, 0x01]), 0x0102);
        assert_eq!(read_u16_le(&[0x00, 0x00]), 0x0000);
        assert_eq!(read_u16_le(&[0xFF, 0xFF]), 0xFFFF);
    }

    #[test]
    fn test_read_u16_be() {
        // 0x0102 in big-endian is stored as [0x01, 0x02]
        assert_eq!(read_u16_be(&[0x01, 0x02]), 0x0102);
        assert_eq!(read_u16_be(&[0x00, 0x00]), 0x0000);
        assert_eq!(read_u16_be(&[0xFF, 0xFF]), 0xFFFF);
    }

    #[test]
    fn test_read_u32_le() {
        // 0x01020304 in little-endian is stored as [0x04, 0x03, 0x02, 0x01]
        assert_eq!(read_u32_le(&[0x04, 0x03, 0x02, 0x01]), 0x01020304);
        assert_eq!(read_u32_le(&[0x00, 0x00, 0x00, 0x00]), 0x00000000);
        assert_eq!(read_u32_le(&[0xFF, 0xFF, 0xFF, 0xFF]), 0xFFFFFFFF);
    }

    #[test]
    fn test_read_u32_be() {
        // 0x01020304 in big-endian is stored as [0x01, 0x02, 0x03, 0x04]
        assert_eq!(read_u32_be(&[0x01, 0x02, 0x03, 0x04]), 0x01020304);
        assert_eq!(read_u32_be(&[0x00, 0x00, 0x00, 0x00]), 0x00000000);
        assert_eq!(read_u32_be(&[0xFF, 0xFF, 0xFF, 0xFF]), 0xFFFFFFFF);
    }

    #[test]
    fn test_read_u64_le() {
        // 0x0102030405060708 in little-endian
        assert_eq!(
            read_u64_le(&[0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]),
            0x0102030405060708
        );
    }

    #[test]
    fn test_read_u64_be() {
        // 0x0102030405060708 in big-endian
        assert_eq!(
            read_u64_be(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]),
            0x0102030405060708
        );
    }
}
