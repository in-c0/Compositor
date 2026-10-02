//! A filter inside the selection: `DocumentSelection.clip(canvas:)` rasterizes the selection over
//! the part of the canvas it can reach (a `SelectionClip`), `PixelAdjust.coverage` draws that
//! through the layer's placement onto the layer's own pixel grid, and `PixelAdjust.blend` mixes
//! the filtered pixels back over the original through it with `CIBlendWithMask`.

use super::{Result, failed, unsupported};
use crate::gpu::Gpu;
use crate::select::{self, Selection, geom};
use comp_format::Transform;

/// `SelectionClip`: coverage bytes for `rect` (whole document pixels: x, y, width, height), or
/// `None` for an empty selection, which clips everything away.
pub struct SelectionClip {
    pub rect: [i64; 4],
    pub coverage: Option<Vec<u8>>,
}

/// A `CGAffineTransform` as [a, b, c, d, tx, ty].
pub type Affine = [f64; 6];

/// `CGPath.boundingBoxOfPath` as [x0, y0, x1, y1]: the outline's own extent, a curve's extremes
/// rather than its control points. `None` when there's no outline.
pub fn bounding_box(region: &geom::Region) -> Option<[f64; 4]> {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    let mut add = |p: geom::Point| {
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    };
    for contour in region {
        for seg in contour {
            match *seg {
                geom::Seg::Line(a, e) => {
                    add(a);
                    add(e);
                }
                geom::Seg::Cubic(p) => {
                    add(p[0]);
                    add(p[3]);
                    for axis in 0..2 {
                        for t in cubic_extrema(p[0][axis], p[1][axis], p[2][axis], p[3][axis]) {
                            let u = 1.0 - t;
                            let at = |i: usize| u * u * u * p[0][i] + 3.0 * u * u * t * p[1][i] + 3.0 * u * t * t * p[2][i] + t * t * t * p[3][i];
                            add([at(0), at(1)]);
                        }
                    }
                }
            }
        }
    }
    (b[0] <= b[2] && b[1] <= b[3]).then_some(b)
}

/// Parameters in (0, 1) where one coordinate of a cubic turns.
fn cubic_extrema(p0: f64, p1: f64, p2: f64, p3: f64) -> Vec<f64> {
    // The derivative, divided by 3: a t² + b t + c.
    let a = -p0 + 3.0 * p1 - 3.0 * p2 + p3;
    let b = 2.0 * (p0 - 2.0 * p1 + p2);
    let c = p1 - p0;
    let mut roots = Vec::new();
    if a.abs() < 1e-12 {
        if b.abs() > 1e-12 {
            roots.push(-c / b);
        }
    } else {
        let disc = b * b - 4.0 * a * c;
        if disc >= 0.0 {
            let s = disc.sqrt();
            roots.push((-b + s) / (2.0 * a));
            roots.push((-b - s) / (2.0 * a));
        }
    }
    roots.retain(|t| *t > 0.0 && *t < 1.0);
    roots
}

/// `DocumentSelection.clip(canvas:)`: the selection's coverage over its reach (the outline's
/// bounds, twice the feather further, one more pixel, in whole pixels, on the canvas), drawn as
/// `DocumentSelection.coverage` draws it with the outline moved to the region's corner.
pub fn clip(gpu: &Gpu, selection: &Selection, (width, height): (u32, u32)) -> Result<SelectionClip> {
    let none = SelectionClip { rect: [0; 4], coverage: None };
    if selection.is_empty() {
        return Ok(none);
    }
    let Some(b) = bounding_box(&selection.region) else { return Ok(none) };
    let reach = (selection.feather * 2.0).ceil() + 1.0;
    // `insetBy(dx: -reach...)`, `integral`, then the intersection with the canvas.
    let (x0, y0) = ((b[0] - reach).floor().max(0.0), (b[1] - reach).floor().max(0.0));
    let (x1, y1) = ((b[2] + reach).ceil().min(width as f64), (b[3] + reach).ceil().min(height as f64));
    if x1 - x0 < 1.0 || y1 - y0 < 1.0 {
        return Ok(none);
    }
    let local = Selection { region: geom::transformed(&selection.region, [1.0, 0.0, 0.0, 1.0, -x0, -y0]), ..selection.clone() };
    let (w, h) = ((x1 - x0) as u32, (y1 - y0) as u32);
    let coverage = select::coverage(gpu, &local, w, h)?;
    Ok(SelectionClip { rect: [x0 as i64, y0 as i64, w as i64, h as i64], coverage: Some(coverage) })
}

