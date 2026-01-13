//! Configuration management for WSI Streamer.
//!
//! This module provides a flexible configuration system that supports:
//! - Command-line arguments via clap with subcommands
//! - Environment variables with `WSI_` prefix
//! - Sensible defaults for all optional settings
//!
//! # Subcommands
//!
//! - `serve` (default): Start the tile server
//! - `sign`: Generate signed URLs for authentication
//! - `check`: Validate configuration and test S3 connectivity
//!
//! # Example
//!
//! ```ignore
//! use wsi_streamer::config::Cli;
//! use clap::Parser;
//!
//! match Cli::parse() {
//!     Cli::Serve(config) => { /* start server */ }
//!     Cli::Sign(config) => { /* generate signed URL */ }
//!     Cli::Check(config) => { /* validate config */ }
//! }
//! ```
//!
//! # Environment Variables
//!
//! All configuration options can be set via environment variables with the `WSI_` prefix:
//!
//! - `WSI_HOST` - Server bind address (default: 0.0.0.0)
//! - `WSI_PORT` - Server port (default: 3000)
//! - `WSI_S3_BUCKET` - S3 bucket name
//! - `WSI_S3_ENDPOINT` - Custom S3 endpoint for S3-compatible services
//! - `WSI_S3_REGION` - AWS region (default: us-east-1)
//! - `WSI_AUTH_SECRET` - HMAC secret for signed URLs
//! - `WSI_AUTH_ENABLED` - Enable authentication (default: false)
//! - `WSI_CACHE_SLIDES` - Max slides to cache (default: 100)
//! - `WSI_CACHE_BLOCKS` - Max blocks per slide (default: 100)
//! - `WSI_CACHE_TILES` - Tile cache size in bytes (default: 100MB)
//! - `WSI_JPEG_QUALITY` - Default JPEG quality (default: 80)
//! - `WSI_CACHE_MAX_AGE` - HTTP cache max-age seconds (default: 3600)

use clap::{Args, Parser, Subcommand, ValueEnum};
use std::fmt;

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

/// Default TTL for signed URLs in seconds (1 hour).
pub const DEFAULT_SIGN_TTL: u64 = 3600;

// =============================================================================
// CLI Structure
// =============================================================================

/// WSI Streamer - A tile server for Whole Slide Images.
///
/// Serves tiles from Whole Slide Images stored in S3 or S3-compatible storage
/// using HTTP range requests. No local file downloads required.
#[derive(Parser, Debug)]
#[command(name = "wsi-streamer")]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
#[command(after_help = "\
EXAMPLES:
    # Start serving slides from an S3 bucket
    wsi-streamer s3://my-slides-bucket

    # Use a custom port
    wsi-streamer s3://my-slides --port 8080

    # With MinIO (local S3-compatible storage)
    wsi-streamer s3://slides --s3-endpoint http://localhost:9000

    # Enable authentication for production
    wsi-streamer s3://my-slides --auth-enabled --auth-secret $SECRET

    # Check S3 connectivity and list slides
    wsi-streamer check s3://my-slides --list-slides

    # Generate a signed URL for a tile
    wsi-streamer sign --path /tiles/slide.svs/0/0/0.jpg --secret $SECRET
")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    #[command(flatten)]
    pub serve: ServeConfig,
}

impl Cli {
    /// Returns the command to execute, defaulting to Serve if none specified.
    pub fn into_command(self) -> Command {
        self.command.unwrap_or(Command::Serve(self.serve))
    }
}

#[derive(Subcommand, Debug, Clone)]
pub enum Command {
    /// Start the tile server (default command)
    Serve(ServeConfig),

    /// Generate a signed URL for authenticated access
    Sign(SignConfig),

    /// Validate configuration and test S3 connectivity
    Check(CheckConfig),
}

// =============================================================================
// Serve Configuration
// =============================================================================

/// Configuration for the `serve` command (tile server).
#[derive(Args, Debug, Clone)]
pub struct ServeConfig {
    /// S3 bucket URI (e.g., s3://my-bucket) or just the bucket name.
    /// Alternative to --s3-bucket flag.
    #[arg(value_name = "S3_URI")]
    pub s3_uri: Option<String>,

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
    /// Can also be provided as a positional argument (s3://bucket).
    #[arg(long, env = "WSI_S3_BUCKET")]
    pub s3_bucket: Option<String>,

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
    /// Required when authentication is enabled.
    #[arg(long, env = "WSI_AUTH_SECRET")]
    pub auth_secret: Option<String>,

