//! The eframe/egui application: toolbar, canvas with zoom/pan/crop, dialogs,
//! and background save/convert jobs.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver};

use eframe::egui;
use egui::{
    Color32, ColorImage, Rect, Sense, TextureHandle, TextureOptions, TextureWrapMode, Vec2,
};

use crate::codec::{self, JpegSaveOptions, JxlSaveOptions, SaveOptions};
use crate::folder::{FolderState, SortKey};
use crate::icons;
use crate::raster::{Raster, RasterRect, ResizeFilter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Pan,
    Crop,
}

/// What the central panel shows: the image canvas or the folder grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Single,
    Folder,
}

#[derive(Clone)]
pub(crate) struct Document {
    pub(crate) path: Option<PathBuf>,
    pub(crate) format: codec::Format,
    pub(crate) raster: Raster,
    pub(crate) dirty: bool,
    /// "PQ"/"HLG" when the source was HDR (display is tone-mapped to SDR).
    pub(crate) hdr: Option<&'static str>,
    /// Animation playback state (rasters live in `frames`).
    pub(crate) anim: Option<AnimPlayer>,
}

/// Animation data + playback position.
#[derive(Clone)]
pub(crate) struct AnimPlayer {
    pub frames: Vec<Raster>,
    pub durations_ms: Vec<u32>,
    pub loops: u32,
    pub frame_idx: usize,
    pub playing: bool,
    pub frame_started: std::time::Instant,
}

impl AnimPlayer {
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }
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
    Gif,
}

pub(crate) struct SaveDialogState {
    pub(crate) format: SaveFormat,
    pub(crate) jxl: JxlSaveOptions,
    pub(crate) jpeg: JpegSaveOptions,
}

pub(crate) struct ResizeDialogState {
    pub(crate) width: i32,
    pub(crate) height: i32,
    /// Aspect ratio (width/height) that "Keep aspect ratio" maintains.
    pub(crate) aspect: f32,
    pub(crate) keep_aspect: bool,
    pub(crate) filter: ResizeFilter,
}

pub(crate) struct BatchDialogState {
    pub(crate) format: SaveFormat,
    pub(crate) jxl: JxlSaveOptions,
    pub(crate) jpeg: JpegSaveOptions,
    pub(crate) conflict: codec::ConflictAction,
}

impl BatchDialogState {
    /// The current dialog settings as `SaveOptions`.
    pub(crate) fn save_options(&self) -> SaveOptions {
        match self.format {
            SaveFormat::Jxl => SaveOptions::Jxl(self.jxl.clone()),
            SaveFormat::Jpeg => SaveOptions::Jpeg(self.jpeg.clone()),
            SaveFormat::Png => SaveOptions::Png,
            SaveFormat::Bmp => SaveOptions::Bmp,
            SaveFormat::Tiff => SaveOptions::Tiff,
            SaveFormat::Gif => SaveOptions::Gif,
        }
    }
}

pub enum UiMsg {
    Done(Result<String, String>),
    /// Batch-conversion heartbeat: file index just started + total.
    BatchProgress {
        done: usize,
        total: usize,
        name: String,
    },
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

    /// Central-panel mode: image canvas or folder grid. `Folder` requires
    /// `folder` to be `Some` (a grid with no scan result shows empty).
    pub(crate) mode: ViewMode,
    /// Folder-grid state (kept alive after opening an image, so ESC back is
    /// instant and thumbnails survive).
    pub(crate) folder: Option<FolderState>,
    /// Pending directory scan (worker thread → UI): (root, entries, switch).
    pub(crate) scan_rx:
        Option<Receiver<(PathBuf, Vec<(PathBuf, Option<std::time::SystemTime>)>, bool)>>,

    /// Folder-grid viewing preferences (session-level, survive folder
    /// switches and rescans).
    pub(crate) grid_sort: (SortKey, bool),
    pub(crate) grid_thumb_size: f32,

    /// Windows file-associations dialog (toolbar gear icon).
    pub(crate) assoc_open: bool,
    pub(crate) assoc_rows: Vec<AssocRow>,
    pub(crate) assoc_msg: Option<(bool, String)>,

    /// Batch-convert dialog (folder grid).
    pub(crate) batch_dialog: Option<BatchDialogState>,
    /// Cancel flag for the running batch job.
    pub(crate) busy_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// (done, total) while a batch job runs, for the overlay progress bar.
    pub(crate) busy_progress: Option<(usize, usize)>,

    pub(crate) busy: Option<String>,
    /// When the current save job started, for the elapsed-seconds readout.
    pub(crate) busy_started: Option<std::time::Instant>,
    pub(crate) task_rx: Option<Receiver<UiMsg>>,
    pub(crate) last_dir: Option<PathBuf>,
    pub(crate) status: String,
    pub(crate) error: Option<String>,
    /// File this session was opened with; Home returns here.
    pub(crate) original_path: Option<PathBuf>,
    /// Last-known canvas rect, used as the anchor for keyboard zooming.
    pub(crate) canvas_rect: Option<egui::Rect>,

    pub(crate) texture: Option<TextureHandle>,
    pub(crate) texture_nearest: bool,
    pub(crate) needs_rebuild: bool,
    pub(crate) checker: TextureHandle,
    pub(crate) pending_open: Option<PathBuf>,
}

const UNDO_LIMIT: usize = 5;

/// Opens the batch-convert dialog for the grid selection with a smart
/// default format (all-.jxl selection → PNG target, otherwise JXL).
fn app_batch_open(app: &mut App, format: SaveFormat) {
    app.batch_dialog = Some(BatchDialogState {
        format,
        jxl: JxlSaveOptions::default(),
        jpeg: JpegSaveOptions::default(),
        conflict: codec::ConflictAction::Skip,
    });
}

