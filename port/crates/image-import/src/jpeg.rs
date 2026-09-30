//! JPEG, as ImageIO reads it. The coefficients come from `jpeg_coefficients`; the inverse DCT and
//! the YCbCr conversion follow what Apple's decoder was measured to do (import/jpeg-* probes):
//! 4:4:4 converts each pixel's rounded YCbCr in float and truncates, while subsampled chroma is
//! interpolated bilinearly between sample centers and the result rounded, as for HEIC.

use crate::color::{Curve, Space};
use crate::develop::{Samples, Source};
use crate::jpeg_coefficients::{self, Coefficients};
use crate::{ImportError, Result, exif, jpeg_markers};

/// One component's samples after the inverse DCT, padded to whole blocks.
struct Plane {
    width: usize,
    samples: Vec<f32>,
}

/// cos((2x + 1)uπ/16) scaled for the 8-point inverse DCT, by [x][u].
fn basis() -> [[f64; 8]; 8] {
    let mut b = [[0.0; 8]; 8];
    for (x, row) in b.iter_mut().enumerate() {
        for (u, v) in row.iter_mut().enumerate() {
            let c = if u == 0 { std::f64::consts::FRAC_1_SQRT_2 } else { 1.0 };
            *v = c * ((2 * x + 1) as f64 * u as f64 * std::f64::consts::PI / 16.0).cos() / 2.0;
        }
    }
    b
}

/// The inverse DCT of one block of quantized coefficients, level-shifted.
fn idct(basis: &[[f64; 8]; 8], block: &[i32; 64], quant: &[u16; 64], out: &mut [f64; 64]) {
    let mut coefficients = [0.0f64; 64];
    for i in 0..64 {
        coefficients[i] = (block[i] * quant[i] as i32) as f64;
    }
    let mut rows = [0.0f64; 64];
    for v in 0..8 {
        for x in 0..8 {
            rows[v * 8 + x] = (0..8).map(|u| basis[x][u] * coefficients[v * 8 + u]).sum();
        }
    }
    for y in 0..8 {
        for x in 0..8 {
            out[y * 8 + x] = (0..8).map(|v| basis[y][v] * rows[v * 8 + x]).sum::<f64>() + 128.0;
        }
    }
}

fn plane(frame: &Coefficients, index: usize) -> Plane {
    let c = &frame.components[index];
    let quant = &frame.quant[c.quant];
    let basis = basis();
    let width = c.blocks_wide * 8;
    let mut samples = vec![0.0f32; width * c.blocks_high * 8];
    let mut out = [0.0f64; 64];
    for by in 0..c.blocks_high {
        for bx in 0..c.blocks_wide {
            idct(&basis, &c.coefficients[by * c.blocks_wide + bx], quant, &mut out);
            for y in 0..8 {
                for x in 0..8 {
                    samples[(by * 8 + y) * width + bx * 8 + x] = (out[y * 8 + x] + 0.51).floor().clamp(0.0, 255.0) as f32;
                }
            }
        }
    }
    Plane { width, samples }
}

pub(crate) fn decode(data: &[u8]) -> Result<Source> {
    let markers = jpeg_markers::read(data);
    let frame = jpeg_coefficients::read(data)?;
    let (w, h) = (frame.width, frame.height);
    let orientation = markers.exif.as_deref().and_then(exif::orientation).unwrap_or(1);
    let (colors, samples, approximation) = match frame.components.len() {
        1 => {
            let p = plane(&frame, 0);
            let gray: Vec<f32> = (0..h).flat_map(|y| p.samples[y * p.width..y * p.width + w].iter().map(|v| v / 255.0)).collect();
            (1, gray, None)
        }
        3 if frame.adobe_transform != Some(0) => ycbcr(&frame)?,
        3 => return Err(ImportError::NotPorted("RGB JPEG (Adobe transform 0)".into())),
        _ => return Err(ImportError::NotPorted("CMYK JPEG (ColorSync converts it with Apple's Generic CMYK profile)".into())),
    };
    let space = match &markers.icc {
        Some(icc) => Space::from_icc(icc, colors)?,
        None if colors == 1 => Space::Gray(Curve::Srgb),
        None => Space::Srgb,
    };
    Ok(Source {
        width: w as u32,
        height: h as u32,
        colors,
        alpha: false,
        premultiplied: false,
        samples: Samples::F32(samples),
        space,
        orientation,
        approximation,
    })
}

type Decoded = (usize, Vec<f32>, Option<String>);

