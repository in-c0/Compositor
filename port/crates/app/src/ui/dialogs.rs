//! The working sheets and floating panels: filters and image adjustments with live preview
//! (`FilterSheet`, `LevelsSheet`, `HueSaturationSheet`, `CurvesControls`), adjustment layers,
//! Canvas Size, Image Size, Trim, Export JPEG, the Photoshop conversion report, layer effects,
//! Rename, the selection amount and Color Range. Each ends in the engine's own operation.
//!
//! They follow the Mac's layout and metrics (the helpers in `sheets.rs`), and `--render-ui` draws
//! them for the parity states; `sheets.rs` keeps the sheets the port has no live version of.

use crate::app::App;
use crate::document::Doc;
use crate::gfx::Gfx;
use crate::theme::{self, color, metric};
use crate::ui::sheets as kit;
use crate::widgets::{self as w, ButtonStyle, SwatchStyle};
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

/// Cancel · Spacer · OK (the default button), the usual sheet footer.
fn footer(ui: &mut Ui, ok: &str, ok_enabled: bool) -> Close {
    let mut close = Close::Open;
    kit::hstack(ui, 8.0, |ui| {
        if w::button(ui, "Cancel", 13.0, ButtonStyle::Bordered, true).clicked() {
            close = Close::Cancel;
        }
        kit::trailing(ui, |ui| {
            if w::button(ui, ok, 13.0, ButtonStyle::Prominent, ok_enabled).clicked() {
                close = Close::Ok;
            }
        });
    });
    keys(ui, ok_enabled, close)
}

/// Return and Escape answer the sheet, as its default and cancel buttons.
fn keys(ui: &mut Ui, ok_enabled: bool, close: Close) -> Close {
    let (enter, escape) = ui.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, egui::Key::Enter), i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)));
    if escape {
        Close::Cancel
    } else if enter && ok_enabled && !ui.ctx().egui_wants_keyboard_input() {
        Close::Ok
    } else {
        close
    }
}

fn title2(ui: &mut Ui, s: &str) {
    kit::title2(ui, s);
}

/// Callout text wrapped across the sheet.
fn para(ui: &mut Ui, s: &str, c: Color32) {
    kit::para(ui, s, 12.0, c);
}

fn divider(ui: &mut Ui) {
    kit::divider(ui);
}

/// `--render-ui` puts a point here to have the open sheet drawn there alone, as the Mac harness
/// captures a sheet's content without its window, and reads the size it took from
/// `CAPTURED_SIZE`.
pub(crate) const CAPTURE: &str = "sheet-capture";
pub(crate) const CAPTURED_SIZE: &str = "sheet-captured-size";

