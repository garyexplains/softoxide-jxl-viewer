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

/// Launches the GUI application.
pub fn run_gui(path: Option<PathBuf>) {
    let mut options = eframe::NativeOptions::default();
    options.viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([480.0, 300.0]);

    eframe::run_native(
        "jxlviewer",
        options,
        Box::new(move |cc| {
            let mut a = app::App::new(cc);
            if let Some(p) = path {
                a.open_on_startup(p);
            }
            Ok(Box::new(a))
        }),
    )
    .expect("gui should not fail to start");
}

/// CLI entry — parses `std::env::args()` and executes; returns a process exit code.
pub fn cli_main() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    cli::run(args)
}
