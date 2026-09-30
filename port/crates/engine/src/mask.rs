//! Layer and folder masks as coverage: a layer's own mask over its pixel grid, and a folder's
//! mask over the canvas.

use crate::gpu::Gpu;
use comp_format::{LayerRecord, Project};

/// The layer's own enabled mask, one value (0...255) per layer pixel, or `None`. Errors name
/// what the port can't draw yet.
///
/// For now this covers masks that line up with their layer pixel for pixel, and uniform 1x1
/// masks; a mask that needs resampling waits for the transform work.
pub fn layer_coverage(project: &Project, layer: &LayerRecord, (width, height): (u32, u32)) -> Result<Option<Vec<u32>>, String> {
    if !layer.mask_enabled() {
        return Ok(None);
    }
    if layer.mask_placement.is_some() && !layer.mask_linked() {
        return Err("unlinked masks".into());
    }
    let mask = &project.masks.get(&layer.id).ok_or("missing mask")?.pixels;
    if let Some(placed) = placed_coverage(layer, mask, (width, height)) {
        return Ok(Some(placed));
    }
    if mask.width() == 1 && mask.height() == 1 {
        return Ok(Some(vec![mask.as_raw()[0] as u32; (width * height) as usize]));
    }
    if mask.dimensions() != (width, height) {
        return Err("resampled masks".into());
    }
    Ok(Some(mask.as_raw().iter().map(|&m| m as u32).collect()))
}

/// A mask placed on the document apart from its layer (as painting past a layer's edge leaves
/// it), when both sit upright and 1:1 on whole pixels: `LayerMask.clipImage`, the mask over its
/// place and its background (`LayerMask.background`) elsewhere.
fn placed_coverage(layer: &LayerRecord, mask: &image::GrayImage, (width, height): (u32, u32)) -> Option<Vec<u32>> {
    let p = layer.mask_placement.as_ref()?;
    let t = &layer.transform;
    let (mw, mh) = mask.dimensions();
    let plain = |t: &comp_format::Transform, w: u32, h: u32| {
        t.rotation == 0.0 && !t.flip_x && !t.flip_y && t.size == [w as f64, h as f64] && t.origin[0].fract() == 0.0 && t.origin[1].fract() == 0.0
    };
    if !plain(t, width, height) || !plain(p, mw, mh) || (mw > 96 || mh > 96) || (p.origin == t.origin && p.size == t.size) {
        return None;
    }
    // The background is read from the mask's 96-pixel thumbnail; a mask this small is its own.
    let edge: Vec<u32> = (0..mh).flat_map(|y| (0..mw).map(move |x| (x, y))).filter(|&(x, y)| x == 0 || y == 0 || x == mw - 1 || y == mh - 1).map(|(x, y)| mask.get_pixel(x, y)[0] as u32).collect();
    let background = if edge.iter().sum::<u32>() * 2 >= edge.len() as u32 * 255 { 255 } else { 0 };
    let (dx, dy) = ((t.origin[0] - p.origin[0]) as i64, (t.origin[1] - p.origin[1]) as i64);
    Some(
        (0..height as i64)
            .flat_map(|y| (0..width as i64).map(move |x| (x + dx, y + dy)))
            .map(|(u, v)| if u >= 0 && v >= 0 && u < mw as i64 && v < mh as i64 { mask.get_pixel(u as u32, v as u32)[0] as u32 } else { background })
            .collect(),
    )
}

/// A folder's enabled mask as coverage per canvas pixel: the mask over the folder's rectangle,
/// nothing outside it.
pub fn folder_coverage(project: &Project, folder: &LayerRecord, (width, height): (u32, u32)) -> Result<Vec<u32>, String> {
    let mask = &project.masks.get(&folder.id).ok_or("missing folder mask")?.pixels;
    let t = &folder.transform;
    let upright = t.rotation == 0.0 && !t.flip_x && !t.flip_y && t.origin[0].fract() == 0.0 && t.origin[1].fract() == 0.0;
    let uniform = mask.width() == 1 && mask.height() == 1;
    if !upright || (!uniform && t.size != [mask.width() as f64, mask.height() as f64]) {
        return crate::transform::clip::folder_coverage(mask, t, (width, height));
    }
    let (ox, oy) = (t.origin[0] as i64, t.origin[1] as i64);
    let (fw, fh) = (t.size[0] as i64, t.size[1] as i64);
    let mut out = vec![0u32; (width * height) as usize];
    for y in 0..height as i64 {
        for x in 0..width as i64 {
            let (mx, my) = (x - ox, y - oy);
            if mx >= 0 && my >= 0 && mx < fw && my < fh {
                let m = if uniform { mask.as_raw()[0] } else { mask.as_raw()[(my * fw + mx) as usize] };
                out[(y * width as i64 + x) as usize] = m as u32;
            }
        }
    }
    Ok(out)
}

/// Two coverages multiplied, (a × b + 127) / 255, as nested clips combine.
pub fn multiply(gpu: &Gpu, a: &wgpu::Buffer, b: &wgpu::Buffer, width: u32, height: u32) -> wgpu::Buffer {
    let pipeline = gpu.pipeline("multiply_coverage", include_str!("mask_multiply.wgsl"));
    let out = gpu.image(width, height).buffer;
    let params = [width.to_le_bytes(), height.to_le_bytes()].concat();
    gpu.dispatch(&pipeline, &params, &[a, b, &out], width, height);
    out
}
