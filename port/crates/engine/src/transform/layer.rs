//! `LayerRenderer.draw` for one layer that isn't a straight pixel copy: the interpolation it picks,
//! the image and mask sources (halved first for large reductions), and the rectangles they fill.

use super::{Draw, Placement, Quality, Source, halve, interpolation, reduction_level};
use crate::RenderError;
use crate::gpu::{Gpu, GpuImage};
use comp_format::{LayerRecord, Project, Sampling, Transform};
use image::{GrayImage, RgbaImage};

type Result<T> = std::result::Result<T, RenderError>;

fn unsupported<T>(what: &str) -> Result<T> {
    Err(RenderError::Unsupported(what.to_string()))
}

/// Whether `layer` needs a resampling draw: it isn't placed 1:1 and upright on whole pixels, or
/// its mask doesn't line up with its pixels.
pub fn needs_resampling(project: &Project, layer: &LayerRecord) -> bool {
    let Some(asset) = project.images.get(&layer.id) else {
        return false;
    };
    let t = &layer.transform;
    let (w, h) = asset.pixels.dimensions();
    let upright = t.rotation % 360.0 == 0.0
        && !t.flip_x
        && !t.flip_y
        && t.size == [w as f64, h as f64]
        && t.origin[0].fract() == 0.0
        && t.origin[1].fract() == 0.0;
    if !upright {
        return true;
    }
    if !layer.mask_enabled() {
        return false;
    }
    let Some(mask) = project.masks.get(&layer.id) else {
        return false;
    };
    let uniform = mask.pixels.width() == 1 && mask.pixels.height() == 1;
    placement_of(layer).is_some() || (!uniform && mask.pixels.dimensions() != (w, h))
}

/// Where the layer's mask sits apart from the layer, if it does: `LayerMask.clipImage` places any
/// mask with a placement of its own (linked or not) unless it's the layer's own placement.
fn placement_of(layer: &LayerRecord) -> Option<&comp_format::Transform> {
    layer.mask_placement.as_ref().filter(|p| {
        let mut same = **p;
        same.sampling = layer.transform.sampling;
        same != layer.transform
    })
}

/// Straight PNG pixels premultiplied as Core Graphics loads them, rounding to nearest.
fn premultiplied(image: &RgbaImage) -> RgbaImage {
    let mut out = image.clone();
    for p in out.pixels_mut() {
        let a = p[3] as u32;
        for c in 0..3 {
            p[c] = ((p[c] as u32 * a + 127) / 255) as u8;
        }
    }
    out
}

/// `LayerRenderer.reduced`: how many halvings to draw from, for a source `pixels` wide landing
/// `width` canvas pixels wide; none for Nearest, and none for a single pixel.
fn halvings(sampling: Sampling, width: f64, (w, h): (u32, u32)) -> u32 {
    if sampling == Sampling::Nearest || (w <= 1 && h <= 1) {
        return 0;
    }
    reduction_level(width / w.max(1) as f64)
}

/// The rectangle a source reduced `level` times fills: the layer's bounds, reaching further right
/// and down by what the halvings rounded up (`LayerRenderer.coverage(of:in:)`).
fn rect(placement: &Placement, level: u32, (w, h): (u32, u32), (rw, rh): (u32, u32)) -> super::Rect {
    placement.rect(((rw as u64) << level) as f64 / w.max(1) as f64, ((rh as u64) << level) as f64 / h.max(1) as f64)
}

/// A layer's image as `LayerRenderer.draw` hands it to Core Graphics: premultiplied, halved for
/// large reductions, with the interpolation it picks and the rectangle it fills.
struct Prepared {
    pixels: RgbaImage,
    quality: Quality,
    rect: super::Rect,
}

