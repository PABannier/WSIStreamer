# WSI Streamer

## Project Overview

WSI Streamer is a tile server for Whole Slide Images (WSI) designed to serve tiles directly from cloud object storage. Unlike traditional approaches that require downloading entire multi-gigabyte slide files before serving, WSI Streamer uses HTTP range requests to fetch only the bytes needed for each tile, eliminating local storage requirements.

The project implements native Rust parsers for a focused subset of WSI formats, understanding their internal structure (TIFF IFDs, tile offsets) to extract tiles with minimal latency.

### Target Users

- Viewer developers building web-based pathology viewers
- Startups needing tile serving infrastructure without operational complexity
- Research platforms requiring scalable slide access
- Organizations wanting a reference implementation for modern WSI serving

### Core Value Proposition

- **Zero local storage**: Tiles are extracted via range requests, never downloading full slides
- **Focused format support**: Native parsing of SVS and generic pyramidal TIFF
- **Clean API**: Simple REST interface with signed URL authentication

---

## Features

### Tile API

- Serve JPEG tiles at arbitrary pyramid levels
- Configurable JPEG quality (default 80, configurable per request)
- HTTP cache headers for CDN/browser caching

### Storage Backend

- S3-compatible object storage (AWS S3, MinIO, GCS, etc.)
- Range request-based tile extraction
- No local file downloads required

### Caching Strategy

- **Block cache**: Fixed-size block cache (256KB) with singleflight for range requests. Prevents duplicate S3 requests and ensures sequential reads benefit from prefetched data.
- **Tile cache**: LRU cache for encoded JPEG tiles (serves repeated tile requests)
- **Metadata cache**: Parsed slide structure (IFDs, tile offsets) cached per slide

### Format Support

- **Aperio SVS**: TIFF-based, JPEG or JPEG 2000 tiles
- **Generic Pyramidal TIFF**: Standard tiled TIFF with multiple resolutions, JPEG or JPEG 2000 compression

### Supported Subset (Strict)

The following constraints define what slides are supported. Slides outside this subset return HTTP 415 Unsupported Media Type:

- **Organization**: Tiled only (no strips)
- **Compression**: JPEG and JPEG 2000 (no LZW, Deflate)
- **Format**: Standard TIFF or BigTIFF
- **Structure**: Must have tile offsets and byte counts tags

### Authentication

- Signed URL authentication using HMAC-SHA256
- Configurable signature TTL
- Path + query signatures preventing URL tampering

### Deployment

- Docker container for server deployment

---

## MVP Scope

### In Scope

1. **Single-tile API endpoint**: `GET /tiles/{slide_id}/{level}/{x}/{y}.jpg`
2. **Health check endpoint**: `GET /health`
3. **S3-compatible storage backend** with range request support
4. **Native format parsers** for SVS and generic pyramidal TIFF (JPEG/JPEG 2000, tiled only)
5. **In-memory LRU caching** for chunks, tiles, and slide metadata
6. **Signed URL authentication** with HMAC-SHA256
7. **Docker container** deployment
8. **HTTP cache headers**: Cache-Control
9. **Basic logging** for debugging and operations

### Out of Scope (Future Work)

1. **No viewer**: This is a tile server only, not a viewer application
2. **No annotation overlays**: Tiles are served without annotation rendering
3. **No write access**: Read-only access to slides
4. **No slide ingestion**: Slides must already exist in S3
5. **No format conversion**: Slides served in their native format
6. **No thumbnail generation**: Only pyramid tiles are served
7. **No multi-region replication**: Single S3 bucket assumed
8. **No user management**: Authentication is URL-based, not user-based
9. **No rate limiting**: Assumed to be handled by API gateway or CDN
10. **No WebSocket support**: HTTP REST only
11. **No tile prefetching**: Tiles served on-demand only
12. **No label/macro images**: Only pyramid levels served
13. **No DICOM support**: WSI formats only
14. **No NDPI support**: Vendor quirks and edge cases deferred
15. **No MRXS support**: Multi-file format complexity deferred
16. **No Lambda/serverless**: Container deployment only
17. **No metrics/observability**: No Prometheus, tracing, or correlation IDs
18. **No slide info endpoint**: Metadata API deferred
19. **No tile resizing**: Tiles served at native size only
20. **No JPEG passthrough optimization**: Always decode and re-encode
21. **No strip support**: Tiled TIFF only
22. **No LZW/Deflate compression**: JPEG and JPEG 2000 only

