//! TIFF tag value reading.
//!
//! This module provides functionality to read tag values from TIFF files.
//! Values can be stored either inline in the IFD entry (for small values)
//! or at an offset in the file (for larger values like arrays).
//!
//! # Performance Considerations
//!
//! For array values (like TileOffsets and TileByteCounts), this module
//! fetches the entire array in a single range request. This is critical
//! for performance when working with remote storage.

use bytes::Bytes;

use crate::error::TiffError;
use crate::io::RangeReader;

use super::parser::{ByteOrder, IfdEntry, TiffHeader};
use super::tags::FieldType;

// =============================================================================
// ValueReader
// =============================================================================

/// Reads tag values from a TIFF file.
///
/// This struct combines a RangeReader with TIFF header information to
/// read values respecting the file's byte order and format.
pub struct ValueReader<'a, R: RangeReader> {
    reader: &'a R,
    header: &'a TiffHeader,
}

impl<'a, R: RangeReader> ValueReader<'a, R> {
    /// Create a new ValueReader.
    pub fn new(reader: &'a R, header: &'a TiffHeader) -> Self {
        Self { reader, header }
    }

    /// Get the byte order from the header.
    #[inline]
    pub fn byte_order(&self) -> ByteOrder {
        self.header.byte_order
    }

    /// Read raw bytes for an IFD entry's value.
    ///
    /// For inline values, returns the bytes from the entry.
    /// For offset values, fetches the bytes from the file.
    pub async fn read_bytes(&self, entry: &IfdEntry) -> Result<Bytes, TiffError> {
        let size = entry
            .value_byte_size()
            .ok_or(TiffError::UnknownFieldType(entry.field_type_raw))?;

        if entry.is_inline {
            // Value is stored inline - extract from entry bytes
            Ok(Bytes::copy_from_slice(
                &entry.value_offset_bytes[..size as usize],
            ))
        } else {
            // Value is at an offset - fetch from file
            let offset = entry.value_offset(self.header.byte_order);
            let bytes = self.reader.read_exact_at(offset, size as usize).await?;
            Ok(bytes)
        }
    }

    /// Read a single u32 value from an entry.
    ///
    /// Handles both Short and Long field types, converting as needed.
    pub async fn read_u32(&self, entry: &IfdEntry) -> Result<u32, TiffError> {
        // Try inline first
        if let Some(value) = entry.inline_u32(self.header.byte_order) {
            return Ok(value);
        }

        // Must fetch from offset
        let field_type = entry
            .field_type
            .ok_or(TiffError::UnknownFieldType(entry.field_type_raw))?;

        if entry.count != 1 {
            return Err(TiffError::InvalidTagValue {
                tag: "unknown",
                message: format!("expected count 1, got {}", entry.count),
            });
        }

        let bytes = self.read_bytes(entry).await?;
        let byte_order = self.header.byte_order;

        match field_type {
            FieldType::Short => Ok(byte_order.read_u16(&bytes) as u32),
            FieldType::Long => Ok(byte_order.read_u32(&bytes)),
            _ => Err(TiffError::InvalidTagValue {
                tag: "unknown",
                message: format!("expected Short or Long, got {:?}", field_type),
            }),
        }
    }

    /// Read a single u64 value from an entry.
    ///
    /// Handles Short, Long, and Long8 field types, converting as needed.
    pub async fn read_u64(&self, entry: &IfdEntry) -> Result<u64, TiffError> {
        // Try inline first
        if let Some(value) = entry.inline_u64(self.header.byte_order) {
            return Ok(value);
        }

        // Must fetch from offset
        let field_type = entry
            .field_type
            .ok_or(TiffError::UnknownFieldType(entry.field_type_raw))?;

        if entry.count != 1 {
            return Err(TiffError::InvalidTagValue {
                tag: "unknown",
                message: format!("expected count 1, got {}", entry.count),
            });
        }

        let bytes = self.read_bytes(entry).await?;
        let byte_order = self.header.byte_order;

        match field_type {
            FieldType::Short => Ok(byte_order.read_u16(&bytes) as u64),
            FieldType::Long => Ok(byte_order.read_u32(&bytes) as u64),
            FieldType::Long8 => Ok(byte_order.read_u64(&bytes)),
            _ => Err(TiffError::InvalidTagValue {
                tag: "unknown",
                message: format!("expected Short, Long, or Long8, got {:?}", field_type),
            }),
        }
    }

