//! Test utilities for integration tests.
//!
//! This module provides mock implementations and helper functions for creating
//! test TIFF files with various configurations.

use async_trait::async_trait;
use bytes::Bytes;
use image::codecs::jpeg::JpegEncoder;
use image::{GrayImage, Luma, Rgb, RgbImage};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

use wsi_streamer::error::IoError;
use wsi_streamer::io::RangeReader;
use wsi_streamer::slide::{SlideListResult, SlideSource};

// =============================================================================
// Mock Range Reader with Request Tracking
// =============================================================================

/// A mock range reader that tracks all read requests.
///
/// This is useful for verifying cache behavior and request patterns.
pub struct TrackingMockReader {
    data: Bytes,
    identifier: String,
    request_count: Arc<AtomicUsize>,
    requests: Arc<RwLock<Vec<(u64, usize)>>>,
}

impl TrackingMockReader {
    pub fn new(data: Vec<u8>, identifier: impl Into<String>) -> Self {
        Self {
            data: Bytes::from(data),
            identifier: identifier.into(),
            request_count: Arc::new(AtomicUsize::new(0)),
            requests: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub fn request_count(&self) -> usize {
        self.request_count.load(Ordering::SeqCst)
    }

    pub async fn get_requests(&self) -> Vec<(u64, usize)> {
        self.requests.read().await.clone()
    }

    pub fn reset_tracking(&self) {
        self.request_count.store(0, Ordering::SeqCst);
    }
}

impl Clone for TrackingMockReader {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            identifier: self.identifier.clone(),
            request_count: Arc::clone(&self.request_count),
            requests: Arc::clone(&self.requests),
        }
    }
}

#[async_trait]
impl RangeReader for TrackingMockReader {
    async fn read_exact_at(&self, offset: u64, len: usize) -> Result<Bytes, IoError> {
        self.request_count.fetch_add(1, Ordering::SeqCst);
        self.requests.write().await.push((offset, len));

        let start = offset as usize;
        let end = start + len;
        if end > self.data.len() {
            return Err(IoError::RangeOutOfBounds {
                offset,
                requested: len as u64,
                size: self.data.len() as u64,
            });
        }
        Ok(self.data.slice(start..end))
    }

    fn size(&self) -> u64 {
        self.data.len() as u64
    }

    fn identifier(&self) -> &str {
        &self.identifier
    }
}

// =============================================================================
// Mock Slide Source
// =============================================================================

/// A mock slide source that serves pre-configured slide data.
pub struct MockSlideSource {
    slides: HashMap<String, Bytes>,
    request_counts: Arc<RwLock<HashMap<String, usize>>>,
}

impl MockSlideSource {
    pub fn new() -> Self {
        Self {
            slides: HashMap::new(),
            request_counts: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn with_slide(mut self, slide_id: impl Into<String>, data: Vec<u8>) -> Self {
        self.slides.insert(slide_id.into(), Bytes::from(data));
        self
    }

    pub async fn get_request_count(&self, slide_id: &str) -> usize {
        self.request_counts
            .read()
            .await
            .get(slide_id)
            .copied()
            .unwrap_or(0)
    }
}

impl Default for MockSlideSource {
    fn default() -> Self {
        Self::new()
    }
}

/// Supported slide file extensions for filtering.
const SLIDE_EXTENSIONS: &[&str] = &[".svs", ".tif", ".tiff"];

/// Check if a file path has a supported slide extension.
fn is_slide_file(path: &str) -> bool {
    let path_lower = path.to_lowercase();
    SLIDE_EXTENSIONS.iter().any(|ext| path_lower.ends_with(ext))
}

#[async_trait]
impl SlideSource for MockSlideSource {
    type Reader = TrackingMockReader;

    async fn create_reader(&self, slide_id: &str) -> Result<Self::Reader, IoError> {
        // Track the request
        {
            let mut counts = self.request_counts.write().await;
            *counts.entry(slide_id.to_string()).or_insert(0) += 1;
        }

        match self.slides.get(slide_id) {
            Some(data) => Ok(TrackingMockReader::new(
                data.to_vec(),
                format!("mock://{}", slide_id),
            )),
            None => Err(IoError::NotFound(slide_id.to_string())),
        }
    }

    async fn list_slides(
        &self,
        limit: u32,
        _cursor: Option<&str>,
    ) -> Result<SlideListResult, IoError> {
        // Get all slide keys that have supported extensions
        let mut slides: Vec<String> = self
            .slides
            .keys()
            .filter(|k| is_slide_file(k))
            .cloned()
            .collect();

        // Sort for consistent ordering
        slides.sort();

        // Apply limit
        let limit = limit as usize;
        let has_more = slides.len() > limit;
        slides.truncate(limit);

        // Simple pagination: use last key as cursor if there are more results
        let next_cursor = if has_more {
            slides.last().cloned()
        } else {
            None
        };

        Ok(SlideListResult {
            slides,
            next_cursor,
        })
    }
}

// =============================================================================
// Test JPEG Creation
// =============================================================================

/// Create a test JPEG image with a simple gradient pattern.
pub fn create_test_jpeg(width: u32, height: u32, quality: u8) -> Vec<u8> {
    let img = GrayImage::from_fn(width, height, |x, y| {
        let val = ((x + y) % 256) as u8;
        Luma([val])
    });

    let mut buf = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut buf, quality);
    encoder.encode_image(&img).unwrap();
    buf
}

/// Create a test RGB JPEG image.
pub fn create_test_rgb_jpeg(width: u32, height: u32, quality: u8) -> Vec<u8> {
    let img = RgbImage::from_fn(width, height, |x, y| {
        let r = (x % 256) as u8;
        let g = (y % 256) as u8;
        let b = ((x + y) % 256) as u8;
        Rgb([r, g, b])
    });

    let mut buf = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut buf, quality);
    encoder.encode_image(&img).unwrap();
    buf
}

