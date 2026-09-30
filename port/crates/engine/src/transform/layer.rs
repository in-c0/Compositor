//! `LayerRenderer.draw` for one layer that isn't a straight pixel copy: the interpolation it picks,
//! the image and mask sources, and the rectangles they fill.

use super::{Draw, Placement, Quality, Source, interpolation, reduction_level};
use crate::RenderError;
use crate::gpu::{Gpu, GpuImage};
use comp_format::{LayerRecord, Project, Sampling};

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
    let unlinked = layer.mask_placement.is_some() && !layer.mask_linked();
    unlinked || (!uniform && mask.pixels.dimensions() != (w, h))
}

/// Draws `layer` over `canvas` through its transform, with `opacity` and `clip`.
pub fn draw_layer(gpu: &Gpu, project: &Project, layer: &LayerRecord, canvas: &GpuImage, opacity: f64, clip: Option<&wgpu::Buffer>) -> Result<GpuImage> {
    let Some(asset) = project.images.get(&layer.id) else {
        return Ok(gpu.copy(canvas));
    };
    let t = &layer.transform;
    let (w, h) = asset.pixels.dimensions();
    let placement = Placement::of(t);
    let width = t.size[0];
    if t.sampling != Sampling::Nearest && reduction_level(width / w.max(1) as f64) > 0 {
        return unsupported("large reductions (vImage halvings)");
    }
    let final_factor = width / w.max(1) as f64;
    let quality = interpolation(t.sampling, final_factor, placement.upright());
    if quality == Quality::High && t.size[1] < h as f64 {
        return unsupported("High-quality interpolation while shrinking");
    }
    let rect = placement.rect(1.0, 1.0);
    let pixels = gpu.upload(w, h, asset.pixels.as_raw());
    let mask_pixels;
    let mask = if layer.mask_enabled() {
        if layer.mask_placement.is_some() && !layer.mask_linked() {
            return unsupported("unlinked masks");
        }
        let Some(m) = project.masks.get(&layer.id) else {
            return unsupported("a missing mask");
        };
        let (mw, mh) = m.pixels.dimensions();
        if t.sampling != Sampling::Nearest && reduction_level(width / mw.max(1) as f64) > 0 {
            return unsupported("large mask reductions (vImage halvings)");
        }
        let coverage: Vec<u32> = m.pixels.as_raw().iter().map(|&v| v as u32).collect();
        mask_pixels = gpu.bytes(bytemuck::cast_slice(&coverage));
        Some(Source { buffer: &mask_pixels, width: mw, height: mh, rect })
    } else {
        None
    };
    let draw = Draw {
        image: Source { buffer: &pixels.buffer, width: w, height: h, rect },
        premultiplied: false,
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
