//! The eframe/egui application: toolbar, canvas with zoom/pan/crop, dialogs,
//! and background save/convert jobs.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use eframe::egui;
use egui::{
    Color32, ColorImage, Rect, Sense, TextureHandle, TextureOptions, TextureWrapMode, Vec2,
};

use crate::codec::{self, JpegSaveOptions, JxlSaveOptions, SaveOptions};
use crate::icons;
use crate::raster::{Raster, RasterRect, ResizeFilter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Pan,
    Crop,
}

#[derive(Clone)]
pub(crate) struct Document {
    pub(crate) path: Option<PathBuf>,
    pub(crate) format: codec::Format,
    pub(crate) raster: Raster,
    pub(crate) dirty: bool,
}

#[derive(Default, Clone, Copy)]
pub(crate) struct ViewerState {
    /// `None` = fit to window.
    pub(crate) zoom: Option<f32>,
    pub(crate) offset: Vec2,
}

#[derive(Clone, Copy)]
pub(crate) struct CropDrag {
    pub(crate) start_px: Vec2,
    pub(crate) current_px: Vec2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveFormat {
    Jxl,
    Png,
    Jpeg,
    Bmp,
    Tiff,
}

pub(crate) struct SaveDialogState {
    pub(crate) format: SaveFormat,
    pub(crate) jxl: JxlSaveOptions,
    pub(crate) jpeg: JpegSaveOptions,
}

pub(crate) struct ResizeDialogState {
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) aspect: f32,
    pub(crate) filter: ResizeFilter,
}

pub enum UiMsg {
    Done(Result<String, String>),
}

pub struct App {
    pub(crate) doc: Option<Document>,
    pub(crate) viewer: ViewerState,
    pub(crate) tool: Tool,
    pub(crate) undo: Vec<Raster>,
    pub(crate) redo: Vec<Raster>,

    pub(crate) crop_drag: Option<CropDrag>,
    pub(crate) crop_rect: Option<RasterRect>,

    pub(crate) save_dialog: Option<SaveDialogState>,
    pub(crate) resize_dialog: Option<ResizeDialogState>,

    pub(crate) busy: Option<String>,
    pub(crate) task_rx: Option<Receiver<UiMsg>>,
    pub(crate) last_dir: Option<PathBuf>,
    pub(crate) status: String,
    pub(crate) error: Option<String>,

    pub(crate) texture: Option<TextureHandle>,
    pub(crate) texture_nearest: bool,
    pub(crate) needs_rebuild: bool,
    pub(crate) checker: TextureHandle,
    pub(crate) pending_open: Option<PathBuf>,
}

