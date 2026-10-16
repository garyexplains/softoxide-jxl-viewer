# Fetches libjxl (pinned) into third_party/libjxl for jxl-native to build against.
# Usage: ./tools/fetch-libjxl.sh   (or: powershell -File tools/fetch-libjxl.ps1)

$ErrorActionPreference = "Stop"
$JXL_VERSION = "v0.12.0"
$dest = Join-Path $PSScriptRoot "..\third_party\libjxl"

if (Test-Path (Join-Path $dest "lib\include\jxl\encode.h")) {
    Write-Host "libjxl $JXL_VERSION already present at $dest"
    exit 0
}

Write-Host "Cloning libjxl $JXL_VERSION (shallow, with submodules)…"
$parent = Split-Path -Parent $dest
New-Item -ItemType Directory -Force -Path $parent | Out-Null
git clone --depth 1 --branch $JXL_VERSION --recurse-submodules --shallow-submodules `
    https://github.com/libjxl/libjxl $dest
if ($LASTEXITCODE -ne 0) { throw "git clone failed" }

Write-Host "Done. libjxl $JXL_VERSION at $dest"
