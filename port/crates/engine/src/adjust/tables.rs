//! Parameter-only precomputation, on the CPU in `f64` exactly as the Swift code does it: the
//! lookup tables and cubes the Mac builds once per setting before its per-pixel pass.

use comp_format::{
    Adjustment, ColorRange, CurvesSettings, ExposureSettings, GradientMapSettings, HueBand, HueSaturationSettings,
    LevelRange, LevelsSettings, RangeAdjustment, RangeMap,
};

/// `LayerAdjustment.resolvedHSV`: the range-aware settings, or the legacy Master-only fields.
pub fn resolved_hsv(adjustment: &Adjustment) -> HueSaturationSettings {
    adjustment.hsv_settings.clone().unwrap_or_else(|| HueSaturationSettings {
        range: ColorRange::Master,
        colorize: adjustment.colorize,
        invert_range: false,
        adjustments: RangeMap(vec![(
            ColorRange::Master,
            RangeAdjustment { hue: adjustment.hue, saturation: adjustment.saturation, lightness: adjustment.lightness },
        )]),
        bands: RangeMap(ColorRange::ALL.iter().map(|&r| (r, r.default_band())).collect()),
    })
}

/// `HueBand.forward`.
fn forward(from: f64, to: f64) -> f64 {
    let delta = (to - from) % 360.0;
    if delta < 0.0 { delta + 360.0 } else { delta }
}

/// `HueBand.weight(of:)`.
fn band_weight(band: &HueBand, hue: f64) -> f64 {
    let span = forward(band.falloff_start, band.falloff_end);
    if !(span > 0.0) {
        return 1.0;
    }
    let position = forward(band.falloff_start, hue);
    if !(position <= span) {
        return 0.0;
    }
    let ramp_in = forward(band.falloff_start, band.range_start);
    let plateau_end = forward(band.falloff_start, band.range_end);
    if position < ramp_in {
        return if ramp_in > 0.0 { position / ramp_in } else { 1.0 };
    }
    if position <= plateau_end {
        return 1.0;
    }
    let ramp_out = span - plateau_end;
    if ramp_out > 0.0 { (span - position) / ramp_out } else { 1.0 }
}

/// `HueSaturationSettings.weight(of:hue:)`.
fn range_weight(settings: &HueSaturationSettings, range: ColorRange, hue: f64) -> f64 {
    if range == ColorRange::Master {
        return 1.0;
    }
    let band = settings.bands.get(range).copied().unwrap_or_else(|| range.default_band());
    let weight = band_weight(&band, hue);
    if settings.invert_range && range == settings.range { 1.0 - weight } else { weight }
}

/// `HueSaturationFilter.hueResponse`: every range's pull on each whole degree 0…360. Swift walks
/// a dictionary here, in an order that changes from run to run; the ranges are summed in the
/// order they are stored, which only differs from the Mac in the last bit when several ranges
/// overlap.
fn hue_response(settings: &HueSaturationSettings) -> Vec<[f64; 3]> {
    let identity = RangeAdjustment { hue: 0.0, saturation: 0.0, lightness: 0.0 };
    (0..=360)
        .map(|degree| {
            let mut response = [0.0; 3];
            for (range, adjustment) in &settings.adjustments.0 {
                if *adjustment == identity {
                    continue;
                }
                let weight = range_weight(settings, *range, degree as f64);
                if !(weight > 0.0) {
                    continue;
                }
                response[0] += adjustment.hue * weight;
                response[1] += adjustment.saturation * weight;
                response[2] += adjustment.lightness * weight;
            }
            response
        })
        .collect()
}

fn to_hsl(red: f64, green: f64, blue: f64) -> (f64, f64, f64) {
    let high = red.max(green).max(blue);
    let low = red.min(green).min(blue);
    let lightness = (high + low) / 2.0;
    let delta = high - low;
    if !(delta > 0.0) {
        return (0.0, 0.0, lightness);
    }
    let saturation = delta / (1.0 - (2.0 * lightness - 1.0).abs());
    let mut hue = if high == red {
        (green - blue) / delta
    } else if high == green {
        (blue - red) / delta + 2.0
    } else {
        (red - green) / delta + 4.0
    };
    hue *= 60.0;
    if hue < 0.0 {
        hue += 360.0;
    }
    (hue, saturation.min(1.0), lightness)
}

