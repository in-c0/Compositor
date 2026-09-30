//! `FilterSettings` (Document/Filters.swift) and `DitherSettings` (Document/Dither.swift), read
//! from a corpus op's `settings` the way the harness's `FilterSettingsJSON` reads them: only the
//! fields present change, nested settings are patched field by field onto their defaults, and a
//! field the harness doesn't know is an error.

use anyhow::{Result, anyhow, bail};
use comp_format::{
    AdjustmentColor, BlackWhiteSettings, ColorBalanceSettings, CurvesSettings, ExposureSettings, GradientMapSettings, GrainSettings,
};
use serde_json::{Map, Value};

/// `JSONValue.patched`: the default encoded, the patch merged in field by field (nested objects
/// recursively), and the result decoded.
macro_rules! patched {
    ($ty:ty, $base:expr, $item:expr, $key:expr) => {{
        let base = serde_json::to_value(&$base)?;
        let merged = merge(base, $item, &format!("settings.{}", $key))?;
        serde_json::from_value::<$ty>(merged).map_err(|e| anyhow!("settings.{}: {e}", $key))?
    }};
}

#[derive(Clone, Debug, PartialEq)]
pub struct FilterSettings {
    pub radius: f64,
    pub angle: f64,
    pub distance: f64,
    pub amount: f64,
    pub gaussian: bool,
    pub monochromatic: bool,
    pub vignette_amount: f64,
    pub vignette_color: AdjustmentColor,
    pub vignette_midpoint: f64,
    pub vignette_roundness: f64,
    pub vignette_feather: f64,
    pub vignette_highlights: f64,
    pub bloom_amount: f64,
    pub bloom_radius: f64,
    pub tonal_amount: f64,
    pub tonal_radius: f64,
    pub tonal_shadows: f64,
    pub tonal_midtones: f64,
    pub tonal_highlights: f64,
    pub distortion: f64,
    pub curves: CurvesSettings,
    pub exposure: ExposureSettings,
    pub gradient_map: GradientMapSettings,
    pub grain: GrainSettings,
    pub black_white: BlackWhiteSettings,
    pub color_balance: ColorBalanceSettings,
    pub dither: DitherSettings,
    /// Remove Background's Advanced mode.
    pub advanced_background: bool,
    pub refine_edges: f64,
    pub matte_contrast: f64,
    pub shift_edge: f64,
}

impl Default for FilterSettings {
    fn default() -> Self {
        let black = AdjustmentColor { red: 0.0, green: 0.0, blue: 0.0 };
        Self {
            radius: 1.0,
            angle: 0.0,
            distance: 10.0,
            amount: 10.0,
            gaussian: false,
            monochromatic: false,
            vignette_amount: 35.0,
            vignette_color: black,
            vignette_midpoint: 50.0,
            vignette_roundness: 100.0,
            vignette_feather: 60.0,
            vignette_highlights: 25.0,
            bloom_amount: 40.0,
            bloom_radius: 24.0,
            tonal_amount: 50.0,
            tonal_radius: 16.0,
            tonal_shadows: 40.0,
            tonal_midtones: 60.0,
            tonal_highlights: 30.0,
            distortion: 0.0,
            curves: CurvesSettings::default(),
            exposure: ExposureSettings::default(),
            gradient_map: GradientMapSettings::default(),
            grain: GrainSettings::default(),
            black_white: BlackWhiteSettings::default(),
            // Swift's default keeps luminosity.
            color_balance: ColorBalanceSettings { preserve_luminosity: true, ..Default::default() },
            dither: DitherSettings::default(),
            advanced_background: false,
            refine_edges: 12.0,
            matte_contrast: 25.0,
            shift_edge: 0.0,
        }
    }
}

/// `ImageAdjustmentPixels.clamp` and `FilterSettings.normalized`'s clamp: non-finite values fall back.
pub fn clamp(value: f64, lo: f64, hi: f64, fallback: f64) -> f64 {
    if value.is_finite() { value.max(lo).min(hi) } else { fallback }
}

fn clamped(c: AdjustmentColor) -> AdjustmentColor {
    AdjustmentColor { red: clamp(c.red, 0.0, 1.0, 0.0), green: clamp(c.green, 0.0, 1.0, 0.0), blue: clamp(c.blue, 0.0, 1.0, 0.0) }
}

/// Swift's `.rounded()`: halves away from zero.
fn rounded(v: f64) -> f64 {
    v.round()
}

