//! Destructive filters: Filter > (kind)…, then OK, as `EditorSession.beginFilter`,
//! `updateFilter` and `commitFilter` (Document/Filters.swift) apply them to one layer.
//!
//! The filter runs on the layer's own pixels, premultiplied as Core Graphics draws them into the
//! app's 8-bit bitmaps, and the result replaces the layer's image. The blurs first pad the layer
//! out so they have room to spread (`FilterEdit.growForBlur`), then trim away what they left
//! empty (`PixelFilter.trimmed`), which moves and resizes the layer. Remove Background writes a
//! mask instead of pixels.
//!
//! Most per-pixel work is plain C on the Mac (`AdjustPixels.c`, `NoisePixels.c`,
//! `LensPixels.c`, `DitherPixels.c`), ported here to WGSL operation for operation. The color
//! adjustments are the same code as the adjustment layers, so they run through
//! [`crate::adjust`].

mod dither;
mod settings;
#[cfg(test)]
mod tests;

pub use settings::{DitherSettings, FilterSettings};

use crate::RenderError;
use crate::adjust::{self, Region};
use crate::gpu::{Gpu, GpuImage};
use comp_format::{Adjustment, AdjustmentKind, Asset, ExposureSettings, LayerRecord, Project, Transform};
use serde_json::Value;

type Result<T> = std::result::Result<T, RenderError>;

/// `FilterKind`'s raw values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterKind {
    GaussianBlur,
    MotionBlur,
    AddNoise,
    Vignette,
    BloomGlow,
    Dither,
    TonalContrast,
    LensCorrection,
    CameraRaw,
    RemoveBackground,
    ContentAwareFill,
    Curves,
    Exposure,
    GradientMap,
    Grain,
    BlackWhite,
    ColorBalance,
}

impl FilterKind {
    pub fn from_name(name: &str) -> Option<Self> {
        use FilterKind::*;
        Some(match name {
            "Gaussian Blur" => GaussianBlur,
            "Motion Blur" => MotionBlur,
            "Add Noise" => AddNoise,
            "Vignette" => Vignette,
            "Bloom / Glow" => BloomGlow,
            "Dither" => Dither,
            "Tonal Contrast" => TonalContrast,
            "Lens Correction" => LensCorrection,
            "Camera Raw Filter" => CameraRaw,
            "Remove Background" => RemoveBackground,
            "Content-Aware Fill" => ContentAwareFill,
            "Curves" => Curves,
            "Exposure" => Exposure,
            "Gradient Map" => GradientMap,
            "Grain" => Grain,
            "Black & White" => BlackWhite,
            "Color Balance" => ColorBalance,
            _ => return None,
        })
    }

    /// The filters whose result depends on a random seed; the harness takes it from the case.
    fn seeded(self) -> bool {
        matches!(self, FilterKind::AddNoise | FilterKind::Grain)
    }

    /// The blurs, which spread past the layer's edge: the layer is grown for them and trimmed after.
    fn spreads(self) -> bool {
        matches!(self, FilterKind::GaussianBlur | FilterKind::MotionBlur | FilterKind::BloomGlow)
    }
}

fn failed(message: String) -> RenderError {
    RenderError::Failed(anyhow::anyhow!(message))
}

fn unsupported<T>(what: &str) -> Result<T> {
    Err(RenderError::Unsupported(what.to_string()))
}

