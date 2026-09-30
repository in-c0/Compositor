//! Filter > Dither: `DitherSettings.apply` (Document/Dither.swift) around `dither_apply` and
//! `dither_dots` (Rendering/DitherPixels.c), whose stages are in `dither.wgsl`.
//!
//! Chunky pixels dither a copy reduced by the pixel size and blow the result back up. Core
//! Graphics makes that copy with its High interpolation (`transform::high`). ASCII draws Core Text
//! glyphs and the scanlines' glow is a Core Image blur; those aren't reproduced.

use super::settings::{DitherColors, DitherSettings, DitherStyle};
use super::{DOUBLE, FLOAT};
use crate::RenderError;
use crate::gpu::{Gpu, GpuImage};

const TONE: u32 = 0;
const DIFFUSE: u32 = 1;
const ORDERED: u32 = 2;
const WRITE_LEVELS: u32 = 3;
const MARKS: u32 = 4;
const SCANLINES: u32 = 5;
const ENLARGE: u32 = 6;
const ROUND_PIXELS: u32 = 7;

/// Error diffusion runs as one invocation per plane; it works through this many rows per
/// dispatch so no single dispatch runs long.
const DIFFUSION_ROWS: u32 = 64;

pub fn apply(gpu: &Gpu, image: &GpuImage, settings: &DitherSettings) -> Result<GpuImage, RenderError> {
    let s = settings.normalized();
    if s.style == DitherStyle::Ascii {
        return Err(RenderError::Unsupported("Dither's ASCII style (Core Text glyphs)".into()));
    }
    if s.style == DitherStyle::Scanlines && s.glow > 0.0 {
        return Err(RenderError::Unsupported("Dither's scanline glow (a Core Image blur)".into()));
    }
    if s.density != 0.0 {
        return Err(RenderError::Unsupported("Dither's Density (powf)".into()));
    }
    let block = if s.style.uses_pixel_size() { s.pixel_size as u32 } else { 1 };
    // Chunky pixels dither a copy Core Graphics draws with High at 1/block the size, top left.
    let small;
    let working = if block > 1 {
        let pixels = image::RgbaImage::from_raw(image.width, image.height, gpu.download(image).map_err(RenderError::Failed)?).expect("image size");
        let size = (image.width.div_ceil(block), image.height.div_ceil(block));
        let rect = [0.0, 0.0, image.width as f64 / block as f64, image.height as f64 / block as f64];
        small = crate::transform::high::draw_into(gpu, &pixels, size, rect, crate::transform::Quality::High);
        &small
    } else {
        image
    };
    let dithered = dither(gpu, working, &s);
    if block == 1 {
        return Ok(dithered);
    }
    let mut params = Params::new(&s, image.width, image.height);
    params.cell = block;
    let full = params.run(gpu, ENLARGE, &dithered, None, None, (image.width, image.height));
    if !s.dot_pixels {
        return Ok(full);
    }
    // The gaps are the dark color: black, or the one picked.
    let gap = if s.colors == DitherColors::TwoColors { color_bytes(s.dark) } else { [0.0; 3] };
    params.dark = [gap[0], gap[1], gap[2], 0.0];
    Ok(params.run(gpu, ROUND_PIXELS, &full, None, None, (image.width, image.height)))
}

/// `UInt8((component * 255).rounded())`.
fn color_bytes(c: comp_format::AdjustmentColor) -> [f32; 3] {
    [c.red, c.green, c.blue].map(|v| (v * 255.0).round() as u8 as f32)
}

/// `DitherSettings.dither` and `dither_apply`.
fn dither(gpu: &Gpu, image: &GpuImage, s: &DitherSettings) -> GpuImage {
    let (w, h) = (image.width, image.height);
    let params = Params::new(s, w, h);
    let tone = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("dither tone"),
        size: (w as u64 * h as u64 * params.planes as u64 * 4).max(4),
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    params.run(gpu, TONE, image, Some(&tone), None, (w, h));
    match s.style {
        DitherStyle::Atkinson | DitherStyle::FloydSteinberg => {
            let mut p = params.clone();
            for row0 in (0..h).step_by(DIFFUSION_ROWS as usize) {
                p.row0 = row0;
                p.row1 = (row0 + DIFFUSION_ROWS).min(h);
                p.run(gpu, DIFFUSE, image, Some(&tone), None, (p.planes, 1));
            }
            params.run(gpu, WRITE_LEVELS, image, Some(&tone), None, (w, h))
        }
        DitherStyle::Bayer2 | DitherStyle::Bayer4 | DitherStyle::Bayer8 => {
            params.run(gpu, ORDERED, image, Some(&tone), None, (w, h));
            params.run(gpu, WRITE_LEVELS, image, Some(&tone), None, (w, h))
        }
        DitherStyle::Scanlines => {
            let spacing = params.cell.max(2);
            let lines = h.div_ceil(spacing);
            // Each line's wobble, `lroundf(wobble * wave)`: the sines depend only on the line.
            let wobble = s.wobble as f32;
            let shifts: Vec<i32> = (0..lines)
                .map(|line| {
                    let sine = |v: f32| (v as f64).sin() as f32;
                    let wave = sine(line as f32 * 0.45).mul_add(0.7, sine((line as f32).mul_add(1.7, 1.3)) * 0.3);
                    (wobble * wave).round() as i32
                })
                .collect();
            let shifts = gpu.bytes(bytemuck::cast_slice(&shifts));
            params.run(gpu, SCANLINES, image, Some(&tone), Some(&shifts), (w, h))
        }
        _ => params.run(gpu, MARKS, image, Some(&tone), None, (w, h)),
    }
}

