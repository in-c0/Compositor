//! Color spaces an imported image can arrive in, and the conversion to linear sRGB that Core Image
//! makes before it writes the layer (`CIContext.createCGImage(…, colorSpace: sRGB)`).
//!
//! Core Image converts matrix/TRC spaces itself, in float: each channel through the space's tone
//! curve, then one 3x3 matrix to linear sRGB, then the sRGB curve on the way out. That is what
//! [`Space::Matrix`] does, in f64. Spaces built from lookup tables (CMYK, most printer profiles) go
//! through ColorSync, which the port doesn't have; those use moxcms and are marked approximate.

use crate::{ImportError, Result};

/// A tone curve, from encoded to linear.
#[derive(Clone, Debug)]
pub(crate) enum Curve {
    Srgb,
    Gamma(f64),
    /// ICC `para` parameters (function type 0-4).
    Parametric(Vec<f64>),
    /// ICC `curv` table, linearly interpolated.
    Table(Vec<u16>),
}

impl Curve {
    pub fn eval(&self, v: f64) -> f64 {
        match self {
            Curve::Srgb => srgb_to_linear(v),
            Curve::Gamma(g) => signed_pow(v, *g),
            Curve::Parametric(p) => parametric(p, v),
            Curve::Table(t) => {
                if t.len() < 2 {
                    return v;
                }
                let x = v.clamp(0.0, 1.0) * (t.len() - 1) as f64;
                let i = (x.floor() as usize).min(t.len() - 2);
                let f = x - i as f64;
                (t[i] as f64 * (1.0 - f) + t[i + 1] as f64 * f) / 65535.0
            }
        }
    }

    fn from_moxcms(curve: &moxcms::ToneReprCurve) -> Curve {
        match curve {
            moxcms::ToneReprCurve::Lut(t) if t.is_empty() => Curve::Gamma(1.0),
            moxcms::ToneReprCurve::Lut(t) if t.len() == 1 => Curve::Gamma(t[0] as f64 / 256.0),
            moxcms::ToneReprCurve::Lut(t) => Curve::Table(t.clone()),
            moxcms::ToneReprCurve::Parametric(p) => Curve::Parametric(p.iter().map(|&v| v as f64).collect()),
        }
    }
}

fn signed_pow(v: f64, g: f64) -> f64 {
    if v < 0.0 { -(-v).powf(g) } else { v.powf(g) }
}

fn parametric(p: &[f64], x: f64) -> f64 {
    let g = p.first().copied().unwrap_or(1.0);
    match p.len() {
        1 => signed_pow(x, g),
        3 => {
            let (a, b) = (p[1], p[2]);
            if x >= -b / a { signed_pow(a * x + b, g) } else { 0.0 }
        }
        4 => {
            let (a, b, c) = (p[1], p[2], p[3]);
            if x >= -b / a { signed_pow(a * x + b, g) + c } else { c }
        }
        5 => {
            let (a, b, c, d) = (p[1], p[2], p[3], p[4]);
            if x >= d { signed_pow(a * x + b, g) } else { c * x }
        }
        7 => {
            let (a, b, c, d, e, f) = (p[1], p[2], p[3], p[4], p[5], p[6]);
            if x >= d { signed_pow(a * x + b, g) + e } else { c * x + f }
        }
        _ => x,
    }
}

pub(crate) fn srgb_to_linear(v: f64) -> f64 {
    let a = v.abs();
    let l = if a <= 0.04045 { a / 12.92 } else { ((a + 0.055) / 1.055).powf(2.4) };
    l.copysign(v)
}

pub(crate) fn linear_to_srgb(l: f64) -> f64 {
    let a = l.abs();
    let v = if a <= 0.0031308 { a * 12.92 } else { 1.055 * a.powf(1.0 / 2.4) - 0.055 };
    v.copysign(l)
}

/// Where a decoded image's samples are.
#[derive(Clone, Debug)]
pub(crate) enum Space {
    /// sRGB: untagged RGB, an sRGB chunk, or a profile that is sRGB.
    Srgb,
    /// Gray with this tone curve.
    Gray(Curve),
    /// RGB through tone curves, then a matrix to linear sRGB.
    Matrix { curves: [Curve; 3], to_srgb: [[f64; 3]; 3] },
}

impl Space {
    /// One pixel's color (0-1 per channel) in linear sRGB, unclamped.
    pub fn to_linear_srgb(&self, px: &[f64]) -> [f64; 3] {
        match self {
            Space::Srgb => [srgb_to_linear(px[0]), srgb_to_linear(px[1]), srgb_to_linear(px[2])],
            Space::Gray(curve) => {
                let l = curve.eval(px[0]);
                [l, l, l]
            }
            Space::Matrix { curves, to_srgb } => {
                let l = [curves[0].eval(px[0]), curves[1].eval(px[1]), curves[2].eval(px[2])];
                [0, 1, 2].map(|r| to_srgb[r][0] * l[0] + to_srgb[r][1] * l[1] + to_srgb[r][2] * l[2])
            }
        }
    }

