//! App-level tests: drive a tool or a command through the UI offscreen and check that the project
//! is what the engine's own operation makes.

use crate::app::App;
use crate::headless::{Offscreen, State, app_for, editor_size};
use crate::menus::Command;
use crate::tools::Tool;
use crate::ui::{self, canvas_tools};
use eframe::egui::{self, Pos2, pos2};
use serde_json::json;
use std::path::{Path, PathBuf};

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../parity/corpus")
}

fn open(off: &Offscreen, case: &str, tool: &str) -> App {
    let state = State { id: "test".into(), view: "window".into(), document: Some(case.into()), tool: Some(tool.into()), sheet: None, layer: None };
    let mut app = app_for(off, &state, &corpus()).unwrap();
    // One pass lays the editor out, so the canvas knows where it is.
    frame(off, &mut app, &[]);
    app
}

fn frame(off: &Offscreen, app: &mut App, events: &[(usize, egui::Event)]) {
    off.render_with_events(editor_size(), events, |ui, rect| {
        let ctx = ui.ctx().clone();
        app.handle_keys(&ctx);
        ui::editor(app, ui, rect);
        ui::dialogs::show(app, &ctx);
    })
    .unwrap();
}

/// With `APP_TEST_SNAPSHOTS` set to a folder, saves the editor as it is now there as `name.png`,
/// for looking at what a test did.
fn snapshot(off: &Offscreen, app: &mut App, name: &str) {
    let Some(dir) = std::env::var_os("APP_TEST_SNAPSHOTS") else { return };
    let image = off.render_with_events(editor_size(), &[], |ui, rect| ui::editor(app, ui, rect)).unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    image.save(Path::new(&dir).join(format!("{name}.png"))).unwrap();
}

/// Where document pixel `p` is on screen.
fn at(app: &App, p: [f64; 2]) -> Pos2 {
    let d = app.doc().unwrap();
    canvas_tools::to_view(&d.view, d.size(), app.canvas_rect, p)
}

fn button(pos: Pos2, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers }
}

/// A press at the first point, moves through the rest, and a release at the last.
fn drag(points: &[Pos2], modifiers: egui::Modifiers) -> Vec<(usize, egui::Event)> {
    let mut events = vec![(1, egui::Event::PointerMoved(points[0])), (1, button(points[0], true, modifiers))];
    for p in &points[1..] {
        events.push((2, egui::Event::PointerMoved(*p)));
    }
    events.push((3, button(*points.last().unwrap(), false, modifiers)));
    events
}

fn layer_pixels(app: &App, index: usize) -> Vec<u8> {
    let d = app.doc().unwrap();
    let id = &d.project.manifest.layers[index].id;
    d.project.images[id].pixels.as_raw().clone()
}

#[test]
fn a_move_drag_moves_the_layer_and_undo_puts_it_back() {
    let off = Offscreen::new().unwrap();
    let mut app = open(&off, "blend/stack", "move");
    // The Screen layer: the active one (Hidden) can't be dragged while it's hidden.
    let id = app.doc().unwrap().project.manifest.layers[1].id.clone();
    app.doc_mut().unwrap().active = Some(id.clone());
    app.snap = false;
    let (from, to) = (at(&app, [20.0, 20.0]), at(&app, [30.0, 27.0]));
    let mid = pos2((from.x + to.x) / 2.0, (from.y + to.y) / 2.0);
    frame(&off, &mut app, &drag(&[from, mid, to], egui::Modifiers::NONE));
    let doc = app.doc_mut().unwrap();
    assert_eq!(doc.layer(&id).unwrap().transform.origin, [10.0, 7.0], "the layer followed the pointer by whole pixels");
    assert_eq!(doc.undo_title(), Some("Transform Layer"));
    snapshot(&off, &mut app, "move");
    let doc = app.doc_mut().unwrap();
    doc.undo();
    assert_eq!(doc.layer(&id).unwrap().transform.origin, [0.0, 0.0]);
}

#[test]
fn a_move_drag_snaps_to_the_canvas_edge() {
    let off = Offscreen::new().unwrap();
    let mut app = open(&off, "blend/stack", "move");
    let id = app.doc().unwrap().project.manifest.layers[1].id.clone();
    app.doc_mut().unwrap().active = Some(id.clone());
    // A pixel off the left edge is within the 10-point pull at this zoom (about 9 points a pixel).
    let (from, to) = (at(&app, [20.0, 20.0]), at(&app, [21.0, 40.0]));
    frame(&off, &mut app, &drag(&[from, to], egui::Modifiers::NONE));
    let doc = app.doc().unwrap();
    assert_eq!(doc.layer(&id).unwrap().transform.origin[0], 0.0, "snapped back to the canvas's left edge");
}

