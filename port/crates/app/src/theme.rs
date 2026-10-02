//! The Mac app's dark appearance: colors, metrics and fonts from docs/port/ui-inventory.md.
//!
//! macOS draws everything in points; the port lays out in egui points, which are the same unit at
//! 1x. System colors are the dark-appearance values of AppKit's semantic colors.

use eframe::egui::{self, Color32, FontFamily, FontId};

pub const fn gray(white: f32) -> Color32 {
    let v = (white * 255.0 + 0.5) as u8;
    Color32::from_rgb(v, v, v)
}

/// White at `alpha`, premultiplied as egui expects.
pub fn white_alpha(alpha: f32) -> Color32 {
    Color32::from_white_alpha((alpha * 255.0 + 0.5) as u8)
}

pub fn black_alpha(alpha: f32) -> Color32 {
    Color32::from_black_alpha((alpha * 255.0 + 0.5) as u8)
}

/// Colors that don't depend on a view.
pub mod color {
    use super::*;

    /// `Color(white: 0.14)` behind the whole editor.
    pub const EDITOR: Color32 = gray(0.14);
    /// `windowBackgroundColor` in the dark appearance (#1E1E1E): what shows behind a panel or
    /// sheet that doesn't paint its own background.
    pub const WINDOW_BACKGROUND: Color32 = Color32::from_rgb(30, 30, 30);
    /// The canvas backdrop around the document.
    pub const CANVAS: Color32 = gray(0.105);
    /// The system accent: #007AFF, as the Mac harness records it (`accentColor` in ui-info.json).
    /// The Mac follows the user's setting; parity runs pin the default. Prominent buttons and
    /// selections draw it as is.
    pub const ACCENT: Color32 = Color32::from_rgb(0, 122, 255);
    /// The accent as macOS 26 draws it in a checkbox, a selected segment or a slider's fill,
    /// measured from the Mac's renders.
    pub const CONTROL_ACCENT: Color32 = Color32::from_rgb(6, 125, 255);
    /// `labelColor`, `secondaryLabelColor`, `tertiaryLabelColor`.
    pub fn label() -> Color32 {
        white_alpha(0.85)
    }
    pub fn secondary() -> Color32 {
        white_alpha(0.55)
    }
    pub fn tertiary() -> Color32 {
        white_alpha(0.25)
    }
    /// `separatorColor`: dividers.
    pub fn separator() -> Color32 {
        white_alpha(0.10)
    }
    /// A table's selection when the table isn't first responder
    /// (`unemphasizedSelectedContentBackgroundColor`).
    pub const UNEMPHASIZED_SELECTION: Color32 = Color32::from_rgb(70, 70, 70);
    /// Bordered buttons and pop-ups in macOS 26's dark appearance: white at 8%, measured.
    pub fn control() -> Color32 {
        white_alpha(0.08)
    }
    /// A checkbox's box, an unselected segmented track and a slider's empty track: white at
    /// about 9.6% and 7.8%, measured.
    pub fn control_well() -> Color32 {
        white_alpha(0.096)
    }
    pub fn segment_track() -> Color32 {
        white_alpha(0.078)
    }
    /// A slider's knob: an opaque light gray capsule.
    pub const KNOB: Color32 = Color32::from_rgb(222, 222, 222);
    pub fn control_pressed() -> Color32 {
        white_alpha(0.26)
    }
    /// A rounded-border text field: `textBackgroundColor` (#1E1E1E) inside a faint bezel.
    pub const FIELD: Color32 = Color32::from_rgb(30, 30, 30);
    pub fn field_border() -> Color32 {
        white_alpha(0.036)
    }
    pub const RED: Color32 = Color32::from_rgb(255, 69, 58);
    pub const ORANGE: Color32 = Color32::from_rgb(255, 159, 10);
}

/// Sizes from the inventory, in points.
pub mod metric {
    pub const WINDOW: [f32; 2] = [1180.0, 780.0];
    pub const WINDOW_MIN: [f32; 2] = [800.0, 520.0];
    pub const TOOLBAR: f32 = 38.0;
    pub const TOOL_HEADER: f32 = 42.0;
    pub const HEADER_PADDING: f32 = 18.0;
    pub const RAIL: f32 = 56.0;
    pub const STATUS: f32 = 30.0;
    pub const LAYERS_DEFAULT: f32 = 252.0;
    pub const LAYERS_MIN: f32 = 202.0;
    pub const LAYERS_MAX: f32 = 352.0;
    pub const RULER: f32 = 18.0;
    pub const FIT_MARGIN: f32 = 48.0;
    /// Buttons, pop-ups, segmented pickers and text fields at the regular control size in
    /// macOS 26 (a text field's bezel draws one point outside it).
    pub const CONTROL_HEIGHT: f32 = 24.0;
    pub const ROW: f32 = 52.0;
    pub const ROW_GAP: f32 = 2.0;
    pub const EFFECT_ROW: f32 = 24.0;
}

pub const SEMIBOLD: &str = "semibold";
pub const BOLD: &str = "bold";
pub const ICONS: &str = "icons";
pub const ICONS_FILL: &str = "icons-fill";

