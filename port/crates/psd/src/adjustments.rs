//! Adjustment layers as the Mac reads them (`PSDAdjustments` in `PSDReader.swift`): Levels, Curves
//! and Hue/Saturation. Its quirks are kept: `curv` is read with a 2-byte channel count where
//! Photoshop stores a 4-byte channel mask, so real Photoshop Curves come in as identity curves.

use comp_format::{Adjustment, AdjustmentKind, ColorRange, CurvePoint, CurvesSettings, HueBand, HueSaturationSettings, LevelRange, RangeAdjustment, RangeMap};
use std::collections::HashMap;

pub fn parse(extra: &HashMap<String, Vec<u8>>) -> Option<Adjustment> {
    if let Some(data) = extra.get("levl") {
        return levels(data);
    }
    if let Some(data) = extra.get("curv") {
        return curves(data);
    }
    if let Some(data) = extra.get("hue2").or_else(|| extra.get("hue ")) {
        return hue(data);
    }
    None
}

fn u16_at(data: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([data[offset], data[offset + 1]])
}

fn i16_at(data: &[u8], offset: usize) -> i16 {
    u16_at(data, offset) as i16
}

/// `LevelRange.normalized`.
fn normalized(r: LevelRange) -> LevelRange {
    fn clamp(n: f64, lo: f64, hi: f64, fallback: f64) -> f64 {
        if n.is_finite() { n.max(lo).min(hi) } else { fallback }
    }
    let black = clamp(r.black, 0.0, 254.0, 0.0);
    LevelRange {
        black,
        white: clamp(r.white, black + 1.0, 255.0, 255.0),
        gamma: clamp(r.gamma, 0.1, 9.99, 1.0),
        output_black: clamp(r.output_black, 0.0, 255.0, 0.0),
        output_white: clamp(r.output_white, 0.0, 255.0, 255.0),
    }
}

/// `levl`: a version, then records of input black, input white, output black, output white and
/// gamma in hundredths, for RGB, red, green and blue.
fn levels(data: &[u8]) -> Option<Adjustment> {
    if data.len() < 292 {
        return None;
    }
    let mut adjustment = Adjustment::new(AdjustmentKind::Levels);
    for channel in 0..4 {
        let base = 2 + channel * 10;
        adjustment.levels.ranges[channel] = normalized(LevelRange {
            black: u16_at(data, base) as f64,
            white: u16_at(data, base + 2) as f64,
            output_black: u16_at(data, base + 4) as f64,
            output_white: u16_at(data, base + 6) as f64,
            gamma: u16_at(data, base + 8) as f64 / 100.0,
        });
    }
    Some(adjustment)
}

/// `CurvesSettings.isValid`.
fn curves_valid(settings: &CurvesSettings) -> bool {
    settings.channels.len() == 4
        && settings.channels.iter().all(|points| {
            (2..=32).contains(&points.len())
                && points.first().map(|p| p.x) == Some(0.0)
                && points.last().map(|p| p.x) == Some(255.0)
                && points.iter().all(|p| p.x.is_finite() && p.y.is_finite() && (0.0..=255.0).contains(&p.x) && (0.0..=255.0).contains(&p.y))
                && points.windows(2).all(|w| w[0].x < w[1].x)
        })
}

