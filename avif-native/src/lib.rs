//! Minimal, permissively-licensed Rust API over a statically-linked libavif +
//! libaom (pinned: libavif v1.4.x, libaom v3.15.x):
//!
//! * [`decode`] â€” still AVIF (single primary item) â†’ RGBA 8/16-bit, with the
//!   container's `irot`/`imir` transforms reported for the caller to apply,
//!   plus the EXIF/XMP payloads the container carries.
//! * [`encode_fixture`] (behind the `fixture-encoder` cargo feature) â€” one
//!   solid-color still-image encoder used solely by `examples/gen_avif.rs`
//!   to (re)generate the small test fixtures committed under `assets/avif/`.
//!   Not wired into the shipped app/CLI in any way.
//!
//! Deliberately not supported, by request: animated AVIF (image sequences)
//! and PQ/HLG HDR transfer characteristics â€” both reject with a friendly
//! error at decode time.
//!
//! License: MIT OR Apache-2.0 (this binding); libavif is BSD-2-Clause and
//! libaom is BSD-2-Clause (with a patented element covered by its own
//! license statement), consistent with this workspace's licensing rules.

use std::error::Error;
use std::fmt;

#[allow(non_camel_case_types, non_snake_case, dead_code, unused)]
pub mod ffi {
    use std::os::raw::{c_char, c_int, c_uint, c_void};

    pub type avifBool = c_int;
    pub const AVIF_TRUE: avifBool = 1;
    pub const AVIF_FALSE: avifBool = 0;

    pub type avifResult = c_int;
    pub const AVIF_RESULT_OK: avifResult = 0;
    pub const AVIF_ADD_IMAGE_FLAG_NONE: c_uint = 0;

    /// `avifPixelFormat`
    pub type avifPixelFormat = c_int;
    pub const AVIF_PIXEL_FORMAT_NONE: avifPixelFormat = 0;
    pub const AVIF_PIXEL_FORMAT_YUV444: avifPixelFormat = 1;

    /// `avifRGBFormat`
    pub type avifRGBFormat = c_int;
    pub const AVIF_RGB_FORMAT_RGB: avifRGBFormat = 0;
    pub const AVIF_RGB_FORMAT_RGBA: avifRGBFormat = 1;

    pub type avifChromaUpsampling = c_int;
    pub const AVIF_CHROMA_UPSAMPLING_AUTOMATIC: avifChromaUpsampling = 0;
    pub const AVIF_CHROMA_UPSAMPLING_BILINEAR: avifChromaUpsampling = 4;

    pub type avifCodecChoice = c_int;
    pub type avifDecoderSource = c_int;
    pub type avifProgressiveState = c_int;
    pub type avifImageContentTypeFlags = c_uint;

    pub const AVIF_TRANSFORM_NONE: u32 = 0;
    pub const AVIF_TRANSFORM_IROT: u32 = 1 << 2;
    pub const AVIF_TRANSFORM_IMIR: u32 = 1 << 3;

    /// Mirrors `avifRWData` from avif.h.
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct avifRWData {
        pub data: *mut u8,
        pub size: usize,
    }