/// Applies one corpus `filter` op to `project`.
pub fn apply(gpu: &Gpu, project: &mut Project, op: &Value) -> Result<()> {
    let layer_id = op.get("layer").and_then(Value::as_str).ok_or_else(|| failed("filter: layer is missing".into()))?;
    let kind_name = op.get("kind").and_then(Value::as_str).ok_or_else(|| failed("filter: kind is missing".into()))?;
    let kind = FilterKind::from_name(kind_name).ok_or_else(|| failed(format!("filter: unknown kind {kind_name}")))?;
    // The harness refuses these before it opens anything.
    if kind == FilterKind::CameraRaw {
        return Err(failed("Camera Raw Filter isn't supported by the harness; its settings have no JSON form".into()));
    }
    let settings = FilterSettings::from_json(op.get("settings")).map_err(RenderError::Failed)?;
    let seed = match op.get("seed") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().and_then(|s| u32::try_from(s).ok()).ok_or_else(|| failed("filter: seed must be 0–4294967295".into()))?),
    };
    let seed = match (kind.seeded(), seed) {
        (true, seed) => seed.unwrap_or(0),
        (false, None) => 0,
        (false, Some(_)) => return Err(failed("filter: seed is only for Add Noise and Grain".into())),
    };

    let index = project
        .manifest
        .layers
        .iter()
        .position(|l| l.id.eq_ignore_ascii_case(layer_id))
        .ok_or_else(|| failed(format!("there's no layer {layer_id}")))?;
    let layer = project.manifest.layers[index].clone();
    if kind == FilterKind::ContentAwareFill {
        return Err(failed("Content-Aware Fill needs a selection, and no op makes one".into()));
    }
    // `canAdjustColors`: one visible pixel layer that isn't a folder or an adjustment layer.
    let refused = || failed(format!("the app won't open {kind_name} on layer “{}”", layer.name));
    if layer.is_group() || layer.adjustment.is_some() || !effectively_visible(&project.manifest.layers, &layer) {
        return Err(refused());
    }
    let Some(asset) = project.images.get(&layer.id) else {
        // Vignette starts an empty layer from clear pixels and frames the canvas; nothing else opens.
        return if kind == FilterKind::Vignette { unsupported("Vignette on an empty layer") } else { Err(refused()) };
    };

    let s = settings.normalized();
    // `commitFilter` closes as Cancel does when there's nothing to apply.
    let nothing = match kind {
        FilterKind::LensCorrection => s.distortion == 0.0,
        FilterKind::Vignette => s.vignette_amount == 0.0,
        FilterKind::BloomGlow => s.bloom_amount == 0.0,
        FilterKind::TonalContrast => {
            s.tonal_amount == 0.0 || (s.tonal_shadows == 0.0 && s.tonal_midtones == 0.0 && s.tonal_highlights == 0.0)
        }
        FilterKind::Exposure => s.exposure == ExposureSettings::default(),
        FilterKind::Grain => s.grain.amount == 0.0,
        _ => false,
    };
    if nothing {
        return Ok(());
    }

    let original = asset.pixels.clone();
    let (mut pixels, mut transform) = (original, layer.transform);
    // `FilterEdit.growForBlur`: the panel opens with the session's settings (a fresh session's are
    // the defaults) and grows the layer for them; the case's settings grow it further if they reach
    // further. It never shrinks.
    let grown = kind.spreads();
    if grown {
        let margin = blur_margin(kind, &FilterSettings::default()).max(blur_margin(kind, &s));
        (pixels, transform) = grow(&pixels, &layer.transform, margin.ceil() as u32)?;
    }
    if grown && project.masks.contains_key(&layer.id) {
        return unsupported("a blur growing a layer with a mask");
    }

    let (w, h) = pixels.dimensions();
    let source = premultiply(gpu, &gpu.upload(w, h, pixels.as_raw()));
    let result = run(gpu, kind, &s, seed, &source)?;
    let straight = crate::blend::unpremultiply(gpu, &result);
    let bytes = gpu.download(&straight).map_err(RenderError::Failed)?;
    let mut image = image::RgbaImage::from_raw(w, h, bytes).expect("filter output size");
    if grown {
        (image, transform) = trimmed(image, &transform);
    }

    let record = &mut project.manifest.layers[index];
    // The committed layer is rebuilt from its id, name, visibility, placement, folder, opacity, blend
    // mode, mask and effects: a shape's or a text's live settings don't survive.
    record.transform = transform;
    record.shape = None;
    record.text = None;
    record.is_group = Some(false);
    if record.image_file.is_none() {
        record.image_file = Some(format!("{}.png", record.id));
    }
    project.images.insert(record.id.clone(), Asset::new(image));
    Ok(())
}