// =============================================================================
// TIFF File Builders
// =============================================================================

/// Builder for creating test TIFF files.
pub struct TiffBuilder {
    byte_order: ByteOrderType,
    is_bigtiff: bool,
    ifds: Vec<IfdBuilder>,
}

#[derive(Clone, Copy)]
pub enum ByteOrderType {
    LittleEndian,
    BigEndian,
}

impl TiffBuilder {
    pub fn new() -> Self {
        Self {
            byte_order: ByteOrderType::LittleEndian,
            is_bigtiff: false,
            ifds: Vec::new(),
        }
    }

    pub fn with_byte_order(mut self, order: ByteOrderType) -> Self {
        self.byte_order = order;
        self
    }

    pub fn with_bigtiff(mut self, is_bigtiff: bool) -> Self {
        self.is_bigtiff = is_bigtiff;
        self
    }

    pub fn add_ifd(mut self, ifd: IfdBuilder) -> Self {
        self.ifds.push(ifd);
        self
    }

    /// Build the TIFF file data.
    pub fn build(self) -> Vec<u8> {
        let mut data = Vec::new();

        // Calculate sizes and offsets
        let header_size = if self.is_bigtiff { 16 } else { 8 };
        let entry_size = if self.is_bigtiff { 20 } else { 12 };
        let offset_size = if self.is_bigtiff { 8 } else { 4 };

        // Write header
        match self.byte_order {
            ByteOrderType::LittleEndian => {
                data.push(b'I');
                data.push(b'I');
            }
            ByteOrderType::BigEndian => {
                data.push(b'M');
                data.push(b'M');
            }
        }

        // Version
        if self.is_bigtiff {
            self.write_u16(&mut data, 43); // BigTIFF version
            self.write_u16(&mut data, 8); // Offset size
            self.write_u16(&mut data, 0); // Reserved
        } else {
            self.write_u16(&mut data, 42); // Classic TIFF version
        }

        // First IFD offset - write as placeholder for now
        let first_ifd_offset_pos = data.len();
        if self.is_bigtiff {
            self.write_u64(&mut data, 0);
        } else {
            self.write_u32(&mut data, 0);
        }

        // Calculate where each IFD and its data will go
        let mut current_offset = header_size as u64;
        let mut ifd_data_sections: Vec<(usize, Vec<u8>)> = Vec::new();

        for (idx, ifd) in self.ifds.iter().enumerate() {
            let entry_count = ifd.entries.len();
            let ifd_size = if self.is_bigtiff {
                8 + entry_count * entry_size + 8 // entry count (8) + entries + next ifd (8)
            } else {
                2 + entry_count * entry_size + 4 // entry count (2) + entries + next ifd (4)
            };

            // Collect external data that needs to go after the IFD
            let external_data = ifd.build_external_data(self.byte_order, self.is_bigtiff);
            ifd_data_sections.push((idx, external_data));

            current_offset += ifd_size as u64;
        }

        // Now write the IFDs
        let first_ifd_offset = data.len() as u64;

        // Go back and write the correct first IFD offset
        let saved_len = data.len();
        data.truncate(first_ifd_offset_pos);
        if self.is_bigtiff {
            self.write_u64(&mut data, first_ifd_offset);
        } else {
            self.write_u32(&mut data, first_ifd_offset as u32);
        }
        while data.len() < saved_len {
            data.push(0);
        }

        // Write IFDs
        for (idx, ifd) in self.ifds.iter().enumerate() {
            let next_ifd_offset = if idx + 1 < self.ifds.len() {
                // Calculate next IFD offset (placeholder - we'll fix this)
                0u64
            } else {
                0u64
            };

            ifd.write_to(&mut data, self.byte_order, self.is_bigtiff, next_ifd_offset);
        }

        // Write external data for each IFD
        for (_, external_data) in ifd_data_sections {
            data.extend(external_data);
        }

        data
    }

