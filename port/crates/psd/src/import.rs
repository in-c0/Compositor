//! A Photoshop file opened as the Mac app opens it: `EditorSession.importImages` into an empty
//! session, which reads the file (`PSDReader`), builds the layers (`PSDDocumentBuilder.makeImport`)
//! and makes the document from them (`EditorSession.insertPhotoshop`). A file with no layer records
//! comes in as its merged image, one layer named after the file.

use crate::reader::{self, CompositeResult, Document, ImportError, Kind, Premultiplied, Record, Result};
use comp_format::{Asset, BlendMode, GrayImage, LayerRecord, Manifest, Project, RgbaImage, Transform};
use std::collections::HashMap;

/// A conversion the Mac lists for the person to accept before importing (`PSDConversion`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conversion {
    pub layer_name: String,
    pub message: String,
}

pub struct Import {
    pub project: Project,
    pub conversions: Vec<Conversion>,
}

/// Opens `path` as a new document.
pub fn import_file(path: &std::path::Path) -> Result<Import> {
    let data = std::fs::read(path).map_err(|_| ImportError::Unreadable)?;
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    import(&data, &stem)
}

/// Opens a Photoshop file's bytes as a new document; `name` is the file name without extension.
pub fn import(data: &[u8], name: &str) -> Result<Import> {
    let document = reader::read(data, reader::DOCUMENT_PIXEL_BUDGET)?;
    if document.layers.is_empty() {
        return flattened(&document, name);
    }
    build(&document)
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string().to_uppercase()
}

fn manifest(width: u32, height: u32, resolution: f64, layers: Vec<LayerRecord>, active: Option<String>) -> Manifest {
    Manifest {
        format: comp_format::FORMAT.into(),
        version: comp_format::CURRENT_VERSION,
        color_space: "sRGB".into(),
        resolution: Some(resolution),
        document_id: new_id(),
        width: width as i64,
        height: height as i64,
        active_layer_id: active,
        layers,
        guides: None,
    }
}

