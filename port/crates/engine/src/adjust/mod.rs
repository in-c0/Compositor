//! Adjustment layers: every `comp_format::Adjustment` kind applied to a premultiplied RGBA8
//! image, as the Mac's `LayerAdjustment.apply(_:region:scale:)` does.
//!
//! The Mac runs most kinds as C loops over an 8-bit premultiplied bitmap (`AdjustPixels.c`,
//! `LevelsPixels.c`, `NoisePixels.c`), Invert through vImage and the blurs through Core Image.
//! Here each per-pixel loop is a WGSL kernel that repeats the C arithmetic operation for
//! operation (see `float.wgsl` for how the rounding is kept), and whatever depends only on the
//! settings (lookup tables, the Hue/Saturation cube, blur kernels) is computed on the CPU in
//! `f64`, as the Swift code does, and uploaded.

pub(crate) mod blur;
pub(crate) mod gaussian;
mod tables;

use crate::gpu::{Gpu, GpuImage};
use comp_format::{Adjustment, AdjustmentKind};
use wgpu::util::DeviceExt;

/// Where the image being adjusted sits in the document, for the kinds that are tied to document
/// space rather than to the image's pixels: Grain's pattern, Add Noise's pattern and the blurs'
/// reach.
///
/// This is the Mac's `region` and `scale` arguments. The export renders the document at one
/// pixel per unit and passes the whole canvas, which is [`Region::whole`]. The canvas view passes
/// the visible part of the document (`x`, `y`, `width`, `height` in document units, covering the
/// whole image) and its zoom as `scale`, image pixels per document pixel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
}

impl Region {
    /// The image is the whole document at one pixel per unit.
    pub fn whole(image: &GpuImage) -> Self {
        Self { x: 0.0, y: 0.0, width: image.width as f64, height: image.height as f64, scale: 1.0 }
    }
}

const FLOAT: &str = include_str!("float.wgsl");

/// Applies `adjustment` to `image` (premultiplied RGBA8) and returns the adjusted pixels, the
/// same size. Settings the Mac rejects (its `apply` throws and the canvas is left as it was)
/// give an unchanged copy.
pub fn apply(gpu: &Gpu, image: &GpuImage, adjustment: &Adjustment, region: Region) -> anyhow::Result<GpuImage> {
    let run = Run { gpu, image };
    Ok(match adjustment.kind {
        AdjustmentKind::HueSaturation => {
            let cube = tables::hsl_cube(&tables::resolved_hsv(adjustment));
            let n = tables::CUBE_DIMENSION as u32;
            let scale = (n - 1) as f32 / 255.0f32;
            run.kernel("adjust.cube", include_str!("cube.wgsl"), &[n, scale.to_bits()], &[&run.floats(&cube)])
        }
        AdjustmentKind::Levels => {
            if tables::levels_identity(&adjustment.levels) {
                return Ok(run.copy());
            }
            run.lut(&tables::levels_tables(&adjustment.levels))
        }
        AdjustmentKind::Curves => {
            if !curves_valid(adjustment) {
                return Ok(run.copy());
            }
            run.lut(&tables::curves_tables(&adjustment.curves))
        }
        AdjustmentKind::Exposure => {
            let e = adjustment.exposure_settings.unwrap_or_default();
            let valid = (-20.0..=20.0).contains(&e.exposure) && (-0.5..=0.5).contains(&e.offset) && (0.01..=9.99).contains(&e.gamma);
            if !valid {
                return Ok(run.copy());
            }
            run.lut(&tables::exposure_tables(&e))
        }
        AdjustmentKind::GradientMap => {
            let g = adjustment.gradient_map_settings.unwrap_or_default();
            let valid = [g.shadows, g.highlights]
                .iter()
                .all(|c| [c.red, c.green, c.blue].iter().all(|v| (0.0..=1.0).contains(v)));
            if !valid {
                return Ok(run.copy());
            }
            let table = tables::gradient_map_table(&g);
            run.kernel("adjust.gradient_map", include_str!("gradient_map.wgsl"), &[], &[&run.words(&table)])
        }
        AdjustmentKind::Grain => grain(&run, adjustment, region),
        AdjustmentKind::AddNoise => noise(&run, adjustment, region),
        AdjustmentKind::Invert => run.kernel("adjust.invert", include_str!("invert.wgsl"), &[], &[]),
        AdjustmentKind::BlackWhite => black_white(&run, adjustment),
        AdjustmentKind::ColorBalance => color_balance(&run, adjustment),
        AdjustmentKind::GaussianBlur => gaussian::gaussian(gpu, image, gaussian_radius(adjustment) * region.scale).map_err(anyhow::Error::msg)?,
        AdjustmentKind::MotionBlur => {
            let (distance, angle) = motion_settings(adjustment);
            blur::motion(gpu, image, distance * region.scale, angle).map_err(anyhow::Error::msg)?
        }
    })
}

