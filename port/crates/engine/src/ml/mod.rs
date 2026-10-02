//! Remove Background's subject mask. The Mac asks Apple's Vision for it
//! (`VNGenerateForegroundInstanceMaskRequest` in Document/SubjectRemoval.swift), a model that
//! only exists on Apple's systems, so the port can't match it exactly. It stands in U²-Netp
//! (Qin et al. 2020, Apache-2.0, 4.6 MB), the small salient-object model rembg ships, run on the
//! CPU through tract, a pure-Rust ONNX runtime that needs no network or native library. What it
//! gets wrong against Vision is measured in `parity/features.toml` (the `ml` feature).
//!
//! The model file isn't in the repository: `port/tools/fetch-models.sh` downloads it, checked
//! against its SHA-256, into `models/` next to the executables. Without it Remove Background
//! reports itself unsupported.
//!
//! Advanced's refinement (`SubjectRemoval.refined` and `GuidedMatte`) is plain arithmetic on the
//! mask and is ported as the Swift does it, in `f32`; it can't be exact while the mask under it
//! isn't.

use crate::RenderError;
use image::{GrayImage, Luma, RgbaImage};

/// The model's input: 320 × 320 RGB, normalized with ImageNet's mean and deviation after scaling
/// the image so its brightest channel is 1, as rembg prepares it.
const SIDE: usize = 320;
const MODEL: &str = "u2netp.onnx";

/// Remove Background's settings that refine the mask (Advanced only).
pub struct Refinement {
    pub refine_edges: f64,
    pub matte_contrast: f64,
    pub shift_edge: f64,
}

/// The model's own answer for an image: saliency on its 320 × 320 grid, stretched to 0…1 as rembg
/// does, and the peak before stretching (the sigmoid's own confidence, 0…1).
pub struct Saliency {
    pub side: usize,
    pub values: Vec<f32>,
    pub peak: f32,
}

/// Where the subject is in `image` (straight RGBA): white over it, black over the background, the
/// image's size. Stands in for `SubjectRemoval.vision`.
pub fn subject_mask(image: &RgbaImage) -> Result<GrayImage, RenderError> {
    let map = saliency(image)?;
    Ok(saliency_to_image(&map, image.width(), image.height()))
}

/// The stretched saliency resampled to `w` × `h` bytes.
pub fn saliency_to_image(map: &Saliency, w: u32, h: u32) -> GrayImage {
    let full = resample(&map.values, 1, (map.side, map.side), (w as usize, h as usize));
    GrayImage::from_fn(w, h, |x, y| Luma([to_byte(full[(y * w + x) as usize])]))
}

/// U²-Netp's saliency for `image` (straight RGBA).
#[cfg(feature = "ml")]
pub fn saliency(image: &RgbaImage) -> Result<Saliency, RenderError> {
    use std::sync::{Mutex, OnceLock};
    use tract_onnx::prelude::*;
    type Model = TypedRunnableModel;
    static MODEL_PLAN: OnceLock<Mutex<Option<std::sync::Arc<Model>>>> = OnceLock::new();

    let Some(path) = model_path() else {
        return Err(RenderError::Unsupported(format!(
            "the subject mask without its model ({MODEL}; run port/tools/fetch-models.sh)"
        )));
    };
    let plan = {
        let mut slot = MODEL_PLAN.get_or_init(|| Mutex::new(None)).lock().unwrap();
        if slot.is_none() {
            let model = tract_onnx::onnx()
                .model_for_path(&path)
                .and_then(|m| m.with_input_fact(0, f32::fact([1, 3, SIDE, SIDE]).into()))
                .and_then(|m| m.into_optimized())
                .and_then(|m| m.into_runnable())
                .map_err(|e| RenderError::Failed(anyhow::anyhow!("loading {MODEL}: {e}")))?;
            *slot = Some(model);
        }
        slot.clone().unwrap()
    };

    let (w, h) = image.dimensions();
    // Straight color over mid gray: transparent pixels are no subject, and a neutral ground doesn't
    // read as one either.
    let rgb: Vec<f32> = image
        .pixels()
        .flat_map(|p| {
            let a = p[3] as f32 / 255.0;
            [0, 1, 2].map(move |c| p[c] as f32 / 255.0 * a + 0.5 * (1.0 - a))
        })
        .collect();
    let small = resample(&rgb, 3, (w as usize, h as usize), (SIDE, SIDE));
    let max = small.iter().cloned().fold(1e-6f32, f32::max);
    const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
    const DEVIATION: [f32; 3] = [0.229, 0.224, 0.225];
    let input: Tensor = tract_ndarray::Array4::from_shape_fn((1, 3, SIDE, SIDE), |(_, c, y, x)| {
        (small[(y * SIDE + x) * 3 + c] / max - MEAN[c]) / DEVIATION[c]
    })
    .into();
    let outputs = plan.run(tvec!(input.into())).map_err(|e| RenderError::Failed(anyhow::anyhow!("{MODEL}: {e}")))?;
    let prediction = outputs[0].to_plain_array_view::<f32>().map_err(|e| RenderError::Failed(anyhow::anyhow!("{MODEL}: {e}")))?;
    let values: Vec<f32> = prediction.iter().cloned().collect();
    // Stretched to 0…1, as rembg does.
    let (lo, hi) = values.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    let span = (hi - lo).max(1e-6);
    Ok(Saliency { side: SIDE, values: values.iter().map(|v| (v - lo) / span).collect(), peak: hi })
}

