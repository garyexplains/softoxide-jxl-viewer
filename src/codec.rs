//! Format detection, decoding (pure-Rust preferred) and encoding.
//!
//! * JPEG XL decode: `jxl-oxide` (pure Rust, moxcms → sRGB output). Fallback:
//!   libjxl via `jxl-native` for exotic files jxl-oxide can't read.
//! * JPEG XL encode: libjxl via `jxl-native` (lossless / distance / effort /
//!   modular mode / tiling via modular group size).
//! * PNG/JPEG/GIF/BMP/TIFF/WebP: `image` crate.

use image::ImageEncoder as _;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use crate::raster::Raster;
use jxl_native::{GroupSize, Pixels};

// ---------------------------------------------------------------------------
// Formats & detection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Jxl,
    Png,
    Jpeg,
    Gif,
    Bmp,
    Tiff,
    WebP,
    Pnm,
    Unknown,
}

impl Format {
    pub fn as_str(self) -> &'static str {
        match self {
            Format::Jxl => "JPEG XL",
            Format::Png => "PNG",
            Format::Jpeg => "JPEG",
            Format::Gif => "GIF",
            Format::Bmp => "BMP",
            Format::Tiff => "TIFF",
            Format::WebP => "WebP",
            Format::Pnm => "PNM (PPM/PGM)",
            Format::Unknown => "Unknown",
        }
    }

    pub fn default_ext(self) -> &'static str {
        match self {
            Format::Jxl => "jxl",
            Format::Png => "png",
            Format::Jpeg => "jpg",
            Format::Bmp => "bmp",
            Format::Tiff => "tiff",
            Format::Pnm => "ppm",
            _ => "png",
        }
    }
}

pub fn format_by_extension(ext: &str) -> Format {
    match ext.to_ascii_lowercase().as_str() {
        "jxl" | "jxlp" | "jph" | "jxs" => Format::Jxl,
        "png" | "apng" => Format::Png,
        "jpg" | "jpeg" | "jfif" | "jpe" => Format::Jpeg,
        "gif" => Format::Gif,
        "bmp" | "dib" => Format::Bmp,
        "tif" | "tiff" => Format::Tiff,
        "webp" => Format::WebP,
        "ppm" | "pgm" | "pbm" | "pnm" => Format::Pnm,
        _ => Format::Unknown,
    }
}

/// Sniffs the canonical format from the file header bytes.
pub fn detect_format(bytes: &[u8], ext_hint: Option<&str>) -> Format {
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0x0A {
        return Format::Jxl; // bare codestream
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"JXL " {
        return Format::Jxl; // container
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Format::Jpeg;
    }
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Format::Png;
    }
    if bytes.starts_with(b"GIF8") {
        return Format::Gif;
    }
    if bytes.starts_with(b"BM") {
        return Format::Bmp;
    }
    if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        return Format::Tiff;
    }
    if bytes.len() > 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Format::WebP;
    }
    // Netpbm: P1..P7 (P6 = color PPM, P5 = gray PGM, PF = float PFM…)
    if bytes.len() >= 3
        && bytes[0] == b'P'
        && matches!(bytes[1], b'1'..=b'7')
        && (bytes[2] as char).is_ascii_whitespace()
    {
        return Format::Pnm;
    }
    if let Some(e) = ext_hint {
        let f = format_by_extension(e);
        if f != Format::Unknown {
            return f;
        }
    }
    Format::Unknown
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct LoadedImage {
    pub raster: Raster,
    pub format: Format,
    pub via_fallback: bool,
    pub note: Option<String>,
    /// "PQ" or "HLG" when the source was HDR (tone-mapped to SDR for display).
    pub hdr: Option<&'static str>,
    /// All keyframes + per-frame durations when the file is animated.
    pub animation: Option<Animation>,
}

/// Decoded animation: one raster per keyframe and its display duration.
#[derive(Debug, Clone)]
pub struct Animation {
    pub frames: Vec<Raster>,
    pub durations_ms: Vec<u32>,
    /// Loop count from the container (0 = loop forever).
    pub loops: u32,
}

impl Animation {
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }
}

/// Converts a frame duration in animation ticks to milliseconds.
/// Ticks per second is `tps_numerator / tps_denominator`.
pub fn animation_duration_ms(ticks: u32, tps_numerator: u32, tps_denominator: u32) -> u32 {
    if tps_numerator == 0 || tps_denominator == 0 {
        return 100;
    }
    let ms = ticks as f64 * (tps_denominator as f64 / tps_numerator as f64) * 1000.0;
    (ms.round() as u64).clamp(10, 60_000) as u32
}

pub fn decode_file(path: &Path) -> Result<LoadedImage, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_string());
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    decode_from_bytes(&bytes, ext.as_deref())
}

pub fn decode_from_bytes(bytes: &[u8], ext_hint: Option<&str>) -> Result<LoadedImage, String> {
    let format = detect_format(bytes, ext_hint);
    match format {
        Format::Jxl => decode_jxl(bytes),
        Format::Gif => decode_gif(bytes),
        Format::Unknown => Err("unknown file format".to_string()),
        _ => decode_with_image_crate(bytes, format, ext_hint),
    }
}