    /// Enable signed URL authentication.
    ///
    /// When disabled (default), all tile requests are allowed without authentication.
    /// Enable for production deployments.
    #[arg(long, default_value_t = false, env = "WSI_AUTH_ENABLED")]
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

    /// Maximum tile cache size in bytes (default: 100MB).
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

impl ServeConfig {
    /// Resolve the S3 bucket name from either the positional URI or --s3-bucket flag.
    pub fn resolve_bucket(&self) -> Result<String, String> {
        // First try the positional S3 URI
        if let Some(ref uri) = self.s3_uri {
            return parse_s3_uri(uri);
        }

        // Fall back to --s3-bucket flag
        if let Some(ref bucket) = self.s3_bucket {
            if bucket.is_empty() {
                return Err("S3 bucket name cannot be empty".to_string());
            }
            return Ok(bucket.clone());
        }

        Err(
            "S3 bucket is required. Use: wsi-streamer s3://bucket-name or --s3-bucket=name"
                .to_string(),
        )
    }

    /// Validate the configuration and return an error message if invalid.
    pub fn validate(&self) -> Result<(), String> {
        // Resolve and validate bucket
        self.resolve_bucket()?;

        // Check auth secret is provided when auth is enabled
        if self.auth_enabled && self.auth_secret.is_none() {
            return Err("Authentication is enabled but no secret provided. \
                Set --auth-secret or WSI_AUTH_SECRET, or disable auth with --auth-enabled=false"
                .to_string());
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

        // Validate block size (must be reasonable)
        if self.block_size < 1024 || self.block_size > 16 * 1024 * 1024 {
            return Err("block_size must be between 1KB and 16MB".to_string());
        }

        Ok(())
    }

    /// Get the server bind address as "host:port".
    pub fn bind_address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Get the auth secret, returning empty string if not set.
    pub fn auth_secret_or_empty(&self) -> &str {
        self.auth_secret.as_deref().unwrap_or("")
    }

    /// Get the resolved bucket name, panicking if not set (call validate() first).
    pub fn bucket(&self) -> String {
        self.resolve_bucket()
            .expect("bucket should be validated before calling this method")
    }
}

// =============================================================================
// Sign Configuration
// =============================================================================

/// Output format for the sign command.
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum SignOutputFormat {
    /// Output the complete signed URL (default)
    #[default]
    Url,
    /// Output as JSON with signature, expiry, and URL
    Json,
    /// Output only the signature (hex-encoded)
    Signature,
}

impl fmt::Display for SignOutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SignOutputFormat::Url => write!(f, "url"),
            SignOutputFormat::Json => write!(f, "json"),
            SignOutputFormat::Signature => write!(f, "signature"),
        }
    }
}

/// Configuration for the `sign` command.
#[derive(Args, Debug, Clone)]
pub struct SignConfig {
    /// Path to sign (e.g., /tiles/slide.svs/0/0/0.jpg)
    #[arg(short, long)]
    pub path: String,

    /// Secret key for HMAC-SHA256 signing.
    /// Can also be set via WSI_AUTH_SECRET environment variable.
    #[arg(short, long, env = "WSI_AUTH_SECRET")]
    pub secret: String,

    /// Time-to-live in seconds (default: 3600 = 1 hour)
    #[arg(short, long, default_value_t = DEFAULT_SIGN_TTL)]
    pub ttl: u64,

    /// Base URL for complete signed URL output (e.g., http://localhost:3000)
    #[arg(short, long)]
    pub base_url: Option<String>,

    /// Additional query parameters (format: key=value, comma-separated)
    #[arg(short = 'P', long, value_delimiter = ',')]
    pub params: Option<Vec<String>>,

    /// Output format: url (default), json, or signature
    #[arg(short, long, default_value = "url")]
    pub format: SignOutputFormat,
}

