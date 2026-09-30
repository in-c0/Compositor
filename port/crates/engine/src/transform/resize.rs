//! Image > Image Size: `ImageResizer.resize`, then `EditorSession.applyImageSize`.
//!
//! Every layer is re-rasterized on its own into the upright box its corners land in on the
//! resized document, with its rotation and flips baked in: `LayerRenderer.draw` of its image into
//! a context scaled by the resize (so uneven scaling of a rotated layer shears it), and
//! `LayerRenderer.drawCoverage` of its mask. The layer then sits in that box, unrotated, with the
//! sampling picked in the sheet. Guides and unlinked mask placements scale with the document;
//! like every document size change, layers lose their live shape, text and effects.

use super::clip::{self, Filter};
use super::{Placement, Quality};
use crate::RenderError;
use crate::gpu::Gpu;
use comp_format::{Asset, GuideAxis, Project, Sampling, Transform};
use image::{GrayImage, RgbaImage};

type Result<T> = std::result::Result<T, RenderError>;

/// `ImageSizeOptions`.
pub struct ImageSize {
    pub width: i64,
    pub height: i64,
    /// `None` keeps the document's.
    pub resolution: Option<f64>,
    pub sampling: Sampling,
}

/// `DocumentLimits.maxSide`.
const MAX_SIDE: i64 = 30_000;

fn failed(what: &str) -> RenderError {
    RenderError::Failed(anyhow::anyhow!("{what}"))
}

/// `LayerTransform.radians`, in the same order of operations.
fn radians(t: &Transform) -> f64 {
    (t.rotation % 360.0) * std::f64::consts::PI / 180.0
}

/// `LayerTransform.point`: a unit-square point of the layer on the document.
fn point(t: &Transform, (ux, uy): (f64, f64)) -> (f64, f64) {
    let r = radians(t);
    let (x, y) = ((ux - 0.5) * t.size[0], (uy - 0.5) * t.size[1]);
    let (cx, cy) = (t.origin[0] + t.size[0] / 2.0, t.origin[1] + t.size[1] / 2.0);
    (cx + x * r.cos() - y * r.sin(), cy + x * r.sin() + y * r.cos())
}

/// An affine map as Core Graphics writes one: (x, y) → (a·x + c·y + tx, b·x + d·y + ty).
#[derive(Clone, Copy)]
struct Affine {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    tx: f64,
    ty: f64,
}

impl Affine {
    fn apply(&self, (x, y): (f64, f64)) -> (f64, f64) {
        (self.a * x + self.c * y + self.tx, self.b * x + self.d * y + self.ty)
    }

    /// `self.concatenating(other)`: `self`, then `other`.
    fn then(&self, o: &Affine) -> Affine {
        Affine {
            a: self.a * o.a + self.b * o.c,
            b: self.a * o.b + self.b * o.d,
            c: self.c * o.a + self.d * o.c,
            d: self.c * o.b + self.d * o.d,
            tx: self.tx * o.a + self.ty * o.c + o.tx,
            ty: self.tx * o.b + self.ty * o.d + o.ty,
        }
    }

    /// `BrushRaster.pixelToDocument(t, width: 1, height: 1)`: translate to the center, rotate,
    /// scale by the size (negative when flipped), and move the unit square's middle to the origin.
    fn unit_to_document(t: &Transform) -> Affine {
        let r = radians(t);
        let (cos, sin) = (r.cos(), r.sin());
        let sx = t.size[0] * if t.flip_x { -1.0 } else { 1.0 };
        let sy = t.size[1] * if t.flip_y { -1.0 } else { 1.0 };
        let (cx, cy) = (t.origin[0] + t.size[0] / 2.0, t.origin[1] + t.size[1] / 2.0);
        // CGAffineTransform(translationX:y:).rotated(by:).scaledBy(x:y:).translatedBy(x: -0.5, y: -0.5)
        let (a, b, c, d) = (cos * sx, sin * sx, -sin * sy, cos * sy);
        Affine { a, b, c, d, tx: cx + a * -0.5 + c * -0.5, ty: cy + b * -0.5 + d * -0.5 }
    }
}

/// `LayerTransform.placing(_:)`: the rotated, maybe flipped rectangle `map` puts the unit square
/// in, keeping `t`'s horizontal flip and a rotation near its own.
fn placing(t: &Transform, map: &Affine) -> Transform {
    let sign = if t.flip_x { -1.0 } else { 1.0 };
    let angle = (map.b * sign).atan2(map.a * sign);
    let along = -map.c * angle.sin() + map.d * angle.cos();
    let middle = map.apply((0.5, 0.5));
    let mut result = *t;
    result.size = [map.a.hypot(map.b), along.abs()];
    let degrees = angle * 180.0 / std::f64::consts::PI;
    result.rotation = degrees + ((t.rotation - degrees) / 360.0).round() * 360.0;
    result.flip_y = along < 0.0;
    result.origin = [middle.0 - result.size[0] / 2.0, middle.1 - result.size[1] / 2.0];
    result
}

