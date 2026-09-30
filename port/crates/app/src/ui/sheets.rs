//! Sheets and floating panels (docs/port/ui-inventory.md §2.4–2.5), drawn as the Mac lays them
//! out. Their commands wait on the engine features behind them, so only `--render-ui` shows them
//! for now; the menu items that open them on the Mac stay disabled in the port.

use crate::app::App;
use crate::icons::{self, Icon};
use crate::theme::{self, black_alpha, color, white_alpha};
use crate::tools::Sampling;
use crate::widgets::{self as w, ButtonStyle, SwatchStyle};
use eframe::egui::{self, Align, Color32, Layout, Mesh, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

/// Every sheet's editable values, at the Mac's defaults.
pub struct SheetState {
    units: usize,
    width: f64,
    height: f64,
    relative: bool,
    locked: bool,
    anchor: usize,
    extension: usize,
    lock_ratio: bool,
    resolution: f64,
    resample: bool,
    sampling: Sampling,
    trim_based: usize,
    trim: [bool; 4],
    grid_color: usize,
    grid_style: usize,
    grid_opacity: f64,
    grid_spacing: f64,
    grid_subdivisions: f64,
    preview: bool,
    radius: f64,
    vignette: [f64; 5],
    dither_style: usize,
    dither: [f64; 5],
    dither_colors: usize,
    pixel_shape: usize,
    curves_channel: usize,
    shadow: [f64; 4],
    levels_channel: usize,
    levels: [f64; 5],
    hue_range: usize,
    hsl: [f64; 3],
    colorize: bool,
    fuzziness: f64,
    invert: bool,
    rgb: [f64; 3],
    search: String,
}

impl SheetState {
    pub fn new(app: &App) -> Self {
        let (width, height) = app.doc().map_or((0.0, 0.0), |d| (d.project.manifest.width as f64, d.project.manifest.height as f64));
        let resolution = app.doc().and_then(|d| d.project.manifest.resolution).unwrap_or(72.0);
        Self {
            units: 0,
            width,
            height,
            relative: false,
            locked: false,
            anchor: 4,
            extension: 0,
            lock_ratio: true,
            resolution,
            resample: true,
            sampling: Sampling::High,
            trim_based: 0,
            trim: [true; 4],
            grid_color: 0,
            grid_style: 0,
            grid_opacity: 45.0,
            grid_spacing: 64.0,
            grid_subdivisions: 8.0,
            preview: true,
            radius: 1.0,
            vignette: [35.0, 50.0, 100.0, 60.0, 25.0],
            dither_style: 0,
            dither: [2.0, 2.0, 100.0, 0.0, 0.0],
            dither_colors: 0,
            pixel_shape: 0,
            curves_channel: 0,
            shadow: [50.0, 90.0, 20.0, 20.0],
            levels_channel: 0,
            levels: [0.0, 1.0, 255.0, 0.0, 255.0],
            hue_range: 0,
            hsl: [0.0; 3],
            colorize: false,
            fuzziness: 40.0,
            invert: false,
            rgb: [0.0; 3],
            search: String::new(),
        }
    }
}

/// The content width of each sheet the port draws, or `None` for one it doesn't yet.
pub fn width(sheet: &str) -> Option<f32> {
    Some(match sheet {
        "new-canvas" => 500.0,
        "canvas-size" => 450.0,
        "image-size" => 430.0,
        "trim" => 320.0,
        "grid-settings" => 360.0,
        "levels" => 440.0,
        "hue-saturation" => 460.0,
        "color-range" => 340.0,
        "layer-effects" => 340.0,
        "filter:Gaussian Blur" | "filter:Curves" | "filter:Dither" | "filter:Vignette" => 380.0,
        "color-picker" => 20.0 + 256.0 + 14.0 + 34.0 + 14.0 + 180.0 + 20.0,
        "keyboard-shortcuts" => 660.0,
        _ => return None,
    })
}

/// Draws `sheet` into `rect` (its width from `width`); returns the height its content needs.
pub fn draw(app: &mut App, st: &mut SheetState, ui: &mut Ui, rect: Rect, sheet: &str) -> f32 {
    if sheet == "new-canvas" {
        super::welcome(app, ui, rect);
        return super::welcome_height();
    }
    let (padding, spacing) = match sheet {
        "layer-effects" | "color-picker" => (20.0, 16.0),
        "trim" | "grid-settings" | "image-size" => (24.0, 18.0),
        "keyboard-shortcuts" => (24.0, 10.0),
        _ => (24.0, 16.0),
    };
    let inner = rect.shrink(padding);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
    // No horizontal spacing here: egui widens a vertical layout by it after each full-width row.
    child.spacing_mut().item_spacing = vec2(0.0, spacing);
    let ui = &mut child;
    match sheet {
        "canvas-size" => canvas_size(app, st, ui),
        "image-size" => image_size(st, ui),
        "trim" => trim(st, ui),
        "grid-settings" => grid_settings(st, ui),
        "levels" => levels(app, st, ui),
        "hue-saturation" => hue_saturation(st, ui),
        "color-range" => color_range(app, st, ui),
        "layer-effects" => effects(st, ui),
        "filter:Gaussian Blur" => filter(st, ui, &[("Radius", 0)]),
        "filter:Vignette" => filter(st, ui, &[("Amount", 1), ("Midpoint", 2), ("Roundness", 3), ("Feather", 4), ("Highlights", 5)]),
        "filter:Dither" => filter(st, ui, &[("Pixel Size", 10), ("Tones", 11), ("Diffusion", 12), ("Density", 13), ("Contrast", 14)]),
        "filter:Curves" => filter(st, ui, &[]),
        "color-picker" => color_picker(st, ui),
        "keyboard-shortcuts" => keyboard_shortcuts(st, ui),
        _ => {}
    }
    ui.min_rect().height() + 2.0 * padding
}

// Pieces shared by the sheets.

fn title2(ui: &mut Ui, s: &str) {
    w::text(ui, s, theme::bold(17.0), color::label());
}

fn headline(ui: &mut Ui, s: &str) {
    w::text(ui, s, theme::semibold(13.0), color::label());
}

fn body(ui: &mut Ui, s: &str, c: Color32) {
    w::text(ui, s, theme::regular(13.0), c);
}

/// Wrapping text across the sheet's width.
fn para(ui: &mut Ui, s: &str, size: f32, c: Color32) {
    ui.add(egui::Label::new(egui::RichText::new(s).font(theme::regular(size)).color(c)).wrap().selectable(false));
}

fn caption(ui: &mut Ui, s: &str) {
    w::text(ui, s, theme::regular(10.0), color::secondary());
}

fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, color::separator());
}

