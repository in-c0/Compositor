//! Core Graphics' High interpolation where it shrinks an image, measured with the harness's
//! `probeDraw` (the `transform/cg-*` probes).
//!
//! Core Graphics first resamples the whole image to the number of device pixels the drawing
//! rectangle spans along each axis it shrinks, rounded up, and then draws that copy into the
//! rectangle as it draws any image, with Low. The copy is made across, then down, each pass rounding
//! to bytes:
//!
//! - Output pixel i of n takes source position x = i·(m − 1)/(n − 1) for n pixels from m: the first
//!   and last pixel centers line up.
//! - The weights are Lanczos 2 widened by the reduction, sinc(d)·sinc(d/2) at d = (j − x)·g with
//!   g = (n − 1)/(m − 1), except that the source pixel nearest x weighs 1, as if it were exactly
//!   under x. Which pixel is nearest comes from i/g with g in single precision, which decides the
//!   ties. Taps beyond the image repeat its edge pixels.
//! - The weights are normalized, rounded to 14 fractional bits, and what rounding lost or gained goes
//!   to the largest; sums round half up.
//! - Premultiplied pixels whose color overshoots their alpha get the alpha raised to the largest color.
//!
//! The draw from the copy then steps its sample positions from a start and steps rounded to
//! nearest in 32.32 fixed point, where drawing an image directly rounds them down
//! (`transform/cg-high-xy-0.33` decides it: a position exactly on an eighth's boundary).

use super::{Draw, Placement, Quality, Source};
use crate::gpu::{Gpu, GpuImage};
use crate::RenderError;
use comp_format::{BlendMode, Project, Transform};
use image::RgbaImage;

fn lanczos2(x: f64) -> f64 {
    if x == 0.0 {
        return 1.0;
    }
    if x.abs() >= 2.0 {
        return 0.0;
    }
    let px = std::f64::consts::PI * x;
    2.0 * px.sin() * (px / 2.0).sin() / (px * px)
}

/// Each output pixel's taps, source index and weight in 1/16384ths, for `from` pixels to `to`.
fn taps(from: u32, to: u32) -> Vec<Vec<(usize, i32)>> {
    let g = if to > 1 { (to - 1) as f64 / (from - 1) as f64 } else { 1.0 };
    // The nearest pixel comes from the position in single-precision steps, which decides ties.
    let g32 = if to > 1 { (to - 1) as f32 / (from - 1) as f32 } else { 1.0 } as f64;
    let last = from as i64 - 1;
    (0..to)
        .map(|i| {
            let x = if to > 1 { (i as u64 * (from as u64 - 1)) as f64 / (to - 1) as f64 } else { 0.0 };
            let nearest = if to > 1 { (i as f64 / g32 + 0.5).floor() as i64 } else { 0 };
            let reach = 2.0 / g;
            let taps: Vec<(i64, f64)> = ((x - reach).floor() as i64..=(x + reach).ceil() as i64)
                .map(|j| (j, if j == nearest { 1.0 } else { lanczos2((j as f64 - x) * g) }))
                .filter(|&(_, w)| w != 0.0)
                .collect();
            let sum: f64 = taps.iter().map(|t| t.1).sum();
            let mut fixed: Vec<(usize, i32)> = taps.iter().map(|&(j, w)| (j.clamp(0, last) as usize, (w / sum * 16384.0 + 0.5).floor() as i32)).collect();
            let lost = 16384 - fixed.iter().map(|t| t.1).sum::<i32>();
            let largest = (0..fixed.len()).max_by_key(|&k| (fixed[k].1, std::cmp::Reverse(k))).expect("a tap");
            fixed[largest].1 += lost;
            fixed
        })
        .collect()
}

/// One pass over rows of `values` (`stride` apart, `count` of them), each `from` long with
/// `step` between pixels, resampled to `to`.
fn pass(values: &[u8], from: u32, to: u32, across: bool, width: u32, height: u32) -> (Vec<u8>, u32, u32) {
    let taps = taps(from, to);
    let (out_w, out_h) = if across { (to, height) } else { (width, to) };
    let mut out = vec![0u8; (out_w * out_h * 4) as usize];
    for y in 0..out_h {
        for x in 0..out_w {
            let (i, line) = if across { (x, y) } else { (y, x) };
            for c in 0..4 {
                let sum: i32 = taps[i as usize]
                    .iter()
                    .map(|&(j, w)| {
                        let (sx, sy) = if across { (j as u32, line) } else { (line, j as u32) };
                        w * values[((sy * width + sx) * 4 + c) as usize] as i32
                    })
                    .sum();
                out[((y * out_w + x) * 4 + c) as usize] = ((sum + 8192) >> 14).clamp(0, 255) as u8;
            }
        }
    }
    (out, out_w, out_h)
}

/// The copy Core Graphics draws from when High shrinks premultiplied `pixels` into a rectangle
/// `width` x `height` device pixels: each axis that shrinks resampled to its span rounded up.
/// `None` when neither axis shrinks.
pub fn shrink(pixels: &RgbaImage, width: f64, height: f64) -> Option<RgbaImage> {
    let (w, h) = pixels.dimensions();
    let to_w = if width < w as f64 { width.ceil().max(1.0) as u32 } else { w };
    let to_h = if height < h as f64 { height.ceil().max(1.0) as u32 } else { h };
    if (to_w, to_h) == (w, h) {
        return None;
    }
    let mut values = pixels.as_raw().clone();
    let (mut cw, mut ch) = (w, h);
    if to_w != w {
        (values, cw, ch) = pass(&values, w, to_w, true, cw, ch);
    }
    if to_h != h {
        (values, cw, ch) = pass(&values, h, to_h, false, cw, ch);
    }
    for p in values.chunks_exact_mut(4) {
        p[3] = p[3].max(p[0]).max(p[1]).max(p[2]);
    }
    Some(RgbaImage::from_raw(cw, ch, values).expect("shrunk size"))
}

