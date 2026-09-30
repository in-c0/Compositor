//! Transformed layers: `LayerRenderer.draw` for a layer that is scaled, rotated, flipped or placed
//! off the pixel grid, and the masks that follow it.
//!
//! The Mac translates to the layer's center, rotates, flips, and has Core Graphics draw the image
//! into the layer's bounds, clipped to its mask. What Core Graphics does there was measured from
//! the references (the `transform/probe-*` cases isolate each part), and `resample.wgsl` spells it
//! out: sample positions stepped in 32.32 fixed point, interpolation `.none` as the pixel under the
//! position, `.low` as a two-pixel shift blend per axis, edges faded across the pixel's L1 width.
//! `.high` is `.low` while enlarging and a box average while shrinking (`clip.rs`); large
//! reductions start from vImage's Lanczos halvings (`halve.rs`).

pub mod clip;
mod halve;
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

/// Where a source's pixels land: the source position (in its pixels, y down) at the center of
/// canvas pixel (0, 0) and its steps per canvas pixel, as Core Graphics steps them.
#[derive(Clone, Copy, Debug)]
pub struct Dda {
    pub u0: f64,
    pub u_dx: f64,
    pub u_dy: f64,
    pub v0: f64,
    pub v_dx: f64,
    pub v_dy: f64,
}

impl Placement {
    /// The source position under canvas points for a `width` x `height` source filling `rect`.
    pub fn dda(&self, rect: &Rect, width: u32, height: u32) -> Dda {
        let sx = if self.flip_x { -1.0 } else { 1.0 };
        let sy = if self.flip_y { 1.0 } else { -1.0 };
        let (c, s) = (self.cos, self.sin);
        let [cx, cy] = self.center;
        let (w, h) = (width as f64, height as f64);
        let (rw, rh) = (rect.max_x - rect.min_x, rect.max_y - rect.min_y);
        // Drawing space at the canvas point (x, y), then the source: u from the left edge, v down
        // from the top edge.
        let at = |x: f64, y: f64| {
            let (px, py) = (x - cx, y - cy);
            let qx = (c * px + s * py) * sx;
            let qy = (-s * px + c * py) * sy;
            ((qx - rect.min_x) * w / rw, (rect.max_y - qy) * h / rh)
        };
        let (u0, v0) = at(0.5, 0.5);
        Dda { u0, u_dx: c * sx * w / rw, u_dy: s * sx * w / rw, v0, v_dx: s * sy * h / rh, v_dy: -c * sy * h / rh }
    }
}

/// `v` in 32.32 fixed point, rounded down, as the low and high words of a two's complement pair.
fn fixed(v: f64) -> [u32; 2] {
    let f = (v * 4294967296.0).floor() as i64;
    [f as u32, (f >> 32) as u32]
}

/// The kernel's `Dda` block: fixed-point positions and steps, canvas pixels per source pixel, and
/// the source size.
fn dda_block(d: &Dda, rect: &Rect, width: u32, height: u32) -> Vec<u8> {
    let mut b = Vec::with_capacity(64);
    for v in [d.u0, d.u_dx, d.u_dy, d.v0, d.v_dx, d.v_dy] {
        for w in fixed(v) {
            b.extend_from_slice(&w.to_le_bytes());
        }
    }
    let scale = [((rect.max_x - rect.min_x) / width as f64) as f32, ((rect.max_y - rect.min_y) / height as f64) as f32];
    b.extend_from_slice(bytemuck::cast_slice(&scale));
    b.extend_from_slice(&width.to_le_bytes());
    b.extend_from_slice(&height.to_le_bytes());
    b
}

impl Placement {
    /// For a layer that isn't rotated, each canvas column's fades at the rectangle's left and
    /// right edges and each row's at its top and bottom, in source terms, with the coverage byte
    /// of their product: in double precision, as Core Graphics measures them, so an edge exactly on
    /// a pixel boundary or center stays exact.
    pub(crate) fn edge_table(&self, rect: &Rect, width: u32, height: u32) -> Vec<[u32; 4]> {
        let sx = if self.flip_x { -1.0 } else { 1.0 };
        let sy = if self.flip_y { 1.0 } else { -1.0 };
        let [cx, cy] = self.center;
        let entry = |a: f64, b: f64| {
            let (a, b) = (a.clamp(0.0, 1.0), b.clamp(0.0, 1.0));
            let byte = ((a * b * 256.0).ceil() - 1.0).clamp(0.0, 255.0) as u32;
            [(a as f32).to_bits(), (b as f32).to_bits(), byte, 0]
        };
        let mut table = Vec::with_capacity((width + height) as usize);
        for x in 0..width {
            let qx = (x as f64 + 0.5 - cx) * sx;
            table.push(entry(0.5 + (qx - rect.min_x), 0.5 + (rect.max_x - qx)));
        }
        for y in 0..height {
            let qy = (y as f64 + 0.5 - cy) * sy;
            table.push(entry(0.5 + (rect.max_y - qy), 0.5 + (qy - rect.min_y)));
        }
        table
    }
}

/// Composites `draw` over `canvas` and returns the new canvas.
pub fn draw(gpu: &Gpu, canvas: &GpuImage, draw: &Draw) -> GpuImage {
    let pipeline = gpu.pipeline("transform_draw", &format!("{MODES}
{}", include_str!("resample.wgsl")));
    let out = gpu.image(canvas.width, canvas.height);
    let p = &draw.placement;
    // The canvas-space bounding box of the image rectangle, for the aliased overlap test.
    let corners = p.corners(&draw.image.rect);
    let min = |i: usize| corners.iter().map(|c| c[i]).fold(f64::INFINITY, f64::min) as f32;
    let max = |i: usize| corners.iter().map(|c| c[i]).fold(f64::NEG_INFINITY, f64::max) as f32;
    let mut params: Vec<u8> = Vec::with_capacity(192);
    for v in [
        canvas.width,
        canvas.height,
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
        (p.l1() as f32).to_bits(),
        p.upright() as u32,
        0,
    ] {
        params.extend_from_slice(&v.to_le_bytes());
    }
    params.extend_from_slice(bytemuck::cast_slice(&[min(0), min(1), max(0), max(1)]));
    let image = &draw.image;
    params.extend(dda_block(&p.dda(&image.rect, image.width, image.height), &image.rect, image.width, image.height));
    let mask = draw.mask.as_ref().unwrap_or(image);
    params.extend(dda_block(&p.dda(&mask.rect, mask.width, mask.height), &mask.rect, mask.width, mask.height));
    let mut edges = vec![[0u32; 4]];
    if p.upright() {
        edges = p.edge_table(&image.rect, canvas.width, canvas.height);
        edges.extend(p.edge_table(&mask.rect, canvas.width, canvas.height));
    }
    let edges = gpu.bytes(bytemuck::cast_slice(&edges));
    let table = opacity_table(gpu, draw.opacity);
    let placeholder = gpu.bytes(&[0u8; 4]);
    let mask = draw.mask.as_ref().map_or(&placeholder, |m| m.buffer);
    let clip = draw.clip.unwrap_or(&placeholder);
    gpu.dispatch(&pipeline, &params, &[&canvas.buffer, draw.image.buffer, &out.buffer, &table, mask, clip, &edges], canvas.width, canvas.height);
    out
}