impl FilterSettings {
    /// Reads `settings` from a corpus op; `None` gives the defaults.
    pub fn from_json(value: Option<&Value>) -> Result<Self> {
        let mut s = Self::default();
        let Some(value) = value else { return Ok(s) };
        let object = value.as_object().ok_or_else(|| anyhow!("settings must be a JSON object"))?;
        let mut keys: Vec<&String> = object.keys().collect();
        keys.sort();
        for key in keys {
            let item = &object[key];
            if item.is_null() {
                continue;
            }
            let number = || item.as_f64().filter(|_| !item.is_boolean()).ok_or_else(|| anyhow!("settings.{key} must be a number"));
            let flag = || item.as_bool().ok_or_else(|| anyhow!("settings.{key} must be true or false"));
            match key.as_str() {
                "radius" => s.radius = number()?,
                "angle" => s.angle = number()?,
                "distance" => s.distance = number()?,
                "amount" => s.amount = number()?,
                "vignetteAmount" => s.vignette_amount = number()?,
                "vignetteMidpoint" => s.vignette_midpoint = number()?,
                "vignetteRoundness" => s.vignette_roundness = number()?,
                "vignetteFeather" => s.vignette_feather = number()?,
                "vignetteHighlights" => s.vignette_highlights = number()?,
                "bloomAmount" => s.bloom_amount = number()?,
                "bloomRadius" => s.bloom_radius = number()?,
                "tonalAmount" => s.tonal_amount = number()?,
                "tonalRadius" => s.tonal_radius = number()?,
                "tonalShadows" => s.tonal_shadows = number()?,
                "tonalMidtones" => s.tonal_midtones = number()?,
                "tonalHighlights" => s.tonal_highlights = number()?,
                "distortion" => s.distortion = number()?,
                "refineEdges" => s.refine_edges = number()?,
                "matteContrast" => s.matte_contrast = number()?,
                "shiftEdge" => s.shift_edge = number()?,
                "gaussian" => s.gaussian = flag()?,
                "monochromatic" => s.monochromatic = flag()?,
                "vignetteColor" => s.vignette_color = patched!(AdjustmentColor, s.vignette_color, item, key),
                "curves" => s.curves = patched!(CurvesSettings, s.curves, item, key),
                "exposure" => s.exposure = patched!(ExposureSettings, s.exposure, item, key),
                "gradientMap" => s.gradient_map = patched!(GradientMapSettings, s.gradient_map, item, key),
                "grain" => s.grain = patched!(GrainSettings, s.grain, item, key),
                "blackWhite" => s.black_white = patched!(BlackWhiteSettings, s.black_white, item, key),
                "colorBalance" => s.color_balance = patched!(ColorBalanceSettings, s.color_balance, item, key),
                "dither" => s.dither = DitherSettings::from_json(item)?,
                "backgroundQuality" => {
                    s.advanced_background = match item.as_str() {
                        Some("Basic") => false,
                        Some("Advanced") => true,
                        _ => bail!("settings.backgroundQuality must be Basic or Advanced"),
                    }
                }
                "cameraRaw" => bail!("settings.cameraRaw: Camera Raw settings aren't supported by the harness"),
                other => bail!("settings.{other} isn't a FilterSettings field"),
            }
        }
        Ok(s)
    }

