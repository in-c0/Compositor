//! Core Graphics' fills, on the GPU.
//!
//! Fitted to references: Core Graphics flattens each cubic by halving it until both second
//! differences of its control points are within `FLATNESS` on each axis; walks each edge in
//! 1/16 px steps along its major axis with a truncated 16.16 minor step (`stepped`, the model
//! feature/selections fitted to its fills); covers each pixel by the exact area of those edges
//! inside it, `min(255, floor(area * 256))`; and premultiplies the color with truncation. A stroke's round caps are two quarter circles each, turned with
//! the line (a zero-length line's with a line at 45 degrees).

use super::Result;
use super::gradient::{Fill, Shape};
use crate::RenderError;
use crate::gpu::Gpu;
use comp_format::{ShapeKind, ShapeStyle};

/// The largest second difference of a flat enough curve piece, on either axis, in pixels.
const FLATNESS: f64 = 1.0 / 15.0;
/// `4/3 * tan(pi/8)`: the control-point distance of a quarter circle of radius 1.
const KAPPA: f64 = 0.552_284_749_830_793_4;

type Point = [f64; 2];
type Cubic = [Point; 4];

fn mid(a: Point, b: Point) -> Point {
    [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]
}

/// Appends the flattened `curve`, without its first point.
fn flatten(curve: Cubic, tolerance: f64, depth: u32, out: &mut Vec<Point>) {
    let [p0, p1, p2, p3] = curve;
    let second = |a: Point, b: Point, c: Point| (a[0] - 2.0 * b[0] + c[0]).abs().max((a[1] - 2.0 * b[1] + c[1]).abs());
    if depth >= 16 || second(p0, p1, p2).max(second(p1, p2, p3)) <= tolerance {
        out.push(p3);
        return;
    }
    let (a, b, c) = (mid(p0, p1), mid(p1, p2), mid(p2, p3));
    let (d, e) = (mid(a, b), mid(b, c));
    let f = mid(d, e);
    flatten([p0, a, d, f], tolerance, depth + 1, out);
    flatten([f, e, c, p3], tolerance, depth + 1, out);
}

/// A quarter of the circle around `center`, from angle `from` a quarter turn on.
fn quarter(center: Point, radius: f64, from: f64) -> Cubic {
    let to = from + std::f64::consts::FRAC_PI_2;
    let (s0, c0, s1, c1) = (from.sin(), from.cos(), to.sin(), to.cos());
    let k = KAPPA * radius;
    let p0 = [center[0] + radius * c0, center[1] + radius * s0];
    let p3 = [center[0] + radius * c1, center[1] + radius * s1];
    [p0, [p0[0] - k * s0, p0[1] + k * c0], [p3[0] + k * s1, p3[1] - k * c1], p3]
}

/// `CGPath(ellipseIn:)`, flattened.
fn ellipse(w: f64, h: f64) -> Vec<Point> {
    let (rx, ry, cx, cy) = (w / 2.0, h / 2.0, w / 2.0, h / 2.0);
    let (kx, ky) = (KAPPA * rx, KAPPA * ry);
    let quarters = [
        [[cx + rx, cy], [cx + rx, cy + ky], [cx + kx, cy + ry], [cx, cy + ry]],
        [[cx, cy + ry], [cx - kx, cy + ry], [cx - rx, cy + ky], [cx - rx, cy]],
        [[cx - rx, cy], [cx - rx, cy - ky], [cx - kx, cy - ry], [cx, cy - ry]],
        [[cx, cy - ry], [cx + kx, cy - ry], [cx + rx, cy - ky], [cx + rx, cy]],
    ];
    let mut out = Vec::new();
    for q in quarters {
        flatten(q, FLATNESS, 0, &mut out);
    }
    out
}

/// `CGPath(roundedRect:)` (or `CGPath(rect:)` without a radius), flattened.
fn rounded_rect(w: f64, h: f64, radius: f64) -> Vec<Point> {
    let r = radius.max(0.0).min(w / 2.0).min(h / 2.0);
    if r <= 0.0 {
        return vec![[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]];
    }
    let k = KAPPA * r;
    let corners = [
        [[w - r, 0.0], [w - r + k, 0.0], [w, r - k], [w, r]],
        [[w, h - r], [w, h - r + k], [w - r + k, h], [w - r, h]],
        [[r, h], [r - k, h], [0.0, h - r + k], [0.0, h - r]],
        [[0.0, r], [0.0, r - k], [r - k, 0.0], [r, 0.0]],
    ];
    let mut out = Vec::new();
    for c in corners {
        out.push(c[0]);
        flatten(c, FLATNESS, 0, &mut out);
    }
    out
}

