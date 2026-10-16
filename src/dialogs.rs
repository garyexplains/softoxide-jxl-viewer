//! Dialog windows: Save-as (per-format options incl. JXL lossless/tiles) and
//! Resize. Both operate directly on `&mut App` and dispatch worker-thread jobs.

use eframe::egui;
use egui::Align2;

use crate::app::{App, SaveFormat};
use crate::codec::{JpegSaveOptions, JxlSaveOptions, SaveOptions};
use crate::icons;
use crate::raster::ResizeFilter;

#[cfg(test)]
mod tests {
    use super::RESIZE_PRESETS;

    #[test]
    fn presets_are_landscape_and_sane() {
        for (name, w, h) in RESIZE_PRESETS {
            assert!(w > h, "preset {name} must be landscape (w > h): {w}×{h}");
            assert!(
                w % 2 == 0 && h % 2 == 0,
                "preset {name} should be even: {w}×{h}"
            );
            assert!(*w >= 640, "preset {name} unexpectedly small");
        }
    }
}

pub fn show_save_dialog(ctx: &egui::Context, app: &mut App) {
    let Some(mut dlg) = app.save_dialog.take() else {
        return;
    };
    let mut open = true;
    let mut consumed = false;

    egui::Window::new("Save image as…")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Format:");
                for (f, label) in [
                    (SaveFormat::Jxl, "JPEG XL"),
                    (SaveFormat::Png, "PNG"),
                    (SaveFormat::Jpeg, "JPEG"),
                    (SaveFormat::Bmp, "BMP"),
                    (SaveFormat::Tiff, "TIFF"),
                    (SaveFormat::Gif, "GIF"),
                ] {
                    if ui.selectable_label(dlg.format == f, label).clicked() {
                        dlg.format = f;
                    }
                }
            });
            if dlg.format == SaveFormat::Gif || dlg.format == SaveFormat::Jxl {
                let animated = app.doc.as_ref().is_some_and(|d| d.anim.is_some());
                let frames = app
                    .doc
                    .as_ref()
                    .and_then(|d| d.anim.as_ref())
                    .map(|a| a.frame_count())
                    .unwrap_or(0);
                if animated {
                    let fmt = if dlg.format == SaveFormat::Gif {
                        "GIF"
                    } else {
                        "JPEG XL"
                    };
                    ui.label(format!(
                        "The open animation ({frames} frames) will be saved as an animated {fmt}."
                    ));
                } else if dlg.format == SaveFormat::Gif {
                    ui.label("Saves the current frame as a still GIF.");
                }
            }
            ui.add_space(8.0);

            match dlg.format {
                SaveFormat::Jxl => jxl_options(ui, &mut dlg.jxl),
                SaveFormat::Jpeg => jpeg_options(ui, &mut dlg.jpeg),
                _ => {
                    ui.label("No format-specific options.");
                }
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Save…").clicked() {
                    if app.doc.is_some() {
                        let opts = match dlg.format {
                            SaveFormat::Jxl => SaveOptions::Jxl(dlg.jxl.clone()),
                            SaveFormat::Jpeg => SaveOptions::Jpeg(dlg.jpeg.clone()),
                            SaveFormat::Png => SaveOptions::Png,
                            SaveFormat::Bmp => SaveOptions::Bmp,
                            SaveFormat::Tiff => SaveOptions::Tiff,
                            SaveFormat::Gif => SaveOptions::Gif,
                        };
                        consumed = true;
                        save_with_picker(app, dlg.format, opts);
                    } else {
                        app.error = Some("no image open".into());
                    }
                }
                if ui.button("Cancel").clicked() {
                    consumed = true;
                }
            });
        });

    // `take()` removed the state for this frame; write it back so the window
    // survives to the next frame unless the user dismissed it (X), clicked
    // Cancel, or clicked Save (job started; a cancelled native picker also
    // counts as finished).
    if !open || consumed {
        app.save_dialog = None;
    } else {
        app.save_dialog = Some(dlg);
    }
}

