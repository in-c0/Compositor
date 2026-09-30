//! The working sheets and floating panels: filters and image adjustments with live preview
//! (`FilterSheet`, `LevelsSheet`, `HueSaturationSheet`, `CurvesControls`), adjustment layers,
//! Canvas Size, Image Size, Trim, Export JPEG, the Photoshop conversion report, layer effects,
//! Rename, the selection amount and Color Range. Each ends in the engine's own operation.
//!
//! `sheets.rs` draws the same sheets for `--render-ui`; these are the live ones.

use crate::app::App;
use crate::document::Doc;
use crate::gfx::Gfx;
use crate::theme::{self, color};
use crate::widgets::{self as w, ButtonStyle};
use comp_format::{Adjustment, AdjustmentKind, Channel, ColorRange, CurvePoint, Project};
use eframe::egui::{self, Align, Color32, Layout, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::{Value, json};

pub enum Sheet {
    Filter(Box<FilterSheet>),
    CanvasSize(CanvasSizeSheet),
    ImageSize(ImageSizeSheet),
    Trim(TrimSheet),
    ExportJpeg(Box<JpegSheet>),
    /// The Photoshop conversion report before an import: the file, what will change, the project.
    Psd { name: String, conversions: Vec<(String, String)>, project: Box<Project>, into_document: bool },
    Rename { id: String, name: String },
    Effect(EffectSheet),
    /// Keyboard Shortcuts, with its search text.
    Shortcuts(String),
    /// Select > Expand (0), Contract (1) or Feather (2).
    Modify { kind: u8, amount: f64 },
    ColorRange(crate::ui::selection::ColorRange),
    /// The color picker for the foreground (or background) color, and the color it opened with.
    ColorPicker { background: bool, color: egui::ecolor::Hsva, original: [f32; 3] },
}

/// What a sheet asked for when it closed this frame.
enum Close {
    Open,
    Cancel,
    Ok,
}

fn footer(ui: &mut Ui, ok: &str, ok_enabled: bool) -> Close {
    let mut close = Close::Open;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if w::button(ui, "Cancel", 13.0, ButtonStyle::Bordered, true).clicked() {
            close = Close::Cancel;
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if w::button(ui, ok, 13.0, ButtonStyle::Prominent, ok_enabled).clicked() {
                close = Close::Ok;
            }
        });
    });
    // Return and Escape answer the sheet, as its default and cancel buttons.
    let (enter, escape) = ui.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, egui::Key::Enter), i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)));
    if escape {
        close = Close::Cancel;
    } else if enter && ok_enabled && !ui.ctx().egui_wants_keyboard_input() {
        close = Close::Ok;
    }
    close
}

fn title2(ui: &mut Ui, s: &str) {
    w::text(ui, s, theme::bold(17.0), color::label());
}

fn para(ui: &mut Ui, s: &str, c: Color32) {
    ui.add(egui::Label::new(egui::RichText::new(s).font(theme::regular(12.0)).color(c)).wrap().selectable(false));
}

fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, color::separator());
}

/// A window for a sheet, `width` wide, centered on the editor.
fn window(ctx: &egui::Context, title: &str, width: f32, content: impl FnOnce(&mut Ui)) {
    egui::Window::new(title)
        .id(egui::Id::new("sheet"))
        .collapsible(false)
        .resizable(false)
        .default_pos(ctx.content_rect().center() - vec2(width / 2.0, 200.0))
        .fixed_size(vec2(width, 0.0))
        .frame(egui::Frame::window(&ctx.global_style()).fill(theme::gray(0.17)).inner_margin(24))
        .show(ctx, |ui| {
            ui.set_width(width - 48.0);
            ui.spacing_mut().item_spacing = vec2(8.0, 14.0);
            content(ui);
        });
}

/// Draws the open sheet, if any, and acts on its buttons.
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut sheet) = app.sheet.take() else { return };
    let keep = match &mut sheet {
        Sheet::Filter(f) => filter_sheet(app, ctx, f),
        Sheet::CanvasSize(s) => canvas_size_sheet(app, ctx, s),
        Sheet::ImageSize(s) => image_size_sheet(app, ctx, s),
        Sheet::Trim(s) => trim_sheet(app, ctx, s),
        Sheet::ExportJpeg(s) => jpeg_sheet(app, ctx, s),
        Sheet::Psd { name, conversions, project, into_document } => psd_sheet(app, ctx, name, conversions, project, *into_document),
        Sheet::Rename { id, name } => rename_sheet(app, ctx, id, name),
        Sheet::Effect(s) => effect_sheet(app, ctx, s),
        Sheet::Shortcuts(search) => shortcuts_sheet(ctx, search),
        Sheet::Modify { kind, amount } => modify_sheet(app, ctx, *kind, amount),
        Sheet::ColorRange(s) => color_range_sheet(app, ctx, s),
        Sheet::ColorPicker { background, color, original } => color_picker_sheet(app, ctx, *background, color, *original),
    };
    if keep && app.sheet.is_none() {
        app.sheet = Some(sheet);
    }
}

/// A press on the canvas while a sheet is open: sampling for the sheets that sample.
pub fn canvas_press(app: &mut App, pixel: [f64; 2], modifiers: egui::Modifiers) {
    match &mut app.sheet {
        Some(Sheet::ColorRange(s)) => {
            let mode = if modifiers.shift {
                "Add"
            } else if modifiers.alt {
                "Remove"
            } else {
                s.samples.clear();
                "Sample"
            };
            s.samples.push((pixel, mode));
        }
        Some(Sheet::ColorPicker { .. }) => {
            // "Click the canvas to sample".
            let gfx = app.gfx.clone();
            let sampled = app.current.and_then(|i| app.docs.get_mut(i)).and_then(|d| d.sample(&gfx, pixel));
            if let (Some(c), Some(Sheet::ColorPicker { color, .. })) = (sampled, &mut app.sheet) {
                *color = egui::ecolor::Hsva::from_rgb(c);
            }
        }
        Some(Sheet::Filter(f)) if f.sampling.is_some() => {
            let gfx = app.gfx.clone();
            let which = f.sampling.take().unwrap();
            let Some(doc) = app.current.and_then(|i| app.docs.get_mut(i)) else { return };
            if let Some(c) = doc.sample(&gfx, pixel) {
                if let Some(Sheet::Filter(f)) = &mut app.sheet {
                    f.levels_sample(which, c);
                }
            }
        }
        _ => {}
    }
}

// Filters and image adjustments.

/// What a filter sheet edits.
#[derive(Clone, PartialEq)]
pub enum Target {
    /// The layer's own pixels (Filter and Image menus).
    Pixels(String),
    /// An adjustment layer's settings.
    Adjustment(String),
}

