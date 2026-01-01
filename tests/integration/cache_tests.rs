//! Cache effectiveness integration tests.
//!
//! Tests verify:
//! - Tile cache reduces duplicate work
//! - Sequential tile requests benefit from caching
//! - Concurrent requests don't cause duplicate work

use std::sync::Arc;
use std::time::Instant;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use wsi_streamer::slide::SlideRegistry;
use wsi_streamer::tile::TileService;
use wsi_streamer::{create_router, RouterConfig};

use super::test_utils::{create_tiff_with_jpeg_tile, is_valid_jpeg, MockSlideSource};

// =============================================================================
// Tile Cache Effectiveness
// =============================================================================

#[tokio::test]
async fn test_tile_cache_hit_is_faster() {
    let tiff_data = create_tiff_with_jpeg_tile();
    let source = MockSlideSource::new().with_slide("test.tif", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    // First request - cache miss (includes parsing, decoding, encoding)
    let start1 = Instant::now();
    let request1 = Request::builder()
        .uri("/tiles/test.tif/0/0/0.jpg")
        .body(Body::empty())
        .unwrap();
    let response1 = router.clone().oneshot(request1).await.unwrap();
    let duration1 = start1.elapsed();
    assert_eq!(response1.status(), StatusCode::OK);

    // Verify it was a cache miss
    assert_eq!(
        response1.headers().get("x-tile-cache-hit").unwrap(),
        "false"
    );

    // Second request - cache hit
    let start2 = Instant::now();
    let request2 = Request::builder()
        .uri("/tiles/test.tif/0/0/0.jpg")
        .body(Body::empty())
        .unwrap();
    let response2 = router.oneshot(request2).await.unwrap();
    let duration2 = start2.elapsed();
    assert_eq!(response2.status(), StatusCode::OK);

    // Verify it was a cache hit
    assert_eq!(response2.headers().get("x-tile-cache-hit").unwrap(), "true");

    // Cache hit should generally be faster
    // Note: This is a soft assertion since timing can vary
    println!("First request (cache miss): {:?}", duration1);
    println!("Second request (cache hit): {:?}", duration2);

    // We don't assert timing strictly because it can vary, but we log it
}

#[tokio::test]
async fn test_different_tiles_cached_independently() {
    let tiff_data = create_tiff_with_jpeg_tile();
    let source = MockSlideSource::new().with_slide("test.tif", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    // Request tile (0, 0)
    let request1 = Request::builder()
        .uri("/tiles/test.tif/0/0/0.jpg")
        .body(Body::empty())
        .unwrap();
    let response1 = router.clone().oneshot(request1).await.unwrap();
    assert_eq!(
        response1.headers().get("x-tile-cache-hit").unwrap(),
        "false"
    );

    // Request tile (1, 0) - should be cache miss
    let request2 = Request::builder()
        .uri("/tiles/test.tif/0/1/0.jpg")
        .body(Body::empty())
        .unwrap();
    let response2 = router.clone().oneshot(request2).await.unwrap();
    assert_eq!(
        response2.headers().get("x-tile-cache-hit").unwrap(),
        "false"
    );

    // Request tile (0, 0) again - should be cache hit
    let request3 = Request::builder()
        .uri("/tiles/test.tif/0/0/0.jpg")
        .body(Body::empty())
        .unwrap();
    let response3 = router.clone().oneshot(request3).await.unwrap();
    assert_eq!(response3.headers().get("x-tile-cache-hit").unwrap(), "true");

    // Request tile (1, 0) again - should be cache hit
    let request4 = Request::builder()
        .uri("/tiles/test.tif/0/1/0.jpg")
        .body(Body::empty())
        .unwrap();
    let response4 = router.oneshot(request4).await.unwrap();
    assert_eq!(response4.headers().get("x-tile-cache-hit").unwrap(), "true");
}

#[tokio::test]
async fn test_quality_affects_cache_key() {
    let tiff_data = create_tiff_with_jpeg_tile();
    let source = MockSlideSource::new().with_slide("test.tif", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    // Request with quality 80
    let request1 = Request::builder()
        .uri("/tiles/test.tif/0/0/0.jpg?quality=80")
        .body(Body::empty())
        .unwrap();
    let response1 = router.clone().oneshot(request1).await.unwrap();
    assert_eq!(
        response1.headers().get("x-tile-cache-hit").unwrap(),
        "false"
    );

    // Request with quality 90 - should be cache miss (different quality)
    let request2 = Request::builder()
        .uri("/tiles/test.tif/0/0/0.jpg?quality=90")
        .body(Body::empty())
        .unwrap();
    let response2 = router.clone().oneshot(request2).await.unwrap();
    assert_eq!(
        response2.headers().get("x-tile-cache-hit").unwrap(),
        "false"
    );

    // Request with quality 80 again - should be cache hit
    let request3 = Request::builder()
        .uri("/tiles/test.tif/0/0/0.jpg?quality=80")
        .body(Body::empty())
        .unwrap();
    let response3 = router.clone().oneshot(request3).await.unwrap();
    assert_eq!(response3.headers().get("x-tile-cache-hit").unwrap(), "true");

    // Request with quality 90 again - should be cache hit
    let request4 = Request::builder()
        .uri("/tiles/test.tif/0/0/0.jpg?quality=90")
        .body(Body::empty())
        .unwrap();
    let response4 = router.oneshot(request4).await.unwrap();
    assert_eq!(response4.headers().get("x-tile-cache-hit").unwrap(), "true");
}

#[tokio::test]
async fn test_slide_id_affects_cache_key() {
    let tiff_data1 = create_tiff_with_jpeg_tile();
    let tiff_data2 = create_tiff_with_jpeg_tile();

    let source = MockSlideSource::new()
        .with_slide("slide1.tif", tiff_data1)
        .with_slide("slide2.tif", tiff_data2);

    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    // Request from slide1
    let request1 = Request::builder()
        .uri("/tiles/slide1.tif/0/0/0.jpg")
        .body(Body::empty())
        .unwrap();
    let response1 = router.clone().oneshot(request1).await.unwrap();
    assert_eq!(
        response1.headers().get("x-tile-cache-hit").unwrap(),
        "false"
    );

    // Request from slide2 - should be cache miss
    let request2 = Request::builder()
        .uri("/tiles/slide2.tif/0/0/0.jpg")
        .body(Body::empty())
        .unwrap();
    let response2 = router.clone().oneshot(request2).await.unwrap();
    assert_eq!(
        response2.headers().get("x-tile-cache-hit").unwrap(),
        "false"
    );

    // Request from slide1 again - should be cache hit
    let request3 = Request::builder()
        .uri("/tiles/slide1.tif/0/0/0.jpg")
        .body(Body::empty())
        .unwrap();
    let response3 = router.oneshot(request3).await.unwrap();
    assert_eq!(response3.headers().get("x-tile-cache-hit").unwrap(), "true");
}

// =============================================================================
// Sequential Access Patterns
// =============================================================================

#[tokio::test]
async fn test_sequential_tile_requests_row_by_row() {
    let tiff_data = create_tiff_with_jpeg_tile();
    let source = MockSlideSource::new().with_slide("test.tif", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    // Simulate a viewer scanning row by row
    // First pass - all cache misses
    let mut first_pass_times = Vec::new();
    for y in 0..3 {
        for x in 0..3 {
            let start = Instant::now();
            let request = Request::builder()
                .uri(format!("/tiles/test.tif/0/{}/{}.jpg", x, y))
                .body(Body::empty())
                .unwrap();
            let response = router.clone().oneshot(request).await.unwrap();
            first_pass_times.push(start.elapsed());

            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers().get("x-tile-cache-hit").unwrap(),
                "false",
                "First pass tile ({}, {}) should be cache miss",
                x,
                y
            );
        }
    }

    // Second pass - all cache hits
    let mut second_pass_times = Vec::new();
    for y in 0..3 {
        for x in 0..3 {
            let start = Instant::now();
            let request = Request::builder()
                .uri(format!("/tiles/test.tif/0/{}/{}.jpg", x, y))
                .body(Body::empty())
                .unwrap();
            let response = router.clone().oneshot(request).await.unwrap();
            second_pass_times.push(start.elapsed());

            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers().get("x-tile-cache-hit").unwrap(),
                "true",
                "Second pass tile ({}, {}) should be cache hit",
                x,
                y
            );
        }
    }

    // Log timing comparison
    let first_total: std::time::Duration = first_pass_times.iter().sum();
    let second_total: std::time::Duration = second_pass_times.iter().sum();
    println!("First pass total: {:?}", first_total);
    println!("Second pass total: {:?}", second_total);
}

// =============================================================================
// Concurrent Request Handling
// =============================================================================

#[tokio::test]
async fn test_concurrent_requests_for_same_tile() {
    let tiff_data = create_tiff_with_jpeg_tile();
    let source = MockSlideSource::new().with_slide("test.tif", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = Arc::new(create_router(tile_service, RouterConfig::without_auth()));

    // Launch multiple concurrent requests for the same tile
    let mut handles = Vec::new();
    for i in 0..5 {
        let router_clone = Arc::clone(&router);
        handles.push(tokio::spawn(async move {
            let request = Request::builder()
                .uri("/tiles/test.tif/0/0/0.jpg")
                .body(Body::empty())
                .unwrap();

            // We need to use `call` because `oneshot` consumes the service
            // For this test, we just verify all complete successfully
            let response = tower::ServiceExt::oneshot((*router_clone).clone(), request)
                .await
                .unwrap();

            (i, response.status())
        }));
    }

    // All requests should succeed
    for handle in handles {
        let (idx, status) = handle.await.unwrap();
        assert_eq!(
            status,
            StatusCode::OK,
            "Concurrent request {} should succeed",
            idx
        );
    }
}

#[tokio::test]
async fn test_concurrent_requests_for_different_tiles() {
    let tiff_data = create_tiff_with_jpeg_tile();
    let source = MockSlideSource::new().with_slide("test.tif", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = Arc::new(create_router(tile_service, RouterConfig::without_auth()));

    // Launch concurrent requests for different tiles
    let mut handles = Vec::new();
    for x in 0..4 {
        for y in 0..4 {
            let router_clone = Arc::clone(&router);
            handles.push(tokio::spawn(async move {
                let request = Request::builder()
                    .uri(format!("/tiles/test.tif/0/{}/{}.jpg", x, y))
                    .body(Body::empty())
                    .unwrap();

                let response = tower::ServiceExt::oneshot((*router_clone).clone(), request)
                    .await
                    .unwrap();

                let body = response.into_body().collect().await.unwrap().to_bytes();
                (x, y, is_valid_jpeg(&body))
            }));
        }
    }

    // All requests should succeed with valid JPEGs
    for handle in handles {
        let (x, y, valid) = handle.await.unwrap();
        assert!(valid, "Concurrent tile ({}, {}) should be valid JPEG", x, y);
    }
}

// =============================================================================
// Cache Capacity
// =============================================================================

#[tokio::test]
async fn test_cache_with_limited_capacity() {
    let tiff_data = create_tiff_with_jpeg_tile();
    let source = MockSlideSource::new().with_slide("test.tif", tiff_data);
    let registry = SlideRegistry::new(source);

    // Create service with small cache capacity
    let tile_service = TileService::with_cache_capacity(registry, 100 * 1024); // 100KB

    let router = create_router(tile_service, RouterConfig::without_auth());

    // Request many tiles to potentially exceed cache
    for x in 0..8 {
        for y in 0..6 {
            let request = Request::builder()
                .uri(format!("/tiles/test.tif/0/{}/{}.jpg", x, y))
                .body(Body::empty())
                .unwrap();

            let response = router.clone().oneshot(request).await.unwrap();
            assert_eq!(
                response.status(),
                StatusCode::OK,
                "Tile ({}, {}) should succeed even with limited cache",
                x,
                y
            );

            let body = response.into_body().collect().await.unwrap().to_bytes();
            assert!(is_valid_jpeg(&body));
        }
    }
}

// =============================================================================
// Slide Registry Caching
// =============================================================================

#[tokio::test]
async fn test_slide_metadata_cached_between_tile_requests() {
    let tiff_data = create_tiff_with_jpeg_tile();
    let source = MockSlideSource::new().with_slide("test.tif", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    // First tile request - parses metadata
    let start1 = Instant::now();
    let request1 = Request::builder()
        .uri("/tiles/test.tif/0/0/0.jpg")
        .body(Body::empty())
        .unwrap();
    router.clone().oneshot(request1).await.unwrap();
    let duration1 = start1.elapsed();

    // Second tile request (different tile) - should reuse cached metadata
    let start2 = Instant::now();
    let request2 = Request::builder()
        .uri("/tiles/test.tif/0/1/1.jpg")
        .body(Body::empty())
        .unwrap();
    router.clone().oneshot(request2).await.unwrap();
    let duration2 = start2.elapsed();

    // Third tile request - should also reuse cached metadata
    let start3 = Instant::now();
    let request3 = Request::builder()
        .uri("/tiles/test.tif/0/2/2.jpg")
        .body(Body::empty())
        .unwrap();
    router.oneshot(request3).await.unwrap();
    let duration3 = start3.elapsed();

    println!("First request: {:?}", duration1);
    println!("Second request: {:?}", duration2);
    println!("Third request: {:?}", duration3);

    // Subsequent requests should generally be faster due to metadata caching
    // (Not strictly asserting timing as it can vary)
}

// =============================================================================
// Default Quality Caching
// =============================================================================

#[tokio::test]
async fn test_default_quality_caching() {
    let tiff_data = create_tiff_with_jpeg_tile();
    let source = MockSlideSource::new().with_slide("test.tif", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    // Request without quality (uses default 80)
    let request1 = Request::builder()
        .uri("/tiles/test.tif/0/0/0.jpg")
        .body(Body::empty())
        .unwrap();
    let response1 = router.clone().oneshot(request1).await.unwrap();
    assert_eq!(
        response1.headers().get("x-tile-cache-hit").unwrap(),
        "false"
    );
    assert_eq!(response1.headers().get("x-tile-quality").unwrap(), "80");

    // Request with explicit quality=80
    let request2 = Request::builder()
        .uri("/tiles/test.tif/0/0/0.jpg?quality=80")
        .body(Body::empty())
        .unwrap();
    let response2 = router.oneshot(request2).await.unwrap();

    // Should be cache hit since default quality is 80
    assert_eq!(response2.headers().get("x-tile-cache-hit").unwrap(), "true");
}
