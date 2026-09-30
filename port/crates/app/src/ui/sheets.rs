//! The Mac's sheet layout: the pieces every sheet is built from (text on SF Pro's line heights,
//! HStacks of 24-point controls, pickers, radio groups, colored sliders, the Levels and color
//! picker parts), used by the live sheets in `dialogs.rs`, and the sheets `--render-ui` draws
//! that the port has no live version of yet: Grid Settings, and the New canvas form alone.

use crate::app::App;
use crate::icons::{self, Icon};
use crate::theme::{self, color, white_alpha};
use crate::widgets::{self as w, ButtonStyle, SwatchStyle};
use eframe::egui::{self, Align, Color32, Layout, Mesh, Rect, Sense, Stroke, Ui, pos2, vec2};

/// Grid Settings' values, at the Mac's defaults.
pub struct SheetState {
    grid_color: usize,
    grid_style: usize,
    grid_opacity: f64,
    grid_spacing: f64,
    grid_subdivisions: f64,
}

impl SheetState {
    pub fn new(_app: &App) -> Self {
        Self { grid_color: 0, grid_style: 0, grid_opacity: 45.0, grid_spacing: 64.0, grid_subdivisions: 8.0 }
    }
}

/// The content width of each sheet the port draws, or `None` for one it doesn't yet.
pub fn width(sheet: &str) -> Option<f32> {
    Some(match sheet {
        // Alone, the form takes its fitting width, which its text fields' ideal width sets;
        // on the empty canvas it grows to its 500-point maximum.
        "new-canvas" => 411.0,
        "grid-settings" => 360.0,
        _ => return None,
    })
}

/// Draws `sheet` into `rect` (its width from `width`); returns the height its content needs.
pub fn draw(app: &mut App, st: &mut SheetState, ui: &mut Ui, rect: Rect, sheet: &str) -> f32 {
    if sheet == "new-canvas" {
        super::welcome(app, ui, rect);
        return super::welcome_height();
    }
    // Grid Settings: `.padding(24)`, a VStack 18 apart.
    let (padding, spacing) = (24.0, 18.0);
    let inner = rect.shrink(padding);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
    // No horizontal spacing here: egui widens a vertical layout by it after each full-width row.
    child.spacing_mut().item_spacing = vec2(0.0, spacing);
    let ui = &mut child;
    if sheet == "grid-settings" {
        grid_settings(st, ui);
    }
    ui.min_rect().height() + 2.0 * padding
}

// Pieces shared by the sheets.

pub(crate) fn title2(ui: &mut Ui, s: &str) {
    w::text(ui, s, theme::bold(17.0), color::label());
}

pub(crate) fn headline(ui: &mut Ui, s: &str) {
    w::text(ui, s, theme::semibold(13.0), color::label());
}

pub(crate) fn body(ui: &mut Ui, s: &str, c: Color32) {
    w::text(ui, s, theme::regular(13.0), c);
}

/// Wrapping text across the sheet's width.
pub(crate) fn para(ui: &mut Ui, s: &str, size: f32, c: Color32) {
    w::wrapped(ui, s, theme::regular(size), c);
}

pub(crate) fn caption(ui: &mut Ui, s: &str) {
    w::text(ui, s, theme::regular(10.0), color::secondary());
}

pub(crate) fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, color::separator());
}

/// An HStack across the sheet, controls centered on a 24-point line.
pub(crate) fn hstack<R>(ui: &mut Ui, spacing: f32, content: impl FnOnce(&mut Ui) -> R) -> R {
    hstack_height(ui, 24.0, spacing, content)
}

/// An HStack `height` points tall.
pub(crate) fn hstack_height<R>(ui: &mut Ui, height: f32, spacing: f32, content: impl FnOnce(&mut Ui) -> R) -> R {
    ui.allocate_ui_with_layout(vec2(ui.available_width(), height), Layout::left_to_right(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = spacing;
        content(ui)
    })
    .inner
}

pub(crate) fn trailing(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    ui.with_layout(Layout::right_to_left(Align::Center), content);
}

/// A label of a fixed width, leading-aligned (`.frame(width:alignment: .leading)`).
pub(crate) fn fixed_label(ui: &mut Ui, s: &str, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, w::line_height(13.0)), Sense::hover());
    w::paint_centered(ui.painter(), s, theme::regular(13.0), color::label(), rect.min.x, rect.center().y);
}

