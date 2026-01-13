//! WSI Streamer - A tile server for Whole Slide Images.
//!
//! This binary starts the HTTP server and configures all components.

use clap::Parser;
use std::process::ExitCode;
use std::time::Duration;
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use wsi_streamer::{
    config::{CheckConfig, Cli, Command, ServeConfig, SignConfig, SignOutputFormat},
    create_s3_client,
    server::{auth::SignedUrlAuth, create_router, RouterConfig},
    slide::{S3SlideSource, SlideRegistry},
    tile::TileService,
};

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.into_command() {
        Command::Serve(config) => run_serve(config).await,
        Command::Sign(config) => run_sign(config),
        Command::Check(config) => run_check(config).await,
    }
}

// =============================================================================
// Serve Command
// =============================================================================

async fn run_serve(config: ServeConfig) -> ExitCode {
    // Initialize logging
    init_logging(config.verbose);

    // Validate configuration
    if let Err(e) = config.validate() {
        error!("Configuration error: {}", e);
        return ExitCode::FAILURE;
    }

    let bucket = config.bucket();

    // Print startup banner and info
    print_banner();

    info!("Configuration:");
    info!("  S3 bucket: {}", bucket);
    if let Some(ref endpoint) = config.s3_endpoint {
        info!("  S3 endpoint: {}", endpoint);
    }
    info!("  S3 region: {}", config.s3_region);

    // Auth status with warning if disabled
    if config.auth_enabled {
        info!("  Auth: enabled");
    } else {
        warn!("  Auth: DISABLED - all endpoints are publicly accessible");
        warn!("        Enable for production: --auth-enabled --auth-secret=<secret>");
    }

    info!(
        "  Cache: {} slides, {} blocks/slide, {}MB tiles",
        config.cache_slides,
        config.cache_blocks,
        config.cache_tiles / (1024 * 1024)
    );

    // Create S3 client
    let s3_client = create_s3_client(config.s3_endpoint.as_deref()).await;

    // Test S3 connectivity
    info!("");
    info!("Connecting to S3...");
    match test_s3_connection(&s3_client, &bucket).await {
        Ok(slide_count) => {
            info!("  Connected successfully");
            info!("  Found {} slide(s) in bucket", slide_count);
        }
        Err(e) => {
            error!("  Failed to connect to S3: {}", e);
            error!("");
            error!("  Please check:");
            error!("    - Your AWS credentials are configured correctly");
            error!("    - The bucket '{}' exists and is accessible", bucket);
            error!("    - The S3 endpoint is correct (if using MinIO/custom S3)");
            return ExitCode::FAILURE;
        }
    }

    // Create slide source and registry
    let source = S3SlideSource::new(s3_client, bucket);
    let registry = SlideRegistry::with_capacity(
        source,
        config.cache_slides,
        config.block_size,
        config.cache_blocks,
    );

    // Create tile service
    let tile_service = TileService::with_cache_capacity(registry, config.cache_tiles);

    // Build router configuration
    let router_config = build_router_config(&config);

    // Create router
    let router = create_router(tile_service, router_config);

    // Bind and serve
    let addr = config.bind_address();

    info!("");
    info!("────────────────────────────────────────────────────────────────");
    info!("  Server listening on: http://{}", addr);
    info!("");
    info!("  Try these endpoints:");
    info!("    curl http://{}/health", addr);
    info!("    curl http://{}/slides", addr);
    info!("");
    info!("  View slides in your browser:");
    info!("    open http://{}/view/<slide_id>", addr);
    if !config.auth_enabled {
        info!("");
        info!("  Fetch a tile directly:");
        info!("    curl http://{}/tiles/<slide_id>/0/0/0.jpg", addr);
    }
    info!("────────────────────────────────────────────────────────────────");
    info!("");

    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(listener) => listener,
        Err(e) => {
            error!("Failed to bind to {}: {}", addr, e);
            return ExitCode::FAILURE;
        }
    };

    if let Err(e) = axum::serve(listener, router).await {
        error!("Server error: {}", e);
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

/// Print the startup banner.
fn print_banner() {
    let version = env!("CARGO_PKG_VERSION");
    info!("");
    info!("██╗    ██╗███████╗██╗                                               ");
    info!("██║    ██║██╔════╝██║                                               ");
    info!("██║ █╗ ██║███████╗██║                                               ");
    info!("██║███╗██║╚════██║██║                                               ");
    info!("╚███╔███╔╝███████║██║                                               ");
    info!(" ╚══╝╚══╝ ╚══════╝╚═╝                                               ");
    info!("");
    info!("███████╗████████╗██████╗ ███████╗ █████╗ ███╗   ███╗███████╗██████╗ ");
    info!("██╔════╝╚══██╔══╝██╔══██╗██╔════╝██╔══██╗████╗ ████║██╔════╝██╔══██╗");
    info!("███████╗   ██║   ██████╔╝█████╗  ███████║██╔████╔██║█████╗  ██████╔╝");
    info!("╚════██║   ██║   ██╔══██╗██╔══╝  ██╔══██║██║╚██╔╝██║██╔══╝  ██╔══██╗");
    info!("███████║   ██║   ██║  ██║███████╗██║  ██║██║ ╚═╝ ██║███████╗██║  ██║");
    info!("╚══════╝   ╚═╝   ╚═╝  ╚═╝╚══════╝╚═╝  ╚═╝╚═╝     ╚═╝╚══════╝╚═╝  ╚═╝");
    info!("");
    info!("                        v", version);
}

/// Test S3 connectivity and count available slides.
async fn test_s3_connection(client: &aws_sdk_s3::Client, bucket: &str) -> Result<usize, String> {
    let result = client
        .list_objects_v2()
        .bucket(bucket)
        .max_keys(1000)
        .send()
        .await
        .map_err(|e| format!("{}", e))?;

    let count = result
        .contents()
        .iter()
        .filter(|obj| {
            obj.key()
                .map(|k| {
                    let k_lower = k.to_lowercase();
                    k_lower.ends_with(".svs")
                        || k_lower.ends_with(".tif")
                        || k_lower.ends_with(".tiff")
                })
                .unwrap_or(false)
        })
        .count();

    Ok(count)
}

/// Initialize the tracing/logging subsystem.
fn init_logging(verbose: bool) {
    let env_filter = if verbose {
        "wsi_streamer=debug,tower_http=debug"
    } else {
        "wsi_streamer=info,tower_http=info"
    };

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| env_filter.into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
}

/// Build RouterConfig from the application ServeConfig.
fn build_router_config(config: &ServeConfig) -> RouterConfig {
    let mut router_config = if config.auth_enabled {
        RouterConfig::new(config.auth_secret_or_empty())
    } else {
        RouterConfig::without_auth()
    };

    // Apply cache max-age
    router_config = router_config.with_cache_max_age(config.cache_max_age);

    // Apply CORS origins
    if let Some(ref origins) = config.cors_origins {
        router_config = router_config.with_cors_origins(origins.clone());
    }

    // Apply tracing setting
    router_config = router_config.with_tracing(!config.no_tracing);

    router_config
}

// =============================================================================
// Sign Command
// =============================================================================

fn run_sign(config: SignConfig) -> ExitCode {
    // Validate configuration
    if let Err(e) = config.validate() {
        eprintln!("Error: {}", e);
        return ExitCode::FAILURE;
    }

    // Parse additional parameters
    let params = match config.parse_params() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Error: {}", e);
            return ExitCode::FAILURE;
        }
    };

    // Create authenticator and generate signature
    let auth = SignedUrlAuth::new(&config.secret);
    let ttl = Duration::from_secs(config.ttl);

    let params_ref: Vec<(&str, &str)> = params
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    let (signature, expiry) = auth.sign_with_params(&config.path, ttl, &params_ref);

    // Output based on format
    match config.format {
        SignOutputFormat::Signature => {
            println!("{}", signature);
        }
        SignOutputFormat::Json => {
            let url = if let Some(ref base_url) = config.base_url {
                Some(build_signed_url(
                    base_url,
                    &config.path,
                    &params,
                    expiry,
                    &signature,
                ))
            } else {
                None
            };

            let json = serde_json::json!({
                "signature": signature,
                "expiry": expiry,
                "path": config.path,
                "ttl": config.ttl,
                "url": url,
            });
            println!("{}", serde_json::to_string_pretty(&json).unwrap());
        }
        SignOutputFormat::Url => {
            if let Some(ref base_url) = config.base_url {
                let url = build_signed_url(base_url, &config.path, &params, expiry, &signature);
                println!("{}", url);
            } else {
                // Output path with query params
                let query = build_query_string(&params, expiry, &signature);
                println!("{}?{}", config.path, query);
                eprintln!();
                eprintln!("Tip: Use --base-url to generate a complete URL");
            }
        }
    }

    ExitCode::SUCCESS
}

