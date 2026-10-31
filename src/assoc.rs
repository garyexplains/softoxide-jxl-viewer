//! Windows per-user file-type associations.
//!
//! On Windows, double-click handling is registry-based, so registration is
//! implemented with plain `HKCU` keys written for the current user only —
//! no administrator rights and nothing outside our own keys is touched:
//!
//! ```text
//! Software\Classes\SoftOxideJXLViewer.<ext>         ProgID tree:
//!   (default)          "SoftOxide JXL Viewer (<description>)"
//!   shell\open\command "<path to jxlviewer.exe>" "%1"
//!   DefaultIcon        "<path to jxlviewer.exe>",0
//! Software\Classes\.<ext>\OpenWithProgids\<progid>  value (always) so the
//!                                                   viewer shows up in
//!                                                   Explorer's Open with…
//! Software\Classes\.<ext>\(default) = <progid>      only when the extension
//!                                                   had no other handler
//! Software\Classes\Applications\<viewer.exe>        exe-level registration
//!   (default), shell\open\command,                  (Open-with lists),
//!   SupportedTypes
//! ```
//!
//! Windows 10/11 protect the actual double-click default (the UserChoice
//! association) with an OS-managed hash, so this module deliberately
//! registers the association and lets Explorer's Open-with / Settings →
//! Default apps be the last word. The extension default is still set when
//! the type had no other handler (the usual case for `.jxl`).
//!
//! Non-Windows targets get no-op stubs; the GUI hides the dialog there.

use std::path::{Path, PathBuf};

/// One registrable file type. Everything listed here is a format the viewer
/// can actually open (see `codec`).
pub struct AssocFormat {
    /// Extension without the leading dot, e.g. `jxl`.
    pub ext: &'static str,
    /// Human description, e.g. `JPEG XL image`.
    pub desc: &'static str,
}

pub const FORMATS: &[AssocFormat] = &[
    AssocFormat {
        ext: "jxl",
        desc: "JPEG XL image",
    },
    AssocFormat {
        ext: "png",
        desc: "PNG image",
    },
    AssocFormat {
        ext: "jpg",
        desc: "JPEG image",
    },
    AssocFormat {
        ext: "jpeg",
        desc: "JPEG image",
    },
    AssocFormat {
        ext: "gif",
        desc: "GIF image",
    },
    AssocFormat {
        ext: "bmp",
        desc: "BMP image",
    },
    AssocFormat {
        ext: "tif",
        desc: "TIFF image",
    },
    AssocFormat {
        ext: "tiff",
        desc: "TIFF image",
    },
    AssocFormat {
        ext: "webp",
        desc: "WebP image",
    },
    AssocFormat {
        ext: "pnm",
        desc: "PNM image",
    },
    AssocFormat {
        ext: "ppm",
        desc: "PPM image",
    },
    AssocFormat {
        ext: "pgm",
        desc: "PGM image",
    },
    AssocFormat {
        ext: "pbm",
        desc: "PBM image",
    },
];

/// The extension registered by default (CLI `register` without arguments and
/// the first row of the GUI dialog).
pub const DEFAULT_EXT: &str = "jxl";

/// Looks a format up by extension; case-insensitive, leading dots allowed.
pub fn format_for(ext: &str) -> Option<&'static AssocFormat> {
    let probe = ext.trim_start_matches('.').to_ascii_lowercase();
    FORMATS.iter().find(|f| f.ext == probe)
}

const PROGID_BASE: &str = "SoftOxideJXLViewer";

fn normalize(ext: &str) -> String {
    ext.trim_start_matches('.').to_ascii_lowercase()
}

fn progid_for(ext: &str) -> String {
    format!("{PROGID_BASE}.{ext}")
}

