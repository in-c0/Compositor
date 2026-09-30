//! The checks `ProjectStore.validate` makes before the Mac app accepts a manifest. The Mac app
//! refuses a whole file that fails any of them, so the port refuses it too, and never writes one.

use crate::*;
use anyhow::{Result, bail, ensure};
use std::collections::{HashMap, HashSet};

pub const MAX_SIDE: i64 = 30_000;

pub fn validate(m: &Manifest) -> Result<()> {
    ensure!(m.format == FORMAT, "not a Compositor project");
    ensure!((1..=CURRENT_VERSION).contains(&m.version), "unsupported format version {}", m.version);
    ensure!(m.color_space == "sRGB", "color space must be sRGB");
    if let Some(r) = m.resolution {
        ensure!(r.is_finite() && (1.0..=9600.0).contains(&r), "resolution out of range");
    }
    ensure!((1..=MAX_SIDE).contains(&m.width) && (1..=MAX_SIDE).contains(&m.height), "canvas too large");
    ensure!(m.layers.len() <= 10_000, "too many layers");
    let v = m.version;
    for layer in &m.layers {
        let group = layer.is_group();
        if let Some(text) = &layer.text {
            ensure!(text_is_valid(text), "invalid text on {}", layer.id);
            ensure!(text.color_runs.is_none() || v >= 10, "colorRuns need version 10");
            ensure!(text.font_runs.is_none() || v >= 11, "fontRuns need version 11");
            ensure!(layer.image_file.is_some() && !group && layer.adjustment.is_none(), "text layer shape");
        }
        if let Some(adj) = &layer.adjustment {
            ensure!(v >= 7 && !group && layer.image_file.is_none(), "adjustment layer shape");
            ensure!(adjustment_is_valid(adj), "invalid adjustment on {}", layer.id);
            if matches!(adj.kind, AdjustmentKind::GaussianBlur | AdjustmentKind::MotionBlur | AdjustmentKind::AddNoise) {
                ensure!(v >= 9, "{} needs version 9", adj.kind.name());
            }
        }
        if let Some(mask) = &layer.mask_file {
            ensure!(v >= if group { 6 } else { 4 }, "mask needs a later version");
            ensure!(*mask == format!("{}.mask.png", layer.id), "mask file must be named after its layer");
        }
        ensure!(layer.mask_enabled.is_none() || layer.mask_file.is_some(), "maskEnabled without mask");
        if let Some(p) = &layer.mask_placement {
            ensure!(transform_is_valid(p) && layer.mask_file.is_some(), "invalid maskPlacement");
        }
        let opacity = layer.opacity();
        let blend = layer.blend_mode();
        ensure!(opacity.is_finite() && (0.0..=1.0).contains(&opacity), "opacity out of range");
        ensure!(v >= 3 || (opacity == 1.0 && blend == BlendMode::Normal), "appearance needs version 3");
        ensure!(!group || (blend == BlendMode::Normal && (v >= 8 || opacity == 1.0)), "folders are pass-through");
    }
    validate_hierarchy(&m.layers)?;
    validate_clipping(&m.layers)?;
    if v < 5 {
        ensure!(m.layers.iter().all(|l| l.mask_source_id.is_none()), "clipping masks need version 5");
    }
    if v == 1 {
        ensure!(m.layers.iter().all(|l| l.parent_id.is_none() && l.is_group != Some(true)), "folders need version 2");
    }
    let mut ids = HashSet::new();
    for layer in &m.layers {
        ensure!(uuid::Uuid::parse_str(&layer.id).is_ok(), "layer id is not a UUID");
        ensure!(ids.insert(layer.id.to_ascii_uppercase()), "duplicate layer id");
        ensure!(transform_is_valid(&layer.transform), "invalid transform on {}", layer.id);
        ensure!(!layer.name.trim().is_empty() && layer.name.len() <= 16_384, "invalid layer name");
        if let Some(file) = &layer.image_file {
            ensure!(*file == format!("{}.png", layer.id), "image file must be named after its layer");
        }
    }
    if let Some(active) = &m.active_layer_id {
        ensure!(ids.contains(&active.to_ascii_uppercase()), "active layer is missing");
    }
    let guides = m.guides.as_deref().unwrap_or(&[]);
    if v < 8 {
        ensure!(guides.is_empty(), "guides need version 8");
    } else {
        ensure!(guides.len() <= 1_000, "too many guides");
        let mut seen = HashSet::new();
        for g in guides {
            ensure!(seen.insert(&g.id) && g.position.is_finite() && g.position.abs() <= 1_000_000.0, "invalid guide");
        }
    }
    Ok(())
}

