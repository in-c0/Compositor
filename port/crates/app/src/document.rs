//! One open project tab: the project, how it's viewed, and its rendered canvas.

use crate::gfx::{CanvasTexture, Gfx};
use comp_format::{BlendMode, LayerRecord, Manifest, Project, Transform};
use eframe::egui::{Vec2, vec2};
use engine::RenderError;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// `CanvasViewport`: zoom is device pixels per document pixel, as on the Mac, so 100% shows
/// actual pixels on any display.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub view_size: Vec2,
    pub pixels_per_point: f32,
    pub zoom: f32,
    pub pan: Vec2,
    pub follows_fit: bool,
}

impl Default for Viewport {
    fn default() -> Self {
        Self { view_size: Vec2::ZERO, pixels_per_point: 1.0, zoom: 1.0, pan: Vec2::ZERO, follows_fit: true }
    }
}

pub const ZOOM_RANGE: (f32, f32) = (0.001, 32.0);
pub const KEYBOARD_ZOOMS: [f32; 17] =
    [0.125, 1.0 / 6.0, 0.25, 1.0 / 3.0, 0.5, 2.0 / 3.0, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 12.0, 16.0];
/// From 200% the canvas shows hard-edged pixels; the pixel grid appears from 800%.
pub const CRISP_ZOOM: f32 = 2.0;
pub const PIXEL_GRID_ZOOM: f32 = 8.0;

impl Viewport {
    pub fn points_per_pixel(&self) -> f32 {
        self.zoom / self.pixels_per_point
    }

    /// The document's rectangle relative to the view's top left.
    pub fn document_rect(&self, size: Vec2) -> eframe::egui::Rect {
        let scaled = size * self.points_per_pixel();
        let min = self.view_size / 2.0 - scaled / 2.0 + self.pan;
        eframe::egui::Rect::from_min_size(min.to_pos2(), scaled)
    }

    pub fn fit(&mut self, size: Vec2) {
        if self.view_size.x <= 0.0 || self.view_size.y <= 0.0 {
            self.follows_fit = true;
            return;
        }
        let m = 2.0 * crate::theme::metric::FIT_MARGIN;
        let zoom = ((self.view_size.x - m).max(1.0) / size.x).min((self.view_size.y - m).max(1.0) / size.y) * self.pixels_per_point;
        self.zoom = zoom.clamp(ZOOM_RANGE.0, ZOOM_RANGE.1);
        self.pan = Vec2::ZERO;
        self.follows_fit = true;
    }

    pub fn resize(&mut self, view_size: Vec2, pixels_per_point: f32, document: Vec2) {
        if view_size == self.view_size && pixels_per_point == self.pixels_per_point {
            return;
        }
        let old = self.points_per_pixel();
        self.view_size = view_size;
        self.pixels_per_point = pixels_per_point.max(1.0);
        if self.follows_fit {
            self.fit(document);
        } else {
            self.pan *= self.points_per_pixel() / old;
        }
    }

    /// Zooms keeping the document point under `anchor` (view coordinates) where it is.
    pub fn set_zoom(&mut self, zoom: f32, anchor: Vec2, document: Vec2) {
        if !zoom.is_finite() {
            return;
        }
        let origin = self.document_rect(document).min.to_vec2();
        let pixel = (anchor - origin) / self.points_per_pixel();
        self.zoom = zoom.clamp(ZOOM_RANGE.0, ZOOM_RANGE.1);
        let moved = self.document_rect(document).min.to_vec2() + pixel * self.points_per_pixel();
        self.pan += anchor - moved;
        self.follows_fit = false;
    }

    pub fn keyboard_zoom_target(&self, step: i32) -> f32 {
        let tolerance = (self.zoom.abs() * 1e-6).max(1e-9);
        if step > 0 {
            KEYBOARD_ZOOMS.iter().copied().find(|z| *z > self.zoom + tolerance).unwrap_or(self.zoom)
        } else {
            KEYBOARD_ZOOMS.iter().copied().rev().find(|z| *z < self.zoom - tolerance).unwrap_or(self.zoom)
        }
    }
}

