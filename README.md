# jxlviewer

A **native image viewer, editor and converter** with first-class **JPEG XL** support, written in Rust.

Everything the app does — decoding, rendering, editing and encoding — happens in-process using
pure-Rust code (`jxl-oxide`, `image`, `fast_image_resize`, `egui/eframe`) plus the **libjxl**
reference encoder, built from pinned C++ sources. The app never calls on OS image codecs or web
views. It runs unchanged on Windows, macOS and Linux.

## Features

### Viewing
- Open **JPEG XL** (codestream or container, lossless and lossy), plus PNG, JPEG, GIF, BMP, TIFF, WebP, PNM.
- **Animation playback** for animated JXL: all keyframes are decoded, played in a loop at the
  authored frame rate, with play/pause (`Space`), frame stepping (`,` / `.`) and a frame
  counter in the toolbar. Editing an animation flattens it to a still (the edited frame);
  Save-as exports the current frame.
- **HDR support**: PQ (ST 2084) and HLG (BT.2100) files are detected automatically, decoded
  as float, and perceptually tone-mapped to SDR (extended Reinhard, BT.2408 203-nit reference
  white, Rec.2100 → sRGB gamut conversion). HDR files are tagged in the title bar, status line
  and `info` output.
- Pure-Rust JXL decode with **color management to sRGB** (moxcms) — CMYK/ICC-profile files render.
- Zoom (wheel at pointer), pan (drag), fit / 1:1, checkerboard behind transparency.
  Pixel snapping at high zoom, linear filtering otherwise.
- **Keyboard folder navigation** while viewing a file: `←`/`→` move to the previous/next
  recognized image in the same folder **alphabetically**; `↑`/`↓` move by **file modification
  time**. Both wrap around at the ends. `Home` returns to the file the session was opened
  with. The folder listing is refreshed on every keystroke, so files added or removed while
  browsing are picked up immediately.

### Keyboard shortcuts

| Keys | Action |
|---|---|
| `Ctrl+Z` / `Ctrl+Y`, `Ctrl+Shift+Z` | Undo / Redo |
| `Ctrl+O` / `Ctrl+S` | Open… / Save as… |
| `P` / `C` | Pan / Crop tool |
| `R` / `L` | Rotate 90° right / left |
| `H` / `V` | Flip horizontal / vertical |
| `Enter` | Apply the crop selection |
| `Esc` | Close dialog, clear crop selection, back to Pan |
| `+` / `−` | Zoom in / out (about canvas center) |
| `0` / `1` | Fit to window / 100% |
| `←`/`→`, `↑`/`↓`, `Home` | Folder navigation (see above) |

Shortcuts never fire while a text field has focus; while the Save-As or Resize dialog is
open, only `Esc` acts.

### Editing
- **Crop** (drag a selection, live size readout), **resize** with real resamplers
  (Lanczos3 / CatmullRom / Box / Bilinear / Nearest via `fast_image_resize` SIMD), **rotate 90°
  CW/CCW**, **flip H/V**.
- Undo/redo (bounded snapshots).
- 16-bit sources (PNG-16, TIFF-16, JXL-16) stay 16-bit through the pipeline so exports don't
  lose depth.

### Conversion
- **Save as** JPEG XL, PNG, JPEG, BMP, TIFF — right in the GUI (Save As dialog with per-format
  options).
- **JPEG XL save options**: one-click **presets** (Lossless, Visually lossless, Balanced, Small,
  Fast) plus *lossless* (mathematically lossless), *distance* (VarDCT quality,
  1.0 ≈ visually lossless), *effort* (1–10), *keep/drop alpha*, *modular mode* and **tile size**
  (modular group size: 128/256/512/1024 or encoder default).
- **Headless CLI** for scripting & batch use:
  - `jxlviewer convert IN OUT [--lossless] [--distance 1.0] [--effort 7] [--modular]
    [--group-size 256] [--keep-alpha] [--quality 90]`
  - `jxlviewer info FILE…` — format, dimensions, bit depth, alpha.