/// Whether the viewer is registered for this extension (its ProgID tree
/// exists in the registry).
#[allow(unused_variables)]
pub fn is_registered(ext: &str) -> bool {
    let ext = normalize(ext);
    #[cfg(windows)]
    if !valid_ext(&ext) {
        return false;
    }
    #[cfg(windows)]
    return imp::is_registered(&ext);
    #[cfg(not(windows))]
    false
}

/// Lists every supported extension the viewer is registered for.
pub fn registered_extensions() -> Vec<&'static str> {
    FORMATS
        .iter()
        .filter(|f| is_registered(f.ext))
        .map(|f| f.ext)
        .collect()
}

/// Registers the viewer for one extension. Returns a short human explanation
/// of the achieved level (default handler vs Open-with only).
#[allow(unused_variables)]
pub fn register(ext: &str) -> Result<String, String> {
    let ext = normalize(ext);
    #[cfg(windows)]
    if valid_ext(&ext) {
        return imp::register(&ext);
    }
    Err(format!("unsupported extension '{ext}'"))
}

/// Removes the viewer's registration for one extension. Only keys and values
/// this module created are deleted (an extension's other handlers stay).
#[allow(unused_variables)]
pub fn unregister(ext: &str) -> Result<String, String> {
    let ext = normalize(ext);
    #[cfg(windows)]
    if valid_ext(&ext) {
        return imp::unregister(&ext);
    }
    Err(format!("unsupported extension '{ext}'"))
}

#[allow(dead_code)] // shared by platform-gated callers
fn valid_ext(ext: &str) -> bool {
    !ext.is_empty() && ext.len() <= 12 && ext.chars().all(|c| c.is_ascii_alphanumeric())
}

/// The executable that file managers will launch, display-friendly (the
/// `\\?\` prefix that `canonicalize` adds on Windows is stripped).
pub fn viewer_exe() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate own exe: {e}"))?;
    let exe = display_path(exe);
    let s = exe.to_string_lossy().into_owned();
    if s.is_empty() {
        Err("cannot locate own exe".into())
    } else {
        Ok(s)
    }
}

fn display_path(p: PathBuf) -> PathBuf {
    let s = p.as_os_str().to_string_lossy().into_owned();
    match s.strip_prefix(r"\\?\") {
        Some(stripped) => PathBuf::from(stripped),
        None => PathBuf::from(s),
    }
}