#[test]
fn a_brush_drag_paints_what_the_stroke_op_paints() {
    let off = Offscreen::new().unwrap();
    let mut app = open(&off, "painting/brush-hard", "brush");
    app.settings.brush.size = 12.0;
    app.settings.foreground = [0.9, 0.2, 0.1];
    let points: Vec<[f64; 2]> = vec![[6.0, 32.0], [18.0, 46.0], [30.0, 35.0], [46.0, 18.0], [58.0, 32.0]];
    let screen: Vec<Pos2> = points.iter().map(|p| at(&app, *p)).collect();
    let before = app.doc().unwrap().project.clone();
    frame(&off, &mut app, &drag(&screen, egui::Modifiers::NONE));
    assert!(app.alert.is_none(), "{:?}", app.alert.as_ref().map(|a| &a.message));
    snapshot(&off, &mut app, "brush");
    assert_eq!(app.doc().unwrap().undo_title(), Some("Brush Stroke"));
    // The same stroke through the engine, from where the pointer events land in the document.
    let d = app.doc().unwrap();
    let landed: Vec<[f64; 2]> = screen.iter().map(|p| canvas_tools::to_doc(&d.view, d.size(), app.canvas_rect, *p)).collect();
    let mut expected = before;
    let op = json!({
        "op": "stroke", "tool": "brush", "layer": d.project.manifest.layers[0].id, "points": landed,
        "settings": { "size": 12.0, "hardness": 1.0, "opacity": 1.0, "color": [0.9f32 as f64, 0.2f32 as f64, 0.1f32 as f64] },
    });
    engine::paint::apply(&off.gfx.engine.gpu, &mut expected, &mut engine::paint::Session::default(), &op).unwrap();
    let id = &expected.manifest.layers[0].id;
    assert!(expected.images[id].pixels.as_raw() == &layer_pixels(&app, 0), "the app's stroke matches the engine's");
}

#[test]
fn a_marquee_drag_selects_and_deselect_undoes() {
    let off = Offscreen::new().unwrap();
    let mut app = open(&off, "blend/stack", "marquee");
    app.snap = false;
    let (from, to) = (at(&app, [10.0, 12.0]), at(&app, [40.0, 30.0]));
    frame(&off, &mut app, &drag(&[from, to], egui::Modifiers::NONE));
    let doc = app.doc_mut().unwrap();
    assert!(doc.selection.is_some(), "the drag made a selection");
    assert_eq!(doc.undo_title(), Some("Rectangular Marquee"));
    snapshot(&off, &mut app, "marquee");
    let doc = app.doc_mut().unwrap();
    assert!(crate::ui::selection::contains(doc, [20.0, 20.0]) && !crate::ui::selection::contains(doc, [5.0, 5.0]));
    doc.undo();
    assert!(app.doc().unwrap().selection.is_none());
}

#[test]
fn crop_and_return_crop_the_canvas() {
    let off = Offscreen::new().unwrap();
    let mut app = open(&off, "blend/stack", "crop");
    app.snap = false;
    let (from, to) = (at(&app, [8.0, 4.0]), at(&app, [40.0, 50.0]));
    frame(&off, &mut app, &drag(&[from, to], egui::Modifiers::NONE));
    assert_eq!(app.doc().unwrap().crop, Some([8.0, 4.0, 32.0, 46.0]));
    snapshot(&off, &mut app, "crop");
    let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    frame(&off, &mut app, &[(1, enter)]);
    let doc = app.doc().unwrap();
    assert_eq!((doc.project.manifest.width, doc.project.manifest.height), (32, 46));
    assert_eq!(doc.undo_title(), Some("Crop"));
}

#[test]
fn layer_commands_make_one_undo_step_each() {
    let off = Offscreen::new().unwrap();
    let mut app = open(&off, "blend/stack", "move");
    let ctx = egui::Context::default();
    let count = |app: &App| app.doc().unwrap().project.manifest.layers.len();
    let n = count(&app);
    app.run(&ctx, Command::NewBlankLayer);
    assert_eq!(count(&app), n + 1);
    assert_eq!(app.doc().unwrap().active_layer().unwrap().name, "Layer 1");
    app.run(&ctx, Command::DuplicateLayer);
    assert_eq!(count(&app), n + 2);
    app.run(&ctx, Command::DeleteLayerOrMask);
    app.run(&ctx, Command::DeleteLayerOrMask);
    assert_eq!(count(&app), n);
    // Merge Down of the top two pixel layers: one layer holding the two composited on their own.
    let gfx = off.gfx.clone();
    let top = app.doc().unwrap().project.manifest.layers.iter().rposition(|l| l.is_visible).unwrap();
    let mut pair = app.doc().unwrap().project.clone();
    pair.manifest.layers = pair.manifest.layers[top - 1..=top].to_vec();
    let c = engine::composite::Compositor::new(&gfx.gpu, &pair).render().unwrap();
    let expected = gfx.gpu.download(&engine::blend::unpremultiply(&gfx.gpu, &c)).unwrap();
    let id = app.doc().unwrap().project.manifest.layers[top].id.clone();
    app.doc_mut().unwrap().active = Some(id);
    app.run(&ctx, Command::MergeLayers);
    assert_eq!(count(&app), n - 1);
    let doc = app.doc().unwrap();
    assert_eq!(doc.undo_title(), Some("Merge Down"));
    let merged = doc.active_layer().unwrap();
    assert_eq!(merged.name, "Color", "the merged layer takes the lower one's name");
    assert_eq!(doc.project.images[&merged.id].pixels.as_raw(), &expected);
    app.doc_mut().unwrap().undo();
    assert_eq!(count(&app), n);
}