---

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

### Key Design Decisions

1. **Range-backed I/O**: All file access is through HTTP range requests. No files are ever downloaded entirely. This eliminates storage management.

2. **Native format parsing**: Rather than using OpenSlide (which requires local files), we implement native Rust parsers that understand TIFF structure and can extract tiles with minimal range requests.

3. **Strict subset**: Rather than attempting broad format coverage, we define a strict supported subset (tiled, JPEG or JPEG 2000 compressed TIFF) and reject unsupported files with 415.

4. **Layered caching**: Three cache levels optimize for different access patterns:
   - Block cache: Fixed-size blocks (256KB) with singleflight to prevent duplicate S3 requests
   - Tile cache: Encoded JPEGs for repeated tile requests
   - Metadata cache: Parsed slide structure to avoid re-parsing IFDs

5. **Format abstraction**: A common `SlideReader` trait allows the tile service to work with any format without format-specific logic.

6. **Always decode/encode**: For simplicity and correctness, tiles are always decoded from source format and re-encoded as JPEG. No passthrough optimization in MVP.

7. **No resizing**: Tiles are served at their native size. The `x` and `y` parameters specify tile indices, not pixel coordinates.

---

## Critical Implementation Challenges

Even with the simplified MVP scope, three areas will consume the majority of implementation and debugging time. These are called out explicitly because underestimating them is the primary risk to the project.

### 1. TIFF Is Deceptively Complex