    fn write_u16(&self, data: &mut Vec<u8>, value: u16) {
        match self.byte_order {
            ByteOrderType::LittleEndian => data.extend(&value.to_le_bytes()),
            ByteOrderType::BigEndian => data.extend(&value.to_be_bytes()),
        }
    }

    fn write_u32(&self, data: &mut Vec<u8>, value: u32) {
        match self.byte_order {
            ByteOrderType::LittleEndian => data.extend(&value.to_le_bytes()),
            ByteOrderType::BigEndian => data.extend(&value.to_be_bytes()),
        }
    }

    fn write_u64(&self, data: &mut Vec<u8>, value: u64) {
        match self.byte_order {
            ByteOrderType::LittleEndian => data.extend(&value.to_le_bytes()),
            ByteOrderType::BigEndian => data.extend(&value.to_be_bytes()),
        }
    }
}

impl Default for TiffBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for creating IFD entries.
pub struct IfdBuilder {
    entries: Vec<IfdEntryBuilder>,
    jpeg_tile_data: Option<Vec<u8>>,
    tile_offsets: Vec<u64>,
    tile_byte_counts: Vec<u64>,
}

impl IfdBuilder {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            jpeg_tile_data: None,
            tile_offsets: Vec::new(),
            tile_byte_counts: Vec::new(),
        }
    }

    /// Create a standard tiled IFD with JPEG compression.
    pub fn tiled_jpeg(
        width: u32,
        height: u32,
        tile_width: u32,
        tile_height: u32,
        jpeg_data: Vec<u8>,
    ) -> Self {
        let tiles_x = (width + tile_width - 1) / tile_width;
        let tiles_y = (height + tile_height - 1) / tile_height;
        let tile_count = (tiles_x * tiles_y) as usize;

        let jpeg_len = jpeg_data.len();

        let mut builder = Self {
            entries: Vec::new(),
            jpeg_tile_data: Some(jpeg_data),
            tile_offsets: vec![0; tile_count], // Will be filled in during build
            tile_byte_counts: vec![jpeg_len as u64; tile_count],
        };

        builder
            .add_entry(256, 4, 1, width as u64) // ImageWidth
            .add_entry(257, 4, 1, height as u64) // ImageLength
            .add_entry(258, 3, 1, 8) // BitsPerSample
            .add_entry(259, 3, 1, 7) // Compression = JPEG
            .add_entry(262, 3, 1, 1) // PhotometricInterpretation = MinIsBlack
            .add_entry(277, 3, 1, 1) // SamplesPerPixel
            .add_entry(322, 3, 1, tile_width as u64) // TileWidth
            .add_entry(323, 3, 1, tile_height as u64) // TileLength
            // TileOffsets and TileByteCounts will be added during build
            ;

        builder
    }

    /// Add a tag entry.
    pub fn add_entry(&mut self, tag: u16, field_type: u16, count: u32, value: u64) -> &mut Self {
        self.entries.push(IfdEntryBuilder {
            tag,
            field_type,
            count,
            value,
            external_data: None,
        });
        self
    }

    /// Add a tag entry with external data (for arrays).
    pub fn add_entry_with_data(
        &mut self,
        tag: u16,
        field_type: u16,
        count: u32,
        data: Vec<u8>,
    ) -> &mut Self {
        self.entries.push(IfdEntryBuilder {
            tag,
            field_type,
            count,
            value: 0, // Will be filled in with offset
            external_data: Some(data),
        });
        self
    }

    fn build_external_data(&self, _byte_order: ByteOrderType, _is_bigtiff: bool) -> Vec<u8> {
        let mut data = Vec::new();

        // Add JPEG tile data
        if let Some(ref jpeg) = self.jpeg_tile_data {
            data.extend(jpeg);
        }

        // Add external data from entries
        for entry in &self.entries {
            if let Some(ref ext) = entry.external_data {
                data.extend(ext);
            }
        }

        data
    }

    fn write_to(
        &self,
        data: &mut Vec<u8>,
        byte_order: ByteOrderType,
        is_bigtiff: bool,
        next_ifd_offset: u64,
    ) {
        // Entry count
        let entry_count = self.entries.len() + 2; // +2 for TileOffsets and TileByteCounts
        if is_bigtiff {
            write_value(data, byte_order, entry_count as u64, 8);
        } else {
            write_value(data, byte_order, entry_count as u64, 2);
        }

        // Calculate data section start offset
        let entry_size = if is_bigtiff { 20 } else { 12 };
        let entries_end = data.len() + entry_count * entry_size + if is_bigtiff { 8 } else { 4 };
        let mut data_offset = entries_end as u64;

        // Write entries (sorted by tag)
        let mut all_entries: Vec<(u16, u16, u32, u64, Option<&Vec<u8>>)> = self
            .entries
            .iter()
            .map(|e| {
                (
                    e.tag,
                    e.field_type,
                    e.count,
                    e.value,
                    e.external_data.as_ref(),
                )
            })
            .collect();

        // Add TileOffsets (324) and TileByteCounts (325) entries
        let tile_count = self.tile_offsets.len() as u32;
        all_entries.push((324, 4, tile_count, 0, None)); // TileOffsets
        all_entries.push((325, 4, tile_count, 0, None)); // TileByteCounts

        all_entries.sort_by_key(|e| e.0);

        // Track where we put the tile data
        let tile_data_offset = data_offset;
        let tile_data_len = self.jpeg_tile_data.as_ref().map(|d| d.len()).unwrap_or(0);

        // Update data offset past tile data
        data_offset += tile_data_len as u64;

        for (tag, field_type, count, value, ext_data) in all_entries {
            write_value(data, byte_order, tag as u64, 2);
            write_value(data, byte_order, field_type as u64, 2);

            if is_bigtiff {
                write_value(data, byte_order, count as u64, 8);
            } else {
                write_value(data, byte_order, count as u64, 4);
            }

            // Value/offset field
            let value_size = field_type_size(field_type) * count as usize;
            let inline_size = if is_bigtiff { 8 } else { 4 };

            if tag == 324 {
                // TileOffsets - all point to same tile data for simplicity
                if value_size <= inline_size {
                    write_value(data, byte_order, tile_data_offset, inline_size);
                } else {
                    // Write offset to array
                    write_value(data, byte_order, data_offset, inline_size);
                    data_offset += (count as u64) * 4; // 4 bytes per offset
                }
            } else if tag == 325 {
                // TileByteCounts
                if value_size <= inline_size {
                    write_value(data, byte_order, tile_data_len as u64, inline_size);
                } else {
                    write_value(data, byte_order, data_offset, inline_size);
                    data_offset += (count as u64) * 4;
                }
            } else if let Some(_ext) = ext_data {
                // External data
                write_value(data, byte_order, data_offset, inline_size);
                data_offset += value_size as u64;
            } else if value_size <= inline_size {
                // Inline value
                write_value(data, byte_order, value, inline_size);
            } else {
                // Should have external data
                write_value(data, byte_order, data_offset, inline_size);
            }
        }

        // Next IFD offset
        if is_bigtiff {
            write_value(data, byte_order, next_ifd_offset, 8);
        } else {
            write_value(data, byte_order, next_ifd_offset, 4);
        }
    }
}

