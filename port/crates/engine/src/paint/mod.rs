//! Painting and retouching: Brush and Eraser on pixels and masks, Clone Stamp, Spot Healing,
//! Blur, Smudge and Liquify, as `EditorSession+Brush.swift`, `BrushStroke.swift` and
//! `SmudgeLiquify.swift` apply them.
//!
//! A `stroke` op is one press, drag and release. `Session` holds what the Mac's `EditorSession`
//! carries from one stroke to the next: each tool family's tip, the colors, Clone Stamp's source
//! and offset, and where the last stroke ended (for Shift-click lines).

mod geometry;
mod heal;
mod stroke;
mod warp;

#[cfg(test)]
mod tests;

use crate::RenderError;
use crate::gpu::Gpu;
use comp_format::Project;
use geometry::Point;
use serde_json::Value;

type Result<T> = std::result::Result<T, RenderError>;

fn failed<T>(message: impl Into<String>) -> Result<T> {
    Err(RenderError::Failed(anyhow::anyhow!(message.into())))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Brush,
    Eraser,
    Heal,
    Clone,
    Blur,
    Smudge,
    Liquify,
}

impl Tool {
    fn parse(name: &str) -> Option<Tool> {
        Some(match name {
            "brush" => Tool::Brush,
            "eraser" => Tool::Eraser,
            "heal" => Tool::Heal,
            "clone" => Tool::Clone,
            "blur" => Tool::Blur,
            "smudge" => Tool::Smudge,
            "liquify" => Tool::Liquify,
            _ => return None,
        })
    }