/// What `apply` can't reproduce exactly for these settings, if anything: the caller reports it
/// as unsupported rather than drawing something close.
pub fn unsupported(adjustment: &Adjustment, region: Region) -> Option<String> {
    match adjustment.kind {
        AdjustmentKind::GaussianBlur => gaussian::unsupported(gaussian_radius(adjustment) * region.scale),
        AdjustmentKind::MotionBlur => blur::motion_unsupported(motion_settings(adjustment).0 * region.scale),
        _ => None,
    }
}

/// Gaussian Blur's radius (Core Image's σ), as `FilterSettings.normalized` clamps it;
/// `gaussianRadius` defaults to 10.
fn gaussian_radius(adjustment: &Adjustment) -> f64 {
    clamp(adjustment.blur_radius.unwrap_or(10.0), 0.1, 250.0, 1.0)
}

/// Motion Blur's distance and angle, as `FilterSettings.normalized` clamps them.
fn motion_settings(adjustment: &Adjustment) -> (f64, f64) {
    let angle = clamp(adjustment.motion_angle.unwrap_or(0.0), -90.0, 90.0, 0.0);
    let distance = clamp(adjustment.motion_distance.unwrap_or(10.0), 1.0, 2000.0, 10.0);
    (distance, angle)
}

/// `FilterSettings.normalized`'s clamp: non-finite values fall back.
fn clamp(value: f64, lo: f64, hi: f64, fallback: f64) -> f64 {
    if value.is_finite() { value.max(lo).min(hi) } else { fallback }
}

/// `CurvesSettings.isValid`.
fn curves_valid(adjustment: &Adjustment) -> bool {
    let channels = &adjustment.curves.channels;
    channels.len() == 4
        && channels.iter().all(|points| {
            (2..=32).contains(&points.len())
                && points.first().is_some_and(|p| p.x == 0.0)
                && points.last().is_some_and(|p| p.x == 255.0)
                && points.iter().all(|p| (0.0..=255.0).contains(&p.x) && (0.0..=255.0).contains(&p.y))
                && points.windows(2).all(|w| w[0].x < w[1].x)
        })
}

fn grain(run: &Run, adjustment: &Adjustment, region: Region) -> GpuImage {
    let g = adjustment.grain_settings.unwrap_or_default();
    let (w, h) = (run.image.width, run.image.height);
    // `LayerAdjustment.apply`: document units per image pixel.
    let units_per_pixel = region.width / w.max(1) as f64;
    let valid = (0.0..=100.0).contains(&g.amount)
        && (0.5..=20.0).contains(&g.size)
        && (0.0..=100.0).contains(&g.roughness)
        && units_per_pixel.is_finite()
        && units_per_pixel > 0.0;
    if !valid || !(g.amount > 0.0) {
        return run.copy();
    }
    let strength = ((g.amount / 100.0).min(1.0) as f32 * 0.35f32) * 255.0f32;
    let rough = (g.roughness / 100.0).clamp(0.0, 1.0) as f32;
    let detail_size = (g.size * 0.35).max(0.5);
    let columns = tables::grain_axis(w, region.x, units_per_pixel, g.size, detail_size);
    let rows = tables::grain_axis(h, region.y, units_per_pixel, g.size, detail_size);
    let fine_seed = mix32(g.seed ^ 0xA511E9B3);
    let columns = run.words(bytemuck::cast_slice(&columns));
    let rows = run.words(bytemuck::cast_slice(&rows));
    run.kernel(
        "adjust.grain",
        include_str!("grain.wgsl"),
        &[g.seed, fine_seed, strength.to_bits(), rough.to_bits()],
        &[&columns, &rows],
    )
}

fn mix32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846ca68b);
    x ^= x >> 16;
    x
}

fn noise(run: &Run, adjustment: &Adjustment, region: Region) -> GpuImage {
    let amount = clamp(adjustment.noise_amount.unwrap_or(10.0), 0.1, 400.0, 10.0);
    let (w, h) = (run.image.width as f64, run.image.height as f64);
    // The region's origin in the image's own pixels, rounded down to whole pixels and truncated
    // to 32 bits, as `LayerAdjustment.apply` and `noise_add_at` take it.
    let origin_x = (region.x * w / region.width.max(1.0)).floor() as i64 as u32;
    let origin_y = (region.y * h / region.height.max(1.0)).floor() as i64 as u32;
    let spread = amount as f32 / 100.0f32 * 127.5f32;
    run.kernel(
        "adjust.noise",
        include_str!("noise.wgsl"),
        &[
            adjustment.noise_seed.unwrap_or(0),
            origin_x,
            origin_y,
            adjustment.noise_gaussian.unwrap_or(false) as u32,
            adjustment.noise_monochromatic.unwrap_or(false) as u32,
            spread.to_bits(),
        ],
        &[],
    )
}