impl Default for IfdBuilder {
    fn default() -> Self {
        Self::new()
    }
}

struct IfdEntryBuilder {
    tag: u16,
    field_type: u16,
    count: u32,
    value: u64,
    external_data: Option<Vec<u8>>,
}

fn field_type_size(field_type: u16) -> usize {
    match field_type {
        1 => 1,  // BYTE
        2 => 1,  // ASCII
        3 => 2,  // SHORT
        4 => 4,  // LONG
        5 => 8,  // RATIONAL
        6 => 1,  // SBYTE
        7 => 1,  // UNDEFINED
        8 => 2,  // SSHORT
        9 => 4,  // SLONG
        10 => 8, // SRATIONAL
        11 => 4, // FLOAT
        12 => 8, // DOUBLE
        16 => 8, // LONG8 (BigTIFF)
        17 => 8, // SLONG8 (BigTIFF)
        18 => 8, // IFD8 (BigTIFF)
        _ => 1,
    }
}

fn write_value(data: &mut Vec<u8>, byte_order: ByteOrderType, value: u64, size: usize) {
    match byte_order {
        ByteOrderType::LittleEndian => match size {
            1 => data.push(value as u8),
            2 => data.extend(&(value as u16).to_le_bytes()),
            4 => data.extend(&(value as u32).to_le_bytes()),
            8 => data.extend(&value.to_le_bytes()),
            _ => {}
        },
        ByteOrderType::BigEndian => match size {
            1 => data.push(value as u8),
            2 => data.extend(&(value as u16).to_be_bytes()),
            4 => data.extend(&(value as u32).to_be_bytes()),
            8 => data.extend(&value.to_be_bytes()),
            _ => {}
        },
    }
}

// =============================================================================
// Simple TIFF Creation (More Direct Approach)
// =============================================================================

/// Create a minimal valid little-endian TIFF file with JPEG tile data.
pub fn create_tiff_with_jpeg_tile() -> Vec<u8> {
    create_tiff_with_jpeg_tile_endian(ByteOrderType::LittleEndian)
}