/// `CGContext.draw(_:in:)` of premultiplied `pixels` into a fresh, transparent `width` x `height`
/// bitmap at `rect` (x, y, width, height in device pixels from the top left), antialiased, with
/// `quality`: what Dither's reduction and the harness's `probeDraw` do.
pub fn draw_into(gpu: &Gpu, pixels: &RgbaImage, (width, height): (u32, u32), rect: [f64; 4], quality: Quality) -> GpuImage {
    let t = Transform::at(rect[0], rect[1], rect[2], rect[3]);
    let mut placement = Placement::of(&t);
    let mut source = std::borrow::Cow::Borrowed(pixels);
    let mut quality = quality;
    if quality == Quality::High {
        if let Some(small) = shrink(pixels, rect[2], rect[3]) {
            source = std::borrow::Cow::Owned(small);
            placement.nearest = true;
        }
        quality = Quality::Low;
    }
    let (w, h) = source.dimensions();
    let image = gpu.upload(w, h, source.as_raw());
    let draw = Draw {
        image: Source { buffer: &image.buffer, width: w, height: h, rect: placement.rect(1.0, 1.0) },
        premultiplied: true,
        placement,
        quality,
        antialias: true,
        mode: BlendMode::Normal,
        opacity: 1.0,
        mask: None,
        clip: None,
    };
    super::draw(gpu, &gpu.image(width, height), &draw)
}

/// The harness's `probeDraw`: the layer's image drawn with `CGContext.draw` into a fresh bitmap,
/// which then replaces its pixels, placed 1:1 at the top left. Only unrotated, unscaled context
/// transforms and the qualities measured here are reproduced.
pub fn probe_draw(gpu: &Gpu, project: &mut Project, op: &serde_json::Value) -> Result<(), RenderError> {
    let unsupported = |what: &str| RenderError::Unsupported(format!("probeDraw {what}"));
    let failed = |what: &str| RenderError::Failed(anyhow::anyhow!("probeDraw: {what}"));
    let layer = op.get("layer").and_then(|v| v.as_str()).ok_or_else(|| failed("no layer"))?.to_string();
    let size = |key: &str| op.get(key).and_then(|v| v.as_u64()).filter(|&v| (1..=4096).contains(&v)).map(|v| v as u32);
    let (width, height) = (size("width").ok_or_else(|| failed("no width"))?, size("height").ok_or_else(|| failed("no height"))?);
    let numbers = |key: &str| op.get(key).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_f64()).collect::<Vec<f64>>());
    let rect = numbers("rect").filter(|r| r.len() == 4).ok_or_else(|| failed("no rect"))?;
    if numbers("transform").is_some_and(|t| t != [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]) {
        return Err(unsupported("with a context transform"));
    }
    if op.get("antialias").and_then(|v| v.as_bool()) == Some(false) {
        return Err(unsupported("without antialiasing"));
    }
    let quality = match op.get("quality").and_then(|v| v.as_str()).unwrap_or("high") {
        "none" => Quality::None,
        "low" => Quality::Low,
        "high" => Quality::High,
        other => return Err(unsupported(&format!("with quality {other}"))),
    };
    let asset = project.images.get(&layer).ok_or_else(|| failed("no such pixel layer"))?;
    let mut pixels = asset.pixels.clone();
    for p in pixels.pixels_mut() {
        let a = p[3] as u32;
        for c in 0..3 {
            p[c] = ((p[c] as u32 * a + 127) / 255) as u8;
        }
    }
    // The context is y up: the rectangle's top is `height` less its top edge.
    let top = height as f64 - (rect[1] + rect[3]);
    let drawn = draw_into(gpu, &pixels, (width, height), [rect[0], top, rect[2], rect[3]], quality);
    let straight = crate::blend::unpremultiply(gpu, &drawn);
    let straight = RgbaImage::from_raw(width, height, gpu.download(&straight)?).expect("probe size");
    project.images.insert(layer.clone(), comp_format::Asset::new(straight));
    if let Some(record) = project.manifest.layers.iter_mut().find(|l| l.id == layer) {
        record.transform = Transform { sampling: record.transform.sampling, ..Transform::at(0.0, 0.0, width as f64, height as f64) };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_sum_to_one_and_favor_the_nearest_pixel() {
        for (from, to) in [(128, 96), (128, 127), (64, 22), (64, 40)] {
            for t in taps(from, to) {
                assert_eq!(t.iter().map(|t| t.1).sum::<i32>(), 16384);
            }
        }
        // Half-way between two pixels, the nearer (rounding up) weighs more.
        let t = &taps(128, 127)[63];
        let w = |j: usize| t.iter().filter(|t| t.0 == j).map(|t| t.1).sum::<i32>();
        assert!(w(63) > w(64));
    }

    #[test]
    fn a_flat_image_stays_flat() {
        let flat = RgbaImage::from_pixel(64, 64, image::Rgba([90, 60, 30, 200]));
        let small = shrink(&flat, 21.0 + 1.0 / 3.0, 40.0).unwrap();
        assert_eq!(small.dimensions(), (22, 40));
        assert!(small.pixels().all(|p| p.0 == [90, 60, 30, 200]));
    }
}
