//! Folder grid view: scanning, ordering, and thumbnail plumbing.
//!
//! `scan` runs on a worker thread; thumbnails are decoded lazily for visible
//! cells (a small in-flight cap keeps memory and CPU sane) and delivered as
//! `Raster`s over a channel. The UI thread turns them into textures and
//! evicts least-recently-used ones so the cache stays inside a byte budget.
//!
//! Sorting (name/date, asc/desc) only reorders the listing — the thumbnail
//! cache is keyed by path and survives re-sorts.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use eframe::egui::TextureHandle;

use crate::codec::format_by_extension;
use crate::raster::{Raster, ResizeFilter};

/// Default thumbnail box (longest side); adjustable 96–512 in the grid.
pub const DEFAULT_THUMB_SIZE: f32 = 200.0;
pub const MIN_THUMB_SIZE: f32 = 96.0;
pub const MAX_THUMB_SIZE: f32 = 512.0;
/// Max simultaneously-decoding thumbnails.
pub const MAX_INFLIGHT: usize = 4;
/// GPU-memory budget for the thumbnail cache.
const THUMB_BUDGET_BYTES: usize = 96 * 1024 * 1024;

pub fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(format_by_extension)
        .is_some_and(|f| f != crate::codec::Format::Unknown)
}

/// Lists the supported images in `dir` (non-recursive) with their modification
/// times, naturally sorted by name ("img2" before "img10"); directories and
/// unrecognized files skipped.
pub fn scan(dir: &Path) -> Vec<(PathBuf, Option<SystemTime>)> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() || !is_supported(&path) {
                continue;
            }
            let mtime = entry.metadata().ok().and_then(|m| m.modified().ok());
            out.push((path, mtime));
        }
    }
    out.sort_by_key(|(p, _)| natural_key(&name_of(p)));
    out
}

fn name_of(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Sort key for the folder grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Date,
}

impl SortKey {
    pub fn label(self) -> &'static str {
        match self {
            SortKey::Name => "Name",
            SortKey::Date => "Date modified",
        }
    }
}

/// Sort key that treats digit runs as numbers ("img2" < "img10").
pub fn natural_key(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 8);
    let mut digits = String::new();
    for ch in name.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
        } else {
            flush_digits(&mut out, &mut digits);
            out.extend(ch.to_lowercase());
        }
    }
    flush_digits(&mut out, &mut digits);
    out
}

fn flush_digits(out: &mut String, digits: &mut String) {
    if digits.is_empty() {
        return;
    }
    let trimmed = digits.trim_start_matches('0');
    let trimmed = if trimmed.is_empty() { "0" } else { trimmed };
    out.push_str(&format!("{trimmed:0>10}"));
    digits.clear();
}

/// Downscale a raster to fit inside a `size`-square box, preserving aspect.
pub fn make_thumb(raster: &Raster, size: f32) -> Raster {
    let (w, h) = (raster.width, raster.height);
    if (w as f32) <= size && (h as f32) <= size {
        return raster.clone();
    }
    let scale = (size / w as f32).min(size / h as f32);
    let tw = ((w as f32 * scale).round() as u32).clamp(1, size as u32);
    let th = ((h as f32 * scale).round() as u32).clamp(1, size as u32);
    raster
        .resized(tw, th, ResizeFilter::Bilinear)
        .unwrap_or_else(|_| raster.clone())
}

/// Grid state for one folder (kept alive while browsing so ESC back is
/// instant and thumbnails survive).
pub struct FolderState {
    pub root: PathBuf,
    pub files: Vec<PathBuf>,
    /// Modification times gathered at scan time (sort-by-date cache).
    dates: HashMap<PathBuf, SystemTime>,
    pub sort_key: SortKey,
    pub sort_desc: bool,
    /// Longest side of the thumbnail box (96..=512).
    pub thumb_size: f32,
    /// Index of the current selection (keyboard + last opened).
    pub selected: usize,
    /// Column count of the last grid layout (keyboard up/down step).
    pub columns: usize,
    /// When set, the grid scrolls this file into view once, then clears.
    pub scroll_to: Option<usize>,
    thumbs: HashMap<PathBuf, TextureHandle>,
    /// Insertion order of `thumbs` for LRU eviction.
    thumb_order: Vec<PathBuf>,
    /// Decode requests sent, result not yet received.
    pub inflight: HashSet<PathBuf>,
    /// Decodes that failed (don't re-request every frame).
    pub failed: HashSet<PathBuf>,
    pub thumb_rx: std::sync::mpsc::Receiver<(PathBuf, f32, Option<Raster>)>,
    thumb_tx: std::sync::mpsc::Sender<(PathBuf, f32, Option<Raster>)>,
}

impl FolderState {
    pub fn new(
        root: PathBuf,
        entries: Vec<(PathBuf, Option<SystemTime>)>,
        sort_key: SortKey,
        sort_desc: bool,
        thumb_size: f32,
    ) -> Self {
        let mut dates = HashMap::new();
        let mut files = Vec::with_capacity(entries.len());
        for (p, mtime) in entries {
            if let Some(t) = mtime {
                dates.insert(p.clone(), t);
            }
            files.push(p);
        }
        let (thumb_tx, thumb_rx) = std::sync::mpsc::channel();
        let mut s = Self {
            root,
            files,
            dates,
            sort_key,
            sort_desc,
            thumb_size: thumb_size.clamp(MIN_THUMB_SIZE, MAX_THUMB_SIZE),
            selected: 0,
            columns: 1,
            scroll_to: None,
            thumbs: HashMap::new(),
            thumb_order: Vec::new(),
            inflight: HashSet::new(),
            failed: HashSet::new(),
            thumb_rx,
            thumb_tx,
        };
        s.apply_sort();
        s
    }