#[derive(Clone, Copy)]
enum Lit {
    Bool(bool),
    Str(&'static str),
}

impl Lit {
    fn value(self) -> Value {
        match self {
            Lit::Bool(b) => json!(b),
            Lit::Str(s) => json!(s),
        }
    }
}

/// One control of a `FilterSheet`, reading and writing `path` in the settings (Swift names).
#[derive(Clone, Copy)]
enum Ctl {
    Slider { title: &'static str, path: &'static str, lo: f64, hi: f64, unit: &'static str, decimals: usize, log: bool },
    Toggle { title: &'static str, path: &'static str },
    Segment { title: &'static str, path: &'static str, options: &'static [(&'static str, Lit)] },
    Menu { title: &'static str, path: &'static str, options: &'static [&'static str] },
    Color { title: &'static str, path: &'static str },
    Heading(&'static str),
    Caption(&'static str),
    Curves,
    GradientBar,
    /// Shown only when `when` holds for the settings.
    If(fn(&Value) -> bool, &'static Ctl),
}

const fn slider(title: &'static str, path: &'static str, lo: f64, hi: f64, unit: &'static str, decimals: usize) -> Ctl {
    Ctl::Slider { title, path, lo, hi, unit, decimals, log: false }
}

const fn log(title: &'static str, path: &'static str, lo: f64, hi: f64, unit: &'static str, decimals: usize) -> Ctl {
    Ctl::Slider { title, path, lo, hi, unit, decimals, log: true }
}

fn style(v: &Value) -> &str {
    v["dither"]["style"].as_str().unwrap_or("")
}
fn is_halftone(v: &Value) -> bool {
    style(v).starts_with("Halftone")
}
fn is_scanlines(v: &Value) -> bool {
    style(v) == "Scanlines (CRT)"
}
fn is_ascii(v: &Value) -> bool {
    style(v) == "ASCII"
}
fn uses_pixels(v: &Value) -> bool {
    !is_ascii(v) && !is_scanlines(v)
}
fn diffuses(v: &Value) -> bool {
    matches!(style(v), "Atkinson (Classic Mac)" | "Floyd–Steinberg")
}
fn toned(v: &Value) -> bool {
    diffuses(v) || style(v).starts_with("Bayer")
}
fn two_colors(v: &Value) -> bool {
    v["dither"]["colors"] == "Two Colors"
}
fn pixel_shape(v: &Value) -> bool {
    uses_pixels(v) && v["dither"]["pixelSize"].as_f64().unwrap_or(1.0) > 1.0
}
fn light_on_dark(v: &Value) -> bool {
    is_halftone(v) || matches!(style(v), "Mac Patterns" | "ASCII")
}
fn advanced(v: &Value) -> bool {
    v["backgroundQuality"] == "Advanced"
}
fn tinted(v: &Value) -> bool {
    v["blackWhite"]["tint"].as_bool().unwrap_or(false)
}

const DITHER_STYLES: &[&str] = &[
    "Atkinson (Classic Mac)",
    "Floyd–Steinberg",
    "Bayer 2 × 2",
    "Bayer 4 × 4",
    "Bayer 8 × 8",
    "Halftone Dots",
    "Halftone Lines",
    "Halftone Diamonds",
    "Mac Patterns",
    "ASCII",
    "Scanlines (CRT)",
];

/// Each kind's controls, in the Mac's order (docs/port/ui-inventory.md §2.4.6).
fn controls(kind: &str) -> Vec<Ctl> {
    match kind {
        "Gaussian Blur" => vec![log("Radius", "radius", 0.1, 250.0, "px", 1)],
        "Motion Blur" => vec![slider("Angle", "angle", -90.0, 90.0, "°", 0), log("Distance", "distance", 1.0, 2000.0, "px", 0)],
        "Add Noise" => vec![
            log("Amount", "amount", 0.1, 400.0, "%", 1),
            Ctl::Segment { title: "Distribution", path: "gaussian", options: &[("Uniform", Lit::Bool(false)), ("Gaussian", Lit::Bool(true))] },
            Ctl::Toggle { title: "Monochromatic", path: "monochromatic" },
        ],
        "Vignette" => vec![
            Ctl::Color { title: "Color", path: "vignetteColor" },
            slider("Amount", "vignetteAmount", 0.0, 100.0, "%", 0),
            slider("Midpoint", "vignetteMidpoint", 0.0, 100.0, "%", 0),
            slider("Roundness", "vignetteRoundness", -100.0, 100.0, "", 0),
            slider("Feather", "vignetteFeather", 0.0, 100.0, "%", 0),
            slider("Highlights", "vignetteHighlights", 0.0, 100.0, "%", 0),
        ],
        "Bloom / Glow" => vec![slider("Amount", "bloomAmount", 0.0, 100.0, "%", 0), log("Radius", "bloomRadius", 1.0, 150.0, "px", 0)],
        "Dither" => {
            const PIXEL: Ctl = slider("Pixel Size", "dither.pixelSize", 1.0, 32.0, "px", 0);
            const TEXT: Ctl = slider("Text Size", "dither.textSize", 6.0, 64.0, "px", 0);
            const SPACING: Ctl = slider("Line Spacing", "dither.lineSpacing", 2.0, 32.0, "px", 0);
            const GLOW: Ctl = slider("Glow", "dither.glow", 0.0, 100.0, "%", 0);
            const DOTS: Ctl = slider("Dots", "dither.dots", 0.0, 100.0, "%", 0);
            const WOBBLE: Ctl = slider("Wobble", "dither.wobble", 0.0, 64.0, "px", 0);
            const CELL: Ctl = slider("Cell Size", "dither.cellSize", 4.0, 64.0, "px", 0);
            const ANGLE: Ctl = slider("Angle", "dither.angle", -90.0, 90.0, "°", 0);
            const TONES: Ctl = slider("Tones", "dither.levels", 2.0, 8.0, "", 0);
            const DIFFUSION: Ctl = slider("Diffusion", "dither.diffusion", 0.0, 100.0, "%", 0);
            const DARK: Ctl = Ctl::Color { title: "Dark", path: "dither.dark" };
            const LIGHT: Ctl = Ctl::Color { title: "Light", path: "dither.light" };
            const SHAPE: Ctl = Ctl::Menu { title: "Pixel Shape", path: "dither.pixelShape", options: &["Square", "Dot"] };
            const LIGHT_ON_DARK: Ctl = Ctl::Toggle { title: "Light on Dark", path: "dither.lightOnDark" };
            vec![
                Ctl::Menu { title: "Style", path: "dither.style", options: DITHER_STYLES },
                Ctl::If(uses_pixels, &PIXEL),
                Ctl::If(is_ascii, &TEXT),
                Ctl::If(is_scanlines, &SPACING),
                Ctl::If(is_scanlines, &GLOW),
                Ctl::If(is_scanlines, &DOTS),
                Ctl::If(is_scanlines, &WOBBLE),
                Ctl::If(is_halftone, &CELL),
                Ctl::If(is_halftone, &ANGLE),
                Ctl::If(toned, &TONES),
                Ctl::If(diffuses, &DIFFUSION),
                slider("Density", "dither.density", -100.0, 100.0, "", 0),
                slider("Contrast", "dither.contrast", -100.0, 100.0, "", 0),
                Ctl::Menu { title: "Colors", path: "dither.colors", options: &["Black & White", "Two Colors", "Original"] },
                Ctl::If(two_colors, &DARK),
                Ctl::If(two_colors, &LIGHT),
                Ctl::If(pixel_shape, &SHAPE),
                Ctl::If(light_on_dark, &LIGHT_ON_DARK),
            ]
        }
        "Tonal Contrast" => vec![
            slider("Amount", "tonalAmount", 0.0, 100.0, "%", 0),
            slider("Shadows", "tonalShadows", -100.0, 100.0, "%", 0),
            slider("Midtones", "tonalMidtones", -100.0, 100.0, "%", 0),
            slider("Highlights", "tonalHighlights", -100.0, 100.0, "%", 0),
            log("Radius", "tonalRadius", 1.0, 100.0, "px", 0),
        ],
        "Lens Correction" => vec![
            slider("Remove Distortion", "distortion", -100.0, 100.0, "", 0),
            Ctl::Caption("Positive straightens lines that bow outward (barrel); negative, lines that bow inward (pincushion)."),
        ],
        "Remove Background" => {
            const REFINE: Ctl = slider("Refine", "refineEdges", 0.0, 40.0, "px", 0);
            const CONTRAST: Ctl = slider("Contrast", "matteContrast", 0.0, 100.0, "%", 0);
            const SHIFT: Ctl = slider("Shift Edge", "shiftEdge", -10.0, 10.0, "px", 0);
            vec![
                Ctl::Caption("Hide the background behind a layer mask. The pixels stay, so you can paint the mask to fix the edge."),
                Ctl::Segment { title: "", path: "backgroundQuality", options: &[("Basic", Lit::Str("Basic")), ("Advanced", Lit::Str("Advanced"))] },
                Ctl::If(advanced, &REFINE),
                Ctl::If(advanced, &CONTRAST),
                Ctl::If(advanced, &SHIFT),
            ]
        }
        "Curves" => vec![Ctl::Curves],
        "Exposure" => vec![
            slider("Exposure", "exposure.exposure", -20.0, 20.0, "", 2),
            slider("Offset", "exposure.offset", -0.5, 0.5, "", 4),
            log("Gamma", "exposure.gamma", 0.01, 9.99, "", 2),
        ],
        "Gradient Map" => vec![
            Ctl::GradientBar,
            Ctl::Color { title: "Shadows", path: "gradientMap.shadows" },
            Ctl::Color { title: "Highlights", path: "gradientMap.highlights" },
            Ctl::Toggle { title: "Reverse", path: "gradientMap.reversed" },
        ],
        "Grain" => vec![slider("Amount", "grain.amount", 0.0, 100.0, "", 0), log("Size", "grain.size", 0.5, 20.0, "px", 1), slider("Roughness", "grain.roughness", 0.0, 100.0, "", 0)],
        "Black & White" => {
            const HUE: Ctl = slider("Hue", "blackWhite.tintHue", 0.0, 360.0, "°", 0);
            const SAT: Ctl = slider("Saturation", "blackWhite.tintSaturation", 0.0, 100.0, "%", 0);
            vec![
                slider("Reds", "blackWhite.reds", -200.0, 300.0, "%", 0),
                slider("Yellows", "blackWhite.yellows", -200.0, 300.0, "%", 0),
                slider("Greens", "blackWhite.greens", -200.0, 300.0, "%", 0),
                slider("Cyans", "blackWhite.cyans", -200.0, 300.0, "%", 0),
                slider("Blues", "blackWhite.blues", -200.0, 300.0, "%", 0),
                slider("Magentas", "blackWhite.magentas", -200.0, 300.0, "%", 0),
                Ctl::Toggle { title: "Tint", path: "blackWhite.tint" },
                Ctl::If(tinted, &HUE),
                Ctl::If(tinted, &SAT),
            ]
        }
        "Color Balance" => {
            let mut v = Vec::new();
            for (heading, prefix) in [("Shadows", "shadow"), ("Midtones", "mid"), ("Highlights", "highlight")] {
                v.push(Ctl::Heading(heading));
                let paths: [&'static str; 3] = match prefix {
                    "shadow" => ["colorBalance.shadowCyanRed", "colorBalance.shadowMagentaGreen", "colorBalance.shadowYellowBlue"],
                    "mid" => ["colorBalance.midCyanRed", "colorBalance.midMagentaGreen", "colorBalance.midYellowBlue"],
                    _ => ["colorBalance.highlightCyanRed", "colorBalance.highlightMagentaGreen", "colorBalance.highlightYellowBlue"],
                };
                for (title, path) in ["Cyan / Red", "Magenta / Green", "Yellow / Blue"].into_iter().zip(paths) {
                    v.push(slider(title, path, -100.0, 100.0, "", 0));
                }
            }
            v.push(Ctl::Toggle { title: "Preserve Luminosity", path: "colorBalance.preserveLuminosity" });
            v
        }
        _ => Vec::new(),
    }
}

/// `FilterSettings()`, in the op's JSON form.
pub fn default_settings() -> Value {
    let color = |r: f64, g: f64, b: f64| json!({ "red": r, "green": g, "blue": b });
    let mut v = json!({
        "radius": 1.0, "angle": 0.0, "distance": 10.0, "amount": 10.0, "gaussian": false, "monochromatic": false,
        "vignetteAmount": 35.0, "vignetteColor": color(0.0, 0.0, 0.0), "vignetteMidpoint": 50.0, "vignetteRoundness": 100.0,
        "vignetteFeather": 60.0, "vignetteHighlights": 25.0,
        "bloomAmount": 40.0, "bloomRadius": 24.0,
    });
    let more = json!({
        "tonalAmount": 50.0, "tonalRadius": 16.0, "tonalShadows": 40.0, "tonalMidtones": 60.0, "tonalHighlights": 30.0,
        "distortion": 0.0,
        "curves": comp_format::CurvesSettings::default(),
        "exposure": comp_format::ExposureSettings::default(),
        "gradientMap": comp_format::GradientMapSettings::default(),
        "grain": comp_format::GrainSettings::default(),
        "blackWhite": comp_format::BlackWhiteSettings::default(),
        "colorBalance": comp_format::ColorBalanceSettings { preserve_luminosity: true, ..Default::default() },
        "backgroundQuality": "Basic", "refineEdges": 12.0, "matteContrast": 25.0, "shiftEdge": 0.0,
    });
    let dither = json!({
        "style": "Atkinson (Classic Mac)", "pixelSize": 2.0, "pixelShape": "Square", "cellSize": 8.0, "textSize": 14.0,
        "lineSpacing": 4.0, "glow": 35.0, "dots": 0.0, "wobble": 0.0, "angle": 45.0, "levels": 2.0, "diffusion": 100.0,
        "density": 0.0, "contrast": 0.0, "colors": "Black & White", "dark": color(0.0, 0.0, 0.0), "light": color(1.0, 1.0, 1.0),
        "lightOnDark": true, "characters": " .:-=+*#%@",
    });
    if let (Some(v), Value::Object(more)) = (v.as_object_mut(), more) {
        v.extend(more);
        v.insert("dither".into(), dither);
    }
    v
}

fn get<'a>(v: &'a Value, path: &str) -> &'a Value {
    path.split('.').fold(v, |v, k| &v[k])
}

fn set(v: &mut Value, path: &str, value: Value) {
    let mut cur = v;
    let keys: Vec<&str> = path.split('.').collect();
    for k in &keys[..keys.len() - 1] {
        cur = &mut cur[*k];
    }
    cur[*keys.last().unwrap()] = value;
}

pub struct FilterSheet {
    /// A `FilterKind` raw value, or "Levels" or "Hue/Saturation".
    kind: &'static str,
    target: Target,
    settings: Value,
    /// Levels and Hue/Saturation (and every adjustment layer) edit an `Adjustment`.
    adjustment: Adjustment,
    seed: u32,
    preview: bool,
    /// What the canvas shows now, so a change is previewed once.
    previewed: Option<(Value, Option<Adjustment>, bool)>,
    error: Option<String>,
    curve_point: Option<usize>,
    /// Levels' Sample buttons: 0 black, 1 gray, 2 white.
    sampling: Option<u8>,
    histogram: Option<[[f64; 256]; 4]>,
}

impl FilterSheet {
    #[cfg(test)]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Sets one setting by its path in the op's settings (`exposure.exposure`).
    #[cfg(test)]
    pub fn set_setting(&mut self, path: &str, value: Value) {
        set(&mut self.settings, path, value);
    }

    /// Levels' eyedroppers: the sampled color sets the input black, gray or white point.
    fn levels_sample(&mut self, which: u8, c: [f32; 3]) {
        let values = c.map(|v| (v * 255.0).round() as f64);
        let luma = (values[0] + values[1] + values[2]) / 3.0;
        let ranges = &mut self.adjustment.levels.ranges;
        match which {
            0 => ranges[0].black = luma.min(ranges[0].white - 2.0).max(0.0),
            2 => ranges[0].white = luma.max(ranges[0].black + 2.0).min(255.0),
            _ => {
                // The gray point: gamma that maps this value to the middle.
                let r = ranges[0];
                let t = ((luma - r.black) / (r.white - r.black)).clamp(0.01, 0.99);
                ranges[0].gamma = (0.5f64.ln() / t.ln()).recip().clamp(0.1, 9.99);
            }
        }
    }
}

/// The Mac's `canAdjustColors`: a shown pixel layer that isn't a folder or an adjustment.
fn adjustable(doc: &Doc) -> Option<String> {
    let l = doc.active_layer()?;
    (!l.is_group() && l.adjustment.is_none() && doc.project.images.contains_key(&l.id) && crate::ui::canvas_tools::effectively_visible(&doc.project, l)).then(|| l.id.clone())
}

/// Filter > (kind)… or Image > (adjustment)… on the active layer.
pub fn open_filter(app: &mut App, kind: &'static str) {
    let Some(doc) = app.doc() else { return };
    if doc.mask_target {
        app.alert("Couldn’t open the filter", "Filters work on a layer's pixels. Select the layer's thumbnail rather than its mask.".into());
        return;
    }
    let Some(id) = adjustable(doc) else { return };
    let adjustment = match kind {
        "Levels" => Adjustment::new(AdjustmentKind::Levels),
        "Hue/Saturation" => Adjustment::new(AdjustmentKind::HueSaturation),
        _ => Adjustment::new(AdjustmentKind::Invert),
    };
    let sheet = FilterSheet {
        kind,
        target: Target::Pixels(id),
        settings: app.filter_settings.clone(),
        adjustment,
        seed: crate::layer_ops::random_seed(),
        preview: true,
        previewed: None,
        error: None,
        curve_point: None,
        sampling: None,
        histogram: None,
    };
    app.sheet = Some(Sheet::Filter(Box::new(sheet)));
}