/// An HStack across the sheet, controls centered on a 22-point line.
fn hstack<R>(ui: &mut Ui, spacing: f32, content: impl FnOnce(&mut Ui) -> R) -> R {
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 24.0), Layout::left_to_right(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = spacing;
        content(ui)
    })
    .inner
}

fn trailing(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    ui.with_layout(Layout::right_to_left(Align::Center), content);
}

/// A label of a fixed width, leading-aligned (`.frame(width:alignment: .leading)`).
fn fixed_label(ui: &mut Ui, s: &str, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 22.0), Sense::hover());
    ui.painter().text(pos2(rect.min.x, rect.center().y), egui::Align2::LEFT_CENTER, s, theme::regular(13.0), color::label());
}

/// Cancel · Spacer · OK (prominent), the usual sheet footer.
fn cancel_ok(ui: &mut Ui, ok: &str, prominent: bool) {
    hstack(ui, 8.0, |ui| {
        w::button(ui, "Cancel", 13.0, ButtonStyle::Bordered, true);
        trailing(ui, |ui| {
            w::button(ui, ok, 13.0, if prominent { ButtonStyle::Prominent } else { ButtonStyle::Bordered }, true);
        });
    });
}

/// A labeled menu picker across the sheet: "Label" then a pop-up filling the rest.
fn picker(ui: &mut Ui, label: &str, value: &mut usize, options: &'static [&'static str]) {
    hstack(ui, 8.0, |ui| {
        body(ui, label, color::label());
        let width = w::fill_width(ui, 0.0, 0);
        w::popup(ui, label, value, &[&(0..options.len()).collect::<Vec<_>>()], |i| options[i], Some(width), true);
    });
}

/// A picker sized to its content (`.fixedSize()`).
fn picker_fixed(ui: &mut Ui, label: &str, value: &mut usize, options: &'static [&'static str]) {
    hstack(ui, 8.0, |ui| {
        body(ui, label, color::label());
        w::popup(ui, label, value, &[&(0..options.len()).collect::<Vec<_>>()], |i| options[i], None, true);
    });
}

fn text_width(ui: &Ui, s: &str) -> f32 {
    ui.painter().layout_no_wrap(s.into(), theme::regular(13.0), color::label()).size().x
}

// The sheets.

fn bytes(n: i64) -> String {
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

const UNITS: &[&str] = &["Pixels", "Percent", "Inches", "Centimeters"];
const ANCHORS: [&str; 9] = ["Top left", "Top center", "Top right", "Middle left", "Center", "Middle right", "Bottom left", "Bottom center", "Bottom right"];

/// `CanvasSizeSheet`.
fn canvas_size(app: &App, st: &mut SheetState, ui: &mut Ui) {
    let (w0, h0) = app.doc().map_or((0, 0), |d| (d.project.manifest.width, d.project.manifest.height));
    title2(ui, "Canvas Size");
    body(ui, &format!("Current: {w0} × {h0} pixels"), color::label());
    para(ui, &format!("{} uncompressed RGBA canvas", bytes(w0 * h0 * 4)), 12.0, color::secondary());
    divider(ui);
    picker(ui, "Units", &mut st.units, UNITS);
    for (label, value) in [("Width", &mut st.width), ("Height", &mut st.height)] {
        hstack(ui, 8.0, |ui| {
            fixed_label(ui, label, 60.0);
            let wd = w::fill_width(ui, 0.0, 0);
            w::number_field(ui, ("canvas-size", label), value, 1.0..=30000.0, w::fmt_trim2, wd, false, true);
        });
    }
    w::checkbox(ui, &mut st.relative, "Relative to current dimensions");
    w::checkbox(ui, &mut st.locked, "Lock original aspect ratio");
    let (nw, nh) = (st.width.round() as i64, st.height.round() as i64);
    para(ui, &format!("New: {nw} × {nh} pixels · {} uncompressed", bytes(nw * nh * 4)), 12.0, color::secondary());
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = vec2(24.0, 8.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            body(ui, "Anchor", color::label());
            egui::Grid::new("anchors").spacing(vec2(3.0, 3.0)).show(ui, |ui| {
                for row in 0..3 {
                    for column in 0..3 {
                        let index = row * 3 + column;
                        let (rect, response) = ui.allocate_exact_size(vec2(33.0, 25.0), Sense::click());
                        ui.painter().rect_filled(rect, egui::CornerRadius::same(12), color::control());
                        let on = index == st.anchor;
                        icons::paint(ui.painter(), Icon::Symbol(if on { "circle.fill" } else { "circle" }), rect.center(), 13.0, if on { color::ACCENT } else { color::secondary() });
                        if response.on_hover_text(ANCHORS[index]).clicked() {
                            st.anchor = index;
                        }
                    }
                    ui.end_row();
                }
            });
        });
        ui.vertical(|ui| {
            ui.add_space(28.0);
            ui.spacing_mut().item_spacing.y = 8.0;
            w::text(ui, ANCHORS[st.anchor], theme::bold(12.0), color::label());
            para(ui, "Keeps this point fixed. Artwork is not scaled; cropped content remains outside the canvas.", 12.0, color::secondary());
        });
    });
    picker(ui, "Canvas extension", &mut st.extension, &["Transparent", "Foreground", "Background", "Black", "White", "Custom"]);
    cancel_ok(ui, "OK", false);
}