/// A line from `a` to `b` stroked `width` wide with round caps: its outline, flattened.
fn capsule(a: Point, b: Point, width: f64) -> Vec<Point> {
    let hw = width / 2.0;
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let angle = if dx == 0.0 && dy == 0.0 { std::f64::consts::FRAC_PI_4 } else { dy.atan2(dx) };
    let half = std::f64::consts::FRAC_PI_2;
    let mut out = Vec::new();
    for (center, start) in [(b, angle - half), (a, angle + half)] {
        out.push([center[0] + hw * start.cos(), center[1] + hw * start.sin()]);
        for q in 0..2 {
            flatten(quarter(center, hw, start + q as f64 * half), FLATNESS, 0, &mut out);
        }
    }
    out
}

/// The outline `EditorSession.shapeImage` fills or strokes for `style` in a box of `size`.
fn outline(style: &ShapeStyle, size: [f64; 2]) -> Result<Vec<Point>> {
    Ok(match style.kind {
        ShapeKind::Ellipse => ellipse(size[0], size[1]),
        ShapeKind::Rectangle => rounded_rect(size[0], size[1], style.corner_radius),
        ShapeKind::Line => {
            let thickness = style.line_width.unwrap_or(0.0).max(1.0);
            if thickness <= 1.0 {
                return Err(RenderError::Unsupported("1 px lines, which Core Graphics strokes differently".into()));
            }
            // The ends as fractions of the box; lines saved before they kept their ends ran corner
            // to corner, inset by half their thickness.
            let inset = [thickness.min(size[0]) / 2.0, thickness.min(size[1]) / 2.0];
            let from = style.start.map_or(inset, |s| [s[0] * size[0], s[1] * size[1]]);
            let to = style.end.map_or([size[0] - inset[0], size[1] - inset[1]], |e| [e[0] * size[0], e[1] * size[1]]);
            capsule(from, to, thickness)
        }
    })
}

/// The shape filling a box of `size` (its pixels are the whole part of it), as
/// `EditorSession.shapeImage` draws it: straight RGBA, as the Mac saves it.
pub fn shape_image(gpu: &Gpu, style: &ShapeStyle, size: [f64; 2]) -> Result<image::RgbaImage> {
    let (width, height) = (size[0] as u32, size[1] as u32);
    let points = outline(style, size)?;
    let mut edges: Vec<f32> = Vec::new();
    for i in 0..points.len() {
        for e in stepped(points[i], points[(i + 1) % points.len()], size[1]) {
            if e[1] != e[3] {
                edges.extend(e.map(|v| v as f32));
            }
        }
    }
    if edges.is_empty() {
        edges.extend([0.0; 4]);
    }
    let byte = |c: f64| (c.clamp(0.0, 1.0) * 255.0 + 0.5).floor() as u32;
    let color = byte(style.red) | byte(style.green) << 8 | byte(style.blue) << 16 | 255 << 24;
    let pipeline = gpu.pipeline("shape_fill", include_str!("fill.wgsl"));
    let out = gpu.image(width, height);
    let params = [width, height, (edges.len() / 4) as u32, color].map(u32::to_le_bytes).concat();
    gpu.dispatch(&pipeline, &params, &[&gpu.bytes(bytemuck::cast_slice(&edges)), &out.buffer], width, height);
    let mut pixels = gpu.download(&out)?;
    super::unpremultiply(&mut pixels);
    Ok(image::RgbaImage::from_raw(width, height, pixels).expect("image size"))
}

/// Sub-steps per pixel along an edge's major axis.
const STEPS: f64 = 16.0;