    /// Mirrors `avifPixelFormatInfo`.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct avifPixelFormatInfo {
        pub monochrome: avifBool,
        pub chromaShiftX: c_int,
        pub chromaShiftY: c_int,
    }

    /// Mirrors `avifPixelAspectRatioBox`.
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct avifPixelAspectRatioBox {
        pub hSpacing: u32,
        pub vSpacing: u32,
    }

    /// Mirrors `avifCleanApertureBox` (v1.4: width/height/offsets, each a
    /// fractional pair — 8 × u32).
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct avifCleanApertureBox {
        pub widthN: u32,
        pub widthD: u32,
        pub heightN: u32,
        pub heightD: u32,
        pub horizOffN: u32,
        pub horizOffD: u32,
        pub vertOffN: u32,
        pub vertOffD: u32,
    }

    /// Mirrors `avifImageRotation`.
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct avifImageRotation {
        pub angle: u8,
    }

    /// Mirrors `avifImageMirror`.
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct avifImageMirror {
        pub axis: u8,
    }

    /// Mirrors `avifContentLightLevelInformationBox`.
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct avifContentLightLevelInformationBox {
        pub maxCLL: u16,
        pub maxPALL: u16,
    }

    /// Mirrors `avifImage` from avif.h (libavif v1.2 layout; fields unknown
    /// to us kept as opaque pointers in their exact positions).
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct avifImage {
        pub width: u32,
        pub height: u32,
        pub depth: u32,
        pub yuvFormat: avifPixelFormat,
        pub yuvRange: c_int,
        pub yuvChromaSamplePosition: c_int,
        pub yuvPlanes: [*mut u8; 3],
        pub yuvRowBytes: [u32; 3],
        pub imageOwnsYUVPlanes: avifBool,
        pub alphaPlane: *mut u8,
        pub alphaRowBytes: u32,
        pub imageOwnsAlphaPlane: avifBool,
        pub alphaPremultiplied: avifBool,
        pub icc: avifRWData,
        pub colorPrimaries: u16,
        pub transferCharacteristics: u16,
        pub matrixCoefficients: u16,
        pub clli: avifContentLightLevelInformationBox,
        pub transformFlags: u32,
        pub pasp: avifPixelAspectRatioBox,
        pub clap: avifCleanApertureBox,
        pub irot: avifImageRotation,
        pub imir: avifImageMirror,
        pub exif: avifRWData,
        pub xmp: avifRWData,
        // Version 1.0.0 ends here.
        pub properties: *mut c_void,
        pub numProperties: usize,
        pub gainMap: *mut c_void,
        // Version 1.2.0 ends here. Add any new members after this line.
    }

    /// Mirrors `avifRGBImage` from avif.h.
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct avifRGBImage {
        pub width: u32,
        pub height: u32,
        pub depth: u32,
        pub format: avifRGBFormat,
        pub chromaUpsampling: avifChromaUpsampling,
        pub chromaDownsampling: c_int,
        pub avoidLibYUV: avifBool,
        pub ignoreAlpha: avifBool,
        pub alphaPremultiplied: avifBool,
        pub isFloat: avifBool,
        pub maxThreads: c_int,
        pub pixels: *mut u8,
        pub rowBytes: u32,
    }

    /// Mirrors `avifImageTiming`.
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct avifImageTiming {
        pub timescale: u64,
        pub pts: f64,
        pub ptsInTimescales: u64,
        pub duration: f64,
        pub durationInTimescales: u64,
    }

    /// Mirrors `avifIOStats`.
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct avifIOStats {
        pub colorOBUSize: usize,
        pub alphaOBUSize: usize,
    }

    /// Mirrors `avifDiagnostics`.
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct avifDiagnostics {
        pub error: [c_char; 256],
    }

    /// Mirrors `avifDecoder` from avif.h (libavif v1.2 layout).
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct avifDecoder {
        // Inputs
        pub codecChoice: avifCodecChoice,
        pub maxThreads: c_int,
        pub requestedSource: avifDecoderSource,
        pub allowProgressive: avifBool,
        pub allowIncremental: avifBool,
        pub ignoreExif: avifBool,
        pub ignoreXMP: avifBool,
        pub imageSizeLimit: u32,
        pub imageDimensionLimit: u32,
        pub imageCountLimit: u32,
        pub strictFlags: u32,
        // Outputs
        pub image: *mut avifImage,
        pub imageIndex: c_int,
        pub imageCount: c_int,
        pub progressiveState: avifProgressiveState,
        pub imageTiming: avifImageTiming,
        pub timescale: u64,
        pub duration: f64,
        pub durationInTimescales: u64,
        pub repetitionCount: c_int,
        pub alphaPresent: avifBool,
        pub ioStats: avifIOStats,
        pub diag: avifDiagnostics,
        // Internals
        pub io: *mut c_void,
        pub data: *mut c_void,
        // Version 1.0.0 ends here.
        pub imageSequenceTrackPresent: avifBool,
        // Version 1.1.0 ends here.
        pub imageContentToDecode: avifImageContentTypeFlags,
        // Version 1.2.0 ends here. Add any new members after this line.
    }

    #[link(name = "avif")]
    unsafe extern "C" {
        pub fn avifResultToString(result: avifResult) -> *const c_char;
        pub fn avifRWDataSet(raw: *mut avifRWData, data: *const u8, len: usize) -> avifResult;
        pub fn avifRWDataFree(raw: *mut avifRWData);
        pub fn avifGetExifTiffHeaderOffset(
            exif: *const u8,
            exifSize: usize,
            offset: *mut usize,
        ) -> avifResult;

        pub fn avifImageCreate(
            width: u32,
            height: u32,
            depth: u32,
            yuvFormat: avifPixelFormat,
        ) -> *mut avifImage;
        pub fn avifImageDestroy(image: *mut avifImage);
        pub fn avifImageSetMetadataExif(
            image: *mut avifImage,
            exif: *const u8,
            exifSize: usize,
        ) -> avifResult;
        pub fn avifImageAllocatePlanes(image: *mut avifImage, planes: c_uint) -> avifResult;
        pub fn avifImageRGBToYUV(image: *mut avifImage, rgb: *const avifRGBImage) -> avifResult;

        pub fn avifRGBImageSetDefaults(rgb: *mut avifRGBImage, image: *const avifImage);
        pub fn avifRGBImageAllocatePixels(rgb: *mut avifRGBImage) -> avifResult;
        pub fn avifRGBImageFreePixels(rgb: *mut avifRGBImage);
        pub fn avifImageYUVToRGB(image: *const avifImage, rgb: *mut avifRGBImage) -> avifResult;

        pub fn avifDecoderCreate() -> *mut avifDecoder;
        pub fn avifDecoderDestroy(decoder: *mut avifDecoder);
        pub fn avifDecoderSetIOMemory(
            decoder: *mut avifDecoder,
            data: *const u8,
            size: usize,
        ) -> avifResult;
        pub fn avifDecoderParse(decoder: *mut avifDecoder) -> avifResult;
        pub fn avifDecoderNextImage(decoder: *mut avifDecoder) -> avifResult;

        pub fn avifEncoderCreate() -> *mut c_void;
        pub fn avifEncoderWrite(
            encoder: *mut c_void,
            image: *const avifImage,
            output: *mut avifRWData,
        ) -> avifResult;
        pub fn avifEncoderAddImage(
            encoder: *mut c_void,
            image: *const avifImage,
            duration_in_timescales: u64,
            add_image_flags: c_uint,
        ) -> avifResult;
        pub fn avifEncoderFinish(encoder: *mut c_void, output: *mut avifRWData) -> avifResult;
        pub fn avifEncoderDestroy(encoder: *mut c_void);
    }
}

