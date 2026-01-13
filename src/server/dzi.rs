//! Deep Zoom Image (DZI) compatibility module.
//!
//! This module provides DZI-compatible endpoints for integration with
//! OpenSeadragon and other Deep Zoom-compatible viewers.
//!
//! # DZI Format Overview
//!
//! Deep Zoom uses an inverted level numbering compared to WSI pyramids:
//! - DZI level 0 = 1x1 pixel (lowest resolution)
//! - DZI max level = full resolution (highest resolution)
//!
//! WSI pyramids typically use:
//! - Level 0 = full resolution (highest resolution)
//! - Level N = lowest resolution
//!
//! This module handles the level mapping between these two conventions.

/// Generate DZI XML descriptor for a slide.
///
/// # Example Output
///
/// ```xml
/// <?xml version="1.0" encoding="UTF-8"?>
/// <Image xmlns="http://schemas.microsoft.com/deepzoom/2008"
///        TileSize="256"
///        Overlap="0"
///        Format="jpg">
///   <Size Width="46920" Height="33600" />
/// </Image>
/// ```
pub fn generate_dzi_xml(width: u32, height: u32, tile_size: u32) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Image xmlns="http://schemas.microsoft.com/deepzoom/2008"
       TileSize="{tile_size}"
       Overlap="0"
       Format="jpg">
  <Size Width="{width}" Height="{height}" />
</Image>"#
    )
}

/// Calculate the maximum DZI level for given image dimensions.
///
/// DZI levels go from 0 (1x1) to max_level (full resolution).
/// max_level = ceil(log2(max(width, height)))
pub fn calculate_max_dzi_level(width: u32, height: u32) -> usize {
    let max_dim = width.max(height) as f64;
    if max_dim <= 1.0 {
        return 0;
    }
    max_dim.log2().ceil() as usize
}

/// Calculate dimensions at a specific DZI level.
///
/// At DZI level L, the dimensions are:
/// - width = ceil(original_width / 2^(max_level - L))
/// - height = ceil(original_height / 2^(max_level - L))
pub fn dzi_level_dimensions(
    width: u32,
    height: u32,
    dzi_level: usize,
    max_dzi_level: usize,
) -> (u32, u32) {
    if dzi_level > max_dzi_level {
        return (0, 0);
    }

    let scale = 1u32 << (max_dzi_level - dzi_level);
    let level_width = width.div_ceil(scale);
    let level_height = height.div_ceil(scale);

    (level_width.max(1), level_height.max(1))
}

/// Calculate the downsample factor for a DZI level.
///
/// Returns the factor by which the full resolution is downsampled at this level.
pub fn dzi_level_downsample(dzi_level: usize, max_dzi_level: usize) -> f64 {
    if dzi_level > max_dzi_level {
        return 0.0;
    }
    (1u64 << (max_dzi_level - dzi_level)) as f64
}

/// Find the best WSI level for a given DZI level.
///
/// Given a list of WSI level downsamples (level 0 = 1.0, higher levels = larger downsamples),
/// finds the WSI level that best matches the requested DZI level.
///
/// Returns `(wsi_level, additional_scale)` where:
/// - `wsi_level` is the WSI pyramid level to read from
/// - `additional_scale` is any additional downsampling needed
pub fn find_best_wsi_level(
    wsi_level_downsamples: &[f64],
    dzi_downsample: f64,
) -> Option<(usize, f64)> {
    if wsi_level_downsamples.is_empty() {
        return None;
    }

    // Find the WSI level with the smallest downsample that is >= dzi_downsample
    // (i.e., the highest resolution level that doesn't need upscaling)
    let mut best_level = 0;
    let mut best_downsample = wsi_level_downsamples[0];

    for (level, &downsample) in wsi_level_downsamples.iter().enumerate() {
        if downsample <= dzi_downsample && downsample >= best_downsample {
            best_level = level;
            best_downsample = downsample;
        }
    }

    // Calculate additional scale factor needed
    let additional_scale = dzi_downsample / best_downsample;

    Some((best_level, additional_scale))
}

