//! Transformed layers: `LayerRenderer.draw` for a layer that is scaled, rotated, flipped or placed
//! off the pixel grid, and the masks that follow it.
//!
//! The Mac translates to the layer's center, rotates, flips, and has Core Graphics draw the image
//! into the layer's bounds. What Core Graphics does there was measured from references (see the
//! probe cases in `parity/corpus/transform/probe-*`):
//!
//! - Every device pixel samples the image at its center, mapped back into image pixels.
//! - Interpolation `.none` takes the pixel under that point, clamped to the image.
//! - `.low` (and `.high` while enlarging) is a 2x2 filter whose phase is rounded to eighths and
//!   whose weights are sharper than bilinear (see `resample.wgsl`). The image is clamped at its
//!   edges.
//! - With antialiasing on, the edges of the image's rectangle fade over one pixel's footprint
//!   measured across the edge (its L1 width, |cos| + |sin|), each edge on its own, multiplied at
//!   corners. With antialiasing off (Nearest), a pixel is drawn when the rectangle overlaps it at
//!   all.

mod layer;

pub use layer::{draw_layer, needs_resampling};

use crate::blend::{mode_index, opacity_table};
use crate::gpu::{Gpu, GpuImage};
use comp_format::{BlendMode, Sampling, Transform};

/// Core Graphics' interpolation quality.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    None,
    Low,
    High,
}

impl Quality {
    /// `LayerSampling.quality`.
    pub fn of(sampling: Sampling) -> Self {
        match sampling {
            Sampling::Nearest => Quality::None,
            Sampling::Smooth => Quality::Low,
            Sampling::High => Quality::High,
        }
    }
}

/// `LayerRenderer.interpolation(_:finalFactor:upright:)`: `final_factor` device pixels per
/// (reduced) image pixel, measured along the layer's width.
pub fn interpolation(sampling: Sampling, final_factor: f64, upright: bool) -> Quality {
    if sampling == Sampling::Nearest || (upright && (final_factor - 1.0).abs() < 0.001) {
        return Quality::None;
    }
    if final_factor <= 1.0 { Quality::Low } else { Quality::of(sampling) }
}

/// `DownsampleCache.level(for:)`: the halvings to draw from when an image lands `factor` output
/// pixels per image pixel.
pub fn reduction_level(factor: f64) -> u32 {
    if !factor.is_finite() || factor <= 0.0 || factor >= 0.5 {
        return 0;
    }
    ((1.0 / factor).log2().floor() as u32).min(6)
}

/// A layer's placement: its center, rotation and flips, as `LayerRenderer.draw` sets up the
/// context, mapping canvas points (y down) to the drawing space the image rectangle is given in
/// (y up, centered on the layer).
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    center: [f64; 2],
    cos: f64,
    sin: f64,
    flip_x: bool,
    flip_y: bool,
    /// The layer's bounds in drawing space, `CGRect(x: -w/2, y: -h/2, width: w, height: h)`.
    pub width: f64,
    pub height: f64,
}

impl Placement {
    pub fn of(t: &Transform) -> Self {
        // `LayerTransform.radians`: the rotation's remainder after whole turns, in radians.
        let radians = (t.rotation % 360.0).to_radians();
        Self {
            center: [t.origin[0] + t.size[0] / 2.0, t.origin[1] + t.size[1] / 2.0],
            cos: radians.cos(),
            sin: radians.sin(),
            flip_x: t.flip_x,
            flip_y: t.flip_y,
            width: t.size[0],
            height: t.size[1],
        }
    }

    /// Canvas to drawing space, as a 2x3 affine `[a, b, c, d, e, f]`: `qx = a·x + b·y + c`,
    /// `qy = d·x + e·y + f`. The inverse of translate(center) · rotate · scale(±1, ±1).
    pub fn inverse(&self) -> [f64; 6] {
        let sx = if self.flip_x { -1.0 } else { 1.0 };
        let sy = if self.flip_y { 1.0 } else { -1.0 };
        let (c, s) = (self.cos, self.sin);
        let [cx, cy] = self.center;
        // Rotate by −θ, then undo the scale (its own inverse).
        let (a, b, d, e) = (c * sx, s * sx, -s * sy, c * sy);
        [a, b, -(a * cx + b * cy), d, e, -(d * cx + e * cy)]
    }

    /// The drawing-space corners of `rect` on the canvas.
    fn corners(&self, rect: &Rect) -> [[f64; 2]; 4] {
        let sx = if self.flip_x { -1.0 } else { 1.0 };
        let sy = if self.flip_y { 1.0 } else { -1.0 };
        let map = |x: f64, y: f64| {
            let (x, y) = (x * sx, y * sy);
            [self.center[0] + self.cos * x - self.sin * y, self.center[1] + self.sin * x + self.cos * y]
        };
        [map(rect.min_x, rect.min_y), map(rect.max_x, rect.min_y), map(rect.max_x, rect.max_y), map(rect.min_x, rect.max_y)]
    }