#[derive(Debug)]
pub struct AvifError(String);

impl fmt::Display for AvifError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for AvifError {}

fn err(msg: impl std::fmt::Display) -> AvifError {
    AvifError(msg.to_string())
}

/// Interleaved RGBA samples, 8 or 16 bits per component (straight alpha).
#[derive(Debug, Clone)]
pub enum Pixels {
    U8(Vec<u8>),
    U16(Vec<u16>),
}

/// Container transforms reported so the caller can apply them (mirrors the
/// MIAF `irot`/`imir` property semantics).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerTransform {
    /// `irot` only; angle is 1..3 anti-clockwise quarter-turns.
    RotateCcw { quarters: u32 },
    /// `imir` only; axis 1 mirrors left/right (EXIF "flip horizontal"),
    /// axis 0 mirrors top/bottom (EXIF "flip vertical").
    Mirror { left_right: bool },
    /// Both: rotation applied first, then the mirror (MIAF 7.3.6.7 order).
    RotateMirror { quarters: u32, left_right: bool },
}

/// A decoded still AVIF.
#[derive(Debug, Clone)]
pub struct AvifStill {
    /// Pixel dimensions from the container.
    pub width: u32,
    pub height: u32,
    /// RGBA/RBG samples: 4 samples/pixel when `has_alpha`, 3 otherwise.
    pub pixels: Pixels,
    pub has_alpha: bool,
    /// Storage depth of the returned samples (8 or 16).
    pub bits: u32,
    /// The file's encoded bit depth (8/10/12), for notes.
    pub source_depth: u32,
    /// True when the container asked for `irot`/`imir` (applied value in
    /// [`Self::transform`]; the pixels we return are NOT rotated).
    pub transform: Option<ContainerTransform>,
    /// Raw Exif payload from the container (TIFF, possibly with an
    /// "Exif\0\0" prefix).
    pub exif: Vec<u8>,
    /// Raw XMP packet from the container.
    pub xmp: Vec<u8>,
}