/// `ImageSizeSheet`.
fn image_size(st: &mut SheetState, ui: &mut Ui) {
    title2(ui, "Image Size");
    body(ui, &format!("Current: {} × {} pixels", st.width as i64, st.height as i64), color::secondary());
    picker(ui, "Units", &mut st.units, UNITS);
    for (label, value) in [("Width", &mut st.width), ("Height", &mut st.height)] {
        hstack(ui, 8.0, |ui| {
            fixed_label(ui, label, 75.0);
            let wd = w::fill_width(ui, 0.0, 0);
            w::number_field(ui, ("image-size", label), value, 1.0..=30000.0, w::fmt_trim2, wd, false, true);
        });
    }
    w::checkbox(ui, &mut st.lock_ratio, "Lock aspect ratio");
    hstack(ui, 8.0, |ui| {
        body(ui, "Resolution", color::label());
        let wd = w::fill_width(ui, text_width(ui, "pixels/inch"), 1);
        w::number_field(ui, "resolution", &mut st.resolution, 1.0..=9600.0, w::fmt_trim2, wd, false, true);
        body(ui, "pixels/inch", color::secondary());
    });
    w::checkbox(ui, &mut st.resample, "Resample");
    hstack(ui, 8.0, |ui| {
        body(ui, "Sampling", color::label());
        let wd = w::fill_width(ui, 0.0, 0);
        w::popup(ui, "sampling", &mut st.sampling, &[Sampling::ALL], Sampling::title, Some(wd), true);
    });
    para(ui, "Resizes layer pixels and applies existing transforms. Undo restores the originals.", 12.0, color::secondary());
    para(ui, &format!("Result: {} × {} pixels", st.width.round() as i64, st.height.round() as i64), 12.0, color::secondary());
    cancel_ok(ui, "Resize", false);
}

/// `TrimSheet`.
fn trim(st: &mut SheetState, ui: &mut Ui) {
    title2(ui, "Trim");
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        headline(ui, "Based On");
        for (i, option) in ["Transparent Pixels", "Top Left Pixel Color", "Bottom Right Pixel Color"].iter().enumerate() {
            if radio(ui, st.trim_based == i, option) {
                st.trim_based = i;
            }
        }
    });
    divider(ui);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        headline(ui, "Trim Away");
        egui::Grid::new("trim-away").spacing(vec2(24.0, 8.0)).show(ui, |ui| {
            let [top, bottom, left, right] = &mut st.trim;
            w::checkbox(ui, top, "Top");
            w::checkbox(ui, bottom, "Bottom");
            ui.end_row();
            w::checkbox(ui, left, "Left");
            w::checkbox(ui, right, "Right");
            ui.end_row();
        });
    });
    divider(ui);
    cancel_ok(ui, "OK", true);
}

