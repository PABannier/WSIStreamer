//! Authentication integration tests.
//!
//! Tests verify:
//! - Valid signed URLs work
//! - Expired signatures are rejected
//! - Invalid signatures are rejected
//! - Missing auth parameters are handled

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use wsi_streamer::slide::SlideRegistry;
use wsi_streamer::tile::TileService;
use wsi_streamer::{RouterConfig, SignedUrlAuth, create_router};

use super::test_utils::{MockSlideSource, create_tiff_with_jpeg_tile, is_valid_jpeg};

const TEST_SECRET: &str = "test-secret-key-for-hmac-signing";

// =============================================================================
// Valid Signatures
// =============================================================================

#[tokio::test]
async fn test_valid_signature_succeeds() {
    let tiff_data = create_tiff_with_jpeg_tile();
    let source = MockSlideSource::new().with_slide("test.tif", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::new(TEST_SECRET));

    // Generate a valid signed URL
    let auth = SignedUrlAuth::new(TEST_SECRET);
    let path = "/tiles/test.tif/0/0/0.jpg";
    let (signature, expiry) = auth.sign(path, Duration::from_secs(3600));

    let request = Request::builder()
        .uri(format!("{}?sig={}&exp={}", path, signature, expiry))
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(is_valid_jpeg(&body));
}

#[tokio::test]
async fn test_valid_signature_with_quality_param() {
    let tiff_data = create_tiff_with_jpeg_tile();
    let source = MockSlideSource::new().with_slide("test.tif", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::new(TEST_SECRET));

    // Generate signed URL
    let auth = SignedUrlAuth::new(TEST_SECRET);
    let path = "/tiles/test.tif/0/0/0.jpg";
    let (signature, expiry) = auth.sign(path, Duration::from_secs(3600));

    // Add quality parameter
    let request = Request::builder()
        .uri(format!("{}?quality=90&sig={}&exp={}", path, signature, expiry))
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers().get("x-tile-quality").unwrap(), "90");
}

// =============================================================================
// Expired Signatures
// =============================================================================

#[tokio::test]
async fn test_expired_signature_rejected() {
    let tiff_data = create_tiff_with_jpeg_tile();
    let source = MockSlideSource::new().with_slide("test.tif", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::new(TEST_SECRET));

    // Generate an already-expired signature
    let auth = SignedUrlAuth::new(TEST_SECRET);
    let path = "/tiles/test.tif/0/0/0.jpg";

    let expired_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        - 100; // 100 seconds in the past

    let signature = auth.sign_with_expiry(path, expired_time);

    let request = Request::builder()
        .uri(format!("{}?sig={}&exp={}", path, signature, expired_time))
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["error"], "signature_expired");
}

#[tokio::test]
async fn test_just_expired_signature_rejected() {
    let tiff_data = create_tiff_with_jpeg_tile();
    let source = MockSlideSource::new().with_slide("test.tif", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::new(TEST_SECRET));

    // Signature that expired 1 second ago
    let auth = SignedUrlAuth::new(TEST_SECRET);
    let path = "/tiles/test.tif/0/0/0.jpg";

    let expired_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        - 1;

    let signature = auth.sign_with_expiry(path, expired_time);

    let request = Request::builder()
        .uri(format!("{}?sig={}&exp={}", path, signature, expired_time))
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// =============================================================================
// Invalid Signatures
// =============================================================================

#[tokio::test]
async fn test_wrong_signature_rejected() {
    let tiff_data = create_tiff_with_jpeg_tile();
    let source = MockSlideSource::new().with_slide("test.tif", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::new(TEST_SECRET));

    let path = "/tiles/test.tif/0/0/0.jpg";
    let expiry = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;

    // Use a completely wrong signature (valid hex but wrong)
    let wrong_signature = "0".repeat(64);

    let request = Request::builder()
        .uri(format!("{}?sig={}&exp={}", path, wrong_signature, expiry))
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["error"], "invalid_signature");
}

