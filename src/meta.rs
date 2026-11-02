//! Image metadata: EXIF / XMP / IPTC extraction for display (and, in a later
//! stage, carry-through on save).
//!
//! * EXIF is parsed with `kamadak-exif` from the containers that carry it:
//!   JPEG (APP1), TIFF, PNG (eXIf chunk), WebP (EXIF chunk), and bare TIFF
//!   blocks (JPEG XL's Exif box payload).
//! * XMP: JPEG APP1 "xap" segments, PNG `iTXt XML:com.adobe.xmp`, TIFF
//!   tag 700, WebP "XMP " chunk, JPEG XL "xml " boxes — carried as raw bytes.
//! * IPTC: raw APP13 "Photoshop 3.0" payloads from JPEG (presence only).
//!
//! EXIF orientation is *not* applied here; the codec decides (it must know
//! whether the underlying decoder already rotated the pixels).

use std::io::Cursor;

use crate::codec::Format;

/// Parsed EXIF: the raw TIFF block (carried through on save) plus
/// display-ready values.
#[derive(Debug, Clone, Default)]
pub struct ExifBlock {
    /// Canonical TIFF block (byte-order header .. end).
    pub raw: Vec<u8>,
    /// 1..=8 per the EXIF spec; 1 = normal.
    pub orientation: u16,
    /// Set once the viewer rotated the pixels for this orientation; a save
    /// must then neutralize the tag instead of copying it verbatim.
    pub applied: bool,
    /// Every present tag as (name, display string).
    pub tags: Vec<(String, String)>,
}

impl ExifBlock {
    /// The curated fields shown first in the info panel / CLI output.
    pub fn key_fields(&self) -> Vec<(String, String)> {
        const KEYS: &[&str] = &[
            "Make",
            "Model",
            "LensModel",
            "ExposureTime",
            "FNumber",
            "ISOSpeedRatings",
            "PhotographicSensitivity",
            "FocalLength",
            "DateTimeOriginal",
            "DateTime",
            "Orientation",
        ];
        self.tags
            .iter()
            .filter(|(name, _)| KEYS.iter().any(|k| k == name))
            .cloned()
            .collect()
    }
}

/// All metadata found in a decoded file.
#[derive(Debug, Clone)]
pub struct ImageMetadata {
    pub exif: Option<ExifBlock>,
    pub xmp: Option<Vec<u8>>,
    pub iptc: Option<Vec<u8>>,
}

impl ImageMetadata {
    pub fn is_empty(&self) -> bool {
        self.exif.is_none() && self.xmp.is_none() && self.iptc.is_none()
    }

    /// EXIF orientation if it requests a transform (2..=8).
    pub fn orientation(&self) -> Option<u16> {
        let o = self.exif.as_ref()?.orientation;
        (2..=8).contains(&o).then_some(o)
    }

    /// True when the viewer rotated the pixels for the orientation tag.
    pub fn orientation_applied(&self) -> bool {
        self.exif.as_ref().is_some_and(|e| e.applied)
    }

    /// Human name for an orientation value (1..8).
    pub fn orientation_name(o: u16) -> &'static str {
        match o {
            1 => "Normal",
            2 => "Mirror horizontal",
            3 => "Rotate 180",
            4 => "Mirror vertical",
            5 => "Transpose",
            6 => "Rotate 90 CW",
            7 => "Transverse",
            8 => "Rotate 90 CCW",
            _ => "Unknown",
        }
    }
}

/// Extracts metadata from raw file bytes for the detected format. Cheap:
/// header/segment scans plus EXIF parsing; no pixel decode. Returns `None`
/// when the container carries nothing we track (GIF, BMP, PNM, bad JXL).
pub fn from_bytes(bytes: &[u8], format: Format) -> Option<ImageMetadata> {
    match format {
        Format::Jpeg => Some(from_jpeg(bytes)),
        Format::Png => Some(from_png(bytes)),
        Format::Tiff => Some(from_tiff(bytes)),
        Format::WebP => Some(from_webp(bytes)),
        Format::Jxl => from_jxl_peek(bytes),
        _ => None,
    }
}

/// Builds metadata from JPEG XL container boxes (Exif box: 4-byte header +
/// TIFF block; "xml " box: raw XMP).
pub fn from_jxl(exif_tiff: Option<Vec<u8>>, xmp: Option<Vec<u8>>) -> Option<ImageMetadata> {
    if exif_tiff.is_none() && xmp.is_none() {
        return None;
    }
    Some(ImageMetadata {
        exif: exif_tiff.and_then(|t| parse_exif_tiff(&t)),
        xmp: xmp.filter(|x| !x.is_empty()),
        iptc: None,
    })
}