/// JPEG XL decode preference order:
/// * HDR (PQ/HLG): jxl-oxide with a perceptual tone map to SDR (all bit depths).
/// * Animated: jxl-oxide decodes every keyframe.
/// * ≤8-bit SDR stills: jxl-oxide (color managed to sRGB in pure Rust) → libjxl fallback.
/// * >8-bit SDR stills: libjxl first (exact 16-bit integer pipeline), jxl-oxide fallback.
fn decode_jxl(bytes: &[u8]) -> Result<LoadedImage, String> {
    let peek = jxl_header_peek(bytes).ok();
    let hdr_tf = peek.as_ref().and_then(|p| p.hdr).map(|h| match h {
        jxl_oxide::HdrType::Pq => crate::hdr::HdrTf::Pq,
        jxl_oxide::HdrType::Hlg => crate::hdr::HdrTf::Hlg,
    });
    let animated = peek.as_ref().is_some_and(|p| p.animated);
    let header_bits = peek.as_ref().map(|p| p.bits).filter(|b| *b > 8);

    // HDR content: never take the integer paths — they would pass PQ/HLG
    // samples through unconverted or clip them harshly. Decode in the source
    // encoding as f32 and tone-map here.
    if let Some(tf) = hdr_tf {
        if animated {
            if let Ok(img) = decode_jxl_anim_hdr_oxide(bytes, tf) {
                return Ok(img);
            }
        }
        if let Ok(img) = decode_jxl_hdr_oxide(bytes, tf) {
            return Ok(img);
        }
        // Fall through to the regular paths if the HDR decode failed.
    }

    // Animated files: decode all keyframes with jxl-oxide (coalesced).
    if animated {
        if let Ok(img) = decode_jxl_anim_oxide(bytes) {
            return Ok(img);
        }
        // Fall through: static first-frame decode below.
    }

    if header_bits.is_some() {
        if let Ok(raster) = decode_jxl_libjxl(bytes, 16) {
            return Ok(LoadedImage {
                raster,
                format: Format::Jxl,
                via_fallback: false,
                note: Some("decoded with libjxl (16-bit exact path)".into()),
                hdr: None,
                animation: None,
            });
        }
    }

    match decode_jxl_oxide(bytes) {
        Ok(img) => {
            if header_bits.is_some() {
                // We preferred libjxl but it failed; still usable.
                let mut img = img;
                img.note = Some("decoded with jxl-oxide (16-bit approximate)".into());
                Ok(img)
            } else {
                Ok(img)
            }
        }
        Err(primary_err) => decode_jxl_libjxl(bytes, 8)
            .map(|raster| LoadedImage {
                raster,
                format: Format::Jxl,
                via_fallback: true,
                note: Some(format!("decoded with libjxl fallback ({primary_err})")),
                hdr: None,
                animation: None,
            })
            .map_err(|fallback_err| {
                format!("JPEG XL decode failed — jxl-oxide: {primary_err}; libjxl: {fallback_err}")
            }),
    }
}

struct JxlPeek {
    bits: u32,
    hdr: Option<jxl_oxide::HdrType>,
    animated: bool,
}

/// Cheap header peek: sample bit depth, HDR transfer function, animated flag.
fn jxl_header_peek(bytes: &[u8]) -> Result<JxlPeek, String> {
    let image = jxl_oxide::JxlImage::builder()
        .read(Cursor::new(bytes))
        .map_err(|e| format!("jxl header: {e}"))?;
    Ok(JxlPeek {
        bits: image.image_header().metadata.bit_depth.bits_per_sample(),
        hdr: image.hdr_type(),
        animated: image.image_header().metadata.animation.is_some(),
    })
}

/// HDR (PQ/HLG) decode: renders in the source encoding as f32 and applies a
/// perceptual tone map to sRGB. Avoids both failure modes of the integer
/// paths (PQ passthrough, and harsh CMS clipping above SDR white).
fn decode_jxl_hdr_oxide(bytes: &[u8], tf: crate::hdr::HdrTf) -> Result<LoadedImage, String> {
    use jxl_oxide::JxlImage;

    let mut image = JxlImage::builder()
        .read(Cursor::new(bytes))
        .map_err(|e| format!("jxl header: {e}"))?;
    image.set_cms(jxl_oxide::Moxcms);

    // Keep the source HDR encoding (don't let the CMS clip to SDR); we do the
    // EOTF + tone map + gamut conversion ourselves.
    let requested = match tf {
        crate::hdr::HdrTf::Pq => {
            jxl_oxide::EnumColourEncoding::bt2100_pq(jxl_oxide::RenderingIntent::Relative)
        }
        crate::hdr::HdrTf::Hlg => {
            jxl_oxide::EnumColourEncoding::bt2100_hlg(jxl_oxide::RenderingIntent::Relative)
        }
    };
    image.request_color_encoding(requested);

    let peak_from_meta = image
        .image_header()
        .metadata
        .tone_mapping
        .intensity_target
        .max(1.0);

    let has_alpha = image
        .image_header()
        .metadata
        .ec_info
        .iter()
        .any(|ec| matches!(ec.ty, jxl_oxide::ExtraChannelType::Alpha { .. }));

    let render = image
        .render_frame(0)
        .map_err(|e| format!("render frame: {e}"))?;
    let raster = hdr_frame_raster(&render, tf, has_alpha, peak_from_meta);

    Ok(LoadedImage {
        raster,
        format: Format::Jxl,
        via_fallback: false,
        note: None,
        hdr: Some(tf.as_str()),
        animation: None,
    })
}