    /// `EditorSession.tipFamily`: Clone Stamp and the Smear tools keep tips of their own.
    fn family(self) -> usize {
        match self {
            Tool::Clone => 1,
            Tool::Blur | Tool::Smudge | Tool::Liquify => 2,
            _ => 0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Tip {
    pub diameter: f64,
    pub hardness: f64,
    pub opacity: f64,
}

/// What `EditorSession` keeps between strokes.
#[derive(Clone)]
pub struct Session {
    /// The tip of each family (`parkedBrushTips` plus the one in use).
    tips: [Tip; 3],
    smoothing: f64,
    blur_radius: f64,
    color: [f64; 3],
    mask_white: bool,
    healing_mode: i32,
    aligned: bool,
    sample_all: bool,
    clone_source: Option<Point>,
    clone_offset: Option<Point>,
    /// `lastBrushPoint`: where the last stroke ended, on which layer, and whether on its mask.
    last_point: Option<(Point, String, bool)>,
}

impl Default for Session {
    fn default() -> Self {
        let soft = Tip { diameter: 40.0, hardness: 0.0, opacity: 1.0 };
        Self {
            tips: [Tip { diameter: 40.0, hardness: 1.0, opacity: 1.0 }, soft, soft],
            smoothing: 0.0,
            blur_radius: 5.0,
            color: [0.0; 3],
            mask_white: false,
            healing_mode: 0,
            aligned: true,
            sample_all: false,
            clone_source: None,
            clone_offset: None,
            last_point: None,
        }
    }
}

/// A `stroke` op, read as the harness reads it (see parity/README.md).
struct Op {
    tool: Tool,
    layer: String,
    mask: bool,
    points: Vec<Point>,
    shift: bool,
}

fn point(v: &Value) -> Option<Point> {
    let a = v.as_array()?;
    (a.len() == 2).then(|| Some([a[0].as_f64()?, a[1].as_f64()?]))?
}

impl Session {
    /// Reads `op`, then selects its tool and sets the options bar, as the harness does.
    fn prepare(&mut self, op: &Value) -> Result<Op> {
        let tool = op.get("tool").and_then(|v| v.as_str()).and_then(Tool::parse);
        let Some(tool) = tool else { return failed("stroke needs a known tool") };
        let layer = op.get("layer").and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let mask = match op.get("target").and_then(|v| v.as_str()) {
            None | Some("pixels") => false,
            Some("mask") => true,
            Some(other) => return failed(format!("unknown stroke target `{other}`")),
        };
        let points: Vec<Point> = op.get("points").and_then(|v| v.as_array()).into_iter().flatten().filter_map(point).collect();
        if points.is_empty() {
            return failed("stroke needs points");
        }
        let settings = op.get("settings").cloned().unwrap_or(Value::Null);
        let tip = &mut self.tips[tool.family()];
        let num = |key: &str| settings.get(key).and_then(|v| v.as_f64());
        if let Some(v) = num("size") {
            tip.diameter = v;
        }
        if let Some(v) = num("hardness") {
            tip.hardness = v;
        }
        if let Some(v) = num("opacity") {
            tip.opacity = v;
        }
        if let Some(v) = num("smoothing") {
            self.smoothing = v;
        }
        if let Some(v) = num("blurRadius") {
            self.blur_radius = v;
        }
        if let Some(c) = settings.get("color").and_then(|v| v.as_array()) {
            for (i, v) in c.iter().take(3).enumerate() {
                self.color[i] = v.as_f64().unwrap_or(0.0);
            }
        }
        if let Some(v) = settings.get("white").and_then(|v| v.as_bool()) {
            self.mask_white = v;
        }
        if let Some(mode) = settings.get("healingMode").and_then(|v| v.as_str()) {
            self.healing_mode = match mode {
                "Content-Aware" => 0,
                "Create Texture" => return Err(RenderError::Unsupported("Spot Healing's Create Texture (a random seed)".into())),
                "Proximity Match" => 2,
                other => return failed(format!("unknown healing mode `{other}`")),
            };
        }
        if let Some(v) = settings.get("aligned").and_then(|v| v.as_bool()) {
            self.aligned = v;
        }
        if let Some(v) = settings.get("sampleAll").and_then(|v| v.as_bool()) {
            self.sample_all = v;
        }
        if let Some(s) = op.get("source").and_then(point) {
            // `setCloneSource`: a new source starts a new alignment.
            self.clone_source = Some(s);
            self.clone_offset = None;
        }
        Ok(Op { tool, layer, mask, points, shift: op.get("shift").and_then(|v| v.as_bool()).unwrap_or(false) })
    }

    /// `cloneStrokeOffset(at:)`.
    fn clone_stroke_offset(&self, at: Point) -> Option<Point> {
        let source = self.clone_source?;
        Some(self.clone_offset.filter(|_| self.aligned).unwrap_or([(source[0] - at[0]).round(), (source[1] - at[1]).round()]))
    }
}

/// `EditorSession.canPaint`, with `paintRefusal`'s reasons: what the Mac refuses to paint on.
fn refusal(project: &Project, op: &Op) -> Result<()> {
    let layers = &project.manifest.layers;
    let Some(layer) = layers.iter().find(|l| l.id == op.layer) else {
        return failed(format!("there's no layer {}", op.layer));
    };
    let by_id: std::collections::HashMap<&str, &comp_format::LayerRecord> = layers.iter().map(|l| (l.id.as_str(), l)).collect();
    if op.mask && !project.masks.contains_key(&op.layer) {
        return failed("the layer has no mask to paint");
    }
    if layer.is_group() && !op.mask {
        return failed(format!("“{}” is a folder, which has no pixels of its own.", layer.name));
    }
    if !layer.is_visible || crate::order::folders(layer, &by_id).iter().any(|f| !f.is_visible) {
        return failed(format!("“{}” is hidden, or inside a hidden folder. Show it to paint on it.", layer.name));
    }
    if op.mask && !layer.mask_enabled() {
        return failed("The layer mask is turned off.");
    }
    if !op.mask && layer.adjustment.is_some() {
        return failed(format!("“{}” is an adjustment layer, with no pixels to paint.", layer.name));
    }
    Ok(())
}

/// Applies one `stroke` op to `project`, as the harness drives the Mac's mouse handlers.
pub fn apply(gpu: &Gpu, project: &mut Project, session: &mut Session, op: &Value) -> Result<()> {
    let op = session.prepare(op)?;
    refusal(project, &op)?;
    let tip = session.tips[op.tool.family()];
    // mouseDown: a Shift-click paints on from the last stroke's end, on the same target.
    let shift_from = session.last_point.as_ref().filter(|(_, id, mask)| *id == op.layer && *mask == op.mask).map(|(p, _, _)| *p);
    let (first, rest): (Point, Vec<Point>) = match (op.shift, shift_from) {
        (true, Some(from)) => (from, op.points.clone()),
        _ => (op.points[0], op.points[1..].to_vec()),
    };
    let warp = matches!(op.tool, Tool::Smudge | Tool::Liquify);
    if warp {
        if op.mask {
            return failed("Smudge and Liquify work on a layer's pixels, not its mask");
        }
        // `beginWarp`, then every drag and the release, then `finishWarp`.
        let mut points = vec![first];
        points.extend(rest.iter().copied());
        points.push(*op.points.last().unwrap());
        // `continueBrush` moves `lastBrushPoint` to every point of a warp.
        session.last_point = Some((*points.last().unwrap(), op.layer.clone(), false));
        return warp::apply(gpu, project, &op.layer, op.tool == Tool::Smudge, tip, &points);
    }
    if op.mask && !matches!(op.tool, Tool::Brush | Tool::Eraser | Tool::Blur) {
        // `beginBrush` ignores the press: Spot Healing and Clone Stamp don't work on masks.
        return failed("the tool does nothing on a mask");
    }
    let offset = if op.tool == Tool::Clone {
        match session.clone_stroke_offset(first) {
            Some(o) => Some(o),
            None => return failed("Option-click where Clone Stamp should copy from first."),
        }
    } else {
        None
    };
    let settings = stroke::Settings {
        diameter: tip.diameter,
        hardness: tip.hardness,
        opacity: tip.opacity,
        color: if op.mask { [if session.mask_white { 1.0 } else { 0.0 }; 3] } else { session.color },
        erasing: op.tool == Tool::Eraser && !op.mask,
        healing: op.tool == Tool::Heal,
        healing_mode: session.healing_mode,
    };
    let mut stroke = stroke::Stroke::new(project, &op.layer, op.mask, settings, op.tool == Tool::Brush || op.tool == Tool::Eraser)?;
    if let Some(offset) = offset {
        stroke.set_clone(gpu, project, offset, session.sample_all)?;
        session.clone_offset = Some(offset);
    }
    if op.tool == Tool::Blur {
        stroke.set_blur(gpu, session.blur_radius)?;
    }
    stroke.path.append(first);
    // `brushAnchor`, `brushPointer` and `lastBrushPoint`, as `beginBrush` sets them.
    let mut anchor = first;
    let mut pointer = first;
    session.last_point = Some((first, op.layer.clone(), op.mask));
    let smoothing = op.tool == Tool::Brush || op.tool == Tool::Eraser;
    // mouseDragged for each later point, then mouseUp's `continueBrush` at the release point.
    for p in rest.iter().copied().chain(std::iter::once(*op.points.last().unwrap())) {
        pointer = p;
        // `smoothed`: the brush trails the pointer on a string `smoothing` long (the zoom is 1).
        let painted = if smoothing && session.smoothing > 0.0 {
            let radius = session.smoothing / 1f64.max(0.01);
            let delta = [p[0] - anchor[0], p[1] - anchor[1]];
            let distance = delta[0].hypot(delta[1]);
            if distance <= radius {
                None
            } else {
                let step = (distance - radius) / distance;
                anchor = [anchor[0] + delta[0] * step, anchor[1] + delta[1] * step];
                Some(anchor)
            }
        } else {
            Some(p)
        };
        if let Some(painted) = painted {
            stroke.path.append(painted);
            if let Some(last) = &mut session.last_point {
                last.0 = painted;
            }
        }
    }
    // `finishBrushImmediately`: with Smoothing, the stroke ends where the hand did.
    if smoothing && session.smoothing > 0.0 && pointer != anchor {
        stroke.path.append(pointer);
    }
    stroke.path.flush();
    stroke.finish(gpu, project)
}
