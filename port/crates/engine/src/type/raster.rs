//! Glyph coverage as Core Graphics draws glyphs with font smoothing on: each glyph's outline,
//! unhinted, with its curves cut into straight pieces, grown outward a little on every side, and
//! filled with exact area coverage (the share of each pixel it covers).
//!
//! All of it was read off the Mac's references for a face installed on both machines (ArialMT),
//! where the port has the very same outlines; see the `probe-*` cases in the `type` corpus.

use super::fonts::Face;
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::prelude::Size;
use skrifa::{GlyphId, MetadataProvider};

/// A glyph's coverage over a pixel rectangle of the layer.
pub struct GlyphMask {
    pub left: i32,
    pub top: i32,
    pub width: usize,
    pub height: usize,
    /// 0...1 per pixel, row by row.
    pub coverage: Vec<f64>,
}

/// How far, in pixels along either axis, a straight piece may stray from its curve. Each curve is
/// halved until its pieces are within it, as the references show: a quadratic whose larger second
/// difference is `d` pixels becomes the fewest pieces `n`, a power of two, with `d / 4 / n²` at
/// most this.
const FLATNESS: f64 = 0.2;

/// The outline's contours as closed polygons, in layer pixels, y down.
struct Flattener {
    scale: f64,
    origin: (f64, f64),
    contours: Vec<Vec<(f64, f64)>>,
    last: (f64, f64),
}

impl Flattener {
    fn point(&self, x: f32, y: f32) -> (f64, f64) {
        (self.origin.0 + x as f64 * self.scale, self.origin.1 - y as f64 * self.scale)
    }

    fn push(&mut self, p: (f64, f64)) {
        if let Some(contour) = self.contours.last_mut() {
            if contour.last() != Some(&p) {
                contour.push(p);
            }
        }
        self.last = p;
    }

    /// The pieces for a curve that strays at most `deviation` from its chord on either axis.
    fn pieces(deviation: f64) -> usize {
        ((deviation / FLATNESS).sqrt().ceil() as usize).clamp(1, 256).next_power_of_two()
    }
}

impl OutlinePen for Flattener {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = self.point(x, y);
        self.contours.push(vec![p]);
        self.last = p;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.point(x, y);
        self.push(p);
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        let (p0, p1, p2) = (self.last, self.point(cx0, cy0), self.point(x, y));
        let second = (p0.0 - 2.0 * p1.0 + p2.0).abs().max((p0.1 - 2.0 * p1.1 + p2.1).abs());
        let n = Self::pieces(0.25 * second);
        for i in 1..=n {
            let t = i as f64 / n as f64;
            let u = 1.0 - t;
            let p = (u * u * p0.0 + 2.0 * u * t * p1.0 + t * t * p2.0, u * u * p0.1 + 2.0 * u * t * p1.1 + t * t * p2.1);
            self.push(if i == n { p2 } else { p });
        }
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let (p0, p1, p2, p3) = (self.last, self.point(cx0, cy0), self.point(cx1, cy1), self.point(x, y));
        let ax = (p0.0 - 2.0 * p1.0 + p2.0).abs().max((p1.0 - 2.0 * p2.0 + p3.0).abs());
        let ay = (p0.1 - 2.0 * p1.1 + p2.1).abs().max((p1.1 - 2.0 * p2.1 + p3.1).abs());
        let n = Self::pieces(0.75 * ax.max(ay));
        for i in 1..=n {
            let t = i as f64 / n as f64;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            let p = (a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0, a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1);
            self.push(if i == n { p3 } else { p });
        }
    }

    fn close(&mut self) {}
}

/// Grows a contour outward by `amount` on every side, as font smoothing does: each edge moves out
/// by a square's reach along its normal (`amount` times |nx| + |ny|), inward corners go where the
/// moved edges meet, and outward corners are cut straight across. `sign` says which side of the
/// edges the ink is on.
fn dilate(contour: &[(f64, f64)], amount: f64, sign: f64) -> Vec<(f64, f64)> {
    let mut points: Vec<(f64, f64)> = contour.to_vec();
    if points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
    let n = points.len();
    if n < 3 {
        return points;
    }
    let normal = |a: (f64, f64), b: (f64, f64)| {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = (dx * dx + dy * dy).sqrt().max(1e-12);
        (sign * dy / len, -sign * dx / len)
    };
    let reach = |n: (f64, f64)| amount * (n.0.abs() + n.1.abs());
    let mut out = Vec::with_capacity(n * 2);
    for i in 0..n {
        let (prev, p, next) = (points[(i + n - 1) % n], points[i], points[(i + 1) % n]);
        let (n0, n1) = (normal(prev, p), normal(p, next));
        let (h0, h1) = (reach(n0), reach(n1));
        let a = (p.0 + n0.0 * h0, p.1 + n0.1 * h0);
        let b = (p.0 + n1.0 * h1, p.1 + n1.1 * h1);
        let det = n0.0 * n1.1 - n0.1 * n1.0;
        if det.abs() < 1e-9 {
            out.push(a);
            continue;
        }
        // Where the two moved edges meet.
        let v = ((h0 * n1.1 - h1 * n0.1) / det, (n0.0 * h1 - n1.0 * h0) / det);
        let e0 = (p.0 - prev.0, p.1 - prev.1);
        let outward = (v.0 - n0.0 * h0) * e0.0 + (v.1 - n0.1 * h0) * e0.1 > 0.0;
        let limit = 16.0 * amount;
        if outward || v.0.abs() > limit || v.1.abs() > limit {
            out.push(a);
            out.push(b);
        } else {
            out.push((p.0 + v.0, p.1 + v.1));
        }
    }
    out
}