TIFF appears simple (it's just IFDs pointing to data) but has many interacting features that must all be handled correctly:

**Byte order**: Every multi-byte value in the file depends on the endianness declared in the header. A single missed swap corrupts all subsequent parsing.

**Classic TIFF vs BigTIFF**: Two different header formats, entry sizes (12 vs 20 bytes), and offset widths (32 vs 64 bit). The parser must branch correctly throughout.

**Inline vs offset values**: IFD entries store values inline if they fit (≤4 bytes for classic, ≤8 bytes for BigTIFF), otherwise store an offset to the actual data. The threshold depends on both the field type AND the count. Getting this wrong causes reads from garbage offsets.

**Array reads for tile locations**: `TileOffsets` and `TileByteCounts` are arrays with one entry per tile. These arrays can be large (thousands of entries for high-resolution slides) and must be read efficiently. A naive implementation that reads one offset at a time will make thousands of S3 requests.

**Identifying pyramid levels**: A WSI file contains multiple IFDs, but not all are pyramid levels. Label images, macro images, and thumbnails are stored as separate IFDs. The parser must identify which IFDs belong to the pyramid (typically by analyzing dimensions and downsample ratios) and skip the rest.

### 2. SVS JPEGTables Handling

This is the most common cause of "tile decoding fails" bugs and must be treated as a first-class requirement, not an afterthought.

**The problem**: Aperio SVS files use "abbreviated JPEG streams" to save space. Each tile's JPEG data is incomplete—it lacks the quantization and Huffman tables needed for decoding. These tables are stored once in a `JPEGTables` TIFF tag and must be merged with each tile's data before decoding.

**The merge is non-trivial**:
- The `JPEGTables` blob starts with `FFD8` (SOI) and ends with `FFD9` (EOI)
- Each tile's data also starts with `FFD8` and ends with `FFD9`
- To merge: strip the trailing `FFD9` from tables, strip the leading `FFD8` from tile, concatenate
- If done incorrectly, the result is an invalid JPEG that crashes decoders or produces garbage

**Not all IFDs have the same tables**: In some SVS files, different pyramid levels may have different `JPEGTables`. The tables must be read and cached per-level, not globally.

**Detection**: If a tile's JPEG data starts with `FFD8` followed immediately by `FFDA` (Start of Scan) without any `FFDB` (Define Quantization Table) or `FFC4` (Define Huffman Table) markers, it's an abbreviated stream that needs tables prepended.

### 3. Range Caching Strategy

The caching layer is the difference between a working system and one that's unusably slow or expensive. A naive implementation will fail.

**The problem**: TIFF parsing requires many small reads at scattered offsets:
- Header: 8-16 bytes at offset 0
- IFD entries: 12-20 bytes each, sequentially
- Tag values: variable size at arbitrary offsets
- Tile offset arrays: potentially large, at arbitrary offsets
- Tile data: 10KB-100KB per tile at arbitrary offsets

Without caching, each `read_exact_at()` call becomes an S3 `GetObject` request. Parsing a single slide could require 50+ requests before serving the first tile.

**Block cache, not request cache**: The cache must operate on fixed-size blocks (e.g., 256KB), not on individual read requests. When a read for bytes 1000-1100 arrives:
1. Calculate which block(s) contain those bytes (block 0 if block size is 256KB)
2. Fetch the entire block if not cached
3. Return the requested slice from the cached block

This ensures that sequential reads (common in IFD parsing) hit cache after the first request.

**Singleflight for concurrent requests**: When multiple requests need the same block simultaneously (e.g., during cold start when parsing begins), only one S3 request should be made. Other waiters should block on the in-flight request, not issue duplicate requests. Without this, cold start on a popular slide can cause request amplification.

**Block alignment matters**: If reads frequently span block boundaries, the cache effectiveness drops. Choose block size based on expected read patterns (256KB is a reasonable default—large enough to amortize S3 latency, small enough to not waste bandwidth).

---

## Project Structure

```
wsi-streamer/
├── src/
│   ├── main.rs                      # Binary entrypoint
│   ├── lib.rs                       # Shared library
│   ├── config.rs                    # Configuration management
│   ├── error.rs                     # Error types and HTTP responses
│   │
│   ├── io/                          # I/O abstraction layer
│   │   ├── mod.rs
│   │   ├── range_reader.rs          # RangeReader trait definition
│   │   ├── s3_reader.rs             # S3 range request implementation
│   │   └── block_cache.rs           # Block cache with singleflight
│   │
│   ├── format/                      # Format parsers
│   │   ├── mod.rs
│   │   ├── detect.rs                # Magic byte format detection
│   │   ├── tiff/
│   │   │   ├── mod.rs
│   │   │   ├── parser.rs            # IFD parsing, byte order handling
│   │   │   ├── tags.rs              # TIFF tag definitions
│   │   │   └── pyramid.rs           # Multi-resolution level handling
│   │   ├── svs.rs                   # Aperio SVS format specifics
│   │   └── generic_tiff.rs          # Standard pyramidal TIFF
│   │
│   ├── slide/                       # Slide abstraction
│   │   ├── mod.rs
│   │   ├── reader.rs                # SlideReader trait definition
│   │   └── registry.rs              # Slide metadata and reader cache
│   │
│   ├── tile/                        # Tile service
│   │   ├── mod.rs
│   │   ├── cache.rs                 # Encoded tile LRU cache
│   │   ├── encoder.rs               # JPEG encoding
│   │   └── service.rs               # Tile generation orchestration
│   │
│   └── server/                      # HTTP layer
│       ├── mod.rs
│       ├── routes.rs                # Route definitions
│       ├── handlers.rs              # Request handlers
│       └── auth.rs                  # Signed URL verification
│
├── tests/
│   └── integration/                 # End-to-end API tests
│
└── Dockerfile
```

---

## Implementation Plan

### Phase 1: I/O Layer (Days 1-2)

The I/O layer provides an abstraction over byte-range reads from remote storage, enabling the rest of the system to work with files without downloading them entirely.

#### Step 1.1: Define RangeReader Trait

Create the core abstraction for range-based reading:
- Define async trait with `read_exact_at(offset, len)` method
- Include method to get total size of resource
- Include resource identifier for logging/caching
- Design for cloneability and thread-safety (Send + Sync)
- Add simple endian helper functions used by TIFF parser

#### Step 1.2: Implement S3 Range Reader

Build the S3-specific implementation:
- Create reader from bucket and key, fetching size via HEAD request
- Implement range reads using GetObject with Range header
- Handle S3 error responses and map to application errors
- Support custom endpoints for S3-compatible services (MinIO, etc.)

#### Step 1.3: Implement Block Cache with Singleflight

Add block-based caching layer (this is critical for performance):
- Wrap any RangeReader with fixed-size block cache (256KB blocks)
- On read request, calculate which block(s) contain the requested bytes
- If block is cached, return slice from cache
- If block is not cached, check if fetch is already in-flight (singleflight)
- If fetch in-flight, wait for it; otherwise initiate fetch and register as in-flight
- Handle reads spanning multiple blocks by fetching all required blocks
- Use LRU eviction when cache reaches capacity
- Block alignment: reads are always expanded to block boundaries for fetching

---

### Phase 2: TIFF Parser Core (Days 3-5)

The TIFF parser is the most complex component. Despite appearing simple, TIFF has many interacting features that must all be handled correctly. Budget extra time for edge cases.

#### Step 2.1: Define TIFF Tags and Types

Establish the vocabulary for TIFF parsing:
- Define enum for relevant TIFF tags (ImageWidth, TileOffsets, TileByteCounts, Compression, JPEGTables, etc.)
- Define enum for TIFF field types (Byte, Short, Long, Long8)
- Include size-in-bytes for each field type (needed for inline vs offset decision)
- Support both standard TIFF and BigTIFF tag value sizes

#### Step 2.2: Implement TIFF Header Parsing

Parse the file header to understand structure:
- Detect byte order from magic bytes (II = little-endian, MM = big-endian)
- Store byte order and use it for ALL subsequent multi-byte reads
- Detect TIFF vs BigTIFF from version number (42 vs 43)
- For BigTIFF, verify offset byte size is 8
- Extract first IFD offset (4 bytes for TIFF, 8 bytes for BigTIFF)
- Validate header structure and fail gracefully on invalid files

#### Step 2.3: Implement IFD Parsing

Parse Image File Directories (this is where byte order and format differences compound):
- Read entry count (2 bytes for TIFF, 8 bytes for BigTIFF)
- Calculate total IFD size and read all entries in one range request
- Parse each entry respecting byte order throughout:
  - Tag ID (2 bytes)
  - Field type (2 bytes)
  - Count (4 bytes for TIFF, 8 bytes for BigTIFF)
  - Value/offset field (4 bytes for TIFF, 8 bytes for BigTIFF)
- Determine if value is inline or at offset:
  - Calculate total value size: `count * field_type.size_in_bytes()`
  - If total ≤ value field size (4 or 8), value is inline
  - Otherwise, value field contains offset to actual data
- Extract next IFD offset from end of directory
- Cache commonly-accessed values (dimensions, tile size, compression)

#### Step 2.4: Implement Value Reading

Read tag values from IFD entries:
- For inline values: extract from the value field bytes, respecting byte order
- For offset values: issue range read to fetch data, respecting byte order
- For array values (TileOffsets, TileByteCounts):
  - Calculate total array size
  - Fetch entire array in single range request (critical for performance)
  - Parse all elements respecting byte order
- Handle field type variations (Short vs Long for same tag depending on file)

#### Step 2.5: Implement Pyramid Level Identification

Build multi-resolution structure from IFDs (requires heuristics):
- Parse all IFDs in the file following next-IFD chain
- For each IFD, extract dimensions and check for tile tags
- Identify pyramid levels vs other images:
  - Pyramid levels have decreasing dimensions with consistent ratios
  - Label images are typically small and square-ish (e.g., 500x500)
  - Macro images are medium-sized with different aspect ratios
  - Thumbnail may be very small or lack tile structure
- Calculate downsample factors relative to largest level
- Sort levels by downsample factor
- Store tile offset and byte count arrays for each pyramid level
- Store JPEGTables for each level (may differ between levels)

#### Step 2.6: Implement Validation

Reject unsupported slides early with clear errors:
- Check for tile tags (TileWidth, TileLength, TileOffsets, TileByteCounts)
- If strip tags present instead (StripOffsets, StripByteCounts), return 415
- Check compression tag value
- If not JPEG (value 7) or JPEG 2000 (value 33003), return 415 with message indicating compression type
- Verify tile dimensions are present and reasonable

---

### Phase 3: Format-Specific Readers (Days 6-7)

Build on the TIFF parser to handle format-specific quirks.

#### Step 3.1: Implement Format Detection

Automatically identify slide format:
- Read initial bytes and check TIFF magic
- For TIFF files, scan for vendor-specific markers
- Check for "Aperio" string for SVS
- Fall back to generic TIFF for standard pyramidal files
- Return 415 for unrecognized formats

#### Step 3.2: Implement SVS Reader

Handle Aperio SVS format specifics, with JPEGTables as first-class requirement:

**JPEGTables handling (critical path)**:
- Read JPEGTables tag from each pyramid level IFD
- Cache tables per-level (different levels may have different tables)
- Implement robust JPEG stream merging:
  - Detect abbreviated streams (starts with FFD8 followed by FFDA without FFD8/FFC4)
  - Strip FFD9 (EOI) from end of tables
  - Strip FFD8 (SOI) from start of tile data
  - Concatenate: SOI + tables_content + tile_content
- Add fallback: if tile data contains full JPEG markers, skip merging
- Test with multiple SVS files from different scanners (table formats vary)

**Metadata parsing**:
- Parse ImageDescription tag for SVS metadata
- Extract MPP (microns per pixel) if present
- Extract magnification if present

**Pyramid identification**:
- SVS files contain: pyramid levels, label, macro, thumbnail
- Identify pyramid IFDs by analyzing dimension ratios
- Skip IFDs that don't fit the pyramid pattern
- Map user-facing level indices to IFD indices

#### Step 3.3: Implement Generic TIFF Reader

Handle standard pyramidal TIFF:
- Support standard tiled TIFF structure with JPEG or JPEG 2000 compression
- Validate tiled organization and JPEG/JPEG 2000 compression
- Return 415 for unsupported configurations

---

### Phase 4: Slide Abstraction Layer (Day 8)

Create a unified interface for working with slides regardless of format.

#### Step 4.1: Define SlideReader Trait

Create format-agnostic interface:
- Define methods for metadata access (dimensions, level count, etc.)
- Define tile reading method with level and tile coordinates
- Define level query methods (dimensions, downsample, tile size)
- Support async tile reading for I/O-bound operations

#### Step 4.2: Implement Slide Registry

Manage slide lifecycle and caching:
- Cache open slide readers with LRU eviction
- Create readers on-demand with format auto-detection
- Share chunk cache across all readers
- Handle concurrent requests for same slide (dedup opens)

---

### Phase 5: Tile Service (Days 9-10)

Orchestrate tile generation from slide reading through encoding.

#### Step 5.1: Implement Tile Cache

Cache encoded tiles for repeated requests:
- Define cache key including slide, level, coordinates, quality
- Implement LRU cache with size-based capacity

#### Step 5.2: Implement JPEG Encoder

Handle tile encoding:
- Decode source JPEG tiles
- Encode to JPEG at requested quality
- No resizing - tiles served at native size

#### Step 5.3: Implement Tile Service

Orchestrate the tile pipeline:
- Validate request parameters (level, tile coordinates, quality)
- Check tile cache for existing result
- Fetch slide reader from registry
- Read raw tile data from slide
- Decode source JPEG, re-encode at requested quality
- Cache and return result
- Handle errors gracefully with appropriate HTTP status codes

---

### Phase 6: HTTP Layer (Days 11-12)

Expose the tile service via HTTP REST API.

#### Step 6.1: Implement Request Handlers

Handle incoming HTTP requests:
- Parse path parameters (slide_id, level, x, y)
- Parse query parameters (quality, auth params)
- Call tile service and return response
- Set appropriate Content-Type (image/jpeg)
- Set cache headers (Cache-Control)
- Handle errors with JSON error responses

#### Step 6.2: Implement Authentication

Verify signed URLs:
- Extract signature and expiry from query parameters
- Verify expiry is in the future
- Reconstruct signed string from path and canonicalized query params (excluding sig)
- Compute expected signature using HMAC-SHA256
- Compare signatures using constant-time comparison
- Reject invalid or expired signatures with 401

#### Step 6.3: Implement Router

Wire together routes:
- Define public routes (health)
- Define protected routes (tiles)
- Apply auth verification to protected routes
- Configure CORS for browser access

---

### Phase 7: Configuration & Error Handling (Day 13)

Make the service configurable and robust.

#### Step 7.1: Implement Configuration

Support flexible configuration:
- Define configuration structure (server, S3, cache, auth, tile defaults)
- Create a CLI to launch and configure the server (the entrypoint is `main.rs`)
- Validate configuration at startup
- Provide sensible defaults

#### Step 7.2: Implement Error Types

Create comprehensive error handling:
- Define error enum covering all failure modes
- Map errors to appropriate HTTP status codes (404, 415, 401, 500)
- Generate JSON error responses
- Include error type and message in responses
- Log errors appropriately

---

### Phase 8: Deployment & Testing (Days 14-15)

Package for production deployment.

#### Step 8.1: Create Docker Container

Build optimized container image:
- Use multi-stage build for small image size
- Include only runtime dependencies
- Configure non-root user
- Set up health check
- Document environment variables
- Create docker-compose for local development with MinIO

#### Step 8.2: Write Integration Tests

Verify end-to-end functionality with focus on critical areas:

**Basic functionality**:
- Test tile retrieval for SVS format
- Test tile retrieval for generic pyramidal TIFF
- Test authentication (valid, expired, invalid signatures)
- Test error cases (missing slide, invalid coordinates, unsupported format)

**TIFF parser edge cases**:
- Test with little-endian TIFF
- Test with big-endian TIFF
- Test with BigTIFF (>4GB offset values)
- Verify label/macro images are excluded from pyramid

**SVS JPEGTables** (most common failure point):
- Test with SVS files that use abbreviated JPEG streams
- Test with SVS files from different scanner versions if available
- Verify decoded tiles are valid images (not corrupted)

**Block cache effectiveness**:
- Log S3 request count during cold start, verify bounded
- Issue concurrent requests for same slide, verify no duplicate fetches
- Issue sequential tile requests, verify latency decreases after first

**Test with real services**:
- Test with MinIO (local development)

---

## Success Criteria

The MVP is complete when:

1. A viewer developer can configure their viewer to use WSI Streamer as a tile source
2. Tiles are served correctly for SVS and generic pyramidal TIFF (JPEG or JPEG 2000, tiled)
3. Unsupported slides (strips, non-JPEG/JPEG 2000 compression, unknown formats) return 415
4. The service runs in Docker
5. Authentication prevents unauthorized access
6. Caching works (repeated requests are faster than initial requests)
7. Basic logging enables debugging
8. S3 request count is reasonable (not exploding on repeated tile requests)

### Critical Path Verification

These specific tests verify the hard parts are working:

**TIFF Parser**:
- Parses both little-endian and big-endian TIFF files correctly
- Parses both classic TIFF and BigTIFF correctly
- Reads TileOffsets/TileByteCounts arrays in single request (verify via logs)
- Correctly identifies pyramid levels and excludes label/macro images

**SVS JPEGTables**:
- Tiles from SVS files decode without errors
- Merged JPEG streams are valid (can be opened by standard JPEG libraries)
- Works with SVS files from different Aperio scanner versions

**Block Cache**:
- Cold start parses slide with bounded S3 request count (e.g., <20 requests)
- Concurrent requests for same slide don't cause duplicate S3 requests (singleflight working)
- Sequential tile requests benefit from block cache (visible in latency)