const UNDO_LIMIT: usize = 5;

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> App {
        // Icon font (vendored Phosphor) as a fallback for UI glyphs.
        let mut fonts = egui::FontDefinitions::default();
        crate::icons::add_to_fonts(&mut fonts);
        cc.egui_ctx.set_fonts(fonts);

        let checker = build_checker_texture(&cc.egui_ctx);
        App {
            doc: None,
            viewer: ViewerState::default(),
            tool: Tool::Pan,
            undo: Vec::new(),
            redo: Vec::new(),
            crop_drag: None,
            crop_rect: None,
            save_dialog: None,
            resize_dialog: None,
            busy: None,
            task_rx: None,
            last_dir: None,
            status: String::new(),
            error: None,
            texture: None,
            texture_nearest: false,
            needs_rebuild: true,
            checker,
            pending_open: None,
        }
    }

    pub fn open_on_startup(&mut self, path: PathBuf) {
        self.pending_open = Some(path);
    }

    // ---- editing -----------------------------------------------------------

    pub(crate) fn apply_edit(
        &mut self,
        ctx: &egui::Context,
        op: impl FnOnce(&Raster) -> Result<Raster, String>,
    ) {
        let Some(old) = self.doc.as_ref().map(|d| d.raster.clone()) else {
            self.error = Some("no image open".into());
            return;
        };
        self.push_undo();
        match op(&old) {
            Ok(new) => {
                if let Some(doc) = &mut self.doc {
                    doc.raster = new;
                    doc.dirty = true;
                }
                self.needs_rebuild = true;
                self.crop_rect = None;
                self.crop_drag = None;
                ctx.request_repaint();
            }
            Err(e) => {
                self.undo.pop();
                self.error = Some(e);
            }
        }
    }

    fn push_undo(&mut self) {
        if let Some(doc) = &self.doc {
            self.undo.push(doc.raster.clone());
            if self.undo.len() > UNDO_LIMIT {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }

    pub(crate) fn apply_crop(&mut self, ctx: &egui::Context) {
        if let Some(r) = self.crop_rect.take() {
            self.apply_edit(ctx, move |ra| Ok(ra.crop(r)));
        }
    }

    pub(crate) fn undo_last(&mut self, ctx: &egui::Context) {
        let Some(prev) = self.undo.pop() else { return };
        if let Some(doc) = &self.doc {
            self.redo.push(doc.raster.clone());
        }
        if let Some(doc) = &mut self.doc {
            doc.raster = prev;
            doc.dirty = true;
        }
        self.needs_rebuild = true;
        self.crop_rect = None;
        self.crop_drag = None;
        ctx.request_repaint();
    }

    pub(crate) fn redo_last(&mut self, ctx: &egui::Context) {
        let Some(next) = self.redo.pop() else { return };
        if let Some(doc) = &self.doc {
            self.undo.push(doc.raster.clone());
        }
        if let Some(doc) = &mut self.doc {
            doc.raster = next;
            doc.dirty = true;
        }
        self.needs_rebuild = true;
        self.crop_rect = None;
        self.crop_drag = None;
        ctx.request_repaint();
    }

    // ---- io ----------------------------------------------------------------

    pub(crate) fn open_path(&mut self, path: PathBuf) {
        match codec::decode_file(&path) {
            Ok(img) => {
                let w = img.raster.width;
                let h = img.raster.height;
                self.doc = Some(Document {
                    path: Some(path.clone()),
                    format: img.format,
                    raster: img.raster,
                    dirty: false,
                });
                if let Some(note) = &img.note {
                    self.error = Some(note.clone());
                }
                self.undo.clear();
                self.redo.clear();
                self.crop_rect = None;
                self.crop_drag = None;
                self.viewer = ViewerState::default(); // fit zoom
                self.needs_rebuild = true;
                self.last_dir = path.parent().map(|p| p.to_path_buf());
                self.status = format!(
                    "Opened {} — {} × {}, {}-bit{}",
                    file_name(&path),
                    w,
                    h,
                    self.doc.as_ref().map(|d| d.raster.bits).unwrap_or(8),
                    if self
                        .doc
                        .as_ref()
                        .map(|d| d.raster.has_alpha)
                        .unwrap_or(false)
                    {
                        ", alpha"
                    } else {
                        ""
                    }
                );
            }
            Err(e) => self.error = Some(e),
        }
    }

    pub(crate) fn start_save_job(&mut self, path: PathBuf, opts: SaveOptions) {
        let Some(doc) = self.doc.clone() else { return };
        let (tx, rx) = mpsc::channel::<UiMsg>();
        self.task_rx = Some(rx);
        self.busy = Some(format!("Encoding {}…", file_name(&path)));
        self.status.clear();
        self.save_dialog = None;
        let raster = doc.raster;
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                codec::save_to_path(&path, &raster, &opts)
            }));
            let msg = match result {
                Ok(Ok(())) => UiMsg::Done(Ok(format!(
                    "Saved {} ({})",
                    file_name(&path),
                    describe_opts(&opts)
                ))),
                Ok(Err(e)) => UiMsg::Done(Err(e)),
                Err(_) => UiMsg::Done(Err("encoder panicked (native code)".into())),
            };
            let _ = tx.send(msg);
        });
    }

    fn poll_messages(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.task_rx {
            if let Ok(msg) = rx.try_recv() {
                match msg {
                    UiMsg::Done(Ok(what)) => {
                        self.busy = None;
                        self.status = what;
                    }
                    UiMsg::Done(Err(e)) => {
                        self.busy = None;
                        self.error = Some(e);
                    }
                }
                self.task_rx = None;
                ctx.request_repaint();
            }
        }
    }
}