/// Whether the layer and every folder above it are visible (`effectiveVisibleIDs`).
fn effectively_visible(layers: &[LayerRecord], layer: &LayerRecord) -> bool {
    let mut current = Some(layer);
    let mut depth = 0;
    while let Some(l) = current {
        if !l.is_visible || depth > layers.len() {
            return false;
        }
        current = l.parent_id.as_deref().and_then(|p| layers.iter().find(|x| x.id == p));
        depth += 1;
    }
    true
}

/// The filter itself: `PixelFilter.run` on premultiplied pixels.
fn run(gpu: &Gpu, kind: FilterKind, s: &FilterSettings, seed: u32, image: &GpuImage) -> Result<GpuImage> {
    let region = Region::whole(image);
    let adjustment = |kind: AdjustmentKind| Adjustment::new(kind);
    Ok(match kind {
        FilterKind::Curves => {
            if !curves_valid(&s.curves) {
                return Err(failed("Curves: the settings are invalid".into()));
            }
            let mut a = adjustment(AdjustmentKind::Curves);
            a.curves = s.curves.clone();
            adjust::apply(gpu, image, &a, region)?
        }
        FilterKind::Exposure => {
            let mut a = adjustment(AdjustmentKind::Exposure);
            a.exposure_settings = Some(s.exposure);
            adjust::apply(gpu, image, &a, region)?
        }
        FilterKind::GradientMap => {
            let mut a = adjustment(AdjustmentKind::GradientMap);
            a.gradient_map_settings = Some(s.gradient_map);
            adjust::apply(gpu, image, &a, region)?
        }
        FilterKind::BlackWhite => {
            let b = &s.black_white;
            let weights = [b.reds, b.yellows, b.greens, b.cyans, b.blues, b.magentas];
            let valid = weights.iter().all(|v| (-200.0..=300.0).contains(v))
                && (0.0..=360.0).contains(&b.tint_hue)
                && (0.0..=100.0).contains(&b.tint_saturation);
            if !valid {
                return Err(failed("Black & White: the settings are invalid".into()));
            }
            let mut a = adjustment(AdjustmentKind::BlackWhite);
            a.black_white_settings = Some(s.black_white);
            adjust::apply(gpu, image, &a, region)?
        }
        FilterKind::ColorBalance => {
            let c = &s.color_balance;
            let all = [
                c.shadow_cyan_red,
                c.shadow_magenta_green,
                c.shadow_yellow_blue,
                c.mid_cyan_red,
                c.mid_magenta_green,
                c.mid_yellow_blue,
                c.highlight_cyan_red,
                c.highlight_magenta_green,
                c.highlight_yellow_blue,
            ];
            if !all.iter().all(|v| (-100.0..=100.0).contains(v)) {
                return Err(failed("Color Balance: the settings are invalid".into()));
            }
            let mut a = adjustment(AdjustmentKind::ColorBalance);
            a.color_balance_settings = Some(s.color_balance);
            adjust::apply(gpu, image, &a, region)?
        }
        FilterKind::Grain => {
            // The filter's grain sits in the layer's own pixels, and the panel's seed replaces the
            // stored pattern.
            let mut a = adjustment(AdjustmentKind::Grain);
            a.grain_settings = Some(comp_format::GrainSettings { seed, ..s.grain });
            adjust::apply(gpu, image, &a, region)?
        }
        FilterKind::AddNoise => {
            let mut a = adjustment(AdjustmentKind::AddNoise);
            a.noise_amount = Some(s.amount);
            a.noise_gaussian = Some(s.gaussian);
            a.noise_monochromatic = Some(s.monochromatic);
            a.noise_seed = Some(seed);
            adjust::apply(gpu, image, &a, region)?
        }
        FilterKind::Vignette => vignette(gpu, image, s),
        FilterKind::LensCorrection => lens(gpu, image, s.distortion / 100.0 * LENS_STRENGTH),
        FilterKind::Dither => dither::apply(gpu, image, &s.dither)?,
        FilterKind::GaussianBlur | FilterKind::MotionBlur | FilterKind::BloomGlow => {
            return unsupported("Core Image's blurs (Gaussian Blur, Motion Blur, Bloom / Glow) aren't exact yet");
        }
        FilterKind::TonalContrast => return unsupported("Tonal Contrast (its base is Core Image's Gaussian blur)"),
        FilterKind::RemoveBackground => return unsupported("Remove Background (Vision subject segmentation)"),
        FilterKind::CameraRaw | FilterKind::ContentAwareFill => unreachable!("refused above"),
    })
}

