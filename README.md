# WSI Streamer

![WSI Streamer Banner](./assets/banner.png)

WSI Streamer is a tile server for Whole Slide Images (WSI) stored in S3-compatible object storage. It serves tiles on-demand using HTTP range requests, so you never have to download or mount multi-gigabyte slides on local disk.

Whole Slide Images are often 1-10GB+ and live in object storage. Traditional viewers expect a local filesystem and force full downloads before a single tile can be served. WSI Streamer is built for the reality of cloud-native storage: it understands the slide formats, pulls only the bytes it needs, and returns JPEG tiles immediately.

## Highlights

- Streams tiles directly from S3 using range requests (no local files, no full downloads)
- Native Rust parsers for SVS and pyramidal TIFF
- Signed URL authentication with HMAC-SHA256
- Multi-level caching: block cache, metadata cache, and tile cache
- Simple HTTP API for tiles and slide listing

## Quick Start (Docker + MinIO)

A local development stack is provided via `docker-compose.yml`.

```bash
# Start WSI Streamer + MinIO
docker compose up --build

# Health check
curl http://localhost:3000/health

# Fetch a tile (auth disabled in compose)
curl -o tile.jpg http://localhost:3000/tiles/slide.svs/0/0/0.jpg
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

WSI Streamer reads configuration from CLI flags and `WSI_` environment variables.

```bash
export WSI_S3_BUCKET="my-slides"
export WSI_S3_REGION="us-east-1"
export WSI_AUTH_SECRET="super-secret"

# Optional for MinIO or other S3-compatible services
export WSI_S3_ENDPOINT="http://localhost:9000"

wsi-streamer --host 0.0.0.0 --port 3000
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
| `WSI_AUTH_ENABLED` | `true` | Enable signed URL auth |
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
- Optional query params:
  - `quality` (1-100)
  - `exp`, `sig` for signed URLs

Example:

```bash
curl -o tile.jpg \
  "http://localhost:3000/tiles/slides%2Fcase-01.svs/0/10/12.jpg?quality=80&exp=1735689600&sig=..."
```

### Slides

```
GET /slides?limit=100&cursor=...
```

Returns a JSON list of `.svs`, `.tif`, and `.tiff` objects in the bucket.

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