#[tokio::test]
async fn test_signature_for_wrong_path_rejected() {
    let tiff_data = create_tiff_with_jpeg_tile();
    let source = MockSlideSource::new().with_slide("test.tif", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::new(TEST_SECRET));

    let auth = SignedUrlAuth::new(TEST_SECRET);

    // Sign for a different path
    let (signature, expiry) = auth.sign("/tiles/other.tif/0/0/0.jpg", Duration::from_secs(3600));

    // But request the original path
    let request = Request::builder()
        .uri(format!(
            "/tiles/test.tif/0/0/0.jpg?sig={}&exp={}",
            signature, expiry
        ))
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_signature_from_different_key_rejected() {
    let tiff_data = create_tiff_with_jpeg_tile();
    let source = MockSlideSource::new().with_slide("test.tif", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::new(TEST_SECRET));

    // Sign with a different key
    let wrong_auth = SignedUrlAuth::new("wrong-secret-key");
    let path = "/tiles/test.tif/0/0/0.jpg";
    let (signature, expiry) = wrong_auth.sign(path, Duration::from_secs(3600));

    let request = Request::builder()
        .uri(format!("{}?sig={}&exp={}", path, signature, expiry))
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_invalid_hex_signature_rejected() {
    let tiff_data = create_tiff_with_jpeg_tile();
    let source = MockSlideSource::new().with_slide("test.tif", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::new(TEST_SECRET));

    let path = "/tiles/test.tif/0/0/0.jpg";
    let expiry = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;

    // Invalid hex - note: URL encoded special characters
    let invalid_signature = "not-valid-hex";

    let request = Request::builder()
        .uri(format!("{}?sig={}&exp={}", path, invalid_signature, expiry))
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    // Invalid signature format currently returns 401 UNAUTHORIZED
    // because it's treated as an auth failure
    assert!(
        response.status() == StatusCode::BAD_REQUEST
        || response.status() == StatusCode::UNAUTHORIZED,
        "Expected 400 or 401, got {}",
        response.status()
    );
}

// =============================================================================
// Missing Auth Parameters
// =============================================================================

#[tokio::test]
async fn test_missing_signature_rejected() {
    let tiff_data = create_tiff_with_jpeg_tile();
    let source = MockSlideSource::new().with_slide("test.tif", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::new(TEST_SECRET));

    let expiry = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;

    // Only expiry, no signature
    let request = Request::builder()
        .uri(format!("/tiles/test.tif/0/0/0.jpg?exp={}", expiry))
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["error"], "missing_signature");
}

#[tokio::test]
async fn test_missing_expiry_rejected() {
    let tiff_data = create_tiff_with_jpeg_tile();
    let source = MockSlideSource::new().with_slide("test.tif", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::new(TEST_SECRET));

    // Only signature, no expiry
    let request = Request::builder()
        .uri("/tiles/test.tif/0/0/0.jpg?sig=abc123")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["error"], "missing_expiry");
}

#[tokio::test]
async fn test_no_auth_params_rejected() {
    let tiff_data = create_tiff_with_jpeg_tile();
    let source = MockSlideSource::new().with_slide("test.tif", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::new(TEST_SECRET));

    // No auth params at all
    let request = Request::builder()
        .uri("/tiles/test.tif/0/0/0.jpg")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// =============================================================================
// Health Endpoint Does Not Require Auth
// =============================================================================

#[tokio::test]
async fn test_health_endpoint_public() {
    let source = MockSlideSource::new();
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::new(TEST_SECRET));

    // Health endpoint should work without auth
    let request = Request::builder()
        .uri("/health")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// =============================================================================
// Auth Disabled Mode
// =============================================================================

#[tokio::test]
async fn test_auth_disabled_allows_unauthenticated() {
    let tiff_data = create_tiff_with_jpeg_tile();
    let source = MockSlideSource::new().with_slide("test.tif", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::without_auth());

    // Should work without auth when auth is disabled
    let request = Request::builder()
        .uri("/tiles/test.tif/0/0/0.jpg")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// =============================================================================
// Signature Verification is Path-Specific
// =============================================================================

#[tokio::test]
async fn test_signature_binds_to_exact_path() {
    let tiff_data = create_tiff_with_jpeg_tile();
    let source = MockSlideSource::new().with_slide("test.tif", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::new(TEST_SECRET));

    let auth = SignedUrlAuth::new(TEST_SECRET);

    // Sign for tile (0, 0)
    let (signature, expiry) = auth.sign("/tiles/test.tif/0/0/0.jpg", Duration::from_secs(3600));

    // Try to use it for tile (1, 1)
    let request = Request::builder()
        .uri(format!(
            "/tiles/test.tif/0/1/1.jpg?sig={}&exp={}",
            signature, expiry
        ))
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_signature_binds_to_level() {
    let tiff_data = create_tiff_with_jpeg_tile();
    let source = MockSlideSource::new().with_slide("test.tif", tiff_data);
    let registry = SlideRegistry::new(source);
    let tile_service = TileService::new(registry);
    let router = create_router(tile_service, RouterConfig::new(TEST_SECRET));

    let auth = SignedUrlAuth::new(TEST_SECRET);

    // Sign for level 0
    let (signature, expiry) = auth.sign("/tiles/test.tif/0/0/0.jpg", Duration::from_secs(3600));

    // Try to use it for level 1 (even though it doesn't exist)
    let request = Request::builder()
        .uri(format!(
            "/tiles/test.tif/1/0/0.jpg?sig={}&exp={}",
            signature, expiry
        ))
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
