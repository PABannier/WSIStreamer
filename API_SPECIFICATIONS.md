# WSI Streamer API Specifications

Complete API reference for the WSI Streamer tile server.

## Table of Contents

- [Overview](#overview)
- [Authentication](#authentication)
- [Base URL](#base-url)
- [Common Headers](#common-headers)
- [Error Handling](#error-handling)
- [Endpoints](#endpoints)
  - [Health Check](#health-check)
  - [Get Tile](#get-tile)
  - [List Slides](#list-slides)
  - [Get Slide Metadata](#get-slide-metadata)
- [Error Reference](#error-reference)

---

## Overview

WSI Streamer provides a REST API for serving tiles from Whole Slide Images (WSI) stored in S3-compatible object storage. The server supports Aperio SVS files and generic pyramidal TIFF files with JPEG or JPEG 2000 compression.

### Supported Formats

| Format | Extensions | Compression |
|--------|------------|-------------|
| Aperio SVS | `.svs` | JPEG, JPEG 2000 |
| Generic Pyramidal TIFF | `.tif`, `.tiff` | JPEG, JPEG 2000 |

---

## Authentication

When authentication is enabled, all endpoints except `/health` require signed URLs.

### Signed URL Scheme

URLs are authenticated using HMAC-SHA256 signatures. The signature is computed over the request path and query parameters (excluding `sig`), bound to an expiry timestamp.

```
signature = HMAC-SHA256(secret_key, "{path}?{canonical_query}")
```

### Authentication Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `sig` | `string` | Yes | Hex-encoded HMAC-SHA256 signature |
| `exp` | `integer` | Yes | Unix timestamp (seconds) when signature expires |

### Example Signed URL

```
/tiles/sample.svs/0/0/0.jpg?quality=80&exp=1735689600&sig=a1b2c3d4e5f6...
```

### Authentication Errors

| Error Code | HTTP Status | Description |
|------------|-------------|-------------|
| `missing_signature` | 401 | The `sig` parameter is missing |
| `missing_expiry` | 401 | The `exp` parameter is missing |
| `signature_expired` | 401 | The signature has expired |
| `invalid_signature` | 401 | The signature does not match |
| `invalid_signature_format` | 400 | The signature is not valid hexadecimal |
| `invalid_expiry_format` | 400 | The expiry is not a valid integer |

---

## Base URL

```
http://localhost:3000
```

Configure via `--bind` CLI argument or `WSI_BIND` environment variable.

---

## Common Headers

### Response Headers

All successful responses include:

| Header | Description |
|--------|-------------|
| `Content-Type` | MIME type of the response body |

Tile responses additionally include:

| Header | Description |
|--------|-------------|
| `Cache-Control` | Caching directive (e.g., `public, max-age=3600`) |
| `X-Tile-Cache-Hit` | `true` if served from cache, `false` otherwise |
| `X-Tile-Quality` | JPEG quality used for encoding (1-100) |

---

## Error Handling

All errors return a JSON response with the following structure:

### Error Response Schema

```typescript
interface ErrorResponse {
  /** Error type identifier */
  error: string;

  /** Human-readable error message */
  message: string;

  /** HTTP status code (optional, included for convenience) */
  status?: number;
}
```

### Example Error Response

```json
{
  "error": "not_found",
  "message": "Slide not found: nonexistent.svs",
  "status": 404
}
```

---

## Endpoints

---

### Health Check

Check if the server is running and healthy.

```
GET /health
```

#### Authentication

None required. This endpoint is always public.

#### Request

No parameters.

#### Response

**Status:** `200 OK`

**Content-Type:** `application/json`

```typescript
interface HealthResponse {
  /** Service status, always "healthy" if responding */
  status: "healthy";

  /** Server version (semver) */
  version: string;
}
```

#### Example

**Request:**
```bash
curl http://localhost:3000/health
```

**Response:**
```json
{
  "status": "healthy",
  "version": "0.1.0"
}
```

---

### Get Tile

Retrieve a single tile from a whole slide image.

```
GET /tiles/{slide_id}/{level}/{x}/{y}.jpg
```

#### Authentication

Required when authentication is enabled.

#### Path Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `slide_id` | `string` | Yes | Slide identifier. URL-encode if contains special characters. |
| `level` | `integer` | Yes | Pyramid level. `0` is highest resolution. |
| `x` | `integer` | Yes | Tile X coordinate (0-indexed from left). |
| `y` | `integer` | Yes | Tile Y coordinate (0-indexed from top). The `.jpg` extension is optional. |

#### Query Parameters

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `quality` | `integer` | No | `80` | JPEG quality (1-100). Higher values produce larger, higher-quality images. |
| `sig` | `string` | Conditional | - | Authentication signature (required when auth enabled). |
| `exp` | `integer` | Conditional | - | Signature expiry timestamp (required when auth enabled). |

#### Response

**Status:** `200 OK`

**Content-Type:** `image/jpeg`

**Body:** Binary JPEG image data.

**Headers:**

| Header | Example | Description |
|--------|---------|-------------|
| `Cache-Control` | `public, max-age=3600` | Browser caching directive |
| `X-Tile-Cache-Hit` | `true` | Whether tile was served from server cache |
| `X-Tile-Quality` | `80` | JPEG quality used for encoding |

#### Errors

| HTTP Status | Error Code | Cause |
|-------------|------------|-------|
| 400 | `invalid_level` | Requested level exceeds available pyramid levels |
| 400 | `tile_out_of_bounds` | Tile coordinates exceed grid dimensions |
| 400 | `invalid_quality` | Quality parameter is not in range 1-100 |
| 401 | `missing_signature` | Authentication enabled but `sig` missing |
| 401 | `missing_expiry` | Authentication enabled but `exp` missing |
| 401 | `signature_expired` | Signature has expired |
| 401 | `invalid_signature` | Signature does not match |
| 404 | `not_found` | Slide does not exist in storage |
| 415 | `unsupported_format` | Slide uses unsupported compression (e.g., LZW) or is not a pyramidal TIFF |
| 500 | `io_error` | Storage read error |
| 500 | `decode_error` | Failed to decode source tile |
| 500 | `encode_error` | Failed to encode JPEG output |
| 502 | `connection_error` | Network error connecting to storage |

#### Examples

**Basic tile request:**
```bash
curl "http://localhost:3000/tiles/sample.svs/0/0/0.jpg" \
  --output tile.jpg
```

**With quality parameter:**
```bash
curl "http://localhost:3000/tiles/sample.svs/0/0/0.jpg?quality=95" \
  --output tile_hq.jpg
```

**Without .jpg extension:**
```bash
curl "http://localhost:3000/tiles/sample.svs/0/0/0" \
  --output tile.jpg
```

**With authentication:**
```bash
curl "http://localhost:3000/tiles/sample.svs/0/0/0.jpg?quality=80&exp=1735689600&sig=a1b2c3..." \
  --output tile.jpg
```

---

### List Slides

List available slides in the storage bucket.

```
GET /slides
```

#### Authentication

Required when authentication is enabled.

#### Query Parameters

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `limit` | `integer` | No | `100` | Maximum slides to return (1-1000). Values outside range are clamped. |
| `cursor` | `string` | No | - | Continuation token from previous response for pagination. |
| `sig` | `string` | Conditional | - | Authentication signature (required when auth enabled). |
| `exp` | `integer` | Conditional | - | Signature expiry timestamp (required when auth enabled). |

#### Response

**Status:** `200 OK`

**Content-Type:** `application/json`

```typescript
interface SlidesResponse {
  /** List of slide identifiers */
  slides: string[];

  /**
   * Continuation token for next page.
   * Null or omitted if no more results.
   */
  next_cursor?: string | null;
}
```

#### Errors

| HTTP Status | Error Code | Cause |
|-------------|------------|-------|
| 401 | `missing_signature` | Authentication enabled but `sig` missing |
| 401 | `missing_expiry` | Authentication enabled but `exp` missing |
| 401 | `signature_expired` | Signature has expired |
| 401 | `invalid_signature` | Signature does not match |
| 500 | `storage_error` | Error listing objects from storage |
| 502 | `connection_error` | Network error connecting to storage |

#### Examples

**List slides:**
```bash
curl "http://localhost:3000/slides"
```

**Response:**
```json
{
  "slides": [
    "sample1.svs",
    "sample2.svs",
    "folder/sample3.tif"
  ],
  "next_cursor": null
}
```

**With pagination:**
```bash
curl "http://localhost:3000/slides?limit=10"
```

**Response with continuation:**
```json
{
  "slides": [
    "slide1.svs",
    "slide2.svs",
    "slide3.svs",
    "slide4.svs",
    "slide5.svs",
    "slide6.svs",
    "slide7.svs",
    "slide8.svs",
    "slide9.svs",
    "slide10.svs"
  ],
  "next_cursor": "c2xpZGUxMC5zdnNbbWluaW9fY2FjaGU6djIscmV0dXJuOl0="
}
```

**Fetch next page:**
```bash
curl "http://localhost:3000/slides?limit=10&cursor=c2xpZGUxMC5zdnNbbWluaW9fY2FjaGU6djIscmV0dXJuOl0="
```

---

### Get Slide Metadata

Retrieve metadata for a specific slide, including dimensions, pyramid levels, and tile information.

```
GET /slides/{slide_id}
```

#### Authentication

Required when authentication is enabled.

#### Path Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `slide_id` | `string` | Yes | Slide identifier. URL-encode if contains special characters. |

#### Query Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `sig` | `string` | Conditional | Authentication signature (required when auth enabled). |
| `exp` | `integer` | Conditional | Signature expiry timestamp (required when auth enabled). |

#### Response

**Status:** `200 OK`

**Content-Type:** `application/json`

```typescript
interface SlideMetadataResponse {
  /** Slide identifier (same as request) */
  slide_id: string;

  /**
   * Detected slide format.
   * Possible values: "Aperio SVS", "Generic Pyramidal TIFF"
   */
  format: string;

  /** Full-resolution image width in pixels */
  width: number;

  /** Full-resolution image height in pixels */
  height: number;

  /** Number of pyramid levels available */
  level_count: number;

  /** Metadata for each pyramid level */
  levels: LevelMetadata[];
}

interface LevelMetadata {
  /** Pyramid level index (0 = highest resolution) */
  level: number;

  /** Width of this level in pixels */
  width: number;

  /** Height of this level in pixels */
  height: number;

  /** Width of each tile in pixels */
  tile_width: number;

  /** Height of each tile in pixels */
  tile_height: number;

  /** Number of tiles in X direction */
  tiles_x: number;

  /** Number of tiles in Y direction */
  tiles_y: number;

  /**
   * Downsample factor relative to level 0.
   * Level 0 has downsample 1.0.
   * Higher levels have higher downsample values (e.g., 4.0, 16.0).
   */
  downsample: number;
}
```

#### Errors

| HTTP Status | Error Code | Cause |
|-------------|------------|-------|
| 401 | `missing_signature` | Authentication enabled but `sig` missing |
| 401 | `missing_expiry` | Authentication enabled but `exp` missing |
| 401 | `signature_expired` | Signature has expired |
| 401 | `invalid_signature` | Signature does not match |
| 404 | `not_found` | Slide does not exist in storage |
| 415 | `unsupported_format` | File is not a supported slide format |
| 500 | `storage_error` | Error reading from storage |
| 502 | `connection_error` | Network error connecting to storage |

#### Example

**Request:**
```bash
curl "http://localhost:3000/slides/sample.svs"
```

**Response:**
```json
{
  "slide_id": "sample.svs",
  "format": "Aperio SVS",
  "width": 125661,
  "height": 61796,
  "level_count": 4,
  "levels": [
    {
      "level": 0,
      "width": 125661,
      "height": 61796,
      "tile_width": 256,
      "tile_height": 256,
      "tiles_x": 491,
      "tiles_y": 242,
      "downsample": 1.0
    },
    {
      "level": 1,
      "width": 31415,
      "height": 15449,
      "tile_width": 256,
      "tile_height": 256,
      "tiles_x": 123,
      "tiles_y": 61,
      "downsample": 4.0
    },
    {
      "level": 2,
      "width": 7853,
      "height": 3862,
      "tile_width": 256,
      "tile_height": 256,
      "tiles_x": 31,
      "tiles_y": 16,
      "downsample": 16.0
    },
    {
      "level": 3,
      "width": 3926,
      "height": 1931,
      "tile_width": 256,
      "tile_height": 256,
      "tiles_x": 16,
      "tiles_y": 8,
      "downsample": 32.0
    }
  ]
}
```

---

## Error Reference

Complete reference of all error codes returned by the API.

### Client Errors (4xx)

| HTTP Status | Error Code | Description |
|-------------|------------|-------------|
| 400 | `invalid_level` | Requested pyramid level does not exist. The response message includes the valid range. |
| 400 | `tile_out_of_bounds` | Tile coordinates exceed the grid dimensions for the specified level. |
| 400 | `invalid_quality` | Quality parameter must be an integer between 1 and 100. |
| 400 | `invalid_signature_format` | The `sig` parameter is not valid hexadecimal. |
| 400 | `invalid_expiry_format` | The `exp` parameter is not a valid Unix timestamp. |
| 401 | `missing_signature` | Request requires authentication but `sig` parameter is missing. |
| 401 | `missing_expiry` | Request requires authentication but `exp` parameter is missing. |
| 401 | `signature_expired` | The signature has expired. Generate a new signed URL. |
| 401 | `invalid_signature` | The signature does not match. Verify the secret key and signing process. |
| 404 | `not_found` | The requested slide or resource does not exist in storage. |
| 415 | `unsupported_format` | The file is not a supported slide format. Supported: pyramidal TIFF with JPEG or JPEG 2000 compression. |

### Server Errors (5xx)

| HTTP Status | Error Code | Description |
|-------------|------------|-------------|
| 500 | `io_error` | General I/O error reading from storage. |
| 500 | `storage_error` | Error communicating with S3-compatible storage. |
| 500 | `decode_error` | Failed to decode the source tile data (corrupted JPEG/J2K). |
| 500 | `encode_error` | Failed to encode the output JPEG (internal error). |
| 502 | `connection_error` | Network error connecting to storage backend. |

### Unsupported Format Details

The `unsupported_format` error (HTTP 415) is returned when:

- File is not a valid TIFF (invalid magic bytes)
- File uses strip organization instead of tiles
- File uses unsupported compression (LZW, Deflate, etc.)
- File is not a pyramidal TIFF (single resolution only)
- File is too small to be a valid TIFF

**Example error response:**
```json
{
  "error": "unsupported_format",
  "message": "Unsupported compression: LZW (only JPEG and JPEG 2000 are supported)",
  "status": 415
}
```

---

## Rate Limiting

The WSI Streamer does not implement rate limiting. If needed, configure rate limiting at the reverse proxy level (e.g., nginx, AWS ALB).

---

## CORS

Cross-Origin Resource Sharing (CORS) is configurable via the `--cors-origins` CLI argument.

- Default: All origins allowed
- Example: `--cors-origins "https://example.com,https://app.example.com"`

Allowed methods: `GET`, `HEAD`, `OPTIONS`

Allowed headers: `Authorization`, `Content-Type`

---

## Versioning

The API does not currently use URL versioning. Breaking changes will be communicated via release notes.

Check the server version via the `/health` endpoint.