pub fn transform_is_valid(t: &Transform) -> bool {
    let [x, y] = t.origin;
    let [w, h] = t.size;
    [x, y, w, h, t.rotation].iter().all(|n| n.is_finite())
        && (1.0..=300_000.0).contains(&w)
        && (1.0..=300_000.0).contains(&h)
        && x.abs() <= 1_000_000.0
        && y.abs() <= 1_000_000.0
}

fn validate_hierarchy(layers: &[LayerRecord]) -> Result<()> {
    let mut by_id: HashMap<&str, &LayerRecord> = HashMap::new();
    for l in layers {
        ensure!(by_id.insert(&l.id, l).is_none(), "duplicate layer id");
        ensure!(!l.is_group() || l.image_file.is_none(), "a folder cannot have pixels");
    }
    for l in layers {
        let mut seen: HashSet<&str> = HashSet::from([l.id.as_str()]);
        let mut parent = l.parent_id.as_deref();
        while let Some(id) = parent {
            let Some(node) = by_id.get(id) else { bail!("missing parent {id}") };
            ensure!(seen.len() <= 64 && seen.insert(id) && node.is_group(), "invalid folder nesting");
            parent = node.parent_id.as_deref();
        }
        ensure!(!(l.is_group() && seen.len() > 64), "folders nested too deeply");
    }
    Ok(())
}

fn validate_clipping(layers: &[LayerRecord]) -> Result<()> {
    let by_id: HashMap<&str, &LayerRecord> = layers.iter().map(|l| (l.id.as_str(), l)).collect();
    for l in layers {
        let mut path = HashSet::new();
        let mut current = Some(l.id.as_str());
        while let Some(id) = current {
            let Some(record) = by_id.get(id) else { bail!("missing clipping base {id}") };
            ensure!(path.len() < 256 && path.insert(id), "clipping cycle");
            if let Some(source) = record.mask_source_id.as_deref() {
                let base = by_id.get(source);
                ensure!(
                    !record.is_group() && base.is_some_and(|b| !b.is_group() && b.adjustment.is_none()),
                    "invalid clipping base"
                );
            }
            current = record.mask_source_id.as_deref();
        }
    }
    Ok(())
}

fn in_range(n: f64, lo: f64, hi: f64) -> bool {
    n.is_finite() && n >= lo && n <= hi
}

fn color_ok(r: f64, g: f64, b: f64) -> bool {
    [r, g, b].iter().all(|c| in_range(*c, 0.0, 1.0))
}

pub fn level_normalized(r: &LevelRange) -> LevelRange {
    let clamp = |n: f64, lo: f64, hi: f64, fallback: f64| if n.is_finite() { n.max(lo).min(hi) } else { fallback };
    let black = clamp(r.black, 0.0, 254.0, 0.0);
    LevelRange {
        black,
        white: clamp(r.white, black + 1.0, 255.0, 255.0),
        gamma: clamp(r.gamma, 0.1, 9.99, 1.0),
        output_black: clamp(r.output_black, 0.0, 255.0, 0.0),
        output_white: clamp(r.output_white, 0.0, 255.0, 255.0),
    }
}

fn curves_valid(c: &CurvesSettings) -> bool {
    c.channels.len() == 4
        && c.channels.iter().all(|p| {
            (2..=32).contains(&p.len())
                && p.first().unwrap().x == 0.0
                && p.last().unwrap().x == 255.0
                && p.iter().all(|q| in_range(q.x, 0.0, 255.0) && in_range(q.y, 0.0, 255.0))
                && p.windows(2).all(|w| w[0].x < w[1].x)
        })
}

