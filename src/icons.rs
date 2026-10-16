//! Toolbar/UI icons.
//!
//! We vendor the Phosphor Icons regular TTF (MIT-licensed glyphs + OFL font)
//! at `assets/fonts/Phosphor.ttf` and register it as a fallback font for
//! egui's proportional text: Latin text keeps rendering from the primary
//! font, while icon codepoints (private-use area) resolve from Phosphor.
//! No OS fonts are involved.

pub const FONT_NAME: &str = "phosphor";

// The handful of icons used by the app (codepoints from Phosphor v2.1).
pub const FOLDER_OPEN: &str = "\u{E256}";
pub const ARROW_COUNTER_CLOCKWISE: &str = "\u{E038}";
pub const ARROW_CLOCKWISE: &str = "\u{E036}";
pub const HAND_PALM: &str = "\u{E57E}";
pub const CROP: &str = "\u{E1D4}";
pub const FLIP_HORIZONTAL: &str = "\u{ED6A}";
pub const FLIP_VERTICAL: &str = "\u{ED6C}";
pub const ARROWS_OUT_CARDINAL: &str = "\u{E0A4}";
pub const FLOPPY_DISK: &str = "\u{E248}";
pub const MAGNIFYING_GLASS_PLUS: &str = "\u{E310}";
pub const MAGNIFYING_GLASS_MINUS: &str = "\u{E30E}";
pub const FRAME_CORNERS: &str = "\u{E626}";
pub const IMAGE: &str = "\u{E2CA}";
pub const WARNING_CIRCLE: &str = "\u{E4E2}";
pub const CHECK_CIRCLE: &str = "\u{E184}";
pub const X_CIRCLE: &str = "\u{E4F8}";
pub const ARROWS_HORIZONTAL: &str = "\u{EB06}";
pub const ARROWS_VERTICAL: &str = "\u{EB04}";

/// Registers the vendored Phosphor font as a proportional-text fallback.
pub fn add_to_fonts(fonts: &mut eframe::egui::FontDefinitions) {
    use eframe::egui::FontData;
    fonts.font_data.insert(
        FONT_NAME.into(),
        FontData::from_static(include_bytes!("../assets/fonts/Phosphor.ttf")).into(),
    );

    // Second in the chain: does not shadow the primary text font, but gets
    // consulted for glyphs (icons) that the text font lacks.
    if let Some(keys) = fonts
        .families
        .get_mut(&eframe::egui::FontFamily::Proportional)
    {
        let at = keys.len().min(1);
        keys.insert(at, FONT_NAME.into());
    }
}