/// The sheet kind that edits an adjustment layer of `kind`.
fn sheet_kind(kind: AdjustmentKind) -> &'static str {
    match kind {
        AdjustmentKind::HueSaturation => "Hue/Saturation",
        AdjustmentKind::Levels => "Levels",
        AdjustmentKind::Curves => "Curves",
        AdjustmentKind::Exposure => "Exposure",
        AdjustmentKind::GradientMap => "Gradient Map",
        AdjustmentKind::Grain => "Grain",
        AdjustmentKind::AddNoise => "Add Noise",
        AdjustmentKind::GaussianBlur => "Gaussian Blur",
        AdjustmentKind::MotionBlur => "Motion Blur",
        AdjustmentKind::Invert => "Invert",
        AdjustmentKind::BlackWhite => "Black & White",
        AdjustmentKind::ColorBalance => "Color Balance",
    }
}

/// The filter-style settings an adjustment layer holds, in the op's JSON form.
fn settings_of(a: &Adjustment) -> Value {
    let mut v = default_settings();
    v["curves"] = json!(a.curves);
    if let Some(e) = a.exposure_settings {
        v["exposure"] = json!(e);
    }
    if let Some(g) = a.gradient_map_settings {
        v["gradientMap"] = json!(g);
    }
    if let Some(g) = a.grain_settings {
        v["grain"] = json!(g);
    }
    if let Some(b) = a.black_white_settings {
        v["blackWhite"] = json!(b);
    }
    if let Some(c) = a.color_balance_settings {
        v["colorBalance"] = json!(c);
    }
    if let Some(r) = a.blur_radius {
        v["radius"] = json!(r);
    }
    if let Some(r) = a.motion_angle {
        v["angle"] = json!(r);
    }
    if let Some(r) = a.motion_distance {
        v["distance"] = json!(r);
    }
    if let Some(r) = a.noise_amount {
        v["amount"] = json!(r);
    }
    if let Some(r) = a.noise_gaussian {
        v["gaussian"] = json!(r);
    }
    if let Some(r) = a.noise_monochromatic {
        v["monochromatic"] = json!(r);
    }
    v
}

/// The adjustment with the sheet's settings written back.
fn adjustment_with(a: &Adjustment, v: &Value) -> Adjustment {
    let mut a = a.clone();
    let parse = |key: &str| v.get(key).cloned().unwrap_or(Value::Null);
    match a.kind {
        AdjustmentKind::Curves => a.curves = serde_json::from_value(parse("curves")).unwrap_or_default(),
        AdjustmentKind::Exposure => a.exposure_settings = serde_json::from_value(parse("exposure")).ok(),
        AdjustmentKind::GradientMap => a.gradient_map_settings = serde_json::from_value(parse("gradientMap")).ok(),
        AdjustmentKind::Grain => {
            let seed = a.grain_settings.map_or(0, |g| g.seed);
            a.grain_settings = serde_json::from_value::<comp_format::GrainSettings>(parse("grain")).ok().map(|g| comp_format::GrainSettings { seed, ..g });
        }
        AdjustmentKind::BlackWhite => a.black_white_settings = serde_json::from_value(parse("blackWhite")).ok(),
        AdjustmentKind::ColorBalance => a.color_balance_settings = serde_json::from_value(parse("colorBalance")).ok(),
        AdjustmentKind::GaussianBlur => a.blur_radius = v["radius"].as_f64(),
        AdjustmentKind::MotionBlur => {
            a.motion_angle = v["angle"].as_f64();
            a.motion_distance = v["distance"].as_f64();
        }
        AdjustmentKind::AddNoise => {
            a.noise_amount = v["amount"].as_f64();
            a.noise_gaussian = v["gaussian"].as_bool();
            a.noise_monochromatic = v["monochromatic"].as_bool();
        }
        _ => {}
    }
    a
}

/// Layer > Edit Adjustment…: the active adjustment layer's sheet.
pub fn edit_adjustment(app: &mut App) {
    let Some(layer) = app.doc().and_then(|d| d.active_layer()).cloned() else { return };
    let Some(adjustment) = layer.adjustment.clone() else { return };
    if adjustment.kind == AdjustmentKind::Invert {
        return;
    }
    let sheet = FilterSheet {
        kind: sheet_kind(adjustment.kind),
        target: Target::Adjustment(layer.id.clone()),
        settings: settings_of(&adjustment),
        adjustment,
        seed: 0,
        preview: true,
        previewed: None,
        error: None,
        curve_point: None,
        sampling: None,
        histogram: None,
    };
    app.sheet = Some(Sheet::Filter(Box::new(sheet)));
}

/// Premultiplies straight RGBA as Core Graphics draws it into a premultiplied bitmap.
fn premultiplied(image: &image::RgbaImage) -> Vec<u8> {
    let mut out = image.as_raw().clone();
    for p in out.chunks_exact_mut(4) {
        let a = p[3] as u32;
        for c in 0..3 {
            p[c] = ((p[c] as u32 * a + 127) / 255) as u8;
        }
    }
    out
}

/// Levels or Hue/Saturation on a layer's own pixels: the adjustment layers' kernel over the
/// premultiplied layer, as `LevelsEdit` and `HueSaturationEdit` commit them.
fn adjust_pixels(gfx: &Gfx, project: &mut Project, id: &str, adjustment: &Adjustment) -> Result<(), engine::RenderError> {
    let gpu = &gfx.engine.gpu;
    let Some(asset) = project.images.get_mut(id) else { return Ok(()) };
    let (w, h) = asset.pixels.dimensions();
    let source = gpu.upload(w, h, &premultiplied(&asset.pixels));
    let result = engine::adjust::apply(gpu, &source, adjustment, engine::adjust::Region::whole(&source))?;
    let straight = engine::blend::unpremultiply(gpu, &result);
    let bytes = gpu.download(&straight)?;
    asset.pixels = image::RgbaImage::from_raw(w, h, bytes).expect("adjusted size");
    asset.png = None;
    Ok(())
}

/// The project the sheet's settings make: the filter or adjustment applied (and limited to the
/// selection where the Mac limits it).
fn filtered(app: &App, doc: &Doc, f: &FilterSheet) -> Result<Project, engine::RenderError> {
    let mut project = doc.project.clone();
    match &f.target {
        Target::Adjustment(id) => {
            let adjustment = match f.kind {
                "Levels" | "Hue/Saturation" => f.adjustment.clone(),
                _ => adjustment_with(&f.adjustment, &f.settings),
            };
            if let Some(l) = project.manifest.layers.iter_mut().find(|l| &l.id == id) {
                l.adjustment = Some(adjustment);
            }
        }
        Target::Pixels(id) => {
            match f.kind {
                "Levels" | "Hue/Saturation" => adjust_pixels(&app.gfx, &mut project, id, &f.adjustment)?,
                kind => {
                    let mut op = json!({ "op": "filter", "layer": id, "kind": kind, "settings": f.settings });
                    if matches!(kind, "Add Noise" | "Grain") {
                        op["seed"] = json!(f.seed);
                    }
                    app.gfx.engine.apply_op(&mut project, &op)?;
                }
            }
            if doc.selection.is_some() && !crate::layer_ops::limit_to_selection(doc, &app.gfx, &mut project, id) {
                return Err(engine::RenderError::Unsupported("limiting this filter to the selection (the layer is transformed, or the filter resizes it)".into()));
            }
        }
    }
    Ok(project)
}

fn describe(e: &engine::RenderError) -> String {
    match e {
        engine::RenderError::Unsupported(what) => format!("The port can’t do this exactly yet: {what}."),
        engine::RenderError::Failed(e) => format!("{e:#}"),
    }
}

fn filter_title(f: &FilterSheet) -> String {
    match &f.target {
        Target::Adjustment(_) => format!("{} Adjustment", f.kind),
        Target::Pixels(_) => f.kind.to_string(),
    }
}

fn filter_sheet(app: &mut App, ctx: &egui::Context, f: &mut FilterSheet) -> bool {
    // A layer that went away (undo, another tab) closes the sheet.
    let exists = app.doc().is_some_and(|d| match &f.target {
        Target::Pixels(id) | Target::Adjustment(id) => d.layer(id).is_some(),
    });
    if !exists {
        if let Some(d) = app.doc_mut() {
            d.set_preview(None);
        }
        return false;
    }
    if f.kind == "Levels" && f.histogram.is_none() {
        f.histogram = Some(levels_histogram(app, f));
    }
    let width = match f.kind {
        "Levels" => 440.0,
        "Hue/Saturation" => 460.0,
        _ => 380.0,
    };
    let mut close = Close::Open;
    let title = filter_title(f);
    let has_selection = app.doc().is_some_and(|d| d.selection.is_some());
    window(ctx, &title, width, |ui| {
        match f.kind {
            "Levels" => levels_controls(ui, f),
            "Hue/Saturation" => hue_saturation_controls(ui, f),
            kind => {
                let ctls = controls(kind);
                let widest = ctls
                    .iter()
                    .filter_map(|c| match c {
                        Ctl::Slider { title, .. } => Some(*title),
                        Ctl::If(_, Ctl::Slider { title, .. }) => Some(*title),
                        _ => None,
                    })
                    .map(|t| ui.painter().layout_no_wrap(t.into(), theme::regular(13.0), color::label()).size().x)
                    .fold(60.0f32, f32::max);
                for c in &ctls {
                    control(ui, c, &mut f.settings, widest, &mut f.curve_point);
                }
            }
        }
        ui.horizontal(|ui| {
            w::checkbox(ui, &mut f.preview, "Preview");
        });
        if let Some(e) = &f.error {
            para(ui, e, color::ORANGE);
        }
        if has_selection && matches!(f.target, Target::Pixels(_)) {
            para(ui, "Limited to the selection", color::secondary());
        }
        divider(ui);
        close = footer(ui, "OK", true);
    });
    // Live preview: the canvas shows the result as the settings change.
    let key = (f.settings.clone(), matches!(f.kind, "Levels" | "Hue/Saturation").then(|| f.adjustment.clone()), f.preview);
    if f.previewed.as_ref() != Some(&key) {
        f.previewed = Some(key);
        let result = if f.preview { app.doc().map(|d| filtered(app, d, f)) } else { None };
        let doc = app.doc_mut().unwrap();
        match result {
            Some(Ok(p)) => {
                doc.set_preview(Some(p));
                f.error = None;
            }
            Some(Err(e)) => {
                doc.set_preview(None);
                f.error = Some(describe(&e));
            }
            None => doc.set_preview(None),
        }
    }
    match close {
        Close::Open => true,
        Close::Cancel => {
            if let Some(d) = app.doc_mut() {
                d.set_preview(None);
            }
            false
        }
        Close::Ok => {
            if matches!(f.target, Target::Pixels(_)) && !matches!(f.kind, "Levels" | "Hue/Saturation") {
                app.filter_settings = f.settings.clone();
            }
            let result = app.doc().map(|d| filtered(app, d, f));
            let doc = app.doc_mut().unwrap();
            match result {
                Some(Ok(p)) => {
                    doc.commit(&title, p);
                    false
                }
                Some(Err(e)) => {
                    f.error = Some(describe(&e));
                    true
                }
                None => false,
            }
        }
    }
}

