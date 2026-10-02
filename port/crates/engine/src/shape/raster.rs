//! Core Graphics' fills, on the GPU.
//!
//! Fitted to references: Core Graphics flattens each cubic as it flattens a selection's
//! (`select::geom::flatten`), and fills the outline with the same rasterizer it fills a selection
//! with: a shape's alpha is exactly the coverage the
//! Mac gives the same path as a selection (the `probe-select-ellipse-*` cases), so the outline
//! goes through `select::raster::coverage`. The color is premultiplied by that coverage with
//! truncation. A stroke's round caps are two quarter circles each, turned with
//! the line (a zero-length line's with a line at 45 degrees).

use super::Result;
use super::gradient::{Fill, Shape};
use crate::RenderError;
use crate::gpu::Gpu;
use crate::select::geom::{self, Contour, Seg};
use comp_format::{ShapeKind, ShapeStyle};

/// `4/3 * tan(pi/8)`: the control-point distance of a quarter circle of radius 1.
const KAPPA: f64 = 0.552_284_749_830_793_4;

type Point = [f64; 2];
type Cubic = [Point; 4];

/// A quarter of the circle around `center`, from angle `from` a quarter turn on.
fn quarter(center: Point, radius: f64, from: f64) -> Cubic {
    let to = from + std::f64::consts::FRAC_PI_2;
    let (s0, c0, s1, c1) = (from.sin(), from.cos(), to.sin(), to.cos());
    let k = KAPPA * radius;
    let p0 = [center[0] + radius * c0, center[1] + radius * s0];
    let p3 = [center[0] + radius * c1, center[1] + radius * s1];
    [p0, [p0[0] - k * s0, p0[1] + k * c0], [p3[0] + k * s1, p3[1] - k * c1], p3]
}

/// Lines and cubics joined end to start, closed with a line when the last ends elsewhere.
fn closed(pieces: Vec<Seg>) -> Contour {
    let mut out: Contour = Vec::new();
    for piece in pieces {
        if let Some(last) = out.last() {
            let end = end_of(last);
            if end != piece.start() {
                out.push(Seg::Line(end, piece.start()));
            }
        }
        out.push(piece);
    }
    if let (Some(first), Some(last)) = (out.first(), out.last()) {
        let (start, end) = (first.start(), end_of(last));
        if end != start {
            out.push(Seg::Line(end, start));
        }
    }
    out
}

fn end_of(seg: &Seg) -> Point {
    match seg {
        Seg::Line(_, b) => *b,
        Seg::Cubic(p) => p[3],
    }
}

/// `CGPath(ellipseIn:)`.
fn ellipse(w: f64, h: f64) -> Contour {
    geom::ellipse(0.0, 0.0, w, h)
}

/// `CGPath(roundedRect:)` (or `CGPath(rect:)` without a radius).
fn rounded_rect(w: f64, h: f64, radius: f64) -> Contour {
    let r = radius.max(0.0).min(w / 2.0).min(h / 2.0);
    if r <= 0.0 {
        return geom::polygon(&[[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]]);
    }
    let k = KAPPA * r;
    let corners = [
        [[w - r, 0.0], [w - r + k, 0.0], [w, r - k], [w, r]],
        [[w, h - r], [w, h - r + k], [w - r + k, h], [w - r, h]],
        [[r, h], [r - k, h], [0.0, h - r + k], [0.0, h - r]],
        [[0.0, r], [0.0, r - k], [r - k, 0.0], [r, 0.0]],
    ];
    closed(corners.into_iter().map(Seg::Cubic).collect())
}

/// A line from `a` to `b` stroked `width` wide with round caps: its outline.
fn capsule(a: Point, b: Point, width: f64) -> Contour {
    let hw = width / 2.0;
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let angle = if dx == 0.0 && dy == 0.0 { std::f64::consts::FRAC_PI_4 } else { dy.atan2(dx) };
    let half = std::f64::consts::FRAC_PI_2;
    let mut pieces = Vec::new();
    for (center, start) in [(b, angle - half), (a, angle + half)] {
        for q in 0..2 {
            pieces.push(Seg::Cubic(quarter(center, hw, start + q as f64 * half)));
        }
    }
    closed(pieces)
}

/// The outline `EditorSession.shapeImage` fills or strokes for `style` in a box of `size`.
fn outline(style: &ShapeStyle, size: [f64; 2]) -> Result<Contour> {
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
            // Level, upright and 45° lines match the Mac; along other slants its stroke's edges
            // come out up to a level (2 to 3 on a few pixels) off the port's.
            let (dx, dy) = ((to[0] - from[0]).abs(), (to[1] - from[1]).abs());
            if dx > 1e-9 && dy > 1e-9 && (dx - dy).abs() > 1e-9 * dx.max(dy) {
                return Err(RenderError::Unsupported("lines that aren't level, upright or at 45 degrees".into()));
            }
            capsule(from, to, thickness)
        }
    })
}

