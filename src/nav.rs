//! Folder navigation: previous/next recognized image file in the current
//! directory, ordered either alphabetically (by file name) or by modification
//! time. Wraps around at both ends.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::codec::{Format, format_by_extension};

fn is_recognized(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(format_by_extension)
        .is_some_and(|f| f != Format::Unknown)
}

fn name_key(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
        .to_lowercase()
}

fn list_images(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && is_recognized(&path) {
                out.push(path);
            }
        }
    }
    out
}

/// Steps to the neighbor of `current` in a cyclic ordering. Returns `None` when
/// the folder has no images or `current` is not among them.
fn step(list: &[PathBuf], current: &Path, forward: bool) -> Option<PathBuf> {
    let n = list.len();
    if n == 0 {
        return None;
    }
    let current_name = name_key(current);
    let idx = list.iter().position(|p| name_key(p) == current_name)?;
    let next = if forward {
        (idx + 1) % n
    } else {
        (idx + n - 1) % n
    };
    Some(list[next].clone())
}

/// Previous/next image alphabetically by file name.
pub fn neighbor_by_name(dir: &Path, current: &Path, forward: bool) -> Option<PathBuf> {
    let mut list = list_images(dir);
    if list.len() < 2 {
        return None;
    }
    list.sort_by_key(|p| name_key(p));
    step(&list, current, forward)
}

/// Previous/next image by modification time (ties broken by name).
pub fn neighbor_by_date(dir: &Path, current: &Path, forward: bool) -> Option<PathBuf> {
    let mut entries: Vec<(SystemTime, String, PathBuf)> = list_images(dir)
        .into_iter()
        .filter_map(|p| {
            let modified = std::fs::metadata(&p).ok()?.modified().ok()?;
            Some((modified, name_key(&p), p))
        })
        .collect();
    if entries.len() < 2 {
        return None;
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let list: Vec<PathBuf> = entries.into_iter().map(|(_, _, p)| p).collect();
    step(&list, current, forward)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jxlviewer-nav-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(dir: &Path, name: &str, mtime_shift_secs: Option<i64>) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, b"x").unwrap();
        if let Some(shift) = mtime_shift_secs {
            // Fixed base so the ordering is deterministic regardless of "now".
            let t = SystemTime::UNIX_EPOCH
                + std::time::Duration::from_secs(1_700_000_000 + shift as u64);
            filetime::set_file_mtime(&p, filetime::FileTime::from_system_time(t)).ok();
        }
        p
    }

    #[test]
    fn name_order_ignores_unrecognized_and_wraps() {
        let dir = scratch("name");
        touch(&dir, "b.png", None);
        touch(&dir, "a.jpg", None);
        touch(&dir, "c.txt", None); // not recognized, skipped
        touch(&dir, "d.jxl", None);
        touch(&dir, "subdir", None); // dir, skipped

        let cur = dir.join("b.png");
        let next = neighbor_by_name(&dir, &cur, true).unwrap();
        assert_eq!(next.file_name().unwrap().to_str().unwrap(), "d.jxl");
        let prev = neighbor_by_name(&dir, &cur, false).unwrap();
        assert_eq!(prev.file_name().unwrap().to_str().unwrap(), "a.jpg");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn name_order_case_insensitive() {
        let dir = scratch("case");
        touch(&dir, "APPLE.png", None);
        touch(&dir, "banana.jpg", None);
        let cur = dir.join("banana.jpg");
        let prev = neighbor_by_name(&dir, &cur, false).unwrap();
        assert_eq!(prev.file_name().unwrap().to_str().unwrap(), "APPLE.png");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn date_order_uses_mtime() {
        let dir = scratch("date");
        // names chosen so mtime order differs from name order
        touch(&dir, "aaa.png", Some(0));
        touch(&dir, "bbb.png", Some(60));
        touch(&dir, "ccc.png", Some(120));
        let cur = dir.join("bbb.png");

        let next = neighbor_by_date(&dir, &cur, true).unwrap();
        assert_eq!(next.file_name().unwrap().to_str().unwrap(), "ccc.png");
        let prev = neighbor_by_date(&dir, &cur, false).unwrap();
        assert_eq!(prev.file_name().unwrap().to_str().unwrap(), "aaa.png");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_current_yields_none() {
        let dir = scratch("missing");
        touch(&dir, "a.png", None);
        touch(&dir, "b.png", None);
        let ghost = dir.join("zzz.png");
        assert!(neighbor_by_name(&dir, &ghost, true).is_none());
        let _ = fs::remove_dir_all(&dir);
    }
}
