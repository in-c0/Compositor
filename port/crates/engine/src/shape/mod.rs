//! The Shape and Gradient tools (`ShapeTool.swift`, `Gradient.swift`) and Move / Transform's
//! resize of a layer, which draws a shape layer again at its new size (`redrawShape`).
//!
//! Shapes are drawn as Core Graphics fills and strokes their paths, and gradients as its axial
//! and radial shadings draw a `CGGradient`: `raster` holds the drawing, fitted to references, and
//! `gradient` the tool's edit of a layer's pixels.

mod gradient;
mod raster;

use crate::RenderError;
use crate::gpu::Gpu;
use comp_format::{Asset, LayerRecord, Project, ShapeKind, ShapeStyle, Transform};
use serde_json::Value;

type Result<T> = std::result::Result<T, RenderError>;

fn failed<T>(message: String) -> Result<T> {
    Err(RenderError::Failed(anyhow::anyhow!(message)))
}

/// `EditorSession.maxShapePixels`, the budget of one import.
const MAX_SHAPE_PIXELS: i64 = 200_000_000;

/// The ops this module handles.
pub const OPS: [&str; 3] = ["shape", "gradient", "resizeLayer"];

pub fn apply_op(gpu: &Gpu, project: &mut Project, op: &Value) -> Result<()> {
    match op.get("op").and_then(|v| v.as_str()) {
        Some("shape") => shape(gpu, project, op),
        Some("gradient") => gradient::apply(gpu, project, op),
        Some("resizeLayer") => resize_layer(gpu, project, op),
        other => failed(format!("not a shape op: {other:?}")),
    }
}

fn point(op: &Value, key: &str) -> Result<[f64; 2]> {
    match op.get(key).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_f64()).collect::<Vec<_>>()) {
        Some(p) if p.len() == 2 => Ok([p[0], p[1]]),
        _ => failed(format!("{key} must be [x, y]")),
    }
}

fn color(op: &Value, key: &str) -> Result<Option<[f64; 3]>> {
    match op.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_array().map(|a| a.iter().filter_map(|v| v.as_f64()).collect::<Vec<_>>()) {
            Some(c) if c.len() == 3 && c.iter().all(|x| (0.0..=1.0).contains(x)) => Ok(Some([c[0], c[1], c[2]])),
            _ => failed(format!("{key} must be [red, green, blue], 0-1 each")),
        },
    }
}

/// A layer by UUID, or by name (the topmost with it) for one an earlier op made; the active layer
/// when `op` names none.
fn target(project: &Project, op: &Value) -> Result<Option<usize>> {
    let layers = &project.manifest.layers;
    match op.get("layer").and_then(|v| v.as_str()) {
        Some(reference) => {
            let by_id = layers.iter().position(|l| l.id.eq_ignore_ascii_case(reference));
            match by_id.or_else(|| layers.iter().rposition(|l| l.name == reference)) {
                Some(i) => Ok(Some(i)),
                None => failed(format!("there's no layer “{reference}”")),
            }
        }
        None => Ok(project.manifest.active_layer_id.as_ref().and_then(|id| layers.iter().position(|l| &l.id == id))),
    }
}

/// `DragBox.rect`: whole pixels from the rounded anchor to the rounded point.
fn drag_box(anchor: [f64; 2], point: [f64; 2], square: bool, from_center: bool) -> [f64; 4] {
    let (mut dx, mut dy) = (point[0].round() - anchor[0], point[1].round() - anchor[1]);
    if square {
        let side = dx.abs().max(dy.abs());
        dx = if dx < 0.0 { -side } else { side };
        dy = if dy < 0.0 { -side } else { side };
    }
    if from_center {
        [anchor[0] - dx.abs(), anchor[1] - dy.abs(), dx.abs() * 2.0, dy.abs() * 2.0]
    } else {
        [anchor[0].min(anchor[0] + dx), anchor[1].min(anchor[1] + dy), dx.abs(), dy.abs()]
    }
}