fn to_rgb(hue: f64, saturation: f64, lightness: f64) -> [f64; 3] {
    if !(saturation > 0.0) {
        return [lightness; 3];
    }
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let sector = hue / 60.0;
    let second = chroma * (1.0 - ((sector % 2.0) - 1.0).abs());
    let base = lightness - chroma / 2.0;
    let (red, green, blue) = match sector as i64 {
        0 => (chroma, second, 0.0),
        1 => (second, chroma, 0.0),
        2 => (0.0, chroma, second),
        3 => (0.0, second, chroma),
        4 => (second, 0.0, chroma),
        _ => (chroma, 0.0, second),
    };
    [(red + base).clamp(0.0, 1.0), (green + base).clamp(0.0, 1.0), (blue + base).clamp(0.0, 1.0)]
}

/// `HueSaturationFilter.adjustedSaturation`.
fn adjusted_saturation(saturation: f64, amount: f64) -> f64 {
    let amount = (amount / 100.0).clamp(-1.0, 1.0);
    if !(amount > 0.0) {
        return (saturation * (1.0 + amount)).max(0.0);
    }
    if amount >= 1.0 {
        if saturation > 0.0 { 1.0 } else { 0.0 }
    } else {
        (saturation / (1.0 - amount)).min(1.0)
    }
}

/// `HueSaturationFilter.adjust`.
fn adjust_hsl(red: f64, green: f64, blue: f64, settings: &HueSaturationSettings, response: &[[f64; 3]]) -> [f64; 3] {
    let (mut hue, mut saturation, mut lightness) = to_hsl(red, green, blue);
    let lightness_amount;
    if settings.colorize {
        let selected = settings.adjustments.get(settings.range).copied();
        let (h, s, l) = selected.map_or((0.0, 0.0, 0.0), |a| (a.hue, a.saturation, a.lightness));
        hue = h % 360.0;
        saturation = (s / 100.0).clamp(0.0, 1.0);
        lightness_amount = l / 100.0;
    } else {
        let index = (hue.round() as i64).clamp(0, response.len() as i64 - 1) as usize;
        let sampled = response[index];
        lightness_amount = sampled[2] / 100.0;
        hue = (hue + sampled[0]) % 360.0;
        if hue < 0.0 {
            hue += 360.0;
        }
        saturation = adjusted_saturation(saturation, sampled[1]);
    }
    let amount = lightness_amount.clamp(-1.0, 1.0);
    lightness = if amount >= 0.0 { lightness + (1.0 - lightness) * amount } else { lightness * (1.0 + amount) };
    to_rgb(hue, saturation, lightness.clamp(0.0, 1.0))
}

/// `HueSaturationFilter.dimension`.
pub const CUBE_DIMENSION: usize = 33;

/// `HueSaturationFilter.buildCube`: RGBA floats, red varying fastest.
pub fn hsl_cube(settings: &HueSaturationSettings) -> Vec<f32> {
    let response = hue_response(settings);
    let n = CUBE_DIMENSION;
    let step = (n - 1) as f64;
    let mut values = Vec::with_capacity(n * n * n * 4);
    for blue in 0..n {
        for green in 0..n {
            for red in 0..n {
                let c = adjust_hsl(red as f64 / step, green as f64 / step, blue as f64 / step, settings, &response);
                values.extend_from_slice(&[c[0] as f32, c[1] as f32, c[2] as f32, 1.0]);
            }
        }
    }
    values
}

/// `LevelRange.normalized`.
fn normalized(range: &LevelRange) -> LevelRange {
    fn clamp(n: f64, lo: f64, hi: f64, fallback: f64) -> f64 {
        if n.is_finite() { n.max(lo).min(hi) } else { fallback }
    }
    let black = clamp(range.black, 0.0, 254.0, 0.0);
    LevelRange {
        black,
        white: clamp(range.white, black + 1.0, 255.0, 255.0),
        gamma: clamp(range.gamma, 0.1, 9.99, 1.0),
        output_black: clamp(range.output_black, 0.0, 255.0, 0.0),
        output_white: clamp(range.output_white, 0.0, 255.0, 255.0),
    }
}

/// `LevelRange.apply`.
fn level(range: &LevelRange, value: f64) -> f64 {
    let s = normalized(range);
    let input = ((value * 255.0 - s.black) / (s.white - s.black)).max(0.0).min(1.0);
    (s.output_black + input.powf(1.0 / s.gamma) * (s.output_white - s.output_black)) / 255.0
}

/// `LevelsSettings.isIdentity`.
pub fn levels_identity(levels: &LevelsSettings) -> bool {
    levels.ranges.iter().all(|r| normalized(r) == LevelRange::default())
}

/// The three `levels_apply` tables `LevelsFilter.run` builds: red, green, blue.
pub fn levels_tables(levels: &LevelsSettings) -> Vec<f32> {
    let ranges = &levels.ranges;
    (1..=3)
        .flat_map(|channel| (0..=255).map(move |i| level(&ranges[0], level(&ranges[channel], i as f64 / 255.0)) as f32))
        .collect()
}

