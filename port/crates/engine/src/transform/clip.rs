//! Masks resampled on the CPU, as `resample.wgsl` samples a layer's own mask: folder masks drawn
//! over the folder's rectangle (`FolderMaskClip.apply`), and unlinked masks placed into their
//! layer's pixel grid (`LayerMask.clipImage`). Mask clips are small and computed once per render,
//! and `mask.rs` hands them over as coverage per pixel.

use super::{Placement, Rect};
use comp_format::{LayerRecord, Project, Transform};
use image::GrayImage;

/// 32.32 fixed point, as the kernel steps sample positions.
fn fixed(v: f64) -> i64 {
    (v * 4294967296.0).floor() as i64
}

/// One axis's taps: the heavy pixel, the light one, and the shift (0: heavy alone).
#[derive(Clone, Copy)]
struct Taps {
    heavy: i64,
    light: i64,
    shift: u32,
}

fn nearest(p: i64) -> Taps {
    Taps { heavy: p >> 32, light: 0, shift: 0 }
}

/// `low` in `resample.wgsl`.
fn low(p: i64) -> Taps {
    let s = p.wrapping_sub(1 << 31);
    let i = s >> 32;
    let f = s as u32;
    let eighths = ((f >> 28) + 1) >> 1;
    let (mut t, reach) = if f >= 0x8000_0000 { (Taps { heavy: i + 1, light: i, shift: 0 }, 8 - eighths) } else { (Taps { heavy: i, light: i + 1, shift: 0 }, eighths) };
    if reach > 0 {
        t.shift = 5 - reach;
    }
    t
}

fn blend(heavy: u32, light: u32, k: u32) -> u32 {
    if k == 0 { heavy } else { heavy - (heavy >> k) + (light >> k) }
}

/// How a mask is sampled.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Filter {
    Nearest,
    Low,
}

/// A mask drawn through `placement` into its rectangle, sampled per target pixel: the mask value
/// and the rectangle's edge coverage (0 outside), both bytes, for a `width` x `height` target.
pub fn sample(mask: &GrayImage, placement: &Placement, rect: &Rect, filter: Filter, (width, height): (u32, u32)) -> Vec<(u32, u32)> {
    let (mw, mh) = mask.dimensions();
    let dda = placement.dda(rect, mw, mh);
    let (u0, udx, udy) = (fixed(dda.u0), fixed(dda.u_dx), fixed(dda.u_dy));
    let (v0, vdx, vdy) = (fixed(dda.v0), fixed(dda.v_dx), fixed(dda.v_dy));
    let at = |x: i64, y: i64| mask.as_raw()[(y.clamp(0, mh as i64 - 1) * mw as i64 + x.clamp(0, mw as i64 - 1)) as usize] as u32;
    let l1 = placement.l1() as f32;
    let upright = placement.upright();
    let table = if upright { placement.edge_table(rect, width, height) } else { Vec::new() };
    let scale = [((rect.max_x - rect.min_x) / mw as f64) as f32, ((rect.max_y - rect.min_y) / mh as f64) as f32];
    let to_float = |p: i64| (p >> 32) as i32 as f32 + ((p as u32) >> 8) as f32 / 16777216.0;
    let mut out = Vec::with_capacity((width * height) as usize);
    for y in 0..height as i64 {
        for x in 0..width as i64 {
            let u = u0.wrapping_add(udx.wrapping_mul(x)).wrapping_add(udy.wrapping_mul(y));
            let v = v0.wrapping_add(vdx.wrapping_mul(x)).wrapping_add(vdy.wrapping_mul(y));
            let (fade, covered) = if upright {
                let col = table[x as usize];
                let row = table[(width as i64 + y) as usize];
                let fade = [f32::from_bits(col[0]), f32::from_bits(row[0]), f32::from_bits(col[1]), f32::from_bits(row[1])];
                let (across, down) = (fade[0] * fade[2], fade[1] * fade[3]);
                let covered = if down >= 1.0 {
                    col[2]
                } else if across >= 1.0 {
                    row[2]
                } else {
                    ((across * down * 256.0).ceil() - 1.0).clamp(0.0, 255.0) as u32
                };
                (fade, covered)
            } else {
                let right = ((mw as i64) << 32).wrapping_add(!u);
                let bottom = ((mh as i64) << 32).wrapping_add(!v);
                let d = [to_float(u) * scale[0], to_float(v) * scale[1], to_float(right) * scale[0], to_float(bottom) * scale[1]];
                let fade = d.map(|d| (0.5 + d / l1).clamp(0.0, 1.0));
                let p = fade[0] * fade[1] * fade[2] * fade[3];
                (fade, ((p * 256.0).ceil() - 1.0).clamp(0.0, 255.0) as u32)
            };
            if covered == 0 {
                out.push((0, 0));
                continue;
            }
            let taps = |p: i64, across_edge: bool| match filter {
                Filter::Nearest => nearest(p),
                Filter::Low => {
                    let mut t = low(p);
                    if across_edge {
                        t.shift = 0;
                    }
                    t
                }
            };
            let tu = taps(u, fade[0] < 1.0 || fade[2] < 1.0);
            let tv = taps(v, fade[1] < 1.0 || fade[3] < 1.0);
            let column = |x: i64| blend(at(x, tv.heavy), at(x, tv.light), tv.shift);
            let mut value = column(tu.heavy);
            if tu.shift != 0 {
                value = blend(value, column(tu.light), tu.shift);
            }
            out.push((value, covered));
        }
    }
    out
}

