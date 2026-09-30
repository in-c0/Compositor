//! Selections: the Marquee, Lasso, Magic Wand, Color Range, the Select menu, Modify and loading a
//! layer or mask as a selection, as `Document/Selection.swift`, `MagicWand.swift`,
//! `ColorRangeSelection.swift` and `MaskTracing.swift` make them.
//!
//! The Mac keeps a selection as a `CGPath` (plus an anti-alias flag and a feather), combines
//! outlines with Core Graphics' path operations, and rasterizes the path whenever it needs
//! coverage (`DocumentSelection.coverage`). The port keeps the outline as lines and cubics
//! ([`geom::Region`]) and rasterizes it on the GPU ([`coverage`]).

pub mod geom;
pub(crate) mod raster;
pub mod wand;

use crate::RenderError;
use crate::gpu::Gpu;
use comp_format::{Project, Transform};
use geom::Region;
use serde_json::Value;

/// `DocumentSelection`. An empty region is an explicit empty selection, which selects nothing.
#[derive(Clone, Debug)]
pub struct Selection {
    pub region: Region,
    pub antialiased: bool,
    /// How far the edge fades, in document pixels.
    pub feather: f64,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        geom::is_empty(&self.region)
    }
}

/// The corpus ops that make or change the selection.
pub const OPS: &[&str] =
    &["marquee", "lasso", "wand", "objectSelection", "colorRange", "selectAll", "deselect", "invertSelection", "modifySelection", "loadSelection"];

type Result<T> = std::result::Result<T, RenderError>;

fn failed<T>(message: String) -> Result<T> {
    Err(RenderError::Failed(anyhow::anyhow!(message)))
}

fn unsupported<T>(what: &str) -> Result<T> {
    Err(RenderError::Unsupported(what.to_string()))
}

/// A path operation's result, or what the port can't match yet.
fn exact<T>(result: std::result::Result<T, geom::Unmatched>) -> Result<T> {
    result.map_err(|what| RenderError::Unsupported(what.to_string()))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    New,
    Add,
    Subtract,
}

/// Reads an op's fields the way the harness does: a missing optional field takes the app's default.
struct Fields<'a>(&'a Value);

impl Fields<'_> {
    fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key).filter(|v| !v.is_null())
    }
    fn number(&self, key: &str) -> Result<Option<f64>> {
        self.get(key).map(|v| v.as_f64().ok_or(())).transpose().or_else(|_| failed(format!("{key} must be a number")))
    }
    fn flag(&self, key: &str, default: bool) -> Result<bool> {
        match self.get(key) {
            None => Ok(default),
            Some(v) => v.as_bool().map_or_else(|| failed(format!("{key} must be true or false")), Ok),
        }
    }
    fn text(&self, key: &str) -> Result<Option<&str>> {
        self.get(key).map(|v| v.as_str().ok_or(())).transpose().or_else(|_| failed(format!("{key} must be a string")))
    }
    fn point_value(v: &Value) -> Option<[f64; 2]> {
        let a = v.as_array()?;
        (a.len() == 2).then_some([a[0].as_f64()?, a[1].as_f64()?])
    }
    fn point(&self, key: &str) -> Result<[f64; 2]> {
        self.get(key).and_then(Self::point_value).map_or_else(|| failed(format!("{key} must be [x, y]")), Ok)
    }
    fn mode(&self) -> Result<Mode> {
        Ok(match self.text("mode")? {
            None | Some("New") => Mode::New,
            Some("Add") => Mode::Add,
            Some("Subtract") => Mode::Subtract,
            Some(other) => return failed(format!("mode “{other}” isn't New, Add or Subtract")),
        })
    }
}

