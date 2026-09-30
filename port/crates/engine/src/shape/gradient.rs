//! The Gradient tool: `beginGradient`, `moveGradient`, `endGradientDrag` and `commitGradient`.
//!
//! The Mac draws the gradient into a `BrushStroke`, 256-pixel tiles over the layer grown to cover
//! the canvas (`makeRasterEdit(growsMask: true)`): each tile the canvas touches starts from the
//! layer's own pixels and gets the `CGGradient` drawn over it, clipped to the canvas, at the
//! tool's opacity. `commitRasterEdit` then assembles the layer's pixels with the tiles over them
//! and trims the result to its non-transparent pixels (`BrushCommit.render`).

use super::{Result, color, failed, point, target, unpremultiply};
use crate::RenderError;
use crate::gpu::Gpu;
use comp_format::{Asset, Project};
use serde_json::Value;

/// `BrushStroke.tileSize`.
const TILE: i64 = 256;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Shape {
    Linear,
    Radial,
}

/// What the gradient is filled with (`GradientEdit.Fill`), in document pixels.
pub(super) struct Fill {
    pub shape: Shape,
    pub start: [f64; 2],
    pub end: [f64; 2],
    /// Straight RGBA at the start and the end, 0-1.
    pub colors: [[f64; 4]; 2],
    pub opacity: f64,
}