    /// `FilterSettings.normalized`.
    pub fn normalized(&self) -> Self {
        let mut r = self.clone();
        r.radius = clamp(self.radius, 0.1, 250.0, 1.0);
        r.angle = clamp(self.angle, -90.0, 90.0, 0.0);
        r.distance = clamp(self.distance, 1.0, 2000.0, 10.0);
        r.amount = clamp(self.amount, 0.1, 400.0, 10.0);
        r.vignette_amount = clamp(self.vignette_amount, 0.0, 100.0, 35.0);
        r.vignette_color = clamped(self.vignette_color);
        r.vignette_midpoint = clamp(self.vignette_midpoint, 0.0, 100.0, 50.0);
        r.vignette_roundness = clamp(self.vignette_roundness, -100.0, 100.0, 100.0);
        r.vignette_feather = clamp(self.vignette_feather, 0.0, 100.0, 60.0);
        r.vignette_highlights = clamp(self.vignette_highlights, 0.0, 100.0, 25.0);
        r.bloom_amount = clamp(self.bloom_amount, 0.0, 100.0, 40.0);
        r.bloom_radius = clamp(self.bloom_radius, 1.0, 150.0, 24.0);
        r.tonal_amount = clamp(self.tonal_amount, 0.0, 100.0, 50.0);
        r.tonal_radius = clamp(self.tonal_radius, 1.0, 100.0, 16.0);
        r.tonal_shadows = clamp(self.tonal_shadows, -100.0, 100.0, 40.0);
        r.tonal_midtones = clamp(self.tonal_midtones, -100.0, 100.0, 60.0);
        r.tonal_highlights = clamp(self.tonal_highlights, -100.0, 100.0, 30.0);
        r.distortion = clamp(self.distortion, -100.0, 100.0, 0.0);
        r.refine_edges = clamp(self.refine_edges, 0.0, 40.0, 12.0);
        r.matte_contrast = clamp(self.matte_contrast, 0.0, 100.0, 25.0);
        r.shift_edge = clamp(self.shift_edge, -10.0, 10.0, 0.0);
        let e = self.exposure;
        r.exposure = ExposureSettings {
            exposure: clamp(e.exposure, -20.0, 20.0, 0.0),
            offset: clamp(e.offset, -0.5, 0.5, 0.0),
            gamma: clamp(e.gamma, 0.01, 9.99, 1.0),
        };
        r.gradient_map.shadows = clamped(self.gradient_map.shadows);
        r.gradient_map.highlights = clamped(self.gradient_map.highlights);
        let g = self.grain;
        r.grain.amount = clamp(g.amount, 0.0, 100.0, 25.0);
        r.grain.size = clamp(g.size, 0.5, 20.0, 1.5);
        r.grain.roughness = clamp(g.roughness, 0.0, 100.0, 50.0);
        r.dither = self.dither.normalized();
        r
    }
}


fn merge(base: Value, patch: &Value, path: &str) -> Result<Value> {
    let Value::Object(mut result) = base else { bail!("{path} can't be set field by field") };
    let patch: &Map<String, Value> = patch.as_object().ok_or_else(|| anyhow!("{path} must be a JSON object"))?;
    for (key, value) in patch {
        let Some(current) = result.get(key) else { bail!("{path}.{key} isn't a field here") };
        let next = if current.is_object() && value.is_object() {
            merge(current.clone(), value, &format!("{path}.{key}"))?
        } else {
            value.clone()
        };
        result.insert(key.clone(), next);
    }
    Ok(Value::Object(result))
}

/// Filter > Dither's looks, in `DitherPixels.h`'s order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DitherStyle {
    Atkinson,
    FloydSteinberg,
    Bayer2,
    Bayer4,
    Bayer8,
    Dots,
    Lines,
    Diamonds,
    Patterns,
    Ascii,
    Scanlines,
}

