//! Gaussian Blur and Motion Blur, which the Mac runs through Core Image (`CIGaussianBlur` and
//! `CIMotionBlur`) on an unclamped image, so transparency spreads in from the edges.
//!
//! Core Image's kernels aren't public; this is what its output shows. Both blurs weight their taps
//! by the Gaussian's mass over each step (`Φ(k + ½) − Φ(k − ½)`, not its value at `k`), keep the
//! taps with |k| ≤ ⌊3σ⌋ + 1 and don't renormalize. Gaussian Blur runs along rows, then columns.
//! Motion Blur is one such pass along its angle, stepping one pixel along the dominant axis, so
//! a 45° streak steps diagonally and its σ is counted in those steps. The remaining difference
//! is Core Image's arithmetic (it keeps half-float intermediates and treats taps near the edge
//! slightly differently), which moves some channels by one or two levels; see the report in
//! `tests.rs`.

use super::{FLOAT, storage};
use crate::gpu::{Gpu, GpuImage};

/// `CIImage.applyingGaussianBlur(sigma:)`.
pub fn gaussian(gpu: &Gpu, image: &GpuImage, sigma: f64) -> GpuImage {
    let weights = taps(sigma);
    let floats = floats(gpu, image.width as u64 * image.height as u64);
    let out = gpu.image(image.width, image.height);
    dispatch(gpu, image, &out, &floats, 0, &weights, (1.0, 0.0));
    dispatch(gpu, image, &out, &floats, 1, &weights, (0.0, 1.0));
    out
}

/// `CIMotionBlur` with `radius` = distance / √12, along `angle` degrees counterclockwise from
/// horizontal.
pub fn motion(gpu: &Gpu, image: &GpuImage, distance: f64, angle: f64) -> GpuImage {
    let sigma = distance * (1.0 / 12f64.sqrt());
    let radians = angle.to_radians();
    // Exact axes: cos 90° isn't quite zero in floating point.
    let exact = |v: f64| if v.abs() < 1e-12 { 0.0 } else { v };
    let (cos, sin) = (exact(radians.cos()), exact(radians.sin()));
    let longest = cos.abs().max(sin.abs());
    let weights = taps(sigma * longest);
    let out = gpu.image(image.width, image.height);
    // Core Image's y axis points up; the image's rows run down.
    let step = ((cos / longest) as f32, (-sin / longest) as f32);
    dispatch(gpu, image, &out, &floats(gpu, 1), 2, &weights, step);
    out
}

/// The Gaussian's mass over each unit step, for steps −R…R.
fn taps(sigma: f64) -> Vec<f32> {
    let radius = (3.0 * sigma).floor() as i64 + 1;
    (-radius..=radius).map(|k| (phi((k as f64 + 0.5) / sigma) - phi((k as f64 - 0.5) / sigma)) as f32).collect()
}

fn floats(gpu: &Gpu, count: u64) -> wgpu::Buffer {
    gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("blur floats"),
        size: count.max(1) * 16,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    })
}

fn dispatch(gpu: &Gpu, image: &GpuImage, out: &GpuImage, floats: &wgpu::Buffer, stage: u32, weights: &[f32], step: (f32, f32)) {
    let (w, h) = (image.width, image.height);
    let radius = (weights.len() / 2) as u32;
    let weights = storage(gpu, bytemuck::cast_slice(weights));
    let pipeline = gpu.pipeline("adjust.blur", &format!("{FLOAT}
{}", include_str!("blur.wgsl")));
    let words = [f32::INFINITY.to_bits(), w, h, stage, radius, step.0.to_bits(), step.1.to_bits()];
    gpu.dispatch(&pipeline, bytemuck::cast_slice(&words), &[&image.buffer, &out.buffer, floats, &weights], w, h);
}

/// Φ(x), the standard normal distribution, to near double precision.
fn phi(x: f64) -> f64 {
    let z = -x / std::f64::consts::SQRT_2;
    0.5 * if z < 0.0 { 2.0 - erfc(-z) } else { erfc(z) }
}

/// erfc for x >= 0: the Taylor series of erf below 2, a continued fraction above.
fn erfc(x: f64) -> f64 {
    let root_pi = std::f64::consts::PI.sqrt();
    if x < 2.0 {
        let (mut sum, mut term, mut n) = (0.0f64, x, 0.0f64);
        while term.abs() > 1e-18 {
            sum += term / (2.0 * n + 1.0);
            n += 1.0;
            term *= -x * x / n;
        }
        return 1.0 - 2.0 / root_pi * sum;
    }
    // erfc x = e^{−x²}/√π · 1/(x + ½/(x + 1/(x + 3⁄2/(x + …)))), evaluated from the tail.
    let mut fraction = x;
    for i in (1..120).rev() {
        fraction = x + (i as f64 / 2.0) / fraction;
    }
    (-x * x).exp() / root_pi / fraction
}
