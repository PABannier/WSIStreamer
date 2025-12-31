# ==============================================================================
# WSI Streamer Dockerfile
# ==============================================================================
# Multi-stage build for optimized container size.
#
# Build: docker build -t wsi-streamer .
# Run:   docker run -p 3000:3000 -e WSI_S3_BUCKET=my-bucket wsi-streamer
#
# ==============================================================================

# ------------------------------------------------------------------------------
# Stage 1: Build
# ------------------------------------------------------------------------------
FROM rust:slim-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy manifests and source code
COPY Cargo.toml Cargo.lock ./
COPY src ./src

# Build the application
RUN cargo build --release

# ------------------------------------------------------------------------------
# Stage 2: Runtime
# ------------------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user for security
RUN groupadd --gid 1000 wsi && \
    useradd --uid 1000 --gid wsi --shell /bin/bash --create-home wsi

WORKDIR /app

# Copy the binary from builder
COPY --from=builder /app/target/release/wsi-streamer /app/wsi-streamer

# Set ownership
RUN chown -R wsi:wsi /app

# Switch to non-root user
USER wsi

# Expose the default port
EXPOSE 3000

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:3000/health || exit 1

# ------------------------------------------------------------------------------
# Environment Variables
# ------------------------------------------------------------------------------
# Required:
#   WSI_S3_BUCKET       - S3 bucket name containing slide files
#
# Optional (with defaults):
#   WSI_HOST            - Server bind address (default: 0.0.0.0)
#   WSI_PORT            - Server port (default: 3000)
#   WSI_S3_ENDPOINT     - Custom S3 endpoint for S3-compatible services
#   WSI_S3_REGION       - AWS region (default: us-east-1)
#   WSI_AUTH_SECRET     - HMAC secret for signed URLs (required if auth enabled)
#   WSI_AUTH_ENABLED    - Enable authentication (default: true)
#   WSI_CACHE_SLIDES    - Max slides to cache (default: 100)
#   WSI_CACHE_BLOCKS    - Max blocks per slide (default: 100)
#   WSI_CACHE_TILES     - Max tiles to cache (default: 104857600)
#   WSI_JPEG_QUALITY    - Default JPEG quality (default: 80)
#   WSI_CACHE_MAX_AGE   - HTTP cache max-age seconds (default: 3600)
#   WSI_CORS_ORIGINS    - Allowed CORS origins, comma-separated
#
# AWS Credentials (standard AWS SDK environment variables):
#   AWS_ACCESS_KEY_ID       - AWS access key
#   AWS_SECRET_ACCESS_KEY   - AWS secret key
#   AWS_REGION              - AWS region (can also use WSI_S3_REGION)
#
# ------------------------------------------------------------------------------

# Default command
ENTRYPOINT ["/app/wsi-streamer"]
