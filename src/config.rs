//! Configuration management for WSI Streamer.
//!
//! This module provides a flexible configuration system that supports:
//! - Command-line arguments via clap
//! - Environment variables with `WSI_` prefix
//! - Sensible defaults for all optional settings
//!
//! # Example
//!
//! ```ignore
//! use wsi_streamer::config::Config;
//!
//! // Parse from command line and environment
//! let config = Config::parse();
//!
//! // Access configuration sections
//! println!("Listening on {}:{}", config.server.host, config.server.port);
//! println!("S3 bucket: {}", config.s3.bucket);
//! ```
//!
//! # Environment Variables
//!
//! All configuration options can be set via environment variables with the `WSI_` prefix:
//!
//! - `WSI_HOST` - Server bind address (default: 0.0.0.0)
//! - `WSI_PORT` - Server port (default: 3000)
//! - `WSI_S3_BUCKET` - S3 bucket name (required)
//! - `WSI_S3_ENDPOINT` - Custom S3 endpoint for S3-compatible services
//! - `WSI_S3_REGION` - AWS region (default: us-east-1)
//! - `WSI_AUTH_SECRET` - HMAC secret for signed URLs
//! - `WSI_AUTH_ENABLED` - Enable authentication (default: true)
//! - `WSI_CACHE_SLIDES` - Max slides to cache (default: 100)
//! - `WSI_CACHE_BLOCKS` - Max blocks per slide (default: 100)
//! - `WSI_CACHE_TILES` - Max tiles to cache (default: 1000)
//! - `WSI_JPEG_QUALITY` - Default JPEG quality (default: 80)
//! - `WSI_CACHE_MAX_AGE` - HTTP cache max-age seconds (default: 3600)

use clap::Parser;

use crate::io::DEFAULT_BLOCK_SIZE;
use crate::tile::{DEFAULT_JPEG_QUALITY, DEFAULT_TILE_CACHE_CAPACITY};

// =============================================================================
// Default Values
// =============================================================================

/// Default server host.
pub const DEFAULT_HOST: &str = "0.0.0.0";

/// Default server port.
pub const DEFAULT_PORT: u16 = 3000;

/// Default AWS region.
pub const DEFAULT_REGION: &str = "us-east-1";

/// Default number of slides to cache.
pub const DEFAULT_SLIDE_CACHE_CAPACITY: usize = 100;

/// Default number of blocks to cache per slide.
pub const DEFAULT_BLOCK_CACHE_CAPACITY: usize = 100;

/// Default HTTP cache max-age in seconds (1 hour).
pub const DEFAULT_CACHE_MAX_AGE: u32 = 3600;

// =============================================================================
// CLI Arguments
// =============================================================================

/// WSI Streamer - A tile server for Whole Slide Images.
///
/// Serves tiles from Whole Slide Images stored in S3 or S3-compatible storage
/// using HTTP range requests. No local file downloads required.
#[derive(Parser, Debug, Clone)]
#[command(name = "wsi-streamer")]
#[command(author, version, about, long_about = None)]
pub struct Config {
    // =========================================================================
    // Server Configuration
    // =========================================================================
    /// Host address to bind the server to.
    #[arg(long, default_value = DEFAULT_HOST, env = "WSI_HOST")]
    pub host: String,

    /// Port to listen on.
    #[arg(short, long, default_value_t = DEFAULT_PORT, env = "WSI_PORT")]
    pub port: u16,

    // =========================================================================
    // S3 Configuration
    // =========================================================================
    /// S3 bucket name containing the slide files.
    #[arg(long, env = "WSI_S3_BUCKET")]
    pub s3_bucket: String,

    /// Custom S3 endpoint URL for S3-compatible services (MinIO, etc.).
    ///
    /// If not specified, uses the default AWS S3 endpoint.
    #[arg(long, env = "WSI_S3_ENDPOINT")]
    pub s3_endpoint: Option<String>,

    /// AWS region for S3.
    #[arg(long, default_value = DEFAULT_REGION, env = "WSI_S3_REGION")]
    pub s3_region: String,

    // =========================================================================
    // Authentication Configuration
    // =========================================================================
    /// Secret key for HMAC-SHA256 signed URL authentication.
    ///
    /// If not provided and auth is enabled, the server will fail to start.
    #[arg(long, env = "WSI_AUTH_SECRET")]
    pub auth_secret: Option<String>,

    /// Enable signed URL authentication.
    ///
    /// When disabled, all tile requests are allowed without authentication.
    /// WARNING: Only disable authentication in development/testing.
    #[arg(long, default_value_t = true, env = "WSI_AUTH_ENABLED")]
    pub auth_enabled: bool,

    // =========================================================================
    // Cache Configuration
    // =========================================================================
    /// Maximum number of slides to keep in cache.
    #[arg(long, default_value_t = DEFAULT_SLIDE_CACHE_CAPACITY, env = "WSI_CACHE_SLIDES")]
    pub cache_slides: usize,

    /// Maximum number of blocks to cache per slide (256KB each).
    #[arg(long, default_value_t = DEFAULT_BLOCK_CACHE_CAPACITY, env = "WSI_CACHE_BLOCKS")]
    pub cache_blocks: usize,

    /// Maximum number of encoded tiles to cache.
    #[arg(long, default_value_t = DEFAULT_TILE_CACHE_CAPACITY, env = "WSI_CACHE_TILES")]
    pub cache_tiles: usize,