/// Parse DZI tile coordinates from a filename like "3_5.jpg" or "3_5".
///
/// Returns `(x, y)` coordinates.
pub fn parse_dzi_tile_coords(filename: &str) -> Option<(u32, u32)> {
    // Strip .jpg or .jpeg extension if present
    let name = filename
        .strip_suffix(".jpg")
        .or_else(|| filename.strip_suffix(".jpeg"))
        .unwrap_or(filename);

    // Parse "x_y" format
    let parts: Vec<&str> = name.split('_').collect();
    if parts.len() != 2 {
        return None;
    }

    let x: u32 = parts[0].parse().ok()?;
    let y: u32 = parts[1].parse().ok()?;

    Some((x, y))
}

/// Calculate tile count at a DZI level.
pub fn dzi_tile_count(level_width: u32, level_height: u32, tile_size: u32) -> (u32, u32) {
    let tiles_x = level_width.div_ceil(tile_size);
    let tiles_y = level_height.div_ceil(tile_size);
    (tiles_x.max(1), tiles_y.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_dzi_xml() {
        let xml = generate_dzi_xml(46920, 33600, 256);

        assert!(xml.contains("TileSize=\"256\""));
        assert!(xml.contains("Width=\"46920\""));
        assert!(xml.contains("Height=\"33600\""));
        assert!(xml.contains("Format=\"jpg\""));
        assert!(xml.contains("Overlap=\"0\""));
        assert!(xml.contains("xmlns=\"http://schemas.microsoft.com/deepzoom/2008\""));
    }

    #[test]
    fn test_calculate_max_dzi_level() {
        // 1x1 image -> level 0
        assert_eq!(calculate_max_dzi_level(1, 1), 0);

        // 2x2 image -> level 1 (log2(2) = 1)
        assert_eq!(calculate_max_dzi_level(2, 2), 1);

        // 256x256 -> level 8 (log2(256) = 8)
        assert_eq!(calculate_max_dzi_level(256, 256), 8);

        // 46920x33600 -> level 16 (log2(46920) ≈ 15.52, ceil = 16)
        assert_eq!(calculate_max_dzi_level(46920, 33600), 16);

        // 1024x768 -> level 10 (log2(1024) = 10)
        assert_eq!(calculate_max_dzi_level(1024, 768), 10);

        // Non-power-of-two: 1000x500 -> level 10 (log2(1000) ≈ 9.97, ceil = 10)
        assert_eq!(calculate_max_dzi_level(1000, 500), 10);
    }

    #[test]
    fn test_dzi_level_dimensions() {
        let width = 1024u32;
        let height = 768u32;
        let max_level = calculate_max_dzi_level(width, height); // 10

        // Level 10 (max) = full resolution
        assert_eq!(
            dzi_level_dimensions(width, height, 10, max_level),
            (1024, 768)
        );

        // Level 9 = half resolution
        assert_eq!(
            dzi_level_dimensions(width, height, 9, max_level),
            (512, 384)
        );

        // Level 8 = quarter resolution
        assert_eq!(
            dzi_level_dimensions(width, height, 8, max_level),
            (256, 192)
        );

        // Level 0 = 1x1 (or close to it)
        let (w, h) = dzi_level_dimensions(width, height, 0, max_level);
        assert!(w <= 2 && h <= 2);
    }

    #[test]
    fn test_dzi_level_dimensions_small_image() {
        // 100x50 image, max_level = 7 (log2(100) ≈ 6.64, ceil = 7)
        let max_level = calculate_max_dzi_level(100, 50);
        assert_eq!(max_level, 7);

        // Level 7 = full res
        assert_eq!(dzi_level_dimensions(100, 50, 7, max_level), (100, 50));

        // Level 6 = half
        assert_eq!(dzi_level_dimensions(100, 50, 6, max_level), (50, 25));

        // Level 0
        let (w, h) = dzi_level_dimensions(100, 50, 0, max_level);
        assert_eq!((w, h), (1, 1));
    }

    #[test]
    fn test_dzi_level_downsample() {
        let max_level = 10usize;

        // Max level = 1x (full res)
        assert_eq!(dzi_level_downsample(10, max_level), 1.0);

        // Level 9 = 2x downsampled
        assert_eq!(dzi_level_downsample(9, max_level), 2.0);

        // Level 8 = 4x downsampled
        assert_eq!(dzi_level_downsample(8, max_level), 4.0);

        // Level 0 = 1024x downsampled
        assert_eq!(dzi_level_downsample(0, max_level), 1024.0);
    }

    #[test]
    fn test_find_best_wsi_level() {
        // WSI levels: 0 = 1x, 1 = 4x, 2 = 16x
        let wsi_downsamples = vec![1.0, 4.0, 16.0];

        // DZI wants 1x -> WSI level 0, scale 1.0
        assert_eq!(find_best_wsi_level(&wsi_downsamples, 1.0), Some((0, 1.0)));

        // DZI wants 2x -> WSI level 0, scale 2.0 (need to downsample from level 0)
        assert_eq!(find_best_wsi_level(&wsi_downsamples, 2.0), Some((0, 2.0)));

        // DZI wants 4x -> WSI level 1, scale 1.0
        assert_eq!(find_best_wsi_level(&wsi_downsamples, 4.0), Some((1, 1.0)));

        // DZI wants 8x -> WSI level 1, scale 2.0
        assert_eq!(find_best_wsi_level(&wsi_downsamples, 8.0), Some((1, 2.0)));

        // DZI wants 16x -> WSI level 2, scale 1.0
        assert_eq!(find_best_wsi_level(&wsi_downsamples, 16.0), Some((2, 1.0)));

        // DZI wants 32x -> WSI level 2, scale 2.0
        assert_eq!(find_best_wsi_level(&wsi_downsamples, 32.0), Some((2, 2.0)));

        // Empty levels
        assert_eq!(find_best_wsi_level(&[], 1.0), None);
    }

    #[test]
    fn test_parse_dzi_tile_coords() {
        assert_eq!(parse_dzi_tile_coords("0_0.jpg"), Some((0, 0)));
        assert_eq!(parse_dzi_tile_coords("3_5.jpg"), Some((3, 5)));
        assert_eq!(parse_dzi_tile_coords("10_20.jpeg"), Some((10, 20)));
        assert_eq!(parse_dzi_tile_coords("0_0"), Some((0, 0)));
        assert_eq!(parse_dzi_tile_coords("123_456"), Some((123, 456)));

        // Invalid formats
        assert_eq!(parse_dzi_tile_coords("invalid"), None);
        assert_eq!(parse_dzi_tile_coords("0-0.jpg"), None);
        assert_eq!(parse_dzi_tile_coords("a_b.jpg"), None);
        assert_eq!(parse_dzi_tile_coords("0_0_0.jpg"), None);
    }

    #[test]
    fn test_dzi_tile_count() {
        // 1024x768 with 256 tile size
        assert_eq!(dzi_tile_count(1024, 768, 256), (4, 3));

        // Non-exact division
        assert_eq!(dzi_tile_count(1000, 500, 256), (4, 2));

        // Single tile
        assert_eq!(dzi_tile_count(100, 100, 256), (1, 1));

        // Exact fit
        assert_eq!(dzi_tile_count(512, 512, 256), (2, 2));
    }

    #[test]
    fn test_dzi_level_out_of_bounds() {
        let max_level = calculate_max_dzi_level(1024, 768);

        // Level beyond max should return (0, 0)
        assert_eq!(
            dzi_level_dimensions(1024, 768, max_level + 1, max_level),
            (0, 0)
        );
        assert_eq!(dzi_level_dimensions(1024, 768, 100, max_level), (0, 0));

        // Downsample for invalid level
        assert_eq!(dzi_level_downsample(max_level + 1, max_level), 0.0);
    }
}
