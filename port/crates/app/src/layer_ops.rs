//! The Layer menu and the Layers panel's commands, as `EditorSession` does them: new layers and
//! folders, delete, duplicate, rename, group and ungroup, clipping, masks, merging, flipping and
//! adjustment layers. Each is one undo step.

use crate::document::{Doc, new_id};
use comp_format::{Adjustment, AdjustmentKind, AdjustmentColor, Asset, GradientMapSettings, LayerRecord, Project, Transform};
use std::collections::HashSet;

/// A layer record with nothing set but its name and place.
pub fn record(name: &str, transform: Transform) -> LayerRecord {
    LayerRecord {
        id: new_id(),
        name: name.into(),
        is_visible: true,
        transform,
        image_file: None,
        parent_id: None,
        is_group: None,
        opacity: None,
        blend_mode: None,
        mask_file: None,
        mask_enabled: None,
        mask_source_id: None,
        adjustment: None,
        mask_placement: None,
        mask_linked: None,
        shape: None,
        effects: None,
        text: None,
    }
}

fn index(p: &Project, id: &str) -> Option<usize> {
    p.manifest.layers.iter().position(|l| l.id == id)
}

/// `descendantIDs(of:)`.
pub fn descendants(p: &Project, id: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut pending = vec![id.to_string()];
    while let Some(parent) = pending.pop() {
        for l in &p.manifest.layers {
            if l.parent_id.as_deref() == Some(parent.as_str()) && out.insert(l.id.clone()) {
                pending.push(l.id.clone());
            }
        }
    }
    out
}

/// The first "{base} N" not taken.
fn next_name(p: &Project, base: &str) -> String {
    let names: HashSet<&str> = p.manifest.layers.iter().map(|l| l.name.as_str()).collect();
    (1..).map(|n| format!("{base} {n}")).find(|n| !names.contains(n.as_str())).unwrap()
}

/// Where a new layer goes: above the active one in its folder, or at the top of an active folder.
fn insertion(doc: &Doc) -> (Option<String>, usize) {
    let p = &doc.project;
    let active = doc.active_layer();
    let parent = match active {
        Some(a) if a.is_group() => Some(a.id.clone()),
        Some(a) => a.parent_id.clone(),
        None => None,
    };
    let mut at = doc.active.as_deref().and_then(|id| index(p, id)).map_or(p.manifest.layers.len(), |i| i + 1);
    if let Some(folder) = active.filter(|a| a.is_group()) {
        let inside = descendants(p, &folder.id);
        if let Some(top) = p.manifest.layers.iter().rposition(|l| inside.contains(&l.id)) {
            at = at.max(top + 1);
        }
    }
    (parent, at)
}

fn canvas(p: &Project) -> Transform {
    Transform::at(0.0, 0.0, p.manifest.width as f64, p.manifest.height as f64)
}

/// `addBlankLayer`: "Layer N", no pixels until painted.
pub fn new_layer(doc: &mut Doc) {
    let (parent, at) = insertion(doc);
    let mut layer = record(&next_name(&doc.project, "Layer"), canvas(&doc.project));
    layer.parent_id = parent.clone();
    let id = layer.id.clone();
    let mut next = doc.project.clone();
    next.manifest.layers.insert(at.min(next.manifest.layers.len()), layer);
    doc.commit("New Blank Layer", next);
    doc.active = Some(id);
    doc.mask_target = false;
    if let Some(parent) = parent {
        doc.collapsed.remove(&parent);
    }
}

/// `addGroup`: an empty "Folder N" above the active layer.
pub fn new_folder(doc: &mut Doc) {
    let p = &doc.project;
    let mut folder = record(&next_name(p, "Folder"), canvas(p));
    folder.is_group = Some(true);
    let active = doc.active_layer();
    folder.parent_id = match active {
        Some(a) if a.is_group() => Some(a.id.clone()),
        Some(a) => a.parent_id.clone(),
        None => None,
    };
    let at = doc.active.as_deref().and_then(|id| index(p, id)).map_or(p.manifest.layers.len(), |i| i + 1);
    let id = folder.id.clone();
    let mut next = doc.project.clone();
    next.manifest.layers.insert(at, folder);
    doc.commit("New Folder", next);
    doc.active = Some(id);
    doc.mask_target = false;
}

