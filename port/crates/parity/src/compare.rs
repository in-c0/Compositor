use anyhow::{Context, Result, bail};
use image::{Rgba, RgbaImage};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const DEFAULT_MAX_CHANNEL_DIFF: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pass,
    Fail,
    Pending,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaseResult {
    pub id: String,
    pub feature: String,
    pub label: String,
    pub status: Status,
    /// The limit this case was held to.
    pub tolerance: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_channel_diff: Option<u8>,
    /// Pixels with any channel over the tolerance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub differing_pixels: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_pixels: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heatmap: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Tolerances {
    #[serde(default = "default_limit")]
    pub default_max_channel_diff: u8,
    #[serde(default, rename = "override")]
    pub overrides: Vec<Override>,
}

fn default_limit() -> u8 {
    DEFAULT_MAX_CHANNEL_DIFF
}

#[derive(Debug, Deserialize)]
pub struct Override {
    pub cases: String,
    pub max_channel_diff: u8,
    /// Pixels allowed over `max_channel_diff`, for gaps confined to a few measured pixels.
    #[serde(default)]
    pub max_pixels_over: u64,
    pub reason: String,
}

/// The limits one case is held to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limit {
    pub max_channel_diff: u8,
    pub max_pixels_over: u64,
}

impl Tolerances {
    pub fn load(path: &Path) -> Result<Self> {
        let tolerances: Tolerances = if path.exists() {
            toml::from_str(&std::fs::read_to_string(path)?).with_context(|| format!("reading {}", path.display()))?
        } else {
            Tolerances { default_max_channel_diff: DEFAULT_MAX_CHANNEL_DIFF, overrides: Vec::new() }
        };
        if tolerances.default_max_channel_diff != DEFAULT_MAX_CHANNEL_DIFF {
            bail!("the default tolerance is fixed at {DEFAULT_MAX_CHANNEL_DIFF}/255; use a per-feature override with a reason");
        }
        for o in &tolerances.overrides {
            if o.reason.trim().len() < 20 {
                bail!("the override for `{}` needs a written reason explaining the gap", o.cases);
            }
            glob::Pattern::new(&o.cases).with_context(|| format!("bad pattern `{}`", o.cases))?;
        }
        Ok(tolerances)
    }

    /// The loosest override matching `id`, or the default.
    pub fn for_case(&self, id: &str) -> Limit {
        let matching: Vec<&Override> =
            self.overrides.iter().filter(|o| glob::Pattern::new(&o.cases).is_ok_and(|p| p.matches(id))).collect();
        Limit {
            max_channel_diff: matching.iter().map(|o| o.max_channel_diff).max().unwrap_or(self.default_max_channel_diff),
            max_pixels_over: matching.iter().map(|o| o.max_pixels_over).max().unwrap_or(0),
        }
    }
}

pub struct Diff {
    pub max_channel_diff: u8,
    pub differing_pixels: u64,
    pub total_pixels: u64,
    /// Largest channel difference per pixel.
    pub per_pixel: Vec<u8>,
}

/// Compares straight-alpha RGBA images. Where both pixels are fully transparent, color is ignored.
pub fn diff(reference: &RgbaImage, port: &RgbaImage, tolerance: u8) -> Result<Diff> {
    if reference.dimensions() != port.dimensions() {
        bail!("size differs: reference {:?}, port {:?}", reference.dimensions(), port.dimensions());
    }
    let mut max = 0u8;
    let mut differing = 0u64;
    let mut per_pixel = Vec::with_capacity((reference.width() * reference.height()) as usize);
    for (a, b) in reference.pixels().zip(port.pixels()) {
        let d = if a[3] == 0 && b[3] == 0 {
            0
        } else {
            (0..4).map(|c| a[c].abs_diff(b[c])).max().unwrap()
        };
        max = max.max(d);
        if d > tolerance {
            differing += 1;
        }
        per_pixel.push(d);
    }
    Ok(Diff { max_channel_diff: max, differing_pixels: differing, total_pixels: per_pixel.len() as u64, per_pixel })
}

/// Reference, port and heatmap side by side, each scaled up so small cases stay legible.
/// Heatmap: black is identical, blue is within tolerance, yellow to red is over it.
pub fn heatmap(reference: &RgbaImage, port: &RgbaImage, d: &Diff, tolerance: u8) -> RgbaImage {
    let (w, h) = reference.dimensions();
    let scale = (256 / w.max(h)).clamp(1, 8);
    let gap = 4;
    let mut out = RgbaImage::from_pixel(3 * w * scale + 2 * gap, h * scale, Rgba([40, 40, 40, 255]));
    let checker = |x: u32, y: u32| if ((x / 4) + (y / 4)) % 2 == 0 { 200u8 } else { 255u8 };
    let over = |p: &Rgba<u8>, x: u32, y: u32| {
        let bg = checker(x, y) as u32;
        let a = p[3] as u32;
        Rgba([0, 1, 2].map(|c| ((p[c] as u32 * a + bg * (255 - a) + 127) / 255) as u8).into_iter().chain([255]).collect::<Vec<_>>().try_into().unwrap())
    };
    for y in 0..h * scale {
        for x in 0..w * scale {
            let (sx, sy) = (x / scale, y / scale);
            out.put_pixel(x, y, over(reference.get_pixel(sx, sy), x, y));
            out.put_pixel(x + w * scale + gap, y, over(port.get_pixel(sx, sy), x, y));
            let v = d.per_pixel[(sy * w + sx) as usize];
            let color = if v == 0 {
                Rgba([0, 0, 0, 255])
            } else if v <= tolerance {
                Rgba([0, 60, 200, 255])
            } else {
                let t = ((v - tolerance) as f32 / 32.0).min(1.0);
                Rgba([255, (230.0 * (1.0 - t)) as u8, 0, 255])
            };
            out.put_pixel(x + 2 * (w * scale + gap), y, color);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_pixels_ignore_color() {
        let a = RgbaImage::from_pixel(2, 2, Rgba([10, 20, 30, 0]));
        let b = RgbaImage::from_pixel(2, 2, Rgba([200, 0, 0, 0]));
        assert_eq!(diff(&a, &b, 1).unwrap().max_channel_diff, 0);
    }

    #[test]
    fn counts_pixels_over_tolerance() {
        let a = RgbaImage::from_pixel(2, 1, Rgba([10, 20, 30, 255]));
        let mut b = a.clone();
        b.put_pixel(0, 0, Rgba([11, 20, 30, 255]));
        b.put_pixel(1, 0, Rgba([10, 23, 30, 255]));
        let d = diff(&a, &b, 1).unwrap();
        assert_eq!((d.max_channel_diff, d.differing_pixels), (3, 1));
    }

    #[test]
    fn overrides_need_a_reason() {
        let t: Tolerances = toml::from_str("[[override]]\ncases = \"type/*\"\nmax_channel_diff = 3\nreason = \"\"").unwrap();
        let dir = std::env::temp_dir().join("parity-tolerance-test.toml");
        std::fs::write(&dir, "[[override]]\ncases = \"type/*\"\nmax_channel_diff = 3\nreason = \"\"").unwrap();
        assert!(Tolerances::load(&dir).is_err());
        assert_eq!(t.for_case("type/a").max_channel_diff, 3);
    }
}
