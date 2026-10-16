#!/usr/bin/env bash
# Fetches libjxl (pinned) into third_party/libjxl for jxl-native to build against.
set -euo pipefail

JXL_VERSION=v0.12.0
DEST="$(dirname "$0")/../third_party/libjxl"

if [ -f "$DEST/lib/include/jxl/encode.h" ]; then
  echo "libjxl $JXL_VERSION already present at $DEST"
  exit 0
fi

echo "Cloning libjxl $JXL_VERSION (shallow, with submodules)…"
mkdir -p "$(dirname "$DEST")"
git clone --depth 1 --branch "$JXL_VERSION" \
  --recurse-submodules --shallow-submodules \
  https://github.com/libjxl/libjxl "$DEST"

echo "Done. libjxl $JXL_VERSION at $DEST"