const TRANSFER_16_PQ: u16 = 16;
const TRANSFER_18_HLG: u16 = 18;

/// Decodes the primary still image of a single-image AVIF.
///
/// Animated image sequences and PQ/HLG (HDR) transfers refuse with friendly
/// errors, per the viewer's stated scope.
pub fn decode(bytes: &[u8]) -> Result<AvifStill, AvifError> {
    unsafe { decode_impl(bytes) }
}

unsafe fn decode_impl(bytes: &[u8]) -> Result<AvifStill, AvifError> {
    unsafe {
        use ffi::*;

        if bytes.len() < 8 {
            return Err(err("AVIF file too small"));
        }
        let decoder = avifDecoderCreate();
        if decoder.is_null() {
            return Err(err("avifDecoderCreate failed"));
        }
        let result = (|| -> Result<AvifStill, AvifError> {
            let threads = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
                .min(16) as i32;
            (*decoder).maxThreads = threads;

            // Data is not copied by SetIOMemory; `bytes` outlives the decoder
            // (decoder destroyed inside this scope).
            avif_status(
                avifDecoderSetIOMemory(decoder, bytes.as_ptr(), bytes.len()),
                decoder,
                "avifDecoderSetIOMemory",
            )?;
            avif_status(avifDecoderParse(decoder), decoder, "avifDecoderParse")?;
            avif_status(
                avifDecoderNextImage(decoder),
                decoder,
                "avifDecoderNextImage",
            )?;

            let image = (*decoder).image;
            if image.is_null() {
                return Err(err("avifDecoderParse returned no image"));
            }

            // Scope rules: still images only â€” sequences (tracks) and animated
            // item-only files are turned away before any pixel work.
            if (*decoder).imageSequenceTrackPresent != AVIF_FALSE || (*decoder).imageCount > 1 {
                return Err(err(
                    "AVIF: animated image sequences are not supported yet (stills only)",
                ));
            }
            match (*image).transferCharacteristics {
                TRANSFER_16_PQ => {
                    return Err(err("AVIF: HDR (PQ / BT.2100) files are not supported yet"));
                }
                TRANSFER_18_HLG => {
                    return Err(err("AVIF: HDR (HLG / BT.2100) files are not supported yet"));
                }
                _ => {}
            }

            let depth = (*image).depth;
            let storage_bits: u32 = if depth > 8 { 16 } else { 8 };
            let has_alpha = (*decoder).alphaPresent != AVIF_FALSE;

            let mut rgb: avifRGBImage = std::mem::zeroed();
            avifRGBImageSetDefaults(&mut rgb, image);
            rgb.format = if has_alpha {
                AVIF_RGB_FORMAT_RGBA
            } else {
                AVIF_RGB_FORMAT_RGB
            };
            rgb.depth = storage_bits;
            // Built-in conversion (no libyuv in this build): AUTOMATIC == bilinear.
            rgb.chromaUpsampling = AVIF_CHROMA_UPSAMPLING_BILINEAR;
            rgb.maxThreads = threads.max(1);
            avif_status(
                avifRGBImageAllocatePixels(&mut rgb),
                decoder,
                "avifRGBImageAllocatePixels",
            )?;

            // Read the decoded pixels behind a guard so a panic can't leak them.
            struct PixelsGuard(avifRGBImage);
            impl Drop for PixelsGuard {
                fn drop(&mut self) {
                    unsafe { avifRGBImageFreePixels(&mut self.0) };
                }
            }
            let mut guard = PixelsGuard(rgb);
            avif_status(
                avifImageYUVToRGB(image, &mut guard.0),
                decoder,
                "avifImageYUVToRGB",
            )?;

            // Copy rows out of libavif's buffer.
            let (w, h) = (rgb.width as usize, rgb.height as usize);
            let channels = if has_alpha { 4 } else { 3 };
            let pixels = match storage_bits {
                16 => {
                    let mut out = Vec::with_capacity(w * h * channels);
                    let row = rgb.rowBytes as usize;
                    for y in 0..h {
                        let src = std::slice::from_raw_parts(
                            guard.0.pixels.add(row * y) as *const u16,
                            w * channels,
                        );
                        out.extend_from_slice(src);
                    }
                    Pixels::U16(out)
                }
                _ => {
                    let mut out = Vec::with_capacity(w * h * channels);
                    let row = rgb.rowBytes as usize;
                    for y in 0..h {
                        let src =
                            std::slice::from_raw_parts(guard.0.pixels.add(row * y), w * channels);
                        out.extend_from_slice(src);
                    }
                    Pixels::U8(out)
                }
            };

            // Container transform (`irot`/`imir`) â€” reported, not applied: the
            // caller owns pixel rotation. MIAF applies irot before imir.
            let transform = probe_transform(image);

            let exif = copy_rw(&(*image).exif);
            let xmp = copy_rw(&(*image).xmp);

            Ok(AvifStill {
                width: (*image).width,
                height: (*image).height,
                pixels,
                has_alpha,
                bits: storage_bits,
                source_depth: depth,
                transform,
                exif,
                xmp,
            })
        })();

        avifDecoderDestroy(decoder);
        result
    }
}

