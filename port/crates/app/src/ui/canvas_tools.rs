//! What the tools do on the canvas (`EditorCanvas`'s mouse handlers): the Move tool's drags and
//! handles, Crop, the painting tools, the Eyedropper and the Zoom drag, with their overlays.
//! Every edit ends in the engine's own operation, so what the canvas shows is what parity checks.

use crate::app::App;
use crate::document::{Doc, Viewport};
use crate::geometry::{self as geo, CropDrag, CropMode, Mode, Point, TransformDrag};
use crate::theme::{black_alpha, color, white_alpha};
use crate::tools::{BrushMode, MaskPaint, SampleLayers, SmearMode, Tool};
use comp_format::{LayerRecord, Project, Transform};
use eframe::egui::{self, Color32, Pos2, Rect, Response, Stroke, Ui, pos2, vec2};
use serde_json::json;

/// The drag in progress on the canvas.
pub enum Gesture {
    /// Moving, resizing or rotating `layer`; `draft` is where it is now.
    Transform { layer: String, drag: TransformDrag, draft: Transform, duplicate: bool },
    Crop(CropDrag),
    /// A brush-family stroke: document points so far, and what the stroke paints.
    Paint(PaintStroke),
    /// Right-drag with a brush: size (or hardness with Shift) from where it started.
    TipDrag { start: Pos2, size: f64, hardness: f64 },
    /// Zoom tool: right zooms in, left out, doubling every 100 points.
    Zoom { start: Pos2, zoom: f32, moved: bool },
    Eyedropper,
    /// A Marquee drag from `anchor`; Shift pressed again mid-drag (`armed`) squares it.
    Marquee { anchor: Point, to: Point, mode: &'static str, square: bool, armed: bool },
    Lasso { points: Vec<Point>, mode: &'static str },
    /// Dragging the selection's outline (a drag inside it in New mode).
    MoveSelection { start: Point, origin: engine::select::Selection, offset: [f64; 2] },
    /// A press while a sheet samples from the canvas (Color Range, Levels, the color picker).
    Sheet,
}

pub struct PaintStroke {
    pub op: serde_json::Value,
    pub points: Vec<Point>,
    /// Shift keeps the stroke on one axis from where it was pressed.
    axis_anchor: Option<Point>,
    axis_horizontal: Option<bool>,
    /// How many points the preview last showed.
    previewed: usize,
}

/// The document pixel under a view position.
pub fn to_doc(view: &Viewport, size: egui::Vec2, canvas: Rect, pos: Pos2) -> Point {
    let r = view.document_rect(size).translate(canvas.min.to_vec2());
    let s = view.points_per_pixel() as f64;
    [(pos.x - r.min.x) as f64 / s, (pos.y - r.min.y) as f64 / s]
}

pub fn to_view(view: &Viewport, size: egui::Vec2, canvas: Rect, p: Point) -> Pos2 {
    let r = view.document_rect(size).translate(canvas.min.to_vec2());
    let s = view.points_per_pixel() as f64;
    pos2(r.min.x + (p[0] * s) as f32, r.min.y + (p[1] * s) as f32)
}

/// Visible, and no folder above it hidden (`effectiveVisibleIDs`).
pub fn effectively_visible(project: &Project, layer: &LayerRecord) -> bool {
    let mut current = Some(layer);
    let mut guard = 0;
    while let Some(l) = current {
        if !l.is_visible {
            return false;
        }
        guard += 1;
        if guard > 64 {
            break;
        }
        current = l.parent_id.as_deref().and_then(|p| project.manifest.layers.iter().find(|x| x.id == p));
    }
    true
}

/// A layer with pixels that the Move tool can place: not a folder or adjustment, and shown.
fn transformable(project: &Project, layer: &LayerRecord) -> bool {
    project.images.contains_key(&layer.id) && !layer.is_group() && layer.adjustment.is_none() && effectively_visible(project, layer)
}

/// The Move tool's handles, in view coordinates, for the active layer (or the drag's draft).
pub struct HandleGeometry {
    pub handles: [Pos2; 8],
    pub rotation: Pos2,
}

impl HandleGeometry {
    pub fn new(t: &Transform, view: &Viewport, size: egui::Vec2, canvas: Rect) -> Self {
        let handles = geo::HANDLES.map(|h| to_view(view, size, canvas, geo::point(t, h)));
        let r = geo::radians(t) as f32;
        let rotation = pos2(handles[1].x + r.sin() * 28.0, handles[1].y - r.cos() * 28.0);
        Self { handles, rotation }
    }

    /// `TransformOverlayGeometry.hit`.
    pub fn hit(&self, p: Pos2) -> Option<Mode> {
        let near = |o: Pos2| (p - o).length() <= 10.0;
        if near(self.rotation) {
            return Some(Mode::Rotate);
        }
        if let Some(i) = self.handles.iter().position(|h| near(*h)) {
            return Some(Mode::Resize(i));
        }
        for (start, end, handle) in [(0, 2, 1), (2, 4, 3), (4, 6, 5), (6, 0, 7)] {
            let (a, b) = (self.handles[start], self.handles[end]);
            let d = b - a;
            let len2 = d.length_sq();
            if len2 <= 0.0 {
                continue;
            }
            let t = ((p - a).dot(d)) / len2;
            if (0.0..=1.0).contains(&t) && (p - a - t * d).length() <= 10.0 {
                return Some(Mode::Resize(handle));
            }
        }
        None
    }
}

/// The transform the Move tool shows for the active layer: the drag's draft while dragging.
fn shown_transform(app: &App) -> Option<(String, Transform)> {
    if let Some(Gesture::Transform { layer, draft, .. }) = &app.gesture {
        return Some((layer.clone(), *draft));
    }
    let doc = app.doc()?;
    let layer = doc.active_layer()?;
    if doc.mask_target || !transformable(&doc.project, layer) {
        return None;
    }
    Some((layer.id.clone(), layer.transform))
}

fn handle_geometry(app: &App, canvas: Rect) -> Option<HandleGeometry> {
    if app.tool != Tool::Move || !app.settings.show_controls {
        return None;
    }
    let doc = app.doc()?;
    let (_, t) = shown_transform(app)?;
    Some(HandleGeometry::new(&t, &doc.view, doc.size(), canvas))
}

/// `alignmentSnapTargets`: the canvas's edges (and center) and every other layer's box.
pub fn snap_targets(project: &Project, excluding: &[&str], centers: bool) -> (Vec<f64>, Vec<f64>) {
    let (w, h) = (project.manifest.width as f64, project.manifest.height as f64);
    let mut xs = vec![0.0, w];
    let mut ys = vec![0.0, h];
    if centers {
        xs.push(w / 2.0);
        ys.push(h / 2.0);
    }
    for layer in &project.manifest.layers {
        if excluding.contains(&layer.id.as_str()) || !project.images.contains_key(&layer.id) || layer.is_group() || !effectively_visible(project, layer) {
            continue;
        }
        let b = geo::bounds(&layer.transform);
        if centers {
            xs.extend([b[0].round(), ((b[0] + b[2]) / 2.0).round(), b[2].round()]);
            ys.extend([b[1].round(), ((b[1] + b[3]) / 2.0).round(), b[3].round()]);
        } else {
            xs.extend([b[0].round(), b[2].round()]);
            ys.extend([b[1].round(), b[3].round()]);
        }
    }
    for guide in project.manifest.guides.iter().flatten() {
        match guide.axis {
            comp_format::GuideAxis::Vertical => xs.push(guide.position),
            comp_format::GuideAxis::Horizontal => ys.push(guide.position),
        }
    }
    (xs, ys)
}

/// The layer a Move press that misses the handles drags (`transformPressLayer`), and whether it
/// was picked from under the pointer.
fn press_layer(doc: &Doc, pixel: Point, auto_select: bool) -> Option<(String, bool)> {
    let p = &doc.project;
    let under = p.manifest.layers.iter().rev().find(|l| transformable(p, l) && geo::contains(&l.transform, pixel)).map(|l| l.id.clone());
    let active = doc.active_layer().filter(|l| transformable(p, l));
    if let Some(active) = active {
        if geo::contains(&active.transform, pixel) {
            if auto_select {
                if let Some(u) = &under {
                    let top = p.manifest.layers.iter().rposition(|l| &l.id == u);
                    let current = p.manifest.layers.iter().rposition(|l| l.id == active.id);
                    if u != &active.id && top > current {
                        return Some((u.clone(), true));
                    }
                }
            }
            return Some((active.id.clone(), false));
        }
    }
    if auto_select {
        if let Some(u) = under {
            return Some((u, true));
        }
    }
    active.map(|l| (l.id.clone(), false))
}

/// `LayerMask.placement(movingLayer:to:)`: where a layer's mask sits once the layer moves.
fn mask_follows(project: &Project, layer: &LayerRecord, new: &Transform) -> Option<Transform> {
    let mask = project.masks.get(&layer.id)?;
    if mask.pixels.width() <= 1 && mask.pixels.height() <= 1 {
        return None;
    }
    let old = layer.transform;
    let moved = if layer.mask_linked() { layer.mask_placement.map(|p| geo::following(&p, &old, new)) } else { Some(layer.mask_placement.unwrap_or(old)) };
    moved.filter(|m| !geo::same_placement(m, new))
}

/// A copy of `project` with `id` placed at `t`, its mask carried as the Mac carries it.
pub fn placed(project: &Project, id: &str, t: Transform) -> Project {
    let mut p = project.clone();
    set_transform(&mut p, id, t);
    p
}

pub fn set_transform(p: &mut Project, id: &str, t: Transform) {
    let Some(index) = p.manifest.layers.iter().position(|l| l.id == id) else { return };
    let layer = &p.manifest.layers[index];
    let placement = if layer.mask_file.is_some() { mask_follows(p, layer, &t) } else { layer.mask_placement };
    let layer = &mut p.manifest.layers[index];
    layer.mask_placement = placement;
    layer.transform = t;
}

/// `commitTransform` for one layer: one undo step, "Transform Layer".
pub fn commit_transform(doc: &mut Doc, id: &str, t: Transform) {
    if !geo::is_valid(&t) {
        doc.set_preview(None);
        return;
    }
    let next = placed(&doc.project, id, t);
    doc.commit("Transform Layer", next);
}

/// Pointer handling on the canvas. `panning` is true while Space, the Hand tool or the middle
/// button has the drag.
pub fn interact(app: &mut App, ui: &Ui, response: &Response, canvas: Rect, panning: bool) {
    if app.doc().is_none() {
        return;
    }
    let (modifiers, pointer, secondary_down) = ui.input(|i| (i.modifiers, i.pointer.interact_pos().or(i.pointer.hover_pos()), i.pointer.secondary_down()));
    let pixel = |app: &App, pos: Pos2| {
        let d = app.doc().unwrap();
        to_doc(&d.view, d.size(), canvas, pos)
    };
    app.hover_pixel = pointer.filter(|p| canvas.contains(*p)).map(|p| pixel(app, p));
    // Right-drag with a brush tool sizes the tip.
    if app.tool.is_brush() && response.secondary_clicked() == false && secondary_down && app.gesture.is_none() && response.hovered() {
        if let Some(pos) = pointer {
            let tip = *app.tip();
            app.gesture = Some(Gesture::TipDrag { start: pos, size: tip.size, hardness: tip.hardness });
        }
    }
    if let Some(Gesture::TipDrag { start, size, hardness }) = app.gesture {
        if !secondary_down {
            app.gesture = None;
        } else if let Some(pos) = pointer {
            let dx = (pos.x - start.x) as f64;
            let per_pixel = app.doc().map_or(1.0, |d| d.view.points_per_pixel() as f64).max(0.0001);
            let tip = app.tip_mut();
            if modifiers.shift {
                tip.hardness = (hardness + dx / 200.0).clamp(0.0, 1.0);
                tip.size = size;
            } else {
                tip.size = (size + 2.0 * dx / per_pixel).round().clamp(1.0, 2000.0);
                tip.hardness = hardness;
            }
            app.brush_pointer = Some(start);
        }
        return;
    }
    if panning {
        return;
    }
    if let Some(polygon) = &mut app.polygon {
        polygon.cursor = app.hover_pixel;
    }
    if response.drag_started_by(egui::PointerButton::Primary) || (response.clicked_by(egui::PointerButton::Primary) && app.gesture.is_none()) {
        if let Some(pos) = response.interact_pointer_pos() {
            let p = pixel(app, pos);
            if app.sheet.is_some() {
                crate::ui::dialogs::canvas_press(app, p, modifiers);
                app.gesture = Some(Gesture::Sheet);
            } else {
                press(app, pos, p, modifiers, canvas, response.double_clicked());
            }
        }
    } else if response.double_clicked() && app.polygon.is_some() {
        if let Some(polygon) = app.polygon.take() {
            crate::ui::selection::finish_polygon(app, polygon);
        }
    }
    if response.dragged_by(egui::PointerButton::Primary) {
        if let Some(pos) = pointer {
            drag(app, pos, pixel(app, pos), modifiers);
        }
    }
    if response.drag_stopped_by(egui::PointerButton::Primary) || (response.clicked_by(egui::PointerButton::Primary) && app.gesture.is_some()) {
        let pos = pointer.or(response.interact_pointer_pos());
        release(app, pos.map(|p| (p, pixel(app, p))), modifiers);
    }
    if app.tool.is_brush() {
        app.brush_pointer = pointer.filter(|p| canvas.contains(*p));
        if app.brush_pointer.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
    }
    cursor(app, ui, pointer, canvas, modifiers);
}

fn cursor(app: &App, ui: &Ui, pointer: Option<Pos2>, canvas: Rect, modifiers: egui::Modifiers) {
    let Some(pos) = pointer.filter(|p| canvas.contains(*p)) else { return };
    let icon = match app.tool {
        Tool::Move => match &app.gesture {
            Some(Gesture::Transform { drag, .. }) => mode_cursor(drag.mode),
            _ => match handle_geometry(app, canvas).and_then(|g| g.hit(pos)) {
                Some(mode) => mode_cursor(mode),
                None if modifiers.alt => egui::CursorIcon::Copy,
                None => egui::CursorIcon::Move,
            },
        },
        Tool::Crop | Tool::Marquee | Tool::Lasso | Tool::Wand | Tool::Shape | Tool::Gradient => egui::CursorIcon::Crosshair,
        Tool::Eyedropper => egui::CursorIcon::Crosshair,
        Tool::Zoom => {
            if modifiers.alt {
                egui::CursorIcon::ZoomOut
            } else {
                egui::CursorIcon::ZoomIn
            }
        }
        Tool::Type => egui::CursorIcon::Text,
        _ => return,
    };
    ui.ctx().set_cursor_icon(icon);
}

fn mode_cursor(mode: Mode) -> egui::CursorIcon {
    match mode {
        Mode::Move => egui::CursorIcon::Move,
        Mode::Rotate => egui::CursorIcon::Alias,
        Mode::Resize(i) => match i % 4 {
            0 => egui::CursorIcon::ResizeNwSe,
            1 => egui::CursorIcon::ResizeVertical,
            2 => egui::CursorIcon::ResizeNeSw,
            _ => egui::CursorIcon::ResizeHorizontal,
        },
    }
}

fn press(app: &mut App, pos: Pos2, pixel: Point, modifiers: egui::Modifiers, canvas: Rect, double: bool) {
    // Alt with a painting tool samples a color, as the Mac's Option does (Clone Stamp sets its source).
    if app.tool == Tool::Eyedropper || (app.tool.is_brush() && modifiers.alt && app.tool != Tool::CloneStamp && app.tool != Tool::Blur) {
        app.gesture = Some(Gesture::Eyedropper);
        sample_color(app, pixel);
        return;
    }
    match app.tool {
        Tool::Move => press_move(app, pos, pixel, modifiers, canvas),
        Tool::Crop => press_crop(app, pos, pixel, canvas),
        Tool::Zoom => {
            let zoom = app.doc().map_or(1.0, |d| d.view.zoom);
            app.gesture = Some(Gesture::Zoom { start: pos, zoom, moved: false });
        }
        t if t.is_brush() => press_paint(app, pixel, modifiers),
        Tool::Marquee | Tool::Lasso | Tool::Wand => crate::ui::selection::press(app, pos, pixel, modifiers, canvas, double),
        _ => {}
    }
}

fn drag(app: &mut App, pos: Pos2, pixel: Point, modifiers: egui::Modifiers) {
    match app.gesture.take() {
        Some(Gesture::Transform { layer, drag, duplicate, .. }) => {
            let mut layer = layer;
            if duplicate {
                // Option-drag: the copy is what moves (`beginDuplicateTransform`).
                if let Some(copy) = app.doc_mut().and_then(|d| crate::layer_ops::duplicate(d, &layer)) {
                    layer = copy;
                }
            }
            let draft = drag_transform(app, &layer, &drag, pixel, modifiers);
            if let Some(d) = app.doc_mut() {
                let preview = placed(&d.project, &layer, draft);
                d.set_preview(Some(preview));
            }
            app.gesture = Some(Gesture::Transform { layer, drag, draft, duplicate: false });
        }
        Some(Gesture::Crop(drag)) => {
            let ratio = app.crop_ratio();
            let symmetric = modifiers.alt;
            let Some(doc) = app.doc_mut() else { return };
            let mut next = drag.updated(pixel, ratio, symmetric);
            if !modifiers.ctrl {
                let (xs, ys) = snap_targets(&doc.project, &[], false);
                let tolerance = 8.0 / (doc.view.points_per_pixel() as f64).max(0.0001);
                next = geo::crop_snap(next, &drag, pixel, ratio, symmetric, &xs, &ys, tolerance);
            }
            if geo::crop_valid(next) {
                doc.crop = Some(next);
            }
            app.gesture = Some(Gesture::Crop(drag));
        }
        Some(Gesture::Paint(mut stroke)) => {
            let mut p = pixel;
            if modifiers.shift {
                let anchor = *stroke.axis_anchor.get_or_insert(*stroke.points.last().unwrap_or(&pixel));
                if stroke.axis_horizontal.is_none() && (p[0] - anchor[0]).hypot(p[1] - anchor[1]) >= 3.0 {
                    stroke.axis_horizontal = Some((p[0] - anchor[0]).abs() >= (p[1] - anchor[1]).abs());
                }
                p = match stroke.axis_horizontal {
                    Some(true) => [p[0], anchor[1]],
                    Some(false) => [anchor[0], p[1]],
                    None => anchor,
                };
            } else {
                stroke.axis_anchor = None;
                stroke.axis_horizontal = None;
            }
            if stroke.points.last() != Some(&p) {
                stroke.points.push(p);
            }
            preview_stroke(app, &mut stroke);
            app.gesture = Some(Gesture::Paint(stroke));
        }
        Some(Gesture::Zoom { start, zoom, mut moved }) => {
            let dx = pos.x - start.x;
            if dx.abs() >= 3.0 {
                moved = true;
            }
            if moved {
                let anchor = start - app.canvas_rect.min;
                if let Some(d) = app.doc_mut() {
                    let size = d.size();
                    d.view.set_zoom(zoom * 2f32.powf(dx / 100.0), anchor, size);
                }
            }
            app.gesture = Some(Gesture::Zoom { start, zoom, moved });
        }
        Some(Gesture::Eyedropper) => {
            sample_color(app, pixel);
            app.gesture = Some(Gesture::Eyedropper);
        }
        Some(g @ (Gesture::Marquee { .. } | Gesture::Lasso { .. } | Gesture::MoveSelection { .. })) => {
            let g = crate::ui::selection::drag(app, g, pixel, modifiers);
            app.gesture = Some(g);
        }
        other => app.gesture = other,
    }
}

fn release(app: &mut App, at: Option<(Pos2, Point)>, modifiers: egui::Modifiers) {
    match app.gesture.take() {
        Some(Gesture::Transform { layer, draft, .. }) => {
            if let Some(d) = app.doc_mut() {
                d.snap_guides = (Vec::new(), Vec::new());
                commit_transform(d, &layer, draft);
            }
        }
        Some(Gesture::Paint(mut stroke)) => {
            if let Some((_, p)) = at {
                if !modifiers.shift && stroke.points.last() != Some(&p) {
                    stroke.points.push(p);
                }
            }
            finish_stroke(app, stroke);
        }
        Some(Gesture::Zoom { start, moved, .. }) => {
            if !moved {
                let anchor = start - app.canvas_rect.min;
                if let Some(d) = app.doc_mut() {
                    let size = d.size();
                    let z = d.view.zoom * if modifiers.alt { 0.5 } else { 2.0 };
                    d.view.set_zoom(z, anchor, size);
                }
            }
        }
        Some(g @ (Gesture::Marquee { .. } | Gesture::Lasso { .. } | Gesture::MoveSelection { .. })) => crate::ui::selection::release(app, g),
        Some(Gesture::Crop(_)) | Some(Gesture::Eyedropper) | Some(Gesture::TipDrag { .. }) | Some(Gesture::Sheet) | None => {}
    }
}

// Move tool.

fn press_move(app: &mut App, pos: Pos2, pixel: Point, modifiers: egui::Modifiers, canvas: Rect) {
    let auto_select = app.settings.auto_select != modifiers.ctrl;
    let mut mode = handle_geometry(app, canvas).and_then(|g| g.hit(pos));
    let Some(doc) = app.doc_mut() else { return };
    if mode.is_none() {
        if let Some((id, picked)) = press_layer(doc, pixel, auto_select) {
            if picked {
                doc.active = Some(id);
                doc.mask_target = false;
            }
            mode = Some(Mode::Move);
        }
    }
    let Some(mode) = mode else { return };
    let Some(layer) = doc.active_layer().filter(|l| transformable(&doc.project, l)) else { return };
    let (id, original) = (layer.id.clone(), layer.transform);
    doc.end_coalescing();
    app.gesture = Some(Gesture::Transform {
        layer: id,
        drag: TransformDrag { original, start: pixel, mode },
        draft: original,
        duplicate: mode == Mode::Move && modifiers.alt,
    });
}

/// The draft for a drag to `pixel`, snapped to the canvas, the other layers and the guides.
fn drag_transform(app: &mut App, layer: &str, drag: &TransformDrag, pixel: Point, modifiers: egui::Modifiers) -> Transform {
    let lock = app.settings.lock_aspect;
    let (shift, option) = (modifiers.shift, modifiers.alt);
    let snapping = app.snap && !modifiers.ctrl;
    let Some(doc) = app.doc_mut() else { return drag.original };
    let tolerance = geo::SNAP_DISTANCE / (doc.view.points_per_pixel() as f64).max(0.0001);
    let (xs, ys) = if snapping { snap_targets(&doc.project, &[layer], true) } else { (Vec::new(), Vec::new()) };
    let mut target = pixel;
    let mut guides = (Vec::new(), Vec::new());
    if matches!(drag.mode, Mode::Resize(_)) && snapping {
        let (p, gx, gy) = geo::snapped_resize_point(pixel, drag, lock != shift, &xs, &ys, tolerance, |q| drag.updated(q, lock, shift, option));
        target = p;
        guides = (gx, gy);
    }
    let mut draft = geo::rounded(&drag.updated(target, lock, shift, option));
    if drag.mode == Mode::Move && snapping {
        let (offset, x, y) = geo::snap_offset(geo::bounds(&draft), &xs, &ys, tolerance);
        draft.origin[0] += offset[0];
        draft.origin[1] += offset[1];
        guides = (x.into_iter().collect(), y.into_iter().collect());
    }
    doc.snap_guides = guides;
    draft
}

// Crop.

fn crop_regions(r: Rect) -> Vec<(usize, Rect)> {
    let handles: Vec<Pos2> = geo::HANDLES.iter().map(|h| pos2(r.min.x + h[0] as f32 * r.width(), r.min.y + h[1] as f32 * r.height())).collect();
    let radius = 10.0;
    let mut out: Vec<(usize, Rect)> = [0, 2, 4, 6].iter().map(|i| (*i, Rect::from_center_size(handles[*i], vec2(radius * 2.0, radius * 2.0)))).collect();
    for i in [1, 5] {
        out.push((i, Rect::from_min_size(pos2(r.min.x + radius, handles[i].y - radius), vec2((r.width() - radius * 2.0).max(0.0), radius * 2.0))));
    }
    for i in [3, 7] {
        out.push((i, Rect::from_min_size(pos2(handles[i].x - radius, r.min.y + radius), vec2(radius * 2.0, (r.height() - radius * 2.0).max(0.0)))));
    }
    out
}

fn crop_view_rect(doc: &Doc, canvas: Rect) -> Option<Rect> {
    let c = doc.visible_crop()?;
    let a = to_view(&doc.view, doc.size(), canvas, [c[0], c[1]]);
    let b = to_view(&doc.view, doc.size(), canvas, [c[0] + c[2], c[1] + c[3]]);
    Some(Rect::from_min_max(a, b))
}

fn press_crop(app: &mut App, pos: Pos2, pixel: Point, canvas: Rect) {
    let Some(doc) = app.doc_mut() else { return };
    let whole = [0.0, 0.0, doc.project.manifest.width as f64, doc.project.manifest.height as f64];
    let rect = doc.visible_crop().unwrap_or([pixel[0], pixel[1], 0.0, 0.0]);
    let region = crop_view_rect(doc, canvas).and_then(|r| crop_regions(r).into_iter().find(|(_, reg)| reg.contains(pos)).map(|(i, _)| i));
    let inside = doc.crop.is_some_and(|c| pixel[0] >= c[0] && pixel[1] >= c[1] && pixel[0] < c[0] + c[2] && pixel[1] < c[1] + c[3]);
    let mode = match region {
        Some(i) => CropMode::Resize(i),
        None if inside && rect != whole => CropMode::Move,
        None => {
            doc.crop = None;
            CropMode::Create
        }
    };
    app.gesture = Some(Gesture::Crop(CropDrag { start: pixel, original: rect, mode }));
}

/// Apply Crop: the engine's `crop` op, as Apply Crop runs `commitCrop`.
pub fn apply_crop(app: &mut App) {
    let engine = app.gfx.engine.clone();
    let Some(doc) = app.doc_mut() else { return };
    let Some(rect) = doc.crop.filter(|r| geo::crop_valid(*r)) else { return };
    let op = json!({ "op": "crop", "rect": rect });
    match doc.apply("Crop", |p| engine.apply_op(p, &op)) {
        Ok(()) => {
            doc.crop = None;
            let size = doc.size();
            doc.view.fit(size);
        }
        Err(e) => {
            let message = e.to_string();
            app.alert("Couldn’t crop", message);
        }
    }
}

// Painting.

/// The `stroke` op the current tool and its header would paint (see parity/README.md).
fn stroke_op(app: &App) -> Option<serde_json::Value> {
    let doc = app.doc()?;
    let layer = doc.active_layer()?;
    let s = &app.settings;
    let mask = doc.mask_target;
    let (tool, tip) = match app.tool {
        Tool::Brush if s.brush_mode == BrushMode::Erase => ("eraser", s.brush),
        Tool::Brush => ("brush", s.brush),
        Tool::SpotHealing => ("heal", s.brush),
        Tool::CloneStamp => ("clone", s.clone),
        Tool::Blur => (
            match s.smear_mode {
                SmearMode::Blur => "blur",
                SmearMode::Smudge => "smudge",
                SmearMode::Liquify => "liquify",
            },
            s.smear,
        ),
        _ => return None,
    };
    // The Mac's smoothing string is in screen points: `smoothing / zoom` document pixels.
    let zoom = (doc.view.zoom as f64).max(0.01);
    let mut settings = json!({
        "size": tip.size,
        "hardness": tip.hardness,
        "opacity": tip.opacity,
        "smoothing": s.smoothing / zoom,
        "blurRadius": s.smear_radius,
        "color": s.foreground.map(|c| c as f64),
        "white": s.mask_paint == MaskPaint::Reveal,
        "aligned": s.clone_aligned,
        "sampleAll": s.clone_sample == SampleLayers::All,
    });
    if app.tool == Tool::SpotHealing {
        settings["healingMode"] = json!(s.healing.title());
    }
    Some(json!({ "op": "stroke", "tool": tool, "layer": layer.id, "target": if mask { "mask" } else { "pixels" }, "settings": settings }))
}

fn press_paint(app: &mut App, pixel: Point, modifiers: egui::Modifiers) {
    if app.tool == Tool::CloneStamp && modifiers.alt {
        if let Some(d) = app.doc_mut() {
            d.clone_source = Some(pixel);
            d.clone_source_pending = true;
        }
        return;
    }
    let Some(mut op) = stroke_op(app) else { return };
    let Some(doc) = app.doc_mut() else { return };
    if doc.clone_source_pending {
        if let Some(source) = doc.clone_source {
            op["source"] = json!(source);
        }
    }
    op["shift"] = json!(modifiers.shift);
    let stroke = PaintStroke { op, points: vec![pixel], axis_anchor: modifiers.shift.then_some(pixel), axis_horizontal: None, previewed: 0 };
    let mut stroke = stroke;
    preview_stroke(app, &mut stroke);
    app.gesture = Some(Gesture::Paint(stroke));
}

/// Runs the stroke so far on copies of the project and the painting session, and shows that.
fn preview_stroke(app: &mut App, stroke: &mut PaintStroke) {
    if stroke.previewed == stroke.points.len() {
        return;
    }
    stroke.previewed = stroke.points.len();
    let engine = app.gfx.engine.clone();
    let Some(doc) = app.doc_mut() else { return };
    let mut op = stroke.op.clone();
    op["points"] = json!(stroke.points);
    let mut project = doc.project.clone();
    let mut session = doc.paint.clone();
    match engine::paint::apply(&engine.gpu, &mut project, &mut session, &op) {
        Ok(()) => doc.set_preview(Some(project)),
        Err(_) => doc.set_preview(None),
    }
}

fn finish_stroke(app: &mut App, stroke: PaintStroke) {
    let engine = app.gfx.engine.clone();
    let title = match stroke.op["tool"].as_str() {
        Some("eraser") => "Erase",
        Some("heal") => "Spot Healing",
        Some("clone") => "Clone Stamp",
        Some("blur") => "Blur",
        Some("smudge") => "Smudge",
        Some("liquify") => "Liquify",
        _ => "Brush Stroke",
    };
    let Some(doc) = app.doc_mut() else { return };
    let mut op = stroke.op;
    op["points"] = json!(stroke.points);
    let mut session = doc.paint.clone();
    let result = doc.apply(title, |p| engine::paint::apply(&engine.gpu, p, &mut session, &op));
    match result {
        Ok(()) => {
            doc.paint = session;
            doc.clone_source_pending = false;
        }
        Err(e) => {
            doc.set_preview(None);
            let message = match e {
                engine::RenderError::Unsupported(what) => format!("The port can’t paint this exactly yet: {what}."),
                engine::RenderError::Failed(e) => format!("{e:#}"),
            };
            app.alert("Couldn’t paint", message);
        }
    }
}

// Eyedropper.

/// `sampleCompositeColor`: the canvas color under `pixel` into the foreground color.
fn sample_color(app: &mut App, pixel: Point) {
    let gfx = app.gfx.clone();
    let Some(doc) = app.doc_mut() else { return };
    if let Some(c) = doc.sample(&gfx, pixel) {
        app.settings.foreground = c;
    }
}

// Overlays.

/// Draws the tools' overlays over the canvas: the selection, transform handles, the crop frame,
/// snap lines, the brush tip.
pub fn overlay(app: &mut App, ui: &Ui, canvas: Rect) {
    crate::ui::selection::overlay(app, ui, canvas);
    let app: &App = app;
    let Some(doc) = app.doc() else { return };
    let p = ui.painter().with_clip_rect(canvas.intersect(ui.clip_rect()));
    let size = doc.size();
    // Snap lines.
    let (gx, gy) = &doc.snap_guides;
    for x in gx {
        let a = to_view(&doc.view, size, canvas, [*x, 0.0]);
        let b = to_view(&doc.view, size, canvas, [*x, size.y as f64]);
        p.line_segment([a, b], Stroke::new(1.0, color::ACCENT));
    }
    for y in gy {
        let a = to_view(&doc.view, size, canvas, [0.0, *y]);
        let b = to_view(&doc.view, size, canvas, [size.x as f64, *y]);
        p.line_segment([a, b], Stroke::new(1.0, color::ACCENT));
    }
    if app.tool == Tool::Crop {
        if let Some(r) = crop_view_rect(doc, canvas) {
            let dim = black_alpha(0.6);
            let outer = canvas;
            for piece in [
                Rect::from_min_max(outer.min, pos2(outer.max.x, r.min.y)),
                Rect::from_min_max(pos2(outer.min.x, r.max.y), outer.max),
                Rect::from_min_max(pos2(outer.min.x, r.min.y), pos2(r.min.x, r.max.y)),
                Rect::from_min_max(pos2(r.max.x, r.min.y), pos2(outer.max.x, r.max.y)),
            ] {
                if piece.is_positive() {
                    p.rect_filled(piece, 0.0, dim);
                }
            }
            p.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Middle);
            for i in 1..=2 {
                let f = i as f32 / 3.0;
                p.line_segment([pos2(r.min.x + r.width() * f, r.min.y), pos2(r.min.x + r.width() * f, r.max.y)], Stroke::new(1.0, white_alpha(0.4)));
                p.line_segment([pos2(r.min.x, r.min.y + r.height() * f), pos2(r.max.x, r.min.y + r.height() * f)], Stroke::new(1.0, white_alpha(0.4)));
            }
            for h in geo::HANDLES {
                let c = pos2(r.min.x + h[0] as f32 * r.width(), r.min.y + h[1] as f32 * r.height());
                let hr = Rect::from_center_size(c, vec2(8.0, 8.0));
                p.rect_filled(hr, 0.0, Color32::WHITE);
                p.rect_stroke(hr, 0.0, Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Middle);
            }
        }
    } else if let Some(g) = handle_geometry(app, canvas) {
        let accent = Stroke::new(1.0, color::ACCENT);
        let corners = [g.handles[0], g.handles[2], g.handles[4], g.handles[6], g.handles[0]];
        p.line(corners.to_vec(), accent);
        p.line_segment([g.handles[1], g.rotation], accent);
        for h in g.handles {
            let r = Rect::from_center_size(h, vec2(7.0, 7.0));
            p.rect_filled(r, 0.0, Color32::WHITE);
            p.rect_stroke(r, 0.0, accent, egui::StrokeKind::Middle);
        }
        p.circle_filled(g.rotation, 4.0, Color32::WHITE);
        p.circle_stroke(g.rotation, 4.0, accent);
    }
    // Clone Stamp's source.
    if app.tool == Tool::CloneStamp {
        if let Some(source) = doc.clone_source {
            let c = to_view(&doc.view, size, canvas, source);
            p.line_segment([c - vec2(6.0, 0.0), c + vec2(6.0, 0.0)], Stroke::new(1.0, Color32::WHITE));
            p.line_segment([c - vec2(0.0, 6.0), c + vec2(0.0, 6.0)], Stroke::new(1.0, Color32::WHITE));
        }
    }
    // The brush tip: a circle as wide as the brush on screen.
    if app.tool.is_brush() {
        if let Some(at) = app.brush_pointer {
            let tip = app.tip();
            let radius = (tip.size as f32 * doc.view.points_per_pixel() / 2.0).max(1.0);
            p.circle_stroke(at, radius, Stroke::new(2.0, black_alpha(0.5)));
            p.circle_stroke(at, radius, Stroke::new(1.0, white_alpha(0.9)));
            if tip.hardness < 1.0 {
                p.circle_stroke(at, radius * tip.hardness.max(0.05) as f32, Stroke::new(1.0, white_alpha(0.35)));
            }
        }
    }
}

/// Canvas keys for the current tool: Return and Escape for Crop, [ and ] for the brush tip,
/// Delete to clear the crop.
pub fn keys(app: &mut App, ctx: &egui::Context) {
    use egui::{Key, Modifiers};
    let pressed = |key: Key, m: Modifiers| ctx.input_mut(|i| i.consume_key(m, key));
    if app.tool == Tool::Crop {
        if pressed(Key::Enter, Modifiers::NONE) {
            apply_crop(app);
        }
        if pressed(Key::Escape, Modifiers::NONE) {
            if let Some(d) = app.doc_mut() {
                d.crop = None;
            }
        }
    }
    if pressed(Key::Escape, Modifiers::NONE) {
        if let Some(Gesture::Transform { .. } | Gesture::Paint(_) | Gesture::Crop(_)) = app.gesture {
            app.gesture = None;
            if let Some(d) = app.doc_mut() {
                d.set_preview(None);
                d.snap_guides = (Vec::new(), Vec::new());
            }
        }
    }
    if app.tool.is_brush() {
        // `[` and `]` step the size as Photoshop does: by 1 under 10, 10 under 100, 25 under 200, then 50, 100.
        let step = |size: f64, up: bool| {
            let s = if up { size } else { size - 0.5 };
            let d = if s < 10.0 { 1.0 } else if s < 100.0 { 10.0 } else if s < 200.0 { 25.0 } else if s < 500.0 { 50.0 } else { 100.0 };
            if up { ((size / d).floor() + 1.0) * d } else { ((size / d).ceil() - 1.0) * d }
        };
        if pressed(Key::CloseBracket, Modifiers::NONE) {
            let t = app.tip_mut();
            t.size = step(t.size, true).clamp(1.0, 2000.0);
        }
        if pressed(Key::OpenBracket, Modifiers::NONE) {
            let t = app.tip_mut();
            t.size = step(t.size, false).clamp(1.0, 2000.0);
        }
        if pressed(Key::CloseBracket, Modifiers::SHIFT) {
            let t = app.tip_mut();
            t.hardness = ((t.hardness * 4.0).round() / 4.0 + 0.25).clamp(0.0, 1.0);
        }
        if pressed(Key::OpenBracket, Modifiers::SHIFT) {
            let t = app.tip_mut();
            t.hardness = ((t.hardness * 4.0).round() / 4.0 - 0.25).clamp(0.0, 1.0);
        }
    }
}
