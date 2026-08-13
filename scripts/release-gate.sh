#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
cargo fmt -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
python3 scripts/test_release_artifacts.py
cargo package --locked --allow-dirty