#[cfg(not(feature = "ml"))]
pub fn saliency(_image: &RgbaImage) -> Result<Saliency, RenderError> {
    Err(RenderError::Unsupported("the subject mask (built without the `ml` feature)".into()))
}

/// `COMPOSITOR_MODELS`, or `models/` next to the executable or one folder up (test binaries run
/// from `target/<profile>/deps`).
#[cfg(feature = "ml")]
fn model_path() -> Option<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::var_os("COMPOSITOR_MODELS") {
        dirs.push(std::path::PathBuf::from(dir));
    }
    if let Some(exe) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) {
        dirs.push(exe.join("models"));
        if let Some(up) = exe.parent() {
            dirs.push(up.join("models"));
        }
    }
    dirs.into_iter().map(|d| d.join(MODEL)).find(|p| p.is_file())
}

/// 0…1 to a byte, as `GuidedMatte.image` and Core Image store a mask.
pub(crate) fn to_byte(v: f32) -> u8 {
    (v * 255.0 + 0.5).clamp(0.0, 255.0) as u8
}

/// A separable triangle-filter resize of `channels`-interleaved floats, widened when shrinking so
/// every source pixel counts.
pub(crate) fn resample(src: &[f32], channels: usize, (sw, sh): (usize, usize), (dw, dh): (usize, usize)) -> Vec<f32> {
    let weights = |from: usize, to: usize| -> Vec<Vec<(usize, f32)>> {
        let scale = from as f32 / to as f32;
        let support = scale.max(1.0);
        (0..to)
            .map(|i| {
                let center = (i as f32 + 0.5) * scale;
                let lo = ((center - support).floor().max(0.0)) as usize;
                let hi = ((center + support).ceil() as usize).min(from);
                let mut taps: Vec<(usize, f32)> =
                    (lo..hi).map(|j| (j, (1.0 - ((j as f32 + 0.5 - center) / support).abs()).max(0.0))).filter(|t| t.1 > 0.0).collect();
                if taps.is_empty() {
                    taps.push(((center as usize).min(from - 1), 1.0));
                }
                let total: f32 = taps.iter().map(|t| t.1).sum();
                taps.iter_mut().for_each(|t| t.1 /= total);
                taps
            })
            .collect()
    };
    let (across, down) = (weights(sw, dw), weights(sh, dh));
    let mut rows = vec![0f32; dw * sh * channels];
    for y in 0..sh {
        for (x, taps) in across.iter().enumerate() {
            for c in 0..channels {
                rows[(y * dw + x) * channels + c] = taps.iter().map(|&(j, k)| src[(y * sw + j) * channels + c] * k).sum();
            }
        }
    }
    let mut out = vec![0f32; dw * dh * channels];
    for (y, taps) in down.iter().enumerate() {
        for x in 0..dw {
            for c in 0..channels {
                out[(y * dw + x) * channels + c] = taps.iter().map(|&(j, k)| rows[(j * dw + x) * channels + c] * k).sum();
            }
        }
    }
    out
}