/// Renders one coalesced HDR frame and tone-maps it to sRGB (u8).
fn hdr_frame_raster(
    render: &jxl_oxide::Render,
    tf: crate::hdr::HdrTf,
    has_alpha: bool,
    peak_from_meta: f32,
) -> Raster {
    let mut stream = render.stream();
    let width = stream.width();
    let height = stream.height();
    let channels = stream.channels() as usize;
    let npix = width as usize * height as usize;

    // Read the raw encoded samples as f32.
    let expect_len = npix * channels;
    let mut raw = vec![0f32; expect_len];
    {
        let mut written = 0;
        while written < expect_len {
            let n = stream.write_to_buffer(&mut raw[written..]);
            if n == 0 {
                break;
            }
            written += n;
        }
    }

    // Pass 1: EOTF → display-linear nits + measure the actual peak luma.
    let mut nits = vec![0f32; npix * 3];
    let mut peak: f32 = 0.0;
    for i in 0..npix {
        let base = i * channels;
        let e = [raw[base], raw[base + 1], raw[base + 2]];
        let lin = crate::hdr::encoded_to_nits(e, tf, peak_from_meta);
        peak = peak.max(crate::hdr::luma2100(lin));
        nits[i * 3..i * 3 + 3].copy_from_slice(&lin);
    }
    // Content peak: prefer the author's intent, but never below what's in the
    // image; clamp to a sane range.
    let peak_nits = if peak_from_meta >= 300.0 {
        peak_from_meta.min(10000.0)
    } else {
        peak.clamp(300.0, 10000.0)
    };

    // Pass 2: tone map → gamut convert → sRGB OETF → u8, with alpha passthrough.
    let mut out = vec![255u8; npix * 4];
    for i in 0..npix {
        let lin = [nits[i * 3], nits[i * 3 + 1], nits[i * 3 + 2]];
        let srgb = crate::hdr::tonemap_to_srgb(lin, peak_nits);
        out[i * 4] = (srgb[0] * 255.0 + 0.5) as u8;
        out[i * 4 + 1] = (srgb[1] * 255.0 + 0.5) as u8;
        out[i * 4 + 2] = (srgb[2] * 255.0 + 0.5) as u8;
        if has_alpha && channels >= 4 {
            let a = raw[i * channels + (channels - 1)].clamp(0.0, 1.0);
            out[i * 4 + 3] = (a * 255.0 + 0.5) as u8;
        }
    }
    let data = if has_alpha {
        out
    } else {
        drop_alpha_to_rgb(&out)
    };

    Raster::from_rgba8(width, height, has_alpha, data)
}

/// GIF decode (static or animated) with proper frame compositing: each frame
/// is alpha-blended onto a canvas, GIF disposal methods (keep / background /
/// previous) are honored, and delays/loop count are preserved.
fn decode_gif(bytes: &[u8]) -> Result<LoadedImage, String> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut reader = options
        .read_info(Cursor::new(bytes))
        .map_err(|e| format!("gif header: {e}"))?;
    let width = reader.width() as u32;
    let height = reader.height() as u32;
    let loops = match reader.repeat() {
        gif::Repeat::Infinite => 0,
        gif::Repeat::Finite(n) => n as u32,
    };

    let canvas_len = width as usize * height as usize * 4;
    let mut canvas = vec![0u8; canvas_len];
    let mut saved_canvas: Option<Vec<u8>> = None;
    let mut frames: Vec<Raster> = Vec::new();
    let mut durations_ms: Vec<u32> = Vec::new();

    while frames.len() < MAX_ANIM_FRAMES {
        let frame = match reader
            .read_next_frame()
            .map_err(|e| format!("gif frame {}: {e}", frames.len()))?
        {
            Some(f) => f,
            None => break,
        };

        // Snapshot for "restore to previous" disposal.
        if matches!(frame.dispose, gif::DisposalMethod::Previous) {
            saved_canvas = Some(canvas.clone());
        }

        // Composite the frame's sub-rect onto the canvas (straight alpha-over).
        let fw = frame.width as usize;
        let fx = frame.left as usize;
        let fy = frame.top as usize;
        for ry in 0..frame.height as usize {
            for rx in 0..fw {
                let s = (ry * fw + rx) * 4;
                let src = &frame.buffer[s..s + 4];
                let d = ((fy + ry) * width as usize + fx + rx) * 4;
                let a = src[3] as u32;
                let ia = 255 - a;
                for c in 0..3 {
                    canvas[d + c] = ((src[c] as u32 * a + canvas[d + c] as u32 * ia) / 255) as u8;
                }
                canvas[d + 3] = ((a * 255 + canvas[d + 3] as u32 * ia) / 255) as u8;
            }
        }

        // The displayed state of this frame.
        frames.push(Raster::from_rgba8(width, height, true, canvas.clone()));
        // GIF delay is in 10 ms units; 0 traditionally displays ~100 ms.
        durations_ms.push(if frame.delay == 0 {
            100
        } else {
            frame.delay as u32 * 10
        });

        // Disposal prepares the canvas for the next frame.
        match frame.dispose {
            gif::DisposalMethod::Background => {
                for ry in 0..frame.height as usize {
                    let d = ((fy + ry) * width as usize + fx) * 4;
                    canvas[d..d + fw * 4].fill(0);
                }
            }
            gif::DisposalMethod::Previous => {
                if let Some(saved) = saved_canvas.take() {
                    canvas = saved;
                }
            }
            _ => {}
        }
    }

    if frames.is_empty() {
        return Err("gif: no frames".into());
    }
    if frames.len() == 1 {
        return Ok(LoadedImage {
            raster: frames.pop().unwrap(),
            format: Format::Gif,
            via_fallback: false,
            note: None,
            hdr: None,
            animation: None,
        });
    }

    let raster = frames[0].clone();
    Ok(LoadedImage {
        raster,
        format: Format::Gif,
        via_fallback: false,
        note: None,
        hdr: None,
        animation: Some(Animation {
            frames,
            durations_ms,
            loops,
        }),
    })
}