/// `CurvesSettings.isValid`.
fn curves_valid(curves: &comp_format::CurvesSettings) -> bool {
    curves.channels.len() == 4
        && curves.channels.iter().all(|points| {
            (2..=32).contains(&points.len())
                && points.first().is_some_and(|p| p.x == 0.0)
                && points.last().is_some_and(|p| p.x == 255.0)
                && points.iter().all(|p| p.x.is_finite() && p.y.is_finite() && (0.0..=255.0).contains(&p.x) && (0.0..=255.0).contains(&p.y))
                && points.windows(2).all(|w| w[0].x < w[1].x)
        })
}

/// `PixelFilter.lensStrength`: Remove Distortion at ±100 moves the corners by this share of their
/// distance from the center.
const LENS_STRENGTH: f64 = 0.35;

/// `FilterEdit.blurMargin`: the room a blur needs around the layer.
fn blur_margin(kind: FilterKind, s: &FilterSettings) -> f64 {
    match kind {
        FilterKind::GaussianBlur => s.radius * 3.0 + 2.0,
        FilterKind::MotionBlur => s.distance / 2.0 + 2.0,
        FilterKind::BloomGlow => s.bloom_radius * 3.0 + 2.0,
        _ => 0.0,
    }
}

/// `BrushRaster.pixelToDocument`: layer pixel coordinates to document coordinates.
fn pixel_to_document(t: &Transform, width: u32, height: u32) -> impl Fn(f64, f64) -> (f64, f64) {
    let radians = (t.rotation % 360.0) * std::f64::consts::PI / 180.0;
    let (sin, cos) = radians.sin_cos();
    let sx = t.size[0] / width as f64 * if t.flip_x { -1.0 } else { 1.0 };
    let sy = t.size[1] / height as f64 * if t.flip_y { -1.0 } else { 1.0 };
    let center = (t.origin[0] + t.size[0] / 2.0, t.origin[1] + t.size[1] / 2.0);
    let (w, h) = (width as f64, height as f64);
    move |x, y| {
        let (x, y) = ((x - w / 2.0) * sx, (y - h / 2.0) * sy);
        (center.0 + x * cos - y * sin, center.1 + x * sin + y * cos)
    }
}

/// A grid of `(width, height)` layer pixels starting at `(x, y)` in the old grid, and the
/// transform that keeps the old pixels where they were (`FilterEdit.grow` and
/// `PixelFilter.trimmed` place their new grids this way).
fn placed(t: &Transform, (old_w, old_h): (u32, u32), (x, y, width, height): (f64, f64, f64, f64)) -> Transform {
    let to_document = pixel_to_document(t, old_w, old_h);
    let mut r = *t;
    r.size = [width * t.size[0] / old_w as f64, height * t.size[1] / old_h as f64];
    let middle = to_document(x + width / 2.0, y + height / 2.0);
    r.origin = [middle.0 - r.size[0] / 2.0, middle.1 - r.size[1] / 2.0];
    r
}