/// An AppKit radio button: a 14-point circle, accent with a white dot when chosen.
fn radio(ui: &mut Ui, on: bool, title: &str) -> bool {
    let font = w::control_font(ui);
    let galley = ui.painter().layout_no_wrap(title.to_string(), font, color::label());
    let (rect, response) = ui.allocate_exact_size(vec2(14.0 + 6.0 + galley.size().x, 16.0), Sense::click());
    let c = rect.left_center() + vec2(7.0, 0.0);
    if on {
        ui.painter().circle_filled(c, 7.0, color::ACCENT);
        ui.painter().circle_filled(c, 2.5, Color32::WHITE);
    } else {
        ui.painter().circle_filled(c, 7.0, white_alpha(0.08));
        ui.painter().circle_stroke(c, 6.5, Stroke::new(1.0, white_alpha(0.22)));
    }
    ui.painter().galley(pos2(rect.min.x + 20.0, rect.center().y - galley.size().y / 2.0), galley, color::label());
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
        let wd = w::fill_width(ui, 34.0, 1);
        w::popup(ui, "grid-color", &mut st.grid_color, &[&(0..PRESETS.len()).collect::<Vec<_>>()], |i| PRESETS[i], Some(wd), true);
        let c = PRESET_COLORS[st.grid_color.min(8)];
        w::swatch(ui, w::rgb(c), vec2(34.0, 18.0), SwatchStyle { radius: 4.0, inner_white: 1.0, outer_black: 1.0 });
    });
    hstack(ui, 8.0, |ui| {
        fixed_label(ui, "Style", 110.0);
        let wd = w::fill_width(ui, 0.0, 0);
        w::popup(ui, "grid-style", &mut st.grid_style, &[&[0usize, 1, 2]], |i| ["Lines", "Dashed Lines", "Dots"][i], Some(wd), true);
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

/// The filter panel's row: title (scrubs), slider, a right-aligned field and its unit.
fn filter_row(ui: &mut Ui, label_width: f32, title: &str, value: &mut f64, range: std::ops::RangeInclusive<f64>, unit: &str, fmt: fn(f64) -> String, log: bool) {
    hstack(ui, 10.0, |ui| {
        let (rect, response) = ui.allocate_exact_size(vec2(label_width, 22.0), Sense::drag());
        ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, title, theme::regular(13.0), color::label());
        if response.dragged() {
            *value = (*value + response.drag_delta().x as f64).clamp(*range.start(), *range.end());
        }
        let unit_width = if unit.is_empty() { 0.0 } else { w::unit_width(ui, unit) };
        let slider_width = w::fill_width(ui, 56.0 + unit_width, if unit.is_empty() { 1 } else { 2 });
        if log {
            let mut l = value.ln();
            if w::slider(ui, &mut l, range.start().ln()..=range.end().ln(), slider_width, true).changed() {
                *value = l.exp();
            }
        } else {
            w::slider(ui, value, range.clone(), slider_width, true);
        }
        w::number_field(ui, ("filter", title), value, range, fmt, 56.0, true, true);
        if !unit.is_empty() {
            w::unit(ui, unit);
        }
    });
}

const DITHER_STYLES: [&[&str]; 4] = [
    &["Atkinson (Classic Mac)", "Floyd–Steinberg"],
    &["Bayer 2 × 2", "Bayer 4 × 4", "Bayer 8 × 8"],
    &["Halftone Dots", "Halftone Lines", "Halftone Diamonds"],
    &["Mac Patterns", "ASCII", "Scanlines (CRT)"],
];

/// `FilterSheet` for the filters listed: each row is (title, which value).
fn filter(st: &mut SheetState, ui: &mut Ui, rows: &[(&str, usize)]) {
    let widest = rows.iter().map(|(t, _)| text_width(ui, t)).fold(0.0f32, f32::max);
    let label_width = widest.max(60.0);
    let curves = rows.is_empty();
    let dither = rows.first().is_some_and(|(_, k)| *k >= 10);
    if curves {
        curves_controls(st, ui);
    }
    if dither {
        hstack(ui, 8.0, |ui| {
            body(ui, "Style", color::label());
            let wd = w::fill_width(ui, 0.0, 0);
            let groups: Vec<Vec<usize>> = {
                let mut start = 0;
                DITHER_STYLES.iter().map(|g| { let v = (start..start + g.len()).collect(); start += g.len(); v }).collect()
            };
            let group_refs: Vec<&[usize]> = groups.iter().map(|g| g.as_slice()).collect();
            let titles: Vec<&'static str> = DITHER_STYLES.iter().flat_map(|g| g.iter().copied()).collect();
            w::popup(ui, "dither-style", &mut st.dither_style, &group_refs, |i| titles[i], Some(wd), true);
        });
    }
    if rows.first().is_some_and(|(_, k)| *k == 1) {
        hstack(ui, 8.0, |ui| {
            fixed_label(ui, "Color", 95.0);
            w::swatch(ui, Color32::BLACK, vec2(24.0, 24.0), SwatchStyle { radius: 6.0, inner_white: 1.5, outer_black: 1.0 });
        });
    }
    for (title, key) in rows {
        let (value, range, unit, fmt, log): (&mut f64, std::ops::RangeInclusive<f64>, &str, fn(f64) -> String, bool) = match key {
            0 => (&mut st.radius, 0.1..=250.0, "px", w::fmt_trim1, true),
            1 => (&mut st.vignette[0], 0.0..=100.0, "%", w::fmt_int, false),
            2 => (&mut st.vignette[1], 0.0..=100.0, "%", w::fmt_int, false),
            3 => (&mut st.vignette[2], -100.0..=100.0, "", w::fmt_int, false),
            4 => (&mut st.vignette[3], 0.0..=100.0, "%", w::fmt_int, false),
            5 => (&mut st.vignette[4], 0.0..=100.0, "%", w::fmt_int, false),
            10 => (&mut st.dither[0], 1.0..=32.0, "px", w::fmt_int, false),
            11 => (&mut st.dither[1], 2.0..=8.0, "", w::fmt_int, false),
            12 => (&mut st.dither[2], 0.0..=100.0, "%", w::fmt_int, false),
            13 => (&mut st.dither[3], -100.0..=100.0, "", w::fmt_int, false),
            _ => (&mut st.dither[4], -100.0..=100.0, "", w::fmt_int, false),
        };
        filter_row(ui, label_width, title, value, range, unit, fmt, log);
    }
    if dither {
        picker_fixed(ui, "Colors", &mut st.dither_colors, &["Black & White", "Two Colors", "Original"]);
        if st.dither[0] > 1.0 {
            picker_fixed(ui, "Pixel Shape", &mut st.pixel_shape, &["Square", "Dot"]);
        }
    }
    w::checkbox(ui, &mut st.preview, "Preview");
    divider(ui);
    cancel_ok(ui, "OK", true);
}

/// `CurvesControls`, at the identity curve.
fn curves_controls(st: &mut SheetState, ui: &mut Ui) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 12.0;
        picker(ui, "Channel", &mut st.curves_channel, &["RGB", "Red", "Green", "Blue"]);
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 260.0), Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, 0.0, black_alpha(0.35));
        for i in 0..=4 {
            let f = i as f32 / 4.0;
            p.line_segment([pos2(rect.min.x + f * rect.width(), rect.min.y), pos2(rect.min.x + f * rect.width(), rect.max.y)], Stroke::new(1.0, white_alpha(0.12)));
            p.line_segment([pos2(rect.min.x, rect.min.y + f * rect.height()), pos2(rect.max.x, rect.min.y + f * rect.height())], Stroke::new(1.0, white_alpha(0.12)));
        }
        p.line_segment([rect.left_bottom(), rect.right_top()], Stroke::new(2.0, Color32::WHITE));
        p.circle_filled(rect.left_bottom(), 4.0, Color32::WHITE);
        p.circle_filled(rect.right_top(), 4.0, Color32::WHITE);
        caption(ui, "Click to add a point. Drag to adjust.");
        hstack(ui, 8.0, |ui| {
            trailing(ui, |ui| {
                w::button(ui, "Remove point", 13.0, ButtonStyle::Bordered, false);
            });
        });
        w::button(ui, "Reset curve", 13.0, ButtonStyle::Bordered, true);
    });
}

