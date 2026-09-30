//! The selection tools and the Select menu (`EditorCanvas.lassoMouseDown`, `Document/Selection.swift`,
//! `MagicWand.swift`, `ColorRangeSelection.swift`), through the engine's selection ops, and the
//! marching ants.

use crate::app::App;
use crate::document::Doc;
use crate::geometry::{self as geo, Point};
use crate::menus::Command;
use crate::tools::{LassoKind, MarqueeKind, SelectionMode, Tool, WandMode};
use crate::ui::canvas_tools::{Gesture, to_view};
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Ui};
use engine::select::Selection;
use serde_json::{Value, json};

/// A Polygonal Lasso being clicked out: its corners and the pointer.
pub struct Polygon {
    pub points: Vec<Point>,
    pub cursor: Option<Point>,
    pub mode: &'static str,
}

/// `selectionMode(shift:option:)`: Shift adds, Option subtracts, otherwise the header's mode.
fn mode(app: &App, modifiers: egui::Modifiers) -> &'static str {
    if modifiers.shift {
        "Add"
    } else if modifiers.alt {
        "Subtract"
    } else {
        match app.settings.selection_mode {
            SelectionMode::New => "New",
            SelectionMode::Add => "Add",
            SelectionMode::Subtract => "Subtract",
        }
    }
}

/// Runs selection `op` on the document's selection and records the result as `title`.
pub fn run(app: &mut App, title: &str, op: Value) {
    let engine = app.gfx.engine.clone();
    let Some(doc) = app.doc_mut() else { return };
    let mut project = doc.project.clone();
    project.manifest.active_layer_id = doc.active.clone();
    let mut selection = doc.selection.clone();
    match engine.apply_op_with_selection(&mut project, &mut selection, &op) {
        Ok(()) => doc.set_selection(title, selection),
        Err(e) => {
            let message = match e {
                engine::RenderError::Unsupported(what) => format!("The port can’t make this selection exactly yet: {what}."),
                engine::RenderError::Failed(e) => format!("{e:#}"),
            };
            app.alert("Couldn’t select", message);
        }
    }
}

/// Whether document point `p` is inside the selection (nonzero winding, as it fills).
pub fn contains(doc: &mut Doc, p: Point) -> bool {
    let Some(ants) = doc.ants() else { return false };
    let mut winding = 0i32;
    for poly in ants {
        let n = poly.len();
        for i in 0..n {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            if a[1] <= p[1] {
                if b[1] > p[1] && (b[0] - a[0]) * (p[1] - a[1]) - (p[0] - a[0]) * (b[1] - a[1]) > 0.0 {
                    winding += 1;
                }
            } else if b[1] <= p[1] && (b[0] - a[0]) * (p[1] - a[1]) - (p[0] - a[0]) * (b[1] - a[1]) < 0.0 {
                winding -= 1;
            }
        }
    }
    winding != 0
}

/// A press with the Marquee, Lasso or Magic tool.
pub fn press(app: &mut App, pos: Pos2, pixel: Point, modifiers: egui::Modifiers, canvas: Rect, double: bool) {
    // A Polygonal Lasso in progress takes the click as its next corner, or closes.
    if let Some(mut polygon) = app.polygon.take() {
        let doc = app.doc().unwrap();
        let first = to_view(&doc.view, doc.size(), canvas, polygon.points[0]);
        if double || (polygon.points.len() >= 3 && (pos - first).length() <= 8.0) {
            finish_polygon(app, polygon);
        } else {
            polygon.points.push(pixel);
            app.polygon = Some(polygon);
        }
        return;
    }
    let m = mode(app, modifiers);
    let inside = m == "New" && app.doc_mut().is_some_and(|d| contains(d, pixel));
    if inside {
        let origin = app.doc().and_then(|d| d.selection.clone()).unwrap();
        app.gesture = Some(Gesture::MoveSelection { start: pixel, origin, offset: [0.0, 0.0] });
        return;
    }
    match app.tool {
        Tool::Wand if app.settings.wand == WandMode::Object => {
            run(app, "Object Selection", json!({ "op": "objectSelection", "point": pixel, "mode": m }));
        }
        Tool::Wand => wand(app, pixel, m),
        Tool::Marquee => {
            let p = snapped(app, pixel, modifiers);
            let anchor = [p[0].round(), p[1].round()];
            // Shift held at the press means Add; it squares only once pressed again mid-drag.
            app.gesture = Some(Gesture::Marquee { anchor, to: anchor, mode: m, square: false, armed: !modifiers.shift });
        }
        Tool::Lasso if app.settings.lasso == LassoKind::Polygonal => {
            app.polygon = Some(Polygon { points: vec![pixel], cursor: None, mode: m });
        }
        Tool::Lasso => app.gesture = Some(Gesture::Lasso { points: vec![pixel], mode: m }),
        _ => {}
    }
}

