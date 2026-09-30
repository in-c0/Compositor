//! Gaussian Blur and Motion Blur, which the Mac runs through Core Image (`CIGaussianBlur` and
//! `CIMotionBlur`) on an unclamped image, so transparency spreads in from the edges.
//!
//! Motion Blur is Core Image's own kernels, measured on the Mac and read from their compiled
//! code (`_gaussianReduce2`/`4`, `_gaussianBlurN`, `_cubicUpsample10h`, in `ci_filters.metallib`).
//! For a streak off the horizontal, Core Image turns the image so the streak runs along its rows,
//! blurs the rows and turns the result back, both turns reading bilinearly. The row blur is one
//! Gaussian kernel when σ ≤ 3; above that the rows are shrunk by 2 or 4 at a time (to 1/S), blurred
//! there and grown back with a cubic B-spline, the shrinking and growing standing in for part of
//! the spread. Every intermediate image is half float. See `blur.wgsl` for the GPU arithmetic.
//!
//! Gaussian Blur goes through Metal Performance Shaders on the Mac (`MPSImageGaussianBlur`,
//! wrapped by Core Image's `CIBlurProcessor`), whose kernel weights come from a fit it runs on
//! the CPU; it isn't reproduced yet.

use super::FLOAT;
use crate::gpu::{Gpu, GpuImage};

/// Why `motion` can't reproduce a streak of `distance` pixels, if it can't.
pub fn motion_unsupported(distance: f64) -> Option<String> {
    MotionPlan::new(distance * (1.0 / 12f64.sqrt())).err()
}

/// `CIMotionBlur` for `PixelFilter`'s Motion Blur: a streak `distance` pixels long at `angle`
/// degrees counterclockwise from the horizontal. `Err` says what isn't reproduced.
pub fn motion(gpu: &Gpu, image: &GpuImage, distance: f64, angle: f64) -> Result<GpuImage, String> {
    // `PixelFilter.motionRadiusPerPixel`: Core Image's radius is the Gaussian's σ.
    let sigma = distance * (1.0 / 12f64.sqrt());
    let plan = MotionPlan::new(sigma)?;
    let run = Pass { gpu, image };
    let radians = angle * std::f64::consts::PI / 180.0;
    if plan.scale == 1 && plan.kernel.is_none() {
        return Ok(super::copy(gpu, image));
    }
    if radians == 0.0 {
        return Ok(run.rows(&plan));
    }
    Ok(run.turned(&plan, radians.cos(), radians.sin()))
}

/// Core Image's `_conv3x3sym` for `CIGaussianBlur` with 0.16 ≤ σ ≤ 0.4: the average of four
/// bilinear reads at (±p, ±p), p = 2(1 − Φ(½/σ)), which is [p/2, 1 − p, p/2] each way.
pub fn conv3x3(gpu: &Gpu, image: &GpuImage, sigma: f64) -> GpuImage {
    let run = Pass { gpu, image };
    let out = gpu.image(image.width, image.height);
    let p = (2.0 * (1.0 - phi(0.5 / sigma))) as f32;
    let params = Params { op: OP_CONV3X3, cos: p, image_h: image.height, ..Default::default() };
    run.dispatch(params, &Source::Image, &Target::Image(&out), &[]);
    out
}

/// What `CIMotionBlur` runs for σ: the shrink factor and the row kernel at that scale.
struct MotionPlan {
    scale: u32,
    kernel: Option<Kernel>,
}

/// A `_gaussianBlurN` kernel: taps on texel centers, or bilinear pairs (offset, weight).
enum Kernel {
    Direct(Vec<f32>),
    Pairs(Vec<(f32, f32)>),
}

/// The variance, in its own input's pixels, that `_gaussianReduce2` and `_gaussianReduce4` stand
/// for, and that of the B-spline that grows the result back (9/8π, its peak matched by a Gaussian),
/// in the small image's pixels.
const REDUCE2_VARIANCE: f64 = 1.75 * 1.75;
const REDUCE4_VARIANCE: f64 = 3.15 * 3.15;

/// The shrink steps for a total factor `scale`: fours first, then a two if one is left over.
fn levels(scale: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let mut s = scale;
    while s >= 4 {
        out.push(4);
        s /= 4;
    }
    if s == 2 {
        out.push(2);
    }
    out
}

/// The spread the shrinking and growing add at total factor `scale`, in the small image's pixels
/// squared.
fn pyramid_variance(scale: u32) -> f64 {
    let (mut variance, mut step) = (0.0, 1.0);
    for level in levels(scale) {
        variance += if level == 2 { REDUCE2_VARIANCE } else { REDUCE4_VARIANCE } * step * step;
        step *= level as f64;
    }
    variance / (scale as f64 * scale as f64) + 9.0 / (8.0 * std::f64::consts::PI)
}