fn save_with_picker(app: &mut App, format: SaveFormat, opts: SaveOptions) {
    let mut dialog = rfd::FileDialog::new().set_title("Save as…");
    if let Some(dir) = &app.last_dir {
        dialog = dialog.set_directory(dir);
    }
    let (name, exts): (&str, &[&str]) = match format {
        SaveFormat::Jxl => ("JPEG XL", &["jxl"]),
        SaveFormat::Png => ("PNG", &["png"]),
        SaveFormat::Jpeg => ("JPEG", &["jpg", "jpeg"]),
        SaveFormat::Bmp => ("BMP", &["bmp"]),
        SaveFormat::Tiff => ("TIFF", &["tif", "tiff"]),
        SaveFormat::Gif => ("GIF", &["gif"]),
    };
    dialog = dialog.add_filter(name, exts);
    if let Some(src) = app.doc.as_ref().and_then(|d| d.path.as_ref()) {
        let stem = src
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "image".into());
        dialog = dialog.set_file_name(format!("{stem}.{}", format_ext(format)));
    }
    if let Some(path) = dialog.save_file() {
        app.start_save_job(path, opts);
    } else {
        // user cancelled the picker — dialog stays closed (Save as… reopens it)
    }
}

fn format_ext(f: SaveFormat) -> &'static str {
    match f {
        SaveFormat::Jxl => "jxl",
        SaveFormat::Png => "png",
        SaveFormat::Jpeg => "jpg",
        SaveFormat::Bmp => "bmp",
        SaveFormat::Tiff => "tiff",
        SaveFormat::Gif => "gif",
    }
}

fn jxl_options(ui: &mut egui::Ui, j: &mut JxlSaveOptions) {
    ui.checkbox(&mut j.lossless, "Lossless (pixel-perfect, larger file)")
        .on_hover_text("Decoded pixels are encoded with zero loss.");
    ui.add_enabled_ui(!j.lossless, |ui| {
        ui.horizontal(|ui| {
            ui.label("Distance:");
            if ui
                .add(egui::Slider::new(&mut j.distance, 0.0..=10.0).step_by(0.05))
                .changed()
            {
                // keep in editor-plausible range
                j.distance = j.distance.clamp(0.05, 10.0);
            }
            if ui.button("1.0").clicked() {
                j.distance = 1.0;
            }
        });
        ui.label("(1.0 = visually lossless; 0.0 only for lossless mode)");
    });
    ui.horizontal(|ui| {
        ui.label("Effort:");
        ui.add(egui::Slider::new(&mut j.effort, 1..=10));
    });
    ui.add_space(4.0);
    ui.checkbox(&mut j.keep_alpha, "Keep alpha channel")
        .on_hover_text("JPEG XL supports RGBA; embedding keeps transparency.");
    ui.checkbox(&mut j.modular, "Modular mode")
        .on_hover_text("Predictive mode; great for graphics/text. Lossless uses it by default.");
    ui.add_enabled_ui(j.modular || j.lossless, |ui| {
        ui.horizontal(|ui| {
            ui.label("Tile size (modular group):");
            let options: &[(&str, Option<jxl_native::GroupSize>)] = &[
                ("default", Some(jxl_native::GroupSize::Default)),
                ("128", Some(jxl_native::GroupSize::G128)),
                ("256", Some(jxl_native::GroupSize::G256)),
                ("512", Some(jxl_native::GroupSize::G512)),
                ("1024", Some(jxl_native::GroupSize::G1024)),
                ("encoder choice", None),
            ];
            for (label, val) in options {
                if ui.selectable_label(j.group_size == *val, *label).clicked() {
                    j.group_size = *val;
                }
            }
        });
    });
}
fn jpeg_options(ui: &mut egui::Ui, j: &mut JpegSaveOptions) {
    ui.horizontal(|ui| {
        ui.label("Quality:");
        ui.add(egui::Slider::new(&mut j.quality, 1..=100));
    });
    ui.label("Alpha is composited over white (JPEG has no alpha).");
}