/// `addAdjustment`: a new adjustment layer above the active one. Gradient Map runs from the
/// foreground to the background color.
pub fn new_adjustment(doc: &mut Doc, kind: AdjustmentKind, foreground: [f32; 3], background: [f32; 3], seed: u32) {
    let (parent, at) = {
        let p = &doc.project;
        let active = doc.active_layer();
        let parent = match active {
            Some(a) if a.is_group() => Some(a.id.clone()),
            Some(a) => a.parent_id.clone(),
            None => None,
        };
        (parent, doc.active.as_deref().and_then(|id| index(p, id)).map_or(p.manifest.layers.len(), |i| i + 1))
    };
    let mut layer = record(kind.name(), canvas(&doc.project));
    let mut adjustment = Adjustment::new(kind);
    let color = |c: [f32; 3]| AdjustmentColor { red: c[0] as f64, green: c[1] as f64, blue: c[2] as f64 };
    match kind {
        AdjustmentKind::GradientMap => adjustment.gradient_map_settings = Some(GradientMapSettings { shadows: color(foreground), highlights: color(background), reversed: false }),
        AdjustmentKind::Grain => {
            let mut grain = comp_format::GrainSettings::default();
            grain.seed = seed;
            adjustment.grain_settings = Some(grain);
        }
        AdjustmentKind::AddNoise => adjustment.noise_seed = Some(seed),
        _ => {}
    }
    layer.adjustment = Some(adjustment);
    layer.parent_id = parent.clone();
    let id = layer.id.clone();
    let mut next = doc.project.clone();
    next.manifest.layers.insert(at, layer);
    doc.commit(&format!("New {} Adjustment", kind.name()), next);
    doc.active = Some(id);
    doc.mask_target = false;
    if let Some(parent) = parent {
        doc.collapsed.remove(&parent);
    }
}

/// Deletes the active layer with everything it holds. Layers clipped to it are released (the
/// Mac asks whether to bake them first; "Remove Links and Delete" is what happens here).
pub fn delete(doc: &mut Doc) {
    let Some(id) = doc.active.clone() else { return };
    let mut removed = descendants(&doc.project, &id);
    removed.insert(id.clone());
    let mut next = doc.project.clone();
    let below = {
        let layers = &next.manifest.layers;
        let i = index(&next, &id).unwrap_or(0);
        let parent = layers.get(i).and_then(|l| l.parent_id.clone());
        layers[..i].iter().rev().find(|l| l.parent_id == parent && !removed.contains(&l.id)).map(|l| l.id.clone())
    };
    next.manifest.layers.retain(|l| !removed.contains(&l.id));
    for l in &mut next.manifest.layers {
        if l.mask_source_id.as_ref().is_some_and(|s| removed.contains(s)) {
            l.mask_source_id = None;
        }
    }
    for r in &removed {
        next.images.remove(r);
        next.masks.remove(r);
    }
    let fallback = below.or_else(|| next.manifest.layers.last().map(|l| l.id.clone()));
    doc.commit("Delete Layer", next);
    doc.active = fallback;
    doc.mask_target = false;
}

/// Delete in the Layers panel: the mask when it's targeted, otherwise the layer.
pub fn delete_layer_or_mask(doc: &mut Doc) {
    if doc.mask_target && doc.active_layer().is_some_and(|l| l.mask_file.is_some()) {
        delete_mask(doc);
    } else {
        delete(doc);
    }
}

/// `insertCopy(of:)`: a copy of `id` and all it holds just above it; returns the copy's id.
fn insert_copy(p: &mut Project, id: &str) -> Option<String> {
    let i = index(p, id)?;
    let mut included = descendants(p, id);
    included.insert(id.to_string());
    let originals: Vec<LayerRecord> = p.manifest.layers.iter().filter(|l| included.contains(&l.id)).cloned().collect();
    let mapping: std::collections::HashMap<String, String> = originals.iter().map(|l| (l.id.clone(), new_id())).collect();
    let mut copies = Vec::new();
    for o in &originals {
        let mut c = o.clone();
        c.id = mapping[&o.id].clone();
        if o.id == id {
            c.name = format!("{} copy", o.name);
        }
        c.parent_id = o.parent_id.as_ref().map(|pid| mapping.get(pid).cloned().unwrap_or_else(|| pid.clone()));
        c.mask_source_id = o.mask_source_id.as_ref().map(|s| mapping.get(s).cloned().unwrap_or_else(|| s.clone()));
        if c.image_file.is_some() {
            c.image_file = Some(format!("{}.png", c.id));
        }
        if c.mask_file.is_some() {
            c.mask_file = Some(format!("{}.mask.png", c.id));
        }
        if let Some(img) = p.images.get(&o.id).cloned() {
            p.images.insert(c.id.clone(), img);
        }
        if let Some(m) = p.masks.get(&o.id).cloned() {
            p.masks.insert(c.id.clone(), m);
        }
        copies.push(c);
    }
    let at = i + 1;
    for (k, c) in copies.into_iter().enumerate() {
        p.manifest.layers.insert(at + k, c);
    }
    mapping.get(id).cloned()
}

