use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

pub const FORMAT: &str = "com.compositor.project";
/// `ProjectManifest.current` in the Mac app.
pub const CURRENT_VERSION: i64 = 11;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub format: String,
    pub version: i64,
    pub color_space: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<f64>,
    #[serde(rename = "documentID")]
    pub document_id: String,
    pub width: i64,
    pub height: i64,
    #[serde(rename = "activeLayerID", default, skip_serializing_if = "Option::is_none")]
    pub active_layer_id: Option<String>,
    pub layers: Vec<LayerRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guides: Option<Vec<Guide>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Guide {
    pub id: String,
    pub axis: GuideAxis,
    pub position: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuideAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerRecord {
    pub id: String,
    pub name: String,
    pub is_visible: bool,
    pub transform: Transform,
    /// Required key; `null` for groups, adjustment layers and blank layers.
    pub image_file: Option<String>,
    #[serde(rename = "parentID", default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_group: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend_mode: Option<BlendMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask_enabled: Option<bool>,
    /// The clipping-mask base: the layer whose coverage clips this one.
    #[serde(rename = "maskSourceID", default, skip_serializing_if = "Option::is_none")]
    pub mask_source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adjustment: Option<Adjustment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask_placement: Option<Transform>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask_linked: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<ShapeStyle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<Effects>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<TextStyle>,
}

impl LayerRecord {
    pub fn is_group(&self) -> bool {
        self.is_group.unwrap_or(false)
    }
    pub fn opacity(&self) -> f64 {
        self.opacity.unwrap_or(1.0)
    }
    pub fn blend_mode(&self) -> BlendMode {
        self.blend_mode.unwrap_or(BlendMode::Normal)
    }
    pub fn mask_enabled(&self) -> bool {
        self.mask_file.is_some() && self.mask_enabled.unwrap_or(true)
    }
    pub fn mask_linked(&self) -> bool {
        self.mask_linked.unwrap_or(true)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transform {
    pub origin: [f64; 2],
    pub size: [f64; 2],
    pub rotation: f64,
    pub flip_x: bool,
    pub flip_y: bool,
    pub sampling: Sampling,
}

impl Transform {
    pub fn at(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { origin: [x, y], size: [w, h], rotation: 0.0, flip_x: false, flip_y: false, sampling: Sampling::High }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Sampling {
    Nearest,
    Smooth,
    #[serde(rename = "High quality")]
    High,
}

macro_rules! string_enum {
    ($name:ident { $($variant:ident = $text:literal),* $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name { $(#[serde(rename = $text)] $variant),* }
        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),*];
            pub fn name(self) -> &'static str { match self { $($name::$variant => $text),* } }
        }
    };
}

// Photoshop's order, as `LayerBlendMode` lists them.
string_enum!(BlendMode {
    Normal = "Normal", Darken = "Darken", Multiply = "Multiply", ColorBurn = "Color Burn",
    LinearBurn = "Linear Burn", Lighten = "Lighten", Screen = "Screen", ColorDodge = "Color Dodge",
    LinearDodge = "Linear Dodge (Add)", Overlay = "Overlay", SoftLight = "Soft Light",
    HardLight = "Hard Light", VividLight = "Vivid Light", LinearLight = "Linear Light",
    PinLight = "Pin Light", HardMix = "Hard Mix", Difference = "Difference", Exclusion = "Exclusion",
    Subtract = "Subtract", Divide = "Divide", Hue = "Hue", Saturation = "Saturation",
    Color = "Color", Luminosity = "Luminosity",
});

string_enum!(AdjustmentKind {
    HueSaturation = "Hue/Saturation", Levels = "Levels", Curves = "Curves", Exposure = "Exposure",
    GradientMap = "Gradient Map", Grain = "Grain", AddNoise = "Add Noise",
    GaussianBlur = "Gaussian Blur", MotionBlur = "Motion Blur", Invert = "Invert",
    BlackWhite = "Black & White", ColorBalance = "Color Balance",
});

string_enum!(Channel { Rgb = "RGB", Red = "Red", Green = "Green", Blue = "Blue" });

string_enum!(ColorRange {
    Master = "Master", Reds = "Reds", Yellows = "Yellows", Greens = "Greens",
    Cyans = "Cyans", Blues = "Blues", Magentas = "Magentas",
});

string_enum!(TextAlignment { Left = "Left", Center = "Center", Right = "Right" });
string_enum!(ShapeKind { Rectangle = "Rectangle", Ellipse = "Ellipse", Line = "Line" });

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Adjustment {
    pub kind: AdjustmentKind,
    pub hue: f64,
    pub saturation: f64,
    pub lightness: f64,
    pub colorize: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hsv_settings: Option<HueSaturationSettings>,
    pub levels: LevelsSettings,
    pub curves: CurvesSettings,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exposure_settings: Option<ExposureSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradient_map_settings: Option<GradientMapSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grain_settings: Option<GrainSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub black_white_settings: Option<BlackWhiteSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_balance_settings: Option<ColorBalanceSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blur_radius: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion_angle: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion_distance: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noise_amount: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noise_gaussian: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noise_monochromatic: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noise_seed: Option<u32>,
}

impl Adjustment {
    /// An identity adjustment of `kind`, as `LayerAdjustment(kind:)` makes one.
    pub fn new(kind: AdjustmentKind) -> Self {
        Self {
            kind,
            hue: 0.0,
            saturation: 0.0,
            lightness: 0.0,
            colorize: false,
            hsv_settings: None,
            levels: LevelsSettings::default(),
            curves: CurvesSettings::default(),
            exposure_settings: None,
            gradient_map_settings: None,
            grain_settings: None,
            black_white_settings: None,
            color_balance_settings: None,
            blur_radius: None,
            motion_angle: None,
            motion_distance: None,
            noise_amount: None,
            noise_gaussian: None,
            noise_monochromatic: None,
            noise_seed: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LevelRange {
    pub black: f64,
    pub gamma: f64,
    pub white: f64,
    pub output_black: f64,
    pub output_white: f64,
}

impl Default for LevelRange {
    fn default() -> Self {
        Self { black: 0.0, gamma: 1.0, white: 255.0, output_black: 0.0, output_white: 255.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LevelsSettings {
    pub channel: Channel,
    /// RGB, then red, green, blue.
    pub ranges: Vec<LevelRange>,
}

impl Default for LevelsSettings {
    fn default() -> Self {
        Self { channel: Channel::Rgb, ranges: vec![LevelRange::default(); 4] }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CurvesSettings {
    pub channel: Channel,
    /// RGB, then red, green, blue.
    pub channels: Vec<Vec<CurvePoint>>,
}

impl Default for CurvesSettings {
    fn default() -> Self {
        let identity = vec![CurvePoint { x: 0.0, y: 0.0 }, CurvePoint { x: 255.0, y: 255.0 }];
        Self { channel: Channel::Rgb, channels: vec![identity; 4] }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RangeAdjustment {
    pub hue: f64,
    pub saturation: f64,
    pub lightness: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HueBand {
    pub falloff_start: f64,
    pub range_start: f64,
    pub range_end: f64,
    pub falloff_end: f64,
}

impl ColorRange {
    pub fn default_band(self) -> HueBand {
        let (a, b, c, d) = match self {
            ColorRange::Master => (0.0, 0.0, 360.0, 360.0),
            ColorRange::Reds => (315.0, 345.0, 15.0, 45.0),
            ColorRange::Yellows => (15.0, 45.0, 75.0, 105.0),
            ColorRange::Greens => (75.0, 105.0, 135.0, 165.0),
            ColorRange::Cyans => (135.0, 165.0, 195.0, 225.0),
            ColorRange::Blues => (195.0, 225.0, 255.0, 285.0),
            ColorRange::Magentas => (255.0, 285.0, 315.0, 345.0),
        };
        HueBand { falloff_start: a, range_start: b, range_end: c, falloff_end: d }
    }
}

/// Swift encodes a `[ColorRange: V]` dictionary as a flat `[key, value, key, value…]` array,
/// because `ColorRange` is not `CodingKeyRepresentable`.
#[derive(Clone, Debug, PartialEq)]
pub struct RangeMap<V>(pub Vec<(ColorRange, V)>);

impl<V> RangeMap<V> {
    pub fn get(&self, range: ColorRange) -> Option<&V> {
        self.0.iter().find(|(r, _)| *r == range).map(|(_, v)| v)
    }
}

impl<V: Serialize> Serialize for RangeMap<V> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut flat = Vec::with_capacity(self.0.len() * 2);
        for (range, value) in &self.0 {
            flat.push(Value::String(range.name().into()));
            flat.push(serde_json::to_value(value).map_err(serde::ser::Error::custom)?);
        }
        flat.serialize(s)
    }
}

impl<'de, V: serde::de::DeserializeOwned> Deserialize<'de> for RangeMap<V> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let raw = Value::deserialize(d)?;
        let pairs: Vec<(ColorRange, V)> = match raw {
            Value::Array(items) => {
                if items.len() % 2 != 0 {
                    return Err(D::Error::custom("odd-length ColorRange dictionary"));
                }
                items
                    .chunks(2)
                    .map(|pair| {
                        let key = serde_json::from_value(pair[0].clone()).map_err(D::Error::custom)?;
                        let value = serde_json::from_value(pair[1].clone()).map_err(D::Error::custom)?;
                        Ok((key, value))
                    })
                    .collect::<Result<_, D::Error>>()?
            }
            // Tolerated on read in case a writer used an object; never written.
            Value::Object(map) => map
                .into_iter()
                .map(|(k, v)| {
                    let key = serde_json::from_value(Value::String(k)).map_err(D::Error::custom)?;
                    Ok((key, serde_json::from_value(v).map_err(D::Error::custom)?))
                })
                .collect::<Result<_, D::Error>>()?,
            _ => return Err(D::Error::custom("expected a ColorRange dictionary")),
        };
        Ok(RangeMap(pairs))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HueSaturationSettings {
    pub range: ColorRange,
    pub colorize: bool,
    pub invert_range: bool,
    pub adjustments: RangeMap<RangeAdjustment>,
    pub bands: RangeMap<HueBand>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExposureSettings {
    pub exposure: f64,
    pub offset: f64,
    pub gamma: f64,
}

impl Default for ExposureSettings {
    fn default() -> Self {
        Self { exposure: 0.0, offset: 0.0, gamma: 1.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdjustmentColor {
    pub red: f64,
    pub green: f64,
    pub blue: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradientMapSettings {
    pub shadows: AdjustmentColor,
    pub highlights: AdjustmentColor,
    pub reversed: bool,
}

impl Default for GradientMapSettings {
    fn default() -> Self {
        Self {
            shadows: AdjustmentColor { red: 0.0, green: 0.0, blue: 0.0 },
            highlights: AdjustmentColor { red: 1.0, green: 1.0, blue: 1.0 },
            reversed: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GrainSettings {
    pub amount: f64,
    pub size: f64,
    pub roughness: f64,
    pub seed: u32,
}

impl Default for GrainSettings {
    fn default() -> Self {
        Self { amount: 25.0, size: 1.5, roughness: 50.0, seed: 0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlackWhiteSettings {
    pub reds: f64,
    pub yellows: f64,
    pub greens: f64,
    pub cyans: f64,
    pub blues: f64,
    pub magentas: f64,
    pub tint: bool,
    pub tint_hue: f64,
    pub tint_saturation: f64,
}

impl Default for BlackWhiteSettings {
    fn default() -> Self {
        Self {
            reds: 40.0, yellows: 60.0, greens: 40.0, cyans: 60.0, blues: 20.0, magentas: 80.0,
            tint: false, tint_hue: 40.0, tint_saturation: 20.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorBalanceSettings {
    pub shadow_cyan_red: f64,
    pub shadow_magenta_green: f64,
    pub shadow_yellow_blue: f64,
    pub mid_cyan_red: f64,
    pub mid_magenta_green: f64,
    pub mid_yellow_blue: f64,
    pub highlight_cyan_red: f64,
    pub highlight_magenta_green: f64,
    pub highlight_yellow_blue: f64,
    pub preserve_luminosity: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Effects {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<StrokeEffect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<ShadowEffect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_overlay: Option<ColorOverlayEffect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner_shadow: Option<ShadowEffect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outer_glow: Option<GlowEffect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner_glow: Option<GlowEffect>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrokeEffect {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    pub size: f64,
    pub red: f64,
    pub green: f64,
    pub blue: f64,
    pub opacity: f64,
    pub inside: bool,
}

/// Drop shadow and inner shadow share a shape; only their defaults differ.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShadowEffect {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    pub angle: f64,
    pub distance: f64,
    pub blur: f64,
    pub red: f64,
    pub green: f64,
    pub blue: f64,
    pub opacity: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColorOverlayEffect {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    pub red: f64,
    pub green: f64,
    pub blue: f64,
    pub opacity: f64,
}

/// Outer glow and inner glow share a shape; only their defaults differ.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GlowEffect {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    pub size: f64,
    pub red: f64,
    pub green: f64,
    pub blue: f64,
    pub opacity: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextStyle {
    pub content: String,
    pub font_name: String,
    pub font_size: f64,
    pub red: f64,
    pub green: f64,
    pub blue: f64,
    pub alignment: TextAlignment,
    pub tracking: f64,
    pub leading: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub box_size: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_runs: Option<Vec<TextColorRun>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_runs: Option<Vec<TextFontRun>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextColorRun {
    pub location: i64,
    pub length: i64,
    pub red: f64,
    pub green: f64,
    pub blue: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextFontRun {
    pub location: i64,
    pub length: i64,
    pub font_name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeStyle {
    pub kind: ShapeKind,
    pub red: f64,
    pub green: f64,
    pub blue: f64,
    pub corner_radius: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_width: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<[f64; 2]>,
}

