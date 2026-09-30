//! Which layers the Mac turns into live shapes or draws from their vector masks
//! (`PSDVector.swift`). The decisions, sizes and refusals are ported; the drawing itself (Core
//! Graphics antialiasing) isn't yet, so the import reports such layers as not supported.

use crate::reader::{ImportError, MAX_SIDE, MAX_SURFACE_PIXELS, Rect, Result};
use std::collections::HashMap;

/// A layer the Mac draws: where, and how many pixels that takes from the budget.
pub struct Drawn {
    pub bounds: Rect,
    pub pixels: i64,
}

type Extra = HashMap<String, Vec<u8>>;

/// `PSDVector.live`: a filled rectangle or ellipse becomes a live shape layer.
pub fn live(extra: &Extra, canvas: (f64, f64), remaining_pixels: i64) -> Result<Option<Drawn>> {
    let stroke = extra.get("vstk");
    let fill_enabled = stroke.and_then(|s| bool_value(s, "fillEnabled")).unwrap_or(extra.contains_key("SoCo"));
    if !fill_enabled || extra.get("SoCo").and_then(|d| rgb(d)).is_none() {
        return Ok(None);
    }
    let origin = match origination(extra.get("vogk")) {
        Some(o) => Some(o),
        None => sharp_rect(extra.get("vmsk").or_else(|| extra.get("vsms")), canvas),
    };
    let Some(origin) = origin else { return Ok(None) };
    let mut rect = integral(origin);
    if !rect.x.is_finite() || !rect.y.is_finite() {
        return Ok(None);
    }
    let Some((width, height)) = pixel_size(rect.width, rect.height, remaining_pixels)? else { return Ok(None) };
    rect.width = width as f64;
    rect.height = height as f64;
    Ok(Some(Drawn { bounds: rect, pixels: width * height }))
}

/// `PSDVector.raster`: a vector mask with a fill or stroke drawn into pixels.
pub fn raster(extra: &Extra, canvas: (f64, f64), remaining_pixels: i64) -> Result<Option<Drawn>> {
    let Some(mask) = extra.get("vmsk").or_else(|| extra.get("vsms")) else { return Ok(None) };
    let Some(path) = path(mask, canvas) else { return Ok(None) };
    let fill = extra.get("SoCo").and_then(|d| rgb(d));
    let stroke = extra.get("vstk");
    let fill_enabled = stroke.and_then(|s| bool_value(s, "fillEnabled")).unwrap_or(fill.is_some());
    let stroke_enabled = stroke.and_then(|s| bool_value(s, "strokeEnabled")).unwrap_or(false);
    let stroke_color = stroke.and_then(|s| rgb(s));
    let stroke_width = stroke.and_then(|s| unit(s, "strokeStyleLineWidth", 0)).unwrap_or(1.0);
    if !(fill_enabled && fill.is_some() || stroke_enabled && stroke_color.is_some()) {
        return Ok(None);
    }
    if !stroke_width.is_finite() {
        return Ok(None);
    }
    if stroke_enabled && !(0.0..=MAX_SIDE as f64).contains(&stroke_width) {
        return Err(ImportError::TooLarge);
    }
    let mut bbox = path;
    if stroke_enabled {
        let d = (stroke_width / 2.0 + 1.0).ceil();
        bbox = Rect { x: bbox.x - d, y: bbox.y - d, width: bbox.width + 2.0 * d, height: bbox.height + 2.0 * d };
    }
    let rect = integral(bbox);
    if !rect.x.is_finite() || !rect.y.is_finite() {
        return Ok(None);
    }
    let Some((width, height)) = pixel_size(rect.width, rect.height, remaining_pixels)? else { return Ok(None) };
    Ok(Some(Drawn { bounds: Rect { x: rect.x, y: rect.y, width: width as f64, height: height as f64 }, pixels: width * height }))
}

/// `CGRect.integral`: the smallest integer rectangle containing it.
fn integral(r: Rect) -> Rect {
    let (x0, y0) = (r.x.floor(), r.y.floor());
    let (x1, y1) = ((r.x + r.width).ceil(), (r.y + r.height).ceil());
    Rect { x: x0, y: y0, width: x1 - x0, height: y1 - y0 }
}