/// Whether a folder's (or an adjustment layer's) mask is resampled to cover its rectangle, rather
/// than lying on the canvas pixel for pixel.
pub fn resampled(project: &Project, layer: &LayerRecord) -> bool {
    let Some(mask) = project.masks.get(&layer.id) else {
        return false;
    };
    let t = &layer.transform;
    let (w, h) = mask.pixels.dimensions();
    let upright = t.rotation % 360.0 == 0.0 && !t.flip_x && !t.flip_y && t.origin[0].fract() == 0.0 && t.origin[1].fract() == 0.0;
    !upright || ((w, h) != (1, 1) && t.size != [w as f64, h as f64])
}

/// `FolderMaskClip.apply`: a folder's mask clipped over the folder's rectangle, as coverage per
/// canvas pixel.
///
/// Core Graphics resamples a clip mask when something is drawn through it, with that drawing's
/// interpolation, so a folder's mask follows the layer inside rather than the folder's own
/// sampling. For a layer drawn 1:1 and upright (the only case `composite.rs` sends here) that is
/// the nearest pixel.
pub fn folder_coverage(mask: &GrayImage, t: &Transform, (width, height): (u32, u32)) -> Result<Vec<u32>, String> {
    let placement = Placement::of(t);
    let rect = placement.rect(1.0, 1.0);
    Ok(sample(mask, &placement, &rect, Filter::Nearest, (width, height)).into_iter().map(|(m, c)| m * c / 255).collect())
}

/// `LayerMask.background(of:)`: what an unlinked mask shows beyond its pixels, white or black,
/// whichever most of its edge is. The Mac reads the edge from a thumbnail at most 96 pixels
/// across; this reads the mask's own edge, which only differs for a mask whose edge averages
/// within a hair of half gray.
fn background(mask: &GrayImage) -> u32 {
    let (w, h) = mask.dimensions();
    let (mut total, mut count) = (0u64, 0u64);
    for y in 0..h {
        for x in 0..w {
            if y == 0 || y == h - 1 || x == 0 || x == w - 1 {
                total += mask.get_pixel(x, y)[0] as u64;
                count += 1;
            }
        }
    }
    if total * 2 >= count * 255 { 255 } else { 0 }
}

/// One axis of a resampled placement: source position `a·(x + ½) + b` for target pixel x.
#[derive(Clone, Copy)]
struct Axis {
    a: f64,
    b: f64,
    /// Source pixels.
    size: u32,
}

/// High's filter along one axis for one target pixel: shrinking averages the target pixel's
/// footprint over the source (a box of source pixels and their weights); enlarging blends like
/// Low.
#[derive(Clone)]
enum AxisTaps {
    Box(Vec<(i64, f64)>),
    Low(Taps),
}