/// `BrushRaster.pixelToDocument`: the layer's pixel grid onto the canvas, composed as
/// `CGAffineTransform(translationX:y:).rotated(by:).scaledBy(x:y:).translatedBy(x:y:)` does.
pub fn pixel_to_document(t: &Transform, width: u32, height: u32) -> Affine {
    let radians = (t.rotation % 360.0) * std::f64::consts::PI / 180.0;
    let (sin, cos) = radians.sin_cos();
    let sx = t.size[0] / width as f64 * if t.flip_x { -1.0 } else { 1.0 };
    let sy = t.size[1] / height as f64 * if t.flip_y { -1.0 } else { 1.0 };
    let (cx, cy) = (t.origin[0] + t.size[0] / 2.0, t.origin[1] + t.size[1] / 2.0);
    let (a, b, c, d) = (cos * sx, sin * sx, -sin * sy, cos * sy);
    let (hx, hy) = (-(width as f64) / 2.0, -(height as f64) / 2.0);
    [a, b, c, d, a * hx + c * hy + cx, b * hx + d * hy + cy]
}

/// `CGAffineTransformInvert`.
pub fn inverted(m: Affine) -> Affine {
    let det = m[0] * m[3] - m[1] * m[2];
    [m[3] / det, -m[1] / det, -m[2] / det, m[0] / det, (m[2] * m[5] - m[3] * m[4]) / det, (m[1] * m[4] - m[0] * m[5]) / det]
}

/// `CGRect.applying(_:)` as [x0, y0, x1, y1]: the bounds of the rectangle's mapped corners.
pub fn rect_applying(r: [f64; 4], m: Affine) -> [f64; 4] {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for (x, y) in [(r[0], r[1]), (r[2], r[1]), (r[0], r[3]), (r[2], r[3])] {
        let (px, py) = (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]);
        b = [b[0].min(px), b[1].min(py), b[2].max(px), b[3].max(py)];
    }
    b
}

/// The mapping takes each layer pixel onto exactly one canvas pixel: no rotation or scale (a flip
/// is fine), whole-pixel offsets. Then the coverage the Mac draws through the clip is a copy of
/// the clip's own bytes.
fn aligned(m: &Affine) -> bool {
    m[1] == 0.0 && m[2] == 0.0 && m[0].abs() == 1.0 && m[3].abs() == 1.0 && m[4].fract() == 0.0 && m[5].fract() == 0.0
}

/// `PixelAdjust.coverage`: the clip drawn onto a `width` x `height` layer grid placed by
/// `mapping` (layer pixels to the document), one byte per layer pixel. `ContentFill.run` draws
/// its mask the same way, filling the layer's rectangle rather than the clip's, which through the
/// clip is the same.
pub fn layer_coverage(clip: &SelectionClip, mapping: &Affine, (width, height): (u32, u32)) -> Result<Vec<u8>> {
    let mut out = vec![0u8; width as usize * height as usize];
    let Some(coverage) = &clip.coverage else { return Ok(out) };
    if !aligned(mapping) {
        // Where each layer pixel spans more than two canvas pixels, the Mac's clip reaches
        // further than the sampler: on a layer drawn at three times its size, one canvas pixel
        // further on each side (`content-fill-probe-third`). How isn't measured yet.
        if mapping[0].hypot(mapping[1]) > 2.0 + 1e-9 || mapping[2].hypot(mapping[3]) > 2.0 + 1e-9 {
            return unsupported("a selection drawn onto a layer shown at more than twice its size");
        }
        return Ok(resampled(clip, coverage, mapping, (width, height)));
    }
    let [rx, ry, rw, rh] = clip.rect;
    for y in 0..height {
        for x in 0..width {
            // The canvas pixel whose center this pixel's center lands on.
            let cx = mapping[0] * (x as f64 + 0.5) + mapping[4];
            let cy = mapping[3] * (y as f64 + 0.5) + mapping[5];
            let (dx, dy) = (cx.floor() as i64 - rx, cy.floor() as i64 - ry);
            if dx >= 0 && dy >= 0 && dx < rw && dy < rh {
                out[(y * width + x) as usize] = coverage[(dy * rw + dx) as usize];
            }
        }
    }
    Ok(out)
}