/// One control row (`FilterSheet.control`): title (scrubs), slider, a right-aligned field and
/// its unit.
fn control(ui: &mut Ui, c: &Ctl, v: &mut Value, label_width: f32, curve_point: &mut Option<usize>) {
    match *c {
        Ctl::If(when, inner) => {
            if when(v) {
                control(ui, inner, v, label_width, curve_point);
            }
        }
        Ctl::Slider { title, path, lo, hi, unit, decimals, log } => {
            let mut value = get(v, path).as_f64().unwrap_or(lo);
            let before = value;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let (rect, response) = ui.allocate_exact_size(vec2(label_width, 22.0), Sense::drag());
                ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, title, theme::regular(13.0), color::label());
                if response.dragged() {
                    value = (value + response.drag_delta().x as f64 * (hi - lo) / 400.0).clamp(lo, hi);
                }
                let unit_width = if unit.is_empty() { 0.0 } else { w::unit_width(ui, unit) };
                let sw = w::fill_width(ui, 56.0 + unit_width, if unit.is_empty() { 1 } else { 2 });
                if log {
                    let mut l = value.max(lo).ln();
                    if w::slider(ui, &mut l, lo.ln()..=hi.ln(), sw, true).changed() {
                        value = l.exp();
                    }
                } else {
                    w::slider(ui, &mut value, lo..=hi, sw, true);
                }
                let fmt: fn(f64) -> String = match decimals {
                    0 => w::fmt_int,
                    1 => w::fmt_trim1,
                    2 => |v| format!("{v:.2}"),
                    _ => |v| format!("{v:.4}"),
                };
                w::number_field(ui, ("filter", path), &mut value, lo..=hi, fmt, 56.0, true, true);
                if !unit.is_empty() {
                    w::unit(ui, unit);
                }
            });
            if value != before {
                let factor = 10f64.powi(decimals as i32);
                set(v, path, json!((value * factor).round() / factor));
            }
        }
        Ctl::Toggle { title, path } => {
            let mut on = get(v, path).as_bool().unwrap_or(false);
            if w::checkbox(ui, &mut on, title).changed() {
                set(v, path, json!(on));
            }
        }
        Ctl::Segment { title, path, options } => {
            ui.horizontal(|ui| {
                if !title.is_empty() {
                    w::text(ui, title, theme::regular(13.0), color::label());
                }
                let current = get(v, path).clone();
                let mut index = options.iter().position(|(_, lit)| lit.value() == current).unwrap_or(0);
                let all: Vec<usize> = (0..options.len()).collect();
                let labels: Vec<&'static str> = options.iter().map(|(l, _)| *l).collect();
                if w::segmented(ui, &mut index, &all, |i| labels[i]).changed() {
                    set(v, path, options[index].1.value());
                }
            });
        }
        Ctl::Menu { title, path, options } => {
            ui.horizontal(|ui| {
                w::text(ui, title, theme::regular(13.0), color::label());
                let current = get(v, path).as_str().unwrap_or("").to_string();
                let mut index = options.iter().position(|o| *o == current).unwrap_or(0);
                let all: Vec<usize> = (0..options.len()).collect();
                if w::popup(ui, ("menu", path), &mut index, &[&all], |i| options[i], Some(220.0), true) {
                    set(v, path, json!(options[index]));
                }
            });
        }
        Ctl::Color { title, path } => {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(95.0, 22.0), Sense::hover());
                ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, title, theme::regular(13.0), color::label());
                let c = get(v, path);
                let mut rgb = [c["red"].as_f64().unwrap_or(0.0) as f32, c["green"].as_f64().unwrap_or(0.0) as f32, c["blue"].as_f64().unwrap_or(0.0) as f32];
                if egui::widgets::color_picker::color_edit_button_rgb(ui, &mut rgb).changed() {
                    set(v, path, json!({ "red": rgb[0] as f64, "green": rgb[1] as f64, "blue": rgb[2] as f64 }));
                }
            });
        }
        Ctl::Heading(s) => {
            w::text(ui, s, theme::semibold(13.0), color::label());
        }
        Ctl::Caption(s) => para(ui, s, color::secondary()),
        Ctl::GradientBar => {
            let gm = &v["gradientMap"];
            let color = |c: &Value| w::rgb([c["red"].as_f64().unwrap_or(0.0) as f32, c["green"].as_f64().unwrap_or(0.0) as f32, c["blue"].as_f64().unwrap_or(0.0) as f32]);
            let (mut a, mut b) = (color(&gm["shadows"]), color(&gm["highlights"]));
            if gm["reversed"].as_bool().unwrap_or(false) {
                std::mem::swap(&mut a, &mut b);
            }
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
            let mut mesh = egui::Mesh::default();
            mesh.colored_vertex(rect.left_top(), a);
            mesh.colored_vertex(rect.right_top(), b);
            mesh.colored_vertex(rect.right_bottom(), b);
            mesh.colored_vertex(rect.left_bottom(), a);
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            ui.painter().add(egui::Shape::mesh(mesh));
            ui.painter().rect_stroke(rect, 4.0, Stroke::new(1.0, theme::black_alpha(0.35)), egui::StrokeKind::Inside);
        }
        Ctl::Curves => curves_controls(ui, v, curve_point),
    }
}

/// `CurvesControls`: the channel, the curve (click to add a point, drag to move, max 32).
fn curves_controls(ui: &mut Ui, v: &mut Value, selected: &mut Option<usize>) {
    let mut curves: comp_format::CurvesSettings = serde_json::from_value(v["curves"].clone()).unwrap_or_default();
    let before = curves.clone();
    let channels = [Channel::Rgb, Channel::Red, Channel::Green, Channel::Blue];
    let mut ci = channels.iter().position(|c| *c == curves.channel).unwrap_or(0);
    ui.horizontal(|ui| {
        w::text(ui, "Channel", theme::regular(13.0), color::label());
        if w::popup(ui, "curves-channel", &mut ci, &[&[0usize, 1, 2, 3]], |i| ["RGB", "Red", "Green", "Blue"][i], Some(200.0), true) {
            *selected = None;
        }
    });
    curves.channel = channels[ci];
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 260.0), Sense::click_and_drag());
    let p = ui.painter();
    p.rect_filled(rect, 0.0, theme::black_alpha(0.35));
    for i in 1..4 {
        let f = i as f32 / 4.0;
        p.line_segment([pos2(rect.min.x + f * rect.width(), rect.min.y), pos2(rect.min.x + f * rect.width(), rect.max.y)], Stroke::new(1.0, theme::white_alpha(0.12)));
        p.line_segment([pos2(rect.min.x, rect.min.y + f * rect.height()), pos2(rect.max.x, rect.min.y + f * rect.height())], Stroke::new(1.0, theme::white_alpha(0.12)));
    }
    let to_view = |pt: &CurvePoint| pos2(rect.min.x + (pt.x / 255.0) as f32 * rect.width(), rect.max.y - (pt.y / 255.0) as f32 * rect.height());
    let from_view = |pos: egui::Pos2| CurvePoint {
        x: (((pos.x - rect.min.x) / rect.width()) as f64 * 255.0).round().clamp(0.0, 255.0),
        y: (((rect.max.y - pos.y) / rect.height()) as f64 * 255.0).round().clamp(0.0, 255.0),
    };
    let points = &mut curves.channels[ci];
    if response.drag_started() || response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let hit = points.iter().position(|pt| (to_view(pt) - pos).length() <= 8.0);
            *selected = match hit {
                Some(i) => Some(i),
                None if points.len() < 32 => {
                    let new = from_view(pos);
                    let at = points.iter().position(|q| q.x > new.x).unwrap_or(points.len());
                    points.insert(at, new);
                    Some(at)
                }
                None => None,
            };
        }
    }
    if response.dragged() {
        if let (Some(i), Some(pos)) = (*selected, response.interact_pointer_pos()) {
            if i < points.len() {
                let mut pt = from_view(pos);
                // A point stays between its neighbors.
                let lo = if i > 0 { points[i - 1].x + 1.0 } else { 0.0 };
                let hi = if i + 1 < points.len() { points[i + 1].x - 1.0 } else { 255.0 };
                pt.x = pt.x.clamp(lo, hi.max(lo));
                points[i] = pt;
            }
        }
    }
    // The curve through the points, as straight pieces between samples of a monotone spline.
    let samples: Vec<egui::Pos2> = (0..=64)
        .map(|k| {
            let x = k as f64 * 255.0 / 64.0;
            to_view(&CurvePoint { x, y: interpolate(points, x) })
        })
        .collect();
    p.add(egui::Shape::line(samples, Stroke::new(2.0, Color32::WHITE)));
    for (i, pt) in points.iter().enumerate() {
        p.circle_filled(to_view(pt), 4.0, if Some(i) == *selected { color::ACCENT } else { Color32::WHITE });
    }
    para(ui, "Click to add a point. Drag to adjust.", color::secondary());
    ui.horizontal(|ui| {
        if let Some(i) = selected.filter(|i| *i < points.len()) {
            let pt = points[i];
            w::text(ui, format!("Input {} · Output {}", pt.x, pt.y), egui::FontId::monospace(11.0), color::secondary());
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let removable = selected.is_some_and(|i| i > 0 && i + 1 < points.len());
            if w::button(ui, "Remove point", 13.0, ButtonStyle::Bordered, removable).clicked() {
                points.remove(selected.unwrap());
                *selected = None;
            }
        });
    });
    if w::button(ui, "Reset curve", 13.0, ButtonStyle::Bordered, true).clicked() {
        *points = vec![CurvePoint { x: 0.0, y: 0.0 }, CurvePoint { x: 255.0, y: 255.0 }];
        *selected = None;
    }
    if curves != before {
        v["curves"] = json!(curves);
    }
}

/// A smooth curve through `points` for the display (the engine computes the real table).
fn interpolate(points: &[CurvePoint], x: f64) -> f64 {
    if points.is_empty() {
        return x;
    }
    if x <= points[0].x {
        return points[0].y;
    }
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if x <= b.x {
            let t = if b.x > a.x { (x - a.x) / (b.x - a.x) } else { 0.0 };
            let t = t * t * (3.0 - 2.0 * t);
            return a.y + (b.y - a.y) * t;
        }
    }
    points.last().unwrap().y
}

/// `levels_histogram` over the layer's original pixels (or, for an adjustment layer, what's under it).
fn levels_histogram(app: &App, f: &FilterSheet) -> [[f64; 256]; 4] {
    let mut bins = [[0.0; 256]; 4];
    let Some(doc) = app.doc() else { return bins };
    let pixels: Option<Vec<u8>> = match &f.target {
        Target::Pixels(id) => doc.project.images.get(id).map(|a| premultiplied(&a.pixels)),
        Target::Adjustment(id) => {
            // Everything below the adjustment, flattened.
            let mut below = doc.project.clone();
            let index = below.manifest.layers.iter().position(|l| &l.id == id).unwrap_or(0);
            for (i, l) in below.manifest.layers.iter_mut().enumerate() {
                if i >= index && !l.is_group() {
                    l.is_visible = false;
                }
            }
            engine::composite::Compositor::new(&app.gfx.gpu, &below).render().ok().and_then(|c| app.gfx.gpu.download(&c).ok())
        }
    };
    let Some(pixels) = pixels else { return bins };
    for p in pixels.chunks_exact(4) {
        let a = p[3] as u32;
        if a == 0 {
            continue;
        }
        let weight = a as f64 / 255.0;
        for c in 0..3 {
            let value = ((p[c] as f64 * 255.0 / a as f64).round() as usize).min(255);
            bins[c + 1][value] += weight;
            bins[0][value] += weight / 3.0;
        }
    }
    bins
}

fn histogram_scale(bins: &[f64; 256]) -> f64 {
    let peak = bins.iter().copied().filter(|b| b.is_finite() && *b > 0.0).fold(0.0, f64::max);
    if peak <= 0.0 {
        return 0.0;
    }
    let mut interior: Vec<f64> = bins[1..255].iter().copied().filter(|b| *b > 0.0).collect();
    if interior.is_empty() {
        return peak;
    }
    interior.sort_by(|a, b| a.partial_cmp(b).unwrap());
    peak.min(interior[((interior.len() - 1) as f64 * 0.95) as usize] * 4.0)
}