impl MotionPlan {
    fn new(sigma: f64) -> Result<Self, String> {
        if sigma <= 3.0 {
            return Ok(Self { scale: 1, kernel: full_kernel(sigma) });
        }
        // The largest factor whose shrinking and growing alone don't spread further than σ.
        let mut scale = 2u32;
        while scale < 1 << 20 && ((2 * scale) as f64).powi(2) * pyramid_variance(2 * scale) <= sigma * sigma {
            scale *= 2;
        }
        let s = scale as f64;
        let variance = sigma * sigma / (s * s) - pyramid_variance(scale);
        Ok(Self { scale, kernel: small_kernel(variance)? })
    }
}

/// The full-resolution kernel: the Gaussian's mass over each step, out to the last step whose
/// tail (to 9.5) still holds 0.15%, the last tap or pair taking the rest of that tail.
fn full_kernel(sigma: f64) -> Option<Kernel> {
    let cdf = |x: f64| phi(x / sigma);
    let mass: Vec<f64> = (0..10).map(|k| cdf(k as f64 + 0.5) - cdf(k as f64 - 0.5)).collect();
    let tail = |k: usize| cdf(9.5) - cdf(k as f64 - 0.5);
    let r = (1..10).filter(|&k| tail(k) >= 0.0015).max()?;
    // There is no 17-tap kernel; 19 taps with the last pair read at 8.
    let n = if 2 * r + 1 == 17 { 19 } else { 2 * r + 1 };
    if n == 3 {
        // `_gaussianBlur3`: two samples at ±o, each weighing a half.
        return Some(Kernel::Pairs(vec![((2.0 * (1.0 - cdf(0.5))) as f32, 0.5)]));
    }
    if matches!(n, 5 | 9 | 13) {
        let mut w: Vec<f64> = mass[..r].to_vec();
        let rest = (1.0 - (w[0] + 2.0 * w[1..].iter().sum::<f64>())) / 2.0;
        w.push(rest);
        return Some(direct(w, n == 9));
    }
    let mut pairs = vec![center_pair(mass[0], mass[1])];
    for j in (2..=n / 2).step_by(2) {
        pairs.push(if n == 19 && j == 8 {
            (8.0, mass[8] + mass[9])
        } else if j + 1 < r {
            split(j, mass[j], mass[j + 1])
        } else {
            let w = tail(j);
            (j as f64 + (cdf(9.5) - cdf(j as f64 + 0.5)) / w, w)
        });
    }
    Some(Kernel::Pairs(pairs.into_iter().map(|(o, w)| (o as f32, w as f32)).collect()))
}

/// The kernel in the shrunk image, where Core Image samples the Gaussian's density at each step
/// instead of integrating it, out to the last step whose tail holds 0.3%.
fn small_kernel(variance: f64) -> Result<Option<Kernel>, String> {
    let density: Vec<f64> =
        (0..10).map(|k| (-(k * k) as f64 / (2.0 * variance)).exp() / (2.0 * std::f64::consts::PI * variance).sqrt()).collect();
    let r = (1..10).filter(|&k| density[k..].iter().sum::<f64>() >= 0.003).max();
    let Some(r) = r else {
        // Too narrow for a tap; Core Image then either skips the blur or runs `_gaussianBlur3`
        // with an offset whose formula isn't known.
        return if variance < 0.038 { Ok(None) } else { Err(three_tap(variance)) };
    };
    if r == 1 {
        return Err(three_tap(variance));
    }
    let n = 2 * r + 1;
    if matches!(n, 5 | 9 | 13) {
        let mut w: Vec<f64> = density[..r].to_vec();
        let rest = (1.0 - (w[0] + 2.0 * w[1..].iter().sum::<f64>())) / 2.0;
        w.push(rest);
        return Ok(Some(direct(w, n == 9)));
    }
    let mut pairs = vec![center_pair(density[0], density[1])];
    for j in (2..=r).step_by(2) {
        pairs.push(if j + 1 < r {
            split(j, density[j], density[j + 1])
        } else {
            // The last pair takes whatever the kernel still lacks.
            let w = 0.5 - density[0] / 2.0 - density[1..j].iter().sum::<f64>();
            (j as f64 + (w - density[j]) / w, w)
        });
    }
    Ok(Some(Kernel::Pairs(pairs.into_iter().map(|(o, w)| (o as f32, w as f32)).collect())))
}

