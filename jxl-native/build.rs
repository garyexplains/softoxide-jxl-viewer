//! Builds libjxl from the checkout in `third_party/libjxl` and links it
//! statically. libjxl is Apache-2.0-licensed, so no copyleft taint (which is
//! why we don't use the GPL-3.0 `jpegxl-rs` wrapper).
//!
//! Get the sources with `tools/fetch-libjxl.ps1` / `tools/fetch-libjxl.sh`,
//! or point `JXL_LIBJXL_DIR` at any libjxl checkout (v0.12.x).

use std::env;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=JXL_LIBJXL_DIR");
    println!("cargo:rerun-if-env-changed=JXL_LIBJXL_REBUILD");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let _ = out_dir; // used implicitly by cmake crate output conventions
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));

    let src_dir: PathBuf = env::var("JXL_LIBJXL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest_dir.join("../third_party/libjxl").join("."));

    // dunce::canonicalize avoids the `\\?\` extended-length prefix, which
    // breaks CMake's file(… GLOB_RECURSE) handling on Windows.
    let src = dunce::canonicalize(&src_dir).unwrap_or_else(|_| src_dir.clone());
    if !src.join("lib/include/jxl/encode.h").is_file() {
        eprintln!(
            "\njxl-native: libjxl sources not found at {}. Fetch them with:\n  \
             tools/fetch-libjxl.ps1   (Windows)\n  tools/fetch-libjxl.sh    (macOS/Linux)\n  \
             or set JXL_LIBJXL_DIR to a v0.12.x libjxl checkout.\n",
            src.display()
        );
        panic!("libjxl sources not found");
    }

    let mut cfg = cmake::Config::new(&src);
    cfg.profile("Release");
    // Static, no demos/tools/anything beyond the core libs.
    cfg.define("BUILD_SHARED_LIBS", "OFF");
    cfg.define("JPEGXL_ENABLE_BENCHMARK", "OFF");
    cfg.define("JPEGXL_ENABLE_EXAMPLES", "OFF");
    cfg.define("JPEGXL_ENABLE_MANPAGES", "OFF");
    cfg.define("JPEGXL_ENABLE_JNI", "OFF");
    cfg.define("JPEGXL_ENABLE_SJPEG", "OFF");
    cfg.define("JPEGXL_ENABLE_OPENEXR", "OFF");
    cfg.define("JPEGXL_ENABLE_SKIFF", "OFF");
    cfg.define("JPEGXL_ENABLE_VIEWERS", "OFF");
    cfg.define("JPEGXL_ENABLE_PLUGINS", "OFF");
    cfg.define("JPEGXL_ENABLE_DEVCONTAINER", "OFF");
    cfg.define("JPEGXL_ENABLE_TCMALLOC", "OFF");
    cfg.define("JPEGXL_ENABLE_WASMU", "OFF");
    cfg.define("BUILD_TESTING", "OFF");
    cfg.always_configure(env::var_os("JXL_LIBJXL_REBUILD").is_none());

    let dst = cfg.build();

    // Locate all the static libraries we may need. libjxl's install tree is
    // conventionally <out>/lib{,64}; be permissive and scan a couple levels.
    let mut search_dirs: Vec<PathBuf> = Vec::new();
    for sub in ["lib", "lib64", "lib/x86_64-linux-gnu", "lib/x86_64-windows"] {
        let d = dst.join(sub);
        if d.is_dir() {
            search_dirs.push(d);
        }
    }
    search_dirs.push(dst.clone());

    let mut found: Vec<(String, PathBuf)> = Vec::new();
    for d in &search_dirs {
        if let Ok(rd) = std::fs::read_dir(d) {
            for entry in rd.flatten() {
                if let Ok(name) = entry.file_name().into_string() {
                    found.push((name, entry.path()));
                }
            }
        }
    }

    // logical-name, filename-candidates (checked in order)
    let wanted: &[(&str, &[&str])] = &[
        ("jxl", &["jxl-static", "jxl"]),
        ("jxl_cms", &["jxl_cms-static", "jxl_cms"]),
        ("jxl_threads", &["jxl_threads-static", "jxl_threads"]),
        ("hwy", &["hwy", "hwy-static"]),
        ("brotlienc", &["brotlienc-static", "brotlienc"]),
        ("brotlidec", &["brotlidec-static", "brotlidec"]),
        ("brotlicommon", &["brotlicommon-static", "brotlicommon"]),
    ];

    let mut link_names: Vec<String> = Vec::new();
    let mut missing: Vec<&str> = Vec::new();

    for &(logical, candidates) in wanted {
        if let Some((link_name, lib_path)) = find_lib(&found, candidates) {
            println!(
                "cargo:rustc-link-search=native={}",
                lib_path.parent().unwrap().display()
            );
            link_names.push(link_name);
        } else {
            missing.push(logical);
            let _ = logical; // silence var in release of this code path
        }
    }

    if !missing.is_empty() {
        eprintln!("\njxl-native: missing static libraries: {missing:?}");
        eprintln!("Clean the cmake build and try again: cargo clean -p jxl-native");
        panic!("libjxl static libs not found");
    }

    for name in &link_names {
        println!("cargo:rustc-link-lib=static={name}");
    }

    // C++ runtime for the static C++ archives.
    if cfg!(unix) {
        if cfg!(target_vendor = "apple") {
            println!("cargo:rustc-link-lib=dylib=c++");
        } else {
            println!("cargo:rustc-link-lib=dylib=stdc++");
        }
    }

    println!("cargo:root={}", dst.display());
}

/// Finds e.g. `jxl-static.lib` / `jxl.lib` / `libjxl.a` among `found` entries.
fn find_lib(found: &[(String, PathBuf)], candidates: &[&str]) -> Option<(String, PathBuf)> {
    let windowsish = cfg!(windows);
    for candidate in candidates {
        for (name, path) in found {
            let (stem, ext) = split_name(name);
            if stem.as_str() == *candidate {
                let ok_ext = if windowsish {
                    ext.eq_ignore_ascii_case("lib")
                } else {
                    ext == "a"
                };
                if ok_ext {
                    return Some((candidate.to_string(), path.clone()));
                }
            }
        }
    }
    None
}

fn split_name(file: &str) -> (String, String) {
    let ext = match file.rsplit_once('.') {
        Some((_, e)) => e.to_string(),
        None => String::new(),
    };
    let stem_no_ext = file.rsplit_once('.').map_or(file, |(s, _)| s);
    let stem = stem_no_ext
        .strip_prefix("lib")
        .unwrap_or(stem_no_ext)
        .to_string();
    (stem, ext)
}

#[allow(dead_code)] // vendored path doc for future contributors
fn unused_markers(_: &Path) {}
