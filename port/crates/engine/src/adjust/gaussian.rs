//! `CIGaussianBlur`, as Core Image runs it on the Mac's GPU for `PixelFilter`'s Gaussian Blur.
//!
//! Core Image picks a path by σ (measured with `CI_PRINT_TREE` and by tracing the Metal passes
//! it encodes):
//! - σ < 0.16: nothing.
//! - σ ≤ 0.4: `_conv3x3sym`, four bilinear reads at (±p, ±p) averaged, p = 2(1 − Φ(½/σ)).
//! - σ ≤ 1.12: `CIConvolutionProcessor`, Metal Performance Shaders' separable convolution (rows,
//!   then columns) of the image padded by ⌈3σ⌉, with half-float weights and arithmetic.
//! - above: `CIBlurProcessor`, which hands `MPSImageGaussianBlur` the image padded by ⌈3σ⌉ in
//!   half floats. Metal Performance Shaders shrinks it by 2, 4 or 8 at a time (`DlFn` passes,
//!   columns then rows), blurs the small image (`Fn`), and grows it back three texels at a time
//!   (`UlP3F3`, rows then columns). Its kernels are reproduced in `gaussian.wgsl`; the weights come
//!   from a fit it runs on the CPU, which isn't known, so they are the ones it chose for
//!   σ = k/20, k = 20…5000, logged on the Mac (`mps_gaussian.bin`). Other σ are unsupported.

use super::FLOAT;
use crate::gpu::{Gpu, GpuImage};
use std::sync::OnceLock;

/// One of Metal Performance Shaders' passes, applied along columns and then rows (or, for the
/// last, rows then columns).
#[derive(Debug, Clone)]
enum MpsPass {
    Down(u32, Vec<f32>),
    Filter(Vec<f32>),
    Up(u32, f32),
}

struct Table {
    first: u32,
    ids: Vec<u16>,
    sets: Vec<Vec<MpsPass>>,
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        let b: &[u8] = include_bytes!("mps_gaussian.bin");
        let u32_at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        assert_eq!(&b[..4], b"MPSG");
        let (first, count, nsets) = (u32_at(8), u32_at(12) as usize, u32_at(16) as usize);
        let ids_at = 20;
        let offs_at = ids_at + 2 * count;
        let blobs_at = offs_at + 4 * nsets;
        let ids = (0..count).map(|i| u16::from_le_bytes([b[ids_at + 2 * i], b[ids_at + 2 * i + 1]])).collect();
        let sets = (0..nsets)
            .map(|s| {
                let mut p = blobs_at + u32_at(offs_at + 4 * s) as usize;
                let n = b[p];
                p += 1;
                (0..n)
                    .map(|_| {
                        let (kind, factor, len) = (b[p], 1u32 << b[p + 1], b[p + 2] as usize);
                        p += 3;
                        let w: Vec<f32> = (0..len).map(|i| f32::from_le_bytes(b[p + 4 * i..p + 4 * i + 4].try_into().unwrap())).collect();
                        p += 4 * len;
                        match kind {
                            0 => MpsPass::Down(factor, w),
                            1 => MpsPass::Filter(w),
                            _ => MpsPass::Up(factor, w[0]),
                        }
                    })
                    .collect()
            })
            .collect();
        Table { first, ids, sets }
    })
}

/// Metal Performance Shaders' passes for σ, if it's one the table holds.
fn mps_passes(sigma: f64) -> Option<&'static [MpsPass]> {
    let t = table();
    let s = sigma as f32;
    let k = (sigma * 20.0).round();
    if !(k >= t.first as f64 && k < (t.first as usize + t.ids.len()) as f64) || ((k / 20.0) as f32) != s {
        return None;
    }
    Some(&t.sets[t.ids[k as usize - t.first as usize] as usize])
}

/// Why `gaussian` can't reproduce σ, if it can't.
pub fn unsupported(sigma: f64) -> Option<String> {
    if sigma < 0.16 || sigma <= 0.4 {
        return None;
    }
    if sigma <= 1.12 {
        return None;
    }
    if mps_passes(sigma).is_none() {
        return Some(format!("Gaussian σ {sigma} (Metal Performance Shaders' weights are known for multiples of 0.05)"));
    }
    None
}