    /// Read an array of u64 values from an entry.
    ///
    /// This is the primary method for reading TileOffsets and TileByteCounts.
    /// The entire array is fetched in a single range request for efficiency.
    ///
    /// Handles Short, Long, and Long8 field types, converting all to u64.
    pub async fn read_u64_array(&self, entry: &IfdEntry) -> Result<Vec<u64>, TiffError> {
        let field_type = entry
            .field_type
            .ok_or(TiffError::UnknownFieldType(entry.field_type_raw))?;

        let count = entry.count as usize;
        if count == 0 {
            return Ok(Vec::new());
        }

        let bytes = self.read_bytes(entry).await?;
        let byte_order = self.header.byte_order;

        let mut values = Vec::with_capacity(count);

        match field_type {
            FieldType::Short => {
                for i in 0..count {
                    let offset = i * 2;
                    values.push(byte_order.read_u16(&bytes[offset..]) as u64);
                }
            }
            FieldType::Long => {
                for i in 0..count {
                    let offset = i * 4;
                    values.push(byte_order.read_u32(&bytes[offset..]) as u64);
                }
            }
            FieldType::Long8 => {
                for i in 0..count {
                    let offset = i * 8;
                    values.push(byte_order.read_u64(&bytes[offset..]));
                }
            }
            _ => {
                return Err(TiffError::InvalidTagValue {
                    tag: "unknown",
                    message: format!(
                        "expected Short, Long, or Long8 for array, got {:?}",
                        field_type
                    ),
                });
            }
        }

        Ok(values)
    }

    /// Read an array of u32 values from an entry.
    ///
    /// Similar to read_u64_array but returns u32 values.
    /// Useful for tile dimensions and other 32-bit array values.
    pub async fn read_u32_array(&self, entry: &IfdEntry) -> Result<Vec<u32>, TiffError> {
        let field_type = entry
            .field_type
            .ok_or(TiffError::UnknownFieldType(entry.field_type_raw))?;

        let count = entry.count as usize;
        if count == 0 {
            return Ok(Vec::new());
        }

        let bytes = self.read_bytes(entry).await?;
        let byte_order = self.header.byte_order;

        let mut values = Vec::with_capacity(count);

        match field_type {
            FieldType::Short => {
                for i in 0..count {
                    let offset = i * 2;
                    values.push(byte_order.read_u16(&bytes[offset..]) as u32);
                }
            }
            FieldType::Long => {
                for i in 0..count {
                    let offset = i * 4;
                    values.push(byte_order.read_u32(&bytes[offset..]));
                }
            }
            _ => {
                return Err(TiffError::InvalidTagValue {
                    tag: "unknown",
                    message: format!("expected Short or Long for u32 array, got {:?}", field_type),
                });
            }
        }

        Ok(values)
    }

    /// Read a string value from an entry (ASCII type).
    ///
    /// The string is expected to be null-terminated. The null terminator
    /// is stripped from the result.
    pub async fn read_string(&self, entry: &IfdEntry) -> Result<String, TiffError> {
        let field_type = entry
            .field_type
            .ok_or(TiffError::UnknownFieldType(entry.field_type_raw))?;

        if field_type != FieldType::Ascii {
            return Err(TiffError::InvalidTagValue {
                tag: "unknown",
                message: format!("expected Ascii type for string, got {:?}", field_type),
            });
        }

        let bytes = self.read_bytes(entry).await?;

        // Find null terminator and convert to string
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        let s = String::from_utf8_lossy(&bytes[..end]).into_owned();

        Ok(s)
    }