/// sRGB target color encoding for the app's SDR decode pipeline.
///
/// When the file is *already* sRGB, the request mirrors the file's own
/// rendering intent so that jxl-oxide treats `request_color_encoding` as an
/// identity operation and skips the CMS pass entirely. Requesting a
/// *different* intent than the file declares (e.g. `Perceptual` over a
/// `Relative`-intent sRGB file) makes moxcms run a float round-trip through
/// a color matrix that perturbs lossless pixels by ±1-2 values, which would
/// break exact-lossless round-trips and shift lossy output slightly.
/// Non-sRGB files (wide gamut, HDR, plain gamma) go through the CMS as
/// intended, targeting sRGB/Perceptual for consistent display.
fn srgb_display_target(image: &jxl_oxide::JxlImage) -> jxl_oxide::EnumColourEncoding {
    use jxl_oxide::color::ColourEncoding;
    match &image.image_header().metadata.colour_encoding {
        ColourEncoding::Enum(enc) if enc.is_srgb() => {
            jxl_oxide::EnumColourEncoding::srgb(enc.rendering_intent)
        }
        _ => jxl_oxide::EnumColourEncoding::srgb(jxl_oxide::RenderingIntent::Perceptual),
    }
}

fn decode_jxl_oxide(bytes: &[u8]) -> Result<LoadedImage, String> {
    use jxl_oxide::JxlImage;

    let mut image = JxlImage::builder()
        .read(Cursor::new(bytes))
        .map_err(|e| format!("jxl header: {e}"))?;

    // Pure-Rust CMS: map everything to sRGB for display/exports.
    // (The moxcms feature is enabled for this crate; make it explicit.)
    image.set_cms(jxl_oxide::Moxcms);

    // See `srgb_display_target`.
    let target = srgb_display_target(&image);
    let _ = image.request_color_encoding(target);

    let file_bits = image.image_header().metadata.bit_depth.bits_per_sample();

    let has_alpha = image
        .image_header()
        .metadata
        .ec_info
        .iter()
        .any(|ec| matches!(ec.ty, jxl_oxide::ExtraChannelType::Alpha { .. }));

    let render = image
        .render_frame(0)
        .map_err(|e| format!("render frame: {e}"))?;
    let raster = sdr_frame_raster(&render, has_alpha, file_bits > 8);

    Ok(LoadedImage {
        raster,
        format: Format::Jxl,
        via_fallback: false,
        note: None,
        hdr: None,
        animation: None,
    })
}

/// Renders one coalesced SDR frame into a Raster (u16 when the file is deep).
fn sdr_frame_raster(render: &jxl_oxide::Render, has_alpha: bool, deep: bool) -> Raster {
    let mut stream = render.stream();
    let width = stream.width();
    let height = stream.height();
    let channels = stream.channels() as usize;
    let npix = width as usize * height as usize;

    if deep {
        let raw = read_stream_u16(&mut stream, npix * channels);
        let out = normalize_stream(&raw, channels, has_alpha, 65535u16);
        let data = if has_alpha { out } else { strip_alpha(out) };
        Raster::from_rgba16(width, height, has_alpha, data)
    } else {
        let raw = read_stream_u8(&mut stream, npix * channels);
        let out = normalize_stream(&raw, channels, has_alpha, 255u8);
        let data = if has_alpha { out } else { strip_alpha(out) };
        Raster::from_rgba8(width, height, has_alpha, data)
    }
}

/// Upper bound on decoded animation keyframes (memory guard).
const MAX_ANIM_FRAMES: usize = 256;

/// Decodes every animation keyframe (coalesced, sRGB) with jxl-oxide.
fn decode_jxl_anim_oxide(bytes: &[u8]) -> Result<LoadedImage, String> {
    use jxl_oxide::JxlImage;

    let mut image = JxlImage::builder()
        .read(Cursor::new(bytes))
        .map_err(|e| format!("jxl header: {e}"))?;
    image.set_cms(jxl_oxide::Moxcms);
    let target = srgb_display_target(&image);
    let _ = image.request_color_encoding(target);

    let has_alpha = image
        .image_header()
        .metadata
        .ec_info
        .iter()
        .any(|ec| matches!(ec.ty, jxl_oxide::ExtraChannelType::Alpha { .. }));
    let deep = image.image_header().metadata.bit_depth.bits_per_sample() > 8;
    let (tps_num, tps_den) = image
        .image_header()
        .metadata
        .animation
        .as_ref()
        .map(|a| (a.tps_numerator, a.tps_denominator))
        .unwrap_or((1000, 1));
    let loops = image
        .image_header()
        .metadata
        .animation
        .as_ref()
        .map(|a| a.num_loops)
        .unwrap_or(0);

    let n = image.num_loaded_keyframes().min(MAX_ANIM_FRAMES);
    let mut frames = Vec::with_capacity(n);
    let mut durations_ms = Vec::with_capacity(n);
    for idx in 0..n {
        let render = image
            .render_frame(idx)
            .map_err(|e| format!("render frame {idx}: {e}"))?;
        frames.push(sdr_frame_raster(&render, has_alpha, deep));
        durations_ms.push(animation_duration_ms(render.duration(), tps_num, tps_den));
    }
    if frames.is_empty() {
        return Err("animation contains no frames".into());
    }

    let raster = frames[0].clone();
    Ok(LoadedImage {
        raster,
        format: Format::Jxl,
        via_fallback: false,
        note: None,
        hdr: None,
        animation: Some(Animation {
            frames,
            durations_ms,
            loops,
        }),
    })
}