impl SignConfig {
    /// Parse the additional parameters into key-value pairs.
    pub fn parse_params(&self) -> Result<Vec<(String, String)>, String> {
        let Some(ref params) = self.params else {
            return Ok(Vec::new());
        };

        params
            .iter()
            .map(|p| {
                let parts: Vec<&str> = p.splitn(2, '=').collect();
                if parts.len() != 2 {
                    Err(format!(
                        "Invalid parameter format '{}'. Expected key=value",
                        p
                    ))
                } else {
                    Ok((parts[0].to_string(), parts[1].to_string()))
                }
            })
            .collect()
    }

    /// Validate the sign configuration.
    pub fn validate(&self) -> Result<(), String> {
        if self.path.is_empty() {
            return Err("Path cannot be empty".to_string());
        }

        if self.secret.is_empty() {
            return Err("Secret cannot be empty. Set --secret or WSI_AUTH_SECRET".to_string());
        }

        if self.ttl == 0 {
            return Err("TTL must be greater than 0".to_string());
        }

        // Validate params format
        self.parse_params()?;

        Ok(())
    }
}

// =============================================================================
// Check Configuration
// =============================================================================

/// Configuration for the `check` command.
#[derive(Args, Debug, Clone)]
pub struct CheckConfig {
    /// S3 bucket URI (e.g., s3://my-bucket) or just the bucket name.
    #[arg(value_name = "S3_URI")]
    pub s3_uri: Option<String>,

    /// S3 bucket name (alternative to positional argument).
    #[arg(long, env = "WSI_S3_BUCKET")]
    pub s3_bucket: Option<String>,

    /// Custom S3 endpoint URL for S3-compatible services.
    #[arg(long, env = "WSI_S3_ENDPOINT")]
    pub s3_endpoint: Option<String>,

    /// AWS region for S3.
    #[arg(long, default_value = DEFAULT_REGION, env = "WSI_S3_REGION")]
    pub s3_region: String,

    /// Test loading a specific slide by name.
    #[arg(long)]
    pub test_slide: Option<String>,

    /// List all slides found in the bucket.
    #[arg(long, default_value_t = false)]
    pub list_slides: bool,

    /// Enable verbose output.
    #[arg(short, long, default_value_t = false)]
    pub verbose: bool,
}

