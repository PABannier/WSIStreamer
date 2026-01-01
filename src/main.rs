//! WSI Streamer - A tile server for Whole Slide Images.
//!
//! This binary starts the HTTP server and configures all components.

use clap::Parser;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use wsi_streamer::{
    config::Config,
    create_s3_client,
    server::{create_router, RouterConfig},
    slide::{S3SlideSource, SlideRegistry},
    tile::TileService,
};

#[tokio::main]
async fn main() {
    // Parse configuration from CLI and environment
    let config = Config::parse();

    // Initialize logging
    init_logging(config.verbose);

    // Validate configuration
    if let Err(e) = config.validate() {
        error!("Configuration error: {}", e);
        std::process::exit(1);
    }

    info!("Starting WSI Streamer");
    info!("  S3 bucket: {}", config.s3_bucket);
    if let Some(ref endpoint) = config.s3_endpoint {
        info!("  S3 endpoint: {}", endpoint);
    }
    info!("  Auth enabled: {}", config.auth_enabled);
    info!(
        "  Cache: {} slides, {} blocks/slide, {} tiles",
        config.cache_slides, config.cache_blocks, config.cache_tiles
    );

    // Create S3 client
    let s3_client = create_s3_client(config.s3_endpoint.as_deref()).await;

    // Create slide source and registry
    let source = S3SlideSource::new(s3_client, config.s3_bucket.clone());
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
    info!("Listening on http://{}", addr);

    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(listener) => listener,
        Err(e) => {
            error!("Failed to bind to {}: {}", addr, e);
            std::process::exit(1);
        }
    };

    if let Err(e) = axum::serve(listener, router).await {
        error!("Server error: {}", e);
        std::process::exit(1);
    }
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

/// Build RouterConfig from the application Config.
fn build_router_config(config: &Config) -> RouterConfig {
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