pub fn adjustment_is_valid(a: &Adjustment) -> bool {
    let hsv_ok = match &a.hsv_settings {
        Some(s) => {
            s.adjustments.0.iter().all(|(_, r)| {
                in_range(r.hue, -360.0, 360.0) && in_range(r.saturation, -100.0, 100.0) && in_range(r.lightness, -100.0, 100.0)
            }) && s.bands.0.iter().all(|(_, b)| {
                [b.falloff_start, b.range_start, b.range_end, b.falloff_end].iter().all(|n| n.is_finite())
            })
        }
        None => true,
    };
    let exposure = a.exposure_settings.unwrap_or_default();
    let gradient = a.gradient_map_settings.unwrap_or_default();
    let grain = a.grain_settings.unwrap_or_default();
    let bw = a.black_white_settings.unwrap_or_default();
    let cb = a.color_balance_settings.unwrap_or_default();
    in_range(a.hue, -360.0, 360.0)
        && in_range(a.saturation, -100.0, 100.0)
        && in_range(a.lightness, -100.0, 100.0)
        && hsv_ok
        && a.levels.ranges.len() == 4
        && a.levels.ranges.iter().all(|r| level_normalized(r) == *r)
        && curves_valid(&a.curves)
        && in_range(exposure.exposure, -20.0, 20.0)
        && in_range(exposure.offset, -0.5, 0.5)
        && in_range(exposure.gamma, 0.01, 9.99)
        && color_ok(gradient.shadows.red, gradient.shadows.green, gradient.shadows.blue)
        && color_ok(gradient.highlights.red, gradient.highlights.green, gradient.highlights.blue)
        && in_range(grain.amount, 0.0, 100.0)
        && in_range(grain.size, 0.5, 20.0)
        && in_range(grain.roughness, 0.0, 100.0)
        && [bw.reds, bw.yellows, bw.greens, bw.cyans, bw.blues, bw.magentas].iter().all(|n| in_range(*n, -200.0, 300.0))
        && in_range(bw.tint_hue, 0.0, 360.0)
        && in_range(bw.tint_saturation, 0.0, 100.0)
        && [
            cb.shadow_cyan_red, cb.shadow_magenta_green, cb.shadow_yellow_blue,
            cb.mid_cyan_red, cb.mid_magenta_green, cb.mid_yellow_blue,
            cb.highlight_cyan_red, cb.highlight_magenta_green, cb.highlight_yellow_blue,
        ]
        .iter()
        .all(|n| in_range(*n, -100.0, 100.0))
        && in_range(a.blur_radius.unwrap_or(10.0), 0.1, 250.0)
        && in_range(a.motion_angle.unwrap_or(0.0), -90.0, 90.0)
        && in_range(a.motion_distance.unwrap_or(10.0), 1.0, 2000.0)
        && in_range(a.noise_amount.unwrap_or(10.0), 0.1, 400.0)
}

fn runs_valid(runs: impl Iterator<Item = (i64, i64)>, content_len: i64) -> bool {
    let mut end = 0;
    let mut any = false;
    for (location, length) in runs {
        if location < end || length <= 0 {
            return false;
        }
        end = location + length;
        any = true;
    }
    any && end <= content_len
}

pub fn text_is_valid(t: &TextStyle) -> bool {
    let len = t.content.encode_utf16().count() as i64;
    let box_ok = t.box_size.is_none_or(|[w, h]| {
        in_range(w, 16.0, MAX_SIDE as f64) && in_range(h, 16.0, MAX_SIDE as f64) && w * h <= 200_000_000.0
    });
    len <= 100_000
        && box_ok
        && in_range(t.font_size, 1.0, 2000.0)
        && color_ok(t.red, t.green, t.blue)
        && in_range(t.tracking, -100.0, 1000.0)
        && in_range(t.leading, 0.0, 5000.0)
        && t.color_runs.as_ref().is_none_or(|runs| {
            runs.iter().all(|r| color_ok(r.red, r.green, r.blue))
                && runs_valid(runs.iter().map(|r| (r.location, r.length)), len)
        })
        && t.font_runs.as_ref().is_none_or(|runs| {
            runs.iter().all(|r| !r.font_name.is_empty() && r.font_name.chars().count() <= 200 && !r.font_name.contains('\n'))
                && runs_valid(runs.iter().map(|r| (r.location, r.length)), len)
        })
}
