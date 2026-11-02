#!/bin/sh
# Fetches libavif + libaom (both pinned) into third_party/ for avif-native to
# build against. avif-native links them statically and exposes decode only.
# Usage: ./tools/fetch-avif.sh

set -e
LIBAVIF_VERSION=v1.4.2
AOM_VERSION=v3.15.1
THIRD="$(cd "$(dirname "$0")/.." && pwd)/third_party"
LIBAVIF="$THIRD/libavif"
AOM="$THIRD/aom"

if [ -f "$LIBAVIF/include/avif/avif.h" ]; then
    echo "libavif $LIBAVIF_VERSION already present at $LIBAVIF"
else
    echo "Cloning libavif $LIBAVIF_VERSION (shallow) ..."
    mkdir -p "$(dirname "$LIBAVIF")"
    git clone --depth 1 --branch "$LIBAVIF_VERSION" https://github.com/AOMediaCodec/libavif "$LIBAVIF"
fi

if [ -f "$AOM/aom/aom_encoder.h" ]; then
    echo "libaom $AOM_VERSION already present at $AOM"
else
    echo "Cloning libaom $AOM_VERSION (shallow) ..."
    mkdir -p "$THIRD"
    git clone --depth 1 --branch "$AOM_VERSION" https://aomedia.googlesource.com/aom "$AOM"
fi

echo "Done. libavif at $LIBAVIF ; libaom at $AOM"