fn three_tap(variance: f64) -> String {
    format!("Core Image's 3-tap motion blur after shrinking (σ² {variance:.3}), whose offset isn't known")
}

/// The sample at ±o covering half the center tap and tap 1.
fn center_pair(w0: f64, w1: f64) -> (f64, f64) {
    let w = w0 / 2.0 + w1;
    (w1 / w, w)
}

/// One bilinear sample covering taps j and j + 1.
fn split(j: usize, a: f64, b: f64) -> (f64, f64) {
    (j as f64 + b / (a + b), a + b)
}

/// Taps on texel centers. `_gaussianBlur9` works its last weight out itself, in floats.
fn direct(w: Vec<f64>, nine: bool) -> Kernel {
    let mut w: Vec<f32> = w.iter().map(|&v| v as f32).collect();
    if nine {
        let last = (0.5f32 - (w[1] + w[2] + w[3])) + w[0] * -0.5;
        w[4] = last;
    }
    Kernel::Direct(w)
}

/// `_gaussianReduce2` and `_gaussianReduce4`: the center weight, the pairs' weights, then the
/// pairs' offsets, as compiled into the kernels.
const REDUCE2: [f32; 5] = [0.432_290_82, 0.240_616_46, 0.043_238_133, 1.846_239_1, 3.745_180_6];
const REDUCE4: [f32; 9] = [
    0.249_105_66,
    0.204_995_26,
    0.114_229_73,
    0.043_355_204,
    0.012_866_975,
    1.950_196_6,
    3.901_370_3,
    5.858_400_8,
    7.868_862_6,
];

/// A float plane: one `vec4<f32>` per texel, row by row.
struct Plane {
    buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    /// The column index 0 stands for, in the plane's own pixels.
    origin: i32,
}

/// Where a pass reads: the 8-bit image, or a plane and the columns it holds.
enum Source<'a> {
    Image,
    Plane(&'a Plane, Edge),
}

/// Past a plane's columns: transparent, or the nearest held column (a texture that covers only
/// the region Core Image asked for, clamped by the GPU).
#[derive(Clone, Copy)]
enum Edge {
    Clear,
    Clamp(i32, i32),
}

#[derive(Default, Clone, Copy)]
struct Params {
    op: u32,
    out_w: u32,
    out_h: u32,
    src_w: u32,
    src_h: u32,
    src_kind: u32,
    out_kind: u32,
    level: u32,
    src_origin: i32,
    out_origin: i32,
    count: u32,
    lo: i32,
    hi: i32,
    cos: f32,
    sin: f32,
    x0: i32,
    y0: i32,
    image_h: u32,
    tx: f32,
    ty: f32,
}

impl Params {
    fn words(&self) -> [u32; 24] {
        [
            f32::INFINITY.to_bits(),
            self.op,
            self.out_w,
            self.out_h,
            self.src_w,
            self.src_h,
            self.src_kind,
            self.out_kind,
            self.level,
            self.src_origin as u32,
            self.out_origin as u32,
            self.count,
            self.lo as u32,
            self.hi as u32,
            self.cos.to_bits(),
            self.sin.to_bits(),
            self.x0 as u32,
            self.y0 as u32,
            self.image_h,
            self.tx.to_bits(),
            self.ty.to_bits(),
            0,
            0,
            0,
        ]
    }
}

const OP_TURN: u32 = 0;
const OP_REDUCE: u32 = 1;
const OP_DIRECT: u32 = 2;
const OP_PAIRS: u32 = 3;
const OP_UPSAMPLE: u32 = 4;
const OP_TURN_BACK: u32 = 5;
const OP_CONV3X3: u32 = 6;
const OUT_HALF: u32 = 0;
const OUT_BYTES: u32 = 1;

struct Pass<'a> {
    gpu: &'a Gpu,
    image: &'a GpuImage,
}