/// Decodes every animation keyframe of an HDR file, tone-mapped to sRGB.
fn decode_jxl_anim_hdr_oxide(bytes: &[u8], tf: crate::hdr::HdrTf) -> Result<LoadedImage, String> {
    use jxl_oxide::JxlImage;

    let mut image = JxlImage::builder()
        .read(Cursor::new(bytes))
        .map_err(|e| format!("jxl header: {e}"))?;
    image.set_cms(jxl_oxide::Moxcms);

    let requested = match tf {
        crate::hdr::HdrTf::Pq => {
            jxl_oxide::EnumColourEncoding::bt2100_pq(jxl_oxide::RenderingIntent::Relative)
        }
        crate::hdr::HdrTf::Hlg => {
            jxl_oxide::EnumColourEncoding::bt2100_hlg(jxl_oxide::RenderingIntent::Relative)
        }
    };
    image.request_color_encoding(requested);

    let peak_from_meta = image
        .image_header()
        .metadata
        .tone_mapping
        .intensity_target
        .max(1.0);
    let has_alpha = image
        .image_header()
        .metadata
        .ec_info
        .iter()
        .any(|ec| matches!(ec.ty, jxl_oxide::ExtraChannelType::Alpha { .. }));
    let (tps_num, tps_den) = image
        .image_header()
        .metadata
        .animation
        .as_ref()
        .map(|a| (a.tps_numerator, a.tps_denominator))
        .unwrap_or((1000, 1));
    let loops = image
        .image_header()
        .metadata
        .animation
        .as_ref()
        .map(|a| a.num_loops)
        .unwrap_or(0);

    let n = image.num_loaded_keyframes().min(MAX_ANIM_FRAMES);
    let mut frames = Vec::with_capacity(n);
    let mut durations_ms = Vec::with_capacity(n);
    for idx in 0..n {
        let render = image
            .render_frame(idx)
            .map_err(|e| format!("render frame {idx}: {e}"))?;
        frames.push(hdr_frame_raster(&render, tf, has_alpha, peak_from_meta));
        durations_ms.push(animation_duration_ms(render.duration(), tps_num, tps_den));
    }
    if frames.is_empty() {
        return Err("animation contains no frames".into());
    }

    let raster = frames[0].clone();
    Ok(LoadedImage {
        raster,
        format: Format::Jxl,
        via_fallback: false,
        note: None,
        hdr: Some(tf.as_str()),
        animation: Some(Animation {
            frames,
            durations_ms,
            loops,
        }),
    })
}

#[inline]
fn read_stream_u8(stream: &mut jxl_oxide::ImageStream<'_>, expect_len: usize) -> Vec<u8> {
    let mut raw = vec![0u8; expect_len];
    let mut written = 0;
    while written < expect_len {
        let n = stream.write_to_buffer(&mut raw[written..]);
        if n == 0 {
            break;
        }
        written += n;
    }
    raw
}

#[inline]
fn read_stream_u16(stream: &mut jxl_oxide::ImageStream<'_>, expect_len: usize) -> Vec<u16> {
    let mut raw = vec![0u16; expect_len];
    let mut written = 0;
    while written < expect_len {
        let n = stream.write_to_buffer(&mut raw[written..]);
        if n == 0 {
            break;
        }
        written += n;
    }
    raw
}

/// Normalizes a raw interleaved pixel stream into straight RGBA with
/// `channels` inputs: 1 (gray), 2 (gray+alpha), 3 (RGB) or 4+ (RGB + extras,
/// alpha taken from the last channel when it exists).
fn normalize_stream<T: Copy>(raw: &[T], channels: usize, has_alpha: bool, full: T) -> Vec<T> {
    let npix = raw.len() / channels.max(1);
    let mut out = vec![full; npix * 4];
    match channels {
        3 => {
            for i in 0..npix {
                out[i * 4..i * 4 + 3].copy_from_slice(&raw[i * 3..i * 3 + 3]);
            }
        }
        4 => {
            if has_alpha {
                out = raw.to_vec();
            } else {
                for i in 0..npix {
                    out[i * 4..i * 4 + 3].copy_from_slice(&raw[i * 4..i * 4 + 3]);
                }
            }
        }
        1 => {
            for i in 0..npix {
                let l = raw[i];
                out[i * 4] = l;
                out[i * 4 + 1] = l;
                out[i * 4 + 2] = l;
            }
        }
        2 => {
            for i in 0..npix {
                out[i * 4] = raw[i * 2];
                out[i * 4 + 1] = raw[i * 2];
                out[i * 4 + 2] = raw[i * 2];
                out[i * 4 + 3] = raw[i * 2 + 1];
            }
        }
        n => {
            // RGB-ish approximation: use the first three channels; alpha (if
            // any) sits in the last channel of the stream.
            let alpha_idx = if has_alpha { n - 1 } else { usize::MAX };
            for i in 0..npix {
                let base = i * n;
                out[i * 4] = raw[base];
                out[i * 4 + 1] = raw[base + 1];
                out[i * 4 + 2] = raw[base + 2];
                if alpha_idx != usize::MAX {
                    out[i * 4 + 3] = raw[base + alpha_idx];
                }
            }
        }
    }
    out
}

/// Compacts interleaved RGBA → RGB.
fn strip_alpha<T: Copy + Default>(rgba: Vec<T>) -> Vec<T> {
    let mut out = Vec::with_capacity(rgba.len() / 4 * 3);
    for px in rgba.chunks_exact(4) {
        out.extend_from_slice(&px[0..3]);
    }
    out
}

