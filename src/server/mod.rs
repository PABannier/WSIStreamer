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

pub mod handlers;

pub use handlers::{
    health_handler, tile_handler, AppState, ErrorResponse, HealthResponse, TilePathParams,
    TileQueryParams,
};
