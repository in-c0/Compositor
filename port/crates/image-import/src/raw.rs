//! Camera RAW: the develop sheet's settings (`RawDevelopSettings`) and `CIRAWFilter`'s develop.
//!
//! Apple's RAW engine isn't available outside macOS, so this is a DNG develop built from the DNG
//! specification: linearize, demosaic, white balance, the camera's matrix to linear sRGB, exposure,
//! and the sRGB curve. It stands in for the Mac's and is always marked approximate; what the
//! references show of Apple's develop is noted where it is followed.

use crate::color::linear_to_srgb;
use crate::develop::unpremultiply;
use crate::{ImportError, Layer, Result, dng};
use comp_format::RgbaImage;

/// The develop sheet's controls (`RawDevelopSettings`). Temperature and tint left `None` keep the
/// camera's own white balance, as the sheet opens with it.
#[derive(Clone, Debug, Default)]
pub struct RawSettings {
    pub exposure: Option<f32>,
    pub temperature: Option<f32>,
    pub tint: Option<f32>,
    pub boost: Option<f32>,
}

/// `RawImporter.matches`: the extensions macOS types as camera RAW images.
pub(crate) fn matches(extension: &str) -> bool {
    matches!(
        extension,
        "dng" | "cr2" | "cr3" | "crw" | "nef" | "nrw" | "arw" | "srf" | "sr2" | "raf" | "orf" | "rw2" | "raw" | "pef" | "srw" | "x3f"
            | "erf" | "mrw" | "mos" | "3fr" | "fff" | "iiq" | "dcr" | "kdc" | "rwl" | "mef" | "k25" | "dcs" | "gpr" | "nksc"
    )
}

type Matrix = [[f64; 3]; 3];

fn mul(m: &Matrix, v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|r| m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2])
}

fn mat_mul(a: &Matrix, b: &Matrix) -> Matrix {
    [0, 1, 2].map(|r| [0, 1, 2].map(|c| a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c]))
}

fn invert(m: &Matrix) -> Option<Matrix> {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if det.abs() < 1e-12 {
        return None;
    }
    let c = |r0: usize, c0: usize, r1: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    Some([
        [c(1, 1, 2, 2) / det, -c(0, 1, 2, 2) / det, c(0, 1, 1, 2) / det],
        [-c(1, 0, 2, 2) / det, c(0, 0, 2, 2) / det, -c(0, 0, 1, 2) / det],
        [c(1, 0, 2, 1) / det, -c(0, 0, 2, 1) / det, c(0, 0, 1, 1) / det],
    ])
}

const XYZ_TO_SRGB: Matrix = [[3.2404542, -1.5371385, -0.4985314], [-0.9692660, 1.8760108, 0.0415560], [0.0556434, -0.2040259, 1.0572252]];