/// `CIImage.applyingGaussianBlur(sigma:)` on the premultiplied 8-bit `image`, cropped to it.
pub fn gaussian(gpu: &Gpu, image: &GpuImage, sigma: f64) -> Result<GpuImage, String> {
    if let Some(why) = unsupported(sigma) {
        return Err(why);
    }
    if sigma < 0.16 {
        return Ok(super::copy(gpu, image));
    }
    if sigma <= 0.4 {
        return Ok(super::blur::conv3x3(gpu, image, sigma));
    }
    let pad = (3.0 * sigma).ceil() as u32;
    if sigma <= 1.12 {
        return Ok(Mps { gpu }.convolve(image, &convolution_weights(sigma), pad));
    }
    let passes = mps_passes(sigma).expect("checked above");
    Ok(Mps { gpu }.run(image, passes, pad))
}

/// `CIConvolutionProcessor`'s kernel: the Gaussian's mass over each step out to 2 (σ ≤ 0.7) or
/// 3, the tail shared evenly among the taps, as floats and then halfs.
fn convolution_weights(sigma: f64) -> Vec<f32> {
    let r = if sigma <= 0.7 { 2 } else { 3 };
    let cdf = |x: f64| super::blur::phi(x / sigma);
    let mass: Vec<f64> = (0..=r).map(|k| cdf(k as f64 + 0.5) - cdf(k as f64 - 0.5)).collect();
    let total = mass[0] + 2.0 * mass[1..].iter().sum::<f64>();
    let share = (1.0 - total) / (2 * r + 1) as f64;
    (-(r as i32)..=r as i32).map(|k| to_half((mass[k.unsigned_abs() as usize] + share) as f32)).collect()
}

/// A float rounded to the nearest half, ties to even (normal halfs only, as the weights are).
fn to_half(x: f32) -> f32 {
    let bits = x.to_bits();
    let rest = bits & 0x1fff;
    let mut down = bits & !0x1fff;
    if rest > 0x1000 || (rest == 0x1000 && down & 0x2000 != 0) {
        down += 0x2000;
    }
    f32::from_bits(down)
}

/// A float plane, and the pixel its index 0 stands for along each axis, in its own level.
struct Plane {
    buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    x0: i32,
    y0: i32,
}

#[derive(Default)]
struct Params {
    op: u32,
    axis: u32,
    out_w: u32,
    out_h: u32,
    src_w: u32,
    src_h: u32,
    count: u32,
    level: u32,
    src_origin: i32,
    out_origin: i32,
    a: f32,
}

const OP_DOWN: u32 = 0;
const OP_FILTER: u32 = 1;
const OP_UP: u32 = 2;
const OP_LOAD: u32 = 3;
const OP_STORE: u32 = 4;
const OP_CONVOLVE: u32 = 5;
const COLUMNS: u32 = 0;
const ROWS: u32 = 1;

struct Mps<'a> {
    gpu: &'a Gpu,
}

