//! Text layers, as the Mac's Type tool makes them (`Document/TypeTool.swift`): the style laid out
//! by TextKit and drawn by Core Text into an sRGB bitmap the size of the text box, which becomes
//! the layer's pixels.
//!
//! Core Text and Apple's fonts aren't available on Windows, so this is an approximation: the same
//! layout rules, glyph outlines from the named face when it is installed (or an open substitute),
//! filled with exact area coverage. See the `type` entry in parity/features.toml for the gap.

pub mod fonts;
pub mod layout;
pub mod raster;

use crate::RenderError;
use comp_format::{Asset, LayerRecord, Project, TextAlignment, TextStyle, Transform};
use image::RgbaImage;
use serde_json::Value;

/// How far Core Graphics' font smoothing grows each glyph edge, in pixels: in proportion to the
/// size, more for lighter text (five steps of the color's luminance), and never over 0.3 px.
pub fn growth(size: f64, color: [f64; 3]) -> f64 {
    let linear = |c: f64| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
    let luminance = 0.2126 * linear(color[0]) + 0.7152 * linear(color[1]) + 0.0722 * linear(color[2]);
    let level = (luminance * 4.0).round();
    (size * (0.0058 + 0.0025 * level)).min(0.3)
}

/// `LayerTextStyle.padding`: the gap between the text and its box.
pub const PADDING: f64 = 12.0;

/// `LayerTextStyle()`: the Type tool's defaults.
pub fn default_style() -> TextStyle {
    TextStyle {
        content: "Text".into(),
        font_name: "Helvetica".into(),
        font_size: 72.0,
        red: 0.0,
        green: 0.0,
        blue: 0.0,
        alignment: TextAlignment::Left,
        tracking: 0.0,
        leading: 0.0,
        box_size: None,
        color_runs: None,
        font_runs: None,
    }
}

fn failed(message: String) -> RenderError {
    RenderError::Failed(anyhow::anyhow!(message))
}

/// `base` with the fields in `patch` changed; `null` clears an optional field.
pub fn patched(base: &TextStyle, patch: &Value) -> Result<TextStyle, RenderError> {
    let Some(patch) = patch.as_object() else { return Err(failed("a text style must be a JSON object".into())) };
    let mut object = serde_json::to_value(base).map_err(|e| failed(e.to_string()))?;
    for (key, value) in patch {
        if value.is_null() {
            object.as_object_mut().unwrap().remove(key);
        } else {
            object[key] = value.clone();
        }
    }
    serde_json::from_value(object).map_err(|e| failed(format!("text style: {e}")))
}

fn line_height(style: &TextStyle) -> f64 {
    if style.leading > 0.0 { style.leading } else { style.font_size * 1.2 }
}

/// `EditorSession.textBoxSize`: a box's own size, or for point text what the text measures plus
/// its padding.
pub fn box_size(style: &TextStyle) -> (f64, f64) {
    if let Some([w, h]) = style.box_size {
        return (w, h);
    }
    let measured = layout::layout(style, 100_000.0, 100_000.0);
    let line = line_height(style).ceil();
    (
        (measured.width + PADDING * 2.0 + style.font_size * 0.1).ceil().max(16.0),
        (measured.height.max(line) + PADDING * 2.0).ceil().max(16.0),
    )
}

/// `EditorSession.textImage`: the text drawn into a transparent box, straight-alpha RGBA8.
pub fn text_image(style: &TextStyle) -> Result<RgbaImage, RenderError> {
    if !comp_format::text_is_valid(style) {
        return Err(failed("the text style is outside the app's limits".into()));
    }
    let (w, h) = box_size(style);
    let (width, height) = (w.ceil(), h.ceil());
    if !(1.0..=30_000.0).contains(&width) || !(1.0..=30_000.0).contains(&height) || width * height > 200_000_000.0 {
        return Err(failed("the text box is too large".into()));
    }
    let (width, height) = (width as u32, height as u32);
    let laid = layout::layout(style, (width as f64 - 2.0 * PADDING).max(1.0), (height as f64 - 2.0 * PADDING).max(1.0));
    // Premultiplied, as the Mac's bitmap context holds it.
    let mut pixels = vec![[0u8; 4]; (width * height) as usize];
    // Core Graphics places a glyph at one of ceil(36 / size) positions per pixel across (whole
    // pixels from 36 px up) and on a whole pixel down, rounding toward the lower right.
    let steps = (36.0 / style.font_size).ceil();
    for glyph in laid.glyphs.iter().filter(|g| !g.blank) {
        let grow = growth(style.font_size, glyph.color);
        // The grown glyph moves up by as much as it grows, so its bottom stays on the baseline.
        let origin = (((PADDING + glyph.x) * steps).floor() / steps, (PADDING + glyph.y).ceil() - grow);
        let Some(mask) = raster::glyph_mask(glyph.face, glyph.id, style.font_size, origin, grow) else { continue };
        let color = glyph.color.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u32);
        for row in 0..mask.height {
            let y = mask.top + row as i32;
            if y < 0 || y >= height as i32 {
                continue;
            }
            for col in 0..mask.width {
                let x = mask.left + col as i32;
                if x < 0 || x >= width as i32 {
                    continue;
                }
                let m = (mask.coverage[row * mask.width + col] * 255.0).round() as u32;
                if m == 0 {
                    continue;
                }
                let p = &mut pixels[(y as u32 * width + x as u32) as usize];
                // Each byte is the color times the coverage, rounded down, and where glyphs overlap
                // the larger value stays: Core Graphics doesn't lay one glyph over another.
                let src = [color[0], color[1], color[2], 255];
                for c in 0..4 {
                    p[c] = p[c].max((src[c] * m / 255) as u8);
                }
            }
        }
    }
    let straight: Vec<u8> = pixels
        .iter()
        .flat_map(|p| {
            let a = p[3] as u32;
            if a == 0 {
                [0, 0, 0, 0]
            } else {
                let un = |c: u8| ((c as u32 * 255 + a / 2) / a).min(255) as u8;
                [un(p[0]), un(p[1]), un(p[2]), p[3]]
            }
        })
        .collect();
    Ok(RgbaImage::from_raw(width, height, straight).expect("box size"))
}

