//! Deterministic test images. Small canvases keep the corpus light, so the images are built to
//! hit many input values per pixel rather than to look like photographs.

use image::{GrayImage, Luma, Rgba, RgbaImage};

/// A fixed hash, so images depend only on their arguments.
pub fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

fn byte(seed: u32, i: u32) -> u8 {
    (hash(seed.wrapping_mul(0x9e37_79b9) ^ i) >> 11) as u8
}

/// Every pixel a different pseudo-random color, channels independent, so blend formulas see
/// thousands of (backdrop, source) pairs. The first row holds the extremes 0 and 255 in every
/// combination, which is where rounding and clamping differ most often.
pub fn noise(w: u32, h: u32, seed: u32, alpha: Alpha) -> RgbaImage {
    let mut img = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let mut p = [byte(seed, i * 4), byte(seed, i * 4 + 1), byte(seed, i * 4 + 2), 255];
            if y == 0 {
                p = [if x & 1 != 0 { 255 } else { 0 }, if x & 2 != 0 { 255 } else { 0 }, if x & 4 != 0 { 255 } else { 0 }, 255];
            }
            p[3] = match alpha {
                Alpha::Opaque => 255,
                Alpha::Varied => match x % 8 {
                    0 => 0,
                    1 => 255,
                    _ => byte(seed ^ 0xa1fa, i),
                },
            };
            img.put_pixel(x, y, Rgba(p));
        }
    }
    img
}

#[derive(Clone, Copy)]
pub enum Alpha {
    Opaque,
    Varied,
}

/// A picture-like image: a hue sweep left to right, dark to light top to bottom, with a
/// neutral gray strip along the bottom. Adjustments are easiest to read on this.
pub fn photo(w: u32, h: u32) -> RgbaImage {
    let mut img = RgbaImage::new(w, h);
    let gray_rows = h / 8;
    for y in 0..h {
        for x in 0..w {
            let t = x as f64 / (w - 1).max(1) as f64;
            let v = y as f64 / (h - 1).max(1) as f64;
            let p = if y >= h - gray_rows {
                let g = (t * 255.0).round() as u8;
                [g, g, g]
            } else {
                let (r, g, b) = hsv(t * 360.0, 0.25 + 0.75 * (1.0 - v * 0.5), 0.1 + 0.9 * (1.0 - v));
                [r, g, b]
            };
            img.put_pixel(x, y, Rgba([p[0], p[1], p[2], 255]));
        }
    }
    img
}

pub fn hsv(h: f64, s: f64, v: f64) -> (u8, u8, u8) {
    let c = v * s;
    let hp = (h / 60.0) % 6.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let q = |n: f64| ((n + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (q(r), q(g), q(b))
}

pub fn solid(w: u32, h: u32, color: [u8; 4]) -> RgbaImage {
    RgbaImage::from_pixel(w, h, Rgba(color))
}

/// A filled disc with a one-pixel soft edge on a transparent ground: the shape layer effects
/// are drawn around.
pub fn disc(w: u32, h: u32, color: [u8; 3]) -> RgbaImage {
    let mut img = RgbaImage::new(w, h);
    let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
    let r = w.min(h) as f64 * 0.35;
    for y in 0..h {
        for x in 0..w {
            let d = ((x as f64 + 0.5 - cx).powi(2) + (y as f64 + 0.5 - cy).powi(2)).sqrt();
            let a = (r - d + 0.5).clamp(0.0, 1.0);
            img.put_pixel(x, y, Rgba([color[0], color[1], color[2], (a * 255.0).round() as u8]));
        }
    }
    img
}

/// A hard-edged glyph-like shape (an L and a bar), for effects that follow corners.
pub fn corners(w: u32, h: u32, color: [u8; 3]) -> RgbaImage {
    let mut img = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let inside = (x >= w / 5 && x < w * 2 / 5 && y >= h / 5 && y < h * 4 / 5)
                || (x >= w / 5 && x < w * 4 / 5 && y >= h * 3 / 5 && y < h * 4 / 5)
                || (x >= w * 3 / 5 && x < w * 4 / 5 && y >= h / 5 && y < h * 2 / 5);
            if inside {
                img.put_pixel(x, y, Rgba([color[0], color[1], color[2], 255]));
            }
        }
    }
    img
}

/// A checkerboard with a diagonal line, which shows resampling and rotation clearly.
pub fn checker(w: u32, h: u32, cell: u32) -> RgbaImage {
    let mut img = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let on = ((x / cell) + (y / cell)) % 2 == 0;
            let mut p = if on { [230, 60, 40, 255] } else { [30, 90, 220, 255] };
            if x == y || x + 1 == y {
                p = [255, 255, 255, 255];
            }
            img.put_pixel(x, y, Rgba(p));
        }
    }
    img
}

pub fn gray_ramp(w: u32, h: u32, horizontal: bool) -> GrayImage {
    GrayImage::from_fn(w, h, |x, y| {
        let t = if horizontal { x as f64 / (w - 1).max(1) as f64 } else { y as f64 / (h - 1).max(1) as f64 };
        Luma([(t * 255.0).round() as u8])
    })
}

/// A mask with hard and soft regions: black left third, a ramp in the middle, white right third.
pub fn mask_mixed(w: u32, h: u32) -> GrayImage {
    GrayImage::from_fn(w, h, |x, _| {
        let t = x as f64 / w as f64;
        let v = if t < 1.0 / 3.0 {
            0.0
        } else if t > 2.0 / 3.0 {
            1.0
        } else {
            (t - 1.0 / 3.0) * 3.0
        };
        Luma([(v * 255.0).round() as u8])
    })
}

pub fn gray_solid(w: u32, h: u32, v: u8) -> GrayImage {
    GrayImage::from_pixel(w, h, Luma([v]))
}
