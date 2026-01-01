//! Integration tests for the /slides endpoint.
//!
//! These tests verify:
//! - Slides listing returns correct results
//! - Extension filtering (.svs, .tif, .tiff)
//! - Pagination with limit parameter
//! - Authentication requirements
//! - Empty bucket handling

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use wsi_streamer::slide::SlideRegistry;
use wsi_streamer::tile::TileService;
use wsi_streamer::{create_router, RouterConfig, SignedUrlAuth};

use super::test_utils::{create_tiff_with_jpeg_tile, MockSlideSource};

// =============================================================================
// Basic Functionality Tests
// =============================================================================

#[tokio::test]
async fn test_slides_list_empty() {
    let source = MockSlideSource::new();
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    let request = Request::builder()
        .uri("/slides")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let result: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert!(result["slides"].as_array().unwrap().is_empty());
    assert!(result.get("next_cursor").is_none());
}

#[tokio::test]
async fn test_slides_list_with_results() {
    let tiff_data = create_tiff_with_jpeg_tile();

    let source = MockSlideSource::new()
        .with_slide("slide1.svs", tiff_data.clone())
        .with_slide("slide2.tif", tiff_data.clone())
        .with_slide("folder/slide3.tiff", tiff_data);

    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    let request = Request::builder()
        .uri("/slides")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let result: serde_json::Value = serde_json::from_slice(&body).unwrap();

    let slides = result["slides"].as_array().unwrap();
    assert_eq!(slides.len(), 3);

    // Check that all expected slides are present
    let slide_names: Vec<&str> = slides.iter().map(|s| s.as_str().unwrap()).collect();
    assert!(slide_names.contains(&"slide1.svs"));
    assert!(slide_names.contains(&"slide2.tif"));
    assert!(slide_names.contains(&"folder/slide3.tiff"));
}

// =============================================================================
// Extension Filtering Tests
// =============================================================================

#[tokio::test]
async fn test_slides_list_filters_extensions() {
    let tiff_data = create_tiff_with_jpeg_tile();

    let source = MockSlideSource::new()
        .with_slide("valid.svs", tiff_data.clone())
        .with_slide("valid.tif", tiff_data.clone())
        .with_slide("valid.tiff", tiff_data.clone())
        .with_slide("invalid.jpg", tiff_data.clone())
        .with_slide("invalid.pdf", tiff_data.clone())
        .with_slide("invalid.png", tiff_data);

    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    let request = Request::builder()
        .uri("/slides")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let result: serde_json::Value = serde_json::from_slice(&body).unwrap();

    let slides = result["slides"].as_array().unwrap();
    assert_eq!(slides.len(), 3);

    // Only .svs, .tif, .tiff should be included
    let slide_names: Vec<&str> = slides.iter().map(|s| s.as_str().unwrap()).collect();
    assert!(slide_names.contains(&"valid.svs"));
    assert!(slide_names.contains(&"valid.tif"));
    assert!(slide_names.contains(&"valid.tiff"));
    assert!(!slide_names.contains(&"invalid.jpg"));
    assert!(!slide_names.contains(&"invalid.pdf"));
    assert!(!slide_names.contains(&"invalid.png"));
}

#[tokio::test]
async fn test_slides_list_case_insensitive_extensions() {
    let tiff_data = create_tiff_with_jpeg_tile();

    let source = MockSlideSource::new()
        .with_slide("lower.svs", tiff_data.clone())
        .with_slide("UPPER.SVS", tiff_data.clone())
        .with_slide("Mixed.Tif", tiff_data.clone())
        .with_slide("mixed.TIFF", tiff_data);

    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    let request = Request::builder()
        .uri("/slides")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let result: serde_json::Value = serde_json::from_slice(&body).unwrap();

    let slides = result["slides"].as_array().unwrap();
    assert_eq!(slides.len(), 4);
}

// =============================================================================
// Pagination Tests
// =============================================================================