/// libjxl integer decode path (no CMS conversion). Orientation is applied by
/// the decoder, so the returned raster has display dimensions.
fn decode_jxl_libjxl(bytes: &[u8], target_bits: u32) -> Result<Raster, String> {
    let decoded = jxl_native::decode(bytes, target_bits).map_err(|e| e.to_string())?;
    let info = &decoded.info;
    let has_alpha = info.has_alpha;
    // Orientation 5..8 transposes the image.
    let (w, h) = if (5..=8).contains(&info.orientation) {
        (info.height, info.width)
    } else {
        (info.width, info.height)
    };

    match decoded.pixels {
        Pixels::U8(buf) => Ok(Raster::from_rgba8(w, h, has_alpha, buf)),
        Pixels::U16(buf) => Ok(Raster::from_rgba16(w, h, has_alpha, buf)),
    }
}

fn decode_with_image_crate(
    bytes: &[u8],
    format: Format,
    ext: Option<&str>,
) -> Result<LoadedImage, String> {
    let dynamic = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| format!("open: {e}"))?
        .decode()
        .map_err(|e| format!("decode {}: {e}", ext.unwrap_or("image")))?;

    let w = dynamic.width();
    let h = dynamic.height();

    // Preserve native 16-bit buffers when available (lossless JXL saves!).
    if let image::DynamicImage::ImageRgba16(rgba16) = &dynamic {
        return Ok(LoadedImage {
            raster: Raster::from_rgba16(w, h, true, rgba16.as_raw().clone()),
            format,
            via_fallback: false,
            note: None,
            hdr: None,
            animation: None,
        });
    }
    if let image::DynamicImage::ImageRgb16(rgb16) = &dynamic {
        return Ok(LoadedImage {
            raster: Raster::from_rgba16(w, h, false, rgb16.as_raw().clone()),
            format,
            via_fallback: false,
            note: None,
            hdr: None,
            animation: None,
        });
    }
    if let image::DynamicImage::ImageLumaA16(la) = &dynamic {
        let npix = w as usize * h as usize;
        let mut out = vec![0u16; npix * 4];
        for (i, px) in la.as_raw().chunks_exact(2).enumerate() {
            out[i * 4] = px[0];
            out[i * 4 + 1] = px[0];
            out[i * 4 + 2] = px[0];
            out[i * 4 + 3] = px[1];
        }
        return Ok(LoadedImage {
            raster: Raster::from_rgba16(w, h, true, out),
            format,
            via_fallback: false,
            note: None,
            hdr: None,
            animation: None,
        });
    }
    if let image::DynamicImage::ImageLuma16(l) = &dynamic {
        let npix = w as usize * h as usize;
        let mut out = vec![0u16; npix * 3];
        for (i, px) in l.as_raw().iter().enumerate() {
            out[i * 3] = *px;
            out[i * 3 + 1] = *px;
            out[i * 3 + 2] = *px;
        }
        return Ok(LoadedImage {
            raster: Raster::from_rgba16(w, h, false, out),
            format,
            via_fallback: false,
            note: None,
            hdr: None,
            animation: None,
        });
    }

    // 8-bit path
    let rgba8 = dynamic.to_rgba8();
    let has_alpha = rgba8.as_raw().chunks_exact(4).any(|p| p[3] != 255);
    let raster = if has_alpha {
        Raster::from_rgba8(w, h, true, rgba8.as_raw().clone())
    } else {
        let npix = w as usize * h as usize;
        let mut out = vec![0u8; npix * 3];
        for i in 0..npix {
            out[i * 3..i * 3 + 3].copy_from_slice(&rgba8.as_raw()[i * 4..i * 4 + 3]);
        }
        Raster::from_rgba8(w, h, false, out)
    };
    Ok(LoadedImage {
        raster,
        format,
        via_fallback: false,
        note: None,
        hdr: None,
        animation: None,
    })
}

// ---------------------------------------------------------------------------
// Saving
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct JxlSaveOptions {
    pub lossless: bool,
    pub distance: f32,
    pub effort: i64,
    pub keep_alpha: bool,
    pub modular: bool,
    pub group_size: Option<GroupSize>,
}

impl Default for JxlSaveOptions {
    fn default() -> Self {
        Self {
            lossless: false,
            distance: 1.0,
            effort: 3,
            keep_alpha: true,
            modular: false,
            group_size: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct JpegSaveOptions {
    pub quality: u8,
}

impl Default for JpegSaveOptions {
    fn default() -> Self {
        Self { quality: 90 }
    }
}

#[derive(Debug, Clone)]
pub enum SaveOptions {
    Jxl(JxlSaveOptions),
    Jpeg(JpegSaveOptions),
    Png,
    Bmp,
    Tiff,
    Gif,
}

pub fn encode_to_vec(raster: &Raster, opts: &SaveOptions) -> Result<Vec<u8>, String> {
    match opts {
        SaveOptions::Jxl(j) => encode_jxl(raster, j),
        SaveOptions::Jpeg(j) => encode_jpeg(raster, j),
        SaveOptions::Png => encode_png(raster),
        SaveOptions::Bmp => encode_bmp(raster),
        SaveOptions::Tiff => encode_tiff(raster),
        // Still image GIF (a single frame, short delay). Animated exports go
        // through `save_animation_to_path`.
        SaveOptions::Gif => encode_gif(std::slice::from_ref(raster), &[100], 0),
    }
}

/// Writes via a `.part` sibling file + rename, so an interrupted save (crash,
/// or the app being closed mid-encode) can never leave a truncated image at
/// `path` — the rename is the only step that touches the real file.
fn write_atomic(path: &Path, buf: &[u8]) -> Result<(), String> {
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".part");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, buf).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("finalize {}: {e}", tmp.display()))
}

pub fn save_to_path(path: &Path, raster: &Raster, opts: &SaveOptions) -> Result<(), String> {
    let buf = encode_to_vec(raster, opts)?;
    write_atomic(path, &buf)
}