/// How much larger Segoe UI is set than the Mac's point size so that its text runs as wide as
/// SF Pro's. SF Pro is 6 to 11% wider than Segoe UI at the same size, more at small sizes where
/// SF Pro Text's tracking opens up (measured on the Mac's renders: 1.11 at 11 points, 1.09 at 12,
/// 1.06 to 1.08 at 13, 1.065 at 17). Matching widths keeps every control after a label where the
/// Mac puts it; the scaled x-height also comes close to SF Pro's.
pub fn width_scale(size: f32) -> f32 {
    (1.07 + 0.015 * (13.0 - size)).clamp(1.065, 1.12)
}

/// The Mac's point size for a font made by `regular`, `semibold`, `medium` or `bold`: line
/// heights and baselines follow SF Pro at that size.
pub fn nominal(size: f32) -> f32 {
    let mut n = size / 1.07;
    for _ in 0..8 {
        n = size / width_scale(n);
    }
    (n * 1000.0).round() / 1000.0
}

pub fn regular(size: f32) -> FontId {
    FontId::new(size * width_scale(size), FontFamily::Proportional)
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size * width_scale(size), FontFamily::Name(SEMIBOLD.into()))
}

/// SwiftUI's `.medium`; the Windows system font has no medium face, so it reads as regular.
pub fn medium(size: f32) -> FontId {
    regular(size)
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size * width_scale(size), FontFamily::Name(BOLD.into()))
}

/// SF Pro is the Mac's only UI font. Windows gets Segoe UI (the system face, the closest in
/// metrics); the Mac build uses SF Pro from the system. Missing files fall back to egui's font.
fn system_faces() -> [(&'static str, &'static [&'static str]); 3] {
    [
        ("regular", &["C:/Windows/Fonts/segoeui.ttf", "/System/Library/Fonts/SFNS.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"]),
        (SEMIBOLD, &["C:/Windows/Fonts/seguisb.ttf", "/System/Library/Fonts/SFNS.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"]),
        (BOLD, &["C:/Windows/Fonts/segoeuib.ttf", "/System/Library/Fonts/SFNS.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"]),
    ]
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let fallback: Vec<String> = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    // Symbols the text uses (arrows, ⌘ and friends) come from Segoe UI Symbol where the face lacks them.
    let mut extra = Vec::new();
    for path in ["C:/Windows/Fonts/seguisym.ttf", "/System/Library/Fonts/Apple Symbols.ttf"] {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert("symbols".into(), egui::FontData::from_owned(bytes).into());
            extra.push("symbols".to_string());
            break;
        }
    }
    for (name, paths) in system_faces() {
        let family = if name == "regular" { FontFamily::Proportional } else { FontFamily::Name(name.into()) };
        let mut keys = Vec::new();
        if let Some(bytes) = paths.iter().find_map(|p| std::fs::read(p).ok()) {
            let key = format!("system-{name}");
            fonts.font_data.insert(key.clone(), egui::FontData::from_owned(bytes).into());
            keys.push(key);
        }
        keys.extend(extra.iter().cloned());
        keys.extend(fallback.iter().cloned());
        fonts.families.insert(family, keys);
    }
    for (name, bytes) in [(ICONS, egui_phosphor::Variant::Regular.font_bytes()), (ICONS_FILL, egui_phosphor::Variant::Fill.font_bytes())] {
        fonts.font_data.insert(name.into(), egui::FontData::from_static(bytes).into());
        fonts.families.insert(FontFamily::Name(name.into()), vec![name.into()]);
    }
    ctx.set_fonts(fonts);
}

pub fn install_style(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.global_style_mut(|style| {
        let v = &mut style.visuals;
        v.dark_mode = true;
        v.panel_fill = color::EDITOR;
        v.window_fill = gray(0.17);
        v.extreme_bg_color = gray(0.12);
        v.override_text_color = Some(color::label());
        v.selection.bg_fill = color::ACCENT;
        v.selection.stroke = egui::Stroke::new(1.0, Color32::WHITE);
        v.hyperlink_color = color::ACCENT;
        v.window_stroke = egui::Stroke::new(1.0, white_alpha(0.12));
        v.menu_corner_radius = egui::CornerRadius::same(6);
        v.window_corner_radius = egui::CornerRadius::same(10);
        for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
            w.corner_radius = egui::CornerRadius::same(5);
            w.fg_stroke.color = color::label();
        }
        v.widgets.noninteractive.fg_stroke.color = color::label();
        v.widgets.noninteractive.bg_stroke.color = color::separator();
        v.widgets.inactive.weak_bg_fill = color::control();
        v.widgets.inactive.bg_fill = color::control();
        v.widgets.hovered.weak_bg_fill = white_alpha(0.2);
        v.widgets.active.weak_bg_fill = color::control_pressed();
        v.text_cursor.stroke = egui::Stroke::new(1.0, color::label());
        style.spacing.interact_size.y = metric::CONTROL_HEIGHT;
        style.spacing.button_padding = egui::vec2(10.0, 3.0);
        style.spacing.menu_margin = egui::Margin::symmetric(5, 5);
        style.text_styles = [
            (egui::TextStyle::Small, regular(10.0)),
            (egui::TextStyle::Body, regular(13.0)),
            (egui::TextStyle::Button, regular(13.0)),
            (egui::TextStyle::Heading, bold(17.0)),
            (egui::TextStyle::Monospace, FontId::monospace(12.0)),
        ]
        .into();
        style.interaction.tooltip_delay = 0.6;
    });
}