/// `FilterEdit.grow` for a blur: the layer's pixels in a clear grid `margin` pixels larger on
/// every side.
fn grow(pixels: &image::RgbaImage, t: &Transform, margin: u32) -> Result<(image::RgbaImage, Transform)> {
    let (w, h) = pixels.dimensions();
    if margin == 0 {
        return Ok((pixels.clone(), *t));
    }
    let (gw, gh) = (w + 2 * margin, h + 2 * margin);
    // `DocumentLimits`: the app refuses a grid this large.
    if gw > 30_000 || gh > 30_000 || gw as u64 * gh as u64 > 268_435_456 {
        return unsupported("a blur growing a layer past the document limits");
    }
    let mut grid = image::RgbaImage::new(gw, gh);
    image::imageops::replace(&mut grid, pixels, margin as i64, margin as i64);
    let m = margin as f64;
    Ok((grid, placed(t, (w, h), (-m, -m, gw as f64, gh as f64))))
}

/// `PixelFilter.trimmed`: the image cropped to the pixels that have any alpha, with the transform
/// that keeps them in place.
fn trimmed(image: image::RgbaImage, t: &Transform) -> (image::RgbaImage, Transform) {
    let (w, h) = image.dimensions();
    let (mut left, mut right, mut top, mut bottom) = (w, 0, h, 0);
    for y in 0..h {
        let Some(first) = (0..w).find(|&x| image.get_pixel(x, y)[3] != 0) else { continue };
        let last = (first..w).rev().find(|&x| image.get_pixel(x, y)[3] != 0).unwrap() + 1;
        left = left.min(first);
        right = right.max(last);
        top = top.min(y);
        bottom = y + 1;
    }
    let (left, top) = if right > 0 { (left, top) } else { (0, 0) };
    let (cw, ch) = (right.saturating_sub(left), bottom.saturating_sub(top));
    if cw < 1 || ch < 1 || (left, top, cw, ch) == (0, 0, w, h) {
        return (image, *t);
    }
    let cropped = image::imageops::crop_imm(&image, left, top, cw, ch).to_image();
    (cropped, placed(t, (w, h), (left as f64, top as f64, cw as f64, ch as f64)))
}

/// Straight PNG pixels premultiplied as Core Graphics draws them into a premultiplied bitmap.
fn premultiply(gpu: &Gpu, image: &GpuImage) -> GpuImage {
    let pipeline = gpu.pipeline("filters.premultiply", include_str!("premultiply.wgsl"));
    let out = gpu.image(image.width, image.height);
    let params = [image.width.to_le_bytes(), image.height.to_le_bytes()].concat();
    gpu.dispatch(&pipeline, &params, &[&image.buffer, &out.buffer], image.width, image.height);
    out
}

const FLOAT: &str = include_str!("../adjust/float.wgsl");
const DOUBLE: &str = include_str!("double.wgsl");

/// A double as the sum of two floats, for the double-single kernels.
fn split(value: f64) -> [f32; 2] {
    let hi = value as f32;
    [hi, (value - hi as f64) as f32]
}

/// Runs a per-pixel kernel whose uniform starts with the float guard, the width and the height.
fn kernel(gpu: &Gpu, name: &'static str, source: &str, image: &GpuImage, params: &[u32], extra: &[&wgpu::Buffer]) -> GpuImage {
    let (w, h) = (image.width, image.height);
    let out = gpu.image(w, h);
    let mut words = vec![f32::INFINITY.to_bits(), w, h];
    words.extend_from_slice(params);
    let mut buffers = vec![&image.buffer, &out.buffer];
    buffers.extend_from_slice(extra);
    let pipeline = gpu.pipeline(name, &format!("{FLOAT}\n{DOUBLE}\n{source}"));
    gpu.dispatch(&pipeline, bytemuck::cast_slice(&words), &buffers, w, h);
    out
}