/// Saves an animation. GIF → animated GIF; JPEG XL → animated JXL
/// (lossless/lossy per the JXL options). Other formats are rejected.
pub fn save_animation_to_path(
    path: &Path,
    frames: &[Raster],
    durations_ms: &[u32],
    loops: u32,
    opts: &SaveOptions,
) -> Result<(), String> {
    let buf = match opts {
        SaveOptions::Gif => encode_gif(frames, durations_ms, loops)?,
        SaveOptions::Jxl(j) => encode_anim_jxl(frames, durations_ms, loops, j)?,
        other => return Err(format!("{:?} does not support animated save", other)),
    };
    write_atomic(path, &buf)
}

/// Encodes frames as an animated JPEG XL (sRGB; lossless or lossy per the
/// options; alpha preserved unless `keep_alpha` is off).
pub fn encode_anim_jxl(
    frames: &[Raster],
    durations_ms: &[u32],
    loops: u32,
    opts: &JxlSaveOptions,
) -> Result<Vec<u8>, String> {
    use crate::raster::Samples;

    let Some(first) = frames.first() else {
        return Err("animation encode: no frames".into());
    };
    if frames
        .iter()
        .any(|f| f.width != first.width || f.height != first.height)
    {
        return Err("animation encode: frames must share one size".into());
    }
    let has_alpha = first.has_alpha && opts.keep_alpha;

    let pixels: Vec<jxl_native::Pixels> = frames
        .iter()
        .map(|f| match (&f.data, has_alpha, f.has_alpha) {
            (Samples::U8(v), true, _) => jxl_native::Pixels::U8(v.clone()),
            (Samples::U8(v), false, _) => jxl_native::Pixels::U8(if f.has_alpha {
                drop_alpha_to_rgb(v)
            } else {
                v.clone()
            }),
            (Samples::U16(v), true, _) => jxl_native::Pixels::U16(v.clone()),
            (Samples::U16(v), false, _) => jxl_native::Pixels::U16(if f.has_alpha {
                drop_alpha_to_rgb(v)
            } else {
                v.clone()
            }),
        })
        .collect();

    jxl_native::encode_anim_dynamic(jxl_native::AnimEncodeInput {
        width: first.width,
        height: first.height,
        has_alpha,
        frames: pixels,
        durations_ms: durations_ms.to_vec(),
        loops,
        options: jxl_native::EncodeOptions {
            lossless: opts.lossless,
            distance: opts.distance,
            effort: opts.effort,
            modular: if opts.modular { Some(true) } else { None },
            group_size: opts.group_size,
        },
    })
    .map_err(|e| format!("jpeg xl animation encode: {e}"))
}

/// Encodes frames as GIF (8-bit indexed, ≤256 colors per frame — the encoder
/// quantizes automatically). Delays are milliseconds; `loops` 0 = forever.
fn encode_gif(frames: &[Raster], durations_ms: &[u32], loops: u32) -> Result<Vec<u8>, String> {
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::{Delay, Frame};

    if frames.is_empty() {
        return Err("gif: no frames".into());
    }
    let mut out = Vec::new();
    let mut enc = GifEncoder::new(std::io::Cursor::new(&mut out));
    enc.set_repeat(if loops == 0 {
        Repeat::Infinite
    } else {
        Repeat::Finite(loops.clamp(1, u16::MAX as u32) as u16)
    })
    .map_err(|e| format!("gif repeat: {e}"))?;

    for (i, frame) in frames.iter().enumerate() {
        let dur = durations_ms.get(i).copied().unwrap_or(100).max(10);
        let rgba = frame.to_rgba8_rgba();
        let img = image::RgbaImage::from_raw(frame.width, frame.height, rgba)
            .ok_or_else(|| "gif: frame buffer size mismatch".to_string())?;
        // image's GifEncoder divides the integer ms ratio by 10 to get GIF's
        // 10-ms units — the ratio must be whole milliseconds.
        let delay = Delay::from_numer_denom_ms(dur, 1);
        enc.encode_frame(Frame::from_parts(img, 0, 0, delay))
            .map_err(|e| format!("gif encode frame {i}: {e}"))?;
    }
    drop(enc); // writes the GIF trailer
    Ok(out)
}

pub fn encode_jxl(raster: &Raster, opts: &JxlSaveOptions) -> Result<Vec<u8>, String> {
    let has_alpha = raster.has_alpha && opts.keep_alpha;
    let pixels = match &raster.data {
        crate::raster::Samples::U8(buf) => {
            if has_alpha {
                Pixels::U8(buf.clone())
            } else if raster.has_alpha {
                Pixels::U8(drop_alpha_to_rgb(buf))
            } else {
                Pixels::U8(buf.clone())
            }
        }
        crate::raster::Samples::U16(buf) => {
            if has_alpha {
                Pixels::U16(buf.clone())
            } else if raster.has_alpha {
                Pixels::U16(drop_alpha_to_rgb(buf))
            } else {
                Pixels::U16(buf.clone())
            }
        }
    };

    jxl_native::encode_dynamic(jxl_native::EncodeInput {
        width: raster.width,
        height: raster.height,
        has_alpha,
        pixels,
        options: jxl_native::EncodeOptions {
            lossless: opts.lossless,
            distance: opts.distance,
            effort: opts.effort,
            modular: if opts.modular { Some(true) } else { None },
            group_size: opts.group_size,
        },
    })
    .map_err(|e| format!("jpeg xl encode: {e}"))
}