/// `EffectsSheet` for a new Drop Shadow.
fn effects(st: &mut SheetState, ui: &mut Ui) {
    hstack(ui, 8.0, |ui| {
        headline(ui, "Drop Shadow");
        trailing(ui, |ui| {
            w::swatch(ui, Color32::BLACK, vec2(36.0, 18.0), SwatchStyle { radius: 3.0, inner_white: 1.0, outer_black: 1.0 });
        });
    });
    let rows: [(&str, usize, std::ops::RangeInclusive<f64>, &str); 4] = [("Opacity", 0, 0.0..=100.0, "%"), ("Angle", 1, -180.0..=180.0, "°"), ("Distance", 2, 0.0..=100.0, "px"), ("Blur", 3, 0.0..=100.0, "px")];
    for (title, i, range, unit) in rows {
        hstack(ui, 10.0, |ui| {
            let mut v = st.shadow[i];
            w::scrub_label(ui, title, theme::regular(13.0), color::label(), &mut v, 1.0, range.clone(), true);
            ui.add_space(64.0 - text_width(ui, title) - 10.0);
            w::slider(ui, &mut v, range.clone(), 130.0, true);
            w::number_field(ui, ("effect", title), &mut v, range, w::fmt_int, 48.0, true, true);
            w::unit(ui, unit);
            st.shadow[i] = v;
        });
    }
    hstack(ui, 10.0, |ui| {
        trailing(ui, |ui| {
            w::button(ui, "OK", 13.0, ButtonStyle::Bordered, true);
            w::button(ui, "Cancel", 13.0, ButtonStyle::Bordered, true);
        });
    });
}

/// The histogram `LevelsSheet` shows: per channel, alpha-weighted, with RGB the mean of the three
/// (`levels_histogram` in LevelsPixels.c).
fn histogram(app: &App, layer: usize) -> [[f64; 256]; 4] {
    let mut bins = [[0.0; 256]; 4];
    let Some(doc) = app.doc() else { return bins };
    let Some(asset) = doc.project.manifest.layers.get(layer).and_then(|l| doc.project.images.get(&l.id)) else { return bins };
    for p in asset.pixels.pixels() {
        let a = p.0[3] as u32;
        if a == 0 {
            continue;
        }
        let weight = a as f64 / 255.0;
        for channel in 0..3 {
            // The layer is drawn premultiplied first, as BrushRaster does, then read back straight.
            let premultiplied = (p.0[channel] as u32 * a + 127) / 255;
            let value = ((premultiplied as f64 * 255.0 / a as f64).round() as usize).min(255);
            bins[channel + 1][value] += weight;
            bins[0][value] += weight / 3.0;
        }
    }
    bins
}

/// `LevelsHistogramDisplay.scale`: the 95th percentile of the interior bins, times four, capped
/// at the peak.
fn histogram_scale(bins: &[f64; 256]) -> f64 {
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

fn triangle(p: &egui::Painter, center: egui::Pos2, fill: Color32) {
    let pts = vec![center + vec2(0.0, -5.5), center + vec2(6.0, 5.0), center + vec2(-6.0, 5.0)];
    p.add(egui::Shape::convex_polygon(pts.iter().map(|q| *q + vec2(0.0, 0.5)).collect(), theme::gray(0.5), Stroke::NONE));
    p.add(egui::Shape::convex_polygon(pts, fill, Stroke::NONE));
}

/// `LevelsSheet` on the first layer.
fn levels(app: &App, st: &mut SheetState, ui: &mut Ui) {
    let bins = histogram(app, 0);
    ui.allocate_ui_with_layout(vec2(180.0, 24.0), Layout::left_to_right(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        body(ui, "Channel", color::label());
        let wd = w::fill_width(ui, 0.0, 0);
        w::popup(ui, "levels-channel", &mut st.levels_channel, &[&[0usize, 1, 2, 3]], |i| ["RGB", "Red", "Green", "Blue"][i], Some(wd), true);
    });
    let full = ui.available_width();
    let [black, gamma, white, out_black, out_white] = st.levels;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let (rect, _) = ui.allocate_exact_size(vec2(full, 150.0), Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, 0.0, black_alpha(0.25));
        let channel = &bins[st.levels_channel];
        let peak = histogram_scale(channel);
        if peak > 0.0 {
            let fill = [Color32::GRAY, Color32::RED, Color32::GREEN, Color32::BLUE][st.levels_channel];
            let mut mesh = Mesh::default();
            for (i, b) in channel.iter().enumerate() {
                let h = rect.height() * (b / peak).clamp(0.0, 1.0) as f32;
                let x = rect.min.x + i as f32 * rect.width() / 256.0;
                mesh.add_colored_rect(Rect::from_min_max(pos2(x, rect.max.y - h), pos2(x + rect.width() / 256.0 + 0.1, rect.max.y)), fill);
            }
            p.add(egui::Shape::mesh(mesh));
        }
        let (strip, _) = ui.allocate_exact_size(vec2(full, 20.0), Sense::hover());
        let gamma_position = black + (white - black) * 0.5f64.powf(gamma);
        for (i, v) in [black, gamma_position, white].iter().enumerate() {
            let fill = [Color32::BLACK, Color32::GRAY, Color32::WHITE][i];
            triangle(ui.painter(), pos2(strip.min.x + (*v / 255.0) as f32 * full, strip.min.y + 9.0), fill);
        }
    });
    hstack_fields(ui, &mut st.levels, &[("Input black", 0), ("Gamma", 1), ("Input white", 2)]);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let (rect, _) = ui.allocate_exact_size(vec2(full, 14.0), Sense::hover());
        let mut mesh = Mesh::default();
        mesh.colored_vertex(rect.left_top(), Color32::BLACK);
        mesh.colored_vertex(rect.right_top(), Color32::WHITE);
        mesh.colored_vertex(rect.right_bottom(), Color32::WHITE);
        mesh.colored_vertex(rect.left_bottom(), Color32::BLACK);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        ui.painter().add(egui::Shape::mesh(mesh));
        let (strip, _) = ui.allocate_exact_size(vec2(full, 20.0), Sense::hover());
        triangle(ui.painter(), pos2(strip.min.x + (out_black / 255.0) as f32 * full, strip.min.y + 9.0), Color32::BLACK);
        triangle(ui.painter(), pos2(strip.min.x + (out_white / 255.0) as f32 * full, strip.min.y + 9.0), Color32::WHITE);
    });
    hstack_fields(ui, &mut st.levels, &[("Output black", 3), ("Output white", 4)]);
    hstack(ui, 8.0, |ui| {
        caption(ui, "Sample");
        for name in ["Black", "Gray", "White"] {
            icon_button(ui, "eyedropper", name);
        }
    });
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        caption(ui, "Auto");
        hstack(ui, 8.0, |ui| {
            for name in ["Contrast", "Color", "Color + neutral midtones"] {
                w::button(ui, name, 13.0, ButtonStyle::Bordered, true);
            }
        });
    });
    hstack(ui, 8.0, |ui| {
        w::checkbox(ui, &mut st.preview, "Preview");
        trailing(ui, |ui| {
            w::button(ui, "Reset", 13.0, ButtonStyle::Bordered, true);
        });
    });
    caption(ui, "Original pixels · alpha-weighted histogram");
    divider(ui);
    cancel_ok(ui, "OK", true);
}

