//! Probes for Core Image's Gaussian and motion blurs, run as adjustment layers over a canvas
//! filled by one image. Each color channel holds its own pattern, since the blurs treat channels
//! independently: lines and steps whose brightness ramps along them, so the 8-bit references
//! round the same kernel weight at many different scales and the weights can be read off far more
//! finely than 1/255.

use super::builder::{CaseWriter, LayerSpec};
use anyhow::Result;
use comp_format::*;
use image::{Rgba, RgbaImage};

/// Brightness `255 - slope * t`, which stays above about 64 on the canvas sizes used here.
fn ramp(n: u32, t: u32) -> u8 {
    let slope = (128 / n).max(1);
    (255 - slope * t) as u8
}

/// Opaque black with, in red, a one-pixel vertical line at `c`; in green, a step up at column `c`
/// (both ramping down the rows); in blue, a horizontal line at row `c` ramping along the columns.
fn lines(n: u32, c: u32) -> RgbaImage {
    RgbaImage::from_fn(n, n, |x, y| {
        let r = if x == c { ramp(n, y) } else { 0 };
        let g = if x >= c { ramp(n, y) } else { 0 };
        let b = if y == c { ramp(n, x) } else { 0 };
        Rgba([r, g, b, 255])
    })
}

/// Opaque black with, in red, a single full-brightness pixel at (`c`, `c`); in green, a step up at
/// column `c` ramping down the rows; in blue, a step down at row `c` ramping along the columns.
fn impulse(n: u32, c: u32) -> RgbaImage {
    RgbaImage::from_fn(n, n, |x, y| {
        let r = if x == c && y == c { 255 } else { 0 };
        let g = if x >= c { ramp(n, y) } else { 0 };
        let b = if y >= c { ramp(n, x) } else { 0 };
        Rgba([r, g, b, 255])
    })
}

/// Transparent, with an opaque white pixel, an opaque orange pixel and, across the lower half,
/// orange whose alpha ramps from 3 to 255 left to right.
fn translucent(n: u32) -> RgbaImage {
    RgbaImage::from_fn(n, n, |x, y| {
        if (x, y) == (n / 4, n / 4) {
            Rgba([255, 255, 255, 255])
        } else if (x, y) == (n * 3 / 4, n / 4) {
            Rgba([200, 100, 50, 255])
        } else if y >= n / 2 {
            Rgba([255, 128, 0, (x * 255 / (n - 1)).max(3) as u8])
        } else {
            Rgba([0, 0, 0, 0])
        }
    })
}

fn size(sigma: f64) -> u32 {
    match sigma {
        s if s <= 3.0 => 64,
        s if s <= 8.0 => 96,
        s if s <= 12.0 => 128,
        _ => 192,
    }
}

fn case(w: &mut CaseWriter, name: &str, label: &str, pixels: RgbaImage, adjustment: Adjustment) -> Result<()> {
    let n = pixels.width();
    let mut d = w.doc("adjust", name, n, n);
    d.image("Probe", pixels, LayerSpec::default());
    d.adjustment(label, adjustment, LayerSpec::default());
    w.write("adjust", name, label, d, vec![])
}

fn gaussian(sigma: f64) -> Adjustment {
    let mut a = Adjustment::new(AdjustmentKind::GaussianBlur);
    a.blur_radius = Some(sigma);
    a
}

fn motion(angle: f64, distance: f64) -> Adjustment {
    let mut a = Adjustment::new(AdjustmentKind::MotionBlur);
    a.motion_angle = Some(angle);
    a.motion_distance = Some(distance);
    a
}

pub fn blur_probes(w: &mut CaseWriter) -> Result<()> {
    for sigma in [0.5, 1.0, 2.0, 3.0, 5.0, 8.0, 12.0, 20.0] {
        let n = size(sigma);
        let label = format!("Gaussian blur {sigma} over ramped lines and a step");
        case(w, &format!("probe-gaussian-{sigma}"), &label, lines(n, n / 2), gaussian(sigma))?;
        if sigma >= 2.0 {
            let label = format!("Gaussian blur {sigma} over ramped lines and a step, one pixel on");
            case(w, &format!("probe-gaussian-{sigma}-odd"), &label, lines(n, n / 2 + 1), gaussian(sigma))?;
        }
    }
    for sigma in [1.0, 3.0, 8.0] {
        let label = format!("Gaussian blur {sigma} over translucent pixels");
        case(w, &format!("probe-alpha-gaussian-{sigma}"), &label, translucent(64), gaussian(sigma))?;
    }
    let label = "Motion blur 45 degrees, 20 px over translucent pixels";
    case(w, "probe-alpha-motion-45-20", label, translucent(64), motion(45.0, 20.0))?;
    let motions = [(0.0, 3.0), (0.0, 10.0), (0.0, 20.0), (0.0, 40.0), (30.0, 10.0), (30.0, 20.0), (45.0, 10.0), (45.0, 20.0), (45.0, 40.0), (90.0, 10.0), (90.0, 20.0)];
    for (angle, distance) in motions {
        let n = if distance > 20.0 { 128 } else { 64 };
        let label = format!("Motion blur {angle} degrees, {distance} px over an impulse and ramped steps");
        case(w, &format!("probe-motion-{angle}-{distance}"), &label, impulse(n, n / 2), motion(angle, distance))?;
    }
    Ok(())
}