/// `LevelsSheet`: channel, histogram with the input handles, the fields, output, samplers.
fn levels_controls(ui: &mut Ui, f: &mut FilterSheet) {
    let channels = [Channel::Rgb, Channel::Red, Channel::Green, Channel::Blue];
    let levels = &mut f.adjustment.levels;
    let mut ci = channels.iter().position(|c| *c == levels.channel).unwrap_or(0);
    ui.horizontal(|ui| {
        w::text(ui, "Channel", theme::regular(13.0), color::label());
        w::popup(ui, "levels-channel", &mut ci, &[&[0usize, 1, 2, 3]], |i| ["RGB", "Red", "Green", "Blue"][i], Some(120.0), true);
    });
    levels.channel = channels[ci];
    let range = &mut levels.ranges[ci];
    let full = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(full, 150.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::black_alpha(0.25));
    if let Some(bins) = &f.histogram {
        let channel = &bins[ci];
        let peak = histogram_scale(channel);
        if peak > 0.0 {
            let fill = [Color32::GRAY, Color32::RED, Color32::GREEN, Color32::BLUE][ci];
            let mut mesh = egui::Mesh::default();
            for (i, b) in channel.iter().enumerate() {
                let h = rect.height() * (b / peak).clamp(0.0, 1.0) as f32;
                let x = rect.min.x + i as f32 * rect.width() / 256.0;
                mesh.add_colored_rect(Rect::from_min_max(pos2(x, rect.max.y - h), pos2(x + rect.width() / 256.0 + 0.1, rect.max.y)), fill);
            }
            ui.painter().add(egui::Shape::mesh(mesh));
        }
    }
    // The input handles: black, gray and white, draggable.
    let (strip, response) = ui.allocate_exact_size(vec2(full, 20.0), Sense::drag());
    let x_of = |v: f64| strip.min.x + (v / 255.0) as f32 * full;
    let v_of = |x: f32| ((x - strip.min.x) / full * 255.0).clamp(0.0, 255.0) as f64;
    let gray_at = range.black + (range.white - range.black) * 0.5f64.powf(range.gamma);
    if response.dragged() {
        if let Some(pos) = response.interact_pointer_pos() {
            let v = v_of(pos.x);
            let nearest = [range.black, gray_at, range.white].iter().enumerate().min_by(|a, b| (a.1 - v).abs().total_cmp(&(b.1 - v).abs())).map(|(i, _)| i).unwrap();
            match nearest {
                0 => range.black = v.round().min(range.white - 2.0),
                2 => range.white = v.round().max(range.black + 2.0),
                _ => {
                    let t = ((v - range.black) / (range.white - range.black)).clamp(0.01, 0.99);
                    range.gamma = (t.ln() / 0.5f64.ln()).recip().clamp(0.1, 9.99);
                    range.gamma = (range.gamma * 100.0).round() / 100.0;
                }
            }
        }
    }
    for (i, v) in [range.black, gray_at, range.white].iter().enumerate() {
        let fill = [Color32::BLACK, Color32::GRAY, Color32::WHITE][i];
        triangle(ui.painter(), pos2(x_of(*v), strip.min.y + 9.0), fill);
    }
    ui.horizontal(|ui| {
        field(ui, "Input black", &mut range.black, 0.0..=253.0, w::fmt_int);
        field(ui, "Gamma", &mut range.gamma, 0.1..=9.99, |v| format!("{v:.2}"));
        field(ui, "Input white", &mut range.white, 2.0..=255.0, w::fmt_int);
    });
    let (bar, _) = ui.allocate_exact_size(vec2(full, 14.0), Sense::hover());
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(bar.left_top(), Color32::BLACK);
    mesh.colored_vertex(bar.right_top(), Color32::WHITE);
    mesh.colored_vertex(bar.right_bottom(), Color32::WHITE);
    mesh.colored_vertex(bar.left_bottom(), Color32::BLACK);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    ui.painter().add(egui::Shape::mesh(mesh));
    let (strip, response) = ui.allocate_exact_size(vec2(full, 20.0), Sense::drag());
    if response.dragged() {
        if let Some(pos) = response.interact_pointer_pos() {
            let v = v_of(pos.x).round();
            if (v - range.output_black).abs() <= (v - range.output_white).abs() {
                range.output_black = v;
            } else {
                range.output_white = v;
            }
        }
    }
    triangle(ui.painter(), pos2(strip.min.x + (range.output_black / 255.0) as f32 * full, strip.min.y + 9.0), Color32::BLACK);
    triangle(ui.painter(), pos2(strip.min.x + (range.output_white / 255.0) as f32 * full, strip.min.y + 9.0), Color32::WHITE);
    ui.horizontal(|ui| {
        field(ui, "Output black", &mut range.output_black, 0.0..=255.0, w::fmt_int);
        field(ui, "Output white", &mut range.output_white, 0.0..=255.0, w::fmt_int);
    });
    ui.horizontal(|ui| {
        w::text(ui, "Sample", theme::regular(10.0), color::secondary());
        for (i, name) in ["Black", "Gray", "White"].iter().enumerate() {
            let armed = f.sampling == Some(i as u8);
            if w::button(ui, name, 13.0, if armed { ButtonStyle::Prominent } else { ButtonStyle::Bordered }, ci == 0).clicked() {
                f.sampling = if armed { None } else { Some(i as u8) };
            }
        }
    });
    if let Some(i) = f.sampling {
        para(ui, &format!("Click the original layer to set {}. Click the eyedropper again to stop.", ["the black point", "the gray point", "the white point"][i as usize]), color::secondary());
    }
    ui.horizontal(|ui| {
        if w::button(ui, "Reset", 13.0, ButtonStyle::Bordered, true).clicked() {
            f.adjustment.levels = comp_format::LevelsSettings::default();
        }
    });
    para(ui, "Original pixels · alpha-weighted histogram", color::secondary());
}

fn field(ui: &mut Ui, title: &str, value: &mut f64, range: std::ops::RangeInclusive<f64>, fmt: fn(f64) -> String) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 5.0;
        w::scrub_label(ui, title, theme::regular(10.0), color::secondary(), value, 1.0, range.clone(), true);
        w::number_field(ui, ("levels", title), value, range, fmt, 80.0, true, true);
    });
}

fn triangle(p: &egui::Painter, center: egui::Pos2, fill: Color32) {
    let pts = vec![center + vec2(0.0, -5.5), center + vec2(6.0, 5.0), center + vec2(-6.0, 5.0)];
    p.add(egui::Shape::convex_polygon(pts.iter().map(|q| *q + vec2(0.0, 0.5)).collect(), theme::gray(0.5), Stroke::NONE));
    p.add(egui::Shape::convex_polygon(pts, fill, Stroke::NONE));
}

/// `HueSaturationSheet`: the range, then Hue, Saturation and Lightness for it; Colorize.
fn hue_saturation_controls(ui: &mut Ui, f: &mut FilterSheet) {
    const RANGES: [ColorRange; 7] = [ColorRange::Master, ColorRange::Reds, ColorRange::Yellows, ColorRange::Greens, ColorRange::Cyans, ColorRange::Blues, ColorRange::Magentas];
    let mut hsv = resolved_hsv(&f.adjustment);
    let before = hsv.clone();
    let mut ri = RANGES.iter().position(|r| *r == hsv.range).unwrap_or(0);
    ui.horizontal(|ui| {
        ui.add_enabled_ui(!hsv.colorize, |ui| {
            w::popup(ui, "hue-range", &mut ri, &[&[0usize, 1, 2, 3, 4, 5, 6]], |i| RANGES[i].name(), Some(160.0), !hsv.colorize);
        });
    });
    if hsv.colorize {
        ri = 0;
    }
    hsv.range = RANGES[ri];
    let mut adj = hsv.adjustments.get(hsv.range).copied().unwrap_or(comp_format::RangeAdjustment { hue: 0.0, saturation: 0.0, lightness: 0.0 });
    let colorize = hsv.colorize;
    let rows: [(&str, &mut f64, std::ops::RangeInclusive<f64>, &str); 3] = [
        ("Hue", &mut adj.hue, if colorize { 0.0..=360.0 } else { -180.0..=180.0 }, "°"),
        ("Saturation", &mut adj.saturation, if colorize { 0.0..=100.0 } else { -100.0..=100.0 }, ""),
        ("Lightness", &mut adj.lightness, -100.0..=100.0, ""),
    ];
    for (title, value, range, unit) in rows {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let (rect, response) = ui.allocate_exact_size(vec2(76.0, 22.0), Sense::click_and_drag());
            ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, title, theme::regular(13.0), color::label());
            if response.dragged() {
                *value = (*value + response.drag_delta().x as f64).clamp(*range.start(), *range.end());
            }
            if response.double_clicked() {
                *value = 0.0;
            }
            let unit_width = if unit.is_empty() { 0.0 } else { w::unit_width(ui, unit) };
            let sw = w::fill_width(ui, 48.0 + unit_width, if unit.is_empty() { 1 } else { 2 });
            w::slider(ui, value, range.clone(), sw, true);
            w::number_field(ui, ("hsl", title), value, range, w::fmt_int, 48.0, true, true);
            if !unit.is_empty() {
                w::unit(ui, unit);
            }
            *value = value.round();
        });
    }
    let mut set_colorize = hsv.colorize;
    let mut reset = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        w::checkbox(ui, &mut set_colorize, "Colorize");
        w::checkbox(ui, &mut f.preview, "Preview");
        reset = w::button(ui, "Reset", 13.0, ButtonStyle::Bordered, true).clicked();
    });
    // Write the range's values back.
    if let Some(slot) = hsv.adjustments.0.iter_mut().find(|(r, _)| *r == hsv.range) {
        slot.1 = adj;
    } else {
        hsv.adjustments.0.push((hsv.range, adj));
    }
    if set_colorize != hsv.colorize {
        // Colorize starts at hue 0 and saturation 25, as on the Mac.
        hsv.colorize = set_colorize;
        hsv.range = ColorRange::Master;
        let master = comp_format::RangeAdjustment { hue: 0.0, saturation: if set_colorize { 25.0 } else { 0.0 }, lightness: 0.0 };
        hsv.adjustments.0.retain(|(r, _)| *r != ColorRange::Master);
        hsv.adjustments.0.insert(0, (ColorRange::Master, master));
    }
    if reset {
        hsv = resolved_hsv(&Adjustment::new(AdjustmentKind::HueSaturation));
    }
    if hsv != before {
        let master = hsv.adjustments.get(ColorRange::Master).copied().unwrap_or(comp_format::RangeAdjustment { hue: 0.0, saturation: 0.0, lightness: 0.0 });
        f.adjustment.hue = master.hue;
        f.adjustment.saturation = master.saturation;
        f.adjustment.lightness = master.lightness;
        f.adjustment.colorize = hsv.colorize;
        f.adjustment.hsv_settings = Some(hsv);
    }
}

/// `LayerAdjustment.resolvedHSV`: the per-range settings, or the legacy Master fields.
fn resolved_hsv(a: &Adjustment) -> comp_format::HueSaturationSettings {
    a.hsv_settings.clone().unwrap_or_else(|| comp_format::HueSaturationSettings {
        range: ColorRange::Master,
        colorize: a.colorize,
        invert_range: false,
        adjustments: comp_format::RangeMap(vec![(ColorRange::Master, comp_format::RangeAdjustment { hue: a.hue, saturation: a.saturation, lightness: a.lightness })]),
        bands: comp_format::RangeMap(ColorRange::ALL.iter().map(|&r| (r, r.default_band())).collect()),
    })
}

// Canvas Size, Image Size, Trim.

pub struct CanvasSizeSheet {
    width: f64,
    height: f64,
    /// 0 pixels, 1 percent.
    units: usize,
    relative: bool,
    anchor: usize,
    /// Transparent, Foreground, Background, Black, White, Custom.
    extension: usize,
    custom: [f32; 3],
    error: Option<String>,
}

pub fn open_canvas_size(app: &mut App) {
    let Some(d) = app.doc() else { return };
    let (w0, h0) = (d.project.manifest.width as f64, d.project.manifest.height as f64);
    app.sheet = Some(Sheet::CanvasSize(CanvasSizeSheet { width: w0, height: h0, units: 0, relative: false, anchor: 4, extension: 0, custom: [1.0; 3], error: None }));
}

const ANCHORS: [&str; 9] = ["Top left", "Top center", "Top right", "Middle left", "Center", "Middle right", "Bottom left", "Bottom center", "Bottom right"];

