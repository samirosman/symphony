#!/usr/bin/env bash
# Render the app in each workspace and save PNGs for design review
# (ENGINEERING E11.5). Usage: scripts/screenshot.sh [out-dir]   (default target/shots)
set -euo pipefail
OUT="${1:-target/shots}"
mkdir -p "$OUT"
cargo build -q --release -p symphony-app
for ws in Default HandDrawn Compositing; do
  SYMPHONY_SCREENSHOT="$OUT/$ws.png" SYMPHONY_WORKSPACE="$ws" SYMPHONY_DEMO_ART=1 ./target/release/symphony
done