/// YCbCr to RGB (0-1, not yet rounded to bytes), and why it is only approximate when it is.
fn ycbcr(frame: &Coefficients) -> Result<Decoded> {
    let (w, h) = (frame.width, frame.height);
    let planes: Vec<Plane> = (0..3).map(|i| plane(frame, i)).collect();
    let luma = &frame.components[0];
    let chroma = &frame.components[1..];
    if chroma.iter().any(|c| c.h != chroma[0].h || c.v != chroma[0].v) || luma.h != frame.max_h || luma.v != frame.max_v {
        return Err(ImportError::NotPorted("JPEG with unusual sampling factors".into()));
    }
    let (fx, mut fy) = (luma.h / chroma[0].h, luma.v / chroma[0].v);
    let full = fx == 1 && fy == 1;
    let (cw, mut ch) = (w.div_ceil(fx), h.div_ceil(fy));
    let mut planes = planes;
    if (fx, fy) == (2, 1) {
        // Apple makes 4:2:2 chroma 4:2:0 before converting: each pair of rows averaged and a
        // [1, 6, 1] / 8 filter across (fitted on import/jpeg-ycc-422), then the 4:2:0 path.
        for p in &mut planes[1..] {
            *p = halve_rows(p, cw, ch);
        }
        ch = ch.div_ceil(2);
        fy = 2;
    }
    let sample = |p: &Plane, x: usize, y: usize| p.samples[y * p.width + x];
    let mut out = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            let luma = sample(&planes[0], x, y);
            let (cb, cr) = if full {
                (sample(&planes[1], x, y), sample(&planes[2], x, y))
            } else {
                (bilinear(&planes[1], x, y, fx, fy, cw, ch), bilinear(&planes[2], x, y, fx, fy, cw, ch))
            };
            // Subsampled chroma goes through the same conversion as HEIC, which divides chroma
            // by 254 where luma is divided by 255.
            let scale = if full { 1.0 } else { 255.0 / 254.0 };
            let (cb, cr) = ((cb - 128.0) * scale, (cr - 128.0) * scale);
            let rgb = [luma + 1.402 * cr, luma - 0.344136 * cb - 0.714136 * cr, luma + 1.772 * cb];
            for v in rgb {
                // The 4:4:4 path truncates; the subsampled one leaves rounding to Core Image.
                let v = if full { v.floor().clamp(0.0, 255.0) } else { v.clamp(0.0, 255.0) };
                out.push(v / 255.0);
            }
        }
    }
    let known = full || (fx, fy) == (2, 2);
    let approximation = (!known).then(|| format!("JPEG with {fx}x{fy} chroma subsampling: Apple's upsampling isn't reproduced yet"));
    Ok((3, out, approximation))
}

/// 4:2:2 chroma as the 4:2:0 plane Apple's decoder makes of it.
fn halve_rows(p: &Plane, cw: usize, ch: usize) -> Plane {
    let rows = ch.div_ceil(2);
    let mut samples = vec![0.0f32; cw * rows];
    let s = |x: usize, y: usize| p.samples[y.min(ch - 1) * p.width + x];
    for y in 0..rows {
        let pair = |x: usize| (s(x, 2 * y) + s(x, 2 * y + 1)) / 2.0;
        for x in 0..cw {
            let (left, right) = (pair(x.saturating_sub(1)), pair((x + 1).min(cw - 1)));
            samples[y * cw + x] = (left + 6.0 * pair(x) + right) / 8.0;
        }
    }
    Plane { width: cw, samples }
}

/// Chroma at luma sample (x, y): bilinear between chroma sample centers, edges clamped.
fn bilinear(p: &Plane, x: usize, y: usize, fx: usize, fy: usize, cw: usize, ch: usize) -> f32 {
    let axis = |i: usize, f: usize, n: usize| -> (usize, usize, f32) {
        let pos = (i as f32 + 0.5) / f as f32 - 0.5;
        if pos <= 0.0 {
            return (0, 0, 0.0);
        }
        let i0 = pos.floor() as usize;
        if i0 + 1 >= n {
            return (n - 1, n - 1, 0.0);
        }
        (i0, i0 + 1, pos - i0 as f32)
    };
    let (x0, x1, wx) = axis(x, fx, cw);
    let (y0, y1, wy) = axis(y, fy, ch);
    let s = |x: usize, y: usize| p.samples[y * p.width + x];
    let top = s(x0, y0) * (1.0 - wx) + s(x1, y0) * wx;
    let bottom = s(x0, y1) * (1.0 - wx) + s(x1, y1) * wx;
    top * (1.0 - wy) + bottom * wy
}