/// Levels' field groups: caption over a right-aligned field, spread across the row.
fn hstack_fields(ui: &mut Ui, values: &mut [f64; 5], fields: &[(&str, usize)]) {
    let full = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(full, 10.0 + 5.0 + 22.0), Sense::hover());
    let n = fields.len();
    for (k, (name, i)) in fields.iter().enumerate() {
        let x = if n == 1 { rect.min.x } else { rect.min.x + (full - 80.0) * k as f32 / (n - 1) as f32 };
        let column = Rect::from_min_size(pos2(x, rect.min.y), vec2(80.0, rect.height()));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(column).layout(Layout::top_down(Align::Min)));
        child.spacing_mut().item_spacing.y = 5.0;
        let (range, fmt): (std::ops::RangeInclusive<f64>, fn(f64) -> String) = if *name == "Gamma" { (0.1..=9.99, |v| format!("{v:.2}")) } else { (0.0..=255.0, w::fmt_int) };
        w::scrub_label(&mut child, name, theme::regular(10.0), color::secondary(), &mut values[*i], 1.0, range.clone(), true);
        w::number_field(&mut child, ("levels", *name), &mut values[*i], range, fmt, 80.0, true, true);
    }
}

/// A bordered button with a leading symbol (`Label(_, systemImage:)`).
fn icon_button(ui: &mut Ui, symbol: &'static str, title: &str) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(title.to_string(), theme::regular(13.0), color::label());
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + 22.0 + 18.0, 24.0), Sense::click());
    ui.painter().rect_filled(rect, egui::CornerRadius::same(12), color::control());
    icons::paint(ui.painter(), Icon::Symbol(symbol), pos2(rect.min.x + 11.0 + 7.0, rect.center().y), 12.0, color::secondary());
    ui.painter().galley(pos2(rect.min.x + 11.0 + 18.0, rect.center().y - galley.size().y / 2.0), galley, color::label());
    response
}

/// `CameraRawSlider`: a 4-point bar in the track's colors under a round knob.
fn colored_slider(ui: &mut Ui, value: &mut f64, range: std::ops::RangeInclusive<f64>, width: f32, stops: &[Color32]) {
    let (rect, response) = ui.allocate_exact_size(vec2(width, 22.0), Sense::click_and_drag());
    let (lo, hi) = (*range.start(), *range.end());
    let track = Rect::from_min_max(pos2(rect.min.x + 8.0, rect.center().y - 2.0), pos2(rect.max.x - 8.0, rect.center().y + 2.0));
    if let Some(pos) = response.interact_pointer_pos().filter(|_| response.dragged() || response.clicked()) {
        *value = lo + ((pos.x - track.min.x) / track.width()).clamp(0.0, 1.0) as f64 * (hi - lo);
    }
    let mut mesh = Mesh::default();
    let n = stops.len().max(2) - 1;
    for (i, c) in stops.iter().enumerate() {
        let x = track.min.x + track.width() * i as f32 / n as f32;
        mesh.colored_vertex(pos2(x, track.min.y), *c);
        mesh.colored_vertex(pos2(x, track.max.y), *c);
        if i > 0 {
            let b = (i * 2) as u32;
            mesh.add_triangle(b - 2, b - 1, b);
            mesh.add_triangle(b - 1, b, b + 1);
        }
    }
    ui.painter().add(egui::Shape::mesh(mesh));
    let t = ((*value - lo) / (hi - lo)).clamp(0.0, 1.0) as f32;
    let center = pos2(track.min.x + t * track.width(), rect.center().y);
    ui.painter().circle_filled(center + vec2(0.0, 0.5), 8.0, black_alpha(0.25));
    ui.painter().circle_filled(center, 7.5, theme::gray(0.84));
}

