//! Minimal safe Rust API over the libjxl C API (pinned to v0.12.x):
//!
//! * [`encode_dynamic`] — single-frame RGBA8/RGBA16 → JPEG XL
//!   (lossless, distance, effort, modular mode, modular group size aka tiles)
//! * [`decode`] — JPEG XL → RGB(A)8/16 integer path.
//!
//! License: MIT OR Apache-2.0 (this binding); libjxl itself is Apache-2.0.

use std::error::Error;
use std::fmt;

#[allow(non_camel_case_types, non_snake_case, dead_code, unused)]
pub mod ffi {
    use std::os::raw::{c_int, c_void};

    pub type Jxl_BOOL = c_int;
    pub type JxlMemoryManager = *const c_void; // pass NULL
    pub type JxlParallelRetCode = c_int;
    pub type JxlParallelRunInit = Option<
        unsafe extern "C" fn(inline_opaque: *mut c_void, num_threads: usize) -> JxlParallelRetCode,
    >;
    pub type JxlParallelRunFunction = Option<
        unsafe extern "C" fn(
            inline_opaque: *mut c_void,
            value: u32,
            thread_id: usize,
            num_threads: usize,
        ),
    >;
    pub type JxlParallelRunner = Option<
        unsafe extern "C" fn(
            runner_opaque: *mut c_void,
            jpegxl_opaque: *mut c_void,
            init: JxlParallelRunInit,
            func: JxlParallelRunFunction,
            start_range: u32,
            end_range: u32,
        ) -> JxlParallelRetCode,
    >;

    pub type JxlDataType = c_int;
    pub const JXL_TYPE_FLOAT: c_int = 0;
    pub const JXL_TYPE_UINT8: c_int = 2;
    pub const JXL_TYPE_UINT16: c_int = 3;

    pub type JxlEndianness = c_int;
    pub const JXL_NATIVE_ENDIAN: c_int = 0;

    // decode.h statuses
    pub type JxlDecoderStatus = c_int;
    pub const JXL_DEC_SUCCESS: c_int = 0;
    pub const JXL_DEC_ERROR: c_int = 1;
    pub const JXL_DEC_NEED_MORE_INPUT: c_int = 2;
    pub const JXL_DEC_NEED_IMAGE_OUT_BUFFER: c_int = 5;
    pub const JXL_DEC_BASIC_INFO: c_int = 0x40;
    pub const JXL_DEC_COLOR_ENCODING: c_int = 0x100;
    pub const JXL_DEC_FULL_IMAGE: c_int = 0x1000;

    // color_encoding.h enums
    pub type JxlColorSpace = c_int;
    pub const JXL_COLOR_SPACE_RGB: c_int = 0;
    pub const JXL_COLOR_SPACE_GRAY: c_int = 1;

    pub type JxlWhitePoint = c_int;
    pub const JXL_WHITE_POINT_D65: c_int = 1;

    pub type JxlPrimaries = c_int;
    pub const JXL_PRIMARIES_SRGB: c_int = 1;

    pub type JxlTransferFunction = c_int;
    pub const JXL_TRANSFER_FUNCTION_SRGB: c_int = 13;

    pub type JxlRenderingIntent = c_int;
    pub const JXL_RENDERING_INTENT_RELATIVE: c_int = 1;