/// A window for a sheet, `width` wide, centered on the editor. Its content is a VStack with the
/// Mac sheet's `padding` and `spacing`, on `windowBackgroundColor`.
fn window(ctx: &egui::Context, title: &str, width: f32, padding: f32, spacing: f32, content: impl FnOnce(&mut Ui)) {
    let body = |ui: &mut Ui| {
        ui.set_width(width - 2.0 * padding);
        // No horizontal spacing: egui widens a vertical layout by it after each full-width row.
        ui.spacing_mut().item_spacing = vec2(0.0, spacing);
        content(ui);
    };
    let margin = egui::Margin::same(padding as i8);
    if let Some(origin) = ctx.data(|d| d.get_temp::<egui::Pos2>(egui::Id::new(CAPTURE))) {
        let frame = egui::Frame::NONE.fill(color::WINDOW_BACKGROUND).inner_margin(margin);
        let shown = egui::Area::new(egui::Id::new("sheet")).fixed_pos(origin).order(egui::Order::Foreground).show(ctx, |ui| frame.show(ui, body));
        ctx.data_mut(|d| d.insert_temp(egui::Id::new(CAPTURED_SIZE), shown.response.rect.size()));
        return;
    }
    egui::Window::new(title)
        .id(egui::Id::new("sheet"))
        .collapsible(false)
        .resizable(false)
        .default_pos(ctx.content_rect().center() - vec2(width / 2.0, 200.0))
        .frame(egui::Frame::window(&ctx.global_style()).fill(color::WINDOW_BACKGROUND).inner_margin(margin))
        .show(ctx, body);
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
            // Shift and Alt pick Add and Remove for one click; otherwise the eyedropper in use.
            let mode = if modifiers.shift || (!modifiers.alt && s.mode == 1) {
                "Add"
            } else if modifiers.alt || s.mode == 2 {
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
                *color = hsb_of(c);
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
    window(ctx, &title, width, 24.0, 16.0, |ui| {
        match f.kind {
            "Levels" => levels_controls(ui, f),
            "Hue/Saturation" => hue_saturation_controls(ui, f),
            kind => {
                let ctls = controls(kind);
                // `labelWidth`: the widest label shown, at least 60.
                let widest = ctls
                    .iter()
                    .filter_map(|c| match c {
                        Ctl::Slider { title, .. } => Some(*title),
                        Ctl::If(when, Ctl::Slider { title, .. }) if when(&f.settings) => Some(*title),
                        _ => None,
                    })
                    .map(|t| kit::text_width(ui, t))
                    .fold(60.0f32, f32::max);
                for c in &ctls {
                    control(ui, c, &mut f.settings, widest, &mut f.curve_point);
                }
                w::checkbox(ui, &mut f.preview, "Preview");
            }
        }
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
/// its unit, 10 points apart; pickers, toggles, swatches and headings as the Mac lays them out.
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
            kit::hstack(ui, 10.0, |ui| {
                let (rect, response) = ui.allocate_exact_size(vec2(label_width, w::line_height(13.0)), Sense::drag());
                w::paint_centered(ui.painter(), title, theme::regular(13.0), color::label(), rect.min.x, rect.center().y);
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
            kit::hstack(ui, 8.0, |ui| {
                if !title.is_empty() {
                    kit::body(ui, title, color::label());
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
            kit::hstack(ui, 8.0, |ui| {
                kit::body(ui, title, color::label());
                let current = get(v, path).as_str().unwrap_or("").to_string();
                let mut index = options.iter().position(|o| *o == current).unwrap_or(0);
                let all: Vec<usize> = (0..options.len()).collect();
                if w::popup(ui, ("menu", path), &mut index, &[&all], |i| options[i], None, true) {
                    set(v, path, json!(options[index]));
                }
            });
        }
        Ctl::Color { title, path } => {
            kit::hstack(ui, 8.0, |ui| {
                kit::fixed_label(ui, title, 95.0);
                let c = get(v, path);
                let rgb = [c["red"].as_f64().unwrap_or(0.0) as f32, c["green"].as_f64().unwrap_or(0.0) as f32, c["blue"].as_f64().unwrap_or(0.0) as f32];
                let style = SwatchStyle { radius: 6.0, inner_white: 1.5, outer_black: 1.0 };
                if let Some(picked) = swatch_picker(ui, ("filter-color", path), rgb, vec2(24.0, 24.0), style) {
                    set(v, path, json!({ "red": picked[0] as f64, "green": picked[1] as f64, "blue": picked[2] as f64 }));
                }
            });
        }
        Ctl::Heading(s) => kit::headline(ui, s),
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

/// A color swatch that opens a color picker below it; returns the color picked this frame.
fn swatch_picker(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, rgb: [f32; 3], size: egui::Vec2, style: SwatchStyle) -> Option<[f32; 3]> {
    let response = w::swatch(ui, w::rgb(rgb), size, style);
    let mut color = w::rgb(rgb);
    let mut picked = None;
    egui::Popup::menu(&response).id(egui::Id::new(id)).show(|ui| {
        if egui::widgets::color_picker::color_picker_color32(ui, &mut color, egui::widgets::color_picker::Alpha::Opaque) {
            picked = Some([color.r(), color.g(), color.b()].map(|c| c as f32 / 255.0));
        }
    });
    picked
}

/// `CurvesControls`: the channel, the curve (click to add a point, drag to move, max 32), then
/// Remove point and Reset curve, 12 points apart.
fn curves_controls(ui: &mut Ui, v: &mut Value, selected: &mut Option<usize>) {
    let mut curves: comp_format::CurvesSettings = serde_json::from_value(v["curves"].clone()).unwrap_or_default();
    let before = curves.clone();
    let channels = [Channel::Rgb, Channel::Red, Channel::Green, Channel::Blue];
    let mut ci = channels.iter().position(|c| *c == curves.channel).unwrap_or(0);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 12.0;
        kit::hstack(ui, 8.0, |ui| {
            kit::body(ui, "Channel", color::label());
            if w::popup(ui, "curves-channel", &mut ci, &[&[0usize, 1, 2, 3]], |i| ["RGB", "Red", "Green", "Blue"][i], None, true) {
                *selected = None;
            }
        });
        curves.channel = channels[ci];
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 260.0), Sense::click_and_drag());
        // A Canvas clips what it draws: half of each end point and of the outer grid lines.
        let p = ui.painter().with_clip_rect(rect);
        p.rect_filled(rect, 0.0, theme::black_alpha(0.35));
        for i in 0..=4 {
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
        // The curve through the points at every input level, as the Mac draws it.
        let samples: Vec<egui::Pos2> = (0..=255).map(|x| to_view(&CurvePoint { x: x as f64, y: interpolate(points, x as f64) })).collect();
        p.add(egui::Shape::line(samples, Stroke::new(2.0, Color32::WHITE)));
        for (i, pt) in points.iter().enumerate() {
            p.circle_filled(to_view(pt), 4.0, if Some(i) == *selected { color::ACCENT } else { Color32::WHITE });
        }
        kit::caption(ui, "Click to add a point. Drag to adjust.");
        kit::hstack(ui, 8.0, |ui| {
            if let Some(i) = selected.filter(|i| *i < points.len()) {
                let pt = points[i];
                kit::body(ui, &format!("Input {} · Output {}", pt.x, pt.y), color::label());
            }
            kit::trailing(ui, |ui| {
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
    });
    if curves != before {
        v["curves"] = json!(curves);
    }
}

/// `CurvesSettings.value`: the curve at `x`, a cubic Hermite through the points with the
/// harmonic mean of the neighboring slopes (zero at a turn), clamped to 0…255.
fn interpolate(p: &[CurvePoint], x: f64) -> f64 {
    if p.len() < 2 {
        return p.first().map_or(x, |q| q.y);
    }
    let i = p.iter().rposition(|q| q.x <= x).unwrap_or(0).min(p.len() - 2);
    let d: Vec<f64> = p.windows(2).map(|w| (w[1].y - w[0].y) / (w[1].x - w[0].x)).collect();
    let slope = |j: usize| {
        if j == 0 {
            d[0]
        } else if j == p.len() - 1 {
            d[d.len() - 1]
        } else if d[j - 1] * d[j] <= 0.0 {
            0.0
        } else {
            2.0 / (1.0 / d[j - 1] + 1.0 / d[j])
        }
    };
    let h = p[i + 1].x - p[i].x;
    let t = ((x - p[i].x) / h).clamp(0.0, 1.0);
    let (t2, t3) = (t * t, t * t * t);
    let y = (2.0 * t3 - 3.0 * t2 + 1.0) * p[i].y + (t3 - 2.0 * t2 + t) * h * slope(i) + (-2.0 * t3 + 3.0 * t2) * p[i + 1].y + (t3 - t2) * h * slope(i + 1);
    y.clamp(0.0, 255.0)
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

/// `LevelsSheet`: the channel (centered in its 180-point frame), the histogram with the input
/// handles, the fields, the output bar and handles, the samplers, Auto, Preview and Reset.
fn levels_controls(ui: &mut Ui, f: &mut FilterSheet) {
    let channels = [Channel::Rgb, Channel::Red, Channel::Green, Channel::Blue];
    let names = ["RGB", "Red", "Green", "Blue"];
    let mut ci = channels.iter().position(|c| *c == f.adjustment.levels.channel).unwrap_or(0);
    let popup = names.iter().map(|n| kit::text_width(ui, n)).fold(0.0f32, f32::max).ceil() + w::POPUP_CHROME;
    let content = kit::text_width(ui, "Channel") + 8.0 + popup;
    ui.allocate_ui_with_layout(vec2(180.0, 24.0), Layout::left_to_right(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.add_space(((180.0 - content) / 2.0).max(0.0) - 8.0);
        kit::body(ui, "Channel", color::label());
        w::popup(ui, "levels-channel", &mut ci, &[&[0usize, 1, 2, 3]], |i| names[i], None, true);
    });
    f.adjustment.levels.channel = channels[ci];
    let full = ui.available_width();
    let range = &mut f.adjustment.levels.ranges[ci];
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let (rect, _) = ui.allocate_exact_size(vec2(full, 150.0), Sense::hover());
        ui.painter().rect_filled(rect, 0.0, theme::black_alpha(0.25));
        if let Some(bins) = &f.histogram {
            let channel = &bins[ci];
            let peak = kit::histogram_scale(channel);
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
        let gray_at = range.black + (range.white - range.black) * 0.5f64.powf(range.gamma);
        for (i, v) in [range.black, gray_at, range.white].iter().enumerate() {
            let fill = [Color32::BLACK, Color32::GRAY, Color32::WHITE][i];
            kit::triangle(ui.painter(), pos2(strip.min.x + (*v / 255.0) as f32 * full, strip.min.y + 9.0), fill);
        }
    });
    level_fields(ui, &mut [("Input black", &mut range.black, 0.0..=253.0), ("Gamma", &mut range.gamma, 0.1..=9.99), ("Input white", &mut range.white, 2.0..=255.0)]);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
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
                let v = (((pos.x - strip.min.x) / full * 255.0).clamp(0.0, 255.0) as f64).round();
                if (v - range.output_black).abs() <= (v - range.output_white).abs() {
                    range.output_black = v;
                } else {
                    range.output_white = v;
                }
            }
        }
        kit::triangle(ui.painter(), pos2(strip.min.x + (range.output_black / 255.0) as f32 * full, strip.min.y + 9.0), Color32::BLACK);
        kit::triangle(ui.painter(), pos2(strip.min.x + (range.output_white / 255.0) as f32 * full, strip.min.y + 9.0), Color32::WHITE);
    });
    level_fields(ui, &mut [("Output black", &mut range.output_black, 0.0..=255.0), ("Output white", &mut range.output_white, 0.0..=255.0)]);
    // Buttons with a symbol come out half a point taller than plain ones.
    kit::hstack_height(ui, 24.5, 8.0, |ui| {
        kit::caption(ui, "Sample");
        for (i, name) in ["Black", "Gray", "White"].iter().enumerate() {
            let armed = f.sampling == Some(i as u8);
            if kit::icon_button(ui, "eyedropper", name, armed).clicked() {
                f.sampling = if armed { None } else { Some(i as u8) };
            }
        }
    });
    if let Some(i) = f.sampling {
        kit::caption(ui, &format!("Click the original layer to set {}. Click the eyedropper again to stop.", ["black", "gray", "white"][i as usize]));
    }
    let mut auto = None;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        kit::caption(ui, "Auto");
        kit::hstack(ui, 8.0, |ui| {
            for (i, name) in ["Contrast", "Color", "Color + neutral midtones"].iter().enumerate() {
                if w::button(ui, name, 13.0, ButtonStyle::Bordered, f.histogram.is_some()).clicked() {
                    auto = Some(i);
                }
            }
        });
    });
    if let (Some(mode), Some(bins)) = (auto, &f.histogram) {
        f.sampling = None;
        let channel = f.adjustment.levels.channel;
        f.adjustment.levels = auto_levels(mode, bins);
        f.adjustment.levels.channel = channel;
    }
    let mut reset = false;
    kit::hstack(ui, 8.0, |ui| {
        w::checkbox(ui, &mut f.preview, "Preview");
        kit::trailing(ui, |ui| {
            reset = w::button(ui, "Reset", 13.0, ButtonStyle::Bordered, true).clicked();
        });
    });
    if reset {
        f.sampling = None;
        f.adjustment.levels = comp_format::LevelsSettings::default();
    }
    let source = if matches!(f.target, Target::Adjustment(_)) { "Underlying pixels" } else { "Original pixels" };
    kit::caption(ui, &format!("{source} · alpha-weighted histogram"));
}

/// Levels' field groups: a caption over an 80-point right-aligned field, spread across the row.
fn level_fields(ui: &mut Ui, fields: &mut [(&str, &mut f64, std::ops::RangeInclusive<f64>)]) {
    let full = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(full, w::line_height(10.0) + 5.0 + 24.0), Sense::hover());
    let n = fields.len();
    for (k, (name, value, range)) in fields.iter_mut().enumerate() {
        let x = if n == 1 { rect.min.x } else { rect.min.x + (full - 80.0) * k as f32 / (n - 1) as f32 };
        let column = Rect::from_min_size(pos2(x, rect.min.y), vec2(80.0, rect.height()));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(column).layout(Layout::top_down(Align::Min)));
        child.spacing_mut().item_spacing.y = 5.0;
        let fmt: fn(f64) -> String = if *name == "Gamma" { |v| format!("{v:.2}") } else { w::fmt_int };
        let sensitivity = if *name == "Gamma" { 0.01 } else { 1.0 };
        w::scrub_label(&mut child, name, theme::regular(10.0), color::secondary(), value, sensitivity, range.clone(), true);
        w::number_field(&mut child, ("levels", *name), value, range.clone(), fmt, 80.0, true, true);
    }
}

/// `LevelsAuto.settings`: Contrast stretches one shared interval between the darkest and lightest
/// 0.1% of the color channels; Color stretches each channel on its own, and neutral midtones also
/// sets each channel's gamma so its mean lands in the middle.
fn auto_levels(mode: usize, bins: &[[f64; 256]; 4]) -> comp_format::LevelsSettings {
    let mut result = comp_format::LevelsSettings::default();
    let endpoints = |b: &[f64; 256]| -> Option<(f64, f64)> {
        let total: f64 = b.iter().sum();
        if total <= 0.0 {
            return None;
        }
        let (mut sum, mut low, mut high) = (0.0, 0usize, 255usize);
        for (i, v) in b.iter().enumerate() {
            sum += v;
            if sum > total * 0.001 {
                low = i;
                break;
            }
        }
        sum = 0.0;
        for i in (0..256).rev() {
            sum += b[i];
            if sum > total * 0.001 {
                high = i;
                break;
            }
        }
        (low < high).then_some((low as f64, high as f64))
    };
    if mode == 0 {
        let limits: Vec<(f64, f64)> = bins[1..].iter().filter_map(endpoints).collect();
        let low = limits.iter().map(|l| l.0).fold(f64::INFINITY, f64::min);
        let high = limits.iter().map(|l| l.1).fold(f64::NEG_INFINITY, f64::max);
        if low < high {
            result.ranges[0] = comp_format::LevelRange { black: low, white: high, ..Default::default() };
        }
    } else {
        for c in 1..=3 {
            let Some((low, high)) = endpoints(&bins[c]) else { continue };
            let mut range = comp_format::LevelRange { black: low, white: high, ..Default::default() };
            if mode == 2 {
                let total: f64 = bins[c].iter().sum();
                let apply = |v: f64| ((v * 255.0 - low) / (high - low)).clamp(0.0, 1.0);
                let mean = bins[c].iter().enumerate().map(|(i, n)| apply(i as f64 / 255.0) * n).sum::<f64>() / total;
                if mean > 0.0 && mean < 1.0 {
                    range.gamma = (mean.ln() / 0.5f64.ln()).clamp(0.1, 9.99);
                }
            }
            result.ranges[c] = range;
        }
    }
    result
}

/// `HueSaturationSheet`: the range picker (centered in its 160-point frame) and the targeted
/// adjustment tool, then Hue, Saturation and Lightness on colored tracks; Colorize, Preview, Reset.
fn hue_saturation_controls(ui: &mut Ui, f: &mut FilterSheet) {
    const RANGES: [ColorRange; 7] = [ColorRange::Master, ColorRange::Reds, ColorRange::Yellows, ColorRange::Greens, ColorRange::Cyans, ColorRange::Blues, ColorRange::Magentas];
    let mut hsv = resolved_hsv(&f.adjustment);
    let before = hsv.clone();
    let mut ri = RANGES.iter().position(|r| *r == hsv.range).unwrap_or(0);
    kit::hstack(ui, 12.0, |ui| {
        let popup = RANGES.iter().map(|r| kit::text_width(ui, r.name())).fold(0.0f32, f32::max).ceil() + w::POPUP_CHROME;
        ui.add_space(((160.0 - popup) / 2.0).max(0.0) - 12.0);
        w::popup(ui, "hue-range", &mut ri, &[&[0usize, 1, 2, 3, 4, 5, 6]], |i| RANGES[i].name(), None, !hsv.colorize);
        ui.add_space(((160.0 - popup) / 2.0).max(0.0));
        kit::trailing(ui, |ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(24.0, 20.0), Sense::hover());
            crate::icons::paint(ui.painter(), crate::icons::Icon::Symbol("hand.point.up.left"), rect.center(), 13.0, color::label());
        });
    });
    if hsv.colorize {
        ri = 0;
    }
    hsv.range = RANGES[ri];
    let mut adj = hsv.adjustments.get(hsv.range).copied().unwrap_or(comp_format::RangeAdjustment { hue: 0.0, saturation: 0.0, lightness: 0.0 });
    let colorize = hsv.colorize;
    let spectrum: Vec<Color32> = (0..13).map(|i| kit::hsb(-180.0 + 30.0 * i as f32, 0.85, 0.9)).collect();
    let chroma = [Color32::from_rgb(158, 158, 163), Color32::from_rgb(219, 46, 51)];
    let lightness = [Color32::BLACK, Color32::WHITE];
    let rows: [(&str, &mut f64, std::ops::RangeInclusive<f64>, &str, &[Color32]); 3] = [
        ("Hue", &mut adj.hue, if colorize { 0.0..=360.0 } else { -180.0..=180.0 }, "°", &spectrum),
        ("Saturation", &mut adj.saturation, if colorize { 0.0..=100.0 } else { -100.0..=100.0 }, "", &chroma),
        ("Lightness", &mut adj.lightness, -100.0..=100.0, "", &lightness),
    ];
    for (title, value, range, unit, stops) in rows {
        kit::hstack(ui, 10.0, |ui| {
            let (rect, response) = ui.allocate_exact_size(vec2(76.0, w::line_height(13.0)), Sense::click_and_drag());
            w::paint_centered(ui.painter(), title, theme::regular(13.0), color::label(), rect.min.x, rect.center().y);
            if response.dragged() {
                *value = (*value + response.drag_delta().x as f64).clamp(*range.start(), *range.end());
            }
            if response.double_clicked() {
                *value = 0.0;
            }
            let unit_width = if unit.is_empty() { 0.0 } else { w::unit_width(ui, unit) };
            let sw = w::fill_width(ui, 48.0 + unit_width, if unit.is_empty() { 1 } else { 2 });
            kit::colored_slider(ui, value, range.clone(), sw, stops);
            w::number_field(ui, ("hsl", title), value, range, w::fmt_int, 48.0, true, true);
            if !unit.is_empty() {
                w::unit(ui, unit);
            }
            *value = value.round();
        });
    }
    let mut set_colorize = hsv.colorize;
    let mut reset = false;
    kit::hstack(ui, 18.0, |ui| {
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
    /// In `units`, and relative to the current size when `relative`.
    width: f64,
    height: f64,
    /// Pixels, Percent, Inches, Centimeters.
    units: usize,
    relative: bool,
    /// Keeps the original aspect ratio as either side changes.
    locked: bool,
    anchor: usize,
    /// Transparent, Foreground, Background, Black, White, Custom.
    extension: usize,
    custom: [f32; 3],
    error: Option<String>,
}

impl CanvasSizeSheet {
    #[cfg(test)]
    pub fn set_size(&mut self, width: f64, height: f64) {
        self.width = width;
        self.height = height;
    }
}

pub fn open_canvas_size(app: &mut App) {
    let Some(d) = app.doc() else { return };
    let (w0, h0) = (d.project.manifest.width as f64, d.project.manifest.height as f64);
    app.sheet = Some(Sheet::CanvasSize(CanvasSizeSheet { width: w0, height: h0, units: 0, relative: false, locked: false, anchor: 4, extension: 0, custom: [1.0; 3], error: None }));
}

/// A length in `units` (Pixels, Percent, Inches, Centimeters) as pixels, and back, for a side
/// `original` pixels long at `resolution` pixels per inch.
fn to_pixels(value: f64, units: usize, original: f64, resolution: f64) -> f64 {
    match units {
        1 => value / 100.0 * original,
        2 => value * resolution,
        3 => value / 2.54 * resolution,
        _ => value,
    }
}

fn from_pixels(pixels: f64, units: usize, original: f64, resolution: f64) -> f64 {
    match units {
        1 => pixels / original * 100.0,
        2 => pixels / resolution,
        3 => pixels / resolution * 2.54,
        _ => pixels,
    }
}


fn canvas_size_sheet(app: &mut App, ctx: &egui::Context, s: &mut CanvasSizeSheet) -> bool {
    let Some(d) = app.doc() else { return false };
    let (w0, h0) = (d.project.manifest.width, d.project.manifest.height);
    let resolution = d.project.manifest.resolution.unwrap_or(72.0);
    let foreground = app.settings.foreground;
    let background = app.settings.background;
    let mut close = Close::Open;
    // The size the sheet asks for, in pixels (unrounded).
    let exact = |s: &CanvasSizeSheet| -> (f64, f64) {
        let rel = |o: i64| if s.relative { o as f64 } else { 0.0 };
        (to_pixels(s.width, s.units, w0 as f64, resolution) + rel(w0), to_pixels(s.height, s.units, h0 as f64, resolution) + rel(h0))
    };
    let pixels = |s: &CanvasSizeSheet| -> (i64, i64) {
        let (w, h) = exact(s);
        (w.round() as i64, h.round() as i64)
    };
    // Shows `px` (absolute pixels) in the sheet's current units.
    let show = |s: &mut CanvasSizeSheet, (pw, ph): (f64, f64)| {
        let rel = |o: i64| if s.relative { o as f64 } else { 0.0 };
        s.width = from_pixels(pw - rel(w0), s.units, w0 as f64, resolution);
        s.height = from_pixels(ph - rel(h0), s.units, h0 as f64, resolution);
    };
    window(ctx, "Canvas Size", 450.0, 24.0, 16.0, |ui| {
        title2(ui, "Canvas Size");
        kit::body(ui, &format!("Current: {w0} × {h0} pixels"), color::label());
        para(ui, &format!("{} uncompressed RGBA canvas", kit::bytes(w0 * h0 * 4)), color::secondary());
        divider(ui);
        kit::hstack(ui, 8.0, |ui| {
            kit::body(ui, "Units", color::label());
            let current = exact(s);
            if w::popup(ui, "cs-units", &mut s.units, &[&[0usize, 1, 2, 3]], |i| kit::UNITS[i], None, true) {
                show(s, current);
            }
        });
        let (pw, ph) = (s.width, s.height);
        for (label, value) in [("Width", &mut s.width), ("Height", &mut s.height)] {
            kit::hstack(ui, 8.0, |ui| {
                kit::fixed_label(ui, label, 60.0);
                let wd = w::fill_width(ui, 0.0, 0);
                w::number_field(ui, ("canvas-size", label), value, -30000.0..=30000.0, w::fmt_trim2, wd, false, true);
            });
        }
        if s.locked && (s.width != pw || s.height != ph) {
            let (w, h) = exact(s);
            let ratio = h0 as f64 / w0 as f64;
            show(s, if s.width != pw { (w, w * ratio) } else { (h / ratio, h) });
        }
        let current = exact(s);
        if w::checkbox(ui, &mut s.relative, "Relative to current dimensions").changed() {
            show(s, current);
        }
        if w::checkbox(ui, &mut s.locked, "Lock original aspect ratio").changed() && s.locked {
            let (w, _) = exact(s);
            show(s, (w, w * h0 as f64 / w0 as f64));
        }
        let (nw, nh) = pixels(s);
        let valid = (1..=30_000).contains(&nw) && (1..=30_000).contains(&nh);
        if valid {
            para(ui, &format!("New: {nw} × {nh} pixels · {} uncompressed", kit::bytes(nw * nh * 4)), color::secondary());
        } else {
            para(ui, "Final dimensions must be 1–30,000 pixels per side.", color::ORANGE);
        }
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing = vec2(24.0, 8.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                kit::body(ui, "Anchor", color::label());
                egui::Grid::new("anchors").spacing(vec2(3.0, 3.0)).show(ui, |ui| {
                    for row in 0..3 {
                        for column in 0..3 {
                            let index = row * 3 + column;
                            // A bordered button around a 25-point image: 49 × 33, tinted with the
                            // accent when chosen and the secondary color otherwise.
                            let (rect, response) = ui.allocate_exact_size(vec2(49.0, 33.0), Sense::click());
                            let on = index == s.anchor;
                            ui.painter().rect_filled(rect, egui::CornerRadius::same(16), if on { color::ACCENT.gamma_multiply(0.05) } else { theme::white_alpha(0.027) });
                            crate::icons::paint(ui.painter(), crate::icons::Icon::Symbol(if on { "circle.fill" } else { "circle" }), rect.center(), 13.0, if on { color::ACCENT } else { color::secondary() });
                            if response.on_hover_text(kit::ANCHORS[index]).clicked() {
                                s.anchor = index;
                            }
                        }
                        ui.end_row();
                    }
                });
            });
            ui.vertical(|ui| {
                ui.add_space(28.0);
                ui.spacing_mut().item_spacing.y = 8.0;
                w::text(ui, kit::ANCHORS[s.anchor], theme::bold(12.0), color::label());
                // The Mac shows it on one line, truncated.
                w::truncated(ui, "Keeps this point fixed. Artwork is not scaled; cropped content remains outside the canvas.", theme::regular(12.0), color::secondary());
            });
        });
        kit::hstack(ui, 8.0, |ui| {
            kit::body(ui, "Canvas extension", color::label());
            w::popup(ui, "cs-extension", &mut s.extension, &[&[0usize, 1, 2, 3, 4, 5]], |i| ["Transparent", "Foreground", "Background", "Black", "White", "Custom"][i], None, true);
        });
        if s.extension == 5 {
            kit::hstack(ui, 8.0, |ui| {
                kit::body(ui, "Extension color", color::label());
                if let Some(c) = swatch_picker(ui, "cs-custom", s.custom, vec2(34.0, 18.0), SwatchStyle { radius: 4.0, inner_white: 1.0, outer_black: 1.0 }) {
                    s.custom = c;
                }
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
                1 => Some(foreground),
                2 => Some(background),
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
    /// In pixels; the fields show them in `units`.
    width: f64,
    height: f64,
    /// Pixels, Percent, Inches, Centimeters.
    units: usize,
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
        units: 0,
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
    window(ctx, "Image Size", 430.0, 24.0, 18.0, |ui| {
        title2(ui, "Image Size");
        kit::body(ui, &format!("Current: {} × {} pixels", w0 as i64, h0 as i64), color::secondary());
        kit::hstack(ui, 8.0, |ui| {
            kit::body(ui, "Units", color::label());
            w::popup(ui, "is-units", &mut s.units, &[&[0usize, 1, 2, 3]], |i| kit::UNITS[i], None, true);
        });
        let res = s.resolution;
        let mut shown = [from_pixels(s.width, s.units, w0, res), from_pixels(s.height, s.units, h0, res)];
        let before = shown;
        for (i, label) in ["Width", "Height"].iter().enumerate() {
            kit::hstack(ui, 8.0, |ui| {
                kit::fixed_label(ui, label, 75.0);
                let wd = w::fill_width(ui, 0.0, 0);
                w::number_field(ui, ("image-size", *label), &mut shown[i], 0.0..=30000.0, w::fmt_trim2, wd, false, s.resample);
            });
        }
        if shown[0] != before[0] {
            let pw = to_pixels(shown[0], s.units, w0, res).max(1.0);
            if s.lock {
                s.height = (pw * s.height / s.width).max(1.0);
            }
            s.width = pw;
        } else if shown[1] != before[1] {
            let ph = to_pixels(shown[1], s.units, h0, res).max(1.0);
            if s.lock {
                s.width = (ph * s.width / s.height).max(1.0);
            }
            s.height = ph;
        }
        ui.add_enabled_ui(s.resample, |ui| {
            w::checkbox(ui, &mut s.lock, "Lock aspect ratio");
        });
        kit::hstack(ui, 8.0, |ui| {
            kit::body(ui, "Resolution", color::label());
            let wd = w::fill_width(ui, kit::text_width(ui, "pixels/inch"), 1);
            w::number_field(ui, "resolution", &mut s.resolution, 1.0..=9600.0, w::fmt_trim2, wd, false, true);
            kit::body(ui, "pixels/inch", color::secondary());
        });
        w::checkbox(ui, &mut s.resample, "Resample");
        if s.resample {
            kit::hstack(ui, 8.0, |ui| {
                kit::body(ui, "Sampling", color::label());
                w::popup(ui, "is-sampling", &mut s.sampling, &[crate::tools::Sampling::ALL], crate::tools::Sampling::title, None, true);
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

fn trim_sheet(app: &mut App, ctx: &egui::Context, s: &mut TrimSheet) -> bool {
    let mut close = Close::Open;
    window(ctx, "Trim", 320.0, 24.0, 18.0, |ui| {
        title2(ui, "Trim");
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            kit::headline(ui, "Based On");
            // A radio group: 16-point cells 6.5 apart, starting a point down.
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.5;
                ui.add_space(1.0);
                for (i, option) in ["Transparent Pixels", "Top Left Pixel Color", "Bottom Right Pixel Color"].iter().enumerate() {
                    if kit::radio(ui, s.based == i, option) {
                        s.based = i;
                    }
                }
            });
        });
        divider(ui);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            kit::headline(ui, "Trim Away");
            // `Grid(alignment: .leading, horizontalSpacing: 24, verticalSpacing: 8)`: the first
            // column as wide as its widest toggle.
            let [top, bottom, left, right] = &mut s.sides;
            let first = ["Top", "Left"].iter().map(|t| kit::text_width(ui, t)).fold(0.0f32, f32::max) + 22.0;
            for (a, a_title, b, b_title) in [(top, "Top", bottom, "Bottom"), (left, "Left", right, "Right")] {
                ui.allocate_ui_with_layout(vec2(ui.available_width(), 16.5), Layout::left_to_right(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let start = ui.cursor().min.x;
                    w::checkbox(ui, a, a_title);
                    ui.add_space(start + first + 24.0 - ui.cursor().min.x);
                    w::checkbox(ui, b, b_title);
                });
            }
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
    /// The encoded file and its decoded preview (smooth for the fitted view, sharp from 100% up),
    /// for the settings they were made with.
    encoded: Option<((u64, u64), Vec<u8>, egui::TextureHandle, egui::TextureHandle)>,
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
    if s.encoded.as_ref().is_none_or(|(k, ..)| *k != key) {
        let options = json!({ "quality": (s.quality * 100.0).round() / 100.0, "matte": s.matte.map(|c| c as f64) });
        match app.gfx.engine.export_jpeg(&doc.project, &options) {
            Ok(bytes) => {
                s.error = None;
                if let Ok(decoded) = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg) {
                    let rgba = decoded.to_rgba8();
                    let image = egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
                    let smooth = ctx.load_texture("jpeg-preview-fit", image.clone(), egui::TextureOptions::LINEAR);
                    let sharp = ctx.load_texture("jpeg-preview", image, egui::TextureOptions::NEAREST);
                    s.encoded = Some((key, bytes, smooth, sharp));
                }
            }
            Err(e) => {
                s.error = Some(describe(&e));
                s.encoded = None;
            }
        }
    }
    // The preview's frame; fitted, the whole image fills it however small it is.
    let frame = vec2(560.0, 330.0);
    let fit = (frame.x / w0 as f32).min(frame.y / h0 as f32);
    const STEPS: [f32; 6] = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0];
    let shown = s.zoom.unwrap_or(fit);
    let step_in = STEPS.iter().copied().find(|z| *z > shown * 1.001);
    let step_out = STEPS.iter().rev().copied().find(|z| *z < shown * 0.999);
    let mut close = Close::Open;
    window(ctx, "Export JPEG", 608.0, 24.0, 16.0, |ui| {
        // Closer to the preview than the rest of the dialog's spacing.
        kit::hstack(ui, 8.0, |ui| {
            title2(ui, "Export JPEG");
            kit::trailing(ui, |ui| {
                if zoom_button(ui, "minus.magnifyingglass", step_out.is_some()).clicked() {
                    s.zoom = step_out;
                }
                if zoom_button(ui, "plus.magnifyingglass", step_in.is_some()).clicked() {
                    s.zoom = step_in;
                }
                if w::button(ui, "Fit", 13.0, ButtonStyle::Bordered, s.zoom.is_some()).clicked() {
                    s.zoom = None;
                }
            });
        });
        ui.add_space(-8.0);
        let (rect, response) = ui.allocate_exact_size(frame, Sense::click());
        ui.painter().rect_filled(rect, 0.0, theme::gray(0.12));
        if let Some((_, _, smooth, sharp)) = &s.encoded {
            let texture = if s.zoom.is_some_and(|z| z >= 1.0) { sharp } else { smooth };
            let image = Rect::from_center_size(rect.center(), vec2(w0 as f32 * shown, h0 as f32 * shown));
            ui.painter().with_clip_rect(rect).image(texture.id(), image, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        // A double-click switches between Fit and 100%.
        if response.double_clicked() {
            s.zoom = if s.zoom.is_some() { None } else { Some(1.0) };
        }
        // A Slider with a step draws its ticks; the row is its 16 points.
        kit::hstack_height(ui, 16.0, 8.0, |ui| {
            kit::body(ui, "Quality", color::label());
            let sw = w::fill_width(ui, 45.0, 1);
            w::slider_with_ticks(ui, &mut s.quality, 0.0..=1.0, sw, true, Some(100));
            s.quality = (s.quality * 100.0).round() / 100.0;
            let (label, _) = ui.allocate_exact_size(vec2(45.0, w::line_height(13.0)), Sense::hover());
            let text = format!("{}%", (s.quality * 100.0).round());
            let g = ui.painter().layout_no_wrap(text, theme::regular(13.0), color::label());
            w::center_line(ui.painter(), g.clone(), label.max.x - g.size().x, label.center().y, color::label());
        });
        // As tall as the 18-point swatch.
        kit::hstack_height(ui, 18.0, 8.0, |ui| {
            kit::body(ui, "Background for transparency", color::label());
            if let Some(c) = swatch_picker(ui, "jpeg-matte", s.matte, vec2(34.0, 18.0), SwatchStyle { radius: 4.0, inner_white: 1.0, outer_black: 1.0 }) {
                s.matte = c;
            }
        });
        kit::hstack(ui, 12.0, |ui| {
            kit::body(ui, &format!("{w0} × {h0} px · sRGB"), color::secondary());
            kit::trailing(ui, |ui| {
                if w::button(ui, "Export…", 13.0, ButtonStyle::Prominent, s.encoded.is_some() && s.error.is_none()).clicked() {
                    close = Close::Ok;
                }
                if w::button(ui, "Cancel", 13.0, ButtonStyle::Bordered, true).clicked() {
                    close = Close::Cancel;
                }
                match (&s.error, &s.encoded) {
                    (Some(e), _) => kit::body(ui, e, color::RED),
                    (None, Some((_, bytes, ..))) => kit::body(ui, &bytes_file(bytes.len()), color::label()),
                    _ => kit::body(ui, "Updating…", color::secondary()),
                }
            });
        });
        close = keys(ui, s.encoded.is_some() && s.error.is_none(), std::mem::replace(&mut close, Close::Open));
    });
    match close {
        Close::Open => true,
        Close::Cancel => false,
        Close::Ok => {
            app.jpeg_quality = s.quality;
            let name = app.doc().map(|d| d.name.clone()).unwrap_or_default();
            let Some((_, bytes, ..)) = &s.encoded else { return true };
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

/// A bordered button showing a symbol, as the JPEG preview's zoom buttons.
fn zoom_button(ui: &mut Ui, symbol: &'static str, enabled: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(40.0, metric::CONTROL_HEIGHT), if enabled { Sense::click() } else { Sense::hover() });
    let dim = if enabled { 1.0 } else { 0.4 };
    ui.painter().rect_filled(rect, egui::CornerRadius::same(12), color::control().gamma_multiply(dim));
    crate::icons::paint(ui.painter(), crate::icons::Icon::Symbol(symbol), rect.center(), 13.0, color::label().gamma_multiply(dim));
    response
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
    window(ctx, "Photoshop", 520.0, 24.0, 16.0, |ui| {
        title2(ui, &format!("Open “{name}”?"));
        para(ui, "Compositor will convert these Photoshop features. Nothing is applied until you continue.", color::secondary());
        egui::ScrollArea::vertical().max_height(260.0).min_scrolled_height(180.0).show(ui, |ui| {
            for (layer, message) in conversions {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    kit::headline(ui, layer);
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
    window(ctx, "Rename Layer", 340.0, 24.0, 16.0, |ui| {
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

/// `KeyboardShortcutsSheet`, showing the default shortcuts. Editing them isn't there yet, so
/// the recorders show their chords and Restore Defaults has nothing to undo.
fn shortcuts_sheet(ctx: &egui::Context, search: &mut String) -> bool {
    let mut open = true;
    window(ctx, "Keyboard Shortcuts", 660.0, 24.0, 10.0, |ui| {
        kit::para(ui, "These are the default shortcuts. Ctrl stands for the Mac's Command key, and Alt for Option.", 13.0, color::secondary());
        let wd = ui.available_width();
        w::text_field(ui, "shortcut-search", search, wd, "Search shortcuts", theme::regular(13.0));
        let (list, _) = ui.allocate_exact_size(vec2(wd, 465.0), Sense::hover());
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list).layout(Layout::top_down(Align::Min)));
        child.set_clip_rect(list);
        egui::ScrollArea::vertical().id_salt("shortcuts").auto_shrink([false, false]).show(&mut child, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.set_width(wd - 8.0);
            for (group, rows) in crate::menus::shortcut_list() {
                ui.add_space(8.0);
                kit::headline(ui, group);
                for (title, chord) in rows {
                    if !search.is_empty() && !title.to_lowercase().contains(&search.to_lowercase()) {
                        continue;
                    }
                    kit::hstack(ui, 8.0, |ui| {
                        kit::body(ui, &title, color::label());
                        kit::trailing(ui, |ui| {
                            // An AppKit rounded-bezel NSButton in a 150 × 26 frame: a 24-point
                            // rounded rectangle, not a capsule.
                            let (rect, _) = ui.allocate_exact_size(vec2(150.0, 26.0), Sense::hover());
                            ui.painter().rect_filled(rect.shrink2(vec2(0.0, 1.0)), egui::CornerRadius::same(6), color::control());
                            let g = ui.painter().layout_no_wrap(chord, theme::regular(13.0), color::label());
                            w::center_line(ui.painter(), g.clone(), rect.center().x - g.size().x / 2.0, rect.center().y, color::label());
                        });
                    });
                }
            }
            ui.add_space(8.0);
            divider(ui);
            ui.add_space(8.0);
            kit::headline(ui, "Contextual keys & mouse gestures");
            para(ui, "Text fields keep standard editing keys. Dialogs share the Apply/Cancel assignments above. Numeric fields use Up/Down, with Shift for larger steps. Standard commands include Alt+F4 to quit and F11 for full screen.", color::label());
            para(ui, "Alt temporarily selects the eyedropper in painting tools. Shift constrains shapes/movement or adds to a selection; Alt subtracts from selections or draws from center. Ctrl-drag moves selected pixels; Ctrl-Alt-drag copies them. Alt-drag duplicates layers/folders/effects; Alt-click at a layer boundary toggles clipping. Ctrl-click a thumbnail loads its selection. Right-drag adjusts brush size. Modifier-and-mouse gestures are fixed.", color::label());
        });
        divider(ui);
        kit::hstack(ui, 8.0, |ui| {
            w::button(ui, "Restore Defaults", 13.0, ButtonStyle::Bordered, true);
            kit::trailing(ui, |ui| {
                if w::button(ui, "Save", 13.0, ButtonStyle::Prominent, true).clicked() {
                    open = false;
                }
                if w::button(ui, "Cancel", 13.0, ButtonStyle::Bordered, true).clicked() {
                    open = false;
                }
            });
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
    window(ctx, title, 380.0, 24.0, 16.0, |ui| {
        kit::hstack(ui, 10.0, |ui| {
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
    window(ctx, "Color Range", 340.0, 24.0, 16.0, |ui| {
        // The eyedroppers: replace, add, remove; the one in use is tinted.
        kit::hstack_height(ui, 20.0, 6.0, |ui| {
            for (i, badge) in [None, Some("plus.circle.fill"), Some("minus.circle.fill")].into_iter().enumerate() {
                let (rect, response) = ui.allocate_exact_size(vec2(24.0, 20.0), Sense::click());
                if s.mode == i as u8 {
                    ui.painter().rect_filled(rect, 4.0, color::ACCENT.gamma_multiply(0.25));
                }
                crate::icons::paint(ui.painter(), crate::icons::Icon::Symbol("eyedropper"), rect.center(), 13.0, color::label());
                if let Some(badge) = badge {
                    crate::icons::paint(ui.painter(), crate::icons::Icon::Symbol(badge), rect.center() + vec2(6.0, 4.0), 8.0, color::label());
                }
                let help = ["Click the image to select that color", "Click the image to add that color to the selection", "Click the image to take that color out of the selection"][i];
                if response.on_hover_text(help).clicked() {
                    s.mode = i as u8;
                }
            }
        });
        // `.frame(maxWidth: .infinity)`: centered across the sheet.
        let scale = (292.0 / canvas.x).min(200.0 / canvas.y);
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), (canvas.y * scale).round()), Sense::hover());
        let rect = Rect::from_center_size(row.center(), canvas * scale);
        ui.painter().rect_filled(rect, 0.0, Color32::BLACK);
        if let Some(t) = &s.preview {
            ui.painter().image(t.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, theme::white_alpha(0.2)), egui::StrokeKind::Inside);
        let hint = if s.samples.is_empty() { "Click the image to pick the color to select." } else { "Shift-click adds a color, Alt-click takes one away." };
        para(ui, hint, color::secondary());
        kit::hstack(ui, 10.0, |ui| {
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

/// One of `EffectsSheet`'s slider rows: a 64-point title (scrubs), a 130-point slider, a
/// 48-point field and its unit, 10 points apart.
fn effect_row(ui: &mut Ui, title: &str, value: &mut f64, slider_range: std::ops::RangeInclusive<f64>, typed: std::ops::RangeInclusive<f64>, unit: &str) {
    kit::hstack(ui, 10.0, |ui| {
        let (rect, response) = ui.allocate_exact_size(vec2(64.0, w::line_height(13.0)), Sense::drag());
        w::paint_centered(ui.painter(), title, theme::regular(13.0), color::label(), rect.min.x, rect.center().y);
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

/// An effect's color swatch (36 × 18, white inner ring, black border), which opens a picker.
fn color_row(ui: &mut Ui, id: &str, r: &mut f64, g: &mut f64, b: &mut f64) {
    let style = SwatchStyle { radius: 3.0, inner_white: 1.0, outer_black: 1.0 };
    if let Some(c) = swatch_picker(ui, ("effect-color", id), [*r as f32, *g as f32, *b as f32], vec2(36.0, 18.0), style) {
        (*r, *g, *b) = (c[0] as f64, c[1] as f64, c[2] as f64);
    }
}

fn effect_sheet(app: &mut App, ctx: &egui::Context, s: &mut EffectSheet) -> bool {
    let before = s.effects.clone();
    let mut close = Close::Open;
    window(ctx, s.kind, 340.0, 20.0, 16.0, |ui| {
        let e = &mut s.effects;
        let kind = s.kind;
        match kind {
            "Stroke" => {
                let x = e.stroke.as_mut().unwrap();
                kit::hstack(ui, 8.0, |ui| {
                    kit::headline(ui, kind);
                    kit::trailing(ui, |ui| {
                        w::segmented(ui, &mut x.inside, &[false, true], |i| if i { "Inside" } else { "Outside" });
                    });
                });
                // As tall as the 18-point swatch.
                kit::hstack_height(ui, 18.0, 8.0, |ui| {
                    kit::fixed_label(ui, "Color", 64.0);
                    color_row(ui, kind, &mut x.red, &mut x.green, &mut x.blue);
                });
                effect_row(ui, "Size", &mut x.size, 0.0..=20.0, 0.0..=500.0, "px");
                let mut o = (x.opacity * 100.0).round();
                effect_row(ui, "Opacity", &mut o, 0.0..=100.0, 0.0..=100.0, "%");
                x.opacity = o / 100.0;
            }
            "Drop Shadow" | "Inner Shadow" => {
                let inner = kind == "Inner Shadow";
                let x = if inner { e.inner_shadow.as_mut().unwrap() } else { e.shadow.as_mut().unwrap() };
                kit::hstack(ui, 8.0, |ui| {
                    kit::headline(ui, kind);
                    kit::trailing(ui, |ui| color_row(ui, kind, &mut x.red, &mut x.green, &mut x.blue));
                });
                let mut o = (x.opacity * 100.0).round();
                effect_row(ui, "Opacity", &mut o, 0.0..=100.0, 0.0..=100.0, "%");
                x.opacity = o / 100.0;
                effect_row(ui, "Angle", &mut x.angle, -180.0..=180.0, -180.0..=180.0, "°");
                effect_row(ui, "Distance", &mut x.distance, if inner { 0.0..=50.0 } else { 0.0..=100.0 }, 0.0..=5000.0, "px");
                effect_row(ui, "Blur", &mut x.blur, 0.0..=100.0, 0.0..=500.0, "px");
            }
            "Color Overlay" => {
                let x = e.color_overlay.as_mut().unwrap();
                kit::hstack(ui, 8.0, |ui| {
                    kit::headline(ui, kind);
                    kit::trailing(ui, |ui| color_row(ui, kind, &mut x.red, &mut x.green, &mut x.blue));
                });
                let mut o = (x.opacity * 100.0).round();
                effect_row(ui, "Opacity", &mut o, 0.0..=100.0, 0.0..=100.0, "%");
                x.opacity = o / 100.0;
            }
            _ => {
                let x = if kind == "Outer Glow" { e.outer_glow.as_mut().unwrap() } else { e.inner_glow.as_mut().unwrap() };
                kit::hstack(ui, 8.0, |ui| {
                    kit::headline(ui, kind);
                    kit::trailing(ui, |ui| color_row(ui, kind, &mut x.red, &mut x.green, &mut x.blue));
                });
                effect_row(ui, "Size", &mut x.size, 0.0..=100.0, 0.0..=500.0, "px");
                let mut o = (x.opacity * 100.0).round();
                effect_row(ui, "Opacity", &mut o, 0.0..=100.0, 0.0..=100.0, "%");
                x.opacity = o / 100.0;
            }
        }
        // Spacer · Cancel · OK, 10 points apart; OK is the default button.
        kit::hstack(ui, 10.0, |ui| {
            kit::trailing(ui, |ui| {
                if w::button(ui, "OK", 13.0, ButtonStyle::Prominent, true).clicked() {
                    close = Close::Ok;
                }
                if w::button(ui, "Cancel", 13.0, ButtonStyle::Bordered, true).clicked() {
                    close = Close::Cancel;
                }
            });
        });
        close = keys(ui, true, std::mem::replace(&mut close, Close::Open));
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

/// The Color Picker panel for the foreground or background color. Its hue, saturation and
/// brightness are `PickerHSB`'s, on the encoded sRGB values.
pub fn open_color_picker(app: &mut App, background: bool) {
    let original = if background { app.settings.background } else { app.settings.foreground };
    app.sheet = Some(Sheet::ColorPicker { background, color: hsb_of(original), original });
}

/// HSB (hue in turns) of an sRGB color, on its encoded values, as `PickerHSB.setRGB`.
pub fn hsb_of(c: [f32; 3]) -> egui::ecolor::Hsva {
    let [r, g, b] = c;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d <= 0.0 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    egui::ecolor::Hsva { h: h / 6.0, s: if max > 0.0 { d / max } else { 0.0 }, v: max, a: 1.0 }
}

/// The sRGB color of an HSB made by `hsb_of`.
pub fn rgb_of(c: &egui::ecolor::Hsva) -> [f32; 3] {
    let k = kit::hsb(c.h * 360.0, c.s, c.v);
    [k.r(), k.g(), k.b()].map(|v| v as f32 / 255.0)
}

/// `ColorPickerSheet`: saturation/brightness field, hue strip, then the preview, OK and Cancel,
/// and the RGB and hex fields in a 180-point column.
fn color_picker_sheet(app: &mut App, ctx: &egui::Context, background: bool, color: &mut egui::ecolor::Hsva, original: [f32; 3]) -> bool {
    let title = if background { "Color Picker (Background Color)" } else { "Color Picker (Foreground Color)" };
    let mut close = Close::Open;
    let width = 20.0 + 256.0 + 14.0 + 34.0 + 14.0 + 180.0 + 20.0;
    window(ctx, title, width, 20.0, 0.0, |ui| {
        let origin = ui.cursor().min;
        let (whole, _) = ui.allocate_exact_size(vec2(256.0 + 14.0 + 34.0 + 14.0 + 180.0, 256.0), Sense::hover());
        let p = ui.painter().clone();
        // Saturation across, brightness down: white to the hue (a SwiftUI gradient, mixed
        // perceptually), under clear to black (composited, so linear in the encoded values).
        let field = Rect::from_min_size(origin, vec2(256.0, 256.0));
        let hue = kit::hsb(color.h * 360.0, 1.0, 1.0);
        let (columns, rows) = (64, 16);
        let mut mesh = egui::Mesh::default();
        for j in 0..=rows {
            let t = j as f32 / rows as f32;
            for i in 0..=columns {
                let top = kit::oklab_mix(Color32::WHITE, hue, i as f32 / columns as f32);
                let c = Color32::from_rgb(kit::scale(top.r(), 1.0 - t), kit::scale(top.g(), 1.0 - t), kit::scale(top.b(), 1.0 - t));
                mesh.colored_vertex(pos2(field.min.x + 256.0 * i as f32 / columns as f32, field.min.y + 256.0 * t), c);
            }
        }
        for j in 0..rows {
            for i in 0..columns {
                let a = (j * (columns + 1) + i) as u32;
                let c = a + (columns + 1) as u32;
                mesh.add_triangle(a, a + 1, c + 1);
                mesh.add_triangle(a, c + 1, c);
            }
        }
        p.add(egui::Shape::mesh(mesh));
        let response = ui.interact(field, ui.id().with("picker-field"), Sense::click_and_drag());
        if let Some(pos) = response.interact_pointer_pos().filter(|_| response.dragged() || response.clicked()) {
            color.s = ((pos.x - field.min.x) / 256.0).clamp(0.0, 1.0);
            color.v = 1.0 - ((pos.y - field.min.y) / 256.0).clamp(0.0, 1.0);
        }
        // The marker, clipped by the field as `clipShape` does.
        let marker = pos2(field.min.x + color.s * 256.0, field.min.y + (1.0 - color.v) * 256.0);
        let clipped = p.with_clip_rect(field);
        clipped.circle_stroke(marker, 5.25, Stroke::new(1.5, Color32::WHITE));
        clipped.circle_stroke(marker, 6.375, Stroke::new(0.75, Color32::BLACK));
        p.rect_stroke(field, 0.0, Stroke::new(1.0, theme::black_alpha(0.6)), egui::StrokeKind::Inside);
        // Hue strip, 360 at the top to 0 at the bottom in 60° stops, with arrows at the hue.
        let slot = Rect::from_min_size(pos2(field.max.x + 14.0, origin.y), vec2(34.0, 256.0));
        let strip = Rect::from_min_size(slot.min + vec2(7.0, 0.0), vec2(20.0, 256.0));
        let mut mesh = egui::Mesh::default();
        let steps = 6 * 16;
        for k in 0..=steps {
            let f = k as f32 / 16.0;
            let stop = (f.floor() as usize).min(5);
            let c = kit::oklab_mix(kit::hsb(360.0 - 60.0 * stop as f32, 1.0, 1.0), kit::hsb(360.0 - 60.0 * (stop + 1) as f32, 1.0, 1.0), f - stop as f32);
            let y = strip.min.y + 256.0 * k as f32 / steps as f32;
            mesh.colored_vertex(pos2(strip.min.x, y), c);
            mesh.colored_vertex(pos2(strip.max.x, y), c);
            if k > 0 {
                let b = (k * 2) as u32;
                mesh.add_triangle(b - 2, b - 1, b);
                mesh.add_triangle(b - 1, b, b + 1);
            }
        }
        p.add(egui::Shape::mesh(mesh));
        p.rect_stroke(strip, 0.0, Stroke::new(1.0, theme::black_alpha(0.6)), egui::StrokeKind::Inside);
        let response = ui.interact(slot, ui.id().with("picker-hue"), Sense::click_and_drag());
        if let Some(pos) = response.interact_pointer_pos().filter(|_| response.dragged() || response.clicked()) {
            color.h = (1.0 - ((pos.y - slot.min.y) / 256.0).clamp(0.0, 1.0)).rem_euclid(1.0);
        }
        let y = strip.min.y + (1.0 - color.h) * 256.0;
        p.add(egui::Shape::convex_polygon(vec![pos2(slot.min.x, y - 5.0), pos2(slot.min.x + 7.0, y), pos2(slot.min.x, y + 5.0)], color::label(), Stroke::NONE));
        p.add(egui::Shape::convex_polygon(vec![pos2(slot.max.x, y - 5.0), pos2(slot.max.x - 7.0, y), pos2(slot.max.x, y + 5.0)], color::label(), Stroke::NONE));
        // The 180 × 256 column: preview, OK and Cancel (large controls, 90 wide) at the top; the
        // fields and the hint pushed to the bottom by the Spacer.
        let column = Rect::from_min_size(pos2(slot.max.x + 14.0, origin.y), vec2(180.0, 256.0));
        let preview = Rect::from_min_size(column.min, vec2(64.0, 64.0));
        let rgb = rgb_of(color);
        p.rect_filled(preview, 5.0, w::rgb(rgb));
        p.rect_stroke(preview, 5.0, Stroke::new(1.0, theme::black_alpha(0.6)), egui::StrokeKind::Inside);
        for (i, (label, prominent)) in [("OK", true), ("Cancel", false)].into_iter().enumerate() {
            let rect = Rect::from_min_size(pos2(preview.max.x + 16.0, column.min.y + i as f32 * 36.0), vec2(90.0, 28.0));
            let response = ui.interact(rect, ui.id().with(("picker-button", i)), Sense::click());
            p.rect_filled(rect, egui::CornerRadius::same(14), if prominent { color::ACCENT } else { color::control() });
            let text = if prominent { Color32::WHITE } else { color::label() };
            let g = p.layout_no_wrap(label.into(), theme::regular(13.0), text);
            w::center_line(&p, g.clone(), rect.center().x - g.size().x / 2.0, rect.center().y, text);
            if response.clicked() {
                close = if prominent { Close::Ok } else { Close::Cancel };
            }
        }
        let hint = w::line_height(10.0);
        let grid = 4.0 * 24.0 + 3.0 * 6.0;
        let top = column.max.y - hint - 8.0 - grid;
        let mut fields = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(column.min.x, top), column.max)).layout(Layout::top_down(Align::Min)));
        fields.spacing_mut().item_spacing = vec2(8.0, 6.0);
        let mut values = rgb.map(|c| (c * 255.0).round() as f64);
        let before = values;
        for (i, label) in ["R", "G", "B"].iter().enumerate() {
            fields.allocate_ui_with_layout(vec2(180.0, 24.0), Layout::left_to_right(Align::Center), |ui| {
                kit::fixed_label(ui, label, 14.0);
                w::number_field(ui, ("picker", *label), &mut values[i], 0.0..=255.0, w::fmt_int, 52.0, false, true);
            });
        }
        if values != before {
            *color = hsb_of(values.map(|v| (v / 255.0) as f32));
        }
        let mut hex = format!("{:02X}{:02X}{:02X}", values[0] as u8, values[1] as u8, values[2] as u8);
        fields.allocate_ui_with_layout(vec2(180.0, 24.0), Layout::left_to_right(Align::Center), |ui| {
            kit::fixed_label(ui, "#", 14.0);
            if w::text_field(ui, "picker-hex", &mut hex, 84.0, "", egui::FontId::monospace(13.0)).changed() {
                if let (6, Ok(v)) = (hex.len(), u32::from_str_radix(hex.trim(), 16)) {
                    *color = hsb_of([(v >> 16) & 255, (v >> 8) & 255, v & 255].map(|x| x as f32 / 255.0));
                }
            }
        });
        let g = p.layout_no_wrap("Click the canvas to sample".into(), theme::regular(10.0), color::secondary());
        w::paint_line(&p, g, pos2(column.min.x, column.max.y - hint), 10.0, color::secondary());
        let _ = whole;
        close = keys(ui, true, std::mem::replace(&mut close, Close::Open));
    });
    match close {
        Close::Open => true,
        Close::Cancel => {
            let _ = original;
            false
        }
        Close::Ok => {
            let rgb = rgb_of(color);
            if background {
                app.settings.background = rgb;
            } else {
                app.settings.foreground = rgb;
            }
            false
        }
    }
}
