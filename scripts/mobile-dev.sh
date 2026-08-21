#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../mobile"

TARGET="${1:-android}"

case "$TARGET" in
  android|ios)
    npm install
    npm run tauri "$TARGET" dev
    ;;
  *)
    echo "Usage: $0 [android|ios]"
    exit 1
    ;;
esac