/// Duplicate Layer (⌘J) on `id`: the copy goes just above and becomes active.
pub fn duplicate(doc: &mut Doc, id: &str) -> Option<String> {
    let mut next = doc.project.clone();
    let copy = insert_copy(&mut next, id)?;
    doc.commit("Duplicate Layer", next);
    doc.active = Some(copy.clone());
    Some(copy)
}

pub fn rename(doc: &mut Doc, id: &str, name: &str) {
    let name = name.trim().to_string();
    if name.is_empty() {
        return;
    }
    doc.edit("Rename Layer", false, |m| {
        if let Some(l) = m.layers.iter_mut().find(|l| l.id == id) {
            l.name = name;
        }
    });
}

/// `releaseDetachedClipping`: a layer stops clipping when it's no longer in the run above its base.
fn release_detached_clipping(layers: &mut [LayerRecord]) {
    let mut release = HashSet::new();
    let parents: HashSet<Option<String>> = layers.iter().map(|l| l.parent_id.clone()).collect();
    for parent in parents {
        let mut base: Option<String> = None;
        for l in layers.iter().filter(|l| l.parent_id == parent) {
            if let Some(source) = &l.mask_source_id {
                if Some(source) != base.as_ref() {
                    release.insert(l.id.clone());
                    base = Some(l.id.clone());
                }
            } else {
                base = if l.is_group() { None } else { Some(l.id.clone()) };
            }
        }
    }
    for l in layers.iter_mut() {
        if release.contains(&l.id) {
            l.mask_source_id = None;
        }
    }
}

/// `groupSelectedLayers` for the active layer: a new "Folder N" holding it, in its place.
pub fn group(doc: &mut Doc) {
    let Some(id) = doc.active.clone() else { return };
    let p = &doc.project;
    let Some(i) = index(p, &id) else { return };
    let parent = p.manifest.layers[i].parent_id.clone();
    let mut folder = record(&next_name(p, "Folder"), canvas(p));
    folder.is_group = Some(true);
    folder.parent_id = parent;
    let folder_id = folder.id.clone();
    let mut next = doc.project.clone();
    // The folder takes the layer's place among its siblings, and the layer goes inside it (the
    // Mac appends the grouped layers after everything else; only sibling order matters).
    let mut layers: Vec<LayerRecord> = next.manifest.layers.iter().filter(|l| l.id != id).cloned().collect();
    layers.insert(i.min(layers.len()), folder);
    let mut child = next.manifest.layers[i].clone();
    child.parent_id = Some(folder_id.clone());
    layers.push(child);
    release_detached_clipping(&mut layers);
    next.manifest.layers = layers;
    doc.commit("Group Layers", next);
    doc.active = Some(folder_id);
}

/// `ungroupLayers`: the folder's children take its place.
pub fn ungroup(doc: &mut Doc) {
    let Some(folder) = doc.active_layer().filter(|l| l.is_group()).cloned() else { return };
    let mut next = doc.project.clone();
    let children: HashSet<String> = next.manifest.layers.iter().filter(|l| l.parent_id.as_deref() == Some(folder.id.as_str())).map(|l| l.id.clone()).collect();
    let mut kids: Vec<LayerRecord> = next.manifest.layers.iter().filter(|l| children.contains(&l.id)).cloned().collect();
    for k in &mut kids {
        k.parent_id = folder.parent_id.clone();
    }
    let mut layers = Vec::new();
    for l in &next.manifest.layers {
        if l.id == folder.id {
            layers.extend(kids.iter().cloned());
        } else if !children.contains(&l.id) {
            layers.push(l.clone());
        }
    }
    release_detached_clipping(&mut layers);
    next.manifest.layers = layers;
    next.masks.remove(&folder.id);
    let first = kids.first().map(|k| k.id.clone());
    doc.commit("Ungroup Layers", next);
    doc.active = first;
    doc.collapsed.remove(&folder.id);
}