/// JXL metadata without a decode: parses the container's boxes via a
/// header-level jxl-oxide read.
fn from_jxl_peek(bytes: &[u8]) -> Option<ImageMetadata> {
    let image = jxl_oxide::JxlImage::builder()
        .read(Cursor::new(bytes))
        .ok()?;
    let boxes = image.aux_boxes();
    let exif_tiff = match boxes.first_exif() {
        Ok(data) if data.has_data() => {
            let raw = data.unwrap();
            let off = raw.tiff_header_offset() as usize;
            raw.payload().get(off..).map(|t| t.to_vec())
        }
        _ => None,
    };
    let xml = boxes.first_xml().unwrap_or(&[]);
    let xmp = (!xml.is_empty()).then(|| xml.to_vec());
    from_jxl(exif_tiff, xmp)
}

fn from_jpeg(bytes: &[u8]) -> ImageMetadata {
    let mut meta = ImageMetadata {
        exif: None,
        xmp: None,
        iptc: None,
    };
    for (marker, payload) in jpeg_app_segments(bytes) {
        match marker {
            0xE1 => {
                // APP1: either EXIF ("Exif\0\0") or XMP ("http://ns.adobe.com/xap/1.0/\0").
                if meta.exif.is_none() && payload.len() > 6 && payload.starts_with(b"Exif\0\0") {
                    meta.exif = parse_exif_tiff(&payload[6..]);
                } else if meta.xmp.is_none()
                    && payload.len() > 29
                    && payload.starts_with(b"http://ns.adobe.com/xap/1.0/\0")
                {
                    meta.xmp = Some(payload[29..].to_vec());
                }
            }
            0xED => {
                // APP13 "Photoshop 3.0\0" — IPTC lives inside the 8BIM blocks.
                if meta.iptc.is_none() && payload.starts_with(b"Photoshop 3.0\0") {
                    meta.iptc = Some(payload);
                }
            }
            _ => {}
        }
    }
    meta
}

/// APPn segments of a JPEG: (marker low byte, payload between the length
/// field and the next marker). Stops at SOS/EOI.
fn jpeg_app_segments(bytes: &[u8]) -> Vec<(u8, Vec<u8>)> {
    let mut out = Vec::new();
    if bytes.first() != Some(&0xFF) || bytes.get(1) != Some(&0xD8) {
        return out; // not a JPEG
    }
    let mut pos = 2usize;
    while pos + 4 <= bytes.len() {
        if bytes[pos] != 0xFF {
            break;
        }
        let marker = bytes[pos + 1];
        if marker == 0xDA || marker == 0xD9 {
            break; // start of scan / end
        }
        let len = u16::from_be_bytes([bytes[pos + 2], bytes[pos + 3]]) as usize;
        if len < 2 || pos + 2 + len > bytes.len() {
            break;
        }
        if (0xE0..=0xEF).contains(&marker) {
            out.push((marker, bytes[pos + 4..pos + 2 + len].to_vec()));
        }
        pos += 2 + len;
    }
    out
}

fn from_png(bytes: &[u8]) -> ImageMetadata {
    let mut meta = ImageMetadata {
        exif: None,
        xmp: None,
        iptc: None,
    };
    // PNG signature is 8 bytes; chunks: len(4) type(4) data(len) crc(4).
    let mut pos = 8usize;
    while pos + 8 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
            as usize;
        let ty = &bytes[pos + 4..pos + 8];
        let data_start = pos + 8;
        let Some(data_end) = data_start.checked_add(len) else {
            break;
        };
        if data_end + 4 > bytes.len() {
            break;
        }
        let data = &bytes[data_start..data_end];
        match ty {
            b"eXIf" if meta.exif.is_none() => {
                meta.exif = parse_exif_tiff(data);
            }
            b"iTXt" if meta.xmp.is_none() => {
                // keyword\0 compression_flag method language\0 translated\0 text
                if let Some(kw_end) = data.iter().position(|&b| b == 0) {
                    if &data[..kw_end] == b"XML:com.adobe.xmp" {
                        let mut p = kw_end + 1 + 2; // keyword nul + flag + method
                        for _ in 0..2 {
                            while p < data.len() && data[p] != 0 {
                                p += 1; // language, then translated keyword
                            }
                            p += 1; // their nul
                        }
                        if p <= data.len() {
                            meta.xmp = Some(data[p..].to_vec());
                        }
                    }
                }
            }
            b"IEND" => break,
            _ => {}
        }
        pos = data_end + 4;
    }
    meta
}