pub struct Doc {
    pub name: String,
    pub path: Option<PathBuf>,
    pub project: Project,
    pub modified: bool,
    pub view: Viewport,
    pub active: Option<String>,
    pub collapsed: HashSet<String>,
    undo: Vec<(String, Manifest)>,
    redo: Vec<(String, Manifest)>,
    /// An edit that keeps extending the top undo step (a slider drag) until `end_coalescing`.
    coalescing: Option<String>,
    pub canvas: Option<CanvasTexture>,
    pub render_error: Option<String>,
    needs_render: bool,
    pub thumbs_version: u64,
    pub thumbs: HashMap<String, (u64, eframe::egui::TextureHandle)>,
}

impl Doc {
    pub fn new(name: String, path: Option<PathBuf>, project: Project) -> Self {
        let active = project.manifest.active_layer_id.clone().or_else(|| project.manifest.layers.last().map(|l| l.id.clone()));
        Self {
            name,
            path,
            project,
            modified: false,
            view: Viewport::default(),
            active,
            collapsed: HashSet::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            coalescing: None,
            canvas: None,
            render_error: None,
            needs_render: true,
            thumbs_version: 1,
            thumbs: HashMap::new(),
        }
    }

    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let project = comp_format::load(path)?;
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Untitled".into());
        Ok(Self::new(name, Some(path.to_path_buf()), project))
    }

    /// `EditorSession.createDocument(width:height:emptyLayer: true)`: one blank "Layer 1".
    pub fn blank(name: String, width: i64, height: i64) -> Self {
        let layer = LayerRecord {
            id: new_id(),
            name: "Layer 1".into(),
            is_visible: true,
            transform: Transform::at(0.0, 0.0, width as f64, height as f64),
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
        };
        let manifest = Manifest {
            format: comp_format::FORMAT.into(),
            version: comp_format::CURRENT_VERSION,
            color_space: "sRGB".into(),
            resolution: Some(72.0),
            document_id: new_id(),
            width,
            height,
            active_layer_id: Some(layer.id.clone()),
            layers: vec![layer],
            guides: None,
        };
        Self::new(name, None, Project { manifest, images: HashMap::new(), masks: HashMap::new() })
    }

    pub fn size(&self) -> Vec2 {
        vec2(self.project.manifest.width as f32, self.project.manifest.height as f32)
    }

    pub fn layer(&self, id: &str) -> Option<&LayerRecord> {
        self.project.manifest.layers.iter().find(|l| l.id == id)
    }

    pub fn active_layer(&self) -> Option<&LayerRecord> {
        self.active.as_deref().and_then(|id| self.layer(id))
    }

    /// The composite through the engine, then onto the canvas texture. Runs when an edit asked for it.
    pub fn refresh(&mut self, gfx: &Gfx) {
        if !self.needs_render {
            return;
        }
        self.needs_render = false;
        let (w, h) = (self.project.manifest.width as u32, self.project.manifest.height as u32);
        match engine::composite::Compositor::new(&gfx.gpu, &self.project).render() {
            Ok(image) => {
                if self.canvas.as_ref().is_some_and(|c| c.width != w || c.height != h) {
                    self.canvas.take().unwrap().free(gfx);
                }
                let canvas = self.canvas.get_or_insert_with(|| CanvasTexture::new(gfx, w, h));
                canvas.upload(gfx, &image);
                self.render_error = None;
            }
            Err(e) => {
                if let Some(c) = self.canvas.take() {
                    c.free(gfx);
                }
                self.render_error = Some(match e {
                    RenderError::Unsupported(what) => format!("Can’t draw {what} yet"),
                    RenderError::Failed(e) => format!("Couldn’t draw the canvas: {e:#}"),
                });
            }
        }
    }

    pub fn release(&mut self, gfx: &Gfx) {
        if let Some(c) = self.canvas.take() {
            c.free(gfx);
        }
    }

    /// The flattened image the Mac's Export PNG writes: straight alpha.
    pub fn export_png(&self, gfx: &Gfx, path: &Path) -> anyhow::Result<()> {
        let canvas = engine::composite::Compositor::new(&gfx.gpu, &self.project).render().map_err(|e| anyhow::anyhow!("{e}"))?;
        let straight = engine::blend::unpremultiply(&gfx.gpu, &canvas);
        let bytes = gfx.gpu.download(&straight)?;
        let (w, h) = (canvas.width, canvas.height);
        let png = comp_format::encode_png(&bytes, w, h, image::ExtendedColorType::Rgba8)?;
        std::fs::write(path, png)?;
        Ok(())
    }

    /// Writes the project as the Mac's `projectSnapshot` would: every appearance field explicit.
    pub fn save(&mut self, path: &Path) -> anyhow::Result<()> {
        self.project.manifest.active_layer_id = self.active.clone();
        let normalized = engine::session::normalize(&self.project);
        comp_format::save(&normalized, path)?;
        self.path = Some(path.to_path_buf());
        self.name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| self.name.clone());
        self.modified = false;
        Ok(())
    }

    // Edits. Each takes an undo step of the manifest (pixels don't change) and re-renders.

    pub fn edit(&mut self, action: &str, coalesce: bool, change: impl FnOnce(&mut Manifest)) {
        let before = self.project.manifest.clone();
        change(&mut self.project.manifest);
        if self.project.manifest == before {
            return;
        }
        if !(coalesce && self.coalescing.as_deref() == Some(action)) {
            self.undo.push((action.to_string(), before));
            self.redo.clear();
        }
        self.coalescing = coalesce.then(|| action.to_string());
        self.changed();
    }

    pub fn end_coalescing(&mut self) {
        self.coalescing = None;
    }

    fn changed(&mut self) {
        self.modified = true;
        self.needs_render = true;
        self.thumbs_version += 1;
    }

    pub fn undo_title(&self) -> Option<&str> {
        self.undo.last().map(|(a, _)| a.as_str())
    }

    pub fn redo_title(&self) -> Option<&str> {
        self.redo.last().map(|(a, _)| a.as_str())
    }

    pub fn undo(&mut self) {
        if let Some((action, manifest)) = self.undo.pop() {
            let now = std::mem::replace(&mut self.project.manifest, manifest);
            self.redo.push((action, now));
            self.coalescing = None;
            self.changed();
        }
    }

    pub fn redo(&mut self) {
        if let Some((action, manifest)) = self.redo.pop() {
            let now = std::mem::replace(&mut self.project.manifest, manifest);
            self.undo.push((action, now));
            self.coalescing = None;
            self.changed();
        }
    }

    pub fn set_visible(&mut self, id: &str, visible: bool) {
        let action = if visible { "Show Layer" } else { "Hide Layer" };
        self.edit(action, false, |m| {
            if let Some(l) = m.layers.iter_mut().find(|l| l.id == id) {
                l.is_visible = visible;
            }
        });
    }

    pub fn set_opacity(&mut self, id: &str, opacity: f64, coalesce: bool) {
        self.edit("Layer Opacity", coalesce, |m| {
            if let Some(l) = m.layers.iter_mut().find(|l| l.id == id) {
                l.opacity = Some(opacity.clamp(0.0, 1.0));
            }
        });
    }

    pub fn set_blend_mode(&mut self, id: &str, mode: BlendMode) {
        self.edit("Blend Mode", false, |m| {
            if let Some(l) = m.layers.iter_mut().find(|l| l.id == id) {
                l.blend_mode = Some(mode);
            }
        });
    }

    /// Swaps the layer with its next sibling above (`up`) or below, within its folder.
    pub fn move_layer(&mut self, id: &str, up: bool) -> bool {
        let layers = &self.project.manifest.layers;
        let Some(index) = layers.iter().position(|l| l.id == id) else { return false };
        let parent = layers[index].parent_id.clone();
        let sibling = if up {
            (index + 1..layers.len()).find(|&i| layers[i].parent_id == parent)
        } else {
            (0..index).rev().find(|&i| layers[i].parent_id == parent)
        };
        let Some(sibling) = sibling else { return false };
        self.edit(if up { "Move Layer Up" } else { "Move Layer Down" }, false, |m| m.layers.swap(index, sibling));
        true
    }

    /// Moves `id` to just above (`above`) or below `target`, which must share its folder.
    pub fn reorder(&mut self, id: &str, target: &str, above: bool) {
        if id == target {
            return;
        }
        let layers = &self.project.manifest.layers;
        let (Some(from), Some(_)) = (layers.iter().position(|l| l.id == id), layers.iter().position(|l| l.id == target)) else { return };
        if layers[from].parent_id != layers.iter().find(|l| l.id == target).unwrap().parent_id {
            return;
        }
        self.edit("Move Layer", false, |m| {
            let layer = m.layers.remove(from);
            let to = m.layers.iter().position(|l| l.id == target).unwrap();
            m.layers.insert(if above { to + 1 } else { to }, layer);
        });
    }

    pub fn can_move(&self, id: &str, up: bool) -> bool {
        let layers = &self.project.manifest.layers;
        let Some(index) = layers.iter().position(|l| l.id == id) else { return false };
        let parent = &layers[index].parent_id;
        if up { layers[index + 1..].iter().any(|l| &l.parent_id == parent) } else { layers[..index].iter().any(|l| &l.parent_id == parent) }
    }
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string().to_uppercase()
}

