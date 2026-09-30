//! Masks resampled on the CPU, as `resample.wgsl` samples a layer's own mask: folder masks drawn
//! over the folder's rectangle (`FolderMaskClip.apply`), and unlinked masks placed into their
//! layer's pixel grid (`LayerMask.clipImage`). Mask clips are small and computed once per render,
//! and `mask.rs` hands them over as coverage per pixel.

use super::{Placement, Rect};
use comp_format::{Sampling, Transform};
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

/// `FolderMaskClip.apply`: a folder's mask clipped over the folder's rectangle, as coverage per
/// canvas pixel. Errors name what isn't reproduced yet.
pub fn folder_coverage(mask: &GrayImage, t: &Transform, (width, height): (u32, u32)) -> Result<Vec<u32>, String> {
    // Clipping to a mask with High samples the nearest pixel (measured shrinking, and rotated at
    // its own size); enlarging with High hasn't been measured.
    let (mw, mh) = mask.dimensions();
    let filter = match t.sampling {
        Sampling::Nearest => Filter::Nearest,
        Sampling::Smooth => return Err("folder masks resampled with Smooth".into()),
        Sampling::High => {
            if t.size[0] > mw as f64 || t.size[1] > mh as f64 {
                return Err("folder masks enlarged with High".into());
            }
            Filter::Nearest
        }
    };
    let placement = Placement::of(t);
    let rect = placement.rect(1.0, 1.0);
    Ok(sample(mask, &placement, &rect, filter, (width, height)).into_iter().map(|(m, c)| m * c / 255).collect())
}