/// Applies one selection op. `project` only changes when the op makes a layer active (the Wand's
/// `layer`), as the Mac saves the active layer with the project.
pub fn apply(gpu: &Gpu, project: &mut Project, selection: &mut Option<Selection>, op: &Value) -> Result<()> {
    let f = Fields(op);
    let name = f.text("op")?.unwrap_or("");
    let canvas: Region = vec![geom::rect(0.0, 0.0, project.manifest.width as f64, project.manifest.height as f64)];
    match name {
        "marquee" | "lasso" => {
            let antialias = f.flag("antialias", true)?;
            let mode = f.mode()?;
            let (points, ellipse) = if name == "marquee" {
                let ellipse = match f.text("shape")? {
                    None | Some("Rectangle") => false,
                    Some("Ellipse") => true,
                    Some(other) => return failed(format!("shape “{other}” isn't Rectangle or Ellipse")),
                };
                // `beginLasso` rounds the anchor; `DragBox.rect` rounds the drag's end.
                let anchor = f.point("from")?.map(f64::round);
                let to = f.point("to")?;
                let (mut dx, mut dy) = (to[0].round() - anchor[0], to[1].round() - anchor[1]);
                if f.flag("square", false)? {
                    let side = dx.abs().max(dy.abs());
                    dx = if dx < 0.0 { -side } else { side };
                    dy = if dy < 0.0 { -side } else { side };
                }
                let (x, y) = (anchor[0].min(anchor[0] + dx), anchor[1].min(anchor[1] + dy));
                let (w, h) = (dx.abs(), dy.abs());
                (vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h]], ellipse)
            } else {
                match f.text("kind")? {
                    None | Some("Freehand") | Some("Polygonal") => {}
                    Some(other) => return failed(format!("kind “{other}” isn't Freehand or Polygonal")),
                }
                let raw = f.get("points").and_then(|v| v.as_array()).map_or_else(|| failed("points must be a list".into()), Ok)?;
                let mut points: Vec<[f64; 2]> = Vec::new();
                for p in raw {
                    let p = Fields::point_value(p).map_or_else(|| failed("each point must be [x, y]".into()), Ok)?;
                    // `extendLasso` skips points closer than a quarter pixel to the last.
                    if let Some(last) = points.last()
                        && (p[0] - last[0]).hypot(p[1] - last[1]) < 0.25
                    {
                        continue;
                    }
                    points.push(p);
                }
                if points.is_empty() {
                    return failed("points is empty".into());
                }
                (points, false)
            };
            // `finishLasso`.
            let (xs, ys) = (points.iter().map(|p| p[0]), points.iter().map(|p| p[1]));
            let (x0, x1) = (xs.clone().fold(f64::INFINITY, f64::min), xs.fold(f64::NEG_INFINITY, f64::max));
            let (y0, y1) = (ys.clone().fold(f64::INFINITY, f64::min), ys.fold(f64::NEG_INFINITY, f64::max));
            if !((points.len() >= 3 || ellipse) && x1 - x0 > 0.0 && y1 - y0 > 0.0) {
                if mode == Mode::New {
                    *selection = None;
                }
                return Ok(());
            }
            let outline = if ellipse { geom::ellipse(x0, y0, x1 - x0, y1 - y0) } else { geom::polygon(&points) };
            apply_selection(selection, vec![outline], mode, antialias, &canvas)?;
        }
        "wand" => {
            let antialias = f.flag("antialias", true)?;
            let mode = f.mode()?;
            if let Some(layer) = f.text("layer")? {
                activate(project, layer)?;
            }
            let tolerance = f.number("tolerance")?.unwrap_or(32.0);
            let radius = match f.text("sampleSize")? {
                None | Some("Point Sample") => 0,
                Some("3 by 3 Average") => 1,
                Some("5 by 5 Average") => 2,
                Some(other) => return failed(format!("sampleSize “{other}” isn't a sample size")),
            };
            let contiguous = f.flag("contiguous", true)?;
            let all = f.flag("sampleAllLayers", false)?;
            let point = f.point("point")?;
            let (w, h) = (project.manifest.width as usize, project.manifest.height as usize);
            if !(point[0] >= 0.0 && point[1] >= 0.0 && point[0] < w as f64 && point[1] < h as f64) {
                return Ok(());
            }
            let sample = sample(gpu, project, all)?;
            let seed = (point[0].floor() as usize, point[1].floor() as usize);
            let mask = wand::wand_mask(&sample, w, h, seed, radius, tolerance.clamp(0.0, 255.0) as i32, contiguous);
            let Some(outline) = outline(&mask, w, h)? else {
                // Nothing matched: New clears the selection.
                if mode == Mode::New {
                    *selection = None;
                }
                return Ok(());
            };
            // A traced outline already lies on the canvas, so New skips the clip.
            if mode == Mode::New {
                *selection = Some(Selection { region: outline, antialiased: antialias, feather: 0.0 });
            } else {
                apply_selection(selection, outline, mode, antialias, &canvas)?;
            }
        }
        "objectSelection" => return unsupported("Object Selection (Vision's foreground instance mask)"),
        "colorRange" => {
            let antialias = f.flag("antialias", true)?;
            let fuzziness = f.number("fuzziness")?.unwrap_or(40.0).round() as i32;
            let invert = f.flag("invert", false)?;
            let (w, h) = (project.manifest.width as usize, project.manifest.height as usize);
            let image = sample(gpu, project, true)?;
            let (mut include, mut exclude): (Vec<[u8; 3]>, Vec<[u8; 3]>) = (Vec::new(), Vec::new());
            for s in f.get("samples").and_then(|v| v.as_array()).map_or_else(|| failed("samples must be a list".into()), Ok)? {
                let s = Fields(s);
                let Some(color) = picked_color(&image, w, h, s.point("point")?) else { continue };
                match s.text("mode")? {
                    None | Some("Sample") => {
                        include = vec![color];
                        exclude.clear();
                    }
                    Some("Add") => include.push(color),
                    Some("Remove") => exclude.push(color),
                    Some(other) => return failed(format!("sample mode “{other}” isn't Sample, Add or Remove")),
                }
            }
            // OK with no color picked leaves the selection as it was.
            if include.is_empty() {
                return Ok(());
            }
            let mask = wand::color_range_mask(&image, w, h, &include, &exclude, fuzziness, invert);
            *selection = outline(&mask, w, h)?.map(|region| Selection { region, antialiased: antialias, feather: 0.0 });
        }
        "selectAll" => *selection = Some(Selection { region: canvas, antialiased: true, feather: 0.0 }),
        "deselect" => *selection = None,
        "invertSelection" => {
            if let Some(current) = selection.as_ref() {
                let inverse = Selection { region: exact(geom::subtracting(&canvas, &current.region))?, ..current.clone() };
                // The inverse of everything is no selection at all.
                *selection = (!inverse.is_empty()).then_some(inverse);
            }
        }
        "modifySelection" => {
            let Some(current) = selection.as_ref().filter(|s| !s.is_empty()) else {
                return failed("Modify needs a selection that isn't empty".into());
            };
            let found: Vec<(&str, f64)> =
                ["expand", "contract", "feather"].into_iter().filter_map(|k| f.number(k).ok().flatten().map(|v| (k, v))).collect();
            let &[(kind, amount)] = found.as_slice() else {
                return failed("modifySelection needs exactly one of expand, contract, feather".into());
            };
            let mut next = current.clone();
            match kind {
                "feather" => next.feather = (current.feather * current.feather + amount * amount).sqrt().min(250.0),
                // A band `amount` wide either side of the outline, added (clipped to the canvas) or
                // taken away.
                "expand" => {
                    let band = exact(geom::stroke_band(&current.region, amount))?;
                    next.region = exact(geom::intersection(&exact(geom::union(&current.region, &band))?, &canvas))?;
                }
                _ => {
                    let band = exact(geom::stroke_band(&current.region, amount))?;
                    next.region = exact(geom::subtracting(&current.region, &band))?;
                }
            }
            *selection = Some(next);
        }
        "loadSelection" => {
            let mode = f.mode()?;
            let id = f.text("layer")?.map_or_else(|| failed("layer is missing".into()), Ok)?;
            let layer = project.manifest.layers.iter().find(|l| l.id == id).map_or_else(|| failed(format!("there's no layer {id}")), Ok)?;
            // Loading keeps the options bar's Anti-alias, which the corpus leaves on.
            if f.flag("mask", false)? {
                let Some(mask) = project.masks.get(id) else { return failed(format!("layer {id} has no mask")) };
                let (mw, mh) = mask.pixels.dimensions();
                let dark: Vec<u8> = mask.pixels.as_raw().iter().map(|&v| if v < 128 { 255 } else { 0 }).collect();
                let Some(traced) = outline(&dark, mw as usize, mh as usize)? else { return Ok(()) };
                let placement = layer.mask_placement.as_ref().unwrap_or(&layer.transform);
                apply_selection(selection, geom::transformed(&traced, pixel_to_document(placement, mw, mh)), mode, true, &canvas)?;
            } else {
                let Some(image) = project.images.get(id).filter(|_| !layer.is_group()) else { return Ok(()) };
                let (iw, ih) = image.pixels.dimensions();
                let opaque: Vec<u8> = image.pixels.pixels().map(|p| if p[3] >= 128 { 255 } else { 0 }).collect();
                let Some(traced) = outline(&opaque, iw as usize, ih as usize)? else { return Ok(()) };
                apply_selection(selection, geom::transformed(&traced, pixel_to_document(&layer.transform, iw, ih)), mode, true, &canvas)?;
            }
        }
        other => return failed(format!("`{other}` isn't a selection op")),
    }
    Ok(())
}