fn from_tiff(bytes: &[u8]) -> ImageMetadata {
    let mut meta = ImageMetadata {
        exif: None,
        xmp: None,
        iptc: None,
    };
    if let Ok(ex) = exif::Reader::new().read_from_container(&mut Cursor::new(bytes)) {
        meta.exif = Some(block_from(ex.buf().to_vec(), &ex));
        // An XMP packet may live in tag 700 (0x02BC).
        meta.xmp = tiff_xmp_packet(&ex);
    }
    meta
}

fn from_webp(bytes: &[u8]) -> ImageMetadata {
    let mut meta = ImageMetadata {
        exif: None,
        xmp: None,
        iptc: None,
    };
    // RIFF container: "RIFF" len "WEBP" then chunks (fourcc, len, data, pad).
    if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        let mut pos = 12usize;
        while pos + 8 <= bytes.len() {
            let ty = &bytes[pos..pos + 4];
            let len = u32::from_le_bytes([
                bytes[pos + 4],
                bytes[pos + 5],
                bytes[pos + 6],
                bytes[pos + 7],
            ]) as usize;
            let data_start = pos + 8;
            let data_end = (data_start + len).min(bytes.len());
            let data = &bytes[data_start..data_end];
            match ty {
                b"EXIF" if meta.exif.is_none() => {
                    // Coded per spec as a raw TIFF block; tolerate the
                    // "Exif\0\0"-style prefix some writers emit.
                    let tiff = data.strip_prefix(b"Exif\0\0").unwrap_or(data);
                    meta.exif = parse_exif_tiff(tiff);
                }
                b"XMP " if meta.xmp.is_none() => {
                    meta.xmp = Some(data.to_vec());
                }
                _ => {}
            }
            pos = data_start + len + (len & 1); // chunks are word-aligned
        }
    }
    if meta.exif.is_none() {
        // Fall back to kamadak's own WebP/EXIF sniffing.
        meta.exif = parse_exif_container(bytes);
    }
    meta
}

/// Parses a bare TIFF/EXIF block into the display model (JPEG APP1 payload,
/// PNG eXIf chunk, WebP EXIF chunk, JXL Exif box payload).
fn parse_exif_tiff(tiff: &[u8]) -> Option<ExifBlock> {
    let ex = exif::Reader::new().read_raw(tiff.to_vec()).ok()?;
    Some(block_from(tiff.to_vec(), &ex))
}

/// Parses EXIF out of a full container file (kamadak sniffs JPEG/TIFF/PNG/
/// WebP/HEIF). Used only as a fallback for WebP oddities.
fn parse_exif_container(bytes: &[u8]) -> Option<ExifBlock> {
    let ex = exif::Reader::new()
        .read_from_container(&mut Cursor::new(bytes))
        .ok()?;
    Some(block_from(ex.buf().to_vec(), &ex))
}

fn block_from(raw: Vec<u8>, ex: &exif::Exif) -> ExifBlock {
    let orientation = ex
        .get_field(exif::Tag::Orientation, exif::In::PRIMARY)
        .and_then(|f| f.value.get_uint(0))
        .unwrap_or(1)
        .clamp(1, 8) as u16;
    let tags: Vec<(String, String)> = ex
        .fields()
        .map(|f| (f.tag.to_string(), f.display_value().to_string()))
        .collect();
    ExifBlock {
        raw,
        orientation,
        applied: false,
        tags,
    }
}

/// XMP packet from TIFF tag 700 (0x02BC), if present.
fn tiff_xmp_packet(ex: &exif::Exif) -> Option<Vec<u8>> {
    let field = ex.get_field(exif::Tag(exif::Context::Tiff, 0x02bc), exif::In::PRIMARY)?;
    let bytes = match &field.value {
        exif::Value::Undefined(v, _) => v.clone(),
        exif::Value::Byte(v) => v.clone(),
        _ => return None,
    };
    (!bytes.is_empty()).then_some(bytes)
}

/// Human-readable byte count ("64000 B", "12.3 KiB", "1.5 MiB").
pub fn human_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    }
}

// ---------------------------------------------------------------------------
// Metadata carry-through on save (the viewer bakes orientation-aware pixels
// upright, so a carried EXIF block must never request another transform).
// ---------------------------------------------------------------------------

