#!/usr/bin/env bash
# Everything a commit must pass (ENGINEERING: each commit builds and is
# clean). Use as: scripts/check.sh && git commit ...
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all -- --check
nice -n 10 cargo clippy -q --workspace --all-targets -- -D warnings
nice -n 10 cargo test -q --workspace
echo "All checks passed."