fn hsb(h: f32, s: f32, b: f32) -> Color32 {
    let rgb = egui::ecolor::Hsva::new((h / 360.0).rem_euclid(1.0), s, b, 1.0).to_srgb();
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

/// `HueSaturationSheet` at Master.
fn hue_saturation(st: &mut SheetState, ui: &mut Ui) {
    const RANGES: &[&str] = &["Master", "Reds", "Yellows", "Greens", "Cyans", "Blues", "Magentas"];
    hstack(ui, 12.0, |ui| {
        w::popup(ui, "hue-range", &mut st.hue_range, &[&(0..RANGES.len()).collect::<Vec<_>>()], |i| RANGES[i], Some(160.0), true);
        trailing(ui, |ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(24.0, 20.0), Sense::click());
            icons::paint(ui.painter(), Icon::Symbol("hand.point.up.left"), rect.center(), 13.0, color::label());
        });
    });
    let spectrum: Vec<Color32> = (0..13).map(|i| hsb(-180.0 + 30.0 * i as f32, 0.85, 0.9)).collect();
    let chroma = [Color32::from_rgb(158, 158, 163), Color32::from_rgb(219, 46, 51)];
    let lightness = [Color32::BLACK, Color32::WHITE];
    let rows: [(&str, std::ops::RangeInclusive<f64>, &str, &[Color32]); 3] = [("Hue", -180.0..=180.0, "°", &spectrum), ("Saturation", -100.0..=100.0, "", &chroma), ("Lightness", -100.0..=100.0, "", &lightness)];
    for (i, (title, range, unit, stops)) in rows.into_iter().enumerate() {
        hstack(ui, 10.0, |ui| {
            fixed_label(ui, title, 76.0);
            let unit_width = if unit.is_empty() { 0.0 } else { w::unit_width(ui, unit) };
            let wd = w::fill_width(ui, 48.0 + unit_width, if unit.is_empty() { 1 } else { 2 });
            colored_slider(ui, &mut st.hsl[i], range.clone(), wd, stops);
            w::number_field(ui, ("hsl", title), &mut st.hsl[i], range, w::fmt_int, 48.0, true, true);
            if !unit.is_empty() {
                w::unit(ui, unit);
            }
        });
    }
    hstack(ui, 18.0, |ui| {
        w::checkbox(ui, &mut st.colorize, "Colorize");
        w::checkbox(ui, &mut st.preview, "Preview");
        w::button(ui, "Reset", 13.0, ButtonStyle::Bordered, true);
    });
    divider(ui);
    cancel_ok(ui, "OK", true);
}

/// `ColorRangeSheet` before a color is picked.
fn color_range(app: &App, st: &mut SheetState, ui: &mut Ui) {
    hstack(ui, 6.0, |ui| {
        for (i, badge) in [None, Some("plus.circle.fill"), Some("minus.circle.fill")].into_iter().enumerate() {
            let (rect, _) = ui.allocate_exact_size(vec2(24.0, 20.0), Sense::click());
            if i == 0 {
                ui.painter().rect_filled(rect, 4.0, color::ACCENT.gamma_multiply(0.25));
            }
            icons::paint(ui.painter(), Icon::Symbol("eyedropper"), rect.center(), 13.0, color::label());
            if let Some(badge) = badge {
                icons::paint(ui.painter(), Icon::Symbol(badge), rect.center() + vec2(6.0, 4.0), 8.0, color::label());
            }
        }
    });
    let canvas = app.doc().map_or(vec2(1.0, 1.0), |d| d.size());
    let s = (292.0 / canvas.x).min(200.0 / canvas.y);
    let (rect, _) = ui.allocate_exact_size(canvas * s, Sense::hover());
    ui.painter().rect_filled(rect, 0.0, Color32::BLACK);
    ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, white_alpha(0.2)), StrokeKind::Inside);
    para(ui, "Click the image to pick the color to select.", 12.0, color::secondary());
    hstack(ui, 10.0, |ui| {
        w::scrub_label(ui, "Fuzziness", theme::regular(13.0), color::label(), &mut st.fuzziness, 1.0, 0.0..=200.0, true);
        let wd = w::fill_width(ui, 48.0, 1);
        w::slider(ui, &mut st.fuzziness, 0.0..=200.0, wd, true);
        w::number_field(ui, "fuzziness", &mut st.fuzziness, 0.0..=200.0, w::fmt_int, 48.0, true, true);
    });
    w::checkbox(ui, &mut st.invert, "Invert");
    divider(ui);
    cancel_ok(ui, "OK", true);
}

