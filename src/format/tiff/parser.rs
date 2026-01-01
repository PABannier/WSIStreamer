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

use std::collections::HashMap;

use crate::error::TiffError;
use crate::io::{read_u16_be, read_u16_le, read_u32_be, read_u32_le, read_u64_be, read_u64_le};

use super::tags::{FieldType, TiffTag};

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
// IfdEntry
// =============================================================================

/// A single entry in an IFD (Image File Directory).
///
/// Each entry describes one piece of metadata about the image. The value may be
/// stored inline (in the `value_offset` field) or at a separate offset in the file.
///
/// ## Classic TIFF Entry Layout (12 bytes)
/// ```text
/// Bytes 0-1:  Tag ID (u16)
/// Bytes 2-3:  Field type (u16)
/// Bytes 4-7:  Count (u32)
/// Bytes 8-11: Value or offset (u32)
/// ```
///
/// ## BigTIFF Entry Layout (20 bytes)
/// ```text
/// Bytes 0-1:   Tag ID (u16)
/// Bytes 2-3:   Field type (u16)
/// Bytes 4-11:  Count (u64)
/// Bytes 12-19: Value or offset (u64)
/// ```
#[derive(Debug, Clone)]
pub struct IfdEntry {
    /// The tag ID (may be a known TiffTag or unknown)
    pub tag_id: u16,

    /// The field type (None if unknown type)
    pub field_type: Option<FieldType>,

    /// Raw field type value (for error reporting)
    pub field_type_raw: u16,

    /// Number of values (not bytes!)
    pub count: u64,

    /// Raw bytes of the value/offset field.
    /// For classic TIFF: 4 bytes, for BigTIFF: 8 bytes.
    /// Contains either the actual value (if inline) or an offset to the value.
    pub value_offset_bytes: Vec<u8>,

    /// Whether the value is stored inline (true) or at an offset (false)
    pub is_inline: bool,
}

impl IfdEntry {
    /// Parse an IFD entry from raw bytes.
    ///
    /// # Arguments
    /// * `bytes` - Raw entry bytes (12 for TIFF, 20 for BigTIFF)
    /// * `header` - The TIFF header (provides byte order and format info)
    fn parse(bytes: &[u8], header: &TiffHeader) -> Self {
        let byte_order = header.byte_order;

        // Tag ID (2 bytes)
        let tag_id = byte_order.read_u16(&bytes[0..2]);

        // Field type (2 bytes)
        let field_type_raw = byte_order.read_u16(&bytes[2..4]);
        let field_type = FieldType::from_u16(field_type_raw);

        // Count and value/offset depend on format
        let (count, value_offset_bytes) = if header.is_bigtiff {
            // BigTIFF: 8-byte count, 8-byte value/offset
            let count = byte_order.read_u64(&bytes[4..12]);
            let value_offset_bytes = bytes[12..20].to_vec();
            (count, value_offset_bytes)
        } else {
            // Classic TIFF: 4-byte count, 4-byte value/offset
            let count = byte_order.read_u32(&bytes[4..8]) as u64;
            let value_offset_bytes = bytes[8..12].to_vec();
            (count, value_offset_bytes)
        };

        // Determine if value is inline
        let is_inline = field_type
            .map(|ft| ft.fits_inline(count, header.is_bigtiff))
            .unwrap_or(false);

        IfdEntry {
            tag_id,
            field_type,
            field_type_raw,
            count,
            value_offset_bytes,
            is_inline,
        }
    }

    /// Get the known TiffTag for this entry, if recognized.
    pub fn tag(&self) -> Option<TiffTag> {
        TiffTag::from_u16(self.tag_id)
    }

    /// Get the offset to the value data (for non-inline values).
    ///
    /// # Arguments
    /// * `byte_order` - The byte order to use for reading
    ///
    /// # Returns
    /// The offset, or 0 if the value is inline (check `is_inline` first).
    pub fn value_offset(&self, byte_order: ByteOrder) -> u64 {
        if self.value_offset_bytes.len() == 8 {
            byte_order.read_u64(&self.value_offset_bytes)
        } else {
            byte_order.read_u32(&self.value_offset_bytes) as u64
        }
    }

