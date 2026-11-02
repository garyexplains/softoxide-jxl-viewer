//! GUI theme: dark by default (the historical look), light via the toolbar
//! toggle. The choice is persisted in a tiny settings file under the OS
//! config directory — the app's only non-image file I/O besides Windows
//! file-association registry writes.
//!
//! The egui chrome (toolbars, dialogs, status bar, canvases painted through
//! `ui.style()`) follows the theme automatically; image-anchored overlays
//! (pixel readout, crop mask) stay theme-independent on purpose — they must
//! stay readable on arbitrary images, not on the app chrome.

use std::path::PathBuf;

use eframe::egui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
}

impl Theme {
    pub fn is_dark(self) -> bool {
        matches!(self, Theme::Dark)
    }

    pub fn toggled(self) -> Theme {
        match self {
            Theme::Dark => Theme::Light,
            Theme::Light => Theme::Dark,
        }
    }

    /// The egui theme this maps to.
    pub fn egui(self) -> egui::Theme {
        match self {
            Theme::Dark => egui::Theme::Dark,
            Theme::Light => egui::Theme::Light,
        }
    }

    // ---- persistence --------------------------------------------------

    /// Loads the saved theme; the file's absence, unreadability, or unknown
    /// content all mean "keep the dark default".
    pub fn load() -> Theme {
        let Some(dir) = settings_dir() else {
            return Theme::Dark;
        };
        match std::fs::read_to_string(dir.join(FILE)) {
            Ok(text) => parse(&text),
            Err(_) => Theme::Dark,
        }
    }

    /// Persists the theme. Best-effort: failures keep the session choice but
    /// the choice simply won't survive a restart.
    pub fn save(self) {
        let Some(dir) = settings_dir() else {
            return;
        };
        let text = match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
        };
        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("theme: cannot create settings dir {}: {e}", dir.display());
            return;
        }
        if let Err(e) = std::fs::write(dir.join(FILE), text) {
            eprintln!(
                "theme: cannot write settings {}: {e}",
                dir.join(FILE).display()
            );
        }
    }
}

const FILE: &str = "theme.txt";

/// `"dark"`/anything unknown/blank → Dark; only `"light"` counts as Light.
fn parse(text: &str) -> Theme {
    match text.trim().to_ascii_lowercase().as_str() {
        "light" => Theme::Light,
        _ => Theme::Dark,
    }
}

fn settings_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("SoftOxide").join("JXL Viewer"))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|h| {
            PathBuf::from(h)
                .join("Library")
                .join("Application Support")
                .join("SoftOxide")
                .join("JXL Viewer")
        })
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            return Some(PathBuf::from(xdg).join("softoxide-jxl-viewer"));
        }
        std::env::var_os("HOME").map(|h| {
            PathBuf::from(h)
                .join(".config")
                .join("softoxide-jxl-viewer")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_uses_light_only_for_light() {
        assert_eq!(parse("dark"), Theme::Dark);
        assert_eq!(parse("  light\n"), Theme::Light);
        assert_eq!(parse("LIGHT"), Theme::Light);
        assert_eq!(parse(""), Theme::Dark);
        assert_eq!(parse("amoled"), Theme::Dark);
    }

    #[test]
    fn toggle_roundtrip() {
        assert_eq!(Theme::Dark.toggled(), Theme::Light);
        assert_eq!(Theme::Light.toggled(), Theme::Dark);
        assert_eq!(Theme::Light.toggled().toggled(), Theme::Light);
    }
}