impl CheckConfig {
    /// Resolve the S3 bucket name from either the positional URI or --s3-bucket flag.
    pub fn resolve_bucket(&self) -> Result<String, String> {
        if let Some(ref uri) = self.s3_uri {
            return parse_s3_uri(uri);
        }

        if let Some(ref bucket) = self.s3_bucket {
            if bucket.is_empty() {
                return Err("S3 bucket name cannot be empty".to_string());
            }
            return Ok(bucket.clone());
        }

        Err(
            "S3 bucket is required. Use: wsi-streamer check s3://bucket-name or --s3-bucket=name"
                .to_string(),
        )
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Parse an S3 URI (s3://bucket-name or s3://bucket-name/prefix) and return the bucket name.
fn parse_s3_uri(uri: &str) -> Result<String, String> {
    // Handle both s3:// prefix and plain bucket names
    let uri = uri.trim();

    if let Some(path) = uri.strip_prefix("s3://") {
        let bucket = path.split('/').next().unwrap_or("");
        if bucket.is_empty() {
            return Err(format!(
                "Invalid S3 URI '{}'. Expected format: s3://bucket-name",
                uri
            ));
        }
        Ok(bucket.to_string())
    } else if uri.contains("://") {
        Err(format!(
            "Invalid URI scheme in '{}'. Expected s3:// or plain bucket name",
            uri
        ))
    } else {
        // Plain bucket name
        if uri.is_empty() {
            return Err("Bucket name cannot be empty".to_string());
        }
        Ok(uri.to_string())
    }
}

// =============================================================================
// Legacy Compatibility
// =============================================================================

/// Legacy Config type alias for backward compatibility.
/// New code should use `Cli` and `ServeConfig` instead.
pub type Config = ServeConfig;

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_serve_config() -> ServeConfig {
        ServeConfig {
            s3_uri: None,
            host: "127.0.0.1".to_string(),
            port: 8080,
            s3_bucket: Some("test-bucket".to_string()),
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
        let config = test_serve_config();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_missing_auth_secret() {
        let mut config = test_serve_config();
        config.auth_secret = None;
        config.auth_enabled = true;

        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("secret"));
    }

    #[test]
    fn test_auth_disabled_no_secret_ok() {
        let mut config = test_serve_config();
        config.auth_secret = None;
        config.auth_enabled = false;

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_empty_bucket() {
        let mut config = test_serve_config();
        config.s3_bucket = Some(String::new());
        config.s3_uri = None;

        let result = config.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_bucket() {
        let mut config = test_serve_config();
        config.s3_bucket = None;
        config.s3_uri = None;

        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("bucket"));
    }

    #[test]
    fn test_s3_uri_parsing() {
        // Valid S3 URIs
        assert_eq!(parse_s3_uri("s3://my-bucket").unwrap(), "my-bucket");
        assert_eq!(
            parse_s3_uri("s3://my-bucket/prefix/path").unwrap(),
            "my-bucket"
        );
        assert_eq!(parse_s3_uri("my-bucket").unwrap(), "my-bucket");

        // Invalid URIs
        assert!(parse_s3_uri("s3://").is_err());
        assert!(parse_s3_uri("http://bucket").is_err());
        assert!(parse_s3_uri("").is_err());
    }

    #[test]
    fn test_s3_uri_takes_precedence() {
        let mut config = test_serve_config();
        config.s3_uri = Some("s3://uri-bucket".to_string());
        config.s3_bucket = Some("flag-bucket".to_string());

        assert_eq!(config.resolve_bucket().unwrap(), "uri-bucket");
    }

    #[test]
    fn test_invalid_cache_sizes() {
        let mut config = test_serve_config();
        config.cache_slides = 0;
        assert!(config.validate().is_err());

        let mut config = test_serve_config();
        config.cache_blocks = 0;
        assert!(config.validate().is_err());

        let mut config = test_serve_config();
        config.cache_tiles = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_invalid_jpeg_quality() {
        let mut config = test_serve_config();
        config.jpeg_quality = 0;
        assert!(config.validate().is_err());

        let mut config = test_serve_config();
        config.jpeg_quality = 101;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_bind_address() {
        let config = test_serve_config();
        assert_eq!(config.bind_address(), "127.0.0.1:8080");
    }

    #[test]
    fn test_auth_secret_or_empty() {
        let config = test_serve_config();
        assert_eq!(config.auth_secret_or_empty(), "test-secret");

        let mut config = test_serve_config();
        config.auth_secret = None;
        assert_eq!(config.auth_secret_or_empty(), "");
    }

    #[test]
    fn test_cors_origins() {
        let mut config = test_serve_config();
        config.cors_origins = Some(vec![
            "https://example.com".to_string(),
            "https://other.com".to_string(),
        ]);
        assert!(config.validate().is_ok());
        assert_eq!(config.cors_origins.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn test_sign_config_parse_params() {
        let config = SignConfig {
            path: "/tiles/test.svs/0/0/0.jpg".to_string(),
            secret: "secret".to_string(),
            ttl: 3600,
            base_url: None,
            params: Some(vec!["quality=90".to_string(), "format=jpg".to_string()]),
            format: SignOutputFormat::Url,
        };

        let params = config.parse_params().unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0], ("quality".to_string(), "90".to_string()));
        assert_eq!(params[1], ("format".to_string(), "jpg".to_string()));
    }

    #[test]
    fn test_sign_config_invalid_params() {
        let config = SignConfig {
            path: "/tiles/test.svs/0/0/0.jpg".to_string(),
            secret: "secret".to_string(),
            ttl: 3600,
            base_url: None,
            params: Some(vec!["invalid_param".to_string()]),
            format: SignOutputFormat::Url,
        };

        assert!(config.parse_params().is_err());
    }

    #[test]
    fn test_check_config_resolve_bucket() {
        let config = CheckConfig {
            s3_uri: Some("s3://check-bucket".to_string()),
            s3_bucket: None,
            s3_endpoint: None,
            s3_region: DEFAULT_REGION.to_string(),
            test_slide: None,
            list_slides: false,
            verbose: false,
        };

        assert_eq!(config.resolve_bucket().unwrap(), "check-bucket");
    }
}
