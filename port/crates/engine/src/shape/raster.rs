//! Core Graphics' fills, on the GPU.
//!
//! Fitted to references: Core Graphics flattens each cubic by halving it until both second
//! differences of its control points are within `FLATNESS` on each axis; coverage is the exact
//! area of the flattened outline in each pixel, `min(255, floor(area * 256))`; and the color is
//! premultiplied with truncation. A stroke's round caps are two quarter circles each, turned with
//! the line (a zero-length line's with a line at 45 degrees).

use super::Result;
use super::gradient::Fill;
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
    let byte = |c: f64| (c.clamp(0.0, 1.0) * 255.0 + 0.5).floor() as u32;
    let color = byte(style.red) | byte(style.green) << 8 | byte(style.blue) << 16 | 255 << 24;
    let flat: Vec<f32> = points.iter().flat_map(|p| [p[0] as f32, p[1] as f32]).collect();
    let pipeline = gpu.pipeline("shape_fill", include_str!("fill.wgsl"));
    let out = gpu.image(width, height);
    let params = [width, height, points.len() as u32, color].map(u32::to_le_bytes).concat();
    gpu.dispatch(&pipeline, &params, &[&gpu.bytes(bytemuck::cast_slice(&flat)), &out.buffer], width, height);
    let mut pixels = gpu.download(&out)?;
    super::unpremultiply(&mut pixels);
    Ok(image::RgbaImage::from_raw(width, height, pixels).expect("image size"))
}

/// `fill` drawn over `base` (premultiplied, `width` x `height`) inside `region`, whose top-left
/// pixel sits at `offset` on the document.
pub fn gradient_over(_gpu: &Gpu, _base: &[u8], _width: u32, _height: u32, _region: [i64; 4], _offset: [f64; 2], _fill: &Fill) -> Result<Vec<u8>> {
    Err(RenderError::Unsupported("drawing gradients".into()))
}