    /// Read raw bytes from an entry (for UNDEFINED or opaque data).
    ///
    /// This is used for JPEGTables and other binary data.
    pub async fn read_raw_bytes(&self, entry: &IfdEntry) -> Result<Bytes, TiffError> {
        self.read_bytes(entry).await
    }
}

// =============================================================================
// Convenience functions for reading from bytes directly
// =============================================================================

/// Parse an array of u64 values from raw bytes.
///
/// This is useful when you already have the bytes and just need to parse them.
pub fn parse_u64_array(
    bytes: &[u8],
    count: usize,
    field_type: FieldType,
    byte_order: ByteOrder,
) -> Vec<u64> {
    let mut values = Vec::with_capacity(count);

    match field_type {
        FieldType::Short => {
            for i in 0..count {
                let offset = i * 2;
                if offset + 2 <= bytes.len() {
                    values.push(byte_order.read_u16(&bytes[offset..]) as u64);
                }
            }
        }
        FieldType::Long => {
            for i in 0..count {
                let offset = i * 4;
                if offset + 4 <= bytes.len() {
                    values.push(byte_order.read_u32(&bytes[offset..]) as u64);
                }
            }
        }
        FieldType::Long8 => {
            for i in 0..count {
                let offset = i * 8;
                if offset + 8 <= bytes.len() {
                    values.push(byte_order.read_u64(&bytes[offset..]));
                }
            }
        }
        _ => {}
    }

    values
}