fn describe_opts(opts: &SaveOptions) -> String {
    match opts {
        SaveOptions::Jxl(j) => {
            if j.lossless {
                "JPEG XL lossless".into()
            } else {
                format!(
                    "JPEG XL distance {:.1}, effort {}, tiles {}",
                    j.distance,
                    j.effort,
                    match j.group_size {
                        Some(jxl_native::GroupSize::Default) => "128",
                        Some(jxl_native::GroupSize::G128) => "128",
                        Some(jxl_native::GroupSize::G256) => "256",
                        Some(jxl_native::GroupSize::G512) => "512",
                        Some(jxl_native::GroupSize::G1024) => "1024",
                        None => "default",
                    }
                )
            }
        }
        SaveOptions::Jpeg(j) => format!("JPEG q{}", j.quality),
        SaveOptions::Png => "PNG".into(),
        SaveOptions::Bmp => "BMP".into(),
        SaveOptions::Tiff => "TIFF".into(),
    }
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

fn build_checker_texture(ctx: &egui::Context) -> TextureHandle {
    let mut px = Vec::with_capacity(16 * 16 * 4);
    let (c0, c1) = ([46u8, 46, 50, 255], [58u8, 58, 64, 255]);
    for y in 0..16 {
        for x in 0..16 {
            let even = ((x / 8) + (y / 8)) % 2 == 0;
            let c = if even { c0 } else { c1 };
            px.extend_from_slice(&c);
        }
    }
    ctx.load_texture(
        "checker",
        ColorImage::from_rgba_unmultiplied([16, 16], &px),
        TextureOptions {
            wrap_mode: TextureWrapMode::Repeat,
            ..TextureOptions::default()
        },
    )
}

impl eframe::App for App {
    /// Non-UI work: message pump, queued/dropped opens.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_messages(ctx);

        if let Some(path) = self.pending_open.take() {
            self.open_path(path);
            ctx.request_repaint();
        }

        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if let Some(first) = dropped.first() {
            self.open_path(first.clone());
            ctx.request_repaint();
        }
    }

    /// Draw the whole app.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.refresh_texture(&ctx);

        egui::Panel::top("toolbar")
            .resizable(false)
            .show(ui, |ui| self.show_toolbar(ui, &ctx));

        egui::Panel::bottom("status")
            .resizable(false)
            .show(ui, |ui| self.show_statusbar(ui));

        egui::CentralPanel::default().show(ui, |ui| self.show_canvas(ui, &ctx));

        self.show_dialogs(&ctx);
        self.update_window_title(&ctx);
    }
}

impl App {
    fn effective_zoom(&self, avail: Vec2, img: Vec2) -> f32 {
        self.viewer.zoom.unwrap_or_else(|| fit_zoom(avail, img))
    }

    fn refresh_texture(&mut self, ctx: &egui::Context) {
        let Some(doc) = &self.doc else { return };
        let img = Vec2::new(doc.raster.width as f32, doc.raster.height as f32);
        let avail = ctx.input(|i| i.content_rect().size());
        let effective = self.effective_zoom(avail, img);
        if self.texture.is_none()
            || self.needs_rebuild
            || self.texture_nearest != (effective >= 3.0)
        {
            self.texture = Some(ctx.load_texture(
                "image",
                doc.raster.to_color_image(),
                if effective >= 3.0 {
                    TextureOptions::NEAREST
                } else {
                    TextureOptions::LINEAR
                },
            ));
            self.texture_nearest = effective >= 3.0;
            self.needs_rebuild = false;
        }
    }

