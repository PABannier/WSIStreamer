#!/bin/bash

if [ -z "$1" ]; then
    echo "Usage: $0 <path/to/slide.svs>"
    exit 1
fi

# Start Docker services
docker-compose up -d

# Set the SVS file path and run tests
export WSI_TEST_SVS_PATH=$1
cargo test --test integration real_service -- --ignored --nocapture