/// The image `PixelAdjust.blend` mixes the filtered pixels over, as Core Image reads it.
pub enum Original<'a> {
    /// The layer's own image as the project loaded it: straight RGBA, which Core Image
    /// premultiplies in `f32` without rounding to bytes first.
    Straight(&'a [u8]),
    /// A grid the panel drew itself (`FilterEdit.grow`): premultiplied bytes.
    Premultiplied(&'a [u8]),
}

impl Original<'_> {
    /// Channel `c` of pixel `i`, premultiplied, in 0...1.
    fn at(&self, i: usize, c: usize) -> f32 {
        match self {
            Original::Premultiplied(p) => p[i * 4 + c] as f32 / 255.0,
            Original::Straight(p) => {
                let v = p[i * 4 + c] as f32 / 255.0;
                if c == 3 { v } else { v * (p[i * 4 + 3] as f32 / 255.0) }
            }
        }
    }
}

/// `PixelAdjust.blend`'s `CIBlendWithMask` on premultiplied RGBA: the original plus the
/// difference times the mask, in `f32` on bytes / 255 as Core Image reads them, rounded to bytes.
pub fn blend(adjusted: &[u8], original: Original, coverage: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; adjusted.len()];
    for (i, &m) in coverage.iter().enumerate() {
        let m = m as f32 / 255.0;
        for c in 0..4 {
            let (a, o) = (adjusted[i * 4 + c] as f32 / 255.0, original.at(i, c));
            let v = o + (a - o) * m;
            out[i * 4 + c] = (v * 255.0 + 0.5).floor().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// The layer's pixels as one premultiplied RGBA grid.
pub fn premultiplied(pixels: &image::RgbaImage) -> Vec<u8> {
    let mut out = pixels.as_raw().clone();
    for p in out.chunks_exact_mut(4) {
        let a = p[3] as u32;
        for c in 0..3 {
            p[c] = ((p[c] as u32 * a + 127) / 255) as u8;
        }
    }
    out
}

/// `Failure.noSource`'s message.
pub const NO_SOURCE: &str =
    "Not enough unselected, opaque image pixels to synthesize a fill. Use a smaller selection with some surrounding image.";

pub fn no_source() -> crate::RenderError {
    failed(format!("Content-Aware Fill: {NO_SOURCE}"))
}

/// The clip drawn onto a layer grid that isn't aligned with the canvas: Core Graphics resamples
/// the clip's coverage through the inverse placement as the transform module samples a mask
/// (`transform::clip::sample`, low quality), times the coverage of the clip's rectangle.
///
/// Where a pixel's center lands exactly halfway between two coverage pixels (a layer scaled by 2
/// or placed on a half pixel), the Mac takes the one nearer the middle of the clip as the heavy
/// one on both sides, as if positions were measured from the middle and rounded toward it. The
/// port gets the same ties by shrinking every offset from the middle by a factor of 2^-30, far
/// below anything else the sampler can resolve.
fn resampled(clip: &SelectionClip, coverage: &[u8], mapping: &Affine, (width, height): (u32, u32)) -> Vec<u8> {
    use crate::transform::{Placement, clip as tclip};
    let inv = inverted(*mapping);
    let [rx, ry, rw, rh] = clip.rect.map(|v| v as f64);
    let (cx, cy) = (rx + rw / 2.0, ry + rh / 2.0);
    let middle = [inv[0] * cx + inv[2] * cy + inv[4], inv[1] * cx + inv[3] * cy + inv[5]];
    let (sx, sy) = (inv[0].hypot(inv[1]), inv[2].hypot(inv[3]));
    let flipped = inv[0] * inv[3] - inv[1] * inv[2] < 0.0;
    let toward_middle = 1.0 + (-30f64).exp2();
    let size = [rw * sx * toward_middle, rh * sy * toward_middle];
    let placed = Transform {
        origin: [middle[0] - size[0] / 2.0, middle[1] - size[1] / 2.0],
        size,
        rotation: inv[1].atan2(inv[0]).to_degrees(),
        flip_x: flipped,
        flip_y: false,
        sampling: comp_format::Sampling::High,
    };
    let placement = Placement::of(&placed);
    let rect = placement.rect(1.0, 1.0);
    let mask = image::GrayImage::from_raw(rw as u32, rh as u32, coverage.to_vec()).expect("clip size");
    tclip::sample(&mask, &placement, &rect, tclip::Filter::Low, true, (width, height)).into_iter().map(|(m, c)| (m * c / 255) as u8).collect()
}