    pub type JxlOrientation = c_int;

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct JxlPixelFormat {
        pub num_channels: u32,
        pub data_type: JxlDataType,
        pub endianness: JxlEndianness,
        pub align: usize,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct JxlPreviewHeader {
        pub xsize: u32,
        pub ysize: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct JxlAnimationHeader {
        pub tps_numerator: u32,
        pub tps_denominator: u32,
        pub num_loops: u32,
        pub have_timecodes: Jxl_BOOL,
    }

    /// Mirrors `jxl/codestream_header.h` for libjxl 0.12 exactly.
    #[repr(C)]
    pub struct JxlBasicInfo {
        pub have_container: Jxl_BOOL,
        pub xsize: u32,
        pub ysize: u32,
        pub bits_per_sample: u32,
        pub exponent_bits_per_sample: u32,
        pub intensity_target: f32,
        pub min_nits: f32,
        pub relative_to_max_display: Jxl_BOOL,
        pub linear_below: f32,
        pub uses_original_profile: Jxl_BOOL,
        pub have_preview: Jxl_BOOL,
        pub have_animation: Jxl_BOOL,
        pub orientation: JxlOrientation,
        pub num_color_channels: u32,
        pub num_extra_channels: u32,
        pub alpha_bits: u32,
        pub alpha_exponent_bits: u32,
        pub alpha_premultiplied: Jxl_BOOL,
        pub preview: JxlPreviewHeader,
        pub animation: JxlAnimationHeader,
        pub intrinsic_xsize: u32,
        pub intrinsic_ysize: u32,
        pub padding: [u8; 100],
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct JxlColorEncoding {
        pub color_space: JxlColorSpace,
        pub white_point: JxlWhitePoint,
        pub white_point_xy: [f64; 2],
        pub primaries: JxlPrimaries,
        pub primaries_red_xy: [f64; 2],
        pub primaries_green_xy: [f64; 2],
        pub primaries_blue_xy: [f64; 2],
        pub transfer_function: JxlTransferFunction,
        pub gamma: f64,
        pub rendering_intent: JxlRenderingIntent,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct JxlBitDepth {
        pub bit_depth_type: c_int,
        pub bits_per_sample: u32,
        pub exponent_bits_per_sample: u32,
    }

    // encode.h statuses
    pub type JxlEncoderStatus = c_int;
    pub type JxlEncoderError = c_int;
    pub const JXL_ENC_SUCCESS: c_int = 0;
    pub const JXL_ENC_ERROR: c_int = 1;
    pub const JXL_ENC_NEED_MORE_OUTPUT: c_int = 2;
    pub const JXL_ENC_ERROR_NOT_SUPPORTED: c_int = 25;

    pub type JxlEncFrameSettingId = c_int;
    pub const JXL_ENC_FRAME_SETTING_EFFORT: c_int = 0;
    pub const JXL_ENC_FRAME_SETTING_MODULAR: c_int = 11;
    /// -1 = encoding default, 0..3 = 128/256/512/1024
    pub const JXL_ENC_FRAME_SETTING_MODULAR_GROUP_SIZE: c_int = 26;

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct JxlEncoderFrameSettings {
        _private: [u8; 0],
    }

    #[link(name = "jxl")]
    unsafe extern "C" {
        pub fn JxlDecoderCreate(mem: JxlMemoryManager) -> *mut c_void;
        pub fn JxlDecoderDestroy(dec: *mut c_void);
        pub fn JxlDecoderSubscribeEvents(dec: *mut c_void, events: c_int) -> JxlDecoderStatus;
        pub fn JxlDecoderSetParallelRunner(
            dec: *mut c_void,
            runner: JxlParallelRunner,
            runner_opaque: *mut c_void,
        ) -> JxlDecoderStatus;
        pub fn JxlDecoderSetInput(
            dec: *mut c_void,
            data: *const u8,
            size: usize,
        ) -> JxlDecoderStatus;
        pub fn JxlDecoderCloseInput(dec: *mut c_void) -> JxlDecoderStatus;
        pub fn JxlDecoderProcessInput(dec: *mut c_void) -> JxlDecoderStatus;
        pub fn JxlDecoderGetBasicInfo(
            dec: *const c_void,
            info: *mut JxlBasicInfo,
        ) -> JxlDecoderStatus;
        pub fn JxlDecoderSetPreferredColorProfile(
            dec: *mut c_void,
            color_encoding: *const JxlColorEncoding,
        ) -> JxlDecoderStatus;
        pub fn JxlDecoderImageOutBufferSize(
            dec: *const c_void,
            format: *const JxlPixelFormat,
            size: *mut usize,
        ) -> JxlDecoderStatus;
        pub fn JxlDecoderSetImageOutBuffer(
            dec: *mut c_void,
            format: *const JxlPixelFormat,
            buffer: *mut u8,
            size: usize,
        ) -> JxlDecoderStatus;
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct JxlBlendInfo {
        pub blendmode: c_int,
        pub source: u32,
        pub alpha: u32,
        pub clamp: Jxl_BOOL,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct JxlLayerInfo {
        pub have_crop: Jxl_BOOL,
        pub crop_x0: i32,
        pub crop_y0: i32,
        pub xsize: u32,
        pub ysize: u32,
        pub blend_info: JxlBlendInfo,
        pub save_as_reference: u32,
    }

    /// Mirrors `JxlFrameHeader` (codestream_header.h).
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct JxlFrameHeader {
        pub duration: u32,
        pub timecode: u32,
        pub name_length: u32,
        pub is_last: Jxl_BOOL,
        pub layer_info: JxlLayerInfo,
    }

    #[link(name = "jxl")]
    unsafe extern "C" {
        pub fn JxlEncoderCreate(mem: JxlMemoryManager) -> *mut c_void;
        pub fn JxlEncoderDestroy(enc: *mut c_void);
        pub fn JxlEncoderSetParallelRunner(
            enc: *mut c_void,
            parallel_runner: JxlParallelRunner,
            parallel_runner_opaque: *mut c_void,
        ) -> JxlEncoderStatus;
        pub fn JxlEncoderGetError(enc: *const c_void) -> JxlEncoderError;
        pub fn JxlEncoderInitBasicInfo(info: *mut JxlBasicInfo);
        pub fn JxlEncoderSetBasicInfo(
            enc: *mut c_void,
            info: *const JxlBasicInfo,
        ) -> JxlEncoderStatus;
        pub fn JxlEncoderSetColorEncoding(
            enc: *mut c_void,
            color: *const JxlColorEncoding,
        ) -> JxlEncoderStatus;
        pub fn JxlEncoderFrameSettingsCreate(
            enc: *mut c_void,
            source: *const c_void,
        ) -> *mut c_void;
        pub fn JxlEncoderFrameSettingsSetOption(
            settings: *mut c_void,
            option_id: JxlEncFrameSettingId,
            value: i64,
        ) -> JxlEncoderStatus;
        pub fn JxlEncoderSetFrameLossless(settings: *mut c_void) -> JxlEncoderStatus;
        pub fn JxlEncoderSetFrameDistance(settings: *mut c_void, distance: f32)
        -> JxlEncoderStatus;
        pub fn JxlEncoderAddImageFrame(
            settings: *const JxlEncoderFrameSettings,
            pixel_format: *const JxlPixelFormat,
            buffer: *const u8,
            size: usize,
        ) -> JxlEncoderStatus;
        pub fn JxlEncoderCloseInput(enc: *mut c_void);
        pub fn JxlEncoderInitFrameHeader(frame_header: *mut JxlFrameHeader);
        pub fn JxlEncoderSetFrameHeader(
            settings: *mut c_void,
            frame_header: *const JxlFrameHeader,
        ) -> JxlEncoderStatus;
        pub fn JxlEncoderProcessOutput(
            enc: *mut c_void,
            next_out: *mut *mut u8,
            avail_out: *mut usize,
        ) -> JxlEncoderStatus;
        pub fn JxlEncoderDistanceFromQuality(quality: f32) -> f32;
    }

    #[link(name = "jxl_threads")]
    unsafe extern "C" {
        pub fn JxlThreadParallelRunnerCreate(
            mem: JxlMemoryManager,
            num_worker_threads: usize,
        ) -> *mut c_void;
        pub fn JxlThreadParallelRunnerDestroy(runner_opaque: *mut c_void);
        pub fn JxlThreadParallelRunner(
            runner_opaque: *mut c_void,
            jpegxl_opaque: *mut c_void,
            init: JxlParallelRunInit,
            func: JxlParallelRunFunction,
            start_range: u32,
            end_range: u32,
        ) -> JxlParallelRetCode;
    }
}

#[derive(Debug, Clone)]
pub struct JxlError(String);

impl fmt::Display for JxlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for JxlError {}

fn err(msg: impl std::fmt::Display) -> JxlError {
    JxlError(msg.to_string())
}

fn enc_status(
    status: ffi::JxlEncoderStatus,
    enc: *mut std::ffi::c_void,
    what: &str,
) -> Result<(), JxlError> {
    if status == ffi::JXL_ENC_SUCCESS {
        return Ok(());
    }
    let code = if status == ffi::JXL_ENC_ERROR && !enc.is_null() {
        unsafe { ffi::JxlEncoderGetError(enc) }
    } else {
        -1
    };
    if code == 25 {
        return Err(err(format!("{what}: not supported")));
    }
    if code > 0 {
        return Err(err(format!("{what}: encoder error code {code}")));
    }
    Err(err(format!("{what}: failed ({status})")))
}

/// Tiling granularity for modular mode (`JXL_ENC_FRAME_SETTING_MODULAR_GROUP_SIZE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupSize {
    Default,
    G128,
    G256,
    G512,
    G1024,
}

impl GroupSize {
    fn setting_value(self) -> i64 {
        match self {
            GroupSize::Default => -1,
            GroupSize::G128 => 0,
            GroupSize::G256 => 1,
            GroupSize::G512 => 2,
            GroupSize::G1024 => 3,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EncodeOptions {
    /// Mathematically lossless.
    pub lossless: bool,
    /// VarDCT distance, 0..=25 (1.0 = "visually lossless"). Used when `!lossless`.
    pub distance: f32,
    /// Effort 0..=10; our default is 3, libjxl's own default is 7.
    pub effort: i64,
    /// Force modular mode.
    pub modular: Option<bool>,
    /// Modular tile/group size.
    pub group_size: Option<GroupSize>,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            lossless: false,
            distance: 1.0,
            effort: 3,
            modular: None,
            group_size: None,
        }
    }
}

/// Straight (non-premultiplied) interleaved RGBA or RGB samples, 8 or 16 bit.
/// The channel count must match `has_alpha` (4 or 3).
#[derive(Debug, Clone)]
pub enum Pixels {
    U8(Vec<u8>),
    U16(Vec<u16>),
}

pub struct ThreadRunner(*mut std::ffi::c_void);

// Only forwarded to libjxl calls that the caller synchronizes.
unsafe impl Send for ThreadRunner {}
unsafe impl Sync for ThreadRunner {}

impl ThreadRunner {
    pub fn with_default_threads() -> ThreadRunner {
        let n = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .min(16);
        Self::for_threads(n)
    }

    pub fn for_threads(num_worker_threads: usize) -> ThreadRunner {
        let ptr =
            unsafe { ffi::JxlThreadParallelRunnerCreate(std::ptr::null(), num_worker_threads) };
        if ptr.is_null() {
            panic!("JxlThreadParallelRunnerCreate failed");
        }
        ThreadRunner(ptr)
    }
}

impl Drop for ThreadRunner {
    fn drop(&mut self) {
        unsafe { ffi::JxlThreadParallelRunnerDestroy(self.0) };
        self.0 = std::ptr::null_mut();
    }
}

pub struct EncodeInput {
    pub width: u32,
    pub height: u32,
    pub has_alpha: bool,
    pub pixels: Pixels,
    pub options: EncodeOptions,
}

/// Encodes a single frame to a JPEG XL codestream.
pub fn encode_dynamic(input: EncodeInput) -> Result<Vec<u8>, JxlError> {
    let EncodeInput {
        width,
        height,
        has_alpha,
        pixels,
        options,
    } = input;
    let w = width as usize;
    let h = height as usize;

    let expected_len: usize = w * h * if has_alpha { 4 } else { 3 };
    let buffer_bytes: usize = match &pixels {
        Pixels::U8(d) => {
            if d.len() != expected_len {
                return Err(err("u8 buffer size mismatch"));
            }
            d.len()
        }
        Pixels::U16(d) => {
            if d.len() != expected_len {
                return Err(err("u16 buffer size mismatch"));
            }
            d.len() * 2
        }
    };

    let bits: u32 = match &pixels {
        Pixels::U16(_) => 16,
        Pixels::U8(_) => 8,
    };

    let enc = unsafe { ffi::JxlEncoderCreate(std::ptr::null()) };
    if enc.is_null() {
        return Err(err("JxlEncoderCreate failed"));
    }

    let result = {
        let runner = ThreadRunner::with_default_threads();
        enc_status(
            unsafe { ffi::JxlEncoderSetParallelRunner(enc, Some(run_callback), runner.0) },
            enc,
            "JxlEncoderSetParallelRunner",
        )?;

        let mut info: ffi::JxlBasicInfo = unsafe { std::mem::zeroed() };
        unsafe { ffi::JxlEncoderInitBasicInfo(&mut info) };
        info.xsize = width;
        info.ysize = height;
        info.orientation = 1;
        info.num_color_channels = 3;
        info.num_extra_channels = if has_alpha { 1 } else { 0 };
        info.bits_per_sample = bits;
        info.exponent_bits_per_sample = 0;
        info.alpha_bits = if has_alpha { bits } else { 0 };
        info.alpha_exponent_bits = 0;
        info.uses_original_profile = if options.lossless { 1 } else { 0 };
        enc_status(
            unsafe { ffi::JxlEncoderSetBasicInfo(enc, &info) },
            enc,
            "JxlEncoderSetBasicInfo",
        )?;

        let mut color: ffi::JxlColorEncoding = unsafe { std::mem::zeroed() };
        color.color_space = ffi::JXL_COLOR_SPACE_RGB;
        color.white_point = ffi::JXL_WHITE_POINT_D65;
        color.primaries = ffi::JXL_PRIMARIES_SRGB;
        color.transfer_function = ffi::JXL_TRANSFER_FUNCTION_SRGB;
        color.rendering_intent = ffi::JXL_RENDERING_INTENT_RELATIVE;
        enc_status(
            unsafe { ffi::JxlEncoderSetColorEncoding(enc, &color) },
            enc,
            "JxlEncoderSetColorEncoding",
        )?;

        let settings = unsafe { ffi::JxlEncoderFrameSettingsCreate(enc, std::ptr::null()) };
        if settings.is_null() {
            return Err(err("JxlEncoderFrameSettingsCreate failed"));
        }

        enc_status(
            unsafe {
                ffi::JxlEncoderFrameSettingsSetOption(
                    settings,
                    ffi::JXL_ENC_FRAME_SETTING_EFFORT,
                    options.effort,
                )
            },
            enc,
            "JxlEncoderFrameSettingsSetOption(EFFORT)",
        )?;

        if options.lossless {
            enc_status(
                unsafe { ffi::JxlEncoderSetFrameLossless(settings) },
                enc,
                "JxlEncoderSetFrameLossless",
            )?;
        } else {
            enc_status(
                unsafe {
                    ffi::JxlEncoderSetFrameDistance(settings, options.distance.clamp(0.0, 25.0))
                },
                enc,
                "JxlEncoderSetFrameDistance",
            )?;
        }

        if let Some(m) = options.modular {
            enc_status(
                unsafe {
                    ffi::JxlEncoderFrameSettingsSetOption(
                        settings,
                        ffi::JXL_ENC_FRAME_SETTING_MODULAR,
                        if m { 1 } else { 0 },
                    )
                },
                enc,
                "JxlEncoderFrameSettingsSetOption(MODULAR)",
            )?;
        }
        if let Some(gs) = options.group_size {
            enc_status(
                unsafe {
                    ffi::JxlEncoderFrameSettingsSetOption(
                        settings,
                        ffi::JXL_ENC_FRAME_SETTING_MODULAR_GROUP_SIZE,
                        gs.setting_value(),
                    )
                },
                enc,
                "JxlEncoderFrameSettingsSetOption(MODULAR_GROUP_SIZE)",
            )?;
        }

        let is_u16 = matches!(pixels, Pixels::U16(_));
        let pixel_format = ffi::JxlPixelFormat {
            num_channels: if has_alpha { 4 } else { 3 },
            data_type: if is_u16 {
                ffi::JXL_TYPE_UINT16
            } else {
                ffi::JXL_TYPE_UINT8
            },
            endianness: ffi::JXL_NATIVE_ENDIAN,
            align: 0,
        };

        let buf_ptr: *const u8 = match &pixels {
            Pixels::U8(d) => d.as_ptr(),
            Pixels::U16(d) => d.as_ptr() as *const u8,
        };
        enc_status(
            unsafe {
                ffi::JxlEncoderAddImageFrame(
                    settings as *const _,
                    &pixel_format,
                    buf_ptr,
                    buffer_bytes,
                )
            },
            enc,
            "JxlEncoderAddImageFrame",
        )?;
        unsafe { ffi::JxlEncoderCloseInput(enc) };

        unsafe { drain_output(enc) }
    };

    unsafe { ffi::JxlEncoderDestroy(enc) };
    result
}

/// Drives `JxlEncoderProcessOutput` until the codestream is complete.
unsafe fn drain_output(enc: *mut std::ffi::c_void) -> Result<Vec<u8>, JxlError> {
    // Output buffer dance: keep `out.len()` as the used prefix and the tail
    // as available space for ProcessOutput to fill.
    let mut out: Vec<u8> = vec![0u8; (1 << 20).max(64)];
    let mut next_out: *mut u8 = out.as_mut_ptr();
    let mut avail: usize = out.len();
    let final_result: Result<Vec<u8>, JxlError> = loop {
        let status = unsafe { ffi::JxlEncoderProcessOutput(enc, &mut next_out, &mut avail) };
        match status {
            ffi::JXL_ENC_SUCCESS => {
                let used = out.len() - avail;
                out.truncate(used);
                break Ok(out);
            }
            ffi::JXL_ENC_NEED_MORE_OUTPUT => {
                let used = out.len() - avail;
                out.resize(out.len() * 2, 0);
                next_out = unsafe { out.as_mut_ptr().add(used) };
                avail = out.len() - used;
            }
            _ => {
                let code = unsafe { ffi::JxlEncoderGetError(enc) };
                if code == 25 {
                    break Err(err(format!("process output not supported ({code})")));
                }
                break Err(err(format!("JxlEncoderProcessOutput failed (code {code})")));
            }
        }
    };
    final_result
}

/// Animated encode: one image per frame with per-frame durations.
///
/// Ticks-per-second is fixed at 1000/1 so `durations_ms` maps 1:1 to ticks.
pub struct AnimEncodeInput {
    pub width: u32,
    pub height: u32,
    pub has_alpha: bool,
    /// One buffer per frame; every buffer uses the same [`Pixels`] variant and
    /// matches `width × height × (4 or 3)` samples.
    pub frames: Vec<Pixels>,
    /// Per-frame display duration in milliseconds.
    pub durations_ms: Vec<u32>,
    /// Animation loop count, 0 = infinite.
    pub loops: u32,
    pub options: EncodeOptions,
}

pub fn encode_anim_dynamic(input: AnimEncodeInput) -> Result<Vec<u8>, JxlError> {
    let AnimEncodeInput {
        width,
        height,
        has_alpha,
        frames,
        durations_ms,
        loops,
        options,
    } = input;
    if frames.is_empty() || frames.len() != durations_ms.len() {
        return Err(err("frame/duration count mismatch"));
    }
    let expected_len: usize = width as usize * height as usize * if has_alpha { 4 } else { 3 };
    let is_u16 = matches!(frames[0], Pixels::U16(_));
    for (i, f) in frames.iter().enumerate() {
        let ok = match f {
            Pixels::U8(d) => !is_u16 && d.len() == expected_len,
            Pixels::U16(d) => is_u16 && d.len() == expected_len,
        };
        if !ok {
            return Err(err(format!("frame {i} buffer size mismatch")));
        }
    }

    let enc = unsafe { ffi::JxlEncoderCreate(std::ptr::null()) };
    if enc.is_null() {
        return Err(err("JxlEncoderCreate failed"));
    }

    let result = {
        let runner = ThreadRunner::with_default_threads();
        enc_status(
            unsafe { ffi::JxlEncoderSetParallelRunner(enc, Some(run_callback), runner.0) },
            enc,
            "JxlEncoderSetParallelRunner",
        )?;

        let mut info: ffi::JxlBasicInfo = unsafe { std::mem::zeroed() };
        unsafe { ffi::JxlEncoderInitBasicInfo(&mut info) };
        info.xsize = width;
        info.ysize = height;
        info.orientation = 1;
        info.num_color_channels = 3;
        info.num_extra_channels = if has_alpha { 1 } else { 0 };
        info.bits_per_sample = if is_u16 { 16 } else { 8 };
        info.alpha_bits = if has_alpha {
            if is_u16 { 16 } else { 8 }
        } else {
            0
        };
        info.alpha_exponent_bits = 0;
        info.uses_original_profile = if options.lossless { 1 } else { 0 };
        info.have_animation = 1;
        // 1000 ticks per second → 1 tick = 1 ms, so durations_ms maps 1:1.
        info.animation = ffi::JxlAnimationHeader {
            tps_numerator: 1000,
            tps_denominator: 1,
            num_loops: loops,
            have_timecodes: 0,
        };
        enc_status(
            unsafe { ffi::JxlEncoderSetBasicInfo(enc, &info) },
            enc,
            "JxlEncoderSetBasicInfo",
        )?;

        let mut color: ffi::JxlColorEncoding = unsafe { std::mem::zeroed() };
        color.color_space = ffi::JXL_COLOR_SPACE_RGB;
        color.white_point = ffi::JXL_WHITE_POINT_D65;
        color.primaries = ffi::JXL_PRIMARIES_SRGB;
        color.transfer_function = ffi::JXL_TRANSFER_FUNCTION_SRGB;
        color.rendering_intent = ffi::JXL_RENDERING_INTENT_RELATIVE;
        enc_status(
            unsafe { ffi::JxlEncoderSetColorEncoding(enc, &color) },
            enc,
            "JxlEncoderSetColorEncoding",
        )?;

        let settings = unsafe { ffi::JxlEncoderFrameSettingsCreate(enc, std::ptr::null()) };
        if settings.is_null() {
            return Err(err("JxlEncoderFrameSettingsCreate failed"));
        }
        enc_status(
            unsafe {
                ffi::JxlEncoderFrameSettingsSetOption(
                    settings,
                    ffi::JXL_ENC_FRAME_SETTING_EFFORT,
                    options.effort,
                )
            },
            enc,
            "JxlEncoderFrameSettingsSetOption(EFFORT)",
        )?;
        if options.lossless {
            enc_status(
                unsafe { ffi::JxlEncoderSetFrameLossless(settings) },
                enc,
                "JxlEncoderSetFrameLossless",
            )?;
        } else {
            enc_status(
                unsafe {
                    ffi::JxlEncoderSetFrameDistance(settings, options.distance.clamp(0.0, 25.0))
                },
                enc,
                "JxlEncoderSetFrameDistance",
            )?;
        }
        if let Some(m) = options.modular {
            enc_status(
                unsafe {
                    ffi::JxlEncoderFrameSettingsSetOption(
                        settings,
                        ffi::JXL_ENC_FRAME_SETTING_MODULAR,
                        if m { 1 } else { 0 },
                    )
                },
                enc,
                "JxlEncoderFrameSettingsSetOption(MODULAR)",
            )?;
        }
        if let Some(gs) = options.group_size {
            enc_status(
                unsafe {
                    ffi::JxlEncoderFrameSettingsSetOption(
                        settings,
                        ffi::JXL_ENC_FRAME_SETTING_MODULAR_GROUP_SIZE,
                        gs.setting_value(),
                    )
                },
                enc,
                "JxlEncoderFrameSettingsSetOption(MODULAR_GROUP_SIZE)",
            )?;
        }

        let pixel_format = ffi::JxlPixelFormat {
            num_channels: if has_alpha { 4 } else { 3 },
            data_type: if is_u16 {
                ffi::JXL_TYPE_UINT16
            } else {
                ffi::JXL_TYPE_UINT8
            },
            endianness: ffi::JXL_NATIVE_ENDIAN,
            align: 0,
        };

        for (i, frame) in frames.iter().enumerate() {
            let mut fh: ffi::JxlFrameHeader = unsafe { std::mem::zeroed() };
            unsafe { ffi::JxlEncoderInitFrameHeader(&mut fh) };
            fh.duration = durations_ms[i].clamp(1, u32::MAX);
            enc_status(
                unsafe { ffi::JxlEncoderSetFrameHeader(settings, &fh) },
                enc,
                "JxlEncoderSetFrameHeader",
            )?;
            let buf_ptr: *const u8 = match frame {
                Pixels::U8(d) => d.as_ptr(),
                Pixels::U16(d) => d.as_ptr() as *const u8,
            };
            enc_status(
                unsafe {
                    ffi::JxlEncoderAddImageFrame(
                        settings as *const _,
                        &pixel_format,
                        buf_ptr,
                        expected_len * if is_u16 { 2 } else { 1 },
                    )
                },
                enc,
                "JxlEncoderAddImageFrame",
            )?;
        }
        unsafe { ffi::JxlEncoderCloseInput(enc) };

        unsafe { drain_output(enc) }
    };

    unsafe { ffi::JxlEncoderDestroy(enc) };
    result
}

unsafe extern "C" fn run_callback(
    runner_opaque: *mut std::ffi::c_void,
    jpegxl_opaque: *mut std::ffi::c_void,
    init: ffi::JxlParallelRunInit,
    func: ffi::JxlParallelRunFunction,
    start_range: u32,
    end_range: u32,
) -> std::os::raw::c_int {
    unsafe {
        ffi::JxlThreadParallelRunner(
            runner_opaque,
            jpegxl_opaque,
            init,
            func,
            start_range,
            end_range,
        )
    }
}

#[derive(Debug, Clone)]
pub struct DecodeInfo {
    pub width: u32,
    pub height: u32,
    pub bits_per_sample: u32,
    pub has_alpha: bool,
    pub num_color_channels: u32,
    pub orientation: u32,
}

#[derive(Debug)]
pub struct Decoded {
    pub info: DecodeInfo,
    pub pixels: Pixels,
}

/// Decodes the first frame of a JPEG XL file to straight RGB(A) samples.
/// Pixel values are full-range for the requested target depth.
pub fn decode(input_bytes: &[u8], target_bits: u32) -> Result<Decoded, JxlError> {
    let dec = unsafe { ffi::JxlDecoderCreate(std::ptr::null()) };
    if dec.is_null() {
        return Err(err("JxlDecoderCreate failed"));
    }

    let result = {
        let runner = ThreadRunner::with_default_threads();
        let events =
            ffi::JXL_DEC_BASIC_INFO | ffi::JXL_DEC_COLOR_ENCODING | ffi::JXL_DEC_FULL_IMAGE;
        if unsafe { ffi::JxlDecoderSubscribeEvents(dec, events) } != ffi::JXL_DEC_SUCCESS {
            return Err(err("JxlDecoderSubscribeEvents failed"));
        }
        if unsafe { ffi::JxlDecoderSetParallelRunner(dec, Some(run_callback), runner.0) }
            != ffi::JXL_DEC_SUCCESS
        {
            return Err(err("JxlDecoderSetParallelRunner failed"));
        }
        if unsafe { ffi::JxlDecoderSetInput(dec, input_bytes.as_ptr(), input_bytes.len()) }
            != ffi::JXL_DEC_SUCCESS
        {
            return Err(err("JxlDecoderSetInput failed"));
        }
        unsafe { ffi::JxlDecoderCloseInput(dec) };

        let mut info: Option<DecodeInfo> = None;
        let mut out_pixels: Option<Pixels> = None;

        loop {
            let status = unsafe { ffi::JxlDecoderProcessInput(dec) };
            match status {
                ffi::JXL_DEC_BASIC_INFO => {
                    let mut bi: ffi::JxlBasicInfo = unsafe { std::mem::zeroed() };
                    if unsafe { ffi::JxlDecoderGetBasicInfo(dec, &mut bi) } != ffi::JXL_DEC_SUCCESS
                    {
                        return Err(err("JxlDecoderGetBasicInfo failed"));
                    }
                    let has_alpha = bi.alpha_bits != 0;
                    info = Some(DecodeInfo {
                        width: bi.xsize,
                        height: bi.ysize,
                        bits_per_sample: bi.bits_per_sample,
                        has_alpha,
                        num_color_channels: bi.num_color_channels,
                        orientation: bi.orientation as u32,
                    });
                }
                ffi::JXL_DEC_COLOR_ENCODING => {
                    // The output color preference can be set only when this
                    // event fires (color info exists now).
                    let i = info
                        .as_ref()
                        .ok_or_else(|| err("color encoding before basic info"))?;
                    let mut color: ffi::JxlColorEncoding = unsafe { std::mem::zeroed() };
                    color.color_space = if i.num_color_channels == 1 {
                        ffi::JXL_COLOR_SPACE_GRAY
                    } else {
                        ffi::JXL_COLOR_SPACE_RGB
                    };
                    color.white_point = ffi::JXL_WHITE_POINT_D65;
                    color.primaries = ffi::JXL_PRIMARIES_SRGB;
                    color.transfer_function = ffi::JXL_TRANSFER_FUNCTION_SRGB;
                    color.rendering_intent = ffi::JXL_RENDERING_INTENT_RELATIVE;
                    unsafe { ffi::JxlDecoderSetPreferredColorProfile(dec, &color) };
                }
                ffi::JXL_DEC_NEED_IMAGE_OUT_BUFFER => {
                    let i = info
                        .as_ref()
                        .ok_or_else(|| err("out buffer before basic info"))?;
                    // Orientation is applied by the decoder by default; swap dims for orientations 5..8.
                    let (rows, cols) = if (5..=8).contains(&i.orientation) {
                        (i.width, i.height)
                    } else {
                        (i.height, i.width)
                    };
                    let channels = if i.has_alpha { 4u32 } else { 3u32 };
                    let format = ffi::JxlPixelFormat {
                        num_channels: channels,
                        data_type: if target_bits <= 8 {
                            ffi::JXL_TYPE_UINT8
                        } else {
                            ffi::JXL_TYPE_UINT16
                        },
                        endianness: ffi::JXL_NATIVE_ENDIAN,
                        align: 0,
                    };
                    let mut size: usize = 0;
                    if unsafe { ffi::JxlDecoderImageOutBufferSize(dec, &format, &mut size) }
                        != ffi::JXL_DEC_SUCCESS
                    {
                        return Err(err("JxlDecoderImageOutBufferSize failed"));
                    }
                    if target_bits <= 8 {
                        let mut buf = vec![0u8; size];
                        if unsafe {
                            ffi::JxlDecoderSetImageOutBuffer(
                                dec,
                                &format,
                                buf.as_mut_ptr(),
                                buf.len(),
                            )
                        } != ffi::JXL_DEC_SUCCESS
                        {
                            return Err(err("JxlDecoderSetImageOutBuffer failed"));
                        }
                        out_pixels = Some(Pixels::U8(buf));
                    } else {
                        // u16 samples, but the FFI takes a u8 pointer.
                        let units = size / 2;
                        let mut buf16 = vec![0u16; units];
                        if unsafe {
                            ffi::JxlDecoderSetImageOutBuffer(
                                dec,
                                &format,
                                buf16.as_mut_ptr() as *mut u8,
                                units * 2,
                            )
                        } != ffi::JXL_DEC_SUCCESS
                        {
                            return Err(err("JxlDecoderSetImageOutBuffer failed"));
                        }
                        out_pixels = Some(Pixels::U16(buf16));
                    }
                    let _ = (rows, cols);
                }
                ffi::JXL_DEC_FULL_IMAGE => break,
                ffi::JXL_DEC_SUCCESS => break,
                ffi::JXL_DEC_ERROR => return Err(err("JxlDecoderProcessInput: error")),
                ffi::JXL_DEC_NEED_MORE_INPUT => return Err(err("unexpected end of codestream")),
                _ => continue, // other statuses are not subscribed
            }
        }

        let info = info.ok_or_else(|| err("no basic info decoded"))?;
        let pixels = out_pixels.ok_or_else(|| err("no pixel data decoded"))?;
        Decoded { info, pixels }
    };

    unsafe { ffi::JxlDecoderDestroy(dec) };
    Ok(result)
}