/// `CurvesSettings.value`: monotone cubic Hermite through the points of one channel.
fn curve_value(curves: &CurvesSettings, x: f64, channel: usize) -> f64 {
    let p = &curves.channels[channel];
    let last = p.iter().rposition(|q| q.x <= x).unwrap_or(0) as i64;
    let i = last.max(0).min(p.len() as i64 - 2) as usize;
    let d: Vec<f64> = p.windows(2).map(|w| (w[1].y - w[0].y) / (w[1].x - w[0].x)).collect();
    let slope = |j: usize| -> f64 {
        if j == 0 {
            return d[0];
        }
        if j == p.len() - 1 {
            return *d.last().unwrap();
        }
        if d[j - 1] * d[j] <= 0.0 {
            return 0.0;
        }
        2.0 / (1.0 / d[j - 1] + 1.0 / d[j])
    };
    let h = p[i + 1].x - p[i].x;
    let t = ((x - p[i].x) / h).max(0.0).min(1.0);
    let y = (2.0 * t * t * t - 3.0 * t * t + 1.0) * p[i].y
        + (t * t * t - 2.0 * t * t + t) * h * slope(i)
        + (-2.0 * t * t * t + 3.0 * t * t) * p[i + 1].y
        + (t * t * t - t * t) * h * slope(i + 1);
    y.max(0.0).min(255.0)
}

/// The `levels_apply` tables `CurvesSettings.apply` builds: each channel's curve, then the RGB one.
pub fn curves_tables(curves: &CurvesSettings) -> Vec<f32> {
    (1..=3)
        .flat_map(|channel| {
            (0..=255).map(move |i| (curve_value(curves, curve_value(curves, i as f64, channel), 0) / 255.0) as f32)
        })
        .collect()
}

/// `ExposureSettings.table`, repeated for red, green and blue.
pub fn exposure_tables(e: &ExposureSettings) -> Vec<f32> {
    let scale = 2f64.powf(e.exposure);
    let table: Vec<f32> = (0..=255)
        .map(|index| {
            let encoded = index as f64 / 255.0;
            let mut linear = if encoded <= 0.04045 { encoded / 12.92 } else { ((encoded + 0.055) / 1.055).powf(2.4) };
            linear = (linear * scale + e.offset).max(0.0).powf(1.0 / e.gamma);
            let output = if linear <= 0.0031308 { linear * 12.92 } else { 1.055 * linear.powf(1.0 / 2.4) - 0.055 };
            output.max(0.0).min(1.0) as f32
        })
        .collect();
    table.repeat(3)
}

/// `GradientMapSettings.apply`'s table: 256 RGB byte triples.
pub fn gradient_map_table(g: &GradientMapSettings) -> Vec<u32> {
    let (dark, light) = if g.reversed { (g.highlights, g.shadows) } else { (g.shadows, g.highlights) };
    let channel = |from: f64, to: f64, t: f64| -> u32 {
        let value = from + (to - from) * t;
        (value * 255.0).round().max(0.0).min(255.0) as u32
    };
    (0..=255)
        .map(|index| {
            let t = index as f64 / 255.0;
            channel(dark.red, light.red, t) | channel(dark.green, light.green, t) << 8 | channel(dark.blue, light.blue, t) << 16
        })
        .collect()
}

/// One axis of `grain_field`'s lattice for every pixel column (or row): the cell index as the C
/// casts it to 32 bits, and the smoothstepped fraction, for the main grain and the fine detail.
/// These depend only on position and the settings, so they are computed here in `double` as the
/// C does, and the shader only combines a column's entry with a row's.
pub fn grain_axis(count: u32, origin: f64, units_per_pixel: f64, size: f64, detail_size: f64) -> Vec<[u32; 4]> {
    let cell = |u: f64, scale: f64| -> (u32, f32) {
        let c = (u / scale).floor();
        let mut t = (u / scale - c) as f32;
        // `t * t * (3.0f - 2.0f * t)`; 2t is exact, so contraction can't change it.
        t = t * t * (3.0f32 - 2.0f32 * t);
        (c as i64 as u32, t)
    };
    (0..count)
        .map(|i| {
            // `originX + ((double)x + 0.5) * unitsPerPixel`, which clang contracts into an fma.
            let u = (i as f64 + 0.5).mul_add(units_per_pixel, origin);
            let (smooth_cell, smooth_t) = cell(u, size);
            let (fine_cell, fine_t) = cell(u, detail_size);
            [smooth_cell, smooth_t.to_bits(), fine_cell, fine_t.to_bits()]
        })
        .collect()
}