/// The edge from `a` to `b` (pixels, y down, in a bitmap `height` tall) as Core Graphics' rasterizer
/// walks it: in device space (y up) from its lower end, along its major axis in 1/16 px steps, the
/// minor coordinate advancing by a 16.16 fixed-point step truncated toward zero. The stepped
/// points lie on one line, so the edge becomes at most three pieces: from its start to the first
/// step, the stepped line, and from the last step to its true end.
fn stepped(a: Point, b: Point, height: f64) -> Vec<[f64; 4]> {
    let (mut p0, mut p1) = ([a[0], height - a[1]], [b[0], height - b[1]]);
    let (dx, dy) = (p1[0] - p0[0], p1[1] - p0[1]);
    let doc = |p: Point, q: Point| [p[0], height - p[1], q[0], height - q[1]];
    if dx == 0.0 || dy == 0.0 {
        return vec![doc(p0, p1)];
    }
    let reversed = p0[1] > p1[1];
    if reversed {
        std::mem::swap(&mut p0, &mut p1);
    }
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
        let vs = v0 + (n0 / STEPS - u0) * slope;
        points.push(point(n0 / STEPS, vs));
        points.push(point(n1 / STEPS, vs + count * dq));
    }
    points.push(p1);
    points.dedup();
    if reversed {
        points.reverse();
    }
    points.windows(2).map(|w| doc(w[0], w[1])).collect()
}

/// How many colors Core Graphics' gradient table holds for a line (or radius) this long: the
/// length rounded up to whole pixels, then up past the next multiple of 16, less two.
fn slots(length: f64) -> f64 {
    16.0 * ((length.ceil() / 16.0).floor() + 1.0) - 2.0
}

/// `v` truncated to 14 significant bits, as Core Graphics keeps a linear gradient's direction and a
/// radial one's slots per pixel: fitted to the slot the Mac picks where t × slots falls just past
/// a whole number.
fn coarse(v: f64) -> f64 {
    if v == 0.0 || !v.is_finite() {
        return v;
    }
    let scale = 2f64.powi(13 - v.abs().log2().floor() as i32);
    (v * scale).trunc() / scale
}

/// Core Graphics' 16 x 16 gradient dither thresholds, in 256ths, row by row.
const DITHER: [u32; 256] = [
    244, 188, 16, 150, 107, 199, 156, 243, 118, 176, 46, 154, 202, 7, 136, 216,
    158, 64, 132, 249, 217, 72, 27, 135, 3, 98, 237, 110, 38, 180, 104, 78,
    42, 206, 92, 51, 5, 184, 112, 229, 205, 53, 144, 220, 84, 254, 196, 28,
    114, 172, 226, 121, 164, 79, 61, 171, 31, 191, 74, 22, 168, 60, 148, 234,
    139, 13, 99, 197, 21, 142, 253, 125, 88, 159, 248, 128, 210, 120, 11, 90,
    69, 247, 36, 151, 238, 45, 209, 8, 231, 101, 17, 65, 177, 33, 224, 185,
    201, 161, 213, 57, 85, 179, 109, 187, 41, 149, 198, 93, 239, 106, 155, 47,
    19, 75, 117, 1, 134, 223, 67, 137, 77, 215, 49, 165, 4, 81, 251, 131,
    97, 175, 235, 189, 105, 25, 167, 241, 12, 119, 227, 113, 143, 193, 55, 218,
    29, 145, 62, 37, 203, 255, 52, 153, 32, 181, 59, 23, 207, 39, 126, 182,
    89, 240, 122, 160, 83, 129, 211, 95, 195, 133, 87, 245, 170, 71, 232, 15,
    204, 44, 221, 10, 183, 30, 73, 111, 233, 0, 157, 103, 9, 138, 108, 152,
    166, 102, 68, 115, 246, 147, 225, 173, 43, 63, 200, 222, 76, 190, 50, 252,
    2, 194, 140, 208, 58, 6, 100, 18, 141, 250, 123, 26, 162, 34, 214, 86,
    54, 230, 24, 82, 169, 127, 192, 219, 163, 80, 186, 94, 242, 116, 174, 146,
    124, 96, 178, 40, 236, 48, 91, 35, 66, 212, 14, 130, 56, 228, 70, 20,
];