pub(crate) fn text_width(ui: &Ui, s: &str) -> f32 {
    ui.painter().layout_no_wrap(s.into(), theme::regular(13.0), color::label()).size().x
}

// The sheets.

pub(crate) fn bytes(n: i64) -> String {
    // `ByteCountFormatter` with `.memory`: powers of 1024, whole kilobytes, one decimal above.
    let n = n as f64;
    if n < 1024.0 {
        format!("{} bytes", n as i64)
    } else if n < 1024.0 * 1024.0 {
        format!("{} KB", (n / 1024.0).round() as i64)
    } else if n < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} MB", n / 1024.0 / 1024.0)
    } else {
        format!("{:.2} GB", n / 1024.0 / 1024.0 / 1024.0)
    }
}

pub(crate) const UNITS: &[&str] = &["Pixels", "Percent", "Inches", "Centimeters"];
pub(crate) const ANCHORS: [&str; 9] = ["Top left", "Top center", "Top right", "Middle left", "Center", "Middle right", "Bottom left", "Bottom center", "Bottom right"];

/// An AppKit radio button.
pub(crate) fn radio(ui: &mut Ui, on: bool, title: &str) -> bool {
    let font = w::control_font(ui);
    let galley = ui.painter().layout_no_wrap(title.to_string(), font, color::label());
    let (rect, response) = ui.allocate_exact_size(vec2(16.0 + 6.0 + galley.size().x, 16.0), Sense::click());
    // macOS 26: a 16-point well, or the accent with a white dot.
    let c = pos2(rect.min.x + 8.0, rect.center().y.round());
    if on {
        ui.painter().circle_filled(c, 8.0, color::CONTROL_ACCENT);
        ui.painter().circle_filled(c, 3.0, Color32::WHITE);
    } else {
        ui.painter().circle_filled(c, 8.0, color::control_well());
    }
    w::paint_centered(ui.painter(), title, w::control_font(ui), color::label(), rect.min.x + 22.0, c.y);
    response.clicked()
}

/// `GridSettingsSheet`.
fn grid_settings(st: &mut SheetState, ui: &mut Ui) {
    const PRESETS: &[&str] = &["Light Gray", "Light Blue", "Light Red", "Green", "Medium Blue", "Yellow", "Magenta", "Cyan", "Black", "Custom"];
    const PRESET_COLORS: [[f32; 3]; 9] = [
        [0.7, 0.7, 0.7],
        [0.45, 0.66, 1.0],
        [1.0, 0.45, 0.45],
        [0.25, 0.8, 0.35],
        [0.25, 0.4, 0.95],
        [1.0, 0.85, 0.2],
        [1.0, 0.3, 1.0],
        [0.3, 0.95, 1.0],
        [0.0, 0.0, 0.0],
    ];
    title2(ui, "Grid");
    hstack(ui, 8.0, |ui| {
        fixed_label(ui, "Color", 110.0);
        w::popup(ui, "grid-color", &mut st.grid_color, &[&(0..PRESETS.len()).collect::<Vec<_>>()], |i| PRESETS[i], None, true);
        let c = PRESET_COLORS[st.grid_color.min(8)];
        w::swatch(ui, w::rgb(c), vec2(34.0, 18.0), SwatchStyle { radius: 4.0, inner_white: 1.0, outer_black: 1.0 });
    });
    hstack(ui, 8.0, |ui| {
        fixed_label(ui, "Style", 110.0);
        w::popup(ui, "grid-style", &mut st.grid_style, &[&[0usize, 1, 2]], |i| ["Lines", "Dashed Lines", "Dots"][i], None, true);
    });
    hstack(ui, 8.0, |ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(110.0, 22.0), Sense::hover());
        ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, "Opacity", theme::regular(13.0), color::label());
        let wd = w::fill_width(ui, 48.0 + w::unit_width(ui, "%"), 2);
        w::slider(ui, &mut st.grid_opacity, 1.0..=100.0, wd, true);
        w::number_field(ui, "grid-opacity", &mut st.grid_opacity, 1.0..=100.0, w::fmt_int, 48.0, true, true);
        w::unit(ui, "%");
    });
    divider(ui);
    hstack(ui, 8.0, |ui| {
        fixed_label(ui, "Gridline every", 110.0);
        let wd = w::fill_width(ui, text_width(ui, "pixels"), 1);
        w::number_field(ui, "grid-spacing", &mut st.grid_spacing, 2.0..=4096.0, w::fmt_int, wd, false, true);
        body(ui, "pixels", color::secondary());
    });
    hstack(ui, 8.0, |ui| {
        fixed_label(ui, "Subdivisions", 110.0);
        let wd = w::fill_width(ui, 0.0, 0);
        w::number_field(ui, "grid-subdivisions", &mut st.grid_subdivisions, 1.0..=64.0, w::fmt_int, wd, false, true);
    });
    let step = st.grid_spacing / st.grid_subdivisions.max(1.0);
    para(ui, &format!("A subdivision every {} pixels.", w::fmt_trim2(step)), 12.0, color::secondary());
    hstack(ui, 8.0, |ui| {
        w::button(ui, "Cancel", 13.0, ButtonStyle::Bordered, true);
        w::button(ui, "Restore Defaults", 13.0, ButtonStyle::Bordered, true);
        trailing(ui, |ui| {
            w::button(ui, "OK", 13.0, ButtonStyle::Prominent, true);
        });
    });
}

