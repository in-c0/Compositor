//! Layer compositing and blend modes.

use crate::gpu::{Gpu, GpuImage};
use comp_format::BlendMode;

const MODES: &str = include_str!("modes.wgsl");

pub fn mode_index(mode: BlendMode) -> u32 {
    BlendMode::ALL.iter().position(|m| *m == mode).unwrap() as u32
}

/// One layer draw, as `LayerRenderer.draw` makes it for a layer placed 1:1 and upright.
pub struct Draw<'a> {
    /// Straight PNG pixels, or a premultiplied surface when `premultiplied` is set.
    pub pixels: &'a GpuImage,
    pub premultiplied: bool,
    /// Where the layer's top-left pixel lands on the canvas.
    pub offset: (i32, i32),
    pub mode: BlendMode,
    pub opacity: f64,
    /// The layer's own mask: coverage per layer pixel.
    pub mask: Option<&'a wgpu::Buffer>,
    /// A clip on the context: coverage per canvas pixel.
    pub clip: Option<&'a wgpu::Buffer>,
}

/// Composites `draw` over `canvas` and returns the new canvas.
pub fn draw_upright(gpu: &Gpu, canvas: &GpuImage, draw: &Draw) -> GpuImage {
    let pipeline = gpu.pipeline("draw_upright", &format!("{MODES}\n{}", include_str!("draw_upright.wgsl")));
    let out = gpu.image(canvas.width, canvas.height);
    let mut params = Vec::with_capacity(48);
    for v in [canvas.width, canvas.height, draw.pixels.width, draw.pixels.height] {
        params.extend_from_slice(&v.to_le_bytes());
    }
    params.extend_from_slice(&draw.offset.0.to_le_bytes());
    params.extend_from_slice(&draw.offset.1.to_le_bytes());
    for v in [
        mode_index(draw.mode),
        draw.mask.is_some() as u32,
        draw.clip.is_some() as u32,
        draw.premultiplied as u32,
        (draw.opacity >= 1.0) as u32,
    ] {
        params.extend_from_slice(&v.to_le_bytes());
    }
    let table = opacity_table(gpu, draw.opacity);
    let placeholder = gpu.bytes(&[0u8; 4]);
    let mask = draw.mask.unwrap_or(&placeholder);
    let clip = draw.clip.unwrap_or(&placeholder);
    gpu.dispatch(&pipeline, &params, &[&canvas.buffer, &draw.pixels.buffer, &out.buffer, &table, mask, clip], canvas.width, canvas.height);
    out
}

/// Opacity as Core Graphics applies it: quantized to a byte, then every premultiplied byte v
/// scaled by it and rounded, (v × alpha + 127) / 255.
pub fn opacity_table(gpu: &Gpu, opacity: f64) -> wgpu::Buffer {
    let alpha = (opacity.clamp(0.0, 1.0) * 255.0).round() as u32;
    let table: Vec<u32> = (0..256u32).map(|v| (v * alpha + 127) / 255).collect();
    gpu.bytes(bytemuck::cast_slice(&table))
}

/// A surface's alpha channel, one value per pixel (`layer_extract_alpha`).
pub fn extract_alpha(gpu: &Gpu, surface: &GpuImage) -> wgpu::Buffer {
    let alpha = gpu.image(surface.width, surface.height).buffer;
    surface_step(gpu, surface, &alpha, 0);
    alpha
}

/// Straight colors at full alpha, in place (`layer_unpremultiply_opaque`).
pub fn unpremultiply_opaque(gpu: &Gpu, surface: &GpuImage) {
    let unused = gpu.image(1, 1).buffer;
    surface_step(gpu, surface, &unused, 1);
}

/// Premultiplies by a saved alpha, in place (`layer_restore_alpha`).
pub fn restore_alpha(gpu: &Gpu, surface: &GpuImage, alpha: &wgpu::Buffer) {
    surface_step(gpu, surface, alpha, 2);
}

fn surface_step(gpu: &Gpu, surface: &GpuImage, alpha: &wgpu::Buffer, step: u32) {
    let pipeline = gpu.pipeline("surface", include_str!("surface.wgsl"));
    let params = [surface.width.to_le_bytes(), surface.height.to_le_bytes(), step.to_le_bytes()].concat();
    gpu.dispatch(&pipeline, &params, &[&surface.buffer, alpha], surface.width, surface.height);
}

/// Premultiplied canvas bytes to the straight bytes a PNG export holds.
pub fn unpremultiply(gpu: &Gpu, canvas: &GpuImage) -> GpuImage {
    let pipeline = gpu.pipeline("unpremultiply", include_str!("unpremultiply.wgsl"));
    let out = gpu.image(canvas.width, canvas.height);
    let params = [canvas.width.to_le_bytes(), canvas.height.to_le_bytes()].concat();
    gpu.dispatch(&pipeline, &params, &[&canvas.buffer, &out.buffer], canvas.width, canvas.height);
    out
}