/// `EditorSession.applySelection`: the shape clipped to the canvas, then combined by `mode`.
/// Add and Subtract make a new selection, so a feather doesn't carry over.
fn apply_selection(selection: &mut Option<Selection>, shape: Region, mode: Mode, antialiased: bool, canvas: &Region) -> Result<()> {
    let clipped = exact(geom::intersection(&shape, canvas))?;
    let region = match (mode, selection.as_ref()) {
        (Mode::New, _) | (Mode::Add, None) => clipped,
        (Mode::Add, Some(current)) => exact(geom::union(&current.region, &clipped))?,
        // Subtracting from no selection selects nothing new, so nothing changes.
        (Mode::Subtract, None) => return Ok(()),
        (Mode::Subtract, Some(current)) => exact(geom::subtracting(&current.region, &clipped))?,
    };
    *selection = Some(Selection { region, antialiased, feather: 0.0 });
    Ok(())
}

/// `MagicWand.outline(of:)` (and `MaskTracing`, which traces the same pixels): the mask's
/// nonzero pixels as pixel-edge loops, or `None` when there are none.
fn outline(mask: &[u8], width: usize, height: usize) -> Result<Option<Region>> {
    let loops = wand::wand_trace(mask, width, height)
        .or_else(|_| failed("That selection is too detailed to outline. Try a different Tolerance, or turn on Contiguous.".into()))?;
    let region: Region = loops.into_iter().map(|l| geom::polygon(&l.into_iter().map(|(x, y)| [x as f64, y as f64]).collect::<Vec<_>>())).collect();
    Ok((!region.is_empty()).then_some(region))
}

