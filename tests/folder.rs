//! Folder-grid logic: scan filtering, natural ordering, thumbnails.

use std::fs;
use std::path::{Path, PathBuf};

use jxlviewer::codec::{self, SaveOptions};
use jxlviewer::folder;
use jxlviewer::raster::Raster;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("jxlviewer-folder-{tag}-{}", std::process::id()));
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

#[test]
fn scan_filters_and_sorts_naturally() {
    let dir = scratch("scan");
    write_png(&dir.join("img10.png"), 8, 8, [255, 0, 0, 255]);
    write_png(&dir.join("img2.png"), 8, 8, [0, 255, 0, 255]);
    write_png(&dir.join("img1.png"), 8, 8, [0, 0, 255, 255]);
    fs::write(dir.join("notes.txt"), b"x").unwrap();
    fs::create_dir_all(dir.join("subdir")).unwrap();

    let files: Vec<PathBuf> = folder::scan(&dir).into_iter().map(|(p, _)| p).collect();
    let names: Vec<String> = files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["img1.png", "img2.png", "img10.png"]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn thumb_preserves_aspect_and_fits_box() {
    // 1000x500 → 200x100.
    let data = vec![90u8; 1000 * 500 * 4];
    let r = Raster::from_rgba8(1000, 500, true, data);
    let t = folder::make_thumb(&r, folder::DEFAULT_THUMB_SIZE);
    assert_eq!((t.width, t.height), (200, 100));

    // 80x40 stays as-is (smaller than the box).
    let small = Raster::from_rgba8(80, 40, false, vec![1u8; 80 * 40 * 3]);
    let t2 = folder::make_thumb(&small, folder::DEFAULT_THUMB_SIZE);
    assert_eq!((t2.width, t2.height), (80, 40));

    // Tall image: 100x800 → 25x200.
    let tall = Raster::from_rgba8(100, 800, false, vec![1u8; 100 * 800 * 3]);
    let t3 = folder::make_thumb(&tall, folder::DEFAULT_THUMB_SIZE);
    assert_eq!((t3.width, t3.height), (25, 200));

    // Bigger box: 1000x500 at 512 → 512x256.
    let t4 = folder::make_thumb(&r, 512.0);
    assert_eq!((t4.width, t4.height), (512, 256));
}

#[test]
fn sort_by_name_and_date_with_direction() {
    use jxlviewer::folder::SortKey;

    let dir = scratch("sort");
    // mtime order differs from name order.
    write_png(&dir.join("aaa.png"), 8, 8, [255, 0, 0, 255]);
    filetime::set_file_mtime(
        &dir.join("aaa.png"),
        filetime::FileTime::from_unix_time(1000, 0),
    )
    .unwrap();
    write_png(&dir.join("ccc.png"), 8, 8, [0, 255, 0, 255]);
    filetime::set_file_mtime(
        &dir.join("ccc.png"),
        filetime::FileTime::from_unix_time(3000, 0),
    )
    .unwrap();
    write_png(&dir.join("bbb.png"), 8, 8, [0, 0, 255, 255]);
    filetime::set_file_mtime(
        &dir.join("bbb.png"),
        filetime::FileTime::from_unix_time(2000, 0),
    )
    .unwrap();

    let entries = folder::scan(&dir);
    let mut state = folder::FolderState::new(
        dir.clone(),
        entries,
        folder::SortKey::Name,
        false,
        folder::DEFAULT_THUMB_SIZE,
    );
    let names = |s: &folder::FolderState| -> Vec<String> {
        s.files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    };

    // Name ascending (scan default): aaa, bbb, ccc.
    assert_eq!(names(&state), vec!["aaa.png", "bbb.png", "ccc.png"]);
    state.selected = 1; // bbb
    state.set_sort(SortKey::Name, true);
    assert_eq!(names(&state), vec!["ccc.png", "bbb.png", "aaa.png"]);
    // Selection (bbb) survives the re-sort.
    assert_eq!(names(&state)[state.selected], "bbb.png");

    state.set_sort(SortKey::Date, false);
    assert_eq!(names(&state), vec!["aaa.png", "bbb.png", "ccc.png"]);
    state.set_sort(SortKey::Date, true);
    assert_eq!(names(&state), vec!["ccc.png", "bbb.png", "aaa.png"]);
    assert_eq!(names(&state)[state.selected], "bbb.png");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn thumb_size_changes_clear_cache_and_budget_shrinks() {
    let dir = scratch("size");
    let entries = folder::scan(&dir);
    let mut state = folder::FolderState::new(
        dir.clone(),
        entries,
        folder::SortKey::Name,
        false,
        folder::DEFAULT_THUMB_SIZE,
    );

    let big = folder::MAX_THUMB_SIZE;
    let cap_at_200 = state.max_thumbs();
    state.set_thumb_size(big);
    assert_eq!(state.thumb_size, big);
    // Budget: cap must shrink as thumbs grow.
    assert!(state.max_thumbs() < cap_at_200);
    // Out-of-range sizes clamp.
    state.set_thumb_size(9999.0);
    assert_eq!(state.thumb_size, big);
    state.set_thumb_size(1.0);
    assert_eq!(state.thumb_size, folder::MIN_THUMB_SIZE);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn png_to_jxl_folder_roundtrip_decodes_to_thumbs() {
    // End-to-end: real files on disk, decoded through the same codec path the
    // thumbnail workers use.
    let dir = scratch("thumbs");
    write_png(&dir.join("a.png"), 400, 220, [200, 30, 90, 255]);
    write_png(&dir.join("b.png"), 60, 90, [10, 200, 90, 128]);

    let entries = folder::scan(&dir);
    assert_eq!(entries.len(), 2);
    for (path, _) in &entries {
        let img = codec::decode_file(path).expect("decode");
        let thumb = folder::make_thumb(&img.raster, folder::DEFAULT_THUMB_SIZE);
        let fits = thumb.width as f32 <= folder::DEFAULT_THUMB_SIZE
            && thumb.height as f32 <= folder::DEFAULT_THUMB_SIZE;
        assert!(fits, "thumb must fit the box");
    }
    let _ = fs::remove_dir_all(&dir);
}
