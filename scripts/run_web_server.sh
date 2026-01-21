#!/usr/bin/env bash
set -euo pipefail

yarn dev &
YARN_PID=$!

cleanup() {
    kill "$YARN_PID" 2>/dev/null || true
}
trap cleanup EXIT

cd ..
cd src-tauri/src/bin
cargo run --bin web_server
