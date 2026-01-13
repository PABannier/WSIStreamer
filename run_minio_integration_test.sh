#!/bin/bash

# Integration test runner for WSI Streamer with MinIO
#
# Usage:
#   ./run_minio_integration_test.sh <path/to/slide.svs>
#       Upload and test a local slide file (uses the original filename)
#
#   WSI_TEST_SLIDE_ID=slide.svs ./run_minio_integration_test.sh
#       Test an existing slide in the MinIO bucket (no upload)
#
# Examples:
#   # Upload and test a local slide
#   ./run_minio_integration_test.sh ~/Downloads/TCGA-G4-6304-01Z-00-DX1.svs
#
#   # Test an existing slide in the bucket
#   WSI_TEST_SLIDE_ID=TCGA-G4-6304-01Z-00-DX1.svs ./run_minio_integration_test.sh
#
# Environment variables:
#   WSI_TEST_SVS_PATH  - Path to a local SVS file to upload and test
#   WSI_TEST_SLIDE_ID  - Slide ID in the bucket to test (skips upload)

# Check if we have either a file path or SLIDE_ID set
if [ -z "$1" ] && [ -z "$WSI_TEST_SLIDE_ID" ]; then
    echo "Usage: $0 <path/to/slide.svs>"
    echo ""
    echo "Or to test an existing slide in the bucket:"
    echo "  WSI_TEST_SLIDE_ID=slide.svs $0"
    exit 1
fi

# Start Docker services
echo "Starting Docker services..."
docker-compose up -d

# Wait for services to be healthy
echo "Waiting for services to be ready..."
sleep 5

if [ -n "$1" ]; then
    # Set the SVS file path (will upload with original filename)
    export WSI_TEST_SVS_PATH=$1
    echo "Testing with local file: $WSI_TEST_SVS_PATH"
elif [ -n "$WSI_TEST_SLIDE_ID" ]; then
    echo "Testing existing slide in bucket: $WSI_TEST_SLIDE_ID"
fi

# Run the integration tests
echo "Running integration tests..."
cargo test --test integration real_service -- --ignored --nocapture