/// The Marquee's corners snap to the canvas, layers and guides within 10 points.
fn snapped(app: &App, p: Point, modifiers: egui::Modifiers) -> Point {
    let Some(doc) = app.doc() else { return p };
    if !app.snap || modifiers.ctrl {
        return p;
    }
    let (xs, ys) = crate::ui::canvas_tools::snap_targets(&doc.project, &[], false);
    let tolerance = geo::SNAP_DISTANCE / (doc.view.points_per_pixel() as f64).max(0.0001);
    [geo::nearest(p[0], &xs, tolerance).unwrap_or(p[0]), geo::nearest(p[1], &ys, tolerance).unwrap_or(p[1])]
}

fn wand(app: &mut App, pixel: Point, m: &str) {
    let s = &app.settings;
    let op = json!({
        "op": "wand",
        "point": pixel,
        "mode": m,
        "tolerance": s.tolerance,
        "sampleSize": s.sample_size.title(),
        "contiguous": s.contiguous,
        "sampleAllLayers": s.wand_layers == crate::tools::SampleLayers::All,
        "antialias": s.anti_alias,
    });
    run(app, "Magic Wand", op);
}

pub fn drag(app: &mut App, gesture: Gesture, pixel: Point, modifiers: egui::Modifiers) -> Gesture {
    match gesture {
        Gesture::Marquee { anchor, mode, mut square, mut armed, .. } => {
            // Shift pressed during the drag makes a square or circle.
            if modifiers.shift && armed {
                square = true;
            }
            if !modifiers.shift {
                armed = true;
                square = false;
            }
            let to = snapped(app, pixel, modifiers);
            Gesture::Marquee { anchor, to, mode, square, armed }
        }
        Gesture::Lasso { mut points, mode } => {
            if points.last().is_none_or(|l| (pixel[0] - l[0]).hypot(pixel[1] - l[1]) >= 0.25) {
                points.push(pixel);
            }
            Gesture::Lasso { points, mode }
        }
        Gesture::MoveSelection { start, origin, .. } => {
            let mut offset = [(pixel[0] - start[0]).round(), (pixel[1] - start[1]).round()];
            if modifiers.shift {
                if offset[0].abs() >= offset[1].abs() {
                    offset[1] = 0.0
                } else {
                    offset[0] = 0.0
                }
            }
            if let Some(d) = app.doc_mut() {
                d.selection = Some(moved(&origin, offset));
                d.ants = None;
            }
            Gesture::MoveSelection { start, origin, offset }
        }
        other => other,
    }
}

fn moved(s: &Selection, offset: [f64; 2]) -> Selection {
    Selection { region: engine::select::geom::transformed(&s.region, [1.0, 0.0, 0.0, 1.0, offset[0], offset[1]]), ..s.clone() }
}

pub fn release(app: &mut App, gesture: Gesture) {
    match gesture {
        Gesture::Marquee { anchor, to, mode, square, .. } => {
            let ellipse = app.settings.marquee == MarqueeKind::Ellipse;
            if anchor == [to[0].round(), to[1].round()] {
                // A click without a drag deselects.
                if mode == "New" {
                    command(app, Command::Deselect);
                }
                return;
            }
            let op = json!({
                "op": "marquee",
                "shape": if ellipse { "Ellipse" } else { "Rectangle" },
                "from": anchor,
                "to": to,
                "square": square,
                "mode": mode,
                "antialias": app.settings.anti_alias,
            });
            run(app, if ellipse { "Elliptical Marquee" } else { "Rectangular Marquee" }, op);
        }
        Gesture::Lasso { points, mode } => {
            let op = json!({ "op": "lasso", "kind": "Freehand", "points": points, "mode": mode, "antialias": app.settings.anti_alias });
            run(app, "Lasso", op);
        }
        Gesture::MoveSelection { origin, offset, start } => {
            let Some(doc) = app.doc_mut() else { return };
            // Put the outline back so the move is one undo step from where it started.
            doc.selection = Some(origin.clone());
            doc.ants = None;
            if offset != [0.0, 0.0] {
                doc.set_selection("Move Selection", Some(moved(&origin, offset)));
            } else if app.tool == Tool::Wand {
                // The wand's click inside the selection selects afresh from that pixel.
                if app.settings.wand == WandMode::Object {
                    run(app, "Object Selection", json!({ "op": "objectSelection", "point": start, "mode": "New" }));
                } else {
                    wand(app, start, "New");
                }
            } else {
                command(app, Command::Deselect);
            }
        }
        _ => {}
    }
}

