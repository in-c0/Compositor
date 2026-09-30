//! `DownsampleCache.halve`: exact halvings with vImage's Lanczos resampling, which the Mac draws
//! large reductions from so that Core Graphics only does the last 2× or less.
//!
//! vImage's high-quality scaling is Lanczos with five lobes, widened to the reduction: each output
//! pixel weighs the 20 source pixels around it at sinc(d)·sinc(d/5), d the distance in output
//! pixels. It runs vertically, then horizontally, each pass rounding and clamping to bytes. Measured
//! on the `probe-halve-*` cases; see `parity/features.toml` for how close this is.

use image::{GrayImage, RgbaImage};

fn lanczos5(x: f64) -> f64 {
    if x == 0.0 {
        return 1.0;
    }
    if x.abs() >= 5.0 {
        return 0.0;
    }
    let px = std::f64::consts::PI * x;
    5.0 * px.sin() * (px / 5.0).sin() / (px * px)
}

/// The weights of source pixels 2j − 9 … 2j + 10 for output pixel j.
fn weights() -> [f64; 20] {
    let mut w = [0.0; 20];
    for (t, w) in w.iter_mut().enumerate() {
        *w = lanczos5((t as f64 - 9.5) * 0.5);
    }
    let sum: f64 = w.iter().sum();
    w.map(|v| v / sum)
}

/// Halves `values` (`channels` per pixel, `width` x `height`, both even) with the weights above,
/// vertically then horizontally. The buffer's edges extend beyond it.
fn halve_even(values: &[u8], width: usize, height: usize, channels: usize) -> Vec<u8> {
    let w = weights();
    let (hw, hh) = (width / 2, height / 2);
    let at = |v: &[u8], stride: usize, count: usize, i: isize, offset: usize| -> f64 { v[i.clamp(0, count as isize - 1) as usize * stride + offset] as f64 };
    let round = |v: f64| (v + 0.5).floor().clamp(0.0, 255.0) as u8;
    let mut rows = vec![0u8; width * hh * channels];
    for j in 0..hh {
        for x in 0..width * channels {
            let mut s = 0.0;
            for (t, w) in w.iter().enumerate() {
                s += w * at(values, width * channels, height, 2 * j as isize - 9 + t as isize, x);
            }
            rows[j * width * channels + x] = round(s);
        }
    }
    let mut out = vec![0u8; hw * hh * channels];
    for y in 0..hh {
        let row = &rows[y * width * channels..(y + 1) * width * channels];
        for i in 0..hw {
            for c in 0..channels {
                let mut s = 0.0;
                for (t, w) in w.iter().enumerate() {
                    s += w * at(row, channels, width, 2 * i as isize - 9 + t as isize, c);
                }
                out[(y * hw + i) * channels + c] = round(s);
            }
        }
    }
    out
}

/// A premultiplied image halved, rounding its size up: padded with 8 transparent pixels first, so
/// its edges fade out, with colors clamped to their alpha afterwards.
pub fn halve_color(image: &RgbaImage) -> RgbaImage {
    let (width, height) = (image.width() as usize, image.height() as usize);
    let (w, h) = (width.div_ceil(2), height.div_ceil(2));
    const PAD: usize = 8;
    let (pw, ph) = (w * 2 + PAD * 2, h * 2 + PAD * 2);
    let mut padded = vec![0u8; pw * ph * 4];
    for y in 0..height {
        let from = &image.as_raw()[y * width * 4..(y + 1) * width * 4];
        padded[((y + PAD) * pw + PAD) * 4..((y + PAD) * pw + PAD + width) * 4].copy_from_slice(from);
    }
    let halved = halve_even(&padded, pw, ph, 4);
    let mut out = RgbaImage::new(w as u32, h as u32);
    for y in 0..h {
        for x in 0..w {
            let i = ((y + PAD / 2) * (pw / 2) + x + PAD / 2) * 4;
            let a = halved[i + 3];
            out.put_pixel(x as u32, y as u32, image::Rgba([halved[i].min(a), halved[i + 1].min(a), halved[i + 2].min(a), a]));
        }
    }
    out
}

/// A mask halved, rounding its size up: an odd last column or row is repeated first.
pub fn halve_mask(mask: &GrayImage) -> GrayImage {
    let (width, height) = (mask.width() as usize, mask.height() as usize);
    let (w, h) = (width.div_ceil(2), height.div_ceil(2));
    let (pw, ph) = (w * 2, h * 2);
    let mut padded = vec![0u8; pw * ph];
    for y in 0..ph {
        for x in 0..pw {
            padded[y * pw + x] = mask.as_raw()[y.min(height - 1) * width + x.min(width - 1)];
        }
    }
    GrayImage::from_raw(w as u32, h as u32, halve_even(&padded, pw, ph, 1)).expect("halved size")
}