### JPEG XL-specific behaviors
- Loading JXL uses **jxl-oxide** (pure Rust, MIT/Apache) with moxcms color management; exotic
  files fall back to **libjxl** via custom, permissively-licensed FFI (`jxl-native`).
- Encoding JXL uses **libjxl** (Apache-2.0) built from source — this is the only complete,
  standards-compliant JXL encoder today (no pure-Rust encoder exists).
- Alpha can be preserved as a real JXL alpha channel; dropping it produces plain RGB files.

> The window/url file-pickers (`rfd`) are the only OS UI surface used — they are just dialogs;
> the image stack is entirely native/embedded. UI icons come from the vendored Phosphor icon
> font (`assets/fonts/Phosphor.ttf`, MIT) — no system fonts or icon themes involved.

## Building

Requirements: Rust 1.85+, CMake ≥ 3.20, a C++17 compiler, and the standard platform toolchain
(MSVC Build Tools, Xcode CLT, or gcc/clang).

```sh
# 1) fetch libjxl sources (pinned v0.12.0, ~150 MB with submodules)
tools/fetch-libjxl.sh        # or: powershell -File tools/fetch-libjxl.ps1

# 2) build
cargo build --release
```

`JXL_LIBJXL_DIR` can point at another libjxl checkout instead of the pinned one.
First build compiles libjxl from scratch and takes several minutes.

## Running

```sh
cargo run --release                    # GUI
cargo run --release -- photo.jxl       # GUI with a file preloaded
cargo run --release -- info photo.jxl
cargo run --release -- convert photo.png out.jxl --lossless --group-size 256 --modular
```

## Architecture

```
src/
  main.rs     CLI parsing, launch of GUI or headless commands
  lib.rs      library root (modules + GUI launcher + cli_main)
  cli.rs      headless CLI: info, convert, help
  app.rs      eframe App: toolbar/status/dialog wiring, undo stack, jobs
  view.rs     canvas: zoom/pan math, crop overlay drawing
  dialogs.rs  Save-as (JXL options incl. tiles) and Resize windows
  raster.rs   Raster pixel buffer (RGB/RGBA, 8/16-bit), crop/rotate/flip/resize
  codec.rs    format sniffing, decode dispatch, save encoders
  icons.rs    vendored Phosphor icon font (assets/fonts/Phosphor.ttf)
  hdr.rs      HDR → SDR: PQ/HLG EOTFs, tone mapping, Rec.2100 → sRGB
  nav.rs      folder navigation: prev/next image by name or mtime
jxl-native/   permissively-licensed libjxl FFI (encode + basic decode), vendored cmake build
third_party/  fetched libjxl checkout (not committed)
```

Decisions worth knowing:
- The GUI decodes JXL with **jxl-oxide + moxcms** for correct sRGB rendering (ICC/CMYK-safe).
  libjxl is used for encoding and as a decode fallback.
- `jpegxl-rs` (the GPL-3.0 Rust wrapper for libjxl) was deliberately avoided so this project can
  stay MIT/Apache.
- Exif orientation for JPEG/PNG inputs is not auto-applied yet.
- Animated JXL plays back, but Save-as exports only the current frame, and editing flattens
  the animation to a still.
- Lossless JXL of grayscale sources is currently rewritten as RGB (channel expansion) — smaller
  gray-true encoding planned.
- **HDR** (PQ/HLG) files are tone-mapped to SDR for display and for SDR exports; there is no
  HDR (float/PQ) *save* path yet.

## License

`jxlviewer` and `jxl-native`: MIT (see `LICENSE.md`).
libjxl and its bundled submodules (fetched, not vendored) remain under their own licenses.
Runtime dependencies are MIT/Apache-2.0/BSD-style (egui, image, fast_image_resize, jxl-oxide,
moxcms, rfd, jpeg-encoder).