/// Standard resolutions, always defined landscape (width > height). They are
/// displayed swapped when the source image is portrait.
const RESIZE_PRESETS: &[(&str, u32, u32)] = &[
    ("VGA", 640, 480),
    ("SVGA", 800, 600),
    ("XGA", 1024, 768),
    ("HD 720p", 1280, 720),
    ("UXGA", 1600, 1200),
    ("Full HD 1080p", 1920, 1080),
    ("QHD 1440p", 2560, 1440),
    ("4K UHD", 3840, 2160),
    ("5K", 5120, 2880),
    ("8K UHD", 7680, 4320),
];

pub fn show_resize_dialog(ctx: &egui::Context, app: &mut App) {
    let Some(mut d) = app.resize_dialog.take() else {
        return;
    };
    let mut open = true;
    let mut consumed = false;
    let portrait = d.aspect < 1.0;

    egui::Window::new(format!("{}  Resize image", icons::ARROWS_OUT_CARDINAL))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, -40.0])
        .show(ctx, |ui| {
            // ---- standard-resolution presets --------------------------------
            // Shown transposed when the source is portrait, so the preset
            // orientation follows the image.
            ui.label("Standard resolutions:");
            ui.horizontal_wrapped(|ui| {
                for (name, pw, ph) in RESIZE_PRESETS {
                    let (tw, th) = if portrait { (*ph, *pw) } else { (*pw, *ph) };
                    let selected = d.width == tw as i32 && d.height == th as i32;
                    if ui
                        .selectable_label(selected, format!("{tw}×{th}"))
                        .on_hover_text(format!("{name} ({}×{})", tw, th))
                        .clicked()
                    {
                        d.width = tw as i32;
                        d.height = th as i32;
                    }
                }
            });
            ui.add_space(4.0);

            // ---- manual size -------------------------------------------------
            ui.label("New size (px):");
            ui.horizontal(|ui| {
                let w0 = d.width;
                let h0 = d.height;
                ui.add(
                    egui::DragValue::new(&mut d.width)
                        .range(1..=20000)
                        .suffix("px"),
                );
                if ui
                    .button(icons::SWAP)
                    .on_hover_text("Swap width and height (and the kept aspect ratio)")
                    .clicked()
                {
                    std::mem::swap(&mut d.width, &mut d.height);
                    d.aspect = 1.0 / d.aspect;
                }
                ui.add(
                    egui::DragValue::new(&mut d.height)
                        .range(1..=20000)
                        .suffix("px"),
                );
                if d.keep_aspect {
                    if d.width != w0 {
                        d.height = ((d.width as f32 / d.aspect).round() as i32).max(1);
                    } else if d.height != h0 {
                        d.width = ((d.height as f32) * d.aspect).round() as i32;
                    }
                }
            });
            ui.checkbox(&mut d.keep_aspect, "Keep aspect ratio").on_hover_text(
                "When enabled, editing one side recalculates the other from the original aspect ratio.",
            );

            // ---- filter ------------------------------------------------------
            ui.horizontal(|ui| {
                ui.label("Filter:");
                for (label, f) in [
                    ("Lanczos3", ResizeFilter::Lanczos3),
                    ("CatmullRom", ResizeFilter::CatmullRom),
                    ("Box", ResizeFilter::Box),
                    ("Bilinear", ResizeFilter::Bilinear),
                    ("Nearest", ResizeFilter::Nearest),
                ] {
                    if ui.selectable_label(d.filter == f, label).clicked() {
                        d.filter = f;
                    }
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui
                    .button(format!("{}  Resize", icons::CHECK_CIRCLE))
                    .clicked()
                {
                    let (w, h) = (d.width as u32, d.height as u32);
                    let filt = d.filter;
                    app.apply_edit(ctx, move |r| r.resized(w, h, filt));
                    consumed = true;
                }
                if ui.button(format!("{}  Cancel", icons::X_CIRCLE)).clicked() {
                    consumed = true;
                }
            });
        });

    // `take()` removed the state for this frame; write it back so the window
    // survives to the next frame unless the user dismissed it (X), clicked
    // Cancel, or clicked Resize (edit applied).
    if !open || consumed {
        app.resize_dialog = None;
    } else {
        app.resize_dialog = Some(d);
    }
}
