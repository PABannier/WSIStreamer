//! Format-specific integration tests.
//!
//! Tests verify:
//! - TIFF parser handles little-endian and big-endian files
//! - BigTIFF files are parsed correctly
//! - SVS JPEGTables handling works correctly
//! - Decoded tiles are valid images

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use wsi_streamer::slide::SlideRegistry;
use wsi_streamer::tile::TileService;
use wsi_streamer::{RouterConfig, create_router};

use super::test_utils::{
    ByteOrderType, MockSlideSource, create_bigtiff_with_jpeg_tile,
    create_svs_with_jpeg_tables, create_tiff_with_jpeg_tile,
    create_tiff_with_jpeg_tile_endian, is_bigtiff_magic, is_tiff_magic, is_valid_jpeg,
};

// =============================================================================
// TIFF Byte Order Tests
// =============================================================================

#[tokio::test]
async fn test_little_endian_tiff() {
    let tiff_data = create_tiff_with_jpeg_tile_endian(ByteOrderType::LittleEndian);

    // Verify it's little-endian
    assert_eq!(tiff_data[0], b'I');
    assert_eq!(tiff_data[1], b'I');
    assert!(is_tiff_magic(&tiff_data));

    let source = MockSlideSource::new().with_slide("le.tif", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    let request = Request::builder()
        .uri("/tiles/le.tif/0/0/0.jpg")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(is_valid_jpeg(&body), "Tile from little-endian TIFF should be valid JPEG");
}

#[tokio::test]
async fn test_big_endian_tiff() {
    let tiff_data = create_tiff_with_jpeg_tile_endian(ByteOrderType::BigEndian);

    // Verify it's big-endian
    assert_eq!(tiff_data[0], b'M');
    assert_eq!(tiff_data[1], b'M');
    assert!(is_tiff_magic(&tiff_data));

    let source = MockSlideSource::new().with_slide("be.tif", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    let request = Request::builder()
        .uri("/tiles/be.tif/0/0/0.jpg")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(is_valid_jpeg(&body), "Tile from big-endian TIFF should be valid JPEG");
}

#[tokio::test]
async fn test_both_byte_orders_produce_equivalent_results() {
    let le_tiff = create_tiff_with_jpeg_tile_endian(ByteOrderType::LittleEndian);
    let be_tiff = create_tiff_with_jpeg_tile_endian(ByteOrderType::BigEndian);

    let source = MockSlideSource::new()
        .with_slide("le.tif", le_tiff)
        .with_slide("be.tif", be_tiff);

    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    // Get tile from little-endian
    let request_le = Request::builder()
        .uri("/tiles/le.tif/0/0/0.jpg")
        .body(Body::empty())
        .unwrap();
    let response_le = router.clone().oneshot(request_le).await.unwrap();
    assert_eq!(response_le.status(), StatusCode::OK);
    let body_le = response_le.into_body().collect().await.unwrap().to_bytes();

    // Get tile from big-endian
    let request_be = Request::builder()
        .uri("/tiles/be.tif/0/0/0.jpg")
        .body(Body::empty())
        .unwrap();
    let response_be = router.oneshot(request_be).await.unwrap();
    assert_eq!(response_be.status(), StatusCode::OK);
    let body_be = response_be.into_body().collect().await.unwrap().to_bytes();

    // Both should be valid JPEGs
    assert!(is_valid_jpeg(&body_le));
    assert!(is_valid_jpeg(&body_be));

    // They should have similar sizes (exact match not guaranteed due to encoding)
    let size_diff = (body_le.len() as i64 - body_be.len() as i64).abs();
    assert!(
        size_diff < 1000,
        "Tiles from LE and BE TIFFs should have similar sizes"
    );
}

// =============================================================================
// BigTIFF Tests
// =============================================================================

#[tokio::test]
async fn test_bigtiff_parsing() {
    let bigtiff_data = create_bigtiff_with_jpeg_tile();

    // Verify it's BigTIFF
    assert!(is_bigtiff_magic(&bigtiff_data), "Should be valid BigTIFF header");

    let source = MockSlideSource::new().with_slide("big.tif", bigtiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    let request = Request::builder()
        .uri("/tiles/big.tif/0/0/0.jpg")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(is_valid_jpeg(&body), "Tile from BigTIFF should be valid JPEG");
}

#[tokio::test]
async fn test_bigtiff_multiple_tiles() {
    let bigtiff_data = create_bigtiff_with_jpeg_tile();
    let source = MockSlideSource::new().with_slide("big.tif", bigtiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    // Request multiple tiles from BigTIFF
    for x in 0..3 {
        for y in 0..3 {
            let request = Request::builder()
                .uri(format!("/tiles/big.tif/0/{}/{}.jpg", x, y))
                .body(Body::empty())
                .unwrap();

            let response = router.clone().oneshot(request).await.unwrap();
            assert_eq!(
                response.status(),
                StatusCode::OK,
                "BigTIFF tile ({}, {}) should succeed",
                x,
                y
            );
        }
    }
}

// =============================================================================
// SVS JPEGTables Tests
// =============================================================================

#[tokio::test]
async fn test_svs_with_jpeg_tables() {
    let svs_data = create_svs_with_jpeg_tables();

    let source = MockSlideSource::new().with_slide("slide.svs", svs_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    let request = Request::builder()
        .uri("/tiles/slide.svs/0/0/0.jpg")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(
        is_valid_jpeg(&body),
        "Tile from SVS with JPEGTables should be valid JPEG"
    );
}

#[tokio::test]
async fn test_svs_decoded_tiles_are_correct_size() {
    let svs_data = create_svs_with_jpeg_tables();
    let source = MockSlideSource::new().with_slide("slide.svs", svs_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    let request = Request::builder()
        .uri("/tiles/slide.svs/0/0/0.jpg")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    let body = response.into_body().collect().await.unwrap().to_bytes();

    // Decode the JPEG and verify dimensions
    let img = image::load_from_memory_with_format(&body, image::ImageFormat::Jpeg)
        .expect("Should be able to decode tile");

    // Our test tiles are 256x256
    assert_eq!(img.width(), 256);
    assert_eq!(img.height(), 256);
}

#[tokio::test]
async fn test_svs_multiple_tiles_all_valid() {
    let svs_data = create_svs_with_jpeg_tables();
    let source = MockSlideSource::new().with_slide("slide.svs", svs_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    // Request multiple tiles
    for x in 0..3 {
        for y in 0..3 {
            let request = Request::builder()
                .uri(format!("/tiles/slide.svs/0/{}/{}.jpg", x, y))
                .body(Body::empty())
                .unwrap();

            let response = router.clone().oneshot(request).await.unwrap();
            assert_eq!(
                response.status(),
                StatusCode::OK,
                "SVS tile ({}, {}) should succeed",
                x,
                y
            );

            let body = response.into_body().collect().await.unwrap().to_bytes();
            assert!(
                is_valid_jpeg(&body),
                "SVS tile ({}, {}) should be valid JPEG",
                x,
                y
            );
        }
    }
}

// =============================================================================
// Tile Data Integrity Tests
// =============================================================================

#[tokio::test]
async fn test_tile_jpeg_markers_correct() {
    let tiff_data = create_tiff_with_jpeg_tile();
    let source = MockSlideSource::new().with_slide("test.tif", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    let request = Request::builder()
        .uri("/tiles/test.tif/0/0/0.jpg")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    let body = response.into_body().collect().await.unwrap().to_bytes();

    // Check JPEG markers
    assert_eq!(body[0], 0xFF, "First byte should be 0xFF (JPEG marker prefix)");
    assert_eq!(body[1], 0xD8, "Second byte should be 0xD8 (SOI marker)");
    assert_eq!(
        body[body.len() - 2],
        0xFF,
        "Second-to-last byte should be 0xFF"
    );
    assert_eq!(body[body.len() - 1], 0xD9, "Last byte should be 0xD9 (EOI marker)");
}

#[tokio::test]
async fn test_different_quality_produces_different_sizes() {
    let tiff_data = create_tiff_with_jpeg_tile();
    let source = MockSlideSource::new().with_slide("test.tif", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    // Request low quality
    let request_low = Request::builder()
        .uri("/tiles/test.tif/0/0/0.jpg?quality=20")
        .body(Body::empty())
        .unwrap();
    let response_low = router.clone().oneshot(request_low).await.unwrap();
    let body_low = response_low.into_body().collect().await.unwrap().to_bytes();

    // Request high quality
    let request_high = Request::builder()
        .uri("/tiles/test.tif/0/1/0.jpg?quality=95")
        .body(Body::empty())
        .unwrap();
    let response_high = router.oneshot(request_high).await.unwrap();
    let body_high = response_high.into_body().collect().await.unwrap().to_bytes();

    // Both should be valid
    assert!(is_valid_jpeg(&body_low));
    assert!(is_valid_jpeg(&body_high));

    // Higher quality should typically be larger
    // (not always guaranteed but usually true for significant quality differences)
    // We just verify they're different
    assert_ne!(
        body_low.len(),
        body_high.len(),
        "Different quality should produce different file sizes"
    );
}

// =============================================================================
// Format Detection Tests
// =============================================================================

#[tokio::test]
async fn test_format_detection_generic_tiff() {
    let tiff_data = create_tiff_with_jpeg_tile();
    let source = MockSlideSource::new().with_slide("generic.tif", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    let request = Request::builder()
        .uri("/tiles/generic.tif/0/0/0.jpg")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_format_detection_svs_extension() {
    let svs_data = create_svs_with_jpeg_tables();
    let source = MockSlideSource::new().with_slide("slide.svs", svs_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    let request = Request::builder()
        .uri("/tiles/slide.svs/0/0/0.jpg")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// =============================================================================
// Edge Cases
// =============================================================================

#[tokio::test]
async fn test_first_and_last_tile_in_row() {
    let tiff_data = create_tiff_with_jpeg_tile();
    let source = MockSlideSource::new().with_slide("test.tif", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    // First tile (0, 0)
    let request_first = Request::builder()
        .uri("/tiles/test.tif/0/0/0.jpg")
        .body(Body::empty())
        .unwrap();
    let response_first = router.clone().oneshot(request_first).await.unwrap();
    assert_eq!(response_first.status(), StatusCode::OK);

    // Last tile in first row (assuming 8 tiles wide based on our test data)
    let request_last = Request::builder()
        .uri("/tiles/test.tif/0/7/0.jpg")
        .body(Body::empty())
        .unwrap();
    let response_last = router.oneshot(request_last).await.unwrap();
    assert_eq!(response_last.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_corner_tiles() {
    let tiff_data = create_tiff_with_jpeg_tile();
    let source = MockSlideSource::new().with_slide("test.tif", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    // Our test TIFF is 2048x1536 with 256x256 tiles = 8x6 tiles
    let corners = [(0, 0), (7, 0), (0, 5), (7, 5)];

    for (x, y) in corners {
        let request = Request::builder()
            .uri(format!("/tiles/test.tif/0/{}/{}.jpg", x, y))
            .body(Body::empty())
            .unwrap();

        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "Corner tile ({}, {}) should succeed",
            x,
            y
        );

        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert!(is_valid_jpeg(&body), "Corner tile ({}, {}) should be valid", x, y);
    }
}