/// Build a complete signed URL.
fn build_signed_url(
    base_url: &str,
    path: &str,
    params: &[(String, String)],
    expiry: u64,
    signature: &str,
) -> String {
    let base_url = base_url.trim_end_matches('/');
    let query = build_query_string(params, expiry, signature);
    format!("{}{}?{}", base_url, path, query)
}

/// Build the query string with expiry and signature.
fn build_query_string(params: &[(String, String)], expiry: u64, signature: &str) -> String {
    let mut parts: Vec<String> = params.iter().map(|(k, v)| format!("{}={}", k, v)).collect();

    parts.push(format!("exp={}", expiry));
    parts.push(format!("sig={}", signature));

    parts.join("&")
}

// =============================================================================
// Check Command
// =============================================================================

async fn run_check(config: CheckConfig) -> ExitCode {
    // Initialize minimal logging for check command
    if config.verbose {
        init_logging(true);
    }

    println!("WSI Streamer Configuration Check");
    println!("═════════════════════════════════");
    println!();

    // Resolve bucket
    let bucket = match config.resolve_bucket() {
        Ok(b) => {
            println!("✓ Bucket: {}", b);
            b
        }
        Err(e) => {
            println!("✗ Bucket: {}", e);
            return ExitCode::FAILURE;
        }
    };

    if let Some(ref endpoint) = config.s3_endpoint {
        println!("✓ Endpoint: {}", endpoint);
    }
    println!("✓ Region: {}", config.s3_region);
    println!();

    // Test S3 connectivity
    print!("Testing S3 connection... ");

    let s3_client = create_s3_client(config.s3_endpoint.as_deref()).await;

    match s3_client
        .list_objects_v2()
        .bucket(&bucket)
        .max_keys(1)
        .send()
        .await
    {
        Ok(_) => {
            println!("✓ success");
        }
        Err(e) => {
            println!("✗ failed");
            println!();
            println!("Error: {}", e);
            println!();
            println!("Please check:");
            println!("  - Your AWS credentials are configured correctly");
            println!("  - The bucket '{}' exists and is accessible", bucket);
            if config.s3_endpoint.is_some() {
                println!("  - The S3 endpoint is correct and reachable");
            }
            return ExitCode::FAILURE;
        }
    }

    // List slides if requested
    if config.list_slides {
        println!();
        println!("Slides in bucket:");
        println!("─────────────────");

        match list_slides(&s3_client, &bucket).await {
            Ok(slides) => {
                if slides.is_empty() {
                    println!("  (no slides found)");
                } else {
                    for slide in &slides {
                        println!("  {}", slide);
                    }
                    println!();
                    println!("Total: {} slide(s)", slides.len());
                }
            }
            Err(e) => {
                println!("  Error listing slides: {}", e);
            }
        }
    }

    // Test specific slide if requested
    if let Some(ref slide_id) = config.test_slide {
        println!();
        print!("Testing slide '{}'... ", slide_id);

        match s3_client
            .head_object()
            .bucket(&bucket)
            .key(slide_id)
            .send()
            .await
        {
            Ok(result) => {
                println!("✓ found");
                if let Some(size) = result.content_length() {
                    let size_mb = size as f64 / (1024.0 * 1024.0);
                    println!("  Size: {:.2} MB", size_mb);
                }
                if let Some(content_type) = result.content_type() {
                    println!("  Content-Type: {}", content_type);
                }
            }
            Err(_) => {
                println!("✗ not found");
                println!();
                println!("  The slide '{}' does not exist in the bucket.", slide_id);
                return ExitCode::FAILURE;
            }
        }
    }

    println!();
    println!("═════════════════════════════════");
    println!("✓ All checks passed!");

    ExitCode::SUCCESS
}

/// List all slides in the bucket.
async fn list_slides(client: &aws_sdk_s3::Client, bucket: &str) -> Result<Vec<String>, String> {
    let mut slides = Vec::new();
    let mut continuation_token: Option<String> = None;

    loop {
        let mut request = client.list_objects_v2().bucket(bucket).max_keys(1000);

        if let Some(token) = continuation_token {
            request = request.continuation_token(token);
        }

        let result = request.send().await.map_err(|e| format!("{}", e))?;

        for obj in result.contents() {
            if let Some(key) = obj.key() {
                let key_lower = key.to_lowercase();
                if key_lower.ends_with(".svs")
                    || key_lower.ends_with(".tif")
                    || key_lower.ends_with(".tiff")
                {
                    slides.push(key.to_string());
                }
            }
        }

        if result.is_truncated() == Some(true) {
            continuation_token = result.next_continuation_token().map(|s| s.to_string());
        } else {
            break;
        }
    }

    Ok(slides)
}
