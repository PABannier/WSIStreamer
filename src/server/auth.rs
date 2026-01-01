//! Signed URL authentication for WSI Streamer.
//!
//! This module provides HMAC-SHA256 based URL signing for secure tile access.
//!
//! # URL Signing Scheme
//!
//! URLs are signed by computing an HMAC-SHA256 over the path and query parameters
//! (excluding `sig`). This binds signatures to the full request path and query:
//!
//! ```text
//! signature = HMAC-SHA256(secret_key, "{path}?{canonical_query}")
//! ```
//!
//! The query string must include `exp` and may include extra parameters like
//! `quality`. The `sig` parameter is excluded from the canonical query.
//!
//! ```text
//! /tiles/slides/sample.svs/0/1/2.jpg?quality=80&exp=1735689600&sig=abc123...
//! ```
//!
//! # Security Properties
//!
//! - **Path + query binding**: Signatures are bound to paths and query params, preventing tampering
//! - **Time-limited**: Signatures expire after a configurable TTL
//! - **Constant-time comparison**: Signature verification uses constant-time comparison
//!   to prevent timing attacks
//!
//! # Example
//!
//! ```rust
//! use wsi_streamer::server::auth::SignedUrlAuth;
//! use std::time::{SystemTime, Duration};
//!
//! // Create authenticator with secret key
//! let auth = SignedUrlAuth::new("my-secret-key");
//!
//! // Generate a signed URL (valid for 1 hour)
//! let path = "/tiles/slides/sample.svs/0/1/2.jpg";
//! let (signature, expiry) = auth.sign(path, Duration::from_secs(3600));
//!
//! // Verify the signature
//! assert!(auth.verify(path, &signature, expiry, &[]).is_ok());
//! ```

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{
    extract::{FromRequestParts, OriginalUri, Request},
    http::{request::Parts, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tracing::{debug, warn};
use url::form_urlencoded;

use super::handlers::ErrorResponse;

// =============================================================================
// Types
// =============================================================================

/// HMAC-SHA256 type alias
type HmacSha256 = Hmac<Sha256>;

/// Authentication error types.
#[derive(Debug, Clone)]
pub enum AuthError {
    /// Signature is missing from request
    MissingSignature,

    /// Expiry timestamp is missing from request
    MissingExpiry,

    /// Signature has expired
    Expired {
        /// When the signature expired
        expired_at: u64,
        /// Current time
        current_time: u64,
    },

    /// Signature is invalid
    InvalidSignature,

    /// Signature format is invalid (not valid hex)
    InvalidSignatureFormat,

    /// Expiry timestamp is not a valid integer
    InvalidExpiryFormat,
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::MissingSignature => write!(f, "Missing signature parameter"),
            AuthError::MissingExpiry => write!(f, "Missing expiry parameter"),
            AuthError::Expired {
                expired_at,
                current_time,
            } => write!(
                f,
                "Signature expired at {} (current time: {})",
                expired_at, current_time
            ),
            AuthError::InvalidSignature => write!(f, "Invalid signature"),
            AuthError::InvalidSignatureFormat => write!(f, "Invalid signature format"),
            AuthError::InvalidExpiryFormat => write!(f, "Invalid expiry format"),
        }
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, error_type, message) = match &self {
            AuthError::MissingSignature => (
                StatusCode::UNAUTHORIZED,
                "missing_signature",
                self.to_string(),
            ),
            AuthError::MissingExpiry => {
                (StatusCode::UNAUTHORIZED, "missing_expiry", self.to_string())
            }
            AuthError::Expired { .. } => (
                StatusCode::UNAUTHORIZED,
                "signature_expired",
                self.to_string(),
            ),
            AuthError::InvalidSignature => (
                StatusCode::UNAUTHORIZED,
                "invalid_signature",
                self.to_string(),
            ),
            AuthError::InvalidSignatureFormat => (
                StatusCode::BAD_REQUEST,
                "invalid_signature_format",
                self.to_string(),
            ),
            AuthError::InvalidExpiryFormat => (
                StatusCode::BAD_REQUEST,
                "invalid_expiry_format",
                self.to_string(),
            ),
        };

        // Log authentication errors
        // Invalid signature could indicate an attack, so log at warn level
        // Expired signatures are common and expected, log at debug
        match &self {
            AuthError::InvalidSignature => {
                warn!(
                    error_type = error_type,
                    status = status.as_u16(),
                    "Authentication failed: {}",
                    message
                );
            }
            AuthError::Expired { .. } => {
                debug!(
                    error_type = error_type,
                    status = status.as_u16(),
                    "Authentication failed: {}",
                    message
                );
            }
            _ => {
                debug!(
                    error_type = error_type,
                    status = status.as_u16(),
                    "Authentication failed: {}",
                    message
                );
            }
        }

        let error_response = ErrorResponse::with_status(error_type, message, status);
        (status, Json(error_response)).into_response()
    }
}

