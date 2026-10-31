//! Headless CLI: `info`, `convert`, `register`/`unregister` (Windows), help text.

use std::path::{Path, PathBuf};

use crate::codec;

pub struct ConvertOptions {
    pub quality: Option<u8>,
    pub lossless: bool,
    pub distance: f32,
    pub effort: i64,
    pub keep_alpha: Option<bool>,
    pub modular: bool,
    pub group_size: Option<jxl_native::GroupSize>,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            quality: None,
            lossless: false,
            distance: 1.0,
            effort: 3,
            keep_alpha: None,
            modular: false,
            group_size: None,
        }
    }
}

pub const CONVERT_HELP: &str = "convert usage:\n  jxlviewer convert IN OUT [--lossless] [--distance D] [--effort N]\n\
     \x20            [--modular] [--group-size N] [--keep-alpha] [--drop-alpha] [--quality Q]\n\n\
 Examples:\n  jxlviewer convert sample.jxl out.png\n  \
 jxlviewer convert sample.png out.jxl --lossless --group-size 256 --modular";

pub const MAIN_HELP: &str = "jxlviewer — SoftOxide JXL Viewer (native JPEG XL viewer & converter, all rendering in-app)\n\n\
 Usage:\n  jxlviewer [FILE]              open the GUI (optionally with FILE)\n\
   jxlviewer info FILE…          print file metadata\n\
   jxlviewer convert IN OUT …    headless conversion\n\
   jxlviewer register [EXTS…]    register Windows file associations (default: jxl;\n\
     \x20                           \"--all\" for every supported type; Windows only)\n\
   jxlviewer unregister [EXTS…]  remove Windows file associations\n\
   jxlviewer registered          list extensions currently registered (Windows only)\n\
   jxlviewer --version\n";