fn kind_of(op: &Value) -> Result<ShapeKind> {
    match op.get("kind").and_then(|v| v.as_str()) {
        Some("Rectangle") => Ok(ShapeKind::Rectangle),
        Some("Ellipse") => Ok(ShapeKind::Ellipse),
        Some("Line") => Ok(ShapeKind::Line),
        other => failed(format!("unknown shape kind {other:?}")),
    }
}

/// `beginShape`, `dragShape` and `finishShape`: the shape on a new layer above the active one.
fn shape(gpu: &Gpu, project: &mut Project, op: &Value) -> Result<()> {
    let kind = kind_of(op)?;
    let (from, to) = (point(op, "from")?, point(op, "to")?);
    let flag = |key: &str| op.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
    let (square, from_center) = (flag("square"), flag("fromCenter"));
    let color = color(op, "color")?.unwrap_or([0.0; 3]);
    let corner_radius = if kind == ShapeKind::Rectangle { op.get("cornerRadius").and_then(|v| v.as_f64()).unwrap_or(0.0) } else { 0.0 };
    let thickness = op.get("lineWidth").and_then(|v| v.as_f64()).unwrap_or(4.0);
    if let Some(i) = target(project, op)? {
        project.manifest.active_layer_id = Some(project.manifest.layers[i].id.clone());
    }
    let anchor = [from[0].round(), from[1].round()];
    // A line keeps the exact point it was dragged to; Shift snaps its angle to eighths of a turn.
    let mut end = to;
    if kind == ShapeKind::Line && square {
        let (dx, dy) = (to[0] - anchor[0], to[1] - anchor[1]);
        let angle = (dy.atan2(dx) / std::f64::consts::FRAC_PI_4).round() * std::f64::consts::FRAC_PI_4;
        let length = dx.hypot(dy);
        end = [anchor[0] + angle.cos() * length, anchor[1] + angle.sin() * length];
    }
    let mut rect = drag_box(anchor, end, square && kind != ShapeKind::Line, from_center);
    let mut ends = None;
    if kind == ShapeKind::Line {
        let (a, b) = (anchor, end);
        // The box of the two ends, grown by half the thickness all round.
        rect = [a[0].min(b[0]) - thickness / 2.0, a[1].min(b[1]) - thickness / 2.0, (b[0] - a[0]).abs() + thickness, (b[1] - a[1]).abs() + thickness];
        ends = Some((a, b));
    }
    if rect[2] < 1.0 || rect[3] < 1.0 {
        return failed("the drag made no shape layer".into());
    }
    if (rect[2] as i64) * (rect[3] as i64) > MAX_SHAPE_PIXELS {
        return failed("that shape is too large".into());
    }
    let unit = |p: [f64; 2]| {
        [
            if rect[2] > 0.0 { (p[0] - rect[0]) / rect[2] } else { 0.5 },
            if rect[3] > 0.0 { (p[1] - rect[1]) / rect[3] } else { 0.5 },
        ]
    };
    let style = ShapeStyle {
        kind,
        red: color[0],
        green: color[1],
        blue: color[2],
        corner_radius,
        line_width: (kind == ShapeKind::Line).then_some(thickness),
        start: ends.map(|e| unit(e.0)),
        end: ends.map(|e| unit(e.1)),
    };
    let image = raster::shape_image(gpu, &style, [rect[2], rect[3]])?;
    add_layer(project, image, [rect[0], rect[1]], next_shape_name(project, kind), style);
    Ok(())
}

/// "Rectangle 1", "Ellipse 2", …, skipping names already in the document.
fn next_shape_name(project: &Project, kind: ShapeKind) -> String {
    let names: std::collections::HashSet<&str> = project.manifest.layers.iter().map(|l| l.name.as_str()).collect();
    (1..).map(|n| format!("{} {n}", kind.name())).find(|name| !names.contains(name.as_str())).expect("a free name")
}