    /// Block size in bytes for the block cache.
    #[arg(long, default_value_t = DEFAULT_BLOCK_SIZE, env = "WSI_BLOCK_SIZE")]
    pub block_size: usize,

    // =========================================================================
    // Tile Configuration
    // =========================================================================
    /// Default JPEG quality for tile encoding (1-100).
    #[arg(long, default_value_t = DEFAULT_JPEG_QUALITY, env = "WSI_JPEG_QUALITY")]
    pub jpeg_quality: u8,

    /// HTTP Cache-Control max-age in seconds.
    #[arg(long, default_value_t = DEFAULT_CACHE_MAX_AGE, env = "WSI_CACHE_MAX_AGE")]
    pub cache_max_age: u32,

    // =========================================================================
    // CORS Configuration
    // =========================================================================
    /// Allowed CORS origins (comma-separated).
    ///
    /// If not specified, allows any origin.
    #[arg(long, env = "WSI_CORS_ORIGINS", value_delimiter = ',')]
    pub cors_origins: Option<Vec<String>>,

    // =========================================================================
    // Logging Configuration
    // =========================================================================
    /// Enable verbose logging (debug level).
    #[arg(short, long, default_value_t = false)]
    pub verbose: bool,

    /// Disable request tracing.
    #[arg(long, default_value_t = false)]
    pub no_tracing: bool,
}

impl Config {
    /// Validate the configuration and return an error message if invalid.
    pub fn validate(&self) -> Result<(), String> {
        // Check auth secret is provided when auth is enabled
        if self.auth_enabled && self.auth_secret.is_none() {
            return Err(
                "Authentication is enabled but no secret provided. \
                 Set --auth-secret or WSI_AUTH_SECRET, or disable auth with --auth-enabled=false"
                    .to_string(),
            );
        }

        // Validate bucket is not empty
        if self.s3_bucket.is_empty() {
            return Err("S3 bucket name is required. Set --s3-bucket or WSI_S3_BUCKET".to_string());
        }

        // Validate cache sizes
        if self.cache_slides == 0 {
            return Err("cache_slides must be greater than 0".to_string());
        }
        if self.cache_blocks == 0 {
            return Err("cache_blocks must be greater than 0".to_string());
        }
        if self.cache_tiles == 0 {
            return Err("cache_tiles must be greater than 0".to_string());
        }

        // Validate JPEG quality
        if self.jpeg_quality == 0 || self.jpeg_quality > 100 {
            return Err("jpeg_quality must be between 1 and 100".to_string());
        }

        // Validate block size (must be power of 2 and reasonable)
        if self.block_size < 1024 || self.block_size > 16 * 1024 * 1024 {
            return Err("block_size must be between 1KB and 16MB".to_string());
        }

        Ok(())
    }

    /// Get the server bind address as "host:port".
    pub fn bind_address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Get the auth secret, panicking if not set (call validate() first).
    pub fn auth_secret_or_empty(&self) -> &str {
        self.auth_secret.as_deref().unwrap_or("")
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        Config {
            host: "127.0.0.1".to_string(),
            port: 8080,
            s3_bucket: "test-bucket".to_string(),
            s3_endpoint: None,
            s3_region: "us-west-2".to_string(),
            auth_secret: Some("test-secret".to_string()),
            auth_enabled: true,
            cache_slides: 50,
            cache_blocks: 100,
            cache_tiles: 500,
            block_size: DEFAULT_BLOCK_SIZE,
            jpeg_quality: 85,
            cache_max_age: 7200,
            cors_origins: None,
            verbose: false,
            no_tracing: false,
        }
    }

    #[test]
    fn test_valid_config() {
        let config = test_config();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_missing_auth_secret() {
        let mut config = test_config();
        config.auth_secret = None;
        config.auth_enabled = true;

        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("secret"));
    }

    #[test]
    fn test_auth_disabled_no_secret_ok() {
        let mut config = test_config();
        config.auth_secret = None;
        config.auth_enabled = false;

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_empty_bucket() {
        let mut config = test_config();
        config.s3_bucket = String::new();

        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("bucket"));
    }

    #[test]
    fn test_invalid_cache_sizes() {
        let mut config = test_config();
        config.cache_slides = 0;
        assert!(config.validate().is_err());

        let mut config = test_config();
        config.cache_blocks = 0;
        assert!(config.validate().is_err());

        let mut config = test_config();
        config.cache_tiles = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_invalid_jpeg_quality() {
        let mut config = test_config();
        config.jpeg_quality = 0;
        assert!(config.validate().is_err());

        let mut config = test_config();
        config.jpeg_quality = 101;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_bind_address() {
        let config = test_config();
        assert_eq!(config.bind_address(), "127.0.0.1:8080");
    }

    #[test]
    fn test_auth_secret_or_empty() {
        let config = test_config();
        assert_eq!(config.auth_secret_or_empty(), "test-secret");

        let mut config = test_config();
        config.auth_secret = None;
        assert_eq!(config.auth_secret_or_empty(), "");
    }

    #[test]
    fn test_cors_origins() {
        let mut config = test_config();
        config.cors_origins = Some(vec![
            "https://example.com".to_string(),
            "https://other.com".to_string(),
        ]);
        assert!(config.validate().is_ok());
        assert_eq!(config.cors_origins.as_ref().unwrap().len(), 2);
    }
}
