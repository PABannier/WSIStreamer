#!/bin/bash
#
# E2E Test Script for WSI Streamer
#
# This script runs end-to-end tests against real TCGA WSI slides stored in S3.
# It verifies that all major features work correctly before a release.
#
# Prerequisites:
#   - AWS credentials configured (via environment variables or ~/.aws/credentials)
#   - Access to the tcga-wsi-slides S3 bucket
#
# Environment Variables:
#   AWS_ACCESS_KEY_ID     - AWS access key (required in CI)
#   AWS_SECRET_ACCESS_KEY - AWS secret key (required in CI)
#   WSI_S3_BUCKET         - S3 bucket name (default: tcga-wsi-slides)
#   WSI_S3_REGION         - S3 region (default: eu-west-3)
#   WSI_TEST_PORT         - Port to run the server on (default: 3000)
#   WSI_BINARY            - Path to wsi-streamer binary (default: auto-detect)
#
# Usage:
#   ./scripts/e2e_test.sh
#
# Exit codes:
#   0 - All tests passed
#   1 - One or more tests failed
#

set -euo pipefail

# Configuration
BUCKET="${WSI_S3_BUCKET:-tcga-wsi-slides}"
REGION="${WSI_S3_REGION:-eu-west-3}"
PORT="${WSI_TEST_PORT:-3000}"
BASE_URL="http://localhost:${PORT}"
TIMEOUT=60
SERVER_PID=""
TEMP_DIR=""

# Test slide (smallest one in the bucket for faster tests)
TEST_SLIDE="TCGA-A6-2686-01Z-00-DX1.0540a027-2a0c-46c7-9af0-7b8672631de7.svs"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Counters
TESTS_PASSED=0
TESTS_FAILED=0

# =============================================================================
# Utility Functions
# =============================================================================

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

log_test() {
    echo -e "  [TEST] $1"
}