fn high_axis(axis: Axis, targets: u32) -> Result<Vec<AxisTaps>, String> {
    let footprint = axis.a.abs();
    let n = axis.size as i64;
    let (start, step) = (fixed(axis.a * 0.5 + axis.b), fixed(axis.a));
    let mut out = Vec::with_capacity(targets as usize);
    for x in 0..targets as i64 {
        let center = axis.a * (x as f64 + 0.5) + axis.b;
        out.push(if footprint > 1.0 {
            let (lo, hi) = (center - footprint / 2.0, center + footprint / 2.0);
            let taps = (lo.floor() as i64..=hi.ceil() as i64)
                .filter_map(|i| {
                    let overlap = hi.min(i as f64 + 1.0) - lo.max(i as f64);
                    (overlap > 0.0).then(|| (i.clamp(0, n - 1), overlap / footprint))
                })
                .collect();
            AxisTaps::Box(taps)
        } else if footprint < 1.0 {
            let mut t = low(start.wrapping_add(step.wrapping_mul(x)));
            t.heavy = t.heavy.clamp(0, n - 1);
            t.light = t.light.clamp(0, n - 1);
            AxisTaps::Low(t)
        } else {
            if center.fract() != 0.5 {
                return Err("unlinked masks placed off the pixel grid at their own size".into());
            }
            AxisTaps::Box(vec![((center.floor() as i64).clamp(0, n - 1), 1.0)])
        });
    }
    Ok(out)
}

/// One axis's taps applied to a line of values; a box rounds half up.
fn apply(taps: &AxisTaps, at: impl Fn(i64) -> u32) -> u32 {
    match taps {
        AxisTaps::Low(t) => blend(at(t.heavy), if t.shift == 0 { 0 } else { at(t.light) }, t.shift),
        AxisTaps::Box(taps) => {
            let sum: f64 = taps.iter().map(|&(i, w)| w * at(i) as f64).sum();
            // Exact halves round up; f64 lands within a hair of them.
            (sum + 0.5 + 1e-7).floor().clamp(0.0, 255.0) as u32
        }
    }
}