unsafe fn avif_status(
    status: ffi::avifResult,
    decoder: *mut ffi::avifDecoder,
    what: &str,
) -> Result<(), AvifError> {
    unsafe {
        if status == ffi::AVIF_RESULT_OK {
            return Ok(());
        }
        let cstr = ffi::avifResultToString(status);
        let text = if cstr.is_null() {
            "unknown".to_string()
        } else {
            std::ffi::CStr::from_ptr(cstr)
                .to_string_lossy()
                .into_owned()
        };
        let mut msg = format!("{what}: {text} ({status})");
        if !decoder.is_null() {
            let diag = &(*decoder).diag;
            let etext = std::ffi::CStr::from_ptr(diag.error.as_ptr()).to_string_lossy();
            if !etext.is_empty() {
                msg.push_str(&format!(" â€” {etext}"));
            }
        }
        Err(err(msg))
    }
}

unsafe fn probe_transform(image: *mut ffi::avifImage) -> Option<ContainerTransform> {
    unsafe {
        use ffi::*;
        let flags = (*image).transformFlags;
        let irot = flags & AVIF_TRANSFORM_IROT != 0;
        let imir = flags & AVIF_TRANSFORM_IMIR != 0;
        let quarters = if irot {
            ((*image).irot.angle & 3).max(1) as u32
        } else {
            0
        };
        let left_right = if imir { (*image).imir.axis == 1 } else { false };
        match (irot, imir) {
            (true, false) if (1..=3).contains(&quarters) => {
                Some(ContainerTransform::RotateCcw { quarters })
            }
            (false, true) => Some(ContainerTransform::Mirror { left_right }),
            (true, true) if (1..=3).contains(&quarters) => Some(ContainerTransform::RotateMirror {
                quarters,
                left_right,
            }),
            _ => None,
        }
    }
}