fn bytes(n: i64) -> String {
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

fn canvas_size_sheet(app: &mut App, ctx: &egui::Context, s: &mut CanvasSizeSheet) -> bool {
    let Some(d) = app.doc() else { return false };
    let (w0, h0) = (d.project.manifest.width, d.project.manifest.height);
    let mut close = Close::Open;
    // The size the sheet asks for, in pixels.
    let pixels = |s: &CanvasSizeSheet| -> (i64, i64) {
        let (mut w, mut h) = if s.units == 1 { (s.width / 100.0 * w0 as f64, s.height / 100.0 * h0 as f64) } else { (s.width, s.height) };
        if s.relative && s.units == 0 {
            w += w0 as f64;
            h += h0 as f64;
        }
        (w.round() as i64, h.round() as i64)
    };
    window(ctx, "Canvas Size", 450.0, |ui| {
        title2(ui, "Canvas Size");
        w::text(ui, format!("Current: {w0} × {h0} pixels"), theme::regular(13.0), color::label());
        para(ui, &format!("{} uncompressed RGBA canvas", bytes(w0 * h0 * 4)), color::secondary());
        divider(ui);
        ui.horizontal(|ui| {
            w::text(ui, "Units", theme::regular(13.0), color::label());
            let before = s.units;
            w::popup(ui, "cs-units", &mut s.units, &[&[0usize, 1]], |i| ["Pixels", "Percent"][i], Some(200.0), true);
            if s.units != before {
                let (w, h) = if s.units == 1 { (100.0, 100.0) } else { (w0 as f64, h0 as f64) };
                s.width = w;
                s.height = h;
                s.relative = false;
            }
        });
        for (label, value) in [("Width", &mut s.width), ("Height", &mut s.height)] {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(60.0, 22.0), Sense::hover());
                ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, label, theme::regular(13.0), color::label());
                let wd = w::fill_width(ui, 0.0, 0);
                w::number_field(ui, ("canvas-size", label), value, -30000.0..=30000.0, w::fmt_trim2, wd, false, true);
            });
        }
        let was = s.relative;
        w::checkbox(ui, &mut s.relative, "Relative to current dimensions");
        if s.relative != was && s.units == 0 {
            if s.relative {
                s.width -= w0 as f64;
                s.height -= h0 as f64;
            } else {
                s.width += w0 as f64;
                s.height += h0 as f64;
            }
        }
        let (nw, nh) = pixels(s);
        let valid = (1..=30_000).contains(&nw) && (1..=30_000).contains(&nh);
        if valid {
            para(ui, &format!("New: {nw} × {nh} pixels · {} uncompressed", bytes(nw * nh * 4)), color::secondary());
        } else {
            para(ui, "Enter a size from 1 to 30,000 pixels on each side.", color::ORANGE);
        }
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing = vec2(24.0, 8.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                w::text(ui, "Anchor", theme::regular(13.0), color::label());
                egui::Grid::new("anchors").spacing(vec2(3.0, 3.0)).show(ui, |ui| {
                    for row in 0..3 {
                        for column in 0..3 {
                            let index = row * 3 + column;
                            let (rect, response) = ui.allocate_exact_size(vec2(33.0, 25.0), Sense::click());
                            ui.painter().rect_filled(rect, egui::CornerRadius::same(12), color::control());
                            let on = index == s.anchor;
                            crate::icons::paint(ui.painter(), crate::icons::Icon::Symbol(if on { "circle.fill" } else { "circle" }), rect.center(), 13.0, if on { color::ACCENT } else { color::secondary() });
                            if response.on_hover_text(ANCHORS[index]).clicked() {
                                s.anchor = index;
                            }
                        }
                        ui.end_row();
                    }
                });
            });
            ui.vertical(|ui| {
                ui.add_space(28.0);
                w::text(ui, ANCHORS[s.anchor], theme::bold(12.0), color::label());
                para(ui, "Keeps this point fixed. Artwork is not scaled; cropped content remains outside the canvas.", color::secondary());
            });
        });
        ui.horizontal(|ui| {
            w::text(ui, "Canvas extension", theme::regular(13.0), color::label());
            w::popup(ui, "cs-extension", &mut s.extension, &[&[0usize, 1, 2, 3, 4, 5]], |i| ["Transparent", "Foreground", "Background", "Black", "White", "Custom"][i], Some(200.0), true);
        });
        if s.extension == 5 {
            ui.horizontal(|ui| {
                w::text(ui, "Extension color", theme::regular(13.0), color::label());
                egui::widgets::color_picker::color_edit_button_rgb(ui, &mut s.custom);
            });
        }
        if let Some(e) = &s.error {
            para(ui, e, color::ORANGE);
        }
        close = footer(ui, "OK", valid);
    });
    match close {
        Close::Open => true,
        Close::Cancel => false,
        Close::Ok => {
            let (nw, nh) = pixels(s);
            let fill = match s.extension {
                1 => Some(app.settings.foreground),
                2 => Some(app.settings.background),
                3 => Some([0.0; 3]),
                4 => Some([1.0; 3]),
                5 => Some(s.custom),
                _ => None,
            };
            let mut op = json!({ "op": "canvasSize", "width": nw, "height": nh, "anchor": s.anchor });
            if let Some(c) = fill {
                op["fill"] = json!(c.map(|v| v as f64));
            }
            run_document_op(app, "Canvas Size", &op).map_or_else(|e| {
                s.error = Some(e);
                true
            }, |_| false)
        }
    }
}

/// Runs a document op (crop, canvas size, image size) as one undo step and fits the view.
fn run_document_op(app: &mut App, title: &str, op: &Value) -> Result<(), String> {
    let engine = app.gfx.engine.clone();
    let Some(doc) = app.doc_mut() else { return Ok(()) };
    doc.apply(title, |p| engine.apply_op(p, op)).map_err(|e| describe(&e))?;
    // The selection doesn't survive a resize here.
    doc.selection = None;
    doc.ants = None;
    let size = doc.size();
    doc.view.fit(size);
    Ok(())
}

pub struct ImageSizeSheet {
    width: f64,
    height: f64,
    lock: bool,
    resolution: f64,
    resample: bool,
    sampling: crate::tools::Sampling,
    error: Option<String>,
}

pub fn open_image_size(app: &mut App) {
    let Some(d) = app.doc() else { return };
    let m = &d.project.manifest;
    app.sheet = Some(Sheet::ImageSize(ImageSizeSheet {
        width: m.width as f64,
        height: m.height as f64,
        lock: true,
        resolution: m.resolution.unwrap_or(72.0),
        resample: true,
        sampling: crate::tools::Sampling::High,
        error: None,
    }));
}

fn image_size_sheet(app: &mut App, ctx: &egui::Context, s: &mut ImageSizeSheet) -> bool {
    let Some(d) = app.doc() else { return false };
    let (w0, h0) = (d.project.manifest.width as f64, d.project.manifest.height as f64);
    let mut close = Close::Open;
    window(ctx, "Image Size", 430.0, |ui| {
        ui.spacing_mut().item_spacing.y = 18.0;
        title2(ui, "Image Size");
        w::text(ui, format!("Current: {} × {} pixels", w0 as i64, h0 as i64), theme::regular(13.0), color::secondary());
        let (pw, ph) = (s.width, s.height);
        for (label, value) in [("Width", &mut s.width), ("Height", &mut s.height)] {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(75.0, 22.0), Sense::hover());
                ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, label, theme::regular(13.0), color::label());
                let wd = w::fill_width(ui, 0.0, 0);
                w::number_field(ui, ("image-size", label), value, 1.0..=30000.0, w::fmt_trim2, wd, false, s.resample);
            });
        }
        if s.lock {
            if s.width != pw {
                s.height = (s.width * h0 / w0).round().max(1.0);
            } else if s.height != ph {
                s.width = (s.height * w0 / h0).round().max(1.0);
            }
        }
        w::checkbox(ui, &mut s.lock, "Lock aspect ratio");
        ui.horizontal(|ui| {
            w::text(ui, "Resolution", theme::regular(13.0), color::label());
            let wd = w::fill_width(ui, 80.0, 1);
            w::number_field(ui, "resolution", &mut s.resolution, 1.0..=9600.0, w::fmt_trim2, wd, false, true);
            w::text(ui, "pixels/inch", theme::regular(13.0), color::secondary());
        });
        w::checkbox(ui, &mut s.resample, "Resample");
        if s.resample {
            ui.horizontal(|ui| {
                w::text(ui, "Sampling", theme::regular(13.0), color::label());
                w::popup(ui, "is-sampling", &mut s.sampling, &[crate::tools::Sampling::ALL], crate::tools::Sampling::title, Some(200.0), true);
            });
            para(ui, "Resizes layer pixels and applies existing transforms. Undo restores the originals.", color::secondary());
        } else {
            s.width = w0;
            s.height = h0;
            para(ui, "Only print dimensions and resolution change. Pixels stay unchanged.", color::secondary());
        }
        para(ui, &format!("Result: {} × {} pixels", s.width.round() as i64, s.height.round() as i64), color::secondary());
        if let Some(e) = &s.error {
            para(ui, e, color::ORANGE);
        }
        close = footer(ui, "Resize", true);
    });
    match close {
        Close::Open => true,
        Close::Cancel => false,
        Close::Ok => {
            let sampling = match s.sampling {
                crate::tools::Sampling::Nearest => "Nearest",
                crate::tools::Sampling::Smooth => "Smooth",
                crate::tools::Sampling::High => "High quality",
            };
            let op = json!({ "op": "imageSize", "width": s.width.round(), "height": s.height.round(), "resolution": s.resolution, "sampling": sampling });
            run_document_op(app, "Image Size", &op).map_or_else(|e| {
                s.error = Some(e);
                true
            }, |_| false)
        }
    }
}

pub struct TrimSheet {
    based: usize,
    sides: [bool; 4],
}

impl Default for TrimSheet {
    fn default() -> Self {
        Self { based: 0, sides: [true; 4] }
    }
}

/// `ImageTrim.calculateTrimRect` over the flattened canvas (premultiplied): [x, y, w, h].
fn trim_rect(pixels: &[u8], width: usize, height: usize, based: usize, sides: [bool; 4]) -> Option<[f64; 4]> {
    let [top, bottom, left, right] = sides;
    let px = |x: usize, y: usize| &pixels[(y * width + x) * 4..(y * width + x) * 4 + 4];
    let matches: Box<dyn Fn(usize, usize) -> bool> = match based {
        0 => Box::new(|x, y| px(x, y)[3] == 0),
        _ => {
            let (sx, sy) = if based == 1 { (0, 0) } else { (width - 1, height - 1) };
            let target: Vec<u8> = px(sx, sy).to_vec();
            Box::new(move |x, y| px(x, y) == target.as_slice())
        }
    };
    let (mut l, mut r, mut t, mut b) = (width, 0, height, 0);
    for y in 0..height {
        let Some(first) = (0..width).find(|&x| !matches(x, y)) else { continue };
        let last = (first..width).rev().find(|&x| !matches(x, y)).unwrap() + 1;
        l = l.min(first);
        r = r.max(last);
        t = t.min(y);
        b = y + 1;
    }
    if r == 0 {
        return None;
    }
    let (x0, y0) = (if left { l } else { 0 }, if top { t } else { 0 });
    let (x1, y1) = (if right { r } else { width }, if bottom { b } else { height });
    (x1 > x0 && y1 > y0).then(|| [x0 as f64, y0 as f64, (x1 - x0) as f64, (y1 - y0) as f64])
}

fn radio(ui: &mut Ui, on: bool, title: &str) -> bool {
    let galley = ui.painter().layout_no_wrap(title.to_string(), theme::regular(13.0), color::label());
    let (rect, response) = ui.allocate_exact_size(vec2(14.0 + 6.0 + galley.size().x, 16.0), Sense::click());
    let c = rect.left_center() + vec2(7.0, 0.0);
    if on {
        ui.painter().circle_filled(c, 7.0, color::ACCENT);
        ui.painter().circle_filled(c, 2.5, Color32::WHITE);
    } else {
        ui.painter().circle_filled(c, 7.0, theme::white_alpha(0.08));
        ui.painter().circle_stroke(c, 6.5, Stroke::new(1.0, theme::white_alpha(0.22)));
    }
    ui.painter().galley(pos2(rect.min.x + 20.0, rect.center().y - galley.size().y / 2.0), galley, color::label());
    response.clicked()
}

