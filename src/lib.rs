//! jxlviewer library: viewer app, codecs, and CLI command implementations.

pub mod app;
pub mod cli;
pub mod codec;
pub mod dialogs;
pub mod hdr;
pub mod icons;
pub mod nav;
pub mod raster;
pub mod view;

use std::path::PathBuf;

/// Default GUI window geometry (logical points), before work-area clamping.
const DEFAULT_WINDOW_SIZE: [f32; 2] = [1280.0, 800.0];
const MIN_WINDOW_SIZE: [f32; 2] = [480.0, 300.0];

/// Requests the *primary monitor work area* (the screen minus the taskbar) and
/// derives an initial client size + position (logical points) so the first
/// window fits without sliding under the taskbar. Windows-only; other
/// platforms return `None` and keep the default size.
///
/// The process is explicitly marked per-monitor-DPI-aware first (winit would
/// do this later anyway) so the returned rectangle is in true physical pixels.
/// All the arithmetic happens in physical pixels — including window chrome
/// (title bar + borders ≈ 48 px tall / 18 px wide) — and only the final client
/// size and position are converted back to logical points for winit.
#[cfg(windows)]
fn windows_initial_window_logical() -> Option<([f32; 2], [f32; 2])> {
    use std::ffi::c_void;

    #[repr(C)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn SetProcessDpiAwarenessContext(ctx: *mut c_void) -> i32;
        fn SystemParametersInfoW(
            action: u32,
            uiparam: u32,
            pvparam: *mut c_void,
            fwinini: u32,
        ) -> i32;
        fn GetDpiForSystem() -> u32;
    }

    const SPI_GETWORKAREA: u32 = 0x0030;
    // DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 is the pointer value -4.
    const PER_MONITOR_AWARE_V2: *mut c_void = -4isize as *mut c_void;
    // Window chrome on top of the client area (title bar + borders + shadows),
    // measured ~18 px wide and ~47 px tall at 125% scaling.
    const CHROME_W: f32 = 20.0;
    const CHROME_H: f32 = 48.0;
    const MARGIN: f32 = 16.0;

    unsafe {
        // Harmless if it fails (already set); must happen before the work-area
        // query so coordinates are physical pixels, not DPI-virtualized.
        SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2);

        let mut rect = Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        if SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut rect as *mut Rect as *mut c_void, 0) == 0
        {
            return None;
        }
        let dpi = GetDpiForSystem().max(96) as f32;
        let scale = dpi / 96.0;

        let wa_w = (rect.right - rect.left) as f32;
        let wa_h = (rect.bottom - rect.top) as f32;

        // Client area in physical pixels, clamped to the work area minus
        // chrome and a small margin.
        let client_w = DEFAULT_WINDOW_SIZE[0]
            .mul_add(scale, 0.0)
            .min(wa_w - CHROME_W - MARGIN)
            .max(MIN_WINDOW_SIZE[0] * scale);
        let client_h = DEFAULT_WINDOW_SIZE[1]
            .mul_add(scale, 0.0)
            .min(wa_h - CHROME_H - MARGIN)
            .max(MIN_WINDOW_SIZE[1] * scale);

        // Center the *outer* window (client + chrome) in the work area.
        let outer_w = client_w + CHROME_W;
        let outer_h = client_h + CHROME_H;
        let x = rect.left as f32 + ((wa_w - outer_w) / 2.0).max(0.0);
        let y = rect.top as f32 + ((wa_h - outer_h) / 2.0).max(0.0);

        Some(([client_w / scale, client_h / scale], [x / scale, y / scale]))
    }
}

#[cfg(not(windows))]
fn windows_initial_window_logical() -> Option<([f32; 2], [f32; 2])> {
    None
}

/// Launches the GUI application.
pub fn run_gui(path: Option<PathBuf>) {
    let mut options = eframe::NativeOptions::default();

    options.renderer = selected_renderer();
    #[cfg(windows)]
    {
        options.wgpu_options = windows_wgpu_configuration();
    }

    let (init_size, position) =
        windows_initial_window_logical().unwrap_or((DEFAULT_WINDOW_SIZE, [0.0, 0.0]));

    let mut builder = eframe::egui::ViewportBuilder::default()
        .with_inner_size(init_size)
        .with_min_inner_size(MIN_WINDOW_SIZE)
        .with_icon(load_icon());
    if cfg!(windows) {
        builder = builder.with_position(eframe::egui::pos2(position[0], position[1]));
    }
    options.viewport = builder;

    if let Err(err) = eframe::run_native(
        "jxlviewer",
        options,
        Box::new(move |cc| {
            let mut a = app::App::new(cc);
            if let Some(p) = path {
                a.open_on_startup(p);
            }
            Ok(Box::new(a))
        }),
    ) {
        report_gui_failure(&err);
        std::process::exit(1);
    }
}

/// The renderer used on each platform:
/// * Windows — the `wgpu` renderer over Direct3D 12 (see
///   `windows_wgpu_configuration`).
/// * macOS / Linux — the `glow` (OpenGL) renderer, unchanged from before.
///
/// Both renderers are compiled in on every platform (see `Cargo.toml`), and
/// the choice is made here explicitly at runtime so that Cargo feature
/// unification across the dependency graph can never silently flip a build
/// to a different renderer.
fn selected_renderer() -> eframe::Renderer {
    if cfg!(windows) {
        eframe::Renderer::Wgpu
    } else {
        eframe::Renderer::Glow
    }
}

