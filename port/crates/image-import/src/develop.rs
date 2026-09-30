//! The Core Image step of `ImageImporter.decode`: the decoded image turned upright
//! (`CIImage.oriented(forExifOrientation:)`) and rendered into 8-bit premultiplied sRGB
//! (`CIContext.createCGImage(…, format: .RGBA8, colorSpace: sRGB)`). The project then stores the
//! layer unpremultiplied, as ImageIO writes a premultiplied image to PNG.

use crate::color::{self, Space};
use crate::{Layer, Result};
use comp_format::RgbaImage;

/// Samples as the file stores them, interleaved, full range.
pub(crate) enum Samples {
    U8(Vec<u8>),
    U16(Vec<u16>),
    /// Already in 0-1 and not rounded, as HEIC's YCbCr to RGB conversion leaves them.
    F32(Vec<f32>),
}

impl Samples {
    fn get(&self, i: usize) -> f64 {
        match self {
            Samples::U8(v) => v[i] as f64 / 255.0,
            Samples::U16(v) => v[i] as f64 / 65535.0,
            Samples::F32(v) => v[i] as f64,
        }
    }
}

/// A decoded image, before Core Image.
pub(crate) struct Source {
    pub width: u32,
    pub height: u32,
    /// Color channels: 1 (gray) or 3 (RGB).
    pub colors: usize,
    pub alpha: bool,
    /// The color channels are already multiplied by alpha (TIFF's associated alpha).
    pub premultiplied: bool,
    pub samples: Samples,
    pub space: Space,
    /// EXIF orientation, 1-8.
    pub orientation: u16,
    pub approximation: Option<String>,
}

pub(crate) fn develop(source: Source) -> Result<Layer> {
    let (w, h) = (source.width as usize, source.height as usize);
    let channels = source.colors + source.alpha as usize;
    let (ow, oh) = if source.orientation >= 5 { (h, w) } else { (w, h) };
    let mut out = RgbaImage::new(ow as u32, oh as u32);
    let mut px = [0.0f64; 3];
    for oy in 0..oh {
        for ox in 0..ow {
            let (x, y) = source_position(source.orientation, ox, oy, w, h);
            let base = (y * w + x) * channels;
            let a = if source.alpha { source.samples.get(base + source.colors) } else { 1.0 };
            for c in 0..source.colors {
                px[c] = source.samples.get(base + c);
                if source.premultiplied {
                    px[c] = if a > 0.0 { px[c] / a } else { 0.0 };
                }
            }
            let linear = source.space.to_linear_srgb(&px[..source.colors.max(3)]);
            let alpha = (a * 255.0).round();
            let mut p = [0u8; 4];
            for c in 0..3 {
                let encoded = color::linear_to_srgb(linear[c]).clamp(0.0, 1.0);
                p[c] = (encoded * a * 255.0).round() as u8;
            }
            p[3] = alpha as u8;
            out.put_pixel(ox as u32, oy as u32, image::Rgba(unpremultiply(p)));
        }
    }
    Ok(Layer { pixels: out, approximation: source.approximation, notes: Vec::new() })
}

/// The stored (straight) pixel for a premultiplied one, as ImageIO writes a PNG.
pub(crate) fn unpremultiply(p: [u8; 4]) -> [u8; 4] {
    let a = p[3] as u32;
    match a {
        0 => [0, 0, 0, 0],
        255 => p,
        _ => {
            let u = |c: u8| ((c as u32 * 255 + a / 2) / a).min(255) as u8;
            [u(p[0]), u(p[1]), u(p[2]), p[3]]
        }
    }
}

/// Which stored pixel lands at (x, y) of the upright image, for EXIF orientation `o`.
fn source_position(o: u16, x: usize, y: usize, w: usize, h: usize) -> (usize, usize) {
    match o {
        2 => (w - 1 - x, y),
        3 => (w - 1 - x, h - 1 - y),
        4 => (x, h - 1 - y),
        5 => (y, x),
        6 => (y, h - 1 - x),
        7 => (w - 1 - y, h - 1 - x),
        8 => (w - 1 - y, x),
        _ => (x, y),
    }
}