unsafe fn copy_rw(rw: &ffi::avifRWData) -> Vec<u8> {
    unsafe {
        if rw.data.is_null() || rw.size == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(rw.data, rw.size).to_vec()
        }
    }
}

// ---------------------------------------------------------------------------
// Fixture encoder (feature "fixture-encoder") â€” for examples/gen_avif.rs.
// ---------------------------------------------------------------------------

/// Locates the TIFF header inside an AVIF Exif payload (handles both the
/// bare-TIFF and "Exif\0\0"-prefixed layouts the spec allows). `None` when
/// the payload doesn't parse or the header sits past the payload end.
pub fn exif_tiff_header_offset(payload: &[u8]) -> Option<usize> {
    let mut offset: usize = 0;
    let status =
        unsafe { ffi::avifGetExifTiffHeaderOffset(payload.as_ptr(), payload.len(), &mut offset) };
    if status == ffi::AVIF_RESULT_OK && offset < payload.len() {
        Some(offset)
    } else {
        None
    }
}

#[cfg(feature = "fixture-encoder")]
pub mod fixtures {
    //! Single-still AV1/AVIF encoder used only to produce test fixtures.
    use super::ffi::*;
    use super::{AvifError, err};

    /// One solid-color 8-bit AVIF with optional alpha, a container transform
    /// (`rotation quarters` = CCW quarter-turns), an EXIF payload, or an
    /// explicit PQ transfer (to exercise the viewer's HDR rejection).
    #[derive(Debug, Clone)]
    pub struct FixtureSpec {
        pub width: u32,
        pub height: u32,
        pub color: [u8; 3],
        /// When set: the RIGHT half of the image uses this color (left half
        /// keeps `color`), making rotation/mirror direction observable.
        pub color_right: Option<[u8; 3]>,
        /// None = no alpha; Some(v) = constant straight alpha v.
        pub alpha: Option<u8>,
        /// `irot` quarters (CCW); 0 = none.
        pub rotation_quarters: u32,
        /// `imir`: None, Some(true) = left/right, Some(false) = top/bottom.
        pub mirror: Option<bool>,
        /// Raw EXIF payload (handled via avifImageSetMetadataExif).
        pub exif: Option<Vec<u8>>,
        /// Force the PQ transfer characteristic in the container's nclx.
        pub pq_transfer: bool,
    }

    /// Encodes a single still (lossy, encoder defaults — plenty for tiny
    /// solid-color fixtures committed to the repo).
    pub fn encode_fixture(spec: &FixtureSpec) -> Result<Vec<u8>, AvifError> {
        unsafe { encode_fixture_impl(spec) }
    }

    /// Encodes 2..n frames as an animated AVIF sequence (used only to test
    /// the viewer's graceful refusal of animations).
    pub fn encode_sequence_fixture(specs: &[FixtureSpec]) -> Result<Vec<u8>, AvifError> {
        unsafe { encode_sequence_fixture_impl(specs) }
    }

    unsafe fn build_fixture_image(spec: &FixtureSpec) -> Result<*mut avifImage, AvifError> {
        unsafe {
            let image = avifImageCreate(spec.width, spec.height, 8, AVIF_PIXEL_FORMAT_YUV444);
            if image.is_null() {
                return Err(err("avifImageCreate failed"));
            }
            if let Some(exif) = &spec.exif {
                avif_status_enc(
                    avifImageSetMetadataExif(image, exif.as_ptr(), exif.len()),
                    "avifImageSetMetadataExif",
                )?;
            }
            if (1..=3).contains(&spec.rotation_quarters) {
                (*image).transformFlags |= AVIF_TRANSFORM_IROT;
                (*image).irot.angle = spec.rotation_quarters as u8;
            }
            if let Some(left_right) = spec.mirror {
                (*image).transformFlags |= AVIF_TRANSFORM_IMIR;
                (*image).imir.axis = if left_right { 1 } else { 0 };
            }
            if spec.pq_transfer {
                (*image).transferCharacteristics = 16;
            }
            Ok(image)
        }
    }