impl Mps<'_> {
    fn plane(&self, width: u32, height: u32, x0: i32, y0: i32) -> Plane {
        let buffer = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gaussian plane"),
            size: (width as u64 * height as u64 * 16).max(16),
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        Plane { buffer, width, height, x0, y0 }
    }

    fn dispatch(&self, p: Params, src: &wgpu::Buffer, dst: &wgpu::Buffer, src8: &wgpu::Buffer, dst8: &wgpu::Buffer, weights: &[f32]) {
        let gpu = self.gpu;
        let words = [
            f32::INFINITY.to_bits(),
            p.op,
            p.axis,
            p.out_w,
            p.out_h,
            p.src_w,
            p.src_h,
            p.count,
            p.level,
            p.src_origin as u32,
            p.out_origin as u32,
            p.a.to_bits(),
        ];
        let weights = gpu.bytes(bytemuck::cast_slice(if weights.is_empty() { &[0f32] } else { weights }));
        let pipeline = gpu.pipeline("adjust.gaussian", &format!("{FLOAT}\n{}", include_str!("gaussian.wgsl")));
        gpu.dispatch(&pipeline, bytemuck::cast_slice(&words), &[src, dst, &weights, src8, dst8], p.out_w, p.out_h);
    }

    /// One pass along `axis` from `src` into a new plane with index 0 at `origin` along the axis.
    fn pass(&self, op: u32, axis: u32, src: &Plane, len: u32, origin: i32, level: u32, weights: &[f32], a: f32) -> Plane {
        let (w, h, x0, y0) = if axis == COLUMNS { (src.width, len, src.x0, origin) } else { (len, src.height, origin, src.y0) };
        let out = self.plane(w, h, x0, y0);
        let p = Params {
            op,
            axis,
            out_w: w,
            out_h: h,
            src_w: src.width,
            src_h: src.height,
            count: weights.len() as u32,
            level,
            src_origin: if axis == COLUMNS { src.y0 } else { src.x0 },
            out_origin: origin,
            a,
        };
        let spare = self.gpu.bytes(&[0u8; 16]);
        let spare_out = self.plane(1, 1, 0, 0).buffer;
        self.dispatch(p, &src.buffer, &out.buffer, &spare, &spare_out, weights);
        out
    }

    /// `CIConvolutionProcessor`: the padded image through `kNx1` (rows) then `k1xN` (columns).
    fn convolve(&self, image: &GpuImage, weights: &[f32], pad: u32) -> GpuImage {
        let (w, h) = (image.width, image.height);
        let src = self.load(image, pad);
        let rows = self.pass(OP_CONVOLVE, ROWS, &src, w, pad as i32, 1, weights, 0.0);
        let columns = self.pass(OP_CONVOLVE, COLUMNS, &rows, h, pad as i32, 1, weights, 0.0);
        self.store(&columns, w, h, 0)
    }

    /// The image padded by `pad` in half floats, as Core Image hands it to a processor.
    fn load(&self, image: &GpuImage, pad: u32) -> Plane {
        let (w, h) = (image.width, image.height);
        let plane = self.plane(w + 2 * pad, h + 2 * pad, 0, 0);
        let spare = self.plane(1, 1, 0, 0).buffer;
        let p = Params { op: OP_LOAD, out_w: plane.width, out_h: plane.height, src_w: w, src_h: h, count: pad, ..Default::default() };
        self.dispatch(p, &spare, &plane.buffer, &image.buffer, &self.gpu.bytes(&[0u8; 16]), &[]);
        plane
    }

    /// The plane's pixels from (`offset`, `offset`) as 8-bit, rounded.
    fn store(&self, plane: &Plane, w: u32, h: u32, offset: u32) -> GpuImage {
        let out = self.gpu.image(w, h);
        let p = Params { op: OP_STORE, out_w: w, out_h: h, src_w: plane.width, src_h: plane.height, count: offset, ..Default::default() };
        self.dispatch(p, &plane.buffer, &self.plane(1, 1, 0, 0).buffer, &self.gpu.bytes(&[0u8; 16]), &out.buffer, &[]);
        out
    }

    fn run(&self, image: &GpuImage, passes: &[MpsPass], pad: u32) -> GpuImage {
        let (w, h) = (image.width, image.height);
        // Index 0 is the padding's corner.
        let mut cur = self.load(image, pad);
        let mut scale = 1u32;
        let mut finished = false;
        for pass in passes {
            match pass {
                MpsPass::Down(l, weights) => {
                    // Each level's pixel j covers the previous level's [j·l, (j + 1)·l), counted from
                    // the padded corner; four spare pixels each side.
                    let lo = |n: i32, o: i32| (o.div_euclid(*l as i32) - 4, (n + o).div_euclid(*l as i32) + 5);
                    let (y_lo, y_hi) = lo(cur.height as i32, cur.y0);
                    cur = self.pass(OP_DOWN, COLUMNS, &cur, (y_hi - y_lo) as u32, y_lo, *l, weights, 0.0);
                    let (x_lo, x_hi) = lo(cur.width as i32, cur.x0);
                    cur = self.pass(OP_DOWN, ROWS, &cur, (x_hi - x_lo) as u32, x_lo, *l, weights, 0.0);
                    scale *= l;
                }
                MpsPass::Filter(weights) => {
                    cur = self.pass(OP_FILTER, COLUMNS, &cur, cur.height, cur.y0, 1, weights, 0.0);
                    cur = self.pass(OP_FILTER, ROWS, &cur, cur.width, cur.x0, 1, weights, 0.0);
                }
                MpsPass::Up(s, a) => {
                    debug_assert_eq!(*s, scale);
                    // Back to full resolution over the image itself: rows first, then columns.
                    cur = self.pass(OP_UP, ROWS, &cur, w, pad as i32, *s, &[], *a);
                    cur = self.pass(OP_UP, COLUMNS, &cur, h, pad as i32, *s, &[], *a);
                    finished = true;
                }
            }
        }
        // Without a pyramid the plane still has its padding.
        self.store(&cur, w, h, if finished { 0 } else { pad })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_holds_the_logged_parameters() {
        let passes = mps_passes(3.0).unwrap();
        assert!(matches!(&passes[0], MpsPass::Down(2, w) if w == &[0.283843935f32, 0.169462174, 0.0466938801]));
        assert!(matches!(&passes[2], MpsPass::Up(2, a) if *a == 0.733496487f32));
        assert!(mps_passes(3.01).is_none());
        assert_eq!(mps_passes(20.0).unwrap().len(), 4);
    }
}