/// Prints a failure message instead of eframe's default panic, with
/// renderer-specific hints, and lets the caller exit.
fn report_gui_failure(err: &eframe::Error) {
    eprintln!("jxlviewer: the GUI failed to start: {err}");
    #[cfg(windows)]
    {
        if let eframe::Error::Wgpu(_) = err {
            eprintln!(
                "hint: Windows rendering needs a Direct3D 12 adapter that can present to the window."
            );
            eprintln!(
                "      Install or update your GPU driver (any modern GPU provides Direct3D 12)."
            );
            eprintln!(
                "      On machines without a GPU, a CPU adapter (Microsoft Basic Render Driver/WARP)"
            );
            eprintln!(
                "      is attempted as a best-effort fallback; if the driver does not expose one,"
            );
            eprintln!("      rendering is not possible.");
        }
    }
}

/// Builds the wgpu configuration for the Windows (wgpu) renderer: Direct3D 12
/// only, with hardware-first adapter selection that verifies the window
/// surface and falls back to CPU/software rendering only when necessary.
///
/// Notes:
/// * `Backends::DX12` pins the backend; the `wgpu` dependency in `Cargo.toml`
///   compiles exactly that backend (plus WGSL shader support).
/// * `eframe` injects the winit display handle into the instance
///   (`WgpuSetupCreateNew::display_handle`), and passes the real window
///   surface into `select_dx12_adapter`, so adapter selection is
///   surface-aware.
#[cfg(windows)]
fn windows_wgpu_configuration() -> eframe::egui_wgpu::WgpuConfiguration {
    use eframe::egui_wgpu::{WgpuSetup, WgpuSetupCreateNew};

    let mut create = WgpuSetupCreateNew::without_display_handle();
    create.instance_descriptor.backends = eframe::egui_wgpu::wgpu::Backends::DX12;
    create.native_adapter_selector = Some(std::sync::Arc::new(select_dx12_adapter));

    eframe::egui_wgpu::WgpuConfiguration {
        wgpu_setup: WgpuSetup::CreateNew(create),
        ..Default::default()
    }
}

/// Sort key for adapter preference: discrete GPUs first, then integrated,
/// then everything hardware-ish; CPU adapters (software rendering) last.
#[cfg(windows)]
fn adapter_rank(device_type: eframe::egui_wgpu::wgpu::DeviceType) -> u8 {
    use eframe::egui_wgpu::wgpu::DeviceType;
    match device_type {
        DeviceType::DiscreteGpu => 0,
        DeviceType::IntegratedGpu => 1,
        DeviceType::VirtualGpu => 2,
        DeviceType::Other => 3,
        DeviceType::Cpu => 4,
    }
}

/// Describes an adapter for diagnostics.
#[cfg(windows)]
fn describe_adapter(adapter: &eframe::egui_wgpu::wgpu::Adapter) -> String {
    let info = adapter.get_info();
    format!(
        "\"{}\" ({:?}, {:?}, driver {} {})",
        info.name, info.backend, info.device_type, info.driver, info.driver_info
    )
}

/// Adapter selection for the Windows (wgpu) renderer.
///
/// Prefers a hardware adapter that can present to the window; only when no
/// compatible hardware adapter exists does it fall back to a CPU adapter
/// (Direct3D 12 software rasterizer, e.g. Microsoft Basic Render Driver /
/// WARP). Every candidate is checked against the actual window surface via
/// `Adapter::is_surface_supported`, so an adapter that cannot present the
/// window is never chosen.
///
/// Software rendering is a best-effort path: the CPU/WARP fallback is *not*
/// yet verified on Windows and may still fail (or perform poorly); it is
/// attempted only because using no GPU is worse than a slow frame.
#[cfg(windows)]
fn select_dx12_adapter(
    adapters: &[eframe::egui_wgpu::wgpu::Adapter],
    surface: Option<&eframe::egui_wgpu::wgpu::Surface<'_>>,
) -> Result<eframe::egui_wgpu::wgpu::Adapter, String> {
    use eframe::egui_wgpu::wgpu::DeviceType;

    let compatible: Vec<eframe::egui_wgpu::wgpu::Adapter> = match surface {
        Some(surface) => adapters
            .iter()
            .cloned()
            .filter(|a| a.is_surface_supported(surface))
            .collect(),
        None => adapters.to_vec(),
    };

    if let Some(adapter) = compatible
        .iter()
        .min_by_key(|a| adapter_rank(a.get_info().device_type))
    {
        // Only a CPU adapter left: tell the user what is happening, without
        // promising it will work.
        if adapter.get_info().device_type == DeviceType::Cpu {
            eprintln!(
                "jxlviewer: no hardware Direct3D 12 adapter compatible with the window was found;"
            );
            eprintln!(
                "           falling back to software rendering ({}), best-effort and untested.",
                describe_adapter(adapter)
            );
        }
        return Ok(adapter.clone());
    }

    Err(format!(
        "no usable Direct3D 12 adapter compatible with the window; {} adapter(s) present: {}",
        adapters.len(),
        adapters
            .iter()
            .map(describe_adapter)
            .collect::<Vec<_>>()
            .join("; ")
    ))
}

/// Runtime window icon (Windows window/taskbar, X11 alt-tab). macOS instead
/// uses `assets/icon.icns` inside the app bundle. Embedded at compile time —
/// no filesystem access needed.
fn load_icon() -> eframe::egui::IconData {
    let png = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/icons/jxlviewer-64.png"
    ));
    let img = image::load_from_memory(png)
        .expect("bundled icon must decode")
        .to_rgba8();
    eframe::egui::IconData {
        width: img.width(),
        height: img.height(),
        rgba: img.into_raw(),
    }
}

/// CLI entry — parses `std::env::args()` and executes; returns a process exit code.
pub fn cli_main() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    cli::run(args)
}