/// `LevelsHistogramDisplay.scale`: the 95th percentile of the interior bins, times four, capped
/// at the peak.
pub(crate) fn histogram_scale(bins: &[f64; 256]) -> f64 {
    let peak = bins.iter().copied().filter(|b| b.is_finite() && *b > 0.0).fold(0.0, f64::max);
    if peak <= 0.0 {
        return 0.0;
    }
    let mut interior: Vec<f64> = bins[1..255].iter().copied().filter(|b| b.is_finite() && *b > 0.0).collect();
    if interior.is_empty() {
        return peak;
    }
    interior.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let typical = interior[((interior.len() - 1) as f64 * 0.95) as usize];
    peak.min(typical * 4.0)
}

pub(crate) fn triangle(p: &egui::Painter, center: egui::Pos2, fill: Color32) {
    let pts = vec![center + vec2(0.0, -5.5), center + vec2(6.0, 5.0), center + vec2(-6.0, 5.0)];
    p.add(egui::Shape::convex_polygon(pts.iter().map(|q| *q + vec2(0.0, 0.5)).collect(), theme::gray(0.5), Stroke::NONE));
    p.add(egui::Shape::convex_polygon(pts, fill, Stroke::NONE));
}

/// A bordered button with a leading symbol (`Label(_, systemImage:)`), tinted `.secondary` as
/// Levels' sampling buttons are (the accent while `armed`): a faint fill and tinted text, the
/// symbol centered 20 points in and the title 36 points in.
pub(crate) fn icon_button(ui: &mut Ui, symbol: &'static str, title: &str, armed: bool) -> egui::Response {
    let tint = if armed { color::ACCENT } else { color::secondary() };
    let galley = ui.painter().layout_no_wrap(title.to_string(), theme::regular(13.0), tint);
    let (rect, response) = ui.allocate_exact_size(vec2((galley.size().x + 36.0 + 13.0).ceil(), 24.0), Sense::click());
    ui.painter().rect_filled(rect, egui::CornerRadius::same(12), if armed { color::ACCENT.gamma_multiply(0.15) } else { white_alpha(0.027) });
    icons::paint(ui.painter(), Icon::Symbol(symbol), pos2(rect.min.x + 20.0, rect.center().y), 12.0, tint);
    w::center_line(ui.painter(), galley, rect.min.x + 36.0, rect.center().y, tint);
    response
}

