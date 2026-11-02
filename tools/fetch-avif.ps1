# Fetches libavif + libaom (both pinned) into third_party/ for avif-native to
# build against. avif-native links them statically and exposes decode only.
# Usage: powershell -File tools/fetch-avif.ps1

$ErrorActionPreference = "Stop"
$LIBAVIF_VERSION = "v1.4.2"
$AOM_VERSION = "v3.15.1"
$third = Join-Path $PSScriptRoot "..\third_party"
$libavif = Join-Path $third "libavif"
$aom = Join-Path $third "aom"

if (Test-Path (Join-Path $libavif "include\avif\avif.h")) {
    Write-Host "libavif $LIBAVIF_VERSION already present at $libavif"
} else {
    Write-Host "Cloning libavif $LIBAVIF_VERSION (shallow) ..."
    git clone --depth 1 --branch $LIBAVIF_VERSION https://github.com/AOMediaCodec/libavif $libavif
    if ($LASTEXITCODE -ne 0) { throw "libavif clone failed" }
}

if (Test-Path (Join-Path $aom "aom\aom_encoder.h")) {
    Write-Host "libaom $AOM_VERSION already present at $aom"
} else {
    Write-Host "Cloning libaom $AOM_VERSION (shallow) ..."
    git clone --depth 1 --branch $AOM_VERSION https://aomedia.googlesource.com/aom $aom
    if ($LASTEXITCODE -ne 0) { throw "libaom clone failed" }
}

Write-Host "Done. libavif at $libavif ; libaom at $aom"