/// `moveActiveLayerOutOfGroup`: into the folder's parent, just above the folder.
pub fn move_out_of_folder(doc: &mut Doc) {
    let Some(layer) = doc.active_layer().cloned() else { return };
    let Some(parent) = layer.parent_id.clone() else { return };
    let Some(folder) = doc.layer(&parent).cloned() else { return };
    let mut next = doc.project.clone();
    let layers = &mut next.manifest.layers;
    let i = layers.iter().position(|l| l.id == layer.id).unwrap();
    let mut moved = layers.remove(i);
    moved.parent_id = folder.parent_id.clone();
    // A folder's contents sit below it in the list, so "just above the folder" is just after it.
    let at = layers.iter().position(|l| l.id == folder.id).map_or(layers.len(), |f| f + 1);
    layers.insert(at, moved);
    release_detached_clipping(layers);
    doc.commit("Move Layer", next);
}

pub fn can_toggle_clipping(doc: &Doc, id: &str) -> bool {
    let p = &doc.project;
    let Some(layer) = doc.layer(id) else { return false };
    if layer.is_group() {
        return false;
    }
    if layer.mask_source_id.is_some() {
        return true;
    }
    let siblings: Vec<&LayerRecord> = p.manifest.layers.iter().filter(|l| l.parent_id == layer.parent_id).collect();
    let Some(i) = siblings.iter().position(|l| l.id == id) else { return false };
    i > 0 && !siblings[i - 1].is_group() && siblings[i - 1].adjustment.is_none() || i > 0 && siblings[i - 1].mask_source_id.is_some()
}

/// `toggleClippingMask` (Alt-click between rows, or the menu): clip to the next lower sibling
/// (its base if it's clipped already), or release this layer and those clipped above it.
pub fn toggle_clipping(doc: &mut Doc, id: &str) {
    let Some(layer) = doc.layer(id).cloned() else { return };
    if layer.is_group() {
        return;
    }
    let siblings: Vec<LayerRecord> = doc.project.manifest.layers.iter().filter(|l| l.parent_id == layer.parent_id).cloned().collect();
    let Some(i) = siblings.iter().position(|l| l.id == id) else { return };
    if let Some(source) = &layer.mask_source_id {
        let releases: HashSet<String> = siblings[i..].iter().take_while(|l| l.id == id || l.mask_source_id.as_ref() == Some(source)).map(|l| l.id.clone()).collect();
        doc.edit("Release Clipping Mask", false, |m| {
            for l in &mut m.layers {
                if releases.contains(&l.id) {
                    l.mask_source_id = None;
                }
            }
        });
        return;
    }
    if i == 0 {
        return;
    }
    let below = &siblings[i - 1];
    if below.is_group() {
        return;
    }
    let source = below.mask_source_id.clone().unwrap_or_else(|| below.id.clone());
    if doc.layer(&source).is_some_and(|s| s.is_group() || s.adjustment.is_some()) {
        return;
    }
    doc.edit("Create Clipping Mask", false, |m| {
        if let Some(l) = m.layers.iter_mut().find(|l| l.id == id) {
            l.mask_source_id = Some(source);
        }
    });
}

/// `addLayerMask`: a 1 × 1 white (reveal) or black (hide) mask.
pub fn add_mask(doc: &mut Doc, reveal: bool) {
    let Some(layer) = doc.active_layer().filter(|l| l.mask_file.is_none()).cloned() else { return };
    let mut next = doc.project.clone();
    let value = if reveal { 255 } else { 0 };
    next.masks.insert(layer.id.clone(), Asset::new(image::GrayImage::from_pixel(1, 1, image::Luma([value]))));
    if let Some(l) = next.manifest.layers.iter_mut().find(|l| l.id == layer.id) {
        l.mask_file = Some(format!("{}.mask.png", l.id));
        l.mask_enabled = Some(true);
        l.mask_linked = Some(true);
        l.mask_placement = None;
    }
    doc.commit(if reveal { "Add Reveal-All Mask" } else { "Add Hide-All Mask" }, next);
    doc.mask_target = true;
}

pub fn delete_mask(doc: &mut Doc) {
    let Some(layer) = doc.active_layer().filter(|l| l.mask_file.is_some()).cloned() else { return };
    let mut next = doc.project.clone();
    next.masks.remove(&layer.id);
    if let Some(l) = next.manifest.layers.iter_mut().find(|l| l.id == layer.id) {
        l.mask_file = None;
        l.mask_enabled = None;
        l.mask_linked = None;
        l.mask_placement = None;
    }
    doc.commit("Delete Layer Mask", next);
    doc.mask_target = false;
}