/// Temperature and tint to chromaticity, as the DNG SDK computes it (Robertson's isotemperature
/// lines; tint in units of 1/3000 of the line's normal).
pub(crate) fn temperature_to_xy(temperature: f64, tint: f64) -> [f64; 2] {
    const TABLE: [[f64; 4]; 31] = [
        [0.0, 0.18006, 0.26352, -0.24341],
        [10.0, 0.18066, 0.26589, -0.25479],
        [20.0, 0.18133, 0.26846, -0.26876],
        [30.0, 0.18208, 0.27119, -0.28539],
        [40.0, 0.18293, 0.27407, -0.30470],
        [50.0, 0.18388, 0.27709, -0.32675],
        [60.0, 0.18494, 0.28021, -0.35156],
        [70.0, 0.18611, 0.28342, -0.37915],
        [80.0, 0.18740, 0.28668, -0.40955],
        [90.0, 0.18880, 0.28997, -0.44278],
        [100.0, 0.19032, 0.29326, -0.47888],
        [125.0, 0.19462, 0.30141, -0.58204],
        [150.0, 0.19962, 0.30921, -0.70471],
        [175.0, 0.20525, 0.31647, -0.84901],
        [200.0, 0.21142, 0.32312, -1.0182],
        [225.0, 0.21807, 0.32909, -1.2168],
        [250.0, 0.22511, 0.33439, -1.4512],
        [275.0, 0.23247, 0.33904, -1.7298],
        [300.0, 0.24010, 0.34308, -2.0637],
        [325.0, 0.24702, 0.34655, -2.4681],
        [350.0, 0.25591, 0.34951, -2.9641],
        [375.0, 0.26400, 0.35200, -3.5814],
        [400.0, 0.27218, 0.35407, -4.3633],
        [425.0, 0.28039, 0.35577, -5.3762],
        [450.0, 0.28863, 0.35714, -6.7262],
        [475.0, 0.29685, 0.35823, -8.5955],
        [500.0, 0.30505, 0.35907, -11.324],
        [525.0, 0.31320, 0.35968, -15.628],
        [550.0, 0.32129, 0.36011, -23.325],
        [575.0, 0.32931, 0.36038, -40.770],
        [600.0, 0.33724, 0.36051, -116.45],
    ];
    let r = 1.0e6 / temperature;
    let offset = tint / -3000.0;
    let mut index = 0;
    while index < 29 && r >= TABLE[index + 1][0] {
        index += 1;
    }
    let (a, b) = (TABLE[index], TABLE[index + 1]);
    let f = (b[0] - r) / (b[0] - a[0]);
    let mut u = a[1] * f + b[1] * (1.0 - f);
    let mut v = a[2] * f + b[2] * (1.0 - f);
    let normal = |t: f64| {
        let len = (1.0 + t * t).sqrt();
        (1.0 / len, t / len)
    };
    let ((u1, v1), (u2, v2)) = (normal(a[3]), normal(b[3]));
    let (mut u3, mut v3) = (u1 * f + u2 * (1.0 - f), v1 * f + v2 * (1.0 - f));
    let len = (u3 * u3 + v3 * v3).sqrt();
    u3 /= len;
    v3 /= len;
    u += u3 * offset;
    v += v3 * offset;
    [1.5 * u / (u - 4.0 * v + 2.0), v / (u - 4.0 * v + 2.0)]
}