/// Create a minimal valid TIFF file with specified byte order.
pub fn create_tiff_with_jpeg_tile_endian(byte_order: ByteOrderType) -> Vec<u8> {
    let jpeg_data = create_test_jpeg(256, 256, 90);
    let jpeg_len = jpeg_data.len() as u32;

    // Layout:
    // 0-7: Header
    // 8-x: IFD (9 entries * 12 bytes + 2 entry count + 4 next ifd)
    // 200-392: TileOffsets array (48 * 4 bytes)
    // 400-592: TileByteCounts array (48 * 4 bytes)
    // 1000+: JPEG tile data

    let tile_data_offset = 1000u32;
    let tile_offsets_offset = 200u32;
    let tile_byte_counts_offset = 400u32;
    let tile_count = 48u32; // 8x6 tiles

    let total_size = tile_data_offset as usize + jpeg_data.len() + 100;
    let mut data = vec![0u8; total_size];

    // Closure to write u16 respecting byte order
    let write_u16 =
        |data: &mut [u8], offset: usize, value: u16, byte_order: ByteOrderType| match byte_order {
            ByteOrderType::LittleEndian => {
                data[offset..offset + 2].copy_from_slice(&value.to_le_bytes())
            }
            ByteOrderType::BigEndian => {
                data[offset..offset + 2].copy_from_slice(&value.to_be_bytes())
            }
        };

    // Closure to write u32 respecting byte order
    let write_u32 =
        |data: &mut [u8], offset: usize, value: u32, byte_order: ByteOrderType| match byte_order {
            ByteOrderType::LittleEndian => {
                data[offset..offset + 4].copy_from_slice(&value.to_le_bytes())
            }
            ByteOrderType::BigEndian => {
                data[offset..offset + 4].copy_from_slice(&value.to_be_bytes())
            }
        };

    // Header
    match byte_order {
        ByteOrderType::LittleEndian => {
            data[0] = b'I';
            data[1] = b'I';
        }
        ByteOrderType::BigEndian => {
            data[0] = b'M';
            data[1] = b'M';
        }
    }
    write_u16(&mut data, 2, 42, byte_order); // Version
    write_u32(&mut data, 4, 8, byte_order); // First IFD offset

    // IFD at offset 8
    write_u16(&mut data, 8, 9, byte_order); // Entry count (9 entries including SamplesPerPixel)

    let mut offset = 10;

    // Helper to write IFD entry
    // For inline values, SHORT values need to be left-justified in the 4-byte value field
    let mut write_entry =
        |data: &mut [u8], offset: &mut usize, tag: u16, typ: u16, count: u32, value: u32| {
            write_u16(data, *offset, tag, byte_order);
            write_u16(data, *offset + 2, typ, byte_order);
            write_u32(data, *offset + 4, count, byte_order);

            // Write value field - for SHORT type (3), left-justify the value
            if typ == 3 && count == 1 {
                // SHORT value: store as u16 in first 2 bytes, then pad with zeros
                write_u16(data, *offset + 8, value as u16, byte_order);
                data[*offset + 10] = 0;
                data[*offset + 11] = 0;
            } else {
                // LONG value or offset: store as u32
                write_u32(data, *offset + 8, value, byte_order);
            }
            *offset += 12;
        };

    // Entries sorted by tag number
    // ImageWidth (256) = 2048
    write_entry(&mut data, &mut offset, 256, 4, 1, 2048);
    // ImageLength (257) = 1536
    write_entry(&mut data, &mut offset, 257, 4, 1, 1536);
    // BitsPerSample (258) = 8
    write_entry(&mut data, &mut offset, 258, 3, 1, 8);
    // Compression (259) = 7 (JPEG)
    write_entry(&mut data, &mut offset, 259, 3, 1, 7);
    // SamplesPerPixel (277) = 1
    write_entry(&mut data, &mut offset, 277, 3, 1, 1);
    // TileWidth (322) = 256
    write_entry(&mut data, &mut offset, 322, 4, 1, 256);
    // TileLength (323) = 256
    write_entry(&mut data, &mut offset, 323, 4, 1, 256);
    // TileOffsets (324)
    write_entry(
        &mut data,
        &mut offset,
        324,
        4,
        tile_count,
        tile_offsets_offset,
    );
    // TileByteCounts (325)
    write_entry(
        &mut data,
        &mut offset,
        325,
        4,
        tile_count,
        tile_byte_counts_offset,
    );

    // Next IFD offset (0 = no more IFDs)
    write_u32(&mut data, offset, 0, byte_order);

    // Write tile offsets array (all point to same tile data)
    for i in 0..tile_count {
        let arr_offset = tile_offsets_offset as usize + (i as usize) * 4;
        write_u32(&mut data, arr_offset, tile_data_offset, byte_order);
    }

    // Write tile byte counts array
    for i in 0..tile_count {
        let arr_offset = tile_byte_counts_offset as usize + (i as usize) * 4;
        write_u32(&mut data, arr_offset, jpeg_len, byte_order);
    }

    // Write the actual JPEG tile data
    data[tile_data_offset as usize..tile_data_offset as usize + jpeg_data.len()]
        .copy_from_slice(&jpeg_data);

    data
}