cleanup() {
    log_info "Cleaning up..."

    # Kill the server if running
    if [ -n "$SERVER_PID" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
        kill "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    fi

    # Remove temp directory
    if [ -n "$TEMP_DIR" ] && [ -d "$TEMP_DIR" ]; then
        rm -rf "$TEMP_DIR"
    fi
}

trap cleanup EXIT

get_response_body() {
    # Extract body from response (all lines except the last one which is status code)
    local response="$1"
    echo "$response" | sed '$d'
}

get_response_status() {
    # Extract status code from response (last line)
    local response="$1"
    echo "$response" | tail -1
}

assert_status() {
    local name="$1"
    local expected="$2"
    local actual="$3"

    if [ "$actual" = "$expected" ]; then
        log_test "${GREEN}PASS${NC}: $name (status $actual)"
        ((TESTS_PASSED++))
        return 0
    else
        log_test "${RED}FAIL${NC}: $name (expected $expected, got $actual)"
        ((TESTS_FAILED++))
        return 1
    fi
}

assert_contains() {
    local name="$1"
    local needle="$2"
    local haystack="$3"

    if echo "$haystack" | grep -q "$needle"; then
        log_test "${GREEN}PASS${NC}: $name"
        ((TESTS_PASSED++))
        return 0
    else
        log_test "${RED}FAIL${NC}: $name (expected to contain '$needle')"
        ((TESTS_FAILED++))
        return 1
    fi
}

assert_file_type() {
    local name="$1"
    local expected_type="$2"
    local file_path="$3"

    if file "$file_path" | grep -qi "$expected_type"; then
        log_test "${GREEN}PASS${NC}: $name"
        ((TESTS_PASSED++))
        return 0
    else
        local actual_type
        actual_type=$(file "$file_path")
        log_test "${RED}FAIL${NC}: $name (expected '$expected_type', got '$actual_type')"
        ((TESTS_FAILED++))
        return 1
    fi
}

wait_for_server() {
    local max_attempts=$1
    local attempt=0

    while [ $attempt -lt $max_attempts ]; do
        if curl -s "${BASE_URL}/health" > /dev/null 2>&1; then
            return 0
        fi
        sleep 1
        ((attempt++))
    done

    return 1
}

# =============================================================================
# Find Binary
# =============================================================================

find_binary() {
    if [ -n "${WSI_BINARY:-}" ] && [ -x "$WSI_BINARY" ]; then
        echo "$WSI_BINARY"
        return 0
    fi

    # Try release binary first
    if [ -x "./target/release/wsi-streamer" ]; then
        echo "./target/release/wsi-streamer"
        return 0
    fi

    # Try debug binary
    if [ -x "./target/debug/wsi-streamer" ]; then
        echo "./target/debug/wsi-streamer"
        return 0
    fi

    # Try system PATH
    if command -v wsi-streamer > /dev/null 2>&1; then
        command -v wsi-streamer
        return 0
    fi

    return 1
}

# =============================================================================
# Main Script
# =============================================================================

echo ""
echo "╔══════════════════════════════════════════════════════════════════╗"
echo "║              WSI Streamer E2E Test Suite                         ║"
echo "╚══════════════════════════════════════════════════════════════════╝"
echo ""

# Create temp directory
TEMP_DIR=$(mktemp -d)
log_info "Temp directory: $TEMP_DIR"

# Find binary
log_info "Looking for wsi-streamer binary..."
BINARY=$(find_binary) || {
    log_error "Could not find wsi-streamer binary. Build it first with 'cargo build --release'"
    exit 1
}
log_info "Using binary: $BINARY"

# Check AWS credentials
log_info "Checking AWS credentials..."
if [ -z "${AWS_ACCESS_KEY_ID:-}" ] && [ ! -f ~/.aws/credentials ]; then
    log_error "AWS credentials not found. Set AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY or configure ~/.aws/credentials"
    exit 1
fi
log_info "AWS credentials found"

# =============================================================================
# Pre-flight Check: Verify S3 Connectivity
# =============================================================================

echo ""
log_info "Running pre-flight S3 connectivity check..."
CHECK_OUTPUT=$("$BINARY" check "s3://${BUCKET}" --s3-region "$REGION" 2>&1) || {
    log_error "S3 connectivity check failed:"
    echo "$CHECK_OUTPUT"
    exit 1
}
log_info "S3 connectivity verified"

# =============================================================================
# Start Server
# =============================================================================

echo ""
log_info "Starting WSI Streamer server on port $PORT..."
"$BINARY" "s3://${BUCKET}" --s3-region "$REGION" --port "$PORT" > "$TEMP_DIR/server.log" 2>&1 &
SERVER_PID=$!
log_info "Server started with PID $SERVER_PID"

# Wait for server to be ready
log_info "Waiting for server to be ready..."
if ! wait_for_server 30; then
    log_error "Server failed to start within 30 seconds"
    log_error "Server log:"
    cat "$TEMP_DIR/server.log"
    exit 1
fi
log_info "Server is ready"

# =============================================================================
# Test Suite
# =============================================================================

echo ""
echo "═══════════════════════════════════════════════════════════════════"
echo "  Running E2E Tests"
echo "═══════════════════════════════════════════════════════════════════"
echo ""

# -----------------------------------------------------------------------------
# Test 1: Health Endpoint
# -----------------------------------------------------------------------------
log_info "Test 1: Health Endpoint"
RESPONSE=$(curl -s -w "\n%{http_code}" "${BASE_URL}/health")
STATUS=$(get_response_status "$RESPONSE")
BODY=$(get_response_body "$RESPONSE")

assert_status "GET /health returns 200" "200" "$STATUS" || true
assert_contains "Health response contains 'healthy'" "healthy" "$BODY" || true

# -----------------------------------------------------------------------------
# Test 2: List Slides Endpoint
# -----------------------------------------------------------------------------
echo ""
log_info "Test 2: List Slides Endpoint"
RESPONSE=$(curl -s -w "\n%{http_code}" "${BASE_URL}/slides")
STATUS=$(get_response_status "$RESPONSE")
BODY=$(get_response_body "$RESPONSE")

assert_status "GET /slides returns 200" "200" "$STATUS" || true
assert_contains "Slides list contains test slide" "$TEST_SLIDE" "$BODY" || true

# -----------------------------------------------------------------------------
# Test 3: Slide Metadata Endpoint
# -----------------------------------------------------------------------------
echo ""
log_info "Test 3: Slide Metadata Endpoint"
RESPONSE=$(curl -s -w "\n%{http_code}" "${BASE_URL}/slides/${TEST_SLIDE}")
STATUS=$(get_response_status "$RESPONSE")
BODY=$(get_response_body "$RESPONSE")

assert_status "GET /slides/{slide_id} returns 200" "200" "$STATUS" || true
assert_contains "Metadata contains slide_id" "slide_id" "$BODY" || true
assert_contains "Metadata contains width" "width" "$BODY" || true
assert_contains "Metadata contains height" "height" "$BODY" || true
assert_contains "Metadata contains levels" "levels" "$BODY" || true
assert_contains "Metadata identifies format as Aperio SVS" "Aperio SVS" "$BODY" || true

# -----------------------------------------------------------------------------
# Test 4: Tile Fetching
# -----------------------------------------------------------------------------
echo ""
log_info "Test 4: Tile Fetching"

# Fetch a tile from level 0
TILE_PATH="$TEMP_DIR/tile_0_0_0.jpg"
STATUS=$(curl -s -o "$TILE_PATH" -w "%{http_code}" "${BASE_URL}/tiles/${TEST_SLIDE}/0/0/0.jpg")

assert_status "GET /tiles/{slide}/0/0/0.jpg returns 200" "200" "$STATUS" || true
assert_file_type "Tile is a valid JPEG image" "JPEG" "$TILE_PATH" || true

# Fetch a tile from a higher level (lower resolution)
TILE_PATH_L2="$TEMP_DIR/tile_2_0_0.jpg"
STATUS=$(curl -s -o "$TILE_PATH_L2" -w "%{http_code}" "${BASE_URL}/tiles/${TEST_SLIDE}/2/0/0.jpg")

assert_status "GET /tiles/{slide}/2/0/0.jpg returns 200" "200" "$STATUS" || true
assert_file_type "Level 2 tile is a valid JPEG image" "JPEG" "$TILE_PATH_L2" || true

# -----------------------------------------------------------------------------
# Test 5: Thumbnail Generation
# -----------------------------------------------------------------------------
echo ""
log_info "Test 5: Thumbnail Generation"

# Default thumbnail
THUMB_PATH="$TEMP_DIR/thumbnail.jpg"
STATUS=$(curl -s -o "$THUMB_PATH" -w "%{http_code}" "${BASE_URL}/slides/${TEST_SLIDE}/thumbnail")

assert_status "GET /slides/{slide}/thumbnail returns 200" "200" "$STATUS" || true
assert_file_type "Thumbnail is a valid JPEG image" "JPEG" "$THUMB_PATH" || true

# Thumbnail with custom size
THUMB_SMALL_PATH="$TEMP_DIR/thumbnail_small.jpg"
STATUS=$(curl -s -o "$THUMB_SMALL_PATH" -w "%{http_code}" "${BASE_URL}/slides/${TEST_SLIDE}/thumbnail?max_size=128")

assert_status "GET /slides/{slide}/thumbnail?max_size=128 returns 200" "200" "$STATUS" || true
assert_file_type "Small thumbnail is a valid JPEG image" "JPEG" "$THUMB_SMALL_PATH" || true

# -----------------------------------------------------------------------------
# Test 6: DZI Descriptor
# -----------------------------------------------------------------------------
echo ""
log_info "Test 6: DZI Descriptor"
RESPONSE=$(curl -s -w "\n%{http_code}" "${BASE_URL}/slides/${TEST_SLIDE}/dzi")
STATUS=$(get_response_status "$RESPONSE")
BODY=$(get_response_body "$RESPONSE")

assert_status "GET /slides/{slide}/dzi returns 200" "200" "$STATUS" || true
assert_contains "DZI contains XML declaration" "<?xml" "$BODY" || true
assert_contains "DZI contains Image element" "<Image" "$BODY" || true
assert_contains "DZI contains Size element" "<Size" "$BODY" || true
assert_contains "DZI specifies TileSize" "TileSize" "$BODY" || true

# -----------------------------------------------------------------------------
# Test 7: Error Handling - Non-existent Slide
# -----------------------------------------------------------------------------
echo ""
log_info "Test 7: Error Handling"

STATUS=$(curl -s -o /dev/null -w "%{http_code}" "${BASE_URL}/slides/non-existent-slide.svs")
assert_status "GET /slides/non-existent.svs returns 404" "404" "$STATUS" || true

STATUS=$(curl -s -o /dev/null -w "%{http_code}" "${BASE_URL}/tiles/non-existent-slide.svs/0/0/0.jpg")
assert_status "GET /tiles/non-existent.svs/... returns 404" "404" "$STATUS" || true

# -----------------------------------------------------------------------------
# Test 8: Check Command with --list-slides
# -----------------------------------------------------------------------------
echo ""
log_info "Test 8: Check Command"
CHECK_OUTPUT=$("$BINARY" check "s3://${BUCKET}" --s3-region "$REGION" --list-slides 2>&1) || true

assert_contains "Check lists test slide" "$TEST_SLIDE" "$CHECK_OUTPUT" || true
assert_contains "Check shows success message" "All checks passed" "$CHECK_OUTPUT" || true

# =============================================================================
# Results Summary
# =============================================================================

echo ""
echo "═══════════════════════════════════════════════════════════════════"
echo "  Test Results Summary"
echo "═══════════════════════════════════════════════════════════════════"
echo ""
echo -e "  ${GREEN}Passed${NC}: $TESTS_PASSED"
echo -e "  ${RED}Failed${NC}: $TESTS_FAILED"
echo ""

if [ "$TESTS_FAILED" -gt 0 ]; then
    log_error "E2E tests failed! $TESTS_FAILED test(s) did not pass."
    echo ""
    echo "Server log:"
    cat "$TEMP_DIR/server.log"
    exit 1
else
    log_info "All E2E tests passed!"
    exit 0
fi
