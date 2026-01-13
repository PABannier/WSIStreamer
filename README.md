# WSI Streamer

[![CI](https://github.com/PABannier/WSIStreamer/actions/workflows/CI.yml/badge.svg)](https://github.com/PABannier/WSIStreamer/actions/workflows/CI.yml/badge.svg)

A tile server for Whole Slide Images stored in S3. One command to start serving tiles from your slides.

![WSI Streamer Banner](./assets/banner.png)

## Quick Start

```bash
# Start serving slides from S3
wsi-streamer s3://my-slides-bucket

# View a slide in your browser
open http://localhost:3000/view/sample.svs
```

That's it. No configuration files, no local storage, no complex setup.

## Why WSI Streamer?

Whole Slide Images are often 1-10GB+ and live in object storage. Traditional viewers expect a local filesystem and force full downloads before a single tile can be served. WSI Streamer is built for cloud-native storage: it understands the slide formats, pulls only the bytes it needs via HTTP range requests, and returns JPEG tiles immediately.

**Key features:**
- Streams tiles directly from S3 using range requests (no local files)
- Built-in web viewer with OpenSeadragon
- Native Rust parsers for SVS and pyramidal TIFF
- Optional signed URL authentication with HMAC-SHA256
- Multi-level caching: slides, blocks, and tiles

## Installation

```bash
# Install with Cargo
cargo install --path .

# Or build from source
cargo build --release
```

## Usage

### Basic Usage

```bash
# Serve slides from an S3 bucket
wsi-streamer s3://my-slides

# Custom port
wsi-streamer s3://my-slides --port 8080

# With MinIO or other S3-compatible storage
wsi-streamer s3://slides --s3-endpoint http://localhost:9000
```

### View Slides

Open slides directly in your browser:

```
http://localhost:3000/view/sample.svs
http://localhost:3000/view/folder%2Fsubfolder%2Fslide.svs
```

The built-in viewer provides pan, zoom, and navigation with a dark theme optimized for slide viewing.

### API Access

```bash
# List available slides
curl http://localhost:3000/slides

# Get slide metadata
curl http://localhost:3000/slides/sample.svs

# Fetch a tile (level 0, position 0,0)
curl -o tile.jpg http://localhost:3000/tiles/sample.svs/0/0/0.jpg

# Get a thumbnail
curl -o thumb.jpg "http://localhost:3000/slides/sample.svs/thumbnail?max_size=256"
```

### Production with Authentication

```bash
# Enable HMAC-SHA256 authentication
wsi-streamer s3://my-slides --auth-enabled --auth-secret "$SECRET"

# Generate signed URLs with the CLI
wsi-streamer sign --path /tiles/slide.svs/0/0/0.jpg --secret "$SECRET" \
  --base-url http://localhost:3000
```

When auth is enabled, the web viewer automatically handles authentication - no additional setup required.

### Validate Configuration

```bash
# Check S3 connectivity
wsi-streamer check s3://my-slides

# List available slides
wsi-streamer check s3://my-slides --list-slides

# Test a specific slide
wsi-streamer check s3://my-slides --test-slide sample.svs
```

## Commands

WSI Streamer provides three commands:

| Command | Description |
|---------|-------------|
| `serve` (default) | Start the tile server |
| `sign` | Generate signed URLs for authenticated access |
| `check` | Validate configuration and test S3 connectivity |

Run `wsi-streamer --help` for all options.

## Configuration

All options can be set via CLI flags or environment variables:

| Option | Env Var | Default | Description |
|--------|---------|---------|-------------|
| `--host` | `WSI_HOST` | `0.0.0.0` | Bind address |
| `--port` | `WSI_PORT` | `3000` | HTTP port |
| `--s3-bucket` | `WSI_S3_BUCKET` | - | S3 bucket name |
| `--s3-endpoint` | `WSI_S3_ENDPOINT` | - | Custom S3 endpoint |
| `--s3-region` | `WSI_S3_REGION` | `us-east-1` | AWS region |
| `--auth-enabled` | `WSI_AUTH_ENABLED` | `false` | Enable authentication |
| `--auth-secret` | `WSI_AUTH_SECRET` | - | HMAC secret key |
| `--cache-slides` | `WSI_CACHE_SLIDES` | `100` | Max slides in cache |
| `--cache-tiles` | `WSI_CACHE_TILES` | `100MB` | Tile cache size |
| `--jpeg-quality` | `WSI_JPEG_QUALITY` | `80` | Default JPEG quality |
| `--cors-origins` | `WSI_CORS_ORIGINS` | any | Allowed CORS origins |

## API Endpoints

| Endpoint | Description |
|----------|-------------|
| `GET /health` | Health check |
| `GET /view/{slide_id}` | Web viewer for a slide |
| `GET /tiles/{slide_id}/{level}/{x}/{y}.jpg` | Fetch a tile |
| `GET /slides` | List available slides |
| `GET /slides/{slide_id}` | Get slide metadata |
| `GET /slides/{slide_id}/thumbnail` | Get slide thumbnail |
| `GET /slides/{slide_id}/dzi` | Get DZI descriptor |

See [API_SPECIFICATIONS.md](./API_SPECIFICATIONS.md) for complete documentation.

## Docker

```bash
# Start with Docker Compose (includes MinIO)
docker compose up --build

# Access MinIO console at http://localhost:9001
# Login: minioadmin / minioadmin
```

## Supported Formats

| Format | Extensions | Compression |
|--------|------------|-------------|
| Aperio SVS | `.svs` | JPEG, JPEG 2000 |
| Pyramidal TIFF | `.tif`, `.tiff` | JPEG, JPEG 2000 |

Files must be tiled (not stripped) and pyramidal. Unsupported files return `415 Unsupported Media Type`.

## Architecture

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

MIT. See [LICENSE](./LICENSE).

## Contributing

Issues and pull requests are welcome. See [CONTRIBUTING.md](./CONTRIBUTING.md) for guidelines.