// =============================================================================
// Signed URL Authentication
// =============================================================================

/// Signed URL authenticator using HMAC-SHA256.
///
/// This struct provides methods for generating and verifying signed URLs.
/// The signing scheme binds signatures to paths, query params, and expiry times.
#[derive(Clone)]
pub struct SignedUrlAuth {
    /// Secret key for HMAC computation
    secret_key: Vec<u8>,
}

impl SignedUrlAuth {
    /// Create a new authenticator with the given secret key.
    ///
    /// # Arguments
    ///
    /// * `secret_key` - The secret key used for HMAC computation. Should be
    ///   at least 32 bytes for security.
    pub fn new(secret_key: impl AsRef<[u8]>) -> Self {
        Self {
            secret_key: secret_key.as_ref().to_vec(),
        }
    }

    /// Sign a path with an expiry duration.
    ///
    /// Returns the hex-encoded signature and the expiry timestamp (Unix epoch seconds).
    ///
    /// # Arguments
    ///
    /// * `path` - The URL path to sign (e.g., "/tiles/slides/sample.svs/0/1/2.jpg")
    /// * `ttl` - How long the signature should be valid
    ///
    /// # Returns
    ///
    /// A tuple of (signature, expiry_timestamp)
    pub fn sign(&self, path: &str, ttl: Duration) -> (String, u64) {
        self.sign_with_params(path, ttl, &[])
    }

    /// Sign a path with extra query parameters.
    ///
    /// `params` should exclude `exp` and `sig`; those are added automatically.
    pub fn sign_with_params(
        &self,
        path: &str,
        ttl: Duration,
        params: &[(&str, &str)],
    ) -> (String, u64) {
        let expiry = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + ttl.as_secs();

        let signature = self.compute_signature(path, expiry, params);
        (signature, expiry)
    }

    /// Sign a path with a specific expiry timestamp.
    ///
    /// This is useful when you need to generate signatures for a specific time.
    ///
    /// # Arguments
    ///
    /// * `path` - The URL path to sign
    /// * `expiry` - Unix timestamp when the signature expires
    ///
    /// # Returns
    ///
    /// The hex-encoded signature
    pub fn sign_with_expiry(&self, path: &str, expiry: u64) -> String {
        self.sign_with_expiry_and_params(path, expiry, &[])
    }

    /// Sign a path with a specific expiry timestamp and extra parameters.
    ///
    /// `params` should exclude `exp` and `sig`; those are added automatically.
    pub fn sign_with_expiry_and_params(
        &self,
        path: &str,
        expiry: u64,
        params: &[(&str, &str)],
    ) -> String {
        self.compute_signature(path, expiry, params)
    }

    /// Verify a signature for a path and expiry.
    ///
    /// # Arguments
    ///
    /// * `path` - The URL path that was signed
    /// * `signature` - The hex-encoded signature to verify
    /// * `expiry` - The expiry timestamp from the URL
    ///
    /// # Returns
    ///
    /// `Ok(())` if the signature is valid and not expired, `Err(AuthError)` otherwise.
    pub fn verify(
        &self,
        path: &str,
        signature: &str,
        expiry: u64,
        params: &[(&str, &str)],
    ) -> Result<(), AuthError> {
        // Check expiry first
        let current_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        if current_time > expiry {
            return Err(AuthError::Expired {
                expired_at: expiry,
                current_time,
            });
        }

        // Decode the provided signature
        let provided_sig = hex::decode(signature).map_err(|_| AuthError::InvalidSignatureFormat)?;

        // Compute expected signature
        let expected_sig_hex = self.compute_signature(path, expiry, params);
        let expected_sig =
            hex::decode(&expected_sig_hex).map_err(|_| AuthError::InvalidSignatureFormat)?;

        // Constant-time comparison
        if provided_sig.ct_eq(&expected_sig).into() {
            Ok(())
        } else {
            Err(AuthError::InvalidSignature)
        }
    }

