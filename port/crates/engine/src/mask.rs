//! Layer and folder masks, as coverage over a layer's own pixel grid.

use comp_format::{LayerRecord, Project};
use std::collections::HashMap;

/// The mask coverage for `layer`, one value (0...255) per layer pixel, or `None` when nothing
/// clips it. Errors name what the port can't do yet.
///
/// For now this covers masks that line up with their layer pixel for pixel, and uniform 1x1
/// masks; a mask that needs resampling waits for the transform work.
pub fn coverage(
    project: &Project,
    layer: &LayerRecord,
    by_id: &HashMap<&str, &LayerRecord>,
    (width, height): (u32, u32),
) -> Result<Option<Vec<u32>>, String> {
    let mut masks: Vec<&image::GrayImage> = Vec::new();
    if layer.mask_enabled() {
        if layer.mask_placement.is_some() && !layer.mask_linked() {
            return Err("unlinked masks".into());
        }
        masks.push(&project.masks.get(&layer.id).ok_or("missing mask")?.pixels);
    }
    for folder in crate::order::folders(layer, by_id) {
        if folder.mask_enabled() {
            // A folder's mask covers the folder's own rectangle; it lines up with the layer's pixels
            // only when both rectangles are the same.
            if folder.transform != layer.transform {
                return Err("folder masks over a different rectangle".into());
            }
            masks.push(&project.masks.get(&folder.id).ok_or("missing folder mask")?.pixels);
        }
    }
    if masks.is_empty() {
        return Ok(None);
    }
    let mut out = vec![255u32; (width * height) as usize];
    for mask in masks {
        let uniform = mask.width() == 1 && mask.height() == 1;
        if !uniform && mask.dimensions() != (width, height) {
            return Err("resampled masks".into());
        }
        for (i, v) in out.iter_mut().enumerate() {
            let m = if uniform { mask.as_raw()[0] } else { mask.as_raw()[i] } as u32;
            *v = (*v * m + 127) / 255;
        }
    }
    Ok(Some(out))
}