/// `vignette_mask_at` (AdjustPixels.c), in `f64` with the fused multiply-adds clang makes of it.
fn vignette_mask_at(px: f64, py: f64, width: f64, height: f64, midpoint: f64, roundness: f64, feather: f64) -> f64 {
    let nx = (px / width).mul_add(2.0, -1.0);
    let ny = (py / height).mul_add(2.0, -1.0);
    let square = nx.abs().max(ny.abs());
    let circle = nx.hypot(ny) / 2f64.sqrt();
    let shape = (1.0 - roundness / 100.0) * 0.5;
    let dist = (square - circle).mul_add(shape, circle);
    let start = (midpoint / 100.0) * 0.85;
    let soft = (feather / 100.0).max(0.05);
    let t = ((dist - start) / soft).clamp(0.0, 1.0);
    t * t * (-2.0f64).mul_add(t, 3.0)
}

/// Vignette (`adjust_colored_vignette`) on a layer's own pixels: the frame is the layer, and only
/// the pixels that are there change color.
fn vignette(gpu: &Gpu, image: &GpuImage, s: &FilterSettings) -> GpuImage {
    let (w, h) = (image.width, image.height);
    let strength = (s.vignette_amount / 100.0).clamp(0.0, 1.0);
    // strength × mask per pixel: it depends only on where the pixel is.
    let mut scaled = Vec::with_capacity((w * h * 2) as usize);
    for y in 0..h {
        for x in 0..w {
            let mask = vignette_mask_at(
                x as f64 + 0.5,
                y as f64 + 0.5,
                w as f64,
                h as f64,
                s.vignette_midpoint,
                s.vignette_roundness,
                s.vignette_feather,
            );
            let v = if mask <= 0.0 { 0.0 } else { strength * mask };
            scaled.extend_from_slice(&split(v));
        }
    }
    let c = s.vignette_color;
    let mut params = vec![0];
    for v in [-(s.vignette_highlights / 100.0), c.red, c.green, c.blue] {
        params.extend(split(v).map(f32::to_bits));
    }
    let table = gpu.bytes(bytemuck::cast_slice(&scaled));
    kernel(gpu, "filters.vignette", include_str!("vignette.wgsl"), image, &params, &[&table])
}

/// Lens Correction (`lens_distort`, LensPixels.c). Where each pixel samples from and the bilinear
/// weights depend only on the settings and the size, so they're worked out here in `f64` as the
/// C does; the kernel sums the pixels.
fn lens(gpu: &Gpu, image: &GpuImage, k: f64) -> GpuImage {
    let (w, h) = (image.width, image.height);
    let (cx, cy) = (w as f64 * 0.5, h as f64 * 0.5);
    let half_diagonal2 = cx.mul_add(cx, cy * cy);
    // Per pixel: the top-left source pixel, then the four weights as double-singles.
    let mut table: Vec<u32> = Vec::with_capacity((w * h * 10) as usize);
    for y in 0..h {
        let dy = y as f64 + 0.5 - cy;
        for x in 0..w {
            let dx = x as f64 + 0.5 - cx;
            let scale = 1.0 - k * dx.mul_add(dx, dy * dy) / half_diagonal2;
            let sx = dx.mul_add(scale, cx) - 0.5;
            let sy = dy.mul_add(scale, cy) - 0.5;
            let (fx0, fy0) = (sx.floor(), sy.floor());
            let (fx, fy) = (sx - fx0, sy - fy0);
            table.push(fx0 as i64 as i32 as u32);
            table.push(fy0 as i64 as i32 as u32);
            for j in 0..2 {
                let wy = if j == 1 { fy } else { 1.0 - fy };
                for i in 0..2 {
                    let weight = wy * if i == 1 { fx } else { 1.0 - fx };
                    table.extend(split(weight).map(f32::to_bits));
                }
            }
        }
    }
    let table = gpu.bytes(bytemuck::cast_slice(&table));
    kernel(gpu, "filters.lens", include_str!("lens.wgsl"), image, &[], &[&table])
}