#[tokio::test]
async fn test_slides_list_with_limit() {
    let tiff_data = create_tiff_with_jpeg_tile();

    let source = MockSlideSource::new()
        .with_slide("slide1.svs", tiff_data.clone())
        .with_slide("slide2.svs", tiff_data.clone())
        .with_slide("slide3.svs", tiff_data.clone())
        .with_slide("slide4.svs", tiff_data.clone())
        .with_slide("slide5.svs", tiff_data);

    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    let request = Request::builder()
        .uri("/slides?limit=2")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let result: serde_json::Value = serde_json::from_slice(&body).unwrap();

    let slides = result["slides"].as_array().unwrap();
    assert_eq!(slides.len(), 2);

    // Should have a next_cursor since there are more results
    assert!(result.get("next_cursor").is_some());
}

#[tokio::test]
async fn test_slides_list_limit_clamped_to_max() {
    let tiff_data = create_tiff_with_jpeg_tile();

    let source = MockSlideSource::new()
        .with_slide("slide1.svs", tiff_data.clone())
        .with_slide("slide2.svs", tiff_data);

    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    // Request with limit > 1000 should be clamped
    let request = Request::builder()
        .uri("/slides?limit=5000")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let result: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Should still work (limit clamped to 1000)
    let slides = result["slides"].as_array().unwrap();
    assert_eq!(slides.len(), 2);
}

#[tokio::test]
async fn test_slides_list_default_limit() {
    let tiff_data = create_tiff_with_jpeg_tile();

    // Create more slides than default limit but we only have a few for test
    let source = MockSlideSource::new()
        .with_slide("slide1.svs", tiff_data.clone())
        .with_slide("slide2.svs", tiff_data);

    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    // No limit parameter - should use default (100)
    let request = Request::builder()
        .uri("/slides")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let result: serde_json::Value = serde_json::from_slice(&body).unwrap();

    let slides = result["slides"].as_array().unwrap();
    assert_eq!(slides.len(), 2);
}

// =============================================================================
// Authentication Tests
// =============================================================================

#[tokio::test]
async fn test_slides_list_requires_auth_when_enabled() {
    let tiff_data = create_tiff_with_jpeg_tile();

    let source = MockSlideSource::new().with_slide("slide.svs", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);

    // Enable authentication
    let config = RouterConfig::new("test-secret");
    let router = create_router(tile_service, config);

    // Request without auth should fail
    let request = Request::builder()
        .uri("/slides")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_slides_list_with_valid_auth() {
    let tiff_data = create_tiff_with_jpeg_tile();

    let source = MockSlideSource::new().with_slide("slide.svs", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);

    let secret = "test-secret";
    let config = RouterConfig::new(secret);
    let router = create_router(tile_service, config);

    // Generate signed URL
    let auth = SignedUrlAuth::new(secret);
    let signed_url =
        auth.generate_signed_url("", "/slides", std::time::Duration::from_secs(300), &[]);

    let request = Request::builder()
        .uri(&signed_url)
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let result: serde_json::Value = serde_json::from_slice(&body).unwrap();

    let slides = result["slides"].as_array().unwrap();
    assert_eq!(slides.len(), 1);
}

// =============================================================================
// Response Format Tests
// =============================================================================

#[tokio::test]
async fn test_slides_response_format() {
    let tiff_data = create_tiff_with_jpeg_tile();

    let source = MockSlideSource::new().with_slide("slide.svs", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    let request = Request::builder()
        .uri("/slides")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();

    // Check content type
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "application/json"
    );

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let result: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Verify response structure
    assert!(result.get("slides").is_some());
    assert!(result["slides"].is_array());
}

#[tokio::test]
async fn test_slides_response_no_cursor_when_all_returned() {
    let tiff_data = create_tiff_with_jpeg_tile();

    let source = MockSlideSource::new()
        .with_slide("slide1.svs", tiff_data.clone())
        .with_slide("slide2.svs", tiff_data);

    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    let request = Request::builder()
        .uri("/slides?limit=100")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let result: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // next_cursor should not be present when all results are returned
    assert!(result.get("next_cursor").is_none());
}
