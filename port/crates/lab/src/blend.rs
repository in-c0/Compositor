//! Candidate models of how the Mac composites one layer over the canvas in each blend mode.
//!
//! Colors are 0...1 floats here; the experiments quantize where a candidate says Core Graphics or
//! Core Image stores bytes.

use comp_format::BlendMode;

/// The separable blend function B(backdrop, source) on straight colors, per the PDF / W3C
/// compositing spec and the usual definitions for the modes Core Image adds.
pub fn separable(mode: BlendMode, b: f64, s: f64) -> Option<f64> {
    use BlendMode::*;
    Some(match mode {
        Normal => s,
        Multiply => b * s,
        Screen => b + s - b * s,
        Overlay => hard_light(s, b),
        Darken => b.min(s),
        Lighten => b.max(s),
        ColorDodge => {
            if b == 0.0 {
                0.0
            } else if s >= 1.0 {
                1.0
            } else {
                (b / (1.0 - s)).min(1.0)
            }
        }
        ColorBurn => {
            if b >= 1.0 {
                1.0
            } else if s <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - b) / s).min(1.0)
            }
        }
        // Core Graphics multiplies up to and including a source byte of 128, a little past 0.5.
        HardLight => {
            if s <= 128.0 / 255.0 + 1e-9 { b * 2.0 * s } else { let t = 2.0 * s - 1.0; b + t - b * t }
        }
        SoftLight => {
            if s <= 0.5 {
                b - (1.0 - 2.0 * s) * b * (1.0 - b)
            } else {
                let d = if b <= 0.25 { ((16.0 * b - 12.0) * b + 4.0) * b } else { b.sqrt() };
                b + (2.0 * s - 1.0) * (d - b)
            }
        }
        Difference => (b - s).abs(),
        Exclusion => b + s - 2.0 * b * s,
        LinearBurn => (b + s - 1.0).max(0.0),
        LinearDodge => (b + s).min(1.0),
        VividLight => {
            if s <= 0.5 {
                separable(ColorBurn, b, 2.0 * s)?
            } else {
                separable(ColorDodge, b, 2.0 * (s - 0.5))?
            }
        }
        LinearLight => (b + 2.0 * s - 1.0).clamp(0.0, 1.0),
        PinLight => {
            if s <= 0.5 {
                b.min(2.0 * s)
            } else {
                b.max(2.0 * s - 1.0)
            }
        }
        HardMix => {
            if b + s > 1.0 + 1e-9 {
                1.0
            } else {
                0.0
            }
        }
        Subtract => (b - s).max(0.0),
        Divide => {
            if s <= 0.0 {
                if b > 0.0 { 1.0 } else { 0.0 }
            } else {
                (b / s).min(1.0)
            }
        }
        Hue | Saturation | Color | Luminosity => return None,
    })
}

fn hard_light(b: f64, s: f64) -> f64 {
    if s <= 0.5 { b * 2.0 * s } else { let t = 2.0 * s - 1.0; b + t - b * t }
}

pub static LUM: std::sync::RwLock<[f64; 3]> = std::sync::RwLock::new([77.0 / 256.0, 151.0 / 256.0, 28.0 / 256.0]);

fn lum(c: [f64; 3]) -> f64 {
    let w = *LUM.read().unwrap();
    w[0] * c[0] + w[1] * c[1] + w[2] * c[2]
}

fn clip_color(c: [f64; 3]) -> [f64; 3] {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut out = c;
    if n < 0.0 {
        out = out.map(|v| l + (v - l) * l / (l - n));
    }
    if x > 1.0 {
        out = out.map(|v| l + (v - l) * (1.0 - l) / (x - l));
    }
    out
}

fn set_lum(c: [f64; 3], l: f64) -> [f64; 3] {
    let d = l - lum(c);
    clip_color(c.map(|v| v + d))
}

fn sat(c: [f64; 3]) -> f64 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