    /// The space an embedded ICC profile describes, for an image with `channels` color channels.
    pub fn from_icc(icc: &[u8], channels: usize) -> Result<Space> {
        let profile = moxcms::ColorProfile::new_from_slice(icc).map_err(|_| ImportError::NotPorted("an ICC profile moxcms can't read".into()))?;
        match (channels, profile.color_space) {
            (1, moxcms::DataColorSpace::Gray) => {
                let curve = profile.gray_trc.as_ref().map(Curve::from_moxcms).unwrap_or(Curve::Gamma(1.0));
                Ok(Space::Gray(curve))
            }
            (3, moxcms::DataColorSpace::Rgb) => {
                let (Some(r), Some(g), Some(b)) = (&profile.red_trc, &profile.green_trc, &profile.blue_trc) else {
                    return Err(ImportError::NotPorted("an RGB profile without tone curves (lookup tables need ColorSync)".into()));
                };
                let c = |x: moxcms::Xyzd| [x.x, x.y, x.z];
                let (rc, gc, bc) = (c(profile.red_colorant), c(profile.green_colorant), c(profile.blue_colorant));
                let to_pcs = [[rc[0], gc[0], bc[0]], [rc[1], gc[1], bc[1]], [rc[2], gc[2], bc[2]]];
                let to_srgb = mat_mul(&invert(&SRGB_TO_PCS), &to_pcs);
                Ok(Space::Matrix { curves: [Curve::from_moxcms(r), Curve::from_moxcms(g), Curve::from_moxcms(b)], to_srgb })
            }
            _ => Err(ImportError::NotPorted("an ICC profile that doesn't match the image's channels".into())),
        }
    }

    /// PNG's gAMA and cHRM, as ImageIO turns them into a color space.
    pub fn from_gamma(gamma: f64, chromaticities: Option<[[f64; 2]; 4]>) -> Space {
        let curve = Curve::Gamma(1.0 / gamma);
        match chromaticities {
            None => Space::Matrix { curves: [curve.clone(), curve.clone(), curve], to_srgb: IDENTITY },
            Some([w, r, g, b]) => {
                let to_pcs = mat_mul(&bradford(xyz_of(w), D50), &rgb_to_xyz([r, g, b], xyz_of(w)));
                Space::Matrix { curves: [curve.clone(), curve.clone(), curve], to_srgb: mat_mul(&invert(&SRGB_TO_PCS), &to_pcs) }
            }
        }
    }
}

const IDENTITY: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
const D50: [f64; 3] = [0.9642, 1.0, 0.8249];

/// Linear sRGB to the D50 PCS: the colorants of the sRGB profile.
const SRGB_TO_PCS: [[f64; 3]; 3] = [
    [0.436_065_673_828_125, 0.385_147_094_726_562_5, 0.143_066_406_25],
    [0.222_488_403_320_312_5, 0.716_873_168_945_312_5, 0.060_607_910_156_25],
    [0.013_916_015_625, 0.097_076_416_015_625, 0.714_096_069_335_937_5],
];

fn xyz_of(xy: [f64; 2]) -> [f64; 3] {
    [xy[0] / xy[1], 1.0, (1.0 - xy[0] - xy[1]) / xy[1]]
}

fn mul(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|r| m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2])
}

fn mat_mul(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    [0, 1, 2].map(|r| [0, 1, 2].map(|c| a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c]))
}

fn invert(m: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let c = |r0: usize, c0: usize, r1: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    [
        [c(1, 1, 2, 2) / det, -c(0, 1, 2, 2) / det, c(0, 1, 1, 2) / det],
        [-c(1, 0, 2, 2) / det, c(0, 0, 2, 2) / det, -c(0, 0, 1, 2) / det],
        [c(1, 0, 2, 1) / det, -c(0, 0, 2, 1) / det, c(0, 0, 1, 1) / det],
    ]
}

fn rgb_to_xyz(primaries: [[f64; 2]; 3], white: [f64; 3]) -> [[f64; 3]; 3] {
    let p = primaries.map(xyz_of);
    let m = [[p[0][0], p[1][0], p[2][0]], [p[0][1], p[1][1], p[2][1]], [p[0][2], p[1][2], p[2][2]]];
    let s = mul(&invert(&m), white);
    [0, 1, 2].map(|r| [m[r][0] * s[0], m[r][1] * s[1], m[r][2] * s[2]])
}

fn bradford(from: [f64; 3], to: [f64; 3]) -> [[f64; 3]; 3] {
    let b = [[0.8951, 0.2664, -0.1614], [-0.7502, 1.7135, 0.0367], [0.0389, -0.0685, 1.0296]];
    let (f, t) = (mul(&b, from), mul(&b, to));
    let d = [[t[0] / f[0], 0.0, 0.0], [0.0, t[1] / f[1], 0.0], [0.0, 0.0, t[2] / f[2]]];
    mat_mul(&invert(&b), &mat_mul(&d, &b))
}