/// The coverage of glyph `glyph` of `face` at `size` pixels per em, its origin (on the baseline)
/// at `origin` in layer pixels, its outline grown by `growth` pixels.
pub fn glyph_mask(face: &Face, glyph: u32, size: f64, origin: (f64, f64), growth: f64) -> Option<GlyphMask> {
    let font = face.font();
    let location = font.axes().location(face.variations(size));
    let outlines = font.outline_glyphs();
    let outline = outlines.get(GlyphId::new(glyph))?;
    let mut pen = Flattener { scale: size / face.units_per_em, origin, contours: Vec::new(), last: origin };
    outline.draw(DrawSettings::unhinted(Size::unscaled(), &location), &mut pen).ok()?;
    // Which side of each edge the ink is on: the outline's winding, read from its largest contour.
    let area = |c: &Vec<(f64, f64)>| {
        (0..c.len())
            .map(|i| {
                let (a, b) = (c[i], c[(i + 1) % c.len()]);
                a.0 * b.1 - b.0 * a.1
            })
            .sum::<f64>()
    };
    let outer = pen.contours.iter().map(area).fold(0.0f64, |m, a| if a.abs() > m.abs() { a } else { m });
    let sign = if outer > 0.0 { 1.0 } else { -1.0 };
    let mut lines = Vec::new();
    for contour in &pen.contours {
        let contour = if growth > 0.0 { dilate(contour, growth, sign) } else { contour.clone() };
        for i in 0..contour.len() {
            lines.push([contour[i], contour[(i + 1) % contour.len()]]);
        }
    }
    if lines.is_empty() {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for [a, b] in &lines {
        for p in [a, b] {
            x0 = x0.min(p.0);
            y0 = y0.min(p.1);
            x1 = x1.max(p.0);
            y1 = y1.max(p.1);
        }
    }
    let (left, top) = (x0.floor() as i32, y0.floor() as i32);
    let width = (x1.ceil() as i32 - left).max(1) as usize + 2;
    let height = (y1.ceil() as i32 - top).max(1) as usize;
    let mut acc = vec![0.0f64; width * height + 2];
    for [a, b] in &lines {
        accumulate(&mut acc, width, height, (a.0 - left as f64, a.1 - top as f64), (b.0 - left as f64, b.1 - top as f64));
    }
    let mut coverage = Vec::with_capacity(width * height);
    for row in 0..height {
        // A closed outline's signed areas sum to nothing across a row, so each row starts from zero.
        let mut sum = 0.0;
        for x in 0..width {
            sum += acc[row * width + x];
            coverage.push(sum.abs().min(1.0));
        }
    }
    Some(GlyphMask { left, top, width, height, coverage })
}

/// Adds the signed area a line from `p0` to `p1` sweeps to the right of itself, per pixel
/// (font-rs's accumulation rasterizer, in double precision).
fn accumulate(acc: &mut [f64], width: usize, height: usize, p0: (f64, f64), p1: (f64, f64)) {
    if p0.1 == p1.1 {
        return;
    }
    let (dir, p0, p1) = if p0.1 < p1.1 { (1.0, p0, p1) } else { (-1.0, p1, p0) };
    let dxdy = (p1.0 - p0.0) / (p1.1 - p0.1);
    let mut x = p0.0;
    let y_start = p0.1.max(0.0).floor() as usize;
    if p0.1 < 0.0 {
        x -= p0.1 * dxdy;
    }
    let y_end = (p1.1.ceil().max(0.0) as usize).min(height);
    for y in y_start..y_end {
        let row = y * width;
        let dy = ((y + 1) as f64).min(p1.1) - (y as f64).max(p0.1);
        let xnext = x + dxdy * dy;
        let d = dy * dir;
        let (xa, xb) = if x < xnext { (x, xnext) } else { (xnext, x) };
        let xa_floor = xa.floor();
        let xa_i = xa_floor as isize;
        let xb_ceil = xb.ceil();
        let xb_i = xb_ceil as isize;
        let last = acc.len() as isize - 1;
        let at = |i: isize| (row as isize + i).clamp(0, last) as usize;
        if xb_i <= xa_i + 1 {
            let xmf = 0.5 * (x + xnext) - xa_floor;
            acc[at(xa_i)] += d - d * xmf;
            acc[at(xa_i + 1)] += d * xmf;
        } else {
            let s = 1.0 / (xb - xa);
            let xaf = xa - xa_floor;
            let a0 = 0.5 * s * (1.0 - xaf) * (1.0 - xaf);
            let xbf = xb - xb_ceil + 1.0;
            let am = 0.5 * s * xbf * xbf;
            acc[at(xa_i)] += d * a0;
            if xb_i == xa_i + 2 {
                acc[at(xa_i + 1)] += d * (1.0 - a0 - am);
            } else {
                let a1 = s * (1.5 - xaf);
                acc[at(xa_i + 1)] += d * (a1 - a0);
                for xi in xa_i + 2..xb_i - 1 {
                    acc[at(xi)] += d * s;
                }
                let a2 = a1 + (xb_i - xa_i - 3) as f64 * s;
                acc[at(xb_i - 1)] += d * (1.0 - a2 - am);
            }
            acc[at(xb_i)] += d * am;
        }
        x = xnext;
    }
}