/// `ColorPickerSheet` for the foreground color (black).
fn color_picker(st: &mut SheetState, ui: &mut Ui) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        // Saturation across, brightness down, for hue 0.
        let (field, _) = ui.allocate_exact_size(vec2(256.0, 256.0), Sense::click());
        let mut mesh = Mesh::default();
        let n = 16;
        for j in 0..=n {
            for i in 0..=n {
                let (s, b) = (i as f32 / n as f32, 1.0 - j as f32 / n as f32);
                mesh.colored_vertex(pos2(field.min.x + 256.0 * i as f32 / n as f32, field.min.y + 256.0 * j as f32 / n as f32), hsb(0.0, s, b));
            }
        }
        for j in 0..n {
            for i in 0..n {
                let a = (j * (n + 1) + i) as u32;
                let b = a + 1;
                let c = a + (n + 1) as u32;
                let d = c + 1;
                mesh.add_triangle(a, b, d);
                mesh.add_triangle(a, d, c);
            }
        }
        ui.painter().add(egui::Shape::mesh(mesh));
        ui.painter().rect_stroke(field, 0.0, Stroke::new(1.0, black_alpha(0.6)), StrokeKind::Inside);
        let marker = field.left_bottom();
        ui.painter().circle_stroke(marker, 6.0, Stroke::new(1.5, Color32::WHITE));
        ui.painter().circle_stroke(marker, 6.75, Stroke::new(0.75, Color32::BLACK));
        // Hue strip, 360 at the top to 0 at the bottom, with arrows at the current hue.
        let (strip_slot, _) = ui.allocate_exact_size(vec2(34.0, 256.0), Sense::click());
        let strip = Rect::from_min_size(strip_slot.min + vec2(7.0, 0.0), vec2(20.0, 256.0));
        let mut mesh = Mesh::default();
        for k in 0..=12 {
            let y = strip.min.y + 256.0 * k as f32 / 12.0;
            let c = hsb(360.0 - 30.0 * k as f32, 1.0, 1.0);
            mesh.colored_vertex(pos2(strip.min.x, y), c);
            mesh.colored_vertex(pos2(strip.max.x, y), c);
            if k > 0 {
                let b = (k * 2) as u32;
                mesh.add_triangle(b - 2, b - 1, b);
                mesh.add_triangle(b - 1, b, b + 1);
            }
        }
        ui.painter().add(egui::Shape::mesh(mesh));
        ui.painter().rect_stroke(strip, 0.0, Stroke::new(1.0, black_alpha(0.6)), StrokeKind::Inside);
        let y = strip.max.y;
        ui.painter().add(egui::Shape::convex_polygon(vec![pos2(strip_slot.min.x, y - 5.0), pos2(strip_slot.min.x + 7.0, y), pos2(strip_slot.min.x, y + 5.0)], color::label(), Stroke::NONE));
        ui.painter().add(egui::Shape::convex_polygon(vec![pos2(strip_slot.max.x, y - 5.0), pos2(strip_slot.max.x - 7.0, y), pos2(strip_slot.max.x, y + 5.0)], color::label(), Stroke::NONE));
        // Preview, OK and Cancel, then the RGB and hex fields.
        ui.allocate_ui_with_layout(vec2(180.0, 256.0), Layout::top_down(Align::Min), |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                let (preview, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
                ui.painter().rect_filled(preview, 5.0, Color32::BLACK);
                ui.painter().rect_stroke(preview, 5.0, Stroke::new(1.0, black_alpha(0.6)), StrokeKind::Inside);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    for (title, style) in [("OK", ButtonStyle::Prominent), ("Cancel", ButtonStyle::Bordered)] {
                        let (rect, _) = ui.allocate_exact_size(vec2(90.0, 28.0), Sense::click());
                        let fill = if style == ButtonStyle::Prominent { color::ACCENT } else { color::control() };
                        ui.painter().rect_filled(rect, egui::CornerRadius::same(14), fill);
                        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, title, theme::regular(13.0), if style == ButtonStyle::Prominent { Color32::WHITE } else { color::label() });
                    }
                });
            });
            let used = ui.min_rect().height();
            let grid_height = 4.0 * 22.0 + 3.0 * 6.0 + 6.0 + 13.0;
            ui.add_space((256.0 - used - grid_height - 8.0).max(12.0));
            egui::Grid::new("rgb").spacing(vec2(8.0, 6.0)).show(ui, |ui| {
                for (i, label) in ["R", "G", "B"].iter().enumerate() {
                    let (rect, _) = ui.allocate_exact_size(vec2(14.0, 22.0), Sense::hover());
                    ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, *label, theme::regular(13.0), color::label());
                    w::number_field(ui, ("rgb", *label), &mut st.rgb[i], 0.0..=255.0, w::fmt_int, 52.0, false, true);
                    ui.end_row();
                }
                let (rect, _) = ui.allocate_exact_size(vec2(14.0, 22.0), Sense::hover());
                ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, "#", theme::regular(13.0), color::label());
                let mut hex = format!("{:02X}{:02X}{:02X}", st.rgb[0] as u8, st.rgb[1] as u8, st.rgb[2] as u8);
                w::text_field(ui, "hex", &mut hex, 84.0, "", egui::FontId::monospace(12.0));
                ui.end_row();
            });
            caption(ui, "Click the canvas to sample");
        });
    });
}

/// `KeyboardShortcutsSheet` with the default shortcuts.
fn keyboard_shortcuts(st: &mut SheetState, ui: &mut Ui) {
    para(ui, "Click a shortcut, then press its new key combination. Changes apply when you save.", 13.0, color::secondary());
    let wd = ui.available_width();
    w::text_field(ui, "shortcut-search", &mut st.search, wd, "Search shortcuts", theme::regular(13.0));
    let (list, _) = ui.allocate_exact_size(vec2(wd, 465.0), Sense::hover());
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list).layout(Layout::top_down(Align::Min)));
    child.set_clip_rect(list);
    egui::ScrollArea::vertical().id_salt("shortcuts").auto_shrink([false, false]).show(&mut child, |ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.set_width(wd - 8.0);
        for (group, rows) in crate::menus::shortcut_list() {
            ui.add_space(8.0);
            headline(ui, group);
            for (title, chord) in rows {
                if !st.search.is_empty() && !title.to_lowercase().contains(&st.search.to_lowercase()) {
                    continue;
                }
                hstack(ui, 8.0, |ui| {
                    body(ui, &title, color::label());
                    trailing(ui, |ui| {
                        let (rect, _) = ui.allocate_exact_size(vec2(150.0, 26.0), Sense::click());
                        ui.painter().rect_filled(rect.shrink2(vec2(0.0, 2.0)), egui::CornerRadius::same(11), color::control());
                        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, chord, theme::regular(13.0), color::label());
                    });
                });
            }
        }
        ui.add_space(8.0);
        divider(ui);
        ui.add_space(8.0);
        headline(ui, "Contextual keys & mouse gestures");
        para(ui, "Text fields keep standard editing keys. Dialogs share the Apply/Cancel assignments above. Numeric fields use Up/Down, with Shift for larger steps. Standard commands include Alt+F4 to quit and F11 for full screen. The shortcut editor itself always uses Enter to save and Esc to cancel when not recording.", 13.0, color::label());
        para(ui, "Alt temporarily selects the eyedropper in painting tools. Shift constrains shapes/movement or adds to a selection; Alt subtracts from selections or draws from center. Ctrl-drag moves selected pixels; Ctrl-Alt-drag copies them. Alt-drag duplicates layers/folders/effects; Alt-click at a layer boundary toggles clipping. Ctrl-click a thumbnail loads its selection. Right-drag adjusts brush size. Modifier-and-mouse gestures are fixed.", 13.0, color::label());
    });
    divider(ui);
    hstack(ui, 8.0, |ui| {
        w::button(ui, "Restore Defaults", 13.0, ButtonStyle::Bordered, true);
        trailing(ui, |ui| {
            w::button(ui, "Save", 13.0, ButtonStyle::Prominent, true);
            w::button(ui, "Cancel", 13.0, ButtonStyle::Bordered, true);
        });
    });
}