    /// |cos| + |sin|: a canvas pixel's width measured across any edge of the layer.
    pub fn l1(&self) -> f64 {
        self.cos.abs() + self.sin.abs()
    }

    /// Whether the layer is drawn without rotation (flips keep it upright).
    pub fn upright(&self) -> bool {
        self.sin == 0.0 && self.cos == 1.0
    }

    /// The layer's bounds in drawing space, extended right and down by `scale` (a reduced image's
    /// reach, `LayerRenderer.coverage(of:in:)`).
    pub fn rect(&self, width_scale: f64, height_scale: f64) -> Rect {
        let (w, h) = (self.width * width_scale, self.height * height_scale);
        let max_y = self.height / 2.0;
        Rect { min_x: -self.width / 2.0, min_y: max_y - h, max_x: -self.width / 2.0 + w, max_y }
    }
}

/// A rectangle in drawing space (y up).
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

/// One source drawn through a placement: an image or a mask, its pixel size, and the rectangle it
/// fills.
pub struct Source<'a> {
    pub buffer: &'a wgpu::Buffer,
    pub width: u32,
    pub height: u32,
    pub rect: Rect,
}

/// One transformed layer draw.
pub struct Draw<'a> {
    /// Straight RGBA8 pixels (`premultiplied` false) or premultiplied ones.
    pub image: Source<'a>,
    pub premultiplied: bool,
    pub placement: Placement,
    pub quality: Quality,
    pub antialias: bool,
    pub mode: BlendMode,
    pub opacity: f64,
    /// The layer's own mask (one u32 coverage per mask pixel), clipped through its own rectangle.
    pub mask: Option<Source<'a>>,
    /// A clip on the context: coverage per canvas pixel.
    pub clip: Option<&'a wgpu::Buffer>,
}

const MODES: &str = include_str!("../blend/modes.wgsl");

/// Composites `draw` over `canvas` and returns the new canvas.
pub fn draw(gpu: &Gpu, canvas: &GpuImage, draw: &Draw) -> GpuImage {
    let pipeline = gpu.pipeline("transform_draw", &format!("{MODES}\n{}", include_str!("resample.wgsl")));
    let out = gpu.image(canvas.width, canvas.height);
    let p = &draw.placement;
    let inverse = p.inverse();
    // The canvas-space bounding box of the image rectangle, for the aliased overlap test.
    let corners = p.corners(&draw.image.rect);
    let min = |i: usize| corners.iter().map(|c| c[i]).fold(f64::INFINITY, f64::min);
    let max = |i: usize| corners.iter().map(|c| c[i]).fold(f64::NEG_INFINITY, f64::max);
    let mut params: Vec<u8> = Vec::with_capacity(160);
    let mut u = |v: u32| params.extend_from_slice(&v.to_le_bytes());
    for v in [canvas.width, canvas.height, draw.image.width, draw.image.height] {
        u(v);
    }
    let (mask_width, mask_height) = draw.mask.as_ref().map_or((1, 1), |m| (m.width, m.height));
    for v in [
        mask_width,
        mask_height,
        mode_index(draw.mode),
        draw.mask.is_some() as u32,
        draw.clip.is_some() as u32,
        draw.premultiplied as u32,
        (draw.opacity >= 1.0) as u32,
        match draw.quality {
            Quality::None => 0,
            Quality::Low | Quality::High => 1,
        },
        draw.antialias as u32,
    ] {
        u(v);
    }
    // Pad to a 16-byte boundary before the float block.
    while params.len() % 16 != 0 {
        params.extend_from_slice(&0u32.to_le_bytes());
    }
    let mask_rect = draw.mask.as_ref().map_or(draw.image.rect, |m| m.rect);
    let r = &draw.image.rect;
    let floats: Vec<f32> = [
        inverse[0], inverse[1], inverse[2], p.l1(), inverse[3], inverse[4], inverse[5], 0.0,
        r.min_x, r.min_y, r.max_x, r.max_y,
        mask_rect.min_x, mask_rect.min_y, mask_rect.max_x, mask_rect.max_y,
        min(0), min(1), max(0), max(1),
    ]
    .iter()
    .map(|&v| v as f32)
    .collect();
    params.extend_from_slice(bytemuck::cast_slice(&floats));
    let table = opacity_table(gpu, draw.opacity);
    let placeholder = gpu.bytes(&[0u8; 4]);
    let mask = draw.mask.as_ref().map_or(&placeholder, |m| m.buffer);
    let clip = draw.clip.unwrap_or(&placeholder);
    gpu.dispatch(&pipeline, &params, &[&canvas.buffer, draw.image.buffer, &out.buffer, &table, mask, clip], canvas.width, canvas.height);
    out
}