fn layer_record(id: &str, name: &str, visible: bool, transform: Transform) -> LayerRecord {
    LayerRecord {
        id: id.into(),
        name: name.into(),
        is_visible: visible,
        transform,
        image_file: None,
        parent_id: None,
        is_group: Some(false),
        opacity: Some(1.0),
        blend_mode: Some(BlendMode::Normal),
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

/// Only a background: Photoshop wrote no layer records, just the merged image, and that comes in
/// as one layer (`ImageImporter.decode(flattenedPhotoshop:)` then `EditorSession.insert`).
fn flattened(document: &Document, name: &str) -> Result<Import> {
    let pixels = match &document.composite {
        Some(CompositeResult::Pixels(p)) => p.clone(),
        Some(CompositeResult::Failed(e)) => return Err(e.clone()),
        None => return Err(ImportError::Unreadable),
    };
    let (w, h) = pixels.dimensions();
    // A new document takes the image's size and the default resolution; the image is centered.
    let id = new_id();
    let origin = ((w as f64 / 2.0 - w as f64 / 2.0).floor(), (h as f64 / 2.0 - h as f64 / 2.0).floor());
    let mut layer = layer_record(&id, name, true, Transform::at(origin.0, origin.1, w as f64, h as f64));
    layer.image_file = Some(format!("{id}.png"));
    let mut images = HashMap::new();
    images.insert(id.clone(), Asset::new(pixels));
    Ok(Import { project: Project { manifest: manifest(w, h, 72.0, vec![layer], Some(id)), images, masks: HashMap::new() }, conversions: Vec::new() })
}

/// `LayerBlendMode.fromPSD`. Dissolve, Darker Color and Lighter Color have no equivalent.
pub fn blend_mode(key: &str) -> Option<BlendMode> {
    Some(match key {
        "norm" => BlendMode::Normal,
        "mul " => BlendMode::Multiply,
        "scrn" => BlendMode::Screen,
        "over" => BlendMode::Overlay,
        "sLit" => BlendMode::SoftLight,
        "dark" => BlendMode::Darken,
        "lite" => BlendMode::Lighten,
        "diff" => BlendMode::Difference,
        "div " => BlendMode::ColorDodge,
        "idiv" => BlendMode::ColorBurn,
        "hue " => BlendMode::Hue,
        "sat " => BlendMode::Saturation,
        "colr" => BlendMode::Color,
        "lum " => BlendMode::Luminosity,
        "lbrn" => BlendMode::LinearBurn,
        "lddg" => BlendMode::LinearDodge,
        "hLit" => BlendMode::HardLight,
        "vLit" => BlendMode::VividLight,
        "lLit" => BlendMode::LinearLight,
        "pLit" => BlendMode::PinLight,
        "hMix" => BlendMode::HardMix,
        "smud" => BlendMode::Exclusion,
        "fsub" => BlendMode::Subtract,
        "fdiv" => BlendMode::Divide,
        _ => return None,
    })
}

/// A premultiplied image as its project PNG stores it: unpremultiplied with `(c*255 + a/2)/a`.
fn straight(image: &Premultiplied) -> RgbaImage {
    let mut out = image.rgba.clone();
    for px in out.chunks_exact_mut(4) {
        let a = px[3] as u32;
        for c in &mut px[..3] {
            *c = if a == 0 { 0 } else { ((*c as u32 * 255 + a / 2) / a).min(255) as u8 };
        }
    }
    RgbaImage::from_raw(image.width, image.height, out).expect("image size")
}

struct Built {
    record: LayerRecord,
    image: Option<RgbaImage>,
    mask: Option<GrayImage>,
}

/// `PSDDocumentBuilder.makeImport`, then `EditorSession.insertPhotoshop` into an empty session.
fn build(document: &Document) -> Result<Import> {
    let canvas = (document.width as f64, document.height as f64);
    let canvas_transform = Transform::at(0.0, 0.0, canvas.0, canvas.1);
    let ids: HashMap<usize, String> = document.layers.iter().map(|r| (r.id, new_id())).collect();
    let mut conversions = Vec::new();
    let mut built: Vec<Built> = Vec::new();
    let mut index_of: HashMap<usize, usize> = HashMap::new();
    for record in &document.layers {
        let note = |conversions: &mut Vec<Conversion>, message: String| conversions.push(Conversion { layer_name: record.name.clone(), message });
        if record.cropped_to_canvas {
            note(&mut conversions, "Cropped to the canvas so the file fits in memory. Pixels outside the canvas weren't imported.".into());
        }
        match record.kind {
            Kind::Text if !record.text => note(&mut conversions, "Editable Photoshop text becomes pixels and can\u{2019}t be retyped.".into()),
            Kind::SmartObject => note(&mut conversions, "The smart object was rasterized. Linked contents can\u{2019}t be edited.".into()),
            Kind::Effects => note(&mut conversions, "Layer effects were discarded, so the appearance may differ.".into()),
            Kind::Vector if !record.is_shape => note(&mut conversions, "Vector shape was rasterized to pixels.".into()),
            _ => {}
        }
        if record.is_group {
            if record.blend_key != "pass" && record.blend_key != "norm" {
                note(&mut conversions, format!("Folder blend mode \u{201c}{}\u{201d} isn\u{2019}t supported. The folder will be pass-through.", record.blend_key));
            }
        } else if blend_mode(&record.blend_key).is_none() && record.blend_key != "pass" {
            note(&mut conversions, format!("Blend mode \u{201c}{}\u{201d} isn\u{2019}t supported and will be applied as Normal.", record.blend_key.trim()));
        }
        if record.kind == Kind::Adjustment {
            let message = if record.adjustment.is_none() {
                "This adjustment type isn\u{2019}t supported and was skipped."
            } else {
                "Adjustment parameters may not match Photoshop exactly."
            };
            note(&mut conversions, message.into());
            if record.adjustment.is_none() {
                continue;
            }
        }
        let id = ids[&record.id].clone();
        let opacity = record.opacity.clamp(0.0, 1.0);
        let mode = blend_mode(&record.blend_key).unwrap_or(BlendMode::Normal);
        let mut layer = layer_record(&id, &record.name, record.is_visible, canvas_transform);
        layer.parent_id = record.parent.map(|p| ids[&p].clone());
        layer.opacity = Some(opacity);
        let mut image = None;
        if record.is_group {
            // Folders are always pass-through, with an opacity of their own.
            layer.is_group = Some(true);
        } else if let Some(adjustment) = &record.adjustment {
            layer.blend_mode = Some(mode);
            layer.adjustment = Some(adjustment.clone());
        } else if record.text {
            return Err(ImportError::NotPorted("editable Photoshop text (drawn by Core Text)".into()));
        } else if let Some(what) = record.drawn_by_mac {
            return Err(ImportError::NotPorted(what.into()));
        } else if let Some(pixels) = &record.image {
            let b = record.bounds;
            let size = if b.width > 0.0 && b.height > 0.0 { [b.width, b.height] } else { [pixels.width as f64, pixels.height as f64] };
            layer.transform = Transform::at(b.x, b.y, size[0], size[1]);
            layer.blend_mode = Some(mode);
            layer.image_file = Some(format!("{id}.png"));
            image = Some(straight(pixels));
        } else {
            layer.blend_mode = Some(mode);
        }
        let mut mask = None;
        if let Some(patch) = &record.mask {
            let grid = image.as_ref().map(|i| (i.width(), i.height())).unwrap_or((document.width, document.height));
            mask = Some(mask_on_layer_grid(patch, record, &layer.transform, grid)?);
            layer.mask_file = Some(format!("{id}.mask.png"));
            layer.mask_enabled = Some(record.mask_enabled);
            layer.mask_linked = Some(record.mask_linked);
        }
        index_of.insert(record.id, built.len());
        built.push(Built { record: layer, image, mask });
    }
    // Clipping: a clipped layer clips to the nearest unclipped pixel layer below it in the same
    // folder; folders and adjustment layers can't be bases.
    let mut base_for_parent: HashMap<Option<usize>, usize> = HashMap::new();
    for record in &document.layers {
        let Some(&index) = index_of.get(&record.id) else { continue };
        let is_base = |b: &Built| !b.record.is_group() && b.record.adjustment.is_none();
        if record.clipping {
            match base_for_parent.get(&record.parent) {
                Some(&source) if is_base(&built[index_of[&source]]) => {
                    built[index].record.mask_source_id = Some(ids[&source].clone());
                }
                _ => conversions.push(Conversion {
                    layer_name: record.name.clone(),
                    message: "This clipping mask\u{2019}s base isn\u{2019}t supported, so clipping was skipped.".into(),
                }),
            }
        } else if is_base(&built[index]) {
            base_for_parent.insert(record.parent, record.id);
        } else {
            base_for_parent.remove(&record.parent);
        }
    }
    let active = built.iter().rev().find(|b| b.record.parent_id.is_none()).or(built.last()).map(|b| b.record.id.clone());
    let mut images = HashMap::new();
    let mut masks = HashMap::new();
    let mut layers = Vec::new();
    for b in built {
        if let Some(image) = b.image {
            images.insert(b.record.id.clone(), Asset::new(image));
        }
        if let Some(mask) = b.mask {
            masks.insert(b.record.id.clone(), Asset::new(mask));
        }
        layers.push(b.record);
    }
    let manifest = manifest(document.width, document.height, document.resolution, layers, active);
    Ok(Import { project: Project { manifest, images, masks }, conversions })
}

/// `PSDDocumentBuilder.maskOnLayerGrid`: the stored patch placed where it sits on the document,
/// on the layer's own pixel grid (the canvas for layers without pixels), and the mask's default
/// value everywhere else.
fn mask_on_layer_grid(patch: &reader::Gray, record: &Record, transform: &Transform, grid: (u32, u32)) -> Result<GrayImage> {
    let as_is = || GrayImage::from_raw(patch.width, patch.height, patch.pixels.clone()).expect("mask size");
    let placed = (transform.origin[0], transform.origin[1], transform.size[0], transform.size[1]);
    let mb = record.mask_bounds;
    if grid.0 < 1 || grid.1 < 1 || placed.2 <= 0.0 || placed.3 <= 0.0 || mb.width <= 0.0 || mb.height <= 0.0 {
        return Ok(as_is());
    }
    let (sx, sy) = (grid.0 as f64 / placed.2, grid.1 as f64 / placed.3);
    let rect = ((mb.x - placed.0) * sx, (mb.y - placed.1) * sy, mb.width * sx, mb.height * sy);
    if rect.0 == 0.0 && rect.1 == 0.0 && rect.2 == grid.0 as f64 && rect.3 == grid.1 as f64 && patch.width == grid.0 && patch.height == grid.1 {
        return Ok(as_is());
    }
    // Layers come in at their pixel size, so the patch always lands 1:1 on whole pixels.
    if sx != 1.0 || sy != 1.0 || rect.0.fract() != 0.0 || rect.1.fract() != 0.0 {
        return Err(ImportError::NotPorted("a Photoshop mask off the layer's pixel grid".into()));
    }
    let (dx, dy) = (rect.0 as i64, rect.1 as i64);
    let mut out = GrayImage::from_pixel(grid.0, grid.1, image::Luma([record.mask_default]));
    for y in 0..grid.1 as i64 {
        let py = y - dy;
        if py < 0 || py >= patch.height as i64 {
            continue;
        }
        for x in 0..grid.0 as i64 {
            let px = x - dx;
            if px < 0 || px >= patch.width as i64 {
                continue;
            }
            out.put_pixel(x as u32, y as u32, image::Luma([patch.pixels[(py * patch.width as i64 + px) as usize]]));
        }
    }
    Ok(out)
}
