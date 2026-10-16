//! Format detection, decoding (pure-Rust preferred) and encoding.
//!
//! * JPEG XL decode: `jxl-oxide` (pure Rust, moxcms → sRGB output). Fallback:
//!   libjxl via `jxl-native` for exotic files jxl-oxide can't read.
//! * JPEG XL encode: libjxl via `jxl-native` (lossless / distance / effort /
//!   modular mode / tiling via modular group size).
//! * PNG/JPEG/GIF/BMP/TIFF/WebP: `image` crate.

use std::io::Cursor;
use std::path::Path;

use image::ImageEncoder as _;

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
        Format::Unknown => Err("unknown file format".to_string()),
        _ => decode_with_image_crate(bytes, format, ext_hint),
    }
}

/// JPEG XL decode preference order:
/// * HDR (PQ/HLG): jxl-oxide with a perceptual tone map to SDR (all bit depths).
/// * ≤8-bit SDR: jxl-oxide (color managed to sRGB in pure Rust) → libjxl fallback.
/// * >8-bit SDR: libjxl first (exact 16-bit integer pipeline; jxl-oxide's float
///   pipeline drifts ±1 LSB), jxl-oxide fallback for files libjxl can't read.
fn decode_jxl(bytes: &[u8]) -> Result<LoadedImage, String> {
    let peek = jxl_header_peek(bytes).ok();
    let hdr_tf = peek.and_then(|(_, hdr)| hdr);
    let header_bits = peek.map(|(bits, _)| bits).filter(|b| *b > 8);

    // HDR content: never take the integer paths — they would pass PQ/HLG
    // samples through unconverted or clip them harshly. Decode in the source
    // encoding as f32 and tone-map here.
    if let Some(tf) = hdr_tf.map(|h| match h {
        jxl_oxide::HdrType::Pq => crate::hdr::HdrTf::Pq,
        jxl_oxide::HdrType::Hlg => crate::hdr::HdrTf::Hlg,
    }) {
        if let Ok(img) = decode_jxl_hdr_oxide(bytes, tf) {
            return Ok(img);
        }
        // Fall through to the regular paths if the HDR decode failed.
    }

    if header_bits.is_some() {
        if let Ok(raster) = decode_jxl_libjxl(bytes, 16) {
            return Ok(LoadedImage {
                raster,
                format: Format::Jxl,
                via_fallback: false,
                note: Some("decoded with libjxl (16-bit exact path)".into()),
                hdr: None,
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
            })
            .map_err(|fallback_err| {
                format!("JPEG XL decode failed — jxl-oxide: {primary_err}; libjxl: {fallback_err}")
            }),
    }
}

/// Cheap header peek for the file's sample bit depth and HDR transfer function.
fn jxl_header_peek(bytes: &[u8]) -> Result<(u32, Option<jxl_oxide::HdrType>), String> {
    let image = jxl_oxide::JxlImage::builder()
        .read(Cursor::new(bytes))
        .map_err(|e| format!("jxl header: {e}"))?;
    let bits = image.image_header().metadata.bit_depth.bits_per_sample();
    Ok((bits, image.hdr_type()))
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

    let render = image
        .render_frame(0)
        .map_err(|e| format!("render frame: {e}"))?;
    let mut stream = render.stream();
    let width = stream.width();
    let height = stream.height();
    let channels = stream.channels() as usize;
    let npix = width as usize * height as usize;

    let has_alpha = image
        .image_header()
        .metadata
        .ec_info
        .iter()
        .any(|ec| matches!(ec.ty, jxl_oxide::ExtraChannelType::Alpha { .. }));

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

    Ok(LoadedImage {
        raster: Raster::from_rgba8(width, height, has_alpha, data),
        format: Format::Jxl,
        via_fallback: false,
        note: None,
        hdr: Some(tf.as_str()),
    })
}

fn decode_jxl_oxide(bytes: &[u8]) -> Result<LoadedImage, String> {
    use jxl_oxide::{EnumColourEncoding, JxlImage, RenderingIntent};

    let mut image = JxlImage::builder()
        .read(Cursor::new(bytes))
        .map_err(|e| format!("jxl header: {e}"))?;

    // Pure-Rust CMS: map everything to sRGB for display/exports.
    // (The moxcms feature is enabled for this crate; make it explicit.)
    image.set_cms(jxl_oxide::Moxcms);

    let _ = image.request_color_encoding(EnumColourEncoding::srgb(RenderingIntent::Perceptual));

    let file_bits = image.image_header().metadata.bit_depth.bits_per_sample();

    let render = image
        .render_frame(0)
        .map_err(|e| format!("render frame: {e}"))?;

    let mut stream = render.stream();
    let width = stream.width();
    let height = stream.height();
    let channels = stream.channels() as usize;
    let npix = width as usize * height as usize;

    // Is there a real alpha extra channel?
    let has_alpha = image
        .image_header()
        .metadata
        .ec_info
        .iter()
        .any(|ec| matches!(ec.ty, jxl_oxide::ExtraChannelType::Alpha { .. }));

    // Decode samples at the file's native depth when > 8 bits.
    if file_bits > 8 {
        let raw = read_stream_u16(&mut stream, npix * channels);
        let out = normalize_stream(&raw, channels, has_alpha, 65535u16);
        let data = if has_alpha { out } else { strip_alpha(out) };
        return Ok(LoadedImage {
            raster: Raster::from_rgba16(width, height, has_alpha, data),
            format: Format::Jxl,
            via_fallback: false,
            note: None,
            hdr: None,
        });
    }

    let raw = read_stream_u8(&mut stream, npix * channels);
    let out = normalize_stream(&raw, channels, has_alpha, 255u8);
    let data = if has_alpha { out } else { strip_alpha(out) };
    Ok(LoadedImage {
        raster: Raster::from_rgba8(width, height, has_alpha, data),
        format: Format::Jxl,
        via_fallback: false,
        note: None,
        hdr: None,
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
        });
    }
    if let image::DynamicImage::ImageRgb16(rgb16) = &dynamic {
        return Ok(LoadedImage {
            raster: Raster::from_rgba16(w, h, false, rgb16.as_raw().clone()),
            format,
            via_fallback: false,
            note: None,
            hdr: None,
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
            effort: 7,
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
}

pub fn encode_to_vec(raster: &Raster, opts: &SaveOptions) -> Result<Vec<u8>, String> {
    match opts {
        SaveOptions::Jxl(j) => encode_jxl(raster, j),
        SaveOptions::Jpeg(j) => encode_jpeg(raster, j),
        SaveOptions::Png => encode_png(raster),
        SaveOptions::Bmp => encode_bmp(raster),
        SaveOptions::Tiff => encode_tiff(raster),
    }
}

pub fn save_to_path(path: &Path, raster: &Raster, opts: &SaveOptions) -> Result<(), String> {
    let buf = encode_to_vec(raster, opts)?;
    std::fs::write(path, buf).map_err(|e| format!("write {}: {e}", path.display()))
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
    Ok(format!(
        "format: {} ({:?})\nsize: {}×{}\nchannels: {}{}\nsample depth: {}{}\nanimated: n/a (first frame shown){}{}{}",
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
        format!("\nfile size: {} bytes", bytes.len()),
    ))
}