#[test]
fn masks_add_invert_and_delete() {
    let off = Offscreen::new().unwrap();
    let mut app = open(&off, "blend/stack", "move");
    let ctx = egui::Context::default();
    app.run(&ctx, Command::AddMask(true));
    let doc = app.doc().unwrap();
    let id = doc.active.clone().unwrap();
    assert!(doc.mask_target && doc.layer(&id).unwrap().mask_file.is_some());
    assert_eq!(doc.project.masks[&id].pixels.as_raw(), &vec![255u8]);
    app.run(&ctx, Command::Invert);
    assert_eq!(app.doc().unwrap().project.masks[&id].pixels.as_raw(), &vec![0u8]);
    assert_eq!(app.doc().unwrap().undo_title(), Some("Invert Mask"));
    app.run(&ctx, Command::DeleteLayerOrMask);
    assert!(app.doc().unwrap().layer(&id).unwrap().mask_file.is_none());
}

#[test]
fn a_filter_sheet_previews_and_applies_the_filter_op() {
    let off = Offscreen::new().unwrap();
    let mut app = open(&off, "filters/exposure", "move");
    let ctx = egui::Context::default();
    let before = app.doc().unwrap().project.clone();
    app.run(&ctx, Command::Filter("Exposure"));
    assert!(app.sheet.is_some());
    if let Some(ui::dialogs::Sheet::Filter(f)) = &mut app.sheet {
        f.set_setting("exposure.exposure", json!(1.5));
    }
    frame(&off, &mut app, &[]);
    assert!(app.doc().unwrap().preview.is_some(), "the canvas previews the filter");
    let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    frame(&off, &mut app, &[(1, enter)]);
    if let Some(ui::dialogs::Sheet::Filter(f)) = &app.sheet {
        panic!("the sheet stayed open: {:?}", f.error());
    }
    let doc = app.doc().unwrap();
    assert_eq!(doc.undo_title(), Some("Exposure"));
    let mut expected = before;
    let id = doc.active.clone().unwrap();
    let mut settings = ui::dialogs::default_settings();
    settings["exposure"]["exposure"] = json!(1.5);
    off.gfx.engine.apply_op(&mut expected, &json!({ "op": "filter", "layer": id, "kind": "Exposure", "settings": settings })).unwrap();
    assert!(expected.images[&id].pixels == doc.project.images[&id].pixels, "the sheet's OK is the filter op");
}

#[test]
fn tool_keys_pick_tools() {
    let off = Offscreen::new().unwrap();
    let mut app = open(&off, "blend/stack", "move");
    let key = |k| egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    frame(&off, &mut app, &[(1, key(egui::Key::B))]);
    assert_eq!(app.tool, Tool::Brush);
    frame(&off, &mut app, &[(1, key(egui::Key::CloseBracket))]);
    assert_eq!(app.settings.brush.size, 50.0, "] steps the brush up");
}

#[test]
fn painting_a_new_blank_layer_gives_it_pixels() {
    let off = Offscreen::new().unwrap();
    let mut app = open(&off, "blend/stack", "brush");
    let ctx = egui::Context::default();
    app.run(&ctx, Command::NewBlankLayer);
    let id = app.doc().unwrap().active.clone().unwrap();
    let (from, to) = (at(&app, [10.0, 10.0]), at(&app, [40.0, 30.0]));
    frame(&off, &mut app, &drag(&[from, to], egui::Modifiers::NONE));
    assert!(app.alert.is_none(), "{:?}", app.alert.as_ref().map(|a| &a.message));
    let doc = app.doc().unwrap();
    let pixels = &doc.project.images[&id].pixels;
    assert!(pixels.get_pixel(20, 16)[3] == 255 && pixels.get_pixel(60, 60)[3] == 0, "the stroke is painted and the rest is clear");
    assert_eq!(doc.layer(&id).unwrap().image_file.as_deref(), Some(format!("{id}.png").as_str()));
}