/// Selecting a layer in the Layers panel.
fn activate(project: &mut Project, id: &str) -> Result<()> {
    if !project.manifest.layers.iter().any(|l| l.id == id) {
        return failed(format!("there's no layer {id}"));
    }
    project.manifest.active_layer_id = Some(id.to_string());
    Ok(())
}

/// `BrushRaster.pixelToDocument` as [a, b, c, d, tx, ty]: the layer's pixel grid onto the canvas.
fn pixel_to_document(t: &Transform, width: u32, height: u32) -> [f64; 6] {
    let radians = (t.rotation % 360.0).to_radians();
    let (cos, sin) = (radians.cos(), radians.sin());
    let sx = t.size[0] / width as f64 * if t.flip_x { -1.0 } else { 1.0 };
    let sy = t.size[1] / height as f64 * if t.flip_y { -1.0 } else { 1.0 };
    let (cx, cy) = (t.origin[0] + t.size[0] / 2.0, t.origin[1] + t.size[1] / 2.0);
    let (a, b, c, d) = (cos * sx, sin * sx, -sin * sy, cos * sy);
    let (hx, hy) = (-(width as f64) / 2.0, -(height as f64) / 2.0);
    [a, b, c, d, a * hx + c * hy + cx, b * hx + d * hy + cy]
}