pub fn toggle_mask_link(doc: &mut Doc, id: &str) {
    let Some(layer) = doc.layer(id).filter(|l| l.mask_file.is_some()).cloned() else { return };
    let linked = layer.mask_linked();
    doc.edit(if linked { "Unlink Layer Mask" } else { "Link Layer Mask" }, false, |m| {
        if let Some(l) = m.layers.iter_mut().find(|l| l.id == id) {
            l.mask_linked = Some(!linked);
        }
    });
}

pub fn toggle_mask_enabled(doc: &mut Doc, id: &str) {
    let Some(layer) = doc.layer(id).filter(|l| l.mask_file.is_some()).cloned() else { return };
    let on = layer.mask_enabled();
    doc.edit(if on { "Disable Layer Mask" } else { "Enable Layer Mask" }, false, |m| {
        if let Some(l) = m.layers.iter_mut().find(|l| l.id == id) {
            l.mask_enabled = Some(!on);
        }
    });
}

/// Straight RGBA premultiplied as Core Graphics draws it into a premultiplied bitmap.
fn premultiplied(c: u8, a: u8) -> u8 {
    ((c as u32 * a as u32 + 127) / 255) as u8
}

/// Premultiplied bytes back to straight, as PNG export writes them.
fn straight(c: u8, a: u8) -> u8 {
    if a == 0 { 0 } else { ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8 }
}

/// `PixelInvert.run` over the whole layer: premultiplied colors become alpha − color (a mask's
/// values 255 − value).
pub fn invert(doc: &mut Doc) {
    let Some(layer) = doc.active_layer().cloned() else { return };
    let mut next = doc.project.clone();
    if doc.mask_target {
        let Some(mask) = next.masks.get_mut(&layer.id) else { return };
        if !layer.mask_enabled() {
            return;
        }
        for v in mask.pixels.iter_mut() {
            *v = 255 - *v;
        }
        mask.png = None;
        doc.commit("Invert Mask", next);
        return;
    }
    if layer.is_group() {
        return;
    }
    let Some(asset) = next.images.get_mut(&layer.id) else { return };
    for px in asset.pixels.pixels_mut() {
        let a = px[3];
        for c in 0..3 {
            let inverted = a - premultiplied(px[c], a);
            px[c] = straight(inverted, a);
        }
    }
    asset.png = None;
    doc.commit("Invert", next);
}

/// `LayerTransform.mirrored(horizontally:across:)`.
fn mirrored(t: &Transform, horizontal: bool, axis: f64) -> Transform {
    let mut r = *t;
    let c = crate::geometry::center(t);
    if horizontal {
        r.flip_x = !r.flip_x;
        r.origin[0] = 2.0 * axis - c[0] - t.size[0] / 2.0;
    } else {
        r.flip_y = !r.flip_y;
        r.origin[1] = 2.0 * axis - c[1] - t.size[1] / 2.0;
    }
    r.rotation = -t.rotation;
    r
}

/// `flipLayers`: the active layer about its own middle; a linked mask flips with it.
pub fn flip_layer(doc: &mut Doc, horizontal: bool) {
    let Some(layer) = doc.active_layer().filter(|l| !l.is_group() && doc.project.images.contains_key(&l.id)).cloned() else { return };
    let c = crate::geometry::center(&layer.transform);
    let flipped = mirrored(&layer.transform, horizontal, if horizontal { c[0] } else { c[1] });
    let next = crate::ui::canvas_tools::placed(&doc.project, &layer.id, flipped);
    doc.commit(if horizontal { "Flip Horizontal" } else { "Flip Vertical" }, next);
}

/// `flipCanvas`: every layer, placed mask and guide mirrored across the canvas's middle.
pub fn flip_canvas(doc: &mut Doc, horizontal: bool) {
    let axis = if horizontal { doc.project.manifest.width as f64 / 2.0 } else { doc.project.manifest.height as f64 / 2.0 };
    doc.edit(if horizontal { "Flip Canvas Horizontal" } else { "Flip Canvas Vertical" }, false, |m| {
        for l in &mut m.layers {
            l.transform = mirrored(&l.transform, horizontal, axis);
            if let Some(p) = &mut l.mask_placement {
                *p = mirrored(p, horizontal, axis);
            }
        }
        for g in m.guides.iter_mut().flatten() {
            let along = matches!(g.axis, comp_format::GuideAxis::Vertical) == horizontal;
            if along {
                g.position = 2.0 * axis - g.position;
            }
        }
    });
}