/// One row of the Windows file-associations dialog.
pub(crate) struct AssocRow {
    pub ext: String,
    pub desc: String,
    pub registered: bool,
}

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
            mode: ViewMode::Single,
            folder: None,
            scan_rx: None,
            grid_sort: (SortKey::Name, false),
            grid_thumb_size: crate::folder::DEFAULT_THUMB_SIZE,
            assoc_open: false,
            assoc_rows: Vec::new(),
            assoc_msg: None,
            batch_dialog: None,
            busy_cancel: None,
            busy_progress: None,
            busy: None,
            busy_started: None,
            task_rx: None,
            last_dir: None,
            status: String::new(),
            error: None,
            original_path: None,
            canvas_rect: None,
            texture: None,
            texture_nearest: false,
            needs_rebuild: true,
            checker,
            pending_open: None,
        }
    }

    pub fn open_on_startup(&mut self, path: PathBuf) {
        if path.is_dir() {
            self.open_folder_at(path, true);
        } else {
            self.pending_open = Some(path);
        }
    }

    // ---- editing -----------------------------------------------------------

    pub(crate) fn apply_edit(
        &mut self,
        ctx: &egui::Context,
        op: impl FnOnce(&Raster) -> Result<Raster, String>,
    ) {
        if self.busy.is_some() {
            return; // strict lockout: document is locked while saving
        }
        let Some(old) = self.doc.as_ref().map(|d| d.raster.clone()) else {
            self.error = Some("no image open".into());
            return;
        };
        self.push_undo();
        match op(&old) {
            Ok(new) => {
                if let Some(doc) = &mut self.doc {
                    // Edits operate on the displayed frame; keeping the rest of
                    // an animation consistent with them is not supported, so
                    // the animation flattens to a still.
                    let flattened = doc.anim.take().is_some();
                    doc.raster = new;
                    doc.dirty = true;
                    if flattened {
                        self.status =
                            "Animation flattened to a still (edit applied to current frame)".into();
                    }
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
        if self.busy.is_some() {
            return; // strict lockout
        }
        if let Some(r) = self.crop_rect.take() {
            self.apply_edit(ctx, move |ra| Ok(ra.crop(r)));
        }
    }

    pub(crate) fn undo_last(&mut self, ctx: &egui::Context) {
        if self.busy.is_some() {
            return; // strict lockout
        }
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
        if self.busy.is_some() {
            return; // strict lockout
        }
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

    /// Explicitly opens a file the user asked for (dialog, CLI argument, drag
    /// & drop). Becomes the session anchor that Home returns to.
    pub(crate) fn open_explicit(&mut self, path: PathBuf) {
        self.original_path = Some(path.clone());
        self.open_path(path);
    }

    /// Opens the folder grid: scans `dir` on a worker thread. When `switch`
    /// is set, the grid replaces the canvas as soon as the listing arrives;
    /// otherwise the scan only (re)builds the grid state in the background so
    /// a later ESC is instant (the viewer stays on the image).
    pub(crate) fn open_folder_at(&mut self, dir: PathBuf, switch: bool) {
        // One scan at a time; the latest request wins (older result ignored).
        if self.scan_rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.scan_rx = Some(rx);
        if switch {
            self.status = format!("Opening folder {}…", file_name(&dir));
        }
        std::thread::spawn(move || {
            let files = crate::folder::scan(&dir);
            let _ = tx.send((dir, files, switch));
        });
    }

    fn poll_folder_scan(&mut self, ctx: &egui::Context) {
        if let Some(rx) = self.scan_rx.take() {
            match rx.try_recv() {
                Ok((root, files, switch)) => {
                    let n = files.len();
                    // Preselect the currently open file, if it's in the list.
                    let selected = self
                        .doc
                        .as_ref()
                        .and_then(|d| d.path.as_ref())
                        .and_then(|p| files.iter().position(|(f, _)| path_key(f) == path_key(p)))
                        .unwrap_or(0);
                    let root_name = file_name(&root);
                    let (sort_key, sort_desc) = self.grid_sort;
                    self.folder = Some(FolderState::new(
                        root,
                        files,
                        sort_key,
                        sort_desc,
                        self.grid_thumb_size,
                    ));
                    if let Some(f) = &mut self.folder {
                        f.selected = selected;
                        f.scroll_to = Some(selected);
                    }
                    if switch {
                        self.mode = ViewMode::Folder;
                        self.status = format!("{} images in {root_name}", n);
                    }
                    ctx.request_repaint();
                }
                Err(_) => {
                    // Scan still running: put the receiver back and retry next
                    // frame.
                    self.scan_rx = Some(rx);
                    ctx.request_repaint_after(std::time::Duration::from_millis(60));
                }
            }
        }
    }

    pub(crate) fn open_path(&mut self, path: PathBuf) {
        // Strict lockout: covers Open, drag-&-drop, and folder navigation.
        if self.busy.is_some() {
            return;
        }
        match codec::decode_file(&path) {
            Ok(img) => {
                // Opening a file always shows the canvas — e.g. when the user
                // picks Open… while the folder grid is up.
                self.mode = ViewMode::Single;
                let w = img.raster.width;
                let h = img.raster.height;
                self.doc = Some(Document {
                    path: Some(path.clone()),
                    format: img.format,
                    raster: img.raster,
                    dirty: false,
                    hdr: img.hdr,
                    anim: img.animation.map(|a| AnimPlayer {
                        frames: a.frames,
                        durations_ms: a.durations_ms,
                        loops: a.loops,
                        frame_idx: 0,
                        playing: true,
                        frame_started: std::time::Instant::now(),
                    }),
                });
                if let Some(note) = &img.note {
                    // Informational (e.g. "decoded with libjxl (16-bit exact
                    // path)") — belongs in the status line, not an error toast.
                    self.status = format!("{} ({note})", self.status);
                }
                self.undo.clear();
                self.redo.clear();
                self.crop_rect = None;
                self.crop_drag = None;
                self.viewer = ViewerState::default(); // fit zoom
                self.needs_rebuild = true;
                self.last_dir = path.parent().map(|p| p.to_path_buf());
                let hdr_note = match img.hdr {
                    Some(h) => format!(", {h} HDR tone-mapped to SDR"),
                    None => String::new(),
                };
                let anim_note = match &self.doc.as_ref().unwrap().anim {
                    Some(a) => format!(", {}-frame animation", a.frame_count()),
                    None => String::new(),
                };
                self.status = format!(
                    "Opened {} — {} × {}, {}-bit{}{}",
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
                    },
                    [hdr_note, anim_note].concat(),
                );
                // Eagerly prepare the folder grid for this image's folder (in
                // the background, silently) so ESC is instant even when the
                // image came from a different folder than the grid shows.
                if let Some(dir) = path.parent().map(|p| p.to_path_buf()) {
                    let same_root = self
                        .folder
                        .as_ref()
                        .is_some_and(|f| path_key(&f.root) == path_key(&dir));
                    if !same_root {
                        self.open_folder_at(dir, false);
                    }
                }
            }
            Err(e) => self.error = Some(e),
        }
    }

    pub(crate) fn start_save_job(&mut self, path: PathBuf, opts: SaveOptions) {
        // Strict lockout backstop: every UI entry point is gated while a job
        // runs; this guarantees no second job can ever start.
        if self.busy.is_some() {
            return;
        }
        let Some(doc) = self.doc.clone() else { return };
        let (tx, rx) = mpsc::channel::<UiMsg>();
        self.task_rx = Some(rx);
        self.busy = Some(format!("Encoding {}…", file_name(&path)));
        self.busy_started = Some(std::time::Instant::now());
        self.busy_progress = None;
        self.status.clear();
        self.save_dialog = None;
        let raster = doc.raster;
        // Animated exports need every frame; only GIF and JPEG XL support
        // them — other formats encode the current raster only.
        let anim = if matches!(opts, SaveOptions::Gif | SaveOptions::Jxl(_)) {
            doc.anim.map(|a| (a.frames, a.durations_ms, a.loops))
        } else {
            None
        };
        std::thread::spawn(move || {
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match (anim, &opts) {
                    (
                        Some((frames, durations_ms, loops)),
                        SaveOptions::Gif | SaveOptions::Jxl(_),
                    ) => codec::save_animation_to_path(&path, &frames, &durations_ms, loops, &opts),
                    _ => codec::save_to_path(&path, &raster, &opts),
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

    /// Batch-converts the given files (folder grid selection) with one set of
    /// save options. Outputs go next to each source (same stem, target
    /// extension). Progress streams via `UiMsg::BatchProgress`; the Cancel
    /// flag stops the run after the file in progress.
    pub(crate) fn start_batch_job(
        &mut self,
        paths: Vec<PathBuf>,
        opts: SaveOptions,
        conflict: codec::ConflictAction,
    ) {
        if self.busy.is_some() || paths.is_empty() {
            return;
        }
        let (tx, rx) = mpsc::channel::<UiMsg>();
        self.task_rx = Some(rx);
        self.busy = Some(format!("Converting 0/{}…", paths.len()));
        self.busy_started = Some(std::time::Instant::now());
        self.busy_progress = None;
        self.status.clear();
        self.batch_dialog = None;
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.busy_cancel = Some(cancel.clone());
        let total = paths.len();
        std::thread::spawn(move || {
            let summary = codec::batch_convert(
                &paths,
                &opts,
                conflict,
                |done, name| {
                    let _ = tx.send(UiMsg::BatchProgress {
                        done,
                        total,
                        name: name.to_string(),
                    });
                },
                &cancel,
            );

            let head = if summary.cancelled {
                format!("Cancelled — {} converted", summary.converted)
            } else {
                format!("Converted {} of {} images", summary.converted, total)
            };
            let mut parts = vec![head];
            if summary.skipped_same_ext > 0 {
                parts.push(format!(
                    "{} already in the target format (skipped)",
                    summary.skipped_same_ext
                ));
            }
            if summary.skipped_existing > 0 {
                parts.push(format!(
                    "{} existing outputs skipped",
                    summary.skipped_existing
                ));
            }
            if !summary.failed.is_empty() {
                parts.push(format!(
                    "{} failed: {}",
                    summary.failed.len(),
                    summary
                        .failed
                        .iter()
                        .take(3)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("; ")
                ));
            }
            let _ = tx.send(UiMsg::Done(Ok(parts.join(" — "))));
        });
    }

    fn poll_messages(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.task_rx else { return };
        let mut progress_seen = false;
        let mut finished = false;
        while let Ok(msg) = rx.try_recv() {
            match msg {
                UiMsg::BatchProgress { done, total, name } => {
                    self.busy = Some(format!("Converting {}/{} — {}…", done + 1, total, name));
                    self.busy_progress = Some((done, total));
                    progress_seen = true;
                }
                UiMsg::Done(Ok(what)) => {
                    self.busy = None;
                    self.busy_started = None;
                    self.busy_progress = None;
                    self.status = what;
                    finished = true;
                }
                UiMsg::Done(Err(e)) => {
                    self.busy = None;
                    self.busy_started = None;
                    self.busy_progress = None;
                    self.error = Some(e);
                    finished = true;
                }
            }
        }
        if finished {
            self.task_rx = None;
            self.busy_cancel = None;
            // The folder listing may have changed (batch created files, or a
            // save renamed nothing but times moved on) — refresh silently.
            if let Some(f) = &self.folder {
                let root = f.root.clone();
                self.open_folder_at(root, false);
            }
        } else if progress_seen {
            ctx.request_repaint();
        } else {
            // Keep the spinner animating and the elapsed counter ticking
            // even when nothing else wants a repaint.
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
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
        SaveOptions::Gif => "GIF".into(),
    }
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

/// Case-insensitive full-path comparison (Windows-friendly).
fn path_key(p: &Path) -> String {
    p.to_string_lossy().to_lowercase()
}

/// File name for display.
fn name_of(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

/// Truncates with an ellipsis so the label fits `max_width` points (rough:
/// assumes ~7.0 points per char at the small font).
fn elide(name: &str, max_width: f32) -> String {
    let max_chars = ((max_width / 7.0).floor() as usize).max(4);
    if name.chars().count() <= max_chars {
        return name.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    let head: String = name.chars().take(keep).collect();
    format!("{head}…")
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
        self.poll_folder_scan(ctx);

        if let Some(path) = self.pending_open.take() {
            self.open_explicit(path);
            ctx.request_repaint();
        }

        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        // Strict lockout: no new documents while a save job runs.
        if self.busy.is_none() {
            if let Some(first) = dropped.first() {
                self.open_explicit(first.clone());
                ctx.request_repaint();
            }
        }
    }

    /// Draw the whole app.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_shortcuts(&ctx);
        if self.mode == ViewMode::Single {
            // Animation frames advance only while the canvas is visible.
            self.tick_animation(&ctx);
        }
        if self.mode == ViewMode::Single {
            self.refresh_texture(&ctx);
        } else if let Some(folder) = &mut self.folder {
            folder.poll_thumbs(&ctx, |path, raster| {
                ctx.load_texture(
                    format!("thumb-{}", path.display()),
                    raster.to_color_image(),
                    egui::TextureOptions::LINEAR,
                )
            });
        }

        egui::Panel::top("toolbar")
            .resizable(false)
            .show(ui, |ui| self.show_toolbar(ui, &ctx));

        egui::Panel::bottom("status")
            .resizable(false)
            .show(ui, |ui| self.show_statusbar(ui));

        egui::CentralPanel::default().show(ui, |ui| match self.mode {
            ViewMode::Folder => self.show_folder_grid(ui, &ctx),
            ViewMode::Single => self.show_canvas(ui, &ctx),
        });

        self.show_dialogs(&ctx);

        // Modal-ish overlay while a job (save/batch) runs: unmissable
        // "working" indicator with progress and Cancel; the central panel
        // beneath stays visible but frozen.
        if self.busy.is_some() {
            self.show_busy_overlay(ui, &ctx);
        }

        self.update_window_title(&ctx);
    }
}

impl App {
    /// Advances animation playback and schedules the next repaint.
    fn tick_animation(&mut self, ctx: &egui::Context) {
        let Some(doc) = &mut self.doc else { return };
        let Some(anim) = &mut doc.anim else { return };
        if !anim.playing {
            return;
        }
        let dur = anim.durations_ms[anim.frame_idx].max(1) as u128;
        let elapsed = anim.frame_started.elapsed().as_millis();
        if elapsed >= dur {
            let n = anim.frames.len();
            let advance = (elapsed / dur) as usize;
            anim.frame_idx = (anim.frame_idx + advance) % n;
            doc.raster = anim.frames[anim.frame_idx].clone();
            anim.frame_started = std::time::Instant::now();
            self.needs_rebuild = true;
        }
        let remaining = dur.saturating_sub(anim.frame_started.elapsed().as_millis()) as u64;
        ctx.request_repaint_after(std::time::Duration::from_millis(remaining.max(1)));
    }

    /// Steps the animation by ±1 frame (pausing playback).
    fn anim_step(&mut self, delta: isize, ctx: &egui::Context) {
        let Some(doc) = &mut self.doc else { return };
        let Some(anim) = &mut doc.anim else { return };
        anim.playing = false;
        let n = anim.frames.len() as isize;
        anim.frame_idx = (anim.frame_idx as isize + delta).rem_euclid(n) as usize;
        doc.raster = anim.frames[anim.frame_idx].clone();
        anim.frame_started = std::time::Instant::now();
        self.needs_rebuild = true;
        ctx.request_repaint();
    }

    /// Toggles animation play/pause.
    fn anim_toggle_play(&mut self, ctx: &egui::Context) {
        let Some(doc) = &mut self.doc else { return };
        let Some(anim) = &mut doc.anim else { return };
        anim.playing = !anim.playing;
        anim.frame_started = std::time::Instant::now();
        if anim.playing {
            let dur = anim.durations_ms[anim.frame_idx].max(1) as u64;
            ctx.request_repaint_after(std::time::Duration::from_millis(dur));
        }
        ctx.request_repaint();
    }

    /// Keyboard shortcuts.
    ///
    /// * Ctrl+Z / Ctrl+Y (or Ctrl+Shift+Z) — undo / redo
    /// * Ctrl+O / Ctrl+S — open / save-as
    /// * P / C — pan / crop tool
    /// * R / L — rotate right / left, H / V — flip horizontal / vertical
    /// * + / − — zoom, 0 — fit, 1 — 1:1
    /// * Enter — apply crop selection, Escape — close dialog / clear crop
    /// * Space — play/pause animation, `,` / `.` — step frame
    /// * ←/→ by name, ↑/↓ by mtime, Home — original file (folder navigation)
    ///
    /// Never fires while a text field has keyboard focus; when a dialog is
    /// open, only Escape acts.
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        use egui::Key;
        let pressed = |key: Key| ctx.input(|i| i.key_pressed(key));

        // Never hijack keys while a text field (e.g. a DragValue) has focus.
        if ctx.egui_wants_keyboard_input() {
            return;
        }

        let escape = pressed(Key::Escape);
        if escape {
            if self.save_dialog.take().is_some() {
                ctx.request_repaint();
                return;
            }
            if self.resize_dialog.take().is_some() {
                ctx.request_repaint();
                return;
            }
            if self.crop_rect.is_some() || self.crop_drag.is_some() {
                self.crop_rect = None;
                self.crop_drag = None;
                ctx.request_repaint();
                return;
            }
            if self.tool == Tool::Crop {
                self.tool = Tool::Pan;
                ctx.request_repaint();
                return;
            }
            if self.error.take().is_some() {
                ctx.request_repaint();
                return;
            }
            // In grid mode ESC clears an active multi-selection.
            if self.mode == ViewMode::Folder {
                let had = self
                    .folder
                    .as_mut()
                    .map(|f| f.multi.take().is_some())
                    .unwrap_or(false);
                if had {
                    ctx.request_repaint();
                }
                return;
            }
            // Back out to the folder grid when one is active (and we're not
            // already looking at it).
            if self.folder.is_some() && self.mode == ViewMode::Single {
                self.return_to_folder();
                ctx.request_repaint();
                return;
            }
        }

        // While a modal-ish dialog is up, other shortcuts are ignored.
        if self.save_dialog.is_some() || self.resize_dialog.is_some() {
            return;
        }

        // Strict lockout while a save job runs — mirrors the greyed toolbar
        // buttons. Viewing shortcuts (zoom, animation) stay live; everything
        // that replaces or mutates the document is ignored.
        let locked = self.busy.is_some();

        let ctrl = ctx.input(|i| i.modifiers.ctrl);
        let shift = ctx.input(|i| i.modifiers.shift);
        let plain = ctx.input(|i| {
            let m = i.modifiers;
            !m.ctrl && !m.alt && !m.shift && !m.mac_cmd && !m.command
        });

        // ---- undo / redo ---------------------------------------------------
        if ctrl && !shift && pressed(Key::Z) && !locked {
            self.undo_last(ctx);
            return;
        }
        if ctrl && (pressed(Key::Y) || (shift && pressed(Key::Z))) && !locked {
            self.redo_last(ctx);
            return;
        }

        // ---- file ------------------------------------------------------------
        if ctrl && shift && pressed(Key::O) && !locked {
            self.pick_open_folder();
            return;
        }
        if ctrl && pressed(Key::O) && !locked {
            self.pick_open_file();
            return;
        }
        if ctrl && pressed(Key::S) && !locked {
            if self.doc.is_some() {
                self.save_dialog = Some(SaveDialogState {
                    format: SaveFormat::Jxl,
                    jxl: JxlSaveOptions::default(),
                    jpeg: JpegSaveOptions::default(),
                });
                ctx.request_repaint();
            }
            return;
        }

        // ---- tools -----------------------------------------------------------
        if plain && pressed(Key::P) {
            self.tool = Tool::Pan;
            self.crop_rect = None;
            self.crop_drag = None;
            ctx.request_repaint();
            return;
        }
        if plain && pressed(Key::C) && !locked {
            self.tool = Tool::Crop;
            self.crop_rect = None;
            self.crop_drag = None;
            ctx.request_repaint();
            return;
        }

        // ---- animation playback ---------------------------------------------
        if self.doc.as_ref().is_some_and(|d| d.anim.is_some()) && plain {
            if pressed(Key::Space) {
                self.anim_toggle_play(ctx);
                return;
            }
            if pressed(Key::Period) {
                self.anim_step(1, ctx);
                return;
            }
            if pressed(Key::Comma) {
                self.anim_step(-1, ctx);
                return;
            }
        }

        // ---- folder-grid mode -------------------------------------------------
        // In grid mode, arrows move the selection, Ctrl+A selects all,
        // Enter/double-click opens, and the image-editing shortcuts don't
        // apply.
        if self.mode == ViewMode::Folder {
            if locked {
                return; // batch conversion running — grid is frozen
            }
            let Some(folder) = &mut self.folder else {
                return;
            };
            if folder.files.is_empty() {
                return;
            }
            if ctrl && pressed(egui::Key::A) {
                folder.select_all();
                ctx.request_repaint();
                return;
            }
            let n = folder.files.len();
            let cols = folder.columns.max(1);
            let mut sel = folder.selected;
            let mut new_size: Option<f32> = None;
            if plain && pressed(Key::ArrowRight) {
                sel = (sel + 1).min(n - 1);
            } else if plain && pressed(Key::ArrowLeft) {
                sel = sel.saturating_sub(1);
            } else if plain && pressed(Key::ArrowDown) {
                sel = (sel + cols).min(n - 1);
            } else if plain && pressed(Key::ArrowUp) {
                sel = sel.saturating_sub(cols);
            } else if plain && pressed(Key::Home) {
                sel = 0;
            } else if plain && pressed(Key::End) {
                sel = n - 1;
            } else if plain && pressed(Key::Enter) {
                let path = folder.files[folder.selected].clone();
                self.open_path(path);
                self.mode = ViewMode::Single;
                ctx.request_repaint();
                return;
            } else if plain && (pressed(Key::Plus) || pressed(Key::Equals)) {
                folder.set_thumb_size(folder.thumb_size + 32.0);
                new_size = Some(folder.thumb_size);
            } else if plain && pressed(Key::Minus) {
                folder.set_thumb_size(folder.thumb_size - 32.0);
                new_size = Some(folder.thumb_size);
            } else {
                return;
            }
            folder.selected = sel;
            folder.scroll_to = Some(sel);
            if let Some(s) = new_size {
                self.grid_thumb_size = s;
            }
            ctx.request_repaint();
            return;
        }

        // ---- edits (need an image) ------------------------------------------
        let has_doc = self.doc.is_some();
        if !has_doc {
            return;
        }
        if plain && pressed(Key::R) && !locked {
            self.apply_edit(ctx, |r| Ok(r.rotate_cw()));
            return;
        }
        if plain && pressed(Key::L) && !locked {
            self.apply_edit(ctx, |r| Ok(r.rotate_ccw()));
            return;
        }
        if plain && pressed(Key::H) && !locked {
            self.apply_edit(ctx, |r| Ok(r.flip_h()));
            return;
        }
        if plain && pressed(Key::V) && !locked {
            self.apply_edit(ctx, |r| Ok(r.flip_v()));
            return;
        }

        // ---- crop selection --------------------------------------------------
        if self.tool == Tool::Crop && self.crop_rect.is_some() && pressed(Key::Enter) && !locked {
            self.apply_crop(ctx);
            return;
        }

        // ---- zoom ------------------------------------------------------------
        if plain && (pressed(Key::Plus) || pressed(Key::Equals)) {
            self.zoom_step(ctx, 1.25);
            return;
        }
        if plain && pressed(Key::Minus) {
            self.zoom_step(ctx, 1.0 / 1.25);
            return;
        }
        if plain && pressed(Key::Num0) {
            self.set_view_zoom(None, ctx);
            return;
        }
        if plain && pressed(Key::Num1) {
            self.set_view_zoom(Some(1.0), ctx);
            return;
        }

        // ---- folder navigation ------------------------------------------------
        let (left, right, up, down, home) = (
            pressed(Key::ArrowLeft),
            pressed(Key::ArrowRight),
            pressed(Key::ArrowUp),
            pressed(Key::ArrowDown),
            pressed(Key::Home),
        );
        if !(left || right || up || down || home) {
            return;
        }
        // Strict lockout: navigation replaces the document mid-save.
        if locked {
            return;
        }

        if home {
            if let Some(original) = self.original_path.clone() {
                let viewing_original = self
                    .doc
                    .as_ref()
                    .and_then(|d| d.path.as_ref())
                    .is_some_and(|p| path_key(p) == path_key(&original));
                if !viewing_original && original.exists() {
                    self.open_path(original);
                }
            }
            return;
        }

        let Some(doc) = &self.doc else { return };
        let Some(path) = doc.path.clone() else {
            return;
        };
        let Some(dir) = path.parent().map(|p| p.to_path_buf()) else {
            return;
        };

        let target = if left {
            crate::nav::neighbor_by_name(&dir, &path, false)
        } else if right {
            crate::nav::neighbor_by_name(&dir, &path, true)
        } else if up {
            crate::nav::neighbor_by_date(&dir, &path, false)
        } else {
            crate::nav::neighbor_by_date(&dir, &path, true)
        };
        if let Some(target) = target {
            self.open_path(target);
        }
    }

    /// Keyboard zoom about the canvas center.
    fn zoom_step(&mut self, ctx: &egui::Context, factor: f32) {
        let Some(doc) = &self.doc else { return };
        let Some(canvas) = self.canvas_rect else {
            return;
        };
        let img_size = Vec2::new(doc.raster.width as f32, doc.raster.height as f32);
        let zoom = self.effective_zoom(canvas.size(), img_size);
        let new_zoom = (zoom * factor).clamp(0.02, 60.0);
        let base_offset = if self.viewer.zoom.is_none() {
            (canvas.size() - img_size * zoom) / 2.0
        } else {
            self.viewer.offset
        };
        let img_rect = Rect::from_min_size(canvas.min + base_offset, img_size * zoom);
        let anchor = crate::view::screen_to_px(canvas.center(), img_rect, img_size);
        self.viewer.zoom = Some(new_zoom);
        self.viewer.offset = (canvas.center() - canvas.min) - anchor * new_zoom;
        self.needs_rebuild = true;
        ctx.request_repaint();
    }

    /// Sets an explicit zoom (or `None` for fit), centering the image in the
    /// last-known canvas.
    fn set_view_zoom(&mut self, zoom: Option<f32>, ctx: &egui::Context) {
        match zoom {
            Some(z) => {
                let Some(doc) = &self.doc else {
                    return;
                };
                let img = Vec2::new(doc.raster.width as f32, doc.raster.height as f32);
                self.viewer.zoom = Some(z);
                self.viewer.offset = self
                    .canvas_rect
                    .map(|c| (c.size() - img * z) / 2.0)
                    .unwrap_or(Vec2::ZERO);
            }
            None => {
                // Fit mode auto-centers every frame.
                self.viewer.zoom = None;
                self.viewer.offset = Vec2::ZERO;
            }
        }
        ctx.request_repaint();
    }

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
        if self.mode == ViewMode::Folder {
            let title = match &self.folder {
                Some(f) => format!(
                    "SoftOxide JXL Viewer — {} ({} images)",
                    file_name(&f.root),
                    f.files.len()
                ),
                None => "SoftOxide JXL Viewer — folder".to_string(),
            };
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
            return;
        }
        let title = match &self.doc {
            Some(doc) => {
                let name = doc
                    .path
                    .as_ref()
                    .map(|p| file_name(p))
                    .unwrap_or_else(|| "untitled".into());
                format!(
                    "SoftOxide JXL Viewer — {name}{} ({}{}{})",
                    if doc.dirty { "*" } else { "" },
                    doc.format.as_str(),
                    if doc.raster.has_alpha { ", alpha" } else { "" },
                    if doc.hdr.is_some() { ", HDR" } else { "" }
                )
            }
            None => "SoftOxide JXL Viewer — open an image".to_string(),
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
    }

    // ---- toolbar -----------------------------------------------------------

    fn show_toolbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal_wrapped(|ui| {
            // Strict lockout while a save job runs: document-mutating buttons
            // go grey; viewing (pan/zoom/animation) stays live.
            let locked = self.busy.is_some();
            if ui
                .add_enabled(
                    !locked,
                    egui::Button::new(format!("{}  Open…", icons::FOLDER_OPEN)),
                )
                .on_hover_text("Open an image (Ctrl+O)")
                .clicked()
            {
                self.pick_open_file();
            }
            if ui
                .add_enabled(
                    !locked,
                    egui::Button::new(format!("{}  Folder…", icons::FOLDER_OPEN)),
                )
                .on_hover_text(
                    "Open a folder as a thumbnail grid (Ctrl+Shift+O); Esc returns from an image",
                )
                .clicked()
            {
                self.pick_open_folder();
            }
            ui.separator();

            let has = self.doc.is_some();
            let can_edit = has && !locked;
            if ui
                .add_enabled(
                    can_edit,
                    egui::Button::new(format!("{}  Undo", icons::ARROW_COUNTER_CLOCKWISE)),
                )
                .on_hover_text("Undo (Ctrl+Z)")
                .clicked()
            {
                self.undo_last(ctx);
            }
            if ui
                .add_enabled(
                    can_edit && !self.redo.is_empty(),
                    egui::Button::new(format!("{}  Redo", icons::ARROW_CLOCKWISE)),
                )
                .on_hover_text("Redo (Ctrl+Y)")
                .clicked()
            {
                self.redo_last(ctx);
            }
            ui.separator();

            if ui
                .selectable_label(self.tool == Tool::Pan, format!("{}  Pan", icons::HAND_PALM))
                .on_hover_text("Pan tool (P) — drag to move the image")
                .clicked()
            {
                self.tool = Tool::Pan;
                self.crop_rect = None;
                self.crop_drag = None;
            }
            if ui
                .add_enabled(
                    !locked,
                    egui::Button::selectable(
                        self.tool == Tool::Crop,
                        format!("{}  Crop", icons::CROP),
                    ),
                )
                .on_hover_text("Crop tool (C) — drag a selection, Enter to apply, Esc to cancel")
                .clicked()
            {
                self.tool = Tool::Crop;
                self.crop_rect = None;
                self.crop_drag = None;
            }

            // Animation playback controls
            let anim_info = self
                .doc
                .as_ref()
                .and_then(|d| d.anim.as_ref())
                .map(|a| (a.playing, a.frame_idx, a.frame_count()));
            if let Some((playing, frame_idx, frame_count)) = anim_info {
                ui.separator();
                if ui
                    .button(if playing {
                        format!("{}", icons::PAUSE)
                    } else {
                        format!("{}", icons::PLAY)
                    })
                    .on_hover_text(if playing {
                        "Pause (Space)"
                    } else {
                        "Play (Space)"
                    })
                    .clicked()
                {
                    self.anim_toggle_play(ctx);
                }
                if ui
                    .button(icons::SKIP_BACK)
                    .on_hover_text("Previous frame (,)")
                    .clicked()
                {
                    self.anim_step(-1, ctx);
                }
                // Fixed-size allocation: the counter would otherwise reflow the
                // toolbar every time it crosses between 1 and 2 digits.
                ui.add_sized(
                    [64.0, 18.0],
                    egui::Label::new(format!("{}/{}", frame_idx + 1, frame_count)),
                );
                if ui
                    .button(icons::SKIP_FORWARD)
                    .on_hover_text("Next frame (.)")
                    .clicked()
                {
                    self.anim_step(1, ctx);
                }
            }

            if ui
                .add_enabled(
                    can_edit,
                    egui::Button::new(format!("{}  Rotate L", icons::ARROW_COUNTER_CLOCKWISE)),
                )
                .on_hover_text("Rotate 90° left (L)")
                .clicked()
            {
                self.apply_edit(ctx, |r| Ok(r.rotate_ccw()));
            }
            if ui
                .add_enabled(
                    can_edit,
                    egui::Button::new(format!("{}  Rotate R", icons::ARROW_CLOCKWISE)),
                )
                .on_hover_text("Rotate 90° right (R)")
                .clicked()
            {
                self.apply_edit(ctx, |r| Ok(r.rotate_cw()));
            }
            if ui
                .add_enabled(
                    can_edit,
                    egui::Button::new(format!("{}  Flip H", icons::FLIP_HORIZONTAL)),
                )
                .on_hover_text("Flip horizontally (H)")
                .clicked()
            {
                self.apply_edit(ctx, |r| Ok(r.flip_h()));
            }
            if ui
                .add_enabled(
                    can_edit,
                    egui::Button::new(format!("{}  Flip V", icons::FLIP_VERTICAL)),
                )
                .on_hover_text("Flip vertically (V)")
                .clicked()
            {
                self.apply_edit(ctx, |r| Ok(r.flip_v()));
            }
            ui.separator();

            if ui
                .add_enabled(
                    can_edit,
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
                    keep_aspect: true,
                    filter: ResizeFilter::default(),
                });
            }
            ui.separator();
            if ui
                .add_enabled(
                    can_edit,
                    egui::Button::new(format!("{}  Save as…", icons::FLOPPY_DISK)),
                )
                .on_hover_text("Save as… (Ctrl+S)")
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
            // Fixed width so the label changing (fit ↔ percent) doesn't reflow.
            if ui
                .add_sized([78.0, 18.0], egui::Button::new(zoom_label))
                .on_hover_text("Fit to window (0); mouse wheel zooms at the pointer")
                .clicked()
            {
                self.set_view_zoom(None, ctx);
            }
            if ui
                .add_enabled(has, egui::Button::new("1:1"))
                .on_hover_text("Actual size, 100% (1)")
                .clicked()
            {
                self.set_view_zoom(Some(1.0), ctx);
            }

            // Windows-only: per-user file associations (gear icon at the end).
            #[cfg(windows)]
            if ui
                .button(format!("{}", icons::GEAR))
                .on_hover_text("File associations…")
                .clicked()
            {
                self.assoc_open = !self.assoc_open;
                if self.assoc_open {
                    self.reload_assoc_rows();
                    self.assoc_msg = None;
                }
            }
        });
    }

    /// Re-reads the registry state for the associations dialog.
    #[cfg(windows)]
    fn reload_assoc_rows(&mut self) {
        self.assoc_rows = crate::assoc::FORMATS
            .iter()
            .map(|f| AssocRow {
                ext: f.ext.to_string(),
                desc: f.desc.to_string(),
                registered: crate::assoc::is_registered(f.ext),
            })
            .collect();
    }

    /// ESC from the image canvas back to the folder grid: selection jumps to
    /// the image being viewed.
    fn return_to_folder(&mut self) {
        let doc_dir = self
            .doc
            .as_ref()
            .and_then(|d| d.path.as_ref())
            .and_then(|p| p.parent().map(|p| p.to_path_buf()));

        match (&self.folder, doc_dir) {
            (Some(_), None) => {
                // Untitled document: show whatever grid we have.
                self.mode = ViewMode::Folder;
            }
            (Some(f), Some(dir)) if path_key(&f.root) == path_key(&dir) => {
                // Same folder: instant, thumbnails still cached.
                let folder = self.folder.as_mut().unwrap();
                folder.selected = self
                    .doc
                    .as_ref()
                    .and_then(|d| d.path.as_ref())
                    .and_then(|p| folder.files.iter().position(|f| path_key(f) == path_key(p)))
                    .unwrap_or(folder.selected);
                folder.scroll_to = Some(folder.selected);
                self.mode = ViewMode::Folder;
            }
            (_, Some(dir)) => {
                // Different folder (or no grid yet): rescan in the background
                // and swap the grid in when the listing lands.
                self.open_folder_at(dir, true);
            }
            (None, None) => {}
        }
    }

    fn pick_open_file(&mut self) {
        if self.busy.is_some() {
            return; // strict lockout
        }
        let mut dialog = rfd::FileDialog::new().set_title("Open image");
        if let Some(dir) = &self.last_dir {
            dialog = dialog.set_directory(dir);
        }
        dialog = dialog.add_filter(
            "Images",
            &[
                "jxl", "png", "jpg", "jpeg", "gif", "bmp", "tif", "tiff", "webp", "ppm", "pgm",
                "pbm", "pnm",
            ],
        );
        if let Some(path) = dialog.pick_file() {
            self.open_explicit(path);
        }
    }

    fn pick_open_folder(&mut self) {
        if self.scan_rx.is_some() {
            return; // a scan is already running
        }
        let mut dialog = rfd::FileDialog::new().set_title("Open folder");
        if let Some(dir) = &self.last_dir {
            dialog = dialog.set_directory(dir);
        }
        if let Some(dir) = dialog.pick_folder() {
            self.open_folder_at(dir, true);
        }
    }

    // ---- canvas ------------------------------------------------------------

    fn show_canvas(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let avail_rect = ui.available_rect_before_wrap();
        self.canvas_rect = Some(avail_rect);
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
        // Fit mode auto-centers the image in the canvas; explicit zoom/pan
        // (wheel, drag, 1:1) takes over from the centered position.
        let base_offset = if self.viewer.zoom.is_none() {
            (avail_rect.size() - shown) / 2.0
        } else {
            self.viewer.offset
        };
        let img_rect = Rect::from_min_size(avail_rect.min + base_offset, shown);

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

        // pan with drag (pan tool) or middle button. Dragging detaches the
        // image from fit-mode auto-centering.
        if response.dragged_by(egui::PointerButton::Middle)
            || (response.dragged_by(egui::PointerButton::Primary) && self.tool == Tool::Pan)
        {
            if self.viewer.zoom.is_none() {
                self.viewer.zoom = Some(zoom);
                self.viewer.offset = base_offset;
            }
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

    // ---- folder grid -------------------------------------------------------

    /// Centered mask + card shown while a job runs: spinner, label, progress
    /// bar (batch), and Cancel (batch). Covers the central panel so grid/canvas
    /// interactions are impossible while working.
    fn show_busy_overlay(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(busy) = self.busy.clone() else {
            return;
        };
        let Some(rect) = self.canvas_rect else { return };

        // Veil over the whole central panel.
        let veil = ui.painter_at(rect);
        veil.rect_filled(rect, 0.0, Color32::from_black_alpha(120));

        let overlay_id = egui::Id::unique(&std::process::id());
        egui::Area::new(overlay_id)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, -20.0])
            .show(ctx, |ui| {
                egui::Frame::window(ui.style()).show(ui, |ui| {
                    ui.set_min_width(280.0);
                    ui.vertical_centered(|ui| {
                        ui.heading(egui::RichText::new("Working…").strong());
                        ui.add_space(4.0);
                        ui.spinner();
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(&busy).weak());
                        if let Some((done, total)) = &self.busy_progress {
                            let frac = if *total > 0 {
                                (*done as f32 / *total as f32).clamp(0.0, 1.0)
                            } else {
                                0.0
                            };
                            ui.add(egui::ProgressBar::new(frac).show_percentage());
                            ui.label(format!("{} / {} files", done, total));
                        }
                        if let Some(cancel) = &self.busy_cancel {
                            ui.add_space(6.0);
                            if ui.button(format!("{}  Cancel", icons::X_CIRCLE)).clicked() {
                                cancel.store(true, Ordering::Relaxed);
                            }
                        }
                    });
                });
            });
        // The spinner needs frames to animate even when nothing else repaints.
        ctx.request_repaint_after(std::time::Duration::from_millis(120));
    }

    // ---- folder grid -------------------------------------------------------

    /// Central-panel folder view: grid of thumbnails, double-click opens.
    /// While a batch job runs, the header and cells render non-interactive
    /// (the progress overlay in `ui()` covers the panel).
    fn show_folder_grid(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        // The central rect doubles as the busy-veil area for `ui()`'s overlay.
        self.canvas_rect = Some(ui.available_rect_before_wrap());
        let locked = self.busy.is_some();

        let Some(folder) = &mut self.folder else {
            ui.centered_and_justified(|ui| {
                ui.label("No folder open — use Open folder… (Ctrl+Shift+O)");
            });
            return;
        };

        let mut open_idx: Option<usize> = None;
        // Preference changes made in the header; synced to App prefs after
        // the folder borrow ends.
        let mut pref_sort: Option<(SortKey, bool)> = None;
        let mut pref_size: Option<f32> = None;
        let mut open_batch: Option<SaveFormat> = None;

        // ---- grid header: select / convert / sort / size --------------------
        if !locked {
            let selected_count = folder.selected_count();
            let multi = folder.multi.is_some();
            ui.horizontal(|ui| {
                ui.menu_button("Select…", |ui| {
                    if ui.button("All").clicked() {
                        folder.select_all();
                        ui.close();
                    }
                    if ui.button("All .jxl images").clicked() {
                        folder.select_all_ext("jxl", false);
                        ui.close();
                    }
                    if ui.button("All except .jxl").clicked() {
                        folder.select_all_ext("jxl", true);
                        ui.close();
                    }
                    if ui.button("None").clicked() {
                        folder.select_none();
                        ui.close();
                    }
                });
                ui.label(
                    egui::RichText::new(if multi {
                        format!("{selected_count} selected")
                    } else {
                        String::new()
                    })
                    .weak(),
                );

                if ui
                    .add_enabled(
                        selected_count > 0,
                        egui::Button::new(format!(
                            "{}  Convert {}…",
                            icons::CHECK_CIRCLE,
                            selected_count
                        )),
                    )
                    .on_hover_text("Batch-convert the selected images with one set of save options")
                    .clicked()
                {
                    // Smart default format: .jxl selection → PNG; everything
                    // else → JPEG XL.
                    let paths = folder.selected_paths();
                    let all_jxl = !paths.is_empty()
                        && paths.iter().all(|p| {
                            p.extension()
                                .and_then(|e| e.to_str())
                                .map(|e| e.eq_ignore_ascii_case("jxl"))
                                .unwrap_or(false)
                        });
                    let format = if all_jxl {
                        SaveFormat::Png
                    } else {
                        SaveFormat::Jxl
                    };
                    open_batch = Some(format);
                }

                ui.separator();
                ui.label("Sort:");
                let mut key = folder.sort_key;
                egui::ComboBox::from_id_salt("grid_sort")
                    .selected_text(key.label())
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        let keys = [(SortKey::Name, "Name"), (SortKey::Date, "Date modified")];
                        for (k, label) in keys {
                            ui.selectable_value(&mut key, k, label);
                        }
                    });
                if key != folder.sort_key {
                    pref_sort = Some((key, folder.sort_desc));
                    folder.set_sort(key, folder.sort_desc);
                }

                let dir_icon = if folder.sort_desc {
                    icons::ARROW_UP
                } else {
                    icons::ARROW_DOWN
                };
                let dir_text = if folder.sort_desc {
                    "descending"
                } else {
                    "ascending"
                };
                if ui
                    .button(dir_icon)
                    .on_hover_text(format!("Toggle order (currently {dir_text})"))
                    .clicked()
                {
                    let desc = !folder.sort_desc;
                    pref_sort = Some((folder.sort_key, desc));
                    folder.set_sort(folder.sort_key, desc);
                }

                ui.separator();
                ui.label("Size:");
                let mut size = folder.thumb_size;
                let slider = ui.add_sized(
                    [150.0, 18.0],
                    egui::Slider::new(
                        &mut size,
                        crate::folder::MIN_THUMB_SIZE..=crate::folder::MAX_THUMB_SIZE,
                    )
                    .step_by(32.0)
                    .show_value(false),
                );
                if size != folder.thumb_size {
                    folder.set_thumb_size(size);
                    pref_size = Some(folder.thumb_size);
                }
                let _ = slider;
                ui.label(egui::RichText::new(format!("{} px", folder.thumb_size)).weak());
            });
            ui.separator();
        }

        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                let cell = folder.thumb_size;
                let label_h = 18.0;
                let pad = 10.0;
                let cell_w = cell + pad * 2.0;
                let cell_h = cell + label_h + pad * 2.0;
                let cols = ((ui.available_width() / cell_w).floor() as usize).clamp(1, 64);
                folder.columns = cols;

                let n = folder.files.len();
                if n == 0 {
                    ui.centered_and_justified(|ui| {
                        ui.label("This folder contains no supported images.");
                    });
                    return;
                }

                let scroll_to = folder.scroll_to.take();

                for row_start in (0..n).step_by(cols) {
                    let row_end = (row_start + cols).min(n);
                    ui.horizontal(|ui| {
                        for idx in row_start..row_end {
                            let path = folder.files[idx].clone();
                            let selected = folder.is_selected(idx);

                            let (rect, resp) = ui.allocate_exact_size(
                                egui::vec2(cell_w, cell_h),
                                if locked {
                                    Sense::hover()
                                } else {
                                    Sense::click()
                                },
                            );
                            let painter = ui.painter_at(rect);
                            let bg = if selected {
                                Color32::from_rgb(52, 62, 92)
                            } else if resp.hovered() {
                                Color32::from_rgb(46, 46, 52)
                            } else {
                                Color32::from_rgb(38, 38, 44)
                            };
                            painter.rect_filled(rect, 6.0, bg);
                            if selected {
                                painter.rect_stroke(
                                    rect,
                                    6.0,
                                    egui::Stroke::new(2.0, Color32::from_rgb(120, 160, 255)),
                                    egui::StrokeKind::Inside,
                                );
                            }

                            // Thumb area.
                            let inner = egui::Rect::from_min_size(
                                rect.min + egui::vec2(pad, pad),
                                egui::vec2(cell, cell),
                            );
                            let tex = folder.thumb(&path);
                            match tex {
                                Some(t) => {
                                    let size = t.size_vec2();
                                    let scale =
                                        (inner.width() / size.x).min(inner.height() / size.y);
                                    let shown = size * scale;
                                    let img_rect = egui::Rect::from_min_size(
                                        inner.center() - shown / 2.0,
                                        shown,
                                    );
                                    painter.image(
                                        t.id(),
                                        img_rect,
                                        Rect::from_min_max(
                                            egui::pos2(0.0, 0.0),
                                            egui::pos2(1.0, 1.0),
                                        ),
                                        Color32::WHITE,
                                    );
                                }
                                None => {
                                    if folder.needs_thumb(&path) {
                                        folder.request_thumb(path.clone());
                                    }
                                    painter.text(
                                        inner.center(),
                                        egui::Align2::CENTER_CENTER,
                                        format!("{}  …", icons::IMAGE),
                                        egui::FontId::proportional(20.0),
                                        Color32::GRAY,
                                    );
                                }
                            }

                            // Filename label (elided to the cell width).
                            let name = name_of(&path);
                            painter.text(
                                egui::pos2(rect.center().x, rect.max.y - pad - label_h / 2.0),
                                egui::Align2::CENTER_CENTER,
                                elide(&name, cell_w - pad),
                                egui::FontId::proportional(11.0),
                                if selected {
                                    Color32::WHITE
                                } else {
                                    Color32::LIGHT_GRAY
                                },
                            );

                            if !locked {
                                if resp.clicked() {
                                    let mods = ctx.input(|i| i.modifiers);
                                    folder.click(idx, mods.ctrl, mods.shift);
                                }
                                if resp.double_clicked() {
                                    open_idx = Some(idx);
                                }
                                resp.context_menu(|ui| {
                                    if ui.button("Open").clicked() {
                                        open_idx = Some(idx);
                                        ui.close();
                                    }
                                    ui.separator();
                                    if ui.button("Select all").clicked() {
                                        folder.select_all();
                                        ui.close();
                                    }
                                    if ui.button("Select all .jxl").clicked() {
                                        folder.select_all_ext("jxl", false);
                                        ui.close();
                                    }
                                    if ui.button("Select all except .jxl").clicked() {
                                        folder.select_all_ext("jxl", true);
                                        ui.close();
                                    }
                                    if ui.button("Deselect all").clicked() {
                                        folder.select_none();
                                        ui.close();
                                    }
                                });
                            }
                            if let Some(target) = scroll_to {
                                if target == idx {
                                    ui.scroll_to_rect(rect, Some(egui::Align::Center));
                                }
                            }
                        }
                    });
                }

                // Drain finished thumbnails (installs textures, requests repaint).
                let ctx2 = ctx.clone();
                folder.poll_thumbs(&ctx2, |path, raster| {
                    ctx.load_texture(
                        format!("thumb-{}", path.display()),
                        raster.to_color_image(),
                        egui::TextureOptions::LINEAR,
                    )
                });
            });

        // Sync header-made preference changes into the session prefs (the
        // folder borrow ended with the ScrollArea above).
        if let Some((k, d)) = pref_sort {
            self.grid_sort = (k, d);
        }
        if let Some(s) = pref_size {
            self.grid_thumb_size = s;
        }
        if let Some(format) = open_batch {
            app_batch_open(self, format);
        }

        // Open after the ScrollArea closure is done (folder borrow released).
        if let Some(idx) = open_idx {
            let Some(folder) = &mut self.folder else {
                return;
            };
            let path = folder.files[idx].clone();
            folder.selected = idx;
            self.open_path(path);
            self.mode = ViewMode::Single;
            ctx.request_repaint();
        }
    }

    // ---- status ------------------------------------------------------------

    fn show_statusbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if self.mode == ViewMode::Folder {
                let folder_info = self.folder.as_ref().map(|f| {
                    let multi = if f.multi.is_some() {
                        format!(" · {} selected", f.selected_count())
                    } else {
                        String::new()
                    };
                    format!(
                        "{} images — {}{}{}",
                        f.files.len(),
                        f.root.display(),
                        multi,
                        if f.selected < f.files.len() {
                            format!(
                                " · {}/{} {}",
                                f.selected + 1,
                                f.files.len(),
                                file_name(&f.files[f.selected])
                            )
                        } else {
                            String::new()
                        }
                    )
                });
                if let Some(info) = folder_info {
                    ui.label(info);
                }
                ui.label(&self.status);
                return;
            }
            if let Some(doc) = &self.doc {
                ui.label(format!(
                    "{} × {} px, {}-bit, alpha {}{}",
                    doc.raster.width,
                    doc.raster.height,
                    doc.raster.bits,
                    if doc.raster.has_alpha { "yes" } else { "no" },
                    if let Some(anim) = &doc.anim {
                        format!(
                            ", frame {}/{}{}",
                            anim.frame_idx + 1,
                            anim.frame_count(),
                            if anim.playing { "" } else { " (paused)" }
                        )
                    } else {
                        String::new()
                    }
                ));
                if doc.dirty {
                    ui.strong("  (modified)");
                }
                ui.separator();
            }
            if let Some(busy) = &self.busy {
                ui.spinner();
                let secs = self.busy_started.map_or(0, |t| t.elapsed().as_secs());
                ui.label(format!("{busy} ({secs}s)"));
                if let Some(cancel) = &self.busy_cancel {
                    if ui.button("Cancel").clicked() {
                        cancel.store(true, Ordering::Relaxed);
                    }
                }
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
        crate::dialogs::show_batch_dialog(ctx, self);
        #[cfg(windows)]
        crate::dialogs::show_associations_dialog(ctx, self);
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