/// Parse an array of u32 values from raw bytes.
pub fn parse_u32_array(
    bytes: &[u8],
    count: usize,
    field_type: FieldType,
    byte_order: ByteOrder,
) -> Vec<u32> {
    let mut values = Vec::with_capacity(count);

    match field_type {
        FieldType::Short => {
            for i in 0..count {
                let offset = i * 2;
                if offset + 2 <= bytes.len() {
                    values.push(byte_order.read_u16(&bytes[offset..]) as u32);
                }
            }
        }
        FieldType::Long => {
            for i in 0..count {
                let offset = i * 4;
                if offset + 4 <= bytes.len() {
                    values.push(byte_order.read_u32(&bytes[offset..]));
                }
            }
        }
        _ => {}
    }

    values
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::IoError;
    use async_trait::async_trait;

    /// Mock reader for testing
    struct MockReader {
        data: Vec<u8>,
    }

    impl MockReader {
        fn new(data: Vec<u8>) -> Self {
            Self { data }
        }
    }

    #[async_trait]
    impl RangeReader for MockReader {
        async fn read_exact_at(&self, offset: u64, len: usize) -> Result<Bytes, IoError> {
            let start = offset as usize;
            let end = start + len;
            if end > self.data.len() {
                return Err(IoError::RangeOutOfBounds {
                    offset,
                    requested: len as u64,
                    size: self.data.len() as u64,
                });
            }
            Ok(Bytes::copy_from_slice(&self.data[start..end]))
        }

        fn size(&self) -> u64 {
            self.data.len() as u64
        }

        fn identifier(&self) -> &str {
            "mock://test"
        }
    }

    fn make_tiff_header() -> TiffHeader {
        TiffHeader {
            byte_order: ByteOrder::LittleEndian,
            is_bigtiff: false,
            first_ifd_offset: 8,
        }
    }

    // -------------------------------------------------------------------------
    // parse_u64_array tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_parse_u64_array_short() {
        // Array of 4 SHORT values: 100, 200, 300, 400
        let bytes = [
            0x64, 0x00, // 100
            0xC8, 0x00, // 200
            0x2C, 0x01, // 300
            0x90, 0x01, // 400
        ];

        let result = parse_u64_array(&bytes, 4, FieldType::Short, ByteOrder::LittleEndian);
        assert_eq!(result, vec![100, 200, 300, 400]);
    }

    #[test]
    fn test_parse_u64_array_long() {
        // Array of 3 LONG values: 1000, 2000, 3000
        let bytes = [
            0xE8, 0x03, 0x00, 0x00, // 1000
            0xD0, 0x07, 0x00, 0x00, // 2000
            0xB8, 0x0B, 0x00, 0x00, // 3000
        ];

        let result = parse_u64_array(&bytes, 3, FieldType::Long, ByteOrder::LittleEndian);
        assert_eq!(result, vec![1000, 2000, 3000]);
    }

    #[test]
    fn test_parse_u64_array_long8() {
        // Array of 2 LONG8 values
        let bytes = [
            0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, // 4GB
            0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, // 8GB
        ];

        let result = parse_u64_array(&bytes, 2, FieldType::Long8, ByteOrder::LittleEndian);
        assert_eq!(result, vec![0x0000_0001_0000_0000, 0x0000_0002_0000_0000]);
    }

    #[test]
    fn test_parse_u64_array_big_endian() {
        // Big-endian LONG values
        let bytes = [
            0x00, 0x00, 0x03, 0xE8, // 1000
            0x00, 0x00, 0x07, 0xD0, // 2000
        ];

        let result = parse_u64_array(&bytes, 2, FieldType::Long, ByteOrder::BigEndian);
        assert_eq!(result, vec![1000, 2000]);
    }

    #[test]
    fn test_parse_u32_array() {
        let bytes = [
            0x00, 0x01, // 256 (SHORT)
            0x00, 0x02, // 512
        ];

        let result = parse_u32_array(&bytes, 2, FieldType::Short, ByteOrder::LittleEndian);
        assert_eq!(result, vec![256, 512]);
    }

    // -------------------------------------------------------------------------
    // ValueReader async tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_value_reader_read_bytes_inline() {
        let reader = MockReader::new(vec![0; 100]);
        let header = make_tiff_header();
        let value_reader = ValueReader::new(&reader, &header);

        // Create an inline entry (count=1, SHORT type = 2 bytes, fits in 4-byte field)
        let entry = IfdEntry {
            tag_id: 256,
            field_type: Some(FieldType::Short),
            field_type_raw: 3,
            count: 1,
            value_offset_bytes: vec![0x00, 0x04, 0x00, 0x00], // 1024
            is_inline: true,
        };

        let bytes = value_reader.read_bytes(&entry).await.unwrap();
        assert_eq!(bytes.len(), 2);
        assert_eq!(&bytes[..], &[0x00, 0x04]);
    }

    #[tokio::test]
    async fn test_value_reader_read_bytes_offset() {
        // File with data at offset 50
        let mut data = vec![0u8; 100];
        data[50] = 0xAB;
        data[51] = 0xCD;
        data[52] = 0xEF;
        data[53] = 0x12;

        let reader = MockReader::new(data);
        let header = make_tiff_header();
        let value_reader = ValueReader::new(&reader, &header);

        // Entry pointing to offset 50, LONG type, count 1
        let entry = IfdEntry {
            tag_id: 256,
            field_type: Some(FieldType::Long),
            field_type_raw: 4,
            count: 1,
            value_offset_bytes: vec![0x32, 0x00, 0x00, 0x00], // offset 50
            is_inline: false,
        };

        let bytes = value_reader.read_bytes(&entry).await.unwrap();
        assert_eq!(bytes.len(), 4);
        assert_eq!(&bytes[..], &[0xAB, 0xCD, 0xEF, 0x12]);
    }

    #[tokio::test]
    async fn test_value_reader_read_u64_array() {
        // File with tile offsets at offset 100
        let mut data = vec![0u8; 200];
        // Write 5 LONG values at offset 100
        let offsets: [u32; 5] = [1000, 2000, 3000, 4000, 5000];
        for (i, &val) in offsets.iter().enumerate() {
            let bytes = val.to_le_bytes();
            let pos = 100 + i * 4;
            data[pos..pos + 4].copy_from_slice(&bytes);
        }

        let reader = MockReader::new(data);
        let header = make_tiff_header();
        let value_reader = ValueReader::new(&reader, &header);

        // TileOffsets entry: 5 LONG values at offset 100
        let entry = IfdEntry {
            tag_id: 324, // TileOffsets
            field_type: Some(FieldType::Long),
            field_type_raw: 4,
            count: 5,
            value_offset_bytes: vec![0x64, 0x00, 0x00, 0x00], // offset 100
            is_inline: false,
        };

        let result = value_reader.read_u64_array(&entry).await.unwrap();
        assert_eq!(result, vec![1000, 2000, 3000, 4000, 5000]);
    }

    #[tokio::test]
    async fn test_value_reader_read_string() {
        // File with ImageDescription at offset 20
        let mut data = vec![0u8; 100];
        let desc = b"Aperio Image\0";
        data[20..20 + desc.len()].copy_from_slice(desc);

        let reader = MockReader::new(data);
        let header = make_tiff_header();
        let value_reader = ValueReader::new(&reader, &header);

        // ImageDescription entry
        let entry = IfdEntry {
            tag_id: 270, // ImageDescription
            field_type: Some(FieldType::Ascii),
            field_type_raw: 2,
            count: desc.len() as u64,
            value_offset_bytes: vec![0x14, 0x00, 0x00, 0x00], // offset 20
            is_inline: false,
        };

        let result = value_reader.read_string(&entry).await.unwrap();
        assert_eq!(result, "Aperio Image");
    }

    #[tokio::test]
    async fn test_value_reader_read_raw_bytes() {
        // File with JPEGTables at offset 30
        let mut data = vec![0u8; 100];
        // JPEG tables typically start with FFD8 and end with FFD9
        data[30] = 0xFF;
        data[31] = 0xD8;
        data[32] = 0xFF;
        data[33] = 0xDB;
        data[34] = 0xFF;
        data[35] = 0xD9;

        let reader = MockReader::new(data);
        let header = make_tiff_header();
        let value_reader = ValueReader::new(&reader, &header);

        // JPEGTables entry (UNDEFINED type)
        let entry = IfdEntry {
            tag_id: 347, // JPEGTables
            field_type: Some(FieldType::Undefined),
            field_type_raw: 7,
            count: 6,
            value_offset_bytes: vec![0x1E, 0x00, 0x00, 0x00], // offset 30
            is_inline: false,
        };

        let result = value_reader.read_raw_bytes(&entry).await.unwrap();
        assert_eq!(result.len(), 6);
        assert_eq!(&result[..], &[0xFF, 0xD8, 0xFF, 0xDB, 0xFF, 0xD9]);
    }

    #[tokio::test]
    async fn test_value_reader_inline_u32() {
        let reader = MockReader::new(vec![0; 100]);
        let header = make_tiff_header();
        let value_reader = ValueReader::new(&reader, &header);

        // Inline LONG value
        let entry = IfdEntry {
            tag_id: 256,
            field_type: Some(FieldType::Long),
            field_type_raw: 4,
            count: 1,
            value_offset_bytes: vec![0x50, 0xC3, 0x00, 0x00], // 50000
            is_inline: true,
        };

        let result = value_reader.read_u32(&entry).await.unwrap();
        assert_eq!(result, 50000);
    }

    #[tokio::test]
    async fn test_value_reader_error_unknown_type() {
        let reader = MockReader::new(vec![0; 100]);
        let header = make_tiff_header();
        let value_reader = ValueReader::new(&reader, &header);

        // Entry with unknown field type
        let entry = IfdEntry {
            tag_id: 256,
            field_type: None,
            field_type_raw: 99,
            count: 1,
            value_offset_bytes: vec![0x00, 0x00, 0x00, 0x00],
            is_inline: false,
        };

        let result = value_reader.read_bytes(&entry).await;
        assert!(matches!(result, Err(TiffError::UnknownFieldType(99))));
    }
}
