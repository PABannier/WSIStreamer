# WSI Streamer

[![CI](https://github.com/PABannier/WSIStreamer/actions/workflows/CI.yml/badge.svg)](https://github.com/PABannier/WSIStreamer/actions/workflows/CI.yml/badge.svg)

Whole Slide Images are often 1-10GB+ and live in object storage. Traditional viewers expect a local filesystem and force full downloads before a single tile can be served. WSI Streamer is built for the reality of cloud-native storage: it understands the slide formats, pulls only the bytes it needs, and returns JPEG tiles immediately.

![WSI Streamer Banner](./assets/banner.png)

[Quick Start](#quick-start) • [API](#api) • [How It Works](#how-it-works)

## Highlights

- Streams tiles directly from S3 using range requests (no local files, no full downloads)
- Native Rust parsers for SVS and pyramidal TIFF
- Signed URL authentication with HMAC-SHA256
- Multi-level caching: block cache, metadata cache, and tile cache
- Simple HTTP API for tiles and slide listing

## Quick Start

```bash
# One-liner to start serving slides from S3
wsi-streamer s3://my-slides-bucket

# Or with explicit flags
wsi-streamer --s3-bucket my-slides-bucket --port 8080

# Health check
curl http://localhost:3000/health

# View a slide in the browser
open http://localhost:3000/view/sample.svs

# Fetch a tile
curl -o tile.jpg http://localhost:3000/tiles/sample.svs/0/0/0.jpg

# Get a thumbnail
curl -o thumb.jpg "http://localhost:3000/slides/sample.svs/thumbnail?max_size=256"
```

### Docker + MinIO

A local development stack is provided via `docker-compose.yml`.

```bash
# Start WSI Streamer + MinIO
docker compose up --build
```

Upload slides with the MinIO console at `http://localhost:9001` (login: `minioadmin` / `minioadmin`) or use `mc` as shown in `docker-compose.yml`.

## Install the Binary

Use Cargo to install the binary locally:

```bash
cargo install --path .
```

This installs `wsi-streamer` into your Cargo bin directory (usually `~/.cargo/bin`).

## Build From Source

```bash
# Debug build
cargo build

# Optimized release build
cargo build --release

# Run directly
cargo run -- --help
```

The release binary is located at `target/release/wsi-streamer`.

## Running the Server

WSI Streamer supports three subcommands:

### Serve (default)

Start the tile server:

```bash
# Simplest invocation (auth disabled by default for local dev)
wsi-streamer s3://my-slides

# With explicit flags
wsi-streamer --s3-bucket my-slides --host 0.0.0.0 --port 3000

# Enable auth for production
wsi-streamer s3://my-slides --auth-enabled --auth-secret "$SECRET"
```

### Sign

Generate signed URLs for authenticated access:

```bash
# Generate a signed URL with 1-hour TTL
wsi-streamer sign --path /tiles/sample.svs/0/0/0.jpg --secret "$SECRET"

# Include quality parameter
wsi-streamer sign --path /tiles/sample.svs/0/0/0.jpg --secret "$SECRET" --params "quality=90"
```

### Check

Validate configuration and test S3 connectivity:

```bash
wsi-streamer check s3://my-slides

# List available slides
wsi-streamer check s3://my-slides --list-slides
```

The AWS SDK default credential chain is used (env vars, shared config, IAM roles, etc.).

## Configuration

Common options (CLI flags mirror these env vars):

| Env Var | Default | Description |
| --- | --- | --- |
| `WSI_HOST` | `0.0.0.0` | Bind address |
| `WSI_PORT` | `3000` | HTTP port |
| `WSI_S3_BUCKET` | (required) | S3 bucket containing slides |
| `WSI_S3_ENDPOINT` | (none) | Custom endpoint for S3-compatible storage |
| `WSI_S3_REGION` | `us-east-1` | AWS region |
| `WSI_AUTH_ENABLED` | `false` | Enable signed URL auth |
| `WSI_AUTH_SECRET` | (required if auth enabled) | HMAC secret |
| `WSI_CACHE_SLIDES` | `100` | Max slides in registry |
| `WSI_CACHE_BLOCKS` | `100` | Blocks cached per slide (256KB each) |
| `WSI_CACHE_TILES` | `104857600` | Tile cache size in bytes |
| `WSI_BLOCK_SIZE` | `262144` | Block cache size in bytes |
| `WSI_JPEG_QUALITY` | `80` | Default JPEG quality |
| `WSI_CACHE_MAX_AGE` | `3600` | Cache-Control max-age (seconds) |
| `WSI_CORS_ORIGINS` | (any) | Comma-separated CORS origins |

Run `wsi-streamer --help` for the full CLI.

## API

### Tiles

```
GET /tiles/{slide_id}/{level}/{x}/{y}.jpg
```

- `slide_id` is the S3 object key. If the key contains `/`, URL-encode it (e.g. `slides%2Fcase-01.svs`).
- `level` is the pyramid level (0 = highest resolution)
- `x`, `y` are tile indices (0-based)
- Optional query params: `quality` (1-100), `exp`, `sig` for signed URLs

### Slides

```
GET /slides?limit=100&cursor=...&prefix=folder/&search=case
```

Returns a JSON list of `.svs`, `.tif`, and `.tiff` objects in the bucket. Supports filtering by `prefix` and `search` (case-insensitive).

### Slide Metadata

```
GET /slides/{slide_id}
```

Returns JSON with slide dimensions, pyramid levels, and tile information.

### DZI Descriptor

```
GET /slides/{slide_id}/dzi
```

Returns a Deep Zoom Image XML descriptor for use with OpenSeadragon and other DZI-compatible viewers.

### Thumbnail

```
GET /slides/{slide_id}/thumbnail?max_size=256&quality=80
```

Returns a JPEG thumbnail of the slide. `max_size` controls the maximum dimension (default: 512).

### Viewer

```
GET /view/{slide_id}
```

Returns an HTML page with an embedded OpenSeadragon viewer. Open in a browser to view the slide interactively.

### Health

```
GET /health
```

Returns JSON with `status` and `version`.

## Authentication (Signed URLs)

When auth is enabled, requests must include `exp` (Unix timestamp) and `sig` (hex HMAC-SHA256). The signature is computed over the request path and canonical query string (sorted by key/value), excluding `sig`.

Signature base string:

```
{path}?{canonical_query}
```

Where `canonical_query` includes `exp` plus any other params (e.g. `quality`) sorted by key. Example canonical query:

```
exp=1735689600&quality=80
```

The server will reject missing, expired, or invalid signatures with `401`.

## Format Support

Supported slide formats (strict subset):

- Aperio SVS
- Pyramidal TIFF / BigTIFF
- Tiled images only (no strips)

- JPEG or JPEG 2000 compression

Unsupported files return `415 Unsupported Media Type` with a helpful error.

## How It Works

1. **Range-based I/O**: slide data is fetched from S3 with HTTP range requests.
2. **Format parsing**: the TIFF structure is parsed to locate tile offsets and sizes.
3. **Tile extraction**: only the requested tile bytes are fetched.
4. **Decode + re-encode**: tiles are decoded (JPEG/JPEG 2000) and re-encoded as JPEG.
5. **Caching**:
   - Block cache for range reads (default 256KB blocks)
   - Metadata cache for parsed slide structure
   - Tile cache for encoded JPEG tiles

This keeps cold-start latency and S3 request counts low without requiring local storage.

## Project Status

WSI Streamer is focused on a tight, production-useful subset of WSI formats. If you need broader vendor support (NDPI, MRXS, DICOM), open an issue or contribute.

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                         HTTP Layer                              │
│         GET /tiles/{slide_id}/{level}/{x}/{y}.jpg               │
└──────────────────────────────┬──────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│                        Tile Service                             │
│  ┌─────────────┐  ┌──────────────┐  ┌────────────────────────┐  │
│  │ Tile Cache  │  │ JPEG Encoder │  │ Slide Registry         │  │
│  │ (encoded    │  │ (decode →    │  │ (format detection,     │  │
│  │  JPEGs)     │  │  encode)     │  │  cached metadata)      │  │
│  └─────────────┘  └──────────────┘  └────────────────────────┘  │
└──────────────────────────────┬──────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│                      Format Layer                               │
│       ┌─────────┐              ┌───────────────────┐            │
│       │   SVS   │              │ Generic Pyramidal │            │
│       │ Reader  │              │   TIFF Reader     │            │
│       └────┬────┘              └─────────┬─────────┘            │
│            │                             │                      │
│            └──────────┬──────────────────┘                      │
│                       │                                         │
│           ┌───────────▼───────────┐                             │
│           │   TIFF Parser Core    │                             │
│           │  (IFD parsing, tile   │                             │
│           │   offset resolution)  │                             │
│           └───────────────────────┘                             │
└──────────────────────────────┬──────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│                        I/O Layer                                │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │              BlockCache + Singleflight                  │    │
│  │  ┌─────────────────┐    ┌─────────────────────────┐     │    │
│  │  │  Block Cache    │    │  Metadata Cache         │     │    │
│  │  │  (256KB blocks) │    │  (IFDs, tile offsets)   │     │    │
│  │  └─────────────────┘    └─────────────────────────┘     │    │
│  └──────────────────────────────┬──────────────────────────┘    │
│                                 │                               │
│                    ┌────────────▼────────────┐                  │
│                    │    S3 Range Reader      │                  │
│                    │  (GetObject + Range)    │                  │
│                    └─────────────────────────┘                  │
└─────────────────────────────────────────────────────────────────┘
```


## License

MIT. See `LICENSE`.

## Contributing

Issues and pull requests are welcome. If you are adding new formats or storage backends, please include tests or sample fixtures where possible.