fn set_sat(c: [f64; 3], s: f64) -> [f64; 3] {
    let mut idx = [0usize, 1, 2];
    idx.sort_by(|&a, &b| c[a].partial_cmp(&c[b]).unwrap());
    let (min, mid, max) = (idx[0], idx[1], idx[2]);
    let mut out = [0.0; 3];
    if c[max] > c[min] {
        out[mid] = (c[mid] - c[min]) * s / (c[max] - c[min]);
        out[max] = s;
    }
    out
}

/// The non-separable modes, on straight colors.
pub fn non_separable(mode: BlendMode, b: [f64; 3], s: [f64; 3]) -> Option<[f64; 3]> {
    use BlendMode::*;
    Some(match mode {
        Hue => set_lum(set_sat(s, sat(b)), lum(b)),
        Saturation => set_lum(set_sat(b, sat(s)), lum(b)),
        Color => set_lum(s, lum(b)),
        Luminosity => set_lum(b, lum(s)),
        _ => return None,
    })
}

/// Straight colors -> the blended straight color, any mode.
pub fn blend(mode: BlendMode, b: [f64; 3], s: [f64; 3]) -> [f64; 3] {
    if let Some(c) = non_separable(mode, b, s) {
        return c;
    }
    [0, 1, 2].map(|i| separable(mode, b[i], s[i]).unwrap())
}

/// W3C general compositing in premultiplied form: returns premultiplied RGBA in 0...1.
/// `b` and `s` are straight colors with their alphas (the source alpha already includes opacity).
pub fn composite(mode: BlendMode, b: [f64; 4], s: [f64; 4]) -> [f64; 4] {
    let (ab, as_) = (b[3], s[3]);
    let mixed = blend(mode, [b[0], b[1], b[2]], [s[0], s[1], s[2]]);
    let ao = as_ + ab * (1.0 - as_);
    let mut out = [0.0; 4];
    for i in 0..3 {
        out[i] = as_ * (1.0 - ab) * s[i] + as_ * ab * mixed[i] + (1.0 - as_) * ab * b[i];
    }
    out[3] = ao;
    out
}

// Core Graphics' non-separable modes, on straight bytes. Luminance is (77 R + 151 G + 28 B) / 256,
// and SetLum and SetSat use 16.16 fixed-point ratios; fitted exactly to the Mac references.

fn lum256(c: [i64; 3]) -> i64 {
    77 * c[0] + 151 * c[1] + 28 * c[2]
}

fn clip_color_int(c: [i64; 3]) -> [i64; 3] {
    let l = (lum256(c) + 128) >> 8;
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut out = c;
    if n < 0 {
        let k = (l << 16).div_euclid(l - n);
        out = out.map(|v| l + (((v - l) * k + 0x8000) >> 16));
    }
    if x > 255 {
        let k = ((255 - l) << 16).div_euclid(x - l);
        out = out.map(|v| l + (((v - l) * k + 0x8000) >> 16));
    }
    out
}

fn set_lum_int(c: [i64; 3], target: [i64; 3]) -> [i64; 3] {
    let d = (lum256(target) - lum256(c) + 128) >> 8;
    clip_color_int(c.map(|v| v + d))
}

fn set_sat_int(c: [i64; 3], s: i64) -> [i64; 3] {
    let mut idx = [0usize, 1, 2];
    idx.sort_by_key(|&i| c[i]);
    let (mn, md, mx) = (idx[0], idx[1], idx[2]);
    let mut out = [0; 3];
    let den = c[mx] - c[mn];
    if den > 0 {
        out[md] = ((c[md] - c[mn]) * ((s << 16) / den) + 0x8000) >> 16;
        out[mx] = s;
    }
    out
}

pub fn non_separable_int(mode: BlendMode, b: [i64; 3], s: [i64; 3]) -> Option<[i64; 3]> {
    use BlendMode::*;
    let sat = |c: [i64; 3]| c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2]);
    Some(match mode {
        Hue => set_lum_int(set_sat_int(s, sat(b)), b),
        Saturation => set_lum_int(set_sat_int(b, sat(s)), b),
        Color => set_lum_int(s, b),
        Luminosity => set_lum_int(b, s),
        _ => return None,
    })
}