/// `device` is the context's scale along each axis (device pixels per unit).
fn prepare(straight: &RgbaImage, t: &Transform, placement: &Placement, device: [f64; 2]) -> Result<Prepared> {
    let size = straight.dimensions();
    let width = t.size[0] * device[0];
    let mut pixels = premultiplied(straight);
    // `DownsampleCache` stops halving at a single pixel.
    let mut level = 0;
    while level < halvings(t.sampling, width, size) && (pixels.width() > 1 || pixels.height() > 1) {
        pixels = halve::halve_color(&pixels);
        level += 1;
    }
    let reduced = pixels.dimensions();
    let final_factor = width / size.0.max(1) as f64 * (1u64 << level) as f64;
    let quality = interpolation(t.sampling, final_factor, placement.upright());
    if quality == Quality::High && t.size[1] * device[1] < reduced.1 as f64 * (1u64 << level) as f64 {
        return unsupported("High-quality interpolation while shrinking");
    }
    Ok(Prepared { rect: rect(placement, level, size, reduced), pixels, quality })
}

/// `LayerRenderer.draw` of `straight` pixels placed by `t` into a context whose own scale is
/// `scale` and offset `offset` (canvas = document × scale + offset), over `canvas`, at full
/// opacity with no mask.
pub fn draw_image(gpu: &Gpu, straight: &RgbaImage, t: &Transform, scale: [f64; 2], offset: [f64; 2], canvas: &GpuImage) -> Result<GpuImage> {
    let placement = Placement::in_context(t, scale, offset);
    let prepared = prepare(straight, t, &placement, scale)?;
    let (w, h) = prepared.pixels.dimensions();
    let image = gpu.upload(w, h, prepared.pixels.as_raw());
    let draw = Draw {
        image: Source { buffer: &image.buffer, width: w, height: h, rect: prepared.rect },
        premultiplied: true,
        placement,
        quality: prepared.quality,
        antialias: t.sampling != Sampling::Nearest,
        mode: comp_format::BlendMode::Normal,
        opacity: 1.0,
        mask: None,
        clip: None,
    };
    Ok(super::draw(gpu, canvas, &draw))
}

/// Draws `layer` over `canvas` through its transform, with `opacity` and `clip`.
pub fn draw_layer(gpu: &Gpu, project: &Project, layer: &LayerRecord, canvas: &GpuImage, opacity: f64, clip: Option<&wgpu::Buffer>) -> Result<GpuImage> {
    let Some(asset) = project.images.get(&layer.id) else {
        return Ok(gpu.copy(canvas));
    };
    let t = &layer.transform;
    let size = asset.pixels.dimensions();
    let placement = Placement::of(t);
    let width = t.size[0];
    let Prepared { pixels, quality, rect: image_rect } = prepare(&asset.pixels, t, &placement, [1.0, 1.0])?;
    let reduced = pixels.dimensions();
    let image = gpu.upload(reduced.0, reduced.1, pixels.as_raw());
    let mask_pixels;
    let mask = if layer.mask_enabled() {
        let Some(m) = project.masks.get(&layer.id) else {
            return unsupported("a missing mask");
        };
        let mut mask: GrayImage = match placement_of(layer) {
            Some(p) if m.pixels.width() > 1 || m.pixels.height() > 1 => super::clip::placed_mask(&m.pixels, p, t, size).map_err(RenderError::Unsupported)?,
            _ => m.pixels.clone(),
        };
        let mask_size = mask.dimensions();
        let mut mask_level = 0;
        while mask_level < halvings(t.sampling, width, mask_size) && (mask.width() > 1 || mask.height() > 1) {
            mask = halve::halve_mask(&mask);
            mask_level += 1;
        }
        let coverage: Vec<u32> = mask.as_raw().iter().map(|&v| v as u32).collect();
        mask_pixels = gpu.bytes(bytemuck::cast_slice(&coverage));
        Some(Source { buffer: &mask_pixels, width: mask.width(), height: mask.height(), rect: rect(&placement, mask_level, mask_size, mask.dimensions()) })
    } else {
        None
    };
    let draw = Draw {
        image: Source { buffer: &image.buffer, width: reduced.0, height: reduced.1, rect: image_rect },
        premultiplied: true,
        placement,
        quality,
        antialias: t.sampling != Sampling::Nearest,
        mode: layer.blend_mode(),
        opacity,
        mask,
        clip,
    };
    Ok(super::draw(gpu, canvas, &draw))
}