/// `LayerMask.clipImage` for an unlinked mask: the mask at `placement` on the document, resampled
/// with High into the `width` x `height` pixel grid of a layer at `layer`, what lies beyond it
/// filled with its background. Errors name what isn't reproduced yet.
pub fn placed_mask(mask: &GrayImage, placement: &Transform, layer: &Transform, (width, height): (u32, u32)) -> Result<GrayImage, String> {
    if placement.rotation % 360.0 != 0.0 || layer.rotation % 360.0 != 0.0 {
        return Err("unlinked masks placed at another rotation".into());
    }
    let (mw, mh) = mask.dimensions();
    let covered = placement.size[0] / layer.size[0].max(1.0) * width as f64;
    if super::reduction_level(covered / mw as f64) > 0 {
        return Err("unlinked masks reduced by halvings".into());
    }
    // Layer grid pixel → document → mask pixel, per axis (flips mirror about the centers).
    let axis = |origin: f64, size: f64, flip: bool, pixels: u32, m_origin: f64, m_size: f64, m_flip: bool, m_pixels: u32| {
        // Document position of grid coordinate g: origin + size·g/pixels (mirrored when flipped).
        let (da, db) = if flip { (-size / pixels as f64, origin + size) } else { (size / pixels as f64, origin) };
        // Mask coordinate of document position p: (p − m_origin)·m_pixels/m_size (mirrored).
        let (ma, mb) = if m_flip { (-(m_pixels as f64) / m_size, (m_origin + m_size) * m_pixels as f64 / m_size) } else { (m_pixels as f64 / m_size, -m_origin * m_pixels as f64 / m_size) };
        Axis { a: ma * da, b: ma * db + mb, size: m_pixels }
    };
    let ax = axis(layer.origin[0], layer.size[0], layer.flip_x, width, placement.origin[0], placement.size[0], placement.flip_x, mw);
    let ay = axis(layer.origin[1], layer.size[1], layer.flip_y, height, placement.origin[1], placement.size[1], placement.flip_y, mh);
    if layer.flip_x || layer.flip_y {
        return Err("unlinked masks over flipped layers".into());
    }
    // Where the mask's rectangle lies on the grid, and each grid pixel's fade at its edges.
    let fades = |origin: f64, size: f64, pixels: u32, m_origin: f64, m_size: f64| {
        let grid = |p: f64| (p - origin) * pixels as f64 / size;
        let (lo, hi) = (grid(m_origin), grid(m_origin + m_size));
        (0..pixels).map(|i| (0.5 + (i as f64 + 0.5 - lo)).clamp(0.0, 1.0) * (0.5 + (hi - i as f64 - 0.5)).clamp(0.0, 1.0)).collect::<Vec<f64>>()
    };
    let fx = fades(layer.origin[0], layer.size[0], width, placement.origin[0], placement.size[0]);
    let fy = fades(layer.origin[1], layer.size[1], height, placement.origin[1], placement.size[1]);
    let tx = high_axis(ax, width)?;
    let ty = high_axis(ay, height)?;
    let shrinks = (ax.a.abs() > 1.0, ay.a.abs() > 1.0);
    if shrinks.0 && !shrinks.1 && ay.a.abs() < 1.0 {
        return Err("unlinked masks shrunk across and enlarged down".into());
    }
    let partial = |f: &[f64]| f.iter().any(|&f| f > 0.0 && f < 1.0);
    if (shrinks.0 && partial(&fx)) || (shrinks.1 && partial(&fy)) {
        return Err("unlinked masks shrunk with edges off the pixel grid".into());
    }
    // Where an edge only partly covers a pixel, Low takes the heavy pixel across it.
    let heavy = |t: &AxisTaps| match t {
        AxisTaps::Low(t) => AxisTaps::Low(Taps { shift: 0, ..*t }),
        AxisTaps::Box(b) => AxisTaps::Box(b.clone()),
    };
    let at = |x: i64, y: i64| mask.as_raw()[(y * mw as i64 + x) as usize] as u32;
    let bg = background(mask);
    let mut out = GrayImage::new(width, height);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let c = ((fx[x] * fy[y] * 256.0).ceil() - 1.0).clamp(0.0, 255.0) as u32;
            let value = if c == 0 {
                bg
            } else {
                let tx = if fx[x] < 1.0 { heavy(&tx[x]) } else { tx[x].clone() };
                let ty = if fy[y] < 1.0 { heavy(&ty[y]) } else { ty[y].clone() };
                let m = if shrinks.0 && shrinks.1 {
                    // Both boxes: across first.
                    apply(&ty, |j| apply(&tx, |i| at(i, j)))
                } else {
                    apply(&tx, |i| apply(&ty, |j| at(i, j)))
                };
                // `LayerMask.placed`: the background, black filled over the rectangle, then white
                // through the mask clipped to it.
                let under = (bg * (255 - c) + 127) / 255;
                let through = (m * c / 255) * c / 255;
                under + ((255 - under) * through + 127) / 255
            };
            out.put_pixel(x as u32, y as u32, image::Luma([value as u8]));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(v: f64) -> i64 {
        fixed(v)
    }

    #[test]
    fn low_rounds_the_phase_half_up_to_eighths() {
        // A position 1/16 past a pixel center (u = i + 0.5 + phase) is the first eighth.
        let t = low(at(3.5 + 1.0 / 16.0));
        assert_eq!((t.heavy, t.light, t.shift), (3, 4, 4));
        // Just short of it, the heavy pixel alone.
        let t = low(at(3.5 + 1.0 / 16.0 - 1e-6));
        assert_eq!((t.heavy, t.shift), (3, 0));
        // Past the middle, the next pixel is heavy, and ties round toward it.
        let t = low(at(3.5 + 15.0 / 16.0));
        assert_eq!((t.heavy, t.shift), (4, 0));
        let t = low(at(3.5 + 0.5));
        assert_eq!((t.heavy, t.light, t.shift), (4, 3, 1));
    }

    #[test]
    fn blend_shifts_the_light_pixel_in() {
        assert_eq!(blend(255, 0, 3), 255 - 31);
        assert_eq!(blend(0, 255, 1), 127);
        assert_eq!(blend(40, 255, 0), 40);
    }
}