/// One row of the Layers list, top of the stack first.
pub struct Row<'a> {
    pub layer: &'a LayerRecord,
    pub depth: usize,
    /// A layer clipped to the one below it (not an adjustment clipped through a stack).
    pub clipped: bool,
    /// Hidden because a folder above it is hidden.
    pub hidden_by_parent: bool,
}

/// `NativeLayerList`'s rows: the tree walked from the top, folders followed by their children
/// unless collapsed.
pub fn rows<'a>(layers: &'a [LayerRecord], collapsed: &HashSet<String>) -> Vec<Row<'a>> {
    let mut out = Vec::new();
    fn visit<'a>(layers: &'a [LayerRecord], parent: Option<&str>, depth: usize, hidden: bool, collapsed: &HashSet<String>, out: &mut Vec<Row<'a>>) {
        if depth > 64 {
            return;
        }
        for layer in layers.iter().rev().filter(|l| l.parent_id.as_deref() == parent) {
            out.push(Row { layer, depth, clipped: layer.mask_source_id.is_some(), hidden_by_parent: hidden });
            if layer.is_group() && !collapsed.contains(&layer.id) {
                visit(layers, Some(&layer.id), depth + 1, hidden || !layer.is_visible, collapsed, out);
            }
        }
    }
    visit(layers, None, 0, false, collapsed, &mut out);
    out
}

/// The effect sub-rows under a layer, in the menu's order.
pub fn effect_rows(layer: &LayerRecord) -> Vec<(&'static str, bool)> {
    let Some(e) = &layer.effects else { return Vec::new() };
    let mut out = Vec::new();
    if let Some(x) = &e.stroke {
        out.push(("Stroke", x.enabled.unwrap_or(true)));
    }
    if let Some(x) = &e.shadow {
        out.push(("Drop Shadow", x.enabled.unwrap_or(true)));
    }
    if let Some(x) = &e.color_overlay {
        out.push(("Color Overlay", x.enabled.unwrap_or(true)));
    }
    if let Some(x) = &e.inner_shadow {
        out.push(("Inner Shadow", x.enabled.unwrap_or(true)));
    }
    if let Some(x) = &e.outer_glow {
        out.push(("Outer Glow", x.enabled.unwrap_or(true)));
    }
    if let Some(x) = &e.inner_glow {
        out.push(("Inner Glow", x.enabled.unwrap_or(true)));
    }
    out
}
