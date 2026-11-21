#!/usr/bin/env bash
# Design review screenshots (ENGINEERING E11.5, DESIGN §15): every
# workspace at 1280x800 and 1024x640, plus Preferences and the loading
# screen. Usage: scripts/screenshot.sh [out-dir]   (default target/shots)
set -euo pipefail
OUT="${1:-target/shots}"
mkdir -p "$OUT"
cargo build -q --release -p symphony-app
shot() { env "$@" ./target/release/symphony; }
for size in 1280x800 1024x640; do
  for ws in Default HandDrawn Compositing; do
    shot SYMPHONY_SHOT_SIZE=$size SYMPHONY_WORKSPACE=$ws SYMPHONY_SCREENSHOT="$OUT/$ws-$size.png"
  done
done
shot SYMPHONY_SHOT_SIZE=1280x800 SYMPHONY_SHOT_PREFS=1 SYMPHONY_SCREENSHOT="$OUT/preferences-1280x800.png"
shot SYMPHONY_SHOT_SIZE=1280x800 SYMPHONY_SHOT_PAINT=1 SYMPHONY_SCREENSHOT="$OUT/paint-1280x800.png"
shot SYMPHONY_SHOT_SIZE=1280x800 SYMPHONY_SHOT_TOOL=Pencil SYMPHONY_SCREENSHOT="$OUT/pencil-1280x800.png"
shot SYMPHONY_SHOT_SIZE=1280x800 SYMPHONY_SHOT_LOADING=1 SYMPHONY_SCREENSHOT="$OUT/loading-1280x800.png"
