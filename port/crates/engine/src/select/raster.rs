//! Filling a selection's outline the way `DocumentSelection.coverage` does: Core Graphics fills
//! the path with the winding rule into an 8-bit gray bitmap, anti-aliased or not.

use super::{Result, Selection};
use crate::gpu::Gpu;

/// Coverage bytes for `selection` over a `width` x `height` canvas.
pub fn coverage(gpu: &Gpu, selection: &Selection, width: u32, height: u32) -> Result<Vec<u8>> {
    // A feathered selection always fills anti-aliased.
    let hard = fill(gpu, selection, selection.antialiased || selection.feather > 0.0, width, height)?;
    if selection.feather <= 0.0 {
        return Ok(hard);
    }
    Ok(feather(gpu, &hard, width, height, selection.feather / 2.0))
}

/// `clampedToExtent().applyingGaussianBlur(sigma:)`, cropped back to the canvas: the edge pixels
/// repeat outward, so the blur doesn't fade in from the canvas's edges.
fn feather(gpu: &Gpu, hard: &[u8], width: u32, height: u32, sigma: f64) -> Vec<u8> {
    let pad = (3.0 * sigma).floor() as u32 + 1;
    let (pw, ph) = (width + 2 * pad, height + 2 * pad);
    let mut padded = Vec::with_capacity((pw * ph * 4) as usize);
    for y in 0..ph {
        let sy = (y as i64 - pad as i64).clamp(0, height as i64 - 1) as u32;
        for x in 0..pw {
            let sx = (x as i64 - pad as i64).clamp(0, width as i64 - 1) as u32;
            let v = hard[(sy * width + sx) as usize];
            padded.extend_from_slice(&[v, v, v, 255]);
        }
    }
    let image = gpu.upload(pw, ph, &padded);
    let blurred = crate::adjust::blur::gaussian(gpu, &image, sigma);
    let bytes = gpu.download(&blurred).expect("readback");
    let mut out = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            out.push(bytes[(((y + pad) * pw + x + pad) * 4) as usize]);
        }
    }
    out
}

fn fill(gpu: &Gpu, selection: &Selection, antialias: bool, width: u32, height: u32) -> Result<Vec<u8>> {
    let edges = edges(selection, height);
    // Each pixel row lists the edges that reach into it.
    let mut rows: Vec<Vec<u32>> = vec![Vec::new(); height as usize];
    for (i, e) in edges.iter().enumerate() {
        let (y0, y1) = (e[1].min(e[3]), e[1].max(e[3]));
        if y1 <= 0.0 || y0 >= height as f32 || y0 == y1 {
            continue;
        }
        let first = y0.floor().max(0.0) as usize;
        let last = (y1.ceil() as usize).min(height as usize);
        for row in &mut rows[first..last] {
            row.push(i as u32);
        }
    }
    let mut offsets = Vec::with_capacity(height as usize + 1);
    let mut index = Vec::new();
    for row in &rows {
        offsets.push(index.len() as u32);
        index.extend_from_slice(row);
    }
    offsets.push(index.len() as u32);
    let pipeline = gpu.pipeline("select.coverage", include_str!("coverage.wgsl"));
    let out = gpu.image(width, height);
    let params = [width, height, antialias as u32, 0];
    let edge_buffer = gpu.bytes(bytemuck::cast_slice(&edges));
    let offsets = gpu.bytes(bytemuck::cast_slice(&offsets));
    let index = gpu.bytes(bytemuck::cast_slice(&index));
    gpu.dispatch(&pipeline, bytemuck::cast_slice(&params), &[&edge_buffer, &offsets, &index, &out.buffer], width, height);
    let words = gpu.download(&out)?;
    Ok(words.chunks_exact(4).map(|w| w[0]).collect())
}

/// Every outline edge as [x0, y0, x1, y1] in document pixels, as Core Graphics steps it:
/// horizontal ones left out (they cover nothing).
fn edges(selection: &Selection, height: u32) -> Vec<[f32; 4]> {
    let mut out = Vec::new();
    for contour in &selection.region {
        for i in 0..contour.len() {
            let (a, b) = (contour[i], contour[(i + 1) % contour.len()]);
            for [x0, y0, x1, y1] in stepped(a, b, height as f64) {
                if y0 != y1 {
                    out.push([x0 as f32, y0 as f32, x1 as f32, y1 as f32]);
                }
            }
        }
    }
    if out.is_empty() {
        out.push([0.0; 4]);
    }
    out
}

/// Sub-steps per pixel along an edge's major axis.
const STEPS: f64 = 16.0;

/// An edge as the Mac's rasterizer walks it, fitted to its coverage. In device space (y up, the
/// bitmap's own), an edge is stepped from its lower end along its major axis in 1/16 px steps,
/// the minor coordinate advancing by a 16.16 fixed-point step truncated toward zero. Every
/// stepped point lies on one line, so the edge becomes at most three segments: from its start to
/// the first 1/16 step, the stepped line, and from the last step to its true end.
fn stepped(a: [f64; 2], b: [f64; 2], height: f64) -> Vec<[f64; 4]> {
    // Device space.
    let (mut p0, mut p1) = ([a[0], height - a[1]], [b[0], height - b[1]]);
    let (dx, dy) = (p1[0] - p0[0], p1[1] - p0[1]);
    let doc = |p: [f64; 2], q: [f64; 2]| [p[0], height - p[1], q[0], height - q[1]];
    if dx == 0.0 || dy == 0.0 {
        return vec![doc(p0, p1)];
    }
    let reversed = p0[1] > p1[1];
    if reversed {
        std::mem::swap(&mut p0, &mut p1);
    }
    // Major axis `u`, minor `v`.
    let y_major = dx.abs() <= dy.abs();
    let (u, v) = if y_major { (1, 0) } else { (0, 1) };
    let point = |uu: f64, vv: f64| if y_major { [vv, uu] } else { [uu, vv] };
    let (u0, v0, u1, v1) = (p0[u], p0[v], p1[u], p1[v]);
    let slope = (v1 - v0) / (u1 - u0);
    let step = if u1 > u0 { 1.0 } else { -1.0 };
    let dq = (slope * step / STEPS * 65536.0).trunc() / 65536.0;
    let n0 = if step > 0.0 { (u0 * STEPS).ceil() } else { (u0 * STEPS).floor() };
    let n1 = if step > 0.0 { (u1 * STEPS).floor() } else { (u1 * STEPS).ceil() };
    let count = (n1 - n0) * step;
    let mut points = vec![p0];
    if count >= 0.0 {
        let (us, vs) = (n0 / STEPS, v0 + (n0 / STEPS - u0) * slope);
        points.push(point(us, vs));
        points.push(point(n1 / STEPS, vs + count * dq));
    }
    points.push(p1);
    points.dedup();
    if reversed {
        points.reverse();
    }
    points.windows(2).map(|w| doc(w[0], w[1])).collect()
}
