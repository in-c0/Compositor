//! Crop, Canvas Size and Image Size, as the Mac app applies them (`CanvasResizer`, `Crop`,
//! `ImageResizer`, then `EditorSession.applyDocumentSize`).

use crate::RenderError;
use comp_format::Project;

pub struct CanvasSize {
    pub width: i64,
    pub height: i64,
    /// 0...8, row by row from the top left; 4 is the center.
    pub anchor: i64,
    /// A color for the new area, 0...1 per channel; transparent when `None`.
    pub fill: Option<[f64; 3]>,
    /// Moves the content by exactly this much, overriding `anchor` (Crop uses it).
    pub content_offset: Option<[f64; 2]>,
}

impl CanvasSize {
    /// `CanvasSizeOptions.offset(fromWidth:height:)`.
    fn offset(&self, old_width: i64, old_height: i64) -> [f64; 2] {
        if let Some(o) = self.content_offset {
            return o;
        }
        // Floor puts the extra pixel on the right and bottom when growing.
        [
            ((self.width - old_width) as f64 * (self.anchor % 3) as f64 / 2.0).floor(),
            ((self.height - old_height) as f64 * (self.anchor / 3) as f64 / 2.0).floor(),
        ]
    }
}

/// `CanvasResizer.resize`, then `applyDocumentSize`.
pub fn canvas_size(project: &Project, options: &CanvasSize) -> Result<Project, RenderError> {
    let old = &project.manifest;
    if !(1..=30_000).contains(&options.width) || !(1..=30_000).contains(&options.height) || !(0..=8).contains(&options.anchor) {
        return Err(RenderError::Failed(anyhow::anyhow!("canvas size out of range")));
    }
    let offset = options.offset(old.width, old.height);
    let mut result = project.clone();
    if options.width == old.width && options.height == old.height && offset == [0.0, 0.0] {
        return Ok(apply_document_size(result));
    }
    let m = &mut result.manifest;
    m.width = options.width;
    m.height = options.height;
    if let Some(guides) = &mut m.guides {
        for g in guides {
            g.position += match g.axis {
                comp_format::GuideAxis::Horizontal => offset[1],
                comp_format::GuideAxis::Vertical => offset[0],
            };
        }
    }
    for layer in &mut m.layers {
        layer.transform.origin[0] += offset[0];
        layer.transform.origin[1] += offset[1];
        if let Some(p) = &mut layer.mask_placement {
            p.origin[0] += offset[0];
            p.origin[1] += offset[1];
        }
        // CanvasResizer doesn't carry layer effects over.
        layer.effects = None;
    }
    // A colored extension is a new bottom layer, the color everywhere but where the old canvas
    // was, which stays transparent.
    if let (Some(fill), true) = (options.fill, options.width > old.width || options.height > old.height) {
        let color = fill.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
        let (w, h) = (options.width as u32, options.height as u32);
        let hole = (offset[0] as i64, offset[1] as i64, old.width, old.height);
        let pixels = image::RgbaImage::from_fn(w, h, |x, y| {
            let (x, y) = (x as i64, y as i64);
            let inside = x >= hole.0 && y >= hole.1 && x < hole.0 + hole.2 && y < hole.1 + hole.3;
            if inside { image::Rgba([0, 0, 0, 0]) } else { image::Rgba([color[0], color[1], color[2], 255]) }
        });
        let id = uuid::Uuid::new_v4().to_string().to_ascii_uppercase();
        result.manifest.layers.insert(0, comp_format::LayerRecord {
            image_file: Some(format!("{id}.png")),
            id: id.clone(),
            name: "Canvas Extension".into(),
            is_visible: true,
            transform: comp_format::Transform::at(0.0, 0.0, w as f64, h as f64),
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
        });
        result.images.insert(id, comp_format::Asset::new(pixels));
    }
    Ok(apply_document_size(result))
}

/// Crop to `rect` ([x, y, width, height] in whole document pixels), as `commitCrop` does.
pub fn crop(project: &Project, rect: [f64; 4]) -> Result<Project, RenderError> {
    canvas_size(project, &CanvasSize {
        width: rect[2] as i64,
        height: rect[3] as i64,
        anchor: 4,
        fill: None,
        content_offset: Some([-rect[0], -rect[1]]),
    })
}

/// `applyDocumentSize` rebuilds every layer without its live shape, text and layer effects.
fn apply_document_size(mut project: Project) -> Project {
    for layer in &mut project.manifest.layers {
        layer.shape = None;
        layer.text = None;
        layer.effects = None;
    }
    project
}

