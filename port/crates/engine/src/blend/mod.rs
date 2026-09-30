//! Layer compositing and blend modes.

use crate::gpu::{Gpu, GpuImage};
use comp_format::BlendMode;

const MODES: &str = include_str!("modes.wgsl");

pub fn mode_index(mode: BlendMode) -> u32 {
    BlendMode::ALL.iter().position(|m| *m == mode).unwrap() as u32
}

/// Draws `layer` (straight RGBA) 1:1 and upright at a whole-pixel `offset`, over `canvas`.
pub fn draw_upright(gpu: &Gpu, canvas: &GpuImage, layer: &GpuImage, offset: (i32, i32), mode: BlendMode, opacity: f64) -> GpuImage {
    let pipeline = gpu.pipeline("draw_upright", &format!("{MODES}\n{}", include_str!("draw_upright.wgsl")));
    let out = gpu.image(canvas.width, canvas.height);
    let mut params = Vec::with_capacity(32);
    for v in [canvas.width, canvas.height, layer.width, layer.height] {
        params.extend_from_slice(&v.to_le_bytes());
    }
    params.extend_from_slice(&offset.0.to_le_bytes());
    params.extend_from_slice(&offset.1.to_le_bytes());
    params.extend_from_slice(&mode_index(mode).to_le_bytes());
    params.extend_from_slice(&(opacity as f32).to_le_bytes());
    let table = opacity_table(gpu, opacity);
    gpu.dispatch(&pipeline, &params, &[&canvas.buffer, &layer.buffer, &out.buffer, &table], canvas.width, canvas.height);
    out
}

/// Opacity as Core Graphics applies it: quantized to a byte, then every premultiplied byte v
/// scaled by it and rounded, (v × alpha + 127) / 255.
pub fn opacity_table(gpu: &Gpu, opacity: f64) -> wgpu::Buffer {
    use wgpu::util::DeviceExt;
    let alpha = (opacity.clamp(0.0, 1.0) * 255.0).round() as u32;
    let table: Vec<u32> = (0..256u32).map(|v| (v * alpha + 127) / 255).collect();
    gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("opacity table"),
        contents: bytemuck::cast_slice(&table),
        usage: wgpu::BufferUsages::STORAGE,
    })
}

/// Premultiplied canvas bytes to the straight bytes a PNG export holds.
pub fn unpremultiply(gpu: &Gpu, canvas: &GpuImage) -> GpuImage {
    let pipeline = gpu.pipeline("unpremultiply", include_str!("unpremultiply.wgsl"));
    let out = gpu.image(canvas.width, canvas.height);
    let params = [canvas.width.to_le_bytes(), canvas.height.to_le_bytes()].concat();
    gpu.dispatch(&pipeline, &params, &[&canvas.buffer, &out.buffer], canvas.width, canvas.height);
    out
}
