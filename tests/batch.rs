//! Folder-grid batch conversion: selection model, conflict naming, and a
//! real end-to-end batch run.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use jxlviewer::codec::{self, ConflictAction, SaveOptions};
use jxlviewer::folder;
use jxlviewer::raster::Raster;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("jxlviewer-batch-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_png(path: &Path, w: u32, h: u32, rgba: [u8; 4]) {
    let data: Vec<u8> = (0..w * h).flat_map(|_| rgba).collect();
    let raster = Raster::from_rgba8(w, h, true, data);
    let bytes = codec::encode_to_vec(&raster, &SaveOptions::Png).expect("png encode");
    fs::write(path, bytes).unwrap();
}

fn state(dir: &Path) -> folder::FolderState {
    let entries = folder::scan(dir);
    folder::FolderState::new(
        dir.to_path_buf(),
        entries,
        folder::SortKey::Name,
        false,
        folder::DEFAULT_THUMB_SIZE,
    )
}

#[test]
fn selection_model_basics() {
    let dir = scratch("sel");
    write_png(&dir.join("a.jxl.png"), 4, 4, [255, 0, 0, 255]); // recognized?
    // Use real supported types instead: png + jxl via codec exports.
    let _ = fs::remove_dir_all(&dir);
    let dir = scratch("sel2");
    write_png(&dir.join("a.png"), 4, 4, [255, 0, 0, 255]);
    write_png(&dir.join("b.png"), 4, 4, [0, 255, 0, 255]);
    // .jxl via encode:
    let r = Raster::from_rgba8(4, 4, false, vec![10u8; 48]);
    let jxl = codec::encode_to_vec(&r, &SaveOptions::Jxl(jxl_opts())).unwrap();
    fs::write(dir.join("c.jxl"), jxl).unwrap();

    let mut s = state(&dir);
    assert_eq!(s.files.len(), 3, "a.png, b.png, c.jxl");

    // Plain click = single.
    s.click(1, false, false);
    assert_eq!(s.selected_paths().len(), 1);
    assert_eq!(s.selected_paths()[0], s.files[1]);

    // Select all .jxl → exactly c.jxl.
    s.select_all_ext("jxl", false);
    let picked = s.selected_paths();
    assert_eq!(picked.len(), 1);
    assert!(picked[0].to_string_lossy().ends_with("c.jxl"));

    // All except .jxl → the two pngs.
    s.select_all_ext("jxl", true);
    let picked = s.selected_paths();
    assert_eq!(picked.len(), 2);
    assert!(picked.iter().all(|p| p.to_string_lossy().ends_with(".png")));

    // Ctrl+click toggles: starting from all-except-jxl, ctrl-clicking a png
    // removes it.
    let anchor = s
        .files
        .iter()
        .position(|p| p.to_string_lossy().ends_with(".png"))
        .unwrap();
    s.click(anchor, true, false);
    assert_eq!(s.selected_paths().len(), 1);

    s.select_all();
    assert_eq!(s.selected_paths().len(), 3);
    s.select_none();
    assert_eq!(s.selected_paths().len(), 1, "none → single selected cell");
    let _ = fs::remove_dir_all(&dir);
}

fn jxl_opts() -> codec::JxlSaveOptions {
    codec::JxlSaveOptions {
        lossless: true,
        distance: 0.0,
        effort: 3,
        keep_alpha: true,
        modular: false,
        group_size: None,
    }
}

#[test]
fn batch_convert_pngs_to_jxl_with_unique_names() {
    let dir = scratch("batch");
    write_png(&dir.join("one.png"), 8, 8, [1, 2, 3, 255]);
    write_png(&dir.join("two.png"), 9, 9, [4, 5, 6, 255]);
    // Conflicts: one.jxl + one (2).jxl exist, and two.jxl exists — so the
    // Unique policy must land on "one (3).jxl" and "two (2).jxl"→ walk past it.
    let r = Raster::from_rgba8(4, 4, false, vec![9u8; 48]);
    let jxl = codec::encode_to_vec(&r, &SaveOptions::Jxl(jxl_opts())).unwrap();
    fs::write(dir.join("one.jxl"), &jxl).unwrap();
    fs::write(dir.join("one (2).jxl"), &jxl).unwrap();
    fs::write(dir.join("two.jxl"), &jxl).unwrap();

    let paths: Vec<PathBuf> = folder::scan(&dir)
        .into_iter()
        .filter(|(p, _)| {
            p.extension()
                .and_then(|e| e.to_str())
                .map(|e| e != "jxl")
                .unwrap_or(false)
        })
        .map(|(p, _)| p)
        .collect();
    assert_eq!(paths.len(), 2);

    let summary = codec::batch_convert(
        &paths,
        &SaveOptions::Jxl(jxl_opts()),
        ConflictAction::Unique,
        false,
        |_done, _name| {},
        &AtomicBool::new(false),
    );
    assert!(!summary.cancelled);
    assert_eq!(summary.converted, 2);
    assert_eq!(summary.skipped_existing, 0);

    // Unique policy walked past existing names: one (3), two (2).
    let files: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(files.contains(&"one.jxl".to_string()));
    assert!(files.contains(&"one (2).jxl".to_string()));
    assert!(files.contains(&"one (3).jxl".to_string()));
    assert!(files.contains(&"two.jxl".to_string()));
    assert!(files.contains(&"two (2).jxl".to_string()));
}

#[test]
fn batch_convert_conflict_policies() {
    let dir = scratch("conflict");
    write_png(&dir.join("a.png"), 8, 8, [1, 1, 1, 255]);
    let r = Raster::from_rgba8(4, 4, false, vec![3u8; 48]);
    let jxl = codec::encode_to_vec(&r, &SaveOptions::Jxl(jxl_opts())).unwrap();
    fs::write(dir.join("a.jxl"), &jxl).unwrap();
    let before = fs::metadata(dir.join("a.jxl")).unwrap().len();

    // Skip: nothing happens.
    let paths = vec![dir.join("a.png")];
    let s = codec::batch_convert(
        &paths,
        &SaveOptions::Jxl(jxl_opts()),
        ConflictAction::Skip,
        false,
        |_d, _n| {},
        &AtomicBool::new(false),
    );
    assert_eq!(s.skipped_existing, 1);
    assert_eq!(fs::metadata(dir.join("a.jxl")).unwrap().len(), before);

    // Overwrite: the file changes.
    let s = codec::batch_convert(
        &paths,
        &SaveOptions::Jxl(jxl_opts()),
        ConflictAction::Overwrite,
        false,
        |_d, _n| {},
        &AtomicBool::new(false),
    );
    assert_eq!(s.converted, 1);
    assert_ne!(fs::metadata(dir.join("a.jxl")).unwrap().len(), before);

    // Cancel: nothing converted.
    let cancel = AtomicBool::new(true);
    let s = codec::batch_convert(
        &paths,
        &SaveOptions::Jxl(jxl_opts()),
        ConflictAction::Overwrite,
        false,
        |_d, _n| {},
        &cancel,
    );
    assert!(s.cancelled);
    assert_eq!(s.converted, 0);
    let _ = fs::remove_dir_all(&dir);
}