    /// Read inline value as a single u16.
    ///
    /// # Arguments
    /// * `byte_order` - The byte order to use for reading
    ///
    /// # Returns
    /// The value, or None if not inline or count != 1 or wrong type.
    pub fn inline_u16(&self, byte_order: ByteOrder) -> Option<u16> {
        if !self.is_inline || self.count != 1 {
            return None;
        }
        match self.field_type? {
            FieldType::Short => Some(byte_order.read_u16(&self.value_offset_bytes)),
            _ => None,
        }
    }

    /// Read inline value as a single u32.
    ///
    /// # Arguments
    /// * `byte_order` - The byte order to use for reading
    ///
    /// # Returns
    /// The value, or None if not inline or count != 1 or wrong type.
    pub fn inline_u32(&self, byte_order: ByteOrder) -> Option<u32> {
        if !self.is_inline || self.count != 1 {
            return None;
        }
        match self.field_type? {
            FieldType::Short => Some(byte_order.read_u16(&self.value_offset_bytes) as u32),
            FieldType::Long => Some(byte_order.read_u32(&self.value_offset_bytes)),
            _ => None,
        }
    }

    /// Read inline value as a single u64.
    ///
    /// # Arguments
    /// * `byte_order` - The byte order to use for reading
    ///
    /// # Returns
    /// The value, or None if not inline or count != 1 or wrong type.
    pub fn inline_u64(&self, byte_order: ByteOrder) -> Option<u64> {
        if !self.is_inline || self.count != 1 {
            return None;
        }
        match self.field_type? {
            FieldType::Short => Some(byte_order.read_u16(&self.value_offset_bytes) as u64),
            FieldType::Long => Some(byte_order.read_u32(&self.value_offset_bytes) as u64),
            FieldType::Long8 => {
                if self.value_offset_bytes.len() >= 8 {
                    Some(byte_order.read_u64(&self.value_offset_bytes))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Calculate total byte size of the value data.
    pub fn value_byte_size(&self) -> Option<u64> {
        self.field_type
            .map(|ft| ft.size_in_bytes() as u64 * self.count)
    }
}

// =============================================================================
// Ifd
// =============================================================================

/// A parsed Image File Directory (IFD).
///
/// An IFD contains metadata about one image in the TIFF file. WSI files typically
/// have multiple IFDs: one for each pyramid level, plus label/macro images.
///
/// The entries are stored both as a vector (preserving order) and as a hashmap
/// (for fast lookup by tag).
#[derive(Debug, Clone)]
pub struct Ifd {
    /// All entries in this IFD, in file order
    pub entries: Vec<IfdEntry>,

    /// Entries indexed by tag ID for fast lookup
    pub(crate) entries_by_tag: HashMap<u16, usize>,

    /// Offset to the next IFD (0 if this is the last IFD)
    pub next_ifd_offset: u64,
}

impl Ifd {
    /// Parse an IFD from raw bytes.
    ///
    /// The bytes should start at the IFD offset and contain:
    /// - Entry count (2 or 8 bytes depending on format)
    /// - All entries (12 or 20 bytes each)
    /// - Next IFD offset (4 or 8 bytes)
    ///
    /// # Arguments
    /// * `bytes` - Raw IFD bytes
    /// * `header` - The TIFF header
    ///
    /// # Errors
    /// Returns an error if the bytes are too short for the declared entry count.
    pub fn parse(bytes: &[u8], header: &TiffHeader) -> Result<Self, TiffError> {
        let byte_order = header.byte_order;
        let count_size = header.ifd_count_size();
        let entry_size = header.ifd_entry_size();
        let next_offset_size = header.ifd_next_offset_size();

        // Read entry count
        if bytes.len() < count_size {
            return Err(TiffError::FileTooSmall {
                required: count_size as u64,
                actual: bytes.len() as u64,
            });
        }

        let entry_count = if header.is_bigtiff {
            byte_order.read_u64(&bytes[0..8])
        } else {
            byte_order.read_u16(&bytes[0..2]) as u64
        };

        // Calculate required size
        let entries_start = count_size;
        let entries_size = entry_count as usize * entry_size;
        let next_offset_start = entries_start + entries_size;
        let total_required = next_offset_start + next_offset_size;

        if bytes.len() < total_required {
            return Err(TiffError::FileTooSmall {
                required: total_required as u64,
                actual: bytes.len() as u64,
            });
        }

        // Parse entries
        let mut entries = Vec::with_capacity(entry_count as usize);
        let mut entries_by_tag = HashMap::with_capacity(entry_count as usize);

        for i in 0..entry_count as usize {
            let entry_start = entries_start + i * entry_size;
            let entry_bytes = &bytes[entry_start..entry_start + entry_size];
            let entry = IfdEntry::parse(entry_bytes, header);

            entries_by_tag.insert(entry.tag_id, entries.len());
            entries.push(entry);
        }

        // Read next IFD offset
        let next_ifd_offset = if header.is_bigtiff {
            byte_order.read_u64(&bytes[next_offset_start..next_offset_start + 8])
        } else {
            byte_order.read_u32(&bytes[next_offset_start..next_offset_start + 4]) as u64
        };

        Ok(Ifd {
            entries,
            entries_by_tag,
            next_ifd_offset,
        })
    }

    /// Calculate the total size in bytes needed to read this IFD.
    ///
    /// This can be used to determine how many bytes to fetch before parsing.
    /// Note: This is the size of the IFD structure itself, not including
    /// any values stored at external offsets.
    ///
    /// # Arguments
    /// * `entry_count` - Number of entries in the IFD
    /// * `header` - The TIFF header
    pub fn calculate_size(entry_count: u64, header: &TiffHeader) -> usize {
        header.ifd_count_size()
            + (entry_count as usize * header.ifd_entry_size())
            + header.ifd_next_offset_size()
    }

    /// Get an entry by its tag ID.
    pub fn get_entry(&self, tag_id: u16) -> Option<&IfdEntry> {
        self.entries_by_tag
            .get(&tag_id)
            .map(|&idx| &self.entries[idx])
    }

    /// Get an entry by its known TiffTag.
    pub fn get_entry_by_tag(&self, tag: TiffTag) -> Option<&IfdEntry> {
        self.get_entry(tag.as_u16())
    }

    /// Get an inline u32 value for a tag.
    ///
    /// This is a convenience method for reading simple scalar values like
    /// ImageWidth, ImageLength, TileWidth, etc.
    pub fn get_u32(&self, tag: TiffTag, byte_order: ByteOrder) -> Option<u32> {
        self.get_entry_by_tag(tag)?.inline_u32(byte_order)
    }

    /// Get an inline u64 value for a tag.
    pub fn get_u64(&self, tag: TiffTag, byte_order: ByteOrder) -> Option<u64> {
        self.get_entry_by_tag(tag)?.inline_u64(byte_order)
    }

    /// Get an inline u16 value for a tag.
    pub fn get_u16(&self, tag: TiffTag, byte_order: ByteOrder) -> Option<u16> {
        self.get_entry_by_tag(tag)?.inline_u16(byte_order)
    }

    /// Check if this IFD has tile organization (vs strip).
    ///
    /// Returns true if TileWidth and TileLength tags are present.
    pub fn is_tiled(&self) -> bool {
        self.get_entry_by_tag(TiffTag::TileWidth).is_some()
            && self.get_entry_by_tag(TiffTag::TileLength).is_some()
    }

    /// Check if this IFD has strip organization.
    ///
    /// Returns true if StripOffsets tag is present.
    pub fn is_stripped(&self) -> bool {
        self.get_entry_by_tag(TiffTag::StripOffsets).is_some()
    }

    /// Get image width from this IFD.
    pub fn image_width(&self, byte_order: ByteOrder) -> Option<u32> {
        self.get_u32(TiffTag::ImageWidth, byte_order)
    }

    /// Get image height (length) from this IFD.
    pub fn image_height(&self, byte_order: ByteOrder) -> Option<u32> {
        self.get_u32(TiffTag::ImageLength, byte_order)
    }

    /// Get tile width from this IFD.
    pub fn tile_width(&self, byte_order: ByteOrder) -> Option<u32> {
        self.get_u32(TiffTag::TileWidth, byte_order)
    }

    /// Get tile height (length) from this IFD.
    pub fn tile_height(&self, byte_order: ByteOrder) -> Option<u32> {
        self.get_u32(TiffTag::TileLength, byte_order)
    }

    /// Get compression scheme from this IFD.
    pub fn compression(&self, byte_order: ByteOrder) -> Option<u16> {
        self.get_u16(TiffTag::Compression, byte_order)
    }

    /// Get the number of entries in this IFD.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Create an empty IFD (for testing).
    #[cfg(test)]
    pub fn empty() -> Self {
        Ifd {
            entries: Vec::new(),
            entries_by_tag: HashMap::new(),
            next_ifd_offset: 0,
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
        assert_eq!(ByteOrder::LittleEndian.read_u64(&bytes), 0x0807060504030201);
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

    // -------------------------------------------------------------------------
    // IfdEntry Tests
    // -------------------------------------------------------------------------

    fn make_tiff_header() -> TiffHeader {
        TiffHeader {
            byte_order: ByteOrder::LittleEndian,
            is_bigtiff: false,
            first_ifd_offset: 8,
        }
    }

    fn make_bigtiff_header() -> TiffHeader {
        TiffHeader {
            byte_order: ByteOrder::LittleEndian,
            is_bigtiff: true,
            first_ifd_offset: 16,
        }
    }

    #[test]
    fn test_ifd_entry_parse_tiff_inline_short() {
        // Classic TIFF entry: ImageWidth = 1024 (SHORT type, count=1, inline)
        // Tag 256 (0x0100), Type 3 (SHORT), Count 1, Value 1024 (0x0400)
        let entry_bytes = [
            0x00, 0x01, // Tag ID = 256 (ImageWidth) - little-endian
            0x03, 0x00, // Type = 3 (SHORT)
            0x01, 0x00, 0x00, 0x00, // Count = 1
            0x00, 0x04, 0x00, 0x00, // Value = 1024 (inline)
        ];

        let header = make_tiff_header();
        let entry = IfdEntry::parse(&entry_bytes, &header);

        assert_eq!(entry.tag_id, 256);
        assert_eq!(entry.tag(), Some(TiffTag::ImageWidth));
        assert_eq!(entry.field_type, Some(FieldType::Short));
        assert_eq!(entry.count, 1);
        assert!(entry.is_inline);
        assert_eq!(entry.inline_u16(header.byte_order), Some(1024));
        assert_eq!(entry.inline_u32(header.byte_order), Some(1024));
    }

    #[test]
    fn test_ifd_entry_parse_tiff_inline_long() {
        // Classic TIFF entry: ImageWidth = 50000 (LONG type, count=1, inline)
        // Tag 256 (0x0100), Type 4 (LONG), Count 1, Value 50000
        let entry_bytes = [
            0x00, 0x01, // Tag ID = 256 (ImageWidth)
            0x04, 0x00, // Type = 4 (LONG)
            0x01, 0x00, 0x00, 0x00, // Count = 1
            0x50, 0xC3, 0x00, 0x00, // Value = 50000 (inline)
        ];

        let header = make_tiff_header();
        let entry = IfdEntry::parse(&entry_bytes, &header);

        assert_eq!(entry.tag_id, 256);
        assert_eq!(entry.field_type, Some(FieldType::Long));
        assert!(entry.is_inline);
        assert_eq!(entry.inline_u32(header.byte_order), Some(50000));
    }

    #[test]
    fn test_ifd_entry_parse_tiff_offset() {
        // Classic TIFF entry: TileOffsets with count > 1 (stored at offset)
        // Tag 324 (0x0144), Type 4 (LONG), Count 100, Offset 1000
        let entry_bytes = [
            0x44, 0x01, // Tag ID = 324 (TileOffsets)
            0x04, 0x00, // Type = 4 (LONG)
            0x64, 0x00, 0x00, 0x00, // Count = 100
            0xE8, 0x03, 0x00, 0x00, // Offset = 1000
        ];

        let header = make_tiff_header();
        let entry = IfdEntry::parse(&entry_bytes, &header);

        assert_eq!(entry.tag_id, 324);
        assert_eq!(entry.tag(), Some(TiffTag::TileOffsets));
        assert_eq!(entry.field_type, Some(FieldType::Long));
        assert_eq!(entry.count, 100);
        assert!(!entry.is_inline); // Not inline because count * 4 > 4
        assert_eq!(entry.value_offset(header.byte_order), 1000);
        assert_eq!(entry.value_byte_size(), Some(400)); // 100 * 4 bytes
    }

    #[test]
    fn test_ifd_entry_parse_bigtiff_inline_long8() {
        // BigTIFF entry: ImageWidth = 100000 (LONG8 type, count=1, inline)
        let entry_bytes = [
            0x00, 0x01, // Tag ID = 256 (ImageWidth)
            0x10, 0x00, // Type = 16 (LONG8)
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Count = 1
            0xA0, 0x86, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, // Value = 100000
        ];

        let header = make_bigtiff_header();
        let entry = IfdEntry::parse(&entry_bytes, &header);

        assert_eq!(entry.tag_id, 256);
        assert_eq!(entry.field_type, Some(FieldType::Long8));
        assert!(entry.is_inline);
        assert_eq!(entry.inline_u64(header.byte_order), Some(100000));
    }

    #[test]
    fn test_ifd_entry_unknown_field_type() {
        // Entry with unknown field type (99)
        let entry_bytes = [
            0x00, 0x01, // Tag ID = 256
            0x63, 0x00, // Type = 99 (unknown)
            0x01, 0x00, 0x00, 0x00, // Count = 1
            0x00, 0x00, 0x00, 0x00, // Value
        ];

        let header = make_tiff_header();
        let entry = IfdEntry::parse(&entry_bytes, &header);

        assert_eq!(entry.field_type, None);
        assert_eq!(entry.field_type_raw, 99);
        assert!(!entry.is_inline); // Unknown types are not considered inline
    }

    // -------------------------------------------------------------------------
    // Ifd Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_ifd_parse_tiff_simple() {
        // Classic TIFF IFD with 3 entries:
        // - ImageWidth = 1024
        // - ImageLength = 768
        // - Compression = 7 (JPEG)
        // Next IFD offset = 500
        let ifd_bytes = [
            // Entry count = 3
            0x03, 0x00, // Entry 1: ImageWidth (256) = 1024
            0x00, 0x01, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00,
            // Entry 2: ImageLength (257) = 768
            0x01, 0x01, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00,
            // Entry 3: Compression (259) = 7
            0x03, 0x01, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00,
            // Next IFD offset = 500
            0xF4, 0x01, 0x00, 0x00,
        ];

        let header = make_tiff_header();
        let ifd = Ifd::parse(&ifd_bytes, &header).unwrap();

        assert_eq!(ifd.entry_count(), 3);
        assert_eq!(ifd.next_ifd_offset, 500);

        // Check values via convenience methods
        assert_eq!(ifd.image_width(header.byte_order), Some(1024));
        assert_eq!(ifd.image_height(header.byte_order), Some(768));
        assert_eq!(ifd.compression(header.byte_order), Some(7));

        // Check entry lookup
        let width_entry = ifd.get_entry_by_tag(TiffTag::ImageWidth).unwrap();
        assert_eq!(width_entry.count, 1);
    }

    #[test]
    fn test_ifd_parse_bigtiff() {
        // BigTIFF IFD with 2 entries
        let ifd_bytes = [
            // Entry count = 2 (8 bytes in BigTIFF)
            0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            // Entry 1: ImageWidth (256) = 50000 (LONG type, count=1)
            0x00, 0x01, // Tag
            0x04, 0x00, // Type = LONG
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Count = 1
            0x50, 0xC3, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Value = 50000
            // Entry 2: ImageLength (257) = 40000
            0x01, 0x01, // Tag
            0x04, 0x00, // Type = LONG
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Count = 1
            0x40, 0x9C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Value = 40000
            // Next IFD offset = 1000 (8 bytes)
            0xE8, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];

        let header = make_bigtiff_header();
        let ifd = Ifd::parse(&ifd_bytes, &header).unwrap();

        assert_eq!(ifd.entry_count(), 2);
        assert_eq!(ifd.next_ifd_offset, 1000);
        assert_eq!(ifd.image_width(header.byte_order), Some(50000));
        assert_eq!(ifd.image_height(header.byte_order), Some(40000));
    }

    #[test]
    fn test_ifd_parse_with_tiles() {
        // IFD with tile-related tags
        let ifd_bytes = [
            // Entry count = 4
            0x04, 0x00, // ImageWidth = 10000
            0x00, 0x01, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00, 0x10, 0x27, 0x00, 0x00,
            // ImageLength = 8000
            0x01, 0x01, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00, 0x40, 0x1F, 0x00, 0x00,
            // TileWidth (322) = 256
            0x42, 0x01, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
            // TileLength (323) = 256
            0x43, 0x01, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
            // Next IFD = 0 (no more IFDs)
            0x00, 0x00, 0x00, 0x00,
        ];

        let header = make_tiff_header();
        let ifd = Ifd::parse(&ifd_bytes, &header).unwrap();

        assert!(ifd.is_tiled());
        assert!(!ifd.is_stripped());
        assert_eq!(ifd.tile_width(header.byte_order), Some(256));
        assert_eq!(ifd.tile_height(header.byte_order), Some(256));
        assert_eq!(ifd.next_ifd_offset, 0);
    }

    #[test]
    fn test_ifd_parse_big_endian() {
        // Big-endian IFD with 1 entry
        let ifd_bytes = [
            // Entry count = 1 (big-endian)
            0x00, 0x01, // ImageWidth = 2048 (big-endian)
            0x01, 0x00, // Tag = 256
            0x00, 0x03, // Type = SHORT
            0x00, 0x00, 0x00, 0x01, // Count = 1
            0x08, 0x00, 0x00, 0x00, // Value = 2048
            // Next IFD = 0
            0x00, 0x00, 0x00, 0x00,
        ];

        let header = TiffHeader {
            byte_order: ByteOrder::BigEndian,
            is_bigtiff: false,
            first_ifd_offset: 8,
        };

        let ifd = Ifd::parse(&ifd_bytes, &header).unwrap();

        assert_eq!(ifd.entry_count(), 1);
        assert_eq!(ifd.image_width(header.byte_order), Some(2048));
    }

    #[test]
    fn test_ifd_calculate_size() {
        let tiff_header = make_tiff_header();
        let bigtiff_header = make_bigtiff_header();

        // Classic TIFF: 2 (count) + 10*12 (entries) + 4 (next) = 126 bytes
        assert_eq!(Ifd::calculate_size(10, &tiff_header), 126);

        // BigTIFF: 8 (count) + 10*20 (entries) + 8 (next) = 216 bytes
        assert_eq!(Ifd::calculate_size(10, &bigtiff_header), 216);
    }

    #[test]
    fn test_ifd_parse_error_too_small() {
        // IFD bytes too small for declared entry count
        let ifd_bytes = [
            0x05, 0x00, // Entry count = 5, but we only provide 2 entries worth
            0x00, 0x01, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x01, 0x01,
            0x03, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00,
        ];

        let header = make_tiff_header();
        let result = Ifd::parse(&ifd_bytes, &header);

        assert!(matches!(result, Err(TiffError::FileTooSmall { .. })));
    }

    #[test]
    fn test_ifd_get_entry_not_found() {
        // IFD with just ImageWidth
        let ifd_bytes = [
            0x01, 0x00, 0x00, 0x01, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00,
        ];

        let header = make_tiff_header();
        let ifd = Ifd::parse(&ifd_bytes, &header).unwrap();

        // Should find ImageWidth
        assert!(ifd.get_entry_by_tag(TiffTag::ImageWidth).is_some());

        // Should not find ImageLength
        assert!(ifd.get_entry_by_tag(TiffTag::ImageLength).is_none());
        assert_eq!(ifd.image_height(header.byte_order), None);
    }
}