fn trim_sheet(app: &mut App, ctx: &egui::Context, s: &mut TrimSheet) -> bool {
    let mut close = Close::Open;
    window(ctx, "Trim", 320.0, |ui| {
        title2(ui, "Trim");
        w::text(ui, "Based On", theme::semibold(13.0), color::label());
        for (i, option) in ["Transparent Pixels", "Top Left Pixel Color", "Bottom Right Pixel Color"].iter().enumerate() {
            if radio(ui, s.based == i, option) {
                s.based = i;
            }
        }
        divider(ui);
        w::text(ui, "Trim Away", theme::semibold(13.0), color::label());
        egui::Grid::new("trim-away").spacing(vec2(24.0, 8.0)).show(ui, |ui| {
            let [top, bottom, left, right] = &mut s.sides;
            w::checkbox(ui, top, "Top");
            w::checkbox(ui, bottom, "Bottom");
            ui.end_row();
            w::checkbox(ui, left, "Left");
            w::checkbox(ui, right, "Right");
            ui.end_row();
        });
        divider(ui);
        close = footer(ui, "OK", s.sides.iter().any(|b| *b));
    });
    match close {
        Close::Open => true,
        Close::Cancel => false,
        Close::Ok => {
            let gfx = app.gfx.clone();
            let Some(d) = app.doc() else { return false };
            let (w, h) = (d.project.manifest.width as usize, d.project.manifest.height as usize);
            let pixels = engine::composite::Compositor::new(&gfx.gpu, &d.project).render().ok().and_then(|c| gfx.gpu.download(&c).ok());
            let Some(pixels) = pixels else { return false };
            if let Some(rect) = trim_rect(&pixels, w, h, s.based, s.sides) {
                if rect != [0.0, 0.0, w as f64, h as f64] {
                    let op = json!({ "op": "crop", "rect": rect });
                    if let Err(e) = run_document_op(app, "Trim", &op) {
                        app.alert("Couldn’t trim image", e);
                    }
                }
            }
            false
        }
    }
}

// Export JPEG.

pub struct JpegSheet {
    quality: f64,
    matte: [f32; 3],
    zoom: Option<f32>,
    /// The encoded file and its decoded preview, for the settings they were made with.
    encoded: Option<((u64, u64), Vec<u8>, egui::TextureHandle)>,
    error: Option<String>,
}

pub fn open_export_jpeg(app: &mut App) {
    if app.doc().is_none() {
        return;
    }
    let quality = app.jpeg_quality;
    app.sheet = Some(Sheet::ExportJpeg(Box::new(JpegSheet { quality, matte: [1.0; 3], zoom: None, encoded: None, error: None })));
}

fn jpeg_sheet(app: &mut App, ctx: &egui::Context, s: &mut JpegSheet) -> bool {
    let Some(doc) = app.doc() else { return false };
    let (w0, h0) = (doc.project.manifest.width as u32, doc.project.manifest.height as u32);
    let key = ((s.quality * 100.0).round() as u64, s.matte.iter().fold(0u64, |k, c| k * 256 + (c * 255.0).round() as u64));
    if s.encoded.as_ref().is_none_or(|(k, _, _)| *k != key) {
        let options = json!({ "quality": (s.quality * 100.0).round() / 100.0, "matte": s.matte.map(|c| c as f64) });
        match app.gfx.engine.export_jpeg(&doc.project, &options) {
            Ok(bytes) => {
                s.error = None;
                if let Ok(decoded) = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg) {
                    let rgba = decoded.to_rgba8();
                    let image = egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
                    let texture = ctx.load_texture("jpeg-preview", image, egui::TextureOptions::NEAREST);
                    s.encoded = Some((key, bytes, texture));
                }
            }
            Err(e) => {
                s.error = Some(describe(&e));
                s.encoded = None;
            }
        }
    }
    let mut close = Close::Open;
    window(ctx, "Export JPEG", 608.0, |ui| {
        ui.horizontal(|ui| {
            title2(ui, "Export JPEG");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if w::button(ui, "−", 13.0, ButtonStyle::Bordered, true).clicked() {
                    s.zoom = Some((s.zoom.unwrap_or(1.0) / 2.0).max(0.25));
                }
                if w::button(ui, "+", 13.0, ButtonStyle::Bordered, true).clicked() {
                    s.zoom = Some((s.zoom.unwrap_or(1.0) * 2.0).min(8.0));
                }
                if w::button(ui, "Fit", 13.0, ButtonStyle::Bordered, true).clicked() {
                    s.zoom = None;
                }
            });
        });
        let (rect, _) = ui.allocate_exact_size(vec2(560.0, 330.0), Sense::hover());
        ui.painter().rect_filled(rect, 0.0, theme::gray(0.12));
        if let Some((_, _, texture)) = &s.encoded {
            let fit = (rect.width() / w0 as f32).min(rect.height() / h0 as f32).min(1.0);
            let scale = s.zoom.unwrap_or(fit);
            let shown = Rect::from_center_size(rect.center(), vec2(w0 as f32 * scale, h0 as f32 * scale));
            ui.painter().with_clip_rect(rect).image(texture.id(), shown, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        ui.horizontal(|ui| {
            w::text(ui, "Quality", theme::regular(13.0), color::label());
            let sw = w::fill_width(ui, 45.0, 1);
            w::slider(ui, &mut s.quality, 0.0..=1.0, sw, true);
            s.quality = (s.quality * 100.0).round() / 100.0;
            w::text(ui, format!("{}%", (s.quality * 100.0).round()), egui::FontId::monospace(12.0), color::label());
        });
        ui.horizontal(|ui| {
            w::text(ui, "Background for transparency", theme::regular(13.0), color::label());
            egui::widgets::color_picker::color_edit_button_rgb(ui, &mut s.matte);
        });
        ui.horizontal(|ui| {
            w::text(ui, format!("{w0} × {h0} px · sRGB"), theme::regular(13.0), color::secondary());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if w::button(ui, "Export…", 13.0, ButtonStyle::Prominent, s.encoded.is_some()).clicked() {
                    close = Close::Ok;
                }
                if w::button(ui, "Cancel", 13.0, ButtonStyle::Bordered, true).clicked() {
                    close = Close::Cancel;
                }
                match (&s.error, &s.encoded) {
                    (Some(e), _) => {
                        w::text(ui, e, theme::regular(12.0), color::RED);
                    }
                    (None, Some((_, bytes, _))) => {
                        w::text(ui, bytes_file(bytes.len()), theme::regular(12.0), color::secondary());
                    }
                    _ => {}
                }
            });
        });
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            close = Close::Cancel;
        }
    });
    match close {
        Close::Open => true,
        Close::Cancel => false,
        Close::Ok => {
            app.jpeg_quality = s.quality;
            let name = app.doc().map(|d| d.name.clone()).unwrap_or_default();
            let Some((_, bytes, _)) = &s.encoded else { return true };
            if app.headless {
                return false;
            }
            if let Some(path) = rfd::FileDialog::new().add_filter("JPEG image", &["jpg", "jpeg"]).set_file_name(format!("{name}.jpg")).save_file() {
                if let Err(e) = std::fs::write(&path, bytes) {
                    app.alert("Couldn’t export JPEG", e.to_string());
                }
            }
            false
        }
    }
}

/// `ByteCountFormatter` with `.file`: powers of 1000.
fn bytes_file(n: usize) -> String {
    let n = n as f64;
    if n < 1000.0 {
        format!("{} bytes", n as i64)
    } else if n < 1e6 {
        format!("{} KB", (n / 1e3).round() as i64)
    } else {
        format!("{:.1} MB", n / 1e6)
    }
}

// The Photoshop conversion report.

fn psd_sheet(app: &mut App, ctx: &egui::Context, name: &str, conversions: &[(String, String)], project: &mut Box<Project>, into_document: bool) -> bool {
    let mut close = Close::Open;
    window(ctx, "Photoshop", 520.0, |ui| {
        title2(ui, &format!("Open “{name}”?"));
        para(ui, "Compositor will convert these Photoshop features. Nothing is applied until you continue.", color::secondary());
        egui::ScrollArea::vertical().max_height(260.0).min_scrolled_height(180.0).show(ui, |ui| {
            for (layer, message) in conversions {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    w::text(ui, layer, theme::semibold(13.0), color::label());
                    para(ui, message, color::label());
                });
            }
        });
        close = footer(ui, "Import", true);
    });
    match close {
        Close::Open => true,
        Close::Cancel => false,
        Close::Ok => {
            let project = std::mem::replace(project.as_mut(), Doc::blank(String::new(), 1, 1).project);
            app.insert_psd(name, project, into_document);
            false
        }
    }
}

// Rename, Keyboard Shortcuts, the selection amount.

fn rename_sheet(app: &mut App, ctx: &egui::Context, id: &str, name: &mut String) -> bool {
    let mut close = Close::Open;
    window(ctx, "Rename Layer", 340.0, |ui| {
        title2(ui, "Rename Layer");
        let response = w::text_field(ui, "rename", name, ui.available_width(), "Name", theme::regular(13.0));
        if !response.has_focus() && !response.lost_focus() {
            response.request_focus();
        }
        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            close = Close::Ok;
        }
        let answer = footer(ui, "Rename", !name.trim().is_empty());
        if !matches!(answer, Close::Open) {
            close = answer;
        }
    });
    match close {
        Close::Open => true,
        Close::Cancel => false,
        Close::Ok => {
            if let Some(d) = app.doc_mut() {
                crate::layer_ops::rename(d, id, name);
            }
            false
        }
    }
}

fn shortcuts_sheet(ctx: &egui::Context, search: &mut String) -> bool {
    let mut open = true;
    window(ctx, "Keyboard Shortcuts", 660.0, |ui| {
        ui.spacing_mut().item_spacing.y = 10.0;
        para(ui, "These are the default shortcuts. Ctrl stands for the Mac's Command key, and Alt for Option.", color::secondary());
        let wd = ui.available_width();
        w::text_field(ui, "shortcut-search", search, wd, "Search shortcuts", theme::regular(13.0));
        egui::ScrollArea::vertical().max_height(465.0).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            for (group, rows) in crate::menus::shortcut_list() {
                ui.add_space(8.0);
                w::text(ui, group, theme::semibold(13.0), color::label());
                for (title, chord) in rows {
                    if !search.is_empty() && !title.to_lowercase().contains(&search.to_lowercase()) {
                        continue;
                    }
                    ui.horizontal(|ui| {
                        w::text(ui, &title, theme::regular(13.0), color::label());
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            w::text(ui, &chord, theme::regular(13.0), color::secondary());
                        });
                    });
                }
            }
        });
        divider(ui);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if w::button(ui, "Done", 13.0, ButtonStyle::Prominent, true).clicked() {
                open = false;
            }
        });
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            open = false;
        }
    });
    open
}

fn modify_sheet(app: &mut App, ctx: &egui::Context, kind: u8, amount: &mut f64) -> bool {
    let max = if kind == 2 { 250.0 } else { 500.0 };
    let title = ["Expand Selection", "Contract Selection", "Feather Selection"][kind as usize];
    let mut close = Close::Open;
    window(ctx, title, 380.0, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            w::scrub_label(ui, "Amount", theme::regular(13.0), color::label(), amount, 1.0, 1.0..=max, true);
            let sw = w::fill_width(ui, 56.0 + w::unit_width(ui, "px"), 2);
            w::slider(ui, amount, 1.0..=max, sw, true);
            *amount = amount.round();
            w::number_field(ui, "selection-amount", amount, 1.0..=max, w::fmt_int, 56.0, true, true);
            w::unit(ui, "px");
        });
        divider(ui);
        close = footer(ui, "OK", true);
    });
    match close {
        Close::Open => true,
        Close::Cancel => false,
        Close::Ok => {
            crate::ui::selection::modify(app, kind, *amount);
            false
        }
    }
}