/// `fill` drawn over `base` (premultiplied, `width` x `height`) inside `region` (x, y, width,
/// height in the grid), whose top-left pixel sits at document pixel `offset`. Returns the
/// premultiplied result.
pub fn gradient_over(gpu: &Gpu, base: &[u8], width: u32, height: u32, region: [i64; 4], offset: [i64; 2], fill: &Fill) -> Result<Vec<u8>> {
    let (dx, dy) = (fill.end[0] - fill.start[0], fill.end[1] - fill.start[1]);
    let length = dx.hypot(dy);
    let slots = slots(length);
    let (kind, base_point, step) = match fill.shape {
        // The distance from the center, times slots / radius kept to 14 significant bits.
        Shape::Radial => (1u32, [fill.start[0] - offset[0] as f64, fill.start[1] - offset[1] as f64], [coarse(slots / length), 0.0]),
        Shape::Linear => {
            // t is the distance along the line's unit direction, whose components Core Graphics
            // keeps to 14 significant bits, truncated; divided by the true length.
            let (c, s) = (coarse(dx / length), coarse(dy / length));
            let (px, py) = (offset[0] as f64 + 0.5 - fill.start[0], offset[1] as f64 + 0.5 - fill.start[1]);
            let k = slots / length;
            (0, [(px * c + py * s) * k, 0.0], [c * k, s * k])
        }
    };
    let premultiplied = |c: [f64; 4]| [c[0] * c[3] * 255.0, c[1] * c[3] * 255.0, c[2] * c[3] * 255.0, c[3] * 255.0];
    let alpha = (fill.opacity.clamp(0.0, 1.0) * 255.0 + 0.5).floor() as u32;
    let mut params: Vec<u8> = Vec::with_capacity(96);
    for v in [width, height, kind, alpha] {
        params.extend_from_slice(&v.to_le_bytes());
    }
    for v in [region[0], region[1], region[2], region[3], offset[0], offset[1]] {
        params.extend_from_slice(&(v as i32).to_le_bytes());
    }
    let floats = [base_point[0], base_point[1], step[0], step[1], slots, 0.0];
    for v in floats.iter().chain(premultiplied(fill.colors[0]).iter()).chain(premultiplied(fill.colors[1]).iter()) {
        params.extend_from_slice(&(*v as f32).to_le_bytes());
    }
    let pipeline = gpu.pipeline("shape_gradient", include_str!("gradient.wgsl"));
    let layer = gpu.upload(width, height, base);
    let out = gpu.image(width, height);
    let dither = gpu.bytes(bytemuck::cast_slice(&DITHER));
    gpu.dispatch(&pipeline, &params, &[&dither, &layer.buffer, &out.buffer], width, height);
    Ok(gpu.download(&out)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradient_tables_follow_the_length() {
        for (length, expected) in [(16.0, 30.0), (24.0, 30.0), (40.0, 46.0), (64.0, 78.0), (73.41, 78.0), (79.2, 94.0), (256.0, 270.0), (600.0, 606.0)] {
            assert_eq!(slots(length), expected, "length {length}");
        }
    }

    #[test]
    fn curves_split_where_they_bend_most() {
        // Measured against references: a 5 px circle's quarters split in six (the ends finer),
        // a 4 px circle's in four, a 6 px circle's in eight.
        assert_eq!(ellipse(5.0, 5.0).len(), 24);
        assert_eq!(ellipse(4.0, 4.0).len(), 16);
        assert_eq!(ellipse(6.0, 6.0).len(), 32);
        // Square corners stay four points.
        assert_eq!(rounded_rect(9.0, 7.0, 0.0).len(), 4);
    }

    #[test]
    fn directions_keep_fourteen_bits() {
        assert_eq!(coarse(1.0), 1.0);
        assert_eq!(coarse(0.95448), 15638.0 / 16384.0);
        assert_eq!(coarse(-0.29828), -9774.0 / 32768.0);
        assert_eq!(coarse(0.0), 0.0);
    }

    #[test]
    fn edges_step_from_their_lower_end() {
        // A level edge is left as it is; a slanted one gains the stepped line between its ends.
        assert_eq!(stepped([0.0, 1.0], [5.0, 1.0], 4.0), vec![[0.0, 1.0, 5.0, 1.0]]);
        let pieces = stepped([0.0, 0.0], [60.0, 18.0], 20.0);
        assert_eq!(pieces.first().unwrap()[..2], [0.0, 0.0]);
        assert_eq!(pieces.last().unwrap()[2..], [60.0, 18.0]);
        // The truncated step leaves the far end short of the true line by up to 16 * 60 / 65536.
        let far = pieces[0];
        assert!((far[3] - 0.3 * far[2]).abs() < 0.015);
    }
}
