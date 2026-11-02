# SoftOxide-JXL-Viewer

**SoftOxide JXL Viewer** — [github.com/garyexplains/softoxide-jxl-viewer](https://github.com/garyexplains/softoxide-jxl-viewer) — is a **native image viewer, editor and converter** with first-class **JPEG XL** support, written in Rust. The binaries and crates keep the short name `jxlviewer`.

Everything the app does — decoding, rendering, editing and encoding — happens in-process using
pure-Rust code (`jxl-oxide`, `image`, `fast_image_resize`, `egui/eframe`) plus the **libjxl**
reference encoder, built from pinned C++ sources. The app never calls on OS image codecs or web
views. It runs unchanged on Windows, macOS and Linux.

## Features

### Viewing
- Open **JPEG XL** (codestream or container, lossless and lossy), plus PNG, JPEG, GIF, BMP, TIFF,
  WebP, PNM, and **AVIF stills** (animated sequences and PQ/HLG HDR files are declined
  gracefully with a status message).
- **Folder grid**: "Folder…" (Ctrl+Shift+O) shows every supported image in a folder as a
  thumbnail grid (lazy-loading, off-thread decode, memory-budgeted LRU cache). The grid
  header sorts by **name or date** (ascending/descending toggle) and adjusts the
  **thumbnail size** (slider 96-512 px, or `+`/`-` keys). Select one file, many
  (Ctrl/Shift-click), or bulk-select via the header `Select…` menu or right-click
  (All / All .jxl / All except .jxl / None). **Convert N…** applies one set of save
  options (the Save-as dialog options) to the whole selection — outputs land next to
  each source with overwrite/skip/numbered-name handling, a progress readout, and a
  Cancel button. Double-click (or Enter)
  opens an image; **Esc** returns to the grid at the same spot. `jxlviewer <FOLDER>` opens
  a folder directly.
- **Animation playback** for animated JXL: all keyframes are decoded, played in a loop at the
  authored frame rate, with play/pause (`Space`), frame stepping (`,` / `.`) and a frame
  counter in the toolbar. Editing an animation flattens it to a still (the edited frame);
  Save-as exports the current frame.
- **HDR support**: PQ (ST 2084) and HLG (BT.2100) files are detected automatically, decoded
  as float, and perceptually tone-mapped to SDR (extended Reinhard, BT.2408 203-nit reference
  white, Rec.2100 → sRGB gamut conversion). HDR files are tagged in the title bar, status line
  and `info` output.
- Pure-Rust JXL decode with **color management to sRGB** (moxcms) — CMYK/ICC-profile files render.
- **Metadata & info**: the toolbar `ⓘ` button (or the `I` key) opens an image-info panel with
  file facts plus every EXIF tag (parsed with kamadak-exif) and the presence of raw XMP/IPTC.
  The `info` CLI command prints the same summary (make/model, exposure trio, orientation, …).
- **Dark / light theme toggle**: the toolbar
  sun/moon button switches the whole UI between a dark and a light theme (canvas, folder grid,
  dialogs, status bar included; image-anchored overlays stay readable on top of images in
  either). The choice persists in a small settings file (`theme.txt`) under the OS
  configuration directory (`%APPDATA%\SoftOxide\JXL Viewer` on Windows,
  `~/Library/Application Support/SoftOxide/JXL Viewer` on macOS, `~/.config/softoxide-jxl-viewer`
  on Linux). Dark is the default; the native window title bar keeps following the OS.
- **EXIF orientation is applied automatically** for JPEG/PNG/TIFF/WebP sources and for JXL
  (header orientation), so camera-rotated photos display upright everywhere.
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
| `I` | Image info (metadata) dialog |
| `Enter` | Apply the crop selection |
| `Esc` | Close dialog, clear crop selection, back to Pan |
| `+` / `−` | Zoom in / out (about canvas center) |
| `0` / `1` | Fit to window / 100% |
| `←`/`→`, `↑`/`↓`, `Home` | Folder navigation (see above) |

Shortcuts never fire while a text field has focus; while the Save-As or Resize dialog is
open, only `Esc` acts. While an encode runs (busy spinner in the status bar, with elapsed
seconds), saving, editing, the crop tool, Open/drag-drop and folder navigation are locked;
pan/zoom and animation playback stay live. Files are written via a temporary `.part` file
and renamed, so a save can never leave a truncated image behind.

### Editing
- **Crop** (drag a selection, live size readout), **resize** with real resamplers
  (Lanczos3 / CatmullRom / Box / Bilinear / Nearest via `fast_image_resize` SIMD), **rotate 90°
  CW/CCW**, **flip H/V**.
- Undo/redo (bounded snapshots).
- 16-bit sources (PNG-16, TIFF-16, JXL-16) stay 16-bit through the pipeline so exports don't
  lose depth.

### Conversion
- **Save as** JPEG XL, PNG, JPEG, BMP, TIFF — right in the GUI (Save As dialog with per-format
  options). (AVIF has no save path; it is decode-only.)
- **AVIF decode**: still images through vendored libavif+libaom (container, alpha, grid images,
  8/10/12-bit sources surfaces as 8/16-bit storage, `irot`/`imir` transforms applied, EXIF/XMP
  carried into the info panel and Save-as carry-through). Animated AVIF and PQ/HLG HDR are
  declined with friendly errors.
- **Metadata carry-through**: a **Keep metadata** checkbox (JPEG XL / PNG / JPEG targets)
  embeds the source file's EXIF/XMP/IPTC — as ISO-BMFF **"Exif"** and **"xml "** boxes in JPEG
  XL containers (encoded with libjxl itself), `eXIf` + `iTXt` chunks in PNG, and `APP1`
  (EXIF + XMP) / `APP13` (IPTC) segments in JPEG. TIFF/BMP/GIF exports don't carry metadata.
  The EXIF **orientation tag is rewritten to 1** in every carried block, because the saved
  pixels are the upright, already-oriented ones — otherwise viewers honoring EXIF would
  rotate the export a second time. The batch **Convert N…** dialog has the same option and
  carries each source file's *own* metadata; the CLI gets `convert --keep-metadata`.
- **JPEG XL save options**: one-click **presets** (Lossless, Visually lossless, Small — each
  also in a *fast* effort-3 variant) plus *lossless* (mathematically lossless), *distance*
  (VarDCT quality, 1.0 ≈ visually lossless), *effort* (1–10, default 7 — libjxl's own default),
  *keep/drop alpha*, *modular mode* and **tile size** (modular group size: 128/256/512/1024 or
  encoder default).
- **Headless CLI** for scripting & batch use:
  - `jxlviewer convert IN OUT [--lossless] [--distance 1.0] [--effort 7] [--modular]
    [--group-size 256] [--keep-alpha] [--quality 90] [--keep-metadata]`
  - `jxlviewer info FILE…` — format, dimensions, bit depth, alpha, plus an EXIF/XMP/IPTC
    summary (camera, exposure, orientation names, raw-packet sizes).

### JPEG XL-specific behaviors
- Loading JXL uses **jxl-oxide** (pure Rust, MIT/Apache) with moxcms color management; exotic
  files fall back to **libjxl** via custom, permissively-licensed FFI (`jxl-native`).
- Encoding JXL uses **libjxl** (BSD-3-Clause) built from source — this is the only complete,
  standards-compliant JXL encoder today (no pure-Rust encoder exists).
- Alpha can be preserved as a real JXL alpha channel; dropping it produces plain RGB files.

> The file-pickers (`rfd`) are the only OS UI surface used — they are just dialogs;
> the image stack is entirely native/embedded. UI icons come from the vendored Phosphor icon
> font (`assets/fonts/Phosphor.ttf`, MIT) — no system fonts or icon themes involved.

## Building

Requirements: Rust 1.85+, CMake ≥ 3.20, a C++17 compiler, and the standard platform toolchain
(MSVC Build Tools, Xcode CLT, or gcc/clang).

```sh
# 1) fetch libjxl sources (pinned v0.12.0, ~150 MB with submodules)
tools/fetch-libjxl.sh        # or: powershell -File tools/fetch-libjxl.ps1

# 2) fetch libavif + libaom sources (pinned v1.4.2 / v3.15.1) for AVIF decode
tools/fetch-avif.sh          # or: powershell -File tools/fetch-avif.ps1

# 3) build
cargo build --release
```

`JXL_LIBJXL_DIR` can point at another libjxl checkout instead of the pinned one; `AVIF_LIBAVIF_DIR` /
`AVIF_AOM_DIR` likewise for the AVIF pair. First build compiles libjxl, libaom and libavif from
scratch and takes several minutes.

### App icon & packaging

The icon (indigo→teal rounded square with white "JXL" strokes and an amber accent)
is original artwork generated from vector math — regenerate everything with:

```sh
cargo run --release --example gen_icon
```

- **Windows** — `assets/icon.ico` is embedded into the executable at build time
  (`build.rs` + winresource); Explorer, the taskbar and the window all show it.
- **macOS** — build an app bundle: create `jxlviewer.app/Contents/{MacOS,Resources}`,
  copy the binary to `MacOS/jxlviewer` and `assets/icon.icns` to
  `Resources/jxlviewer.icns`, and add an `Info.plist` with
  `CFBundleIconFile` = `jxlviewer`.
- **Linux** — install icons and desktop entry:

  ```sh
  for s in 16 32 48 64 128 256; do
    install -Dm644 assets/icons/jxlviewer-$s.png \
      ~/.local/share/icons/hicolor/${s}x${s}/apps/jxlviewer.png
  done
  install -m644 assets/jxlviewer.desktop ~/.local/share/applications/
  ```
  (Window/taskbar icons also work on X11 without a desktop entry; Wayland
  requires it.)

### Windows file associations (double-click integration)

The viewer can register itself as the program Windows uses to open image files:

- **GUI** — the gear icon at the end of the toolbar opens a dialog listing every
  supported type (`.jxl`, `.png`, `.jpg`, `.gif`, `.bmp`, `.tiff`, `.webp`, `.avif`,
  `.pn*` …); toggling a checkbox registers or removes that association at once.
- **CLI** — `jxlviewer register [EXTS…]` (default: `jxl`; `--all` for every
  supported type), `jxlviewer unregister [EXTS…]`, and `jxlviewer registered`
  to list current associations.

Registration is per-user (HKCU registry, no admin rights). When another app is
already the default handler for a type, the viewer is added to its "Open with…"
list instead of silently stealing the default; Windows then offers it through
right-click ▸ Open with / Settings ▸ Default apps.

## Running

```sh
cargo run --release                    # GUI
cargo run --release -- photo.jxl       # GUI with a file preloaded
cargo run --release -- info photo.jxl
cargo run --release -- convert photo.png out.jxl --lossless --group-size 256 --modular
```

On Windows the release binary is a GUI executable (no console is attached); the
headless commands above still work from scripts — their stdout/stderr comes
through when piped or redirected, and the process exit code is preserved.

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
  codec.rs    format sniffing, decode dispatch, save encoders incl. metadata carry
  meta.rs     EXIF/XMP/IPTC extraction (kamadak-exif), save-side box/chunk writers
  avif-native/ pinned libavif+libaom FFI (AVIF still decode), test-fixture encoder
  icons.rs    vendored Phosphor icon font (assets/fonts/Phosphor.ttf)
  hdr.rs      HDR → SDR: PQ/HLG EOTFs, tone mapping, Rec.2100 → sRGB
  nav.rs      folder navigation: prev/next image by name or mtime
jxl-native/   permissively-licensed libjxl FFI (encode + basic decode), vendored cmake build
third_party/  fetched libjxl checkout (not committed)
```

Decisions worth knowing:
- **Rendering**: Windows uses eframe's `wgpu` renderer over **Direct3D 12** — a
  hardware adapter that can present to the window is preferred, and if none
  exists a CPU adapter (Microsoft Basic Render Driver / WARP) is tried as a
  best-effort software-rendering fallback *not yet verified on real
  hardware-less machines*. macOS and Linux keep the OpenGL (`glow`) renderer.
  The renderer is selected explicitly per platform in `run_gui`, so Cargo
  feature unification can never silently change it; the wgpu build is pinned
  to the Direct3D 12 backend + WGSL shader support (`wgpu/dx12`, `wgpu/wgsl`).
- The GUI decodes JXL with **jxl-oxide + moxcms** for correct sRGB rendering (ICC/CMYK-safe).
  libjxl is used for encoding and as a decode fallback.
- `jpegxl-rs` (the GPL-3.0 Rust wrapper for libjxl) was deliberately avoided so this project can
  stay MIT/Apache.
- EXIF orientation is applied automatically at decode (JPEG/PNG/TIFF/WebP via the EXIF tag,
  JXL via its header orientation). On save, the **Keep metadata** option carries EXIF/XMP/
  IPTC through with the orientation tag neutralized to 1 (pixels are saved upright), so
  camera info survives JXL↔PNG/JPEG round-trips. TIFF/BMP/GIF have no carry path.
- Animated JXL plays back, but Save-as exports only the current frame, and editing flattens
  the animation to a still.
- Lossless JXL of grayscale sources is currently rewritten as RGB (channel expansion) — smaller
  gray-true encoding planned.
- **HDR** (PQ/HLG) files are tone-mapped to SDR for display and for SDR exports; there is no
  HDR (float/PQ) *save* path yet.

## License

SoftOxide-JXL-Viewer is dual-licensed: **MIT** (`LICENSE-MIT`) or **Apache-2.0**
(`LICENSE-APACHE`) — use whichever you prefer. `jxl-native` (our libjxl FFI) carries
the same terms.

- **libjxl** (fetched at build time from the pinned v0.12.0 sources, not committed) remains
  under its own BSD-3-Clause license (JPEG XL Project Authors), as do its bundled submodules.
- Runtime dependencies are MIT/Apache-2.0/BSD-style: egui/eframe, image, fast_image_resize,
  jxl-oxide, moxcms, rfd, jpeg-encoder, gif, kamadak-exif (BSD-2-Clause). AVIF stills decode
  through vendored **libavif + libaom** (both BSD-2-Clause; pinned sources fetched by
  `tools/fetch-avif.*`, not committed — same model as libjxl). The vendored Phosphor icon
  font is MIT.
- The application icon is original artwork generated by `examples/gen_icon.rs` —
  no third-party assets.