/// Create a BigTIFF file with JPEG tile data.
pub fn create_bigtiff_with_jpeg_tile() -> Vec<u8> {
    let jpeg_data = create_test_jpeg(256, 256, 90);
    let jpeg_len = jpeg_data.len() as u64;

    // BigTIFF layout:
    // 0-15: Header (II/MM + 43 + 8 + 0 + first_ifd_offset)
    // 16-x: IFD
    // After IFD: arrays and tile data

    let tile_count = 48u64; // 8x6 tiles

    let mut data = Vec::new();

    // Header - Little endian BigTIFF
    data.extend(b"II"); // Little endian
    data.extend(&43u16.to_le_bytes()); // BigTIFF version
    data.extend(&8u16.to_le_bytes()); // Offset byte size
    data.extend(&0u16.to_le_bytes()); // Reserved
    data.extend(&16u64.to_le_bytes()); // First IFD offset

    // IFD at offset 16
    let entry_count = 8u64;
    data.extend(&entry_count.to_le_bytes()); // Entry count (8 bytes for BigTIFF)

    // BigTIFF entries are 20 bytes each:
    // tag (2) + type (2) + count (8) + value/offset (8)

    let write_entry = |data: &mut Vec<u8>, tag: u16, typ: u16, count: u64, value: u64| {
        data.extend(&tag.to_le_bytes());
        data.extend(&typ.to_le_bytes());
        data.extend(&count.to_le_bytes());
        data.extend(&value.to_le_bytes());
    };

    // Calculate offsets
    let ifd_end = 16 + 8 + (entry_count as usize * 20) + 8; // header + count + entries + next_ifd
    let tile_offsets_offset = ifd_end as u64;
    let tile_byte_counts_offset = tile_offsets_offset + tile_count * 8;
    let tile_data_offset = tile_byte_counts_offset + tile_count * 8;

    // ImageWidth (2048)
    write_entry(&mut data, 256, 4, 1, 2048);
    // ImageLength (1536)
    write_entry(&mut data, 257, 4, 1, 1536);
    // BitsPerSample
    write_entry(&mut data, 258, 3, 1, 8);
    // Compression (7 = JPEG)
    write_entry(&mut data, 259, 3, 1, 7);
    // TileWidth (256)
    write_entry(&mut data, 322, 3, 1, 256);
    // TileLength (256)
    write_entry(&mut data, 323, 3, 1, 256);
    // TileOffsets (LONG8 = type 16 for BigTIFF)
    write_entry(&mut data, 324, 16, tile_count, tile_offsets_offset);
    // TileByteCounts (LONG8 = type 16 for BigTIFF)
    write_entry(&mut data, 325, 16, tile_count, tile_byte_counts_offset);

    // Next IFD offset (0 = no more IFDs)
    data.extend(&0u64.to_le_bytes());

    // Write tile offsets array (all point to same tile data)
    for _ in 0..tile_count {
        data.extend(&tile_data_offset.to_le_bytes());
    }

    // Write tile byte counts array
    for _ in 0..tile_count {
        data.extend(&jpeg_len.to_le_bytes());
    }

    // Write the actual JPEG tile data
    data.extend(&jpeg_data);

    data
}

/// Create a TIFF file with unsupported LZW compression.
pub fn create_tiff_with_lzw_compression() -> Vec<u8> {
    let mut data = create_tiff_with_jpeg_tile();

    // Change compression tag value from 7 (JPEG) to 5 (LZW)
    // The compression entry is at offset 10 + 3*12 + 8 = 10 + 36 + 8 = 54
    // Actually, let's find it more carefully...
    // Entry format: tag(2) + type(2) + count(4) + value(4) = 12 bytes
    // Entry 3 (0-indexed) is compression at offset 10 + 3*12 = 46
    // Value is at offset 46 + 8 = 54

    // After looking at the structure:
    // IFD starts at offset 8
    // Entry count: 2 bytes
    // Entries start at offset 10
    // Entry 0: ImageWidth (tag 256)
    // Entry 1: ImageLength (tag 257)
    // Entry 2: BitsPerSample (tag 258)
    // Entry 3: Compression (tag 259) at offset 10 + 3*12 = 46
    // Value/offset field at 46 + 8 = 54

    data[54] = 5; // LZW compression

    data
}