/// Refuses sizes past the side limit or the pixel budget.
fn pixel_size(width: f64, height: f64, remaining_pixels: i64) -> Result<Option<(i64, i64)>> {
    if !width.is_finite() || !height.is_finite() {
        return Ok(None);
    }
    let max = MAX_SIDE as f64;
    if width.abs() > max || height.abs() > max {
        return Err(ImportError::TooLarge);
    }
    let budget = MAX_SURFACE_PIXELS.min(remaining_pixels.max(0));
    if width * height > budget as f64 {
        return Err(ImportError::TooLarge);
    }
    let (w, h) = ((width as i64).max(1), (height as i64).max(1));
    if w * h > budget {
        return Err(ImportError::TooLarge);
    }
    Ok(Some((w, h)))
}

/// `vogk`: origin type 1 or 2 is a rectangle (2 rounded), 5 an ellipse.
fn origination(data: Option<&Vec<u8>>) -> Option<Rect> {
    let data = data?;
    let kind = int32(data, "keyOriginType")?;
    if !matches!(kind, 1 | 2 | 5) {
        return None;
    }
    let from = offset_of(b"keyOriginShapeBBox", data, 0).unwrap_or(0);
    let left = unit(data, "Left", from)?;
    let top = unit(data, "Top ", from)?;
    let right = unit(data, "Rght", from)?;
    let bottom = unit(data, "Btom", from)?;
    let bounds = Rect { x: left, y: top, width: right - left, height: bottom - top };
    if !(bounds.width >= 1.0 && bounds.height >= 1.0 && bounds.x.is_finite() && bounds.y.is_finite() && bounds.width.is_finite() && bounds.height.is_finite()) {
        return None;
    }
    if kind != 5 && let Some(radii_at) = offset_of(b"keyOriginRRectRadii", data, 0) {
        let radii: Vec<f64> = ["topLeft", "topRight", "bottomRight", "bottomLeft"].iter().filter_map(|k| unit(data, k, radii_at)).collect();
        if radii.len() == 4 {
            let lo = radii.iter().cloned().fold(f64::INFINITY, f64::min);
            let hi = radii.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            if hi - lo > 0.5 {
                return None;
            }
        }
    }
    Some(bounds)
}

/// A vector mask of four sharp corners, as a rectangle.
fn sharp_rect(data: Option<&Vec<u8>>, canvas: (f64, f64)) -> Option<Rect> {
    let data = data?;
    let bbox = path(data, canvas)?;
    let mut offset = 8;
    let mut remaining = 0;
    let mut anchors = 0;
    let mut sharp = true;
    while offset + 26 <= data.len() {
        let kind = i16_at(data, offset);
        let body = &data[offset + 2..offset + 26];
        offset += 26;
        match kind {
            0 | 3 => {
                if anchors > 0 {
                    return None;
                }
                remaining = i16_at(body, 0) as i32;
            }
            1 | 2 | 4 | 5 => {
                if remaining <= 0 {
                    continue;
                }
                remaining -= 1;
                let incoming = point(body, 0, canvas);
                let anchor = point(body, 8, canvas);
                let outgoing = point(body, 16, canvas);
                if (incoming.0 - anchor.0).hypot(incoming.1 - anchor.1) > 0.5 || (outgoing.0 - anchor.0).hypot(outgoing.1 - anchor.1) > 0.5 {
                    sharp = false;
                }
                anchors += 1;
            }
            _ => {}
        }
    }
    if !sharp || anchors != 4 {
        return None;
    }
    // A path's `boundingBoxOfPath` for four sharp corners is their bounding box.
    (bbox.width >= 1.0 && bbox.height >= 1.0).then_some(bbox)
}