/// What ⌘E merges: (ids in stacking order, removed, name, parent, anchor, title).
struct MergePlan {
    ids: Vec<String>,
    removed: HashSet<String>,
    name: String,
    parent: Option<String>,
    anchor: String,
    action: &'static str,
}

fn merge_plan(doc: &Doc) -> Option<MergePlan> {
    let active = doc.active_layer()?;
    let layers = &doc.project.manifest.layers;
    if active.is_group() {
        let inside = descendants(&doc.project, &active.id);
        if !layers.iter().any(|l| inside.contains(&l.id) && !l.is_group()) {
            return None;
        }
        let ids: Vec<String> = layers.iter().filter(|l| inside.contains(&l.id) || l.id == active.id).map(|l| l.id.clone()).collect();
        return Some(MergePlan { removed: ids.iter().cloned().collect(), ids, name: active.name.clone(), parent: active.parent_id.clone(), anchor: active.id.clone(), action: "Merge Group" });
    }
    let i = layers.iter().position(|l| l.id == active.id)?;
    let below = layers[..i].iter().rev().find(|l| l.parent_id == active.parent_id)?;
    if below.is_group() {
        return None;
    }
    let ids = vec![below.id.clone(), active.id.clone()];
    Some(MergePlan { removed: ids.iter().cloned().collect(), ids, name: below.name.clone(), parent: active.parent_id.clone(), anchor: active.id.clone(), action: "Merge Down" })
}

pub fn merge_title(doc: &Doc) -> Option<&'static str> {
    merge_plan(doc).map(|p| p.action)
}

/// `mergeLayers`: the layers composited as the canvas shows them into one pixel layer, trimmed.
pub fn merge(doc: &mut Doc, gpu: &engine::gpu::Gpu) -> Result<(), engine::RenderError> {
    let Some(plan) = merge_plan(doc) else { return Ok(()) };
    let kept: HashSet<&String> = plan.ids.iter().collect();
    let mut subset = doc.project.clone();
    subset.manifest.layers = doc
        .project
        .manifest
        .layers
        .iter()
        .filter(|l| kept.contains(&l.id))
        .map(|l| {
            let mut c = l.clone();
            if c.parent_id.as_ref().is_some_and(|p| !kept.contains(p)) {
                c.parent_id = None;
            }
            if c.mask_source_id.as_ref().is_some_and(|s| !kept.contains(s)) {
                c.mask_source_id = None;
            }
            c
        })
        .collect();
    let canvas = engine::composite::Compositor::new(gpu, &subset).render()?;
    let straight_canvas = engine::blend::unpremultiply(gpu, &canvas);
    let bytes = gpu.download(&straight_canvas)?;
    let full = image::RgbaImage::from_raw(canvas.width, canvas.height, bytes).expect("canvas size");
    // `PixelFilter.trimmed`.
    let (w, h) = full.dimensions();
    let (mut left, mut right, mut top, mut bottom) = (w, 0, h, 0);
    for y in 0..h {
        let Some(first) = (0..w).find(|&x| full.get_pixel(x, y)[3] != 0) else { continue };
        let last = (first..w).rev().find(|&x| full.get_pixel(x, y)[3] != 0).unwrap() + 1;
        left = left.min(first);
        right = right.max(last);
        top = top.min(y);
        bottom = y + 1;
    }
    let (image, rect) = if right > left && bottom > top {
        (image::imageops::crop_imm(&full, left, top, right - left, bottom - top).to_image(), (left, top, right - left, bottom - top))
    } else {
        (full, (0, 0, w, h))
    };
    let mut merged = record(&plan.name, Transform::at(rect.0 as f64, rect.1 as f64, rect.2 as f64, rect.3 as f64));
    merged.parent_id = plan.parent.clone();
    merged.image_file = Some(format!("{}.png", merged.id));
    let id = merged.id.clone();
    let mut next = doc.project.clone();
    let slot = next.manifest.layers.iter().position(|l| l.id == plan.anchor).unwrap_or(next.manifest.layers.len());
    let at = slot - next.manifest.layers[..slot].iter().filter(|l| plan.removed.contains(&l.id)).count();
    next.manifest.layers.retain(|l| !plan.removed.contains(&l.id));
    for l in &mut next.manifest.layers {
        if l.mask_source_id.as_ref().is_some_and(|s| plan.removed.contains(s)) {
            l.mask_source_id = Some(id.clone());
        }
    }
    let at = at.min(next.manifest.layers.len());
    next.manifest.layers.insert(at, merged);
    for r in &plan.removed {
        next.images.remove(r);
        next.masks.remove(r);
    }
    next.images.insert(id.clone(), Asset::new(image));
    doc.commit(plan.action, next);
    doc.active = Some(id);
    doc.mask_target = false;
    Ok(())
}

