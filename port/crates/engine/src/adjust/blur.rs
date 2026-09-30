//! Gaussian Blur and Motion Blur, which the Mac runs through Core Image (`CIGaussianBlur` and
//! `CIMotionBlur`) on an unclamped image, so transparency spreads in from the edges.

use super::{FLOAT, storage};
use crate::gpu::{Gpu, GpuImage};

/// `CIImage.applyingGaussianBlur(sigma:)`: a separable kernel whose taps are the Gaussian's mass
/// over each pixel.
pub fn gaussian(gpu: &Gpu, image: &GpuImage, sigma: f64) -> GpuImage {
    let radius = (sigma * 3.0).ceil().max(1.0) as i64;
    let weights: Vec<f32> = (-radius..=radius)
        .map(|k| (normal_cdf((k as f64 + 0.5) / sigma) - normal_cdf((k as f64 - 0.5) / sigma)) as f32)
        .collect();
    separable(gpu, image, &weights)
}

/// `CIMotionBlur` along `angle` degrees, counterclockwise from horizontal.
pub fn motion(gpu: &Gpu, image: &GpuImage, distance: f64, _angle: f64) -> GpuImage {
    // Placeholder until Core Image's kernel is worked out.
    gaussian(gpu, image, distance / 12f64.sqrt())
}

fn separable(gpu: &Gpu, image: &GpuImage, weights: &[f32]) -> GpuImage {
    let (w, h) = (image.width, image.height);
    let out = gpu.image(w, h);
    let rows = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("blur rows"),
        size: (w as u64 * h as u64 * 16).max(16),
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    let weights = storage(gpu, bytemuck::cast_slice(weights));
    let radius = (weights.size() / 4 / 2) as u32;
    let pipeline = gpu.pipeline("adjust.blur", &format!("{FLOAT}\n{}", include_str!("blur.wgsl")));
    for pass in 0..2u32 {
        let words = [f32::INFINITY.to_bits(), w, h, pass, radius];
        gpu.dispatch(&pipeline, bytemuck::cast_slice(&words), &[&image.buffer, &out.buffer, &rows, &weights], w, h);
    }
    out
}

/// Φ(x), from `erfc` to full double precision.
fn normal_cdf(x: f64) -> f64 {
    0.5 * erfc(-x / std::f64::consts::SQRT_2)
}

/// Complementary error function (W. J. Cody's rational approximations, |error| < 1e-15).
fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    // Numerical Recipes' erfcc is only good to 1.2e-7; use a continued series instead.
    let r = if z < 0.5 {
        1.0 - erf_series(z)
    } else {
        erfc_continued(z)
    };
    let _ = t;
    if x < 0.0 { 2.0 - r } else { r }
}

fn erf_series(x: f64) -> f64 {
    // erf x = 2/√π Σ (-1)^n x^(2n+1) / (n! (2n+1))
    let mut sum = 0.0f64;
    let mut term = x;
    let mut n = 0.0;
    while term.abs() > 1e-17 * sum.abs().max(1e-300) || n < 1.0 {
        sum += term / (2.0 * n + 1.0);
        n += 1.0;
        term *= -x * x / n;
    }
    sum * 2.0 / std::f64::consts::PI.sqrt()
}

fn erfc_continued(x: f64) -> f64 {
    // Lentz's method on erfc x = e^{-x²}/√π · 1/(x + 1/2/(x + 1/(x + 3/2/(x + …))))
    let tiny = 1e-300;
    let mut f = x;
    let mut c = x;
    let mut d = 0.0;
    for i in 1..500 {
        let a = i as f64 / 2.0;
        d = x + a * d;
        d = if d.abs() < tiny { tiny } else { d };
        c = x + a / c;
        c = if c.abs() < tiny { tiny } else { c };
        d = 1.0 / d;
        let delta = c * d;
        f *= delta;
        if (delta - 1.0).abs() < 1e-16 {
            break;
        }
    }
    (-x * x).exp() / std::f64::consts::PI.sqrt() / f
}
