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

    // Toolchain selection (Windows only).
    //
    // Upstream builds the libjxl Windows release binaries with Visual
    // Studio's ClangCL toolset (`-T ClangCL` in their release workflow).
    // Measurements on identical sources (v0.12.0), machine and inputs show
    // the MSVC-built encoder runs **~50x slower** (0.2 MP/s vs 15 MP/s per
    // 12 threads on an 18 MP photo at effort 7), which is why clang-cl is
    // preferred automatically here when LLVM is found. Override with
    // `JXL_LIBJXL_TOOLCHAIN=msvc` (or `clang`; full path via
    // `JXL_LIBJXL_CLANG_CL`).
    #[derive(Clone, Copy, PartialEq)]
    enum Toolchain {
        Clang,
        Msvc,
    }
    let chosen = if cfg!(windows) {
        let name = env::var("JXL_LIBJXL_TOOLCHAIN")
            .as_deref()
            .ok()
            .map(|s| s.trim().to_ascii_lowercase());
        let clang = find_clang_cl();
        match name.as_deref() {
            Some("msvc") => Toolchain::Msvc,
            Some("clang") => match &clang {
                Some(_) => Toolchain::Clang,
                None => panic!("JXL_LIBJXL_TOOLCHAIN=clang but clang-cl.exe was not found"),
            },
            Some(other) => panic!("jxl-native: unknown JXL_LIBJXL_TOOLCHAIN={other} (clang|msvc)"),
            None => {
                if clang.is_some() {
                    Toolchain::Clang
                } else {
                    Toolchain::Msvc
                }
            }
        }
    } else {
        Toolchain::Msvc
    };

    if chosen == Toolchain::Clang {
        let clang_cl = find_clang_cl().expect("clang-cl.exe");
        let llvm_bin = clang_cl
            .parent()
            .expect("clang-cl.exe must have a parent directory")
            .to_path_buf();
        // Ninja honors CC/CXX; the VS generator would need a toolset
        // (`ClangCL` requires VS's optional LLVM component, which we
        // cannot assume is installed).
        cfg.generator("Ninja");
        // clang-cl needs lld-link (and friends) on PATH; CMake needs MSVC
        // headers, which clang-cl locates itself.
        let current_path = env::var("PATH").unwrap_or_default();
        let new_path = format!("{};{}", llvm_bin.display(), current_path);
        // Edition 2024: env mutation requires unsafe.
        unsafe { env::set_var("PATH", &new_path) };
        cfg.env("PATH", &new_path);
        // cmake's compiler detection runs in this process, so `CC`/`CXX`
        // must also be in the real environment.
        unsafe { env::set_var("CC", &clang_cl) };
        unsafe { env::set_var("CXX", &clang_cl) };
        // The crate's MSVC defaults (`/nologo -MD -Brepro -W0`) don't fly
        // under clang-cl; use CMake's own defaults and pin the dynamic CRT
        // the same way the MSVC build did.
        cfg.no_default_flags(true);
        cfg.cflag("/MD");
        cfg.cxxflag("/MD");
        cfg.define("CMAKE_BUILD_TYPE", "Release");
    }

    // Wipe a stale CMake cache when the generator/toolchain changed
    // (otherwise CMake errors instead of reconfiguring in place).
    #[cfg(windows)]
    {
        let build_dir_for_wipe = out_dir.join("build");
        let out_cache = build_dir_for_wipe.join("CMakeCache.txt");
        if out_cache.exists() {
            if let Ok(text) = std::fs::read_to_string(&out_cache) {
                let cached_ninja = text.contains("CMAKE_GENERATOR:INTERNAL=Ninja");
                let cached_clang = text.contains("clang-cl");
                let gen_changed = cached_ninja != (chosen == Toolchain::Clang);
                let tool_changed = cached_clang != (chosen == Toolchain::Clang);
                if gen_changed || tool_changed {
                    eprintln!(
                        "jxl-native: toolchain changed, wiping CMake cache in {}",
                        build_dir_for_wipe.display()
                    );
                    if let Err(e) = std::fs::remove_dir_all(&build_dir_for_wipe) {
                        eprintln!("jxl-native: cache wipe failed: {e}");
                    }
                }
            }
        }
    }

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

/// Locates a clang-cl.exe: `JXL_LIBJXL_CLANG_CL` override, LLVM's standard
/// install location, or Visual Studio's bundled LLVM component.
#[cfg(windows)]
fn find_clang_cl() -> Option<PathBuf> {
    if let Some(p) = env::var_os("JXL_LIBJXL_CLANG_CL") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let pf: PathBuf = env::var_os("ProgramFiles")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));

    // Standalone LLVM install.
    let llvm = pf.join("LLVM").join("bin").join("clang-cl.exe");
    if llvm.is_file() {
        return Some(llvm);
    }

    // Visual Studio's optional LLVM toolset component.
    let vs_root = pf.join("Microsoft Visual Studio");
    if vs_root.is_dir() {
        for year in ["2022", "2019"] {
            for edition in ["Community", "Professional", "Enterprise", "BuildTools"] {
                let candidate = vs_root
                    .join(year)
                    .join(edition)
                    .join("VC")
                    .join("Tools")
                    .join("Llvm")
                    .join("x64")
                    .join("bin")
                    .join("clang-cl.exe");
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

#[cfg(not(windows))]
fn find_clang_cl() -> Option<PathBuf> {
    None
}