/// Create a TIFF file with strip organization (not tiled).
pub fn create_strip_tiff() -> Vec<u8> {
    let jpeg_data = create_test_jpeg(256, 256, 90);
    let jpeg_len = jpeg_data.len() as u32;

    let strip_offset = 200u32;
    let total_size = strip_offset as usize + jpeg_data.len() + 100;
    let mut data = vec![0u8; total_size];

    // Header
    data[0] = b'I';
    data[1] = b'I';
    data[2..4].copy_from_slice(&42u16.to_le_bytes());
    data[4..8].copy_from_slice(&8u32.to_le_bytes());

    // IFD at offset 8
    data[8..10].copy_from_slice(&8u16.to_le_bytes()); // 8 entries

    let mut offset = 10;

    let write_entry =
        |data: &mut [u8], offset: &mut usize, tag: u16, typ: u16, count: u32, value: u32| {
            data[*offset..*offset + 2].copy_from_slice(&tag.to_le_bytes());
            data[*offset + 2..*offset + 4].copy_from_slice(&typ.to_le_bytes());
            data[*offset + 4..*offset + 8].copy_from_slice(&count.to_le_bytes());
            data[*offset + 8..*offset + 12].copy_from_slice(&value.to_le_bytes());
            *offset += 12;
        };

    // Entries sorted by tag
    // ImageWidth (256)
    write_entry(&mut data, &mut offset, 256, 4, 1, 512);
    // ImageLength (257)
    write_entry(&mut data, &mut offset, 257, 4, 1, 512);
    // BitsPerSample (258)
    write_entry(&mut data, &mut offset, 258, 3, 1, 8);
    // Compression (259) = JPEG
    write_entry(&mut data, &mut offset, 259, 3, 1, 7);
    // StripOffsets (273) instead of TileOffsets
    write_entry(&mut data, &mut offset, 273, 4, 1, strip_offset);
    // SamplesPerPixel (277)
    write_entry(&mut data, &mut offset, 277, 3, 1, 1);
    // RowsPerStrip (278)
    write_entry(&mut data, &mut offset, 278, 4, 1, 512);
    // StripByteCounts (279) instead of TileByteCounts
    write_entry(&mut data, &mut offset, 279, 4, 1, jpeg_len);

    // Next IFD offset
    data[offset..offset + 4].copy_from_slice(&0u32.to_le_bytes());

    // Write strip data
    data[strip_offset as usize..strip_offset as usize + jpeg_data.len()]
        .copy_from_slice(&jpeg_data);

    data
}

// =============================================================================
// SVS-specific Test Data
// =============================================================================

/// Create an SVS-like TIFF with JPEGTables (abbreviated JPEG streams).
///
/// This simulates how Aperio SVS files store JPEG data without embedded tables.
pub fn create_svs_with_jpeg_tables() -> Vec<u8> {
    // Create a full JPEG first
    let full_jpeg = create_test_jpeg(256, 256, 90);

    // Extract JPEG tables (everything between SOI and SOS, excluding SOI)
    // and the scan data (from SOS to EOI)
    let (tables, scan_data) = split_jpeg_stream(&full_jpeg);

    // Create TIFF with JPEGTables tag
    let mut data = Vec::new();

    // We'll store:
    // - Header at 0
    // - IFD at 8
    // - JPEGTables data after IFD
    // - Tile data after JPEGTables

    let jpeg_tables = create_jpeg_tables_blob(&tables);

    // Calculate offsets
    let ifd_offset = 8u32;
    let entry_count = 9; // Standard entries + JPEGTables
    let ifd_size = 2 + entry_count * 12 + 4;
    let arrays_offset = ifd_offset as usize + ifd_size;

    let tile_count = 48u32;
    let tile_offsets_offset = arrays_offset as u32;
    let tile_byte_counts_offset = tile_offsets_offset + tile_count * 4;
    let jpeg_tables_offset = tile_byte_counts_offset + tile_count * 4;
    let tile_data_offset = jpeg_tables_offset + jpeg_tables.len() as u32;

    // Prepare abbreviated tile data (just SOI + scan data + EOI)
    let abbreviated_tile = create_abbreviated_jpeg(&scan_data);
    let tile_len = abbreviated_tile.len() as u32;

    let total_size = tile_data_offset as usize + abbreviated_tile.len() + 100;
    data.resize(total_size, 0);

    // Header
    data[0] = b'I';
    data[1] = b'I';
    data[2..4].copy_from_slice(&42u16.to_le_bytes());
    data[4..8].copy_from_slice(&ifd_offset.to_le_bytes());

    // IFD
    let mut offset = ifd_offset as usize;
    data[offset..offset + 2].copy_from_slice(&(entry_count as u16).to_le_bytes());
    offset += 2;

    let write_entry =
        |data: &mut [u8], offset: &mut usize, tag: u16, typ: u16, count: u32, value: u32| {
            data[*offset..*offset + 2].copy_from_slice(&tag.to_le_bytes());
            data[*offset + 2..*offset + 4].copy_from_slice(&typ.to_le_bytes());
            data[*offset + 4..*offset + 8].copy_from_slice(&count.to_le_bytes());
            data[*offset + 8..*offset + 12].copy_from_slice(&value.to_le_bytes());
            *offset += 12;
        };

    // Entries (sorted by tag)
    write_entry(&mut data, &mut offset, 256, 4, 1, 2048); // ImageWidth
    write_entry(&mut data, &mut offset, 257, 4, 1, 1536); // ImageLength
    write_entry(&mut data, &mut offset, 258, 3, 1, 8); // BitsPerSample
    write_entry(&mut data, &mut offset, 259, 3, 1, 7); // Compression = JPEG
    write_entry(&mut data, &mut offset, 322, 3, 1, 256); // TileWidth
    write_entry(&mut data, &mut offset, 323, 3, 1, 256); // TileLength
    write_entry(
        &mut data,
        &mut offset,
        324,
        4,
        tile_count,
        tile_offsets_offset,
    ); // TileOffsets
    write_entry(
        &mut data,
        &mut offset,
        325,
        4,
        tile_count,
        tile_byte_counts_offset,
    ); // TileByteCounts
    write_entry(
        &mut data,
        &mut offset,
        347,
        7, // UNDEFINED type
        jpeg_tables.len() as u32,
        jpeg_tables_offset,
    ); // JPEGTables

    // Next IFD
    data[offset..offset + 4].copy_from_slice(&0u32.to_le_bytes());

    // Tile offsets
    for i in 0..tile_count {
        let arr_offset = tile_offsets_offset as usize + (i as usize) * 4;
        data[arr_offset..arr_offset + 4].copy_from_slice(&tile_data_offset.to_le_bytes());
    }

    // Tile byte counts
    for i in 0..tile_count {
        let arr_offset = tile_byte_counts_offset as usize + (i as usize) * 4;
        data[arr_offset..arr_offset + 4].copy_from_slice(&tile_len.to_le_bytes());
    }

    // JPEGTables
    data[jpeg_tables_offset as usize..jpeg_tables_offset as usize + jpeg_tables.len()]
        .copy_from_slice(&jpeg_tables);

    // Tile data
    data[tile_data_offset as usize..tile_data_offset as usize + abbreviated_tile.len()]
        .copy_from_slice(&abbreviated_tile);

    data
}