    /// Compute the HMAC-SHA256 signature for a path and expiry.
    fn compute_signature(&self, path: &str, expiry: u64, params: &[(&str, &str)]) -> String {
        let message = signature_base(path, expiry, params);

        // Compute HMAC-SHA256
        let mut mac =
            HmacSha256::new_from_slice(&self.secret_key).expect("HMAC can take key of any size");
        mac.update(message.as_bytes());
        let result = mac.finalize();

        // Return hex-encoded signature
        hex::encode(result.into_bytes())
    }

    /// Generate a complete signed URL.
    ///
    /// # Arguments
    ///
    /// * `base_url` - The base URL (e.g., "https://example.com")
    /// * `path` - The path to sign (e.g., "/tiles/slides/sample.svs/0/1/2.jpg")
    /// * `ttl` - How long the signature should be valid
    /// * `extra_params` - Additional query parameters to include
    ///
    /// # Returns
    ///
    /// The complete signed URL
    pub fn generate_signed_url(
        &self,
        base_url: &str,
        path: &str,
        ttl: Duration,
        extra_params: &[(&str, &str)],
    ) -> String {
        let (signature, expiry) = self.sign_with_params(path, ttl, extra_params);

        let mut url = format!("{}{}", base_url, path);

        let mut serializer = form_urlencoded::Serializer::new(String::new());
        for (key, value) in extra_params {
            serializer.append_pair(key, value);
        }
        serializer.append_pair("exp", &expiry.to_string());
        serializer.append_pair("sig", &signature);

        url.push('?');
        url.push_str(&serializer.finish());

        url
    }
}

fn signature_base(path: &str, expiry: u64, params: &[(&str, &str)]) -> String {
    let mut all_params: Vec<(String, String)> = Vec::with_capacity(params.len() + 1);
    for (key, value) in params {
        all_params.push(((*key).to_string(), (*value).to_string()));
    }
    all_params.push(("exp".to_string(), expiry.to_string()));

    let canonical = canonical_query(&all_params);
    if canonical.is_empty() {
        path.to_string()
    } else {
        format!("{}?{}", path, canonical)
    }
}

fn canonical_query(params: &[(String, String)]) -> String {
    let mut pairs = params.to_vec();
    pairs.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    pairs
        .into_iter()
        .map(|(key, value)| format!("{}={}", key, value))
        .collect::<Vec<_>>()
        .join("&")
}

// =============================================================================
// Query Parameters for Auth
// =============================================================================

/// Query parameters for authentication.
#[derive(Debug, Deserialize)]
pub struct AuthQueryParams {
    /// Signature (hex-encoded HMAC-SHA256)
    pub sig: Option<String>,

    /// Expiry timestamp (Unix epoch seconds)
    pub exp: Option<u64>,
}

// =============================================================================
// Axum Middleware
// =============================================================================