/// `dither.wgsl`'s uniform.
#[derive(Clone)]
struct Params {
    width: u32,
    height: u32,
    planes: u32,
    levels: u32,
    style: u32,
    light_on_dark: u32,
    original: u32,
    cell: u32,
    row0: u32,
    row1: u32,
    diffusion: f32,
    contrast: f32,
    cos_a: f32,
    sin_a: f32,
    dots: f32,
    dark: [f32; 4],
    light: [f32; 4],
}

impl Params {
    /// `DitherParams` as `DitherSettings.dither` fills it, and the per-image constants
    /// `dither_apply` works out from it.
    fn new(s: &DitherSettings, width: u32, height: u32) -> Self {
        let original = s.colors == DitherColors::Original;
        let (dark, light) = if s.colors == DitherColors::TwoColors {
            (color_bytes(s.dark), color_bytes(s.light))
        } else {
            ([0.0; 3], [255.0; 3])
        };
        let contrast = (s.contrast / 100.0) as f32;
        let contrast = if contrast >= 0.0 { 1.0 / (-0.95f32).mul_add(contrast, 1.0) } else { 1.0 + contrast };
        let angle = (s.angle * std::f64::consts::PI / 180.0) as f32;
        let cell = if s.style == DitherStyle::Scanlines { s.line_spacing } else { s.cell_size } as u32;
        Self {
            width,
            height,
            planes: if original { 3 } else { 1 },
            levels: (s.levels as u32).clamp(2, 16),
            style: s.style.code(),
            light_on_dark: s.light_on_dark as u32,
            original: original as u32,
            cell: if s.style == DitherStyle::Scanlines { cell } else { cell.max(2) },
            row0: 0,
            row1: 0,
            diffusion: (s.diffusion / 100.0) as f32,
            contrast,
            cos_a: (angle as f64).cos() as f32,
            sin_a: (angle as f64).sin() as f32,
            dots: (s.dots / 100.0) as f32,
            dark: [dark[0] / 255.0, dark[1] / 255.0, dark[2] / 255.0, 0.0],
            light: [light[0] / 255.0, light[1] / 255.0, light[2] / 255.0, 0.0],
        }
    }

    /// Runs one stage over a `grid`, reading `image` and returning what it wrote.
    fn run(
        &self,
        gpu: &Gpu,
        stage: u32,
        image: &GpuImage,
        tone: Option<&wgpu::Buffer>,
        shifts: Option<&wgpu::Buffer>,
        grid: (u32, u32),
    ) -> GpuImage {
        let (width, height) = if stage == ENLARGE || stage == ROUND_PIXELS { grid } else { (self.width, self.height) };
        let mut words: Vec<u32> = vec![
            f32::INFINITY.to_bits(),
            width,
            height,
            stage,
            self.planes,
            self.levels,
            self.style,
            self.light_on_dark,
            self.original,
            self.cell,
            self.row0,
            self.row1,
            self.diffusion.to_bits(),
            self.contrast.to_bits(),
            self.cos_a.to_bits(),
            self.sin_a.to_bits(),
            self.dots.to_bits(),
            0,
            0,
            0,
        ];
        words.extend(self.dark.map(f32::to_bits));
        words.extend(self.light.map(f32::to_bits));
        let out = gpu.image(width, height);
        // Separate stand-ins: one binding is written, so they can't share a buffer.
        let (no_tone, no_shifts) = (gpu.bytes(&[0u8; 4]), gpu.bytes(&[0u8; 4]));
        let pipeline = gpu.pipeline("filters.dither", &format!("{FLOAT}\n{DOUBLE}\n{}", include_str!("dither.wgsl")));
        let buffers = [&image.buffer, &out.buffer, tone.unwrap_or(&no_tone), shifts.unwrap_or(&no_shifts)];
        gpu.dispatch(&pipeline, bytemuck::cast_slice(&words), &buffers, grid.0, grid.1);
        out
    }
}