fn curves(data: &[u8]) -> Option<Adjustment> {
    if data.len() < 5 {
        return None;
    }
    let mut offset = 0;
    if data[offset] == 0 {
        offset += 1;
    }
    if offset + 2 > data.len() {
        return None;
    }
    let version = u16_at(data, offset);
    offset += 2;
    if version != 1 && version != 4 {
        return None;
    }
    if offset + 2 > data.len() {
        return None;
    }
    let count = u16_at(data, offset) as usize;
    offset += 2;
    let mut settings = CurvesSettings::default();
    for channel in 0..count.min(4) {
        if offset + 2 > data.len() {
            return None;
        }
        let points = u16_at(data, offset) as usize;
        offset += 2;
        let mut curve = Vec::new();
        for _ in 0..points {
            if offset + 4 > data.len() {
                return None;
            }
            let output = u16_at(data, offset) as f64;
            let input = u16_at(data, offset + 2) as f64;
            offset += 4;
            curve.push(CurvePoint { x: input.clamp(0.0, 255.0), y: output.clamp(0.0, 255.0) });
        }
        if curve.len() >= 2 {
            // Swift's sort isn't stable, but equal inputs fail validation below either way.
            curve.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap());
            if curve[0].x != 0.0 {
                curve.insert(0, CurvePoint { x: 0.0, y: curve[0].y });
            }
            let last = *curve.last().unwrap();
            if last.x != 255.0 {
                curve.push(CurvePoint { x: 255.0, y: last.y });
            }
            settings.channels[channel] = curve;
        }
    }
    if !curves_valid(&settings) {
        return None;
    }
    let mut adjustment = Adjustment::new(AdjustmentKind::Curves);
    adjustment.curves = settings;
    Some(adjustment)
}

/// `ColorRange.colorRanges`: every range but Master, in declaration order.
const COLOR_RANGES: [ColorRange; 6] =
    [ColorRange::Reds, ColorRange::Yellows, ColorRange::Greens, ColorRange::Cyans, ColorRange::Blues, ColorRange::Magentas];

const ALL_RANGES: [ColorRange; 7] = [
    ColorRange::Master,
    ColorRange::Reds,
    ColorRange::Yellows,
    ColorRange::Greens,
    ColorRange::Cyans,
    ColorRange::Blues,
    ColorRange::Magentas,
];

fn set<V>(map: &mut RangeMap<V>, range: ColorRange, value: V) {
    match map.0.iter_mut().find(|(r, _)| *r == range) {
        Some(entry) => entry.1 = value,
        None => map.0.push((range, value)),
    }
}

/// `hue2`: a version, the Colorize switch and a pad byte, the Colorize hue, saturation and
/// lightness, the Master's, then for Reds through Magentas the band and its values.
fn hue(data: &[u8]) -> Option<Adjustment> {
    if data.len() < 16 {
        return None;
    }
    let colorize = data[2] != 0;
    let values = |at: usize| RangeAdjustment {
        hue: i16_at(data, at) as f64,
        saturation: i16_at(data, at + 2) as f64,
        lightness: i16_at(data, at + 4) as f64,
    };
    // `HueSaturationSettings(colorize:)`: Master at zero, and every range's default band.
    let mut settings = HueSaturationSettings {
        range: ColorRange::Master,
        colorize,
        invert_range: false,
        adjustments: RangeMap(vec![(ColorRange::Master, RangeAdjustment { hue: 0.0, saturation: 0.0, lightness: 0.0 })]),
        bands: RangeMap(ALL_RANGES.iter().map(|&r| (r, r.default_band())).collect()),
    };
    // Colorize has values of its own; the Master applies otherwise.
    set(&mut settings.adjustments, ColorRange::Master, values(if colorize { 4 } else { 10 }));
    if !colorize {
        let mut offset = 16;
        for range in COLOR_RANGES {
            if offset + 14 > data.len() {
                break;
            }
            let degrees = |at: usize| {
                let value = (i16_at(data, at) as f64) % 360.0;
                if value < 0.0 { value + 360.0 } else { value }
            };
            let band =
                HueBand { falloff_start: degrees(offset), range_start: degrees(offset + 2), range_end: degrees(offset + 4), falloff_end: degrees(offset + 6) };
            set(&mut settings.bands, range, band);
            set(&mut settings.adjustments, range, values(offset + 8));
            offset += 14;
        }
    }
    let mut adjustment = Adjustment::new(AdjustmentKind::HueSaturation);
    adjustment.hsv_settings = Some(settings);
    Some(adjustment)
}