/// `EditorSession.layerName(for:)`: the text's first words on one line.
pub fn layer_name(content: &str) -> String {
    let flattened = content.split(|c: char| c.is_whitespace()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ");
    if flattened.is_empty() { "Text".into() } else { flattened.chars().take(40).collect() }
}

fn numbers(op: &Value, key: &str, count: usize) -> Option<Vec<f64>> {
    let values: Vec<f64> = op.get(key)?.as_array()?.iter().filter_map(|v| v.as_f64()).collect();
    (values.len() == count).then_some(values)
}

/// The `text` op: the Type tool set to the op's style, a click at `point` (point text) or a box
/// dragged out as `rect`, the text typed and committed (`beginText`, then `applyText`).
pub fn apply_text(project: &Project, op: &Value) -> Result<Project, RenderError> {
    let style = patched(&default_style(), op.get("style").unwrap_or(&Value::Object(Default::default())))?;
    let (point, rect) = (numbers(op, "point", 2), numbers(op, "rect", 4));
    // What `beginText` starts from: the controls' style without its content, runs or box.
    let mut draft = style.clone();
    let origin = match (point, rect) {
        (_, Some(r)) => {
            let size = [r[2].round().max(16.0), r[3].round().max(16.0)];
            draft.box_size = Some(size);
            [r[0], r[1]]
        }
        (Some(p), None) => {
            draft.box_size = None;
            // The first baseline on the click: a fixed line height leaves its extra room above the letters.
            let descent = fonts::face(&style.font_name).descender_at(style.font_size).abs();
            let baseline = PADDING + line_height(&style) - descent;
            [p[0] - PADDING, p[1] - baseline]
        }
        (None, None) => return Err(failed("text needs a point or a rect".into())),
    };
    if !comp_format::text_is_valid(&draft) {
        return Err(failed("the text style is outside the app's limits".into()));
    }
    let mut result = project.clone();
    if draft.content.trim().is_empty() {
        return Ok(result);
    }
    let image = text_image(&draft)?;
    let m = &mut result.manifest;
    let id = uuid::Uuid::new_v4().to_string().to_ascii_uppercase();
    let active = m.active_layer_id.clone();
    let active_layer = active.as_deref().and_then(|a| m.layers.iter().find(|l| l.id == a));
    let parent = match active_layer {
        Some(l) if l.is_group() => Some(l.id.clone()),
        Some(l) => l.parent_id.clone(),
        None => None,
    };
    let index = active.as_deref().and_then(|a| m.layers.iter().position(|l| l.id == a)).map(|i| i + 1).unwrap_or(m.layers.len());
    let name = layer_name(&draft.content);
    m.layers.insert(index, LayerRecord {
        image_file: Some(format!("{id}.png")),
        id: id.clone(),
        name,
        is_visible: true,
        transform: Transform::at(origin[0], origin[1], image.width() as f64, image.height() as f64),
        parent_id: parent,
        is_group: None,
        opacity: None,
        blend_mode: None,
        mask_file: None,
        mask_enabled: None,
        mask_source_id: None,
        adjustment: None,
        mask_placement: None,
        mask_linked: None,
        shape: None,
        effects: None,
        text: Some(draft),
    });
    m.active_layer_id = Some(id.clone());
    result.images.insert(id, Asset::new(image));
    Ok(result)
}

/// The `editText` op: Layer > Edit Text on `layer`, the style's fields changed, then committed.
pub fn apply_edit_text(project: &Project, op: &Value) -> Result<Project, RenderError> {
    let id = op.get("layer").and_then(|v| v.as_str()).ok_or_else(|| failed("editText needs a layer".into()))?;
    let mut result = project.clone();
    let m = &mut result.manifest;
    let index = m.layers.iter().position(|l| l.id.eq_ignore_ascii_case(id)).ok_or_else(|| failed(format!("there's no layer {id}")))?;
    let layer = &m.layers[index];
    let (Some(old), Some(asset)) = (layer.text.clone(), result.images.get(&layer.id)) else {
        return Err(failed(format!("layer “{}” isn't editable text", layer.name)));
    };
    let style = patched(&old, op.get("style").unwrap_or(&Value::Null))?;
    let image = text_image(&style)?;
    let (old_w, old_h) = asset.pixels.dimensions();
    m.active_layer_id = Some(m.layers[index].id.clone());
    if style == old {
        return Ok(result);
    }
    let layer = &mut m.layers[index];
    let mut t = layer.transform;
    if style.box_size.is_none() {
        // Keep the transformed upper-left corner and the scale, rotation and flips.
        let anchor = corner(&t);
        t.size = [image.width() as f64 * t.size[0] / old_w as f64, image.height() as f64 * t.size[1] / old_h as f64];
        let moved = corner(&t);
        t.origin[0] += anchor[0] - moved[0];
        t.origin[1] += anchor[1] - moved[1];
    }
    if layer.mask_file.is_some() && layer.mask_placement.is_none() {
        layer.mask_placement = Some(layer.transform);
    }
    layer.transform = t;
    layer.text = Some(style);
    let key = layer.id.clone();
    result.images.insert(key, Asset::new(image));
    Ok(result)
}

/// `LayerTransform.point(.zero)`: where the layer's upper-left corner lands.
fn corner(t: &Transform) -> [f64; 2] {
    let radians = (t.rotation % 360.0) * std::f64::consts::PI / 180.0;
    let (x, y) = (-0.5 * t.size[0], -0.5 * t.size[1]);
    let center = [t.origin[0] + t.size[0] / 2.0, t.origin[1] + t.size[1] / 2.0];
    [center[0] + x * radians.cos() - y * radians.sin(), center[1] + x * radians.sin() + y * radians.cos()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn growth_steps_with_luminance_and_caps() {
        assert!((growth(20.0, [0.0; 3]) - 0.116).abs() < 1e-9);
        assert!((growth(10.0, [0.5; 3]) - 0.083).abs() < 1e-9);
        assert!((growth(10.0, [1.0; 3]) - 0.158).abs() < 1e-9);
        assert_eq!(growth(40.0, [1.0; 3]), 0.3);
        // Blue is dark, green light.
        assert_eq!(growth(10.0, [0.0, 0.0, 1.0]), growth(10.0, [0.0; 3]));
        assert!(growth(10.0, [0.0, 1.0, 0.0]) > growth(10.0, [0.5; 3]));
    }

    #[test]
    fn tracking_follows_every_letter() {
        let mut style = default_style();
        style.content = "llll".into();
        style.font_size = 20.0;
        let plain = layout::layout(&style, 1e5, 1e5).width;
        style.tracking = 2.5;
        let tracked = layout::layout(&style, 1e5, 1e5);
        assert!((tracked.width - (plain + 4.0 * 2.5)).abs() < 1e-9);
        assert!((tracked.glyphs[1].x - tracked.glyphs[0].x - plain / 4.0 - 2.5).abs() < 1e-9);
    }

    #[test]
    fn lines_break_between_words_and_stack_by_leading() {
        let mut style = default_style();
        style.content = "one two three".into();
        style.font_size = 20.0;
        style.leading = 30.0;
        let one = layout::layout(&style, 1e5, 1e5);
        let narrow = layout::layout(&style, one.width * 0.6, 1e5);
        let lines: std::collections::BTreeSet<i64> = narrow.glyphs.iter().map(|g| (g.y * 1000.0) as i64).collect();
        assert_eq!(lines.len(), 2);
        let ys: Vec<i64> = lines.into_iter().collect();
        let gap = fonts::face(&style.font_name).line_gap_at(20.0);
        assert!(((ys[1] - ys[0]) as f64 / 1000.0 - (30.0 + gap)).abs() < 1e-2);
    }

    #[test]
    fn point_text_box_is_what_it_measures_plus_padding() {
        let mut style = default_style();
        style.content = "Text".into();
        style.font_size = 24.0;
        let measured = layout::layout(&style, 1e5, 1e5);
        let (w, h) = box_size(&style);
        assert_eq!(w, (measured.width + 24.0 + 2.4).ceil());
        assert_eq!(h, (measured.height.max((24.0f64 * 1.2).ceil()) + 24.0).ceil());
        let image = text_image(&style).unwrap();
        assert_eq!(image.dimensions(), (w as u32, h as u32));
        assert!(image.pixels().any(|p| p[3] == 255));
    }
}