/// Compacts an interleaved RGBA buffer to RGB by dropping every 4th channel.
fn drop_alpha_to_rgb<T: Copy + Default>(buf: &[T]) -> Vec<T> {
    let mut out = Vec::with_capacity(buf.len() / 4 * 3);
    for px in buf.chunks_exact(4) {
        out.extend_from_slice(&px[0..3]);
    }
    out
}

fn encode_jpeg(raster: &Raster, opts: &JpegSaveOptions) -> Result<Vec<u8>, String> {
    let flat = if raster.has_alpha {
        raster.flattened_over([255, 255, 255])
    } else {
        raster.clone()
    };
    let pixels: Vec<u8> = match &flat.data {
        crate::raster::Samples::U8(v) => v.clone(),
        crate::raster::Samples::U16(v) => {
            let mut out = vec![0u8; v.len()];
            for (o, s) in out.iter_mut().zip(v.iter()) {
                // u32 math: u16 * 255 overflows u16 for s >= 258.
                *o = (((*s as u32) * 255 + 32767) / 65535) as u8;
            }
            out
        }
    };
    let mut out = Vec::new();
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, opts.quality);
    enc.write_image(
        &pixels,
        raster.width,
        raster.height,
        image::ExtendedColorType::Rgb8,
    )
    .map_err(|e| format!("jpeg encode: {e}"))?;
    Ok(out)
}

fn encode_png(raster: &Raster) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let enc = image::codecs::png::PngEncoder::new(&mut out);
    match &raster.data {
        crate::raster::Samples::U8(v) => {
            let ct = if raster.has_alpha {
                image::ExtendedColorType::Rgba8
            } else {
                image::ExtendedColorType::Rgb8
            };
            enc.write_image(v, raster.width, raster.height, ct)
        }
        crate::raster::Samples::U16(v) => {
            let ct = if raster.has_alpha {
                image::ExtendedColorType::Rgba16
            } else {
                image::ExtendedColorType::Rgb16
            };
            let bytes = unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 2) };
            enc.write_image(bytes, raster.width, raster.height, ct)
        }
    }
    .map_err(|e| format!("png encode: {e}"))?;
    Ok(out)
}

fn encode_tiff(raster: &Raster) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    // tiff encoder requires Write + Seek; Cursor<&mut Vec<u8>> provides both.
    let mut cursor = Cursor::new(&mut out);
    let enc = image::codecs::tiff::TiffEncoder::new(&mut cursor);
    match &raster.data {
        crate::raster::Samples::U8(v) => {
            let ct = if raster.has_alpha {
                image::ExtendedColorType::Rgba8
            } else {
                image::ExtendedColorType::Rgb8
            };
            enc.write_image(v, raster.width, raster.height, ct)
        }
        crate::raster::Samples::U16(v) => {
            let ct = if raster.has_alpha {
                image::ExtendedColorType::Rgba16
            } else {
                image::ExtendedColorType::Rgb16
            };
            let bytes = unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 2) };
            enc.write_image(bytes, raster.width, raster.height, ct)
        }
    }
    .map_err(|e| format!("tiff encode: {e}"))?;
    Ok(out)
}

fn encode_bmp(raster: &Raster) -> Result<Vec<u8>, String> {
    // BMP is 8-bit; flatten alpha and downscale 16-bit sources.
    let flat = if raster.has_alpha {
        raster.flattened_over([255, 255, 255])
    } else {
        raster.clone()
    };
    let pixels = match &flat.data {
        crate::raster::Samples::U8(v) => v.clone(),
        crate::raster::Samples::U16(v) => {
            let mut out = vec![0u8; v.len()];
            for (o, s) in out.iter_mut().zip(v.iter()) {
                // u32 math: u16 * 255 overflows u16 for s >= 258.
                *o = (((*s as u32) * 255 + 32767) / 65535) as u8;
            }
            out
        }
    };
    let mut out = Vec::new();
    let enc = image::codecs::bmp::BmpEncoder::new(&mut out);
    enc.write_image(
        &pixels,
        flat.width,
        flat.height,
        image::ExtendedColorType::Rgb8,
    )
    .map_err(|e| format!("bmp encode: {e}"))?;
    Ok(out)
}

// ---------------------------------------------------------------------------
// Simple info abstraction for CLI
// ---------------------------------------------------------------------------

pub fn info_from_bytes(bytes: &[u8], ext: Option<&str>) -> Result<String, String> {
    let format = detect_format(bytes, ext);
    let img = decode_from_bytes(bytes, ext)?;
    let r = &img.raster;
    let hdr_line = match img.hdr {
        Some(h) => format!("\nHDR: {} (tone-mapped to SDR for display)", h),
        None => String::new(),
    };
    let anim_line = match &img.animation {
        Some(a) => format!(
            "\nanimation: {} frames, per-frame durations {}…{} ms",
            a.frame_count(),
            a.durations_ms.iter().min().copied().unwrap_or(0),
            a.durations_ms.iter().max().copied().unwrap_or(0)
        ),
        None => "\nanimation: no (still image)".to_string(),
    };
    Ok(format!(
        "format: {} ({:?})\nsize: {}×{}\nchannels: {}{}\nsample depth: {}{}{}{}{}{}",
        format.as_str(),
        format,
        r.width,
        r.height,
        r.channels(),
        if r.has_alpha { " (alpha)" } else { "" },
        r.bits,
        if img.via_fallback {
            " (libjxl fallback)"
        } else {
            ""
        },
        img.note
            .as_ref()
            .map(|n| format!("\nnote: {n}"))
            .unwrap_or_default(),
        hdr_line,
        anim_line,
        format!("\nfile size: {} bytes", bytes.len()),
    ))
}