impl DitherStyle {
    const NAMES: [(&'static str, DitherStyle); 11] = [
        ("Atkinson (Classic Mac)", DitherStyle::Atkinson),
        ("Floyd–Steinberg", DitherStyle::FloydSteinberg),
        ("Bayer 2 × 2", DitherStyle::Bayer2),
        ("Bayer 4 × 4", DitherStyle::Bayer4),
        ("Bayer 8 × 8", DitherStyle::Bayer8),
        ("Halftone Dots", DitherStyle::Dots),
        ("Halftone Lines", DitherStyle::Lines),
        ("Halftone Diamonds", DitherStyle::Diamonds),
        ("Mac Patterns", DitherStyle::Patterns),
        ("ASCII", DitherStyle::Ascii),
        ("Scanlines (CRT)", DitherStyle::Scanlines),
    ];

    pub fn code(self) -> u32 {
        self as u32
    }

    /// ASCII's characters and a CRT's lines are drawn at full resolution, not in chunky pixels.
    pub fn uses_pixel_size(self) -> bool {
        self != DitherStyle::Ascii && self != DitherStyle::Scanlines
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DitherColors {
    BlackWhite,
    TwoColors,
    Original,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DitherSettings {
    pub style: DitherStyle,
    pub pixel_size: f64,
    /// Square, or round dots.
    pub dot_pixels: bool,
    pub cell_size: f64,
    pub text_size: f64,
    pub line_spacing: f64,
    pub glow: f64,
    pub dots: f64,
    pub wobble: f64,
    pub angle: f64,
    pub levels: f64,
    pub diffusion: f64,
    pub density: f64,
    pub contrast: f64,
    pub colors: DitherColors,
    pub dark: AdjustmentColor,
    pub light: AdjustmentColor,
    pub light_on_dark: bool,
    pub characters: String,
}

impl Default for DitherSettings {
    fn default() -> Self {
        Self {
            style: DitherStyle::Atkinson,
            pixel_size: 2.0,
            dot_pixels: false,
            cell_size: 8.0,
            text_size: 14.0,
            line_spacing: 4.0,
            glow: 35.0,
            dots: 0.0,
            wobble: 0.0,
            angle: 45.0,
            levels: 2.0,
            diffusion: 100.0,
            density: 0.0,
            contrast: 0.0,
            colors: DitherColors::BlackWhite,
            dark: AdjustmentColor { red: 0.0, green: 0.0, blue: 0.0 },
            light: AdjustmentColor { red: 1.0, green: 1.0, blue: 1.0 },
            light_on_dark: true,
            characters: " .:-=+*#%@".into(),
        }
    }
}

impl DitherSettings {
    /// The harness's `FilterSettingsJSON.dither`: `DitherSettings` isn't Codable, so its fields are
    /// read one by one onto the defaults.
    fn from_json(value: &Value) -> Result<Self> {
        let object = value.as_object().ok_or_else(|| anyhow!("settings.dither must be a JSON object"))?;
        let mut s = Self::default();
        let mut keys: Vec<&String> = object.keys().collect();
        keys.sort();
        for key in keys {
            let item = &object[key];
            if item.is_null() {
                continue;
            }
            let at = format!("settings.dither.{key}");
            let number = || item.as_f64().filter(|_| !item.is_boolean()).ok_or_else(|| anyhow!("{at} must be a number"));
            let text = || item.as_str().ok_or_else(|| anyhow!("{at} must be a string"));
            match key.as_str() {
                "pixelSize" => s.pixel_size = number()?,
                "cellSize" => s.cell_size = number()?,
                "textSize" => s.text_size = number()?,
                "lineSpacing" => s.line_spacing = number()?,
                "glow" => s.glow = number()?,
                "dots" => s.dots = number()?,
                "wobble" => s.wobble = number()?,
                "angle" => s.angle = number()?,
                "levels" => s.levels = number()?,
                "diffusion" => s.diffusion = number()?,
                "density" => s.density = number()?,
                "contrast" => s.contrast = number()?,
                "style" => {
                    let name = text()?;
                    s.style = DitherStyle::NAMES.iter().find(|(n, _)| *n == name).map(|(_, v)| *v).ok_or_else(|| anyhow!("{at}: unknown style {name}"))?;
                }
                "pixelShape" => {
                    s.dot_pixels = match text()? {
                        "Square" => false,
                        "Dot" => true,
                        other => bail!("{at}: unknown pixel shape {other}"),
                    }
                }
                "colors" => {
                    s.colors = match text()? {
                        "Black & White" => DitherColors::BlackWhite,
                        "Two Colors" => DitherColors::TwoColors,
                        "Original" => DitherColors::Original,
                        other => bail!("{at}: unknown colors {other}"),
                    }
                }
                "dark" => s.dark = patched!(AdjustmentColor, s.dark, item, "dither.dark"),
                "light" => s.light = patched!(AdjustmentColor, s.light, item, "dither.light"),
                "lightOnDark" => s.light_on_dark = item.as_bool().ok_or_else(|| anyhow!("{at} must be true or false"))?,
                "characters" => s.characters = text()?.to_string(),
                other => bail!("settings.dither.{other} isn't a DitherSettings field"),
            }
        }
        Ok(s)
    }

    /// `DitherSettings.normalized`.
    pub fn normalized(&self) -> Self {
        let mut r = self.clone();
        r.pixel_size = rounded(clamp(self.pixel_size, 1.0, 32.0, 2.0));
        r.cell_size = rounded(clamp(self.cell_size, 4.0, 64.0, 8.0));
        r.text_size = rounded(clamp(self.text_size, 6.0, 64.0, 14.0));
        r.line_spacing = rounded(clamp(self.line_spacing, 2.0, 32.0, 4.0));
        r.glow = clamp(self.glow, 0.0, 100.0, 35.0);
        r.dots = clamp(self.dots, 0.0, 100.0, 0.0);
        r.wobble = clamp(self.wobble, 0.0, 64.0, 0.0);
        r.angle = clamp(self.angle, -90.0, 90.0, 45.0);
        r.levels = rounded(clamp(self.levels, 2.0, 8.0, 2.0));
        r.diffusion = clamp(self.diffusion, 0.0, 100.0, 100.0);
        r.density = clamp(self.density, -100.0, 100.0, 0.0);
        r.contrast = clamp(self.contrast, -100.0, 100.0, 0.0);
        r.dark = clamped(self.dark);
        r.light = clamped(self.light);
        r.characters = self.characters.chars().filter(|c| !matches!(c, '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}')).take(64).collect();
        r
    }
}