/// A seed for Grain and Add Noise, which the Mac draws at random for each new layer or panel.
pub fn random_seed() -> u32 {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos() as u64 ^ d.as_secs());
    let mut x = nanos.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (std::process::id() as u64);
    x ^= x >> 33;
    x as u32
}

/// Whether `t` places a `w` × `h` image 1:1 and upright on whole pixels.
fn plain(t: &Transform, w: u32, h: u32) -> bool {
    t.rotation == 0.0 && !t.flip_x && !t.flip_y && t.size == [w as f64, h as f64] && t.origin[0].fract() == 0.0 && t.origin[1].fract() == 0.0
}

/// The selection's coverage (0…255) over a layer's pixel grid at `t`, or `None` when the layer
/// isn't placed 1:1 and upright.
fn coverage_in_grid(doc: &Doc, gfx: &crate::gfx::Gfx, t: &Transform, w: u32, h: u32) -> Option<Vec<u8>> {
    let selection = doc.selection.as_ref()?;
    if !plain(t, w, h) {
        return None;
    }
    let canvas = gfx.engine.selection_coverage(&doc.project, selection).ok()?;
    let (cw, ch) = canvas.dimensions();
    let (ox, oy) = (t.origin[0] as i64, t.origin[1] as i64);
    let mut out = vec![0u8; (w * h) as usize];
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let (dx, dy) = (x + ox, y + oy);
            if dx >= 0 && dy >= 0 && dx < cw as i64 && dy < ch as i64 {
                out[(y * w as i64 + x) as usize] = canvas.get_pixel(dx as u32, dy as u32)[0];
            }
        }
    }
    Some(out)
}

/// Blends `after`'s pixels for `id` over `before`'s through the selection, premultiplied, as
/// `PixelAdjust.blend` does with Core Image's blend-with-mask. The engine's operations don't take
/// a selection, so this is the app's approximation: returns false when it can't (a transformed
/// layer, or the operation resized the layer).
pub fn limit_to_selection(doc: &Doc, gfx: &crate::gfx::Gfx, after: &mut Project, id: &str) -> bool {
    let (Some(old), Some(layer)) = (doc.project.images.get(id), doc.layer(id)) else { return false };
    let Some(new) = after.images.get_mut(id) else { return false };
    let Some(new_layer) = after.manifest.layers.iter().find(|l| l.id == id) else { return false };
    if new.pixels.dimensions() != old.pixels.dimensions() || new_layer.transform != layer.transform {
        return false;
    }
    let (w, h) = old.pixels.dimensions();
    let Some(cover) = coverage_in_grid(doc, gfx, &layer.transform, w, h) else { return false };
    for (i, (n, o)) in new.pixels.pixels_mut().zip(old.pixels.pixels()).enumerate() {
        let m = cover[i] as f64 / 255.0;
        let (na, oa) = (n[3], o[3]);
        let mut out = [0f64; 4];
        for c in 0..3 {
            let (pn, po) = (premultiplied(n[c], na) as f64, premultiplied(o[c], oa) as f64);
            out[c] = po + (pn - po) * m;
        }
        out[3] = oa as f64 + (na as f64 - oa as f64) * m;
        let a = out[3].round().clamp(0.0, 255.0) as u8;
        for c in 0..3 {
            n[c] = straight(out[c].round().clamp(0.0, 255.0) as u8, a);
        }
        n[3] = a;
    }
    new.png = None;
    true
}