    fn update_window_title(&mut self, ctx: &egui::Context) {
        let title = match &self.doc {
            Some(doc) => {
                let name = doc
                    .path
                    .as_ref()
                    .map(|p| file_name(p))
                    .unwrap_or_else(|| "untitled".into());
                format!(
                    "JXL Viewer — {name}{} ({}{})",
                    if doc.dirty { "*" } else { "" },
                    doc.format.as_str(),
                    if doc.raster.has_alpha { ", alpha" } else { "" }
                )
            }
            None => "JXL Viewer — open an image".to_string(),
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
    }

    // ---- toolbar -----------------------------------------------------------

    fn show_toolbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal_wrapped(|ui| {
            if ui
                .button(format!("{}  Open…", icons::FOLDER_OPEN))
                .clicked()
            {
                self.pick_open_file();
            }
            ui.separator();

            let has = self.doc.is_some();
            if ui
                .add_enabled(
                    has,
                    egui::Button::new(format!("{}  Undo", icons::ARROW_COUNTER_CLOCKWISE)),
                )
                .clicked()
            {
                self.undo_last(ctx);
            }
            if ui
                .add_enabled(
                    has && !self.redo.is_empty(),
                    egui::Button::new(format!("{}  Redo", icons::ARROW_CLOCKWISE)),
                )
                .clicked()
            {
                self.redo_last(ctx);
            }
            ui.separator();

            if ui
                .selectable_label(self.tool == Tool::Pan, format!("{}  Pan", icons::HAND_PALM))
                .clicked()
            {
                self.tool = Tool::Pan;
                self.crop_rect = None;
                self.crop_drag = None;
            }
            if ui
                .selectable_label(self.tool == Tool::Crop, format!("{}  Crop", icons::CROP))
                .clicked()
            {
                self.tool = Tool::Crop;
                self.crop_rect = None;
                self.crop_drag = None;
            }

            if ui
                .add_enabled(
                    has,
                    egui::Button::new(format!("{}  Rotate L", icons::ARROW_COUNTER_CLOCKWISE)),
                )
                .clicked()
            {
                self.apply_edit(ctx, |r| Ok(r.rotate_ccw()));
            }
            if ui
                .add_enabled(
                    has,
                    egui::Button::new(format!("{}  Rotate R", icons::ARROW_CLOCKWISE)),
                )
                .clicked()
            {
                self.apply_edit(ctx, |r| Ok(r.rotate_cw()));
            }
            if ui
                .add_enabled(
                    has,
                    egui::Button::new(format!("{}  Flip H", icons::FLIP_HORIZONTAL)),
                )
                .clicked()
            {
                self.apply_edit(ctx, |r| Ok(r.flip_h()));
            }
            if ui
                .add_enabled(
                    has,
                    egui::Button::new(format!("{}  Flip V", icons::FLIP_VERTICAL)),
                )
                .clicked()
            {
                self.apply_edit(ctx, |r| Ok(r.flip_v()));
            }
            ui.separator();

            if ui
                .add_enabled(
                    has,
                    egui::Button::new(format!("{}  Resize…", icons::ARROWS_OUT_CARDINAL)),
                )
                .clicked()
            {
                let (w, h) = {
                    let d = self.doc.as_ref().unwrap();
                    (d.raster.width, d.raster.height)
                };
                self.resize_dialog = Some(ResizeDialogState {
                    width: w as i32,
                    height: h as i32,
                    aspect: w as f32 / h as f32,
                    filter: ResizeFilter::default(),
                });
            }
            ui.separator();
            if ui
                .add_enabled(
                    has,
                    egui::Button::new(format!("{}  Save as…", icons::FLOPPY_DISK)),
                )
                .clicked()
            {
                self.save_dialog = Some(SaveDialogState {
                    format: SaveFormat::Jxl,
                    jxl: JxlSaveOptions::default(),
                    jpeg: JpegSaveOptions::default(),
                });
            }

            ui.separator();
            let zoom_label = match self.viewer.zoom {
                Some(z) => format!("{} {:.0}%", icons::MAGNIFYING_GLASS_PLUS, z * 100.0),
                None => format!("{}  fit", icons::FRAME_CORNERS),
            };
            if ui.button(zoom_label).clicked() {
                self.viewer.zoom = None;
                self.viewer.offset = Vec2::ZERO;
            }
            if ui.add_enabled(has, egui::Button::new("1:1")).clicked() {
                self.viewer.zoom = Some(1.0);
                self.viewer.offset = Vec2::ZERO;
            }
        });
    }

    fn pick_open_file(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("Open image");
        if let Some(dir) = &self.last_dir {
            dialog = dialog.set_directory(dir);
        }
        dialog = dialog.add_filter(
            "Images",
            &[
                "jxl", "png", "jpg", "jpeg", "gif", "bmp", "tif", "tiff", "webp",
            ],
        );
        if let Some(path) = dialog.pick_file() {
            self.open_path(path);
        }
    }

    // ---- canvas ------------------------------------------------------------