/// `CameraRawSlider` with a colored track (`GradientSliderCell`): a 4-point gradient bar across
/// the whole frame under macOS 26's 20 × 16 glass knob, which the bar shows through (a screen
/// blend, measured on the Mac's renders).
pub(crate) fn colored_slider(ui: &mut Ui, value: &mut f64, range: std::ops::RangeInclusive<f64>, width: f32, stops: &[Color32]) {
    let (rect, response) = ui.allocate_exact_size(vec2(width, 22.0), Sense::click_and_drag());
    let (lo, hi) = (*range.start(), *range.end());
    let (start, end) = (rect.min.x + 10.0, rect.max.x - 10.0);
    if let Some(pos) = response.interact_pointer_pos().filter(|_| response.dragged() || response.clicked()) {
        *value = lo + ((pos.x - start) / (end - start)).clamp(0.0, 1.0) as f64 * (hi - lo);
    }
    let cy = rect.center().y;
    let track = Rect::from_min_max(pos2(rect.min.x, cy - 2.0), pos2(rect.max.x, cy + 2.0));
    let n = stops.len().max(2) - 1;
    let color_at = |x: f32| {
        let f = ((x - track.min.x) / track.width()).clamp(0.0, 1.0) * n as f32;
        let i = (f.floor() as usize).min(n - 1);
        let t = f - i as f32;
        let (a, b) = (stops[i], stops[i + 1]);
        let mix = |p: u8, q: u8| (p as f32 + (q as f32 - p as f32) * t).round() as u8;
        Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
    };
    let strip = |p: &egui::Painter, from: f32, to: f32, map: &dyn Fn(Color32) -> Color32| {
        let mut mesh = Mesh::default();
        let steps = 48;
        for k in 0..=steps {
            let x = from + (to - from) * k as f32 / steps as f32;
            let c = map(color_at(x));
            mesh.colored_vertex(pos2(x, track.min.y), c);
            mesh.colored_vertex(pos2(x, track.max.y), c);
            if k > 0 {
                let b = (k * 2) as u32;
                mesh.add_triangle(b - 2, b - 1, b);
                mesh.add_triangle(b - 1, b, b + 1);
            }
        }
        p.add(egui::Shape::mesh(mesh));
    };
    let p = ui.painter();
    strip(&p.with_clip_rect(track.intersect(p.clip_rect())), track.min.x, track.max.x, &|c| c);
    let t = ((*value - lo) / (hi - lo)).clamp(0.0, 1.0) as f32;
    let knob = Rect::from_center_size(pos2(start + t * (end - start), cy.round()), vec2(20.0, 16.0));
    p.rect_filled(knob, 8.0, color::KNOB);
    let screen = |c: Color32| {
        let s = |k: u8, v: u8| 255 - ((255 - k as u32) * (255 - v as u32) / 255) as u8;
        Color32::from_rgb(s(color::KNOB.r(), c.r()), s(color::KNOB.g(), c.g()), s(color::KNOB.b(), c.b()))
    };
    strip(&p.with_clip_rect(knob.intersect(track).intersect(p.clip_rect())), knob.min.x, knob.max.x, &screen);
}

pub(crate) fn hsb(h: f32, s: f32, b: f32) -> Color32 {
    // HSB on the encoded sRGB values, as `PickerHSB` and NSColor compute it (egui's `Hsva` is
    // linear).
    let h = (h / 60.0).rem_euclid(6.0);
    let c = b * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, bl) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = b - c;
    let to = |v: f32| ((v + m) * 255.0).round() as u8;
    Color32::from_rgb(to(r), to(g), to(bl))
}

/// `c × k`, rounded, for an 8-bit channel.
pub(crate) fn scale(c: u8, k: f32) -> u8 {
    (c as f32 * k).round() as u8
}

/// Mixes two colors in Oklab, the perceptual space SwiftUI's gradients come closest to on the Mac.
pub(crate) fn oklab_mix(a: Color32, b: Color32, t: f32) -> Color32 {
    fn lin(c: u8) -> f32 {
        let c = c as f32 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    }
    fn enc(l: f32) -> u8 {
        let l = l.clamp(0.0, 1.0);
        let c = if l <= 0.0031308 { 12.92 * l } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 };
        (c * 255.0).round() as u8
    }
    fn to_lab(c: Color32) -> [f32; 3] {
        let (r, g, b) = (lin(c.r()), lin(c.g()), lin(c.b()));
        let l = (0.412_221_47 * r + 0.536_332_55 * g + 0.051_445_99 * b).cbrt();
        let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
        let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
        [0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s, 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s, 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s]
    }
    let (x, y) = (to_lab(a), to_lab(b));
    let [l, aa, bb] = [0, 1, 2].map(|i| x[i] + (y[i] - x[i]) * t);
    let l_ = (l + 0.396_337_78 * aa + 0.215_803_76 * bb).powi(3);
    let m_ = (l - 0.105_561_346 * aa - 0.063_854_17 * bb).powi(3);
    let s_ = (l - 0.089_484_18 * aa - 1.291_485_5 * bb).powi(3);
    Color32::from_rgb(
        enc(4.076_741_7 * l_ - 3.307_711_6 * m_ + 0.230_969_94 * s_),
        enc(-1.268_438 * l_ + 2.609_757_4 * m_ - 0.341_319_38 * s_),
        enc(-0.004_196_086 * l_ - 0.703_418_6 * m_ + 1.707_614_7 * s_),
    )
}