/// Out: a plane, or 8-bit pixels.
enum Target<'a> {
    Plane(&'a Plane),
    Image(&'a GpuImage),
}

fn floor_div(a: i32, b: i32) -> i32 {
    a.div_euclid(b)
}

fn ceil_div(a: i32, b: i32) -> i32 {
    -(-a).div_euclid(b)
}

impl Pass<'_> {
    fn plane(&self, width: u32, height: u32, origin: i32) -> Plane {
        let buffer = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("blur plane"),
            size: (width as u64 * height as u64 * 16).max(16),
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        Plane { buffer, width, height, origin }
    }

    fn dispatch(&self, mut p: Params, source: &Source, target: &Target, weights: &[f32]) {
        let gpu = self.gpu;
        // Unused bindings still need a buffer each: one read, one written.
        let dummy = gpu.bytes(&[0u8; 16]);
        let spare = self.plane(1, 1, 0).buffer;
        let (src8, srcf) = match source {
            Source::Image => {
                p.src_kind = 0;
                p.src_w = self.image.width;
                p.src_h = self.image.height;
                (&self.image.buffer, &dummy)
            }
            Source::Plane(plane, edge) => {
                p.src_kind = 1;
                p.src_w = plane.width;
                p.src_h = plane.height;
                p.src_origin = plane.origin;
                (p.lo, p.hi) = match *edge {
                    Edge::Clear => (i32::MIN / 2, i32::MAX / 2),
                    Edge::Clamp(lo, hi) => (lo, hi),
                };
                (&dummy, &plane.buffer)
            }
        };
        let (dstf, dst8, w, h) = match target {
            Target::Plane(plane) => {
                p.out_kind = OUT_HALF;
                (&plane.buffer, &spare, plane.width, plane.height)
            }
            Target::Image(image) => {
                p.out_kind = OUT_BYTES;
                (&spare, &image.buffer, image.width, image.height)
            }
        };
        (p.out_w, p.out_h) = (w, h);
        let weights = gpu.bytes(bytemuck::cast_slice(if weights.is_empty() { &[0f32] } else { weights }));
        let pipeline = gpu.pipeline("adjust.blur", &format!("{FLOAT}\n{}", include_str!("blur.wgsl")));
        gpu.dispatch(&pipeline, bytemuck::cast_slice(&p.words()), &[src8, srcf, dstf, dst8, &weights], w, h);
    }

    /// Shrinks `source` (origin `origin`, `n` columns) by `level` along its rows.
    fn reduce(&self, source: &Source, origin: i32, n: u32, rows: u32, level: u32) -> Plane {
        let l = level as i32;
        let first = floor_div(origin, l) - 3;
        let last = ceil_div(n as i32 + origin, l) + 3;
        let out = self.plane((last - first) as u32, rows, first);
        let weights: &[f32] = if level == 2 { &REDUCE2 } else { &REDUCE4 };
        let p = Params { op: OP_REDUCE, level, out_origin: first, count: (weights.len() / 2) as u32, ..Default::default() };
        let p = Params { src_origin: origin, ..p };
        self.dispatch(p, source, &Target::Plane(&out), weights);
        out
    }

    /// The row kernel from `source` into `target`, whose column 0 is `out_origin`.
    fn kernel(&self, kernel: &Kernel, source: &Source, src_origin: i32, target: &Target, out_origin: i32) {
        let (op, weights, count) = match kernel {
            Kernel::Direct(w) => (OP_DIRECT, w.clone(), w.len() as u32),
            Kernel::Pairs(pairs) => (OP_PAIRS, pairs.iter().flat_map(|&(o, w)| [o, w]).collect(), pairs.len() as u32),
        };
        let p = Params { op, count, src_origin, out_origin, ..Default::default() };
        self.dispatch(p, source, target, &weights);
    }

    /// Everything but the turns: rows of `source` (origin `origin`, `n` columns) blurred into
    /// `target` (columns from `out0`).
    fn row_blur(&self, plan: &MotionPlan, source: Source, origin: i32, n: u32, rows: u32, target: &Target, out0: i32, out_w: u32) {
        if plan.scale == 1 {
            let kernel = plan.kernel.as_ref().expect("a full-resolution kernel");
            return self.kernel(kernel, &source, origin, target, out0);
        }
        // The first shrink reads the source; the later ones and the blur read the previous plane,
        // which is transparent past its columns.
        let mut plane = self.reduce(&source, origin, n, rows, levels(plan.scale)[0]);
        for &level in &levels(plan.scale)[1..] {
            let next = self.reduce(&Source::Plane(&plane, Edge::Clear), plane.origin, plane.width, rows, level);
            plane = next;
        }
        if let Some(kernel) = &plan.kernel {
            let blurred = self.plane(plane.width, rows, plane.origin);
            self.kernel(kernel, &Source::Plane(&plane, Edge::Clear), plane.origin, &Target::Plane(&blurred), plane.origin);
            plane = blurred;
        }
        // Core Image renders the small image only over [⌊x0/S⌋ - 2, ⌈x1/S⌉ + 1), one texel short of
        // what the B-spline reads at the right end; the GPU clamps to the last one.
        let s = plan.scale as i32;
        let lo = floor_div(out0, s) - 2 - plane.origin;
        let hi = ceil_div(out0 + out_w as i32, s) + 1 - plane.origin;
        let p = Params { op: OP_UPSAMPLE, level: plan.scale, out_origin: out0, ..Default::default() };
        self.dispatch(p, &Source::Plane(&plane, Edge::Clamp(lo, hi)), target, &[]);
    }

    /// A horizontal streak: the image's own rows.
    fn rows(&self, plan: &MotionPlan) -> GpuImage {
        let (w, h) = (self.image.width, self.image.height);
        if plan.scale == 1 && plan.kernel.is_none() {
            return super::copy(self.gpu, self.image);
        }
        let out = self.gpu.image(w, h);
        self.row_blur(plan, Source::Image, 0, w, h, &Target::Image(&out), 0, w);
        out
    }

    /// Any other streak: turned by -angle so it runs along the rows, blurred, turned back.
    fn turned(&self, plan: &MotionPlan, cos: f64, sin: f64) -> GpuImage {
        let (w, h) = (self.image.width as f64, self.image.height as f64);
        // The image's corners in the turned frame (Core Image's y up).
        let corners = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)];
        let xs = corners.map(|(x, y)| cos * x + sin * y);
        let ys = corners.map(|(x, y)| -sin * x + cos * y);
        let min = |v: [f64; 4]| v.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = |v: [f64; 4]| v.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        // The turned-back image asks for its bounding box; the turned image is rendered one column
        // further each way for the row blur.
        let (x0, x1) = (min(xs).floor() as i32, max(xs).ceil() as i32);
        let (y0, y1) = (min(ys).floor() as i32, max(ys).ceil() as i32);
        let rows = (y1 - y0) as u32;
        let turned = self.plane((x1 - x0 + 2) as u32, rows, x0 - 1);
        let p = Params { op: OP_TURN, cos: cos as f32, sin: sin as f32, x0: x0 - 1, y0, image_h: self.image.height, ..Default::default() };
        self.dispatch(p, &Source::Image, &Target::Plane(&turned), &[]);
        let blurred = self.plane((x1 - x0) as u32, rows, x0);
        let source = Source::Plane(&turned, Edge::Clamp(0, turned.width as i32));
        self.row_blur(plan, source, turned.origin, turned.width, rows, &Target::Plane(&blurred), x0, blurred.width);
        let out = self.gpu.image(self.image.width, self.image.height);
        // Core Image's inverse of the turn back, with the image's flip folded in.
        let (tx, ty) = ((sin * h) as f32, (cos * h) as f32);
        let p = Params { op: OP_TURN_BACK, cos: cos as f32, sin: sin as f32, x0, y0, image_h: self.image.height, tx, ty, ..Default::default() };
        self.dispatch(p, &Source::Plane(&blurred, Edge::Clamp(0, blurred.width as i32)), &Target::Image(&out), &[]);
        out
    }
}