    fn show_canvas(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let avail_rect = ui.available_rect_before_wrap();
        let (_, response) = ui.allocate_exact_size(avail_rect.size(), Sense::click_and_drag());
        let painter = ui.painter_at(avail_rect);
        painter.rect_filled(avail_rect, 0.0, Color32::from_rgb(26, 26, 30));

        let Some(doc) = &self.doc else {
            painter.text(
                avail_rect.center(),
                egui::Align2::CENTER_CENTER,
                "Drag an image here, or press Open…",
                egui::FontId::proportional(16.0),
                Color32::GRAY,
            );
            return;
        };

        let img_size = Vec2::new(doc.raster.width as f32, doc.raster.height as f32);
        let zoom = self.effective_zoom(avail_rect.size(), img_size);
        let shown = img_size * zoom;
        let img_rect = Rect::from_min_size(avail_rect.min + self.viewer.offset, shown);

        let pointer_hover = ctx.input(|i| i.pointer.hover_pos());
        let pointer_in_image = pointer_hover.map(|p| img_rect.contains(p)).unwrap_or(false);

        if doc.raster.has_alpha {
            let uv = Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                Vec2::new(shown.x / 16.0, shown.y / 16.0),
            );
            painter.image(self.checker.id(), img_rect, uv, Color32::LIGHT_GRAY);
        }
        if let Some(texture) = self.texture.as_ref() {
            painter.image(
                texture.id(),
                img_rect,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }

        // zoom around the pointer on wheel scroll / pinch
        let scroll = ctx.input(|i| i.smooth_scroll_delta);
        let pinch = ctx.input(|i| i.zoom_delta());
        let zfactor = pinch * (1.0 + scroll.y * 0.0025);
        if pointer_in_image && zfactor != 0.0 && (zoom_pinch_differs(pinch) || scroll.y != 0.0) {
            let pointer = pointer_hover.unwrap_or(img_rect.center());
            let new_zoom = (zoom * zfactor).clamp(0.02, 60.0);
            let anchor = crate::view::screen_to_px(pointer, img_rect, img_size);
            self.viewer.zoom = Some(new_zoom);
            self.viewer.offset = (pointer - avail_rect.min) - anchor * new_zoom;
            self.needs_rebuild = true;
            ctx.request_repaint();
        }

        // pan with drag (pan tool) or middle button
        if response.dragged_by(egui::PointerButton::Middle)
            || (response.dragged_by(egui::PointerButton::Primary) && self.tool == Tool::Pan)
        {
            self.viewer.offset += response.drag_delta();
            ctx.request_repaint();
        }

        if response.clicked() && self.tool == Tool::Crop {
            self.crop_rect = None;
            self.crop_drag = None;
        }

        if self.tool == Tool::Crop {
            crate::view::draw_crop(self, &painter, img_rect, img_size);
            crate::view::crop_input(self, &response, img_rect, img_size, ctx);
        }

        if let Some(pos) = pointer_hover {
            if img_rect.contains(pos) {
                let px = crate::view::screen_to_px(pos, img_rect, img_size);
                painter.text(
                    avail_rect.right_top() + egui::vec2(-8.0, 8.0),
                    egui::Align2::RIGHT_TOP,
                    format!("({}, {})", px.x as i32, px.y as i32),
                    egui::FontId::proportional(11.0),
                    Color32::LIGHT_GRAY,
                );
            }
        }
    }

    // ---- status ------------------------------------------------------------

    fn show_statusbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(doc) = &self.doc {
                ui.label(format!(
                    "{} × {} px, {}-bit, alpha {}",
                    doc.raster.width,
                    doc.raster.height,
                    doc.raster.bits,
                    if doc.raster.has_alpha { "yes" } else { "no" }
                ));
                if doc.dirty {
                    ui.strong("  (modified)");
                }
                ui.separator();
            }
            if let Some(busy) = &self.busy {
                ui.spinner();
                ui.label(busy);
            }
            if let Some(err) = &self.error {
                ui.colored_label(
                    Color32::from_rgb(255, 130, 130),
                    format!("{}  {err}", icons::WARNING_CIRCLE),
                );
            }
            ui.label(&self.status);
        });
    }

    fn show_dialogs(&mut self, ctx: &egui::Context) {
        crate::dialogs::show_save_dialog(ctx, self);
        crate::dialogs::show_resize_dialog(ctx, self);
        self.show_crop_bar(ctx);
    }

    fn show_crop_bar(&mut self, ctx: &egui::Context) {
        if self.tool != Tool::Crop || self.crop_rect.is_none() {
            return;
        }
        let rect_to_apply = self.crop_rect;
        let mut open = true;
        egui::Window::new(format!("{}  Crop selection", icons::CROP))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::RIGHT_BOTTOM, [-12.0, -12.0])
            .show(ctx, |ui| {
                if let Some(r) = rect_to_apply {
                    ui.label(format!(
                        "{} × {} px at ({}, {})",
                        r.width, r.height, r.x, r.y
                    ));
                }
                ui.horizontal(|ui| {
                    if ui
                        .button(format!("{}  Apply", icons::CHECK_CIRCLE))
                        .clicked()
                    {
                        self.apply_crop(ctx);
                    }
                    if ui.button(format!("{}  Cancel", icons::X_CIRCLE)).clicked() {
                        self.crop_rect = None;
                        self.crop_drag = None;
                    }
                });
            });
        if !open {
            self.crop_rect = None;
            self.crop_drag = None;
        }
    }
}

pub fn fit_zoom(avail: Vec2, img: Vec2) -> f32 {
    if img.x <= 0.0 || img.y <= 0.0 {
        return 1.0;
    }
    (avail.x / img.x).min(avail.y / img.y).min(1.0).max(0.02)
}

#[inline]
fn zoom_pinch_differs(pinch: f32) -> bool {
    pinch != 1.0 && pinch != 0.0
}