    unsafe fn fill_rgb(spec: &FixtureSpec, image: *mut avifImage) -> Result<Vec<u8>, AvifError> {
        unsafe {
            let channels = if spec.alpha.is_some() { 4 } else { 3 };
            let w = spec.width as usize;
            let h = spec.height as usize;
            let split_x = w / 2;
            let mut bytes = Vec::with_capacity(w * h * channels);
            for _y in 0..h {
                for x in 0..w {
                    let color = if spec.color_right.is_some() && x >= split_x {
                        spec.color_right.unwrap()
                    } else {
                        spec.color
                    };
                    bytes.push(color[0]);
                    bytes.push(color[1]);
                    bytes.push(color[2]);
                    if let Some(a) = spec.alpha {
                        bytes.push(a);
                    }
                }
            }
            let mut rgb: avifRGBImage = std::mem::zeroed();
            avifRGBImageSetDefaults(&mut rgb, image);
            rgb.format = if channels == 4 {
                AVIF_RGB_FORMAT_RGBA
            } else {
                AVIF_RGB_FORMAT_RGB
            };
            rgb.pixels = bytes.as_mut_ptr();
            rgb.rowBytes = (w * channels) as u32;
            avif_status_enc(avifImageRGBToYUV(image, &rgb), "avifImageRGBToYUV")?;
            Ok(bytes)
        }
    }

    unsafe fn encode_fixture_impl(spec: &FixtureSpec) -> Result<Vec<u8>, AvifError> {
        unsafe {
            let image = build_fixture_image(spec)?;
            let out: Result<Vec<u8>, AvifError> = (|| {
                let _keep = fill_rgb(spec, image)?;

                let encoder = avifEncoderCreate();
                if encoder.is_null() {
                    return Err(err("avifEncoderCreate failed"));
                }
                let mut output = avifRWData {
                    data: std::ptr::null_mut(),
                    size: 0,
                };
                let status = avifEncoderWrite(encoder, image, &mut output);
                avifEncoderDestroy(encoder);
                avif_status_enc(status, "avifEncoderWrite")?;
                let bytes = std::slice::from_raw_parts(output.data, output.size).to_vec();
                avifRWDataFree(&mut output);
                Ok(bytes)
            })();

            avifImageDestroy(image);
            out
        }
    }

    unsafe fn encode_sequence_fixture_impl(specs: &[FixtureSpec]) -> Result<Vec<u8>, AvifError> {
        unsafe {
            let encoder = avifEncoderCreate();
            if encoder.is_null() {
                return Err(err("avifEncoderCreate failed"));
            }
            let mut images: Vec<*mut avifImage> = Vec::new();
            let out: Result<Vec<u8>, AvifError> = (|| {
                for spec in specs {
                    let image = build_fixture_image(spec)?;
                    images.push(image);
                    let _keep = fill_rgb(spec, image)?;
                    avif_status_enc(
                        avifEncoderAddImage(encoder, image, 1, AVIF_ADD_IMAGE_FLAG_NONE),
                        "avifEncoderAddImage",
                    )?;
                }
                let mut output = avifRWData {
                    data: std::ptr::null_mut(),
                    size: 0,
                };
                avif_status_enc(avifEncoderFinish(encoder, &mut output), "avifEncoderFinish")?;
                let bytes = std::slice::from_raw_parts(output.data, output.size).to_vec();
                avifRWDataFree(&mut output);
                Ok(bytes)
            })();
            avifEncoderDestroy(encoder);
            for image in images {
                avifImageDestroy(image);
            }
            out
        }
    }

    unsafe fn avif_status_enc(status: avifResult, what: &str) -> Result<(), AvifError> {
        unsafe {
            if status == AVIF_RESULT_OK {
                return Ok(());
            }
            let cstr = avifResultToString(status);
            let text = if cstr.is_null() {
                "unknown".to_string()
            } else {
                std::ffi::CStr::from_ptr(cstr)
                    .to_string_lossy()
                    .into_owned()
            };
            Err(err(format!("{what}: {text} ({status})")))
        }
    }
}