fn black_white(run: &Run, adjustment: &Adjustment) -> GpuImage {
    let s = adjustment.black_white_settings.unwrap_or_default();
    let weights = [s.reds, s.yellows, s.greens, s.cyans, s.blues, s.magentas];
    let valid = weights.iter().all(|v| (-200.0..=300.0).contains(v))
        && (0.0..=360.0).contains(&s.tint_hue)
        && (0.0..=100.0).contains(&s.tint_saturation);
    if !valid {
        return run.copy();
    }
    let weights: Vec<f32> = weights.iter().map(|v| (v / 100.0) as f32).collect();
    // The tint's per-image doubles; the kernel does the per-pixel rest in double-single.
    let saturation = s.tint_saturation / 100.0;
    let hp = (s.tint_hue % 360.0) / 60.0;
    let second = 1.0 - ((hp % 2.0) - 1.0).abs();
    let sector = (hp.floor() as u32).min(5);
    let (saturation_hi, saturation_lo) = split(saturation);
    let (second_hi, second_lo) = split(second);
    run.kernel(
        "adjust.black_white",
        include_str!("black_white.wgsl"),
        &[
            (s.tint && saturation > 0.0) as u32,
            sector,
            saturation_hi.to_bits(),
            saturation_lo.to_bits(),
            second_hi.to_bits(),
            second_lo.to_bits(),
        ],
        &[&run.floats(&weights)],
    )
}

/// A double as the sum of two floats.
fn split(value: f64) -> (f32, f32) {
    let hi = value as f32;
    (hi, (value - hi as f64) as f32)
}

fn color_balance(run: &Run, adjustment: &Adjustment) -> GpuImage {
    let s = adjustment.color_balance_settings.unwrap_or_default();
    let all = [
        s.shadow_cyan_red,
        s.shadow_magenta_green,
        s.shadow_yellow_blue,
        s.mid_cyan_red,
        s.mid_magenta_green,
        s.mid_yellow_blue,
        s.highlight_cyan_red,
        s.highlight_magenta_green,
        s.highlight_yellow_blue,
    ];
    // Invalid settings throw; all-zero ones return the image untouched.
    if !all.iter().all(|v| (-100.0..=100.0).contains(v)) || all.iter().all(|&v| v == 0.0) {
        return run.copy();
    }
    let shifts: Vec<f32> = all.iter().map(|v| (v / 100.0) as f32).collect();
    run.kernel(
        "adjust.color_balance",
        include_str!("color_balance.wgsl"),
        &[s.preserve_luminosity as u32],
        &[&run.floats(&shifts)],
    )
}

/// One image through one kernel.
struct Run<'a> {
    gpu: &'a Gpu,
    image: &'a GpuImage,
}

impl Run<'_> {
    /// Runs `source` once per pixel. Its uniform gets the float guard (+infinity), the width and
    /// the height, then `params`; the input is bound at 1, the output at 2, then `extra` in order.
    fn kernel(&self, name: &'static str, source: &str, params: &[u32], extra: &[&wgpu::Buffer]) -> GpuImage {
        let (w, h) = (self.image.width, self.image.height);
        let out = self.gpu.image(w, h);
        let mut words = vec![f32::INFINITY.to_bits(), w, h];
        words.extend_from_slice(params);
        let mut buffers = vec![&self.image.buffer, &out.buffer];
        buffers.extend_from_slice(extra);
        let pipeline = self.gpu.pipeline(name, &format!("{FLOAT}\n{source}"));
        self.gpu.dispatch(&pipeline, bytemuck::cast_slice(&words), &buffers, w, h);
        out
    }

    fn lut(&self, tables: &[f32]) -> GpuImage {
        self.kernel("adjust.lut", include_str!("lut.wgsl"), &[], &[&self.floats(tables)])
    }

    fn copy(&self) -> GpuImage {
        copy(self.gpu, self.image)
    }

    fn floats(&self, values: &[f32]) -> wgpu::Buffer {
        storage(self.gpu, bytemuck::cast_slice(values))
    }

    fn words(&self, values: &[u32]) -> wgpu::Buffer {
        storage(self.gpu, bytemuck::cast_slice(values))
    }
}

fn copy(gpu: &Gpu, image: &GpuImage) -> GpuImage {
    let out = gpu.image(image.width, image.height);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(&image.buffer, 0, &out.buffer, 0, image.buffer.size());
    gpu.queue.submit([encoder.finish()]);
    out
}

fn storage(gpu: &Gpu, bytes: &[u8]) -> wgpu::Buffer {
    gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("adjust table"),
        contents: bytes,
        usage: wgpu::BufferUsages::STORAGE,
    })
}

#[cfg(test)]
mod tests;