/// `addPixelLayer`: a new layer just above the active one (inside it when it's a folder), made active.
fn add_layer(project: &mut Project, straight: image::RgbaImage, origin: [f64; 2], name: String, style: ShapeStyle) {
    let m = &mut project.manifest;
    let active = m.active_layer_id.as_ref().and_then(|id| m.layers.iter().position(|l| &l.id == id));
    let parent_id = active.and_then(|i| if m.layers[i].is_group() { Some(m.layers[i].id.clone()) } else { m.layers[i].parent_id.clone() });
    let index = active.map_or(m.layers.len(), |i| i + 1);
    let id = uuid::Uuid::new_v4().to_string().to_ascii_uppercase();
    let (w, h) = straight.dimensions();
    m.layers.insert(index, LayerRecord {
        image_file: Some(format!("{id}.png")),
        id: id.clone(),
        name,
        is_visible: true,
        transform: Transform::at(origin[0], origin[1], w as f64, h as f64),
        parent_id,
        is_group: None,
        opacity: None,
        blend_mode: None,
        mask_file: None,
        mask_enabled: None,
        mask_source_id: None,
        adjustment: None,
        mask_placement: None,
        mask_linked: None,
        shape: Some(style),
        effects: None,
        text: None,
    });
    m.active_layer_id = Some(id.clone());
    project.images.insert(id, Asset::new(straight));
}

/// `beginTransform`, `previewTransform` and `commitTransform` with the layer's box set to `rect`:
/// a shape layer given a new size draws its shape again at that size (`redrawShape`).
fn resize_layer(gpu: &Gpu, project: &mut Project, op: &Value) -> Result<()> {
    let Some(i) = target(project, op)? else { return failed("there's no active layer to resize".into()) };
    let rect = match op.get("rect").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_f64()).collect::<Vec<_>>()) {
        Some(r) if r.len() == 4 => [r[0], r[1], r[2], r[3]],
        _ => return failed("rect must be [x, y, width, height]".into()),
    };
    let layer = &project.manifest.layers[i];
    if layer.is_group() || layer.image_file.is_none() || layer.adjustment.is_some() {
        return Err(RenderError::Unsupported("transforming folders, adjustment layers and empty layers".into()));
    }
    if layer.mask_file.is_some() {
        return Err(RenderError::Unsupported("transforming a layer with a mask".into()));
    }
    project.manifest.active_layer_id = Some(layer.id.clone());
    let mut transform = layer.transform;
    transform.origin = [rect[0], rect[1]];
    transform.size = [rect[2], rect[3]];
    let id = layer.id.clone();
    let shape = layer.shape.clone();
    project.manifest.layers[i].transform = transform;
    // `redrawShape`: the shape at the box's size in whole pixels, when that differs from its pixels.
    let Some(style) = shape else { return Ok(()) };
    let (w, h) = ((rect[2].round() as i64).max(1), (rect[3].round() as i64).max(1));
    let current = project.images.get(&id).map(|a| a.pixels.dimensions());
    if current == Some((w as u32, h as u32)) || w * h > MAX_SHAPE_PIXELS {
        return Ok(());
    }
    let image = raster::shape_image(gpu, &style, [w as f64, h as f64])?;
    project.images.insert(id, Asset::new(image));
    Ok(())
}

/// `(c*255 + a/2)/a`: premultiplied bytes as the Mac writes them to PNG.
pub(crate) fn unpremultiply(pixels: &mut [u8]) {
    for p in pixels.chunks_exact_mut(4) {
        let a = p[3] as u32;
        if a == 0 {
            p[..3].fill(0);
        } else if a < 255 {
            for c in &mut p[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drags_make_whole_pixel_boxes() {
        assert_eq!(drag_box([10.0, 10.0], [30.4, 25.6], false, false), [10.0, 10.0, 20.0, 16.0]);
        assert_eq!(drag_box([10.0, 10.0], [4.0, 40.0], true, false), [-20.0, 10.0, 30.0, 30.0]);
        assert_eq!(drag_box([32.0, 32.0], [45.0, 40.0], false, true), [19.0, 24.0, 26.0, 16.0]);
    }

    #[test]
    fn premultiplied_bytes_are_saved_as_the_mac_saves_them() {
        let mut pixels = [7, 70, 0, 73, 0, 0, 0, 0, 10, 20, 30, 255];
        unpremultiply(&mut pixels);
        assert_eq!(pixels, [24, 245, 0, 73, 0, 0, 0, 0, 10, 20, 30, 255]);
    }
}