pub fn run(args: Vec<String>) -> i32 {
    match args.first().map(|s| s.as_str()) {
        None => {
            crate::run_gui(None);
            0
        }
        Some("info") if args.len() > 1 => {
            let mut code = 0;
            for f in &args[1..] {
                match info_cmd(Path::new(f)) {
                    Ok(text) => println!("{text}"),
                    Err(e) => {
                        eprintln!("error: {e}");
                        code = 1;
                    }
                }
            }
            code
        }
        Some("convert") => match parse_convert_args(&args[1..]) {
            Ok((input, output, opts)) => match convert_one(&input, &output, &opts) {
                Ok(summary) => {
                    println!("{summary}");
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            },
            Err(e) => {
                eprintln!("error: {e}\n\n{CONVERT_HELP}");
                2
            }
        },
        Some("help" | "-h" | "--help") => {
            print!("{MAIN_HELP}\n\n{CONVERT_HELP}\n");
            0
        }
        Some("register") => assoc_cli("register", &args[1..]),
        Some("unregister") => assoc_cli("unregister", &args[1..]),
        Some("registered") => {
            if !args[1..].is_empty() {
                eprintln!("error: 'registered' takes no arguments\n{MAIN_HELP}");
                2
            } else {
                let registered = crate::assoc::registered_extensions();
                if registered.is_empty() {
                    println!("no file associations registered");
                } else {
                    for ext in registered {
                        let desc = crate::assoc::format_for(ext).map(|f| f.desc).unwrap_or("");
                        println!(".{ext}  SoftOxide JXL Viewer ({desc})");
                    }
                }
                0
            }
        }
        Some("--version" | "-V") => {
            println!("jxlviewer {}", env!("CARGO_PKG_VERSION"));
            0
        }
        // A single non-flag argument opens the GUI with that file.
        Some(first) if args.len() == 1 && !first.starts_with('-') => {
            crate::run_gui(Some(PathBuf::from(first)));
            0
        }
        Some(_) => {
            eprintln!("unrecognized arguments ({args:?})\n{MAIN_HELP}");
            2
        }
    }
}

/// Windows file-association commands: `register`/`unregister [EXTS…|--all]`.
/// Without arguments, registers/removes the viewer's primary type (`.jxl`).
fn assoc_cli(op: &str, args: &[String]) -> i32 {
    let valid: Vec<String> = crate::assoc::FORMATS
        .iter()
        .map(|f| f.ext.to_string())
        .collect();
    let all = args.iter().any(|a| a == "--all");
    if all && args.iter().any(|a| a != "--all") {
        eprintln!("error: \"--all\" cannot be combined with explicit extensions");
        return 2;
    }
    let mut exts: Vec<String> = Vec::new();
    if all {
        exts.extend(valid.iter().cloned());
    } else {
        for a in args {
            let ext = a.trim_start_matches('.').to_ascii_lowercase();
            if !valid.iter().any(|v| *v == ext) {
                eprintln!(
                    "error: unknown file type '{a}'\nsupported types: {} (or \"--all\")",
                    valid.join(", ")
                );
                return 2;
            }
            if !exts.contains(&ext) {
                exts.push(ext);
            }
        }
        if exts.is_empty() {
            exts.push(crate::assoc::DEFAULT_EXT.to_string());
        }
    }

    let mut code = 0;
    for ext in &exts {
        let result = if op == "register" {
            crate::assoc::register(ext)
        } else {
            crate::assoc::unregister(ext)
        };
        match result {
            Ok(msg) => println!(".{ext}: {msg}"),
            Err(e) => {
                eprintln!("error: .{ext}: {e}");
                code = 1;
            }
        }
    }
    code
}

pub fn info_cmd(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let ext = path.extension().and_then(|e| e.to_str());
    codec::info_from_bytes(&bytes, ext)
}

pub fn convert_one(input: &Path, output: &Path, opts: &ConvertOptions) -> Result<String, String> {
    let img = codec::decode_file(input)?;
    let raster = img.raster;

    let out_ext = output
        .extension()
        .and_then(|e| e.to_str())
        .ok_or("output path lacks an extension (e.g. out.png)")?
        .to_ascii_lowercase();

    let save_opts: codec::SaveOptions = match out_ext.as_str() {
        "jxl" => {
            let mut jxs = codec::JxlSaveOptions {
                lossless: opts.lossless,
                distance: opts.distance,
                effort: opts.effort,
                modular: opts.modular,
                group_size: opts.group_size,
                keep_alpha: opts.keep_alpha.unwrap_or(true),
            };
            if jxs.lossless {
                jxs.distance = 0.0;
            }
            codec::SaveOptions::Jxl(jxs)
        }
        "png" => codec::SaveOptions::Png,
        "jpg" | "jpeg" => {
            let mut j = codec::JpegSaveOptions::default();
            if let Some(q) = opts.quality {
                j.quality = q;
            }
            codec::SaveOptions::Jpeg(j)
        }
        "bmp" => codec::SaveOptions::Bmp,
        "tif" | "tiff" => codec::SaveOptions::Tiff,
        "gif" => codec::SaveOptions::Gif,
        other => return Err(format!("conversion to .{other} is not supported")),
    };

    // Animated inputs save as animated GIF or animated JXL.
    if matches!(
        save_opts,
        codec::SaveOptions::Gif | codec::SaveOptions::Jxl(_)
    ) {
        if let Some(anim) = &img.animation {
            codec::save_animation_to_path(
                output,
                &anim.frames,
                &anim.durations_ms,
                anim.loops,
                &save_opts,
            )?;
            let size = std::fs::metadata(output).map(|m| m.len()).unwrap_or(0);
            return Ok(format!(
                "converted {} ({} {}×{}, {} frames) → {} ({} bytes, animated)",
                input.display(),
                img.format.as_str(),
                raster.width,
                raster.height,
                anim.frame_count(),
                output.display(),
                size
            ));
        }
    }
    codec::save_to_path(output, &raster, &save_opts)?;
    let size = std::fs::metadata(output).map(|m| m.len()).unwrap_or(0);
    Ok(format!(
        "converted {} ({} {}×{}) → {} ({} bytes)",
        input.display(),
        img.format.as_str(),
        raster.width,
        raster.height,
        output.display(),
        size
    ))
}

pub fn parse_convert_args(args: &[String]) -> Result<(PathBuf, PathBuf, ConvertOptions), String> {
    if args.len() < 2 {
        return Err("expected IN and OUT paths".into());
    }
    let mut opts = ConvertOptions::default();
    let mut i = 2;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "--lossless" => opts.lossless = true,
            "--modular" => opts.modular = true,
            "--distance" => {
                i += 1;
                let v = args.get(i).ok_or("--distance needs a value")?;
                opts.distance = v
                    .parse()
                    .map_err(|_| "--distance wants a float".to_string())?;
            }
            "--effort" => {
                i += 1;
                let v = args.get(i).ok_or("--effort needs a value")?;
                opts.effort = v
                    .parse()
                    .map_err(|_| "--effort wants an integer".to_string())?;
            }
            "--group-size" => {
                i += 1;
                let v = args.get(i).ok_or("--group-size needs a value")?;
                opts.group_size = match v.as_str() {
                    "128" => Some(jxl_native::GroupSize::G128),
                    "256" => Some(jxl_native::GroupSize::G256),
                    "512" => Some(jxl_native::GroupSize::G512),
                    "1024" => Some(jxl_native::GroupSize::G1024),
                    "default" | "-1" => Some(jxl_native::GroupSize::Default),
                    other => return Err(format!("--group-size: unknown value {other}")),
                };
            }
            "--keep-alpha" => opts.keep_alpha = Some(true),
            "--drop-alpha" => opts.keep_alpha = Some(false),
            "--quality" => {
                i += 1;
                let v = args.get(i).ok_or("--quality needs a value")?;
                opts.quality = Some(
                    v.parse()
                        .map_err(|_| "--quality wants a number".to_string())?,
                );
            }
            other => return Err(format!("unknown option {other}")),
        }
        i += 1;
    }
    Ok((PathBuf::from(&args[0]), PathBuf::from(&args[1]), opts))
}