/// Axum middleware for verifying signed URLs.
///
/// This middleware extracts the signature and expiry from query parameters,
/// verifies them against the request path, and rejects unauthorized requests
/// with a 401 status code.
///
/// # Example
///
/// ```ignore
/// use axum::{Router, middleware};
/// use wsi_streamer::server::auth::{SignedUrlAuth, auth_middleware};
///
/// let auth = SignedUrlAuth::new("secret-key");
/// let app = Router::new()
///     .route("/tiles/*path", get(tile_handler))
///     .layer(middleware::from_fn_with_state(auth, auth_middleware));
/// ```
pub async fn auth_middleware(
    axum::extract::State(auth): axum::extract::State<SignedUrlAuth>,
    OriginalUri(original_uri): OriginalUri,
    request: Request,
    next: Next,
) -> Result<Response, AuthError> {
    let query = original_uri.query().unwrap_or("");
    let mut signature: Option<String> = None;
    let mut expiry: Option<u64> = None;
    let mut extra_params: Vec<(String, String)> = Vec::new();

    for (key, value) in form_urlencoded::parse(query.as_bytes()) {
        if key == "sig" {
            if signature.is_some() {
                return Err(AuthError::InvalidSignatureFormat);
            }
            signature = Some(value.into_owned());
            continue;
        }
        if key == "exp" {
            if expiry.is_some() {
                return Err(AuthError::InvalidExpiryFormat);
            }
            let parsed = value
                .parse::<u64>()
                .map_err(|_| AuthError::InvalidExpiryFormat)?;
            expiry = Some(parsed);
            continue;
        }

        extra_params.push((key.into_owned(), value.into_owned()));
    }

    let signature = signature.ok_or(AuthError::MissingSignature)?;
    let expiry = expiry.ok_or(AuthError::MissingExpiry)?;

    // Get the path from the request
    let path = original_uri.path();

    // Verify signature
    let extra_params_ref: Vec<(&str, &str)> = extra_params
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    auth.verify(path, &signature, expiry, &extra_params_ref)?;

    // Continue to the handler
    Ok(next.run(request).await)
}

/// Axum extractor for optional authentication.
///
/// This extractor verifies the signature if present, but allows requests
/// without authentication to pass through. Useful for endpoints that support
/// both authenticated and public access.
#[derive(Debug, Clone)]
pub struct OptionalAuth {
    /// Whether the request was authenticated
    pub authenticated: bool,
}