/// Split a JPEG stream into tables and scan data.
fn split_jpeg_stream(jpeg: &[u8]) -> (Vec<u8>, Vec<u8>) {
    // Find SOS marker (0xFFDA)
    let mut i = 2; // Skip SOI
    while i < jpeg.len() - 1 {
        if jpeg[i] == 0xFF && jpeg[i + 1] == 0xDA {
            // Found SOS
            let tables = jpeg[2..i].to_vec(); // Everything between SOI and SOS
            let scan_data = jpeg[i..jpeg.len() - 2].to_vec(); // SOS to before EOI
            return (tables, scan_data);
        }

        if jpeg[i] == 0xFF && jpeg[i + 1] != 0x00 && jpeg[i + 1] != 0xFF {
            // Marker found, skip segment
            if i + 4 <= jpeg.len() {
                let len = u16::from_be_bytes([jpeg[i + 2], jpeg[i + 3]]) as usize;
                i += 2 + len;
            } else {
                break;
            }
        } else {
            i += 1;
        }
    }

    // Fallback: return everything as tables
    (jpeg[2..jpeg.len() - 2].to_vec(), vec![])
}

/// Create a JPEGTables blob (SOI + tables + EOI).
fn create_jpeg_tables_blob(tables: &[u8]) -> Vec<u8> {
    let mut blob = vec![0xFF, 0xD8]; // SOI
    blob.extend(tables);
    blob.extend(&[0xFF, 0xD9]); // EOI
    blob
}

/// Create an abbreviated JPEG stream (SOI + scan data + EOI).
fn create_abbreviated_jpeg(scan_data: &[u8]) -> Vec<u8> {
    let mut abbreviated = vec![0xFF, 0xD8]; // SOI
    abbreviated.extend(scan_data);
    abbreviated.extend(&[0xFF, 0xD9]); // EOI
    abbreviated
}

// =============================================================================
// Validation Helpers
// =============================================================================

/// Check if data is a valid JPEG.
pub fn is_valid_jpeg(data: &[u8]) -> bool {
    if data.len() < 4 {
        return false;
    }

    // Check SOI marker
    if data[0] != 0xFF || data[1] != 0xD8 {
        return false;
    }

    // Check EOI marker at end
    if data[data.len() - 2] != 0xFF || data[data.len() - 1] != 0xD9 {
        return false;
    }

    // Try to decode it
    image::load_from_memory_with_format(data, image::ImageFormat::Jpeg).is_ok()
}

/// Check if data starts with TIFF magic bytes.
pub fn is_tiff_magic(data: &[u8]) -> bool {
    if data.len() < 4 {
        return false;
    }

    (data[0] == b'I' && data[1] == b'I' && data[2] == 42 && data[3] == 0)
        || (data[0] == b'M' && data[1] == b'M' && data[2] == 0 && data[3] == 42)
}

/// Check if data starts with BigTIFF magic bytes.
pub fn is_bigtiff_magic(data: &[u8]) -> bool {
    if data.len() < 8 {
        return false;
    }

    if data[0] == b'I' && data[1] == b'I' {
        let version = u16::from_le_bytes([data[2], data[3]]);
        version == 43
    } else if data[0] == b'M' && data[1] == b'M' {
        let version = u16::from_be_bytes([data[2], data[3]]);
        version == 43
    } else {
        false
    }
}
