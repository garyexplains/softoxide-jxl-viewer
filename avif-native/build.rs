//! Builds libaom and libavif from the checkouts in `third_party/aom` and
//! `third_party/libavif` and links them statically.
//!
//! Get the sources with `tools/fetch-avif.ps1` / `tools/fetch-avif.sh`, or
//! point `AVIF_LIBAVIF_DIR` / `AVIF_AOM_DIR` at compatible checkouts
//! (libavif v1.4.x, libaom v3.15.x).

use std::env;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=AVIF_LIBAVIF_DIR");
    println!("cargo:rerun-if-env-changed=AVIF_AOM_DIR");
    println!("cargo:rerun-if-env-changed=AVIF_LIBAVIF_REBUILD");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));

    let libavif_dir = env::var("AVIF_LIBAVIF_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest_dir.join("../third_party/libavif"));
    let aom_dir = env::var("AVIF_AOM_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest_dir.join("../third_party/aom"));

    // dunce::canonicalize avoids the `\\?\` extended-length prefix, which
    // breaks CMake's file(… GLOB_RECURSE) handling on Windows (same reason as
    // jxl-native).
    let libavif_src = dunce::canonicalize(&libavif_dir).unwrap_or_else(|_| libavif_dir.clone());
    let aom_src = dunce::canonicalize(&aom_dir).unwrap_or_else(|_| aom_dir.clone());
    if !libavif_src.join("include/avif/avif.h").is_file() {
        eprintln!(
            "\navif-native: libavif sources not found at {}. Fetch them with:\n  \
             tools/fetch-avif.ps1 (Windows)\n  tools/fetch-avif.sh (macOS/Linux)\n  \
             or set AVIF_LIBAVIF_DIR to a v1.4.x libavif checkout.\n",
            libavif_src.display()
        );
        panic!("libavif sources not found");
    }
    if !aom_src.join("aom/aom_encoder.h").is_file() {
        eprintln!(
            "\navif-native: libaom sources not found at {}. Fetch them with:\n  \
             tools/fetch-avif.ps1 (Windows)\n  tools/fetch-avif.sh (macOS/Linux)\n  \
             or set AVIF_AOM_DIR to a v3.15.x libaom checkout.\n",
            aom_src.display()
        );
        panic!("libaom sources not found");
    }

    let toolchain = choose_toolchain();

    // Build libaom first (libavif's SYSTEM aom discovery points at its
    // install tree), then libavif itself.
    let dst_aom = build_cmake(&aom_src, out_dir.join("aom"), toolchain, |cfg| {
        // Static, nothing beyond the codec library.
        cfg.define("BUILD_SHARED_LIBS", "OFF");
        cfg.define("ENABLE_DOCS", "OFF");
        cfg.define("ENABLE_EXAMPLES", "OFF");
        cfg.define("ENABLE_TESTS", "OFF");
        cfg.define("ENABLE_TOOLS", "OFF");
        // Keep both decoder and encoder: the decoder is used by the app,
        // the encoder only by the `fixture-encoder` feature (test-fixture
        // generator. Never wired into the shipped app/CLI).
        cfg.define("CONFIG_AV1_DECODER", "1");
        cfg.define("CONFIG_AV1_ENCODER", "1");
    });

    let aom_include = dst_aom.join("include");
    let aom_lib = find_first_file(&dst_aom, &["aom.lib", "libaom.a"])
        .expect("avif-native: aom static library missing after build");

    let dst_avif = build_cmake(&libavif_src, out_dir.join("avif"), toolchain, |cfg| {
        cfg.define("BUILD_SHARED_LIBS", "OFF");
        cfg.define("AVIF_BUILD_APPS", "OFF");
        cfg.define("AVIF_BUILD_TESTS", "OFF");
        cfg.define("AVIF_ENABLE_WERROR", "OFF");
        // Only libaom is enabled; the other optional deps are OFF so
        // CMake doesn't hard-error on missing system packages (their
        // SYSTEM defaults are fatal in libavif's CMake).
        cfg.define("AVIF_CODEC_AOM", "SYSTEM");
        cfg.define("AVIF_LIBYUV", "OFF");
        cfg.define("AVIF_LIBSHARPYUV", "OFF");
        cfg.define("AVIF_LIBXML2", "OFF");
        cfg.define("AVIF_ZLIBPNG", "OFF");
        cfg.define("AVIF_JPEG", "OFF");
        // Direct cache overrides honored by Findaom.cmake (no
        // pkg-config needed on Windows).
        cfg.define("AOM_INCLUDE_DIR", aom_include.as_os_str().to_str().unwrap());
        cfg.define("AOM_LIBRARY", aom_lib.as_os_str().to_str().unwrap());
    });

    // Link the static artifacts; aom first, then avif (avif calls into aom).
    for (dst, logical) in [(&dst_aom, "aom"), (&dst_avif, "avif")] {
        let lib = find_first_file(
            dst,
            if logical == "aom" {
                &["aom.lib", "libaom.a"]
            } else {
                &["avif.lib", "libavif.a"]
            },
        )
        .unwrap_or_else(|| panic!("avif-native: {logical} static library missing"));
        println!(
            "cargo:rustc-link-search=native={}",
            lib.parent().unwrap().display()
        );
        println!("cargo:rustc-link-lib=static={logical}");
    }

    // C++ runtime for the static archives (aom/libavif are C, but stay in
    // step with jxl-native's linking hygiene).
    if cfg!(unix) {
        if cfg!(target_vendor = "apple") {
            println!("cargo:rustc-link-lib=dylib=c++");
        } else {
            println!("cargo:rustc-link-lib=dylib=stdc++");
        }
    }
}

// ---------------------------------------------------------------------------
// Toolchain — mirrors jxl-native/build.rs (clang-cl preferred on Windows for
// the established reasons there: MSVC-built C++ codecs measured dramatically
// slower; aom/libavif are mostly C but share the one toolchain story).
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Toolchain {
    Clang,
    Msvc,
}

fn choose_toolchain() -> Toolchain {
    if !cfg!(windows) {
        return Toolchain::Msvc;
    }
    let clang = find_clang_cl();
    let name = env::var("AVIF_LIBAVIF_TOOLCHAIN")
        .as_deref()
        .ok()
        .map(|s| s.trim().to_ascii_lowercase());
    match name.as_deref() {
        Some("msvc") => Toolchain::Msvc,
        Some("clang") => match &clang {
            Some(_) => Toolchain::Clang,
            None => panic!("AVIF_LIBAVIF_TOOLCHAIN=clang but clang-cl.exe was not found"),
        },
        Some(other) => panic!("avif-native: unknown AVIF_LIBAVIF_TOOLCHAIN={other} (clang|msvc)"),
        None => {
            if clang.is_some() {
                Toolchain::Clang
            } else {
                Toolchain::Msvc
            }
        }
    }
}

fn apply_toolchain(cfg: &mut cmake::Config, toolchain: Toolchain) {
    if toolchain != Toolchain::Clang {
        return;
    }
    let clang_cl = find_clang_cl().expect("clang-cl.exe");
    let llvm_bin = clang_cl
        .parent()
        .expect("clang-cl.exe must have a parent directory")
        .to_path_buf();
    let current_path = env::var("PATH").unwrap_or_default();
    let new_path = format!("{};{}", llvm_bin.display(), current_path);
    // Edition 2024: env mutation requires unsafe. The cmake crate's compiler
    // detection runs in this process, so CC/CXX and PATH must be in the real
    // environment as well as the child's.
    unsafe { env::set_var("PATH", &new_path) };
    unsafe { env::set_var("CC", &clang_cl) };
    unsafe { env::set_var("CXX", &clang_cl) };
    cfg.env("PATH", &new_path);
    // The cmake crate's MSVC defaults (`/nologo -MD -Brepro -W0`) don't fly
    // under clang-cl; use CMake's own defaults and pin the dynamic CRT the
    // same way the MSVC build did in jxl-native.
    cfg.no_default_flags(true);
    cfg.cflag("/MD");
    cfg.cxxflag("/MD");
}

/// Wipes a stale CMake cache when the generator/toolchain changed (otherwise
/// CMake errors instead of reconfiguring in place). Mirrors jxl-native.
#[cfg(windows)]
fn wipe_if_toolchain_changed(build_dir: &Path, use_clang: bool) {
    let cache = build_dir.join("CMakeCache.txt");
    if !cache.is_file() {
        return;
    }
    if let Ok(text) = std::fs::read_to_string(&cache) {
        let cached_ninja = text.contains("CMAKE_GENERATOR:INTERNAL=Ninja");
        let cached_clang = text.contains("clang-cl");
        let gen_changed = cached_ninja != use_clang;
        let tool_changed = cached_clang != use_clang;
        if gen_changed || tool_changed {
            eprintln!(
                "avif-native: toolchain changed, wiping CMake cache in {}",
                build_dir.display()
            );
            if let Err(e) = std::fs::remove_dir_all(build_dir) {
                eprintln!("avif-native: cache wipe failed: {e}");
            }
        }
    }
}

#[cfg(not(windows))]
fn wipe_if_toolchain_changed(_build_dir: &Path, _use_clang: bool) {}

fn build_cmake(
    src: &Path,
    out_dir: PathBuf,
    toolchain: Toolchain,
    define: impl FnOnce(&mut cmake::Config),
) -> PathBuf {
    let mut cfg = cmake::Config::new(src);
    cfg.profile("Release");
    cfg.out_dir(&out_dir);
    define(&mut cfg);
    apply_toolchain(&mut cfg, toolchain);
    cfg.always_configure(env::var_os("AVIF_LIBAVIF_REBUILD").is_none());
    wipe_if_toolchain_changed(&out_dir.join("build"), toolchain == Toolchain::Clang);
    cfg.build()
}

fn find_first_file(dst: &Path, names: &[&str]) -> Option<PathBuf> {
    for d in [dst.join("lib"), dst.join("lib64"), dst.to_path_buf()] {
        for name in names {
            let candidate = d.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Locates a clang-cl.exe: `AVIF_LIBAVIF_CLANG_CL` override, LLVM's standard
/// install location, or Visual Studio's bundled LLVM component.
#[cfg(windows)]
fn find_clang_cl() -> Option<PathBuf> {
    if let Some(p) = env::var_os("AVIF_LIBAVIF_CLANG_CL") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let pf: PathBuf = env::var_os("ProgramFiles")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));

    let llvm = pf.join("LLVM").join("bin").join("clang-cl.exe");
    if llvm.is_file() {
        return Some(llvm);
    }

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