/// Φ(x), the standard normal distribution, to near double precision.
pub(super) fn phi(x: f64) -> f64 {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The parameters Core Image printed (`CI_PRINT_TREE`) for these streak lengths.
    #[test]
    fn motion_plans_match_core_image() {
        let plan = |d: f64| MotionPlan::new(d / 12f64.sqrt()).unwrap();
        let pairs = |p: &MotionPlan| match p.kernel.as_ref().unwrap() {
            Kernel::Pairs(v) => v.clone(),
            Kernel::Direct(_) => panic!("expected pairs"),
        };
        let taps = |p: &MotionPlan| match p.kernel.as_ref().unwrap() {
            Kernel::Direct(v) => v.clone(),
            Kernel::Pairs(_) => panic!("expected taps"),
        };
        let close = |a: f32, b: f32| assert!((a - b).abs() < 5e-6, "{a} vs {b}");
        let p = plan(10.0);
        assert_eq!(p.scale, 1);
        for (got, want) in pairs(&p).iter().zip([(0.653338, 0.198334), (2.42629, 0.188993), (4.36943, 0.0842994), (6.31598, 0.023686), (8.0, 0.00418804)]) {
            close(got.0, want.0);
            close(got.1, want.1);
        }
        let p = plan(20.0);
        assert_eq!(p.scale, 4);
        for (got, want) in pairs(&p).iter().zip([(0.559884, 0.431138), (2.09794, 0.0688621)]) {
            close(got.0, want.0);
            close(got.1, want.1);
        }
        let p = plan(40.0);
        assert_eq!(p.scale, 8);
        for (got, want) in taps(&p).iter().zip([0.444762, 0.238911, 0.0387081]) {
            close(*got, want);
        }
        assert_eq!(plan(400.0).scale, 64);
        assert_eq!(plan(2.0).scale, 1);
        close(taps(&plan(2.0))[0], 0.613524);
    }
}