fn classes_ext_key(ext: &str) -> String {
    format!("Software\\Classes\\.{ext}")
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_MULTI_SZ,
        REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW,
        RegDeleteTreeW, RegDeleteValueW, RegGetValueW, RegOpenKeyExW, RegSetValueExW,
    };

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    struct OwnedKey(HKEY);

    impl OwnedKey {
        fn create(path: &str) -> Result<Self, String> {
            let mut hk: HKEY = std::ptr::null_mut();
            // SAFETY: path outlives the call; the handle is closed in Drop.
            let st = unsafe {
                RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    to_wide(path).as_ptr(),
                    0,
                    std::ptr::null(),
                    REG_OPTION_NON_VOLATILE,
                    KEY_SET_VALUE,
                    std::ptr::null(),
                    &mut hk,
                    std::ptr::null_mut(),
                )
            };
            if st != ERROR_SUCCESS {
                return Err(format!(
                    "registry create '{path}' failed (Win32 error {st})"
                ));
            }
            Ok(Self(hk))
        }

        fn open(path: &str, access: u32) -> Option<Self> {
            let mut hk: HKEY = std::ptr::null_mut();
            // SAFETY: as above.
            let st = unsafe {
                RegOpenKeyExW(
                    HKEY_CURRENT_USER,
                    to_wide(path).as_ptr(),
                    0,
                    access,
                    &mut hk,
                )
            };
            if st == ERROR_SUCCESS {
                Some(Self(hk))
            } else {
                None
            }
        }
    }

    impl Drop for OwnedKey {
        fn drop(&mut self) {
            // SAFETY: handle was created/opened above.
            unsafe { RegCloseKey(self.0) };
        }
    }

    fn set_sz(hk: HKEY, name: Option<&str>, value: &str) -> Result<(), String> {
        let name_w = name.map(to_wide);
        let name_ptr = name_w
            .as_ref()
            .map(|n| n.as_ptr())
            .unwrap_or(std::ptr::null());
        let mut data = to_wide(value);
        // SAFETY: value bytes include the trailing nul REG_SZ requires.
        let st = unsafe {
            RegSetValueExW(
                hk,
                name_ptr,
                0,
                REG_SZ,
                data.as_mut_ptr() as *const u8,
                (data.len() * 2) as u32,
            )
        };
        if st != ERROR_SUCCESS {
            return Err(format!("registry set value failed (Win32 error {st})"));
        }
        Ok(())
    }

    fn set_multi_sz(hk: HKEY, name: Option<&str>, values: &[String]) -> Result<(), String> {
        let name_w = name.map(to_wide);
        let name_ptr = name_w
            .as_ref()
            .map(|n| n.as_ptr())
            .unwrap_or(std::ptr::null());
        let mut data: Vec<u16> = Vec::new();
        for v in values {
            data.extend(to_wide(v));
            data.pop();
        }
        data.push(0);
        // SAFETY: multi-sz encoding is string-list + double nul.
        let st = unsafe {
            RegSetValueExW(
                hk,
                name_ptr,
                0,
                REG_MULTI_SZ,
                data.as_mut_ptr() as *const u8,
                (data.len() * 2) as u32,
            )
        };
        if st != ERROR_SUCCESS {
            return Err(format!("registry set multi-sz failed (Win32 error {st})"));
        }
        Ok(())
    }

    fn delete_tree(path: &str) -> Result<(), String> {
        // SAFETY: deletes only the Software\Classes tree this module created.
        let st = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, to_wide(path).as_ptr()) };
        if st != ERROR_SUCCESS {
            return Err(format!(
                "registry delete '{path}' failed (Win32 error {st})"
            ));
        }
        Ok(())
    }

    fn delete_value(path: &str, name: Option<&str>) -> Result<(), String> {
        let Some(key) = OwnedKey::open(path, KEY_SET_VALUE) else {
            return Ok(()); // nothing to delete
        };
        let name_w = name.map(to_wide);
        let name_ptr = name_w
            .as_ref()
            .map(|n| n.as_ptr())
            .unwrap_or(std::ptr::null());
        // SAFETY: deletes the named value under our own key.
        let st = unsafe { RegDeleteValueW(key.0, name_ptr) };
        if st != ERROR_SUCCESS {
            return Err(format!("registry delete value failed (Win32 error {st})"));
        }
        Ok(())
    }

    /// Reads `HKCU\Software\Classes\.<ext>`'s default (unnamed) REG_SZ value.
    fn read_extension_default(ext: &str) -> Option<String> {
        let mut buf = [0u16; 512];
        let mut len: u32 = (buf.len() * 2) as u32;
        let path = to_wide(&classes_ext_key(ext));
        // SAFETY: output buffer is fixed-size; length is in/out bytes.
        let st = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                path.as_ptr(),
                std::ptr::null::<u16>(), // default value
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr() as *mut core::ffi::c_void,
                &mut len,
            )
        };
        if st != ERROR_SUCCESS {
            return None;
        }
        let n = (len as usize) / 2;
        Some(
            String::from_utf16_lossy(&buf[..n])
                .trim_end_matches('\0')
                .to_string(),
        )
    }

    pub fn is_registered(ext: &str) -> bool {
        OwnedKey::open(
            &format!("Software\\Classes\\{}", progid_for(ext)),
            KEY_QUERY_VALUE,
        )
        .is_some()
    }

    pub fn register(ext: &str) -> Result<String, String> {
        let exe = viewer_exe()?;
        let progid = progid_for(ext);
        let desc = format_for(ext).map(|f| f.desc).unwrap_or("image file");

        // 1) ProgID tree: description, open command, icon.
        {
            let p = format!("Software\\Classes\\{progid}");
            let hk = OwnedKey::create(&p)?;
            set_sz(hk.0, None, &format!("SoftOxide JXL Viewer ({desc})"))?;
            let cmd = OwnedKey::create(&format!("{p}\\shell\\open\\command"))?;
            set_sz(cmd.0, None, &format!("\"{exe}\" \"%1\""))?;
            let icon = OwnedKey::create(&format!("{p}\\DefaultIcon"))?;
            set_sz(icon.0, None, &format!("\"{exe}\",0"))?;
        }

        // 2) Open-with entry for the extension (always).
        {
            let hk = OwnedKey::create(&format!("{}\\OpenWithProgids", classes_ext_key(ext)))?;
            set_sz(hk.0, Some(&progid), "")?;
        }

        // 3) Extension default handler: only when the type is unclaimed (or
        //    already ours). Otherwise the other handler stays and the viewer
        //    is reachable via its Open-with entry.
        let existing = read_extension_default(ext);
        let default_msg = match existing.as_deref().map(|v| v.trim()) {
            Some(v) if v.eq_ignore_ascii_case(&progid) => "already the default handler".to_string(),
            Some(v) if !v.is_empty() => {
                format!("added to Open with… (existing default handler kept: {v})")
            }
            _ => {
                let hk = OwnedKey::create(&classes_ext_key(ext))?;
                set_sz(hk.0, None, &progid)?;
                "set as the default handler".to_string()
            }
        };

        // 4) Exe-level registration so the viewer shows up in Open-with lists
        //    regardless of individual defaults.
        {
            let exe_name = Path::new(&exe)
                .file_name()
                .ok_or_else(|| "bad exe path".to_string())?
                .to_string_lossy()
                .into_owned();
            let app_path = format!("Software\\Classes\\Applications\\{exe_name}");
            let hk = OwnedKey::create(&app_path)?;
            set_sz(hk.0, None, "SoftOxide JXL Viewer")?;
            // Explicit friendly name: some Open-with views read this value
            // instead of the exe's version resource.
            set_sz(hk.0, Some("FriendlyAppName"), "SoftOxide JXL Viewer")?;
            let cmd = OwnedKey::create(&format!("{app_path}\\shell\\open\\command"))?;
            set_sz(cmd.0, None, &format!("\"{exe}\" \"%1\""))?;
            let supported: Vec<String> = FORMATS.iter().map(|f| format!(".{}", f.ext)).collect();
            set_multi_sz(hk.0, Some("SupportedTypes"), &supported)?;
        }

        Ok(format!("associated: {default_msg}"))
    }

    pub fn unregister(ext: &str) -> Result<String, String> {
        let progid = progid_for(ext);
        delete_tree(&format!("Software\\Classes\\{progid}"))?;
        delete_value(
            &format!("{}\\OpenWithProgids", classes_ext_key(ext)),
            Some(&progid),
        )
        .unwrap_or_else(|e| {
            eprintln!("warning: {e}");
        });
        // Remove the default only if it still points at our ProgID.
        if read_extension_default(ext)
            .as_deref()
            .map(|v| v.eq_ignore_ascii_case(&progid))
            == Some(true)
        {
            delete_value(&classes_ext_key(ext), None)?;
        }
        if is_registered(ext) {
            Err(format!("association for {ext} could not be fully removed"))
        } else {
            Ok(format!("association for {ext} removed"))
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn is_registered(_ext: &str) -> bool {
        false
    }

    pub fn register(_ext: &str) -> Result<String, String> {
        Err("file-association registration is a Windows-only feature".into())
    }

    pub fn unregister(_ext: &str) -> Result<String, String> {
        Err("file-association registration is a Windows-only feature".into())
    }
}