pub(crate) fn develop(data: &[u8], settings: RawSettings) -> Result<Layer> {
    let raw = dng::read(data)?;
    let color_matrix = raw.color_matrix.ok_or_else(|| ImportError::NotPorted("DNG without ColorMatrix1".into()))?;
    let to_xyz = invert(&color_matrix).ok_or(ImportError::Unreadable)?;
    let (w, h, spp) = (raw.width, raw.height, raw.samples_per_pixel);
    // Levels per sample (LinearRaw) or per CFA position. The references show Apple ignoring a
    // BlackLevel whose count doesn't follow the specification, as `dng::read` does.
    let level = |levels: &[f64], i: usize, default: f64| if levels.is_empty() { default } else { levels[i % levels.len()] };
    let mut camera = vec![0.0f64; w * h * 3];
    match raw.cfa {
        None => {
            for (i, &s) in raw.samples.iter().enumerate() {
                let c = i % spp;
                let (black, white) = (level(&raw.black, c, 0.0), level(&raw.white, c, 65535.0));
                camera[i] = (s as f64 - black) / (white - black);
            }
        }
        Some(pattern) => {
            let mut mosaic = vec![0.0f64; w * h];
            for y in 0..h {
                for x in 0..w {
                    let cell = (y % 2) * 2 + x % 2;
                    let (black, white) = (level(&raw.black, cell, 0.0), level(&raw.white, 0, 65535.0));
                    mosaic[y * w + x] = (raw.samples[y * w + x] as f64 - black) / (white - black);
                }
            }
            demosaic(&mosaic, w, h, pattern, &mut camera);
        }
    }
    let as_shot = raw.as_shot_neutral.unwrap_or([1.0; 3]);
    let neutral = match (settings.temperature, settings.tint) {
        (None, None) => as_shot,
        (temperature, tint) => {
            let (t0, n0) = as_shot_temperature(&to_xyz, as_shot);
            let xy = temperature_to_xy(temperature.map_or(t0, |t| t as f64), tint.map_or(n0, |t| t as f64));
            let xyz = [xy[0] / xy[1], 1.0, (1.0 - xy[0] - xy[1]) / xy[1]];
            let n = mul(&color_matrix, xyz);
            let peak = n.iter().cloned().fold(f64::MIN, f64::max);
            n.map(|c| c / peak)
        }
    };
    // Camera to linear sRGB, scaled so the neutral comes out white.
    let to_srgb = mat_mul(&XYZ_TO_SRGB, &to_xyz);
    let white = mul(&to_srgb, neutral);
    let gain = 2f64.powf(settings.exposure.unwrap_or(0.0) as f64 + raw.baseline_exposure);
    let mut pixels = RgbaImage::new(w as u32, h as u32);
    for (i, p) in pixels.pixels_mut().enumerate() {
        let lin = mul(&to_srgb, [camera[i * 3], camera[i * 3 + 1], camera[i * 3 + 2]]);
        let rgb = [0, 1, 2].map(|c| (linear_to_srgb(lin[c] / white[c] * gain).clamp(0.0, 1.0) * 255.0).round() as u8);
        p.0 = unpremultiply([rgb[0], rgb[1], rgb[2], 255]);
    }
    if raw.orientation != 1 {
        return Err(ImportError::NotPorted(format!("DNG orientation {}", raw.orientation)));
    }
    let boost = settings.boost.unwrap_or(1.0);
    Ok(Layer {
        pixels,
        approximation: Some(format!(
            "camera RAW develop: Apple's RAW engine (demosaic, color, tone curve at boost {boost}) is approximated from the DNG specification"
        )),
        notes: Vec::new(),
    })
}

/// The temperature and tint of the as-shot neutral, found by search (the DNG SDK's inverse).
fn as_shot_temperature(to_xyz: &Matrix, neutral: [f64; 3]) -> (f64, f64) {
    let xyz = mul(to_xyz, neutral);
    let sum = xyz[0] + xyz[1] + xyz[2];
    let target = [xyz[0] / sum, xyz[1] / sum];
    let (mut best, mut best_error) = ((5000.0, 0.0), f64::MAX);
    let mut t = 2000.0;
    while t <= 50000.0 {
        for n in -150..=150 {
            let xy = temperature_to_xy(t, n as f64);
            let e = (xy[0] - target[0]).powi(2) + (xy[1] - target[1]).powi(2);
            if e < best_error {
                best_error = e;
                best = (t, n as f64);
            }
        }
        t *= 1.01;
    }
    best
}

/// Bilinear demosaic of a 2 x 2 color filter array.
fn demosaic(mosaic: &[f64], w: usize, h: usize, pattern: [u8; 4], out: &mut [f64]) {
    let at = |x: isize, y: isize| {
        let (x, y) = (x.clamp(0, w as isize - 1) as usize, y.clamp(0, h as isize - 1) as usize);
        (mosaic[y * w + x], pattern[(y % 2) * 2 + x % 2])
    };
    for y in 0..h as isize {
        for x in 0..w as isize {
            let mut sum = [0.0f64; 3];
            let mut count = [0.0f64; 3];
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (v, c) = at(x + dx, y + dy);
                    let weight = if dx == 0 && dy == 0 { 4.0 } else if dx == 0 || dy == 0 { 2.0 } else { 1.0 };
                    sum[c as usize] += v * weight;
                    count[c as usize] += weight;
                }
            }
            let (own, c) = at(x, y);
            let i = (y as usize * w + x as usize) * 3;
            for k in 0..3 {
                out[i + k] = if k == c as usize { own } else if count[k] > 0.0 { sum[k] / count[k] } else { 0.0 };
            }
        }
    }
}