fn color_range_sheet(app: &mut App, ctx: &egui::Context, s: &mut crate::ui::selection::ColorRange) -> bool {
    // The preview: what the settings would select, black and white.
    let key = (s.samples.len(), s.fuzziness.round() as i64, s.invert);
    if s.previewed != Some(key) {
        s.previewed = Some(key);
        s.preview = None;
        if !s.samples.is_empty() {
            let engine = app.gfx.engine.clone();
            if let Some(doc) = app.doc() {
                let mut project = doc.project.clone();
                let mut selection = None;
                if engine.apply_op_with_selection(&mut project, &mut selection, &s.op(true)).is_ok() {
                    let gray = match &selection {
                        Some(sel) => engine.selection_coverage(&project, sel).ok(),
                        None => Some(image::GrayImage::new(project.manifest.width as u32, project.manifest.height as u32)),
                    };
                    if let Some(gray) = gray {
                        let image = egui::ColorImage::from_gray([gray.width() as usize, gray.height() as usize], gray.as_raw());
                        s.preview = Some(ctx.load_texture("color-range", image, egui::TextureOptions::LINEAR));
                    }
                }
            }
        }
    }
    let canvas = app.doc().map_or(vec2(1.0, 1.0), |d| d.size());
    let mut close = Close::Open;
    window(ctx, "Color Range", 340.0, |ui| {
        let scale = (292.0 / canvas.x).min(200.0 / canvas.y);
        let (rect, _) = ui.allocate_exact_size(canvas * scale, Sense::hover());
        ui.painter().rect_filled(rect, 0.0, Color32::BLACK);
        if let Some(t) = &s.preview {
            ui.painter().image(t.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, theme::white_alpha(0.2)), egui::StrokeKind::Inside);
        let hint = if s.samples.is_empty() { "Click the image to pick the color to select." } else { "Shift-click adds a color, Alt-click takes one away." };
        para(ui, hint, color::secondary());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            w::scrub_label(ui, "Fuzziness", theme::regular(13.0), color::label(), &mut s.fuzziness, 1.0, 0.0..=200.0, true);
            let sw = w::fill_width(ui, 48.0, 1);
            w::slider(ui, &mut s.fuzziness, 0.0..=200.0, sw, true);
            w::number_field(ui, "fuzziness", &mut s.fuzziness, 0.0..=200.0, w::fmt_int, 48.0, true, true);
        });
        w::checkbox(ui, &mut s.invert, "Invert");
        divider(ui);
        close = footer(ui, "OK", true);
    });
    match close {
        Close::Open => true,
        Close::Cancel => false,
        Close::Ok => {
            if !s.samples.is_empty() {
                let op = s.op(app.settings.anti_alias);
                crate::ui::selection::run(app, "Color Range", op);
            }
            false
        }
    }
}

// Layer effects.

pub struct EffectSheet {
    kind: &'static str,
    layer: String,
    effects: comp_format::Effects,
    original: Option<comp_format::Effects>,
}

/// The effect's defaults (`EffectsSheet`): stroke in the background color, black shadows, white glows.
fn default_effect(effects: &mut comp_format::Effects, kind: &str, background: [f32; 3]) {
    let [r, g, b] = background.map(|c| c as f64);
    match kind {
        "Stroke" => effects.stroke = Some(comp_format::StrokeEffect { enabled: None, size: 4.0, red: r, green: g, blue: b, opacity: 1.0, inside: false }),
        "Drop Shadow" => effects.shadow = Some(comp_format::ShadowEffect { enabled: None, angle: 90.0, distance: 20.0, blur: 20.0, red: 0.0, green: 0.0, blue: 0.0, opacity: 0.5 }),
        "Color Overlay" => effects.color_overlay = Some(comp_format::ColorOverlayEffect { enabled: None, red: r, green: g, blue: b, opacity: 1.0 }),
        "Inner Shadow" => effects.inner_shadow = Some(comp_format::ShadowEffect { enabled: None, angle: 90.0, distance: 10.0, blur: 10.0, red: 0.0, green: 0.0, blue: 0.0, opacity: 0.5 }),
        "Outer Glow" => effects.outer_glow = Some(comp_format::GlowEffect { enabled: None, size: 20.0, red: 1.0, green: 1.0, blue: 1.0, opacity: 0.75 }),
        "Inner Glow" => effects.inner_glow = Some(comp_format::GlowEffect { enabled: None, size: 10.0, red: 1.0, green: 1.0, blue: 1.0, opacity: 0.75 }),
        _ => {}
    }
}

/// The layer effects menu (the footer's sparkles): add the effect, or edit it if it's there.
pub fn open_effect(app: &mut App, kind: &'static str) {
    let background = app.settings.background;
    let Some(layer) = app.doc().and_then(|d| d.active_layer()).filter(|l| l.adjustment.is_none()).cloned() else { return };
    let original = layer.effects.clone();
    let mut effects = original.clone().unwrap_or_default();
    let present = match kind {
        "Stroke" => effects.stroke.is_some(),
        "Drop Shadow" => effects.shadow.is_some(),
        "Color Overlay" => effects.color_overlay.is_some(),
        "Inner Shadow" => effects.inner_shadow.is_some(),
        "Outer Glow" => effects.outer_glow.is_some(),
        "Inner Glow" => effects.inner_glow.is_some(),
        _ => return,
    };
    if !present {
        default_effect(&mut effects, kind, background);
    }
    app.sheet = Some(Sheet::Effect(EffectSheet { kind, layer: layer.id, effects, original }));
}

fn effect_row(ui: &mut Ui, title: &str, value: &mut f64, slider_range: std::ops::RangeInclusive<f64>, typed: std::ops::RangeInclusive<f64>, unit: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let (rect, response) = ui.allocate_exact_size(vec2(64.0, 22.0), Sense::drag());
        ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, title, theme::regular(13.0), color::label());
        if response.dragged() {
            *value = (*value + response.drag_delta().x as f64).clamp(*typed.start(), *typed.end());
        }
        let mut shown = value.clamp(*slider_range.start(), *slider_range.end());
        if w::slider(ui, &mut shown, slider_range, 130.0, true).changed() {
            *value = shown.round();
        }
        w::number_field(ui, ("effect", title), value, typed, w::fmt_int, 48.0, true, true);
        w::unit(ui, unit);
    });
}

fn color_row(ui: &mut Ui, title: &str, r: &mut f64, g: &mut f64, b: &mut f64) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(64.0, 22.0), Sense::hover());
        ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, title, theme::regular(13.0), color::label());
        let mut rgb = [*r as f32, *g as f32, *b as f32];
        if egui::widgets::color_picker::color_edit_button_rgb(ui, &mut rgb).changed() {
            (*r, *g, *b) = (rgb[0] as f64, rgb[1] as f64, rgb[2] as f64);
        }
    });
}

fn effect_sheet(app: &mut App, ctx: &egui::Context, s: &mut EffectSheet) -> bool {
    let before = s.effects.clone();
    let mut close = Close::Open;
    window(ctx, s.kind, 340.0, |ui| {
        w::text(ui, s.kind, theme::semibold(13.0), color::label());
        let e = &mut s.effects;
        match s.kind {
            "Stroke" => {
                let x = e.stroke.as_mut().unwrap();
                ui.horizontal(|ui| {
                    let mut inside = x.inside as usize;
                    if w::segmented(ui, &mut inside, &[0, 1], |i| ["Outside", "Inside"][i]).changed() {
                        x.inside = inside == 1;
                    }
                });
                color_row(ui, "Color", &mut x.red, &mut x.green, &mut x.blue);
                effect_row(ui, "Size", &mut x.size, 0.0..=20.0, 0.0..=500.0, "px");
                let mut o = (x.opacity * 100.0).round();
                effect_row(ui, "Opacity", &mut o, 0.0..=100.0, 0.0..=100.0, "%");
                x.opacity = o / 100.0;
            }
            "Drop Shadow" | "Inner Shadow" => {
                let inner = s.kind == "Inner Shadow";
                let x = if inner { e.inner_shadow.as_mut().unwrap() } else { e.shadow.as_mut().unwrap() };
                color_row(ui, "Color", &mut x.red, &mut x.green, &mut x.blue);
                let mut o = (x.opacity * 100.0).round();
                effect_row(ui, "Opacity", &mut o, 0.0..=100.0, 0.0..=100.0, "%");
                x.opacity = o / 100.0;
                effect_row(ui, "Angle", &mut x.angle, -180.0..=180.0, -180.0..=180.0, "°");
                effect_row(ui, "Distance", &mut x.distance, if inner { 0.0..=50.0 } else { 0.0..=100.0 }, 0.0..=5000.0, "px");
                effect_row(ui, "Blur", &mut x.blur, 0.0..=100.0, 0.0..=500.0, "px");
            }
            "Color Overlay" => {
                let x = e.color_overlay.as_mut().unwrap();
                color_row(ui, "Color", &mut x.red, &mut x.green, &mut x.blue);
                let mut o = (x.opacity * 100.0).round();
                effect_row(ui, "Opacity", &mut o, 0.0..=100.0, 0.0..=100.0, "%");
                x.opacity = o / 100.0;
            }
            _ => {
                let x = if s.kind == "Outer Glow" { e.outer_glow.as_mut().unwrap() } else { e.inner_glow.as_mut().unwrap() };
                color_row(ui, "Color", &mut x.red, &mut x.green, &mut x.blue);
                effect_row(ui, "Size", &mut x.size, 0.0..=100.0, 0.0..=500.0, "px");
                let mut o = (x.opacity * 100.0).round();
                effect_row(ui, "Opacity", &mut o, 0.0..=100.0, 0.0..=100.0, "%");
                x.opacity = o / 100.0;
            }
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if w::button(ui, "OK", 13.0, ButtonStyle::Bordered, true).clicked() {
                    close = Close::Ok;
                }
                if w::button(ui, "Cancel", 13.0, ButtonStyle::Bordered, true).clicked() {
                    close = Close::Cancel;
                }
            });
        });
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            close = Close::Cancel;
        }
    });
    // Preview on the canvas as it changes.
    let layer = s.layer.clone();
    let effects = s.effects.clone();
    let Some(doc) = app.doc_mut() else { return false };
    if s.effects != before || doc.preview.is_none() {
        let mut p = doc.project.clone();
        if let Some(l) = p.manifest.layers.iter_mut().find(|l| l.id == layer) {
            l.effects = Some(effects.clone());
        }
        doc.set_preview(Some(p));
    }
    match close {
        Close::Open => true,
        Close::Cancel => {
            doc.set_preview(None);
            false
        }
        Close::Ok => {
            let title = if s.original.as_ref().is_some_and(|o| *o != comp_format::Effects::default()) { "Edit Layer Effect" } else { "Add Layer Effect" };
            doc.edit(title, false, |m| {
                if let Some(l) = m.layers.iter_mut().find(|l| l.id == layer) {
                    l.effects = Some(effects);
                }
            });
            doc.set_preview(None);
            false
        }
    }
}

// The color picker.

/// The Color Picker panel for the foreground or background color.
pub fn open_color_picker(app: &mut App, background: bool) {
    let original = if background { app.settings.background } else { app.settings.foreground };
    app.sheet = Some(Sheet::ColorPicker { background, color: egui::ecolor::Hsva::from_rgb(original), original });
}

fn color_picker_sheet(app: &mut App, ctx: &egui::Context, background: bool, color: &mut egui::ecolor::Hsva, original: [f32; 3]) -> bool {
    let title = if background { "Color Picker (Background Color)" } else { "Color Picker (Foreground Color)" };
    let mut close = Close::Open;
    window(ctx, title, 520.0, |ui| {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            egui::widgets::color_picker::color_picker_hsva_2d(ui, color, egui::widgets::color_picker::Alpha::Opaque);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                let rgb = color.to_rgb();
                let (preview, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
                ui.painter().rect_filled(preview, 5.0, w::rgb(rgb));
                let mut values = rgb.map(|c| (c * 255.0).round() as f64);
                let before = values;
                for (i, label) in ["R", "G", "B"].iter().enumerate() {
                    ui.horizontal(|ui| {
                        w::text(ui, *label, theme::regular(13.0), color::label());
                        w::number_field(ui, ("picker", *label), &mut values[i], 0.0..=255.0, w::fmt_int, 52.0, false, true);
                    });
                }
                if values != before {
                    *color = egui::ecolor::Hsva::from_rgb(values.map(|v| (v / 255.0) as f32));
                }
                let mut hex = format!("{:02X}{:02X}{:02X}", values[0] as u8, values[1] as u8, values[2] as u8);
                ui.horizontal(|ui| {
                    w::text(ui, "#", theme::regular(13.0), color::label());
                    if w::text_field(ui, "picker-hex", &mut hex, 84.0, "", egui::FontId::monospace(12.0)).changed() {
                        if let (6, Ok(v)) = (hex.len(), u32::from_str_radix(hex.trim(), 16)) {
                            let c = [(v >> 16) & 255, (v >> 8) & 255, v & 255].map(|x| x as f32 / 255.0);
                            *color = egui::ecolor::Hsva::from_rgb(c);
                        }
                    }
                });
                para(ui, "Click the canvas to sample", color::secondary());
            });
        });
        divider(ui);
        close = footer(ui, "OK", true);
    });
    match close {
        Close::Open => true,
        Close::Cancel => {
            let _ = original;
            false
        }
        Close::Ok => {
            let rgb = color.to_rgb();
            if background {
                app.settings.background = rgb;
            } else {
                app.settings.foreground = rgb;
            }
            false
        }
    }
}