pub fn finish_polygon(app: &mut App, polygon: Polygon) {
    let op = json!({ "op": "lasso", "kind": "Polygonal", "points": polygon.points, "mode": polygon.mode, "antialias": app.settings.anti_alias });
    run(app, "Polygonal Lasso", op);
}

/// Return closes a Polygonal Lasso, Escape drops it, Delete takes back its last corner.
pub fn keys(app: &mut App, ctx: &egui::Context) {
    use egui::{Key, Modifiers};
    if app.polygon.is_none() {
        return;
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
        let polygon = app.polygon.take().unwrap();
        finish_polygon(app, polygon);
    } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        app.polygon = None;
    } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Backspace) || i.consume_key(Modifiers::NONE, Key::Delete)) {
        if let Some(p) = &mut app.polygon {
            p.points.pop();
            if p.points.is_empty() {
                app.polygon = None;
            }
        }
    }
}

/// The Select menu's commands, and Edit's that work on the selected pixels.
pub fn command(app: &mut App, command: Command) {
    let active = app.doc().and_then(|d| d.active.clone());
    match command {
        Command::SelectAll => run(app, "Select All", json!({ "op": "selectAll" })),
        Command::Deselect => {
            if let Some(d) = app.doc_mut() {
                if d.selection.is_some() {
                    d.set_selection("Deselect", None);
                }
            }
        }
        Command::InverseSelection => run(app, "Inverse", json!({ "op": "invertSelection" })),
        Command::LayerPixels => {
            if let Some(id) = active {
                run(app, "Load Layer Selection", json!({ "op": "loadSelection", "layer": id, "mode": "New" }));
            }
        }
        Command::MaskBlackAreas => {
            if let Some(id) = active {
                run(app, "Load Mask Selection", json!({ "op": "loadSelection", "layer": id, "mask": true, "mode": "New" }));
            }
        }
        Command::SelectSubject => app.alert("Couldn’t select the subject", "The port can’t find the subject exactly yet: the engine has no Select Subject (Vision's foreground mask).".into()),
        Command::ClearSelectionPixels => {
            let gfx = app.gfx.clone();
            if let Some(d) = app.doc_mut() {
                crate::layer_ops::clear_selected(d, &gfx);
            }
        }
        Command::ContentAwareFill => {
            app.alert("Couldn’t fill", "The port can’t run Content-Aware Fill yet: the engine's filters don't take a selection.".into())
        }
        _ => {}
    }
}

pub fn open_modify(app: &mut App, kind: u8) {
    let amount = match kind {
        0 => app.settings.expand,
        1 => app.settings.contract,
        _ => app.settings.feather,
    };
    app.sheet = Some(crate::ui::dialogs::Sheet::Modify { kind, amount });
}

/// Select > Expand, Contract or Feather by `amount` pixels (`SelectionAmountSheet`'s OK).
pub fn modify(app: &mut App, kind: u8, amount: f64) {
    let (key, title) = match kind {
        0 => ("expand", "Expand Selection"),
        1 => ("contract", "Contract Selection"),
        _ => ("feather", "Feather Selection"),
    };
    match kind {
        0 => app.settings.expand = amount,
        1 => app.settings.contract = amount,
        _ => app.settings.feather = amount,
    }
    run(app, title, json!({ "op": "modifySelection", key: amount }));
}

pub fn open_color_range(app: &mut App) {
    app.sheet = Some(crate::ui::dialogs::Sheet::ColorRange(ColorRange { samples: Vec::new(), fuzziness: 40.0, invert: false, preview: None, previewed: None }));
}