/// `SubjectRemoval.refined` for Advanced: the mask pulled onto the layer's edges, its edge moved,
/// its grays pushed apart.
pub fn refined(mask: &GrayImage, guide: &RgbaImage, r: &Refinement) -> GrayImage {
    let (w, h) = mask.dimensions();
    let (wu, hu) = (w as usize, h as usize);
    let mut levels: Vec<f32> = mask.pixels().map(|p| p[0] as f32 / 255.0).collect();
    if r.refine_edges > 0.0 {
        // `GuidedMatte.levels(of: guide)`: the layer drawn into a gray bitmap over black.
        let gray: Vec<f32> = guide
            .pixels()
            .map(|p| {
                let a = p[3] as f32 / 255.0;
                let y = 0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32;
                (y * a).round() / 255.0
            })
            .collect();
        let steps = (r.refine_edges.round() as usize).max(1);
        levels = guided_filter(&levels, &gray, wu, hu, steps, 1e-4);
        // `GuidedMatte.image`: back to bytes.
        levels = levels.iter().map(|&v| to_byte(v) as f32 / 255.0).collect();
    }
    if r.shift_edge != 0.0 {
        // A blur, then a hard threshold at the matching level, moves the edge by the blur's reach.
        let blurred = gaussian(&levels, wu, hu, r.shift_edge.abs() as f32 / 2.0);
        let level = if r.shift_edge < 0.0 { 0.75 } else { 0.25 };
        levels = blurred.iter().map(|&v| ((v.clamp(level, level + 0.001) - level) / 0.001).clamp(0.0, 1.0)).collect();
    }
    if r.matte_contrast > 0.0 {
        // 0 leaves the mask as it is; 100 is a hard cut at the middle.
        let strength = (r.matte_contrast / 100.0) as f32;
        let slope = 1.0 / (1.0 - strength * 0.98).max(0.02);
        levels = levels.iter().map(|&v| (v * slope + (1.0 - slope) / 2.0).clamp(0.0, 1.0)).collect();
    }
    GrayImage::from_fn(w, h, |x, y| Luma([to_byte(levels[y as usize * wu + x as usize])]))
}

/// `GuidedMatte.box`: the mean over a (2r+1)² square with the edges repeated, as running sums.
fn box_mean(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let span = (r * 2 + 1) as f32;
    let at = |i: isize, n: usize| i.clamp(0, n as isize - 1) as usize;
    let mut pass = vec![0f32; w * h];
    for y in 0..h {
        let row = y * w;
        let mut sum = 0f32;
        for x in -(r as isize)..=r as isize {
            sum += src[row + at(x, w)];
        }
        for x in 0..w {
            pass[row + x] = sum / span;
            sum -= src[row + at(x as isize - r as isize, w)];
            sum += src[row + at(x as isize + r as isize + 1, w)];
        }
    }
    let mut out = vec![0f32; w * h];
    for x in 0..w {
        let mut sum = 0f32;
        for y in -(r as isize)..=r as isize {
            sum += pass[at(y, h) * w + x];
        }
        for y in 0..h {
            out[y * w + x] = sum / span;
            sum -= pass[at(y as isize - r as isize, h) * w + x];
            sum += pass[at(y as isize + r as isize + 1, h) * w + x];
        }
    }
    out
}

/// `GuidedMatte.filter` (He, Sun & Tang's guided filter).
fn guided_filter(mask: &[f32], guide: &[f32], w: usize, h: usize, r: usize, epsilon: f32) -> Vec<f32> {
    let mean_guide = box_mean(guide, w, h, r);
    let mean_mask = box_mean(mask, w, h, r);
    let squares: Vec<f32> = guide.iter().map(|g| g * g).collect();
    let products: Vec<f32> = guide.iter().zip(mask).map(|(g, m)| g * m).collect();
    let mean_squares = box_mean(&squares, w, h, r);
    let mean_products = box_mean(&products, w, h, r);
    let mut slope = vec![0f32; w * h];
    let mut offset = vec![0f32; w * h];
    for i in 0..w * h {
        let variance = mean_squares[i] - mean_guide[i] * mean_guide[i];
        let covariance = mean_products[i] - mean_guide[i] * mean_mask[i];
        slope[i] = covariance / (variance + epsilon);
        offset[i] = mean_mask[i] - slope[i] * mean_guide[i];
    }
    let mean_slope = box_mean(&slope, w, h, r);
    let mean_offset = box_mean(&offset, w, h, r);
    (0..w * h).map(|i| (mean_slope[i] * guide[i] + mean_offset[i]).clamp(0.0, 1.0)).collect()
}

/// A Gaussian blur with the edges repeated (Core Image's `clampedToExtent`), for Shift Edge.
fn gaussian(src: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    if sigma <= 0.0 {
        return src.to_vec();
    }
    let radius = (sigma * 3.0).ceil() as isize;
    let kernel: Vec<f32> = (-radius..=radius).map(|k| (-(k * k) as f32 / (2.0 * sigma * sigma)).exp()).collect();
    let total: f32 = kernel.iter().sum();
    let at = |i: isize, n: usize| i.clamp(0, n as isize - 1) as usize;
    let mut pass = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            pass[y * w + x] = (-radius..=radius).map(|k| src[y * w + at(x as isize + k, w)] * kernel[(k + radius) as usize]).sum::<f32>() / total;
        }
    }
    let mut out = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            out[y * w + x] = (-radius..=radius).map(|k| pass[at(y as isize + k, h) * w + x] * kernel[(k + radius) as usize]).sum::<f32>() / total;
        }
    }
    out
}