/// `LayerRenderer.drawCoverage`: `mask` clipped to the layer's bounds at `t` and filled white, in
/// a gray `width` x `height` context scaled by `scale` and moved by `offset`.
fn draw_coverage(mask: &GrayImage, t: &Transform, scale: [f64; 2], offset: [f64; 2], (width, height): (u32, u32)) -> Result<GrayImage> {
    let placement = Placement::in_context(t, scale, offset);
    let filter = match Quality::of(t.sampling) {
        Quality::None => Filter::Nearest,
        Quality::Low => Filter::Low,
        // High enlarging is Low; shrinking a clip mask is measured only for upright unlinked masks.
        Quality::High => {
            let (mw, mh) = mask.dimensions();
            if t.size[0] * scale[0] < mw as f64 || t.size[1] * scale[1] < mh as f64 {
                return Err(RenderError::Unsupported("Image Size shrinking a mask with High quality".into()));
            }
            Filter::Low
        }
    };
    let rect = placement.rect(1.0, 1.0);
    let samples = clip::sample(mask, &placement, &rect, filter, t.sampling != Sampling::Nearest, (width, height));
    let bytes = samples.into_iter().map(|(value, covered)| (value * covered / 255) as u8).collect();
    Ok(GrayImage::from_raw(width, height, bytes).expect("mask size"))
}

/// `ImageResizer.resize` with `options`, then `applyImageSize`.
pub fn image_size(gpu: &Gpu, project: &Project, options: &ImageSize) -> Result<Project> {
    let old = &project.manifest;
    let resolution = options.resolution.unwrap_or(old.resolution.unwrap_or(72.0));
    if !(1..=MAX_SIDE).contains(&options.width) || !(1..=MAX_SIDE).contains(&options.height) || !resolution.is_finite() || !(1.0..=9600.0).contains(&resolution) {
        return Err(failed("image size out of range"));
    }
    let mut result = project.clone();
    result.manifest.resolution = Some(resolution);
    if old.width != options.width || old.height != options.height {
        let (sx, sy) = (options.width as f64 / old.width as f64, options.height as f64 / old.height as f64);
        result.manifest.width = options.width;
        result.manifest.height = options.height;
        for g in result.manifest.guides.iter_mut().flatten() {
            g.position *= if g.axis == GuideAxis::Vertical { sx } else { sy };
        }
        result.images.clear();
        result.masks.clear();
        for layer in &mut result.manifest.layers {
            let t = layer.transform;
            let corners = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].map(|u| {
                let (x, y) = point(&t, u);
                (x * sx, y * sy)
            });
            let min = |f: fn(&(f64, f64)) -> f64| corners.iter().map(f).fold(f64::INFINITY, f64::min);
            let max = |f: fn(&(f64, f64)) -> f64| corners.iter().map(f).fold(f64::NEG_INFINITY, f64::max);
            let (left, top) = (min(|c| c.0).floor(), min(|c| c.1).floor());
            let (width, height) = ((max(|c| c.0).ceil() - left) as i64, (max(|c| c.1).ceil() - top) as i64);
            if !(1..=MAX_SIDE).contains(&width) || !(1..=MAX_SIDE).contains(&height) {
                return Err(failed("a layer is too large after Image Size"));
            }
            let size = (width as u32, height as u32);
            let mut drawn = t;
            drawn.sampling = options.sampling;
            let (scale, offset) = ([sx, sy], [-left, -top]);
            if layer.image_file.is_some() {
                let source = project.images.get(&layer.id).ok_or_else(|| failed("a layer's image is missing"))?;
                let canvas = gpu.image(size.0, size.1);
                let premultiplied = super::draw_image(gpu, &source.pixels, &drawn, scale, offset, &canvas)?;
                let straight = crate::blend::unpremultiply(gpu, &premultiplied);
                let pixels = RgbaImage::from_raw(size.0, size.1, gpu.download(&straight)?).expect("layer size");
                result.images.insert(layer.id.clone(), Asset::new(pixels));
            }
            if layer.mask_file.is_some() {
                let source = project.masks.get(&layer.id).ok_or_else(|| failed("a layer's mask is missing"))?;
                // Uniform masks and masks on a placement of their own keep their pixels.
                let uniform = source.pixels.width() == 1 && source.pixels.height() == 1;
                let mask = if uniform || layer.mask_placement.is_some() {
                    source.clone()
                } else {
                    Asset::new(draw_coverage(&source.pixels, &drawn, scale, offset, size)?)
                };
                result.masks.insert(layer.id.clone(), mask);
            }
            let scaled = Affine { a: sx, b: 0.0, c: 0.0, d: sy, tx: 0.0, ty: 0.0 };
            layer.mask_placement = layer.mask_placement.map(|p| placing(&p, &Affine::unit_to_document(&p).then(&scaled)));
            layer.transform = Transform { sampling: options.sampling, ..Transform::at(left, top, width as f64, height as f64) };
        }
    }
    // `applyDocumentSize` rebuilds every layer without its live shape, text and effects.
    for layer in &mut result.manifest.layers {
        layer.shape = None;
        layer.text = None;
        layer.effects = None;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placing_the_unit_square_gives_the_transform_back() {
        let t = Transform { rotation: 30.0, flip_y: true, ..Transform::at(10.0, 20.0, 40.0, 16.0) };
        let back = placing(&t, &Affine::unit_to_document(&t));
        for (a, b) in [(back.origin[0], 10.0), (back.origin[1], 20.0), (back.size[0], 40.0), (back.size[1], 16.0), (back.rotation, 30.0)] {
            assert!((a - b).abs() < 1e-9, "{a} vs {b}");
        }
        assert!(back.flip_y && !back.flip_x);
    }
}