/// `EditorSession.selectionSample`: premultiplied RGBA at document size, from every visible
/// layer as shown, or from the active layer's own pixels (a folder or blank layer reads as
/// transparent).
fn sample(gpu: &Gpu, project: &Project, all_layers: bool) -> Result<Vec<u8>> {
    let (w, h) = (project.manifest.width as u32, project.manifest.height as u32);
    if all_layers {
        let canvas = crate::composite::Compositor::new(gpu, project).render()?;
        return Ok(gpu.download(&canvas)?);
    }
    let mut out = vec![0u8; (w * h * 4) as usize];
    let active = project.manifest.active_layer_id.as_deref();
    let Some(layer) = project.manifest.layers.iter().find(|l| Some(l.id.as_str()) == active) else { return Ok(out) };
    let Some(image) = project.images.get(&layer.id).filter(|_| !layer.is_group()) else { return Ok(out) };
    let (iw, ih) = image.pixels.dimensions();
    if !upright(&layer.transform, iw, ih) {
        return unsupported("the Magic Wand reading a transformed layer");
    }
    let (ox, oy) = (layer.transform.origin[0] as i64, layer.transform.origin[1] as i64);
    for (x, y, p) in image.pixels.enumerate_pixels() {
        let (dx, dy) = (ox + x as i64, oy + y as i64);
        if dx < 0 || dy < 0 || dx >= w as i64 || dy >= h as i64 {
            continue;
        }
        let a = p[3] as u32;
        let i = ((dy as u32 * w + dx as u32) * 4) as usize;
        for c in 0..3 {
            out[i + c] = ((p[c] as u32 * a + 127) / 255) as u8;
        }
        out[i + 3] = p[3];
    }
    Ok(out)
}

fn upright(t: &Transform, width: u32, height: u32) -> bool {
    t.rotation == 0.0 && !t.flip_x && !t.flip_y && t.size == [width as f64, height as f64] && t.origin[0].fract() == 0.0 && t.origin[1].fract() == 0.0
}

/// Color Range's eyedropper: the straight color under `point`, averaged over the 3 × 3 pixels
/// around it (pixels past the image count as transparent), or `None` where all nine are.
fn picked_color(image: &[u8], w: usize, h: usize, point: [f64; 2]) -> Option<[u8; 3]> {
    let (x, y) = (point[0].floor(), point[1].floor());
    if !(x >= 0.0 && y >= 0.0 && x < w as f64 && y < h as f64) {
        return None;
    }
    let (x, y) = (x as i64, y as i64);
    let mut sums = [0u64; 4];
    for dy in -1..=1 {
        for dx in -1..=1 {
            let (px, py) = (x + dx, y + dy);
            if px < 0 || py < 0 || px >= w as i64 || py >= h as i64 {
                continue;
            }
            let i = (py as usize * w + px as usize) * 4;
            for c in 0..4 {
                sums[c] += image[i + c] as u64;
            }
        }
    }
    (sums[3] > 0).then(|| [0, 1, 2].map(|c| ((sums[c] * 255 + sums[3] / 2) / sums[3]).min(255) as u8))
}

/// `DocumentSelection.coverage(width:height:)`: one byte per document pixel, 255 where selected.
pub fn coverage(gpu: &Gpu, selection: &Selection, width: u32, height: u32) -> Result<Vec<u8>> {
    raster::coverage(gpu, selection, width, height)
}