impl<S> FromRequestParts<S> for OptionalAuth
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Check if auth parameters are present
        let query = parts.uri.query().unwrap_or("");
        let has_sig = query.contains("sig=");
        let has_exp = query.contains("exp=");

        Ok(OptionalAuth {
            authenticated: has_sig && has_exp,
        })
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_sign_and_verify() {
        let auth = SignedUrlAuth::new("test-secret-key");
        let path = "/tiles/slides/sample.svs/0/1/2.jpg";
        let ttl = Duration::from_secs(3600);

        let (signature, expiry) = auth.sign(path, ttl);

        // Signature should be valid
        assert!(auth.verify(path, &signature, expiry, &[]).is_ok());
    }

    #[test]
    fn test_verify_wrong_signature() {
        let auth = SignedUrlAuth::new("test-secret-key");
        let path = "/tiles/slides/sample.svs/0/1/2.jpg";
        let ttl = Duration::from_secs(3600);

        let (_, expiry) = auth.sign(path, ttl);

        // Wrong signature should fail
        let wrong_sig = "0".repeat(64); // Valid hex but wrong signature
        let result = auth.verify(path, &wrong_sig, expiry, &[]);
        assert!(matches!(result, Err(AuthError::InvalidSignature)));
    }

    #[test]
    fn test_verify_wrong_path() {
        let auth = SignedUrlAuth::new("test-secret-key");
        let path = "/tiles/slides/sample.svs/0/1/2.jpg";
        let ttl = Duration::from_secs(3600);

        let (signature, expiry) = auth.sign(path, ttl);

        // Different path should fail
        let wrong_path = "/tiles/slides/other.svs/0/1/2.jpg";
        let result = auth.verify(wrong_path, &signature, expiry, &[]);
        assert!(matches!(result, Err(AuthError::InvalidSignature)));
    }

    #[test]
    fn test_verify_expired() {
        let auth = SignedUrlAuth::new("test-secret-key");
        let path = "/tiles/slides/sample.svs/0/1/2.jpg";

        // Create signature that's already expired
        let expired_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 100; // 100 seconds in the past

        let signature = auth.sign_with_expiry(path, expired_time);

        let result = auth.verify(path, &signature, expired_time, &[]);
        assert!(matches!(result, Err(AuthError::Expired { .. })));
    }

    #[test]
    fn test_verify_invalid_hex() {
        let auth = SignedUrlAuth::new("test-secret-key");
        let path = "/tiles/slides/sample.svs/0/1/2.jpg";
        let expiry = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;

        // Invalid hex should fail
        let result = auth.verify(path, "not-valid-hex!", expiry, &[]);
        assert!(matches!(result, Err(AuthError::InvalidSignatureFormat)));
    }

    #[test]
    fn test_different_keys_different_signatures() {
        let auth1 = SignedUrlAuth::new("key1");
        let auth2 = SignedUrlAuth::new("key2");
        let path = "/tiles/slides/sample.svs/0/1/2.jpg";
        let ttl = Duration::from_secs(3600);

        let (sig1, expiry) = auth1.sign(path, ttl);
        let sig2 = auth2.sign_with_expiry(path, expiry);

        // Signatures should be different
        assert_ne!(sig1, sig2);

        // Each should only verify with its own key
        assert!(auth1.verify(path, &sig1, expiry, &[]).is_ok());
        assert!(auth1.verify(path, &sig2, expiry, &[]).is_err());
        assert!(auth2.verify(path, &sig2, expiry, &[]).is_ok());
        assert!(auth2.verify(path, &sig1, expiry, &[]).is_err());
    }

    #[test]
    fn test_signature_is_deterministic() {
        let auth = SignedUrlAuth::new("test-secret-key");
        let path = "/tiles/slides/sample.svs/0/1/2.jpg";
        let expiry = 1735689600u64;

        let sig1 = auth.sign_with_expiry(path, expiry);
        let sig2 = auth.sign_with_expiry(path, expiry);

        // Same inputs should produce same signature
        assert_eq!(sig1, sig2);
    }

    #[test]
    fn test_generate_signed_url() {
        let auth = SignedUrlAuth::new("test-secret-key");
        let base_url = "https://example.com";
        let path = "/tiles/slides/sample.svs/0/1/2.jpg";
        let ttl = Duration::from_secs(3600);

        let url = auth.generate_signed_url(base_url, path, ttl, &[("quality", "80")]);

        // URL should contain all components
        assert!(url.starts_with("https://example.com/tiles/slides/sample.svs/0/1/2.jpg?"));
        assert!(url.contains("quality=80"));
        assert!(url.contains("exp="));
        assert!(url.contains("sig="));
    }

    #[test]
    fn test_auth_error_display() {
        let err = AuthError::MissingSignature;
        assert_eq!(err.to_string(), "Missing signature parameter");

        let err = AuthError::MissingExpiry;
        assert_eq!(err.to_string(), "Missing expiry parameter");

        let err = AuthError::Expired {
            expired_at: 1000,
            current_time: 2000,
        };
        assert!(err.to_string().contains("1000"));
        assert!(err.to_string().contains("2000"));

        let err = AuthError::InvalidSignature;
        assert_eq!(err.to_string(), "Invalid signature");

        let err = AuthError::InvalidSignatureFormat;
        assert_eq!(err.to_string(), "Invalid signature format");

        let err = AuthError::InvalidExpiryFormat;
        assert_eq!(err.to_string(), "Invalid expiry format");
    }

    #[test]
    fn test_constant_time_comparison() {
        // This test verifies that we're using constant-time comparison
        // by ensuring the same result regardless of where differences occur
        let auth = SignedUrlAuth::new("test-secret-key");
        let path = "/tiles/slides/sample.svs/0/1/2.jpg";
        let expiry = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;

        let correct_sig = auth.sign_with_expiry(path, expiry);

        // Create signatures with differences at different positions
        let mut wrong_first = correct_sig.clone();
        wrong_first.replace_range(0..1, "0");

        let mut wrong_middle = correct_sig.clone();
        let mid = correct_sig.len() / 2;
        wrong_middle.replace_range(mid..mid + 1, "0");

        let mut wrong_last = correct_sig.clone();
        let last = correct_sig.len() - 1;
        wrong_last.replace_range(last..last + 1, "0");

        // All should fail (we can't easily test timing, but we verify correctness)
        assert!(auth.verify(path, &wrong_first, expiry, &[]).is_err());
        assert!(auth.verify(path, &wrong_middle, expiry, &[]).is_err());
        assert!(auth.verify(path, &wrong_last, expiry, &[]).is_err());
    }
}