/// Fill with Foreground or Background Color (`fillSelection`): the whole layer without a
/// selection (grown to cover the canvas, as the brush raster is), or through the selection; on a
/// mask, the color's gray.
pub fn fill(doc: &mut Doc, color: [f32; 3], gfx: &crate::gfx::Gfx) {
    let Some(layer) = doc.active_layer().filter(|l| !l.is_group() && l.adjustment.is_none()).cloned() else { return };
    let rgba = [0, 1, 2].map(|i| (color[i].clamp(0.0, 1.0) * 255.0).round() as u8);
    let mut next = doc.project.clone();
    if doc.mask_target {
        let Some(mask) = next.masks.get_mut(&layer.id) else { return };
        let value = rgba[0];
        if doc.selection.is_none() {
            mask.pixels = image::GrayImage::from_pixel(1, 1, image::Luma([value]));
        } else {
            let (w, h) = mask.pixels.dimensions();
            let placement = layer.mask_placement.unwrap_or(layer.transform);
            let Some(cover) = coverage_in_grid(doc, gfx, &placement, w, h) else { return };
            for (i, v) in mask.pixels.iter_mut().enumerate() {
                let m = cover[i] as f64 / 255.0;
                *v = (*v as f64 + (value as f64 - *v as f64) * m).round() as u8;
            }
        }
        mask.png = None;
        doc.commit("Fill Mask", next);
        return;
    }
    let (w, h) = (next.manifest.width as f64, next.manifest.height as f64);
    let t = layer.transform;
    let existing = next.images.get(&layer.id).map(|a| a.pixels.clone());
    if existing.as_ref().is_some_and(|img| !plain(&t, img.width(), img.height())) {
        return;
    }
    // The raster covers the layer and the canvas.
    let (x0, y0) = (t.origin[0].min(0.0), t.origin[1].min(0.0));
    let (x1, y1) = ((t.origin[0] + t.size[0]).max(w), (t.origin[1] + t.size[1]).max(h));
    let (gw, gh) = ((x1 - x0) as u32, (y1 - y0) as u32);
    let mut grid = image::RgbaImage::new(gw, gh);
    if let Some(img) = &existing {
        image::imageops::replace(&mut grid, img, (t.origin[0] - x0) as i64, (t.origin[1] - y0) as i64);
    }
    let before = grid.clone();
    for p in grid.pixels_mut() {
        *p = image::Rgba([rgba[0], rgba[1], rgba[2], 255]);
    }
    let grown = Transform { origin: [x0, y0], size: [gw as f64, gh as f64], ..t };
    if let Some(l) = next.manifest.layers.iter_mut().find(|l| l.id == layer.id) {
        l.transform = grown;
        l.image_file = Some(format!("{}.png", l.id));
        l.shape = None;
        l.text = None;
    }
    if doc.selection.is_some() {
        let Some(cover) = coverage_in_grid(doc, gfx, &grown, gw, gh) else { return };
        for (i, (n, o)) in grid.pixels_mut().zip(before.pixels()).enumerate() {
            let m = cover[i] as f64 / 255.0;
            let mut out = [0f64; 4];
            for c in 0..3 {
                let (pn, po) = (n[c] as f64, premultiplied(o[c], o[3]) as f64);
                out[c] = po + (pn - po) * m;
            }
            out[3] = o[3] as f64 + (255.0 - o[3] as f64) * m;
            let a = out[3].round() as u8;
            for c in 0..3 {
                n[c] = straight(out[c].round() as u8, a);
            }
            n[3] = a;
        }
    }
    next.images.insert(layer.id.clone(), Asset::new(grid));
    doc.commit("Fill", next);
}

/// Delete with a selection (`clearSelectedPixels`): the selected pixels become transparent; on a
/// mask the selection fills with black.
pub fn clear_selected(doc: &mut Doc, gfx: &crate::gfx::Gfx) {
    if doc.selection.is_none() {
        return;
    }
    if doc.mask_target {
        fill(doc, [0.0; 3], gfx);
        return;
    }
    let Some(layer) = doc.active_layer().cloned() else { return };
    let Some(img) = doc.project.images.get(&layer.id) else { return };
    let (w, h) = img.pixels.dimensions();
    let Some(cover) = coverage_in_grid(doc, gfx, &layer.transform, w, h) else { return };
    let mut next = doc.project.clone();
    let asset = next.images.get_mut(&layer.id).unwrap();
    for (i, p) in asset.pixels.pixels_mut().enumerate() {
        let keep = 1.0 - cover[i] as f64 / 255.0;
        let a = p[3];
        let na = (a as f64 * keep).round() as u8;
        for c in 0..3 {
            p[c] = straight((premultiplied(p[c], a) as f64 * keep).round() as u8, na);
        }
        p[3] = na;
    }
    asset.png = None;
    doc.commit("Clear", next);
}