/// Select > Color Range…: the colors picked on the canvas, and the mask they select.
pub struct ColorRange {
    pub samples: Vec<(Point, &'static str)>,
    pub fuzziness: f64,
    pub invert: bool,
    /// The selection preview (black and white, canvas-sized), and what it was made from.
    pub preview: Option<egui::TextureHandle>,
    pub previewed: Option<(usize, i64, bool)>,
}

impl ColorRange {
    pub fn op(&self, antialias: bool) -> Value {
        let samples: Vec<Value> = self.samples.iter().map(|(p, m)| json!({ "point": p, "mode": m })).collect();
        json!({ "op": "colorRange", "samples": samples, "fuzziness": self.fuzziness.round(), "invert": self.invert, "antialias": antialias })
    }
}

/// Draws the marching ants and a selection tool's draft outline.
pub fn overlay(app: &mut App, ui: &Ui, canvas: Rect) {
    let time = ui.input(|i| i.time);
    let draft: Option<(Vec<Point>, bool, bool)> = match &app.gesture {
        Some(Gesture::Marquee { anchor, to, square, .. }) => {
            let (mut dx, mut dy) = (to[0].round() - anchor[0], to[1].round() - anchor[1]);
            if *square {
                let side = dx.abs().max(dy.abs());
                dx = side * dx.signum();
                dy = side * dy.signum();
            }
            let (x, y) = (anchor[0].min(anchor[0] + dx), anchor[1].min(anchor[1] + dy));
            let (w, h) = (dx.abs(), dy.abs());
            Some((vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h]], true, app.settings.marquee == MarqueeKind::Ellipse))
        }
        Some(Gesture::Lasso { points, .. }) => Some((points.clone(), false, false)),
        _ => app.polygon.as_ref().map(|p| {
            let mut points = p.points.clone();
            if let Some(c) = p.cursor {
                points.push(c);
            }
            (points, false, false)
        }),
    };
    let polygon_start = app.polygon.as_ref().map(|p| p.points[0]);
    let Some(doc) = app.doc_mut() else { return };
    let (view, size) = (doc.view, doc.size());
    let p = ui.painter().with_clip_rect(canvas.intersect(ui.clip_rect()));
    let map = |q: &Point| to_view(&view, size, canvas, *q);
    if let Some(ants) = doc.ants() {
        let phase = ((time / 0.12) as i64 % 8) as f32;
        for poly in ants {
            let mut pts: Vec<Pos2> = poly.iter().map(map).collect();
            if let Some(first) = pts.first().copied() {
                pts.push(first);
            }
            p.add(egui::Shape::line(pts.clone(), Stroke::new(1.0, Color32::WHITE)));
            p.extend(egui::Shape::dashed_line_with_offset(&pts, Stroke::new(1.0, Color32::BLACK), &[4.0], &[4.0], phase));
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
    }
    if let Some((points, closed, ellipse)) = draft {
        let pts: Vec<Pos2> = points.iter().map(map).collect();
        if ellipse && pts.len() == 4 {
            let r = Rect::from_two_pos(pts[0], pts[2]);
            let shape = egui::epaint::EllipseShape { center: r.center(), radius: r.size() / 2.0, fill: Color32::TRANSPARENT, stroke: Stroke::new(2.0, Color32::from_black_alpha(204)), angle: 0.0 };
            p.add(shape);
            let shape = egui::epaint::EllipseShape { center: r.center(), radius: r.size() / 2.0, fill: Color32::TRANSPARENT, stroke: Stroke::new(1.0, Color32::WHITE), angle: 0.0 };
            p.add(shape);
        } else {
            let mut line = pts.clone();
            if closed {
                if let Some(first) = line.first().copied() {
                    line.push(first);
                }
            }
            p.add(egui::Shape::line(line.clone(), Stroke::new(2.0, Color32::from_black_alpha(204))));
            p.add(egui::Shape::line(line, Stroke::new(1.0, Color32::WHITE)));
        }
        if let Some(start) = polygon_start {
            let c = map(&start);
            let r = Rect::from_center_size(c, egui::vec2(8.0, 8.0));
            p.rect_filled(r, 0.0, Color32::WHITE);
            p.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Middle);
        }
    }
}