pub fn apply(gpu: &Gpu, project: &mut Project, op: &Value) -> Result<()> {
    let (start, end) = (point(op, "from")?, point(op, "to")?);
    if (end[0] - start[0]).hypot(end[1] - start[1]) < 0.5 {
        return failed("the drag was too short to leave a gradient".into());
    }
    let on_mask = op.get("mask").and_then(|v| v.as_bool()).unwrap_or(false);
    let shape = match op.get("type").and_then(|v| v.as_str()).unwrap_or("Linear") {
        "Linear" => Shape::Linear,
        "Radial" => Shape::Radial,
        other => return failed(format!("unknown gradient type {other}")),
    };
    let both = match op.get("style").and_then(|v| v.as_str()).unwrap_or("Foreground to Transparent") {
        "Foreground to Background" => true,
        "Foreground to Transparent" => false,
        other => return failed(format!("unknown gradient style {other}")),
    };
    let reversed = op.get("reversed").and_then(|v| v.as_bool()).unwrap_or(false);
    let opacity = op.get("opacity").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let mut fg = color(op, "foreground")?.unwrap_or([0.0; 3]);
    let mut bg = color(op, "background")?.unwrap_or([1.0; 3]);
    if on_mask {
        // On a mask the swatches are black and white: `setPaletteColor` makes white the
        // foreground only for white, and the background's choice decides it when both are set.
        let mut white = false;
        if let Some(c) = color(op, "foreground")? {
            white = c == [1.0; 3];
        }
        if let Some(c) = color(op, "background")? {
            white = c != [1.0; 3];
        }
        (fg, bg) = if white { ([1.0; 3], [0.0; 3]) } else { ([0.0; 3], [1.0; 3]) };
    }
    // `gradientColors`: the foreground, then the background or the foreground made transparent.
    let mut colors = [[fg[0], fg[1], fg[2], 1.0], if both { [bg[0], bg[1], bg[2], 1.0] } else { [fg[0], fg[1], fg[2], 0.0] }];
    if reversed {
        colors.reverse();
    }
    let fill = Fill { shape, start, end, colors, opacity: opacity.clamp(0.0, 1.0) };

    let Some(index) = target(project, op)? else { return failed("there's no active layer".into()) };
    let layer = &project.manifest.layers[index];
    project.manifest.active_layer_id = Some(layer.id.clone());
    if layer.is_group() || layer.adjustment.is_some() {
        return failed(format!("“{}” has no pixels to paint", layer.name));
    }
    if !crate::order::visible_layers(&project.manifest.layers).iter().any(|l| l.id == layer.id) {
        return failed(format!("“{}” is hidden", layer.name));
    }
    if on_mask && !layer.mask_enabled() {
        return failed(format!("“{}” has no mask turned on to paint", layer.name));
    }
    if layer.mask_placement.is_some() {
        return Err(RenderError::Unsupported("a gradient on a layer whose mask is placed on its own".into()));
    }
    let t = layer.transform;
    let source = project.images.get(&layer.id).map(|a| &a.pixels);
    let (w, h) = match source {
        Some(img) => (img.width() as i64, img.height() as i64),
        None => (t.size[0].round() as i64, t.size[1].round() as i64),
    };
    // Only a layer drawn 1:1 and upright on whole pixels maps its grid onto the canvas exactly.
    if t.rotation != 0.0 || t.flip_x || t.flip_y || t.size != [w as f64, h as f64] || t.origin[0].fract() != 0.0 || t.origin[1].fract() != 0.0 {
        return Err(RenderError::Unsupported("a gradient on a transformed layer".into()));
    }
    let (ox, oy) = (t.origin[0] as i64, t.origin[1] as i64);
    let (cw, ch) = (project.manifest.width, project.manifest.height);
    // The layer's grid grown to cover the canvas.
    let (ex0, ey0) = ((-ox).min(0), (-oy).min(0));
    let (ex1, ey1) = ((cw - ox).max(w), (ch - oy).max(h));
    let (width, height) = (ex1 - ex0, ey1 - ey0);
    let source_rect = [-ex0, -ey0, w, h];
    // The canvas in that grid: the tiles it touches are the ones the gradient allocates.
    let canvas = [-ox - ex0, -oy - ey0, -ox - ex0 + cw, -oy - ey0 + ch];
    let (tx0, ty0, tx1, ty1) = (canvas[0] / TILE, canvas[1] / TILE, (canvas[2] - 1) / TILE, (canvas[3] - 1) / TILE);
    let tile = |x: i64, y: i64| [x * TILE, y * TILE, ((x + 1) * TILE).min(width), ((y + 1) * TILE).min(height)];
    let (mut b0, mut b1) = ([tile(tx0, ty0)[0], tile(tx0, ty0)[1]], [tile(tx1, ty1)[2], tile(tx1, ty1)[3]]);
    if source.is_some() {
        b0 = [b0[0].min(source_rect[0]), b0[1].min(source_rect[1])];
        b1 = [b1[0].max(source_rect[0] + w), b1[1].max(source_rect[1] + h)];
    }
    let committed = [b0[0], b0[1], b1[0] - b0[0], b1[1] - b0[1]];
    // A mask grown past its layer starts as its background, read off its thumbnail; only masks
    // that already cover the canvas are drawn so far.
    if on_mask {
        let grows = committed != source_rect || source.is_none();
        let mask = project.masks.get(&layer.id).map(|m| &m.pixels);
        let Some(mask) = mask.filter(|m| !grows && m.dimensions() == (w as u32, h as u32)) else {
            return Err(RenderError::Unsupported("a gradient on a mask that grows or is sized apart from its layer".into()));
        };
        let base: Vec<u8> = mask.pixels().flat_map(|p| [p.0[0], p.0[0], p.0[0], 255]).collect();
        let region = [canvas[0] - committed[0], canvas[1] - committed[1], cw, ch];
        let offset = [ox + ex0 + committed[0], oy + ey0 + committed[1]];
        let drawn = super::raster::gradient_over(gpu, &base, w as u32, h as u32, region, offset, &fill)?;
        let gray = image::GrayImage::from_raw(w as u32, h as u32, drawn.chunks_exact(4).map(|p| p[0]).collect()).expect("mask size");
        let id = layer.id.clone();
        project.masks.insert(id, Asset::new(gray));
        return Ok(());
    }
    // The committed grid, premultiplied: the layer's pixels, then the gradient over them inside
    // the canvas.
    let mut base = vec![0u8; (committed[2] * committed[3] * 4) as usize];
    if let Some(img) = source {
        let (dx, dy) = (source_rect[0] - committed[0], source_rect[1] - committed[1]);
        for y in 0..h {
            for x in 0..w {
                let p = img.get_pixel(x as u32, y as u32).0;
                let a = p[3] as u32;
                let i = (((y + dy) * committed[2] + x + dx) * 4) as usize;
                for c in 0..3 {
                    base[i + c] = ((p[c] as u32 * a + 127) / 255) as u8;
                }
                base[i + 3] = p[3];
            }
        }
    }
    let region = [canvas[0] - committed[0], canvas[1] - committed[1], cw, ch];
    // Document coordinates of the committed grid's top-left corner.
    let offset = [ox + ex0 + committed[0], oy + ey0 + committed[1]];
    let mut pixels = super::raster::gradient_over(gpu, &base, committed[2] as u32, committed[3] as u32, region, offset, &fill)?;
    // `brush_alpha_bounds`: trimmed to the pixels that aren't transparent.
    let (cwid, chei) = (committed[2] as usize, committed[3] as usize);
    let (mut left, mut right, mut top, mut bottom) = (cwid, 0, chei, 0);
    for y in 0..chei {
        let row = &pixels[y * cwid * 4..(y + 1) * cwid * 4];
        let Some(first) = (0..cwid).find(|&x| row[x * 4 + 3] != 0) else { continue };
        let last = (first..cwid).rev().find(|&x| row[x * 4 + 3] != 0).unwrap() + 1;
        left = left.min(first);
        right = right.max(last);
        top = top.min(y);
        bottom = y + 1;
    }
    let crop = if right == 0 { [0, 0, cwid, chei] } else { [left, top, right - left, bottom - top] };
    if crop != [0, 0, cwid, chei] {
        let mut cropped = Vec::with_capacity(crop[2] * crop[3] * 4);
        for y in crop[1]..crop[1] + crop[3] {
            cropped.extend_from_slice(&pixels[(y * cwid + crop[0]) * 4..(y * cwid + crop[0] + crop[2]) * 4]);
        }
        pixels = cropped;
    }
    // `commitRasterEdit` carries a mask over only as far as the layer's grid stayed put.
    let moved = [crop[0] as i64 + committed[0], crop[1] as i64 + committed[1], crop[2] as i64, crop[3] as i64] != source_rect;
    if layer.mask_file.is_some() && (moved || source.is_none()) {
        return Err(RenderError::Unsupported("a gradient that grows a layer with a mask".into()));
    }
    unpremultiply(&mut pixels);
    let image = image::RgbaImage::from_raw(crop[2] as u32, crop[3] as u32, pixels).expect("cropped size");
    let layer = &mut project.manifest.layers[index];
    layer.transform.origin = [(offset[0] + crop[0] as i64) as f64, (offset[1] + crop[1] as i64) as f64];
    layer.transform.size = [crop[2] as f64, crop[3] as f64];
    layer.image_file = Some(format!("{}.png", layer.id));
    // The layer is rebuilt from its pixels: no longer a live shape or text.
    layer.shape = None;
    layer.text = None;
    let id = layer.id.clone();
    project.images.insert(id, Asset::new(image));
    Ok(())
}
