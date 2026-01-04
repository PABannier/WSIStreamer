//! HTTP server layer for WSI Streamer.
//!
//! This module provides the HTTP API for serving tiles from Whole Slide Images.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                         HTTP Layer                              │
//! │         GET /tiles/{slide_id}/{level}/{x}/{y}.jpg               │
//! │                                                                 │
//! │  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐  │
//! │  │  handlers   │  │    auth     │  │        routes           │  │
//! │  │ (requests)  │  │ (signed URL)│  │  (router config)        │  │
//! │  └─────────────┘  └─────────────┘  └─────────────────────────┘  │
//! └─────────────────────────────────────────────────────────────────┘
//! ```

pub mod auth;
pub mod handlers;
pub mod routes;

pub use auth::{auth_middleware, AuthError, AuthQueryParams, OptionalAuth, SignedUrlAuth};
pub use handlers::{
    health_handler, slide_metadata_handler, slides_handler, tile_handler, AppState, ErrorResponse,
    HealthResponse, LevelMetadataResponse, SlideMetadataResponse, SlidesQueryParams, SlidesResponse,
    TilePathParams, TileQueryParams,
};
pub use routes::{create_dev_router, create_production_router, create_router, RouterConfig};