/// Rewrites the EXIF orientation tag (0x0112) in a raw TIFF block to 1
/// ("no transform") per the block's byte order. Returns true when changed.
///
/// This is what makes carried metadata safe: the pixels we save are the
/// upright (already-oriented) ones, so keeping the original orientation would
/// make viewers rotate the image a second time.
pub fn neutralize_orientation(tiff: &mut Vec<u8>) -> bool {
    if tiff.len() < 8 {
        return false;
    }
    let little = match &tiff[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return false,
    };
    let u16_of = |b: &[u8]| -> u16 {
        if little {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        }
    };
    let u32_of = |b: &[u8]| -> u32 {
        if little {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        }
    };
    if u16_of(&tiff[2..4]) != 42 {
        return false;
    }
    let ifd = u32_of(&tiff[4..8]) as usize;
    // Worst-case entry chain: count, 12-byte entries, next-IFD word.
    if ifd + 2 + 12 + 4 > tiff.len() {
        return false;
    }
    let count = u16_of(&tiff[ifd..ifd + 2]) as usize;
    if ifd + 2 + 12 * count + 4 > tiff.len() {
        return false;
    }
    let mut changed = false;
    for e in 0..count {
        let base = ifd + 2 + 12 * e;
        if u16_of(&tiff[base..base + 2]) == 0x0112 {
            if u16_of(&tiff[base + 2..base + 4]) != 3 {
                return false; // orientation must be SHORT; leave unknown alone
            }
            // Inline value: SHORT count=1 lives in the first 2 value bytes.
            let pos = base + 8;
            if u16_of(&tiff[pos..pos + 2]) != 1 {
                tiff[pos] = 1;
                tiff[pos + 1] = 0;
                changed = true;
            }
        }
    }
    changed
}

/// TIFF payload for a save: the carried block with its orientation tag
/// neutralized (pixels are saved upright; see [`neutralize_orientation`]).
fn orientation_neutral_tiff(ex: &ExifBlock) -> Vec<u8> {
    let mut tiff = ex.raw.clone();
    neutralize_orientation(&mut tiff);
    tiff
}

/// JPEG XL "Exif" box contents: 4 zero bytes (TIFF header follows
/// immediately) + the neutralized TIFF block.
pub fn exif_box_content(ex: &ExifBlock) -> Vec<u8> {
    let mut v = vec![0u8; 4];
    v.extend_from_slice(&orientation_neutral_tiff(ex));
    v
}

/// PNG "eXIf" chunk data: the neutralized TIFF block.
pub fn exif_png_chunk_data(ex: &ExifBlock) -> Vec<u8> {
    orientation_neutral_tiff(ex)
}

/// JPEG APP1 payload: "Exif\0\0" + the neutralized TIFF block.
pub fn jpeg_exif_app1_payload(ex: &ExifBlock) -> Vec<u8> {
    let mut v = b"Exif\0\0".to_vec();
    v.extend_from_slice(&orientation_neutral_tiff(ex));
    v
}

pub fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for i in 0..256u32 {
        let mut c = i;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        table[i as usize] = c;
    }
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

/// Inserts a chunk right after the 8-byte signature + IHDR (25 bytes →
/// offset 33) of an encoded PNG.
pub fn png_insert_chunk(png: &mut Vec<u8>, ty: &[u8; 4], data: &[u8]) {
    let mut chunk = Vec::with_capacity(data.len() + 12);
    chunk.extend_from_slice(&(data.len() as u32).to_be_bytes());
    chunk.extend_from_slice(ty);
    chunk.extend_from_slice(data);
    let mut crc = ty.to_vec();
    crc.extend_from_slice(data);
    chunk.extend_from_slice(&crc32(&crc).to_be_bytes());

    const AFTER_IHDR: usize = 8 + 25;
    if png.len() < AFTER_IHDR {
        png.extend_from_slice(&chunk);
        return;
    }
    let mut out = Vec::with_capacity(png.len() + chunk.len());
    out.extend_from_slice(&png[..AFTER_IHDR]);
    out.extend_from_slice(&chunk);
    out.extend_from_slice(&png[AFTER_IHDR..]);
    *png = out;
}

/// iTXt chunk data carrying XMP (keyword `XML:com.adobe.xmp`, uncompressed).
pub fn png_xmp_itxt_data(xmp: &[u8]) -> Vec<u8> {
    let mut v = b"XML:com.adobe.xmp\0".to_vec();
    v.extend_from_slice(&[0, 0, 0, 0]); // flag, method, lang\0, translated\0
    v.extend_from_slice(xmp);
    v
}

/// Inserts one marker segment right after the JPEG SOI (APPn length =
/// payload + the 2 length bytes, big-endian).
pub fn jpeg_insert_segment(jpeg: &mut Vec<u8>, marker: u8, payload: &[u8]) {
    let mut out = Vec::with_capacity(jpeg.len() + payload.len() + 4);
    out.extend_from_slice(b"\xFF\xD8");
    out.push(0xFF);
    out.push(marker);
    out.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(payload);
    out.extend_from_slice(&jpeg[2.min(jpeg.len())..]);
    *jpeg = out;
}