    /// Re-sorts the listing (stable for ties: name is the final tie-breaker
    /// in date mode). The currently selected file stays selected and is
    /// scrolled back into view.
    pub fn apply_sort(&mut self) {
        let sel_path = self.files.get(self.selected).cloned();
        let (key, desc) = (self.sort_key, self.sort_desc);
        let dates = &self.dates;
        self.files.sort_by(|a, b| {
            let ord = match key {
                SortKey::Name => natural_key(&name_of(a)).cmp(&natural_key(&name_of(b))),
                SortKey::Date => dates
                    .get(a)
                    .copied()
                    .unwrap_or(SystemTime::UNIX_EPOCH)
                    .cmp(&dates.get(b).copied().unwrap_or(SystemTime::UNIX_EPOCH))
                    .then_with(|| natural_key(&name_of(a)).cmp(&natural_key(&name_of(b)))),
            };
            if desc { ord.reverse() } else { ord }
        });
        if let Some(p) = sel_path {
            if let Some(i) = self.files.iter().position(|f| *f == p) {
                self.selected = i;
                self.scroll_to = Some(i);
            }
        }
    }

    /// Changes the sort key/direction and re-sorts.
    pub fn set_sort(&mut self, key: SortKey, desc: bool) {
        if key != self.sort_key || desc != self.sort_desc {
            self.sort_key = key;
            self.sort_desc = desc;
            self.apply_sort();
        }
    }

    /// Max cached thumbnails at the current size, from the byte budget.
    pub fn max_thumbs(&self) -> usize {
        let per = (self.thumb_size * self.thumb_size * 4.0) as usize;
        (THUMB_BUDGET_BYTES / per.max(1)).max(8)
    }

    /// Changes the thumbnail size; clears the cache so thumbs re-decode at
    /// the new size (upscaled old thumbs would blur).
    pub fn set_thumb_size(&mut self, size: f32) {
        let size = size.clamp(MIN_THUMB_SIZE, MAX_THUMB_SIZE);
        if size != self.thumb_size {
            self.thumb_size = size;
            self.thumbs.clear();
            self.thumb_order.clear();
        }
    }

    pub fn has_thumb(&self, path: &Path) -> bool {
        self.thumbs.contains_key(path)
    }

    pub fn thumb(&self, path: &Path) -> Option<&TextureHandle> {
        self.thumbs.get(path)
    }

    /// Inserts a decoded thumbnail (UI thread), evicting the LRU entry beyond
    /// the byte budget.
    pub fn store_thumb(&mut self, path: PathBuf, tex: TextureHandle) {
        if !self.thumbs.contains_key(&path) {
            self.thumb_order.push(path.clone());
        }
        self.thumbs.insert(path.clone(), tex);
        self.inflight.remove(&path);
        self.failed.remove(&path);
        while self.thumb_order.len() > self.max_thumbs() {
            let oldest = self.thumb_order.remove(0);
            self.thumbs.remove(&oldest);
        }
    }

    pub fn mark_failed(&mut self, path: &Path) {
        self.inflight.remove(path);
        self.failed.insert(path.to_path_buf());
    }

    pub fn needs_thumb(&self, path: &Path) -> bool {
        !self.thumbs.contains_key(path)
            && !self.inflight.contains(path)
            && !self.failed.contains(path)
            && self.inflight.len() < MAX_INFLIGHT
    }

    /// Sends a decode job to the thumbnail worker pool; the requested size
    /// rides along so stale results (size changed meanwhile) are discarded.
    pub fn request_thumb(&mut self, path: PathBuf) {
        if !self.needs_thumb(&path) {
            return;
        }
        self.inflight.insert(path.clone());
        let tx = self.thumb_tx.clone();
        let size = self.thumb_size;
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                crate::codec::decode_file(&path).map(|img| make_thumb(&img.raster, size))
            }));
            let payload = match result {
                Ok(Ok(raster)) => (path, size, Some(raster)),
                Ok(Err(e)) => {
                    eprintln!("thumbnail: {e}");
                    (path, size, None)
                }
                Err(_) => {
                    eprintln!("thumbnail: decoder panicked");
                    (path, size, None)
                }
            };
            let _ = tx.send(payload);
        });
    }

    /// Drains finished thumbnails (call from the UI thread each frame).
    /// Returns the number of new thumbs installed.
    pub fn poll_thumbs(
        &mut self,
        ctx: &eframe::egui::Context,
        load_tex: impl Fn(&Path, &Raster) -> TextureHandle,
    ) -> usize {
        let mut installed = 0;
        let current_size = self.thumb_size;
        while let Ok((path, made_for, result)) = self.thumb_rx.try_recv() {
            self.inflight.remove(&path);
            if made_for != current_size {
                continue; // stale: decoded at an older size; will be re-requested
            }
            match result {
                Some(raster) => {
                    let tex = load_tex(&path, &raster);
                    self.store_thumb(path, tex);
                }
                None => self.mark_failed(&path),
            }
            installed += 1;
        }
        if installed > 0 {
            ctx.request_repaint();
        }
        installed
    }
}