/// The shape filling a box of `size` (its pixels are the whole part of it), as
/// `EditorSession.shapeImage` draws it: straight RGBA, as the Mac saves it.
pub fn shape_image(gpu: &Gpu, style: &ShapeStyle, size: [f64; 2]) -> Result<image::RgbaImage> {
    let (width, height) = (size[0] as u32, size[1] as u32);
    // Measured: the Mac's coverage is one level off the port's on about 2% of an ellipse's edge
    // pixels, and where that lands on a faint pixel the saved color moves by more.
    if style.kind == ShapeKind::Ellipse {
        return Err(RenderError::Unsupported("ellipses, whose edge coverage is one level off on a few pixels".into()));
    }
    // The outline filled as the Mac fills a selection's path: the same rasterizer.
    let outline = crate::select::Selection { region: vec![outline(style, size)?], antialiased: true, feather: 0.0 };
    let coverage = crate::select::raster::coverage(gpu, &outline, width, height)?;
    let words: Vec<u32> = coverage.iter().map(|&c| c as u32).collect();
    let byte = |c: f64| (c.clamp(0.0, 1.0) * 255.0 + 0.5).floor() as u32;
    let color = byte(style.red) | byte(style.green) << 8 | byte(style.blue) << 16 | 255 << 24;
    let pipeline = gpu.pipeline("shape_tint", include_str!("tint.wgsl"));
    let out = gpu.image(width, height);
    let params = [width, height, color, 0].map(u32::to_le_bytes).concat();
    gpu.dispatch(&pipeline, &params, &[&gpu.bytes(bytemuck::cast_slice(&words)), &out.buffer], width, height);
    let mut pixels = gpu.download(&out)?;
    super::unpremultiply(&mut pixels);
    Ok(image::RgbaImage::from_raw(width, height, pixels).expect("image size"))
}

/// How many colors Core Graphics' gradient table holds for a line (or radius) this long: the
/// length rounded up to whole pixels, then up past the next multiple of 16, less two.
fn slots(length: f64) -> f64 {
    16.0 * ((length.ceil() / 16.0).floor() + 1.0) - 2.0
}

/// Whether a linear gradient puts any canvas pixel's t × slots within 1e-4 of a whole number
/// (but not on it): there the Mac and the port's model of its precision can pick different slots.
/// Measured: every pixel the port got wrong lay within 7.4e-5; none past 4e-4.
pub fn near_slot_boundary(fill: &Fill, width: i64, height: i64) -> bool {
    if fill.shape != Shape::Linear {
        return false;
    }
    let (dx, dy) = (fill.end[0] - fill.start[0], fill.end[1] - fill.start[1]);
    let length = dx.hypot(dy);
    let slots = slots(length);
    let (c, s) = (coarse(dx / length), coarse(dy / length));
    let k = slots / length;
    (0..height).any(|y| {
        (0..width).any(|x| {
            let u = ((x as f64 + 0.5 - fill.start[0]) * c + (y as f64 + 0.5 - fill.start[1]) * s) * k;
            let d = (u - u.round()).abs();
            u > 0.0 && u < slots && d > 0.0 && d < 1e-4
        })
    })
}

/// `v` truncated to 14 significant bits, as Core Graphics keeps a linear gradient's direction:
/// fitted to the slot the Mac picks where t × slots falls just past a whole number.
fn coarse(v: f64) -> f64 {
    coarse_to(v, 14)
}

/// `v` truncated to `bits` significant bits.
fn coarse_to(v: f64, bits: i32) -> f64 {
    if v == 0.0 || !v.is_finite() {
        return v;
    }
    let scale = 2f64.powi(bits - 1 - v.abs().log2().floor() as i32);
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
        // The distance from the center times slots / radius (kept to 16 significant bits), the
        // product kept to 14 (in the shader).
        Shape::Radial => (1u32, [fill.start[0] - offset[0] as f64, fill.start[1] - offset[1] as f64], [coarse_to(slots / length, 16), 0.0]),
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
    let split = |v: f64| {
        let hi = v as f32;
        [hi, (v - hi as f64) as f32]
    };
    let (b0, b1, s0, s1) = (split(base_point[0]), split(base_point[1]), split(step[0]), split(step[1]));
    let floats = [slots as f32, f32::INFINITY, b0[0], b1[0], b0[1], b1[1], s0[0], s1[0], s0[1], s1[1]];
    let colors = premultiplied(fill.colors[0]).into_iter().chain(premultiplied(fill.colors[1])).map(|v| v as f32);
    for v in floats.into_iter().chain(colors) {
        params.extend_from_slice(&v.to_le_bytes());
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
        assert_eq!(geom::flatten(&ellipse(5.0, 5.0)).len(), 24);
        assert_eq!(geom::flatten(&ellipse(4.0, 4.0)).len(), 16);
        assert_eq!(geom::flatten(&ellipse(6.0, 6.0)).len(), 32);
        // Square corners stay four points.
        assert_eq!(geom::flatten(&rounded_rect(9.0, 7.0, 0.0)).len(), 4);
    }

    #[test]
    fn directions_keep_fourteen_bits() {
        assert_eq!(coarse(1.0), 1.0);
        assert_eq!(coarse(0.95448), 15638.0 / 16384.0);
        assert_eq!(coarse(-0.29828), -9774.0 / 32768.0);
        assert_eq!(coarse(0.0), 0.0);
    }

}