/// The vector mask's path, as its bounding box including control points
/// (`CGPath.boundingBoxOfPath`); `None` when it has no points.
fn path(data: &[u8], canvas: (f64, f64)) -> Option<Rect> {
    if data.len() < 8 || canvas.0 <= 0.0 || canvas.1 <= 0.0 {
        return None;
    }
    let mut offset = 8;
    let mut remaining = 0;
    let mut first = true;
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    let mut include = |p: (f64, f64)| {
        x0 = x0.min(p.0);
        y0 = y0.min(p.1);
        x1 = x1.max(p.0);
        y1 = y1.max(p.1);
    };
    let mut previous_out = (0.0, 0.0);
    let mut any = false;
    while offset + 26 <= data.len() {
        let kind = i16_at(data, offset);
        let body = &data[offset + 2..offset + 26];
        offset += 26;
        match kind {
            0 | 3 => {
                remaining = i16_at(body, 0) as i32;
                first = true;
            }
            1 | 2 | 4 | 5 => {
                if remaining <= 0 {
                    continue;
                }
                remaining -= 1;
                let incoming = point(body, 0, canvas);
                let anchor = point(body, 8, canvas);
                let outgoing = point(body, 16, canvas);
                if first {
                    include(anchor);
                    first = false;
                } else {
                    include(previous_out);
                    include(incoming);
                    include(anchor);
                }
                any = true;
                previous_out = outgoing;
            }
            _ => {}
        }
    }
    any.then(|| Rect { x: x0, y: y0, width: x1 - x0, height: y1 - y0 })
}

fn point(body: &[u8], at: usize, canvas: (f64, f64)) -> (f64, f64) {
    let y = i32_at(body, at) as f64 / 16_777_216.0;
    let x = i32_at(body, at + 4) as f64 / 16_777_216.0;
    (x * canvas.0, y * canvas.1)
}

/// `PSDVector.rgb`: `Rd  `, `Grn `, `Bl  ` doubles, as 0-1 or 0-255.
fn rgb(data: &[u8]) -> Option<(f64, f64, f64)> {
    let channel = |v: f64| if v > 1.0 { v.clamp(0.0, 255.0) / 255.0 } else { v.clamp(0.0, 1.0) };
    Some((channel(double(data, "Rd  ")?), channel(double(data, "Grn ")?), channel(double(data, "Bl  ")?)))
}

fn bool_value(data: &[u8], key: &str) -> Option<bool> {
    let start = offset_of(key.as_bytes(), data, 0)?;
    let at = start + key.len();
    if at + 5 > data.len() || &data[at..at + 4] != b"bool" {
        return None;
    }
    Some(data[at + 4] != 0)
}

fn unit(data: &[u8], key: &str, from: usize) -> Option<f64> {
    let key_at = offset_of(key.as_bytes(), data, from)?;
    let unit_at = offset_of(b"UntF", data, key_at)?;
    double_at(data, unit_at + 8)
}

fn int32(data: &[u8], key: &str) -> Option<i32> {
    let start = offset_of(key.as_bytes(), data, 0)?;
    let at = start + key.len();
    if at + 8 > data.len() || &data[at..at + 4] != b"long" {
        return None;
    }
    Some(i32_at(data, at + 4))
}

fn double(data: &[u8], key: &str) -> Option<f64> {
    let start = offset_of(key.as_bytes(), data, 0)?;
    let at = start + key.len();
    if at + 12 > data.len() || &data[at..at + 4] != b"doub" {
        return None;
    }
    double_at(data, at + 4)
}

fn double_at(data: &[u8], at: usize) -> Option<f64> {
    (at + 8 <= data.len()).then(|| f64::from_bits(u64::from_be_bytes(data[at..at + 8].try_into().unwrap())))
}

fn offset_of(needle: &[u8], data: &[u8], start: usize) -> Option<usize> {
    if start >= data.len() || needle.is_empty() {
        return None;
    }
    data[start..].windows(needle.len()).position(|w| w == needle).map(|p| p + start)
}

fn i16_at(data: &[u8], at: usize) -> i16 {
    i16::from_be_bytes([data[at], data[at + 1]])
}

fn i32_at(data: &[u8], at: usize) -> i32 {
    i32::from_be_bytes(data[at..at + 4].try_into().unwrap())
}
